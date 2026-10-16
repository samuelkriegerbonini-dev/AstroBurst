use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub type FieldMap = BTreeMap<String, Value>;

pub(crate) fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

pub fn default_total_results() -> i64 {
    -1
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_units: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_desc: Option<Vec<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_count: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_cols: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_key: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_fields: Option<Vec<FieldMap>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Vec<FieldMap>>,
}

impl SearchParams {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target.get_or_insert_with(Vec::new).push(target.into());
        self
    }

    pub fn radius(mut self, radius: f64, units: impl Into<String>) -> Self {
        self.radius = Some(radius);
        self.radius_units = Some(units.into());
        self
    }

    pub fn page(mut self, offset: i64, limit: i64) -> Self {
        self.offset = Some(offset);
        self.limit = Some(limit);
        self
    }

    pub fn sort(mut self, column: impl Into<String>, descending: bool) -> Self {
        self.sort_by
            .get_or_insert_with(Vec::new)
            .push(column.into());
        self.sort_desc.get_or_insert_with(Vec::new).push(descending);
        self
    }

    pub fn select<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.select_cols
            .get_or_insert_with(Vec::new)
            .extend(columns.into_iter().map(Into::into));
        self
    }

    pub fn condition(mut self, column: impl Into<String>, value: impl Into<Value>) -> Self {
        let mut condition = FieldMap::new();
        condition.insert(column.into(), value.into());
        self.conditions.get_or_insert_with(Vec::new).push(condition);
        self
    }

    pub fn count_only(mut self) -> Self {
        self.count_only = Some(true);
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct InfoColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub column_type: Option<String>,
    pub sortable: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawInfoColumn {
    Name(String),
    Object {
        name: String,
        #[serde(rename = "type", default)]
        column_type: Option<Value>,
        #[serde(default)]
        sortable: Option<Value>,
    },
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::String(text) => {
            let text = text.trim();
            text.eq_ignore_ascii_case("yes") || text.eq_ignore_ascii_case("true")
        }
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        _ => false,
    }
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

impl<'de> Deserialize<'de> for InfoColumn {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match RawInfoColumn::deserialize(deserializer)? {
            RawInfoColumn::Name(name) => InfoColumn { name, column_type: None, sortable: false },
            RawInfoColumn::Object { name, column_type, sortable } => InfoColumn {
                name,
                column_type: column_type.as_ref().and_then(value_text),
                sortable: sortable.as_ref().is_some_and(truthy),
            },
        })
    }
}

