use std::error::Error as StdError;

use super::types::{MastCmdError, MastErrorCode, MastErrorContext, MastMissionId};
use crate::infra::mast::MastError;

pub const DETAIL_LIMIT: usize = 300;
pub const NETWORK_MESSAGE_LIMIT: usize = 200;
pub const RETRY_AFTER_CAP_SECS: u64 = 120;
pub const RETRY_LADDER_SECS: [u64; 3] = [10, 30, 60];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Search,
    Probe,
    Lookup,
    ImageList,
    Thumbnail,
    Preview,
    Products,
    AuthCheck,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    pub mission: Option<MastMissionId>,
    pub kind: OpKind,
    pub path: String,
}

impl Operation {
    pub fn new(mission: Option<MastMissionId>, kind: OpKind, path: impl Into<String>) -> Self {
        let path = path.into();
        let path = path.split('?').next().unwrap_or_default().trim_start_matches('/').to_owned();
        Self { mission, kind, path }
    }

    pub fn mission(mission: MastMissionId, kind: OpKind, operation: &str) -> Self {
        Self::new(Some(mission), kind, mission.operation_path(operation))
    }

    pub fn is_image(&self) -> bool {
        matches!(self.kind, OpKind::ImageList | OpKind::Thumbnail | OpKind::Preview)
    }

    pub fn needs_token(&self) -> bool {
        matches!(self.mission, Some(MastMissionId::Roman | MastMissionId::RomanCgi))
    }
}

pub fn retry_after_secs(header: Option<u64>, consecutive: u32) -> u64 {
    match header {
        Some(secs) => secs.min(RETRY_AFTER_CAP_SECS),
        None => {
            let step = consecutive.saturating_sub(1) as usize;
            RETRY_LADDER_SECS[step.min(RETRY_LADDER_SECS.len() - 1)]
        }
    }
}

fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn innermost_cause(error: &(dyn StdError + 'static)) -> String {
    let mut current = error;
    while let Some(next) = current.source() {
        current = next;
    }
    let text = current.to_string();
    let cleaned = text
        .split_whitespace()
        .filter(|word| !word.contains("://") && !word.contains('?'))
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = truncate(cleaned.trim(), NETWORK_MESSAGE_LIMIT);
    if cleaned.is_empty() {
        "connection failed".to_owned()
    } else {
        cleaned
    }
}

fn json_detail(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    match value.get("detail")? {
        serde_json::Value::String(text) => Some(text.trim().to_owned()).filter(|t| !t.is_empty()),
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .filter_map(|item| item.get("msg").and_then(|m| m.as_str()).map(str::to_owned))
                .collect();
            (!parts.is_empty()).then(|| parts.join("; "))
        }
        _ => None,
    }
}

fn is_nginx_404(body: &str, content_type: Option<&str>) -> bool {
    let html = content_type.is_some_and(|ct| ct.to_ascii_lowercase().contains("text/html"))
        || body.trim_start().starts_with('<');
    html && body.to_ascii_lowercase().contains("nginx")
}

fn is_fastapi_route_404(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .is_some_and(|value| value == serde_json::json!({"detail": "Not Found"}))
}

fn base_context(op: &Operation) -> MastErrorContext {
    MastErrorContext {
        operation: Some(op.path.clone()),
        ..Default::default()
    }
}

struct StatusInput<'a> {
    status: u16,
    body: &'a str,
    detail: Option<&'a str>,
    content_type: Option<&'a str>,
    retry_after: Option<u64>,
}

