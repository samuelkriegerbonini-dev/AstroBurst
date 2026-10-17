use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::fs::File;
use std::io;
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
#[cfg(feature = "vizier")]
use std::time::Duration;

use memmap2::Mmap;
use thiserror::Error;

use crate::core::astrometry::catalog::{propagate_epoch, CatalogRow};
use crate::core::astrometry::wcs::angular_separation;
use crate::infra::fits::reader::create_mmap_random;

pub const HEADER_LEN: usize = 128;
pub const TITLE_LEN: usize = 48;
pub const MAX_LEVEL: u8 = 12;
pub const ASTROBURST_RECORD_KIND: u8 = 1;
pub const COMPACT_NO_MAGNITUDE: u8 = 255;
pub const COALESCE_GAP_BYTES: u64 = 64 * 1024;
pub const INDEX_CACHE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_READ_BYTES: usize = 4 * 1024 * 1024;
pub const CONE_RECORD_BUDGET: usize = 2_000_000;
pub const SIRIL_EPOCH_YEAR: f64 = 2016.0;
pub const ENV_SIRIL_CATALOG_DIR: &str = "ASTROBURST_SIRIL_CATALOG_DIR";
pub const SIRIL_ASTROMETRIC_FILE: &str = "siril_cat_healpix8_astro.dat";

const OFFSET_GAIA_VERSION: usize = 48;
const OFFSET_LEVEL: usize = 49;
const OFFSET_CAT_TYPE: usize = 50;
const OFFSET_CHUNKED: usize = 51;
const OFFSET_CHUNK_LEVEL: usize = 52;
const OFFSET_CHUNK_HEALPIX: usize = 53;
const OFFSET_FIRST_HEALPIXEL: usize = 57;
const OFFSET_LAST_HEALPIXEL: usize = 61;
const OFFSET_EPOCH_YEAR: usize = 65;
const OFFSET_RECORD_KIND: usize = 69;
const GAIA_DR3: u8 = 3;
const DEFAULT_COMPACT_EPOCH_YEAR: f32 = 2016.0;
const PLAUSIBLE_EPOCH_YEARS: Range<f32> = 1000.0..3000.0;
const BASE_PIXELS: u64 = 12;
const INDEX_ENTRY_BYTES: u64 = 4;
const INDEX_PAGE_ENTRIES: u64 = 4096;
const INDEX_CACHE_PAGES: usize =
    INDEX_CACHE_BYTES / (INDEX_PAGE_ENTRIES * INDEX_ENTRY_BYTES) as usize;
const CONE_DELTA_DEPTH: u8 = 2;
const WIDE_CONE_RADIUS_DEG: f64 = 90.0;
const SIRIL_COORDINATE_COUNTS: f64 = 2_147_483_647.0;
const DEGREES_PER_TURN: f64 = 360.0;
const MILLI: f64 = 1000.0;
const ARCSEC_PER_RADIAN: f64 = 180.0 * 3600.0 / PI;
const COMPACT_STEPS_PER_ARCSEC: f64 = 10.0;
const COMPACT_MAG_STEPS_PER_MAG: f64 = 10.0;
const COMPACT_MAG_CODE_AT_ZERO: f64 = 15.0;
const COMPACT_COLOUR_STEPS_PER_MAG: f64 = 50.0;
const COMPACT_NO_COLOUR: i8 = i8::MIN;
const WIDE_NO_VALUE: i16 = i16::MIN;
const WIDE_LIMIT: f64 = i16::MAX as f64;
const COLOUR_LIMIT: f64 = i8::MAX as f64;
#[cfg(feature = "vizier")]
const RANGE_TIMEOUT_SECS: u64 = 60;
#[cfg(feature = "vizier")]
const USER_AGENT: &str = concat!("AstroBurst/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HttpRangeError {
    #[error("{url} answered HTTP {status} to a byte-range request; a catalogue read needs 206 Partial Content")]
    Status { url: String, status: u16 },
    #[error("{url} answered 416 Range Not Satisfiable for bytes {start}-{end}")]
    NotSatisfiable { url: String, start: u64, end: u64 },
    #[error("{url} changed between requests: {what} was {before}, now {after}")]
    Changed {
        url: String,
        what: &'static str,
        before: String,
        after: String,
    },
    #[error("{url} answered with an unusable Content-Range ({detail})")]
    ContentRange { url: String, detail: String },
    #[error("{url}: {detail}")]
    Transport { url: String, detail: String },
}

