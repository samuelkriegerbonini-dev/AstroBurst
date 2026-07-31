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
    BadRequestWithHint {
        code: &'static str,
        message: String,
        hint: Option<String>,
    },
    ServiceUnavailable(String),
    Internal(anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let mut hint: Option<String> = None;
        let (status, code, msg) = match self {
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, "not_found", m),
            AppError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m),
            AppError::TooManyRequests => (
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_requests",
                "job queue full (max 4 concurrent)".into(),
            ),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m),
            AppError::BadRequestWithHint { code, message, hint: h } => {
                hint = h;
                (StatusCode::BAD_REQUEST, code, message)
            }
            AppError::ServiceUnavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, "service_unavailable", m),
            AppError::Internal(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                format!("{:#}", e),
            ),
        };
        let mut err = json!({ "code": code, "message": msg });
        if let Some(h) = hint {
            err["hint"] = json!(h);
        }
        (status, Json(json!({ "success": false, "error": err }))).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Internal(e)
    }
}
