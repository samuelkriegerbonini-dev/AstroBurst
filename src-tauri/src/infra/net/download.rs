use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, CONTENT_RANGE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH,
    IF_RANGE, LAST_MODIFIED, RANGE,
};
use reqwest::{Client, RequestBuilder, Response, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::http::error_chain;

const FORMAT_VERSION: u32 = 1;
const BUFFER_BYTES: usize = 1 << 20;
const SIDECAR_EVERY: u64 = 8 << 20;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const CANCEL_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub url: Url,
    pub dest: PathBuf,
    pub expected_size: Option<u64>,
    pub expected_sha256: Option<[u8; 32]>,
    pub expected_etag: Option<String>,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub max_bytes: u64,
    pub overwrite: bool,
}

pub trait DownloadObserver: Sync {
    fn progress(&self, received: u64, total: Option<u64>);
    fn is_cancelled(&self) -> bool;
}

#[derive(Debug, Clone, Default)]
pub struct AtomicCancel(pub Arc<AtomicBool>);

impl DownloadObserver for AtomicCancel {
    fn progress(&self, _received: u64, _total: Option<u64>) {}

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOutcome {
    pub path: PathBuf,
    pub bytes: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub resumed_from: u64,
    pub sha256_hex: Option<String>,
    pub revalidated: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("download cancelled")]
    Cancelled,
    #[error("transfer failed: {0}")]
    Transport(String),
    #[error("server answered HTTP {status}")]
    Http { status: u16 },
    #[error("size mismatch: expected {expected} bytes, received {actual}")]
    SizeMismatch { expected: u64, actual: u64 },
    #[error("sha256 mismatch: expected {expected_hex}, received {actual_hex}")]
    ChecksumMismatch {
        expected_hex: String,
        actual_hex: String,
    },
    #[error("download larger than the {cap}-byte cap")]
    TooLarge { cap: u64 },
    #[error("server refused the requested byte range")]
    RangeNotSatisfiable,
    #[error("a download to this destination is already running")]
    Busy,
    #[error("destination exists and was not written by the download engine")]
    Exists,
    #[error(
        "local copy is stale: local ETag {}, remote ETag {}",
        .local_etag.as_deref().unwrap_or("none"),
        .remote_etag.as_deref().unwrap_or("none")
    )]
    Stale {
        local_etag: Option<String>,
        remote_etag: Option<String>,
    },
    #[error("download I/O failed: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Sidecar {
    version: u32,
    url: String,
    etag: Option<String>,
    last_modified: Option<String>,
    expected_size: Option<u64>,
    received: u64,
    started_unix_s: u64,
}

impl Sidecar {
    fn if_range(&self) -> Option<&str> {
        self.etag
            .as_deref()
            .filter(|etag| !etag.starts_with("W/"))
            .or(self.last_modified.as_deref())
    }

    fn validators_differ(&self, etag: &Option<String>, last_modified: &Option<String>) -> bool {
        match (etag, &self.etag) {
            (Some(remote), Some(local)) => remote != local,
            _ => matches!(
                (last_modified, &self.last_modified),
                (Some(remote), Some(local)) if remote != local
            ),
        }
    }

    fn finished(&self, url: &Url, len: u64) -> bool {
        self.version == FORMAT_VERSION
            && self.url == url.as_str()
            && self.received == len
            && self.expected_size == Some(len)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    version: u32,
    url: String,
    etag: Option<String>,
    last_modified: Option<String>,
    size: u64,
    sha256_hex: Option<String>,
    finished_unix_s: u64,
}

impl Record {
    fn describes(&self, url: &Url, len: u64) -> bool {
        self.version == FORMAT_VERSION && self.url == url.as_str() && self.size == len
    }

    fn same_version(
        &self,
        remote_etag: &Option<String>,
        remote_modified: &Option<String>,
    ) -> Option<bool> {
        match (&self.etag, remote_etag) {
            (Some(local), Some(remote)) => Some(local == remote),
            _ => match (&self.last_modified, remote_modified) {
                (Some(local), Some(remote)) => Some(local == remote),
                _ => None,
            },
        }
    }

