use std::collections::BTreeMap;

use serde_json::Value;

use super::missions::{column, dataset_column, has_synthetic, synthetic_columns, CATEGORY_COLUMN, PROG_OBS_COLUMN, SEP_COLUMN};
use super::object_filters::{category_text, is_object_column, object_type_column, row_categories};
use super::presets::{self, PresetSpec};
use super::query::{ValidQuery, MAX_ROWS};
use super::types::{AppliedEcho, MastColumn, MastLevelClass, MastMissionId, MastRow, MastSearchPage, SearchPosition};
use crate::infra::mast::models::{FieldMap, SearchResults};

pub struct PageRequest<'a> {
    pub request_id: &'a str,
    pub query: &'a ValidQuery,
    pub request_body: Value,
    pub elapsed_ms: u64,
}

const UNRESOLVED_PREFIX: &str = "Unable to resolve target: '";

pub fn clean_value(value: &Value) -> Value {
    match value {
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                Value::Null
            } else {
                Value::String(text.to_owned())
            }
        }
        other => other.clone(),
    }
}

fn text(raw: &FieldMap, key: &str) -> Option<String> {
    match raw.get(key)? {
        Value::String(text) => Some(text.trim()).filter(|t| !t.is_empty()).map(str::to_owned),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn number(raw: &FieldMap, key: &str) -> Option<f64> {
    let value = match raw.get(key)? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    };
    value.filter(|v| v.is_finite())
}

fn float_value(value: Option<f64>) -> Value {
    value.and_then(serde_json::Number::from_f64).map_or(Value::Null, Value::Number)
}

pub fn split_optical_elements(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for element in text.split(',') {
        let parts: Vec<&str> = element
            .split(';')
            .map(str::trim)
            .filter(|part| !part.is_empty() && !part.eq_ignore_ascii_case("CLEAR"))
            .collect();
        if parts.is_empty() {
            continue;
        }
        let joined = parts.join("-");
        if !out.contains(&joined) {
            out.push(joined);
        }
    }
    out
}

pub fn unresolved_targets(messages: &[String]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| {
            let start = message.find(UNRESOLVED_PREFIX)? + UNRESOLVED_PREFIX.len();
            let rest = &message[start..];
            let end = rest.find("'.").or_else(|| rest.rfind('\''))?;
            Some(rest[..end].to_owned())
        })
        .collect()
}

fn prog_obs(program: &Option<String>, observation: &Option<String>) -> Value {
    match (program, observation) {
        (Some(program), Some(observation)) => Value::String(format!("{program}.{observation}")),
        (Some(program), None) => Value::String(program.clone()),
        (None, _) => Value::Null,
    }
}

struct Fields {
    level: Option<String>,
    level_class: MastLevelClass,
    instrument: Option<String>,
    filter: Option<String>,
    filters: Option<Vec<String>>,
    exp_type: Option<String>,
    target: Option<String>,
    program: Option<String>,
    observation: Option<String>,
    date_obs: Option<String>,
    exp_time_s: Option<f64>,
    sep_arcmin: Option<f64>,
    sep_verified: bool,
    access: Option<String>,
    release_date: Option<String>,
    ra_deg: Option<f64>,
    dec_deg: Option<f64>,
}

impl Fields {
    fn empty() -> Self {
        Self {
            level: None,
            level_class: MastLevelClass::Unknown,
            instrument: None,
            filter: None,
            filters: None,
            exp_type: None,
            target: None,
            program: None,
            observation: None,
            date_obs: None,
            exp_time_s: None,
            sep_arcmin: None,
            sep_verified: false,
            access: None,
            release_date: None,
            ra_deg: None,
            dec_deg: None,
        }
    }
}

fn jwst_fields(raw: &FieldMap) -> Fields {
    let level = text(raw, "productLevel");
    let level_class = match level.as_deref() {
        Some("3") => MastLevelClass::Combined,
        Some(_) => MastLevelClass::Exposure,
        None => MastLevelClass::Unknown,
    };
    Fields {
        level,
        level_class,
        instrument: text(raw, "instrume"),
        filter: text(raw, "filter"),
        filters: text(raw, "opticalElements").map(|t| split_optical_elements(&t)).filter(|f| !f.is_empty()),
        exp_type: text(raw, "exp_type"),
        target: text(raw, "targname"),
        program: text(raw, "program"),
        observation: text(raw, "observtn"),
        date_obs: text(raw, "date_obs"),
        exp_time_s: number(raw, "effexptm"),
        sep_arcmin: number(raw, "ang_sep"),
        sep_verified: false,
        access: text(raw, "access"),
        release_date: text(raw, "publicReleaseDate"),
        ra_deg: number(raw, "targ_ra"),
        dec_deg: number(raw, "targ_dec"),
    }
}

