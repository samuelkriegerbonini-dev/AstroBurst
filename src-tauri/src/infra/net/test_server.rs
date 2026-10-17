use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::StatusCode;

pub const LAST_MODIFIED: [&str; 2] = [
    "Wed, 14 Oct 2026 10:00:00 GMT",
    "Thu, 15 Oct 2026 10:00:00 GMT",
];
const SLOW_TICK: Duration = Duration::from_millis(50);
const SLOW_TICKS_PER_SECOND: u64 = 20;
const STALL: Duration = Duration::from_secs(30);
const WRITE_BLOCK: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub status: u16,
    pub body_bytes: u64,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Resource {
    bodies: [Vec<u8>; 2],
    change_after: Option<usize>,
    ignore_range: bool,
    etag: bool,
    weak_etag: bool,
    last_modified: bool,
    ignore_if_none_match: bool,
    drop_after: Option<u64>,
    stall_after: Option<u64>,
    flip_byte_at: Option<u64>,
    answer_416: bool,
    status: Option<u16>,
    bytes_per_s: Option<u64>,
    content_length: bool,
    segment_cap: Option<u64>,
    hang_up: bool,
}

impl Resource {
    pub fn new(body: Vec<u8>) -> Self {
        Self {
            bodies: [body.clone(), body],
            change_after: None,
            ignore_range: false,
            etag: true,
            weak_etag: false,
            last_modified: true,
            ignore_if_none_match: false,
            drop_after: None,
            stall_after: None,
            flip_byte_at: None,
            answer_416: false,
            status: None,
            bytes_per_s: None,
            content_length: true,
            segment_cap: None,
            hang_up: false,
        }
    }

    pub fn change_etag_after(mut self, requests: usize, body: Vec<u8>) -> Self {
        self.change_after = Some(requests);
        self.bodies[1] = body;
        self
    }

    pub fn ignore_range(mut self) -> Self {
        self.ignore_range = true;
        self
    }

    pub fn no_etag(mut self) -> Self {
        self.etag = false;
        self
    }

    pub fn weak_etag(mut self) -> Self {
        self.weak_etag = true;
        self
    }

    pub fn no_last_modified(mut self) -> Self {
        self.last_modified = false;
        self
    }

    pub fn ignore_if_none_match(mut self) -> Self {
        self.ignore_if_none_match = true;
        self
    }

    pub fn drop_after(mut self, bytes: u64) -> Self {
        self.drop_after = Some(bytes);
        self
    }

    pub fn stall_after(mut self, bytes: u64) -> Self {
        self.stall_after = Some(bytes);
        self
    }

    pub fn flip_byte_at(mut self, offset: u64) -> Self {
        self.flip_byte_at = Some(offset);
        self
    }

    pub fn answer_416(mut self) -> Self {
        self.answer_416 = true;
        self
    }

    pub fn status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn slow_steady(mut self, bytes_per_s: u64) -> Self {
        self.bytes_per_s = Some(bytes_per_s);
        self
    }

    pub fn no_content_length(mut self) -> Self {
        self.content_length = false;
        self
    }

    pub fn segment_cap(mut self, bytes: u64) -> Self {
        self.segment_cap = Some(bytes.max(1));
        self
    }

    pub fn hang_up(mut self) -> Self {
        self.hang_up = true;
        self
    }

    fn etag_of(&self, version: usize) -> Option<String> {
        self.etag.then(|| {
            let tag = format!("\"ab-v{version}\"");
            if self.weak_etag {
                format!("W/{tag}")
            } else {
                tag
            }
        })
    }

    fn served_body(&self, version: usize) -> Vec<u8> {
        let mut body = self.bodies[version].clone();
        if let Some(byte) = self
            .flip_byte_at
            .and_then(|offset| body.get_mut(offset as usize))
        {
            *byte ^= 0xFF;
        }
        body
    }

    fn version_of(&self, ordinal: usize) -> usize {
        usize::from(self.change_after.is_some_and(|after| ordinal >= after))
    }
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

struct Delivery {
    limit: usize,
    stall: bool,
}

impl Reply {
    fn empty(status: u16, headers: Vec<(String, String)>) -> Self {
        Self {
            status,
            headers,
            body: Vec::new(),
        }
    }
}

pub struct TestServer {
    pub base: String,
    log: Arc<Mutex<Vec<Recorded>>>,
}

impl TestServer {
    pub fn url(&self, path: &str) -> reqwest::Url {
        reqwest::Url::parse(&format!("{}{path}", self.base)).unwrap()
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.log.lock().unwrap().clone()
    }
}

#[derive(Default)]
struct Counters {
    requests: AtomicUsize,
    dropped: AtomicBool,
    stalled: AtomicBool,
}

pub fn serve(resource: Resource) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::new((resource, Counters::default(), Arc::clone(&log)));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let (resource, counters, log) = &*shared;
                handle(stream, resource, counters, log);
            });
        }
    });
    TestServer { base, log }
}

fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut data = Vec::new();
    let mut buffer = [0u8; 4096];
    while !data.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return None,
            Ok(read) => data.extend_from_slice(&buffer[..read]),
        }
    }
    Some(String::from_utf8_lossy(&data).into_owned())
}

