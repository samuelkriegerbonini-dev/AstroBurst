use std::time::Duration;

use anyhow::{bail, Result};
use reqwest::{redirect, Client, StatusCode};

const USER_AGENT: &str = concat!("AstroBurst/", env!("CARGO_PKG_VERSION"));
const MAX_REDIRECT_HOPS: usize = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const TRANSFER_READ_TIMEOUT: Duration = Duration::from_secs(60);
const GET_ATTEMPTS: u32 = 3;
const RETRY_BACKOFF: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedirectRule {
    pub https_only: bool,
    pub max_hops: usize,
    pub hosts: &'static [&'static str],
}

impl RedirectRule {
    pub const fn https(hosts: &'static [&'static str]) -> Self {
        Self {
            https_only: true,
            max_hops: MAX_REDIRECT_HOPS,
            hosts,
        }
    }

    fn allows(&self, url: &reqwest::Url) -> bool {
        if self.https_only && url.scheme() != "https" {
            return false;
        }
        url.host_str().is_some_and(|host| {
            self.hosts
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(host))
        })
    }

    fn policy(self) -> redirect::Policy {
        redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > self.max_hops || !self.allows(attempt.url()) {
                attempt.error("redirect refused by the profile's redirect rule")
            } else {
                attempt.follow()
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpProfile {
    pub user_agent: &'static str,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub total_timeout: Option<Duration>,
    pub redirect: RedirectRule,
}

impl HttpProfile {
    pub const fn request() -> Self {
        Self {
            user_agent: USER_AGENT,
            connect_timeout: CONNECT_TIMEOUT,
            read_timeout: REQUEST_TIMEOUT,
            total_timeout: Some(REQUEST_TIMEOUT),
            redirect: RedirectRule::https(&[]),
        }
    }

    pub const fn transfer() -> Self {
        Self {
            user_agent: USER_AGENT,
            connect_timeout: CONNECT_TIMEOUT,
            read_timeout: TRANSFER_READ_TIMEOUT,
            total_timeout: None,
            redirect: RedirectRule::https(&[]),
        }
    }
}

pub fn build_client(profile: &HttpProfile) -> reqwest::Result<Client> {
    let mut builder = Client::builder()
        .user_agent(profile.user_agent)
        .connect_timeout(profile.connect_timeout)
        .read_timeout(profile.read_timeout)
        .redirect(profile.redirect.policy());
    if let Some(total) = profile.total_timeout {
        builder = builder.timeout(total);
    }
    builder.build()
}

pub(crate) fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !parts.iter().any(|p| p.contains(&text)) {
            parts.push(text);
        }
        source = cause.source();
    }
    parts.join(": ")
}

pub(crate) fn is_transient(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_request() || error.is_timeout() || error.is_body()
}

