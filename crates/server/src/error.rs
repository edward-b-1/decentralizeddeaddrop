//! The one error type handlers return. Messages are fixed text: nothing a caller sent
//! is ever echoed into a response or a log line.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: &'static str,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub const fn new(status: StatusCode, message: &'static str) -> ApiError {
        ApiError { status, message }
    }
    pub const fn bad(message: &'static str) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, message)
    }
    pub const fn unauthorized() -> ApiError {
        ApiError::new(StatusCode::UNAUTHORIZED, "sign in first")
    }
    pub const fn forbidden(message: &'static str) -> ApiError {
        ApiError::new(StatusCode::FORBIDDEN, message)
    }
    pub const fn not_found() -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, "not found")
    }
    pub const fn conflict(message: &'static str) -> ApiError {
        ApiError::new(StatusCode::CONFLICT, message)
    }
    pub const fn internal() -> ApiError {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> ApiError {
        // SQLite's messages can quote stored values, so release builds say nothing more.
        if cfg!(debug_assertions) {
            eprintln!("database error: {error}");
        } else {
            eprintln!("database error");
        }
        ApiError::internal()
    }
}
