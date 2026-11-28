//! Getting onto the server (registration, the ways in) and proving who you are
//! afterwards (challenge, signature, session token).

use axum::Json;
use axum::body::Bytes;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use deaddrop_core::encoding::{b64, random_bytes, random_token, unb64};
use deaddrop_core::keys;
use deaddrop_core::records::{Leave, Signed};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{ApiError, ApiResult};
use crate::logging::{PasswordEvent, password_line};
use crate::password;
use crate::state::{self, AppState, Caller, Wake};
use crate::store::{self, sha256};

const CHALLENGE_SECONDS: i64 = 120;
const SESSION_SECONDS: i64 = 3600;

const NOT_ACCEPTED: ApiError = ApiError::forbidden("not accepted");
const SIGN_IN_FAILED: ApiError = ApiError::new(StatusCode::UNAUTHORIZED, "sign-in failed");

/// The device a request comes from. Taking this as a handler argument is what makes a
/// route require a signed-in, approved device.
pub struct Auth {
    pub device: String,
    pub operator: bool,
}

impl Auth {
    pub fn require_operator(&self) -> ApiResult<()> {
        if self.operator { Ok(()) } else { Err(ApiError::forbidden("operator only")) }
    }
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Auth> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(ApiError::unauthorized())?;
        authenticate(state, token).await
    }
}

pub async fn authenticate(state: &AppState, token: &str) -> ApiResult<Auth> {
    let hash = sha256(token);
    let now = state.clock.now();
    state
        .store
        .run(move |db| {
            let found: Option<(String, String, bool)> = db
                .query_row(
                    "SELECT d.id, d.status, d.operator FROM sessions s JOIN devices d ON d.id = s.device
                     WHERE s.token_hash = ?1 AND s.expires > ?2",
                    params![hash, now],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            match found {
                None => Err(ApiError::unauthorized()),
                Some((_, status, _)) if status != "active" => Err(ApiError::forbidden("awaiting approval")),
                Some((device, _, operator)) => Ok(Auth { device, operator }),
            }
        })
        .await
}

/// The network address a request came from. Only registration asks for it, and only
/// uses it when a server password is wrong.
pub struct Peer(pub Option<IpAddr>);

impl<S: Sync> FromRequestParts<S> for Peer {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Peer, Infallible> {
        let address = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|info| info.0.ip());
        Ok(Peer(address))
    }
}

/// `GET /v1/server`: what a phone needs to know before registering.
pub async fn server_info(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let settings = state.store.run(|db| Ok(store::load_settings(db)?)).await?;
    Ok(Json(json!({
        "protocol": 1,
        "server_id": &*state.server_id,
        "ways_in": {
            "open": settings.open,
            "token": settings.tokens,
            "password": settings.password_hash.is_some(),
            "approval": settings.approval,
        },
    })))
}

