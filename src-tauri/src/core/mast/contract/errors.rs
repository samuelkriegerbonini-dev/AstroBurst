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
    Download,
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

fn os_error_phrase(kind: std::io::ErrorKind) -> Option<&'static str> {
    use std::io::ErrorKind as Kind;
    let phrase = match kind {
        Kind::ConnectionRefused => "connection refused",
        Kind::ConnectionReset => "connection reset",
        Kind::ConnectionAborted => "connection aborted",
        Kind::NotConnected => "not connected",
        Kind::TimedOut => "timed out",
        Kind::HostUnreachable => "host unreachable",
        Kind::NetworkUnreachable => "network unreachable",
        Kind::NetworkDown => "network down",
        Kind::AddrNotAvailable => "address not available",
        Kind::AddrInUse => "address in use",
        Kind::BrokenPipe => "broken pipe",
        Kind::UnexpectedEof => "connection closed early",
        Kind::Interrupted => "interrupted",
        Kind::NotFound => "not found",
        Kind::PermissionDenied => "permission denied",
        Kind::AlreadyExists => "already exists",
        Kind::StorageFull => "disk full",
        Kind::ReadOnlyFilesystem => "read-only file system",
        Kind::ResourceBusy => "resource busy",
        _ => return None,
    };
    Some(phrase)
}

const LOOKUP_FAILED: &str = "host name lookup failed";
const STD_LOOKUP_FAILURE_PREFIX: &str = "failed to lookup address information";

#[cfg(windows)]
fn os_code_phrase(code: i32) -> Option<&'static str> {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    const WSAHOST_NOT_FOUND: i32 = 11001;
    const WSATRY_AGAIN: i32 = 11002;
    const WSANO_RECOVERY: i32 = 11003;
    const WSANO_DATA: i32 = 11004;
    match code {
        ERROR_SHARING_VIOLATION => Some("file in use by another program"),
        ERROR_LOCK_VIOLATION => Some("file locked by another program"),
        WSAHOST_NOT_FOUND => Some("host not found"),
        WSATRY_AGAIN | WSANO_RECOVERY => Some(LOOKUP_FAILED),
        WSANO_DATA => Some("host has no address"),
        _ => None,
    }
}

#[cfg(not(windows))]
fn os_code_phrase(_code: i32) -> Option<&'static str> {
    None
}

fn kind_phrase(kind: std::io::ErrorKind) -> Option<String> {
    if let Some(phrase) = os_error_phrase(kind) {
        return Some(phrase.to_owned());
    }
    let name = format!("{kind:?}");
    if matches!(name.as_str(), "Uncategorized" | "Other") {
        return None;
    }
    let mut phrase = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        if c.is_ascii_uppercase() && !phrase.is_empty() {
            phrase.push(' ');
        }
        phrase.push(c.to_ascii_lowercase());
    }
    Some(phrase)
}

fn os_cause(error: &std::io::Error) -> Option<String> {
    let code = error.raw_os_error()?;
    os_code_phrase(code).map(str::to_owned).or_else(|| kind_phrase(error.kind()))
}

fn without_urls(text: &str) -> String {
    let cleaned = text
        .split_whitespace()
        .filter(|word| !word.contains("://") && !word.contains('?'))
        .collect::<Vec<_>>()
        .join(" ");
    truncate(cleaned.trim(), NETWORK_MESSAGE_LIMIT)
}

fn innermost_cause(error: &(dyn StdError + 'static)) -> String {
    let mut parent = None;
    let mut current = error;
    while let Some(next) = current.source() {
        parent = Some(current);
        current = next;
    }
    let os_error = current.downcast_ref::<std::io::Error>().and_then(|io| io.raw_os_error().map(|code| (io, code)));
    if let Some((io, code)) = os_error {
        if let Some(phrase) = os_cause(io) {
            return phrase;
        }
        let what = parent
            .map(|p| without_urls(&p.to_string().replace(&io.to_string(), "")).trim_end_matches([':', ' ']).to_owned())
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "connection failed".to_owned());
        return format!("{what} (os error {code})");
    }
    let text = current.to_string();
    if text.starts_with(STD_LOOKUP_FAILURE_PREFIX) {
        return LOOKUP_FAILED.to_owned();
    }
    let cleaned = without_urls(&text);
    if cleaned.is_empty() {
        "connection failed".to_owned()
    } else {
        cleaned
    }
}

