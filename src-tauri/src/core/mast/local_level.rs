use super::normalize::dropped_conditions;
use super::object_filters::{is_object_column, LEVEL_COLUMN};
use super::query::{ValidQuery, MAX_ROWS};
use super::types::{MastMissionId, MastRow, MastSearchPage};

pub struct LocalLevel {
    pub wide: ValidQuery,
    pub tokens: Vec<String>,
    pub text: String,
}

fn level_parts(text: &str) -> impl Iterator<Item = &str> {
    text.split(',').map(str::trim).filter(|part| !part.is_empty())
}

pub fn plan(valid: &ValidQuery) -> Option<LocalLevel> {
    if valid.mission != MastMissionId::Jwst || !valid.conditions.iter().any(|(column, _)| is_object_column(valid.mission, column)) {
        return None;
    }
    let levels: Vec<&str> = valid
        .conditions
        .iter()
        .filter(|(column, _)| column == LEVEL_COLUMN)
        .map(|(_, value)| value.as_str())
        .collect::<Option<_>>()?;
    let [level] = levels.as_slice() else {
        return None;
    };
    let shown: Vec<&str> = level_parts(level).collect();
    let text = match shown.as_slice() {
        [single] => format!("{LEVEL_COLUMN} = {single}"),
        _ => format!("{LEVEL_COLUMN} in {}", shown.join(", ")),
    };
    let wide = ValidQuery {
        conditions: valid.conditions.iter().filter(|(column, _)| column != LEVEL_COLUMN).cloned().collect(),
        offset: 0,
        limit: MAX_ROWS,
        ..valid.clone()
    };
    Some(LocalLevel { wide, tokens: shown.iter().map(|part| part.to_ascii_lowercase()).collect(), text })
}

pub fn matches(level: Option<&str>, tokens: &[String]) -> bool {
    level.is_some_and(|level| level_parts(level).any(|part| tokens.contains(&part.to_ascii_lowercase())))
}