fn hst_fields(raw: &FieldMap) -> Fields {
    Fields {
        instrument: text(raw, "sci_instrume"),
        filter: text(raw, "sci_spec_1234"),
        exp_type: text(raw, "sci_aper_1234"),
        target: text(raw, "sci_targname"),
        program: text(raw, "sci_pep_id"),
        date_obs: text(raw, "sci_start_time"),
        exp_time_s: number(raw, "sci_actual_duration"),
        sep_arcmin: number(raw, "ang_sep").map(|degrees| degrees * 60.0),
        sep_verified: true,
        release_date: text(raw, "sci_release_date"),
        ra_deg: number(raw, "sci_ra"),
        dec_deg: number(raw, "sci_dec"),
        ..Fields::empty()
    }
}

fn roman_fields(raw: &FieldMap) -> Fields {
    Fields {
        level: text(raw, "dataset_category"),
        instrument: text(raw, "instrument_name"),
        filter: text(raw, "optical_element"),
        exp_type: text(raw, "exposure_type"),
        program: text(raw, "program"),
        observation: text(raw, "observation"),
        date_obs: text(raw, "exposure_start_time"),
        exp_time_s: number(raw, "exposure_time"),
        access: text(raw, "access"),
        ra_deg: number(raw, "ra"),
        dec_deg: number(raw, "dec"),
        ..Fields::empty()
    }
}

fn mission_fields(mission: MastMissionId, raw: &FieldMap) -> Fields {
    match mission {
        MastMissionId::Jwst => jwst_fields(raw),
        MastMissionId::Hst => hst_fields(raw),
        MastMissionId::Roman | MastMissionId::RomanCgi => roman_fields(raw),
        MastMissionId::Iue => Fields {
            target: text(raw, "iue_target_name").or_else(|| text(raw, "iue_go_object_name")),
            program: text(raw, "iue_program_id"),
            date_obs: text(raw, "iue_obs_start_time"),
            exp_time_s: number(raw, "iue_com_exp_time"),
            ra_deg: number(raw, "iue_ra2000"),
            dec_deg: number(raw, "iue_dec2000"),
            ..Fields::empty()
        },
        MastMissionId::Classy => Fields { target: text(raw, "Target"), ..Fields::empty() },
        MastMissionId::Ullyses => Fields {
            target: text(raw, "target_name_ullyses"),
            instrument: text(raw, "instrument"),
            filter: text(raw, "filter"),
            ra_deg: number(raw, "targ_ra"),
            dec_deg: number(raw, "targ_dec"),
            ..Fields::empty()
        },
        MastMissionId::RomanSpectra | MastMissionId::Tess | MastMissionId::MultiMission => Fields::empty(),
    }
}

pub fn normalize_row(mission: MastMissionId, raw: &FieldMap, index: i64) -> MastRow {
    let fields = mission_fields(mission, raw);
    let dataset_id = dataset_column(mission)
        .and_then(|column| text(raw, column))
        .or_else(|| text(raw, "search_key"))
        .or_else(|| text(raw, "ArchiveFileID"))
        .unwrap_or_else(|| format!("row{index}"));
    let mut values: BTreeMap<String, Value> = raw.iter().map(|(key, value)| (key.clone(), clean_value(value))).collect();
    if has_synthetic(mission, PROG_OBS_COLUMN) {
        values.insert(PROG_OBS_COLUMN.to_owned(), prog_obs(&fields.program, &fields.observation));
    }
    if has_synthetic(mission, SEP_COLUMN) {
        values.insert(SEP_COLUMN.to_owned(), float_value(fields.sep_arcmin));
    }
    let categories = row_categories(mission, raw);
    if has_synthetic(mission, CATEGORY_COLUMN) {
        values.insert(CATEGORY_COLUMN.to_owned(), category_text(&categories).map_or(Value::Null, Value::String));
    }
    let filters = fields.filters.unwrap_or_else(|| fields.filter.iter().cloned().collect());
    MastRow {
        dataset_id,
        level_class: fields.level_class,
        level: fields.level,
        instrument: fields.instrument,
        filter: fields.filter,
        filters,
        exp_type: fields.exp_type,
        target: fields.target,
        program: fields.program,
        observation: fields.observation,
        date_obs: fields.date_obs,
        exp_time_s: fields.exp_time_s,
        sep_arcmin: fields.sep_arcmin,
        sep_verified: fields.sep_verified,
        access: fields.access,
        release_date: fields.release_date,
        ra_deg: fields.ra_deg,
        dec_deg: fields.dec_deg,
        s_region: text(raw, "s_region"),
        values,
        categories,
        target_description: object_type_column(mission).and_then(|column| text(raw, column)),
    }
}