pub async fn get_with_retry(
    client: &Client,
    url: &str,
    label: &str,
    subject: &str,
    referer: Option<&str>,
) -> Result<(StatusCode, Vec<u8>)> {
    let mut last_error: Option<reqwest::Error> = None;
    for attempt in 1..=GET_ATTEMPTS {
        if attempt > 1 {
            tokio::time::sleep(RETRY_BACKOFF * (attempt - 1)).await;
        }
        let mut request = client.get(url);
        if let Some(referer) = referer {
            request = request.header(reqwest::header::REFERER, referer);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) if is_transient(&error) => {
                log::warn!(
                    "{label} ({subject}) attempt {attempt}/{GET_ATTEMPTS}: {}",
                    error_chain(&error)
                );
                last_error = Some(error);
                continue;
            }
            Err(error) => bail!(
                "{label} request failed ({subject}): {}",
                error_chain(&error)
            ),
        };
        let status = response.status();
        if status.is_server_error() && attempt < GET_ATTEMPTS {
            log::warn!("{label} ({subject}) attempt {attempt}/{GET_ATTEMPTS}: HTTP {status}");
            continue;
        }
        match response.bytes().await {
            Ok(body) => return Ok((status, body.to_vec())),
            Err(error) if is_transient(&error) => {
                log::warn!(
                    "{label} ({subject}) body attempt {attempt}/{GET_ATTEMPTS}: {}",
                    error_chain(&error)
                );
                last_error = Some(error);
            }
            Err(error) => bail!(
                "{label}: failed to read response body ({subject}): {}",
                error_chain(&error)
            ),
        }
    }
    match last_error {
        Some(error) => bail!(
            "{label} request failed after {GET_ATTEMPTS} attempts ({subject}): {}",
            error_chain(&error)
        ),
        None => bail!("{label} request failed after {GET_ATTEMPTS} attempts ({subject})"),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use reqwest::StatusCode;

    use super::{build_client, get_with_retry, HttpProfile, RedirectRule};

    enum Reply {
        Status(u16, &'static str),
        HangUp,
        Redirect(String),
        Trickle(&'static [u8], Duration),
    }

    struct Canned {
        url: String,
        heads: Arc<Mutex<Vec<String>>>,
    }

    impl Canned {
        fn heads(&self) -> Vec<String> {
            self.heads.lock().unwrap().clone()
        }
    }

    fn read_head(stream: &mut TcpStream) -> String {
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        while !data.windows(4).any(|w| w == b"\r\n\r\n") {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => data.extend_from_slice(&buf[..n]),
            }
        }
        String::from_utf8_lossy(&data).to_ascii_lowercase()
    }

    fn respond(stream: &mut TcpStream, code: u16, extra: &[(&str, &str)], body: &[u8]) {
        let reason = StatusCode::from_u16(code)
            .unwrap()
            .canonical_reason()
            .unwrap_or("");
        let mut head = format!(
            "HTTP/1.1 {code} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in extra {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body);
    }

    fn serve(reply: impl Fn(usize, &str) -> Reply + Send + 'static) -> Canned {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let heads = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&heads);
        let base = url.clone();
        std::thread::spawn(move || {
            for (index, stream) in listener.incoming().enumerate() {
                let Ok(mut stream) = stream else { break };
                seen.lock().unwrap().push(read_head(&mut stream));
                match reply(index, &base) {
                    Reply::Status(code, body) => respond(&mut stream, code, &[], body.as_bytes()),
                    Reply::HangUp => {
                        let _ = stream.shutdown(Shutdown::Both);
                    }
                    Reply::Redirect(location) => {
                        respond(&mut stream, 307, &[("Location", &location)], b"")
                    }
                    Reply::Trickle(body, gap) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        for byte in body {
                            std::thread::sleep(gap);
                            if stream.write_all(&[*byte]).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });
        Canned { url, heads }
    }

    fn loopback_redirects(https_only: bool, hosts: &'static [&'static str]) -> HttpProfile {
        HttpProfile {
            redirect: RedirectRule {
                https_only,
                max_hops: 3,
                hosts,
            },
            ..HttpProfile::request()
        }
    }

    #[tokio::test]
    async fn retries_a_503_then_succeeds() {
        let server = serve(|n, _| {
            if n == 0 {
                Reply::Status(503, "busy")
            } else {
                Reply::Status(200, "ok")
            }
        });
        let client = reqwest::Client::new();
        let (status, body) = get_with_retry(
            &client,
            &format!("{}/x", server.url),
            "Probe",
            "item 1",
            None,
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"ok");
        assert_eq!(server.heads().len(), 2);
    }

    #[tokio::test]
    async fn a_dropped_connection_is_retried() {
        let server = serve(|n, _| {
            if n == 0 {
                Reply::HangUp
            } else {
                Reply::Status(200, "after drop")
            }
        });
        let client = reqwest::Client::new();
        let (status, body) = get_with_retry(
            &client,
            &format!("{}/x", server.url),
            "Probe",
            "item 1",
            None,
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"after drop");
        assert_eq!(server.heads().len(), 2);
    }

    #[tokio::test]
    async fn a_server_error_on_the_last_attempt_is_handed_back() {
        let server = serve(|_, _| Reply::Status(503, "still busy"));
        let client = reqwest::Client::new();
        let (status, body) = get_with_retry(
            &client,
            &format!("{}/x", server.url),
            "Probe",
            "item 1",
            None,
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, b"still busy");
        assert_eq!(server.heads().len(), 3);
    }

    #[tokio::test]
    async fn a_request_that_cannot_be_built_fails_once_naming_label_and_subject() {
        let client = reqwest::Client::new();
        let err = get_with_retry(&client, "not a url", "Probe", "item 1", None)
            .await
            .unwrap_err();
        let text = format!("{err:#}");
        assert!(
            text.starts_with("Probe request failed (item 1): "),
            "{text}"
        );
    }

    #[tokio::test]
    async fn the_referer_is_sent_only_when_given() {
        let server = serve(|_, _| Reply::Status(200, "ok"));
        let client = reqwest::Client::new();
        let url = format!("{}/x", server.url);
        get_with_retry(
            &client,
            &url,
            "Probe",
            "item 1",
            Some("https://nova.example/login"),
        )
        .await
        .unwrap();
        get_with_retry(&client, &url, "Probe", "item 2", None)
            .await
            .unwrap();
        let heads = server.heads();
        assert!(
            heads[0].contains("\r\nreferer: https://nova.example/login\r\n"),
            "{}",
            heads[0]
        );
        assert!(!heads[1].contains("referer:"), "{}", heads[1]);
    }

    #[test]
    fn the_request_profile_keeps_a_total_timeout_and_the_transfer_profile_has_none() {
        let request = HttpProfile::request();
        assert_eq!(request.total_timeout, Some(Duration::from_secs(300)));
        assert_eq!(request.connect_timeout, Duration::from_secs(30));
        let transfer = HttpProfile::transfer();
        assert_eq!(transfer.total_timeout, None);
        assert_eq!(transfer.connect_timeout, Duration::from_secs(30));
        assert_eq!(transfer.read_timeout, Duration::from_secs(60));
        for profile in [request, transfer] {
            assert_eq!(
                profile.user_agent,
                concat!("AstroBurst/", env!("CARGO_PKG_VERSION"))
            );
            assert!(profile.redirect.https_only);
            assert_eq!(profile.redirect.max_hops, 3);
            assert!(profile.redirect.hosts.is_empty());
        }
    }

    #[tokio::test]
    async fn the_built_client_sends_the_astroburst_user_agent() {
        let server = serve(|_, _| Reply::Status(200, "ok"));
        let client = build_client(&HttpProfile::transfer()).unwrap();
        client
            .get(format!("{}/x", server.url))
            .send()
            .await
            .unwrap();
        let expected = format!(
            "\r\nuser-agent: astroburst/{}\r\n",
            env!("CARGO_PKG_VERSION")
        );
        assert!(
            server.heads()[0].contains(&expected),
            "{}",
            server.heads()[0]
        );
    }

    #[tokio::test]
    async fn a_slow_steady_body_outlives_the_read_timeout_but_not_a_total_timeout() {
        let body: &'static [u8] = b"0123456789";
        let server = serve(move |_, _| Reply::Trickle(body, Duration::from_millis(100)));
        let url = format!("{}/x", server.url);
        let steady = HttpProfile {
            read_timeout: Duration::from_millis(500),
            total_timeout: None,
            ..HttpProfile::transfer()
        };
        let client = build_client(&steady).unwrap();
        let received = client
            .get(&url)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(&received[..], body);

        let bounded = HttpProfile {
            total_timeout: Some(Duration::from_millis(400)),
            ..steady
        };
        let client = build_client(&bounded).unwrap();
        let outcome = async { client.get(&url).send().await?.bytes().await }.await;
        assert!(outcome.unwrap_err().is_timeout());
    }

    #[tokio::test]
    async fn a_stalled_body_fails_on_the_read_timeout_without_a_total_timeout() {
        let server = serve(|_, _| Reply::Trickle(b"x", Duration::from_secs(2)));
        let stalled = HttpProfile {
            read_timeout: Duration::from_millis(200),
            ..HttpProfile::transfer()
        };
        let client = build_client(&stalled).unwrap();
        let started = std::time::Instant::now();
        let outcome = async {
            client
                .get(format!("{}/x", server.url))
                .send()
                .await?
                .bytes()
                .await
        }
        .await;
        assert!(outcome.unwrap_err().is_timeout());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the read timeout did not fire: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn a_redirect_to_a_listed_host_is_followed() {
        let target = serve(|_, _| Reply::Status(200, "final"));
        let location = format!("{}/final", target.url);
        let origin = serve(move |_, _| Reply::Redirect(location.clone()));
        let client = build_client(&loopback_redirects(false, &["127.0.0.1"])).unwrap();
        let response = client
            .get(format!("{}/start", origin.url))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(&response.bytes().await.unwrap()[..], b"final");
        assert!(
            target.heads()[0].starts_with("get /final "),
            "{}",
            target.heads()[0]
        );
    }

    #[tokio::test]
    async fn a_redirect_to_an_unlisted_host_is_refused() {
        let target = serve(|_, _| Reply::Status(200, "final"));
        let location = format!("{}/final", target.url);
        let origin = serve(move |_, _| Reply::Redirect(location.clone()));
        let client = build_client(&loopback_redirects(false, &["mast.stsci.edu"])).unwrap();
        let err = client
            .get(format!("{}/start", origin.url))
            .send()
            .await
            .unwrap_err();
        assert!(err.is_redirect(), "{err:?}");
        assert!(target.heads().is_empty());
    }

    #[tokio::test]
    async fn an_https_only_rule_refuses_a_plain_http_redirect() {
        let target = serve(|_, _| Reply::Status(200, "final"));
        let location = format!("{}/final", target.url);
        let origin = serve(move |_, _| Reply::Redirect(location.clone()));
        let client = build_client(&loopback_redirects(true, &["127.0.0.1"])).unwrap();
        let err = client
            .get(format!("{}/start", origin.url))
            .send()
            .await
            .unwrap_err();
        assert!(err.is_redirect(), "{err:?}");
        assert!(target.heads().is_empty());
    }

    #[tokio::test]
    async fn redirects_stop_after_the_hop_limit() {
        let origin = serve(|n, base| Reply::Redirect(format!("{base}/hop{}", n + 1)));
        let client = build_client(&loopback_redirects(false, &["127.0.0.1"])).unwrap();
        let err = client
            .get(format!("{}/hop0", origin.url))
            .send()
            .await
            .unwrap_err();
        assert!(err.is_redirect(), "{err:?}");
        assert_eq!(origin.heads().len(), 4);
    }
}