pub fn page(wide: &MastSearchPage, valid: &ValidQuery, plan: &LocalLevel) -> Option<MastSearchPage> {
    if wide.rows.len() >= MAX_ROWS as usize {
        return None;
    }
    let kept: Vec<&MastRow> = wide.rows.iter().filter(|row| matches(row.level.as_deref(), &plan.tokens)).collect();
    let total = kept.len() as i64;
    let short_of_reported = wide.total_reliable && wide.total_reported.is_some_and(|reported| reported > wide.rows.len() as i64);
    let rows: Vec<MastRow> = kept
        .into_iter()
        .skip(usize::try_from(valid.offset).unwrap_or(usize::MAX))
        .take(usize::try_from(valid.limit).unwrap_or(0))
        .cloned()
        .collect();
    let mut dropped = dropped_conditions(&plan.wide, &wide.applied);
    dropped.retain(|column| column != LEVEL_COLUMN);
    Some(MastSearchPage {
        request_id: wide.request_id.clone(),
        mission: wide.mission,
        columns: wide.columns.clone(),
        rows,
        offset: valid.offset,
        limit: valid.limit,
        has_more: valid.offset + valid.limit < total,
        total_reported: Some(total),
        total_reliable: !short_of_reported,
        radius_arcmin: wide.radius_arcmin,
        radius_clamped: wide.radius_clamped,
        search_position: wide.search_position,
        applied: wide.applied.clone(),
        dropped_conditions: dropped,
        request_body: wide.request_body.clone(),
        messages: wide.messages.clone(),
        unresolved_targets: wide.unresolved_targets.clone(),
        elapsed_ms: wide.elapsed_ms,
        from_cache: wide.from_cache,
        level_filtered_locally: true,
        level_applied_locally: Some(plan.text.clone()),
        unfiltered_total: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::normalize::{normalize_page, PageRequest};
    use crate::core::mast::object_filters::LEVEL_COLUMN;
    use crate::core::mast::query::{build_params, validate, MAX_ROWS, PAGE_LIMIT};
    use crate::core::mast::types::{
        LevelFilter, MastFilterValues, MastMissionId, MastQuery, MastWhere, ObjectCategory, ObjectType,
    };
    use crate::infra::mast::models::{FieldMap, SearchResults};
    use serde_json::{json, Value};

    fn query(mission: MastMissionId, filters: MastFilterValues, preset: Option<&str>, offset: i64) -> ValidQuery {
        validate(&MastQuery {
            mission,
            where_: MastWhere::Position { ra_deg: 274.688, dec_deg: -13.792, radius_arcmin: 3.0, label: None },
            program: None,
            dataset_id: None,
            filters,
            preset: preset.map(str::to_owned),
            sort: Vec::new(),
            offset,
            limit: PAGE_LIMIT,
        })
        .unwrap()
    }

    fn galaxies(level: LevelFilter) -> MastFilterValues {
        MastFilterValues { level: Some(level), categories: vec![ObjectCategory::Galaxies], ..Default::default() }
    }

    fn wide_page(levels: &[Option<&str>], echo: Value, wide: &ValidQuery) -> MastSearchPage {
        reported_page(levels, levels.len() as i64, echo, wide)
    }

    fn reported_page(levels: &[Option<&str>], total: i64, echo: Value, wide: &ValidQuery) -> MastSearchPage {
        let results: Vec<Value> = levels
            .iter()
            .enumerate()
            .map(|(i, level)| json!({"fileSetName": format!("jw0{i:05}"), "productLevel": level, "targcat": "Galaxy"}))
            .collect();
        let results: SearchResults<FieldMap> = serde_json::from_value(json!({
            "results": results,
            "totalResults": total,
            "search_params": {"conditions": echo},
            "messages": []
        }))
        .unwrap();
        normalize_page(
            MastMissionId::Jwst,
            results,
            PageRequest {
                request_id: "g1:search",
                query: wide,
                request_body: serde_json::to_value(build_params(wide)).unwrap(),
                elapsed_ms: 2500,
            },
        )
    }

    const FORMS: [Option<&str>; 5] = [Some("3"), Some("2bs, 3"), Some("1b, 2a, 2b"), Some("2bs"), None];

    #[test]
    fn plans_only_for_jwst_object_searches_with_a_level() {
        let combined = query(MastMissionId::Jwst, galaxies(LevelFilter::Combined), None, 0);
        let planned = plan(&combined).expect("a plan");
        assert_eq!(planned.wide.conditions, [("targcat".to_owned(), json!("Galaxy"))]);
        assert_eq!((planned.wide.offset, planned.wide.limit), (0, MAX_ROWS));
        assert_eq!((planned.wide.target.clone(), planned.wide.radius_arcmin, planned.wide.sort.clone()), (combined.target.clone(), Some(3.0), combined.sort.clone()));
        assert_eq!(planned.tokens, ["3"]);
        assert_eq!(planned.text, "productLevel = 3");
        let body = serde_json::to_value(build_params(&planned.wide)).unwrap();
        assert_eq!(body["limit"], json!(2000));
        assert_eq!(body["offset"], json!(0));
        assert_eq!(body["conditions"], json!([{"targcat": "Galaxy"}]));
        assert_eq!(body["select_cols"].as_array().unwrap().len(), 19);
        let exposures = plan_of(MastMissionId::Jwst, galaxies(LevelFilter::Exposures), None);
        assert_eq!(exposures.tokens, ["1b", "2a", "2b", "2c"]);
        assert_eq!(exposures.text, "productLevel in 1b, 2a, 2b, 2c");
        let later = query(MastMissionId::Jwst, galaxies(LevelFilter::Combined), None, 400);
        assert_eq!(plan(&later).unwrap().wide.offset, 0);
        let typed = MastFilterValues { level: Some(LevelFilter::Combined), object_types: vec![ObjectType::Exoplanets], ..Default::default() };
        assert_eq!(plan_of(MastMissionId::Jwst, typed, None).wide.conditions, [("targdesc".to_owned(), json!("*Exoplanet*"))]);
        let ifu = plan_of(MastMissionId::Jwst, galaxies(LevelFilter::All), Some("ifu_cubes"));
        assert_eq!(ifu.wide.conditions, [("targcat".to_owned(), json!("Galaxy")), ("exp_type".to_owned(), json!("NRS_IFU,MIR_MRS"))]);
        assert_eq!(ifu.tokens, ["3"]);
        assert!(plan(&query(MastMissionId::Jwst, MastFilterValues { level: Some(LevelFilter::Combined), ..Default::default() }, None, 0)).is_none());
        assert!(plan(&query(MastMissionId::Jwst, galaxies(LevelFilter::All), None, 0)).is_none());
        assert!(plan(&query(MastMissionId::Hst, galaxies(LevelFilter::Combined), None, 0)).is_none());
        assert!(plan(&query(MastMissionId::Iue, galaxies(LevelFilter::Combined), None, 0)).is_none());
    }

    fn plan_of(mission: MastMissionId, filters: MastFilterValues, preset: Option<&str>) -> LocalLevel {
        plan(&query(mission, filters, preset, 0)).expect("a plan")
    }

    #[test]
    fn level_tokens_match_by_list_membership() {
        let l3 = vec!["3".to_owned()];
        let exposures: Vec<String> = ["1b", "2a", "2b", "2c"].map(str::to_owned).to_vec();
        assert!(matches(Some("3"), &l3));
        assert!(matches(Some("2bs, 3"), &l3));
        assert!(matches(Some(" 3 "), &l3));
        assert!(!matches(Some("2bs"), &l3));
        assert!(!matches(Some("1b, 2a, 2b"), &l3));
        assert!(!matches(None, &l3));
        assert!(matches(Some("1b, 2a, 2b"), &exposures));
        assert!(matches(Some("2B"), &exposures));
        assert!(!matches(Some("2bs"), &exposures));
        assert!(!matches(Some("2bs, 3"), &exposures));
    }

    #[test]
    fn local_level_plan_and_page() {
        let valid = query(MastMissionId::Jwst, galaxies(LevelFilter::Combined), None, 0);
        let planned = plan(&valid).unwrap();
        let wide = wide_page(&FORMS, json!([{"targcat": "Galaxy"}]), &planned.wide);
        let combined = page(&wide, &valid, &planned).expect("a complete set");
        let kept: Vec<&str> = combined.rows.iter().map(|row| row.level.as_deref().unwrap()).collect();
        assert_eq!(kept, ["3", "2bs, 3"]);
        assert_eq!((combined.total_reported, combined.total_reliable, combined.has_more), (Some(2), true, false));
        assert_eq!((combined.offset, combined.limit), (0, PAGE_LIMIT));
        assert!(combined.level_filtered_locally);
        assert_eq!(combined.level_applied_locally.as_deref(), Some("productLevel = 3"));
        assert_eq!(combined.dropped_conditions, Vec::<String>::new());
        assert_eq!(combined.applied, wide.applied);
        assert_eq!(combined.request_body, wide.request_body);
        assert_eq!(combined.request_body["limit"], json!(2000));
        assert_eq!(combined.columns, wide.columns);
        assert_eq!(combined.unfiltered_total, None);
        let exposures_query = query(MastMissionId::Jwst, galaxies(LevelFilter::Exposures), None, 0);
        let exposures_plan = plan(&exposures_query).unwrap();
        let exposures = page(&wide, &exposures_query, &exposures_plan).unwrap();
        assert_eq!(exposures.rows.len(), 1);
        assert_eq!(exposures.rows[0].level.as_deref(), Some("1b, 2a, 2b"));
        assert_eq!(exposures.level_applied_locally.as_deref(), Some("productLevel in 1b, 2a, 2b, 2c"));

        let many: Vec<Option<&str>> = (0..450).map(|i| if i % 3 == 0 { Some("1b, 2a, 2b, 2c") } else { Some("2bs, 3") }).collect();
        let wide = wide_page(&many, json!([{"targcat": "Galaxy"}]), &planned.wide);
        let first = page(&wide, &valid, &planned).unwrap();
        assert_eq!((first.rows.len(), first.total_reported, first.has_more), (200, Some(300), true));
        assert_eq!(first.rows[0].dataset_id, "jw000001");
        let second_query = query(MastMissionId::Jwst, galaxies(LevelFilter::Combined), None, 200);
        let second = page(&wide, &second_query, &planned).unwrap();
        assert_eq!((second.rows.len(), second.offset, second.has_more), (100, 200, false));
        assert_eq!(second.rows[0].dataset_id, wide.rows.iter().filter(|row| row.level.as_deref() == Some("2bs, 3")).nth(200).unwrap().dataset_id);
        let past = page(&wide, &query(MastMissionId::Jwst, galaxies(LevelFilter::Combined), None, 400), &planned).unwrap();
        assert!(past.rows.is_empty() && !past.has_more);

        let full: Vec<Option<&str>> = vec![Some("3"); MAX_ROWS as usize];
        assert!(page(&wide_page(&full, json!([{"targcat": "Galaxy"}]), &planned.wide), &valid, &planned).is_none());
        let short: Vec<Option<&str>> = vec![Some("3"); MAX_ROWS as usize - 1];
        assert_eq!(page(&wide_page(&short, json!([{"targcat": "Galaxy"}]), &planned.wide), &valid, &planned).unwrap().rows.len(), 200);

        let orion: Vec<Option<&str>> = (0..1651).map(|i| if i % 19 == 0 { Some("3") } else { Some("1b, 2a, 2b") }).collect();
        let reliability = |total: i64| {
            let local = page(&reported_page(&orion, total, json!([{"targcat": "Galaxy"}]), &planned.wide), &valid, &planned).expect("rows are kept");
            (local.rows.len(), local.total_reported, local.total_reliable, local.has_more)
        };
        assert_eq!(reliability(1674), (87, Some(87), false, false), "MAST reported more rows than it returned");
        assert_eq!(reliability(1651), (87, Some(87), true, false));
        assert_eq!(reliability(-1), (87, Some(87), true, false));

        let ignored = wide_page(&FORMS, json!([{"targcat": ""}]), &planned.wide);
        assert_eq!(page(&ignored, &valid, &planned).unwrap().dropped_conditions, ["targcat"]);
        let ifu_query = query(MastMissionId::Jwst, galaxies(LevelFilter::All), Some("ifu_cubes"), 0);
        let ifu_plan = plan(&ifu_query).unwrap();
        let ifu_wide = wide_page(&FORMS, json!([{"targcat": "Galaxy"}, {"exp_type": "NRS_IFU,MIR_MRS"}]), &ifu_plan.wide);
        let ifu = page(&ifu_wide, &ifu_query, &ifu_plan).unwrap();
        assert_eq!(ifu.dropped_conditions, Vec::<String>::new());
        assert!(!ifu.dropped_conditions.iter().any(|column| column == LEVEL_COLUMN));
        assert_eq!(ifu.rows.len(), 2);
        assert_eq!(ifu.level_applied_locally.as_deref(), Some("productLevel = 3"));
    }
}