pub fn columns(mission: MastMissionId, results: &SearchResults<FieldMap>) -> Vec<MastColumn> {
    let mut out: Vec<MastColumn> = Vec::new();
    for info in &results.info {
        if info.name.is_empty() || out.iter().any(|c| c.name == info.name) {
            continue;
        }
        out.push(column(mission, &info.name, info.column_type.as_deref(), info.sortable));
    }
    for synthetic in synthetic_columns(mission) {
        if !out.iter().any(|c| c.name == synthetic.name) {
            out.push(synthetic);
        }
    }
    out
}

fn echo_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn applied(results: &SearchResults<FieldMap>) -> AppliedEcho {
    let params = &results.search_params;
    AppliedEcho {
        conditions: params
            .conditions
            .iter()
            .flatten()
            .map(|condition| condition.iter().map(|(key, value)| (key.clone(), echo_text(value))).collect())
            .collect(),
        sort_by: params.sort_by.clone().unwrap_or_default(),
        sort_desc: params.sort_desc.clone().unwrap_or_default(),
    }
}

pub fn dropped_conditions(query: &ValidQuery, applied: &AppliedEcho) -> Vec<String> {
    let mut sent = query.preset.map(PresetSpec::sent).unwrap_or_default();
    sent.extend(query.conditions.iter().filter(|(column, _)| is_object_column(query.mission, column)).cloned());
    presets::dropped_conditions(&sent, applied)
}

fn parse_search_pos(text: &str) -> Option<SearchPosition> {
    let mut parts = text.split_whitespace();
    let ra_deg = parts.next()?.parse::<f64>().ok()?;
    let dec_deg = parts.next()?.parse::<f64>().ok()?;
    (parts.next().is_none() && ra_deg.is_finite() && dec_deg.is_finite()).then_some(SearchPosition { ra_deg, dec_deg })
}

