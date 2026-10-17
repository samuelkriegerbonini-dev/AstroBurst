use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Error as AnyError;
use reqwest::header::{
    HeaderMap, HeaderName, CONTENT_RANGE, CONTENT_TYPE, ETAG, RANGE, RETRY_AFTER,
};
use reqwest::{Client, Response, StatusCode, Url};
use sha2::{Digest, Sha256};

use super::cache::{read_stored, touch, write_stored, LISTS_DIR};
use super::error::MastError;
use super::hub::MastHub;
use super::ratelimit::RateLimiter;
use super::service::note_list_write;
use super::transfer::{download_url, plan, transfer_client, DOWNLOAD_FILE_PATH};
use crate::core::mast::errors::{bad_input, classify, OpKind, Operation};
use crate::core::mast::types::{
    MastCmdError, MastDownloadInput, MastErrorCode, MastErrorContext, MastMissionId, MastPeek,
    MastPeekFootprint, MastPeekHdu,
};
use crate::infra::fits::header_peek::{
    Footprint, HeaderWalker, PeekBudget, PeekTarget, PeekedHdu, PeekedHeaders,
};
use crate::infra::net::http::HttpProfile;
use crate::types::header::HduHeader;

pub const PEEK_BUDGET: PeekBudget = PeekBudget {
    first_range: 131_072,
    next_range: 65_536,
    max_ranges: 8,
    max_bytes: 524_288,
};
pub const DEGRADE_CAP: u64 = 393_216;
pub const PEEK_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
pub const PEEK_DIR: &str = "peek";
pub const MAX_PEEK_HDUS: usize = 64;
const ERROR_BODY_CAP: u64 = 4096;
const REASON_LIMIT: usize = 200;
const FITS_SUFFIXES: [&str; 4] = [".fits", ".fit", ".fts", ".fits.fz"];
const EXPOSURE_KEYS: [&str; 3] = ["EXPTIME", "EFFEXPTM", "XPOSURE"];
const FILTER_WHEELS: [&str; 2] = ["FILTER1", "FILTER2"];
const CLEAR_PREFIX: &str = "CLEAR";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeekRequest {
    pub mission: MastMissionId,
    pub dataset_id: String,
    pub filename: String,
    pub uri: Option<String>,
}

struct Walked {
    headers: PeekedHeaders,
    file_bytes: Option<u64>,
    range_ignored: bool,
    etag: Option<String>,
}

struct ContentRange {
    start: Option<u64>,
    total: Option<u64>,
}

pub fn target_for(mission: MastMissionId) -> PeekTarget {
    match mission {
        MastMissionId::Hst => PeekTarget::AllSci,
        _ => PeekTarget::FirstImage,
    }
}

fn is_fits_name(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    FITS_SUFFIXES.iter().any(|suffix| lower.ends_with(suffix))
}

pub fn peek_uri(request: &PeekRequest) -> Result<String, MastCmdError> {
    let input = MastDownloadInput {
        dataset_id: request.dataset_id.clone(),
        filename: request.filename.clone(),
        program: None,
        size_bytes: 0,
        uri: request.uri.clone(),
        opens_as: None,
        replace: false,
    };
    let target = plan(request.mission, &input)?;
    if !is_fits_name(&request.filename) {
        return Err(bad_input(
            "filename",
            "only .fits, .fit, .fts and .fits.fz files have a header that can be read in part",
        ));
    }
    Ok(target.uri)
}

