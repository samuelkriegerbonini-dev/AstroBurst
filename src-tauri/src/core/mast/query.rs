use serde_json::Value;

use super::errors::bad_input;
use super::ids::validate_dataset_id;
use super::missions::{verified_server_sort, HST_DATASET_CONDITION_OK, JWST_FILTER_WILDCARD_OK};
use super::types::{LevelFilter, MastCmdError, MastFilterValues, MastMissionId, MastQuery, MastWhere, SearchPosition};
use crate::infra::mast::models::SearchParams;

pub const PAGE_LIMIT: i64 = 200;
pub const MAX_ROWS: i64 = 2000;
pub const MAX_OFFSET: i64 = MAX_ROWS - PAGE_LIMIT;
pub const MIN_RADIUS_ARCMIN: f64 = 0.1;
pub const MAX_RADIUS_ARCMIN: f64 = 30.0;
pub const RADIUS_UNITS: &str = "arcminutes";
pub const NAME_MAX: usize = 200;
pub const PROGRAM_MAX: i64 = 99_999;
pub const EMPTY_QUERY: &str = "Add a target, coordinates, programme or dataset id";

const JWST_COLUMNS: [&str; 16] = [
    "fileSetName",
    "productLevel",
    "targname",
    "targ_ra",
    "targ_dec",
    "instrume",
    "exp_type",
    "filter",
    "opticalElements",
    "program",
    "observtn",
    "date_obs",
    "effexptm",
    "access",
    "publicReleaseDate",
    "s_region",
];
const HST_COLUMNS: [&str; 12] = [
    "sci_data_set_name",
    "sci_targname",
    "sci_instrume",
    "sci_aper_1234",
    "sci_spec_1234",
    "sci_actual_duration",
    "sci_start_time",
    "sci_pep_id",
    "sci_ra",
    "sci_dec",
    "sci_release_date",
    "s_region",
];
const ROMAN_COLUMNS: [&str; 13] = [
    "fileSetName",
    "dataset_category",
    "instrument_name",
    "optical_element",
    "exposure_type",
    "program",
    "observation",
    "exposure_start_time",
    "exposure_time",
    "access",
    "ra",
    "dec",
    "s_region",
];

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Position { ra_deg: f64, dec_deg: f64, label: Option<String> },
    Name(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidQuery {
    pub mission: MastMissionId,
    pub target: Option<Target>,
    pub radius_arcmin: Option<f64>,
    pub radius_clamped: bool,
    pub conditions: Vec<(String, Value)>,
    pub sort: Vec<(String, bool)>,
    pub offset: i64,
    pub limit: i64,
}

impl ValidQuery {
    pub fn wildcard(&self) -> bool {
        self.conditions.iter().any(|(_, value)| value.as_str().is_some_and(|text| text.contains('*')))
    }

    pub fn position(&self) -> Option<SearchPosition> {
        match &self.target {
            Some(Target::Position { ra_deg, dec_deg, .. }) => Some(SearchPosition { ra_deg: *ra_deg, dec_deg: *dec_deg }),
            _ => None,
        }
    }

    pub fn target_text(&self) -> Option<String> {
        match &self.target {
            Some(Target::Position { ra_deg, dec_deg, .. }) => Some(format!("{ra_deg} {dec_deg}")),
            Some(Target::Name(name)) => Some(name.clone()),
            None => None,
        }
    }
}

pub fn select_cols(mission: MastMissionId) -> Option<&'static [&'static str]> {
    match mission {
        MastMissionId::Jwst => Some(&JWST_COLUMNS),
        MastMissionId::Hst => Some(&HST_COLUMNS),
        MastMissionId::Roman => Some(&ROMAN_COLUMNS),
        _ => None,
    }
}

struct FilterColumns {
    level: Option<&'static str>,
    instrument: Option<&'static str>,
    filter: Option<&'static str>,
    filter_wildcard: bool,
    date: Option<&'static str>,
    public: Option<&'static str>,
}

