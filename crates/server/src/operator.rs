//! What the server's operator can do from the app: choose the ways in, hand out
//! invites, approve and remove devices. The operator has no special access to channels.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{self, Auth};
use crate::error::{ApiError, ApiResult};
use crate::password;
use crate::state::{AppState, Wake};
use crate::store;

fn settings_json(settings: &store::Settings) -> Json<Value> {
    Json(json!({
        "open": settings.open,
        "tokens": settings.tokens,
        "approval": settings.approval,
        "password": settings.password_hash.is_some(),
    }))
}

/// `GET /v1/operator/settings`
pub async fn get_settings(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    auth.require_operator()?;
    let settings = state.store.run(|db| Ok(store::load_settings(db)?)).await?;
    Ok(settings_json(&settings))
}

#[derive(Deserialize)]
pub struct WaysIn {
    open: bool,
    tokens: bool,
    approval: bool,
}

/// `PUT /v1/operator/settings`
pub async fn put_settings(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<WaysIn>,
) -> ApiResult<Json<Value>> {
    auth.require_operator()?;
    let settings = state
        .store
        .run(move |db| {
            let mut settings = store::load_settings(db)?;
            settings.open = body.open;
            settings.tokens = body.tokens;
            settings.approval = body.approval;
            store::save_settings(db, &settings)?;
            Ok(settings)
        })
        .await?;
    Ok(settings_json(&settings))
}

#[derive(Deserialize)]
pub struct NewPassword {
    /// `null` removes the password.
    password: Option<String>,
}

/// `PUT /v1/operator/password`
pub async fn put_password(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<NewPassword>,
) -> ApiResult<StatusCode> {
    auth.require_operator()?;
    let hash = match body.password {
        None => None,
        Some(password) if password.is_empty() || password.len() > 256 => {
            return Err(ApiError::bad("password must be 1 to 256 bytes"));
        }
        Some(password) => Some(
            tokio::task::spawn_blocking(move || password::hash(&password))
                .await
                .ok()
                .flatten()
                .ok_or(ApiError::internal())?,
        ),
    };
    state
        .store
        .run(move |db| {
            let mut settings = store::load_settings(db)?;
            settings.password_hash = hash;
            store::save_settings(db, &settings)?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct NewInvite {
    #[serde(default = "one")]
    pub uses: i64,
    /// Seconds until the invite stops working.
    #[serde(default = "one_day")]
    pub ttl: i64,
}

fn one() -> i64 {
    1
}

fn one_day() -> i64 {
    86_400
}

impl NewInvite {
    pub fn check(&self) -> ApiResult<()> {
        if (1..=10_000).contains(&self.uses) && (60..=366 * 86_400).contains(&self.ttl) {
            Ok(())
        } else {
            Err(ApiError::bad("uses must be 1 to 10000 and ttl 60 seconds to 366 days"))
        }
    }
}

/// `POST /v1/operator/invites`: a token that lets devices register.
pub async fn new_invite(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<NewInvite>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    auth.require_operator()?;
    body.check()?;
    let expires = state.clock.now() + body.ttl;
    let token = state.store.run(move |db| Ok(store::create_token(db, "server", body.uses, expires)?)).await?;
    Ok((StatusCode::CREATED, Json(json!({ "token": token, "expires": expires }))))
}

/// `GET /v1/operator/devices`
pub async fn devices(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    auth.require_operator()?;
    let devices = state
        .store
        .run(|db| {
            let mut query = db.prepare("SELECT id, status, operator FROM devices ORDER BY id")?;
            let rows = query
                .query_map([], |row| {
                    Ok(json!({
                        "device_id": row.get::<_, String>(0)?,
                        "status": row.get::<_, String>(1)?,
                        "operator": row.get::<_, bool>(2)?,
                    }))
                })?
                .collect::<rusqlite::Result<Vec<Value>>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(json!({ "devices": devices })))
}

/// `POST /v1/operator/devices/{device}/approve`
pub async fn approve(State(state): State<AppState>, auth: Auth, Path(device): Path<String>) -> ApiResult<StatusCode> {
    auth.require_operator()?;
    let changed = state
        .store
        .run(move |db| Ok(db.execute("UPDATE devices SET status = 'active' WHERE id = ?1", params![device])?))
        .await?;
    if changed == 0 { Err(ApiError::not_found()) } else { Ok(StatusCode::NO_CONTENT) }
}

/// `DELETE /v1/operator/devices/{device}`: remove a device from the server.
pub async fn remove(State(state): State<AppState>, auth: Auth, Path(device): Path<String>) -> ApiResult<StatusCode> {
    auth.require_operator()?;
    if device == auth.device {
        return Err(ApiError::conflict("to remove yourself, delete your own device"));
    }
    let now = state.clock.now();
    let affected = state.store.run(move |db| auth::delete_device(db, &device, &[], &[], now)).await?;
    state.wake_all(&affected, Wake::Changed);
    Ok(StatusCode::NO_CONTENT)
}