#[derive(Debug, Error)]
pub enum CatalogReadError {
    #[error("catalogue header field {field}: {detail}")]
    Header { field: &'static str, detail: String },
    #[error("catalogue is {actual} bytes, too short for its {what} ({expected} bytes)")]
    Truncated {
        what: &'static str,
        expected: u64,
        actual: u64,
    },
    #[error("catalogue index entry of pixel {pixel} is inconsistent: {detail}")]
    Index { pixel: u32, detail: String },
    #[error("pixel {pixel} is not in this file (pixels {first}..={last})")]
    PixelOutside { pixel: u32, first: u32, last: u32 },
    #[error("{0} records are not decoded by cone(); read them with read_records_raw")]
    UnsupportedType(&'static str),
    #[error("cone: {0}")]
    Cone(String),
    #[error("record {record} of pixel {pixel} does not decode to a position on the sky")]
    Record { pixel: u32, record: usize },
    #[error("pixel {pixel}: {detail}")]
    Encode { pixel: u32, detail: String },
    #[error("cannot open {path}: {detail}")]
    Open { path: String, detail: String },
    #[error(transparent)]
    Http(HttpRangeError),
    #[error("catalogue read failed: {0}")]
    Io(io::Error),
}

impl From<io::Error> for CatalogReadError {
    fn from(error: io::Error) -> Self {
        match error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<HttpRangeError>())
        {
            Some(http) => Self::Http(http.clone()),
            None => Self::Io(error),
        }
    }
}

fn header_error(field: &'static str, detail: String) -> CatalogReadError {
    CatalogReadError::Header { field, detail }
}

fn encode_error(pixel: u32, detail: impl Into<String>) -> CatalogReadError {
    CatalogReadError::Encode {
        pixel,
        detail: detail.into(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatType {
    Astrometric,
    XpSampled,
    XpContinuous,
    Compact6,
    Compact12,
}

impl CatType {
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Astrometric),
            2 => Some(Self::XpSampled),
            3 => Some(Self::XpContinuous),
            128 => Some(Self::Compact6),
            129 => Some(Self::Compact12),
            _ => None,
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::Astrometric => 1,
            Self::XpSampled => 2,
            Self::XpContinuous => 3,
            Self::Compact6 => 128,
            Self::Compact12 => 129,
        }
    }

    pub fn record_len(self) -> Option<usize> {
        match self {
            Self::Astrometric => Some(16),
            Self::XpSampled => Some(701),
            Self::XpContinuous => None,
            Self::Compact6 => Some(6),
            Self::Compact12 => Some(12),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Astrometric => "astrometric",
            Self::XpSampled => "XP sampled",
            Self::XpContinuous => "XP continuous",
            Self::Compact6 => "AstroBurst compact 6-byte",
            Self::Compact12 => "AstroBurst compact 12-byte",
        }
    }

    pub fn is_compact(self) -> bool {
        matches!(self, Self::Compact6 | Self::Compact12)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatHeader {
    pub title: String,
    pub gaia_version: u8,
    pub healpix_level: u8,
    pub cat_type: u8,
    pub chunked: u8,
    pub chunk_level: u8,
    pub chunk_healpix: u32,
    pub first_healpixel: u32,
    pub last_healpixel: u32,
    pub epoch_year: f32,
    pub record_kind: u8,
}

fn whole_index_entries(level: u8) -> u64 {
    1u64.checked_shl(2 * u32::from(level))
        .and_then(|per_base| per_base.checked_mul(BASE_PIXELS))
        .unwrap_or(u64::MAX)
}

fn chunk_index_entries(level: u8, chunk_level: u8) -> u64 {
    level
        .checked_sub(chunk_level)
        .and_then(|depth| 1u64.checked_shl(2 * u32::from(depth)))
        .unwrap_or(0)
}

fn saturating_u32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn take<const N: usize>(bytes: &[u8], at: usize) -> [u8; N] {
    bytes
        .get(at..at.saturating_add(N))
        .and_then(|slice| slice.try_into().ok())
        .unwrap_or([0; N])
}

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    if let Some(target) = bytes.get_mut(at..at.saturating_add(value.len())) {
        target.copy_from_slice(value);
    }
}

impl CatHeader {
    pub fn whole(title: &str, level: u8, cat_type: u8) -> Self {
        let compact = CatType::from_code(cat_type).is_some_and(CatType::is_compact);
        Self {
            title: title.to_owned(),
            gaia_version: if compact { 0 } else { GAIA_DR3 },
            healpix_level: level,
            cat_type,
            chunked: 0,
            chunk_level: 0,
            chunk_healpix: 0,
            first_healpixel: 0,
            last_healpixel: saturating_u32(whole_index_entries(level).saturating_sub(1)),
            epoch_year: if compact {
                DEFAULT_COMPACT_EPOCH_YEAR
            } else {
                0.0
            },
            record_kind: if compact { ASTROBURST_RECORD_KIND } else { 0 },
        }
    }

    pub fn chunk(
        title: &str,
        level: u8,
        cat_type: u8,
        chunk_level: u8,
        chunk_healpix: u32,
    ) -> Self {
        let entries = chunk_index_entries(level, chunk_level);
        let first = u64::from(chunk_healpix).saturating_mul(entries);
        Self {
            chunked: 1,
            chunk_level,
            chunk_healpix,
            first_healpixel: saturating_u32(first),
            last_healpixel: saturating_u32(first.saturating_add(entries).saturating_sub(1)),
            ..Self::whole(title, level, cat_type)
        }
    }

    pub fn with_epoch_year(mut self, epoch_year: f32) -> Self {
        self.epoch_year = epoch_year;
        self
    }

    pub fn parse(bytes: &[u8; HEADER_LEN]) -> Self {
        let title_bytes = bytes.get(..TITLE_LEN).unwrap_or_default();
        let title_end = title_bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(title_bytes.len());
        let title = String::from_utf8_lossy(title_bytes.get(..title_end).unwrap_or_default())
            .trim_end()
            .to_owned();
        Self {
            title,
            gaia_version: u8::from_le_bytes(take(bytes, OFFSET_GAIA_VERSION)),
            healpix_level: u8::from_le_bytes(take(bytes, OFFSET_LEVEL)),
            cat_type: u8::from_le_bytes(take(bytes, OFFSET_CAT_TYPE)),
            chunked: u8::from_le_bytes(take(bytes, OFFSET_CHUNKED)),
            chunk_level: u8::from_le_bytes(take(bytes, OFFSET_CHUNK_LEVEL)),
            chunk_healpix: u32::from_le_bytes(take(bytes, OFFSET_CHUNK_HEALPIX)),
            first_healpixel: u32::from_le_bytes(take(bytes, OFFSET_FIRST_HEALPIXEL)),
            last_healpixel: u32::from_le_bytes(take(bytes, OFFSET_LAST_HEALPIXEL)),
            epoch_year: f32::from_le_bytes(take(bytes, OFFSET_EPOCH_YEAR)),
            record_kind: u8::from_le_bytes(take(bytes, OFFSET_RECORD_KIND)),
        }
    }

    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut bytes = [0u8; HEADER_LEN];
        let title = self.title.as_bytes();
        put(&mut bytes, 0, title.get(..TITLE_LEN).unwrap_or(title));
        put(&mut bytes, OFFSET_GAIA_VERSION, &[self.gaia_version]);
        put(&mut bytes, OFFSET_LEVEL, &[self.healpix_level]);
        put(&mut bytes, OFFSET_CAT_TYPE, &[self.cat_type]);
        put(&mut bytes, OFFSET_CHUNKED, &[self.chunked]);
        put(&mut bytes, OFFSET_CHUNK_LEVEL, &[self.chunk_level]);
        put(
            &mut bytes,
            OFFSET_CHUNK_HEALPIX,
            &self.chunk_healpix.to_le_bytes(),
        );
        put(
            &mut bytes,
            OFFSET_FIRST_HEALPIXEL,
            &self.first_healpixel.to_le_bytes(),
        );
        put(
            &mut bytes,
            OFFSET_LAST_HEALPIXEL,
            &self.last_healpixel.to_le_bytes(),
        );
        put(
            &mut bytes,
            OFFSET_EPOCH_YEAR,
            &self.epoch_year.to_le_bytes(),
        );
        put(&mut bytes, OFFSET_RECORD_KIND, &[self.record_kind]);
        bytes
    }

    pub fn cat_type(&self) -> Option<CatType> {
        CatType::from_code(self.cat_type)
    }

    pub fn record_len(&self) -> Option<usize> {
        self.cat_type().and_then(CatType::record_len)
    }

    pub fn is_chunked(&self) -> bool {
        self.chunked == 1
    }

    pub fn chunk_level_of_file(&self) -> Option<u8> {
        self.is_chunked().then_some(self.chunk_level)
    }

    pub fn chunk_id(&self) -> u32 {
        if self.is_chunked() {
            self.chunk_healpix
        } else {
            0
        }
    }

    pub fn first_pixel(&self) -> u32 {
        if self.is_chunked() {
            self.first_healpixel
        } else {
            0
        }
    }

    pub fn index_entries(&self) -> u64 {
        if self.is_chunked() {
            chunk_index_entries(self.healpix_level, self.chunk_level)
        } else {
            whole_index_entries(self.healpix_level)
        }
    }

    pub fn index_bytes(&self) -> u64 {
        self.index_entries().saturating_mul(INDEX_ENTRY_BYTES)
    }

    pub fn validate(&self, file_len: u64) -> Result<(), CatalogReadError> {
        let cat_type = self.cat_type().ok_or_else(|| {
            header_error(
                "cat_type",
                format!(
                    "{} is not a known record type (1 astrometric, 2 XP sampled, 3 XP continuous, 128 and 129 AstroBurst compact)",
                    self.cat_type
                ),
            )
        })?;
        if cat_type.record_len().is_none() {
            return Err(header_error(
                "cat_type",
                format!(
                    "{} ({}) has no fixed record length this reader knows",
                    self.cat_type,
                    cat_type.name()
                ),
            ));
        }
        if self.healpix_level > MAX_LEVEL {
            return Err(header_error(
                "healpix_level",
                format!("{} is outside 0..={MAX_LEVEL}", self.healpix_level),
            ));
        }
        if self.chunked > 1 {
            return Err(header_error(
                "chunked",
                format!(
                    "{} is neither 0 (whole sky) nor 1 (one chunk)",
                    self.chunked
                ),
            ));
        }
        if self.is_chunked() {
            if self.chunk_level > self.healpix_level {
                return Err(header_error(
                    "chunk_level",
                    format!(
                        "{} is above healpix_level {}",
                        self.chunk_level, self.healpix_level
                    ),
                ));
            }
            let chunks = whole_index_entries(self.chunk_level);
            if u64::from(self.chunk_healpix) >= chunks {
                return Err(header_error(
                    "chunk_healpix",
                    format!(
                        "{} is not one of the {chunks} chunks of chunk level {}",
                        self.chunk_healpix, self.chunk_level
                    ),
                ));
            }
        }
        if self.last_healpixel < self.first_healpixel {
            return Err(header_error(
                "last_healpixel",
                format!(
                    "{} is below first_healpixel {}",
                    self.last_healpixel, self.first_healpixel
                ),
            ));
        }
        if self.is_chunked() {
            let entries = self.index_entries();
            let span = u64::from(self.last_healpixel) - u64::from(self.first_healpixel) + 1;
            if span != entries {
                return Err(header_error(
                    "last_healpixel",
                    format!(
                        "first_healpixel {} to last_healpixel {} spans {span} pixels, but a level-{} chunk of chunk level {} holds {entries}",
                        self.first_healpixel, self.last_healpixel, self.healpix_level, self.chunk_level
                    ),
                ));
            }
            let expected_first = u64::from(self.chunk_healpix) * entries;
            if u64::from(self.first_healpixel) != expected_first {
                return Err(header_error(
                    "first_healpixel",
                    format!(
                        "{} is not {expected_first}, the first pixel of chunk {}",
                        self.first_healpixel, self.chunk_healpix
                    ),
                ));
            }
        }
        if cat_type.is_compact() {
            if self.record_kind != ASTROBURST_RECORD_KIND {
                return Err(header_error(
                    "record_kind",
                    format!(
                        "{} is not AstroBurst's record kind {ASTROBURST_RECORD_KIND}; type {} files of other applications are not read",
                        self.record_kind, self.cat_type
                    ),
                ));
            }
            if !PLAUSIBLE_EPOCH_YEARS.contains(&self.epoch_year) {
                return Err(header_error(
                    "epoch_year",
                    format!("{} is not a plausible epoch", self.epoch_year),
                ));
            }
        }
        let needed = (HEADER_LEN as u64).saturating_add(self.index_bytes());
        if file_len < needed {
            return Err(CatalogReadError::Truncated {
                what: "header and index",
                expected: needed,
                actual: file_len,
            });
        }
        Ok(())
    }
}

pub trait ByteSource: Send + Sync {
    fn len(&self) -> u64;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
}

fn copy_out(bytes: &[u8], offset: u64, buf: &mut [u8]) -> io::Result<()> {
    let slice = usize::try_from(offset)
        .ok()
        .and_then(|start| bytes.get(start..start.checked_add(buf.len())?));
    match slice {
        Some(slice) => {
            buf.copy_from_slice(slice);
            Ok(())
        }
        None => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "bytes {offset}..{} lie past the end of a {}-byte source",
                offset.saturating_add(buf.len() as u64),
                bytes.len()
            ),
        )),
    }
}

