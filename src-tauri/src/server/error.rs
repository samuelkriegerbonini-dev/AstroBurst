// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    Conflict(String),
    TooManyRequests,
    BadRequest(String),
    ServiceUnavailable(String),
    Internal(anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match self {
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, "not_found", m),
            AppError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m),
            AppError::TooManyRequests => (
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_requests",
                "job queue full (max 4 concurrent)".into(),
            ),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m),
            AppError::ServiceUnavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, "service_unavailable", m),
            AppError::Internal(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                format!("{:#}", e),
            ),
        };
        (status, Json(json!({ "success": false, "error": { "code": code, "message": msg } })))
            .into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Internal(e)
    }
}