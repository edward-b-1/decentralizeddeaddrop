//! Messaging keys and prekeys (docs/PROTOCOL.md section 6).
//!
//! The server only keeps and hands out these public keys. It checks that a device's
//! messaging keys are signed by that device, but phones never rely on that: each checks
//! the whole chain itself, from the owner-signed roster down to the prekey.

use axum::Json;
use axum::extract::{Path, State};
use deaddrop_core::records::{MessagingKeys, Signed, SignedKey};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store;

/// One-time prekeys accepted per upload, and kept per device.
const MAX_UPLOAD: usize = 50;
const MAX_STORED: i64 = 100;
/// Generous bound on a base64 Curve25519 key or Ed25519 signature.
const MAX_FIELD: usize = 128;

const BAD_KEYS: ApiError = ApiError::bad("messaging keys are not valid or not signed by this device");

fn sane(key: &SignedKey) -> bool {
    !key.key.is_empty() && key.key.len() <= MAX_FIELD && !key.signature.is_empty() && key.signature.len() <= MAX_FIELD
}

#[derive(Deserialize)]
pub struct Publish {
    keys: Signed,
    #[serde(default)]
    one_time: Vec<SignedKey>,
    #[serde(default)]
    fallback: Option<SignedKey>,
}

fn counts(db: &Connection, device: &str) -> rusqlite::Result<Value> {
    let published: bool =
        db.query_row("SELECT EXISTS (SELECT 1 FROM messaging_keys WHERE device = ?1)", [device], |row| row.get(0))?;
    let (one_time, fallback): (i64, i64) = db.query_row(
        "SELECT count(*) FILTER (WHERE fallback = 0), count(*) FILTER (WHERE fallback = 1)
         FROM prekeys WHERE device = ?1",
        [device],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(json!({ "published": published, "one_time": one_time, "fallback": fallback > 0 }))
}

/// `PUT /v1/keys`: publish this device's signed messaging keys, and add prekeys.
///
/// Publishing different messaging keys from before discards every prekey made with
/// the old ones.
pub async fn publish(State(state): State<AppState>, auth: Auth, Json(body): Json<Publish>) -> ApiResult<Json<Value>> {
    if body.one_time.len() > MAX_UPLOAD || !body.one_time.iter().chain(&body.fallback).all(sane) {
        return Err(ApiError::bad("at most 50 one-time prekeys, each a key and a signature"));
    }
    state
        .store
        .run(move |db| {
            let signing_key = store::signing_key(db, &auth.device)?.ok_or(ApiError::internal())?;
            let keys: MessagingKeys = body.keys.open(&signing_key).map_err(|_| BAD_KEYS)?;
            if keys.device != auth.device || keys.curve25519.len() > MAX_FIELD || keys.ed25519.len() > MAX_FIELD {
                return Err(BAD_KEYS);
            }
            let tx = db.transaction()?;
            let before: Option<String> = tx
                .query_row("SELECT record FROM messaging_keys WHERE device = ?1", [&auth.device], |row| row.get(0))
                .optional()?;
            let same_keys = before
                .and_then(|record| serde_json::from_str::<MessagingKeys>(&record).ok())
                .is_some_and(|old| old.curve25519 == keys.curve25519 && old.ed25519 == keys.ed25519);
            if !same_keys {
                tx.execute("DELETE FROM prekeys WHERE device = ?1", [&auth.device])?;
            }
            tx.execute(
                "INSERT INTO messaging_keys (device, record, signature) VALUES (?1, ?2, ?3)
                 ON CONFLICT (device) DO UPDATE SET record = ?2, signature = ?3",
                params![auth.device, body.keys.record, body.keys.signature],
            )?;
            if let Some(fallback) = &body.fallback {
                tx.execute("DELETE FROM prekeys WHERE device = ?1 AND fallback = 1", [&auth.device])?;
                tx.execute(
                    "INSERT OR IGNORE INTO prekeys (device, key, signature, fallback) VALUES (?1, ?2, ?3, 1)",
                    params![auth.device, fallback.key, fallback.signature],
                )?;
            }
            let mut stored: i64 = tx.query_row(
                "SELECT count(*) FROM prekeys WHERE device = ?1 AND fallback = 0",
                [&auth.device],
                |row| row.get(0),
            )?;
            for prekey in &body.one_time {
                if stored >= MAX_STORED {
                    break;
                }
                stored += tx.execute(
                    "INSERT OR IGNORE INTO prekeys (device, key, signature, fallback) VALUES (?1, ?2, ?3, 0)",
                    params![auth.device, prekey.key, prekey.signature],
                )? as i64;
            }
            tx.commit()?;
            Ok(Json(counts(db, &auth.device)?))
        })
        .await
}

/// `GET /v1/keys`: how many of this device's prekeys are left, so it knows when to
/// upload more.
pub async fn status(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    state.store.run(move |db| Ok(Json(counts(db, &auth.device)?))).await
}

#[derive(Deserialize)]
pub struct Claim {
    devices: Vec<String>,
}

/// `POST /v1/channels/{channel}/keys/claim`: for each named member, their signed
/// messaging keys and one prekey to start a private session with.
///
/// A one-time prekey is handed out once and deleted. When a device has none left, its
/// fallback prekey is handed out instead (and kept). Only members of the same channel
/// can claim, which limits who can use a device's prekeys up.
pub async fn claim(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel): Path<String>,
    Json(body): Json<Claim>,
) -> ApiResult<Json<Value>> {
    if body.devices.len() > 1000 {
        return Err(ApiError::bad("at most 1000 devices at a time"));
    }
    state
        .store
        .run(move |db| {
            if !store::is_member(db, &channel, &auth.device)? {
                return Err(ApiError::not_found());
            }
            let tx = db.transaction()?;
            let mut claimed = Vec::new();
            for device in &body.devices {
                if *device == auth.device || !store::is_member(&tx, &channel, device)? {
                    return Err(ApiError::bad("every device must be another member of the channel"));
                }
                let keys: Option<(String, String)> = tx
                    .query_row("SELECT record, signature FROM messaging_keys WHERE device = ?1", [device], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })
                    .optional()?;
                // One-time prekeys first, oldest first; the fallback only when they are gone.
                let prekey: Option<(i64, String, String, bool)> = tx
                    .query_row(
                        "SELECT id, key, signature, fallback FROM prekeys WHERE device = ?1
                         ORDER BY fallback, id LIMIT 1",
                        [device],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .optional()?;
                if let Some((id, _, _, false)) = &prekey {
                    tx.execute("DELETE FROM prekeys WHERE id = ?1", [id])?;
                }
                claimed.push(json!({
                    "device_id": device,
                    "keys": keys.map(|(record, signature)| json!({ "record": record, "signature": signature })),
                    "prekey": prekey.as_ref().map(|(_, key, signature, _)| json!({ "key": key, "signature": signature })),
                    "fallback": prekey.is_some_and(|(_, _, _, fallback)| fallback),
                }));
            }
            tx.commit()?;
            Ok(Json(json!({ "claimed": claimed })))
        })
        .await
}
