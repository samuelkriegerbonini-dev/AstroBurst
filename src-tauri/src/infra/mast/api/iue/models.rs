use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::infra::mast::models::{FieldMap, SearchResults};

pub type IueSearchResults = SearchResults<IueSearchResultItem>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IueSearchResultItem {
    pub iue_data_id: Option<String>,
    pub iue_go_object_name: Option<String>,
    pub iue_target_name: Option<String>,
    pub iue_unique_id: Option<String>,
    pub iue_ra2000: Option<f64>,
    pub iue_dec2000: Option<f64>,
    pub iue_camera_name: Option<Value>,
    pub iue_disp: Option<Value>,
    pub iue_aper: Option<Value>,
    pub iue_obs_start_time: Option<Value>,
    pub iue_com_exp_time: Option<Value>,
    pub iue_pi_name: Option<Value>,
    pub search_key: Option<String>,
    #[serde(flatten)]
    pub extra: FieldMap,
}