fn status_error(op: &Operation, consecutive: u32, input: StatusInput<'_>) -> MastCmdError {
    let StatusInput { status, body, detail, content_type, retry_after } = input;
    let auth_host = op.kind == OpKind::Info;
    let detail = if auth_host {
        None
    } else {
        detail.map(str::to_owned).or_else(|| json_detail(body)).map(|d| truncate(&d, DETAIL_LIMIT))
    };
    let mut context = base_context(op);
    context.status = Some(status);
    if auth_host {
        return if (400..500).contains(&status) {
            token_rejected()
        } else if status >= 500 {
            MastCmdError::new(MastErrorCode::Server, format!("HTTP {status}")).with_context(context)
        } else {
            MastCmdError::new(MastErrorCode::Decode, format!("unexpected HTTP {status}")).with_context(context)
        };
    }
    if status == 429 || (status == 503 && retry_after.is_some()) {
        context.retry_after_sec = Some(retry_after_secs(retry_after, consecutive));
        context.consecutive = Some(consecutive);
        return MastCmdError::new(MastErrorCode::RateLimited, format!("rate limited (HTTP {status})")).with_context(context);
    }
    if status >= 500 {
        return MastCmdError::new(MastErrorCode::Server, format!("HTTP {status}")).with_context(context);
    }
    if status == 400 || status == 422 {
        let message = detail.clone().unwrap_or_else(|| format!("HTTP {status}"));
        context.detail = detail;
        return MastCmdError::new(MastErrorCode::Rejected, message).with_context(context);
    }
    if status == 404 && (is_nginx_404(body, content_type) || is_fastapi_route_404(body)) {
        context.mission = op.mission;
        return MastCmdError::new(MastErrorCode::NotDeployed, "not deployed (HTTP 404)").with_context(context);
    }
    if status == 401 && op.needs_token() {
        context.mission = op.mission;
        return MastCmdError::new(MastErrorCode::Unauthorized, "authorization required").with_context(context);
    }
    if op.is_image() && (400..500).contains(&status) {
        let message = detail.clone().unwrap_or_else(|| format!("no image (HTTP {status})"));
        context.detail = detail;
        return MastCmdError::new(MastErrorCode::NoImage, message).with_context(context);
    }
    if status == 404 {
        return MastCmdError::new(MastErrorCode::Server, "HTTP 404").with_context(context);
    }
    if status == 401 {
        context.mission = op.mission;
        return MastCmdError::new(MastErrorCode::Unauthorized, "authorization required").with_context(context);
    }
    let message = detail.clone().unwrap_or_else(|| format!("HTTP {status}"));
    context.detail = detail;
    MastCmdError::new(MastErrorCode::Rejected, message).with_context(context)
}

pub fn classify(error: &MastError, op: &Operation, consecutive: u32) -> MastCmdError {
    match error {
        MastError::Client(source) => {
            MastCmdError::new(MastErrorCode::Network, innermost_cause(source)).with_context(base_context(op))
        }
        MastError::Transport { source, .. } => {
            MastCmdError::new(MastErrorCode::Network, innermost_cause(source)).with_context(base_context(op))
        }
        MastError::RedirectRefused { .. } => {
            MastCmdError::new(MastErrorCode::Network, "redirect refused").with_context(base_context(op))
        }
        MastError::Timeout { timeout_secs, .. } => {
            let mut context = base_context(op);
            context.timeout_sec = Some(*timeout_secs);
            MastCmdError::new(MastErrorCode::Timeout, format!("no answer within {timeout_secs} s")).with_context(context)
        }
        MastError::TooLarge { bytes, limit, .. } => {
            let mut context = base_context(op);
            context.bytes = Some(*bytes);
            MastCmdError::new(MastErrorCode::TooLarge, format!("body exceeds {limit} bytes")).with_context(context)
        }
        MastError::NotImage { .. } => {
            MastCmdError::new(MastErrorCode::NotImage, "response is not an image").with_context(base_context(op))
        }
        MastError::Status { status, body, detail, content_type, retry_after_secs, .. } => status_error(
            op,
            consecutive,
            StatusInput {
                status: *status,
                body,
                detail: detail.as_deref(),
                content_type: content_type.as_deref(),
                retry_after: *retry_after_secs,
            },
        ),
        MastError::Validation { summary, .. } => {
            if op.kind == OpKind::Info {
                return token_rejected();
            }
            let mut context = base_context(op);
            context.status = Some(422);
            let detail = truncate(summary, DETAIL_LIMIT);
            context.detail = Some(detail.clone());
            MastCmdError::new(MastErrorCode::Rejected, detail).with_context(context)
        }
        MastError::Decode { .. } | MastError::Encode(_) => unexpected_format(op),
        MastError::Io(source) => MastCmdError::new(MastErrorCode::Cache, truncate(&source.to_string(), NETWORK_MESSAGE_LIMIT)),
    }
}

