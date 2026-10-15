use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;

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

pub const DEFAULT_BASE_URL: &str = "https://mast.stsci.edu";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const ERROR_BODY_LIMIT: usize = 512;
const JSON_MIME: &str = "application/json";

pub type QueryPairs<'a> = [(&'a str, &'a str)];

#[derive(Debug, Clone)]
pub struct MastClientBuilder {
    base_url: String,
    timeout: Duration,
    user_agent: String,
    token: Option<String>,
}

impl Default for MastClientBuilder {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: concat!("AstroBurst/", env!("CARGO_PKG_VERSION")).to_owned(),
            token: None,
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
        self.token = Some(token.into()).filter(|value| !value.trim().is_empty());
        self
    }

    pub fn build(self) -> MastResult<MastClient> {
        let http = Client::builder()
            .timeout(self.timeout)
            .user_agent(self.user_agent)
            .build()
            .map_err(MastError::Client)?;
        Ok(MastClient {
            http,
            base_url: normalize_base_url(&self.base_url),
            token: self.token,
        })
    }
}

#[derive(Debug, Clone)]
pub struct MastClient {
    http: Client,
    base_url: String,
    token: Option<String>,
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
        }
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into()).filter(|value| !value.trim().is_empty());
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
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
        let (status, body) = self.execute(&url, request).await?;
        decode_json(status, &url, &body)
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
        let (status, body) = self.execute(&url, request).await?;
        decode_json(status, &url, &body)
    }

    pub async fn get_bytes(&self, path: &str, query: &QueryPairs<'_>) -> MastResult<Vec<u8>> {
        let url = self.url(path);
        let request = self.http.get(&url).query(query);
        let (_, body) = self.execute(&url, request).await?;
        Ok(body)
    }

    async fn execute(
        &self,
        url: &str,
        request: RequestBuilder,
    ) -> MastResult<(StatusCode, Vec<u8>)> {
        let request = match &self.token {
            Some(token) => request.header(AUTHORIZATION, format!("token {token}")),
            None => request,
        };
        let transport = |source| MastError::Transport {
            url: url.to_owned(),
            source,
        };
        let response = request.send().await.map_err(transport)?;
        let status = response.status();
        let body = response.bytes().await.map_err(transport)?.to_vec();
        if status.is_success() {
            Ok((status, body))
        } else {
            Err(error_for_status(status, url, &body))
        }
    }
}

fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        DEFAULT_BASE_URL.to_owned()
    } else {
        trimmed.to_owned()
    }
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

fn error_for_status(status: StatusCode, url: &str, body: &[u8]) -> MastError {
    if status == StatusCode::UNPROCESSABLE_ENTITY {
        if let Ok(detail) = serde_json::from_slice::<HttpValidationError>(body) {
            return MastError::Validation {
                url: url.to_owned(),
                summary: detail.summary(),
                detail,
            };
        }
    }
    let text = String::from_utf8_lossy(body);
    MastError::Status {
        status: status.as_u16(),
        url: url.to_owned(),
        body: text.chars().take(ERROR_BODY_LIMIT).collect(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::infra::mast::models::SearchResults;
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    pub(crate) struct CapturedRequest {
        pub method: String,
        pub target: String,
        pub headers: String,
        pub body: String,
    }

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
        match error_for_status(StatusCode::BAD_REQUEST, "u", body.as_bytes()) {
            MastError::Status { status, body, .. } => {
                assert_eq!(status, 400);
                assert_eq!(body.len(), ERROR_BODY_LIMIT);
            }
            other => panic!("unexpected error {other:?}"),
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