    fn outcome(
        self,
        dest: &Path,
        sha256_hex: Option<String>,
        revalidated: bool,
    ) -> DownloadOutcome {
        DownloadOutcome {
            path: dest.to_path_buf(),
            bytes: self.size,
            etag: self.etag,
            last_modified: self.last_modified,
            resumed_from: self.size,
            sha256_hex,
            revalidated,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Files {
    pub(crate) dest: PathBuf,
    part: PathBuf,
    sidecar: PathBuf,
    record: PathBuf,
}

impl Files {
    pub(crate) fn recorded_sha256_hex(&self, size: u64) -> Option<String> {
        read_json::<Record>(&self.record)
            .filter(|record| record.version == FORMAT_VERSION && record.size == size)
            .and_then(|record| record.sha256_hex)
    }

    pub(crate) fn resumable_bytes(&self, expected_size: u64) -> Option<u64> {
        let part_len = fs::metadata(&self.part)
            .ok()
            .filter(|meta| meta.is_file())?
            .len();
        let sidecar = read_json::<Sidecar>(&self.sidecar)?;
        let offset = part_len.min(sidecar.received);
        (sidecar.version == FORMAT_VERSION
            && offset > 0
            && offset <= expected_size
            && sidecar
                .expected_size
                .is_none_or(|recorded| recorded == expected_size))
        .then_some(offset)
    }

    pub(crate) fn for_dest(dest: &Path) -> Self {
        let with = |suffix: &str| {
            let mut name = dest.as_os_str().to_owned();
            name.push(suffix);
            PathBuf::from(name)
        };
        Self {
            dest: dest.to_path_buf(),
            part: with(".part"),
            sidecar: with(".part.json"),
            record: with(".done.json"),
        }
    }

    fn discard_partial(&self) -> io::Result<()> {
        remove_if_present(&self.part)?;
        remove_if_present(&self.sidecar)
    }
}

struct Claim(PathBuf);

fn claims() -> &'static Mutex<HashSet<PathBuf>> {
    static CLAIMS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    CLAIMS.get_or_init(Mutex::default)
}

impl Claim {
    fn take(dest: &Path) -> Result<Self, DownloadError> {
        let key = claim_key(std::path::absolute(dest)?);
        let mut held = claims().lock().unwrap_or_else(PoisonError::into_inner);
        if held.insert(key.clone()) {
            Ok(Self(key))
        } else {
            Err(DownloadError::Busy)
        }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        claims()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.0);
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn claim_key(path: PathBuf) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn claim_key(path: PathBuf) -> PathBuf {
    path
}

async fn off_thread<T, F>(claim: &Arc<Claim>, work: F) -> Result<T, DownloadError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, DownloadError> + Send + 'static,
{
    let held = Arc::clone(claim);
    tokio::task::spawn_blocking(move || {
        let _held = held;
        work()
    })
    .await
    .map_err(|error| DownloadError::Io(io::Error::other(error)))?
}

struct Sink {
    file: File,
    pending: Vec<u8>,
    claim: Arc<Claim>,
}

impl Sink {
    async fn open(
        files: &Files,
        sidecar: Sidecar,
        offset: u64,
        claim: &Arc<Claim>,
    ) -> Result<Self, DownloadError> {
        let (part, path) = (files.part.clone(), files.sidecar.clone());
        let file = off_thread(claim, move || {
            write_json(&path, &sidecar)?;
            let file = if offset == 0 {
                File::create(&part)?
            } else {
                OpenOptions::new().append(true).open(&part)?
            };
            Ok(file)
        })
        .await?;
        Ok(Self {
            file,
            pending: Vec::with_capacity(BUFFER_BYTES),
            claim: Arc::clone(claim),
        })
    }

    async fn push(mut self, bytes: &[u8]) -> Result<Self, DownloadError> {
        self.pending.extend_from_slice(bytes);
        if self.pending.len() < BUFFER_BYTES {
            return Ok(self);
        }
        self.flush(None).await
    }

    async fn flush(self, checkpoint: Option<(PathBuf, Sidecar)>) -> Result<Self, DownloadError> {
        let Self {
            mut file,
            mut pending,
            claim,
        } = self;
        let (file, pending) = off_thread(&claim, move || {
            file.write_all(&pending)?;
            pending.clear();
            if let Some((path, sidecar)) = checkpoint {
                file.sync_data()?;
                write_json(&path, &sidecar)?;
            }
            Ok((file, pending))
        })
        .await?;
        Ok(Self {
            file,
            pending,
            claim,
        })
    }

    async fn suspend(self, files: &Files, sidecar: Sidecar, error: DownloadError) -> DownloadError {
        match self.flush(Some((files.sidecar.clone(), sidecar))).await {
            Ok(_) => error,
            Err(failed) => failed,
        }
    }
}

struct Resume {
    offset: u64,
    sidecar: Sidecar,
}

struct Start {
    offset: u64,
    total: Option<u64>,
    etag: Option<String>,
    last_modified: Option<String>,
    started_unix_s: u64,
}

enum Next {
    Stream(Start),
    Finalise(Resume),
    Restart,
}

struct Carry {
    offset: u64,
    resumed_from: u64,
    hasher: Option<Sha256>,
    throttle: Throttle,
}

enum Streamed {
    Done(DownloadOutcome),
    Short(Carry),
}

#[derive(Default)]
struct Throttle {
    last: Option<Instant>,
    reported: Option<u64>,
}

impl Throttle {
    fn tick(&mut self, observer: &dyn DownloadObserver, received: u64, total: Option<u64>) {
        if self.last.is_some_and(|at| at.elapsed() < PROGRESS_INTERVAL) {
            return;
        }
        self.emit(observer, received, total);
    }

    fn finish(&mut self, observer: &dyn DownloadObserver, received: u64, total: Option<u64>) {
        if self.reported != Some(received) {
            self.emit(observer, received, total);
        }
    }

    fn emit(&mut self, observer: &dyn DownloadObserver, received: u64, total: Option<u64>) {
        self.last = Some(Instant::now());
        self.reported = Some(received);
        observer.progress(received, total);
    }
}

pub async fn download(
    client: &Client,
    req: &DownloadRequest,
    observer: &dyn DownloadObserver,
) -> Result<DownloadOutcome, DownloadError> {
    let claim = Arc::new(Claim::take(&req.dest)?);
    let files = Files::for_dest(&req.dest);
    if let Some(parent) = req.dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    match fs::metadata(&files.dest) {
        Ok(meta) => {
            let record = match read_json::<Record>(&files.record) {
                Some(record) => Some(record),
                None => recover_record(&files, &req.url, meta.len())?,
            }
            .filter(|record| record.describes(&req.url, meta.len()));
            if let Some(record) = record {
                return reuse(client, req, &files, record, observer).await;
            }
            if !req.overwrite {
                return Err(DownloadError::Exists);
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    remove_if_present(&files.record)?;
    if observer.is_cancelled() {
        return Err(DownloadError::Cancelled);
    }
    let mut resume = resume_point(&files, req)?;
    let mut carry = None;
    loop {
        let response = send(ranged(client, req, resume.as_ref()), observer).await?;
        match accept(&response, resume.take())? {
            Next::Restart => carry = None,
            Next::Finalise(resume) => return finalise_part(req, &files, resume, &claim).await,
            Next::Stream(start) => {
                let streamed =
                    stream(response, start, req, &files, observer, carry.take(), &claim).await?;
                match streamed {
                    Streamed::Done(outcome) => return Ok(outcome),
                    Streamed::Short(next) => {
                        carry = Some(next);
                        resume = resume_point(&files, req)?;
                    }
                }
            }
        }
    }
}

fn recover_record(files: &Files, url: &Url, len: u64) -> io::Result<Option<Record>> {
    if files.record.exists() || files.part.exists() {
        return Ok(None);
    }
    let Some(sidecar) = read_json::<Sidecar>(&files.sidecar).filter(|s| s.finished(url, len))
    else {
        return Ok(None);
    };
    let record = Record {
        version: FORMAT_VERSION,
        url: sidecar.url,
        etag: sidecar.etag,
        last_modified: sidecar.last_modified,
        size: len,
        sha256_hex: None,
        finished_unix_s: unix_now(),
    };
    write_json(&files.record, &record)?;
    remove_if_present(&files.sidecar)?;
    Ok(Some(record))
}

fn resume_point(files: &Files, req: &DownloadRequest) -> io::Result<Option<Resume>> {
    let part_len = match fs::metadata(&files.part) {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(sidecar) = read_json::<Sidecar>(&files.sidecar) else {
        return Ok(None);
    };
    let offset = part_len.min(sidecar.received);
    let compatible = sidecar.version == FORMAT_VERSION
        && sidecar.url == req.url.as_str()
        && offset > 0
        && sidecar.expected_size.is_none_or(|size| offset <= size)
        && !matches!(
            (req.expected_size, sidecar.expected_size),
            (Some(wanted), Some(recorded)) if wanted != recorded
        )
        && req
            .expected_etag
            .as_ref()
            .is_none_or(|pinned| sidecar.etag.as_ref() == Some(pinned));
    if !compatible {
        return Ok(None);
    }
    if part_len > offset {
        OpenOptions::new()
            .write(true)
            .open(&files.part)?
            .set_len(offset)?;
    }
    Ok(Some(Resume { offset, sidecar }))
}

fn base(client: &Client, req: &DownloadRequest) -> RequestBuilder {
    req.headers
        .iter()
        .fold(client.get(req.url.clone()), |builder, (name, value)| {
            builder.header(name.clone(), value.clone())
        })
}

fn ranged(client: &Client, req: &DownloadRequest, resume: Option<&Resume>) -> RequestBuilder {
    let Some(resume) = resume else {
        return base(client, req);
    };
    let builder = base(client, req).header(RANGE, format!("bytes={}-", resume.offset));
    match resume.sidecar.if_range() {
        Some(validator) => builder.header(IF_RANGE, validator),
        None => builder,
    }
}

pub(crate) async fn send(
    builder: RequestBuilder,
    observer: &dyn DownloadObserver,
) -> Result<Response, DownloadError> {
    let sending = builder.send();
    tokio::pin!(sending);
    let mut poll = tokio::time::interval(CANCEL_POLL);
    loop {
        tokio::select! {
            sent = &mut sending => {
                return sent.map_err(|error| DownloadError::Transport(error_chain(&error)));
            }
            _ = poll.tick() => {
                if observer.is_cancelled() {
                    return Err(DownloadError::Cancelled);
                }
            }
        }
    }
}

fn accept(response: &Response, resume: Option<Resume>) -> Result<Next, DownloadError> {
    let headers = response.headers();
    let etag = header_text(headers, &ETAG);
    let last_modified = header_text(headers, &LAST_MODIFIED);
    match (response.status(), resume) {
        (StatusCode::OK, _) => Ok(Next::Stream(Start {
            offset: 0,
            total: response.content_length(),
            etag,
            last_modified,
            started_unix_s: unix_now(),
        })),
        (StatusCode::PARTIAL_CONTENT, Some(resume)) => {
            let Some((start, total)) = content_range(headers) else {
                return Ok(Next::Restart);
            };
            let total = total.or_else(|| response.content_length().map(|n| resume.offset + n));
            let etag_changed = matches!(
                (&etag, &resume.sidecar.etag),
                (Some(remote), Some(local)) if remote != local
            );
            let unvalidated_total_differs = resume.sidecar.if_range().is_none()
                && (total.is_none() || total != resume.sidecar.expected_size);
            if start != resume.offset || etag_changed || unvalidated_total_differs {
                return Ok(Next::Restart);
            }
            Ok(Next::Stream(Start {
                offset: resume.offset,
                total,
                etag: etag.or(resume.sidecar.etag),
                last_modified: last_modified.or(resume.sidecar.last_modified),
                started_unix_s: resume.sidecar.started_unix_s,
            }))
        }
        (StatusCode::RANGE_NOT_SATISFIABLE, Some(resume)) => {
            let complete = Some(resume.offset) == resume.sidecar.expected_size
                && complete_length(headers).is_none_or(|total| total == resume.offset)
                && !resume.sidecar.validators_differ(&etag, &last_modified);
            Ok(if complete {
                Next::Finalise(resume)
            } else {
                Next::Restart
            })
        }
        (StatusCode::RANGE_NOT_SATISFIABLE, None) => Err(DownloadError::RangeNotSatisfiable),
        (status, _) => Err(DownloadError::Http {
            status: status.as_u16(),
        }),
    }
}

fn admit(start: &Start, req: &DownloadRequest) -> Result<Option<u64>, DownloadError> {
    pinned(req, &start.etag)?;
    if let Some(total) = start.total {
        if total > req.max_bytes {
            return Err(DownloadError::TooLarge { cap: req.max_bytes });
        }
        if let Some(expected) = req.expected_size.filter(|&expected| expected != total) {
            return Err(DownloadError::SizeMismatch {
                expected,
                actual: total,
            });
        }
    }
    Ok(req.expected_size.or(start.total))
}

async fn stream(
    mut response: Response,
    start: Start,
    req: &DownloadRequest,
    files: &Files,
    observer: &dyn DownloadObserver,
    carry: Option<Carry>,
    claim: &Arc<Claim>,
) -> Result<Streamed, DownloadError> {
    let expected = match admit(&start, req) {
        Ok(expected) => expected,
        Err(error) => {
            if matches!(
                error,
                DownloadError::TooLarge { .. } | DownloadError::SizeMismatch { .. }
            ) {
                files.discard_partial()?;
            }
            return Err(error);
        }
    };
    let mut sidecar = Sidecar {
        version: FORMAT_VERSION,
        url: req.url.to_string(),
        etag: start.etag.clone(),
        last_modified: start.last_modified.clone(),
        expected_size: expected,
        received: start.offset,
        started_unix_s: start.started_unix_s,
    };
    let mut sink = Sink::open(files, sidecar.clone(), start.offset, claim).await?;
    let (resumed_from, mut throttle, seeded) = match carry.filter(|c| c.offset == start.offset) {
        Some(carry) => (carry.resumed_from, carry.throttle, carry.hasher),
        None => (start.offset, Throttle::default(), None),
    };
    let mut hasher = match (req.expected_sha256, seeded) {
        (None, _) => None,
        (Some(_), Some(hasher)) => Some(hasher),
        (Some(_), None) => Some(hash_prefix(&files.part, start.offset).await?),
    };
    let mut received = start.offset;
    let mut checkpoint = start.offset + SIDECAR_EVERY;
    throttle.tick(observer, received, expected);
    let mut poll = tokio::time::interval(CANCEL_POLL);
    loop {
        if observer.is_cancelled() {
            let sidecar = Sidecar {
                received,
                ..sidecar
            };
            return Err(sink.suspend(files, sidecar, DownloadError::Cancelled).await);
        }
        let chunk = tokio::select! {
            chunk = response.chunk() => chunk,
            _ = poll.tick() => continue,
        };
        let bytes = match chunk {
            Ok(Some(bytes)) => bytes,
            Ok(None) => break,
            Err(error) => {
                let error = DownloadError::Transport(error_chain(&error));
                let sidecar = Sidecar {
                    received,
                    ..sidecar
                };
                return Err(sink.suspend(files, sidecar, error).await);
            }
        };
        received += bytes.len() as u64;
        if received > req.max_bytes {
            drop(sink);
            files.discard_partial()?;
            return Err(DownloadError::TooLarge { cap: req.max_bytes });
        }
        if let Some(expected) = expected.filter(|&expected| received > expected) {
            drop(sink);
            files.discard_partial()?;
            return Err(DownloadError::SizeMismatch {
                expected,
                actual: received,
            });
        }
        if let Some(hasher) = hasher.as_mut() {
            hasher.update(&bytes);
        }
        sink = sink.push(&bytes).await?;
        if received >= checkpoint {
            sidecar.received = received;
            sink = sink
                .flush(Some((files.sidecar.clone(), sidecar.clone())))
                .await?;
            checkpoint = received + SIDECAR_EVERY;
        }
        throttle.tick(observer, received, expected);
    }
    sidecar.received = received;
    let short = start.offset > 0 && expected.is_some_and(|expected| received < expected);
    if short && received == start.offset {
        let error = DownloadError::Transport(format!(
            "the partial response from byte {received} ended without a body"
        ));
        return Err(sink.suspend(files, sidecar, error).await);
    }
    if short {
        sink.flush(Some((files.sidecar.clone(), sidecar))).await?;
        return Ok(Streamed::Short(Carry {
            offset: received,
            resumed_from,
            hasher,
            throttle,
        }));
    }
    let sink = sink.flush(None).await?;
    throttle.finish(observer, received, expected);
    finish(sink.file, hasher, sidecar, resumed_from, req, files, claim)
        .await
        .map(Streamed::Done)
}

async fn finalise_part(
    req: &DownloadRequest,
    files: &Files,
    resume: Resume,
    claim: &Arc<Claim>,
) -> Result<DownloadOutcome, DownloadError> {
    pinned(req, &resume.sidecar.etag)?;
    if resume.offset > req.max_bytes {
        files.discard_partial()?;
        return Err(DownloadError::TooLarge { cap: req.max_bytes });
    }
    let hasher = match req.expected_sha256 {
        Some(_) => Some(hash_prefix(&files.part, resume.offset).await?),
        None => None,
    };
    let file = OpenOptions::new().append(true).open(&files.part)?;
    let Resume { offset, sidecar } = resume;
    let sidecar = Sidecar {
        received: offset,
        ..sidecar
    };
    finish(file, hasher, sidecar, offset, req, files, claim).await
}

async fn finish(
    file: File,
    hasher: Option<Sha256>,
    sidecar: Sidecar,
    resumed_from: u64,
    req: &DownloadRequest,
    files: &Files,
    claim: &Arc<Claim>,
) -> Result<DownloadOutcome, DownloadError> {
    let expected_sha256 = req.expected_sha256;
    let files = files.clone();
    off_thread(claim, move || {
        seal(file, hasher, sidecar, resumed_from, expected_sha256, &files)
    })
    .await
}

fn seal(
    file: File,
    hasher: Option<Sha256>,
    sidecar: Sidecar,
    resumed_from: u64,
    expected_sha256: Option<[u8; 32]>,
    files: &Files,
) -> Result<DownloadOutcome, DownloadError> {
    let received = sidecar.received;
    if let Some(expected) = sidecar
        .expected_size
        .filter(|&expected| expected != received)
    {
        drop(file);
        files.discard_partial()?;
        return Err(DownloadError::SizeMismatch {
            expected,
            actual: received,
        });
    }
    let sha256_hex = hasher.map(|hasher| hex(&hasher.finalize()));
    if let (Some(wanted), Some(actual_hex)) = (expected_sha256, &sha256_hex) {
        let expected_hex = hex(&wanted);
        if &expected_hex != actual_hex {
            drop(file);
            files.discard_partial()?;
            return Err(DownloadError::ChecksumMismatch {
                expected_hex,
                actual_hex: actual_hex.clone(),
            });
        }
    }
    file.sync_all()?;
    drop(file);
    write_json(&files.sidecar, &sidecar)?;
    fs::rename(&files.part, &files.dest)?;
    let record = Record {
        version: FORMAT_VERSION,
        url: sidecar.url,
        etag: sidecar.etag,
        last_modified: sidecar.last_modified,
        size: received,
        sha256_hex,
        finished_unix_s: unix_now(),
    };
    write_json(&files.record, &record)?;
    remove_if_present(&files.sidecar)?;
    Ok(DownloadOutcome {
        path: files.dest.clone(),
        bytes: received,
        etag: record.etag,
        last_modified: record.last_modified,
        resumed_from,
        sha256_hex: record.sha256_hex,
        revalidated: false,
    })
}

async fn reuse(
    client: &Client,
    req: &DownloadRequest,
    files: &Files,
    record: Record,
    observer: &dyn DownloadObserver,
) -> Result<DownloadOutcome, DownloadError> {
    if req.expected_size.is_some_and(|size| size != record.size) {
        return Err(DownloadError::Stale {
            local_etag: record.etag,
            remote_etag: None,
        });
    }
    let sha256_hex = match req.expected_sha256 {
        None => record.sha256_hex.clone(),
        Some(wanted) => {
            let actual_hex = match &record.sha256_hex {
                Some(known) => known.clone(),
                None => hex(&hash_prefix(&files.dest, record.size).await?.finalize()),
            };
            let expected_hex = hex(&wanted);
            if expected_hex != actual_hex {
                return Err(DownloadError::ChecksumMismatch {
                    expected_hex,
                    actual_hex,
                });
            }
            Some(actual_hex)
        }
    };
    let probe = base(client, req).header(RANGE, "bytes=0-0");
    let probe = match (&record.etag, &record.last_modified) {
        (Some(etag), _) => probe.header(IF_NONE_MATCH, etag.as_str()),
        (None, Some(date)) => probe.header(IF_MODIFIED_SINCE, date.as_str()),
        (None, None) => return unrevalidated(req, record, files, sha256_hex),
    };
    let response = match send(probe, observer).await {
        Ok(response) => response,
        Err(DownloadError::Cancelled) => return Err(DownloadError::Cancelled),
        Err(_) => return unrevalidated(req, record, files, sha256_hex),
    };
    let remote_etag = header_text(response.headers(), &ETAG);
    let remote_modified = header_text(response.headers(), &LAST_MODIFIED);
    match response.status() {
        StatusCode::NOT_MODIFIED => {}
        StatusCode::OK | StatusCode::PARTIAL_CONTENT => {
            match record.same_version(&remote_etag, &remote_modified) {
                Some(true) => {}
                Some(false) => {
                    return Err(DownloadError::Stale {
                        local_etag: record.etag,
                        remote_etag,
                    })
                }
                None => return unrevalidated(req, record, files, sha256_hex),
            }
        }
        status if status.is_server_error() => return unrevalidated(req, record, files, sha256_hex),
        status => {
            return Err(DownloadError::Http {
                status: status.as_u16(),
            })
        }
    }
    drop(response);
    pinned(req, &remote_etag.or_else(|| record.etag.clone()))?;
    Ok(record.outcome(&files.dest, sha256_hex, true))
}

fn unrevalidated(
    req: &DownloadRequest,
    record: Record,
    files: &Files,
    sha256_hex: Option<String>,
) -> Result<DownloadOutcome, DownloadError> {
    pinned(req, &record.etag)?;
    Ok(record.outcome(&files.dest, sha256_hex, false))
}

fn pinned(req: &DownloadRequest, remote_etag: &Option<String>) -> Result<(), DownloadError> {
    match &req.expected_etag {
        Some(expected) if remote_etag.as_ref() != Some(expected) => Err(DownloadError::Stale {
            local_etag: Some(expected.clone()),
            remote_etag: remote_etag.clone(),
        }),
        _ => Ok(()),
    }
}

async fn hash_prefix(path: &Path, len: u64) -> io::Result<Sha256> {
    if len == 0 {
        return Ok(Sha256::new());
    }
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut reader = File::open(&path)?.take(len);
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; BUFFER_BYTES];
        let mut seen = 0u64;
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            seen += read as u64;
        }
        if seen != len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("{} holds {seen} bytes, {len} expected", path.display()),
            ));
        }
        Ok(hasher)
    })
    .await
    .map_err(io::Error::other)?
}

fn header_text(headers: &HeaderMap, name: &HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn content_range(headers: &HeaderMap) -> Option<(u64, Option<u64>)> {
    let text = headers.get(CONTENT_RANGE)?.to_str().ok()?;
    let (span, total) = text.trim().strip_prefix("bytes ")?.split_once('/')?;
    let (start, _) = span.split_once('-')?;
    let total = match total.trim() {
        "*" => None,
        digits => Some(digits.parse().ok()?),
    };
    Some((start.trim().parse().ok()?, total))
}

fn complete_length(headers: &HeaderMap) -> Option<u64> {
    let text = headers.get(CONTENT_RANGE)?.to_str().ok()?;
    let (_, total) = text.trim().strip_prefix("bytes ")?.split_once('/')?;
    total.trim().parse().ok()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    fs::write(path, serde_json::to_vec(value).map_err(io::Error::other)?)
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sha2::{Digest, Sha256};

    use super::super::http::{build_client, HttpProfile};
    use super::super::test_server::{serve, Resource, TestServer, LAST_MODIFIED};
    use super::{
        download, AtomicCancel, Claim, DownloadError, DownloadObserver, DownloadOutcome,
        DownloadRequest, Files, Record, Sidecar,
    };

    const KIB: u64 = 1 << 10;
    const MIB: u64 = 1 << 20;
    const SIZE: u64 = 3 * MIB;
    const ETAG_V0: &str = "\"ab-v0\"";
    const ETAG_V1: &str = "\"ab-v1\"";

    fn body(len: u64, seed: u64) -> Vec<u8> {
        (0..len)
            .map(|i| ((i * 31 + i / 4099 + seed) % 251) as u8)
            .collect()
    }

    fn digest(bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }

    fn hex_of(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn flipped(bytes: &[u8], at: u64) -> Vec<u8> {
        let mut out = bytes.to_vec();
        out[at as usize] ^= 0xFF;
        out
    }

    fn client() -> reqwest::Client {
        build_client(&HttpProfile::transfer()).unwrap()
    }

    fn client_with_read_timeout(read_timeout: Duration) -> reqwest::Client {
        build_client(&HttpProfile {
            read_timeout,
            ..HttpProfile::transfer()
        })
        .unwrap()
    }

    struct Spot {
        _dir: tempfile::TempDir,
        files: Files,
    }

    fn spot() -> Spot {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("JWST").join("2739").join("product.fits");
        Spot {
            files: Files::for_dest(&dest),
            _dir: dir,
        }
    }

    fn request(server: &TestServer, spot: &Spot) -> DownloadRequest {
        DownloadRequest {
            url: server.url("/api/v0.1/Download/file?uri=mast:JWST/product/product.fits"),
            dest: spot.files.dest.clone(),
            expected_size: None,
            expected_sha256: None,
            expected_etag: None,
            headers: Vec::new(),
            max_bytes: 20 << 30,
            overwrite: false,
        }
    }

    fn file_digest(path: &Path) -> [u8; 32] {
        digest(&fs::read(path).unwrap())
    }

    fn left_behind(files: &Files) -> [bool; 4] {
        [
            files.dest.exists(),
            files.part.exists(),
            files.sidecar.exists(),
            files.record.exists(),
        ]
    }

    const ONLY_DEST_AND_RECORD: [bool; 4] = [true, false, false, true];
    const NOTHING: [bool; 4] = [false, false, false, false];

    fn read_sidecar(files: &Files) -> Sidecar {
        serde_json::from_slice(&fs::read(&files.sidecar).unwrap()).unwrap()
    }

    fn read_record(files: &Files) -> Record {
        serde_json::from_slice(&fs::read(&files.record).unwrap()).unwrap()
    }

    fn sidecar_for(req: &DownloadRequest, expected_size: u64, received: u64) -> Sidecar {
        Sidecar {
            version: 1,
            url: req.url.to_string(),
            etag: Some(ETAG_V0.to_owned()),
            last_modified: Some(LAST_MODIFIED[0].to_owned()),
            expected_size: Some(expected_size),
            received,
            started_unix_s: 1,
        }
    }

    fn write_sidecar(files: &Files, sidecar: &Sidecar) {
        fs::write(&files.sidecar, serde_json::to_vec(sidecar).unwrap()).unwrap();
    }

    fn edit_record(files: &Files, edit: impl FnOnce(&mut Record)) {
        let mut record = read_record(files);
        edit(&mut record);
        fs::write(&files.record, serde_json::to_vec(&record).unwrap()).unwrap();
    }

    fn plant_part_at(
        spot: &Spot,
        req: &DownloadRequest,
        bytes: &[u8],
        expected_size: u64,
        received: u64,
    ) {
        fs::create_dir_all(spot.files.dest.parent().unwrap()).unwrap();
        fs::write(&spot.files.part, bytes).unwrap();
        write_sidecar(&spot.files, &sidecar_for(req, expected_size, received));
    }

    fn plant_part(spot: &Spot, req: &DownloadRequest, bytes: &[u8], expected_size: u64) {
        plant_part_at(spot, req, bytes, expected_size, bytes.len() as u64);
    }

    struct Check {
        part_len: Option<u64>,
        sidecar: bool,
    }

    #[derive(Default)]
    struct Watcher {
        cancel_at: Option<u64>,
        watched: Option<(PathBuf, PathBuf)>,
        calls: Mutex<Vec<(u64, Option<u64>, Instant)>>,
        checks: Mutex<Vec<Check>>,
        sidecar_received: Mutex<Vec<u64>>,
        cancelled: AtomicBool,
    }

    impl Watcher {
        fn watching(files: &Files) -> Self {
            Self {
                watched: Some((files.part.clone(), files.sidecar.clone())),
                ..Self::default()
            }
        }

        fn cancelling_at(received: u64) -> Self {
            Self {
                cancel_at: Some(received),
                ..Self::default()
            }
        }
    }

    impl DownloadObserver for Watcher {
        fn progress(&self, received: u64, total: Option<u64>) {
            self.calls
                .lock()
                .unwrap()
                .push((received, total, Instant::now()));
            if let Some((_, sidecar)) = &self.watched {
                let parsed = fs::read(sidecar)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Sidecar>(&bytes).ok());
                if let Some(sidecar) = parsed {
                    self.sidecar_received.lock().unwrap().push(sidecar.received);
                }
            }
            if self.cancel_at.is_some_and(|at| received >= at) {
                self.cancelled.store(true, Ordering::SeqCst);
            }
        }

        fn is_cancelled(&self) -> bool {
            if let Some((part, sidecar)) = &self.watched {
                self.checks.lock().unwrap().push(Check {
                    part_len: fs::metadata(part).ok().map(|meta| meta.len()),
                    sidecar: sidecar.exists(),
                });
            }
            self.cancelled.load(Ordering::SeqCst)
        }
    }

    async fn fetch(req: &DownloadRequest) -> Result<DownloadOutcome, DownloadError> {
        download(&client(), req, &AtomicCancel::default()).await
    }

    #[tokio::test]
    async fn t01_a_full_download_lands_atomically_with_a_completion_record() {
        let payload = body(SIZE, 1);
        let server = serve(Resource::new(payload.clone()));
        let spot = spot();
        let req = request(&server, &spot);
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        assert_eq!(left_behind(&spot.files), ONLY_DEST_AND_RECORD);
        assert_eq!(
            outcome,
            DownloadOutcome {
                path: spot.files.dest.clone(),
                bytes: SIZE,
                etag: Some(ETAG_V0.to_owned()),
                last_modified: Some(LAST_MODIFIED[0].to_owned()),
                resumed_from: 0,
                sha256_hex: None,
                revalidated: false,
            }
        );
        let record = read_record(&spot.files);
        assert_eq!(record.version, 1);
        assert_eq!(record.url, req.url.as_str());
        assert_eq!(record.size, SIZE);
        assert_eq!(record.etag.as_deref(), Some(ETAG_V0));
        assert_eq!(record.last_modified.as_deref(), Some(LAST_MODIFIED[0]));
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].header("range"), None);
    }