fn filter_columns(mission: MastMissionId) -> FilterColumns {
    match mission {
        MastMissionId::Jwst => FilterColumns {
            level: Some("productLevel"),
            instrument: Some("instrume"),
            filter: Some(if JWST_FILTER_WILDCARD_OK { "opticalElements" } else { "filter" }),
            filter_wildcard: JWST_FILTER_WILDCARD_OK,
            date: Some("date_obs"),
            public: Some("access"),
        },
        MastMissionId::Hst => FilterColumns {
            level: None,
            instrument: Some("sci_instrume"),
            filter: Some("sci_spec_1234"),
            filter_wildcard: false,
            date: Some("sci_start_time"),
            public: None,
        },
        MastMissionId::Roman => FilterColumns {
            level: None,
            instrument: Some("instrument_name"),
            filter: Some("optical_element"),
            filter_wildcard: false,
            date: Some("exposure_start_time"),
            public: Some("access"),
        },
        _ => FilterColumns { level: None, instrument: None, filter: None, filter_wildcard: false, date: None, public: None },
    }
}

fn program_column(mission: MastMissionId) -> Option<&'static str> {
    match mission {
        MastMissionId::Jwst => Some("program"),
        MastMissionId::Hst => Some("sci_pep_id"),
        _ => None,
    }
}

fn dataset_condition_column(mission: MastMissionId) -> Option<&'static str> {
    match mission {
        MastMissionId::Jwst | MastMissionId::Roman => Some("fileSetName"),
        MastMissionId::Hst if HST_DATASET_CONDITION_OK => Some("sci_data_set_name"),
        _ => None,
    }
}

fn trimmed(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|text| !text.is_empty())
}

fn token_value(field: &str, value: &str) -> Result<String, MastCmdError> {
    let valid = value.len() <= 32 && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-'));
    if valid {
        Ok(value.to_ascii_uppercase())
    } else {
        Err(bad_input(field, "use up to 32 letters, digits or _ . + -"))
    }
}

fn date_value(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    let shape = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes.iter().enumerate().all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if !shape {
        return None;
    }
    let month: u32 = value[5..7].parse().ok()?;
    let day: u32 = value[8..10].parse().ok()?;
    ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some(value)
}

fn parse_date(text: &str) -> Result<&str, MastCmdError> {
    date_value(text).ok_or_else(|| bad_input("date", "use the form YYYY-MM-DD"))
}

fn filter_conditions(mission: MastMissionId, filters: &MastFilterValues) -> Result<Vec<(String, Value)>, MastCmdError> {
    let columns = filter_columns(mission);
    let mut conditions = Vec::new();
    if let (Some(column), Some(LevelFilter::Combined)) = (columns.level, filters.level) {
        conditions.push((column.to_owned(), Value::from("3")));
    }
    if let (Some(column), Some(text)) = (columns.instrument, trimmed(&filters.instrument)) {
        conditions.push((column.to_owned(), Value::from(token_value("instrument", text)?)));
    }
    if let (Some(column), Some(text)) = (columns.filter, trimmed(&filters.filter)) {
        let value = token_value("filter", text)?;
        let value = if columns.filter_wildcard { format!("*{value}*") } else { value };
        conditions.push((column.to_owned(), Value::from(value)));
    }
    if let Some(column) = columns.date {
        let from = trimmed(&filters.date_from);
        let to = trimmed(&filters.date_to);
        let from = from.map(parse_date).transpose()?;
        let to = to.map(parse_date).transpose()?;
        if let (Some(from), Some(to)) = (from, to) {
            if from > to {
                return Err(bad_input("date", "From must not be after To"));
            }
        }
        if let Some(from) = from {
            conditions.push((column.to_owned(), Value::from(format!(">={from}"))));
        }
        if let Some(to) = to {
            conditions.push((column.to_owned(), Value::from(format!("<={to}"))));
        }
    }
    if let (Some(column), Some(true)) = (columns.public, filters.public_only) {
        conditions.push((column.to_owned(), Value::from("PUBLIC")));
    }
    Ok(conditions)
}

pub fn validate_name(name: &str) -> Result<String, MastCmdError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(bad_input("name", "must not be empty"));
    }
    if trimmed.chars().count() > NAME_MAX {
        return Err(bad_input("name", "must be at most 200 characters"));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(bad_input("name", "must not contain control characters"));
    }
    Ok(trimmed.to_owned())
}

