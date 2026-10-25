use serde::Serialize;
use serde_json::Value;

use super::errors::bad_input;
use super::missions::label as mission_label;
use super::types::{
    FilterGroupId, LevelFilter, MastCmdError, MastFilterGroup, MastFilterOption, MastMissionId, ObjectCategory, ObjectType,
};
use crate::infra::mast::models::FieldMap;

pub const LEVEL_COLUMN: &str = "productLevel";
const COMBINED_LEVEL: &str = "3";
const EXPOSURE_LEVELS: &str = "1b, 2a, 2b, 2c";
const CATEGORY_UNAVAILABLE: &str = "Category is available for JWST, HST and IUE";
const OBJECT_TYPE_UNAVAILABLE: &str = "Object type is available for JWST and HST";

struct CategoryRow {
    id: ObjectCategory,
    label: &'static str,
    jwst: &'static str,
    hst: &'static [&'static str],
    iue: &'static [(u32, u32)],
}

struct ObjectTypeRow {
    id: ObjectType,
    label: &'static str,
    jwst: &'static [&'static str],
    hst: &'static [&'static str],
}

static CATEGORIES: [CategoryRow; 8] = [
    CategoryRow { id: ObjectCategory::Galaxies, label: "Galaxies", jwst: "Galaxy", hst: &["GALAXY"], iue: &[(80, 82), (84, 88)] },
    CategoryRow {
        id: ObjectCategory::GalaxyClusters,
        label: "Galaxy clusters and groups",
        jwst: "Clusters of Galaxies",
        hst: &["CLUSTER OF GALAXIES"],
        iue: &[],
    },
    CategoryRow { id: ObjectCategory::Ism, label: "Nebulae and ISM", jwst: "ISM", hst: &["ISM", "EXT-MEDIUM"], iue: &[(69, 76)] },
    CategoryRow {
        id: ObjectCategory::StarClusters,
        label: "Star clusters",
        jwst: "Stellar Cluster",
        hst: &["STELLAR CLUSTER", "EXT-CLUSTER"],
        iue: &[(83, 83)],
    },
    CategoryRow { id: ObjectCategory::Stars, label: "Stars", jwst: "Star", hst: &["STAR", "EXT-STAR"], iue: &[(10, 66)] },
    CategoryRow { id: ObjectCategory::SolarSystem, label: "Solar System", jwst: "Solar System", hst: &["SOLAR SYSTEM"], iue: &[(0, 9)] },
    CategoryRow { id: ObjectCategory::Fields, label: "Deep and blank fields", jwst: "Unidentified", hst: &["UNIDENTIFIED"], iue: &[] },
    CategoryRow { id: ObjectCategory::Calibration, label: "Calibration targets", jwst: "Calibration", hst: &["CALIBRATION"], iue: &[] },
];

