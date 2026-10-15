use serde::{Deserialize, Serialize};

use crate::infra::mast::models::{FieldMap, SearchResults};

pub type JwstSearchResults = SearchResults<JwstSearchResultItem>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JwstSearchResultItem {
    pub search_pos: Option<String>,
    #[serde(rename = "ArchiveFileID")]
    pub archive_file_id: Option<i64>,
    #[serde(rename = "fileSetName")]
    pub file_set_name: Option<String>,
    #[serde(rename = "includedLevels")]
    pub included_levels: Option<String>,
    #[serde(rename = "productLevel")]
    pub product_level: Option<String>,
    pub sdp_ver: Option<String>,
    pub s_region: Option<String>,
    pub readpatt: Option<String>,
    pub targname: Option<String>,
    pub targ_ra: Option<f64>,
    pub targ_dec: Option<f64>,
    pub instrume: Option<String>,
    pub exp_type: Option<String>,
    #[serde(rename = "opticalElements")]
    pub optical_elements: Option<String>,
    pub filter: Option<String>,
    pub date_obs: Option<String>,
    pub duration: Option<f64>,
    pub program: Option<i64>,
    pub observtn: Option<i64>,
    pub visit: Option<i64>,
    #[serde(rename = "publicReleaseDate")]
    pub public_release_date: Option<String>,
    pub pi_name: Option<String>,
    pub proposal_type: Option<String>,
    pub proposal_cycle: Option<i64>,
    pub targtype: Option<String>,
    pub obslabel: Option<String>,
    pub template: Option<String>,
    pub access: Option<String>,
    pub act_id: Option<String>,
    pub apername: Option<String>,
    #[serde(flatten)]
    pub extra: FieldMap,
}