    #[tokio::test]
    async fn t02_a_dropped_transfer_resumes_with_range_and_a_strong_if_range() {
        let payload = body(SIZE, 2);
        let server = serve(Resource::new(payload.clone()).drop_after(MIB));
        let spot = spot();
        let req = request(&server, &spot);
        let watcher = Watcher::watching(&spot.files);
        let first = download(&client(), &req, &watcher).await.unwrap_err();
        assert!(matches!(first, DownloadError::Transport(_)), "{first:?}");
        assert_eq!(fs::metadata(&spot.files.part).unwrap().len(), MIB);
        assert_eq!(read_sidecar(&spot.files).received, MIB);
        {
            let checks = watcher.checks.lock().unwrap();
            let opened: Vec<&Check> = checks.iter().filter(|c| c.part_len.is_some()).collect();
            assert!(!opened.is_empty());
            assert_eq!(opened[0].part_len, Some(0));
            assert!(opened.iter().all(|check| check.sidecar));
        }
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, MIB);
        assert_eq!(outcome.bytes, SIZE);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        assert_eq!(left_behind(&spot.files), ONLY_DEST_AND_RECORD);
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].header("range"), Some("bytes=1048576-"));
        assert_eq!(requests[1].header("if-range"), Some(ETAG_V0));
        assert_eq!(requests[1].status, 206);
    }

    #[tokio::test]
    async fn t03_an_etag_change_between_attempts_restarts_from_zero() {
        let old = body(SIZE, 3);
        let new = body(SIZE - 4 * KIB, 4);
        let server = serve(
            Resource::new(old)
                .drop_after(MIB)
                .change_etag_after(1, new.clone()),
        );
        let spot = spot();
        let req = request(&server, &spot);
        assert!(matches!(
            fetch(&req).await,
            Err(DownloadError::Transport(_))
        ));
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(outcome.bytes, SIZE - 4 * KIB);
        assert_eq!(outcome.etag.as_deref(), Some(ETAG_V1));
        assert_eq!(file_digest(&spot.files.dest), digest(&new));
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].header("if-range"), Some(ETAG_V0));
        assert_eq!(requests[1].status, 200);
    }

    #[tokio::test]
    async fn t04_a_range_answered_200_without_validators_restarts_and_completes() {
        let payload = body(SIZE, 5);
        let server = serve(
            Resource::new(payload.clone())
                .ignore_range()
                .no_etag()
                .no_last_modified()
                .drop_after(MIB),
        );
        let spot = spot();
        let req = request(&server, &spot);
        assert!(matches!(
            fetch(&req).await,
            Err(DownloadError::Transport(_))
        ));
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(outcome.etag, None);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].header("range"), Some("bytes=1048576-"));
        assert_eq!(requests[1].header("if-range"), None);
        assert_eq!(requests[1].status, 200);
    }

    #[tokio::test]
    async fn t04b_without_validators_a_206_with_the_expected_total_is_appended() {
        let payload = body(SIZE, 6);
        let server = serve(
            Resource::new(payload.clone())
                .no_etag()
                .no_last_modified()
                .drop_after(MIB),
        );
        let spot = spot();
        let req = request(&server, &spot);
        assert!(fetch(&req).await.is_err());
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, MIB);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        let requests = server.requests();
        assert_eq!(requests[1].header("if-range"), None);
        assert_eq!(requests[1].status, 206);
    }

    #[tokio::test]
    async fn t04c_without_validators_a_206_with_another_total_restarts_without_range() {
        let old = body(SIZE, 7);
        let new = body(2 * MIB, 8);
        let server = serve(
            Resource::new(old)
                .no_etag()
                .no_last_modified()
                .drop_after(MIB)
                .change_etag_after(1, new.clone()),
        );
        let spot = spot();
        let req = request(&server, &spot);
        assert!(fetch(&req).await.is_err());
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(outcome.bytes, 2 * MIB);
        assert_eq!(file_digest(&spot.files.dest), digest(&new));
        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[1].status, 206);
        assert_eq!(requests[2].header("range"), None);
        assert_eq!(requests[2].status, 200);
    }

    #[tokio::test]
    async fn t05_a_flipped_byte_is_refused_naming_both_digests() {
        let payload = body(SIZE, 9);
        let at = 2 * MIB + 17;
        let server = serve(Resource::new(payload.clone()).flip_byte_at(at));
        let spot = spot();
        let mut req = request(&server, &spot);
        req.expected_sha256 = Some(digest(&payload));
        match fetch(&req).await.unwrap_err() {
            DownloadError::ChecksumMismatch {
                expected_hex,
                actual_hex,
            } => {
                assert_eq!(expected_hex, hex_of(&digest(&payload)));
                assert_eq!(actual_hex, hex_of(&digest(&flipped(&payload, at))));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(left_behind(&spot.files), NOTHING);
    }

    #[tokio::test]
    async fn t06_an_expected_size_against_another_content_length_fails_before_any_byte() {
        let server = serve(Resource::new(body(SIZE, 10)));
        let spot = spot();
        let mut req = request(&server, &spot);
        req.expected_size = Some(4 * MIB);
        let watcher = Watcher::watching(&spot.files);
        let error = download(&client(), &req, &watcher).await.unwrap_err();
        assert!(
            matches!(
                error,
                DownloadError::SizeMismatch {
                    expected,
                    actual
                } if expected == 4 * MIB && actual == SIZE
            ),
            "{error:?}"
        );
        assert!(watcher
            .checks
            .lock()
            .unwrap()
            .iter()
            .all(|check| check.part_len.is_none()));
        assert_eq!(left_behind(&spot.files), NOTHING);
    }

    #[tokio::test]
    async fn t07_a_cancelled_transfer_keeps_its_part_and_resumes() {
        let payload = body(SIZE, 11);
        let server = serve(Resource::new(payload.clone()).slow_steady(4 * MIB));
        let spot = spot();
        let req = request(&server, &spot);
        let watcher = Watcher::cancelling_at(SIZE / 2);
        let error = download(&client(), &req, &watcher).await.unwrap_err();
        assert!(matches!(error, DownloadError::Cancelled), "{error:?}");
        let kept = fs::metadata(&spot.files.part).unwrap().len();
        assert!((SIZE / 2..SIZE).contains(&kept), "{kept}");
        assert_eq!(read_sidecar(&spot.files).received, kept);
        assert!(!spot.files.dest.exists());
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, kept);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
    }

    #[tokio::test]
    async fn t08_a_declared_length_above_max_bytes_is_refused_leaving_nothing() {
        let server = serve(Resource::new(body(SIZE, 12)));
        let spot = spot();
        let mut req = request(&server, &spot);
        req.max_bytes = MIB;
        let error = fetch(&req).await.unwrap_err();
        assert!(
            matches!(error, DownloadError::TooLarge { cap } if cap == MIB),
            "{error:?}"
        );
        assert_eq!(left_behind(&spot.files), NOTHING);
    }

    #[tokio::test]
    async fn t08b_a_body_without_length_growing_past_max_bytes_is_refused_leaving_nothing() {
        let server = serve(Resource::new(body(SIZE, 13)).no_content_length());
        let spot = spot();
        let mut req = request(&server, &spot);
        req.max_bytes = MIB;
        let error = fetch(&req).await.unwrap_err();
        assert!(
            matches!(error, DownloadError::TooLarge { cap } if cap == MIB),
            "{error:?}"
        );
        assert_eq!(left_behind(&spot.files), NOTHING);
    }

    #[tokio::test]
    async fn t09_a_second_download_of_the_same_destination_is_busy() {
        let server = serve(Resource::new(body(SIZE, 14)).slow_steady(64 * KIB));
        let spot = spot();
        let req = request(&server, &spot);
        let cancel = AtomicCancel::default();
        let running = {
            let (req, cancel) = (req.clone(), cancel.clone());
            tokio::spawn(async move { download(&client(), &req, &cancel).await })
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while server.requests().is_empty() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let second = fetch(&req).await.unwrap_err();
        assert!(matches!(second, DownloadError::Busy), "{second:?}");
        cancel.0.store(true, Ordering::SeqCst);
        let first = running.await.unwrap().unwrap_err();
        assert!(matches!(first, DownloadError::Cancelled), "{first:?}");
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn t10_a_complete_part_answered_416_is_finalised_without_a_body() {
        let payload = body(SIZE, 15);
        let server = serve(Resource::new(payload.clone()).answer_416());
        let spot = spot();
        let req = request(&server, &spot);
        plant_part(&spot, &req, &payload, SIZE);
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, SIZE);
        assert_eq!(outcome.bytes, SIZE);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        assert_eq!(left_behind(&spot.files), ONLY_DEST_AND_RECORD);
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].header("range"), Some("bytes=3145728-"));
        assert_eq!(requests[0].status, 416);
        assert_eq!(requests[0].body_bytes, 0);
    }

    #[tokio::test]
    async fn t10b_a_partial_part_answered_416_restarts_from_zero() {
        let payload = body(SIZE, 16);
        let server = serve(Resource::new(payload.clone()).answer_416());
        let spot = spot();
        let req = request(&server, &spot);
        plant_part(&spot, &req, &payload[..MIB as usize], SIZE);
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].status, 416);
        assert_eq!(requests[1].header("range"), None);
        assert_eq!(requests[1].status, 200);
    }

    #[tokio::test]
    async fn t11_a_resumed_download_is_hashed_over_the_whole_file() {
        let payload = body(SIZE, 17);
        let server = serve(Resource::new(payload.clone()).drop_after(MIB));
        let spot = spot();
        let mut req = request(&server, &spot);
        req.expected_sha256 = Some(digest(&payload));
        assert!(fetch(&req).await.is_err());
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, MIB);
        assert_eq!(outcome.sha256_hex, Some(hex_of(&digest(&payload))));
        assert_eq!(
            read_record(&spot.files).sha256_hex,
            Some(hex_of(&digest(&payload)))
        );
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
    }

    #[tokio::test]
    async fn t11b_a_byte_flipped_after_the_resume_point_is_refused_naming_both_digests() {
        let payload = body(SIZE, 18);
        let at = 2 * MIB + 5;
        let server = serve(
            Resource::new(payload.clone())
                .drop_after(MIB)
                .flip_byte_at(at),
        );
        let spot = spot();
        let mut req = request(&server, &spot);
        req.expected_sha256 = Some(digest(&payload));
        assert!(matches!(
            fetch(&req).await,
            Err(DownloadError::Transport(_))
        ));
        match fetch(&req).await.unwrap_err() {
            DownloadError::ChecksumMismatch {
                expected_hex,
                actual_hex,
            } => {
                assert_eq!(expected_hex, hex_of(&digest(&payload)));
                assert_eq!(actual_hex, hex_of(&digest(&flipped(&payload, at))));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(server.requests()[1].status, 206);
        assert_eq!(left_behind(&spot.files), NOTHING);
    }

    #[tokio::test]
    async fn t12_a_slow_steady_transfer_outlives_the_read_timeout() {
        let payload = body(64 * KIB * 20, 19);
        let server = serve(Resource::new(payload.clone()).slow_steady(64 * KIB));
        let spot = spot();
        let req = request(&server, &spot);
        let started = Instant::now();
        let outcome = download(
            &client_with_read_timeout(Duration::from_secs(5)),
            &req,
            &AtomicCancel::default(),
        )
        .await
        .unwrap();
        assert!(
            started.elapsed() >= Duration::from_secs(19),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(outcome.bytes, payload.len() as u64);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
    }

    #[tokio::test]
    async fn t13_a_completed_file_is_revalidated_and_a_changed_etag_is_stale() {
        let payload = body(SIZE, 20);
        let server = serve(Resource::new(payload.clone()).change_etag_after(2, body(SIZE, 21)));
        let spot = spot();
        let req = request(&server, &spot);
        fetch(&req).await.unwrap();
        let again = fetch(&req).await.unwrap();
        assert_eq!(
            again,
            DownloadOutcome {
                path: spot.files.dest.clone(),
                bytes: SIZE,
                etag: Some(ETAG_V0.to_owned()),
                last_modified: Some(LAST_MODIFIED[0].to_owned()),
                resumed_from: SIZE,
                sha256_hex: None,
                revalidated: true,
            }
        );
        let error = fetch(&req).await.unwrap_err();
        assert!(
            matches!(
                &error,
                DownloadError::Stale { local_etag, remote_etag }
                    if local_etag.as_deref() == Some(ETAG_V0)
                        && remote_etag.as_deref() == Some(ETAG_V1)
            ),
            "{error:?}"
        );
        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[1].header("range"), Some("bytes=0-0"));
        assert_eq!(requests[1].header("if-none-match"), Some(ETAG_V0));
        assert_eq!(requests[1].status, 304);
        assert_eq!(requests[1].body_bytes, 0);
        assert_eq!(requests[2].status, 206);
        assert_eq!(requests[2].body_bytes, 1);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        assert_eq!(left_behind(&spot.files), ONLY_DEST_AND_RECORD);
    }

    #[tokio::test]
    async fn t13b_a_206_with_the_same_etag_counts_as_current() {
        let server = serve(Resource::new(body(SIZE, 22)).ignore_if_none_match());
        let spot = spot();
        let req = request(&server, &spot);
        fetch(&req).await.unwrap();
        let again = fetch(&req).await.unwrap();
        assert!(again.revalidated);
        assert_eq!(again.resumed_from, SIZE);
        let requests = server.requests();
        assert_eq!(requests[1].status, 206);
        assert_eq!(requests[1].body_bytes, 1);
    }

    #[tokio::test]
    async fn t13c_a_record_with_only_last_modified_revalidates_with_if_modified_since() {
        let server = serve(Resource::new(body(SIZE, 23)).no_etag());
        let spot = spot();
        let req = request(&server, &spot);
        fetch(&req).await.unwrap();
        let again = fetch(&req).await.unwrap();
        assert!(again.revalidated);
        let requests = server.requests();
        assert_eq!(requests[1].header("if-none-match"), None);
        assert_eq!(
            requests[1].header("if-modified-since"),
            Some(LAST_MODIFIED[0])
        );
        assert_eq!(requests[1].status, 304);
    }

    #[tokio::test]
    async fn t13d_a_record_without_validators_is_reused_on_its_size_alone() {
        let server = serve(Resource::new(body(SIZE, 24)).no_etag().no_last_modified());
        let spot = spot();
        let req = request(&server, &spot);
        fetch(&req).await.unwrap();
        let again = fetch(&req).await.unwrap();
        assert!(!again.revalidated);
        assert_eq!(again.resumed_from, SIZE);
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn t14_a_weak_etag_is_never_sent_in_if_range_and_last_modified_is_used() {
        let payload = body(SIZE, 25);
        let server = serve(Resource::new(payload.clone()).weak_etag().drop_after(MIB));
        let spot = spot();
        let req = request(&server, &spot);
        assert!(fetch(&req).await.is_err());
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, MIB);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        let requests = server.requests();
        assert_eq!(requests[1].header("if-range"), Some(LAST_MODIFIED[0]));
        assert_eq!(requests[1].status, 206);
    }

    #[tokio::test]
    async fn t15_a_part_without_sidecar_restarts_from_zero() {
        let payload = body(SIZE, 26);
        let server = serve(Resource::new(payload.clone()));
        let spot = spot();
        let req = request(&server, &spot);
        fs::create_dir_all(spot.files.dest.parent().unwrap()).unwrap();
        fs::write(&spot.files.part, vec![0xEE; MIB as usize]).unwrap();
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        assert_eq!(server.requests()[0].header("range"), None);
        assert_eq!(left_behind(&spot.files), ONLY_DEST_AND_RECORD);
    }

    #[tokio::test]
    async fn a_destination_without_a_record_is_foreign_unless_overwrite_is_set() {
        let payload = body(SIZE, 27);
        let server = serve(Resource::new(payload.clone()));
        let spot = spot();
        let mut req = request(&server, &spot);
        fs::create_dir_all(spot.files.dest.parent().unwrap()).unwrap();
        fs::write(&spot.files.dest, b"user file").unwrap();
        let error = fetch(&req).await.unwrap_err();
        assert!(matches!(error, DownloadError::Exists), "{error:?}");
        assert_eq!(fs::read(&spot.files.dest).unwrap(), b"user file");
        assert!(server.requests().is_empty());
        req.overwrite = true;
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        assert_eq!(left_behind(&spot.files), ONLY_DEST_AND_RECORD);
    }

    #[tokio::test]
    async fn a_pinned_etag_the_server_does_not_present_is_stale() {
        let server = serve(Resource::new(body(SIZE, 28)));
        let spot = spot();
        let mut req = request(&server, &spot);
        req.expected_etag = Some("\"ab-v9\"".to_owned());
        let error = fetch(&req).await.unwrap_err();
        assert!(
            matches!(
                &error,
                DownloadError::Stale { local_etag, remote_etag }
                    if local_etag.as_deref() == Some("\"ab-v9\"")
                        && remote_etag.as_deref() == Some(ETAG_V0)
            ),
            "{error:?}"
        );
        assert_eq!(left_behind(&spot.files), NOTHING);
        req.expected_etag = Some(ETAG_V0.to_owned());
        assert_eq!(fetch(&req).await.unwrap().bytes, SIZE);
    }

    #[tokio::test]
    async fn an_http_error_names_its_status_and_leaves_nothing() {
        let server = serve(Resource::new(body(SIZE, 29)).status(404));
        let spot = spot();
        let error = fetch(&request(&server, &spot)).await.unwrap_err();
        assert!(
            matches!(error, DownloadError::Http { status: 404 }),
            "{error:?}"
        );
        assert_eq!(left_behind(&spot.files), NOTHING);
    }

    #[tokio::test]
    async fn a_416_to_a_request_without_range_is_range_not_satisfiable() {
        let server = serve(Resource::new(body(SIZE, 30)).status(416));
        let spot = spot();
        let error = fetch(&request(&server, &spot)).await.unwrap_err();
        assert!(
            matches!(error, DownloadError::RangeNotSatisfiable),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_cancel_during_a_stalled_body_is_noticed_without_waiting_for_the_read_timeout() {
        let server = serve(Resource::new(body(SIZE, 31)).stall_after(256 * KIB));
        let spot = spot();
        let req = request(&server, &spot);
        let cancel = AtomicCancel::default();
        let flag = cancel.0.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            flag.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let error = download(
            &client_with_read_timeout(Duration::from_secs(30)),
            &req,
            &cancel,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, DownloadError::Cancelled), "{error:?}");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(fs::metadata(&spot.files.part).unwrap().len(), 256 * KIB);
        assert_eq!(read_sidecar(&spot.files).received, 256 * KIB);
    }

    async fn cancel_once_received(
        server: &TestServer,
        req: DownloadRequest,
    ) -> (Result<DownloadOutcome, DownloadError>, Duration) {
        let cancel = AtomicCancel::default();
        let flag = cancel.0.clone();
        let running = tokio::spawn(async move {
            download(
                &client_with_read_timeout(Duration::from_secs(30)),
                &req,
                &cancel,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(5), server.received(1))
            .await
            .expect("the request never reached the canned server");
        flag.store(true, Ordering::SeqCst);
        let asked = Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(3), running)
            .await
            .expect("the cancel was not seen while the server withheld its headers")
            .unwrap();
        (result, asked.elapsed())
    }

    #[tokio::test]
    async fn a_cancel_while_the_server_withholds_the_headers_is_acknowledged_within_a_second() {
        let server = serve(Resource::new(body(SIZE, 48)).withhold_headers());
        let spot = spot();
        let req = request(&server, &spot);
        let (result, waited) = cancel_once_received(&server, req).await;
        let error = result.unwrap_err();
        assert!(matches!(error, DownloadError::Cancelled), "{error:?}");
        assert!(waited < Duration::from_secs(1), "{waited:?}");
        assert_eq!(left_behind(&spot.files), NOTHING);
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn a_cancel_while_a_revalidation_waits_for_headers_is_acknowledged_within_a_second() {
        let payload = body(SIZE, 49);
        let origin = serve(Resource::new(payload.clone()));
        let spot = spot();
        let mut req = request(&origin, &spot);
        fetch(&req).await.unwrap();
        let silent = serve(Resource::new(Vec::new()).withhold_headers());
        req.url = silent.url("/product.fits");
        edit_record(&spot.files, |record| record.url = req.url.to_string());
        let (result, waited) = cancel_once_received(&silent, req).await;
        let error = result.unwrap_err();
        assert!(matches!(error, DownloadError::Cancelled), "{error:?}");
        assert!(waited < Duration::from_secs(1), "{waited:?}");
        assert_eq!(left_behind(&spot.files), ONLY_DEST_AND_RECORD);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        assert_eq!(silent.requests()[0].header("range"), Some("bytes=0-0"));
    }

    #[tokio::test]
    async fn progress_is_reported_at_most_every_100_ms() {
        let server = serve(Resource::new(body(SIZE, 32)).slow_steady(4 * MIB));
        let spot = spot();
        let watcher = Watcher::default();
        download(&client(), &request(&server, &spot), &watcher)
            .await
            .unwrap();
        let calls = watcher.calls.lock().unwrap();
        assert!(calls.len() >= 4, "{}", calls.len());
        assert_eq!((calls[0].0, calls[0].1), (0, Some(SIZE)));
        let last = calls.last().unwrap();
        assert_eq!((last.0, last.1), (SIZE, Some(SIZE)));
        for pair in calls[..calls.len() - 1].windows(2) {
            let gap = pair[1].2.duration_since(pair[0].2);
            assert!(gap >= Duration::from_millis(95), "{gap:?}");
        }
    }

    #[tokio::test]
    async fn the_sidecar_is_rewritten_every_8_mib() {
        let payload = body(10 * MIB, 33);
        let server = serve(Resource::new(payload.clone()).slow_steady(4 * MIB));
        let spot = spot();
        let watcher = Watcher::watching(&spot.files);
        download(&client(), &request(&server, &spot), &watcher)
            .await
            .unwrap();
        let seen = watcher.sidecar_received.lock().unwrap();
        assert_eq!(seen.first(), Some(&0));
        assert!(seen.iter().all(|&r| r == 0 || r >= 8 * MIB), "{seen:?}");
        assert!(
            seen.iter().any(|&r| (8 * MIB..10 * MIB).contains(&r)),
            "{seen:?}"
        );
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
    }

    #[tokio::test]
    async fn a_completed_file_is_checked_against_a_later_expected_sha256() {
        let payload = body(SIZE, 34);
        let server = serve(Resource::new(payload.clone()));
        let spot = spot();
        let mut req = request(&server, &spot);
        fetch(&req).await.unwrap();
        req.expected_sha256 = Some(digest(&payload));
        let again = fetch(&req).await.unwrap();
        assert_eq!(again.sha256_hex, Some(hex_of(&digest(&payload))));
        req.expected_sha256 = Some([0u8; 32]);
        match fetch(&req).await.unwrap_err() {
            DownloadError::ChecksumMismatch {
                expected_hex,
                actual_hex,
            } => {
                assert_eq!(expected_hex, "0".repeat(64));
                assert_eq!(actual_hex, hex_of(&digest(&payload)));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(left_behind(&spot.files), ONLY_DEST_AND_RECORD);
    }

    #[tokio::test]
    async fn t10c_a_complete_part_answered_416_with_a_new_etag_restarts_from_zero() {
        let old = body(SIZE, 35);
        let new = body(SIZE, 36);
        let server = serve(
            Resource::new(old.clone())
                .answer_416()
                .change_etag_after(0, new.clone()),
        );
        let spot = spot();
        let req = request(&server, &spot);
        plant_part(&spot, &req, &old, SIZE);
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(outcome.etag.as_deref(), Some(ETAG_V1));
        assert_eq!(file_digest(&spot.files.dest), digest(&new));
        assert_eq!(read_record(&spot.files).etag.as_deref(), Some(ETAG_V1));
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].status, 416);
        assert_eq!(requests[1].header("range"), None);
        assert_eq!(requests[1].status, 200);
    }

    #[tokio::test]
    async fn t10d_a_complete_part_answered_416_with_another_length_restarts_from_zero() {
        let old = body(SIZE, 37);
        let new = body(SIZE + 4 * KIB, 38);
        let server = serve(
            Resource::new(old.clone())
                .no_etag()
                .no_last_modified()
                .answer_416()
                .change_etag_after(0, new.clone()),
        );
        let spot = spot();
        let req = request(&server, &spot);
        plant_part(&spot, &req, &old, SIZE);
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(outcome.bytes, SIZE + 4 * KIB);
        assert_eq!(file_digest(&spot.files.dest), digest(&new));
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].status, 416);
        assert_eq!(requests[1].status, 200);
    }

    #[tokio::test]
    async fn t13e_a_completed_file_whose_expected_size_changed_is_stale_without_a_request() {
        let server = serve(Resource::new(body(SIZE, 39)));
        let spot = spot();
        let mut req = request(&server, &spot);
        fetch(&req).await.unwrap();
        req.expected_size = Some(SIZE + 1);
        let error = fetch(&req).await.unwrap_err();
        assert!(
            matches!(
                &error,
                DownloadError::Stale { local_etag, remote_etag: None }
                    if local_etag.as_deref() == Some(ETAG_V0)
            ),
            "{error:?}"
        );
        assert_eq!(server.requests().len(), 1);
        assert_eq!(left_behind(&spot.files), ONLY_DEST_AND_RECORD);
        req.expected_size = Some(SIZE);
        assert!(fetch(&req).await.unwrap().revalidated);
    }

    #[tokio::test]
    async fn a_short_206_segment_is_continued_from_where_it_ended() {
        let payload = body(SIZE, 40);
        let server = serve(Resource::new(payload.clone()).segment_cap(MIB));
        let spot = spot();
        let mut req = request(&server, &spot);
        req.expected_sha256 = Some(digest(&payload));
        plant_part(&spot, &req, &payload[..MIB as usize], SIZE);
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, MIB);
        assert_eq!(outcome.bytes, SIZE);
        assert_eq!(outcome.sha256_hex, Some(hex_of(&digest(&payload))));
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        assert_eq!(left_behind(&spot.files), ONLY_DEST_AND_RECORD);
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].header("range"), Some("bytes=1048576-"));
        assert_eq!((requests[0].status, requests[0].body_bytes), (206, MIB));
        assert_eq!(requests[1].header("range"), Some("bytes=2097152-"));
        assert_eq!(requests[1].header("if-range"), Some(ETAG_V0));
        assert_eq!((requests[1].status, requests[1].body_bytes), (206, MIB));
    }

    #[tokio::test]
    async fn an_empty_206_body_keeps_the_part_and_fails_as_a_transfer_error() {
        let payload = body(SIZE, 41);
        let server = serve(
            Resource::new(payload.clone())
                .no_content_length()
                .drop_after(0),
        );
        let spot = spot();
        let req = request(&server, &spot);
        plant_part(&spot, &req, &payload[..MIB as usize], SIZE);
        let error = fetch(&req).await.unwrap_err();
        assert!(matches!(error, DownloadError::Transport(_)), "{error:?}");
        assert_eq!(fs::metadata(&spot.files.part).unwrap().len(), MIB);
        assert_eq!(read_sidecar(&spot.files).received, MIB);
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, MIB);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!((requests[0].status, requests[0].body_bytes), (206, 0));
        assert_eq!(requests[1].status, 206);
    }

    #[tokio::test]
    async fn a_resume_starts_at_the_count_the_sidecar_made_durable() {
        let payload = body(SIZE, 42);
        let server = serve(Resource::new(payload.clone()));
        let spot = spot();
        let req = request(&server, &spot);
        let mut torn = payload[..MIB as usize].to_vec();
        torn.extend(vec![0u8; MIB as usize]);
        plant_part_at(&spot, &req, &torn, SIZE, MIB);
        let outcome = fetch(&req).await.unwrap();
        assert_eq!(outcome.resumed_from, MIB);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        assert_eq!(server.requests()[0].header("range"), Some("bytes=1048576-"));
    }

    #[tokio::test]
    async fn a_revalidation_that_cannot_reach_the_server_reuses_the_file_unrevalidated() {
        let payload = body(SIZE, 43);
        let origin = serve(Resource::new(payload.clone()));
        let spot = spot();
        let mut req = request(&origin, &spot);
        fetch(&req).await.unwrap();
        let unreachable = serve(Resource::new(Vec::new()).hang_up());
        let failing = serve(Resource::new(Vec::new()).status(503));
        for server in [&unreachable, &failing] {
            req.url = server.url("/product.fits");
            edit_record(&spot.files, |record| record.url = req.url.to_string());
            let again = fetch(&req).await.unwrap();
            assert!(!again.revalidated);
            assert_eq!((again.bytes, again.etag.as_deref()), (SIZE, Some(ETAG_V0)));
            assert_eq!(server.requests().len(), 1);
        }
        let missing = serve(Resource::new(Vec::new()).status(404));
        req.url = missing.url("/product.fits");
        edit_record(&spot.files, |record| record.url = req.url.to_string());
        let error = fetch(&req).await.unwrap_err();
        assert!(
            matches!(error, DownloadError::Http { status: 404 }),
            "{error:?}"
        );
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        assert_eq!(left_behind(&spot.files), ONLY_DEST_AND_RECORD);
    }

    #[tokio::test]
    async fn a_file_renamed_before_its_record_was_written_is_recovered_from_the_sidecar() {
        let payload = body(SIZE, 44);
        let server = serve(Resource::new(payload.clone()));
        let spot = spot();
        let req = request(&server, &spot);
        fetch(&req).await.unwrap();
        fs::remove_file(&spot.files.record).unwrap();
        write_sidecar(&spot.files, &sidecar_for(&req, SIZE, SIZE - 1));
        let error = fetch(&req).await.unwrap_err();
        assert!(matches!(error, DownloadError::Exists), "{error:?}");
        write_sidecar(&spot.files, &sidecar_for(&req, SIZE, SIZE));
        let again = fetch(&req).await.unwrap();
        assert!(again.revalidated);
        assert_eq!(again.etag.as_deref(), Some(ETAG_V0));
        assert_eq!(left_behind(&spot.files), ONLY_DEST_AND_RECORD);
        assert_eq!(read_record(&spot.files).size, SIZE);
        assert_eq!(file_digest(&spot.files.dest), digest(&payload));
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].header("range"), Some("bytes=0-0"));
    }

    #[tokio::test]
    async fn a_server_that_stops_sending_etags_is_compared_by_last_modified() {
        let server = serve(Resource::new(body(SIZE, 45)).no_etag());
        let spot = spot();
        let req = request(&server, &spot);
        fetch(&req).await.unwrap();
        edit_record(&spot.files, |record| record.etag = Some(ETAG_V0.to_owned()));
        let again = fetch(&req).await.unwrap();
        assert!(again.revalidated);
        edit_record(&spot.files, |record| {
            record.last_modified = Some(LAST_MODIFIED[1].to_owned())
        });
        let error = fetch(&req).await.unwrap_err();
        assert!(
            matches!(
                &error,
                DownloadError::Stale { local_etag, remote_etag: None }
                    if local_etag.as_deref() == Some(ETAG_V0)
            ),
            "{error:?}"
        );
        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[1].header("if-none-match"), Some(ETAG_V0));
        assert_eq!(requests[1].status, 206);
    }

    #[tokio::test]
    async fn a_server_without_validators_leaves_a_record_with_an_etag_unrevalidated() {
        let server = serve(Resource::new(body(SIZE, 46)).no_etag().no_last_modified());
        let spot = spot();
        let req = request(&server, &spot);
        fetch(&req).await.unwrap();
        edit_record(&spot.files, |record| {
            record.etag = Some(ETAG_V0.to_owned());
            record.last_modified = Some(LAST_MODIFIED[0].to_owned());
        });
        let again = fetch(&req).await.unwrap();
        assert!(!again.revalidated);
        assert_eq!(server.requests().len(), 2);
        assert_eq!(left_behind(&spot.files), ONLY_DEST_AND_RECORD);
    }

    #[tokio::test]
    async fn a_body_without_length_running_past_the_expected_size_stops_at_the_overrun() {
        let server = serve(Resource::new(body(SIZE, 47)).no_content_length());
        let spot = spot();
        let mut req = request(&server, &spot);
        req.expected_size = Some(MIB);
        let error = fetch(&req).await.unwrap_err();
        assert!(
            matches!(
                error,
                DownloadError::SizeMismatch { expected, actual }
                    if expected == MIB && actual > MIB && actual < SIZE
            ),
            "{error:?}"
        );
        assert_eq!(left_behind(&spot.files), NOTHING);
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn a_destination_differing_only_in_case_is_busy() {
        let spot = spot();
        let held = Claim::take(&spot.files.dest).unwrap();
        let shouted = PathBuf::from(spot.files.dest.to_string_lossy().to_uppercase());
        assert!(matches!(Claim::take(&shouted), Err(DownloadError::Busy)));
        drop(held);
        assert!(Claim::take(&shouted).is_ok());
    }
}
