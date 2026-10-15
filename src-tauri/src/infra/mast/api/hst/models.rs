use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::infra::mast::models::{FieldMap, SearchResults};

pub type HstSearchResults = SearchResults<HstSearchResultItem>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HstSearchResultItem {
    pub search_pos: Option<String>,
    pub sci_data_set_name: Option<String>,
    pub sci_archive_class: Option<String>,
    pub sci_program_id: Option<String>,
    pub sci_obset_id: Option<String>,
    pub sci_obsnum: Option<String>,
    pub sci_asn_id: Option<String>,
    pub sci_targname: Option<String>,
    pub sci_target_descrip: Option<String>,
    pub sci_broad_category: Option<String>,
    pub sci_ra: Option<f64>,
    pub sci_dec: Option<f64>,
    pub sci_start_time: Option<String>,
    pub sci_stop_time: Option<String>,
    pub sci_actual_duration: Option<f64>,
    pub sci_instrume: Option<String>,
    pub sci_instrument_config: Option<String>,
    pub sci_operating_mode: Option<String>,
    pub sci_aper_1234: Option<String>,
    pub sci_spec_1234: Option<String>,
    pub sci_central_wavelength: Option<f64>,
    pub sci_pep_id: Option<i64>,
    pub sci_pi_last_name: Option<String>,
    pub sci_release_date: Option<String>,
    pub sci_preview_name: Option<String>,
    pub sci_obs_type: Option<String>,
    pub sci_status: Option<String>,
    pub ang_sep: Option<f64>,
    pub search_key: Option<String>,
    #[serde(flatten)]
    pub extra: FieldMap,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClassicSearchParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolver: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ra: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equinox: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spectrum: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sci_data_set_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sci_program_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sci_targname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sci_instrume: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sci_aper_1234: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sci_spec_1234: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sci_start_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sci_release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sci_pi_last_name: Option<String>,
    #[serde(
        rename = "selectedColumnsCsv",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub selected_columns_csv: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordercolumn1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descending1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordformat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputformat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_records: Option<i64>,
    #[serde(flatten)]
    pub extra: FieldMap,
}

impl ClassicSearchParams {
    pub fn to_query_pairs(&self) -> Vec<(String, String)> {
        let Ok(Value::Object(fields)) = serde_json::to_value(self) else {
            return Vec::new();
        };
        let mut pairs = Vec::new();
        for (key, value) in fields {
            match value {
                Value::Null => {}
                Value::Array(items) => {
                    let array_key = format!("{key}[]");
                    pairs.extend(
                        items
                            .iter()
                            .filter_map(scalar_to_string)
                            .map(|item| (array_key.clone(), item)),
                    );
                }
                scalar => {
                    if let Some(text) = scalar_to_string(&scalar) {
                        pairs.push((key, text));
                    }
                }
            }
        }
        pairs
    }
}

fn scalar_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null | Value::Array(_) | Value::Object(_) => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}