pub fn token_rejected() -> MastCmdError {
    MastCmdError::new(MastErrorCode::TokenRejected, "token not accepted")
}

pub fn unexpected_format(op: &Operation) -> MastCmdError {
    MastCmdError::new(MastErrorCode::Decode, "unexpected response format").with_context(base_context(op))
}

pub fn cancelled() -> MastCmdError {
    MastCmdError::new(MastErrorCode::Cancelled, "cancelled")
}

pub fn bad_input(field: &str, reason: &str) -> MastCmdError {
    MastCmdError::new(MastErrorCode::BadInput, format!("{field}: {reason}"))
}

pub fn unauthorized_without_token(mission: MastMissionId) -> MastCmdError {
    MastCmdError::new(MastErrorCode::Unauthorized, "a MAST token is required").with_context(MastErrorContext {
        mission: Some(mission),
        ..Default::default()
    })
}

pub fn internal() -> MastCmdError {
    MastCmdError::new(MastErrorCode::Decode, "internal error")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::mast::models::{HttpValidationError, ValidationError};
    use serde_json::json;

    fn status(code: u16, body: &str, content_type: &str, retry_after: Option<u64>) -> MastError {
        MastError::Status {
            status: code,
            url: "https://mast.stsci.edu/search/jwst/api/v0.1/search?x=1".into(),
            body: body.into(),
            detail: None,
            content_type: Some(content_type.into()),
            retry_after_secs: retry_after,
        }
    }

    fn op(mission: MastMissionId, kind: OpKind, operation: &str) -> Operation {
        Operation::mission(mission, kind, operation)
    }

    fn assert_safe_message(error: &MastCmdError) {
        assert!(!error.message.contains("://"), "{}", error.message);
        assert!(!error.message.contains('?'), "{}", error.message);
        assert!(!error.message.is_empty());
    }

    const NGINX_404: &str = "<html>\r\n<head><title>404 Not Found</title></head>\r\n<body>\r\n<center><h1>404 Not Found</h1></center>\r\n<hr><center>nginx/1.30.5</center>\r\n</body>\r\n</html>\r\n";

    #[test]
    fn error_classification_table() {
        let search = op(MastMissionId::Jwst, OpKind::Search, "search");
        let thumb = op(MastMissionId::Jwst, OpKind::Thumbnail, "get_image_thumbnail");
        let preview = op(MastMissionId::Hst, OpKind::Preview, "get_image_preview?filename=a.jpg");

        let builder_error = reqwest::Client::new().get("not a url").build().unwrap_err();
        let network = classify(
            &MastError::Transport { url: "https://mast.stsci.edu/x?y=1".into(), source: builder_error },
            &search,
            0,
        );
        assert_eq!(network.code, MastErrorCode::Network);
        assert_eq!(network.context.operation.as_deref(), Some("search/jwst/api/v0.1/search"));
        assert!(network.message.chars().count() <= NETWORK_MESSAGE_LIMIT);
        assert_safe_message(&network);

        let redirect = classify(&MastError::RedirectRefused { url: "https://evil.example/x".into() }, &thumb, 0);
        assert_eq!(redirect.code, MastErrorCode::Network);
        assert_eq!(redirect.message, "redirect refused");

        let timeout = classify(&MastError::Timeout { url: "u".into(), timeout_secs: 30 }, &thumb, 0);
        assert_eq!(timeout.code, MastErrorCode::Timeout);
        assert_eq!(timeout.context.timeout_sec, Some(30));
        assert_eq!(timeout.context.operation.as_deref(), Some("search/jwst/api/v0.1/get_image_thumbnail"));

        assert_eq!(cancelled().code, MastErrorCode::Cancelled);
        assert_eq!(cancelled().context, MastErrorContext::default());

        let limited = classify(&status(429, "", "text/plain", Some(500)), &thumb, 1);
        assert_eq!(limited.code, MastErrorCode::RateLimited);
        assert_eq!(limited.context.retry_after_sec, Some(RETRY_AFTER_CAP_SECS));
        assert_eq!(limited.context.consecutive, Some(1));
        assert_eq!(limited.context.status, Some(429));
        assert!(limited.context.operation.is_some());
        for (consecutive, expected) in [(1, 10), (2, 30), (3, 60), (7, 60)] {
            let ladder = classify(&status(429, "", "text/plain", None), &thumb, consecutive);
            assert_eq!(ladder.context.retry_after_sec, Some(expected), "consecutive {consecutive}");
        }
        let unavailable = classify(&status(503, "", "text/plain", Some(7)), &thumb, 1);
        assert_eq!(unavailable.code, MastErrorCode::RateLimited);
        assert_eq!(unavailable.context.retry_after_sec, Some(7));

        let server = classify(&status(503, "", "text/plain", None), &thumb, 1);
        assert_eq!(server.code, MastErrorCode::Server);
        assert_eq!(server.context.status, Some(503));
        let gateway = classify(&status(504, "<html>504 Gateway Time-out</html>", "text/html", None), &search, 0);
        assert_eq!(gateway.code, MastErrorCode::Server);
        assert_eq!(gateway.context.status, Some(504));
        assert_eq!(gateway.context.detail, None);

        let rejected = classify(
            &status(400, r#"{"detail":"Radius greater than 30 arcminutes is not supported"}"#, "application/json", None),
            &search,
            0,
        );
        assert_eq!(rejected.code, MastErrorCode::Rejected);
        assert_eq!(rejected.context.status, Some(400));
        assert_eq!(rejected.context.detail.as_deref(), Some("Radius greater than 30 arcminutes is not supported"));
        let long = format!(r#"{{"detail":"{}"}}"#, "x".repeat(900));
        let truncated = classify(&status(400, &long, "application/json", None), &search, 0);
        assert_eq!(truncated.context.detail.as_ref().map(|d| d.chars().count()), Some(DETAIL_LIMIT));
        let validation = classify(
            &MastError::Validation {
                url: "u".into(),
                summary: "body.radius: Input should be a valid number".into(),
                detail: HttpValidationError {
                    detail: vec![ValidationError { msg: "Input should be a valid number".into(), ..Default::default() }],
                },
            },
            &search,
            0,
        );
        assert_eq!(validation.code, MastErrorCode::Rejected);
        assert_eq!(validation.context.status, Some(422));
        assert_eq!(validation.context.detail.as_deref(), Some("body.radius: Input should be a valid number"));

        for mission in [MastMissionId::Roman, MastMissionId::RomanCgi] {
            let roman = classify(
                &status(401, r#"{"detail":"Only authorized users can make search API calls"}"#, "application/json", None),
                &op(mission, OpKind::Search, "search"),
                0,
            );
            assert_eq!(roman.code, MastErrorCode::Unauthorized);
            assert_eq!(roman.context.mission, Some(mission));
            assert_eq!(roman.context.status, Some(401));
        }

        let anonymous = classify(&status(401, r#"{"detail":"UNAUTHORIZED USER: anonymous"}"#, "application/json", None), &thumb, 0);
        assert_eq!(anonymous.code, MastErrorCode::NoImage);
        assert_eq!(anonymous.context.status, Some(401));
        assert_eq!(anonymous.context.detail.as_deref(), Some("UNAUTHORIZED USER: anonymous"));
        let missing = classify(&status(404, r#"{"detail":"Image not found"}"#, "application/json", None), &preview, 0);
        assert_eq!(missing.code, MastErrorCode::NoImage);
        assert_eq!(missing.context.operation.as_deref(), Some("search/hst/api/v0.1/get_image_preview"));

        let nginx = classify(&status(404, NGINX_404, "text/html", None), &op(MastMissionId::Tess, OpKind::Search, "search"), 0);
        assert_eq!(nginx.code, MastErrorCode::NotDeployed);
        assert_eq!(nginx.context.mission, Some(MastMissionId::Tess));
        assert_eq!(nginx.context.status, Some(404));
        assert_eq!(nginx.context.operation.as_deref(), Some("search/tess/api/v0.1/search"));
        let fastapi = classify(
            &status(404, r#"{"detail":"Not Found"}"#, "application/json", None),
            &op(MastMissionId::Classy, OpKind::ImageList, "get_image_list"),
            0,
        );
        assert_eq!(fastapi.code, MastErrorCode::NotDeployed);

        let other_404 = classify(&status(404, r#"{"detail":"Dataset gone"}"#, "application/json", None), &search, 0);
        assert_eq!(other_404.code, MastErrorCode::Server);
        assert_eq!(other_404.context.status, Some(404));

        let decode_source = serde_json::from_str::<serde_json::Value>("{oops").unwrap_err();
        let decode = classify(&MastError::Decode { url: "u".into(), source: decode_source }, &search, 0);
        assert_eq!(decode.code, MastErrorCode::Decode);
        assert!(decode.context.operation.is_some());

        let too_large = classify(&MastError::TooLarge { url: "u".into(), bytes: 300_000, limit: 262_144 }, &thumb, 0);
        assert_eq!(too_large.code, MastErrorCode::TooLarge);
        assert_eq!(too_large.context.bytes, Some(300_000));
        assert!(too_large.context.operation.is_some());

        let not_image = classify(&MastError::NotImage { url: "u".into() }, &thumb, 0);
        assert_eq!(not_image.code, MastErrorCode::NotImage);
        assert!(not_image.context.operation.is_some());

        let bad = bad_input("name", "must not be empty");
        assert_eq!(bad.code, MastErrorCode::BadInput);
        assert_eq!(bad.message, "name: must not be empty");

        let info = Operation::new(None, OpKind::Info, "info");
        for code in [400, 401, 403] {
            let rejected = classify(&status(code, r#"{"detail":"user alice"}"#, "application/json", None), &info, 0);
            assert_eq!(rejected.code, MastErrorCode::TokenRejected, "{code}");
            assert_eq!(rejected.context, MastErrorContext::default());
        }
        let info_down = classify(&status(500, r#"{"detail":"user alice"}"#, "application/json", None), &info, 0);
        assert_eq!(info_down.code, MastErrorCode::Server);
        assert_eq!(info_down.context.detail, None);

        let io = classify(&MastError::Io(std::io::Error::other("disk full")), &thumb, 0);
        assert_eq!(io.code, MastErrorCode::Cache);
        assert!(io.message.contains("disk full"));

        for error in [&network, &redirect, &timeout, &limited, &server, &rejected, &anonymous, &nginx, &decode, &too_large, &not_image, &io] {
            assert_safe_message(error);
        }
    }

    #[test]
    fn status_detail_field_is_preferred_and_local_errors_are_shaped() {
        let error = MastError::Status {
            status: 400,
            url: "u".into(),
            body: "{\"detail\":\"cut".into(),
            detail: Some("w".repeat(900)),
            content_type: Some("application/json".into()),
            retry_after_secs: None,
        };
        let rejected = classify(&error, &op(MastMissionId::Jwst, OpKind::Search, "search"), 0);
        assert_eq!(rejected.code, MastErrorCode::Rejected);
        assert_eq!(rejected.context.detail.as_ref().map(String::len), Some(DETAIL_LIMIT));
        assert_eq!(rejected.message.chars().count(), DETAIL_LIMIT);
        let info = classify(&error, &Operation::new(None, OpKind::Info, "info"), 0);
        assert_eq!(info.code, MastErrorCode::TokenRejected);
        assert_eq!(info.context, MastErrorContext::default());
        let local = unauthorized_without_token(MastMissionId::Roman);
        assert_eq!(local.code, MastErrorCode::Unauthorized);
        assert_eq!(local.context, MastErrorContext { mission: Some(MastMissionId::Roman), ..Default::default() });
        assert_eq!(local.message, "a MAST token is required");
        let internal = internal();
        assert_eq!(internal.code, MastErrorCode::Decode);
        assert_eq!(internal.message, "internal error");
    }

    #[test]
    fn retry_after_caps_header_and_falls_back_to_ladder() {
        assert_eq!(retry_after_secs(Some(5), 1), 5);
        assert_eq!(retry_after_secs(Some(121), 1), RETRY_AFTER_CAP_SECS);
        assert_eq!(retry_after_secs(None, 0), 10);
        assert_eq!(retry_after_secs(None, 1), 10);
        assert_eq!(retry_after_secs(None, 2), 30);
        assert_eq!(retry_after_secs(None, 3), 60);
        let value = serde_json::to_value(bad_input("x", "y")).unwrap();
        assert_eq!(value, json!({"code": "mast_bad_input", "message": "x: y", "context": {}}));
    }
}
