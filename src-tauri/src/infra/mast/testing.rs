use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const MARKER_TOKEN: &str = "AB-TEST-TOKEN-0123456789abcdef";
const STALL_LIMIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub method: String,
    pub target: String,
    pub headers: String,
    pub body: String,
    pub aborted: bool,
}

impl Recorded {
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        let prefix = format!("{}:", name.to_ascii_lowercase());
        self.headers
            .lines()
            .find_map(|line| line.strip_prefix(prefix.as_str()).map(str::trim))
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            other => out.push(other),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn form_value(body: &str, key: &str) -> Option<String> {
    body.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (percent_decode(name) == key).then(|| percent_decode(value))
    })
}

#[derive(Clone)]
pub enum Body {
    Bytes(Vec<u8>),
    Oversized { total: usize, declare_length: bool, head: Vec<u8> },
    Stall { total: usize, head: Vec<u8> },
}

type Predicate = Arc<dyn Fn(&Recorded) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct Route {
    method: String,
    prefix: String,
    status: u16,
    headers: Vec<(String, String)>,
    body: Body,
    delay: Option<Duration>,
    close: bool,
    when: Option<Predicate>,
    times: Option<usize>,
    used: Arc<AtomicUsize>,
}

impl Route {
    pub fn new(method: &str, prefix: &str) -> Self {
        Self {
            method: method.to_owned(),
            prefix: prefix.to_owned(),
            status: 200,
            headers: Vec::new(),
            body: Body::Bytes(Vec::new()),
            delay: None,
            close: false,
            when: None,
            times: None,
            used: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn get(prefix: &str) -> Self {
        Self::new("GET", prefix)
    }

    pub fn post(prefix: &str) -> Self {
        Self::new("POST", prefix)
    }

    pub fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn json(self, body: impl Into<String>) -> Self {
        self.header("Content-Type", "application/json").bytes(body.into().into_bytes())
    }

    pub fn html(self, body: impl Into<String>) -> Self {
        self.header("Content-Type", "text/html").bytes(body.into().into_bytes())
    }

    pub fn bytes(mut self, body: Vec<u8>) -> Self {
        self.body = Body::Bytes(body);
        self
    }

    pub fn oversized(mut self, head: Vec<u8>, total: usize, declare_length: bool) -> Self {
        self.body = Body::Oversized { total, declare_length, head };
        self
    }

    pub fn stall(mut self, head: Vec<u8>, total: usize) -> Self {
        self.body = Body::Stall { total, head };
        self
    }

    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    pub fn close_without_answer(mut self) -> Self {
        self.close = true;
        self
    }

    pub fn when(mut self, predicate: impl Fn(&Recorded) -> bool + Send + Sync + 'static) -> Self {
        self.when = Some(Arc::new(predicate));
        self
    }

    pub fn times(mut self, times: usize) -> Self {
        self.times = Some(times);
        self
    }

    fn matches(&self, request: &Recorded) -> bool {
        if !self.method.eq_ignore_ascii_case(&request.method) || !request.target.starts_with(&self.prefix) {
            return false;
        }
        if let Some(predicate) = &self.when {
            if !predicate(request) {
                return false;
            }
        }
        match self.times {
            Some(limit) => self.used.load(Ordering::SeqCst) < limit,
            None => true,
        }
    }
}

pub struct TestServer {
    pub base: String,
    pub port: u16,
    log: Arc<Mutex<Vec<Recorded>>>,
    handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    pub fn requests(&self) -> Vec<Recorded> {
        self.log.lock().unwrap().clone()
    }

    pub fn count(&self, path_prefix: &str) -> usize {
        self.requests().iter().filter(|r| r.target.starts_with(path_prefix)).count()
    }

    pub async fn wait_for(&self, predicate: impl Fn(&[Recorded]) -> bool) -> Vec<Recorded> {
        for _ in 0..200 {
            let requests = self.requests();
            if predicate(&requests) {
                return requests;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.requests()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub async fn serve_routes(routes: Vec<Route>) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let routes = Arc::new(routes);
    let accept_log = Arc::clone(&log);
    let handle = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else { break };
            let routes = Arc::clone(&routes);
            let log = Arc::clone(&accept_log);
            tokio::spawn(async move {
                handle_connection(socket, &routes, &log).await;
            });
        }
    });
    TestServer { base: format!("http://{address}"), port: address.port(), log, handle }
}

async fn read_request(socket: &mut TcpStream) -> Option<Recorded> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok()).flatten()
        })
        .unwrap_or(0);
    while buffer.len() < header_end + content_length {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let mut request_line = head.lines().next().unwrap_or_default().split_whitespace();
    let headers = head
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':').map(|(n, v)| format!("{}: {}", n.trim().to_ascii_lowercase(), v.trim())))
        .collect::<Vec<_>>()
        .join("\n");
    Some(Recorded {
        method: request_line.next().unwrap_or_default().to_owned(),
        target: request_line.next().unwrap_or_default().to_owned(),
        headers,
        body: String::from_utf8_lossy(&buffer[header_end..]).to_string(),
        aborted: false,
    })
}

