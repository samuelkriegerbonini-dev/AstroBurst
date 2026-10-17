use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    All,
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
    pub sort: Vec<MastSort>,
    #[serde(default)]
    pub offset: i64,
    #[serde(default)]
    pub limit: i64,
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
    pub request_body: Value,
    pub messages: Vec<String>,
    pub unresolved_targets: Vec<String>,
    pub elapsed_ms: u64,
    pub from_cache: bool,
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
}

impl MastErrorCode {
    pub const ALL: [MastErrorCode; 15] = [
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
        }
    }
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
}
