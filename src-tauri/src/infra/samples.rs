use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use reqwest::{Client, Url};

use crate::infra::net::download::{
    download, DownloadError, DownloadObserver, DownloadRequest, Files,
};
use crate::infra::net::http::{build_client, HttpProfile, RedirectRule};

pub const SAMPLES_DIR: &str = "samples";
pub const ENV_SAMPLE_BASE_URL: &str = "ASTROBURST_SAMPLE_BASE_URL";
pub const DEFAULT_SAMPLE_BASE_URL: &str = "https://raw.githubusercontent.com/samuelkriegerbonini-dev/AstroBurst/360a86196b9316e3cda5c610bc4e6be7225e0988/exampleFits/sample-data";
pub const RELEASE_DOWNLOAD_BASE_URL: &str =
    "https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/download";
pub const SAMPLE_RELEASE_TAG: &str = "data-v1";
pub const RELEASE_ASSET_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];
const REFUSED_REDIRECT: &str = "redirect refused by the profile's redirect rule";
const LOOPBACK_HOSTS: &[&str] = &["127.0.0.1", "localhost", "[::1]"];
const SHA256_HEX_LEN: usize = 64;
const HST_M16_FRAME_BYTES: u64 = 10_298_880;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleSource {
    Hosted { base_url: &'static str },
    ReleaseAsset { tag: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleFile {
    pub name: &'static str,
    pub bytes: u64,
    pub sha256_hex: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleSet {
    pub id: &'static str,
    pub title: &'static str,
    pub credit: &'static str,
    pub licence: &'static str,
    pub demonstrates: &'static str,
    pub source: SampleSource,
    pub files: &'static [SampleFile],
}

impl SampleSet {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.bytes).sum()
    }
}

const JWST_LICENCE: &str = "Public JWST archive data; acknowledgement requested by STScI";

static SAMPLE_SETS: [SampleSet; 3] = [
    SampleSet {
        id: "hst-m16-wfpc2",
        title: "HST/WFPC2 narrowband frames of M16 (Eagle Nebula)",
        credit: "HST/WFPC2 narrowband frames of the Eagle Nebula (M16): NASA, ESA, J. Hester and P. Scowen (Arizona State University), data via STScI",
        licence: "Public-domain Hubble archive data; credit requested by STScI",
        demonstrates: "Compose (SHO and HOO narrowband palettes), photometry and masks",
        source: SampleSource::Hosted {
            base_url: DEFAULT_SAMPLE_BASE_URL,
        },
        files: &[
            SampleFile {
                name: "502nmos.fits",
                bytes: HST_M16_FRAME_BYTES,
                sha256_hex: "ad8e67285cd987968edf0e0b1b46ee8077fc33feb2728472d57147a5afbd0421",
            },
            SampleFile {
                name: "656nmos.fits",
                bytes: HST_M16_FRAME_BYTES,
                sha256_hex: "9c1c87da9e992cbaa784edca8d480afed504374ef9b7b8cc3632281427f06fe4",
            },
            SampleFile {
                name: "673nmos.fits",
                bytes: HST_M16_FRAME_BYTES,
                sha256_hex: "3eceec970765095fb3ef1624af1bb3fa31adfd0b8bbb0905d6dcbf24c396f2bf",
            },
        ],
    },
    SampleSet {
        id: "jwst-gc-nirspec-ifu",
        title: "JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum",
        credit: "JWST/NIRSpec IFU cube and 1-D spectrum of the Galactic Center (GC-NIRSPEC-1): NASA, ESA, CSA, GTO programme 1266 (PI M. Garcia Marin), data via MAST at STScI",
        licence: JWST_LICENCE,
        demonstrates: "Cube: spectra with error bands, moment maps, line fits and line measurement, next to the pipeline's 1-D spectrum",
        source: SampleSource::ReleaseAsset {
            tag: SAMPLE_RELEASE_TAG,
        },
        files: &[
            SampleFile {
                name: "jw01266-o005_t001_nirspec_g395h-f290lp_s3d.fits",
                bytes: 174_983_040,
                sha256_hex: "9897a80e06896184b1f554022760f2de937bd7a2c77552e70ee9d90ea8125b0a",
            },
            SampleFile {
                name: "jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits",
                bytes: 696_960,
                sha256_hex: "eb56296ed3da7b80e03bcdac91fbe22854a352ad7096bb3c00e01a826a5d6b10",
            },
        ],
    },
    SampleSet {
        id: "jwst-miri-lrs",
        title: "JWST/MIRI LRS spectrum of the Type Ia supernova SN 2021aefx with its 2-D spectrum and a calibrated exposure",
        credit: "JWST/MIRI LRS slit spectroscopy of SN 2021aefx: NASA, ESA, CSA, GO programme 2072 (PI S. W. Jha), data via MAST at STScI",
        licence: JWST_LICENCE,
        demonstrates: "1-D spectra: the spectrum document with line measurement, the 2-D resampled slit spectrum and a level-2 calibrated exposure",
        source: SampleSource::ReleaseAsset {
            tag: SAMPLE_RELEASE_TAG,
        },
        files: &[
            SampleFile {
                name: "jw02072-o001_t010_miri_p750l_x1d.fits",
                bytes: 178_560,
                sha256_hex: "c6fdd9e50577e1bc48ab582efaaee171d421b615eb5d4b3211fcdeaefe165478",
            },
            SampleFile {
                name: "jw02072-o001_t010_miri_p750l_s2d.fits",
                bytes: 990_720,
                sha256_hex: "514645befe2b31955134e1b2696e24195ba0439798e763a23987d90116f44958",
            },
            SampleFile {
                name: "jw02072001001_06101_00001_mirimage_cal.fits",
                bytes: 33_995_520,
                sha256_hex: "73d89cf8b1b3aef7b995a5933caa85ed9dee57540e9f9e01b864cedfb64756fc",
            },
        ],
    },
];

pub fn sample_sets() -> &'static [SampleSet] {
    &SAMPLE_SETS
}

pub fn find_set(id: &str) -> Option<&'static SampleSet> {
    SAMPLE_SETS.iter().find(|set| set.id == id)
}