pub struct MemSource(Vec<u8>);

impl MemSource {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl ByteSource for MemSource {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        copy_out(&self.0, offset, buf)
    }
}

pub struct MmapSource {
    map: Mmap,
}

impl MmapSource {
    pub fn open(path: &Path) -> Result<Self, CatalogReadError> {
        let open_error = |detail: String| CatalogReadError::Open {
            path: path.display().to_string(),
            detail,
        };
        let file = File::open(path).map_err(|e| open_error(e.to_string()))?;
        let map = create_mmap_random(&file).map_err(|e| open_error(format!("{e:#}")))?;
        Ok(Self { map })
    }
}

impl ByteSource for MmapSource {
    fn len(&self) -> u64 {
        self.map.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        copy_out(&self.map, offset, buf)
    }
}

#[cfg(feature = "vizier")]
pub struct HttpRangeSource {
    client: reqwest::blocking::Client,
    url: String,
    len: u64,
    etag: Option<String>,
    last_modified: Option<String>,
    head: Vec<u8>,
}

#[cfg(feature = "vizier")]
fn header_text(
    response: &reqwest::blocking::Response,
    name: reqwest::header::HeaderName,
) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[cfg(feature = "vizier")]
fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let (span, total) = value.trim().strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = span.split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end: u64 = end.trim().parse().ok()?;
    let total: u64 = total.trim().parse().ok()?;
    (start <= end && end < total).then_some((start, end, total))
}

#[cfg(feature = "vizier")]
fn outside_async_runtime<T: Send>(
    url: &str,
    work: impl FnOnce() -> Result<T, HttpRangeError> + Send,
) -> Result<T, HttpRangeError> {
    if tokio::runtime::Handle::try_current().is_err() {
        return work();
    }
    std::thread::scope(|scope| {
        scope.spawn(work).join().unwrap_or_else(|_| {
            Err(HttpRangeError::Transport {
                url: url.to_owned(),
                detail: "the byte-range request thread stopped".to_owned(),
            })
        })
    })
}

#[cfg(feature = "vizier")]
impl HttpRangeSource {
    pub fn open(url: &str) -> Result<Self, HttpRangeError> {
        outside_async_runtime(url, || Self::open_on_this_thread(url))
    }