static OBJECT_TYPES: [ObjectTypeRow; 8] = [
    ObjectTypeRow {
        id: ObjectType::PlanetaryNebulae,
        label: "Planetary nebulae",
        jwst: &["Planetary nebulae"],
        hst: &["PLANETARY NEBULA"],
    },
    ObjectTypeRow {
        id: ObjectType::NebulaeStarForming,
        label: "Nebulae and star-forming regions",
        jwst: &[
            "nebula",
            "H II regions",
            "Supernova remnants",
            "Herbig-Haro objects",
            "Photodissociation regions",
            "Interstellar clouds",
            "Molecular clouds",
            "Molecular gas",
            "Protoclusters",
        ],
        hst: &["NEBULA", "HII REGION", "SNR", "HERBIG-HARO", "DARK CLOUD", "STAR FORMING REGION", "MOLECULAR CLOUD"],
    },
    ObjectTypeRow {
        id: ObjectType::YoungStarsDisks,
        label: "Young stars and disks",
        jwst: &[
            "Young stellar objects",
            "Protostars",
            "Protoplanetary disks",
            "T Tauri stars",
            "Pre-main sequence stars",
            "Debris disks",
            "Circumstellar disks",
            "Proplyds",
        ],
        hst: &["T TAURI", "PROTOPLANETARY DISK", "PRE-MAIN SEQUENCE", "YSO", "HERBIG AE"],
    },
    ObjectTypeRow { id: ObjectType::Exoplanets, label: "Exoplanets", jwst: &["Exoplanet"], hst: &["EXTRA-SOLAR PLANET"] },
    ObjectTypeRow {
        id: ObjectType::AgnQuasars,
        label: "Active galaxies, quasars and black holes",
        jwst: &[
            "Active galactic nuclei",
            "Active galaxies",
            "Quasars",
            "Seyfert galaxies",
            "Radio galaxies",
            "Quasar-galaxy pairs",
            "Black holes",
        ],
        hst: &["SEYFERT", "QSO", "QUASAR", "RADIO GALAXY", "BLR", "NLR", "BL LAC", "LINER", "BLACK HOLE"],
    },
    ObjectTypeRow {
        id: ObjectType::InteractingGalaxies,
        label: "Interacting galaxies",
        jwst: &["Interacting galaxies", "Galaxy mergers"],
        hst: &["INTERACTING GALAX"],
    },
    ObjectTypeRow {
        id: ObjectType::SupernovaeTransients,
        label: "Supernovae and transients",
        jwst: &["Supernovae", "Gamma Ray bursters"],
        hst: &["SUPERNOVA", "GAMMA RAY BURST"],
    },
    ObjectTypeRow {
        id: ObjectType::CompactSubstellar,
        label: "White dwarfs and brown dwarfs",
        jwst: &["White dwarfs", "Brown dwarfs", "T dwarfs", "Y dwarfs", "L dwarfs"],
        hst: &["BROWN DWARF", "WHITE DWARF", "STAR;DA", "STAR;DB", "STAR;DC", "STAR;DO", "STAR;DZ", "EXT-STAR;DA"],
    },
];

pub fn level_value(level: LevelFilter) -> Option<&'static str> {
    match level {
        LevelFilter::Combined => Some(COMBINED_LEVEL),
        LevelFilter::Exposures => Some(EXPOSURE_LEVELS),
        LevelFilter::All => None,
    }
}

pub fn category_column(mission: MastMissionId) -> Option<&'static str> {
    match mission {
        MastMissionId::Jwst => Some("targcat"),
        MastMissionId::Hst => Some("sci_broad_category"),
        MastMissionId::Iue => Some("iue_category"),
        _ => None,
    }
}

pub fn object_type_column(mission: MastMissionId) -> Option<&'static str> {
    match mission {
        MastMissionId::Jwst => Some("targdesc"),
        MastMissionId::Hst => Some("sci_target_descrip"),
        _ => None,
    }
}

pub fn is_object_column(mission: MastMissionId, column: &str) -> bool {
    category_column(mission) == Some(column) || object_type_column(mission) == Some(column)
}

fn range_text(&(low, high): &(u32, u32)) -> String {
    if low == high {
        low.to_string()
    } else {
        format!("{low}..{high}")
    }
}

fn category_parts(mission: MastMissionId, row: &CategoryRow) -> Vec<String> {
    match mission {
        MastMissionId::Jwst => vec![row.jwst.to_owned()],
        MastMissionId::Hst => row
            .hst
            .iter()
            .flat_map(|token| [(*token).to_owned(), format!("{token};*"), format!("*;{token}"), format!("*;{token};*")])
            .collect(),
        MastMissionId::Iue => row.iue.iter().map(range_text).collect(),
        _ => Vec::new(),
    }
}

fn object_type_phrases(mission: MastMissionId, row: &ObjectTypeRow) -> &'static [&'static str] {
    match mission {
        MastMissionId::Jwst => row.jwst,
        MastMissionId::Hst => row.hst,
        _ => &[],
    }
}

fn object_type_parts(mission: MastMissionId, row: &ObjectTypeRow) -> Vec<String> {
    let phrases = object_type_phrases(mission, row).iter();
    match mission {
        MastMissionId::Jwst => phrases.map(|phrase| format!("*{phrase}*")).collect(),
        MastMissionId::Hst => phrases.map(|phrase| (*phrase).to_owned()).collect(),
        _ => Vec::new(),
    }
}