fn radius(requested: f64) -> Result<(f64, bool), MastCmdError> {
    if !requested.is_finite() || requested <= 0.0 {
        return Err(bad_input("radius", "Radius must be a positive number of arcminutes"));
    }
    let applied = requested.clamp(MIN_RADIUS_ARCMIN, MAX_RADIUS_ARCMIN);
    Ok((applied, applied != requested))
}

fn target(where_: &MastWhere) -> Result<Option<(Target, f64, bool)>, MastCmdError> {
    match where_ {
        MastWhere::None => Ok(None),
        MastWhere::Position { ra_deg, dec_deg, radius_arcmin, label } => {
            if !ra_deg.is_finite() || !(0.0..360.0).contains(ra_deg) {
                return Err(bad_input("ra", "RA must be at least 0 and below 360 degrees"));
            }
            if !dec_deg.is_finite() || !(-90.0..=90.0).contains(dec_deg) {
                return Err(bad_input("dec", "Dec must be between -90 and 90 degrees"));
            }
            let (applied, clamped) = radius(*radius_arcmin)?;
            let label = label.as_deref().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned);
            Ok(Some((Target::Position { ra_deg: *ra_deg, dec_deg: *dec_deg, label }, applied, clamped)))
        }
        MastWhere::Name { name, radius_arcmin } => {
            let name = validate_name(name)?;
            let (applied, clamped) = radius(*radius_arcmin)?;
            Ok(Some((Target::Name(name), applied, clamped)))
        }
    }
}

fn identity_conditions(query: &MastQuery) -> Result<Vec<(String, Value)>, MastCmdError> {
    let mut conditions = Vec::new();
    if let Some(program) = query.program {
        let Some(column) = program_column(query.mission) else {
            return Err(bad_input("program", "Programme search is available for JWST and HST"));
        };
        if !(1..=PROGRAM_MAX).contains(&program) {
            return Err(bad_input("program", "must be a number from 1 to 99999"));
        }
        conditions.push((column.to_owned(), Value::from(program)));
    }
    if let Some(raw) = query.dataset_id.as_deref() {
        let Some(column) = dataset_condition_column(query.mission) else {
            return Err(bad_input("datasetId", "Dataset search is available for JWST, HST and Roman"));
        };
        let id = raw.trim();
        validate_dataset_id(id)?;
        let id = if query.mission == MastMissionId::Hst { id.to_ascii_uppercase() } else { id.to_owned() };
        conditions.push((column.to_owned(), Value::from(id)));
    }
    Ok(conditions)
}

pub fn validate(query: &MastQuery) -> Result<ValidQuery, MastCmdError> {
    let located = target(&query.where_)?;
    let mut conditions = identity_conditions(query)?;
    if located.is_none() && conditions.is_empty() {
        return Err(bad_input("query", EMPTY_QUERY));
    }
    if query.offset < 0 || query.offset % PAGE_LIMIT != 0 || query.offset > MAX_OFFSET {
        return Err(bad_input("offset", "must be a multiple of 200 between 0 and 1800"));
    }
    if query.limit != PAGE_LIMIT {
        return Err(bad_input("limit", "must be 200"));
    }
    conditions.extend(filter_conditions(query.mission, &query.filters)?);
    let sort = query
        .sort
        .iter()
        .filter(|s| verified_server_sort(query.mission, &s.column))
        .map(|s| (s.column.clone(), s.desc))
        .collect();
    let (target, radius_arcmin, radius_clamped) = match located {
        Some((target, applied, clamped)) => (Some(target), Some(applied), clamped),
        None => (None, None, false),
    };
    Ok(ValidQuery {
        mission: query.mission,
        target,
        radius_arcmin,
        radius_clamped,
        conditions,
        sort,
        offset: query.offset,
        limit: query.limit,
    })
}

