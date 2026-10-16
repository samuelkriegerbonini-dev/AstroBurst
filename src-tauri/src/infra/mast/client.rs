use std::fmt;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, COOKIE, RETRY_AFTER};
use reqwest::{redirect, Client, RequestBuilder, Response, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::api::classy::ClassyApi;
use super::api::hst::HstApi;
use super::api::iue::IueApi;
use super::api::jwst::JwstApi;
use super::api::multi_mission::MultiMissionApi;
use super::api::roman::RomanApi;
use super::api::roman_cgi::RomanCgiApi;
use super::api::roman_spectra::RomanSpectraApi;
use super::api::tess::TessApi;
use super::api::ullyses::UllysesApi;
use super::error::{MastError, MastResult};
use super::models::HttpValidationError;
use super::secret::SecretToken;
use crate::core::mast::jpeg::{sniff, ImageKind, SNIFF_LEN};

pub const DEFAULT_BASE_URL: &str = "https://mast.stsci.edu";
pub const DEFAULT_JSON_CAP: u64 = 16 * 1024 * 1024;
pub const THUMBNAIL_CAP: u64 = 256 * 1024;
pub const PREVIEW_CAP: u64 = 64 * 1024 * 1024;
pub const MAX_REDIRECT_HOPS: usize = 3;
pub const MAST_REDIRECT_HOSTS: [&str; 3] = [
    "mast.stsci.edu",
    "stpubdata.s3.us-east-1.amazonaws.com",
    "roman-hlpp-arch-stor-*-ops.s3.us-east-1.amazonaws.com",
];
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const ERROR_BODY_LIMIT: usize = 512;
const ERROR_READ_LIMIT: usize = 4096;
const JSON_MIME: &str = "application/json";
const TOKEN_COOKIE: &str = "mast_token";

pub type QueryPairs<'a> = [(&'a str, &'a str)];
pub type ChunkSink<'a> = dyn FnMut(&[u8], Option<u64>) -> std::io::Result<()> + Send + 'a;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthTransport {
    #[default]
    Header,
    Cookie,
}

impl AuthTransport {
    pub const fn as_str(self) -> &'static str {
        match self {
            AuthTransport::Header => "header",
            AuthTransport::Cookie => "cookie",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "header" => Some(AuthTransport::Header),
            "cookie" => Some(AuthTransport::Cookie),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectAllowlist {
    hosts: Vec<String>,
    loopback: Option<(String, u16)>,
}

impl RedirectAllowlist {
    pub fn empty() -> Self {
        Self { hosts: Vec::new(), loopback: None }
    }

    pub fn mast() -> Self {
        Self {
            hosts: MAST_REDIRECT_HOSTS.iter().map(|host| (*host).to_owned()).collect(),
            loopback: None,
        }
    }

    pub fn allow_https_host(mut self, host: &str) -> Self {
        self.hosts.push(host.to_ascii_lowercase());
        self
    }

    pub fn allow_loopback(mut self, host: &str, port: u16) -> Self {
        self.loopback = Some((host.to_ascii_lowercase(), port));
        self
    }

    pub fn allows(&self, url: &Url) -> bool {
        let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
            return false;
        };
        match url.scheme() {
            "https" => self.hosts.iter().any(|pattern| host_matches(pattern, &host)),
            "http" => self
                .loopback
                .as_ref()
                .is_some_and(|(allowed, port)| *allowed == host && url.port_or_known_default() == Some(*port)),
            _ => false,
        }
    }
}

fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == host,
        Some((prefix, suffix)) => {
            host.len() > prefix.len() + suffix.len()
                && host.starts_with(prefix)
                && host.ends_with(suffix)
                && !host[prefix.len()..host.len() - suffix.len()].contains('.')
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectRules {
    Never,
    Allowlist(RedirectAllowlist),
}

#[derive(Debug)]
struct RedirectRefusal;

impl fmt::Display for RedirectRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("redirect refused")
    }
}

impl std::error::Error for RedirectRefusal {}

fn redirect_policy(rules: &RedirectRules) -> redirect::Policy {
    match rules {
        RedirectRules::Never => redirect::Policy::none(),
        RedirectRules::Allowlist(list) => {
            let list = list.clone();
            redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() > MAX_REDIRECT_HOPS || !list.allows(attempt.url()) {
                    attempt.error(RedirectRefusal)
                } else {
                    attempt.follow()
                }
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageOp {
    Thumbnail,
    Preview,
}

impl ImageOp {
    pub const fn operation(self) -> &'static str {
        match self {
            ImageOp::Thumbnail => "get_image_thumbnail",
            ImageOp::Preview => "get_image_preview",
        }
    }

    pub const fn cap(self) -> u64 {
        match self {
            ImageOp::Thumbnail => THUMBNAIL_CAP,
            ImageOp::Preview => PREVIEW_CAP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageDownload {
    pub final_url: String,
    pub content_type: Option<String>,
    pub declared: Option<u64>,
    pub received: u64,
    pub kind: ImageKind,
}

#[derive(Clone)]
pub struct MastClientBuilder {
    base_url: String,
    timeout: Duration,
    user_agent: String,
    token: Option<SecretToken>,
    transport: AuthTransport,
    redirects: RedirectRules,
    json_cap: u64,
    error_bodies: bool,
}

impl fmt::Debug for MastClientBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MastClientBuilder")
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .field("has_token", &self.token.is_some())
            .field("transport", &self.transport)
            .field("redirects", &self.redirects)
            .finish()
    }
}

impl Default for MastClientBuilder {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: concat!("AstroBurst/", env!("CARGO_PKG_VERSION")).to_owned(),
            token: None,
            transport: AuthTransport::Header,
            redirects: RedirectRules::Allowlist(RedirectAllowlist::mast()),
            json_cap: DEFAULT_JSON_CAP,
            error_bodies: true,
        }
    }
}

impl MastClientBuilder {
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = SecretToken::new(token);
        self
    }

    pub fn secret(mut self, token: SecretToken) -> Self {
        self.token = Some(token);
        self
    }

    pub fn transport(mut self, transport: AuthTransport) -> Self {
        self.transport = transport;
        self
    }

    pub fn redirects(mut self, redirects: RedirectRules) -> Self {
        self.redirects = redirects;
        self
    }

    pub fn json_cap(mut self, json_cap: u64) -> Self {
        self.json_cap = json_cap;
        self
    }

    pub fn redact_error_bodies(mut self) -> Self {
        self.error_bodies = false;
        self
    }

    pub fn build(self) -> MastResult<MastClient> {
        let http = Client::builder()
            .timeout(self.timeout)
            .user_agent(self.user_agent)
            .redirect(redirect_policy(&self.redirects))
            .build()
            .map_err(MastError::Client)?;
        Ok(MastClient {
            http,
            base_url: normalize_base_url(&self.base_url),
            token: self.token,
            transport: self.transport,
            timeout: self.timeout,
            json_cap: self.json_cap,
            error_bodies: self.error_bodies,
        })
    }
}

#[derive(Clone)]
pub struct MastClient {
    http: Client,
    base_url: String,
    token: Option<SecretToken>,
    transport: AuthTransport,
    timeout: Duration,
    json_cap: u64,
    error_bodies: bool,
}

impl fmt::Debug for MastClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MastClient")
            .field("base_url", &self.base_url)
            .field("has_token", &self.token.is_some())
            .field("transport", &self.transport)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl MastClient {
    pub fn new() -> MastResult<Self> {
        Self::builder().build()
    }

    pub fn builder() -> MastClientBuilder {
        MastClientBuilder::default()
    }

    pub fn from_http(http: Client, base_url: impl AsRef<str>) -> Self {
        Self {
            http,
            base_url: normalize_base_url(base_url.as_ref()),
            token: None,
            transport: AuthTransport::Header,
            timeout: DEFAULT_TIMEOUT,
            json_cap: DEFAULT_JSON_CAP,
            error_bodies: true,
        }
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = SecretToken::new(token);
        self
    }

    pub fn with_transport(mut self, transport: AuthTransport) -> Self {
        self.transport = transport;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    pub fn transport(&self) -> AuthTransport {
        self.transport
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn multi_mission(&self) -> MultiMissionApi {
        MultiMissionApi::new(self.clone())
    }

    pub fn classy(&self) -> ClassyApi {
        ClassyApi::new(self.clone())
    }

    pub fn hst(&self) -> HstApi {
        HstApi::new(self.clone())
    }

    pub fn iue(&self) -> IueApi {
        IueApi::new(self.clone())
    }

    pub fn jwst(&self) -> JwstApi {
        JwstApi::new(self.clone())
    }

    pub fn roman(&self) -> RomanApi {
        RomanApi::new(self.clone())
    }

    pub fn roman_cgi(&self) -> RomanCgiApi {
        RomanCgiApi::new(self.clone())
    }

    pub fn roman_spectra(&self) -> RomanSpectraApi {
        RomanSpectraApi::new(self.clone())
    }

    pub fn tess(&self) -> TessApi {
        TessApi::new(self.clone())
    }

    pub fn ullyses(&self) -> UllysesApi {
        UllysesApi::new(self.clone())
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    pub async fn get_json<T>(&self, path: &str, query: &QueryPairs<'_>) -> MastResult<T>
    where
        T: DeserializeOwned + Default,
    {
        let url = self.url(path);
        let request = self.http.get(&url).query(query).header(ACCEPT, JSON_MIME);
        self.json_response(&url, request).await
    }

    pub async fn post_json<B, T>(&self, path: &str, payload: &B) -> MastResult<T>
    where
        B: Serialize + ?Sized,
        T: DeserializeOwned + Default,
    {
        let url = self.url(path);
        let body = serde_json::to_vec(payload).map_err(MastError::Encode)?;
        let request = self
            .http
            .post(&url)
            .header(ACCEPT, JSON_MIME)
            .header(CONTENT_TYPE, JSON_MIME)
            .body(body);
        self.json_response(&url, request).await
    }

    pub async fn post_form<T>(&self, path: &str, form: &QueryPairs<'_>) -> MastResult<T>
    where
        T: DeserializeOwned + Default,
    {
        let url = self.url(path);
        let request = self.http.post(&url).header(ACCEPT, JSON_MIME).form(form);
        self.json_response(&url, request).await
    }

    pub async fn get_bytes(&self, path: &str, query: &QueryPairs<'_>) -> MastResult<Vec<u8>> {
        let url = self.url(path);
        let request = self.http.get(&url).query(query);
        let response = self.send(&url, request).await?;
        self.read_capped(&url, response, self.json_cap).await
    }

    pub async fn download_image(
        &self,
        path: &str,
        query: &QueryPairs<'_>,
        cap: u64,
        sink: &mut ChunkSink<'_>,
    ) -> MastResult<ImageDownload> {
        let url = self.url(path);
        let request = self.http.get(&url).query(query);
        let mut response = self.send(&url, request).await?;
        let final_url = response.url().to_string();
        let content_type = header_text(response.headers(), CONTENT_TYPE);
        let declared = response.content_length();
        if let Some(bytes) = declared.filter(|bytes| *bytes > cap) {
            return Err(MastError::TooLarge { url, bytes, limit: cap });
        }
        let mut received = 0u64;
        let mut head: Vec<u8> = Vec::with_capacity(SNIFF_LEN);
        let mut kind = None;
        while let Some(chunk) = response.chunk().await.map_err(|e| self.transport_error(&url, e))? {
            received += chunk.len() as u64;
            if received > cap {
                return Err(MastError::TooLarge { url, bytes: received, limit: cap });
            }
            if kind.is_none() {
                let take = chunk.len().min(SNIFF_LEN - head.len());
                head.extend_from_slice(&chunk[..take]);
                if head.len() == SNIFF_LEN {
                    kind = Some(sniff(&head).ok_or_else(|| MastError::NotImage { url: url.clone() })?);
                }
            }
            sink(&chunk, declared).map_err(MastError::Io)?;
        }
        let kind = match kind {
            Some(kind) => kind,
            None => sniff(&head).ok_or_else(|| MastError::NotImage { url: url.clone() })?,
        };
        Ok(ImageDownload { final_url, content_type, declared, received, kind })
    }

    async fn json_response<T>(&self, url: &str, request: RequestBuilder) -> MastResult<T>
    where
        T: DeserializeOwned + Default,
    {
        let response = self.send(url, request).await?;
        let status = response.status();
        let body = self.read_capped(url, response, self.json_cap).await?;
        decode_json(status, url, &body)
    }

    fn authorize(&self, request: RequestBuilder) -> RequestBuilder {
        let Some(token) = &self.token else { return request };
        let (name, raw) = match self.transport {
            AuthTransport::Header => (AUTHORIZATION, format!("token {}", token.expose())),
            AuthTransport::Cookie => (COOKIE, format!("{TOKEN_COOKIE}={}", token.expose())),
        };
        match HeaderValue::from_str(&raw) {
            Ok(mut value) => {
                value.set_sensitive(true);
                request.header(name, value)
            }
            Err(_) => request,
        }
    }

    async fn send(&self, url: &str, request: RequestBuilder) -> MastResult<Response> {
        let request = self.authorize(request).timeout(self.timeout);
        let response = request.send().await.map_err(|e| self.transport_error(url, e))?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(self.status_error(url, response).await)
        }
    }

    fn transport_error(&self, url: &str, source: reqwest::Error) -> MastError {
        if source.is_timeout() {
            MastError::Timeout { url: url.to_owned(), timeout_secs: rounded_secs(self.timeout) }
        } else if source.is_redirect() {
            MastError::RedirectRefused { url: url.to_owned() }
        } else {
            MastError::Transport { url: url.to_owned(), source }
        }
    }

    async fn status_error(&self, url: &str, mut response: Response) -> MastError {
        let status = response.status();
        let headers = response.headers().clone();
        let mut body = Vec::new();
        while body.len() < ERROR_READ_LIMIT {
            match response.chunk().await {
                Ok(Some(chunk)) => body.extend_from_slice(&chunk),
                _ => break,
            }
        }
        body.truncate(ERROR_READ_LIMIT);
        if !self.error_bodies {
            body.clear();
        }
        error_for_status(status, url, &body, &headers)
    }

    async fn read_capped(&self, url: &str, mut response: Response, cap: u64) -> MastResult<Vec<u8>> {
        if let Some(bytes) = response.content_length().filter(|bytes| *bytes > cap) {
            return Err(MastError::TooLarge { url: url.to_owned(), bytes, limit: cap });
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| self.transport_error(url, e))? {
            body.extend_from_slice(&chunk);
            if body.len() as u64 > cap {
                return Err(MastError::TooLarge { url: url.to_owned(), bytes: body.len() as u64, limit: cap });
            }
        }
        Ok(body)
    }
}

fn rounded_secs(duration: Duration) -> u64 {
    duration.as_millis().div_ceil(1000) as u64
}

fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        DEFAULT_BASE_URL.to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn header_text(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    headers.get(name).and_then(|value| value.to_str().ok()).map(|value| value.trim().to_owned())
}

fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
    header_text(headers, RETRY_AFTER).and_then(|value| value.parse::<u64>().ok())
}

fn decode_json<T>(status: StatusCode, url: &str, body: &[u8]) -> MastResult<T>
where
    T: DeserializeOwned + Default,
{
    if status == StatusCode::NO_CONTENT || body.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    serde_json::from_slice(body).map_err(|source| MastError::Decode {
        url: url.to_owned(),
        source,
    })
}

fn status_detail(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let detail = match value.get("detail")? {
        serde_json::Value::String(text) => text.trim().to_owned(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("msg").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join("; "),
        _ => return None,
    };
    (!detail.is_empty()).then_some(detail)
}

fn error_for_status(status: StatusCode, url: &str, body: &[u8], headers: &HeaderMap) -> MastError {
    if status == StatusCode::UNPROCESSABLE_ENTITY {
        if let Ok(detail) = serde_json::from_slice::<HttpValidationError>(body) {
            if !detail.detail.is_empty() {
                return MastError::Validation {
                    url: url.to_owned(),
                    summary: detail.summary(),
                    detail,
                };
            }
        }
    }
    let text = String::from_utf8_lossy(body);
    MastError::Status {
        status: status.as_u16(),
        url: url.to_owned(),
        body: text.chars().take(ERROR_BODY_LIMIT).collect(),
        detail: status_detail(body),
        content_type: header_text(headers, CONTENT_TYPE),
        retry_after_secs: parse_retry_after(headers),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::infra::mast::models::SearchResults;
    use crate::infra::mast::testing::{serve_routes, Route, MARKER_TOKEN};
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    pub(crate) struct CapturedRequest {
        pub method: String,
        pub target: String,
        pub headers: String,
        pub body: String,
    }

    pub(crate) const JPEG_HEAD: [u8; 8] = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F'];

    pub(crate) async fn serve_once(
        status_line: &'static str,
        content_type: &'static str,
        response_body: Vec<u8>,
    ) -> (String, tokio::task::JoinHandle<CapturedRequest>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 4096];
            let header_end = loop {
                let read = socket.read(&mut chunk).await.unwrap();
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(position) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                    break position + 4;
                }
                assert!(read > 0, "connection closed before headers were complete");
            };
            let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
            let content_length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            while buffer.len() < header_end + content_length {
                let read = socket.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            let mut response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response_body.len()
            )
            .into_bytes();
            response.extend_from_slice(&response_body);
            socket.write_all(&response).await.unwrap();
            socket.shutdown().await.ok();
            let mut request_line = head.lines().next().unwrap_or_default().split_whitespace();
            CapturedRequest {
                method: request_line.next().unwrap_or_default().to_owned(),
                target: request_line.next().unwrap_or_default().to_owned(),
                headers: head.to_ascii_lowercase(),
                body: String::from_utf8_lossy(&buffer[header_end..]).to_string(),
            }
        });
        (format!("http://{address}"), handle)
    }

    pub(crate) fn test_client(base_url: &str) -> MastClient {
        MastClient::builder().base_url(base_url).build().unwrap()
    }

    fn loopback_client(base: &str, port: u16) -> MastClient {
        MastClient::builder()
            .base_url(base)
            .redirects(RedirectRules::Allowlist(RedirectAllowlist::mast().allow_loopback("127.0.0.1", port)))
            .build()
            .unwrap()
    }

    #[test]
    fn base_url_is_normalized() {
        let client = MastClient::from_http(Client::new(), " https://example.org/ ");
        assert_eq!(client.base_url(), "https://example.org");
        assert_eq!(
            client.url("/search/jwst"),
            "https://example.org/search/jwst"
        );
        assert_eq!(
            MastClient::from_http(Client::new(), "").base_url(),
            DEFAULT_BASE_URL
        );
    }

    #[test]
    fn blank_token_is_ignored() {
        let client = MastClient::from_http(Client::new(), DEFAULT_BASE_URL).with_token("  ");
        assert!(!client.has_token());
        assert!(client.with_token("abc").has_token());
    }

    #[test]
    fn client_debug_never_contains_token() {
        let builder = MastClient::builder().token(MARKER_TOKEN).transport(AuthTransport::Cookie);
        assert!(!format!("{builder:?}").contains(MARKER_TOKEN));
        let client = builder.build().unwrap();
        let printed = format!("{client:?} {:#?}", client.clone().with_transport(AuthTransport::Header));
        assert!(!printed.contains(MARKER_TOKEN), "{printed}");
        assert!(printed.contains("MastClient"));
    }

    #[test]
    fn allowlist_matches_mast_hosts_only() {
        let list = RedirectAllowlist::mast();
        for allowed in [
            "https://mast.stsci.edu/search/x",
            "https://stpubdata.s3.us-east-1.amazonaws.com/hst/x_thumb.jpg",
            "https://roman-hlpp-arch-stor-123-ops.s3.us-east-1.amazonaws.com/x",
        ] {
            assert!(list.allows(&Url::parse(allowed).unwrap()), "{allowed}");
        }
        for refused in [
            "http://mast.stsci.edu/x",
            "https://mast.stsci.edu.evil.example/x",
            "https://evilmast.stsci.edu/x",
            "https://roman-hlpp-arch-stor-a.b-ops.s3.us-east-1.amazonaws.com/x",
            "https://roman-hlpp-arch-stor--ops.s3.us-east-1.amazonaws.com/x",
            "http://127.0.0.1:8080/x",
            "ftp://mast.stsci.edu/x",
        ] {
            assert!(!list.allows(&Url::parse(refused).unwrap()), "{refused}");
        }
        let loopback = RedirectAllowlist::empty().allow_loopback("127.0.0.1", 8080);
        assert!(loopback.allows(&Url::parse("http://127.0.0.1:8080/x").unwrap()));
        assert!(!loopback.allows(&Url::parse("http://127.0.0.1:8081/x").unwrap()));
        assert!(!loopback.allows(&Url::parse("https://127.0.0.1:8080/x").unwrap()));
    }

    #[tokio::test]
    async fn redirect_to_allowed_host_is_followed() {
        let server = serve_routes(vec![
            Route::get("/final").json("[1]"),
            Route::get("/start").status(307).header("Location", "/final"),
        ])
        .await;
        let decoded: Vec<Value> = loopback_client(&server.base, server.port).get_json("start", &[]).await.unwrap();
        assert_eq!(decoded, vec![json!(1)]);
        assert_eq!(server.count("/final"), 1);
    }

    #[tokio::test]
    async fn redirect_to_foreign_host_is_refused() {
        let foreign = serve_routes(vec![Route::get("/").json("[]")]).await;
        let server = serve_routes(vec![Route::get("/start")
            .status(307)
            .header("Location", &format!("{}/steal", foreign.base))])
        .await;
        let error = loopback_client(&server.base, server.port)
            .get_json::<Vec<Value>>("start", &[])
            .await
            .unwrap_err();
        assert!(matches!(error, MastError::RedirectRefused { .. }), "{error:?}");
        assert!(foreign.requests().is_empty());
    }

    #[tokio::test]
    async fn redirect_to_http_is_refused() {
        let server = serve_routes(vec![Route::get("/start")
            .status(307)
            .header("Location", "http://mast.stsci.edu/search")])
        .await;
        let error = loopback_client(&server.base, server.port)
            .get_json::<Vec<Value>>("start", &[])
            .await
            .unwrap_err();
        assert!(matches!(error, MastError::RedirectRefused { .. }), "{error:?}");
    }

    #[tokio::test]
    async fn redirects_stop_after_three_hops() {
        let server = serve_routes(vec![
            Route::get("/r1").status(307).header("Location", "/r2"),
            Route::get("/r2").status(307).header("Location", "/r3"),
            Route::get("/r3").status(307).header("Location", "/r4"),
            Route::get("/r4").status(307).header("Location", "/r5"),
            Route::get("/r5").json("[]"),
        ])
        .await;
        let client = loopback_client(&server.base, server.port);
        let error = client.get_json::<Vec<Value>>("r1", &[]).await.unwrap_err();
        assert!(matches!(error, MastError::RedirectRefused { .. }), "{error:?}");
        assert_eq!(server.count("/r5"), 0);
        let decoded: Vec<Value> = client.get_json("r2", &[]).await.unwrap();
        assert!(decoded.is_empty());
    }

    #[tokio::test]
    async fn token_client_does_not_follow_redirects() {
        let server = serve_routes(vec![
            Route::get("/final").json("[]"),
            Route::get("/start").status(307).header("Location", "/final"),
        ])
        .await;
        let client = MastClient::builder()
            .base_url(&server.base)
            .token(MARKER_TOKEN)
            .redirects(RedirectRules::Never)
            .build()
            .unwrap();
        let error = client.get_json::<Vec<Value>>("start", &[]).await.unwrap_err();
        assert!(matches!(error, MastError::Status { status: 307, .. }), "{error:?}");
        assert_eq!(server.count("/final"), 0);
    }

    #[tokio::test]
    async fn json_body_over_cap_is_too_large() {
        let body = json!({"results": vec!["x"; 64]}).to_string();
        let server = serve_routes(vec![
            Route::get("/declared").json(body.clone()),
            Route::get("/streamed").oversized(b"[".to_vec(), 4096, false),
        ])
        .await;
        let client = MastClient::builder().base_url(&server.base).json_cap(100).build().unwrap();
        let declared = client.get_json::<Value>("declared", &[]).await.unwrap_err();
        assert!(matches!(declared, MastError::TooLarge { bytes, limit: 100, .. } if bytes == body.len() as u64), "{declared:?}");
        let streamed = client.get_json::<Value>("streamed", &[]).await.unwrap_err();
        assert!(matches!(streamed, MastError::TooLarge { bytes, limit: 100, .. } if bytes > 100), "{streamed:?}");
        let fits = MastClient::builder().base_url(&server.base).json_cap(4096).build().unwrap();
        assert!(fits.get_json::<Value>("declared", &[]).await.is_ok());
    }

    #[tokio::test]
    async fn image_body_html_is_not_image() {
        let server = serve_routes(vec![Route::get("/thumb").html("<html><body>Service unavailable</body></html>")]).await;
        let client = test_client(&server.base);
        let mut written = Vec::new();
        let error = client
            .download_image("thumb", &[("fileset_name", "x")], THUMBNAIL_CAP, &mut |chunk: &[u8], _total: Option<u64>| {
                written.extend_from_slice(chunk);
                Ok(())
            })
            .await
            .unwrap_err();
        assert!(matches!(error, MastError::NotImage { .. }), "{error:?}");
    }

    #[tokio::test]
    async fn image_download_streams_chunks_and_reports_final_url() {
        let mut jpeg = JPEG_HEAD.to_vec();
        jpeg.extend(std::iter::repeat_n(7u8, 50_000));
        let server = serve_routes(vec![
            Route::get("/s3/x_drc_thumb.jpg").header("Content-Type", "image/jpeg").bytes(jpeg.clone()),
            Route::get("/thumb").status(307).header("Location", "/s3/x_drc_thumb.jpg"),
        ])
        .await;
        let client = loopback_client(&server.base, server.port);
        let mut written = Vec::new();
        let download = client
            .download_image("thumb", &[("fileset_name", "x")], THUMBNAIL_CAP, &mut |chunk: &[u8], _total: Option<u64>| {
                written.extend_from_slice(chunk);
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(written, jpeg);
        assert_eq!(download.received, jpeg.len() as u64);
        assert_eq!(download.kind, ImageKind::Jpeg);
        assert!(download.final_url.ends_with("/s3/x_drc_thumb.jpg"), "{}", download.final_url);
    }

    #[tokio::test]
    async fn per_request_timeout_maps_to_timeout() {
        let server = serve_routes(vec![Route::get("/slow").delay(Duration::from_secs(5)).json("[]")]).await;
        let client = test_client(&server.base).with_timeout(Duration::from_millis(300));
        let started = std::time::Instant::now();
        let error = client.get_json::<Vec<Value>>("slow", &[]).await.unwrap_err();
        assert!(matches!(error, MastError::Timeout { timeout_secs: 1, .. }), "{error:?}");
        assert!(started.elapsed() < Duration::from_secs(3));
        let error = test_client(&server.base)
            .with_timeout(Duration::from_millis(2100))
            .get_json::<Vec<Value>>("slow", &[])
            .await
            .unwrap_err();
        assert!(matches!(error, MastError::Timeout { timeout_secs: 3, .. }), "{error:?}");
    }

    #[tokio::test]
    async fn cookie_transport_sends_only_the_cookie() {
        let server = serve_routes(vec![Route::get("/ping").json("[]")]).await;
        let client = test_client(&server.base).with_token(MARKER_TOKEN).with_transport(AuthTransport::Cookie);
        let _: Vec<Value> = client.get_json("ping", &[]).await.unwrap();
        let request = &server.requests()[0];
        assert_eq!(request.header("cookie"), Some(format!("mast_token={MARKER_TOKEN}").as_str()));
        assert_eq!(request.header("authorization"), None);
    }

    #[tokio::test]
    async fn post_form_encodes_fields() {
        let server = serve_routes(vec![Route::post("/api/v0/invoke").json(r#"{"resolvedCoordinate":[]}"#)]).await;
        let value: Value = test_client(&server.base)
            .post_form("api/v0/invoke", &[("request", r#"{"service":"Mast.Name.Lookup"}"#)])
            .await
            .unwrap();
        assert_eq!(value["resolvedCoordinate"], json!([]));
        let request = &server.requests()[0];
        assert_eq!(request.header("content-type"), Some("application/x-www-form-urlencoded"));
        assert!(request.body.starts_with("request=%7B%22service%22"), "{}", request.body);
    }

    #[tokio::test]
    async fn status_errors_carry_content_type_and_retry_after() {
        let server = serve_routes(vec![Route::get("/busy").status(429).header("Retry-After", "5").json(r#"{"detail":"slow down"}"#)]).await;
        let error = test_client(&server.base).get_json::<Value>("busy", &[]).await.unwrap_err();
        match error {
            MastError::Status { status, content_type, retry_after_secs, body, .. } => {
                assert_eq!(status, 429);
                assert_eq!(content_type.as_deref(), Some("application/json"));
                assert_eq!(retry_after_secs, Some(5));
                assert!(body.contains("slow down"));
            }
            other => panic!("unexpected error {other:?}"),
        }
        let redacted = MastClient::builder().base_url(&server.base).redact_error_bodies().build().unwrap();
        let error = redacted.get_json::<Value>("busy", &[]).await.unwrap_err();
        assert!(matches!(error, MastError::Status { ref body, .. } if body.is_empty()), "{error:?}");
    }

    #[test]
    fn empty_or_no_content_body_decodes_to_default() {
        let decoded: SearchResults<Value> = decode_json(StatusCode::NO_CONTENT, "u", b"").unwrap();
        assert_eq!(decoded.total_results, -1);
        let decoded: Vec<Value> = decode_json(StatusCode::OK, "u", b"  \n").unwrap();
        assert!(decoded.is_empty());
        assert!(matches!(
            decode_json::<Vec<Value>>(StatusCode::OK, "u", b"{oops"),
            Err(MastError::Decode { .. })
        ));
    }

    #[test]
    fn unprocessable_entity_maps_to_validation_error() {
        let body = json!({"detail": [{"loc": ["query", "fileset_name"], "msg": "Field required", "type": "missing"}]});
        let error = error_for_status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "u",
            body.to_string().as_bytes(),
            &HeaderMap::new(),
        );
        match error {
            MastError::Validation {
                summary, detail, ..
            } => {
                assert_eq!(summary, "query.fileset_name: Field required");
                assert_eq!(detail.detail.len(), 1);
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn other_statuses_keep_truncated_body() {
        let body = "x".repeat(ERROR_BODY_LIMIT * 2);
        match error_for_status(StatusCode::BAD_REQUEST, "u", body.as_bytes(), &HeaderMap::new()) {
            MastError::Status { status, body, .. } => {
                assert_eq!(status, 400);
                assert_eq!(body.len(), ERROR_BODY_LIMIT);
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn status_detail_survives_bodies_longer_than_the_kept_text() {
        let detail = "z".repeat(ERROR_BODY_LIMIT + 300);
        let body = json!({"detail": detail}).to_string();
        match error_for_status(StatusCode::BAD_REQUEST, "u", body.as_bytes(), &HeaderMap::new()) {
            MastError::Status { body: kept, detail: parsed, .. } => {
                assert_eq!(kept.len(), ERROR_BODY_LIMIT);
                assert_eq!(parsed.as_deref(), Some(detail.as_str()));
            }
            other => panic!("unexpected error {other:?}"),
        }
        for (body, expected) in [
            (r#"{"detail":"  Radius too big  "}"#, Some("Radius too big")),
            (r#"{"detail":[{"msg":"a"},{"msg":"b"}]}"#, Some("a; b")),
            (r#"{"detail":""}"#, None),
            (r#"{"detail":5}"#, None),
            ("<html>nginx</html>", None),
            ("", None),
        ] {
            assert_eq!(status_detail(body.as_bytes()).as_deref(), expected, "{body}");
        }
    }

    #[tokio::test]
    async fn token_is_sent_as_authorization_header() {
        let (base, server) = serve_once("200 OK", "application/json", b"[]".to_vec()).await;
        let client = test_client(&base).with_token("secret");
        let decoded: Vec<Value> = client.get_json("ping", &[("a", "1")]).await.unwrap();
        assert!(decoded.is_empty());
        let request = server.await.unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.target, "/ping?a=1");
        assert!(request.headers.contains("authorization: token secret"));
    }

    #[tokio::test]
    async fn server_error_is_reported_with_status() {
        let (base, server) =
            serve_once("500 Internal Server Error", "text/plain", b"boom".to_vec()).await;
        let error = test_client(&base).get_bytes("x", &[]).await.unwrap_err();
        server.await.unwrap();
        assert!(matches!(error, MastError::Status { status: 500, ref body, .. } if body == "boom"));
    }
}
