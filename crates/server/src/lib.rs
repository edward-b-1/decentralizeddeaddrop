//! The Dead Drop server: registers devices, keeps owner-signed channel records, and
//! holds sealed messages until they are delivered or too old. See `docs/PROTOCOL.md`.

mod auth;
mod channels;
pub mod error;
pub mod logging;
mod operator;
mod password;
mod queue;
pub mod state;
pub mod store;

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{delete, get, post, put};

pub use queue::MAX_BODY_BYTES;
pub use state::{AppState, Clock};
pub use store::Store;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/server", get(auth::server_info))
        .route("/v1/devices", post(auth::register))
        .route("/v1/devices/me", get(auth::me).delete(auth::delete_me))
        .route("/v1/sessions/challenge", post(auth::challenge))
        .route("/v1/sessions", post(auth::open_session))
        .route("/v1/operator/settings", get(operator::get_settings).put(operator::put_settings))
        .route("/v1/operator/password", put(operator::put_password))
        .route("/v1/operator/invites", post(operator::new_invite))
        .route("/v1/operator/devices", get(operator::devices))
        .route("/v1/operator/devices/{device}/approve", post(operator::approve))
        .route("/v1/operator/devices/{device}", delete(operator::remove))
        .route("/v1/channels", get(channels::mine).post(channels::create))
        .route("/v1/channels/{channel}", get(channels::show).put(channels::update).delete(channels::destroy))
        .route("/v1/channels/{channel}/invites", post(channels::new_invite))
        .route("/v1/channels/{channel}/join-requests", get(channels::requests).post(channels::ask))
        .route("/v1/channels/{channel}/join-requests/{device}", delete(channels::decline))
        .route("/v1/channels/{channel}/roster", put(channels::put_roster))
        .route("/v1/channels/{channel}/members", get(channels::members))
        .route("/v1/channels/{channel}/members/me", delete(channels::leave))
        .route("/v1/channels/{channel}/messages", post(queue::send))
        .route("/v1/queue", get(queue::fetch))
        .route("/v1/queue/ack", post(queue::ack))
        .route("/v1/events", get(queue::events))
        .layer(DefaultBodyLimit::max(4 * MAX_BODY_BYTES))
        .layer(middleware::from_fn(logging::log))
        .with_state(state)
}

/// Serves the API on `listener` until `shutdown` completes. Each request carries the
/// network address it came from, which only the wrong-password limit ever looks at.
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: AppState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router(state).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown)
        .await
}

/// Wipes messages past their maximum age, now. Returns how many went.
pub async fn sweep(state: &AppState) -> error::ApiResult<usize> {
    let now = state.clock.now();
    state.failures.lock().unwrap().forget_old(now);
    state.store.run(move |db| Ok(queue::sweep(db, now)?)).await
}

/// Runs [`sweep`] every `every`, for as long as the server lives.
pub fn spawn_sweeper(state: AppState, every: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(every);
        loop {
            ticks.tick().await;
            // A failure is already logged (without details) where it happened.
            let _ = sweep(&state).await;
        }
    })
}
