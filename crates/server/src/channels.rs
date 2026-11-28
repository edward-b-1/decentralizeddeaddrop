//! Channels: owner-signed settings, the owner-signed member list (roster), invites and
//! join requests.
//!
//! The server checks every record against the owner's key before storing it, but the
//! real protection is that phones check too: nothing here lets the server add a member,
//! keep a removed one, or change a channel's settings. Nor can it get a member dropped:
//! the owner's phone drops one unasked only on that member's own signed word
//! (`records::Leave`), which the server keeps and hands on but cannot make.

use std::collections::HashSet;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use deaddrop_core::encoding::{ID_LEN, b64, is_base32};
use deaddrop_core::records::{ChannelRecord, Roster, Signed};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::operator::NewInvite;
use crate::state::{AppState, Wake};
use crate::store;

const MAX_MEMBERS: usize = 1000;
/// A sanity bound only (a century); how long messages wait is the channel owner's choice.
const MAX_MAX_AGE: u64 = 100 * 366 * 86_400;

const NOT_OWNER: ApiError = ApiError::forbidden("only the channel owner can do that");
const BAD_CHANNEL: ApiError = ApiError::bad("channel record is not valid or not signed by its owner");
const BAD_ROSTER: ApiError = ApiError::bad("roster is not valid or not signed by the channel owner");

fn valid_name(name: &str, max_chars: usize) -> bool {
    let count = name.chars().count();
    count >= 1 && count <= max_chars && name.trim() == name && !name.chars().any(char::is_control)
}

fn check_channel(record: &ChannelRecord) -> ApiResult<()> {
    let ok = is_base32(&record.id, ID_LEN)
        && valid_name(&record.name, 64)
        && (1..=MAX_MAX_AGE).contains(&record.max_age)
        && (1..=i64::MAX as u64).contains(&record.version);
    if ok { Ok(()) } else { Err(BAD_CHANNEL) }
}

/// Checks a roster's own shape: right channel, includes the owner, no device or name
/// twice (names compared without regard to case).
fn check_roster(roster: &Roster, channel: &str, owner: &str) -> ApiResult<()> {
    let mut devices = HashSet::new();
    let mut names = HashSet::new();
    let ok = roster.channel == channel
        && (1..=i64::MAX as u64).contains(&roster.version)
        && (1..=MAX_MEMBERS).contains(&roster.members.len())
        && roster.members.iter().any(|member| member.device == owner)
        && roster.members.iter().all(|member| {
            valid_name(&member.name, 32)
                && devices.insert(member.device.as_str())
                && names.insert(member.name.to_lowercase())
        });
    if ok { Ok(()) } else { Err(BAD_ROSTER) }
}

struct Channel {
    owner: String,
    version: u64,
    roster_version: u64,
}

fn load(db: &Connection, channel: &str) -> ApiResult<Channel> {
    db.query_row("SELECT owner, version, roster_version FROM channels WHERE id = ?1", [channel], |row| {
        Ok(Channel {
            owner: row.get(0)?,
            version: row.get::<_, i64>(1)? as u64,
            roster_version: row.get::<_, i64>(2)? as u64,
        })
    })
    .optional()?
    .ok_or(ApiError::not_found())
}

fn load_as_owner(db: &Connection, channel: &str, device: &str) -> ApiResult<Channel> {
    let found = load(db, channel)?;
    if found.owner == device { Ok(found) } else { Err(NOT_OWNER) }
}

fn owner_key(db: &Connection, owner: &str) -> ApiResult<Vec<u8>> {
    store::signing_key(db, owner)?.ok_or(ApiError::internal())
}

#[derive(Deserialize)]
pub struct Create {
    channel: Signed,
    roster: Signed,
}