pub fn normalize_page(mission: MastMissionId, results: SearchResults<FieldMap>, request: PageRequest<'_>) -> MastSearchPage {
    let query = request.query;
    let limit = usize::try_from(query.limit).unwrap_or(0);
    let rows: Vec<MastRow> = results
        .results
        .iter()
        .take(limit)
        .enumerate()
        .map(|(i, raw)| normalize_row(mission, raw, query.offset + i as i64))
        .collect();
    let total_reported = (results.total_results >= 0).then_some(results.total_results);
    let total_reliable = !query.unreliable_total() && total_reported.is_some();
    let loaded = query.offset + rows.len() as i64;
    let more_reported = match total_reported.filter(|_| total_reliable) {
        Some(total) => loaded < total,
        None => rows.len() == limit && limit > 0,
    };
    let echo = applied(&results);
    let search_position = query
        .position()
        .or_else(|| results.results.iter().find_map(|raw| text(raw, "search_pos").and_then(|t| parse_search_pos(&t))));
    MastSearchPage {
        request_id: request.request_id.to_owned(),
        mission,
        columns: columns(mission, &results),
        has_more: more_reported && loaded < MAX_ROWS,
        rows,
        offset: query.offset,
        limit: query.limit,
        total_reported,
        total_reliable,
        radius_arcmin: query.radius_arcmin,
        radius_clamped: query.radius_clamped,
        search_position,
        dropped_conditions: dropped_conditions(query, &echo),
        applied: echo,
        request_body: request.request_body,
        unresolved_targets: unresolved_targets(&results.messages),
        messages: results.messages,
        elapsed_ms: request.elapsed_ms,
        from_cache: false,
        level_applied_locally: None,
        unfiltered_total: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::missions::verified_server_sort;
    use crate::core::mast::query::{build_params, validate, Target, PAGE_LIMIT};
    use crate::core::mast::types::{MastFilterValues, MastQuery, MastWhere, ObjectCategory};
    use serde_json::json;

    fn fixture(name: &str) -> SearchResults<FieldMap> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mast").join(name);
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn position_query(mission: MastMissionId, radius: f64) -> ValidQuery {
        validate(&MastQuery {
            mission,
            where_: MastWhere::Position { ra_deg: 274.688, dec_deg: -13.792, radius_arcmin: radius, label: Some("M 16".into()) },
            program: None,
            dataset_id: None,
            filters: MastFilterValues::default(),
            preset: None,
            sort: Vec::new(),
            offset: 0,
            limit: PAGE_LIMIT,
        })
        .unwrap()
    }

    fn page_of(mission: MastMissionId, name: &str, query: &ValidQuery) -> MastSearchPage {
        normalize_page(
            mission,
            fixture(name),
            PageRequest {
                request_id: "g1:search",
                query,
                request_body: serde_json::to_value(build_params(query)).unwrap(),
                elapsed_ms: 1700,
            },
        )
    }

    #[test]
    fn no_two_page_columns_share_a_label() {
        for (mission, name) in [
            (MastMissionId::Jwst, "jwst_m16_ism.json"),
            (MastMissionId::Hst, "hst_m16_ism.json"),
            (MastMissionId::Iue, "iue_m16_stars.json"),
        ] {
            let page = page_of(mission, name, &position_query(mission, 3.0));
            let mut seen: Vec<(&str, &str)> = Vec::new();
            for column in &page.columns {
                if let Some((first, _)) = seen.iter().find(|(_, label)| *label == column.label) {
                    panic!("{mission:?}: {first} and {} are both labelled {:?}", column.name, column.label);
                }
                seen.push((column.name.as_str(), column.label.as_str()));
            }
        }
    }

    #[test]
    fn optical_elements_split_and_clear_dropped() {
        assert_eq!(
            split_optical_elements("F090W;CLEAR, F187N;CLEAR, F200W;CLEAR, F335M;CLEAR, F444W;CLEAR, F444W;F470N"),
            vec!["F090W", "F187N", "F200W", "F335M", "F444W", "F444W-F470N"]
        );
        assert_eq!(split_optical_elements("F1130W, F1500W, F770W"), vec!["F1130W", "F1500W", "F770W"]);
        assert_eq!(split_optical_elements("CLEAR;PRISM, CLEAR"), vec!["PRISM"]);
        assert_eq!(split_optical_elements(" F090W ;clear,F090W;CLEAR "), vec!["F090W"]);
        assert!(split_optical_elements("  ").is_empty());
    }

    #[test]
    fn values_are_trimmed_and_blank_strings_become_null() {
        assert_eq!(clean_value(&json!("WFC3  ")), json!("WFC3"));
        assert_eq!(clean_value(&json!("   ")), Value::Null);
        assert_eq!(clean_value(&json!("")), Value::Null);
        assert_eq!(clean_value(&json!(5797.86)), json!(5797.86));
        assert_eq!(clean_value(&json!(true)), json!(true));
        assert_eq!(clean_value(&json!(["a"])), json!(["a"]));
        assert_eq!(
            unresolved_targets(&["Unable to resolve target: 'Qzxv-Not-A-Star-77'. This target will be skipped.".to_owned(), "other".to_owned()]),
            vec!["Qzxv-Not-A-Star-77"]
        );
    }

    #[test]
    fn normalize_jwst_l3_fixture() {
        let query = position_query(MastMissionId::Jwst, 3.0);
        for name in ["jwst_m16_l3.json", "jwst_m16_l3_optical.json"] {
            let page = page_of(MastMissionId::Jwst, name, &query);
            assert_eq!(page.rows.len(), 2, "{name}");
            assert_eq!(page.request_id, "g1:search");
            assert_eq!(page.total_reported, Some(2));
            assert!(page.total_reliable);
            assert!(!page.has_more);
            assert_eq!((page.offset, page.limit), (0, 200));
            assert_eq!(page.radius_arcmin, Some(3.0));
            assert!(!page.radius_clamped);
            assert_eq!(page.search_position, Some(SearchPosition { ra_deg: 274.688, dec_deg: -13.792 }));
            assert!(!page.from_cache);
            assert_eq!(page.elapsed_ms, 1700);
            assert_eq!(page.request_body["target"], json!(["274.688 -13.792"]));
            let nircam = &page.rows[0];
            assert_eq!(nircam.dataset_id, "jw02739-o001_t001_nircam");
            assert_eq!(nircam.level_class, MastLevelClass::Combined);
            assert_eq!(nircam.level.as_deref(), Some("3"));
            assert_eq!(nircam.instrument.as_deref(), Some("NIRCAM"));
            assert_eq!(nircam.filter.as_deref(), Some("F187N"));
            assert_eq!(nircam.exp_type.as_deref(), Some("NRC_IMAGE"));
            assert_eq!(nircam.target.as_deref(), Some("M 16"));
            assert_eq!(nircam.program.as_deref(), Some("2739"));
            assert_eq!(nircam.date_obs.as_deref(), Some("2022-08-14T08:33:45.2960000"));
            assert_eq!(nircam.exp_time_s, Some(5797.86));
            assert_eq!(nircam.sep_arcmin, Some(0.44908023333333336));
            assert!(!nircam.sep_verified);
            assert_eq!(nircam.access.as_deref(), Some("PUBLIC"));
            assert!(nircam.s_region.as_deref().unwrap().starts_with("POLYGON ICRS"));
            assert_eq!(nircam.values["fileSetName"], json!("jw02739-o001_t001_nircam"));
            assert_eq!(nircam.values["program"], json!(2739));
            assert_eq!(nircam.values["includedLevels"], Value::Null);
            assert_eq!(nircam.values[SEP_COLUMN], json!(0.44908023333333336));
            assert_eq!(page.rows[1].dataset_id, "jw02739-o002_t001_miri");
            assert_eq!(page.rows[1].level_class, MastLevelClass::Combined);
            let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
            assert_eq!(names.first(), Some(&"fileSetName"));
            assert!(names.ends_with(&[PROG_OBS_COLUMN, SEP_COLUMN, CATEGORY_COLUMN]), "{names:?}");
            let file_set = &page.columns[0];
            assert_eq!(file_set.label, "Dataset");
            assert_eq!(file_set.sortable, verified_server_sort(MastMissionId::Jwst, "fileSetName"));
            let synthetic = page.columns.iter().filter(|c| c.name.starts_with('_'));
            assert!(synthetic.clone().all(|c| !c.sortable));
            assert_eq!(synthetic.count(), 3);
            assert_eq!(nircam.values[CATEGORY_COLUMN], Value::Null);
            assert!(nircam.categories.is_empty());
            assert_eq!(nircam.target_description, None);
        }
        let plain = page_of(MastMissionId::Jwst, "jwst_m16_l3.json", &query);
        assert_eq!(plain.rows[0].filters, vec!["F187N"]);
        assert_eq!(plain.rows[0].observation.as_deref(), Some("1"));
        assert_eq!(plain.rows[0].values[PROG_OBS_COLUMN], json!("2739.1"));
        assert_eq!(plain.rows[1].values[PROG_OBS_COLUMN], json!("2739.2"));
        let optical = page_of(MastMissionId::Jwst, "jwst_m16_l3_optical.json", &query);
        assert_eq!(optical.rows[0].filters, vec!["F090W", "F187N", "F200W", "F335M", "F444W", "F444W-F470N"]);
        assert_eq!(optical.rows[1].filters, vec!["F1130W", "F1500W", "F770W"]);
        assert_eq!(optical.rows[0].values[PROG_OBS_COLUMN], json!("2739"));
    }

    #[test]
    fn jwst_exposure_levels_are_exposures() {
        let mut raw = FieldMap::new();
        raw.insert("fileSetName".into(), json!("jw02739001002_02101_00001"));
        raw.insert("productLevel".into(), json!("2b"));
        assert_eq!(normalize_row(MastMissionId::Jwst, &raw, 0).level_class, MastLevelClass::Exposure);
        raw.insert("productLevel".into(), json!(" "));
        assert_eq!(normalize_row(MastMissionId::Jwst, &raw, 0).level_class, MastLevelClass::Unknown);
        let unnamed = normalize_row(MastMissionId::Tess, &FieldMap::new(), 7);
        assert_eq!(unnamed.dataset_id, "row7");
    }

    #[test]
    fn normalize_hst_fixture_trims_and_converts_sep() {
        let query = position_query(MastMissionId::Hst, 3.0);
        let wfc3 = page_of(MastMissionId::Hst, "hst_search_wfc3.json", &query);
        assert_eq!(wfc3.total_reported, Some(29));
        assert_eq!(wfc3.rows.len(), 6);
        let row = &wfc3.rows[0];
        assert_eq!(row.dataset_id, "IDNM20010");
        assert_eq!(row.level_class, MastLevelClass::Unknown);
        assert_eq!(row.level, None);
        assert_eq!(row.instrument.as_deref(), Some("WFC3"));
        assert_eq!(row.values["sci_instrume"], json!("WFC3"));
        assert_eq!(row.filter.as_deref(), Some("F160W"));
        assert_eq!(row.filters, vec!["F160W"]);
        assert_eq!(row.target.as_deref(), Some("M16EAGLENEBULA-A"));
        assert!(row.sep_verified);
        let sep = row.sep_arcmin.unwrap();
        assert!((sep - 0.0157923796374351 * 60.0).abs() < 1e-12, "{sep}");
        assert_eq!(row.values[SEP_COLUMN], json!(sep));
        assert_eq!(row.values[PROG_OBS_COLUMN], Value::Null);
        for row in &wfc3.rows {
            for value in row.values.values() {
                if let Value::String(text) = value {
                    assert_eq!(text.trim(), text);
                    assert!(!text.is_empty());
                }
            }
        }
        let m16 = page_of(MastMissionId::Hst, "hst_m16.json", &query);
        let first = &m16.rows[0];
        assert_eq!(first.dataset_id, "N9CS020E0");
        assert_eq!(first.program.as_deref(), Some("10533"));
        assert_eq!(first.values[PROG_OBS_COLUMN], json!("10533"));
        assert_eq!(first.exp_type.as_deref(), Some("NIC2-FIX"));
        assert_eq!(first.exp_time_s, Some(127.83688));
        assert_eq!(first.ra_deg, Some(274.6806591846));
        assert_eq!(first.dec_deg, Some(-13.79206426926));
        assert_eq!(first.release_date.as_deref(), Some("2007-05-18T18:56:50.210000"));
        assert_eq!(first.access, None);
        let names: Vec<&str> = m16.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names[0], "search_pos");
        let sep_column = m16.columns.iter().find(|c| c.name == SEP_COLUMN).unwrap();
        assert_eq!(sep_column.unit.as_deref(), Some("arcmin"));
        let start = m16.columns.iter().find(|c| c.name == "sci_start_time").unwrap();
        assert_eq!(start.kind, crate::core::mast::types::ColumnKind::Datetime);
        assert_eq!(start.label, "Date obs (UTC)");
        assert_eq!(start.sortable, verified_server_sort(MastMissionId::Hst, "sci_start_time"));
        assert!(m16.has_more);
    }

    #[test]
    fn applied_echo_merges_same_column_conditions() {
        let mut query = position_query(MastMissionId::Hst, 10.0);
        query.conditions = vec![
            ("sci_start_time".into(), json!(">=2014-01-01")),
            ("sci_start_time".into(), json!("<=2014-12-31")),
        ];
        let page = page_of(MastMissionId::Hst, "d.json", &query);
        assert_eq!(page.request_body["conditions"], json!([{"sci_start_time": ">=2014-01-01"}, {"sci_start_time": "<=2014-12-31"}]));
        let mut merged = BTreeMap::new();
        merged.insert("sci_start_time".to_owned(), ">=2014-01-01,<=2014-12-31".to_owned());
        assert_eq!(page.applied.conditions, vec![merged]);
        assert_eq!(page.applied.sort_by, vec!["ang_sep", "sci_targname", "sci_data_set_name"]);
        assert_eq!(page.applied.sort_desc, vec![false, false, false]);
        assert_eq!(page.total_reported, Some(85));
        assert!(page.has_more);
        let mut numbers = position_query(MastMissionId::Jwst, 3.0);
        numbers.conditions = vec![("program".into(), json!(2739))];
        let echoed = normalize_page(
            MastMissionId::Jwst,
            fixture("jwst_p2739.json"),
            PageRequest { request_id: "g2:search", query: &numbers, request_body: json!({}), elapsed_ms: 0 },
        );
        assert_eq!(echoed.applied.conditions[0]["program"], "2739");
        assert_eq!(echoed.applied.conditions[1]["productLevel"], "3");
        assert_eq!(echoed.applied.sort_desc, vec![true, false, false, false]);
    }

    #[test]
    fn a_preset_condition_missing_from_the_echo_is_named_on_the_page() {
        let mut query = position_query(MastMissionId::Jwst, 3.0);
        query.preset = Some(crate::core::mast::presets::find(MastMissionId::Jwst, "tso").unwrap());
        let dropped = page_of(MastMissionId::Jwst, "jwst_m16_l3.json", &query);
        assert_eq!(dropped.applied.conditions.len(), 1);
        assert_eq!(dropped.dropped_conditions, ["tsovisit", "exp_type"]);
        let mut honoured = fixture("jwst_m16_l3.json");
        honoured.search_params.conditions = Some(
            [("productLevel", "3"), ("tsovisit", "t"), ("exp_type", "NRS_BRIGHTOBJ,NIS_SOSS,MIR_LRS-SLITLESS,NRC_TSGRISM")]
                .iter()
                .map(|(column, value)| FieldMap::from_iter([((*column).to_owned(), json!(value))]))
                .collect(),
        );
        let page = normalize_page(
            MastMissionId::Jwst,
            honoured,
            PageRequest { request_id: "g1:search", query: &query, request_body: json!({}), elapsed_ms: 0 },
        );
        assert_eq!(page.dropped_conditions, Vec::<String>::new());
        let plain = page_of(MastMissionId::Jwst, "jwst_m16_l3.json", &position_query(MastMissionId::Jwst, 3.0));
        assert_eq!(plain.dropped_conditions, Vec::<String>::new());
        assert_eq!(serde_json::to_value(&dropped).unwrap()["droppedConditions"], json!(["tsovisit", "exp_type"]));
    }

    #[test]
    fn wildcard_marks_total_unreliable() {
        let mut query = position_query(MastMissionId::Jwst, 10.0);
        query.conditions = vec![("exp_type".into(), json!("NRC_*"))];
        assert!(query.unreliable_total());
        let page = page_of(MastMissionId::Jwst, "c_wildcard.json", &query);
        assert_eq!(page.total_reported, Some(237));
        assert_eq!(page.rows.len(), 61);
        assert!(!page.total_reliable);
        assert!(!page.has_more);
        let mut full = fixture("c_wildcard.json");
        let first = full.results[0].clone();
        full.results = vec![first; 200];
        let full_page = normalize_page(
            MastMissionId::Jwst,
            full,
            PageRequest { request_id: "g1:search", query: &query, request_body: json!({}), elapsed_ms: 0 },
        );
        assert!(full_page.has_more);
        let plain = position_query(MastMissionId::Jwst, 10.0);
        assert!(!plain.unreliable_total());
        let reliable = page_of(MastMissionId::Jwst, "c_wildcard.json", &plain);
        assert!(reliable.total_reliable);
        assert!(reliable.has_more);
        for (column, value) in [("targdesc", "*Planetary nebulae*"), ("opticalElements", "*F444W*")] {
            let mut contains = position_query(MastMissionId::Jwst, 10.0);
            contains.conditions = vec![(column.into(), json!(value))];
            let page = page_of(MastMissionId::Jwst, "c_wildcard.json", &contains);
            assert!(page.total_reliable, "{column}");
            assert_eq!(page.total_reported, Some(237));
            assert!(page.has_more, "{column}");
        }
    }

    fn object_query(mission: MastMissionId, categories: Vec<ObjectCategory>) -> ValidQuery {
        validate(&MastQuery {
            mission,
            where_: MastWhere::Position { ra_deg: 274.688, dec_deg: -13.792, radius_arcmin: 3.0, label: None },
            program: None,
            dataset_id: None,
            filters: MastFilterValues { categories, ..Default::default() },
            preset: None,
            sort: Vec::new(),
            offset: 0,
            limit: PAGE_LIMIT,
        })
        .unwrap()
    }

    #[test]
    fn rows_carry_categories() {
        let jwst = page_of(MastMissionId::Jwst, "jwst_m16_ism.json", &object_query(MastMissionId::Jwst, vec![ObjectCategory::Ism]));
        assert_eq!(jwst.rows.len(), 113);
        for row in &jwst.rows {
            assert_eq!(row.categories, [ObjectCategory::Ism], "{}", row.dataset_id);
            assert_eq!(row.target_description.as_deref(), Some("Molecular clouds; Nebulae; Protostars"));
            assert_eq!(row.values[CATEGORY_COLUMN], json!("Nebulae and ISM"));
            assert_eq!(row.values["targcat"], json!("ISM"));
        }
        let names: Vec<&str> = jwst.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"targcat") && names.contains(&"targdesc"), "{names:?}");
        assert_eq!(jwst.columns.iter().find(|c| c.name == "targdesc").unwrap().label, "Target description");
        assert_eq!(names.last(), Some(&CATEGORY_COLUMN));
        let serialised = serde_json::to_value(&jwst.rows[0]).unwrap();
        assert_eq!(serialised["categories"], json!(["ism"]));
        assert_eq!(serialised["targetDescription"], json!("Molecular clouds; Nebulae; Protostars"));
        let hst = page_of(MastMissionId::Hst, "hst_m16_ism.json", &object_query(MastMissionId::Hst, vec![ObjectCategory::Ism]));
        assert_eq!((hst.rows.len(), hst.total_reported, hst.total_reliable), (37, Some(37), true));
        assert!(hst.rows.iter().all(|row| row.categories == [ObjectCategory::Ism]));
        assert!(hst.rows.iter().all(|row| row.target_description.as_deref().is_some_and(|text| text.starts_with("ISM;HII REGION"))));
        assert_eq!(hst.rows[0].values[CATEGORY_COLUMN], json!("Nebulae and ISM"));
        let iue = page_of(MastMissionId::Iue, "iue_m16_stars.json", &object_query(MastMissionId::Iue, vec![ObjectCategory::Stars]));
        assert_eq!(iue.rows.len(), 24);
        for row in &iue.rows {
            assert_eq!(row.categories, [ObjectCategory::Stars], "{}", row.dataset_id);
            assert_eq!(row.target_description, None);
            assert_eq!(row.values[CATEGORY_COLUMN], json!("Stars"));
        }
        assert_eq!(iue.columns.last().map(|c| (c.name.as_str(), c.label.as_str())), Some((CATEGORY_COLUMN, "Category")));
        assert_eq!(iue.columns.iter().find(|c| c.name == "iue_category").unwrap().label, "Object class");
        let row = |mission: MastMissionId, column: &str, value: Value| normalize_row(mission, &FieldMap::from([(column.to_owned(), value)]), 0);
        let both = row(MastMissionId::Hst, "sci_broad_category", json!("STAR;GALAXY"));
        assert_eq!(both.categories, [ObjectCategory::Galaxies, ObjectCategory::Stars]);
        assert_eq!(both.values[CATEGORY_COLUMN], json!("Galaxies, Stars"));
        let commas = row(MastMissionId::Hst, "sci_broad_category", json!("STELLAR CLUSTER, OPEN CLUSTER"));
        assert!(commas.categories.is_empty());
        assert_eq!(commas.values[CATEGORY_COLUMN], Value::Null);
        assert_eq!(row(MastMissionId::Iue, "iue_category", json!(83)).categories, [ObjectCategory::StarClusters]);
        let unknown = row(MastMissionId::Jwst, "targcat", Value::Null);
        assert!(unknown.categories.is_empty());
        assert_eq!(unknown.values[CATEGORY_COLUMN], Value::Null);
        assert_eq!(row(MastMissionId::Jwst, "targdesc", json!("  ")).target_description, None);
        assert_eq!(row(MastMissionId::Hst, "sci_target_descrip", json!(" STAR;DA ")).target_description.as_deref(), Some("STAR;DA"));
        let roman = row(MastMissionId::Roman, "targcat", json!("Galaxy"));
        assert!(roman.categories.is_empty());
        assert!(!roman.values.contains_key(CATEGORY_COLUMN));
        assert_eq!(roman.target_description, None);
    }

    #[test]
    fn dropped_object_conditions_are_named_on_the_page() {
        let query = object_query(MastMissionId::Jwst, vec![ObjectCategory::Ism]);
        assert_eq!(page_of(MastMissionId::Jwst, "jwst_m16_ism.json", &query).dropped_conditions, Vec::<String>::new());
        let mut ignored = fixture("jwst_m16_ism.json");
        ignored.search_params.conditions = Some(vec![FieldMap::from([("targcat".to_owned(), json!(""))])]);
        let page = normalize_page(
            MastMissionId::Jwst,
            ignored,
            PageRequest { request_id: "g1:search", query: &query, request_body: json!({}), elapsed_ms: 0 },
        );
        assert_eq!(page.dropped_conditions, ["targcat"]);
        let iue = object_query(MastMissionId::Iue, vec![ObjectCategory::Stars]);
        let page = page_of(MastMissionId::Iue, "iue_m16_stars.json", &iue);
        assert_eq!(page.applied.conditions[0]["iue_category"], "10..66,!=98,!=99");
        assert_eq!(page.dropped_conditions, Vec::<String>::new());
        let hst = object_query(MastMissionId::Hst, vec![ObjectCategory::Ism]);
        assert_eq!(page_of(MastMissionId::Hst, "hst_m16_ism.json", &hst).dropped_conditions, Vec::<String>::new());
        let mut silent = fixture("hst_m16_ism.json");
        silent.search_params.conditions = Some(Vec::new());
        let page = normalize_page(
            MastMissionId::Hst,
            silent,
            PageRequest { request_id: "g1:search", query: &hst, request_body: json!({}), elapsed_ms: 0 },
        );
        assert_eq!(page.dropped_conditions, ["sci_broad_category"]);
    }

    #[test]
    fn paging_stops_at_2000_rows_and_truncates_long_pages() {
        let mut results = fixture("c_wildcard.json");
        let first = results.results[0].clone();
        results.results = vec![first; 250];
        results.total_results = 5000;
        for (offset, expected) in [(0, true), (1600, true), (1800, false)] {
            let mut query = position_query(MastMissionId::Jwst, 10.0);
            query.offset = offset;
            let page = normalize_page(
                MastMissionId::Jwst,
                results.clone(),
                PageRequest { request_id: "g1:page", query: &query, request_body: json!({}), elapsed_ms: 0 },
            );
            assert_eq!(page.rows.len(), 200, "{offset}");
            assert_eq!(page.has_more, expected, "{offset}");
            assert_eq!(page.offset, offset);
        }
        assert_eq!(MAX_ROWS, 2000);
        let mut unknown = fixture("jwst_m16_l3.json");
        unknown.total_results = -1;
        let query = position_query(MastMissionId::Jwst, 3.0);
        let page = normalize_page(
            MastMissionId::Jwst,
            unknown,
            PageRequest { request_id: "g1:search", query: &query, request_body: json!({}), elapsed_ms: 0 },
        );
        assert_eq!(page.total_reported, None);
        assert!(!page.has_more);
    }

    #[test]
    fn name_queries_report_the_server_position_and_unresolved_targets() {
        let query = validate(&MastQuery {
            mission: MastMissionId::Hst,
            where_: MastWhere::Name { name: "M16".into(), radius_arcmin: 3.0 },
            program: None,
            dataset_id: None,
            filters: MastFilterValues::default(),
            preset: None,
            sort: Vec::new(),
            offset: 0,
            limit: PAGE_LIMIT,
        })
        .unwrap();
        assert_eq!(query.target, Some(Target::Name("M16".into())));
        let found = page_of(MastMissionId::Hst, "hst_m16.json", &query);
        assert_eq!(found.search_position, Some(SearchPosition { ra_deg: 274.688, dec_deg: -13.792 }));
        let mut missing = fixture("hst_m16.json");
        missing.results.clear();
        missing.total_results = 0;
        missing.messages = vec!["Unable to resolve target: 'Qzxv-Not-A-Star-77'. This target will be skipped.".into()];
        let page = normalize_page(
            MastMissionId::Hst,
            missing,
            PageRequest { request_id: "g1:search", query: &query, request_body: json!({}), elapsed_ms: 0 },
        );
        assert_eq!(page.search_position, None);
        assert_eq!(page.unresolved_targets, vec!["Qzxv-Not-A-Star-77"]);
        assert_eq!(page.messages.len(), 1);
    }
}
