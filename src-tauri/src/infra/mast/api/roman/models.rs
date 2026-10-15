use serde::{Deserialize, Serialize};

use crate::infra::mast::models::{DatasetProduct, DatasetProducts, FieldMap, SearchResults};

pub type RomanSearchResults = SearchResults<RomanSearchResultItem>;
pub type RomanDatasetProducts = DatasetProducts<RomanDatasetProduct>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RomanSearchResultItem {
    pub search_pos: Option<String>,
    pub dataset_category: Option<String>,
    #[serde(rename = "fileSetName")]
    pub file_set_name: Option<String>,
    pub filename: Option<String>,
    pub ra: Option<f64>,
    pub dec: Option<f64>,
    pub pi_name: Option<String>,
    pub instrument_name: Option<String>,
    pub exposure_time: Option<f64>,
    pub exposure_type: Option<String>,
    pub exposure_start_time: Option<String>,
    pub exposure_end_time: Option<String>,
    pub program: Option<i64>,
    pub visit: Option<i64>,
    pub observation: Option<String>,
    pub detector: Option<String>,
    pub optical_element: Option<String>,
    pub skycell_name: Option<String>,
    pub access: Option<String>,
    pub s_region: Option<String>,
    #[serde(rename = "ArchiveFileID")]
    pub archive_file_id: Option<String>,
    pub data_release: Option<String>,
    pub calibration_software_version: Option<String>,
    #[serde(flatten)]
    pub extra: FieldMap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RomanDatasetProduct {
    #[serde(flatten)]
    pub product: DatasetProduct,
    #[serde(default)]
    pub calibration_software_version: String,
    #[serde(default)]
    pub crds_context_used: String,
    #[serde(default)]
    pub crds_software_version: String,
    #[serde(default)]
    pub edp_url: String,
}
