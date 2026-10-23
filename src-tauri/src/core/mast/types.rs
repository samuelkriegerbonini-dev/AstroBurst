use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::core::astrometry::stcs::BoundingCone;
pub use crate::infra::mast::Mission as MastMissionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MastAvailability {
    Open,
    Token,
    NotDeployed,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickFilterId {
    Level,
    Instrument,
    Filter,
    Date,
    Public,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastQuickFilter {
    pub id: QuickFilterId,
    pub label: String,
    pub column: String,
    pub options: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnKind {
    Text,
    Int,
    Float,
    Datetime,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastColumn {
    pub name: String,
    pub label: String,
    pub kind: ColumnKind,
    pub sortable: bool,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionCapabilities {
    pub search: bool,
    pub images: bool,
    pub products: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionViews {
    pub gallery: bool,
    pub table: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterGroupId {
    Category,
    ObjectType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastFilterOption {
    pub id: String,
    pub label: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastFilterGroup {
    pub id: FilterGroupId,
    pub label: String,
    pub column: String,
    pub info: String,
    pub options: Vec<MastFilterOption>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastMission {
    pub id: MastMissionId,
    pub label: String,
    pub availability: MastAvailability,
    pub capabilities: MissionCapabilities,
    pub views: MissionViews,
    pub quick_filters: Vec<MastQuickFilter>,
    pub default_columns: Vec<MastColumn>,
    pub note: Option<String>,
    pub checked_on: String,
    pub filter_groups: Vec<MastFilterGroup>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastResolved {
    pub query: String,
    pub canonical_name: String,
    pub resolver: String,
    pub ra_deg: f64,
    pub dec_deg: f64,
    pub object_type: Option<String>,
    pub elapsed_ms: u64,
    pub from_cache: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum MastWhere {
    None,
    Position {
        ra_deg: f64,
        dec_deg: f64,
        radius_arcmin: f64,
        #[serde(default)]
        label: Option<String>,
    },
    Name {
        name: String,
        radius_arcmin: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelFilter {
    Combined,
    Exposures,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectCategory {
    Galaxies,
    GalaxyClusters,
    Ism,
    StarClusters,
    Stars,
    SolarSystem,
    Fields,
    Calibration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectType {
    PlanetaryNebulae,
    NebulaeStarForming,
    YoungStarsDisks,
    Exoplanets,
    AgnQuasars,
    InteractingGalaxies,
    SupernovaeTransients,
    CompactSubstellar,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MastFilterValues {
    pub level: Option<LevelFilter>,
    pub instrument: Option<String>,
    pub filter: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub public_only: Option<bool>,
    pub categories: Vec<ObjectCategory>,
    pub object_types: Vec<ObjectType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastSort {
    pub column: String,
    #[serde(default)]
    pub desc: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastQuery {
    pub mission: MastMissionId,
    #[serde(rename = "where")]
    pub where_: MastWhere,
    #[serde(default)]
    pub program: Option<i64>,
    #[serde(default)]
    pub dataset_id: Option<String>,
    #[serde(default)]
    pub filters: MastFilterValues,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub sort: Vec<MastSort>,
    #[serde(default)]
    pub offset: i64,
    #[serde(default)]
    pub limit: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastPresetCondition {
    pub column: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastPreset {
    pub id: String,
    pub mission: MastMissionId,
    pub label: String,
    pub description: String,
    pub conditions: Vec<MastPresetCondition>,
    pub products: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MastLevelClass {
    Combined,
    Exposure,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastRow {
    pub dataset_id: String,
    pub level_class: MastLevelClass,
    pub level: Option<String>,
    pub instrument: Option<String>,
    pub filter: Option<String>,
    pub filters: Vec<String>,
    pub exp_type: Option<String>,
    pub target: Option<String>,
    pub program: Option<String>,
    pub observation: Option<String>,
    pub date_obs: Option<String>,
    pub exp_time_s: Option<f64>,
    pub sep_arcmin: Option<f64>,
    pub sep_verified: bool,
    pub access: Option<String>,
    pub release_date: Option<String>,
    pub ra_deg: Option<f64>,
    pub dec_deg: Option<f64>,
    pub s_region: Option<String>,
    pub values: BTreeMap<String, Value>,
    #[serde(default)]
    pub categories: Vec<ObjectCategory>,
    #[serde(default)]
    pub target_description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPosition {
    pub ra_deg: f64,
    pub dec_deg: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedEcho {
    pub conditions: Vec<BTreeMap<String, String>>,
    pub sort_by: Vec<String>,
    pub sort_desc: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastSearchPage {
    pub request_id: String,
    pub mission: MastMissionId,
    pub columns: Vec<MastColumn>,
    pub rows: Vec<MastRow>,
    pub offset: i64,
    pub limit: i64,
    pub has_more: bool,
    pub total_reported: Option<i64>,
    pub total_reliable: bool,
    pub radius_arcmin: Option<f64>,
    pub radius_clamped: bool,
    pub search_position: Option<SearchPosition>,
    pub applied: AppliedEcho,
    #[serde(default)]
    pub dropped_conditions: Vec<String>,
    pub request_body: Value,
    pub messages: Vec<String>,
    pub unresolved_targets: Vec<String>,
    pub elapsed_ms: u64,
    pub from_cache: bool,
    #[serde(default)]
    pub level_filtered_locally: bool,
    #[serde(default)]
    pub level_applied_locally: Option<String>,
    #[serde(default)]
    pub unfiltered_total: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastImageEntry {
    pub key: String,
    pub label: String,
    pub group: String,
    pub product_filename: String,
    pub thumbnail_filename: Option<String>,
    pub preview_filename: Option<String>,
    pub level: Option<String>,
    pub suffix: String,
    pub detector: Option<String>,
    pub filter: Option<String>,
    pub science_group: Option<String>,
    pub colour: bool,
    pub mast_uri: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGroup {
    pub key: String,
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastImageSet {
    pub mission: MastMissionId,
    pub dataset_id: String,
    pub entries: Vec<MastImageEntry>,
    pub groups: Vec<ImageGroup>,
    pub default_group: String,
    pub representative: Option<usize>,
    pub elapsed_ms: u64,
    pub from_cache: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WcsOrientation {
    CdMatrix,
    Unreliable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastQuickWcs {
    pub centre_ra_deg: f64,
    pub centre_dec_deg: f64,
    pub pixel_scale_arcsec: f64,
    pub field_arcmin: [f64; 2],
    pub orientation: WcsOrientation,
    pub ctype: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastImageRef {
    pub request_id: String,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub filename: String,
    pub wcs: Option<MastQuickWcs>,
    pub image_count: Option<usize>,
    pub elapsed_ms: u64,
    pub from_cache: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewSize {
    Pane,
    Large,
}

impl PreviewSize {
    pub const fn long_edge(self) -> u32 {
        match self {
            PreviewSize::Pane => 2048,
            PreviewSize::Large => 4096,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSource {
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastPreviewRef {
    #[serde(flatten)]
    pub image: MastImageRef,
    pub size: PreviewSize,
    pub channels: u8,
    pub source: PreviewSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MastProductClass {
    Image,
    Cube,
    Spectrum,
    Ramp,
    Catalog,
    Preview,
    Association,
    Auxiliary,
    Reference,
    Guidestar,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpensAs {
    Image,
    Cube,
    Spectrum,
    Ramp,
    Catalog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductBadge {
    HapAligned,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastProduct {
    pub dataset_id: String,
    pub filename: String,
    pub size_bytes: u64,
    pub category: String,
    #[serde(rename = "type")]
    pub product_type: String,
    pub suffix: String,
    pub cls: MastProductClass,
    pub opens_as: Option<OpensAs>,
    pub access: String,
    pub mast_uri: String,
    pub download_url: String,
    pub badges: Vec<ProductBadge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductGroup {
    pub cls: MastProductClass,
    pub count: usize,
    pub bytes: u64,
    pub default_visible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastProductList {
    pub mission: MastMissionId,
    pub dataset_ids: Vec<String>,
    pub products: Vec<MastProduct>,
    pub groups: Vec<ProductGroup>,
    pub total_count: usize,
    pub total_bytes: u64,
    pub elapsed_ms: u64,
    pub from_cache: bool,
    pub cached_until: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastTokenStatus {
    pub present: bool,
    pub authorized: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastCacheInfo {
    pub bytes: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MastCacheKind {
    Images,
    Lists,
    Names,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStage {
    Download,
    Decode,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastProgressEvent {
    pub request_id: String,
    pub received: u64,
    pub total: Option<u64>,
    pub stage: ProgressStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MastErrorCode {
    #[serde(rename = "mast_network")]
    Network,
    #[serde(rename = "mast_timeout")]
    Timeout,
    #[serde(rename = "mast_cancelled")]
    Cancelled,
    #[serde(rename = "mast_rate_limited")]
    RateLimited,
    #[serde(rename = "mast_server")]
    Server,
    #[serde(rename = "mast_rejected")]
    Rejected,
    #[serde(rename = "mast_unauthorized")]
    Unauthorized,
    #[serde(rename = "mast_no_image")]
    NoImage,
    #[serde(rename = "mast_not_deployed")]
    NotDeployed,
    #[serde(rename = "mast_decode")]
    Decode,
    #[serde(rename = "mast_too_large")]
    TooLarge,
    #[serde(rename = "mast_not_image")]
    NotImage,
    #[serde(rename = "mast_bad_input")]
    BadInput,
    #[serde(rename = "mast_token_rejected")]
    TokenRejected,
    #[serde(rename = "mast_cache")]
    Cache,
    #[serde(rename = "mast_size_mismatch")]
    SizeMismatch,
    #[serde(rename = "mast_busy")]
    Busy,
    #[serde(rename = "mast_stale")]
    Stale,
    #[serde(rename = "mast_quota")]
    Quota,
    #[serde(rename = "mast_uri_refused")]
    UriRefused,
    #[serde(rename = "mast_peek_incomplete")]
    PeekIncomplete,
}

impl MastErrorCode {
    pub const ALL: [MastErrorCode; 21] = [
        MastErrorCode::Network,
        MastErrorCode::Timeout,
        MastErrorCode::Cancelled,
        MastErrorCode::RateLimited,
        MastErrorCode::Server,
        MastErrorCode::Rejected,
        MastErrorCode::Unauthorized,
        MastErrorCode::NoImage,
        MastErrorCode::NotDeployed,
        MastErrorCode::Decode,
        MastErrorCode::TooLarge,
        MastErrorCode::NotImage,
        MastErrorCode::BadInput,
        MastErrorCode::TokenRejected,
        MastErrorCode::Cache,
        MastErrorCode::SizeMismatch,
        MastErrorCode::Busy,
        MastErrorCode::Stale,
        MastErrorCode::Quota,
        MastErrorCode::UriRefused,
        MastErrorCode::PeekIncomplete,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            MastErrorCode::Network => "mast_network",
            MastErrorCode::Timeout => "mast_timeout",
            MastErrorCode::Cancelled => "mast_cancelled",
            MastErrorCode::RateLimited => "mast_rate_limited",
            MastErrorCode::Server => "mast_server",
            MastErrorCode::Rejected => "mast_rejected",
            MastErrorCode::Unauthorized => "mast_unauthorized",
            MastErrorCode::NoImage => "mast_no_image",
            MastErrorCode::NotDeployed => "mast_not_deployed",
            MastErrorCode::Decode => "mast_decode",
            MastErrorCode::TooLarge => "mast_too_large",
            MastErrorCode::NotImage => "mast_not_image",
            MastErrorCode::BadInput => "mast_bad_input",
            MastErrorCode::TokenRejected => "mast_token_rejected",
            MastErrorCode::Cache => "mast_cache",
            MastErrorCode::SizeMismatch => "mast_size_mismatch",
            MastErrorCode::Busy => "mast_busy",
            MastErrorCode::Stale => "mast_stale",
            MastErrorCode::Quota => "mast_quota",
            MastErrorCode::UriRefused => "mast_uri_refused",
            MastErrorCode::PeekIncomplete => "mast_peek_incomplete",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MastDownloadState {
    Queued,
    Downloading,
    Done,
    Error,
    Cancelled,
}

impl MastDownloadState {
    pub const fn is_active(self) -> bool {
        matches!(self, MastDownloadState::Queued | MastDownloadState::Downloading)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MastDownloadAction {
    Resume,
    Retry,
    Replace,
    Redownload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastDownloadInput {
    pub dataset_id: String,
    pub filename: String,
    #[serde(default)]
    pub program: Option<String>,
    pub size_bytes: u64,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub opens_as: Option<OpensAs>,
    #[serde(default)]
    pub replace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastDownloadItem {
    pub id: String,
    pub request_id: String,
    pub mission: MastMissionId,
    pub dataset_id: String,
    pub filename: String,
    pub program: Option<String>,
    pub uri: String,
    pub size_bytes: u64,
    pub opens_as: Option<OpensAs>,
    pub open: bool,
    pub replace: bool,
    pub state: MastDownloadState,
    pub received: u64,
    pub total: Option<u64>,
    pub path: String,
    pub resumed_from: u64,
    pub reused: bool,
    pub verified: Option<bool>,
    pub action: Option<MastDownloadAction>,
    pub error: Option<MastCmdError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastFootprintInput {
    pub dataset_id: String,
    #[serde(default)]
    pub s_region: Option<String>,
    #[serde(default)]
    pub polygons: Option<Vec<Vec<[f64; 2]>>>,
    #[serde(default)]
    pub targtype: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MastFootprint {
    pub dataset_id: String,
    pub polygons: Vec<Vec<[f64; 2]>>,
    pub on_image: bool,
    pub vertices: usize,
    pub cone: Option<BoundingCone>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MastFootprintSet {
    pub path: String,
    pub width: usize,
    pub height: usize,
    pub footprints: Vec<MastFootprint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MastFootprintNote {
    pub dataset_id: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MastFootprintRegions {
    pub reg_text: String,
    pub polygons: usize,
    pub notes: Vec<MastFootprintNote>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastPeekHdu {
    pub index: usize,
    pub extname: Option<String>,
    pub extver: Option<i64>,
    pub bitpix: i64,
    pub dims: Vec<u64>,
    pub wcs_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastPeekFootprint {
    pub hdu: usize,
    pub corners: [[f64; 2]; 4],
    pub center: [f64; 2],
    pub fov_arcmin: [f64; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastPeek {
    pub dataset_id: String,
    pub filename: String,
    pub uri: String,
    pub dims: Vec<u64>,
    pub bitpix: Option<i64>,
    pub bunit: Option<String>,
    pub exptime: Option<f64>,
    pub date_obs: Option<String>,
    pub filter: Option<String>,
    pub instrume: Option<String>,
    pub footprints: Vec<MastPeekFootprint>,
    pub hdus: Vec<MastPeekHdu>,
    pub bytes_fetched: u64,
    pub ranges: usize,
    pub file_bytes: Option<u64>,
    pub complete: bool,
    pub range_ignored: bool,
    pub note: Option<String>,
    pub etag: Option<String>,
    pub elapsed_ms: u64,
    pub from_cache: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MastErrorContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission: Option<MastMissionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consecutive: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MastCmdError {
    pub code: MastErrorCode,
    pub message: String,
    pub context: MastErrorContext,
}

impl MastCmdError {
    pub fn new(code: MastErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            context: MastErrorContext::default(),
        }
    }

    pub fn with_context(mut self, context: MastErrorContext) -> Self {
        self.context = context;
        self
    }
}

impl fmt::Display for MastCmdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for MastCmdError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn query_decodes_the_frozen_contract_shape() {
        let query: MastQuery = serde_json::from_value(json!({
            "mission": "jwst",
            "where": {"kind": "position", "raDeg": 274.688, "decDeg": -13.792, "radiusArcmin": 3.0, "label": "M 16"},
            "program": null,
            "datasetId": null,
            "filters": {"level": "combined", "dateFrom": "2022-01-01", "publicOnly": true},
            "sort": [{"column": "date_obs", "desc": true}],
            "offset": 0,
            "limit": 200
        }))
        .unwrap();
        assert_eq!(query.mission, MastMissionId::Jwst);
        assert_eq!(
            query.where_,
            MastWhere::Position { ra_deg: 274.688, dec_deg: -13.792, radius_arcmin: 3.0, label: Some("M 16".into()) }
        );
        assert_eq!(query.filters.level, Some(LevelFilter::Combined));
        assert_eq!(query.filters.public_only, Some(true));
        let name: MastWhere = serde_json::from_value(json!({"kind": "name", "name": "M16", "radiusArcmin": 2.0})).unwrap();
        assert_eq!(name, MastWhere::Name { name: "M16".into(), radius_arcmin: 2.0 });
        assert_eq!(serde_json::to_value(MastWhere::None).unwrap(), json!({"kind": "none"}));
        assert_eq!(query.preset, None);
    }

    #[test]
    fn query_carries_an_optional_preset_and_pages_name_dropped_conditions() {
        let query: MastQuery = serde_json::from_value(json!({
            "mission": "jwst",
            "where": {"kind": "name", "name": "WASP-39", "radiusArcmin": 3.0},
            "filters": {},
            "preset": "tso",
            "offset": 0,
            "limit": 200
        }))
        .unwrap();
        assert_eq!(query.preset.as_deref(), Some("tso"));
        assert_eq!(serde_json::to_value(&query).unwrap()["preset"], json!("tso"));
        let none: MastQuery = serde_json::from_value(json!({"mission": "hst", "where": {"kind": "none"}, "preset": null})).unwrap();
        assert_eq!(none.preset, None);
        let page = MastSearchPage {
            request_id: "g1:search".into(),
            mission: MastMissionId::Jwst,
            columns: Vec::new(),
            rows: Vec::new(),
            offset: 0,
            limit: 200,
            has_more: false,
            total_reported: Some(0),
            total_reliable: true,
            radius_arcmin: None,
            radius_clamped: false,
            search_position: None,
            applied: AppliedEcho::default(),
            dropped_conditions: vec!["tsovisit".into()],
            request_body: Value::Null,
            messages: Vec::new(),
            unresolved_targets: Vec::new(),
            elapsed_ms: 0,
            from_cache: false,
            level_filtered_locally: true,
            level_applied_locally: Some("productLevel = 3".into()),
            unfiltered_total: Some(1913),
        };
        let value = serde_json::to_value(&page).unwrap();
        assert_eq!(value["droppedConditions"], json!(["tsovisit"]));
        assert_eq!((&value["levelFilteredLocally"], &value["unfilteredTotal"]), (&json!(true), &json!(1913)));
        assert_eq!(value["levelAppliedLocally"], json!("productLevel = 3"));
        let mut old = value;
        old.as_object_mut().unwrap().retain(|key, _| !["levelFilteredLocally", "levelAppliedLocally", "unfilteredTotal"].contains(&key.as_str()));
        let decoded: MastSearchPage = serde_json::from_value(old).unwrap();
        assert_eq!((decoded.level_filtered_locally, decoded.level_applied_locally, decoded.unfiltered_total), (false, None, None));
    }

    #[test]
    fn filters_decode_the_three_levels_and_object_ids_and_reject_unknown_ids() {
        let filters: MastFilterValues = serde_json::from_value(json!({
            "level": "exposures",
            "instrument": "NIRCAM",
            "publicOnly": true,
            "categories": ["galaxies", "ism"],
            "objectTypes": ["planetary_nebulae", "compact_substellar"]
        }))
        .unwrap();
        assert_eq!(filters.level, Some(LevelFilter::Exposures));
        assert_eq!(filters.categories, [ObjectCategory::Galaxies, ObjectCategory::Ism]);
        assert_eq!(filters.object_types, [ObjectType::PlanetaryNebulae, ObjectType::CompactSubstellar]);
        let old: MastFilterValues = serde_json::from_value(json!({"level": "combined", "dateFrom": "2022-01-01"})).unwrap();
        assert_eq!((old.categories.len(), old.object_types.len()), (0, 0));
        for (level, id) in [(LevelFilter::Combined, "combined"), (LevelFilter::Exposures, "exposures"), (LevelFilter::All, "all")] {
            assert_eq!(serde_json::to_value(level).unwrap(), json!(id));
        }
        let ids: Vec<Value> = [
            ObjectCategory::Galaxies,
            ObjectCategory::GalaxyClusters,
            ObjectCategory::Ism,
            ObjectCategory::StarClusters,
            ObjectCategory::Stars,
            ObjectCategory::SolarSystem,
            ObjectCategory::Fields,
            ObjectCategory::Calibration,
        ]
        .iter()
        .map(|id| serde_json::to_value(id).unwrap())
        .collect();
        assert_eq!(
            ids,
            ["galaxies", "galaxy_clusters", "ism", "star_clusters", "stars", "solar_system", "fields", "calibration"].map(Value::from)
        );
        let types: Vec<Value> = [
            ObjectType::PlanetaryNebulae,
            ObjectType::NebulaeStarForming,
            ObjectType::YoungStarsDisks,
            ObjectType::Exoplanets,
            ObjectType::AgnQuasars,
            ObjectType::InteractingGalaxies,
            ObjectType::SupernovaeTransients,
            ObjectType::CompactSubstellar,
        ]
        .iter()
        .map(|id| serde_json::to_value(id).unwrap())
        .collect();
        assert_eq!(
            types,
            [
                "planetary_nebulae",
                "nebulae_star_forming",
                "young_stars_disks",
                "exoplanets",
                "agn_quasars",
                "interacting_galaxies",
                "supernovae_transients",
                "compact_substellar"
            ]
            .map(Value::from)
        );
        let unknown = serde_json::from_value::<MastFilterValues>(json!({"categories": ["nebulas"]})).unwrap_err().to_string();
        assert!(unknown.starts_with("unknown variant `nebulas`"), "{unknown}");
        assert!(serde_json::from_value::<MastFilterValues>(json!({"objectTypes": ["Galaxies"]})).is_err());
        assert!(serde_json::from_value::<MastFilterValues>(json!({"level": "l3"})).is_err());
    }

    #[test]
    fn error_serialises_all_three_keys_and_skips_empty_context_fields() {
        let error = MastCmdError::new(MastErrorCode::Cancelled, "cancelled");
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!({"code": "mast_cancelled", "message": "cancelled", "context": {}})
        );
        let error = MastCmdError::new(MastErrorCode::RateLimited, "rate limited").with_context(MastErrorContext {
            status: Some(429),
            retry_after_sec: Some(5),
            consecutive: Some(1),
            mission: Some(MastMissionId::RomanCgi),
            ..Default::default()
        });
        assert_eq!(
            serde_json::to_value(&error).unwrap()["context"],
            json!({"status": 429, "retryAfterSec": 5, "consecutive": 1, "mission": "roman_cgi"})
        );
        for code in MastErrorCode::ALL {
            assert_eq!(serde_json::to_value(code).unwrap(), json!(code.as_str()));
        }
        for (kind, text) in [
            (MastCacheKind::Images, "images"),
            (MastCacheKind::Lists, "lists"),
            (MastCacheKind::Names, "names"),
            (MastCacheKind::All, "all"),
        ] {
            assert_eq!(serde_json::from_value::<MastCacheKind>(json!(text)).unwrap(), kind);
        }
        assert!(serde_json::from_value::<MastCacheKind>(json!("img")).is_err());
    }

    #[test]
    fn download_codes_join_the_contract_list_and_items_use_camel_case() {
        let names: Vec<&str> = MastErrorCode::ALL.iter().map(|code| code.as_str()).collect();
        assert_eq!(
            names,
            [
                "mast_network",
                "mast_timeout",
                "mast_cancelled",
                "mast_rate_limited",
                "mast_server",
                "mast_rejected",
                "mast_unauthorized",
                "mast_no_image",
                "mast_not_deployed",
                "mast_decode",
                "mast_too_large",
                "mast_not_image",
                "mast_bad_input",
                "mast_token_rejected",
                "mast_cache",
                "mast_size_mismatch",
                "mast_busy",
                "mast_stale",
                "mast_quota",
                "mast_uri_refused",
                "mast_peek_incomplete",
            ]
        );
        for code in MastErrorCode::ALL {
            assert_eq!(serde_json::from_value::<MastErrorCode>(json!(code.as_str())).unwrap(), code);
        }

        let input: MastDownloadInput = serde_json::from_value(json!({
            "datasetId": "jw02739001001_02105_00001_nrca1",
            "filename": "jw02739001001_02105_00001_nrca1_cal.fits",
            "sizeBytes": 117_573_120u64
        }))
        .unwrap();
        assert_eq!((input.program, input.uri, input.opens_as, input.replace), (None, None, None, false));

        let item = MastDownloadItem {
            id: "dl1:0".into(),
            request_id: "dl1".into(),
            mission: MastMissionId::Jwst,
            dataset_id: "d".into(),
            filename: "d_cal.fits".into(),
            program: Some("2739".into()),
            uri: "mast:JWST/product/d_cal.fits".into(),
            size_bytes: 10,
            opens_as: Some(OpensAs::Image),
            open: true,
            replace: true,
            state: MastDownloadState::Error,
            received: 4,
            total: Some(10),
            path: "C:/data/mast/JWST/2739/d_cal.fits".into(),
            resumed_from: 0,
            reused: false,
            verified: None,
            action: Some(MastDownloadAction::Redownload),
            error: Some(MastCmdError::new(MastErrorCode::Stale, "MAST has a newer version of this file")),
        };
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["requestId"], json!("dl1"));
        assert_eq!(value["datasetId"], json!("d"));
        assert_eq!(value["sizeBytes"], json!(10));
        assert_eq!(value["opensAs"], json!("image"));
        assert_eq!(value["resumedFrom"], json!(0));
        assert_eq!(value["state"], json!("error"));
        assert_eq!(value["action"], json!("redownload"));
        assert_eq!(value["replace"], json!(true));
        assert_eq!(value["verified"], json!(null));
        assert_eq!(value["error"]["code"], json!("mast_stale"));
        assert!(MastDownloadState::Queued.is_active() && MastDownloadState::Downloading.is_active());
        assert!(!MastDownloadState::Done.is_active() && !MastDownloadState::Cancelled.is_active());
    }

    #[test]
    fn footprint_items_decode_with_optional_sources_and_results_use_camel_case() {
        let bare: MastFootprintInput = serde_json::from_value(json!({"datasetId": "jw02739-o001_t001_nircam"})).unwrap();
        assert_eq!((bare.s_region, bare.polygons, bare.targtype), (None, None, None));
        let peeked: MastFootprintInput = serde_json::from_value(json!({
            "datasetId": "j9irw3fwq",
            "sRegion": null,
            "polygons": [[[274.7, -13.8], [274.8, -13.8], [274.8, -13.7]]],
            "targtype": null
        }))
        .unwrap();
        assert_eq!(peeked.polygons, Some(vec![vec![[274.7, -13.8], [274.8, -13.8], [274.8, -13.7]]]));
        assert!(serde_json::from_value::<MastFootprintInput>(json!({"datasetId": "a", "polygons": [[[1.0]]]})).is_err());

        let set = MastFootprintSet {
            path: "C:/data/a_cal.fits".into(),
            width: 2048,
            height: 2048,
            footprints: vec![MastFootprint {
                dataset_id: "a".into(),
                polygons: vec![vec![[0.5, 1.5], [2.0, 3.0], [4.0, 0.0]]],
                on_image: true,
                vertices: 3,
                cone: Some(BoundingCone { ra: 274.7, dec: -13.8, radius_deg: 0.05 }),
                note: None,
            }],
        };
        let value = serde_json::to_value(&set).unwrap();
        assert_eq!(value["footprints"][0]["datasetId"], json!("a"));
        assert_eq!(value["footprints"][0]["onImage"], json!(true));
        assert_eq!(value["footprints"][0]["polygons"][0][0], json!([0.5, 1.5]));
        assert_eq!(value["footprints"][0]["cone"], json!({"ra": 274.7, "dec": -13.8, "radiusDeg": 0.05}));
        assert_eq!(value["footprints"][0]["note"], json!(null));
        let regions = MastFootprintRegions {
            reg_text: "icrs\n".into(),
            polygons: 0,
            notes: vec![MastFootprintNote { dataset_id: "b".into(), note: "moving".into() }],
        };
        assert_eq!(
            serde_json::to_value(&regions).unwrap(),
            json!({"regText": "icrs\n", "polygons": 0, "notes": [{"datasetId": "b", "note": "moving"}]})
        );
    }

    #[test]
    fn preview_ref_flattens_the_image_ref() {
        let preview = MastPreviewRef {
            image: MastImageRef {
                request_id: "g1:preview:a".into(),
                path: "C:/x/a.2048.jpg".into(),
                width: 2048,
                height: 1211,
                bytes: 10,
                filename: "a.jpg".into(),
                wcs: None,
                image_count: Some(6),
                elapsed_ms: 1,
                from_cache: false,
            },
            size: PreviewSize::Pane,
            channels: 1,
            source: PreviewSource { width: 7065, height: 4177, bytes: 99 },
        };
        let value = serde_json::to_value(&preview).unwrap();
        assert_eq!(value["requestId"], json!("g1:preview:a"));
        assert_eq!(value["imageCount"], json!(6));
        assert_eq!(value["size"], json!("pane"));
        assert_eq!(value["source"]["width"], json!(7065));
        assert!(value.get("image").is_none());
    }

    #[test]
    fn peek_results_use_camel_case_and_round_trip_through_the_cache_shape() {
        let peek = MastPeek {
            dataset_id: "j9irw3fwq".into(),
            filename: "j9irw3fwq_flc.fits".into(),
            uri: "mast:HST/product/j9irw3fwq_flc.fits".into(),
            dims: vec![4096, 2048],
            bitpix: Some(-32),
            bunit: Some("ELECTRONS".into()),
            exptime: Some(507.0),
            date_obs: Some("2004-09-12T06:35:21".into()),
            filter: Some("F606W".into()),
            instrume: Some("ACS".into()),
            footprints: vec![MastPeekFootprint {
                hdu: 1,
                corners: [[150.0, 2.0], [150.1, 2.0], [150.1, 2.1], [150.0, 2.1]],
                center: [150.05, 2.05],
                fov_arcmin: [3.4, 1.7],
            }],
            hdus: vec![MastPeekHdu {
                index: 1,
                extname: Some("SCI".into()),
                extver: Some(1),
                bitpix: -32,
                dims: vec![4096, 2048],
                wcs_error: None,
            }],
            bytes_fetched: 524_288,
            ranges: 7,
            file_bytes: Some(168_000_000),
            complete: true,
            range_ignored: false,
            note: None,
            etag: Some("\"ab\"".into()),
            elapsed_ms: 12,
            from_cache: false,
        };
        let value = serde_json::to_value(&peek).unwrap();
        let keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        for key in ["datasetId", "dateObs", "bytesFetched", "fileBytes", "rangeIgnored", "elapsedMs", "fromCache"] {
            assert!(keys.contains(&key), "{key} missing from {keys:?}");
        }
        assert_eq!(value["footprints"][0]["fovArcmin"], json!([3.4, 1.7]));
        assert_eq!(value["hdus"][0]["wcsError"], json!(null));
        assert_eq!(serde_json::from_value::<MastPeek>(value).unwrap(), peek);
    }
}
