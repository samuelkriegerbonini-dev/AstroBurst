use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::error::AppError;

#[derive(Debug)]
pub(crate) enum ScienceError {
    Coded {
        status: StatusCode,
        code: &'static str,
        message: String,
        hint: Option<String>,
        retry_after: Option<u64>,
    },
    App(AppError),
}

impl ScienceError {
    fn coded(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
        hint: Option<String>,
    ) -> Self {
        Self::Coded {
            status,
            code,
            message: message.into(),
            hint,
            retry_after: None,
        }
    }

    pub(crate) fn bad_request(
        code: &'static str,
        message: impl Into<String>,
        hint: Option<String>,
    ) -> Self {
        Self::coded(StatusCode::BAD_REQUEST, code, message, hint)
    }

    pub(crate) fn not_found(
        code: &'static str,
        message: impl Into<String>,
        hint: Option<String>,
    ) -> Self {
        Self::coded(StatusCode::NOT_FOUND, code, message, hint)
    }

    pub(crate) fn unauthorized(
        code: &'static str,
        message: impl Into<String>,
        hint: Option<String>,
    ) -> Self {
        Self::coded(StatusCode::UNAUTHORIZED, code, message, hint)
    }

    pub(crate) fn forbidden(
        code: &'static str,
        message: impl Into<String>,
        hint: Option<String>,
    ) -> Self {
        Self::coded(StatusCode::FORBIDDEN, code, message, hint)
    }

    pub(crate) fn misdirected(
        code: &'static str,
        message: impl Into<String>,
        hint: Option<String>,
    ) -> Self {
        Self::coded(StatusCode::MISDIRECTED_REQUEST, code, message, hint)
    }

    pub(crate) fn unavailable(
        code: &'static str,
        message: impl Into<String>,
        retry_after: Option<u64>,
    ) -> Self {
        Self::Coded {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code,
            message: message.into(),
            hint: None,
            retry_after,
        }
    }
}

impl IntoResponse for ScienceError {
    fn into_response(self) -> Response {
        let (status, code, message, hint, retry_after) = match self {
            Self::App(e) => return e.into_response(),
            Self::Coded {
                status,
                code,
                message,
                hint,
                retry_after,
            } => (status, code, message, hint, retry_after),
        };
        let mut error = json!({ "code": code, "message": message });
        if let Some(hint) = hint {
            error["hint"] = json!(hint);
        }
        let mut response =
            (status, Json(json!({ "success": false, "error": error }))).into_response();
        if let Some(seconds) = retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        }
        if status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

impl From<AppError> for ScienceError {
    fn from(e: AppError) -> Self {
        Self::App(e)
    }
}

impl From<anyhow::Error> for ScienceError {
    fn from(e: anyhow::Error) -> Self {
        Self::App(AppError::Internal(e))
    }
}