    fn open_on_this_thread(url: &str) -> Result<Self, HttpRangeError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(RANGE_TIMEOUT_SECS))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| HttpRangeError::Transport {
                url: url.to_owned(),
                detail: e.to_string(),
            })?;
        let mut source = Self {
            client,
            url: url.to_owned(),
            len: 0,
            etag: None,
            last_modified: None,
            head: Vec::new(),
        };
        let last_header_byte = HEADER_LEN as u64 - 1;
        let response = source.send(0, last_header_byte, false)?;
        source.check_status(response.status().as_u16(), 0, last_header_byte)?;
        source.etag = header_text(&response, reqwest::header::ETAG);
        source.last_modified = header_text(&response, reqwest::header::LAST_MODIFIED);
        let (start, end, total) = source.content_range(&response)?;
        let expected_end = last_header_byte.min(total.saturating_sub(1));
        if (start, end) != (0, expected_end) {
            return Err(source.range_error(format!(
                "bytes {start}-{end}/{total} for a request of 0-{last_header_byte}"
            )));
        }
        source.len = total;
        source.head = source.body(response, end - start + 1)?;
        Ok(source)
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    fn send(
        &self,
        start: u64,
        end: u64,
        conditional: bool,
    ) -> Result<reqwest::blocking::Response, HttpRangeError> {
        let mut request = self
            .client
            .get(&self.url)
            .header(reqwest::header::RANGE, format!("bytes={start}-{end}"));
        if let Some(validator) = self.if_range().filter(|_| conditional) {
            request = request.header(reqwest::header::IF_RANGE, validator);
        }
        request.send().map_err(|e| HttpRangeError::Transport {
            url: self.url.clone(),
            detail: e.to_string(),
        })
    }

    fn if_range(&self) -> Option<&str> {
        self.etag
            .as_deref()
            .filter(|tag| !tag.starts_with("W/"))
            .or(self.last_modified.as_deref())
    }

    fn check_status(&self, status: u16, start: u64, end: u64) -> Result<(), HttpRangeError> {
        match status {
            206 => Ok(()),
            416 => Err(HttpRangeError::NotSatisfiable {
                url: self.url.clone(),
                start,
                end,
            }),
            _ => Err(HttpRangeError::Status {
                url: self.url.clone(),
                status,
            }),
        }
    }

    fn check_validators(
        &self,
        response: &reqwest::blocking::Response,
    ) -> Result<(), HttpRangeError> {
        let validators = [
            (
                "ETag",
                &self.etag,
                header_text(response, reqwest::header::ETAG),
            ),
            (
                "Last-Modified",
                &self.last_modified,
                header_text(response, reqwest::header::LAST_MODIFIED),
            ),
        ];
        for (what, before, after) in validators {
            if let (Some(before), Some(after)) = (before, after) {
                if *before != after {
                    return Err(HttpRangeError::Changed {
                        url: self.url.clone(),
                        what,
                        before: before.clone(),
                        after,
                    });
                }
            }
        }
        Ok(())
    }

    fn range_error(&self, detail: String) -> HttpRangeError {
        HttpRangeError::ContentRange {
            url: self.url.clone(),
            detail,
        }
    }

    fn content_range(
        &self,
        response: &reqwest::blocking::Response,
    ) -> Result<(u64, u64, u64), HttpRangeError> {
        let value = header_text(response, reqwest::header::CONTENT_RANGE)
            .ok_or_else(|| self.range_error("no Content-Range header".to_owned()))?;
        parse_content_range(&value).ok_or_else(|| self.range_error(value))
    }

    fn body(
        &self,
        response: reqwest::blocking::Response,
        expected: u64,
    ) -> Result<Vec<u8>, HttpRangeError> {
        let body = response.bytes().map_err(|e| HttpRangeError::Transport {
            url: self.url.clone(),
            detail: e.to_string(),
        })?;
        if body.len() as u64 != expected {
            return Err(self.range_error(format!(
                "{} body bytes for a {expected}-byte range",
                body.len()
            )));
        }
        Ok(body.to_vec())
    }

    fn fetch(&self, start: u64, end: u64, buf: &mut [u8]) -> Result<(), HttpRangeError> {
        outside_async_runtime(&self.url, || self.fetch_on_this_thread(start, end, buf))
    }

    fn fetch_on_this_thread(
        &self,
        start: u64,
        end: u64,
        buf: &mut [u8],
    ) -> Result<(), HttpRangeError> {
        let response = self.send(start, end, true)?;
        self.check_validators(&response)?;
        self.check_status(response.status().as_u16(), start, end)?;
        let (got_start, got_end, total) = self.content_range(&response)?;
        if total != self.len {
            return Err(HttpRangeError::Changed {
                url: self.url.clone(),
                what: "length",
                before: self.len.to_string(),
                after: total.to_string(),
            });
        }
        if (got_start, got_end) != (start, end) {
            return Err(self.range_error(format!(
                "bytes {got_start}-{got_end} for a request of {start}-{end}"
            )));
        }
        let body = self.body(response, end - start + 1)?;
        buf.copy_from_slice(&body);
        Ok(())
    }
}

#[cfg(feature = "vizier")]
impl ByteSource for HttpRangeSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        let cached = usize::try_from(offset)
            .ok()
            .and_then(|start| self.head.get(start..start.checked_add(buf.len())?));
        if let Some(cached) = cached {
            buf.copy_from_slice(cached);
            return Ok(());
        }
        let end = offset
            .checked_add(buf.len() as u64 - 1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "range end overflows"))?;
        self.fetch(offset, end, buf).map_err(io::Error::other)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CatalogStar {
    pub ra_deg: f64,
    pub dec_deg: f64,
    pub pm_ra_masyr: Option<f64>,
    pub pm_dec_masyr: Option<f64>,
    pub teff_k: Option<f64>,
    pub mag: Option<f64>,
    pub colour: Option<f64>,
}