#[derive(Deserialize)]
pub struct Register {
    signing_key: String,
    agreement_key: String,
    agreement_sig: String,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

/// `POST /v1/devices`: register a device through one of the ways in.
pub async fn register(
    State(state): State<AppState>,
    Peer(address): Peer,
    Json(body): Json<Register>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    const BAD_KEYS: ApiError = ApiError::bad("keys must be P-256, with the agreement key signed by the signing key");
    let signing_key = unb64(&body.signing_key).ok_or(BAD_KEYS)?;
    let agreement_key = unb64(&body.agreement_key).ok_or(BAD_KEYS)?;
    let agreement_sig = unb64(&body.agreement_sig).ok_or(BAD_KEYS)?;
    if keys::parse_public_key(&signing_key).is_none()
        || keys::parse_public_key(&agreement_key).is_none()
        || !keys::verify(&signing_key, &keys::agreement_key_message(&agreement_key), &agreement_sig)
    {
        return Err(BAD_KEYS);
    }
    let device = keys::device_id(&signing_key);
    let now = state.clock.now();

    // The password check is slow on purpose, so it happens before the database is held.
    let mut password_ok = false;
    if body.token.is_none()
        && let Some(candidate) = body.password.clone()
    {
        let caller = Caller::of(address);
        if state.failures.lock().unwrap().blocked(caller, now) {
            state.failure_log.write(password_line(now, PasswordEvent::Refused, address));
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too many wrong passwords; try again in ten minutes",
            ));
        }
        let settings = state.store.run(|db| Ok(store::load_settings(db)?)).await?;
        if let Some(stored) = settings.password_hash {
            password_ok =
                tokio::task::spawn_blocking(move || password::verify(&candidate, &stored)).await.unwrap_or(false);
        }
        if !password_ok {
            state.failures.lock().unwrap().record(caller, now);
            state.failure_log.write(password_line(now, PasswordEvent::Failure, address));
            return Err(NOT_ACCEPTED);
        }
    }

    let id = device.clone();
    let (status, created, operator) = state
        .store
        .run(move |db| {
            let tx = db.transaction()?;
            let existing: Option<String> =
                tx.query_row("SELECT status FROM devices WHERE id = ?1", [&id], |row| row.get(0)).optional()?;
            if let Some(status) = existing {
                return Ok((status, false, false));
            }
            let settings = store::load_settings(&tx)?;
            let mut operator = false;
            if let Some(token) = &body.token {
                if state::is_operator_token(&tx, token)? {
                    store::delete_meta(&tx, "operator_token")?;
                    operator = true;
                } else if !(settings.tokens && store::consume_token(&tx, "server", token, now)?) {
                    return Err(NOT_ACCEPTED);
                }
            } else if !(password_ok || settings.open) {
                return Err(NOT_ACCEPTED);
            }
            let status = if settings.approval && !operator { "pending" } else { "active" };
            tx.execute(
                "INSERT INTO devices (id, signing_key, agreement_key, agreement_sig, status, operator)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, signing_key, agreement_key, agreement_sig, status, operator],
            )?;
            tx.commit()?;
            Ok((status.to_owned(), true, operator))
        })
        .await?;

    if created && status == "pending" {
        let operators = state
            .store
            .run(|db| {
                let mut query = db.prepare("SELECT id FROM devices WHERE operator = 1")?;
                let ids = query.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
                Ok(ids)
            })
            .await?;
        state.wake_all(&operators, Wake::Changed);
    }
    let code = if created { StatusCode::CREATED } else { StatusCode::OK };
    Ok((code, Json(json!({ "device_id": device, "status": status, "operator": operator }))))
}

#[derive(Deserialize)]
pub struct ChallengeRequest {
    device_id: String,
}

/// `POST /v1/sessions/challenge`: step one of signing in. Unknown devices get a
/// challenge too (one that can never succeed), so this does not reveal who is registered.
pub async fn challenge(State(state): State<AppState>, Json(body): Json<ChallengeRequest>) -> ApiResult<Json<Value>> {
    let challenge = b64(&random_bytes::<32>());
    let expires = state.clock.now() + CHALLENGE_SECONDS;
    let stored = challenge.clone();
    state
        .store
        .run(move |db| {
            db.execute(
                "INSERT INTO challenges (challenge, device, expires)
                 SELECT ?1, id, ?3 FROM devices WHERE id = ?2",
                params![stored, body.device_id, expires],
            )?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({ "challenge": challenge, "server_id": &*state.server_id })))
}

#[derive(Deserialize)]
pub struct SessionRequest {
    device_id: String,
    challenge: String,
    signature: String,
}