/// `POST /v1/channels`: the caller becomes the owner and, so far, the only member.
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<Create>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let id = state
        .store
        .run(move |db| {
            let key = owner_key(db, &auth.device)?;
            let record: ChannelRecord = body.channel.open(&key).map_err(|_| BAD_CHANNEL)?;
            check_channel(&record)?;
            if record.owner != auth.device {
                return Err(BAD_CHANNEL);
            }
            let roster: Roster = body.roster.open(&key).map_err(|_| BAD_ROSTER)?;
            check_roster(&roster, &record.id, &auth.device)?;
            if roster.members.len() != 1 {
                return Err(ApiError::bad("a new channel starts with only its owner"));
            }
            let tx = db.transaction()?;
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO channels
                    (id, owner, record, signature, version, max_age, roster, roster_sig, roster_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    record.id,
                    auth.device,
                    body.channel.record,
                    body.channel.signature,
                    record.version as i64,
                    record.max_age as i64,
                    body.roster.record,
                    body.roster.signature,
                    roster.version as i64,
                ],
            )?;
            if inserted == 0 {
                return Err(ApiError::conflict("that channel ID is taken"));
            }
            tx.execute("INSERT INTO members (channel, device) VALUES (?1, ?2)", [&record.id, &auth.device])?;
            tx.commit()?;
            Ok(record.id)
        })
        .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// `GET /v1/channels`: the caller's channels and the join requests they have open.
pub async fn mine(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let (channels, requests) = state
        .store
        .run(move |db| {
            let mut query = db.prepare(
                "SELECT c.id, c.owner = ?1, c.version, c.roster_version
                 FROM members m JOIN channels c ON c.id = m.channel
                 WHERE m.device = ?1 AND m.gone = 0 ORDER BY c.id",
            )?;
            let channels = query
                .query_map([&auth.device], |row| {
                    Ok(json!({
                        "id": row.get::<_, String>(0)?,
                        "owner": row.get::<_, bool>(1)?,
                        "version": row.get::<_, i64>(2)?,
                        "roster_version": row.get::<_, i64>(3)?,
                    }))
                })?
                .collect::<rusqlite::Result<Vec<Value>>>()?;
            let mut query = db.prepare("SELECT channel FROM join_requests WHERE device = ?1 ORDER BY channel")?;
            let requests =
                query.query_map([&auth.device], |row| row.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok((channels, requests))
        })
        .await?;
    Ok(Json(json!({ "channels": channels, "requests": requests })))
}

/// `GET /v1/channels/{channel}`: the signed channel record and the owner's key, for
/// anyone on the server who knows the channel's ID (they are unguessable). A joiner
/// compares the owner's key with the fingerprint in their invite.
pub async fn show(State(state): State<AppState>, auth: Auth, Path(channel): Path<String>) -> ApiResult<Json<Value>> {
    state
        .store
        .run(move |db| {
            let found: Option<(String, String, String, i64)> = db
                .query_row(
                    "SELECT record, signature, owner, roster_version FROM channels WHERE id = ?1",
                    [&channel],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let (record, signature, owner, roster_version) = found.ok_or(ApiError::not_found())?;
            Ok(Json(json!({
                "channel": { "record": record, "signature": signature },
                "owner_key": b64(&owner_key(db, &owner)?),
                "roster_version": roster_version,
                "member": store::is_member(db, &channel, &auth.device)?,
            })))
        })
        .await
}

#[derive(Deserialize)]
pub struct Update {
    channel: Signed,
}

/// `PUT /v1/channels/{channel}`: the owner replaces the channel record (name, maximum
/// message age). A new maximum message age applies to messages already queued too.
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel): Path<String>,
    Json(body): Json<Update>,
) -> ApiResult<StatusCode> {
    let members = state
        .store
        .run(move |db| {
            let current = load_as_owner(db, &channel, &auth.device)?;
            let record: ChannelRecord = body.channel.open(&owner_key(db, &auth.device)?).map_err(|_| BAD_CHANNEL)?;
            check_channel(&record)?;
            if record.id != channel || record.owner != auth.device {
                return Err(BAD_CHANNEL);
            }
            if record.version <= current.version {
                return Err(ApiError::conflict("channel version must go up"));
            }
            let tx = db.transaction()?;
            tx.execute(
                "UPDATE channels SET record = ?2, signature = ?3, version = ?4, max_age = ?5 WHERE id = ?1",
                params![
                    channel,
                    body.channel.record,
                    body.channel.signature,
                    record.version as i64,
                    record.max_age as i64
                ],
            )?;
            tx.execute(
                "UPDATE messages SET expires = created + ?2 WHERE channel = ?1",
                params![channel, record.max_age as i64],
            )?;
            tx.commit()?;
            Ok(store::member_devices(db, &channel)?)
        })
        .await?;
    state.wake_all(&members, Wake::Changed);
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /v1/channels/{channel}`: the owner deletes the channel and its queue.
pub async fn destroy(State(state): State<AppState>, auth: Auth, Path(channel): Path<String>) -> ApiResult<StatusCode> {
    let members = state
        .store
        .run(move |db| {
            load_as_owner(db, &channel, &auth.device)?;
            let members = store::member_devices(db, &channel)?;
            db.execute("DELETE FROM channels WHERE id = ?1", [&channel])?;
            db.execute("DELETE FROM tokens WHERE scope = ?1", [&channel])?;
            store::checkpoint(db)?;
            Ok(members)
        })
        .await?;
    state.wake_all(&members, Wake::Changed);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/channels/{channel}/invites`: the owner makes a token that lets a device
/// ask to join.
pub async fn new_invite(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel): Path<String>,
    Json(body): Json<NewInvite>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    body.check()?;
    let expires = state.clock.now() + body.ttl;
    let token = state
        .store
        .run(move |db| {
            load_as_owner(db, &channel, &auth.device)?;
            Ok(store::create_token(db, &channel, body.uses, expires)?)
        })
        .await?;
    Ok((StatusCode::CREATED, Json(json!({ "token": token, "expires": expires }))))
}