fn handle(
    mut stream: TcpStream,
    resource: &Resource,
    counters: &Counters,
    log: &Mutex<Vec<Recorded>>,
) {
    let Some(head) = read_head(&mut stream) else {
        return;
    };
    let mut lines = head.lines();
    let target = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let ordinal = counters.requests.fetch_add(1, Ordering::SeqCst);
    let mut recorded = Recorded {
        target,
        headers,
        status: 0,
        body_bytes: 0,
    };
    if resource.hang_up {
        log.lock().unwrap().push(recorded);
        return;
    }
    let reply = plan(resource, resource.version_of(ordinal), &recorded);
    let delivery = deliver(resource, counters, &reply);
    recorded.status = reply.status;
    recorded.body_bytes = delivery.limit as u64;
    log.lock().unwrap().push(recorded);
    send(&mut stream, resource, &reply, &delivery);
}

fn parse_range(value: &str) -> Option<(u64, Option<u64>)> {
    let (start, end) = value.trim().strip_prefix("bytes=")?.split_once('-')?;
    let end = match end.trim() {
        "" => None,
        text => Some(text.parse().ok()?),
    };
    Some((start.trim().parse().ok()?, end))
}

fn opaque(tag: &str) -> &str {
    tag.strip_prefix("W/").unwrap_or(tag)
}

fn plan(resource: &Resource, version: usize, request: &Recorded) -> Reply {
    if let Some(status) = resource.status {
        return Reply::empty(status, Vec::new());
    }
    let body = resource.served_body(version);
    let len = body.len() as u64;
    let etag = resource.etag_of(version);
    let modified = resource.last_modified.then_some(LAST_MODIFIED[version]);
    let mut validators = Vec::new();
    if let Some(tag) = &etag {
        validators.push(("ETag".to_owned(), tag.clone()));
    }
    if let Some(date) = modified {
        validators.push(("Last-Modified".to_owned(), date.to_owned()));
    }
    if !resource.ignore_range {
        validators.push(("Accept-Ranges".to_owned(), "bytes".to_owned()));
    }
    if !resource.ignore_if_none_match {
        let not_modified = match (request.header("if-none-match"), &etag) {
            (Some(wanted), Some(tag)) => opaque(wanted) == opaque(tag),
            (Some(_), None) => false,
            (None, _) => request
                .header("if-modified-since")
                .is_some_and(|since| modified == Some(since)),
        };
        if not_modified {
            return Reply::empty(304, validators);
        }
    }
    let range = request
        .header("range")
        .filter(|_| !resource.ignore_range)
        .and_then(parse_range);
    if let Some((start, end)) = range {
        if resource.answer_416 || start >= len {
            validators.push(("Content-Range".to_owned(), format!("bytes */{len}")));
            return Reply::empty(416, validators);
        }
        let current = match request.header("if-range") {
            None => true,
            Some(value) if value.starts_with("W/") => false,
            Some(value) if value.starts_with('"') => etag.as_deref() == Some(value),
            Some(value) => modified == Some(value),
        };
        if current {
            let end = end.unwrap_or(len - 1).min(len - 1);
            let end = resource
                .segment_cap
                .map_or(end, |cap| end.min(start + cap - 1));
            validators.push((
                "Content-Range".to_owned(),
                format!("bytes {start}-{end}/{len}"),
            ));
            return Reply {
                status: 206,
                headers: validators,
                body: body[start as usize..=end as usize].to_vec(),
            };
        }
    }
    Reply {
        status: 200,
        headers: validators,
        body,
    }
}

fn deliver(resource: &Resource, counters: &Counters, reply: &Reply) -> Delivery {
    let has_body = !reply.body.is_empty();
    let cut = resource
        .drop_after
        .filter(|_| has_body && !counters.dropped.swap(true, Ordering::SeqCst));
    let stall = resource
        .stall_after
        .filter(|_| has_body && !counters.stalled.swap(true, Ordering::SeqCst));
    let limit = cut.or(stall).map_or(reply.body.len(), |bytes| {
        (bytes as usize).min(reply.body.len())
    });
    Delivery {
        limit,
        stall: stall.is_some(),
    }
}

fn send(stream: &mut TcpStream, resource: &Resource, reply: &Reply, delivery: &Delivery) {
    let reason = StatusCode::from_u16(reply.status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("Status");
    let mut head = format!(
        "HTTP/1.1 {} {reason}\r\nConnection: close\r\n",
        reply.status
    );
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if resource.content_length || reply.body.is_empty() {
        head.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
    }
    head.push_str("\r\n");
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    let limit = delivery.limit;
    let block = resource.bytes_per_s.map_or(WRITE_BLOCK, |rate| {
        ((rate / SLOW_TICKS_PER_SECOND).max(1)) as usize
    });
    let mut sent = 0usize;
    while sent < limit {
        let end = (sent + block).min(limit);
        if stream.write_all(&reply.body[sent..end]).is_err() {
            return;
        }
        sent = end;
        if resource.bytes_per_s.is_some() && sent < limit {
            std::thread::sleep(SLOW_TICK);
        }
    }
    if delivery.stall {
        std::thread::sleep(STALL);
    }
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
}
