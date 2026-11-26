//! The delivery queue (docs/PROTOCOL.md section 7).
//!
//! A message is one sealed copy plus a pending mark per recipient. It leaves the queue
//! in exactly two ways: every recipient has acknowledged it, or the channel's maximum
//! message age has passed. The server never looks inside a body.

use std::time::Duration;

use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use deaddrop_core::encoding::{b64, random_id, unb64};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;

use crate::auth::{self, Auth};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, Wake};
use crate::store;

/// Largest sealed body, in bytes.
pub const MAX_BODY_BYTES: usize = 64 * 1024;
const PAGE: usize = 100;

#[derive(Deserialize)]
pub struct Send {
    /// The sealed message, base64. Opaque to the server.
    body: String,
    /// Deliver only to these members (key hand-overs between two devices). Left out,
    /// the message goes to every other member.
    #[serde(default)]
    to: Option<Vec<String>>,
}

/// `POST /v1/channels/{channel}/messages`
pub async fn send(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel): Path<String>,
    Json(request): Json<Send>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let body = unb64(&request.body).filter(|body| !body.is_empty()).ok_or(ApiError::bad("body must be base64"))?;
    if body.len() > MAX_BODY_BYTES {
        return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "message too large"));
    }
    let now = state.clock.now();
    let (id, expires, recipients) = state
        .store
        .run(move |db| {
            let max_age: Option<i64> =
                db.query_row("SELECT max_age FROM channels WHERE id = ?1", [&channel], |row| row.get(0)).optional()?;
            let (Some(max_age), true) = (max_age, store::is_member(db, &channel, &auth.device)?) else {
                return Err(ApiError::not_found());
            };
            let mut recipients = store::member_devices(db, &channel)?;
            recipients.retain(|device| *device != auth.device);
            if let Some(mut to) = request.to {
                to.sort();
                to.dedup();
                if !to.iter().all(|device| recipients.contains(device)) {
                    return Err(ApiError::bad("every recipient must be another member of the channel"));
                }
                recipients = to;
            }
            let id = random_id();
            let expires = now.saturating_add(max_age);
            if recipients.is_empty() {
                // Nobody to deliver to, so nothing is stored at all.
                return Ok((id, expires, recipients));
            }
            let tx = db.transaction()?;
            tx.execute(
                "INSERT INTO messages (id, channel, sender, body, created, expires) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, channel, auth.device, body, now, expires],
            )?;
            let seq = tx.last_insert_rowid();
            for device in &recipients {
                tx.execute("INSERT INTO pending (device, seq) VALUES (?1, ?2)", params![device, seq])?;
            }
            tx.commit()?;
            Ok((id, expires, recipients))
        })
        .await?;
    state.wake_all(&recipients, Wake::Message);
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "recipients": recipients.len(), "expires": expires }))))
}

struct Queued {
    seq: i64,
    json: Value,
}

/// The next messages waiting for `device`, oldest first, after position `after`.
fn waiting(db: &Connection, device: &str, after: i64, now: i64, limit: usize) -> rusqlite::Result<Vec<Queued>> {
    let mut query = db.prepare(
        "SELECT m.seq, m.id, m.channel, m.sender, m.body, m.created, m.expires
         FROM pending p JOIN messages m ON m.seq = p.seq
         WHERE p.device = ?1 AND p.seq > ?2 AND m.expires > ?3
         ORDER BY p.seq LIMIT ?4",
    )?;
    query
        .query_map(params![device, after, now, limit as i64], |row| {
            Ok(Queued {
                seq: row.get(0)?,
                json: json!({
                    "id": row.get::<_, String>(1)?,
                    "channel": row.get::<_, String>(2)?,
                    "sender": row.get::<_, String>(3)?,
                    "body": b64(&row.get::<_, Vec<u8>>(4)?),
                    "sent": row.get::<_, i64>(5)?,
                    "expires": row.get::<_, i64>(6)?,
                }),
            })
        })?
        .collect()
}

