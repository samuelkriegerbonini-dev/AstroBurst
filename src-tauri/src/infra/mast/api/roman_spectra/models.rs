use serde::{Deserialize, Serialize};

use crate::infra::mast::models::{FieldMap, SearchResults};

pub type RomanSpectraSearchResults = SearchResults<RomanSpectraSearchResultItem>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RomanSpectraSearchResultItem {
    pub search_pos: Option<String>,
    pub source_data_id: Option<i64>,
    pub id_spec_source: Option<String>,
    pub wfi_spec_1d_file_name: Option<String>,
    pub ra: Option<f64>,
    pub dec: Option<f64>,
    pub classification: Option<String>,
    pub redshift: Option<f64>,
    pub redshift_err: Option<f64>,
    pub redshift_reliability_index: Option<f64>,
    pub stat_snr_global: Option<f64>,
    pub phot_mag: Option<f64>,
    pub phot_mag_error: Option<f64>,
    pub max_line_name: Option<String>,
    pub obs_exposure_time: Option<f64>,
    pub search_key: Option<String>,
    #[serde(flatten)]
    pub extra: FieldMap,
}