fn condition(
    field: &str,
    column: Option<&str>,
    unavailable: &str,
    mission: MastMissionId,
    selected: Vec<(&str, Vec<String>)>,
) -> Result<Option<(String, Value)>, MastCmdError> {
    if selected.is_empty() {
        return Ok(None);
    }
    let column = column.ok_or_else(|| bad_input(field, unavailable))?;
    let mut parts: Vec<String> = Vec::new();
    for (label, offered) in selected {
        if offered.is_empty() {
            return Err(bad_input(field, &format!("{label} is not available for {}", mission_label(mission))));
        }
        for part in offered {
            if !parts.contains(&part) {
                parts.push(part);
            }
        }
    }
    Ok(Some((column.to_owned(), Value::from(parts.join(", ")))))
}

pub fn category_condition(mission: MastMissionId, ids: &[ObjectCategory]) -> Result<Option<(String, Value)>, MastCmdError> {
    let selected = CATEGORIES.iter().filter(|row| ids.contains(&row.id)).map(|row| (row.label, category_parts(mission, row))).collect();
    condition("categories", category_column(mission), CATEGORY_UNAVAILABLE, mission, selected)
}

pub fn object_type_condition(mission: MastMissionId, ids: &[ObjectType]) -> Result<Option<(String, Value)>, MastCmdError> {
    let selected = OBJECT_TYPES.iter().filter(|row| ids.contains(&row.id)).map(|row| (row.label, object_type_parts(mission, row))).collect();
    condition("objectTypes", object_type_column(mission), OBJECT_TYPE_UNAVAILABLE, mission, selected)
}

fn wire_id(id: impl Serialize) -> String {
    serde_json::to_value(id).ok().and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default()
}

fn or_list(items: &[&str]) -> String {
    match items.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        Some((last, _)) => (*last).to_owned(),
        None => String::new(),
    }
}

fn category_title(mission: MastMissionId, column: &str, row: &CategoryRow) -> String {
    match mission {
        MastMissionId::Jwst => format!("{column} = {}", row.jwst),
        MastMissionId::Hst => format!("{column} has {}", or_list(row.hst)),
        MastMissionId::Iue => format!("{column} {}", category_parts(mission, row).join(", ")),
        _ => String::new(),
    }
}

fn category_info(mission: MastMissionId) -> &'static str {
    match mission {
        MastMissionId::Jwst => {
            "From the proposal: the target category chosen in APT (TARGCAT). Proposers choose it, so a field can be filed under another class (Orion programme 1256 is a star cluster). Datasets with no category never match."
        }
        MastMissionId::Hst => "From the proposal: SCI_BROAD_CATEGORY. Datasets with no category never match.",
        MastMissionId::Iue => "IUE object class (IUE_CATEGORY).",
        _ => "",
    }
}

fn object_type_info(mission: MastMissionId) -> &'static str {
    match mission {
        MastMissionId::Jwst => {
            "From the proposal's target description keywords (TARGDESC). Each type groups several keywords. Datasets with no description never match."
        }
        MastMissionId::Hst => "From the proposal's target description (SCI_TARGET_DESCRIP). Each type groups several terms.",
        _ => "",
    }
}

pub fn filter_groups(mission: MastMissionId) -> Vec<MastFilterGroup> {
    let categories = category_column(mission).map(|column| MastFilterGroup {
        id: FilterGroupId::Category,
        label: "Category".to_owned(),
        column: column.to_owned(),
        info: category_info(mission).to_owned(),
        options: CATEGORIES
            .iter()
            .filter(|row| !category_parts(mission, row).is_empty())
            .map(|row| MastFilterOption { id: wire_id(row.id), label: row.label.to_owned(), title: category_title(mission, column, row) })
            .collect(),
    });
    let object_types = object_type_column(mission).map(|column| MastFilterGroup {
        id: FilterGroupId::ObjectType,
        label: "Object type".to_owned(),
        column: column.to_owned(),
        info: object_type_info(mission).to_owned(),
        options: OBJECT_TYPES
            .iter()
            .filter(|row| !object_type_phrases(mission, row).is_empty())
            .map(|row| MastFilterOption {
                id: wire_id(row.id),
                label: row.label.to_owned(),
                title: format!("{column} contains {}", or_list(object_type_phrases(mission, row))),
            })
            .collect(),
    });
    categories.into_iter().chain(object_types).collect()
}

fn raw_text<'a>(raw: &'a FieldMap, column: &str) -> Option<&'a str> {
    raw.get(column)?.as_str().map(str::trim).filter(|text| !text.is_empty())
}