/// Removes `device`'s pending marks for `ids` and wipes every message that leaves
/// with nobody waiting.
fn acknowledge(db: &mut Connection, device: &str, ids: &[String]) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    let mut wiped = 0;
    for id in ids {
        tx.execute(
            "DELETE FROM pending WHERE device = ?1 AND seq = (SELECT seq FROM messages WHERE id = ?2)",
            [device, id.as_str()],
        )?;
        wiped += tx.execute(
            "DELETE FROM messages WHERE id = ?1
             AND NOT EXISTS (SELECT 1 FROM pending WHERE pending.seq = messages.seq)",
            [id.as_str()],
        )?;
    }
    tx.commit()?;
    if wiped > 0 {
        store::checkpoint(db)?;
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct FetchQuery {
    #[serde(default)]
    limit: Option<usize>,
}

/// `GET /v1/queue`: what is waiting for the caller. Messages stay queued until
/// acknowledged, so a phone that crashes mid-download loses nothing.
pub async fn fetch(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<FetchQuery>,
) -> ApiResult<Json<Value>> {
    let limit = query.limit.unwrap_or(PAGE).clamp(1, 500);
    let now = state.clock.now();
    let messages = state.store.run(move |db| Ok(waiting(db, &auth.device, 0, now, limit)?)).await?;
    let messages: Vec<Value> = messages.into_iter().map(|message| message.json).collect();
    Ok(Json(json!({ "messages": messages })))
}

#[derive(Deserialize)]
pub struct Ack {
    ids: Vec<String>,
}

/// `POST /v1/queue/ack`: the caller has safely stored these messages.
pub async fn ack(State(state): State<AppState>, auth: Auth, Json(body): Json<Ack>) -> ApiResult<StatusCode> {
    if body.ids.len() > 1000 {
        return Err(ApiError::bad("at most 1000 ids at a time"));
    }
    state.store.run(move |db| Ok(acknowledge(db, &auth.device, &body.ids)?)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/events` (WebSocket): messages pushed the moment they arrive.
///
/// The client's first frame is `{"token": "<session token>"}` (in a frame rather than
/// the URL, so it can never end up in a proxy's log). The server answers
/// `{"type":"ready"}`, then sends `{"type":"message", ...}` for everything waiting and
/// everything new, and `{"type":"changed"}` when a channel, roster or join request the
/// device can see has changed. The client acknowledges with `{"ack": ["<id>", ...]}`.
pub async fn events(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| async move {
        // Errors here only ever mean the connection has gone.
        let _ = serve(state, socket).await;
    })
}

#[derive(Deserialize)]
struct Hello {
    token: String,
}

#[derive(Deserialize)]
struct ClientFrame {
    #[serde(default)]
    ack: Vec<String>,
}

fn text(value: Value) -> Message {
    Message::Text(value.to_string().into())
}

async fn serve(state: AppState, mut socket: WebSocket) -> Result<(), ()> {
    let first = tokio::time::timeout(Duration::from_secs(10), socket.recv()).await;
    let Ok(Some(Ok(Message::Text(first)))) = first else { return Err(()) };
    let hello: Hello = serde_json::from_str(first.as_str()).map_err(drop)?;
    let Ok(auth) = auth::authenticate(&state, &hello.token).await else {
        let _ = socket.send(text(json!({ "type": "error", "error": "sign in first" }))).await;
        return Err(());
    };
    let device = auth.device;

    // Subscribe before the first look at the queue, so nothing can slip between them.
    let mut wakes = state.subscribe();
    socket.send(text(json!({ "type": "ready" }))).await.map_err(drop)?;

    let mut sent_up_to = 0i64;
    let mut keepalive = tokio::time::interval(Duration::from_secs(30));
    loop {
        // Push everything waiting that this connection has not already pushed.
        loop {
            let (owner, after, now) = (device.clone(), sent_up_to, state.clock.now());
            let batch = state.store.run(move |db| Ok(waiting(db, &owner, after, now, PAGE)?)).await.map_err(drop)?;
            let full = batch.len() == PAGE;
            for message in batch {
                sent_up_to = message.seq;
                let mut frame = message.json;
                frame["type"] = json!("message");
                socket.send(text(frame)).await.map_err(drop)?;
            }
            if !full {
                break;
            }
        }

        // Then sleep until something concerns this device.
        loop {
            tokio::select! {
                wake = wakes.recv() => match wake {
                    Ok((target, kind)) if *target == *device => {
                        if kind == Wake::Changed {
                            socket.send(text(json!({ "type": "changed" }))).await.map_err(drop)?;
                        }
                        break;
                    }
                    Ok(_) => {}
                    // Too busy to keep up with the nudges: look at everything again.
                    Err(RecvError::Lagged(_)) => {
                        socket.send(text(json!({ "type": "changed" }))).await.map_err(drop)?;
                        break;
                    }
                    Err(RecvError::Closed) => return Err(()),
                },
                frame = socket.recv() => match frame {
                    Some(Ok(Message::Text(frame))) => {
                        if let Ok(ClientFrame { ack }) = serde_json::from_str(frame.as_str())
                            && !ack.is_empty() && ack.len() <= 1000
                        {
                            let owner = device.clone();
                            state.store.run(move |db| Ok(acknowledge(db, &owner, &ack)?)).await.map_err(drop)?;
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return Ok(()),
                    Some(Ok(_)) => {}
                },
                _ = keepalive.tick() => {
                    socket.send(Message::Ping(Vec::new().into())).await.map_err(drop)?;
                }
            }
        }
    }
}

/// Wipes what has passed its channel's maximum message age, and tidies expired
/// sign-in leftovers. Returns how many messages were wiped.
pub fn sweep(db: &mut Connection, now: i64) -> rusqlite::Result<usize> {
    let tx = db.transaction()?;
    // Pending marks go with their message (ON DELETE CASCADE).
    let wiped = tx.execute("DELETE FROM messages WHERE expires <= ?1", [now])?;
    tx.execute("DELETE FROM sessions WHERE expires <= ?1", [now])?;
    tx.execute("DELETE FROM challenges WHERE expires <= ?1", [now])?;
    tx.execute("DELETE FROM tokens WHERE expires <= ?1", [now])?;
    tx.commit()?;
    store::checkpoint(db)?;
    Ok(wiped)
}