pub fn build_params(query: &ValidQuery) -> SearchParams {
    let mut params = SearchParams::new();
    if let (Some(target), Some(radius)) = (query.target_text(), query.radius_arcmin) {
        params = params.target(target).radius(radius, RADIUS_UNITS);
    }
    params = params.page(query.offset, query.limit);
    if let Some(columns) = select_cols(query.mission) {
        params = params.select(columns.iter().copied());
    }
    for (column, value) in &query.conditions {
        params = params.condition(column.clone(), value.clone());
    }
    for (column, desc) in &query.sort {
        params = params.sort(column.clone(), *desc);
    }
    params
}

pub fn canonical_key(mission: MastMissionId, params: &SearchParams) -> String {
    let body = serde_json::to_string(params).unwrap_or_default();
    format!("{}|{body}", mission.slug())
}

pub fn probe_params() -> SearchParams {
    SearchParams::new().target("0 0").radius(MIN_RADIUS_ARCMIN, RADIUS_UNITS).count_only()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::types::{MastErrorCode, MastSort};
    use serde_json::json;

    fn base(mission: MastMissionId, where_: MastWhere) -> MastQuery {
        MastQuery {
            mission,
            where_,
            program: None,
            dataset_id: None,
            filters: MastFilterValues::default(),
            sort: Vec::new(),
            offset: 0,
            limit: PAGE_LIMIT,
        }
    }

    fn at(ra: f64, dec: f64, radius: f64) -> MastWhere {
        MastWhere::Position { ra_deg: ra, dec_deg: dec, radius_arcmin: radius, label: None }
    }

    fn m16(mission: MastMissionId) -> MastQuery {
        base(mission, at(274.688, -13.792, 3.0))
    }

    fn bad(query: &MastQuery) -> String {
        let error = validate(query).unwrap_err();
        assert_eq!(error.code, MastErrorCode::BadInput, "{error:?}");
        assert_eq!(error.context, Default::default());
        error.message
    }

    fn conditions(query: &MastQuery) -> Value {
        serde_json::to_value(build_params(&validate(query).unwrap()))
            .unwrap()
            .get("conditions")
            .cloned()
            .unwrap_or(Value::Null)
    }

    #[test]
    fn query_requires_target_programme_or_dataset() {
        let empty = base(MastMissionId::Jwst, MastWhere::None);
        assert_eq!(bad(&empty), format!("query: {EMPTY_QUERY}"));
        let mut filters_only = empty.clone();
        filters_only.filters.level = Some(LevelFilter::Combined);
        filters_only.filters.instrument = Some("NIRCAM".into());
        assert_eq!(bad(&filters_only), format!("query: {EMPTY_QUERY}"));
        let blank_name = base(MastMissionId::Jwst, MastWhere::Name { name: "   ".into(), radius_arcmin: 3.0 });
        assert!(bad(&blank_name).starts_with("name: "));
        let long_name = base(MastMissionId::Jwst, MastWhere::Name { name: "m".repeat(NAME_MAX + 1), radius_arcmin: 3.0 });
        assert!(bad(&long_name).starts_with("name: "));
        let control = base(MastMissionId::Jwst, MastWhere::Name { name: "M\u{7}16".into(), radius_arcmin: 3.0 });
        assert!(bad(&control).starts_with("name: "));
        let mut programme = empty.clone();
        programme.program = Some(2739);
        let valid = validate(&programme).unwrap();
        assert_eq!(valid.target, None);
        assert_eq!(valid.radius_arcmin, None);
        let mut dataset = empty;
        dataset.dataset_id = Some("jw02739-o001_t001_nircam".into());
        assert!(validate(&dataset).is_ok());
        let name = validate(&base(MastMissionId::Hst, MastWhere::Name { name: "  M16 ".into(), radius_arcmin: 3.0 })).unwrap();
        assert_eq!(name.target, Some(Target::Name("M16".into())));
        assert_eq!(name.target_text().as_deref(), Some("M16"));
        assert_eq!(name.position(), None);
    }

    #[test]
    fn radius_is_clamped_and_flagged() {
        for (requested, applied, clamped) in [(3.0, 3.0, false), (60.0, 30.0, true), (30.0, 30.0, false), (0.05, 0.1, true), (0.1, 0.1, false)] {
            let valid = validate(&base(MastMissionId::Jwst, at(10.0, 20.0, requested))).unwrap();
            assert_eq!(valid.radius_arcmin, Some(applied), "{requested}");
            assert_eq!(valid.radius_clamped, clamped, "{requested}");
            let params = serde_json::to_value(build_params(&valid)).unwrap();
            assert_eq!(params["radius"], json!(applied));
            assert_eq!(params["radius_units"], json!("arcminutes"));
        }
        let named = validate(&base(MastMissionId::Jwst, MastWhere::Name { name: "M16".into(), radius_arcmin: 45.0 })).unwrap();
        assert_eq!((named.radius_arcmin, named.radius_clamped), (Some(30.0), true));
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(bad(&base(MastMissionId::Jwst, at(10.0, 20.0, invalid))).starts_with("radius: "), "{invalid}");
        }
    }

    #[test]
    fn ra_dec_ranges_are_validated() {
        for (ra, dec) in [(0.0, 0.0), (359.9999, 90.0), (274.688, -90.0)] {
            let valid = validate(&base(MastMissionId::Hst, at(ra, dec, 3.0))).unwrap();
            assert_eq!(valid.position(), Some(SearchPosition { ra_deg: ra, dec_deg: dec }));
        }
        for ra in [360.0, -0.001, f64::NAN, f64::INFINITY] {
            assert!(bad(&base(MastMissionId::Hst, at(ra, 0.0, 3.0))).starts_with("ra: "), "{ra}");
        }
        for dec in [90.001, -91.0, f64::NAN] {
            assert!(bad(&base(MastMissionId::Hst, at(10.0, dec, 3.0))).starts_with("dec: "), "{dec}");
        }
        let valid = validate(&m16(MastMissionId::Jwst)).unwrap();
        assert_eq!(valid.target_text().as_deref(), Some("274.688 -13.792"));
        let params = serde_json::to_value(build_params(&valid)).unwrap();
        assert_eq!(params["target"], json!(["274.688 -13.792"]));
    }

    #[test]
    fn jwst_filters_map_to_conditions() {
        let mut query = m16(MastMissionId::Jwst);
        query.filters = MastFilterValues {
            level: Some(LevelFilter::Combined),
            instrument: Some(" nircam ".into()),
            filter: Some("f444w".into()),
            date_from: Some("2022-01-01".into()),
            date_to: Some("2022-12-31".into()),
            public_only: Some(true),
        };
        let filter = if JWST_FILTER_WILDCARD_OK { json!({"opticalElements": "*F444W*"}) } else { json!({"filter": "F444W"}) };
        assert_eq!(
            conditions(&query),
            json!([
                {"productLevel": "3"},
                {"instrume": "NIRCAM"},
                filter,
                {"date_obs": ">=2022-01-01"},
                {"date_obs": "<=2022-12-31"},
                {"access": "PUBLIC"}
            ])
        );
        assert_eq!(validate(&query).unwrap().wildcard(), JWST_FILTER_WILDCARD_OK);
        let mut all_levels = m16(MastMissionId::Jwst);
        all_levels.filters.level = Some(LevelFilter::All);
        all_levels.filters.public_only = Some(false);
        all_levels.filters.instrument = Some("  ".into());
        assert_eq!(conditions(&all_levels), Value::Null);
        let mut only_from = m16(MastMissionId::Jwst);
        only_from.filters.date_from = Some("2023-01-01".into());
        assert_eq!(conditions(&only_from), json!([{"date_obs": ">=2023-01-01"}]));
        for (field, value) in [("filter", "F4*"), ("filter", "F444W,F470N"), ("instrument", "NIR CAM"), ("filter", &"F".repeat(33))] {
            let mut query = m16(MastMissionId::Jwst);
            if field == "filter" {
                query.filters.filter = Some(value.to_owned());
            } else {
                query.filters.instrument = Some(value.to_owned());
            }
            assert!(bad(&query).starts_with(&format!("{field}: ")), "{value}");
        }
        for (from, to) in [("2022-13-01", "2022-12-31"), ("2022/01/01", "2022-12-31"), ("2022-01-01", "22-12-31"), ("2023-01-01", "2022-12-31")] {
            let mut query = m16(MastMissionId::Jwst);
            query.filters.date_from = Some(from.into());
            query.filters.date_to = Some(to.into());
            assert!(bad(&query).starts_with("date: "), "{from} {to}");
        }
    }

    #[test]
    fn hst_filters_map_to_conditions() {
        let mut query = m16(MastMissionId::Hst);
        query.filters = MastFilterValues {
            level: Some(LevelFilter::Combined),
            instrument: Some("wfc3".into()),
            filter: Some("F502N".into()),
            date_from: Some("2014-01-01".into()),
            date_to: Some("2014-12-31".into()),
            public_only: Some(true),
        };
        assert_eq!(
            conditions(&query),
            json!([
                {"sci_instrume": "WFC3"},
                {"sci_spec_1234": "F502N"},
                {"sci_start_time": ">=2014-01-01"},
                {"sci_start_time": "<=2014-12-31"}
            ])
        );
        assert!(!validate(&query).unwrap().wildcard());
        let mut roman = m16(MastMissionId::Roman);
        roman.filters = MastFilterValues {
            level: Some(LevelFilter::Combined),
            instrument: Some("wfi".into()),
            filter: Some("f158".into()),
            date_from: Some("2027-01-01".into()),
            date_to: None,
            public_only: Some(true),
        };
        assert_eq!(
            conditions(&roman),
            json!([
                {"instrument_name": "WFI"},
                {"optical_element": "F158"},
                {"exposure_start_time": ">=2027-01-01"},
                {"access": "PUBLIC"}
            ])
        );
        let mut iue = m16(MastMissionId::Iue);
        iue.filters.instrument = Some("SWP".into());
        assert_eq!(conditions(&iue), Value::Null);
    }

    #[test]
    fn programme_and_dataset_conditions() {
        let mut jwst = base(MastMissionId::Jwst, MastWhere::None);
        jwst.program = Some(2739);
        jwst.filters.level = Some(LevelFilter::Combined);
        let params = serde_json::to_value(build_params(&validate(&jwst).unwrap())).unwrap();
        assert_eq!(params["conditions"], json!([{"program": 2739}, {"productLevel": "3"}]));
        assert!(params.get("target").is_none());
        assert!(params.get("radius").is_none());
        let mut hst = base(MastMissionId::Hst, MastWhere::None);
        hst.program = Some(15238);
        assert_eq!(conditions(&hst), json!([{"sci_pep_id": 15238}]));
        let mut both = m16(MastMissionId::Jwst);
        both.program = Some(2739);
        let params = serde_json::to_value(build_params(&validate(&both).unwrap())).unwrap();
        assert_eq!(params["conditions"], json!([{"program": 2739}]));
        assert_eq!(params["target"], json!(["274.688 -13.792"]));
        let mut dataset = base(MastMissionId::Jwst, MastWhere::None);
        dataset.dataset_id = Some(" jw02739-o001_t001_nircam ".into());
        assert_eq!(conditions(&dataset), json!([{"fileSetName": "jw02739-o001_t001_nircam"}]));
        let mut hst_dataset = base(MastMissionId::Hst, MastWhere::None);
        hst_dataset.dataset_id = Some("ick909030".into());
        if HST_DATASET_CONDITION_OK {
            assert_eq!(conditions(&hst_dataset), json!([{"sci_data_set_name": "ICK909030"}]));
        }
        let mut roman_dataset = base(MastMissionId::Roman, MastWhere::None);
        roman_dataset.dataset_id = Some("r0000101001001001001_0001_wfi01".into());
        assert_eq!(conditions(&roman_dataset), json!([{"fileSetName": "r0000101001001001001_0001_wfi01"}]));
        for program in [0, -5, PROGRAM_MAX + 1] {
            let mut query = base(MastMissionId::Jwst, MastWhere::None);
            query.program = Some(program);
            assert!(bad(&query).starts_with("program: "), "{program}");
        }
        let mut iue = base(MastMissionId::Iue, MastWhere::None);
        iue.program = Some(2739);
        assert!(bad(&iue).starts_with("program: "));
        let mut traversal = base(MastMissionId::Jwst, MastWhere::None);
        traversal.dataset_id = Some("../x".into());
        assert!(bad(&traversal).starts_with("datasetId: "));
        let mut classy = base(MastMissionId::Classy, MastWhere::None);
        classy.dataset_id = Some("J0021+0052".into());
        assert!(bad(&classy).starts_with("datasetId: "));
    }

    #[test]
    fn paging_limits_200_and_2000_rows() {
        for offset in [0, 200, 1800] {
            let mut query = m16(MastMissionId::Jwst);
            query.offset = offset;
            let params = serde_json::to_value(build_params(&validate(&query).unwrap())).unwrap();
            assert_eq!(params["offset"], json!(offset));
            assert_eq!(params["limit"], json!(200));
        }
        for offset in [-200, 100, 1801, 2000, 4000] {
            let mut query = m16(MastMissionId::Jwst);
            query.offset = offset;
            assert!(bad(&query).starts_with("offset: "), "{offset}");
        }
        for limit in [0, 199, 201, 5000] {
            let mut query = m16(MastMissionId::Jwst);
            query.limit = limit;
            assert!(bad(&query).starts_with("limit: "), "{limit}");
        }
        assert_eq!(MAX_OFFSET, 1800);
    }

    #[test]
    fn select_cols_are_fixed_per_mission() {
        let jwst = serde_json::to_value(build_params(&validate(&m16(MastMissionId::Jwst)).unwrap())).unwrap();
        assert_eq!(
            jwst["select_cols"],
            json!([
                "fileSetName", "productLevel", "targname", "targ_ra", "targ_dec", "instrume", "exp_type", "filter",
                "opticalElements", "program", "observtn", "date_obs", "effexptm", "access", "publicReleaseDate", "s_region"
            ])
        );
        let hst = serde_json::to_value(build_params(&validate(&m16(MastMissionId::Hst)).unwrap())).unwrap();
        assert_eq!(
            hst["select_cols"],
            json!([
                "sci_data_set_name", "sci_targname", "sci_instrume", "sci_aper_1234", "sci_spec_1234", "sci_actual_duration",
                "sci_start_time", "sci_pep_id", "sci_ra", "sci_dec", "sci_release_date", "s_region"
            ])
        );
        assert_eq!(select_cols(MastMissionId::Roman).map(<[&str]>::len), Some(13));
        for mission in [MastMissionId::Iue, MastMissionId::Classy, MastMissionId::Ullyses] {
            let params = serde_json::to_value(build_params(&validate(&m16(mission)).unwrap())).unwrap();
            assert!(params.get("select_cols").is_none(), "{mission:?}");
        }
    }

    #[test]
    fn only_verified_sorts_reach_the_server_and_keys_are_canonical() {
        let mut query = m16(MastMissionId::Jwst);
        query.sort = vec![
            MastSort { column: "date_obs".into(), desc: true },
            MastSort { column: "fileSetName".into(), desc: false },
        ];
        let valid = validate(&query).unwrap();
        let expected: Vec<(String, bool)> =
            if verified_server_sort(MastMissionId::Jwst, "date_obs") { vec![("date_obs".into(), true)] } else { Vec::new() };
        assert_eq!(valid.sort, expected);
        let a = canonical_key(MastMissionId::Jwst, &build_params(&valid));
        let b = canonical_key(MastMissionId::Jwst, &build_params(&validate(&query).unwrap()));
        assert_eq!(a, b);
        assert!(a.starts_with("jwst|"));
        let mut other = query.clone();
        other.offset = 200;
        assert_ne!(a, canonical_key(MastMissionId::Jwst, &build_params(&validate(&other).unwrap())));
        assert_ne!(a, canonical_key(MastMissionId::Hst, &build_params(&valid)));
        let probe = serde_json::to_value(probe_params()).unwrap();
        assert_eq!(probe, json!({"target": ["0 0"], "radius": 0.1, "radius_units": "arcminutes", "count_only": true}));
    }
}