#[derive(Deserialize)]
pub struct Ask {
    token: String,
    /// The name the device would like to go by in this channel.
    name: String,
}

/// `POST /v1/channels/{channel}/join-requests`: ask to join, with a channel invite.
pub async fn ask(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel): Path<String>,
    Json(body): Json<Ask>,
) -> ApiResult<StatusCode> {
    if !valid_name(&body.name, 32) {
        return Err(ApiError::bad("name must be 1 to 32 characters"));
    }
    let now = state.clock.now();
    let owner = state
        .store
        .run(move |db| {
            // An unknown channel and a bad invite look the same from outside.
            const REFUSED: ApiError = ApiError::forbidden("invite not accepted");
            let tx = db.transaction()?;
            let owner: Option<String> =
                tx.query_row("SELECT owner FROM channels WHERE id = ?1", [&channel], |row| row.get(0)).optional()?;
            let Some(owner) = owner else { return Err(REFUSED) };
            if store::is_member(&tx, &channel, &auth.device)? {
                return Err(ApiError::conflict("already a member"));
            }
            if !store::consume_token(&tx, &channel, &body.token, now)? {
                return Err(REFUSED);
            }
            tx.execute(
                "INSERT INTO join_requests (channel, device, name) VALUES (?1, ?2, ?3)
                 ON CONFLICT (channel, device) DO UPDATE SET name = ?3",
                params![channel, auth.device, body.name],
            )?;
            tx.commit()?;
            Ok(owner)
        })
        .await?;
    state.wake(&owner, Wake::Changed);
    Ok(StatusCode::ACCEPTED)
}