pub fn set_dir(data_root: &Path, set: &SampleSet) -> PathBuf {
    data_root.join(SAMPLES_DIR).join(set.id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalState {
    Absent,
    Partial,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalStatus {
    pub state: LocalState,
    pub received_bytes: u64,
    pub total_bytes: u64,
}

fn frame_files(dir: &Path, file: &SampleFile) -> Files {
    Files::for_dest(&dir.join(file.name))
}

fn is_complete(files: &Files, file: &SampleFile) -> bool {
    fs::metadata(&files.dest).is_ok_and(|meta| meta.is_file() && meta.len() == file.bytes)
        && files
            .recorded_sha256_hex(file.bytes)
            .is_some_and(|recorded| recorded.eq_ignore_ascii_case(file.sha256_hex))
}

pub fn local_status(data_root: &Path, set: &SampleSet) -> LocalStatus {
    let dir = set_dir(data_root, set);
    let mut complete = 0;
    let mut started = false;
    let mut received_bytes = 0;
    for file in set.files {
        let files = frame_files(&dir, file);
        if is_complete(&files, file) {
            complete += 1;
            received_bytes += file.bytes;
        } else if let Some(resumable) = files.resumable_bytes(file.bytes) {
            started = true;
            received_bytes += resumable;
        }
    }
    let state = if complete == set.files.len() {
        LocalState::Ready
    } else if complete > 0 || started {
        LocalState::Partial
    } else {
        LocalState::Absent
    };
    LocalStatus {
        state,
        received_bytes,
        total_bytes: set.total_bytes(),
    }
}

pub fn ready_paths(data_root: &Path, set: &SampleSet) -> Option<Vec<PathBuf>> {
    let dir = set_dir(data_root, set);
    set.files
        .iter()
        .map(|file| {
            let files = frame_files(&dir, file);
            is_complete(&files, file).then_some(files.dest)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origins {
    pub hosted: Option<String>,
    pub release: Option<String>,
}

impl Origins {
    pub fn from_env() -> Self {
        Self::with_override(std::env::var(ENV_SAMPLE_BASE_URL).ok())
    }

    pub fn with_override(override_value: Option<String>) -> Self {
        let origins = Self {
            hosted: hosted_override(override_value.clone()),
            release: release_override(override_value),
        };
        if origins.hosted.is_some() && origins.release.is_none() {
            log::info!(
                "{ENV_SAMPLE_BASE_URL} moves the HST set only; the JWST sets keep downloading from GitHub unless it is an http URL on 127.0.0.1, localhost or [::1]"
            );
        }
        origins
    }
}

fn trimmed(override_value: Option<String>) -> Option<String> {
    override_value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub fn release_override(override_value: Option<String>) -> Option<String> {
    accepted_base_url(&trimmed(override_value)?).filter(|base| is_loopback_http(base))
}

fn is_loopback_http(base: &str) -> bool {
    Url::parse(base).is_ok_and(|url| {
        url.scheme() == "http"
            && url
                .host_str()
                .is_some_and(|host| LOOPBACK_HOSTS.contains(&host.to_ascii_lowercase().as_str()))
    })
}

pub fn hosted_override(override_value: Option<String>) -> Option<String> {
    let raw = trimmed(override_value)?;
    let accepted = accepted_base_url(&raw);
    if accepted.is_none() {
        log::warn!(
            "{ENV_SAMPLE_BASE_URL} ignored: expected https://<host>/<path> or http://127.0.0.1, localhost or [::1]"
        );
    }
    accepted
}

fn accepted_base_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    let clean = url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none();
    let host = url.host_str()?.to_ascii_lowercase();
    let allowed = match url.scheme() {
        "https" => true,
        "http" => LOOPBACK_HOSTS.contains(&host.as_str()),
        _ => false,
    };
    (clean && allowed).then(|| url.as_str().trim_end_matches('/').to_owned())
}

pub fn file_url(set: &SampleSet, origins: &Origins, name: &str) -> Result<Url, String> {
    match set.source {
        SampleSource::Hosted { base_url } => {
            joined_url(origins.hosted.as_deref().unwrap_or(base_url), name)
        }
        SampleSource::ReleaseAsset { tag } => match origins.release.as_deref() {
            Some(base) => joined_url(base, name),
            None => joined_url(&format!("{RELEASE_DOWNLOAD_BASE_URL}/{tag}"), name),
        },
    }
}

fn joined_url(base: &str, name: &str) -> Result<Url, String> {
    Url::parse(&format!("{}/{name}", base.trim_end_matches('/')))
        .map_err(|error| format!("the sample URL is not valid: {error}"))
}

pub fn release_profile(origins: &Origins) -> HttpProfile {
    let redirect = match origins.release {
        Some(_) => RedirectRule {
            https_only: false,
            ..RedirectRule::https(LOOPBACK_HOSTS)
        },
        None => RedirectRule::https(RELEASE_ASSET_HOSTS),
    };
    HttpProfile {
        redirect,
        ..HttpProfile::transfer()
    }
}

fn sample_client(set: &SampleSet, origins: &Origins) -> reqwest::Result<Client> {
    match set.source {
        SampleSource::Hosted { .. } => build_client(&HttpProfile::transfer()),
        SampleSource::ReleaseAsset { .. } => build_client(&release_profile(origins)),
    }
}

pub trait SampleObserver: Sync {
    fn progress(&self, received_bytes: u64, total_bytes: u64, file: &str);
    fn is_cancelled(&self) -> bool;
}

#[derive(Debug, thiserror::Error)]
pub enum SampleError {
    #[error("cancelled")]
    Cancelled,
    #[error("the HTTP client could not be built: {0}")]
    Client(reqwest::Error),
    #[error("{file}: the download did not match the published checksum")]
    Checksum { file: String },
    #[error("{file}: the server answered HTTP {status}")]
    Http { file: String, status: u16 },
    #[error(
        "{file}: not available on GitHub (HTTP 404); the sample release may not be published yet"
    )]
    NotPublished { file: String },
    #[error("{file}: the server redirected the download to a host that AstroBurst does not download samples from")]
    Redirected { file: String },
    #[error("{file}: the size did not match")]
    Size { file: String },
    #[error("{file}: {message}")]
    Network { file: String, message: String },
    #[error("{file}: {source}")]
    Io { file: String, source: io::Error },
}

impl SampleError {
    fn from_download(file: &str, error: DownloadError) -> Self {
        let file = file.to_owned();
        match error {
            DownloadError::Cancelled => Self::Cancelled,
            DownloadError::ChecksumMismatch { .. } => Self::Checksum { file },
            DownloadError::Http { status } => Self::Http { file, status },
            DownloadError::SizeMismatch { .. } | DownloadError::TooLarge { .. } => {
                Self::Size { file }
            }
            DownloadError::Io(source) => Self::Io { file, source },
            DownloadError::Transport(message) if message.contains(REFUSED_REDIRECT) => {
                Self::Redirected { file }
            }
            other => Self::Network {
                file,
                message: other.to_string(),
            },
        }
    }

    fn unpublished_when_missing(self) -> Self {
        match self {
            Self::Http { file, status: 404 } => Self::NotPublished { file },
            other => other,
        }
    }

    fn io(file: &SampleFile) -> impl Fn(io::Error) -> Self + '_ {
        move |source| Self::Io {
            file: file.name.to_owned(),
            source,
        }
    }
}

struct FileProgress<'a> {
    observer: &'a dyn SampleObserver,
    file: &'static str,
    before: u64,
    total: u64,
}

impl DownloadObserver for FileProgress<'_> {
    fn progress(&self, received: u64, _total: Option<u64>) {
        self.observer
            .progress(self.before + received, self.total, self.file);
    }

    fn is_cancelled(&self) -> bool {
        self.observer.is_cancelled()
    }
}

pub async fn download_set(
    origins: &Origins,
    data_root: &Path,
    set: &SampleSet,
    observer: &dyn SampleObserver,
) -> Result<Vec<PathBuf>, SampleError> {
    let client = sample_client(set, origins).map_err(SampleError::Client)?;
    download_with(&client, origins, data_root, set, observer).await
}

async fn download_with(
    client: &Client,
    origins: &Origins,
    data_root: &Path,
    set: &SampleSet,
    observer: &dyn SampleObserver,
) -> Result<Vec<PathBuf>, SampleError> {
    let dir = set_dir(data_root, set);
    let total = set.total_bytes();
    let mut before = 0;
    let mut paths = Vec::with_capacity(set.files.len());
    for file in set.files {
        let files = frame_files(&dir, file);
        if !is_complete(&files, file) {
            let progress = FileProgress {
                observer,
                file: file.name,
                before,
                total,
            };
            let url = sample_url(set, origins, file)?;
            let fetched = fetch_file(client, url, &files, file, &progress).await;
            match set.source {
                SampleSource::Hosted { .. } => fetched?,
                SampleSource::ReleaseAsset { .. } => {
                    fetched.map_err(SampleError::unpublished_when_missing)?
                }
            }
        }
        before += file.bytes;
        paths.push(files.dest);
    }
    Ok(paths)
}

fn sample_url(set: &SampleSet, origins: &Origins, file: &SampleFile) -> Result<Url, SampleError> {
    file_url(set, origins, file.name).map_err(|message| SampleError::Network {
        file: file.name.to_owned(),
        message,
    })
}

fn pinned_request(
    url: Url,
    dest: &Path,
    file: &SampleFile,
) -> Result<DownloadRequest, SampleError> {
    let expected_sha256 = published_digest(file.sha256_hex).ok_or_else(|| {
        SampleError::io(file)(io::Error::new(
            io::ErrorKind::InvalidData,
            "the published checksum is not a sha-256 digest",
        ))
    })?;
    Ok(DownloadRequest {
        url,
        dest: dest.to_path_buf(),
        expected_size: Some(file.bytes),
        expected_sha256: Some(expected_sha256),
        expected_etag: None,
        headers: Vec::new(),
        max_bytes: file.bytes,
        overwrite: true,
    })
}

async fn fetch_file(
    client: &Client,
    url: Url,
    files: &Files,
    file: &SampleFile,
    progress: &FileProgress<'_>,
) -> Result<(), SampleError> {
    remove_if_present(&files.dest).map_err(SampleError::io(file))?;
    let request = pinned_request(url, &files.dest, file)?;
    download(client, &request, progress)
        .await
        .map(drop)
        .map_err(|error| SampleError::from_download(file.name, error))
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn published_digest(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != SHA256_HEX_LEN || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut digest = [0u8; 32];
    for (slot, pair) in digest.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(digest)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use sha2::{Digest, Sha256};

    use super::*;
    use crate::infra::net::test_server::{serve, Recorded, Resource, TestServer};

    const LEN: u64 = 300 << 10;
    const DROP_AT: u64 = 100 << 10;
    const SLOW_BYTES_PER_S: u64 = 512 << 10;
    const NAMES: [&str; 3] = ["a.fits", "b.fits", "c.fits"];
    const OFFLINE: &str = "http://127.0.0.1:9";
    const ETAG_V0: &str = "\"ab-v0\"";
    const IFU_S3D: &str = "jw01266-o005_t001_nirspec_g395h-f290lp_s3d.fits";
    const IFU_X1D: &str = "jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits";
    const LRS_X1D: &str = "jw02072-o001_t010_miri_p750l_x1d.fits";
    const LRS_S2D: &str = "jw02072-o001_t010_miri_p750l_s2d.fits";
    const LRS_CAL: &str = "jw02072001001_06101_00001_mirimage_cal.fits";
    const TEST_TAG: &str = "test-tag";
    const ASSET_PATH: &str = "/release-asset";
    const JWST_LICENCE_TEXT: &str = "Public JWST archive data; acknowledgement requested by STScI";

    fn body(seed: u64) -> Vec<u8> {
        (0..LEN)
            .map(|i| ((i * 31 + i / 4099 + seed) % 251) as u8)
            .collect()
    }

    fn hex_of(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn sha_hex(bytes: &[u8]) -> String {
        hex_of(&Sha256::digest(bytes))
    }

    fn leaked_set(
        id: &'static str,
        source: SampleSource,
        names: &[&'static str],
        payload: &[u8],
    ) -> &'static SampleSet {
        let sha256_hex: &'static str = Box::leak(sha_hex(payload).into_boxed_str());
        let files: Vec<SampleFile> = names
            .iter()
            .map(|&name| SampleFile {
                name,
                bytes: payload.len() as u64,
                sha256_hex,
            })
            .collect();
        Box::leak(Box::new(SampleSet {
            id,
            title: "Test frames",
            credit: "Test credit",
            licence: "Test licence",
            demonstrates: "Test tools",
            source,
            files: Box::leak(files.into_boxed_slice()),
        }))
    }

    fn release_set(id: &'static str, payload: &[u8]) -> &'static SampleSet {
        leaked_set(
            id,
            SampleSource::ReleaseAsset { tag: TEST_TAG },
            &NAMES,
            payload,
        )
    }

    fn released_at(base: &str) -> Origins {
        Origins {
            hosted: None,
            release: Some(base.to_owned()),
        }
    }

    fn pinned_origins() -> Origins {
        Origins {
            hosted: None,
            release: None,
        }
    }

    async fn fetch_release(
        base: &str,
        root: &Path,
        set: &SampleSet,
    ) -> Result<Vec<PathBuf>, SampleError> {
        let origins = released_at(base);
        download_set(&origins, root, set, &Recorder::default()).await
    }

    struct Redirector {
        base: String,
        log: Arc<Mutex<Vec<Recorded>>>,
    }

    impl Redirector {
        fn requests(&self) -> Vec<Recorded> {
            self.log.lock().unwrap().clone()
        }

        fn targets(&self) -> Vec<String> {
            self.requests()
                .into_iter()
                .map(|request| request.target)
                .collect()
        }
    }

    fn read_request(stream: &mut TcpStream) -> Option<(String, Vec<(String, String)>)> {
        let mut data = Vec::new();
        let mut buffer = [0u8; 4096];
        while !data.windows(4).any(|window| window == b"\r\n\r\n") {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => return None,
                Ok(read) => data.extend_from_slice(&buffer[..read]),
            }
        }
        let head = String::from_utf8_lossy(&data).into_owned();
        let mut lines = head.lines();
        let target = lines.next()?.split_whitespace().nth(1)?.to_owned();
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        Some((target, headers))
    }

    fn redirector(asset_base: &str, published: &'static [&'static str]) -> Redirector {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&log);
        let location = format!("{asset_base}{ASSET_PATH}");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let Some((target, headers)) = read_request(&mut stream) else {
                    continue;
                };
                let name = target.trim_start_matches('/');
                let (status, reply) = if published.contains(&name) {
                    (
                        302,
                        format!("HTTP/1.1 302 Found\r\nLocation: {location}{target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                    )
                } else {
                    (
                        404,
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot Found".to_owned(),
                    )
                };
                seen.lock().unwrap().push(Recorded {
                    target,
                    headers,
                    status,
                    body_bytes: 0,
                });
                let _ = stream.write_all(reply.as_bytes());
                let _ = stream.flush();
            }
        });
        Redirector { base, log }
    }

    fn test_set(id: &'static str, payload: &[u8]) -> &'static SampleSet {
        leaked_set(
            id,
            SampleSource::Hosted { base_url: OFFLINE },
            &NAMES,
            payload,
        )
    }

    fn hosted_at(base: &str) -> Origins {
        Origins {
            hosted: Some(base.to_owned()),
            release: None,
        }
    }

    fn targets(server: &TestServer) -> Vec<String> {
        server
            .requests()
            .into_iter()
            .map(|request| request.target)
            .collect()
    }

    fn sibling(path: &Path, suffix: &str) -> PathBuf {
        PathBuf::from(format!("{}{suffix}", path.display()))
    }

    fn edit_json(path: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        edit(&mut value);
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }

    #[derive(Default)]
    struct Recorder {
        cancel_from: Option<u64>,
        cancelled: AtomicBool,
        calls: Mutex<Vec<(u64, u64, String)>>,
    }

    impl Recorder {
        fn cancelling_from(received_bytes: u64) -> Self {
            Self {
                cancel_from: Some(received_bytes),
                ..Self::default()
            }
        }

        fn first_received(&self) -> u64 {
            self.calls.lock().unwrap()[0].0
        }
    }

    impl SampleObserver for Recorder {
        fn progress(&self, received_bytes: u64, total_bytes: u64, file: &str) {
            self.calls
                .lock()
                .unwrap()
                .push((received_bytes, total_bytes, file.to_owned()));
            if self.cancel_from.is_some_and(|from| received_bytes >= from) {
                self.cancelled.store(true, Ordering::SeqCst);
            }
        }

        fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::SeqCst)
        }
    }

    async fn fetch_set(
        base: &str,
        root: &Path,
        set: &SampleSet,
    ) -> Result<Vec<PathBuf>, SampleError> {
        download_set(&hosted_at(base), root, set, &Recorder::default()).await
    }

    fn expected_paths(root: &Path, set: &SampleSet) -> Vec<PathBuf> {
        let dir = root.join("samples").join(set.id);
        set.files.iter().map(|file| dir.join(file.name)).collect()
    }

    fn status(state: LocalState, received_bytes: u64, total_bytes: u64) -> LocalStatus {
        LocalStatus {
            state,
            received_bytes,
            total_bytes,
        }
    }

    #[test]
    fn r1_an_empty_root_is_absent_with_nothing_received() {
        let root = tempfile::tempdir().unwrap();
        let set = test_set("r1-set", &body(1));
        assert_eq!(
            local_status(root.path(), set),
            status(LocalState::Absent, 0, 3 * LEN)
        );
        assert_eq!(ready_paths(root.path(), set), None);
        assert_eq!(
            set_dir(root.path(), set),
            root.path().join("samples").join("r1-set")
        );
    }

    #[tokio::test]
    async fn r2_a_set_downloads_in_order_and_ends_ready() {
        let payload = body(2);
        let server = serve(Resource::new(payload.clone()));
        let root = tempfile::tempdir().unwrap();
        let set = test_set("r2-set", &payload);
        let recorder = Recorder::default();
        let paths = download_set(&hosted_at(&server.base), root.path(), set, &recorder)
            .await
            .unwrap();
        assert_eq!(paths, expected_paths(root.path(), set));
        for path in &paths {
            assert_eq!(fs::read(path).unwrap(), payload, "{}", path.display());
        }
        assert_eq!(ready_paths(root.path(), set), Some(paths));
        assert_eq!(
            local_status(root.path(), set),
            status(LocalState::Ready, 3 * LEN, 3 * LEN)
        );
        assert_eq!(targets(&server), ["/a.fits", "/b.fits", "/c.fits"]);
        let calls = recorder.calls.lock().unwrap();
        assert!(calls.iter().all(|(_, total, _)| *total == 3 * LEN));
        assert!(calls.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        assert_eq!(
            calls.last().unwrap(),
            &(3 * LEN, 3 * LEN, "c.fits".to_owned())
        );
        let files_seen: Vec<&str> = calls.iter().map(|(_, _, file)| file.as_str()).collect();
        let first_of = |name: &str| files_seen.iter().position(|seen| *seen == name).unwrap();
        assert!(first_of("a.fits") < first_of("b.fits") && first_of("b.fits") < first_of("c.fits"));
    }

    #[tokio::test]
    async fn r3_a_dropped_transfer_resumes_with_a_range_request() {
        let payload = body(3);
        let server = serve(Resource::new(payload.clone()).drop_after(DROP_AT));
        let root = tempfile::tempdir().unwrap();
        let set = test_set("r3-set", &payload);
        let first = fetch_set(&server.base, root.path(), set).await.unwrap_err();
        assert!(
            matches!(&first, SampleError::Network { file, .. } if file == "a.fits"),
            "{first:?}"
        );
        assert!(first.to_string().starts_with("a.fits: "), "{first}");
        assert_eq!(
            local_status(root.path(), set),
            status(LocalState::Partial, DROP_AT, 3 * LEN)
        );
        let paths = fetch_set(&server.base, root.path(), set).await.unwrap();
        assert_eq!(local_status(root.path(), set).state, LocalState::Ready);
        assert_eq!(fs::read(&paths[0]).unwrap(), payload);
        let requests = server.requests();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[1].target, "/a.fits");
        assert_eq!(requests[1].header("range"), Some("bytes=102400-"));
        assert_eq!(requests[1].status, 206);
        assert_eq!(targets(&server)[2..], ["/b.fits", "/c.fits"]);
    }

    #[tokio::test]
    async fn r4_a_flipped_byte_is_a_checksum_error_naming_the_file() {
        let payload = body(4);
        let server = serve(Resource::new(payload.clone()).flip_byte_at(LEN / 2));
        let root = tempfile::tempdir().unwrap();
        let set = test_set("r4-set", &payload);
        let error = fetch_set(&server.base, root.path(), set).await.unwrap_err();
        assert!(
            matches!(&error, SampleError::Checksum { file } if file == "a.fits"),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "a.fits: the download did not match the published checksum"
        );
        let status = local_status(root.path(), set);
        assert_ne!(status.state, LocalState::Ready);
        assert_eq!(status.received_bytes, 0);
        assert_eq!(ready_paths(root.path(), set), None);
        assert_eq!(targets(&server), ["/a.fits"]);
    }

    #[tokio::test]
    async fn r4b_an_http_error_names_the_file_and_the_status() {
        let payload = body(5);
        let server = serve(Resource::new(payload.clone()).status(404));
        let root = tempfile::tempdir().unwrap();
        let set = test_set("r4b-set", &payload);
        let error = fetch_set(&server.base, root.path(), set).await.unwrap_err();
        assert_eq!(error.to_string(), "a.fits: the server answered HTTP 404");
        assert_eq!(local_status(root.path(), set).state, LocalState::Absent);
    }

    #[tokio::test]
    async fn r5_a_cancel_mid_file_stays_partial_and_the_next_call_resumes_with_a_range() {
        let payload = body(6);
        let server = serve(Resource::new(payload.clone()).slow_steady(SLOW_BYTES_PER_S));
        let root = tempfile::tempdir().unwrap();
        let set = test_set("r5-set", &payload);
        let recorder = Recorder::cancelling_from(LEN + (16 << 10));
        let error = download_set(&hosted_at(&server.base), root.path(), set, &recorder)
            .await
            .unwrap_err();
        assert!(matches!(error, SampleError::Cancelled), "{error:?}");
        assert_eq!(error.to_string(), "cancelled");
        let (cancelled_at, _, file) = recorder.calls.lock().unwrap().last().cloned().unwrap();
        assert_eq!(file, "b.fits");
        let kept = cancelled_at - LEN;
        assert!(kept > 0 && kept < LEN, "{kept}");
        assert_eq!(
            local_status(root.path(), set),
            status(LocalState::Partial, LEN + kept, 3 * LEN)
        );
        let before = server.requests().len();
        let resumed = Recorder::default();
        let paths = download_set(&hosted_at(&server.base), root.path(), set, &resumed)
            .await
            .unwrap();
        assert_eq!(paths, expected_paths(root.path(), set));
        assert_eq!(local_status(root.path(), set).state, LocalState::Ready);
        assert_eq!(targets(&server)[before..], ["/b.fits", "/c.fits"]);
        let range = format!("bytes={kept}-");
        let requests = server.requests();
        assert_eq!(requests[before].header("range"), Some(range.as_str()));
        assert_eq!(requests[before].status, 206);
        assert_eq!(resumed.first_received(), LEN + kept);
        for path in &paths {
            assert_eq!(fs::read(path).unwrap(), payload);
        }
    }

    #[tokio::test]
    async fn r6_a_ready_set_reopens_without_any_request() {
        let payload = body(7);
        let first = serve(Resource::new(payload.clone()));
        let root = tempfile::tempdir().unwrap();
        let set = test_set("r6-set", &payload);
        let downloaded = fetch_set(&first.base, root.path(), set).await.unwrap();
        assert_eq!(ready_paths(root.path(), set), Some(downloaded.clone()));
        let second = serve(Resource::new(payload.clone()));
        let again = fetch_set(&second.base, root.path(), set).await.unwrap();
        assert_eq!(again, downloaded);
        assert!(second.requests().is_empty(), "{:?}", second.requests());
        let offline = fetch_set(OFFLINE, root.path(), set).await.unwrap();
        assert_eq!(offline, downloaded);
        assert_eq!(first.requests().len(), 3);
    }

    #[test]
    fn r7_a_hosted_set_uses_its_pinned_base_and_accepts_only_safe_overrides() {
        assert_eq!(
            DEFAULT_SAMPLE_BASE_URL,
            "https://raw.githubusercontent.com/samuelkriegerbonini-dev/AstroBurst/360a86196b9316e3cda5c610bc4e6be7225e0988/exampleFits/sample-data"
        );
        assert_eq!(ENV_SAMPLE_BASE_URL, "ASTROBURST_SAMPLE_BASE_URL");
        let hst = find_set("hst-m16-wfpc2").unwrap();
        assert_eq!(
            hst.source,
            SampleSource::Hosted {
                base_url: DEFAULT_SAMPLE_BASE_URL
            }
        );
        let pinned = pinned_origins();
        assert_eq!(
            file_url(hst, &pinned, "502nmos.fits").unwrap().as_str(),
            format!("{DEFAULT_SAMPLE_BASE_URL}/502nmos.fits")
        );
        assert_eq!(hosted_override(None), None);
        let accepted = [
            (
                "https://mirror.example.org/samples",
                "https://mirror.example.org/samples",
            ),
            (
                "https://mirror.example.org/samples/",
                "https://mirror.example.org/samples",
            ),
            (
                "  https://mirror.example.org  ",
                "https://mirror.example.org",
            ),
            ("http://127.0.0.1:8080", "http://127.0.0.1:8080"),
            ("http://localhost:8080/fits/", "http://localhost:8080/fits"),
            ("http://[::1]:8080", "http://[::1]:8080"),
        ];
        for (raw, base) in accepted {
            let origins = Origins {
                hosted: hosted_override(Some(raw.to_owned())),
                release: None,
            };
            assert_eq!(origins.hosted.as_deref(), Some(base), "{raw}");
            assert_eq!(
                file_url(hst, &origins, "502nmos.fits").unwrap().as_str(),
                format!("{base}/502nmos.fits"),
                "{raw}"
            );
        }
        let refused = [
            "",
            "   ",
            "http://mirror.example.org/samples",
            "http://192.168.1.10:8080",
            "ftp://mirror.example.org/samples",
            "not a url",
            "https://user:secret@mirror.example.org",
            "https://mirror.example.org/samples?token=1",
        ];
        for raw in refused {
            assert_eq!(hosted_override(Some(raw.to_owned())), None, "{raw:?}");
        }
        assert_eq!(
            file_url(hst, &hosted_at("http://127.0.0.1:9/"), "a.fits")
                .unwrap()
                .as_str(),
            "http://127.0.0.1:9/a.fits"
        );
    }

    #[test]
    fn r8_the_published_set_matches_the_repository_frames() {
        let set = find_set("hst-m16-wfpc2").expect("the HST M16 set is published");
        assert_eq!(find_set("no-such-set"), None);
        assert_eq!(
            set.title,
            "HST/WFPC2 narrowband frames of M16 (Eagle Nebula)"
        );
        assert_eq!(
            set.credit,
            "HST/WFPC2 narrowband frames of the Eagle Nebula (M16): NASA, ESA, J. Hester and P. Scowen (Arizona State University), data via STScI"
        );
        assert_eq!(
            set.licence,
            "Public-domain Hubble archive data; credit requested by STScI"
        );
        assert_eq!(set.total_bytes(), 30_896_640);
        let names: Vec<&str> = set.files.iter().map(|file| file.name).collect();
        assert_eq!(names, ["502nmos.fits", "656nmos.fits", "673nmos.fits"]);
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("exampleFits")
            .join("sample-data");
        for file in set.files {
            let bytes = fs::read(repo.join(file.name)).unwrap();
            assert_eq!(bytes.len() as u64, file.bytes, "{}", file.name);
            assert_eq!(sha_hex(&bytes), file.sha256_hex, "{}", file.name);
        }
    }

    #[test]
    fn a1_the_jwst_sets_ship_as_data_v1_release_assets_pinned_by_size_and_sha256() {
        let ids: Vec<&str> = sample_sets().iter().map(|set| set.id).collect();
        assert_eq!(
            ids,
            ["hst-m16-wfpc2", "jwst-gc-nirspec-ifu", "jwst-miri-lrs"]
        );
        assert_eq!(SAMPLE_RELEASE_TAG, "data-v1");
        let pins: [(&str, &[(&str, u64, &str)], u64); 2] = [
            (
                "jwst-gc-nirspec-ifu",
                &[
                    (
                        IFU_S3D,
                        174_983_040,
                        "9897a80e06896184b1f554022760f2de937bd7a2c77552e70ee9d90ea8125b0a",
                    ),
                    (
                        IFU_X1D,
                        696_960,
                        "eb56296ed3da7b80e03bcdac91fbe22854a352ad7096bb3c00e01a826a5d6b10",
                    ),
                ],
                175_680_000,
            ),
            (
                "jwst-miri-lrs",
                &[
                    (
                        LRS_X1D,
                        178_560,
                        "c6fdd9e50577e1bc48ab582efaaee171d421b615eb5d4b3211fcdeaefe165478",
                    ),
                    (
                        LRS_S2D,
                        990_720,
                        "514645befe2b31955134e1b2696e24195ba0439798e763a23987d90116f44958",
                    ),
                    (
                        LRS_CAL,
                        33_995_520,
                        "73d89cf8b1b3aef7b995a5933caa85ed9dee57540e9f9e01b864cedfb64756fc",
                    ),
                ],
                35_164_800,
            ),
        ];
        for (id, files, total) in pins {
            let set = find_set(id).unwrap();
            assert_eq!(
                set.source,
                SampleSource::ReleaseAsset { tag: "data-v1" },
                "{id}"
            );
            let pinned: Vec<(&str, u64, &str)> = set
                .files
                .iter()
                .map(|file| (file.name, file.bytes, file.sha256_hex))
                .collect();
            assert_eq!(pinned, files, "{id}");
            assert_eq!(set.total_bytes(), total, "{id}");
            assert_eq!(set.licence, JWST_LICENCE_TEXT, "{id}");
            for file in set.files {
                assert_eq!(
                    file_url(set, &pinned_origins(), file.name).unwrap().as_str(),
                    format!(
                        "https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/download/data-v1/{}",
                        file.name
                    )
                );
            }
        }
        let ifu = find_set("jwst-gc-nirspec-ifu").unwrap();
        assert_eq!(
            ifu.title,
            "JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum"
        );
        assert_eq!(
            ifu.credit,
            "JWST/NIRSpec IFU cube and 1-D spectrum of the Galactic Center (GC-NIRSPEC-1): NASA, ESA, CSA, GTO programme 1266 (PI M. Garcia Marin), data via MAST at STScI"
        );
        assert_eq!(
            ifu.demonstrates,
            "Cube: spectra with error bands, moment maps, line fits and line measurement, next to the pipeline's 1-D spectrum"
        );
        let lrs = find_set("jwst-miri-lrs").unwrap();
        assert_eq!(
            lrs.title,
            "JWST/MIRI LRS spectrum of the Type Ia supernova SN 2021aefx with its 2-D spectrum and a calibrated exposure"
        );
        assert_eq!(
            lrs.credit,
            "JWST/MIRI LRS slit spectroscopy of SN 2021aefx: NASA, ESA, CSA, GO programme 2072 (PI S. W. Jha), data via MAST at STScI"
        );
        assert_eq!(
            lrs.demonstrates,
            "1-D spectra: the spectrum document with line measurement, the 2-D resampled slit spectrum and a level-2 calibrated exposure"
        );
        assert_eq!(
            find_set("hst-m16-wfpc2").unwrap().demonstrates,
            "Compose (SHO and HOO narrowband palettes), photometry and masks"
        );
    }

    #[tokio::test]
    async fn a2_a_release_asset_set_follows_the_redirect_to_the_asset_host_and_reopens_without_a_request(
    ) {
        let payload = body(30);
        let assets = serve(Resource::new(payload.clone()));
        let origin = redirector(&assets.base, &NAMES);
        let root = tempfile::tempdir().unwrap();
        let set = release_set("a2-set", &payload);
        let paths = fetch_release(&origin.base, root.path(), set).await.unwrap();
        assert_eq!(paths, expected_paths(root.path(), set));
        for path in &paths {
            assert_eq!(fs::read(path).unwrap(), payload, "{}", path.display());
        }
        assert_eq!(
            local_status(root.path(), set),
            status(LocalState::Ready, 3 * LEN, 3 * LEN)
        );
        assert_eq!(ready_paths(root.path(), set), Some(paths.clone()));
        assert_eq!(origin.targets(), ["/a.fits", "/b.fits", "/c.fits"]);
        assert!(origin
            .requests()
            .iter()
            .all(|request| request.status == 302));
        assert_eq!(
            answered(&assets.requests()),
            NAMES
                .iter()
                .map(|name| (format!("{ASSET_PATH}/{name}"), 200))
                .collect::<Vec<_>>()
        );
        let again = fetch_release(&origin.base, root.path(), set).await.unwrap();
        assert_eq!(again, paths);
        let offline = fetch_release(OFFLINE, root.path(), set).await.unwrap();
        assert_eq!(offline, paths);
        assert_eq!(origin.requests().len(), 3);
        assert_eq!(assets.requests().len(), 3);
    }

    #[tokio::test]
    async fn a3_a_dropped_release_asset_resumes_through_a_fresh_redirect_with_a_range() {
        let payload = body(31);
        let assets = serve(Resource::new(payload.clone()).drop_after(DROP_AT));
        let origin = redirector(&assets.base, &NAMES);
        let root = tempfile::tempdir().unwrap();
        let set = release_set("a3-set", &payload);
        let first = fetch_release(&origin.base, root.path(), set)
            .await
            .unwrap_err();
        assert!(
            matches!(&first, SampleError::Network { file, .. } if file == "a.fits"),
            "{first:?}"
        );
        assert_eq!(
            local_status(root.path(), set),
            status(LocalState::Partial, DROP_AT, 3 * LEN)
        );
        let paths = fetch_release(&origin.base, root.path(), set).await.unwrap();
        assert_eq!(fs::read(&paths[0]).unwrap(), payload);
        assert_eq!(local_status(root.path(), set).state, LocalState::Ready);
        let resumed_origin = &origin.requests()[1];
        assert_eq!(resumed_origin.target, "/a.fits");
        assert_eq!(resumed_origin.header("range"), Some("bytes=102400-"));
        let resumed_asset = &assets.requests()[1];
        assert_eq!(resumed_asset.target, format!("{ASSET_PATH}/a.fits"));
        assert_eq!(resumed_asset.header("range"), Some("bytes=102400-"));
        assert_eq!(resumed_asset.header("if-range"), Some(ETAG_V0));
        assert_eq!(resumed_asset.status, 206);
    }

    #[tokio::test]
    async fn a4_a_release_asset_whose_bytes_differ_from_the_pin_is_refused_and_nothing_is_kept() {
        let payload = body(32);
        let assets = serve(Resource::new(payload.clone()).flip_byte_at(LEN / 2));
        let origin = redirector(&assets.base, &NAMES);
        let root = tempfile::tempdir().unwrap();
        let set = release_set("a4-set", &payload);
        let error = fetch_release(&origin.base, root.path(), set)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, SampleError::Checksum { file } if file == "a.fits"),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "a.fits: the download did not match the published checksum"
        );
        let status = local_status(root.path(), set);
        assert_eq!(status.state, LocalState::Absent);
        assert_eq!(status.received_bytes, 0);
        assert_eq!(ready_paths(root.path(), set), None);
        assert!(!set_dir(root.path(), set).join("a.fits").exists());
        assert_eq!(origin.targets(), ["/a.fits"]);
    }

    #[tokio::test]
    async fn a5_a_missing_release_asset_reads_not_available_on_github_with_the_404() {
        let payload = body(33);
        let assets = serve(Resource::new(payload.clone()));
        let nothing = redirector(&assets.base, &[]);
        let root = tempfile::tempdir().unwrap();
        let set = release_set("a5-set", &payload);
        let error = fetch_release(&nothing.base, root.path(), set)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, SampleError::NotPublished { file } if file == "a.fits"),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "a.fits: not available on GitHub (HTTP 404); the sample release may not be published yet"
        );
        assert_eq!(local_status(root.path(), set).state, LocalState::Absent);
        assert_eq!(nothing.targets(), ["/a.fits"]);
        assert!(assets.requests().is_empty());
        let first_only = redirector(&assets.base, &["a.fits"]);
        let error = fetch_release(&first_only.base, root.path(), set)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "b.fits: not available on GitHub (HTTP 404); the sample release may not be published yet"
        );
        assert_eq!(
            local_status(root.path(), set),
            status(LocalState::Partial, LEN, 3 * LEN)
        );
        assert_eq!(first_only.targets(), ["/a.fits", "/b.fits"]);
        let hosted = test_set("a5-hosted-set", &payload);
        let raw = fetch_set(&nothing.base, root.path(), hosted)
            .await
            .unwrap_err();
        assert_eq!(raw.to_string(), "a.fits: the server answered HTTP 404");
    }

    #[test]
    fn a6_release_assets_follow_redirects_only_between_github_release_hosts_or_loopback_under_the_override(
    ) {
        const GITHUB: &[&str] = &[
            "github.com",
            "objects.githubusercontent.com",
            "release-assets.githubusercontent.com",
        ];
        assert_eq!(RELEASE_ASSET_HOSTS, GITHUB);
        let production = release_profile(&pinned_origins());
        assert_eq!(production.redirect, RedirectRule::https(GITHUB));
        assert_eq!(
            HttpProfile {
                redirect: HttpProfile::transfer().redirect,
                ..production
            },
            HttpProfile::transfer()
        );
        let looped = release_profile(&released_at("http://127.0.0.1:9"));
        assert_eq!(
            looped.redirect,
            RedirectRule {
                https_only: false,
                max_hops: 3,
                hosts: &["127.0.0.1", "localhost", "[::1]"],
            }
        );
        assert!(HttpProfile::transfer().redirect.hosts.is_empty());
    }

    #[tokio::test]
    async fn a7_the_production_release_client_refuses_a_redirect_off_the_github_asset_host() {
        let payload = body(34);
        let assets = serve(Resource::new(payload.clone()));
        let origin = redirector(&assets.base, &NAMES);
        let root = tempfile::tempdir().unwrap();
        let set = release_set("a7-set", &payload);
        let client = build_client(&release_profile(&pinned_origins())).unwrap();
        let error = download_with(
            &client,
            &released_at(&origin.base),
            root.path(),
            set,
            &Recorder::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&error, SampleError::Redirected { file } if file == "a.fits"),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "a.fits: the server redirected the download to a host that AstroBurst does not download samples from"
        );
        assert_eq!(origin.targets(), ["/a.fits"]);
        assert!(assets.requests().is_empty());
        assert_eq!(local_status(root.path(), set).state, LocalState::Absent);
    }

    #[test]
    fn a8_the_sample_base_override_reaches_release_assets_only_on_loopback_http() {
        let cases = [
            ("http://127.0.0.1:8080", Some("http://127.0.0.1:8080")),
            (
                "http://localhost:8080/fits/",
                Some("http://localhost:8080/fits"),
            ),
            (" http://[::1]:8080 ", Some("http://[::1]:8080")),
            ("https://mirror.example.org/samples", None),
            ("http://192.168.1.10:8080", None),
            ("http://127.0.0.1:8080/?token=1", None),
            ("ftp://127.0.0.1/samples", None),
            ("", None),
            ("not a url", None),
        ];
        for (raw, release) in cases {
            assert_eq!(
                release_override(Some(raw.to_owned())).as_deref(),
                release,
                "{raw:?}"
            );
        }
        assert_eq!(release_override(None), None);
        let set = release_set("a8-set", &body(0));
        let mirrored = Origins::with_override(Some("https://mirror.example.org".to_owned()));
        assert_eq!(
            mirrored.hosted.as_deref(),
            Some("https://mirror.example.org")
        );
        assert_eq!(mirrored.release, None);
        assert_eq!(
            file_url(set, &mirrored, "a.fits").unwrap().as_str(),
            "https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/download/test-tag/a.fits"
        );
        let looped = Origins::with_override(Some("http://127.0.0.1:43123/".to_owned()));
        assert_eq!(looped.hosted.as_deref(), Some("http://127.0.0.1:43123"));
        assert_eq!(looped.release.as_deref(), Some("http://127.0.0.1:43123"));
        assert_eq!(
            file_url(set, &looped, "a.fits").unwrap().as_str(),
            "http://127.0.0.1:43123/a.fits"
        );
        assert_eq!(Origins::with_override(None).release, None);
    }

    #[test]
    #[ignore = "reads the data-v1 release assets from ASTROBURST_SAMPLE_ASSETS_DIR"]
    fn a9_the_release_asset_folder_matches_the_pins_and_opens_with_the_app_readers() {
        use crate::core::cube::companions::open_cube_set;
        use crate::infra::fits::spectrum1d::load_spectrum1d;
        use crate::infra::image_source::load_plane;
        use crate::types::image_ref::ImageRef;
        let dir = PathBuf::from(
            std::env::var("ASTROBURST_SAMPLE_ASSETS_DIR")
                .expect("set ASTROBURST_SAMPLE_ASSETS_DIR to the data-v1 folder"),
        );
        let release_sets: Vec<&SampleSet> = sample_sets()
            .iter()
            .filter(|set| matches!(set.source, SampleSource::ReleaseAsset { .. }))
            .collect();
        assert_eq!(release_sets.len(), 2);
        for set in release_sets {
            for file in set.files {
                let path = dir.join(file.name);
                let bytes = fs::read(&path).unwrap();
                assert_eq!(bytes.len() as u64, file.bytes, "{}", file.name);
                assert_eq!(sha_hex(&bytes), file.sha256_hex, "{}", file.name);
                drop(bytes);
                let key = path.to_string_lossy().into_owned();
                let opened = if file.name.ends_with("_s3d.fits") {
                    let cube = open_cube_set(&key).unwrap();
                    let geometry = &cube.sci.geometry;
                    assert!(geometry.naxis3 > 1 && cube.err.is_some(), "{}", file.name);
                    format!(
                        "cube {}x{}x{}, ERR {}, DQ {}",
                        geometry.naxis1,
                        geometry.naxis2,
                        geometry.naxis3,
                        cube.err.is_some(),
                        cube.dq.is_some()
                    )
                } else if file.name.ends_with("_x1d.fits") {
                    let spectrum = load_spectrum1d(&key, None).unwrap();
                    assert!(spectrum.flux.len() > 1, "{}", file.name);
                    format!(
                        "1-D spectrum {} points, ERR {}",
                        spectrum.flux.len(),
                        spectrum.err.is_some()
                    )
                } else {
                    let plane = load_plane(&ImageRef::parse(&key)).unwrap();
                    let (rows, cols) = plane.arr.dim();
                    assert!(rows > 1 && cols > 1, "{}", file.name);
                    format!("image {cols}x{rows}")
                };
                println!("{} {}: {opened}", set.id, file.name);
            }
        }
    }

    #[tokio::test]
    #[ignore = "asks github.com for a release download without reading its body"]
    async fn a10_github_redirects_a_release_download_to_the_listed_asset_host() {
        let url =
            format!("{RELEASE_DOWNLOAD_BASE_URL}/v0.7.0/astroburst-0.7.0-linux-x86_64.tar.gz");
        let unfollowed = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .head(&url)
            .send()
            .await
            .unwrap();
        assert_eq!(unfollowed.status().as_u16(), 302);
        let location = Url::parse(
            unfollowed
                .headers()
                .get(reqwest::header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(location.scheme(), "https");
        assert!(
            RELEASE_ASSET_HOSTS.contains(&location.host_str().unwrap()),
            "{:?}",
            location.host_str()
        );
        let client = build_client(&release_profile(&pinned_origins())).unwrap();
        let followed = client.head(&url).send().await.unwrap();
        assert_eq!(followed.status().as_u16(), 200);
        assert_eq!(followed.url().host_str(), location.host_str());
        println!(
            "302 to {}; final {} {}",
            location.host_str().unwrap(),
            followed.status(),
            followed.url().host_str().unwrap()
        );
    }

    #[tokio::test]
    #[ignore = "asks github.com for a release download of a transferred repository without reading its body"]
    async fn a11_the_production_release_client_follows_the_github_hop_of_a_transferred_repository()
    {
        let url = "https://github.com/zeit/hyper/releases/download/v4.0.0-canary.5/Hyper-4.0.0-canary.5-arm64.exe.blockmap";
        let client = build_client(&release_profile(&pinned_origins())).unwrap();
        let followed = client.head(url).send().await.unwrap();
        assert_eq!(followed.status().as_u16(), 200);
        let host = followed.url().host_str().unwrap().to_owned();
        assert!(RELEASE_ASSET_HOSTS.contains(&host.as_str()), "{host}");
        println!("final {} {host}", followed.status());
    }

    #[tokio::test]
    async fn a12_a_stray_newer_at_mast_marker_from_an_older_build_does_not_flag_a_release_asset_set(
    ) {
        let payload = body(35);
        let root = tempfile::tempdir().unwrap();
        let set = release_set("a12-set", &payload);
        let dir = set_dir(root.path(), set);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.fits.newer-at-mast"), ETAG_V0).unwrap();
        assert_eq!(
            local_status(root.path(), set),
            status(LocalState::Absent, 0, 3 * LEN)
        );
        let assets = serve(Resource::new(payload.clone()));
        let origin = redirector(&assets.base, &NAMES);
        let paths = fetch_release(&origin.base, root.path(), set).await.unwrap();
        assert_eq!(
            local_status(root.path(), set),
            status(LocalState::Ready, 3 * LEN, 3 * LEN)
        );
        assert_eq!(ready_paths(root.path(), set), Some(paths));
    }

    #[tokio::test]
    async fn r9_a_right_sized_frame_without_the_engine_record_is_not_complete() {
        let payload = body(8);
        let root = tempfile::tempdir().unwrap();
        let set = test_set("r9-set", &payload);
        let dir = set_dir(root.path(), set);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.fits"), vec![0u8; LEN as usize]).unwrap();
        fs::write(dir.join("b.fits.part"), vec![0u8; 10]).unwrap();
        fs::write(dir.join("c.fits.done.json"), b"{\"version\":1}").unwrap();
        fs::write(dir.join("c.fits.part.json"), b"{}").unwrap();
        assert_eq!(
            local_status(root.path(), set),
            status(LocalState::Absent, 0, 3 * LEN)
        );
        assert_eq!(ready_paths(root.path(), set), None);
        let server = serve(Resource::new(payload.clone()));
        let paths = fetch_set(&server.base, root.path(), set).await.unwrap();
        assert_eq!(paths, expected_paths(root.path(), set));
        for path in &paths {
            assert_eq!(fs::read(path).unwrap(), payload, "{}", path.display());
        }
        assert_eq!(local_status(root.path(), set).state, LocalState::Ready);
    }

    #[tokio::test]
    async fn r9b_a_missing_frame_or_a_record_with_another_checksum_is_fetched_again_without_a_probe(
    ) {
        let payload = body(9);
        let server = serve(Resource::new(payload.clone()));
        let root = tempfile::tempdir().unwrap();
        let set = test_set("r9b-set", &payload);
        let paths = fetch_set(&server.base, root.path(), set).await.unwrap();
        fs::remove_file(&paths[1]).unwrap();
        let record = PathBuf::from(format!("{}.done.json", paths[2].display()));
        let mut edited: serde_json::Value =
            serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
        edited["sha256_hex"] = serde_json::Value::String("00".repeat(32));
        fs::write(&record, serde_json::to_vec(&edited).unwrap()).unwrap();
        fs::write(
            PathBuf::from(format!("{}.part", paths[0].display())),
            vec![0u8; 7],
        )
        .unwrap();
        assert_eq!(
            local_status(root.path(), set),
            status(LocalState::Partial, LEN, 3 * LEN)
        );
        assert_eq!(ready_paths(root.path(), set), None);
        let again = fetch_set(&server.base, root.path(), set).await.unwrap();
        assert_eq!(again, paths);
        let requests = server.requests();
        assert_eq!(targets(&server)[3..], ["/b.fits", "/c.fits"]);
        assert!(requests[3..]
            .iter()
            .all(|request| request.header("range").is_none()
                && request.header("if-none-match").is_none()));
        assert_eq!(local_status(root.path(), set).state, LocalState::Ready);
        assert_eq!(fs::read(&paths[2]).unwrap(), payload);
    }

    #[tokio::test]
    async fn r10_only_the_bytes_the_engine_resumes_from_count_as_received() {
        const KEPT: u64 = 4 << 10;
        let payload = body(10);
        let server = serve(Resource::new(payload.clone()));
        let root = tempfile::tempdir().unwrap();
        let set = test_set("r10-set", &payload);
        let absent = status(LocalState::Absent, 0, 3 * LEN);
        let at_once = Recorder::cancelling_from(0);
        let error = download_set(&hosted_at(&server.base), root.path(), set, &at_once)
            .await
            .unwrap_err();
        assert!(matches!(error, SampleError::Cancelled), "{error:?}");
        let frame = set_dir(root.path(), set).join("a.fits");
        let (part, sidecar) = (sibling(&frame, ".part"), sibling(&frame, ".part.json"));
        assert_eq!(fs::metadata(&part).unwrap().len(), 0);
        assert_eq!(local_status(root.path(), set), absent);
        fs::write(&part, &payload[..2 * KEPT as usize]).unwrap();
        assert_eq!(local_status(root.path(), set), absent);
        edit_json(&sidecar, |json| json["received"] = KEPT.into());
        assert_eq!(
            local_status(root.path(), set),
            status(LocalState::Partial, KEPT, 3 * LEN)
        );
        edit_json(&sidecar, |json| json["expected_size"] = (LEN + 1).into());
        assert_eq!(local_status(root.path(), set), absent);
        edit_json(&sidecar, |json| json["expected_size"] = LEN.into());
        let resumed = Recorder::default();
        download_set(&hosted_at(&server.base), root.path(), set, &resumed)
            .await
            .unwrap();
        assert_eq!(resumed.first_received(), KEPT);
        let requests = server.requests();
        assert_eq!(requests[1].target, "/a.fits");
        assert_eq!(requests[1].header("range"), Some("bytes=4096-"));
        assert_eq!(requests[1].status, 206);
        assert_eq!(local_status(root.path(), set).state, LocalState::Ready);
    }

    fn answered(requests: &[Recorded]) -> Vec<(String, u16)> {
        requests
            .iter()
            .map(|request| (request.target.clone(), request.status))
            .collect()
    }
}
