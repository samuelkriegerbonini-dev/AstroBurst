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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NetKind {
    Refused,
    Unresolved,
    ConnectTimeout,
    Unreachable,
    Timeout,
    Closed,
    CutOff,
    Tls,
    Redirect,
    Other,
}

const DNS_FAILURE: &str = "dns error";
const CLOSED_EARLY: &str = "connection closed before message completed";
const CHECK_CONNECTION: &str = "check the internet connection or a firewall, then try again";
const TRY_LATER: &str = "try again in a few minutes";

fn causes(error: &reqwest::Error) -> Vec<&(dyn std::error::Error + 'static)> {
    std::iter::successors(std::error::Error::source(error), |cause| cause.source()).collect()
}

fn any_io_kind(causes: &[&(dyn std::error::Error + 'static)], kinds: &[std::io::ErrorKind]) -> bool {
    causes
        .iter()
        .any(|cause| cause.downcast_ref::<std::io::Error>().is_some_and(|io| kinds.contains(&io.kind())))
}

fn any_text(causes: &[&(dyn std::error::Error + 'static)], text: &str) -> bool {
    causes.iter().any(|cause| cause.to_string().starts_with(text))
}

pub(crate) fn net_kind(error: &reqwest::Error) -> NetKind {
    use std::io::ErrorKind;
    if error.is_redirect() {
        return NetKind::Redirect;
    }
    if error.is_timeout() {
        return if error.is_connect() { NetKind::ConnectTimeout } else { NetKind::Timeout };
    }
    let causes = causes(error);
    if any_io_kind(&causes, &[ErrorKind::ConnectionRefused]) {
        return NetKind::Refused;
    }
    if any_text(&causes, DNS_FAILURE) {
        return NetKind::Unresolved;
    }
    if any_text(&causes, CLOSED_EARLY)
        || any_io_kind(&causes, &[ErrorKind::ConnectionReset, ErrorKind::ConnectionAborted, ErrorKind::BrokenPipe])
    {
        return NetKind::Closed;
    }
    if error.is_decode() || error.is_body() {
        return NetKind::CutOff;
    }
    if error.is_connect() {
        return if any_io_kind(&causes, &[ErrorKind::InvalidData]) { NetKind::Tls } else { NetKind::Unreachable };
    }
    NetKind::Other
}

pub(crate) fn host_and_port(url: &reqwest::Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

pub(crate) fn net_predicate(kind: NetKind) -> &'static str {
    match kind {
        NetKind::Refused => "refused the connection",
        NetKind::Unresolved => "could not be found",
        NetKind::ConnectTimeout => "did not accept the connection in time",
        NetKind::Unreachable => "could not be reached",
        NetKind::Timeout => "did not finish answering in time",
        NetKind::Closed => "closed the connection without answering",
        NetKind::CutOff => "sent an incomplete answer",
        NetKind::Tls => "could not set up a secure connection",
        NetKind::Redirect => "redirected the request to an address that is not allowed",
        NetKind::Other => "did not complete the request",
    }
}

pub(crate) fn net_advice(kind: NetKind) -> &'static str {
    match kind {
        NetKind::Refused | NetKind::ConnectTimeout | NetKind::Unreachable => CHECK_CONNECTION,
        NetKind::Unresolved => "check the internet connection, then try again",
        NetKind::Tls => "check the computer's date and time and any proxy, then try again",
        NetKind::Timeout | NetKind::Closed | NetKind::CutOff | NetKind::Redirect | NetKind::Other => TRY_LATER,
    }
}

pub(crate) fn net_failure_text(kind: NetKind, host: &str) -> String {
    format!("{host} {}; {}", net_predicate(kind), net_advice(kind))
}

pub(crate) fn status_failure_text(code: u16, host: &str) -> String {
    format!("{host} answered HTTP {code}; {TRY_LATER}")
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

    use super::{
        build_client, get_with_retry, net_failure_text, net_kind, status_failure_text, HttpProfile, NetKind,
        RedirectRule,
    };

    enum Reply {
        Status(u16, &'static str),
        HangUp,
        Redirect(String),
        Trickle(&'static [u8], Duration),
        Stall(Duration),
        Truncated,
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
                    Reply::Stall(pause) => std::thread::sleep(pause),
                    Reply::Truncated => {
                        let _ = stream.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n0123456789",
                        );
                        let _ = stream.shutdown(Shutdown::Both);
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

    const EVERY_KIND: [NetKind; 10] = [
        NetKind::Refused,
        NetKind::Unresolved,
        NetKind::ConnectTimeout,
        NetKind::Unreachable,
        NetKind::Timeout,
        NetKind::Closed,
        NetKind::CutOff,
        NetKind::Tls,
        NetKind::Redirect,
        NetKind::Other,
    ];

    struct NoSuchHost;

    impl reqwest::dns::Resolve for NoSuchHost {
        fn resolve(&self, _name: reqwest::dns::Name) -> reqwest::dns::Resolving {
            Box::pin(async {
                Err::<reqwest::dns::Addrs, Box<dyn std::error::Error + Send + Sync>>(Box::new(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "this host is not known",
                )))
            })
        }
    }

    fn closed_port_url() -> String {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        format!("http://127.0.0.1:{port}/x")
    }

    #[tokio::test]
    async fn a_refused_connection_and_an_unknown_host_are_classified_without_the_os_text() {
        let refused = reqwest::Client::new().get(closed_port_url()).send().await.unwrap_err();
        assert_eq!(net_kind(&refused), NetKind::Refused, "{refused:?}");
        let resolver = reqwest::Client::builder().dns_resolver(Arc::new(NoSuchHost)).build().unwrap();
        let unknown = resolver.get("http://astroburst-no-such-host.invalid/x").send().await.unwrap_err();
        assert_eq!(net_kind(&unknown), NetKind::Unresolved, "{unknown:?}");
        assert_eq!(
            net_failure_text(NetKind::Refused, "127.0.0.1:9"),
            "127.0.0.1:9 refused the connection; check the internet connection or a firewall, then try again"
        );
    }

    #[test]
    fn every_failure_sentence_names_its_subject_once_and_ends_with_advice() {
        for kind in EVERY_KIND {
            let text = net_failure_text(kind, "zenodo.org");
            assert!(text.starts_with("zenodo.org "), "{kind:?}: {text}");
            assert_eq!(text.matches("zenodo.org").count(), 1, "{kind:?}: {text}");
            assert!(text.contains("; "), "{kind:?}: {text}");
            for banned in ["os error", "http://", "https://"] {
                assert!(!text.contains(banned), "{kind:?}: {text}");
            }
        }
        assert_eq!(status_failure_text(503, "zenodo.org"), "zenodo.org answered HTTP 503; try again in a few minutes");
        assert_eq!(
            net_failure_text(NetKind::Timeout, "zenodo.org"),
            "zenodo.org did not finish answering in time; try again in a few minutes"
        );
    }

    #[tokio::test]
    async fn hang_ups_timeouts_and_cut_bodies_are_classified_by_kind() {
        let hang_up = serve(|_, _| Reply::HangUp);
        let err = reqwest::Client::new().get(format!("{}/x", hang_up.url)).send().await.unwrap_err();
        assert_eq!(net_kind(&err), NetKind::Closed, "{err:?}");

        let bounded = build_client(&HttpProfile {
            total_timeout: Some(Duration::from_millis(300)),
            ..HttpProfile::request()
        })
        .unwrap();
        let silent = serve(|_, _| Reply::Stall(Duration::from_secs(2)));
        let err = bounded.get(format!("{}/x", silent.url)).send().await.unwrap_err();
        assert_eq!(net_kind(&err), NetKind::Timeout, "{err:?}");

        let stalled_body = serve(|_, _| Reply::Trickle(b"x", Duration::from_secs(2)));
        let err = async { bounded.get(format!("{}/x", stalled_body.url)).send().await?.bytes().await }
            .await
            .unwrap_err();
        assert_eq!(net_kind(&err), NetKind::Timeout, "{err:?}");

        let truncated = serve(|_, _| Reply::Truncated);
        let err = async { reqwest::Client::new().get(format!("{}/x", truncated.url)).send().await?.bytes().await }
            .await
            .unwrap_err();
        assert_eq!(net_kind(&err), NetKind::CutOff, "{err:?}");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_connect_that_outlasts_the_connect_timeout_is_classified_connect_timeout_where_windows_refuses_a_loopback_port_after_two_seconds() {
        let client = build_client(&HttpProfile {
            connect_timeout: Duration::from_millis(300),
            ..HttpProfile::request()
        })
        .unwrap();
        let err = client.get(closed_port_url()).send().await.unwrap_err();
        assert_eq!(
            net_kind(&err),
            NetKind::ConnectTimeout,
            "Windows reports a refused loopback connect only after about 2 s, so the 300 ms connect timeout fires first: {err:?}"
        );
    }
}
