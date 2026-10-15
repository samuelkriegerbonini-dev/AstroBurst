use serde::{Deserialize, Serialize};

use crate::infra::mast::models::{FieldMap, SearchResults};

pub type TessSearchResults = SearchResults<TessSearchResultItem>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TessSearchResultItem {
    pub search_pos: Option<String>,
    #[serde(rename = "fileSetName")]
    pub file_set_name: Option<String>,
    #[serde(rename = "ArchiveFileID")]
    pub archive_file_id: Option<i64>,
    #[serde(rename = "fileName")]
    pub file_name: Option<String>,
    #[serde(rename = "fileSize")]
    pub file_size: Option<i64>,
    #[serde(rename = "publicReleaseDate")]
    pub public_release_date: Option<String>,
    pub mission: Option<String>,
    pub camera: Option<i64>,
    pub ccd: Option<i64>,
    pub sector: Option<i64>,
    pub ticid: Option<i64>,
    pub object: Option<String>,
    pub ra_obj: Option<f64>,
    pub dec_obj: Option<f64>,
    pub tessmag: Option<f64>,
    pub teff: Option<f64>,
    pub ang_sep: Option<f64>,
    #[serde(flatten)]
    pub extra: FieldMap,
}