async fn wait_or_abort(socket: &mut TcpStream, delay: Duration) -> bool {
    let mut probe = [0u8; 64];
    let sleep = tokio::time::sleep(delay);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            _ = &mut sleep => return false,
            read = socket.read(&mut probe) => match read {
                Ok(0) | Err(_) => return true,
                Ok(_) => continue,
            },
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        302 => "Found",
        307 => "Temporary Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        504 => "Gateway Time-out",
        _ => "Status",
    }
}

async fn handle_connection(mut socket: TcpStream, routes: &[Route], log: &Mutex<Vec<Recorded>>) {
    let Some(mut request) = read_request(&mut socket).await else { return };
    let route = routes.iter().find(|route| route.matches(&request)).cloned();
    let index = {
        let mut log = log.lock().unwrap();
        log.push(request.clone());
        log.len() - 1
    };
    let Some(route) = route else {
        let _ = socket
            .write_all(b"HTTP/1.1 599 Unrouted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
        return;
    };
    route.used.fetch_add(1, Ordering::SeqCst);
    if let Some(delay) = route.delay {
        if wait_or_abort(&mut socket, delay).await {
            request.aborted = true;
            log.lock().unwrap()[index] = request;
            return;
        }
    }
    if route.close {
        let _ = socket.shutdown().await;
        return;
    }
    let mut head = format!("HTTP/1.1 {} {}\r\nConnection: close\r\n", route.status, reason(route.status));
    for (name, value) in &route.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    match &route.body {
        Body::Bytes(bytes) => {
            head.push_str(&format!("Content-Length: {}\r\n\r\n", bytes.len()));
            let mut response = head.into_bytes();
            response.extend_from_slice(bytes);
            let _ = socket.write_all(&response).await;
        }
        Body::Oversized { total, declare_length, head: first } => {
            if *declare_length {
                head.push_str(&format!("Content-Length: {total}\r\n\r\n"));
            } else {
                head.push_str("Transfer-Encoding: chunked\r\n\r\n");
            }
            if socket.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            let filler = vec![0x5Au8; 16 * 1024];
            let mut sent = 0usize;
            let mut first_block = true;
            while sent < *total {
                let block: &[u8] = if first_block && !first.is_empty() { first } else { &filler };
                first_block = false;
                let block = &block[..block.len().min(*total - sent)];
                let framed = if *declare_length {
                    block.to_vec()
                } else {
                    let mut framed = format!("{:x}\r\n", block.len()).into_bytes();
                    framed.extend_from_slice(block);
                    framed.extend_from_slice(b"\r\n");
                    framed
                };
                if socket.write_all(&framed).await.is_err() {
                    request.aborted = true;
                    log.lock().unwrap()[index] = request;
                    return;
                }
                sent += block.len();
            }
            if !*declare_length {
                let _ = socket.write_all(b"0\r\n\r\n").await;
            }
        }
        Body::Stall { total, head: first } => {
            head.push_str(&format!("Content-Length: {total}\r\n\r\n"));
            let mut response = head.into_bytes();
            response.extend_from_slice(first);
            if socket.write_all(&response).await.is_err() || wait_or_abort(&mut socket, STALL_LIMIT).await {
                request.aborted = true;
                log.lock().unwrap()[index] = request;
                return;
            }
        }
    }
    let _ = socket.shutdown().await;
}

pub fn synthetic_jpeg(width: u32, height: u32, rgb: bool, comment: Option<&str>) -> Vec<u8> {
    let image = if rgb {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(width, height, |x, y| image::Rgb([(x % 251) as u8, (y % 241) as u8, 90])))
    } else {
        image::DynamicImage::ImageLuma8(image::GrayImage::from_fn(width, height, |x, y| image::Luma([((x + y) % 253) as u8])))
    };
    let mut encoded = Vec::new();
    image
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 80))
        .unwrap();
    match comment {
        None => encoded,
        Some(text) => {
            let mut out = encoded[..2].to_vec();
            out.extend_from_slice(&[0xFF, 0xFE]);
            out.extend_from_slice(&((text.len() + 2) as u16).to_be_bytes());
            out.extend_from_slice(text.as_bytes());
            out.extend_from_slice(&encoded[2..]);
            out
        }
    }
}
