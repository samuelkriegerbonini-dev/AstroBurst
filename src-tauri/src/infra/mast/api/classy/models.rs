use serde::{Deserialize, Serialize};

use crate::infra::mast::models::{FieldMap, SearchResults};

pub type ClassySearchResults = SearchResults<ClassySearchResultItem>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClassySearchResultItem {
    pub search_pos: Option<String>,
    #[serde(rename = "Target")]
    pub target: Option<String>,
    pub target_name_csv: Option<String>,
    #[serde(rename = "Alt_Name")]
    pub alt_name: Option<String>,
    pub z: Option<f64>,
    pub distance: Option<f64>,
    #[serde(rename = "log_Mstar")]
    pub log_mstar: Option<f64>,
    #[serde(rename = "log_SFR")]
    pub log_sfr: Option<f64>,
    #[serde(rename = "RA_deg")]
    pub ra_deg: Option<f64>,
    #[serde(rename = "DEC_deg")]
    pub dec_deg: Option<f64>,
    pub ang_sep: Option<f64>,
    pub search_key: Option<String>,
    #[serde(flatten)]
    pub extra: FieldMap,
}