/// `GET /v1/channels/{channel}/join-requests`: for the owner, with each asker's keys.
pub async fn requests(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel): Path<String>,
) -> ApiResult<Json<Value>> {
    let requests = state
        .store
        .run(move |db| {
            load_as_owner(db, &channel, &auth.device)?;
            let mut query = db.prepare(
                "SELECT d.id, r.name, d.signing_key, d.agreement_key, d.agreement_sig
                 FROM join_requests r JOIN devices d ON d.id = r.device
                 WHERE r.channel = ?1 ORDER BY d.id",
            )?;
            let rows = query
                .query_map([&channel], |row| {
                    Ok(json!({
                        "device_id": row.get::<_, String>(0)?,
                        "name": row.get::<_, String>(1)?,
                        "signing_key": b64(&row.get::<_, Vec<u8>>(2)?),
                        "agreement_key": b64(&row.get::<_, Vec<u8>>(3)?),
                        "agreement_sig": b64(&row.get::<_, Vec<u8>>(4)?),
                    }))
                })?
                .collect::<rusqlite::Result<Vec<Value>>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(json!({ "requests": requests })))
}

/// `DELETE /v1/channels/{channel}/join-requests/{device}`: the owner declines.
pub async fn decline(
    State(state): State<AppState>,
    auth: Auth,
    Path((channel, device)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let asker = device.clone();
    let deleted = state
        .store
        .run(move |db| {
            load_as_owner(db, &channel, &auth.device)?;
            Ok(db.execute("DELETE FROM join_requests WHERE channel = ?1 AND device = ?2", [&channel, &device])?)
        })
        .await?;
    if deleted == 0 {
        return Err(ApiError::not_found());
    }
    state.wake(&asker, Wake::Changed);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct PutRoster {
    roster: Signed,
}

/// `PUT /v1/channels/{channel}/roster`: the owner publishes a new member list. This is
/// the only way members are added or removed.
///
/// Anyone new must have asked to join, so nobody is put in a channel unasked. Anyone
/// dropped stops receiving at once: what was still queued for them is wiped.
///
/// Someone in the last list may stay in this one whatever has become of them since:
/// whether to drop a member who has left, or whose device is gone, is the owner's
/// decision, not the server's. Staying listed does not start delivery to them again.
pub async fn put_roster(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel): Path<String>,
    Json(body): Json<PutRoster>,
) -> ApiResult<StatusCode> {
    let affected = state
        .store
        .run(move |db| {
            let current = load_as_owner(db, &channel, &auth.device)?;
            let roster: Roster = body.roster.open(&owner_key(db, &auth.device)?).map_err(|_| BAD_ROSTER)?;
            check_roster(&roster, &channel, &auth.device)?;
            if roster.version <= current.roster_version {
                return Err(ApiError::conflict("roster version must go up"));
            }

            let tx = db.transaction()?;
            let before: Vec<String> = {
                let mut query = tx.prepare("SELECT device FROM members WHERE channel = ?1")?;
                query.query_map([&channel], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
            };
            // Who the roster being replaced lists.
            let last: String =
                tx.query_row("SELECT roster FROM channels WHERE id = ?1", [&channel], |row| row.get(0))?;
            let last: Roster = serde_json::from_str(&last).map_err(|_| ApiError::internal())?;
            let was_listed: HashSet<&str> = last.members.iter().map(|member| member.device.as_str()).collect();
            let listed: HashSet<&str> = roster.members.iter().map(|member| member.device.as_str()).collect();

            // Everyone in the new list either was in the last one or has asked to join.
            let mut joining = Vec::new();
            for member in &roster.members {
                let asked: bool = tx.query_row(
                    "SELECT EXISTS (SELECT 1 FROM join_requests WHERE channel = ?1 AND device = ?2)",
                    [&channel, &member.device],
                    |row| row.get(0),
                )?;
                if asked {
                    joining.push(member.device.as_str());
                } else if !was_listed.contains(member.device.as_str()) {
                    return Err(ApiError::conflict("roster lists a device that has not asked to join"));
                }
            }
            for device in &before {
                if !listed.contains(device.as_str()) {
                    store::drop_pending(&tx, &channel, device)?;
                    tx.execute("DELETE FROM members WHERE channel = ?1 AND device = ?2", [&channel, device])?;
                }
            }
            // Delivery starts (or, for someone who left and has asked again, starts
            // again) only for those who asked. Anything they signed on leaving is spent.
            for device in joining {
                tx.execute(
                    "INSERT INTO members (channel, device) VALUES (?1, ?2)
                     ON CONFLICT (channel, device) DO UPDATE SET gone = 0",
                    [channel.as_str(), device],
                )?;
                tx.execute("DELETE FROM join_requests WHERE channel = ?1 AND device = ?2", [channel.as_str(), device])?;
                tx.execute("DELETE FROM leaves WHERE channel = ?1 AND device = ?2", [channel.as_str(), device])?;
            }
            // A leave has done its job once the owner's list no longer has the member.
            let kept: Vec<String> = {
                let mut query = tx.prepare("SELECT device FROM leaves WHERE channel = ?1")?;
                query.query_map([&channel], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
            };
            for device in kept.iter().filter(|device| !listed.contains(device.as_str())) {
                tx.execute("DELETE FROM leaves WHERE channel = ?1 AND device = ?2", [&channel, device])?;
            }
            tx.execute(
                "UPDATE channels SET roster = ?2, roster_sig = ?3, roster_version = ?4 WHERE id = ?1",
                params![channel, body.roster.record, body.roster.signature, roster.version as i64],
            )?;
            store::wipe_delivered(&tx)?;
            tx.commit()?;
            store::checkpoint(db)?;

            let mut affected = before;
            affected.extend(roster.members.into_iter().map(|member| member.device));
            Ok(affected)
        })
        .await?;
    state.wake_all(&affected, Wake::Changed);
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/channels/{channel}/members`: for members. The signed roster, who has left
/// since it was signed, what those who left signed to say so, and every listed device's
/// keys (which each phone checks against the device IDs in the roster).
///
/// `left` is only this server's word. `leaves` is each leaver's own, for phones to check.
pub async fn members(State(state): State<AppState>, auth: Auth, Path(channel): Path<String>) -> ApiResult<Json<Value>> {
    state
        .store
        .run(move |db| {
            if !store::is_member(db, &channel, &auth.device)? {
                return Err(ApiError::not_found());
            }
            let (roster, roster_sig): (String, String) =
                db.query_row("SELECT roster, roster_sig FROM channels WHERE id = ?1", [&channel], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?;
            let mut query = db.prepare(
                "SELECT d.id, m.gone, d.signing_key, d.agreement_key, d.agreement_sig, k.record, k.signature
                 FROM members m JOIN devices d ON d.id = m.device
                 LEFT JOIN messaging_keys k ON k.device = d.id
                 WHERE m.channel = ?1 ORDER BY d.id",
            )?;
            let mut gone = Vec::new();
            let mut keys = Vec::new();
            let mut rows = query.query([&channel])?;
            while let Some(row) = rows.next()? {
                let device: String = row.get(0)?;
                if row.get::<_, bool>(1)? {
                    gone.push(device.clone());
                }
                let messaging = match (row.get::<_, Option<String>>(5)?, row.get::<_, Option<String>>(6)?) {
                    (Some(record), Some(signature)) => json!({ "record": record, "signature": signature }),
                    _ => Value::Null,
                };
                keys.push(json!({
                    "device_id": device,
                    "signing_key": b64(&row.get::<_, Vec<u8>>(2)?),
                    "agreement_key": b64(&row.get::<_, Vec<u8>>(3)?),
                    "agreement_sig": b64(&row.get::<_, Vec<u8>>(4)?),
                    "messaging_keys": messaging,
                }));
            }
            drop(rows);
            let mut query = db.prepare(
                "SELECT device, signing_key, record, signature FROM leaves WHERE channel = ?1 ORDER BY device",
            )?;
            let leaves = query
                .query_map([&channel], |row| {
                    Ok(json!({
                        "device_id": row.get::<_, String>(0)?,
                        "signing_key": b64(&row.get::<_, Vec<u8>>(1)?),
                        "leave": { "record": row.get::<_, String>(2)?, "signature": row.get::<_, String>(3)? },
                    }))
                })?
                .collect::<rusqlite::Result<Vec<Value>>>()?;
            Ok(Json(json!({
                "roster": { "record": roster, "signature": roster_sig },
                "left": gone,
                "leaves": leaves,
                "keys": keys,
            })))
        })
        .await
}

#[derive(Deserialize)]
pub struct Leaving {
    leave: Signed,
}

/// `DELETE /v1/channels/{channel}/members/me`: leave. Delivery to the leaver stops now.
///
/// The body is `{"leave": ...}`, the leaver's signed [`deaddrop_core::records::Leave`].
/// The owner's phone checks it and then signs a roster without them. Versions up to
/// 0.1.1 send no body: their leaving is then only this server's word, and it is for
/// the channel's owner to decide whether to drop them.
pub async fn leave(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel): Path<String>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    const BAD_LEAVE: ApiError = ApiError::bad("leave is not valid or not signed by this device");
    let leaving: Option<Leaving> =
        if body.is_empty() { None } else { Some(serde_json::from_slice(&body).map_err(|_| BAD_LEAVE)?) };
    let remaining = state
        .store
        .run(move |db| {
            let current = load(db, &channel)?;
            if !store::is_member(db, &channel, &auth.device)? {
                return Err(ApiError::not_found());
            }
            if current.owner == auth.device {
                return Err(ApiError::conflict("the owner cannot leave; delete the channel instead"));
            }
            let tx = db.transaction()?;
            if let Some(leaving) = &leaving {
                let key = store::signing_key(&tx, &auth.device)?.ok_or(ApiError::internal())?;
                if !store::keep_leave(&tx, &channel, &auth.device, &key, &leaving.leave)? {
                    return Err(BAD_LEAVE);
                }
            }
            tx.execute("UPDATE members SET gone = 1 WHERE channel = ?1 AND device = ?2", [&channel, &auth.device])?;
            store::drop_pending(&tx, &channel, &auth.device)?;
            store::wipe_delivered(&tx)?;
            tx.commit()?;
            store::checkpoint(db)?;
            Ok(store::member_devices(db, &channel)?)
        })
        .await?;
    state.wake_all(&remaining, Wake::Changed);
    Ok(StatusCode::NO_CONTENT)
}
