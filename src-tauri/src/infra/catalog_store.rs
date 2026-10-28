use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
#[cfg(feature = "net")]
use std::net::IpAddr;
use std::ops::Deref;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(feature = "net")]
use reqwest::{Client, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::astrometry::healpix_cat::{
    CatHeader, CatType, HealpixCatalog, MmapSource, HEADER_LEN, SIRIL_ASTROMETRIC_FILE,
};
#[cfg(feature = "net")]
use crate::infra::net::download::{download, DownloadError, DownloadObserver, DownloadRequest};
#[cfg(feature = "net")]
use crate::infra::net::http::{build_client, HttpProfile, RedirectRule};
use crate::infra::net::paths;
use crate::types::config::AppConfig;

#[cfg(feature = "net")]
mod zenodo;

#[cfg(feature = "net")]
use zenodo::fetch_zenodo_record;
#[cfg(feature = "net")]
pub use zenodo::{parse_sha256sum, parse_zenodo_record, ZenodoListing};

pub const CATALOGS_DIR: &str = "catalogs";
pub const INSTALL_PROGRESS_EVENT: &str = "catalog-install-progress";
pub const ENV_CATALOG_BASE_URL: &str = "ASTROBURST_CATALOG_BASE_URL";
pub const ENV_TIER0_PATH: &str = "ASTROBURST_TIER0_PATH";
pub const DEFAULT_CATALOG_BASE_URL: &str =
    "https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/download/catalogs-v1/";
pub const FIELD_EXTERNAL_CATALOGS: &str = "external_catalogs";
pub const FIELD_SIRIL_CATALOG_DIR: &str = "siril_catalog_dir";
pub const TYCHO2_ID: &str = "tycho2-l6-v1";
pub const TYCHO2_FILE: &str = "tycho2-l6-v1.abcat";
pub const TYCHO2_BYTES: u64 = 15_541_742;
pub const TYCHO2_SHA256_HEX: &str =
    "bf01b058e7c9babe03d38d367c98d6a0dbb0e9785741a9c842d053574fa17076";
pub const TYCHO2_SHA256: [u8; 32] = sha256_from_hex(TYCHO2_SHA256_HEX);
pub const BUNDLED_RESOURCES_DIR: &str = "resources";
pub const TWOMASS_ID: &str = "2mass-l8-v1";
pub const TWOMASS_FILE: &str = "2mass-l8-v1.dat";
pub const TWOMASS_BYTES: u64 = 154_140_794;
pub const TWOMASS_SHA256_HEX: &str =
    "c9abb1b90124f310cb3b2875037a9fcb0cf94b977df59e21f9ceb9433eb05d96";
pub const TWOMASS_SHA256: [u8; 32] = sha256_from_hex(TWOMASS_SHA256_HEX);
pub const SIRIL_GAIA_ID: &str = "siril-gaia-astro-v1";
pub const SIRIL_GAIA_URL: &str =
    "https://zenodo.org/records/14692304/files/siril_cat_healpix8_astro.dat.bz2";
pub const SIRIL_GAIA_BZ2_BYTES: u64 = 1_142_276_749;
pub const ZENODO_RECORD_ID: &str = "14692304";
pub const ENV_ZENODO_RECORD_URL: &str = "ASTROBURST_ZENODO_RECORD_URL";
pub const DEFAULT_ZENODO_RECORD_URL: &str = "https://zenodo.org/api/records/14692304";
pub const SIRIL_GAIA_ARCHIVE_KEY: &str = "siril_cat_healpix8_astro.dat.bz2";
pub const SIRIL_GAIA_ARCHIVE_SUM_KEY: &str = "siril_cat_healpix8_astro.dat.bz2.sha256sum";
pub const SIRIL_GAIA_DECODED_SUM_KEY: &str = "uncompressed_files.sha256sum";
pub const ASTROMETRIC_RECORD_BYTES: u64 = 16;
pub const SIRIL_GAIA_LEVEL8_PIXELS: u64 = 12 << 16;
pub const SIRIL_GAIA_MAX_STARS_PER_PIXEL: u64 = 127;
pub const SIRIL_GAIA_DECODED_MAX_BYTES: u64 = HEADER_LEN as u64
    + SIRIL_GAIA_LEVEL8_PIXELS
        * (INDEX_ENTRY_BYTES + SIRIL_GAIA_MAX_STARS_PER_PIXEL * ASTROMETRIC_RECORD_BYTES);
pub const GAIA_TERMS: &str = "Gaia-derived data: Gaia DR3 (ESA/Gaia/DPAC), whose terms are CC BY-NC 3.0 IGO. Your copy of AstroBurst downloads it from Zenodo with your consent and uses it on this computer only; it is never redistributed or hosted by AstroBurst. Credit Gaia DR3 (Vallenari et al. 2023, ESA/Gaia/DPAC) wherever results use it.";
pub const VERIFIED_MARKER_SUFFIX: &str = ".sha256.ok";
pub const DIGEST_SIDECAR_SUFFIX: &str = ".sha256";
pub const DECODE_READ_BUFFER_BYTES: usize = 64 * 1024;
pub const DECODE_WRITE_BUFFER_BYTES: usize = 1 << 20;

const BZIP2_SUFFIX: &str = ".bz2";
const INDEX_ENTRY_BYTES: u64 = 4;
#[cfg(feature = "net")]
const CONSENT_NEEDED: &str = "downloading the Siril Gaia extract from Zenodo needs your consent: review its source, size and licence in Settings > Catalogs and confirm the download there";
#[cfg(feature = "net")]
const DECODING_SUFFIX: &str = ".decoding";
const ENGINE_PART_SUFFIX: &str = ".part";
const ENGINE_SIDECAR_SUFFIX: &str = ".part.json";
#[cfg(feature = "net")]
const ENGINE_RECORD_SUFFIX: &str = ".done.json";
const HASH_BUFFER_BYTES: usize = 1 << 20;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const MAX_ID_LEN: usize = 64;
#[cfg(not(feature = "net"))]
const NO_NETWORK: &str = "this build has no network support (cargo feature \"net\"), so it cannot download catalogues; point it at an existing file instead";
const SHA256_HEX_LEN: usize = 64;
const DEFAULT_HOLDER: &str = "a running solve or SPCC";
const INSTALL_HOLDER: &str = "another install or removal of this dataset";
#[cfg(feature = "net")]
const REDIRECT_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
    "zenodo.org",
];
const COMPACT_TYPES: &[CatType] = &[CatType::Compact6, CatType::Compact12];
const ASTROMETRIC_TYPES: &[CatType] = &[CatType::Astrometric];

const fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => panic!("a pinned sha-256 holds only hexadecimal digits"),
    }
}

pub const fn sha256_from_hex(text: &str) -> [u8; 32] {
    let bytes = text.as_bytes();
    assert!(
        bytes.len() == 64,
        "a pinned sha-256 holds 64 hexadecimal digits"
    );
    let mut digest = [0u8; 32];
    let mut index = 0;
    while index < 32 {
        digest[index] = (hex_nibble(bytes[2 * index]) << 4) | hex_nibble(bytes[2 * index + 1]);
        index += 1;
    }
    digest
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetKind {
    Tier0Bundled,
    Hosted,
    External,
    Cache { cap_bytes: u64 },
    Registered,
}

impl DatasetKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Tier0Bundled => "tier0_bundled",
            Self::Hosted => "hosted",
            Self::External => "external",
            Self::Cache { .. } => "cache",
            Self::Registered => "registered",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Bzip2 {
        decoded_size: Option<u64>,
        decoded_sha256: Option<[u8; 32]>,
    },
}