pub fn transport_message(text: &str) -> String {
    let cleaned = without_urls(text);
    if cleaned.is_empty() {
        "connection failed".to_owned()
    } else {
        cleaned
    }
}

pub fn io_cause(error: &std::io::Error) -> String {
    match error.raw_os_error() {
        Some(code) => os_cause(error).unwrap_or_else(|| format!("OS error {code}")),
        None => truncate(&error.to_string(), NETWORK_MESSAGE_LIMIT),
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

fn plain_detail(body: &str, content_type: Option<&str>) -> Option<String> {
    let typed = content_type.map(str::to_ascii_lowercase).unwrap_or_default();
    if ["html", "json", "xml"].iter().any(|kind| typed.contains(kind)) {
        return None;
    }
    let text = body.trim();
    if text.is_empty() || text.contains('<') || text.starts_with('{') || text.starts_with('[') {
        return None;
    }
    let folded = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let printable: String = folded.chars().filter(|c| !c.is_control()).collect();
    Some(truncate(&printable, DETAIL_LIMIT)).filter(|d| !d.is_empty())
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

fn host_of(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

fn reach_context(op: &Operation, url: &str) -> MastErrorContext {
    MastErrorContext {
        host: host_of(url),
        ..base_context(op)
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
    let server_detail = || detail.clone().or_else(|| plain_detail(body, content_type));
    if status >= 500 {
        context.detail = server_detail();
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
        context.detail = server_detail();
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
        MastError::Transport { source, url } => {
            MastCmdError::new(MastErrorCode::Network, innermost_cause(source)).with_context(reach_context(op, url))
        }
        MastError::RedirectRefused { url } => {
            MastCmdError::new(MastErrorCode::Network, "redirect refused").with_context(reach_context(op, url))
        }
        MastError::Timeout { timeout_secs, url } => {
            let mut context = reach_context(op, url);
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
        MastError::Empty { .. } => MastCmdError::new(MastErrorCode::Decode, "empty answer").with_context(base_context(op)),
        MastError::Decode { .. } | MastError::Encode(_) => unexpected_format(op),
        MastError::Io(source) => MastCmdError::new(MastErrorCode::Cache, io_cause(source)),
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
    fn network_and_timeout_errors_name_the_host_they_tried() {
        let thumb = op(MastMissionId::Hst, OpKind::Thumbnail, "get_image_thumbnail");
        let builder_error = reqwest::Client::new().get("not a url").build().unwrap_err();
        let refused = classify(
            &MastError::Transport {
                url: "http://127.0.0.1:9/search/hst/api/v0.1/get_image_thumbnail?fileset_name=X".into(),
                source: builder_error,
            },
            &thumb,
            0,
        );
        assert_eq!(refused.context.host.as_deref(), Some("127.0.0.1:9"));
        let slow = MastError::Timeout {
            url: "https://mast.stsci.edu/search/hst/api/v0.1/get_image_thumbnail?fileset_name=X".into(),
            timeout_secs: 30,
        };
        assert_eq!(classify(&slow, &thumb, 0).context.host.as_deref(), Some("mast.stsci.edu"));
        let redirect = classify(&MastError::RedirectRefused { url: "http://127.0.0.1:43123/x".into() }, &thumb, 0);
        assert_eq!(redirect.context.host.as_deref(), Some("127.0.0.1:43123"));
        assert_eq!(classify(&MastError::Timeout { url: "u".into(), timeout_secs: 30 }, &thumb, 0).context.host, None);
        assert_eq!(classify(&status(500, "Internal Server Error", "text/plain", None), &thumb, 0).context.host, None);
        let json = serde_json::to_value(&refused).unwrap();
        assert_eq!(json["context"]["host"], "127.0.0.1:9");
    }

    #[test]
    fn server_status_keeps_what_mast_said() {
        let thumb = op(MastMissionId::Hst, OpKind::Thumbnail, "get_image_thumbnail");
        let search = op(MastMissionId::Jwst, OpKind::Search, "search");

        let json = classify(&status(500, r#"{"detail":"image index backend timed out"}"#, "application/json", None), &thumb, 0);
        assert_eq!(json.code, MastErrorCode::Server);
        assert_eq!(json.message, "HTTP 500");
        assert_eq!(json.context.status, Some(500));
        assert_eq!(json.context.detail.as_deref(), Some("image index backend timed out"));
        assert_eq!(json.context.operation.as_deref(), Some("search/hst/api/v0.1/get_image_thumbnail"));

        let plain = classify(&status(500, "Internal Server Error", "text/plain; charset=utf-8", None), &thumb, 0);
        assert_eq!(plain.message, "HTTP 500");
        assert_eq!(plain.context.detail.as_deref(), Some("Internal Server Error"));

        let folded = classify(&status(503, "Service\r\n  Unavailable\n", "text/plain", None), &thumb, 0);
        assert_eq!(folded.code, MastErrorCode::Server);
        assert_eq!(folded.context.detail.as_deref(), Some("Service Unavailable"));

        let nginx = classify(&status(504, "<html><body><h1>504 Gateway Time-out</h1></body></html>", "text/html", None), &thumb, 0);
        assert_eq!(nginx.context.status, Some(504));
        assert_eq!(nginx.context.detail, None);
        let tagless_html = classify(&status(502, "Bad Gateway", "text/html", None), &thumb, 0);
        assert_eq!(tagless_html.context.detail, None);

        let long = classify(&status(502, &"b".repeat(900), "text/plain", None), &thumb, 0);
        assert_eq!(long.context.detail.as_ref().map(|d| d.chars().count()), Some(DETAIL_LIMIT));

        for body in [r#"{"error":"x"}"#, r#"["x"]"#, "   ", ""] {
            let quiet = classify(&status(500, body, "application/json", None), &thumb, 0);
            assert_eq!(quiet.context.detail, None, "{body}");
            let untyped = classify(&status(500, body, "", None), &thumb, 0);
            assert_eq!(untyped.context.detail, None, "{body}");
        }

        let gone = classify(&status(404, r#"{"detail":"Dataset gone"}"#, "application/json", None), &search, 0);
        assert_eq!(gone.code, MastErrorCode::Server);
        assert_eq!(gone.message, "HTTP 404");
        assert_eq!(gone.context.detail.as_deref(), Some("Dataset gone"));
        let gone_plain = classify(&status(404, "no such dataset", "text/plain", None), &search, 0);
        assert_eq!(gone_plain.context.detail.as_deref(), Some("no such dataset"));

        for error in [&json, &plain, &folded, &nginx, &long, &gone] {
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

    #[derive(Debug)]
    struct Wrapped {
        text: &'static str,
        source: std::io::Error,
    }

    impl std::fmt::Display for Wrapped {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.text)
        }
    }

    impl StdError for Wrapped {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            Some(&self.source)
        }
    }

    fn os_error_where(wanted: impl Fn(&std::io::Error) -> bool) -> std::io::Error {
        (1..20_000).map(std::io::Error::from_raw_os_error).find(|e| wanted(e)).expect("an OS error code of that kind")
    }

    fn os_error_of(kind: std::io::ErrorKind) -> std::io::Error {
        os_error_where(|e| e.kind() == kind)
    }

    fn uncategorized_os_error() -> std::io::Error {
        os_error_where(|e| format!("{:?}", e.kind()) == "Uncategorized")
    }

    #[test]
    fn os_error_causes_read_the_same_in_every_system_language() {
        use std::io::ErrorKind;
        for (kind, phrase) in [
            (ErrorKind::ConnectionRefused, "connection refused"),
            (ErrorKind::ConnectionReset, "connection reset"),
            (ErrorKind::ConnectionAborted, "connection aborted"),
            (ErrorKind::TimedOut, "timed out"),
        ] {
            let wrapped = Wrapped { text: "tcp connect error", source: os_error_of(kind) };
            assert_eq!(innermost_cause(&wrapped), phrase, "{kind:?}");
        }

        let unmapped = uncategorized_os_error();
        let code = unmapped.raw_os_error().unwrap();
        assert_eq!(innermost_cause(&Wrapped { text: "dns error", source: unmapped }), format!("dns error (os error {code})"));
        assert_eq!(innermost_cause(&uncategorized_os_error()), format!("connection failed (os error {code})"));

        let custom = std::io::Error::other("peer closed connection without sending TLS close_notify");
        assert_eq!(
            innermost_cause(&Wrapped { text: "client error (Connect)", source: custom }),
            "peer closed connection without sending TLS close_notify"
        );

        let thumb = op(MastMissionId::Jwst, OpKind::Thumbnail, "get_image_thumbnail");
        let missing = classify(&MastError::Io(os_error_of(ErrorKind::NotFound)), &thumb, 0);
        assert_eq!(missing.code, MastErrorCode::Cache);
        assert_eq!(missing.message, "not found");
        assert_eq!(io_cause(&os_error_of(ErrorKind::PermissionDenied)), "permission denied");
        assert_eq!(io_cause(&uncategorized_os_error()), format!("OS error {code}"));
        assert_eq!(io_cause(&std::io::Error::other("disk full")), "disk full");
    }

    fn plain_phrase(text: &str) -> bool {
        !text.is_empty() && text.chars().all(|c| c.is_ascii_lowercase() || c == ' ' || c == '-')
    }

    #[test]
    fn every_os_error_kind_std_names_reads_as_a_fixed_phrase() {
        let named: Vec<std::io::Error> = (1..20_000)
            .map(std::io::Error::from_raw_os_error)
            .filter(|e| !matches!(format!("{:?}", e.kind()).as_str(), "Uncategorized" | "Other"))
            .collect();
        assert!(named.len() > 10, "{}", named.len());
        for error in named {
            let kind = error.kind();
            let code = error.raw_os_error();
            let cause = io_cause(&error);
            assert!(plain_phrase(&cause), "{kind:?} {code:?}: {cause}");
            assert_eq!(innermost_cause(&Wrapped { text: "tcp connect error", source: error }), cause, "{kind:?} {code:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_name_lookup_and_file_lock_codes_read_as_phrases() {
        let lookup = |code| innermost_cause(&Wrapped { text: "dns error", source: std::io::Error::from_raw_os_error(code) });
        assert_eq!(lookup(11001), "host not found");
        assert_eq!(lookup(11002), "host name lookup failed");
        assert_eq!(lookup(11003), "host name lookup failed");
        assert_eq!(lookup(11004), "host has no address");
        assert_eq!(io_cause(&std::io::Error::from_raw_os_error(32)), "file in use by another program");
        assert_eq!(io_cause(&std::io::Error::from_raw_os_error(33)), "file locked by another program");
        let thumb = op(MastMissionId::Jwst, OpKind::Thumbnail, "get_image_thumbnail");
        let shared = classify(&MastError::Io(std::io::Error::from_raw_os_error(32)), &thumb, 0);
        assert_eq!((shared.code, shared.message.as_str()), (MastErrorCode::Cache, "file in use by another program"));
    }

    #[test]
    fn name_lookup_failures_from_std_read_the_same_in_every_system_language() {
        let localized = std::io::Error::other("failed to lookup address information: Nome ou serviço desconhecido");
        assert_eq!(innermost_cause(&Wrapped { text: "dns error", source: localized }), "host name lookup failed");
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