/// `POST /v1/sessions`: step two. The device signs the challenge and gets a session token.
pub async fn open_session(State(state): State<AppState>, Json(body): Json<SessionRequest>) -> ApiResult<Json<Value>> {
    let signature = unb64(&body.signature).ok_or(SIGN_IN_FAILED)?;
    let message = keys::session_message(&state.server_id, &body.challenge);
    let now = state.clock.now();
    let token = random_token();
    let token_hash = sha256(&token);
    state
        .store
        .run(move |db| {
            let tx = db.transaction()?;
            // A challenge is spent whether or not the signature turns out to be good.
            let found: Option<(String, i64)> = tx
                .query_row(
                    "DELETE FROM challenges WHERE challenge = ?1 RETURNING device, expires",
                    [&body.challenge],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            tx.commit()?;
            let Some((device, expires)) = found else { return Err(SIGN_IN_FAILED) };
            if device != body.device_id || expires <= now {
                return Err(SIGN_IN_FAILED);
            }
            let key = store::signing_key(db, &device)?.ok_or(SIGN_IN_FAILED)?;
            if !keys::verify(&key, &message, &signature) {
                return Err(SIGN_IN_FAILED);
            }
            db.execute(
                "INSERT INTO sessions (token_hash, device, expires) VALUES (?1, ?2, ?3)",
                params![token_hash, device, now + SESSION_SECONDS],
            )?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({ "token": token, "expires_in": SESSION_SECONDS })))
}

/// `GET /v1/devices/me`
pub async fn me(auth: Auth) -> Json<Value> {
    Json(json!({ "device_id": auth.device, "operator": auth.operator }))
}

#[derive(Deserialize, Default)]
pub struct Deleting {
    /// A signed `records::Leave` for each channel the device is in and does not own.
    #[serde(default)]
    leaves: Vec<Signed>,
    /// A signed `records::Closed` for each channel the device owns.
    #[serde(default)]
    closed: Vec<Signed>,
}

/// `DELETE /v1/devices/me`: delete this device and everything tied to it. Channels it
/// owns go with it, since nobody else could ever sign for them.
///
/// The body, if there is one, is `{"leaves": [...], "closed": [...]}`: the device's
/// signed word that it is leaving each channel it is in, and that each channel it owns
/// is deleted. The first are kept for those channels' owners and the second for its own
/// channels' members, whose phones would otherwise have only this server's word.
pub async fn delete_me(State(state): State<AppState>, auth: Auth, body: Bytes) -> ApiResult<StatusCode> {
    let deleting: Deleting = if body.is_empty() {
        Deleting::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| ApiError::bad("leaves are not valid"))?
    };
    let now = state.clock.now();
    let affected =
        state.store.run(move |db| delete_device(db, &auth.device, &deleting.leaves, &deleting.closed, now)).await?;
    state.wake_all(&affected, Wake::Changed);
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a device; returns the devices that shared a channel with it. `leaves` and
/// `closed` are what the device signed on its way out, if it is the one asking: each
/// that checks out, for a channel it is in or owns, is kept.
pub fn delete_device(
    db: &mut rusqlite::Connection,
    device: &str,
    leaves: &[Signed],
    closed: &[Signed],
    now: i64,
) -> ApiResult<Vec<String>> {
    let tx = db.transaction()?;
    if let Some(key) = store::signing_key(&tx, device)? {
        // The channels it owns are about to go with it.
        let owned: Vec<String> = {
            let mut query = tx.prepare("SELECT id FROM channels WHERE owner = ?1")?;
            query.query_map([device], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        for channel in &owned {
            let signed = closed.iter().find(|signed| store::is_closure(channel, &key, signed));
            store::close_channel(&tx, channel, &key, signed, now)?;
        }
        for signed in leaves {
            // Which channel it is for is read from the record; the signature is checked
            // before anything is kept.
            let Ok(claimed) = serde_json::from_str::<Leave>(&signed.record) else { continue };
            let member: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM members WHERE channel = ?1 AND device = ?2)",
                [claimed.channel.as_str(), device],
                |row| row.get(0),
            )?;
            if member {
                store::keep_leave(&tx, &claimed.channel, device, &key, signed)?;
            }
        }
    }
    let affected = {
        let mut query = tx.prepare(
            "SELECT DISTINCT other.device FROM members mine
             JOIN members other ON other.channel = mine.channel
             WHERE mine.device = ?1 AND other.device != ?1",
        )?;
        query.query_map([device], |row| row.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?
    };
    // Everything else follows by ON DELETE CASCADE: sessions, owned channels (and their
    // queued messages), memberships, join requests and pending marks.
    let deleted = tx.execute("DELETE FROM devices WHERE id = ?1", [device])?;
    store::wipe_delivered(&tx)?;
    tx.commit()?;
    store::checkpoint(db)?;
    if deleted == 0 { Err(ApiError::not_found()) } else { Ok(affected) }
}
