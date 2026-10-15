use serde::{Deserialize, Serialize};

use crate::infra::mast::models::{FieldMap, SearchResults};

pub type UllysesSearchResults = SearchResults<UllysesSearchResultItem>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UllysesSearchResultItem {
    pub search_pos: Option<String>,
    pub target_id: Option<String>,
    pub target_name_std: Option<String>,
    pub target_name_simbad: Option<String>,
    pub target_name_ullyses: Option<String>,
    pub target_classification: Option<String>,
    pub targ_ra: Option<f64>,
    pub targ_dec: Option<f64>,
    pub host_galaxy_name: Option<String>,
    pub host_cluster_name: Option<String>,
    pub spectral_type: Option<String>,
    pub observatory: Option<String>,
    pub instrument: Option<String>,
    pub grating: Option<String>,
    pub cenwave: Option<String>,
    pub filter: Option<String>,
    pub observation_id: Option<String>,
    pub date_beg: Option<String>,
    pub exptime_total: Option<f64>,
    pub dataproduct_type: Option<String>,
    #[serde(flatten)]
    pub extra: FieldMap,
}
