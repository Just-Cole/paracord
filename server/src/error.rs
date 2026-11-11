use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tracing::error;

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized,
    Forbidden(String),
    NotFound(String),
    Conflict(String),
    TooManyRequests,
    Internal(String),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn bad(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }
    pub fn internal(message: impl std::fmt::Display) -> Self {
        Self::Internal(message.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "Sign in required.".to_string()),
            Self::Forbidden(m) => (StatusCode::FORBIDDEN, m),
            Self::NotFound(m) => (StatusCode::NOT_FOUND, m),
            Self::Conflict(m) => (StatusCode::CONFLICT, m),
            Self::TooManyRequests => (
                StatusCode::TOO_MANY_REQUESTS,
                "Slow down a little and try again.".to_string(),
            ),
            Self::Internal(m) => {
                error!("internal error: {m}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Something went wrong on the server.".to_string(),
                )
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> Self {
        Self::internal(error)
    }
}