pub(crate) fn lenient_messages<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<Vec<Value>>::deserialize(deserializer)?.unwrap_or_default();
    Ok(raw
        .into_iter()
        .filter_map(|item| match item {
            Value::Null => None,
            Value::String(text) => Some(text),
            Value::Object(map) => Some(
                ["message", "msg", "text"]
                    .iter()
                    .find_map(|key| map.get(*key).and_then(Value::as_str).map(str::to_owned))
                    .unwrap_or_else(|| Value::Object(map).to_string()),
            ),
            other => Some(other.to_string()),
        })
        .collect())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthCheck {
    #[serde(default)]
    pub authorized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct SearchResults<T> {
    #[serde(default, deserialize_with = "null_as_default")]
    pub info: Vec<InfoColumn>,
    #[serde(default, deserialize_with = "lenient_messages")]
    pub messages: Vec<String>,
    #[serde(default = "Vec::new", deserialize_with = "null_as_default")]
    pub results: Vec<T>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub search_params: SearchParams,
    #[serde(rename = "totalResults", default = "default_total_results")]
    pub total_results: i64,
}

impl<T> Default for SearchResults<T> {
    fn default() -> Self {
        Self {
            info: Vec::new(),
            messages: Vec::new(),
            results: Vec::new(),
            search_params: SearchParams::default(),
            total_results: default_total_results(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DatasetIds {
    #[serde(default)]
    pub dataset_ids: Vec<String>,
}

impl DatasetIds {
    pub fn new<I, S>(ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            dataset_ids: ids.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetProduct {
    pub product_key: String,
    pub access: String,
    #[serde(default)]
    pub dataset: Option<String>,
    #[serde(default)]
    pub instrument_name: Option<String>,
    #[serde(default)]
    pub filters: Option<String>,
    pub filename: String,
    pub uri: String,
    pub authz_primary_identifier: String,
    pub authz_secondary_identifier: String,
    pub file_suffix: String,
    #[serde(default)]
    pub suffix_description: String,
    pub category: String,
    pub size: i64,
    #[serde(rename = "type")]
    pub product_type: String,
    #[serde(default)]
    pub obs_collection: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub mrp: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "P: Deserialize<'de>"))]
pub struct DatasetProducts<P = DatasetProduct> {
    #[serde(default = "Vec::new", deserialize_with = "null_as_default")]
    pub products: Vec<P>,
}

impl<P> Default for DatasetProducts<P> {
    fn default() -> Self {
        Self {
            products: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageListResult {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub category: String,
    #[serde(rename = "productLevel", default, skip_serializing_if = "Option::is_none")]
    pub product_level: Option<String>,
    #[serde(rename = "vizUri", default, skip_serializing_if = "Option::is_none")]
    pub viz_uri: Option<String>,
    #[serde(rename = "scienceGroup", default, skip_serializing_if = "Option::is_none")]
    pub science_group: Option<String>,
    #[serde(flatten)]
    pub extra: FieldMap,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HttpValidationError {
    #[serde(default, deserialize_with = "null_as_default")]
    pub detail: Vec<ValidationError>,
}

impl HttpValidationError {
    pub fn summary(&self) -> String {
        self.detail
            .iter()
            .map(ValidationError::summary)
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ValidationError {
    #[serde(default)]
    pub loc: Vec<Value>,
    #[serde(default)]
    pub msg: String,
    #[serde(rename = "type", default)]
    pub error_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx: Option<Value>,
}

impl ValidationError {
    pub fn summary(&self) -> String {
        let location = self
            .loc
            .iter()
            .map(|part| match part {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(".");
        if location.is_empty() {
            self.msg.clone()
        } else {
            format!("{location}: {}", self.msg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Deserialize)]
    struct Item {
        name: Option<String>,
        #[serde(flatten)]
        extra: FieldMap,
    }

    #[test]
    fn search_params_skip_unset_fields() {
        let params = SearchParams::new()
            .target("M31")
            .radius(3.0, "arcminutes")
            .page(0, 50)
            .sort("date_obs", true)
            .condition("instrume", "NIRCAM");
        let encoded = serde_json::to_value(&params).unwrap();
        assert_eq!(
            encoded,
            json!({
                "target": ["M31"],
                "radius": 3.0,
                "radius_units": "arcminutes",
                "offset": 0,
                "limit": 50,
                "sort_by": ["date_obs"],
                "sort_desc": [true],
                "conditions": [{"instrume": "NIRCAM"}]
            })
        );
        assert_eq!(
            serde_json::to_value(SearchParams::new()).unwrap(),
            json!({})
        );
    }

    #[test]
    fn search_results_tolerate_nulls_and_missing_fields() {
        let decoded: SearchResults<Item> = serde_json::from_value(json!({
            "info": null,
            "results": [{"name": "a", "extra_col": 4}],
            "search_params": null
        }))
        .unwrap();
        assert!(decoded.info.is_empty());
        assert_eq!(decoded.total_results, -1);
        assert_eq!(decoded.results[0].name.as_deref(), Some("a"));
        assert_eq!(decoded.results[0].extra["extra_col"], json!(4));
        assert_eq!(decoded.search_params, SearchParams::default());
    }

    #[test]
    fn dataset_product_uses_spec_field_names() {
        let decoded: DatasetProducts = serde_json::from_value(json!({
            "products": [{
                "product_key": "jw01_x_cal.fits",
                "access": "PUBLIC",
                "dataset": "jw01",
                "instrument_name": null,
                "filename": "jw01_x_cal.fits",
                "uri": "mast:JWST/product/jw01_x_cal.fits",
                "authz_primary_identifier": "jw01",
                "authz_secondary_identifier": "JWST",
                "file_suffix": "_cal",
                "category": "2b",
                "size": 1024,
                "type": "science"
            }]
        }))
        .unwrap();
        let product = &decoded.products[0];
        assert_eq!(product.product_type, "science");
        assert_eq!(product.size, 1024);
        assert!(product.instrument_name.is_none());
        assert!(!product.mrp);
    }

    #[test]
    fn info_columns_decode_both_conventions() {
        let decoded: SearchResults<FieldMap> = serde_json::from_value(json!({
            "info": [
                {"name": "fileSetName", "type": "char", "sortable": "yes"},
                {"name": "access", "type": "char", "sortable": "no"},
                {"name": "search_pos", "type": "VARCHAR(255)", "sortable": true},
                {"name": "ang_sep", "type": "NULL", "sortable": false},
                {"name": "bare"},
                "legacy_column"
            ],
            "messages": ["plain", {"message": "from object"}, {"code": 3}],
            "results": []
        }))
        .unwrap();
        let columns: Vec<(&str, Option<&str>, bool)> = decoded
            .info
            .iter()
            .map(|c| (c.name.as_str(), c.column_type.as_deref(), c.sortable))
            .collect();
        assert_eq!(
            columns,
            vec![
                ("fileSetName", Some("char"), true),
                ("access", Some("char"), false),
                ("search_pos", Some("VARCHAR(255)"), true),
                ("ang_sep", Some("NULL"), false),
                ("bare", None, false),
                ("legacy_column", None, false),
            ]
        );
        assert_eq!(decoded.messages, vec!["plain", "from object", "{\"code\":3}"]);
    }

    #[test]
    fn live_fixtures_decode_with_object_info() {
        for name in ["jwst_m16_l3.json", "hst_m16.json", "d.json", "c_wildcard.json"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/mast")
                .join(name);
            let bytes = std::fs::read(&path).unwrap();
            let decoded: SearchResults<FieldMap> = serde_json::from_slice(&bytes)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!decoded.info.is_empty(), "{name}");
            assert!(decoded.total_results >= 0, "{name}");
        }
    }

    #[test]
    fn auth_check_keeps_authorized_flag() {
        let denied: AuthCheck = serde_json::from_str(r#"{"authorized":false}"#).unwrap();
        let granted: AuthCheck = serde_json::from_str(r#"{"authorized":true,"extra":1}"#).unwrap();
        assert!(!denied.authorized);
        assert!(granted.authorized);
    }

    #[test]
    fn image_list_entries_type_undocumented_keys() {
        let entry: ImageListResult = serde_json::from_value(json!({
            "url": "/get_image_thumbnail?fileset_name=ICK909030&filename=x_thumb.jpg",
            "filename": "x_thumb.jpg",
            "category": "THUMBNAIL",
            "productLevel": "None",
            "vizUri": "mast:hst/products/x.fits",
            "scienceGroup": "HapScience",
            "other": 1
        }))
        .unwrap();
        assert_eq!(entry.product_level.as_deref(), Some("None"));
        assert_eq!(entry.viz_uri.as_deref(), Some("mast:hst/products/x.fits"));
        assert_eq!(entry.science_group.as_deref(), Some("HapScience"));
        assert_eq!(entry.extra.len(), 1);
    }

    #[test]
    fn validation_error_summary_joins_locations() {
        let error: HttpValidationError = serde_json::from_value(json!({
            "detail": [
                {"loc": ["query", "fileset_name"], "msg": "Field required", "type": "missing"},
                {"loc": ["body", 0], "msg": "bad", "type": "value_error"}
            ]
        }))
        .unwrap();
        assert_eq!(
            error.summary(),
            "query.fileset_name: Field required; body.0: bad"
        );
    }
}