impl CatalogStar {
    pub fn at(ra_deg: f64, dec_deg: f64, mag: Option<f64>) -> Self {
        Self {
            ra_deg,
            dec_deg,
            pm_ra_masyr: None,
            pm_dec_masyr: None,
            teff_k: None,
            mag,
            colour: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    Star(CatalogStar),
    Raw(Vec<u8>),
}

#[derive(Default)]
struct IndexPages {
    pages: HashMap<u64, Arc<[u32]>>,
    order: VecDeque<u64>,
}

struct PixelSpan {
    pixel: u32,
    records: Range<u32>,
}

fn coalesces(group: &Range<u64>, next: &Range<u64>) -> bool {
    next.start.saturating_sub(group.end) < COALESCE_GAP_BYTES
        && group.end.max(next.end).saturating_sub(group.start) <= MAX_READ_BYTES as u64
}

pub struct HealpixCatalog<S> {
    source: S,
    header: CatHeader,
    cat_type: CatType,
    record_len: u64,
    entries: u32,
    first_pixel: u32,
    total_records: u32,
    index_pages: Mutex<IndexPages>,
}

impl<S: ByteSource> HealpixCatalog<S> {
    pub fn new(source: S) -> Result<Self, CatalogReadError> {
        let len = source.len();
        if len < HEADER_LEN as u64 {
            return Err(CatalogReadError::Truncated {
                what: "header",
                expected: HEADER_LEN as u64,
                actual: len,
            });
        }
        let mut head = [0u8; HEADER_LEN];
        source.read_at(0, &mut head)?;
        let header = CatHeader::parse(&head);
        header.validate(len)?;
        let cat_type = header.cat_type().ok_or_else(|| {
            header_error("cat_type", format!("{} is not readable", header.cat_type))
        })?;
        let record_len = cat_type.record_len().ok_or_else(|| {
            header_error(
                "cat_type",
                format!("{} has no record length", cat_type.name()),
            )
        })? as u64;
        let entries = u32::try_from(header.index_entries()).map_err(|_| {
            header_error(
                "healpix_level",
                format!("{} gives too many index entries", header.healpix_level),
            )
        })?;
        let mut catalog = Self {
            source,
            first_pixel: header.first_pixel(),
            header,
            cat_type,
            record_len,
            entries,
            total_records: 0,
            index_pages: Mutex::new(IndexPages::default()),
        };
        let total = catalog.index_value(entries.saturating_sub(1))?;
        let needed = catalog
            .data_offset()
            .saturating_add(u64::from(total).saturating_mul(record_len));
        if needed > len {
            return Err(CatalogReadError::Truncated {
                what: "records",
                expected: needed,
                actual: len,
            });
        }
        catalog.total_records = total;
        Ok(catalog)
    }

    pub fn header(&self) -> &CatHeader {
        &self.header
    }

    pub fn cat_type(&self) -> CatType {
        self.cat_type
    }

    pub fn total_records(&self) -> u32 {
        self.total_records
    }

    pub fn record_offset(&self, record: u32) -> u64 {
        self.data_offset() + u64::from(record) * self.record_len
    }

    fn data_offset(&self) -> u64 {
        HEADER_LEN as u64 + u64::from(self.entries) * INDEX_ENTRY_BYTES
    }

    fn pages(&self) -> MutexGuard<'_, IndexPages> {
        self.index_pages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn index_page(&self, page: u64) -> Result<Arc<[u32]>, CatalogReadError> {
        if let Some(found) = self.pages().pages.get(&page) {
            return Ok(Arc::clone(found));
        }
        let first_entry = page * INDEX_PAGE_ENTRIES;
        let count = INDEX_PAGE_ENTRIES.min(u64::from(self.entries).saturating_sub(first_entry));
        let mut bytes = vec![0u8; (count * INDEX_ENTRY_BYTES) as usize];
        self.source.read_at(
            HEADER_LEN as u64 + first_entry * INDEX_ENTRY_BYTES,
            &mut bytes,
        )?;
        let values: Arc<[u32]> = bytes
            .chunks_exact(INDEX_ENTRY_BYTES as usize)
            .map(|entry| u32::from_le_bytes(take(entry, 0)))
            .collect();
        let mut cache = self.pages();
        if cache.order.len() >= INDEX_CACHE_PAGES {
            if let Some(oldest) = cache.order.pop_front() {
                cache.pages.remove(&oldest);
            }
        }
        if cache.pages.insert(page, Arc::clone(&values)).is_none() {
            cache.order.push_back(page);
        }
        Ok(values)
    }

    fn index_value(&self, entry: u32) -> Result<u32, CatalogReadError> {
        let entry64 = u64::from(entry);
        self.index_page(entry64 / INDEX_PAGE_ENTRIES)?
            .get((entry64 % INDEX_PAGE_ENTRIES) as usize)
            .copied()
            .ok_or_else(|| CatalogReadError::Index {
                pixel: self.first_pixel.saturating_add(entry),
                detail: format!("entry {entry} is past the {} index entries", self.entries),
            })
    }

    fn entry_of(&self, pixel: u32) -> Result<u32, CatalogReadError> {
        pixel
            .checked_sub(self.first_pixel)
            .filter(|entry| *entry < self.entries)
            .ok_or(CatalogReadError::PixelOutside {
                pixel,
                first: self.first_pixel,
                last: self
                    .first_pixel
                    .saturating_add(self.entries.saturating_sub(1)),
            })
    }

    fn bounds(&self, first_entry: u32, last_entry: u32) -> Result<Vec<u32>, CatalogReadError> {
        let mut bounds = Vec::with_capacity(last_entry.saturating_sub(first_entry) as usize + 2);
        bounds.push(match first_entry.checked_sub(1) {
            Some(previous) => self.index_value(previous)?,
            None => 0,
        });
        for entry in first_entry..=last_entry {
            let start = bounds.last().copied().unwrap_or(0);
            let end = self.index_value(entry)?;
            if end < start || end > self.total_records {
                return Err(CatalogReadError::Index {
                    pixel: self.first_pixel.saturating_add(entry),
                    detail: format!(
                        "cumulative count {end} follows {start} in a file of {} records",
                        self.total_records
                    ),
                });
            }
            bounds.push(end);
        }
        Ok(bounds)
    }

    pub fn pixel_range(&self, pixel: u32) -> Result<Range<u32>, CatalogReadError> {
        let entry = self.entry_of(pixel)?;
        match self.bounds(entry, entry)?.as_slice() {
            [start, end] => Ok(*start..*end),
            _ => Err(CatalogReadError::Index {
                pixel,
                detail: "no bounds".to_owned(),
            }),
        }
    }

    fn byte_len(&self, records: Range<u32>) -> Result<usize, CatalogReadError> {
        let bytes = u64::from(records.end.saturating_sub(records.start)) * self.record_len;
        usize::try_from(bytes).map_err(|_| CatalogReadError::Index {
            pixel: self.first_pixel,
            detail: format!("{bytes} bytes do not fit in memory"),
        })
    }

    fn records_in_runs(&self, runs: &[Range<u32>]) -> Result<usize, CatalogReadError> {
        let mut total: usize = 0;
        for run in runs.iter().filter(|run| !run.is_empty()) {
            let first = self.pixel_range(run.start)?.start;
            let end = self.pixel_range(run.end - 1)?.end;
            total = total.saturating_add(end.saturating_sub(first) as usize);
        }
        Ok(total)
    }

    pub fn read_records_raw(&self, pixels: Range<u32>) -> Result<Vec<u8>, CatalogReadError> {
        if pixels.is_empty() {
            return Ok(Vec::new());
        }
        let first = self.entry_of(pixels.start)?;
        let last = self.entry_of(pixels.end - 1)?;
        let bounds = self.bounds(first, last)?;
        let start = bounds.first().copied().unwrap_or(0);
        let end = bounds.last().copied().unwrap_or(start);
        let mut bytes = vec![0u8; self.byte_len(start..end)?];
        self.read_into(self.record_offset(start), &mut bytes)?;
        Ok(bytes)
    }

    fn read_into(&self, offset: u64, buffer: &mut [u8]) -> Result<(), CatalogReadError> {
        let mut at = offset;
        for piece in buffer.chunks_mut(MAX_READ_BYTES) {
            self.source.read_at(at, piece)?;
            at = at.saturating_add(piece.len() as u64);
        }
        Ok(())
    }

    pub fn visit_pixel_runs<F>(
        &self,
        runs: &[Range<u32>],
        mut visit: F,
    ) -> Result<(), CatalogReadError>
    where
        F: FnMut(u32, &[u8]) -> Result<(), CatalogReadError>,
    {
        let mut ordered: Vec<&Range<u32>> = runs.iter().filter(|run| !run.is_empty()).collect();
        ordered.sort_by_key(|run| run.start);
        let mut group: Vec<PixelSpan> = Vec::new();
        let mut group_bytes: Option<Range<u64>> = None;
        for run in ordered {
            let first = self.entry_of(run.start)?;
            let last = self.entry_of(run.end - 1)?;
            let bounds = self.bounds(first, last)?;
            for (pixel, pair) in (run.start..).zip(bounds.windows(2)) {
                let &[start, end] = pair else { continue };
                if start == end {
                    continue;
                }
                let bytes = self.record_offset(start)..self.record_offset(end);
                match group_bytes.as_mut() {
                    Some(current) if coalesces(current, &bytes) => {
                        current.end = current.end.max(bytes.end);
                    }
                    _ => {
                        if let Some(current) = group_bytes.replace(bytes) {
                            self.read_group(current, &group, &mut visit)?;
                            group.clear();
                        }
                    }
                }
                group.push(PixelSpan {
                    pixel,
                    records: start..end,
                });
            }
        }
        if let Some(current) = group_bytes {
            self.read_group(current, &group, &mut visit)?;
        }
        Ok(())
    }

    fn read_group<F>(
        &self,
        span: Range<u64>,
        pixels: &[PixelSpan],
        visit: &mut F,
    ) -> Result<(), CatalogReadError>
    where
        F: FnMut(u32, &[u8]) -> Result<(), CatalogReadError>,
    {
        let span_len = span.end - span.start;
        let len = usize::try_from(span_len).map_err(|_| CatalogReadError::Index {
            pixel: self.first_pixel,
            detail: format!("a {span_len}-byte read does not fit in memory"),
        })?;
        let mut buffer = vec![0u8; len];
        self.read_into(span.start, &mut buffer)?;
        let local = |record: u32| {
            self.record_offset(record)
                .checked_sub(span.start)
                .and_then(|offset| usize::try_from(offset).ok())
        };
        for PixelSpan { pixel, records } in pixels {
            let bytes = local(records.start)
                .zip(local(records.end))
                .and_then(|(from, to)| buffer.get(from..to))
                .ok_or_else(|| CatalogReadError::Index {
                    pixel: *pixel,
                    detail: format!(
                        "records {}..{} fall outside the bytes read",
                        records.start, records.end
                    ),
                })?;
            visit(*pixel, bytes)?;
        }
        Ok(())
    }

    pub fn cone(
        &self,
        ra_deg: f64,
        dec_deg: f64,
        radius_deg: f64,
        epoch_year: Option<f64>,
    ) -> Result<Vec<CatalogRow>, CatalogReadError> {
        self.cone_within(ra_deg, dec_deg, radius_deg, epoch_year, CONE_RECORD_BUDGET)
    }

    fn cone_within(
        &self,
        ra_deg: f64,
        dec_deg: f64,
        radius_deg: f64,
        epoch_year: Option<f64>,
        record_budget: usize,
    ) -> Result<Vec<CatalogRow>, CatalogReadError> {
        if !matches!(
            self.cat_type,
            CatType::Astrometric | CatType::Compact6 | CatType::Compact12
        ) {
            return Err(CatalogReadError::UnsupportedType(self.cat_type.name()));
        }
        let level = self.header.healpix_level;
        let chunk = self.header.chunk_id();
        let runs: Vec<Range<u32>> = pixel_ranges(
            level,
            self.header.chunk_level_of_file(),
            ra_deg,
            dec_deg,
            radius_deg,
        )?
        .into_iter()
        .filter(|(run_chunk, _)| *run_chunk == chunk)
        .map(|(_, run)| run)
        .collect();
        let covered = self.records_in_runs(&runs)?;
        if covered > record_budget {
            return Err(CatalogReadError::Cone(format!(
                "the cone covers {covered} records, more than the {record_budget} one call holds in memory; read wide cones through visit_pixel_runs"
            )));
        }
        let record_len = self.record_len as usize;
        let mut hits: Vec<(u32, usize, CatalogStar)> = Vec::new();
        self.visit_pixel_runs(&runs, |pixel, records| {
            for (n, record) in records.chunks_exact(record_len).enumerate() {
                let star = decode_star(self.cat_type, level, pixel, record)
                    .ok_or(CatalogReadError::Record { pixel, record: n })?;
                if angular_separation(ra_deg, dec_deg, star.ra_deg, star.dec_deg) <= radius_deg {
                    hits.push((pixel, n, star));
                }
            }
            Ok(())
        })?;
        let mut order: Vec<(Option<f64>, usize)> = hits
            .iter()
            .enumerate()
            .map(|(index, (_, _, star))| (star.mag, index))
            .collect();
        order.sort_by(|a, b| by_magnitude(a.0, b.0));
        let mut rows: Vec<CatalogRow> = order
            .iter()
            .filter_map(|&(_, index)| hits.get(index))
            .map(|&(pixel, n, star)| self.row(pixel, n, star))
            .collect();
        if let Some(year) = epoch_year.filter(|year| year.is_finite()) {
            propagate_epoch(&mut rows, year);
        }
        Ok(rows)
    }

    fn row(&self, pixel: u32, n: usize, star: CatalogStar) -> CatalogRow {
        let prefix = if self.cat_type == CatType::Astrometric {
            'G'
        } else {
            'T'
        };
        CatalogRow {
            id: format!("{prefix}{pixel}-{n}"),
            ra: star.ra_deg,
            dec: star.dec_deg,
            ra_epoch: star.ra_deg,
            dec_epoch: star.dec_deg,
            pm_ra_masyr: star.pm_ra_masyr,
            pm_dec_masyr: star.pm_dec_masyr,
            g: star.mag,
            bp: None,
            rp: None,
            bp_rp: None,
            parallax_mas: None,
            teff_k: star.teff_k,
        }
    }
}

impl HealpixCatalog<MmapSource> {
    pub fn open_file(path: &Path) -> Result<Self, CatalogReadError> {
        Self::new(MmapSource::open(path)?)
    }
}

#[cfg(feature = "vizier")]
impl HealpixCatalog<HttpRangeSource> {
    pub fn open_http(base_url: &str, file_name: &str) -> Result<Self, CatalogReadError> {
        let url = format!(
            "{}/{}",
            base_url.trim_end_matches('/'),
            file_name.trim_start_matches('/')
        );
        Self::new(HttpRangeSource::open(&url).map_err(CatalogReadError::Http)?)
    }
}

fn by_magnitude(a: Option<f64>, b: Option<f64>) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn check_cone(ra_deg: f64, dec_deg: f64, radius_deg: f64) -> Result<(), CatalogReadError> {
    if !ra_deg.is_finite() {
        return Err(CatalogReadError::Cone(format!(
            "RA {ra_deg} is not a finite number"
        )));
    }
    if !(-90.0..=90.0).contains(&dec_deg) {
        return Err(CatalogReadError::Cone(format!(
            "Dec {dec_deg} is outside -90..=90"
        )));
    }
    if radius_deg.is_nan() || radius_deg <= 0.0 || radius_deg > 180.0 {
        return Err(CatalogReadError::Cone(format!(
            "radius {radius_deg} deg is outside (0, 180]"
        )));
    }
    if radius_deg.to_radians() <= 0.0 {
        return Err(CatalogReadError::Cone(format!(
            "radius {radius_deg} deg is too small to express in radians"
        )));
    }
    Ok(())
}

pub fn pixel_ranges(
    level: u8,
    chunk_level: Option<u8>,
    ra_deg: f64,
    dec_deg: f64,
    radius_deg: f64,
) -> Result<Vec<(u32, Range<u32>)>, CatalogReadError> {
    if level > MAX_LEVEL {
        return Err(CatalogReadError::Cone(format!(
            "level {level} is outside 0..={MAX_LEVEL}"
        )));
    }
    if let Some(chunk_level) = chunk_level.filter(|chunk_level| *chunk_level > level) {
        return Err(CatalogReadError::Cone(format!(
            "chunk level {chunk_level} is above level {level}"
        )));
    }
    check_cone(ra_deg, dec_deg, radius_deg)?;
    if radius_deg > WIDE_CONE_RADIUS_DEG {
        return Ok(group_by_chunk(
            level,
            chunk_level,
            std::iter::once(0..whole_index_entries(level)),
        ));
    }
    let coverage = cdshealpix::nested::cone_coverage_approx_custom(
        level,
        CONE_DELTA_DEPTH,
        ra_deg.to_radians().rem_euclid(TAU),
        dec_deg.to_radians().clamp(-FRAC_PI_2, FRAC_PI_2),
        radius_deg.to_radians().min(PI),
    );
    Ok(group_by_chunk(
        level,
        chunk_level,
        coverage.to_ranges().iter().cloned(),
    ))
}

fn group_by_chunk(
    level: u8,
    chunk_level: Option<u8>,
    ranges: impl IntoIterator<Item = Range<u64>>,
) -> Vec<(u32, Range<u32>)> {
    let shift = chunk_level.map(|chunk_level| 2 * u32::from(level.saturating_sub(chunk_level)));
    let mut runs: Vec<(u32, Range<u32>)> = Vec::new();
    for range in ranges {
        let mut start = saturating_u32(range.start);
        let end = saturating_u32(range.end);
        while start < end {
            let (chunk, piece_end) = match shift {
                Some(shift) => {
                    let chunk = start >> shift;
                    let next_chunk_start = (u64::from(chunk) + 1) << shift;
                    (chunk, end.min(saturating_u32(next_chunk_start)))
                }
                None => (0, end),
            };
            match runs.last_mut() {
                Some((last_chunk, last)) if *last_chunk == chunk && last.end == start => {
                    last.end = piece_end;
                }
                _ => runs.push((chunk, start..piece_end)),
            }
            start = piece_end;
        }
    }
    runs
}

pub fn write_catalog(
    header: &CatHeader,
    pixels: &[(u32, Vec<Record>)],
    out: &mut Vec<u8>,
) -> Result<(), CatalogReadError> {
    header.validate(u64::MAX)?;
    let cat_type = header
        .cat_type()
        .ok_or_else(|| header_error("cat_type", format!("{} is not writable", header.cat_type)))?;
    let first_pixel = u64::from(header.first_pixel());
    let entries = header.index_entries();
    let mut previous: Option<u32> = None;
    for &(pixel, _) in pixels {
        if let Some(previous) = previous.filter(|previous| pixel <= *previous) {
            return Err(encode_error(
                pixel,
                format!(
                    "pixels must be listed once each in ascending order; {pixel} follows {previous}"
                ),
            ));
        }
        let inside = u64::from(pixel)
            .checked_sub(first_pixel)
            .is_some_and(|entry| entry < entries);
        if !inside {
            return Err(encode_error(
                pixel,
                format!(
                    "pixel {pixel} is outside this file's pixels {first_pixel}..={}",
                    first_pixel + entries - 1
                ),
            ));
        }
        previous = Some(pixel);
    }
    out.extend_from_slice(&header.to_bytes());
    let mut cumulative: u32 = 0;
    let mut listed = pixels.iter().peekable();
    for entry in 0..entries {
        if let Some((pixel, records)) =
            listed.next_if(|(pixel, _)| u64::from(*pixel) == first_pixel + entry)
        {
            cumulative = u32::try_from(records.len())
                .ok()
                .and_then(|count| cumulative.checked_add(count))
                .ok_or_else(|| encode_error(*pixel, "more records than a u32 index holds"))?;
        }
        out.extend_from_slice(&cumulative.to_le_bytes());
    }
    for (pixel, records) in pixels {
        for record in records {
            encode_record(cat_type, header.healpix_level, *pixel, record, out)?;
        }
    }
    Ok(())
}

pub fn encode_siril_coordinate(deg: f64) -> Option<i32> {
    let counts = (deg * SIRIL_COORDINATE_COUNTS / DEGREES_PER_TURN).round();
    (f64::from(i32::MIN)..=f64::from(i32::MAX))
        .contains(&counts)
        .then_some(counts as i32)
}

fn decode_siril_coordinate(counts: i32) -> f64 {
    f64::from(counts) * DEGREES_PER_TURN / SIRIL_COORDINATE_COUNTS
}

fn decode_siril_dec(counts: i32) -> Option<f64> {
    let dec = decode_siril_coordinate(counts);
    let pole_with_rounding = 90.0 + DEGREES_PER_TURN / SIRIL_COORDINATE_COUNTS;
    (dec.abs() <= pole_with_rounding).then(|| dec.clamp(-90.0, 90.0))
}

fn scaled_i16(value: f64, scale: f64) -> Option<i16> {
    let scaled = (value * scale).round();
    (-WIDE_LIMIT..=WIDE_LIMIT)
        .contains(&scaled)
        .then_some(scaled as i16)
}

fn wide_value(raw: i16) -> Option<f64> {
    if raw == WIDE_NO_VALUE {
        None
    } else {
        Some(f64::from(raw) / MILLI)
    }
}

pub fn compact_mag_code(mag: Option<f64>) -> Result<u8, String> {
    let Some(mag) = mag else {
        return Ok(COMPACT_NO_MAGNITUDE);
    };
    let code = (mag * COMPACT_MAG_STEPS_PER_MAG + COMPACT_MAG_CODE_AT_ZERO).round();
    if (0.0..f64::from(COMPACT_NO_MAGNITUDE)).contains(&code) {
        Ok(code as u8)
    } else {
        Err(format!(
            "magnitude {mag} is outside the compact range -1.5..=23.9"
        ))
    }
}

fn compact_mag(code: u8) -> Option<f64> {
    if code == COMPACT_NO_MAGNITUDE {
        None
    } else {
        Some((f64::from(code) - COMPACT_MAG_CODE_AT_ZERO) / COMPACT_MAG_STEPS_PER_MAG)
    }
}

fn compact_colour_code(colour: Option<f64>) -> Result<i8, String> {
    let Some(colour) = colour else {
        return Ok(COMPACT_NO_COLOUR);
    };
    let code = (colour * COMPACT_COLOUR_STEPS_PER_MAG).round();
    if (-COLOUR_LIMIT..=COLOUR_LIMIT).contains(&code) {
        Ok(code as i8)
    } else {
        Err(format!(
            "colour {colour} is outside the compact range -2.54..=2.54"
        ))
    }
}

fn compact_colour(code: i8) -> Option<f64> {
    if code == COMPACT_NO_COLOUR {
        None
    } else {
        Some(f64::from(code) / COMPACT_COLOUR_STEPS_PER_MAG)
    }
}

fn pixel_centre(level: u8, pixel: u32) -> Option<(f64, f64)> {
    (level <= MAX_LEVEL && u64::from(pixel) < whole_index_entries(level))
        .then(|| cdshealpix::nested::center(level, u64::from(pixel)))
}

fn standard_coordinates(ra0: f64, dec0: f64, ra: f64, dec: f64) -> Option<(f64, f64)> {
    let (sin_dec0, cos_dec0) = dec0.sin_cos();
    let (sin_dec, cos_dec) = dec.sin_cos();
    let (sin_dra, cos_dra) = (ra - ra0).sin_cos();
    let denominator = sin_dec0 * sin_dec + cos_dec0 * cos_dec * cos_dra;
    if denominator <= 0.0 {
        return None;
    }
    Some((
        cos_dec * sin_dra / denominator,
        (cos_dec0 * sin_dec - sin_dec0 * cos_dec * cos_dra) / denominator,
    ))
}

fn from_standard(ra0: f64, dec0: f64, xi: f64, eta: f64) -> (f64, f64) {
    let (sin_dec0, cos_dec0) = dec0.sin_cos();
    let denominator = cos_dec0 - eta * sin_dec0;
    let ra = ra0 + xi.atan2(denominator);
    let dec = (sin_dec0 + eta * cos_dec0).atan2(xi.hypot(denominator));
    (ra, dec)
}

fn encode_position(pixel: u32, star: &CatalogStar) -> Result<(i32, i32), CatalogReadError> {
    let ra = encode_siril_coordinate(star.ra_deg.rem_euclid(DEGREES_PER_TURN))
        .ok_or_else(|| encode_error(pixel, format!("RA {} cannot be encoded", star.ra_deg)))?;
    let dec = Some(star.dec_deg)
        .filter(|dec| (-90.0..=90.0).contains(dec))
        .and_then(encode_siril_coordinate)
        .ok_or_else(|| encode_error(pixel, format!("Dec {} cannot be encoded", star.dec_deg)))?;
    Ok((ra, dec))
}

fn encode_astrometric(
    pixel: u32,
    star: &CatalogStar,
    out: &mut Vec<u8>,
) -> Result<(), CatalogReadError> {
    let (ra, dec) = encode_position(pixel, star)?;
    let proper_motion = |value: Option<f64>, what: &str| {
        scaled_i16(value.unwrap_or(0.0), 1.0)
            .ok_or_else(|| encode_error(pixel, format!("{what} {value:?} mas/yr exceeds 16 bits")))
    };
    let pm_ra = proper_motion(star.pm_ra_masyr, "proper motion in RA")?;
    let pm_dec = proper_motion(star.pm_dec_masyr, "proper motion in Dec")?;
    let teff = match star.teff_k {
        None => 0,
        Some(teff) => {
            let rounded = teff.round();
            if !(1.0..=f64::from(u16::MAX)).contains(&rounded) {
                return Err(encode_error(
                    pixel,
                    format!("Teff {teff} K exceeds 16 bits"),
                ));
            }
            rounded as u16
        }
    };
    let mag = star
        .mag
        .ok_or_else(|| encode_error(pixel, "an astrometric record needs a magnitude"))
        .and_then(|mag| {
            scaled_i16(mag, MILLI)
                .ok_or_else(|| encode_error(pixel, format!("magnitude {mag} exceeds 16 bits")))
        })?;
    out.extend_from_slice(&ra.to_le_bytes());
    out.extend_from_slice(&dec.to_le_bytes());
    out.extend_from_slice(&pm_ra.to_le_bytes());
    out.extend_from_slice(&pm_dec.to_le_bytes());
    out.extend_from_slice(&teff.to_le_bytes());
    out.extend_from_slice(&mag.to_le_bytes());
    Ok(())
}

fn encode_compact6(
    level: u8,
    pixel: u32,
    star: &CatalogStar,
    out: &mut Vec<u8>,
) -> Result<(), CatalogReadError> {
    let (centre_lon, centre_lat) = pixel_centre(level, pixel)
        .ok_or_else(|| encode_error(pixel, format!("not a level-{level} pixel")))?;
    let (xi, eta) = standard_coordinates(
        centre_lon,
        centre_lat,
        star.ra_deg.to_radians(),
        star.dec_deg.to_radians(),
    )
    .ok_or_else(|| encode_error(pixel, "the star is more than 90 deg from the pixel centre"))?;
    let (xi_arcsec, eta_arcsec) = (xi * ARCSEC_PER_RADIAN, eta * ARCSEC_PER_RADIAN);
    let (Some(xi_steps), Some(eta_steps)) = (
        scaled_i16(xi_arcsec, COMPACT_STEPS_PER_ARCSEC),
        scaled_i16(eta_arcsec, COMPACT_STEPS_PER_ARCSEC),
    ) else {
        return Err(encode_error(
            pixel,
            format!(
                "offset ({xi_arcsec:.1}\", {eta_arcsec:.1}\") from the pixel centre exceeds 16 bits of 0.1\" steps"
            ),
        ));
    };
    let mag = compact_mag_code(star.mag).map_err(|detail| encode_error(pixel, detail))?;
    let colour = compact_colour_code(star.colour).map_err(|detail| encode_error(pixel, detail))?;
    out.extend_from_slice(&xi_steps.to_le_bytes());
    out.extend_from_slice(&eta_steps.to_le_bytes());
    out.push(mag);
    out.extend_from_slice(&colour.to_le_bytes());
    Ok(())
}

fn encode_compact12(
    pixel: u32,
    star: &CatalogStar,
    out: &mut Vec<u8>,
) -> Result<(), CatalogReadError> {
    let (ra, dec) = encode_position(pixel, star)?;
    let wide = |value: Option<f64>, what: &str| match value {
        None => Ok(WIDE_NO_VALUE),
        Some(value) => scaled_i16(value, MILLI)
            .ok_or_else(|| encode_error(pixel, format!("{what} {value} exceeds 16 bits"))),
    };
    let mag = wide(star.mag, "magnitude")?;
    let colour = wide(star.colour, "colour")?;
    out.extend_from_slice(&ra.to_le_bytes());
    out.extend_from_slice(&dec.to_le_bytes());
    out.extend_from_slice(&mag.to_le_bytes());
    out.extend_from_slice(&colour.to_le_bytes());
    Ok(())
}

pub fn encode_record(
    cat_type: CatType,
    level: u8,
    pixel: u32,
    record: &Record,
    out: &mut Vec<u8>,
) -> Result<(), CatalogReadError> {
    let record_len = cat_type.record_len().ok_or_else(|| {
        encode_error(
            pixel,
            format!("{} records have no fixed length", cat_type.name()),
        )
    })?;
    match record {
        Record::Raw(bytes) if bytes.len() == record_len => {
            out.extend_from_slice(bytes);
            Ok(())
        }
        Record::Raw(bytes) => Err(encode_error(
            pixel,
            format!(
                "a raw {} record is {record_len} bytes, not {}",
                cat_type.name(),
                bytes.len()
            ),
        )),
        Record::Star(star) => match cat_type {
            CatType::Astrometric => encode_astrometric(pixel, star, out),
            CatType::Compact6 => encode_compact6(level, pixel, star, out),
            CatType::Compact12 => encode_compact12(pixel, star, out),
            CatType::XpSampled | CatType::XpContinuous => Err(encode_error(
                pixel,
                format!("{} records are written raw", cat_type.name()),
            )),
        },
    }
}

pub fn decode_star(cat_type: CatType, level: u8, pixel: u32, record: &[u8]) -> Option<CatalogStar> {
    if Some(record.len()) != cat_type.record_len() {
        return None;
    }
    match cat_type {
        CatType::Astrometric => Some(CatalogStar {
            ra_deg: decode_siril_coordinate(i32::from_le_bytes(take(record, 0)))
                .rem_euclid(DEGREES_PER_TURN),
            dec_deg: decode_siril_dec(i32::from_le_bytes(take(record, 4)))?,
            pm_ra_masyr: Some(f64::from(i16::from_le_bytes(take(record, 8)))),
            pm_dec_masyr: Some(f64::from(i16::from_le_bytes(take(record, 10)))),
            teff_k: Some(u16::from_le_bytes(take(record, 12)))
                .filter(|teff| *teff > 0)
                .map(f64::from),
            mag: Some(f64::from(i16::from_le_bytes(take(record, 14))) / MILLI),
            colour: None,
        }),
        CatType::Compact6 => {
            let (centre_lon, centre_lat) = pixel_centre(level, pixel)?;
            let steps_to_radians =
                |steps: i16| f64::from(steps) / COMPACT_STEPS_PER_ARCSEC / ARCSEC_PER_RADIAN;
            let (ra, dec) = from_standard(
                centre_lon,
                centre_lat,
                steps_to_radians(i16::from_le_bytes(take(record, 0))),
                steps_to_radians(i16::from_le_bytes(take(record, 2))),
            );
            Some(CatalogStar {
                mag: compact_mag(u8::from_le_bytes(take(record, 4))),
                colour: compact_colour(i8::from_le_bytes(take(record, 5))),
                ..CatalogStar::at(
                    ra.to_degrees().rem_euclid(DEGREES_PER_TURN),
                    dec.to_degrees().clamp(-90.0, 90.0),
                    None,
                )
            })
        }
        CatType::Compact12 => Some(CatalogStar {
            mag: wide_value(i16::from_le_bytes(take(record, 8))),
            colour: wide_value(i16::from_le_bytes(take(record, 10))),
            ..CatalogStar::at(
                decode_siril_coordinate(i32::from_le_bytes(take(record, 0)))
                    .rem_euclid(DEGREES_PER_TURN),
                decode_siril_dec(i32::from_le_bytes(take(record, 4)))?,
                None,
            )
        }),
        CatType::XpSampled | CatType::XpContinuous => None,
    }
}

#[cfg(test)]
mod tests;