fn raw_class(raw: &FieldMap, column: &str) -> Option<u32> {
    match raw.get(column)? {
        Value::Number(number) => number.as_u64().and_then(|class| u32::try_from(class).ok()),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

pub fn row_categories(mission: MastMissionId, raw: &FieldMap) -> Vec<ObjectCategory> {
    let Some(column) = category_column(mission) else {
        return Vec::new();
    };
    let text = raw_text(raw, column);
    let tokens: Vec<&str> = text.map(|text| text.split(';').map(str::trim).collect()).unwrap_or_default();
    let class = raw_class(raw, column);
    CATEGORIES
        .iter()
        .filter(|row| match mission {
            MastMissionId::Jwst => text.is_some_and(|value| value.eq_ignore_ascii_case(row.jwst)),
            MastMissionId::Hst => row.hst.iter().any(|known| tokens.iter().any(|token| token.eq_ignore_ascii_case(known))),
            MastMissionId::Iue => class.is_some_and(|class| row.iue.iter().any(|&(low, high)| (low..=high).contains(&class))),
            _ => false,
        })
        .map(|row| row.id)
        .collect()
}

pub fn category_text(ids: &[ObjectCategory]) -> Option<String> {
    let labels: Vec<&str> = CATEGORIES.iter().filter(|row| ids.contains(&row.id)).map(|row| row.label).collect();
    (!labels.is_empty()).then(|| labels.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::errors::OpKind;
    use crate::infra::mast::hub::MastHub;
    use crate::infra::mast::models::SearchParams;
    use crate::infra::mast::settings::MastSettings;
    use serde_json::json;
    use std::collections::BTreeSet;

    const OFFERING: [MastMissionId; 3] = [MastMissionId::Jwst, MastMissionId::Hst, MastMissionId::Iue];

    fn sent(condition: Result<Option<(String, Value)>, MastCmdError>) -> (String, String) {
        let (column, value) = condition.unwrap().expect("a condition");
        (column, value.as_str().expect("a text value").to_owned())
    }

    fn split(text: &str) -> Vec<String> {
        text.split(',').map(|part| part.trim().to_owned()).collect()
    }

    fn all_categories() -> Vec<ObjectCategory> {
        CATEGORIES.iter().map(|row| row.id).collect()
    }

    fn all_types() -> Vec<ObjectType> {
        OBJECT_TYPES.iter().map(|row| row.id).collect()
    }

    fn raw(column: &str, value: Value) -> FieldMap {
        FieldMap::from([(column.to_owned(), value)])
    }

    #[test]
    fn hst_categories_never_send_a_bare_token() {
        for row in &CATEGORIES {
            let (column, text) = sent(category_condition(MastMissionId::Hst, &[row.id]));
            assert_eq!(column, "sci_broad_category");
            let parts = split(&text);
            assert_eq!(parts.len(), 4 * row.hst.len(), "{text}");
            for (token, group) in row.hst.iter().zip(parts.chunks(4)) {
                assert_eq!(group, [(*token).to_owned(), format!("{token};*"), format!("*;{token}"), format!("*;{token};*")], "{text}");
                assert!(!parts.contains(&format!("*{token}*")), "{text}");
            }
        }
        assert_eq!(
            sent(category_condition(MastMissionId::Hst, &[ObjectCategory::Stars])).1,
            "STAR, STAR;*, *;STAR, *;STAR;*, EXT-STAR, EXT-STAR;*, *;EXT-STAR, *;EXT-STAR;*"
        );
    }

    #[test]
    fn object_types_union_phrases_in_table_order() {
        let (column, jwst) = sent(object_type_condition(MastMissionId::Jwst, &all_types()));
        assert_eq!(column, "targdesc");
        let expected: Vec<String> = OBJECT_TYPES.iter().flat_map(|row| row.jwst.iter().map(|phrase| format!("*{phrase}*"))).collect();
        assert_eq!(split(&jwst), expected);
        assert_eq!(expected.len(), 35);
        let mut reversed = all_types();
        reversed.reverse();
        assert_eq!(sent(object_type_condition(MastMissionId::Jwst, &reversed)).1, jwst);
        let (column, hst) = sent(object_type_condition(MastMissionId::Hst, &reversed));
        assert_eq!(column, "sci_target_descrip");
        let bare: Vec<String> = OBJECT_TYPES.iter().flat_map(|row| row.hst.iter().map(|phrase| (*phrase).to_owned())).collect();
        assert_eq!(split(&hst), bare);
        assert_eq!(bare.len(), 34);
        for text in [&jwst, &hst] {
            let parts = split(text);
            assert_eq!(parts.iter().collect::<BTreeSet<_>>().len(), parts.len(), "{text}");
        }
        let twice = [ObjectType::Exoplanets, ObjectType::PlanetaryNebulae, ObjectType::Exoplanets];
        assert_eq!(sent(object_type_condition(MastMissionId::Jwst, &twice)).1, "*Planetary nebulae*, *Exoplanet*");
        assert_eq!(sent(object_type_condition(MastMissionId::Hst, &twice)).1, "PLANETARY NEBULA, EXTRA-SOLAR PLANET");
        let nebulae = [ObjectType::NebulaeStarForming];
        assert_eq!(split(&sent(object_type_condition(MastMissionId::Jwst, &nebulae)).1).len(), 9);
        assert_eq!(split(&sent(object_type_condition(MastMissionId::Hst, &nebulae)).1).len(), 7);
        assert_eq!(object_type_condition(MastMissionId::Jwst, &[]).unwrap(), None);
        assert_eq!(category_condition(MastMissionId::Roman, &[]).unwrap(), None);
    }

    fn safe(mission: MastMissionId, column: &str, part: &str) -> Result<(), String> {
        let fail = |why: &str| Err(format!("{mission:?} {column} {part:?}: {why}"));
        if part.is_empty() || part != part.trim() {
            return fail("blank or padded");
        }
        if part.chars().any(|c| c.is_control() || matches!(c, ',' | '%' | '_' | '[' | ']')) {
            return fail("a LIKE or list character");
        }
        if part.starts_with(['!', '<', '>', '=']) {
            return fail("a leading operator");
        }
        let range = |text: &str| !text.is_empty() && text.chars().all(|c| c.is_ascii_digit());
        if column == "iue_category" {
            let ok = match part.split_once("..") {
                Some((low, high)) => range(low) && range(high),
                None => range(part),
            };
            return if ok { Ok(()) } else { fail("not an IUE range") };
        }
        if part.contains("..") {
            return fail("a range outside IUE");
        }
        let starred = part.matches('*').count();
        let allowed = match column {
            "targdesc" => starred == 2 && part.starts_with('*') && part.ends_with('*'),
            "sci_broad_category" => {
                starred == 0 || (starred == 1 && (part.ends_with(";*") || part.starts_with("*;"))) || (starred == 2 && part.starts_with("*;") && part.ends_with(";*"))
            }
            _ => starred == 0,
        };
        if allowed {
            Ok(())
        } else {
            fail("a wildcard outside the allowed forms")
        }
    }

    #[test]
    fn table_values_are_safe() {
        for row in &CATEGORIES {
            assert!(!row.jwst.contains([',', '*']), "{}", row.jwst);
            assert!(row.hst.iter().all(|token| !token.contains([',', '*', ';'])), "{:?}", row.hst);
            assert!(row.iue.iter().all(|(low, high)| low <= high && *high < 98), "{:?}", row.iue);
        }
        for row in &OBJECT_TYPES {
            assert!(row.jwst.iter().chain(row.hst).all(|phrase| !phrase.contains([',', '*'])), "{}", row.label);
        }
        for mission in OFFERING {
            for row in &CATEGORIES {
                match category_condition(mission, &[row.id]) {
                    Ok(_) => {
                        let (column, text) = sent(category_condition(mission, &[row.id]));
                        for part in split(&text) {
                            safe(mission, &column, &part).unwrap();
                        }
                    }
                    Err(_) => assert!(mission == MastMissionId::Iue && row.iue.is_empty(), "{mission:?} {}", row.label),
                }
            }
        }
        for mission in [MastMissionId::Jwst, MastMissionId::Hst] {
            let (column, text) = sent(object_type_condition(mission, &all_types()));
            for part in split(&text) {
                safe(mission, &column, &part).unwrap();
            }
        }
        assert!(safe(MastMissionId::Jwst, "targcat", "Gal_xy").is_err());
        assert!(safe(MastMissionId::Jwst, "targcat", "!Galaxy").is_err());
        assert!(safe(MastMissionId::Jwst, "targcat", "*Galaxy*").is_err());
        assert!(safe(MastMissionId::Iue, "iue_category", "80..").is_err());
        assert!(safe(MastMissionId::Hst, "sci_broad_category", "*STAR*").is_err());
        assert!(safe(MastMissionId::Jwst, "targdesc", "1..2").is_err());
    }

    fn column_list(mission: &str) -> Vec<Value> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/mast/column_list_{mission}.json"));
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn filter_columns_exist_in_column_list() {
        for (mission, slug, qual_types) in [
            (MastMissionId::Jwst, "jwst", [("targcat", "string"), ("targdesc", "string")]),
            (MastMissionId::Hst, "hst", [("sci_broad_category", "string"), ("sci_target_descrip", "substring")]),
        ] {
            let listed = column_list(slug);
            for (name, qual_type) in qual_types {
                let entry = listed.iter().find(|entry| entry["column_name"] == name).unwrap_or_else(|| panic!("{slug} lacks {name}"));
                assert_eq!(entry["queryable"], "yes", "{name}");
                assert_eq!(entry["qual_type"], qual_type, "{name}");
                assert!(crate::core::mast::query::select_cols(mission).unwrap().contains(&name), "{name} is not selected");
            }
            assert_eq!(category_column(mission), Some(qual_types[0].0));
            assert_eq!(object_type_column(mission), Some(qual_types[1].0));
        }
        let jwst = column_list("jwst");
        assert!(jwst.iter().any(|entry| entry["column_name"] == LEVEL_COLUMN && entry["queryable"] == "yes"));
        let iue = column_list("iue");
        let class = iue.iter().find(|entry| entry["column_name"] == "iue_category").unwrap();
        assert_eq!((&class["queryable"], &class["qual_type"]), (&json!("yes"), &json!("integer")));
        assert_eq!(category_column(MastMissionId::Iue), Some("iue_category"));
        assert_eq!(object_type_column(MastMissionId::Iue), None);
    }

    #[test]
    fn filter_groups_per_mission() {
        let ids = |group: &MastFilterGroup| group.options.iter().map(|option| option.id.clone()).collect::<Vec<_>>();
        let every_category = ["galaxies", "galaxy_clusters", "ism", "star_clusters", "stars", "solar_system", "fields", "calibration"];
        let every_type = [
            "planetary_nebulae",
            "nebulae_star_forming",
            "young_stars_disks",
            "exoplanets",
            "agn_quasars",
            "interacting_galaxies",
            "supernovae_transients",
            "compact_substellar",
        ];
        for (mission, columns) in [(MastMissionId::Jwst, ["targcat", "targdesc"]), (MastMissionId::Hst, ["sci_broad_category", "sci_target_descrip"])] {
            let groups = filter_groups(mission);
            assert_eq!(groups.len(), 2, "{mission:?}");
            assert_eq!((groups[0].id, groups[1].id), (FilterGroupId::Category, FilterGroupId::ObjectType));
            assert_eq!((groups[0].label.as_str(), groups[1].label.as_str()), ("Category", "Object type"));
            assert_eq!([groups[0].column.as_str(), groups[1].column.as_str()], columns);
            assert_eq!(ids(&groups[0]), every_category);
            assert_eq!(ids(&groups[1]), every_type);
        }
        let iue = filter_groups(MastMissionId::Iue);
        assert_eq!(iue.len(), 1);
        assert_eq!((iue[0].id, iue[0].column.as_str()), (FilterGroupId::Category, "iue_category"));
        assert_eq!(ids(&iue[0]), ["galaxies", "ism", "star_clusters", "stars", "solar_system"]);
        for mission in [
            MastMissionId::Roman,
            MastMissionId::Classy,
            MastMissionId::Ullyses,
            MastMissionId::RomanCgi,
            MastMissionId::RomanSpectra,
            MastMissionId::Tess,
            MastMissionId::MultiMission,
        ] {
            assert!(filter_groups(mission).is_empty(), "{mission:?}");
        }
        let jwst = filter_groups(MastMissionId::Jwst);
        let hst = filter_groups(MastMissionId::Hst);
        let title = |group: &MastFilterGroup, id: &str| group.options.iter().find(|option| option.id == id).unwrap().title.clone();
        assert_eq!(title(&jwst[0], "galaxies"), "targcat = Galaxy");
        assert_eq!(title(&jwst[0], "galaxy_clusters"), "targcat = Clusters of Galaxies");
        assert_eq!(title(&hst[0], "galaxies"), "sci_broad_category has GALAXY");
        assert_eq!(title(&hst[0], "stars"), "sci_broad_category has STAR or EXT-STAR");
        assert_eq!(title(&iue[0], "galaxies"), "iue_category 80..82, 84..88");
        assert_eq!(title(&iue[0], "star_clusters"), "iue_category 83");
        assert_eq!(title(&jwst[1], "planetary_nebulae"), "targdesc contains Planetary nebulae");
        assert_eq!(title(&jwst[1], "interacting_galaxies"), "targdesc contains Interacting galaxies or Galaxy mergers");
        assert_eq!(
            title(&jwst[1], "nebulae_star_forming"),
            "targdesc contains nebula, H II regions, Supernova remnants, Herbig-Haro objects, Photodissociation regions, Interstellar clouds, Molecular clouds, Molecular gas or Protoclusters"
        );
        assert_eq!(title(&hst[1], "planetary_nebulae"), "sci_target_descrip contains PLANETARY NEBULA");
        assert_eq!(
            title(&hst[1], "young_stars_disks"),
            "sci_target_descrip contains T TAURI, PROTOPLANETARY DISK, PRE-MAIN SEQUENCE, YSO or HERBIG AE"
        );
        assert_eq!(
            title(&hst[1], "compact_substellar"),
            "sci_target_descrip contains BROWN DWARF, WHITE DWARF, STAR;DA, STAR;DB, STAR;DC, STAR;DO, STAR;DZ or EXT-STAR;DA"
        );
        let label = |group: &MastFilterGroup, id: &str| group.options.iter().find(|option| option.id == id).unwrap().label.clone();
        assert_eq!(label(&jwst[0], "galaxy_clusters"), "Galaxy clusters and groups");
        assert_eq!(label(&hst[1], "agn_quasars"), "Active galaxies, quasars and black holes");
        assert_eq!(
            jwst[0].info,
            "From the proposal: the target category chosen in APT (TARGCAT). Proposers choose it, so a field can be filed under another class (Orion programme 1256 is a star cluster). Datasets with no category never match."
        );
        assert_eq!(hst[0].info, "From the proposal: SCI_BROAD_CATEGORY. Datasets with no category never match.");
        assert_eq!(iue[0].info, "IUE object class (IUE_CATEGORY).");
        assert_eq!(
            jwst[1].info,
            "From the proposal's target description keywords (TARGDESC). Each type groups several keywords. Datasets with no description never match."
        );
        assert_eq!(hst[1].info, "From the proposal's target description (SCI_TARGET_DESCRIP). Each type groups several terms.");
        assert_eq!(
            serde_json::to_value(&iue[0]).unwrap()["options"][0],
            json!({"id": "galaxies", "label": "Galaxies", "title": "iue_category 80..82, 84..88"})
        );
        assert_eq!(serde_json::to_value(&jwst[1]).unwrap()["id"], json!("object_type"));
    }

    #[test]
    fn row_categories_are_exactly_the_categories_whose_filter_matches() {
        let jwst = |value: Value| row_categories(MastMissionId::Jwst, &raw("targcat", value));
        assert_eq!(jwst(json!("Galaxy")), [ObjectCategory::Galaxies]);
        assert_eq!(jwst(json!(" stellar cluster ")), [ObjectCategory::StarClusters]);
        assert_eq!(jwst(json!("Clusters of Galaxies")), [ObjectCategory::GalaxyClusters]);
        assert!(jwst(json!("Galaxies")).is_empty());
        assert!(jwst(json!("None")).is_empty());
        assert!(jwst(Value::Null).is_empty());
        let hst = |value: Value| row_categories(MastMissionId::Hst, &raw("sci_broad_category", value));
        assert_eq!(hst(json!("STAR;GALAXY")), [ObjectCategory::Galaxies, ObjectCategory::Stars]);
        assert_eq!(hst(json!("EXT-STAR;EXT-MEDIUM")), [ObjectCategory::Ism, ObjectCategory::Stars]);
        assert_eq!(hst(json!("ISM;HII REGION;IONIZATION FRONT;")), [ObjectCategory::Ism]);
        assert_eq!(hst(json!("ext-cluster")), [ObjectCategory::StarClusters]);
        assert!(hst(json!("STELLAR CLUSTER, OPEN CLUSTER")).is_empty());
        assert!(hst(json!("STARS")).is_empty());
        assert!(hst(json!("")).is_empty());
        let iue = |value: Value| row_categories(MastMissionId::Iue, &raw("iue_category", value));
        assert_eq!(iue(json!(83)), [ObjectCategory::StarClusters]);
        assert_eq!(iue(json!("12")), [ObjectCategory::Stars]);
        assert_eq!(iue(json!(0)), [ObjectCategory::SolarSystem]);
        assert_eq!(iue(json!(88)), [ObjectCategory::Galaxies]);
        for unmapped in [json!(67), json!(77), json!(89), json!(90), json!(99), Value::Null, json!("x")] {
            assert!(iue(unmapped.clone()).is_empty(), "{unmapped}");
        }
        assert!(row_categories(MastMissionId::Roman, &raw("targcat", json!("Galaxy"))).is_empty());
        for row in &CATEGORIES {
            assert_eq!(jwst(json!(row.jwst)), [row.id]);
            for token in row.hst {
                assert_eq!(hst(json!(token)), [row.id]);
                assert!(hst(json!(format!("CALIBRATION;{token}"))).contains(&row.id), "{token}");
            }
            for (low, high) in row.iue {
                assert_eq!(iue(json!(low)), [row.id]);
                assert_eq!(iue(json!(high)), [row.id]);
            }
        }
        assert_eq!(category_text(&[ObjectCategory::Galaxies, ObjectCategory::Stars]).as_deref(), Some("Galaxies, Stars"));
        assert_eq!(category_text(&[ObjectCategory::Ism]).as_deref(), Some("Nebulae and ISM"));
        assert_eq!(category_text(&[]), None);
        assert_eq!(all_categories().len(), 8);
    }

    #[test]
    fn levels_map_to_their_tokens() {
        assert_eq!(level_value(LevelFilter::Combined), Some("3"));
        assert_eq!(level_value(LevelFilter::Exposures), Some("1b, 2a, 2b, 2c"));
        assert_eq!(level_value(LevelFilter::All), None);
        assert!(is_object_column(MastMissionId::Jwst, "targdesc"));
        assert!(is_object_column(MastMissionId::Iue, "iue_category"));
        assert!(!is_object_column(MastMissionId::Iue, "targcat"));
        assert!(!is_object_column(MastMissionId::Jwst, LEVEL_COLUMN));
    }

    fn live_cases() -> Vec<(MastMissionId, &'static str, String)> {
        let mut cases = Vec::new();
        for mission in OFFERING {
            let column = category_column(mission).expect("a category column");
            for row in &CATEGORIES {
                let parts = category_parts(mission, row);
                let probes: Vec<String> = match mission {
                    MastMissionId::Hst => parts.chunks(4).map(|patterns| patterns.join(", ")).collect(),
                    _ => parts,
                };
                cases.extend(probes.into_iter().map(|value| (mission, column, value)));
            }
            if let Some(column) = object_type_column(mission) {
                for row in &OBJECT_TYPES {
                    cases.extend(object_type_parts(mission, row).into_iter().map(|value| (mission, column, value)));
                }
            }
        }
        cases
    }

    #[tokio::test]
    #[ignore = "live MAST request"]
    async fn live_every_table_value_matches_archive_rows() {
        let dir = tempfile::tempdir().unwrap();
        let hub = MastHub::new(MastSettings::default(), dir.path().to_path_buf()).unwrap();
        let mut dead = Vec::new();
        for (mission, column, value) in live_cases() {
            let endpoint = hub.endpoint(mission, OpKind::Search).unwrap();
            let params = SearchParams::new().count_only().condition(column, value.as_str());
            let total = endpoint.search::<FieldMap>(&params).await.unwrap().total_results;
            eprintln!("{mission:?} {column} {value:?}: {total}");
            if total < 1 {
                dead.push(format!("{mission:?} {column} {value:?}"));
            }
        }
        assert!(dead.is_empty(), "values that match no MAST row: {dead:?}");
    }
}