impl Compression {
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bzip2 { .. } => "bzip2",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderRule {
    pub cat_types: &'static [CatType],
    pub level: u8,
    pub whole_sky: bool,
}

impl HeaderRule {
    pub fn check(&self, header: &CatHeader) -> Result<(), String> {
        let found = header.cat_type();
        if !found.is_some_and(|cat_type| self.cat_types.contains(&cat_type)) {
            return Err(format!(
                "it holds type {} ({}), this slot needs {}",
                header.cat_type,
                found.map_or("unknown", CatType::name),
                self.wanted_types()
            ));
        }
        if header.healpix_level != self.level {
            return Err(format!(
                "it is HEALPix level {}, this slot needs level {}",
                header.healpix_level, self.level
            ));
        }
        if self.whole_sky && header.is_chunked() {
            return Err(format!(
                "it is chunk {} of a chunked catalogue, this slot needs the whole-sky file",
                header.chunk_id()
            ));
        }
        Ok(())
    }

    fn wanted_types(&self) -> String {
        self.cat_types
            .iter()
            .map(|cat_type| format!("type {} ({})", cat_type.code(), cat_type.name()))
            .collect::<Vec<_>>()
            .join(" or ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub kind: DatasetKind,
    pub file_name: &'static str,
    pub url: Option<String>,
    pub size_bytes: Option<u64>,
    pub sha256: Option<[u8; 32]>,
    pub compressed: Compression,
    pub header: Option<HeaderRule>,
    pub licence: &'static str,
    pub credit: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DatasetStatus {
    Missing,
    Partial {
        received: u64,
        total: Option<u64>,
    },
    Installed {
        path: PathBuf,
        verified: bool,
    },
    External {
        path: PathBuf,
        valid: bool,
        problem: Option<String>,
    },
    Cache {
        bytes: u64,
        files: u64,
        cap: u64,
    },
    Registered {
        bytes: u64,
        files: u64,
        folder: Option<PathBuf>,
        folder_present: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatasetReport {
    pub id: &'static str,
    pub title: &'static str,
    pub kind: &'static str,
    pub status: DatasetStatus,
    pub size_bytes: Option<u64>,
    pub installable: bool,
    pub compressed: &'static str,
    pub licence: &'static str,
    pub credit: &'static str,
    pub url_host: Option<String>,
    pub zenodo_record: Option<&'static str>,
    pub folder: PathBuf,
    pub folder_bytes: u64,
    pub free_space_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallOutcome {
    pub id: &'static str,
    pub path: PathBuf,
    pub bytes: u64,
    pub resumed_from: u64,
    pub sha256_hex: String,
    pub sha256_verified: bool,
    pub already_installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZenodoSource {
    pub record_url: String,
    pub decoded_max_bytes: u64,
}

impl ZenodoSource {
    pub fn from_env() -> Self {
        zenodo_source(std::env::var_os(ENV_ZENODO_RECORD_URL))
    }
}

pub fn zenodo_source(env: Option<OsString>) -> ZenodoSource {
    let record_url = env
        .and_then(|value| value.into_string().ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| DEFAULT_ZENODO_RECORD_URL.to_owned());
    ZenodoSource {
        record_url,
        decoded_max_bytes: SIRIL_GAIA_DECODED_MAX_BYTES,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ZenodoFile {
    pub key: String,
    pub size: u64,
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ZenodoOffer {
    pub id: &'static str,
    pub record_url: String,
    pub record_page: Option<String>,
    pub doi: Option<String>,
    pub title: Option<String>,
    pub record_licence: Option<String>,
    pub gaia_terms: &'static str,
    pub host: String,
    pub files: Vec<ZenodoFile>,
    pub archive_key: &'static str,
    pub archive_url: String,
    pub archive_bytes: u64,
    pub archive_sha256: Option<String>,
    pub decoded_sha256: Option<String>,
    pub decoded_max_bytes: u64,
    pub resume_from: u64,
    pub need_bytes: u64,
    pub free_space_bytes: Option<u64>,
    pub folder: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZenodoConsent {
    pub archive_bytes: u64,
    pub archive_sha256: Option<String>,
    pub licence_acknowledged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Removed {
    pub id: &'static str,
    pub freed_bytes: u64,
    pub forgot_external: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallStage {
    Download,
    Decode,
}

impl InstallStage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::Decode => "decode",
        }
    }
}

pub trait InstallObserver: Send + Sync {
    fn progress(&self, stage: InstallStage, done: u64, total: Option<u64>);
    fn is_cancelled(&self) -> bool;
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("unknown catalogue dataset {id:?}; known datasets: {known}")]
    UnknownDataset { id: String, known: String },
    #[error("catalogue dataset id {id:?} is refused: {reason}")]
    BadId { id: String, reason: &'static str },
    #[error("catalogue dataset {id} is already registered")]
    Duplicate { id: String },
    #[error("{id}: {reason}")]
    Refused { id: String, reason: String },
    #[error("{id} is in use by {holders}; try again when it has finished")]
    InUse { id: String, holders: String },
    #[error("{id} needs {need} bytes free in {}, but only {available} are available", .dir.display())]
    NoSpace {
        id: String,
        dir: PathBuf,
        need: u64,
        available: u64,
    },
    #[error("{id} failed its sha-256 check: expected {expected_hex}, received {actual_hex}")]
    Checksum {
        id: String,
        expected_hex: String,
        actual_hex: String,
    },
    #[error("{id}: the install was cancelled; what was downloaded is kept and the next install resumes from there")]
    Cancelled { id: String },
    #[error("{id} is not published yet: {url} answered HTTP 404; it can be installed once its release asset is uploaded")]
    NotPublished { id: String, url: String },
    #[cfg(feature = "net")]
    #[error("{id}: the download failed: {source}")]
    Download { id: String, source: DownloadError },
    #[error("{id}: {} cannot be used: {detail}", .path.display())]
    InvalidFile {
        id: String,
        path: PathBuf,
        detail: String,
    },
    #[error("{id}: bzip2 {stage} failed: {source}")]
    Decode {
        id: String,
        stage: &'static str,
        source: io::Error,
    },
    #[error("{id}: {action} {}: {source}", .path.display())]
    Io {
        id: String,
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

fn refused(id: &str, reason: impl Into<String>) -> StoreError {
    StoreError::Refused {
        id: id.to_owned(),
        reason: reason.into(),
    }
}

fn io_error(id: &str, action: &'static str, path: &Path, source: io::Error) -> StoreError {
    StoreError::Io {
        id: id.to_owned(),
        action,
        path: path.to_path_buf(),
        source,
    }
}

fn invalid_file(id: &str, path: &Path, detail: impl Into<String>) -> StoreError {
    StoreError::InvalidFile {
        id: id.to_owned(),
        path: path.to_path_buf(),
        detail: detail.into(),
    }
}

pub fn hosted_url(file_name: &str) -> String {
    let base = std::env::var(ENV_CATALOG_BASE_URL)
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| DEFAULT_CATALOG_BASE_URL.to_owned());
    if base.ends_with('/') {
        format!("{base}{file_name}")
    } else {
        format!("{base}/{file_name}")
    }
}

pub fn built_in_datasets() -> Vec<DatasetSpec> {
    vec![
        DatasetSpec {
            id: TYCHO2_ID,
            title: "Tycho-2 stars (bundled offline tier 0)",
            kind: DatasetKind::Tier0Bundled,
            file_name: TYCHO2_FILE,
            url: None,
            size_bytes: Some(TYCHO2_BYTES),
            sha256: Some(TYCHO2_SHA256),
            compressed: Compression::None,
            header: Some(HeaderRule {
                cat_types: COMPACT_TYPES,
                level: 6,
                whole_sky: true,
            }),
            licence: "CC BY-NC 3.0 IGO (CDS ReadMe of I/259, ESA) or BSD-2-Clause (Debian tycho2 package); licenses/tycho2-catalogue-NOTICE.txt gives both",
            credit: "Tycho-2 Catalogue, Høg et al. 2000, A&A 355, L27; data from the CDS, Strasbourg; Credit: ESA",
        },
        DatasetSpec {
            id: TWOMASS_ID,
            title: "2MASS star cut (hosted offline tier 1)",
            kind: DatasetKind::Hosted,
            file_name: TWOMASS_FILE,
            url: Some(hosted_url(TWOMASS_FILE)),
            size_bytes: Some(TWOMASS_BYTES),
            sha256: Some(TWOMASS_SHA256),
            compressed: Compression::None,
            header: Some(HeaderRule {
                cat_types: COMPACT_TYPES,
                level: 8,
                whole_sky: true,
            }),
            licence: "Public domain (2MASS data products)",
            credit: "Two Micron All Sky Survey: University of Massachusetts and IPAC/Caltech, funded by NASA and NSF",
        },
        DatasetSpec {
            id: SIRIL_GAIA_ID,
            title: "Gaia DR3 astrometric extract from Siril (existing file, folder or download)",
            kind: DatasetKind::External,
            file_name: SIRIL_ASTROMETRIC_FILE,
            url: Some(SIRIL_GAIA_URL.to_owned()),
            size_bytes: Some(SIRIL_GAIA_BZ2_BYTES),
            sha256: None,
            compressed: Compression::Bzip2 {
                decoded_size: None,
                decoded_sha256: None,
            },
            header: Some(HeaderRule {
                cat_types: ASTROMETRIC_TYPES,
                level: 8,
                whole_sky: true,
            }),
            licence: "Siril catalogue format specification: CC BY 4.0; data: Gaia DR3, ESA/Gaia/DPAC, CC BY-NC 3.0 IGO (Zenodo record 14692304 labels the extract CC BY 4.0); downloaded only with your consent, never bundled or hosted by AstroBurst",
            credit: "ESA/Gaia/DPAC; HEALPix extract published by the Siril project (Zenodo record 14692304)",
        },
    ]
}

static REGISTRY: LazyLock<RwLock<Vec<DatasetSpec>>> =
    LazyLock::new(|| RwLock::new(built_in_datasets()));

fn check_id(id: &str) -> Result<(), StoreError> {
    let bad = |reason| StoreError::BadId {
        id: id.to_owned(),
        reason,
    };
    if id.is_empty() || id.len() > MAX_ID_LEN {
        return Err(bad("an id holds 1 to 64 characters"));
    }
    let allowed = id.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'.'
    });
    if !allowed || id.starts_with(['-', '.']) || id.ends_with('.') || id.contains("..") {
        return Err(bad(
            "an id uses a-z, 0-9, '-' and single '.' between other characters, and does not start with '-'",
        ));
    }
    Ok(())
}

fn check_file_name(spec: &DatasetSpec) -> Result<(), StoreError> {
    let mut components = Path::new(spec.file_name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err(refused(
            spec.id,
            format!(
                "its file name {:?} is not a plain file name inside its dataset folder",
                spec.file_name
            ),
        )),
    }
}

pub fn register(spec: DatasetSpec) -> Result<(), StoreError> {
    check_id(spec.id)?;
    check_file_name(&spec)?;
    let mut specs = REGISTRY.write().unwrap_or_else(PoisonError::into_inner);
    if specs.iter().any(|known| known.id == spec.id) {
        return Err(StoreError::Duplicate {
            id: spec.id.to_owned(),
        });
    }
    specs.push(spec);
    Ok(())
}

pub fn registered() -> Vec<DatasetSpec> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

#[cfg(feature = "net")]
pub fn catalog_client() -> reqwest::Result<Client> {
    build_client(&HttpProfile {
        redirect: RedirectRule::https(REDIRECT_HOSTS),
        ..HttpProfile::transfer()
    })
}

pub fn chosen_data_root_missing(
    env_data_dir: Option<OsString>,
    config: &AppConfig,
    data_root: &Path,
) -> bool {
    let from_env = env_data_dir
        .as_deref()
        .is_some_and(|dir| Path::new(dir).is_absolute());
    let from_settings = config
        .data_dir_override
        .as_deref()
        .is_some_and(|dir| Path::new(dir.trim()).is_absolute());
    (from_env || from_settings) && !data_root.is_dir()
}

#[cfg(feature = "net")]
fn download_url(text: &str) -> Result<Url, String> {
    let url = Url::parse(text)
        .map_err(|error| format!("its download address {text:?} is not a URL ({error})"))?;
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    match url.scheme() {
        "https" => Ok(url),
        "http" if loopback => Ok(url),
        _ => Err(format!(
            "its download address {text} is refused: only https://, or http:// to this computer, is used"
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreRoots {
    pub data: PathBuf,
    pub catalogs: PathBuf,
    pub cache: PathBuf,
    pub tier0_dirs: Vec<PathBuf>,
}

impl StoreRoots {
    pub fn under(data_root: &Path, cache_root: &Path) -> Self {
        Self {
            data: data_root.to_path_buf(),
            catalogs: data_root.join(CATALOGS_DIR),
            cache: cache_root.to_path_buf(),
            tier0_dirs: Vec::new(),
        }
    }

    pub fn with_tier0_dir(mut self, dir: PathBuf) -> Self {
        self.tier0_dirs.push(dir);
        self
    }

    pub fn from_app(resource_dir: Option<&Path>) -> anyhow::Result<Self> {
        let mut roots = Self::under(&paths::try_data_root()?, &paths::cache_root());
        roots.tier0_dirs = tier0_dirs(std::env::var_os(ENV_TIER0_PATH), resource_dir);
        Ok(roots)
    }
}

fn tier0_dirs(env: Option<OsString>, resource_dir: Option<&Path>) -> Vec<PathBuf> {
    env.filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .into_iter()
        .chain(resource_dir.into_iter().flat_map(|dir| {
            [
                dir.join(BUNDLED_RESOURCES_DIR).join(CATALOGS_DIR),
                dir.join(CATALOGS_DIR),
            ]
        }))
        .collect()
}

type DiskStamp = Option<(u64, Option<SystemTime>)>;

fn disk_stamp(path: &Path) -> DiskStamp {
    fs::metadata(path)
        .ok()
        .map(|meta| (meta.len(), meta.modified().ok()))
}

struct OpenEntry {
    catalog: Arc<HealpixCatalog<MmapSource>>,
    stamp: DiskStamp,
    holders: Vec<(u64, &'static str)>,
}

type OpenCatalogues = HashMap<PathBuf, OpenEntry>;

static OPEN: LazyLock<Mutex<OpenCatalogues>> = LazyLock::new(Mutex::default);
static INSTALLING: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(Mutex::default);
static NEXT_LEASE: AtomicU64 = AtomicU64::new(1);

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct CatalogLease {
    path: PathBuf,
    token: u64,
    catalog: Arc<HealpixCatalog<MmapSource>>,
}

impl CatalogLease {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Deref for CatalogLease {
    type Target = HealpixCatalog<MmapSource>;

    fn deref(&self) -> &Self::Target {
        &self.catalog
    }
}

impl Drop for CatalogLease {
    fn drop(&mut self) {
        let mut open = locked(&OPEN);
        let idle = open.get_mut(&self.path).is_some_and(|entry| {
            entry.holders.retain(|(token, _)| *token != self.token);
            entry.holders.is_empty()
        });
        if idle {
            open.remove(&self.path);
        }
    }
}

fn release_mappings(
    id: &str,
    covers: impl Fn(&Path) -> bool,
) -> Result<MutexGuard<'static, OpenCatalogues>, StoreError> {
    let mut open = locked(&OPEN);
    let covered: Vec<PathBuf> = open.keys().filter(|path| covers(path)).cloned().collect();
    let mut holders: Vec<&'static str> = Vec::new();
    let mut held = false;
    for entry in covered.iter().filter_map(|path| open.get(path)) {
        if Arc::strong_count(&entry.catalog) > 1 || !entry.holders.is_empty() {
            held = true;
            for (_, name) in &entry.holders {
                if !holders.contains(name) {
                    holders.push(name);
                }
            }
        }
    }
    if held {
        return Err(StoreError::InUse {
            id: id.to_owned(),
            holders: if holders.is_empty() {
                DEFAULT_HOLDER.to_owned()
            } else {
                holders.join(", ")
            },
        });
    }
    for path in &covered {
        open.remove(path);
    }
    Ok(open)
}

struct InstallGuard(PathBuf);

impl InstallGuard {
    fn take(dir: &Path, id: &str) -> Result<Self, StoreError> {
        if locked(&INSTALLING).insert(dir.to_path_buf()) {
            Ok(Self(dir.to_path_buf()))
        } else {
            Err(StoreError::InUse {
                id: id.to_owned(),
                holders: INSTALL_HOLDER.to_owned(),
            })
        }
    }
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        locked(&INSTALLING).remove(&self.0);
    }
}

fn is_installing(dir: &Path) -> bool {
    locked(&INSTALLING).contains(dir)
}

#[cfg(feature = "net")]
struct DownloadPhase<'a>(&'a dyn InstallObserver);

#[cfg(feature = "net")]
impl DownloadObserver for DownloadPhase<'_> {
    fn progress(&self, received: u64, total: Option<u64>) {
        self.0.progress(InstallStage::Download, received, total);
    }

    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

#[cfg(feature = "net")]
struct InstallPlan {
    url: Url,
    size: u64,
    sha256: Option<[u8; 32]>,
    decode: Option<DecodePlan>,
}

#[cfg(feature = "net")]
#[derive(Debug, Clone, Copy)]
struct DecodePlan {
    size: Option<u64>,
    cap: u64,
    sha256: Option<[u8; 32]>,
}

#[cfg(feature = "net")]
impl InstallPlan {
    fn final_bytes(&self) -> (Option<u64>, Option<[u8; 32]>) {
        match self.decode {
            None => (Some(self.size), self.sha256),
            Some(decode) => (decode.size, decode.sha256),
        }
    }

    fn checks_sha256(&self) -> bool {
        self.sha256.is_some() || self.final_bytes().1.is_some()
    }
}

#[cfg(feature = "net")]
fn install_plan(spec: &DatasetSpec) -> Result<InstallPlan, StoreError> {
    let missing = |what: &str| refused(spec.id, format!("its {what} is not published yet"));
    match spec.kind {
        DatasetKind::Tier0Bundled => {
            return Err(refused(
                spec.id,
                "it ships with the app and is never downloaded",
            ))
        }
        DatasetKind::Cache { .. } => {
            return Err(refused(spec.id, "it is a cache that its own module fills"))
        }
        DatasetKind::Registered => {
            return Err(refused(
                spec.id,
                "its own module fills and reads its folder",
            ))
        }
        DatasetKind::Hosted | DatasetKind::External => {}
    }
    let text = spec
        .url
        .as_deref()
        .ok_or_else(|| refused(spec.id, "it has no download address"))?;
    let url = download_url(text).map_err(|reason| refused(spec.id, reason))?;
    let size = spec.size_bytes.ok_or_else(|| missing("download size"))?;
    let sha256 = spec.sha256.ok_or_else(|| missing("download sha-256"))?;
    let decode = match spec.compressed {
        Compression::None => None,
        Compression::Bzip2 {
            decoded_size,
            decoded_sha256,
        } => {
            let size = decoded_size.ok_or_else(|| missing("decoded size"))?;
            Some(DecodePlan {
                size: Some(size),
                cap: size,
                sha256: Some(decoded_sha256.ok_or_else(|| missing("decoded sha-256"))?),
            })
        }
    };
    Ok(InstallPlan {
        url,
        size,
        sha256: Some(sha256),
        decode,
    })
}

#[cfg(feature = "net")]
fn is_installable(spec: &DatasetSpec) -> bool {
    install_plan(spec).is_ok()
}

#[cfg(not(feature = "net"))]
fn is_installable(_spec: &DatasetSpec) -> bool {
    false
}

#[cfg(feature = "net")]
fn url_host(text: &str) -> Option<String> {
    Url::parse(text)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
}

#[cfg(not(feature = "net"))]
fn url_host(_text: &str) -> Option<String> {
    None
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct VerifiedMarker {
    path: String,
    size: u64,
    modified_unix_ms: Option<u64>,
    sha256: String,
    #[serde(default = "checked_before_the_flag_existed")]
    checked: bool,
}

fn checked_before_the_flag_existed() -> bool {
    true
}

#[derive(Deserialize)]
struct EnginePart {
    received: u64,
    expected_size: Option<u64>,
}

pub struct Decoded<W> {
    pub writer: W,
    pub bytes: u64,
    pub sha256: [u8; 32],
}

pub fn decode_bzip2<R: Read, W: Write>(
    id: &str,
    compressed: R,
    out: W,
    observer: &dyn InstallObserver,
    total: Option<u64>,
    limit: u64,
) -> Result<Decoded<W>, StoreError> {
    let failed = |stage: &'static str, source: io::Error| StoreError::Decode {
        id: id.to_owned(),
        stage,
        source,
    };
    let mut decoder = bzip2::bufread::MultiBzDecoder::new(BufReader::with_capacity(
        DECODE_READ_BUFFER_BYTES,
        compressed,
    ));
    let mut writer = BufWriter::with_capacity(DECODE_WRITE_BUFFER_BYTES, out);
    let mut hasher = Sha256::new();
    let mut block = vec![0u8; DECODE_READ_BUFFER_BYTES];
    let mut bytes = 0u64;
    let mut reported: Option<Instant> = None;
    loop {
        if observer.is_cancelled() {
            return Err(StoreError::Cancelled { id: id.to_owned() });
        }
        let read = match decoder.read(&mut block) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(failed("decoding", error)),
        };
        let chunk = block.get(..read).ok_or_else(|| {
            failed(
                "decoding",
                io::Error::other("the decoder reported more bytes than its buffer holds"),
            )
        })?;
        bytes += chunk.len() as u64;
        if bytes > limit {
            return Err(failed(
                "decoding",
                io::Error::other(format!(
                    "the archive decodes past {limit} bytes, the most this catalogue can hold"
                )),
            ));
        }
        hasher.update(chunk);
        writer
            .write_all(chunk)
            .map_err(|error| failed("writing", error))?;
        if reported.is_none_or(|at| at.elapsed() >= PROGRESS_INTERVAL) {
            observer.progress(InstallStage::Decode, bytes, total);
            reported = Some(Instant::now());
        }
    }
    let writer = writer
        .into_inner()
        .map_err(|error| failed("writing", error.into_error()))?;
    observer.progress(InstallStage::Decode, bytes, total);
    Ok(Decoded {
        writer,
        bytes,
        sha256: hasher.finalize().into(),
    })
}

#[cfg(feature = "net")]
struct DecodeJob {
    id: &'static str,
    source: PathBuf,
    target: PathBuf,
    plan: DecodePlan,
}

#[cfg(feature = "net")]
impl DecodeJob {
    fn run(self, observer: &dyn InstallObserver) -> Result<String, StoreError> {
        let temp = sibling(&self.target, DECODING_SUFFIX);
        let decoded = self.decode_into(&temp, observer);
        if decoded.is_err() {
            remove_if_present(&temp).map_err(|error| {
                io_error(self.id, "cannot delete the partial decode", &temp, error)
            })?;
        }
        decoded
    }

    fn decode_into(
        &self,
        temp: &Path,
        observer: &dyn InstallObserver,
    ) -> Result<String, StoreError> {
        let source = File::open(&self.source)
            .map_err(|error| io_error(self.id, "cannot open", &self.source, error))?;
        let target =
            File::create(temp).map_err(|error| io_error(self.id, "cannot create", temp, error))?;
        let decoded = decode_bzip2(
            self.id,
            source,
            target,
            observer,
            self.plan.size,
            self.plan.cap,
        )?;
        if let Some(size) = self.plan.size.filter(|&size| size != decoded.bytes) {
            return Err(invalid_file(
                self.id,
                &self.source,
                format!("it decodes to {} bytes, {size} expected", decoded.bytes),
            ));
        }
        if let Some(expected) = self.plan.sha256.filter(|&digest| digest != decoded.sha256) {
            return Err(StoreError::Checksum {
                id: self.id.to_owned(),
                expected_hex: hex(&expected),
                actual_hex: hex(&decoded.sha256),
            });
        }
        decoded
            .writer
            .sync_all()
            .map_err(|error| io_error(self.id, "cannot flush", temp, error))?;
        drop(decoded.writer);
        fs::rename(temp, &self.target).map_err(|error| {
            io_error(
                self.id,
                "cannot move the decoded file to",
                &self.target,
                error,
            )
        })?;
        discard(
            self.id,
            &[
                self.source.clone(),
                sibling(&self.source, ENGINE_RECORD_SUFFIX),
            ],
        )?;
        Ok(hex(&decoded.sha256))
    }
}

pub type FreeSpace = Arc<dyn Fn(&Path) -> io::Result<u64> + Send + Sync>;

pub struct CatalogStore {
    roots: StoreRoots,
    datasets: Vec<DatasetSpec>,
    free_space: FreeSpace,
}

impl CatalogStore {
    pub fn new(roots: StoreRoots) -> Self {
        Self {
            roots,
            datasets: registered(),
            free_space: Arc::new(paths::free_space),
        }
    }

    #[cfg(all(test, feature = "net"))]
    pub(crate) fn with_dataset(mut self, spec: DatasetSpec) -> Self {
        self.datasets.retain(|known| known.id != spec.id);
        self.datasets.push(spec);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_free_space(mut self, free_space: FreeSpace) -> Self {
        self.free_space = free_space;
        self
    }

    pub fn roots(&self) -> &StoreRoots {
        &self.roots
    }

    pub fn datasets(&self) -> &[DatasetSpec] {
        &self.datasets
    }

    pub fn spec(&self, id: &str) -> Result<&DatasetSpec, StoreError> {
        self.datasets
            .iter()
            .find(|spec| spec.id == id)
            .ok_or_else(|| StoreError::UnknownDataset {
                id: id.to_owned(),
                known: self
                    .datasets
                    .iter()
                    .map(|spec| spec.id)
                    .collect::<Vec<_>>()
                    .join(", "),
            })
    }

    pub fn dataset_dir(&self, spec: &DatasetSpec) -> PathBuf {
        match spec.kind {
            DatasetKind::Cache { .. } => self.roots.cache.join(CATALOGS_DIR).join(spec.id),
            _ => self.roots.catalogs.join(spec.id),
        }
    }

    fn installed_path(&self, spec: &DatasetSpec) -> PathBuf {
        self.dataset_dir(spec).join(spec.file_name)
    }

    fn download_dest(&self, spec: &DatasetSpec) -> PathBuf {
        let installed = self.installed_path(spec);
        match spec.compressed {
            Compression::None => installed,
            Compression::Bzip2 { .. } => sibling(&installed, BZIP2_SUFFIX),
        }
    }

    fn marker_path(&self, spec: &DatasetSpec) -> PathBuf {
        self.dataset_dir(spec)
            .join(format!("{}{VERIFIED_MARKER_SUFFIX}", spec.id))
    }

    fn tier0_file(&self, spec: &DatasetSpec) -> Option<PathBuf> {
        self.roots
            .tier0_dirs
            .iter()
            .filter_map(|candidate| {
                if candidate.is_file() {
                    (candidate.file_name() == Some(std::ffi::OsStr::new(spec.file_name)))
                        .then(|| candidate.clone())
                } else {
                    Some(candidate.join(spec.file_name))
                }
            })
            .chain(std::iter::once(self.installed_path(spec)))
            .find(|path| path.is_file())
    }

    fn external_path(&self, spec: &DatasetSpec, config: &AppConfig) -> Option<PathBuf> {
        if spec.kind != DatasetKind::External {
            return None;
        }
        config
            .external_catalogs
            .get(spec.id)
            .map(PathBuf::from)
            .or_else(|| {
                config
                    .siril_catalog_dir
                    .as_deref()
                    .filter(|_| spec.id == SIRIL_GAIA_ID)
                    .map(|dir| Path::new(dir).join(spec.file_name))
            })
    }

    fn expected_digest(&self, spec: &DatasetSpec, path: &Path) -> Option<[u8; 32]> {
        let published = match spec.compressed {
            Compression::None => spec.sha256,
            Compression::Bzip2 { decoded_sha256, .. } => decoded_sha256,
        };
        published.or_else(|| {
            (spec.kind == DatasetKind::Tier0Bundled)
                .then(|| fs::read_to_string(sibling(path, DIGEST_SIDECAR_SUFFIX)).ok())
                .flatten()
                .and_then(|text| parse_digest(&text))
        })
    }

    fn current_marker(&self, spec: &DatasetSpec, path: &Path) -> Option<VerifiedMarker> {
        let stored = read_json::<VerifiedMarker>(&self.marker_path(spec))?;
        let current = marker_for(path, &stored.sha256, stored.checked).ok()?;
        let published = self
            .expected_digest(spec, path)
            .is_none_or(|expected| hex(&expected) == stored.sha256);
        (stored == current && published).then_some(stored)
    }

    fn is_verified(&self, spec: &DatasetSpec, path: &Path) -> bool {
        self.current_marker(spec, path).is_some_and(|marker| marker.checked)
    }

    fn write_marker(
        &self,
        spec: &DatasetSpec,
        path: &Path,
        sha256_hex: &str,
        checked: bool,
    ) -> Result<(), StoreError> {
        let marker_path = self.marker_path(spec);
        let marker = marker_for(path, sha256_hex, checked)
            .map_err(|error| io_error(spec.id, "cannot read", path, error))?;
        let bytes = serde_json::to_vec_pretty(&marker).map_err(|error| {
            io_error(
                spec.id,
                "cannot encode",
                &marker_path,
                io::Error::other(error),
            )
        })?;
        let dir = self.dataset_dir(spec);
        fs::create_dir_all(&dir)
            .map_err(|error| io_error(spec.id, "cannot create", &dir, error))?;
        fs::write(&marker_path, bytes)
            .map_err(|error| io_error(spec.id, "cannot write", &marker_path, error))
    }

    fn ensure_verified(&self, spec: &DatasetSpec, path: &Path) -> Result<(), StoreError> {
        if self.is_verified(spec, path) {
            return Ok(());
        }
        let expected = self.expected_digest(spec, path).ok_or_else(|| {
            invalid_file(spec.id, path, "no published sha-256 to verify it against")
        })?;
        let actual =
            hash_file(path).map_err(|error| io_error(spec.id, "cannot read", path, error))?;
        if actual != expected {
            return Err(StoreError::Checksum {
                id: spec.id.to_owned(),
                expected_hex: hex(&expected),
                actual_hex: hex(&actual),
            });
        }
        if let Err(error) = self.write_marker(spec, path, &hex(&actual), true) {
            log::warn!(
                "{}: {} matches its sha-256, but the check could not be recorded, so it runs again at the next open: {error}",
                spec.id,
                path.display()
            );
        }
        Ok(())
    }

    fn check_file(&self, spec: &DatasetSpec, path: &Path) -> Result<(), String> {
        if !path.is_file() {
            return Err("the file does not exist".to_owned());
        }
        let catalog = HealpixCatalog::open_file(path).map_err(|error| error.to_string())?;
        match spec.header {
            Some(rule) => rule.check(catalog.header()),
            None => Ok(()),
        }
    }

    fn download_status(&self, spec: &DatasetSpec) -> DatasetStatus {
        let installed = self.installed_path(spec);
        if installed.is_file() {
            return DatasetStatus::Installed {
                verified: self.is_verified(spec, &installed),
                path: installed,
            };
        }
        let dest = self.download_dest(spec);
        let complete_download = fs::metadata(&dest)
            .ok()
            .filter(|meta| dest != installed && meta.is_file());
        if let Some(meta) = complete_download {
            return DatasetStatus::Partial {
                received: meta.len(),
                total: spec.size_bytes.or(Some(meta.len())),
            };
        }
        match partial_download(&dest) {
            Some((received, total)) => DatasetStatus::Partial {
                received,
                total: total.or(spec.size_bytes),
            },
            None => DatasetStatus::Missing,
        }
    }

    fn status_of(&self, spec: &DatasetSpec, config: &AppConfig) -> DatasetStatus {
        match spec.kind {
            DatasetKind::Cache { cap_bytes } => {
                let (bytes, files) = dir_usage(&self.dataset_dir(spec));
                DatasetStatus::Cache {
                    bytes,
                    files,
                    cap: cap_bytes,
                }
            }
            DatasetKind::Tier0Bundled => match self.tier0_file(spec) {
                Some(path) => DatasetStatus::Installed {
                    verified: self.is_verified(spec, &path),
                    path,
                },
                None => DatasetStatus::Missing,
            },
            DatasetKind::Registered => {
                let (bytes, files) = dir_usage(&self.dataset_dir(spec));
                let folder = config.external_catalogs.get(spec.id).map(PathBuf::from);
                DatasetStatus::Registered {
                    bytes,
                    files,
                    folder_present: folder.as_deref().is_some_and(Path::is_dir),
                    folder,
                }
            }
            DatasetKind::Hosted => self.download_status(spec),
            DatasetKind::External => match self.external_path(spec, config) {
                Some(path) => {
                    let problem = self.check_file(spec, &path).err();
                    DatasetStatus::External {
                        valid: problem.is_none(),
                        problem,
                        path,
                    }
                }
                None => self.download_status(spec),
            },
        }
    }

    pub fn status(&self, id: &str, config: &AppConfig) -> Result<DatasetStatus, StoreError> {
        Ok(self.status_of(self.spec(id)?, config))
    }

    pub fn report(&self, config: &AppConfig) -> Vec<DatasetReport> {
        self.datasets
            .iter()
            .map(|spec| {
                let folder = self.dataset_dir(spec);
                DatasetReport {
                    id: spec.id,
                    title: spec.title,
                    kind: spec.kind.name(),
                    status: self.status_of(spec, config),
                    size_bytes: spec.size_bytes,
                    installable: is_installable(spec),
                    compressed: spec.compressed.name(),
                    licence: spec.licence,
                    credit: spec.credit,
                    url_host: spec.url.as_deref().and_then(url_host),
                    zenodo_record: (cfg!(feature = "net") && spec.id == SIRIL_GAIA_ID)
                        .then_some(ZENODO_RECORD_ID),
                    free_space_bytes: (self.free_space)(&folder).ok(),
                    folder_bytes: dir_usage(&folder).0,
                    folder,
                }
            })
            .collect()
    }

    #[cfg(feature = "net")]
    pub async fn install(
        &self,
        id: &str,
        client: &Client,
        observer: Arc<dyn InstallObserver>,
    ) -> Result<InstallOutcome, StoreError> {
        let spec = self.spec(id)?.clone();
        let plan = install_plan(&spec)?;
        self.run_install(spec, plan, client, observer).await
    }

    #[cfg(feature = "net")]
    async fn run_install(
        &self,
        spec: DatasetSpec,
        plan: InstallPlan,
        client: &Client,
        observer: Arc<dyn InstallObserver>,
    ) -> Result<InstallOutcome, StoreError> {
        let dir = self.dataset_dir(&spec);
        let _installing = InstallGuard::take(&dir, spec.id)?;
        drop(release_mappings(spec.id, |path| path.starts_with(&dir))?);
        let installed = self.installed_path(&spec);
        discard(spec.id, &[sibling(&installed, DECODING_SUFFIX)])?;
        let (final_size, final_digest) = plan.final_bytes();
        let sha256_verified = plan.checks_sha256();
        if installed.is_file() {
            if let Some(sha256_hex) = self
                .installed_digest(&spec, &installed, final_size, final_digest)
                .await?
            {
                let bytes = file_len(spec.id, &installed)?;
                return Ok(InstallOutcome {
                    id: spec.id,
                    path: installed,
                    bytes,
                    resumed_from: bytes,
                    sha256_hex,
                    sha256_verified,
                    already_installed: true,
                });
            }
            discard(
                spec.id,
                &[
                    installed.clone(),
                    sibling(&installed, ENGINE_RECORD_SUFFIX),
                    self.marker_path(&spec),
                ],
            )?;
        }
        let dest = self.download_dest(&spec);
        let kept_download = dest != installed
            && dest.is_file()
            && holds_published_bytes(spec.id, &dest, Some(plan.size), plan.sha256).await?;
        if dest != installed && !kept_download {
            discard(
                spec.id,
                &[dest.clone(), sibling(&dest, ENGINE_RECORD_SUFFIX)],
            )?;
        }
        let need = plan
            .size
            .saturating_sub(already_downloaded(&dest))
            .saturating_add(plan.decode.map_or(0, |decode| decode.cap));
        let available = (self.free_space)(&dir)
            .map_err(|error| io_error(spec.id, "cannot measure the free space of", &dir, error))?;
        if available < need {
            return Err(StoreError::NoSpace {
                id: spec.id.to_owned(),
                dir,
                need,
                available,
            });
        }
        let resumed_from = if kept_download {
            plan.size
        } else {
            let request = DownloadRequest {
                url: plan.url.clone(),
                dest: dest.clone(),
                expected_size: Some(plan.size),
                expected_sha256: plan.sha256,
                expected_etag: None,
                headers: Vec::new(),
                max_bytes: plan.size,
                overwrite: true,
            };
            let outcome = download(client, &request, &DownloadPhase(observer.as_ref()))
                .await
                .map_err(|error| download_failure(spec.id, &request.url, &dest, error))?;
            let confirmed = plan.sha256.is_none_or(|expected| {
                outcome.sha256_hex.as_deref() == Some(hex(&expected).as_str())
            });
            if !confirmed {
                return Err(invalid_file(
                    spec.id,
                    &dest,
                    "the download engine did not confirm its sha-256, so it is not marked verified",
                ));
            }
            outcome.resumed_from
        };
        let sha256_hex = match plan.decode {
            None => match plan.sha256 {
                Some(expected) => hex(&expected),
                None => hex(&hash_off_thread(spec.id, &dest).await?),
            },
            Some(decode) => {
                let job = DecodeJob {
                    id: spec.id,
                    source: dest.clone(),
                    target: installed.clone(),
                    plan: decode,
                };
                let observer = Arc::clone(&observer);
                tokio::task::spawn_blocking(move || job.run(observer.as_ref()))
                    .await
                    .map_err(|error| {
                        io_error(
                            spec.id,
                            "the decode task failed for",
                            &dest,
                            io::Error::other(error),
                        )
                    })??
            }
        };
        self.seal(&spec, &installed, &sha256_hex, sha256_verified)?;
        let bytes = file_len(spec.id, &installed)?;
        Ok(InstallOutcome {
            id: spec.id,
            path: installed,
            bytes,
            resumed_from,
            sha256_hex,
            sha256_verified,
            already_installed: false,
        })
    }

    #[cfg(feature = "net")]
    async fn installed_digest(
        &self,
        spec: &DatasetSpec,
        installed: &Path,
        size: Option<u64>,
        digest: Option<[u8; 32]>,
    ) -> Result<Option<String>, StoreError> {
        if let Some(marker) = self.current_marker(spec, installed) {
            return Ok(Some(marker.sha256));
        }
        let Some(expected) = digest else {
            return Ok(None);
        };
        if !holds_published_bytes(spec.id, installed, size, Some(expected)).await? {
            return Ok(None);
        }
        let sha256_hex = hex(&expected);
        self.seal(spec, installed, &sha256_hex, true)?;
        Ok(Some(sha256_hex))
    }

    #[cfg(not(feature = "net"))]
    pub async fn install(
        &self,
        id: &str,
        _observer: Arc<dyn InstallObserver>,
    ) -> Result<InstallOutcome, StoreError> {
        Err(refused(self.spec(id)?.id, NO_NETWORK))
    }

    #[cfg(feature = "net")]
    pub async fn zenodo_offer(
        &self,
        client: &Client,
        source: &ZenodoSource,
    ) -> Result<ZenodoOffer, StoreError> {
        let spec = self.spec(SIRIL_GAIA_ID)?;
        let record = fetch_zenodo_record(spec.id, client, source).await?;
        let folder = self.dataset_dir(spec);
        let resume_from = already_downloaded(&self.download_dest(spec)).min(record.archive_bytes);
        Ok(ZenodoOffer {
            id: spec.id,
            host: record.record_url.host_str().unwrap_or_default().to_owned(),
            record_url: record.record_url.to_string(),
            record_page: record.listing.page,
            doi: record.listing.doi,
            title: record.listing.title,
            record_licence: record.listing.licence,
            gaia_terms: GAIA_TERMS,
            files: record
                .listing
                .files
                .into_iter()
                .map(|(file, _)| file)
                .collect(),
            archive_key: SIRIL_GAIA_ARCHIVE_KEY,
            archive_url: record.archive_url.to_string(),
            archive_bytes: record.archive_bytes,
            archive_sha256: record.archive_sha256.map(|digest| hex(&digest)),
            decoded_sha256: record.decoded_sha256.map(|digest| hex(&digest)),
            decoded_max_bytes: source.decoded_max_bytes,
            resume_from,
            need_bytes: (record.archive_bytes - resume_from).saturating_add(source.decoded_max_bytes),
            free_space_bytes: (self.free_space)(&folder).ok(),
            folder,
        })
    }

    #[cfg(not(feature = "net"))]
    pub async fn zenodo_offer(&self) -> Result<ZenodoOffer, StoreError> {
        Err(refused(self.spec(SIRIL_GAIA_ID)?.id, NO_NETWORK))
    }

    #[cfg(feature = "net")]
    pub async fn install_siril_from_zenodo(
        &self,
        client: &Client,
        source: &ZenodoSource,
        consent: Option<&ZenodoConsent>,
        observer: Arc<dyn InstallObserver>,
    ) -> Result<InstallOutcome, StoreError> {
        let base = self.spec(SIRIL_GAIA_ID)?.clone();
        let consent = consent
            .filter(|consent| consent.licence_acknowledged)
            .ok_or_else(|| refused(base.id, CONSENT_NEEDED))?;
        let record = fetch_zenodo_record(base.id, client, source).await?;
        record.check_consent(base.id, consent)?;
        let plan = InstallPlan {
            url: record.archive_url.clone(),
            size: record.archive_bytes,
            sha256: record.archive_sha256,
            decode: Some(DecodePlan {
                size: None,
                cap: source.decoded_max_bytes,
                sha256: record.decoded_sha256,
            }),
        };
        let spec = DatasetSpec {
            compressed: Compression::Bzip2 {
                decoded_size: None,
                decoded_sha256: record.decoded_sha256,
            },
            ..base
        };
        self.run_install(spec, plan, client, observer).await
    }

    pub fn use_downloaded_siril(&self, config: &mut AppConfig) -> Result<PathBuf, StoreError> {
        let spec = self.spec(SIRIL_GAIA_ID)?;
        let file = self.use_siril_dir(&self.dataset_dir(spec), config)?;
        config.external_catalogs.remove(spec.id);
        Ok(file)
    }

    #[cfg(feature = "net")]
    fn seal(
        &self,
        spec: &DatasetSpec,
        path: &Path,
        sha256_hex: &str,
        checked: bool,
    ) -> Result<(), StoreError> {
        if let Some(rule) = spec.header {
            let catalog = HealpixCatalog::open_file(path)
                .map_err(|error| invalid_file(spec.id, path, error.to_string()))?;
            rule.check(catalog.header())
                .map_err(|detail| invalid_file(spec.id, path, detail))?;
        }
        self.write_marker(spec, path, sha256_hex, checked)
    }

    fn refuse_while_installing(&self, spec: &DatasetSpec) -> Result<(), StoreError> {
        if is_installing(&self.dataset_dir(spec)) {
            return Err(refused(spec.id, "it is being installed or removed"));
        }
        Ok(())
    }

    pub fn open(
        &self,
        id: &str,
        config: &AppConfig,
        holder: &'static str,
    ) -> Result<CatalogLease, StoreError> {
        let spec = self.spec(id)?;
        self.refuse_while_installing(spec)?;
        let path = match self.status_of(spec, config) {
            DatasetStatus::Installed { path, .. } => {
                self.ensure_verified(spec, &path)?;
                path
            }
            DatasetStatus::External {
                path, valid: true, ..
            } => path,
            DatasetStatus::External { path, problem, .. } => {
                return Err(invalid_file(spec.id, &path, problem.unwrap_or_default()))
            }
            DatasetStatus::Cache { .. } => {
                return Err(refused(spec.id, "it is a cache that its own module reads"))
            }
            DatasetStatus::Registered { .. } => {
                return Err(refused(
                    spec.id,
                    "its own module fills and reads its folder",
                ))
            }
            DatasetStatus::Missing | DatasetStatus::Partial { .. } => {
                return Err(refused(spec.id, "it is not installed"))
            }
        };
        let mut open = locked(&OPEN);
        self.refuse_while_installing(spec)?;
        let stamp = disk_stamp(&path);
        let current = open
            .get(&path)
            .filter(|entry| stamp.is_some() && entry.stamp == stamp)
            .map(|entry| Arc::clone(&entry.catalog));
        let catalog = match current {
            Some(catalog) => catalog,
            None => {
                let catalog = HealpixCatalog::open_file(&path)
                    .map_err(|error| invalid_file(spec.id, &path, error.to_string()))?;
                if let Some(rule) = spec.header {
                    rule.check(catalog.header())
                        .map_err(|detail| invalid_file(spec.id, &path, detail))?;
                }
                let catalog = Arc::new(catalog);
                let entry = open.entry(path.clone()).or_insert_with(|| OpenEntry {
                    catalog: Arc::clone(&catalog),
                    stamp,
                    holders: Vec::new(),
                });
                entry.catalog = Arc::clone(&catalog);
                entry.stamp = stamp;
                catalog
            }
        };
        let token = NEXT_LEASE.fetch_add(1, Ordering::Relaxed);
        if let Some(entry) = open.get_mut(&path) {
            entry.holders.push((token, holder));
        }
        Ok(CatalogLease {
            path,
            token,
            catalog,
        })
    }

    pub fn use_external_file(
        &self,
        id: &str,
        path: &Path,
        config: &mut AppConfig,
    ) -> Result<PathBuf, StoreError> {
        let spec = self.spec(id)?;
        if spec.kind != DatasetKind::External {
            return Err(refused(
                spec.id,
                format!(
                    "it is a {} dataset; only external datasets take an existing file",
                    spec.kind.name()
                ),
            ));
        }
        let text = absolute_utf8(spec.id, path)?;
        self.check_file(spec, path)
            .map_err(|detail| invalid_file(spec.id, path, detail))?;
        config.external_catalogs.insert(spec.id.to_owned(), text);
        Ok(path.to_path_buf())
    }

    pub fn use_siril_dir(&self, dir: &Path, config: &mut AppConfig) -> Result<PathBuf, StoreError> {
        let spec = self.spec(SIRIL_GAIA_ID)?;
        let text = absolute_utf8(spec.id, dir)?;
        if !dir.is_dir() {
            return Err(invalid_file(spec.id, dir, "the folder does not exist"));
        }
        let file = dir.join(spec.file_name);
        self.check_file(spec, &file)
            .map_err(|detail| invalid_file(spec.id, &file, detail))?;
        config.siril_catalog_dir = Some(text);
        Ok(file)
    }

    pub fn remove(&self, id: &str, config: &mut AppConfig) -> Result<Removed, StoreError> {
        let spec = self.spec(id)?;
        let dir = self.dataset_dir(spec);
        let _installing = InstallGuard::take(&dir, spec.id)?;
        let external = self.external_path(spec, config);
        let keeps_folder = external
            .as_deref()
            .is_some_and(|path| !path.starts_with(&dir));
        let open = release_mappings(spec.id, |path| {
            (!keeps_folder && path.starts_with(&dir)) || external.as_deref() == Some(path)
        })?;
        let freed_bytes = if keeps_folder {
            0
        } else {
            let (bytes, _) = dir_usage(&dir);
            match fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error(spec.id, "cannot delete", &dir, error)),
            }
            bytes
        };
        drop(open);
        let outside = |choice: &str| !Path::new(choice).starts_with(&dir);
        let forgets = |choice: &String| !keeps_folder || outside(choice);
        let mut forgotten = Vec::new();
        if let Some(choice) = config.external_catalogs.get(spec.id).filter(|c| forgets(c)) {
            forgotten.push(choice.clone());
            config.external_catalogs.remove(spec.id);
        }
        if spec.id == SIRIL_GAIA_ID {
            forgotten.extend(config.siril_catalog_dir.take_if(|choice| forgets(choice)));
        }
        Ok(Removed {
            id: spec.id,
            freed_bytes,
            forgot_external: forgotten.iter().any(|choice| outside(choice)),
        })
    }
}

#[cfg(feature = "net")]
fn download_failure(id: &str, url: &Url, dest: &Path, error: DownloadError) -> StoreError {
    match error {
        DownloadError::Cancelled => StoreError::Cancelled { id: id.to_owned() },
        DownloadError::Http { status: 404 } => StoreError::NotPublished {
            id: id.to_owned(),
            url: url.to_string(),
        },
        DownloadError::ChecksumMismatch {
            expected_hex,
            actual_hex,
        } => {
            for leftover in [
                dest.to_path_buf(),
                sibling(dest, ENGINE_RECORD_SUFFIX),
                sibling(dest, ENGINE_PART_SUFFIX),
                sibling(dest, ENGINE_SIDECAR_SUFFIX),
            ] {
                if let Err(error) = remove_if_present(&leftover) {
                    log::warn!("{id}: could not delete {}: {error}", leftover.display());
                }
            }
            StoreError::Checksum {
                id: id.to_owned(),
                expected_hex,
                actual_hex,
            }
        }
        other => StoreError::Download {
            id: id.to_owned(),
            source: other,
        },
    }
}

#[cfg(feature = "net")]
async fn holds_published_bytes(
    id: &'static str,
    path: &Path,
    size: Option<u64>,
    sha256: Option<[u8; 32]>,
) -> Result<bool, StoreError> {
    let len = file_len(id, path)?;
    if let Some(size) = size.filter(|&size| size != len) {
        log::warn!(
            "{id}: {} holds {len} bytes where {size} are published; it is downloaded again",
            path.display()
        );
        return Ok(false);
    }
    let Some(sha256) = sha256 else {
        return Ok(true);
    };
    let actual = hash_off_thread(id, path).await?;
    if actual != sha256 {
        log::warn!(
            "{id}: {} failed its sha-256 check (expected {}, found {}); it is downloaded again",
            path.display(),
            hex(&sha256),
            hex(&actual)
        );
    }
    Ok(actual == sha256)
}

#[cfg(feature = "net")]
fn file_len(id: &str, path: &Path) -> Result<u64, StoreError> {
    fs::metadata(path)
        .map(|meta| meta.len())
        .map_err(|error| io_error(id, "cannot read", path, error))
}

#[cfg(feature = "net")]
fn discard(id: &str, paths: &[PathBuf]) -> Result<(), StoreError> {
    for path in paths {
        remove_if_present(path).map_err(|error| io_error(id, "cannot delete", path, error))?;
    }
    Ok(())
}

fn absolute_utf8(id: &str, path: &Path) -> Result<String, StoreError> {
    if !path.is_absolute() {
        return Err(invalid_file(id, path, "an absolute path is needed"));
    }
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid_file(id, path, "the path is not valid UTF-8"))
}

fn partial_download(dest: &Path) -> Option<(u64, Option<u64>)> {
    let part_len = fs::metadata(sibling(dest, ENGINE_PART_SUFFIX)).ok()?.len();
    let sidecar = read_json::<EnginePart>(&sibling(dest, ENGINE_SIDECAR_SUFFIX))?;
    Some((part_len.min(sidecar.received), sidecar.expected_size))
}

#[cfg(feature = "net")]
fn already_downloaded(dest: &Path) -> u64 {
    match fs::metadata(dest) {
        Ok(meta) if meta.is_file() => meta.len(),
        _ => partial_download(dest).map_or(0, |(received, _)| received),
    }
}

fn dir_usage(dir: &Path) -> (u64, u64) {
    let mut pending = vec![dir.to_path_buf()];
    let (mut bytes, mut files) = (0u64, 0u64);
    while let Some(next) = pending.pop() {
        let Ok(entries) = fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                pending.push(entry.path());
            } else if meta.is_file() {
                bytes = bytes.saturating_add(meta.len());
                files += 1;
            }
        }
    }
    (bytes, files)
}

fn marker_for(path: &Path, sha256_hex: &str, checked: bool) -> io::Result<VerifiedMarker> {
    let meta = fs::metadata(path)?;
    Ok(VerifiedMarker {
        path: path.to_string_lossy().into_owned(),
        size: meta.len(),
        modified_unix_ms: meta
            .modified()
            .ok()
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .and_then(|since| u64::try_from(since.as_millis()).ok()),
        sha256: sha256_hex.to_owned(),
        checked,
    })
}

fn hash_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    loop {
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        hasher.update(buffer.get(..read).unwrap_or_default());
    }
    Ok(hasher.finalize().into())
}

fn parse_digest(text: &str) -> Option<[u8; 32]> {
    let token = text.split_whitespace().next()?;
    if token.len() != SHA256_HEX_LEN {
        return None;
    }
    let mut digest = [0u8; 32];
    for (slot, pair) in digest.iter_mut().zip(token.as_bytes().chunks_exact(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(digest)
}

#[cfg(feature = "net")]
async fn hash_off_thread(id: &'static str, path: &Path) -> Result<[u8; 32], StoreError> {
    let hashed = path.to_path_buf();
    tokio::task::spawn_blocking(move || hash_file(&hashed))
        .await
        .map_err(|error| {
            io_error(
                id,
                "the hash task failed for",
                path,
                io::Error::other(error),
            )
        })?
        .map_err(|error| io_error(id, "cannot read", path, error))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

#[cfg(feature = "net")]
fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(all(test, feature = "net"))]
mod tests;

#[cfg(all(test, not(feature = "net")))]
mod offline_tests {
    use super::*;

    struct Silent;

    impl InstallObserver for Silent {
        fn progress(&self, _stage: InstallStage, _done: u64, _total: Option<u64>) {}

        fn is_cancelled(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn a_build_without_network_reports_the_catalogues_and_names_the_feature_on_install() {
        let dir = tempfile::tempdir().unwrap();
        let roots = StoreRoots::under(&dir.path().join("data"), &dir.path().join("cache"));
        let store = CatalogStore::new(roots);
        let config = AppConfig::default();
        let report = store.report(&config);
        assert_eq!(report.len(), 3);
        assert!(report
            .iter()
            .all(|row| !row.installable && row.url_host.is_none()));
        assert_eq!(
            store.status(TYCHO2_ID, &config).unwrap(),
            DatasetStatus::Missing
        );
        let refused = store
            .install(SIRIL_GAIA_ID, Arc::new(Silent))
            .await
            .unwrap_err();
        assert!(refused.to_string().contains("network"), "{refused}");
        assert!(!dir.path().join("data").exists());
    }
}