pub fn cache_file(root: &Path, uri: &str) -> PathBuf {
    let name: String = Sha256::digest(uri.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    root.join(LISTS_DIR)
        .join(PEEK_DIR)
        .join(format!("{name}.json"))
}

pub fn cached(path: &Path, uri: &str, now: SystemTime) -> Option<MastPeek> {
    read_stored::<MastPeek>(path, PEEK_TTL, now).filter(|peek| peek.uri == uri && peek.complete)
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn whole_secs(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis().div_ceil(1000)).unwrap_or(u64::MAX)
}

fn clipped(text: &str) -> String {
    text.chars().take(REASON_LIMIT).collect()
}

fn header_text(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn content_range(headers: &HeaderMap) -> Option<ContentRange> {
    let text = header_text(headers, CONTENT_RANGE)?;
    let rest = text.strip_prefix("bytes")?.trim_start();
    let (span, total) = rest.split_once('/')?;
    let total = match total.trim() {
        "*" => None,
        digits => Some(digits.parse().ok()?),
    };
    let start = match span.trim() {
        "*" => None,
        range => Some(range.split_once('-')?.0.trim().parse().ok()?),
    };
    Some(ContentRange { start, total })
}

fn transport_timeout(connect: bool) -> Duration {
    let profile = HttpProfile::transfer();
    if connect {
        profile.connect_timeout
    } else {
        profile.read_timeout
    }
}

fn transport(url: &Url, error: reqwest::Error, op: &Operation) -> MastCmdError {
    let url = url.to_string();
    let mapped = if error.is_timeout() {
        MastError::Timeout {
            url,
            timeout_secs: whole_secs(transport_timeout(error.is_connect())),
        }
    } else if error.is_redirect() {
        MastError::RedirectRefused { url }
    } else {
        MastError::Transport { url, source: error }
    };
    classify(&mapped, op, 1)
}

fn unreadable(op: &Operation, error: &AnyError) -> MastCmdError {
    MastCmdError::new(
        MastErrorCode::Decode,
        format!(
            "the file header could not be read: {}",
            clipped(&format!("{error:#}"))
        ),
    )
    .with_context(MastErrorContext {
        operation: Some(op.path.clone()),
        mission: op.mission,
        ..Default::default()
    })
}

async fn read_capped(mut response: Response, cap: u64) -> Result<Vec<u8>, reqwest::Error> {
    let limit = usize::try_from(cap).unwrap_or(usize::MAX);
    let mut out = Vec::new();
    while out.len() < limit {
        let Some(chunk) = response.chunk().await? else {
            break;
        };
        out.extend_from_slice(&chunk);
    }
    out.truncate(limit);
    Ok(out)
}

async fn status_failure(
    response: Response,
    url: &Url,
    op: &Operation,
    limiter: &RateLimiter,
    sent: Instant,
) -> MastCmdError {
    let status = response.status().as_u16();
    let content_type = header_text(response.headers(), CONTENT_TYPE);
    let retry_after_secs =
        header_text(response.headers(), RETRY_AFTER).and_then(|text| text.parse().ok());
    let body = read_capped(response, ERROR_BODY_CAP)
        .await
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    let answered = MastError::Status {
        status,
        url: url.to_string(),
        body,
        detail: None,
        content_type,
        retry_after_secs,
    };
    let consecutive = limiter.record(&answered, op, sent, Instant::now());
    classify(&answered, op, consecutive)
}

async fn walk_remote(
    client: &Client,
    url: &Url,
    target: PeekTarget,
    op: &Operation,
    limiter: &RateLimiter,
) -> Result<Walked, MastCmdError> {
    let mut walker = HeaderWalker::new(target, MAX_PEEK_HDUS, PEEK_BUDGET);
    let mut file_bytes = None;
    let mut range_ignored = false;
    let mut etag = None;
    while let Some((offset, len)) = walker.next_range() {
        let sent = Instant::now();
        let response = client
            .get(url.clone())
            .header(RANGE, format!("bytes={offset}-{}", offset + len - 1))
            .send()
            .await
            .map_err(|error| transport(url, error, op))?;
        if etag.is_none() {
            etag = header_text(response.headers(), ETAG);
        }
        match response.status() {
            StatusCode::PARTIAL_CONTENT => {
                let range = content_range(response.headers());
                let Some(start) = range.as_ref().and_then(|range| range.start) else {
                    return Err(unreadable(
                        op,
                        &anyhow::anyhow!("a 206 answer came without a byte range"),
                    ));
                };
                if let (None, Some(total)) = (file_bytes, range.and_then(|range| range.total)) {
                    file_bytes = Some(total);
                    walker.set_file_len(total);
                }
                let bytes = read_capped(response, len)
                    .await
                    .map_err(|error| transport(url, error, op))?;
                walker
                    .feed(&bytes, start)
                    .map_err(|error| unreadable(op, &error))?;
            }
            StatusCode::OK if offset > 0 => {
                file_bytes = file_bytes.or(response.content_length());
                range_ignored = true;
                break;
            }
            StatusCode::OK => {
                let total = response.content_length();
                range_ignored = total.is_none_or(|total| total > len);
                if let Some(total) = total {
                    file_bytes = Some(total);
                    walker.set_file_len(total);
                }
                let bytes = read_capped(response, DEGRADE_CAP)
                    .await
                    .map_err(|error| transport(url, error, op))?;
                walker
                    .feed(&bytes, 0)
                    .map_err(|error| unreadable(op, &error))?;
                break;
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                let total = content_range(response.headers())
                    .and_then(|range| range.total)
                    .unwrap_or(offset);
                file_bytes.get_or_insert(total);
                walker.set_file_len(total);
                break;
            }
            _ => return Err(status_failure(response, url, op, limiter, sent).await),
        }
    }
    Ok(Walked {
        headers: walker.finish(),
        file_bytes,
        range_ignored,
        etag,
    })
}

fn text(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn filter_of(header: &HduHeader) -> Option<String> {
    if let Some(filter) = text(header, "FILTER") {
        return Some(filter);
    }
    let wheels: Vec<String> = FILTER_WHEELS
        .iter()
        .filter_map(|key| text(header, key))
        .filter(|value| !value.to_ascii_uppercase().starts_with(CLEAR_PREFIX))
        .collect();
    (!wheels.is_empty()).then(|| wheels.join("+"))
}

fn date_obs_of(header: &HduHeader) -> Option<String> {
    if let Some(begin) = text(header, "DATE-BEG") {
        return Some(begin);
    }
    let date = text(header, "DATE-OBS")?;
    match text(header, "TIME-OBS") {
        Some(time) if !date.contains('T') => Some(format!("{date}T{time}")),
        _ => Some(date),
    }
}

fn science_header(headers: &PeekedHeaders) -> HduHeader {
    match (headers.primary.as_ref(), headers.hdus.first()) {
        (Some(primary), Some(hdu)) if hdu.index > 0 => primary.merge_with(&hdu.header),
        (_, Some(hdu)) => hdu.header.clone(),
        (Some(primary), None) => primary.clone(),
        (None, None) => HduHeader::empty(),
    }
}

fn notes(headers: &PeekedHeaders, range_ignored: bool) -> Option<String> {
    let mut notes = Vec::new();
    if range_ignored {
        notes.push(format!(
            "MAST ignored the byte range, so the walk stopped after {} bytes",
            headers.bytes_fetched
        ));
    }
    if !headers.complete {
        notes.push(format!(
            "the science headers continue past the {} bytes read, so the footprint may be partial",
            headers.bytes_fetched
        ));
    }
    if let Some(reason) = &headers.walk_error {
        notes.push(format!(
            "the walk stopped at a header it could not read: {}",
            clipped(reason)
        ));
    }
    for hdu in &headers.hdus {
        if let Some(reason) = &hdu.wcs_error {
            notes.push(format!(
                "HDU {} has no usable WCS: {}",
                hdu.index,
                clipped(reason)
            ));
        }
    }
    if headers.complete && headers.hdus.is_empty() {
        notes.push("no image header with two axes was found".to_owned());
    }
    (!notes.is_empty()).then(|| notes.join("; "))
}

fn hdu_of(hdu: &PeekedHdu) -> MastPeekHdu {
    MastPeekHdu {
        index: hdu.index,
        extname: hdu.extname.clone(),
        extver: hdu.extver,
        bitpix: hdu.bitpix,
        dims: hdu.dims.clone(),
        wcs_error: hdu.wcs_error.clone(),
    }
}

fn footprint_of(footprint: &Footprint) -> MastPeekFootprint {
    MastPeekFootprint {
        hdu: footprint.hdu_index,
        corners: footprint.corners,
        center: footprint.center,
        fov_arcmin: footprint.fov_arcmin,
    }
}

fn ranges_text(ranges: usize) -> String {
    match ranges {
        1 => "1 range".to_owned(),
        n => format!("{n} ranges"),
    }
}

fn summarize(
    request: &PeekRequest,
    uri: String,
    walked: Walked,
    elapsed_ms: u64,
) -> Result<MastPeek, MastCmdError> {
    let headers = &walked.headers;
    if !headers.complete && headers.hdus.is_empty() {
        return Err(MastCmdError::new(
            MastErrorCode::PeekIncomplete,
            format!(
                "no science header within the first {} bytes ({})",
                headers.bytes_fetched,
                ranges_text(headers.ranges)
            ),
        )
        .with_context(MastErrorContext {
            bytes: Some(headers.bytes_fetched),
            operation: Some(DOWNLOAD_FILE_PATH.to_owned()),
            mission: Some(request.mission),
            ..Default::default()
        }));
    }
    let science = science_header(headers);
    let first = headers.hdus.first();
    Ok(MastPeek {
        dataset_id: request.dataset_id.clone(),
        filename: request.filename.clone(),
        uri,
        dims: first.map(|hdu| hdu.dims.clone()).unwrap_or_default(),
        bitpix: first.map(|hdu| hdu.bitpix),
        bunit: text(&science, "BUNIT"),
        exptime: EXPOSURE_KEYS
            .iter()
            .find_map(|key| science.get_f64(key).filter(|value| value.is_finite())),
        date_obs: date_obs_of(&science),
        filter: filter_of(&science),
        instrume: text(&science, "INSTRUME"),
        footprints: headers.footprints.iter().map(footprint_of).collect(),
        hdus: headers.hdus.iter().map(hdu_of).collect(),
        bytes_fetched: headers.bytes_fetched,
        ranges: headers.ranges,
        file_bytes: walked.file_bytes,
        complete: headers.complete,
        range_ignored: walked.range_ignored,
        note: notes(headers, walked.range_ignored),
        etag: walked.etag,
        elapsed_ms,
        from_cache: false,
    })
}

fn store(hub: &MastHub, path: &Path, peek: &MastPeek) {
    match write_stored(path, peek, SystemTime::now()) {
        Ok(bytes) => note_list_write(hub, bytes),
        Err(error) => log::warn!("the MAST header peek was not cached: {error}"),
    }
}

pub async fn peek_product(hub: &MastHub, request: &PeekRequest) -> Result<MastPeek, MastCmdError> {
    let started = Instant::now();
    let uri = peek_uri(request)?;
    let path = cache_file(hub.cache_root(), &uri);
    if let Some(hit) = cached(&path, &uri, SystemTime::now()) {
        touch(&path);
        return Ok(MastPeek {
            dataset_id: request.dataset_id.clone(),
            elapsed_ms: elapsed_ms(started),
            from_cache: true,
            ..hit
        });
    }
    let op = Operation::new(Some(request.mission), OpKind::Download, DOWNLOAD_FILE_PATH);
    let url = download_url(&hub.settings().base_url, &uri)?;
    let client = transfer_client().map_err(|error| classify(&MastError::Client(error), &op, 1))?;
    let limit = hub.settings().timeout(OpKind::Download);
    let walk = walk_remote(
        &client,
        &url,
        target_for(request.mission),
        &op,
        hub.limiter(),
    );
    let walked = match tokio::time::timeout(limit, walk).await {
        Ok(walked) => walked?,
        Err(_) => {
            let timed_out = MastError::Timeout {
                url: url.to_string(),
                timeout_secs: whole_secs(limit),
            };
            return Err(classify(&timed_out, &op, 1));
        }
    };
    let peek = summarize(request, uri, walked, elapsed_ms(started))?;
    if peek.complete {
        store(hub, &path, &peek);
    }
    Ok(peek)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::types::{MastCacheKind, MastErrorCode};
    use crate::infra::fits::header_peek::test_support::{
        header_bytes, image_cards, primary_cards, tan_wcs_cards, Cards, SparseFits, MAST_BUDGET,
    };
    use crate::infra::fits::header_peek::{parse_peek, PeekedHeaders};
    use crate::infra::mast::cache::write_stored;
    use crate::infra::mast::service::cache_clear;
    use crate::infra::mast::settings::{MastSettings, ENV_MAST_TIMEOUT_MS, ENV_MAST_URL};
    use crate::infra::mast::transfer::download_url;
    use crate::infra::net::test_server::{serve, Resource, TestServer};
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    const HST_DATASET: &str = "j9irw3fwq";
    const HST_FILE: &str = "j9irw3fwq_flc.fits";
    const HST_URI: &str = "mast:HST/product/j9irw3fwq_flc.fits";
    const JWST_DATASET: &str = "jw02739001001_02105_00001_nrca1";
    const JWST_FILE: &str = "jw02739001001_02105_00001_nrca1_cal.fits";
    const JWST_URI: &str = "mast:JWST/product/jw02739001001_02105_00001_nrca1_cal.fits";
    const CHIP: u64 = 512;
    const DAY: Duration = Duration::from_secs(24 * 3600);

    fn settings_for(base: &str, timeout_ms: Option<&str>) -> MastSettings {
        let base = base.to_owned();
        let timeout = timeout_ms.map(str::to_owned);
        MastSettings::from_lookup(move |name| match name {
            ENV_MAST_URL => Some(base.clone()),
            ENV_MAST_TIMEOUT_MS => timeout.clone(),
            _ => None,
        })
    }

    struct Rig {
        server: TestServer,
        root: tempfile::TempDir,
        hub: MastHub,
    }

    fn canned(resource: Resource) -> Rig {
        canned_with(resource, None)
    }

    fn canned_with(resource: Resource, timeout_ms: Option<&str>) -> Rig {
        let server = serve(resource);
        let root = tempfile::tempdir().unwrap();
        let hub = MastHub::new(
            settings_for(&server.base, timeout_ms),
            root.path().to_path_buf(),
        )
        .unwrap();
        Rig { server, root, hub }
    }

    fn request(mission: MastMissionId, dataset: &str, filename: &str) -> PeekRequest {
        PeekRequest {
            mission,
            dataset_id: dataset.to_owned(),
            filename: filename.to_owned(),
            uri: None,
        }
    }

    fn hst() -> PeekRequest {
        request(MastMissionId::Hst, HST_DATASET, HST_FILE)
    }

    fn jwst() -> PeekRequest {
        request(MastMissionId::Jwst, JWST_DATASET, JWST_FILE)
    }

    fn target_of(server: &TestServer, uri: &str) -> String {
        let url = download_url(&server.base, uri).unwrap();
        format!("{}?{}", url.path(), url.query().unwrap())
    }

    fn ranges_sent(server: &TestServer) -> Vec<String> {
        server
            .requests()
            .iter()
            .map(|r| r.header("range").unwrap_or("none").to_owned())
            .collect()
    }

    fn files_under(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(files_under(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    struct Canned {
        status: u16,
        headers: Vec<(&'static str, String)>,
        body: Vec<u8>,
    }

    struct Scripted {
        statuses: Arc<Mutex<Vec<u16>>>,
        root: tempfile::TempDir,
        hub: MastHub,
    }

    impl Scripted {
        fn statuses(&self) -> Vec<u16> {
            self.statuses.lock().unwrap().clone()
        }
    }

    fn requested_range(head: &str) -> Option<(u64, u64)> {
        let value = head.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("range")
                .then(|| value.trim().to_owned())
        })?;
        let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
        Some((start.parse().ok()?, end.parse().ok()?))
    }

    fn answer_one<F>(mut stream: TcpStream, answer: &F, log: &Mutex<Vec<u16>>, seen: &AtomicUsize)
    where
        F: Fn(usize, Option<(u64, u64)>) -> Canned,
    {
        let mut head = Vec::new();
        let mut buffer = [0u8; 4096];
        while !head.windows(4).any(|window| window == b"\r\n\r\n") {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(read) => head.extend_from_slice(&buffer[..read]),
            }
        }
        let ordinal = seen.fetch_add(1, Ordering::SeqCst);
        let canned = answer(ordinal, requested_range(&String::from_utf8_lossy(&head)));
        log.lock().unwrap().push(canned.status);
        let mut reply = format!(
            "HTTP/1.1 {} Scripted\r\nConnection: close\r\nContent-Length: {}\r\n",
            canned.status,
            canned.body.len()
        );
        for (name, value) in &canned.headers {
            reply.push_str(&format!("{name}: {value}\r\n"));
        }
        reply.push_str("\r\n");
        let _ = stream
            .write_all(reply.as_bytes())
            .and_then(|()| stream.write_all(&canned.body));
    }

    fn scripted<F>(answer: F) -> Scripted
    where
        F: Fn(usize, Option<(u64, u64)>) -> Canned + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::new((answer, Arc::clone(&statuses), AtomicUsize::new(0)));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let shared = Arc::clone(&shared);
                std::thread::spawn(move || {
                    let (answer, log, seen) = &*shared;
                    answer_one(stream, answer, log, seen);
                });
            }
        });
        let root = tempfile::tempdir().unwrap();
        let hub = MastHub::new(settings_for(&base, None), root.path().to_path_buf()).unwrap();
        Scripted {
            statuses,
            root,
            hub,
        }
    }

    fn slice(body: &[u8], start: u64, end: u64) -> (u64, Vec<u8>) {
        let end = end.min(body.len() as u64 - 1);
        (end, body[start as usize..=end as usize].to_vec())
    }

    fn error_extensions_only() -> SparseFits {
        let mut file = SparseFits::default();
        file.push_hdu(&primary_cards(&[]), 0);
        for extver in 1..=20 {
            file.push_hdu(
                &image_cards("ERR", extver, -32, 256, 256, &[]),
                256 * 256 * 4,
            );
        }
        file
    }

    fn table_cards(extname: &str, row_bytes: u64, rows: u64) -> Cards {
        vec![
            ("XTENSION", "'BINTABLE'".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", row_bytes.to_string()),
            ("NAXIS2", rows.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("TFIELDS", "1".into()),
            ("EXTNAME", format!("'{extname:<8}'")),
        ]
    }

    fn hst_flc() -> (SparseFits, Vec<u64>) {
        let mut file = SparseFits::default();
        let mut starts = vec![file.push_hdu(
            &primary_cards(&[
                ("TELESCOP", "'HST     '".into()),
                ("INSTRUME", "'ACS     '".into()),
                ("DETECTOR", "'WFC     '".into()),
                ("FILTER1", "'F606W   '".into()),
                ("FILTER2", "'CLEAR2L '".into()),
                ("EXPTIME", "507.0".into()),
                ("DATE-OBS", "'2004-09-12'".into()),
                ("TIME-OBS", "'06:35:21'".into()),
            ]),
            0,
        )];
        for (chip, dec) in [(1, 2.2), (2, 2.25)] {
            let mut sci = image_cards(
                "SCI",
                chip,
                -32,
                CHIP,
                CHIP,
                &tan_wcs_cards((150.1, dec), (256.5, 256.5), 1.4e-5, 30.0),
            );
            sci.push(("BUNIT", "'ELECTRONS'".into()));
            starts.push(file.push_hdu(&sci, CHIP * CHIP * 4));
            starts.push(file.push_hdu(
                &image_cards("ERR", chip, -32, CHIP, CHIP, &[]),
                CHIP * CHIP * 4,
            ));
            starts.push(file.push_hdu(
                &image_cards("DQ", chip, 16, CHIP, CHIP, &[]),
                CHIP * CHIP * 2,
            ));
        }
        starts.push(file.push_hdu(&image_cards("D2IMARR", 1, -32, 64, 32, &[]), 64 * 32 * 4));
        starts.push(file.push_hdu(&table_cards("WCSCORR", 512, 14), 512 * 14));
        (file, starts)
    }

    fn jwst_cal_with(primary_extra: Cards) -> SparseFits {
        let mut primary = vec![
            ("TELESCOP", "'JWST    '".to_string()),
            ("INSTRUME", "'NIRCAM  '".into()),
            ("FILTER", "'F200W   '".into()),
            ("DATE-OBS", "'2022-08-14'".into()),
            ("TIME-OBS", "'09:40:51.669'".into()),
            ("DATE-BEG", "'2022-08-14T09:40:51.669'".into()),
            ("EFFEXPTM", "161.052".into()),
        ];
        primary.extend(primary_extra);
        let mut sci = image_cards(
            "SCI",
            1,
            -32,
            600,
            500,
            &tan_wcs_cards((274.73, -13.87), (300.5, 250.5), 8.6e-6, 12.0),
        );
        sci.push(("BUNIT", "'MJy/sr  '".into()));
        sci.push(("XPOSURE", "161.052".into()));
        let mut file = SparseFits::default();
        file.push_hdu(&primary_cards(&primary), 0);
        file.push_hdu(&sci, 600 * 500 * 4);
        file.push_hdu(&image_cards("ERR", 1, -32, 600, 500, &[]), 600 * 500 * 4);
        file.push_hdu(&image_cards("DQ", 1, 32, 600, 500, &[]), 600 * 500 * 4);
        file.push_hdu(&table_cards("ASDF", 1, 4000), 4000);
        file
    }

    fn jwst_cal() -> SparseFits {
        jwst_cal_with(Vec::new())
    }

    fn assert_matches_local_parse(peek: &MastPeek, local: &PeekedHeaders) {
        assert_eq!(peek.footprints.len(), local.footprints.len());
        for (got, want) in peek.footprints.iter().zip(&local.footprints) {
            assert_eq!(got.hdu, want.hdu_index);
            assert_eq!(got.corners, want.corners);
            assert_eq!(got.center, want.center);
            assert_eq!(got.fov_arcmin, want.fov_arcmin);
        }
    }

    #[test]
    fn the_production_budget_is_the_one_the_modelled_acs_flc_fits_in() {
        assert_eq!(PEEK_BUDGET, MAST_BUDGET);
        assert_eq!(DEGRADE_CAP, 3 * PEEK_BUDGET.first_range);
        assert_eq!(PEEK_TTL, 30 * DAY);
        assert_eq!(target_for(MastMissionId::Hst), PeekTarget::AllSci);
        assert_eq!(target_for(MastMissionId::Jwst), PeekTarget::FirstImage);
        assert_eq!(target_for(MastMissionId::Tess), PeekTarget::FirstImage);
    }

    #[test]
    fn peek_uris_follow_the_download_rules_before_any_request() {
        assert_eq!(peek_uri(&hst()).unwrap(), HST_URI);
        assert_eq!(peek_uri(&jwst()).unwrap(), JWST_URI);
        let hlsp = "mast:HLSP/ceers/nircam1/ceers_nircam1_f200w_v0.5_i2d.fits";
        let ceers = PeekRequest {
            uri: Some(hlsp.into()),
            ..request(
                MastMissionId::Jwst,
                "ceers_nircam1",
                "ceers_nircam1_f200w_v0.5_i2d.fits",
            )
        };
        assert_eq!(peek_uri(&ceers).unwrap(), hlsp);
        let refused = [
            (
                request(MastMissionId::Hst, HST_DATASET, "j9irw3fwq_flc.fits.gz"),
                MastErrorCode::BadInput,
                "filename: ",
            ),
            (
                request(MastMissionId::Hst, HST_DATASET, "j9irw3fwq_drz.jpg"),
                MastErrorCode::BadInput,
                "filename: ",
            ),
            (
                request(MastMissionId::Hst, "../x", HST_FILE),
                MastErrorCode::BadInput,
                "datasetId: ",
            ),
            (
                request(MastMissionId::Roman, "r0001", "r0001_cal.fits"),
                MastErrorCode::UriRefused,
                "Roman",
            ),
            (
                PeekRequest {
                    uri: Some("mast:HST/product/other_flc.fits".into()),
                    ..hst()
                },
                MastErrorCode::BadInput,
                "uri: ",
            ),
            (
                PeekRequest {
                    uri: Some("mast:JWST/product/j9irw3fwq_flc.fits".into()),
                    ..hst()
                },
                MastErrorCode::BadInput,
                "uri: ",
            ),
            (
                PeekRequest {
                    uri: Some("https://example.com/j9irw3fwq_flc.fits".into()),
                    ..hst()
                },
                MastErrorCode::UriRefused,
                "mast:",
            ),
        ];
        for (bad, code, start) in refused {
            let error = peek_uri(&bad).unwrap_err();
            assert_eq!(error.code, code, "{bad:?}");
            assert!(error.message.contains(start), "{bad:?}: {}", error.message);
        }
        assert!(peek_uri(&request(
            MastMissionId::Hst,
            HST_DATASET,
            "j9irw3fwq_flc.fits.fz"
        ))
        .is_ok());
    }

    #[tokio::test]
    async fn refused_requests_send_nothing() {
        let (file, _) = hst_flc();
        let rig = canned(Resource::new(file.bytes()));
        for bad in [
            request(MastMissionId::Hst, HST_DATASET, "j9irw3fwq_flc.fits.gz"),
            request(MastMissionId::Roman, "r0001", "r0001_cal.fits"),
            request(MastMissionId::Hst, "a b", HST_FILE),
        ] {
            assert!(peek_product(&rig.hub, &bad).await.is_err(), "{bad:?}");
        }
        assert!(rig.server.requests().is_empty());
        assert!(files_under(rig.root.path()).is_empty());
    }

    #[tokio::test]
    async fn an_hst_flc_is_walked_range_by_range_into_two_chip_footprints_inside_the_budget() {
        let (file, starts) = hst_flc();
        let rig = canned(Resource::new(file.bytes()));
        let peek = peek_product(&rig.hub, &hst()).await.unwrap();
        assert!(peek.complete, "{:?}", peek.note);
        assert_eq!(peek.note, None);
        assert_eq!(
            (
                peek.dataset_id.as_str(),
                peek.filename.as_str(),
                peek.uri.as_str()
            ),
            (HST_DATASET, HST_FILE, HST_URI)
        );
        let sci: Vec<_> = peek
            .hdus
            .iter()
            .map(|h| (h.index, h.extname.as_deref(), h.extver))
            .collect();
        assert_eq!(
            sci,
            vec![(1, Some("SCI"), Some(1)), (4, Some("SCI"), Some(2))]
        );
        assert_eq!(
            (peek.dims.clone(), peek.bitpix),
            (vec![CHIP, CHIP], Some(-32))
        );
        assert_eq!(peek.instrume.as_deref(), Some("ACS"));
        assert_eq!(peek.filter.as_deref(), Some("F606W"));
        assert_eq!(peek.exptime, Some(507.0));
        assert_eq!(peek.date_obs.as_deref(), Some("2004-09-12T06:35:21"));
        assert_eq!(peek.bunit.as_deref(), Some("ELECTRONS"));
        assert_eq!(peek.file_bytes, Some(file.len()));
        assert_eq!(peek.etag.as_deref(), Some("\"ab-v0\""));
        assert!(!peek.range_ignored && !peek.from_cache);
        assert_matches_local_parse(
            &peek,
            &parse_peek(&file.bytes(), PeekTarget::AllSci, MAX_PEEK_HDUS).unwrap(),
        );
        assert!((peek.footprints[1].center[1] - 2.25).abs() < 1e-9);

        let walked = [
            starts[0], starts[2], starts[3], starts[4], starts[5], starts[6], starts[7],
        ];
        let expected: Vec<String> = walked
            .iter()
            .enumerate()
            .map(|(i, &offset)| {
                let want = if i == 0 {
                    PEEK_BUDGET.first_range
                } else {
                    PEEK_BUDGET.next_range
                };
                let len = want.min(file.len() - offset);
                format!("bytes={offset}-{}", offset + len - 1)
            })
            .collect();
        assert_eq!(ranges_sent(&rig.server), expected);
        assert_eq!(peek.ranges, 7);
        assert!(
            peek.bytes_fetched <= PEEK_BUDGET.max_bytes,
            "{}",
            peek.bytes_fetched
        );
        let target = target_of(&rig.server, HST_URI);
        assert!(rig
            .server
            .requests()
            .iter()
            .all(|r| r.target == target && r.status == 206));
    }

    #[tokio::test]
    async fn a_jwst_cal_is_read_in_one_range_through_its_first_image() {
        let file = jwst_cal();
        let rig = canned(Resource::new(file.bytes()));
        let peek = peek_product(&rig.hub, &jwst()).await.unwrap();
        assert!(peek.complete);
        assert_eq!(ranges_sent(&rig.server), vec!["bytes=0-131071".to_owned()]);
        assert_eq!(
            (peek.ranges, peek.bytes_fetched),
            (1, PEEK_BUDGET.first_range)
        );
        assert_eq!(peek.hdus.len(), 1);
        assert_eq!(
            (peek.hdus[0].index, peek.hdus[0].extname.as_deref()),
            (1, Some("SCI"))
        );
        assert_eq!(peek.dims, vec![600, 500]);
        assert_eq!(peek.filter.as_deref(), Some("F200W"));
        assert_eq!(peek.instrume.as_deref(), Some("NIRCAM"));
        assert_eq!(peek.exptime, Some(161.052));
        assert_eq!(peek.date_obs.as_deref(), Some("2022-08-14T09:40:51.669"));
        assert_eq!(peek.bunit.as_deref(), Some("MJy/sr"));
        assert_matches_local_parse(
            &peek,
            &parse_peek(&file.bytes(), PeekTarget::FirstImage, MAX_PEEK_HDUS).unwrap(),
        );
    }

    #[tokio::test]
    async fn a_primary_header_longer_than_the_first_range_takes_a_second_range() {
        let history: Cards = (0..2000)
            .map(|i| ("HISTORY", format!("padding card {i}")))
            .collect();
        let file = jwst_cal_with(history.clone());
        assert!(header_bytes(&primary_cards(&history)).len() as u64 > PEEK_BUDGET.first_range);
        let rig = canned(Resource::new(file.bytes()));
        let peek = peek_product(&rig.hub, &jwst()).await.unwrap();
        assert!(peek.complete);
        assert_eq!(
            ranges_sent(&rig.server),
            vec![
                "bytes=0-131071".to_owned(),
                "bytes=131072-196607".to_owned()
            ]
        );
        assert_eq!((peek.ranges, peek.bytes_fetched), (2, 196_608));
        assert_eq!(peek.footprints.len(), 1);
    }

    #[tokio::test]
    async fn a_server_that_ignores_the_range_is_read_to_393216_bytes_and_dropped() {
        let (file, _) = hst_flc();
        let rig = canned(Resource::new(file.bytes()).ignore_range());
        let peek = peek_product(&rig.hub, &hst()).await.unwrap();
        assert!(peek.range_ignored);
        assert!(!peek.complete);
        assert_eq!((peek.ranges, peek.bytes_fetched), (1, DEGRADE_CAP));
        assert_eq!(peek.footprints.len(), 1);
        assert_eq!(peek.file_bytes, Some(file.len()));
        let note = peek.note.clone().unwrap();
        assert!(
            note.contains("ignored the byte range") && note.contains("partial"),
            "{note}"
        );
        let sent = rig.server.requests();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].status, 200);
        assert!(
            !cache_file(rig.hub.cache_root(), HST_URI).exists(),
            "an incomplete peek is not cached"
        );

        let whole = jwst_cal();
        let rig = canned(Resource::new(whole.bytes()).ignore_range());
        let peek = peek_product(&rig.hub, &jwst()).await.unwrap();
        assert!(peek.complete && peek.range_ignored);
        assert_eq!((peek.ranges, peek.bytes_fetched), (1, DEGRADE_CAP));
        assert!(cache_file(rig.hub.cache_root(), JWST_URI).is_file());

        let mut small = SparseFits::default();
        small.push_hdu(&primary_cards(&[]), 0);
        small.push_hdu(
            &image_cards(
                "SCI",
                1,
                -32,
                10,
                10,
                &tan_wcs_cards((10.0, 20.0), (5.5, 5.5), 1e-4, 0.0),
            ),
            400,
        );
        let rig = canned(Resource::new(small.bytes()).ignore_range());
        let peek = peek_product(&rig.hub, &jwst()).await.unwrap();
        assert!(
            peek.complete && !peek.range_ignored,
            "a whole file shorter than the range is not a refused range"
        );
        assert_eq!(peek.note, None);
        assert_eq!(peek.bytes_fetched, small.len());
    }

    #[tokio::test]
    async fn the_peek_is_cached_under_lists_peek_for_30_days_and_cleared_with_the_lists() {
        let rig = canned(Resource::new(jwst_cal().bytes()));
        let first = peek_product(&rig.hub, &jwst()).await.unwrap();
        let path = cache_file(rig.hub.cache_root(), JWST_URI);
        let digest: String = Sha256::digest(JWST_URI.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            path,
            rig.root
                .path()
                .join("lists")
                .join("peek")
                .join(format!("{digest}.json"))
        );
        assert_eq!(files_under(rig.root.path()), vec![path.clone()]);

        let other = PeekRequest {
            dataset_id: "jw02739-o001_t001_nircam".into(),
            ..jwst()
        };
        let second = peek_product(&rig.hub, &other).await.unwrap();
        assert!(second.from_cache && !first.from_cache);
        assert_eq!(second.dataset_id, "jw02739-o001_t001_nircam");
        assert_eq!(
            (
                second.footprints.clone(),
                second.bytes_fetched,
                second.ranges
            ),
            (first.footprints.clone(), first.bytes_fetched, first.ranges)
        );
        assert_eq!(
            rig.server.requests().len(),
            1,
            "the second peek sent a request"
        );

        let now = SystemTime::now();
        assert!(cached(&path, JWST_URI, now + 29 * DAY).is_some());
        assert!(cached(&path, JWST_URI, now + 31 * DAY).is_none());
        assert!(cached(&path, HST_URI, now).is_none());
        let partial = MastPeek {
            complete: false,
            ..first.clone()
        };
        let elsewhere = rig.root.path().join("partial.json");
        write_stored(&elsewhere, &partial, now).unwrap();
        assert!(cached(&elsewhere, JWST_URI, now).is_none());
        std::fs::remove_file(&elsewhere).unwrap();

        cache_clear(&rig.hub, MastCacheKind::Lists).unwrap();
        assert!(!path.exists());
        let third = peek_product(&rig.hub, &jwst()).await.unwrap();
        assert!(!third.from_cache);
        assert_eq!(rig.server.requests().len(), 2);
    }

    #[tokio::test]
    async fn http_failures_map_to_mast_codes_and_nothing_is_cached() {
        for (status, code) in [
            (404, MastErrorCode::Server),
            (503, MastErrorCode::Server),
            (429, MastErrorCode::RateLimited),
        ] {
            let rig = canned(Resource::new(Vec::new()).status(status));
            let error = peek_product(&rig.hub, &hst()).await.unwrap_err();
            assert_eq!(error.code, code, "HTTP {status}");
            assert_eq!(error.context.status, Some(status));
            assert_eq!(
                error.context.operation.as_deref(),
                Some("api/v0.1/Download/file")
            );
            assert!(files_under(rig.root.path()).is_empty());
        }
        let mut html = b"<!DOCTYPE html><html><body>Service unavailable</body></html>".to_vec();
        html.resize(4000, b' ');
        let rig = canned(Resource::new(html));
        let error = peek_product(&rig.hub, &hst()).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::Decode);
        assert!(
            error.message.contains("not a FITS primary header"),
            "{}",
            error.message
        );
        assert!(files_under(rig.root.path()).is_empty());
    }

    #[tokio::test]
    async fn no_science_header_inside_the_budget_is_reported_as_an_incomplete_peek() {
        let rig = canned(Resource::new(error_extensions_only().bytes()));
        let error = peek_product(&rig.hub, &hst()).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::PeekIncomplete);
        assert_eq!(error.context.bytes, Some(PEEK_BUDGET.max_bytes));
        assert!(
            error.message.contains("524288") && error.message.contains("7 ranges"),
            "{}",
            error.message
        );
        assert_eq!(rig.server.requests().len(), 7);
        assert!(files_under(rig.root.path()).is_empty());
    }

    #[tokio::test]
    async fn science_headers_without_wcs_or_before_a_broken_header_are_kept_with_a_note() {
        let mut file = SparseFits::default();
        file.push_hdu(&primary_cards(&[]), 0);
        let wcs = tan_wcs_cards((40.0, -1.0), (50.5, 50.5), 1e-4, 0.0);
        file.push_hdu(&image_cards("SCI", 1, -32, 100, 100, &wcs), 100 * 100 * 4);
        let broken_at = file.push_hdu(
            &[
                ("XTENSION", "'IMAGE   '".into()),
                ("BITPIX", "-32".into()),
                ("NAXIS", "2".into()),
                ("NAXIS1", "-5".into()),
                ("NAXIS2", "10".into()),
                ("EXTNAME", "'ERR     '".into()),
            ],
            0,
        );
        let rig = canned(Resource::new(file.bytes()));
        let peek = peek_product(&rig.hub, &hst()).await.unwrap();
        assert!(peek.complete);
        assert_eq!(peek.footprints.len(), 1);
        let note = peek.note.unwrap();
        assert!(
            note.contains(&format!("HDU 2 header at file offset {broken_at}")),
            "{note}"
        );

        let mut bare = SparseFits::default();
        bare.push_hdu(&primary_cards(&[]), 0);
        bare.push_hdu(&image_cards("SCI", 1, -32, 100, 100, &[]), 100 * 100 * 4);
        let rig = canned_with(Resource::new(bare.bytes()), None);
        let peek = peek_product(&rig.hub, &hst()).await.unwrap();
        assert!(peek.complete && peek.footprints.is_empty());
        assert_eq!(peek.hdus.len(), 1);
        assert!(peek.hdus[0]
            .wcs_error
            .as_deref()
            .is_some_and(|e| e.contains("CRPIX1")));
        assert!(peek.note.unwrap().contains("HDU 1 has no usable WCS"));
    }

    #[tokio::test]
    async fn a_stalled_answer_ends_with_the_mast_timeout() {
        let rig = canned_with(
            Resource::new(jwst_cal().bytes()).stall_after(100),
            Some("300"),
        );
        let started = Instant::now();
        let error = peek_product(&rig.hub, &jwst()).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::Timeout);
        assert_eq!(error.context.timeout_sec, Some(1));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert!(files_under(rig.root.path()).is_empty());
    }

    #[tokio::test]
    async fn a_200_after_the_first_range_is_dropped_unread_and_the_walk_stops() {
        let (file, _) = hst_flc();
        let body = file.bytes();
        let len = file.len();
        let rig = scripted(move |ordinal, range| match range {
            Some((start, end)) if ordinal < 6 => {
                let (end, part) = slice(&body, start, end);
                Canned {
                    status: 206,
                    headers: vec![("Content-Range", format!("bytes {start}-{end}/{len}"))],
                    body: part,
                }
            }
            _ => Canned {
                status: 200,
                headers: Vec::new(),
                body: body.clone(),
            },
        });
        let peek = peek_product(&rig.hub, &hst()).await.unwrap();
        assert_eq!(rig.statuses(), vec![206, 206, 206, 206, 206, 206, 200]);
        let ranged = PEEK_BUDGET.first_range + 5 * PEEK_BUDGET.next_range;
        assert_eq!((peek.ranges, peek.bytes_fetched), (6, ranged));
        assert!(peek.bytes_fetched <= PEEK_BUDGET.max_bytes);
        assert!(peek.range_ignored && !peek.complete);
        assert_eq!(peek.footprints.len(), 2);
        assert_eq!(peek.file_bytes, Some(len));
        let note = peek.note.clone().unwrap();
        assert!(
            note.contains("ignored the byte range")
                && note.contains(&ranged.to_string())
                && !note.contains(&DEGRADE_CAP.to_string()),
            "{note}"
        );
        assert!(!cache_file(rig.hub.cache_root(), HST_URI).exists());
    }

    #[tokio::test]
    async fn a_416_past_an_end_of_file_the_206_answers_left_open_ends_the_walk() {
        let mut small = SparseFits::default();
        small.push_hdu(&primary_cards(&[]), 0);
        small.push_hdu(
            &image_cards(
                "SCI",
                1,
                -32,
                10,
                10,
                &tan_wcs_cards((10.0, 20.0), (5.5, 5.5), 1e-4, 0.0),
            ),
            400,
        );
        let body = small.bytes();
        let len = small.len();
        let rig = scripted(move |_, range| match range {
            Some((start, _)) if start >= len => Canned {
                status: 416,
                headers: vec![("Content-Range", format!("bytes */{len}"))],
                body: Vec::new(),
            },
            Some((start, end)) => {
                let (end, part) = slice(&body, start, end);
                Canned {
                    status: 206,
                    headers: vec![("Content-Range", format!("bytes {start}-{end}/*"))],
                    body: part,
                }
            }
            None => Canned {
                status: 200,
                headers: Vec::new(),
                body: body.clone(),
            },
        });
        let peek = peek_product(&rig.hub, &hst()).await.unwrap();
        assert_eq!(rig.statuses(), vec![206, 416]);
        assert!(peek.complete && !peek.range_ignored);
        assert_eq!(peek.note, None);
        assert_eq!(
            (peek.ranges, peek.bytes_fetched, peek.file_bytes),
            (1, len, Some(len))
        );
        assert_eq!(peek.footprints.len(), 1);
        assert!(cache_file(rig.hub.cache_root(), HST_URI).is_file());
    }

    #[tokio::test]
    async fn a_206_without_a_byte_range_is_refused_and_nothing_is_cached() {
        let body = jwst_cal().bytes();
        let rig = scripted(move |_, range| {
            let (start, end) = range.unwrap_or((0, 0));
            Canned {
                status: 206,
                headers: Vec::new(),
                body: slice(&body, start, end).1,
            }
        });
        let error = peek_product(&rig.hub, &jwst()).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::Decode);
        assert!(
            error
                .message
                .contains("a 206 answer came without a byte range"),
            "{}",
            error.message
        );
        assert_eq!(rig.statuses(), vec![206]);
        assert!(files_under(rig.root.path()).is_empty());
    }

    #[tokio::test]
    async fn a_429_opens_the_hub_rate_limit_window_for_the_other_mast_requests() {
        let rig = canned(Resource::new(Vec::new()).status(429));
        assert_eq!(rig.hub.limiter().open_window(Instant::now()), None);
        let first = peek_product(&rig.hub, &hst()).await.unwrap_err();
        assert_eq!(
            (first.code, first.context.consecutive),
            (MastErrorCode::RateLimited, Some(1))
        );
        let window = rig.hub.limiter().open_window(Instant::now());
        assert_eq!(window.map(|w| w.consecutive), Some(1), "{window:?}");
        let second = peek_product(&rig.hub, &hst()).await.unwrap_err();
        assert_eq!(
            (second.context.consecutive, second.context.retry_after_sec),
            (Some(2), Some(30))
        );
    }

    #[tokio::test]
    async fn an_incomplete_peek_read_in_one_answer_says_one_range() {
        let rig = canned(Resource::new(error_extensions_only().bytes()).ignore_range());
        let error = peek_product(&rig.hub, &hst()).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::PeekIncomplete);
        assert_eq!(
            error.message,
            format!("no science header within the first {DEGRADE_CAP} bytes (1 range)")
        );
    }

    #[test]
    fn transport_timeouts_name_the_timeout_that_fired() {
        let profile = HttpProfile::transfer();
        assert_ne!(profile.connect_timeout, profile.read_timeout);
        assert_eq!(transport_timeout(true), profile.connect_timeout);
        assert_eq!(transport_timeout(false), profile.read_timeout);
    }

    #[tokio::test]
    #[ignore]
    async fn jwst_nrca1_cal_peek_over_http_reads_one_range_inside_262144_bytes() {
        let Some(path) = std::env::var_os("ASTROBURST_MAST_PEEK_SAMPLE") else {
            eprintln!(
                "SKIPPED: ASTROBURST_MAST_PEEK_SAMPLE unset, the nrca1 HTTP peek did not run"
            );
            return;
        };
        let mut head = Vec::new();
        std::io::Read::read_to_end(
            &mut std::io::Read::take(std::fs::File::open(&path).unwrap(), 1 << 20),
            &mut head,
        )
        .unwrap();
        let rig = canned(Resource::new(head.clone()));
        let peek = peek_product(&rig.hub, &jwst()).await.unwrap();
        assert!(peek.complete);
        assert!(peek.bytes_fetched <= 262_144, "{}", peek.bytes_fetched);
        assert_eq!(peek.hdus[0].index, 1);
        assert_matches_local_parse(
            &peek,
            &parse_peek(&head[..131_072], PeekTarget::FirstImage, MAX_PEEK_HDUS).unwrap(),
        );
        println!(
            "nrca1 HTTP peek: {:?} {:?} {} bytes in {} ranges, filter {:?}, exptime {:?}, date {:?}",
            peek.dims, peek.bitpix, peek.bytes_fetched, peek.ranges, peek.filter, peek.exptime, peek.date_obs
        );
    }

    #[tokio::test]
    #[ignore]
    async fn live_hst_flc_peeks_report_two_chips_inside_the_budget() {
        if std::env::var_os("ASTROBURST_MAST_LIVE_PEEK").is_none() {
            eprintln!("SKIPPED: ASTROBURST_MAST_LIVE_PEEK unset, no live MAST request was made");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let hub = MastHub::new(MastSettings::default(), root.path().to_path_buf()).unwrap();
        for dataset in ["j9irw3fwq", "j9irw4b1q", "j9irw5kaq"] {
            let filename = format!("{dataset}_flc.fits");
            let started = Instant::now();
            let peek = peek_product(&hub, &request(MastMissionId::Hst, dataset, &filename))
                .await
                .unwrap();
            println!(
                "{filename}: complete {} dims {:?} bitpix {:?} {} footprints, hdus {:?}, {} bytes in {} ranges of {:?}, filter {:?}, exptime {:?}, date {:?}, note {:?}, {:?}",
                peek.complete,
                peek.dims,
                peek.bitpix,
                peek.footprints.len(),
                peek.hdus.iter().map(|h| (h.index, h.extname.clone(), h.extver)).collect::<Vec<_>>(),
                peek.bytes_fetched,
                peek.ranges,
                peek.file_bytes,
                peek.filter,
                peek.exptime,
                peek.date_obs,
                peek.note,
                started.elapsed()
            );
            for footprint in &peek.footprints {
                println!(
                    "  HDU {} corners {:?} fov {:?}",
                    footprint.hdu, footprint.corners, footprint.fov_arcmin
                );
            }
            assert!(peek.complete, "{filename}: {:?}", peek.note);
            assert_eq!(peek.footprints.len(), 2, "{filename}");
            assert!(
                peek.bytes_fetched <= PEEK_BUDGET.max_bytes
                    && peek.ranges <= PEEK_BUDGET.max_ranges
            );
            let again = peek_product(&hub, &request(MastMissionId::Hst, dataset, &filename))
                .await
                .unwrap();
            assert!(again.from_cache);
        }
    }
}
