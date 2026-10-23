use std::time::SystemTime;

use super::object_filters::{filter_groups, LEVEL_COLUMN};
use super::types::{
    ColumnKind, MastAvailability, MastColumn, MastMission, MastMissionId, MastQuickFilter, MissionCapabilities,
    MissionViews, QuickFilterId,
};

pub const CHECKED_ON: &str = "2026-10-16";
pub const PROG_OBS_COLUMN: &str = "_prog_obs";
pub const SEP_COLUMN: &str = "_sep_arcmin";
pub const CATEGORY_COLUMN: &str = "_category";
pub const JWST_FILTER_WILDCARD_OK: bool = true;
pub const JWST_DATE_SORT_OK: bool = false;
pub const HST_DATASET_CONDITION_OK: bool = true;
pub const IUE_PRODUCTS_OK: bool = true;
pub const CLASSY_PRODUCTS_OK: bool = false;
pub const ULLYSES_PRODUCTS_OK: bool = false;

pub const MISSION_ORDER: [MastMissionId; 10] = [
    MastMissionId::Jwst,
    MastMissionId::Hst,
    MastMissionId::Roman,
    MastMissionId::Iue,
    MastMissionId::Classy,
    MastMissionId::Ullyses,
    MastMissionId::RomanCgi,
    MastMissionId::RomanSpectra,
    MastMissionId::Tess,
    MastMissionId::MultiMission,
];

pub fn label(mission: MastMissionId) -> &'static str {
    match mission {
        MastMissionId::Jwst => "JWST",
        MastMissionId::Hst => "HST",
        MastMissionId::Roman => "Roman",
        MastMissionId::Iue => "IUE",
        MastMissionId::Classy => "CLASSY",
        MastMissionId::Ullyses => "ULLYSES",
        MastMissionId::RomanCgi => "Roman CGI",
        MastMissionId::RomanSpectra => "Roman Spectra",
        MastMissionId::Tess => "TESS",
        MastMissionId::MultiMission => "All missions",
    }
}

pub fn dataset_column(mission: MastMissionId) -> Option<&'static str> {
    match mission {
        MastMissionId::Jwst | MastMissionId::Roman | MastMissionId::RomanCgi => Some("fileSetName"),
        MastMissionId::Hst => Some("sci_data_set_name"),
        MastMissionId::Iue => Some("iue_data_id"),
        MastMissionId::Classy => Some("Target"),
        MastMissionId::Ullyses => Some("target_id"),
        MastMissionId::RomanSpectra | MastMissionId::Tess | MastMissionId::MultiMission => None,
    }
}

pub fn verified_server_sort(mission: MastMissionId, column: &str) -> bool {
    mission == MastMissionId::Jwst && column == "date_obs" && JWST_DATE_SORT_OK
}

fn shared_label(name: &str) -> Option<&'static str> {
    let label = match name {
        PROG_OBS_COLUMN => "Prog.Obs",
        CATEGORY_COLUMN => "Category",
        "s_region" => "Footprint",
        _ => return None,
    };
    Some(label)
}

fn jwst_label(name: &str) -> Option<&'static str> {
    let label = match name {
        "fileSetName" => "Dataset",
        "productLevel" => "Lvl",
        "instrume" => "Instr",
        "filter" => "Filter",
        "exp_type" => "Exp type",
        "targname" => "Target",
        "date_obs" => "Date obs (UTC)",
        "effexptm" => "Exp (s)",
        "access" => "Access",
        "program" => "Program",
        "observtn" => "Obs",
        "opticalElements" => "Optical elements",
        "targ_ra" => "RA",
        "targ_dec" => "Dec",
        "publicReleaseDate" => "Public release",
        "targcat" => "Category",
        "targdesc" => "Target description",
        SEP_COLUMN => "Sep (MAST)",
        _ => return None,
    };
    Some(label)
}

fn hst_label(name: &str) -> Option<&'static str> {
    let label = match name {
        "sci_data_set_name" => "Dataset",
        "sci_instrume" => "Instr",
        "sci_spec_1234" => "Filter",
        "sci_aper_1234" => "Aperture",
        "sci_targname" => "Target",
        "sci_start_time" => "Date obs (UTC)",
        "sci_actual_duration" => "Exp (s)",
        "sci_pep_id" => "Program",
        "sci_ra" => "RA",
        "sci_dec" => "Dec",
        "sci_release_date" => "Public release",
        "sci_broad_category" => "Category",
        "sci_target_descrip" => "Target description",
        "ang_sep" => "Sep (MAST)",
        SEP_COLUMN => "Sep",
        _ => return None,
    };
    Some(label)
}

fn roman_label(name: &str) -> Option<&'static str> {
    let label = match name {
        "fileSetName" => "Dataset",
        "dataset_category" => "Lvl",
        "instrument_name" => "Instr",
        "optical_element" => "Filter",
        "exposure_type" => "Exp type",
        "exposure_start_time" => "Date obs (UTC)",
        "exposure_time" => "Exp (s)",
        "access" => "Access",
        "program" => "Program",
        "observation" => "Obs",
        "ra" => "RA",
        "dec" => "Dec",
        _ => return None,
    };
    Some(label)
}

fn other_label(mission: MastMissionId, name: &str) -> Option<&'static str> {
    let label = match (mission, name) {
        (MastMissionId::Iue, "iue_data_id") => "Dataset",
        (MastMissionId::Iue, "iue_target_name") => "Target",
        (MastMissionId::Iue, "iue_disp") => "Dispersion",
        (MastMissionId::Iue, "iue_aper") => "Aperture",
        (MastMissionId::Iue, "iue_obs_start_time") => "Date obs (UTC)",
        (MastMissionId::Iue, "iue_com_exp_time") => "Exp (s)",
        (MastMissionId::Iue, "iue_category") => "Object class",
        (MastMissionId::Classy, "Target") => "Target",
        (MastMissionId::Classy, "distance") => "Distance",
        (MastMissionId::Classy, "log_Mstar") => "log M*",
        (MastMissionId::Classy, "log_SFR") => "log SFR",
        (MastMissionId::Ullyses, "target_name_ullyses") => "Target",
        (MastMissionId::Ullyses, "target_classification") => "Class",
        (MastMissionId::Ullyses, "spectral_type") => "Spectral type",
        (MastMissionId::Ullyses, "instrument") => "Instr",
        (MastMissionId::Ullyses, "grating") => "Grating",
        (MastMissionId::Ullyses, "filter") => "Filter",
        (MastMissionId::RomanCgi, "fileSetName") => "Dataset",
        _ => return None,
    };
    Some(label)
}

fn column_label(mission: MastMissionId, name: &str) -> Option<&'static str> {
    let specific = match mission {
        MastMissionId::Jwst => jwst_label(name),
        MastMissionId::Hst => hst_label(name),
        MastMissionId::Roman => roman_label(name),
        _ => other_label(mission, name),
    };
    specific.or_else(|| shared_label(name))
}

fn is_date_column(name: &str) -> bool {
    matches!(
        name,
        "date_obs"
            | "publicReleaseDate"
            | "exposure_start_time"
            | "sci_start_time"
            | "sci_stop_time"
            | "sci_release_date"
            | "iue_obs_start_time"
    )
}

fn kind_hint(name: &str) -> ColumnKind {
    match name {
        _ if is_date_column(name) => ColumnKind::Datetime,
        "program" | "observtn" | "observation" | "sci_pep_id" | "target_id" => ColumnKind::Int,
        "effexptm" | "sci_actual_duration" | "exposure_time" | "iue_com_exp_time" | "ang_sep" | SEP_COLUMN | "targ_ra"
        | "targ_dec" | "sci_ra" | "sci_dec" | "ra" | "dec" | "z" | "distance" | "log_Mstar" | "log_SFR" => ColumnKind::Float,
        _ => ColumnKind::Text,
    }
}

fn kind_from_type(name: &str, info_type: Option<&str>) -> ColumnKind {
    if is_date_column(name) || name == "ang_sep" || name.starts_with('_') {
        return kind_hint(name);
    }
    let Some(raw) = info_type.map(str::to_ascii_lowercase) else {
        return kind_hint(name);
    };
    if raw.contains("datetime") || raw.contains("timestamp") || raw == "date" {
        ColumnKind::Datetime
    } else if ["float", "double", "real", "numeric", "decimal"].iter().any(|t| raw.contains(t)) {
        ColumnKind::Float
    } else if ["long", "int", "short"].iter().any(|t| raw.contains(t)) {
        ColumnKind::Int
    } else if raw.contains("char") || raw.contains("text") || raw.contains("bool") {
        ColumnKind::Text
    } else {
        kind_hint(name)
    }
}

fn unit(mission: MastMissionId, name: &str) -> Option<&'static str> {
    match name {
        "effexptm" | "sci_actual_duration" | "exposure_time" | "iue_com_exp_time" => Some("s"),
        "targ_ra" | "targ_dec" | "sci_ra" | "sci_dec" | "ra" | "dec" => Some("deg"),
        SEP_COLUMN if mission == MastMissionId::Hst => Some("arcmin"),
        _ => None,
    }
}

pub fn column(mission: MastMissionId, name: &str, info_type: Option<&str>, info_sortable: bool) -> MastColumn {
    MastColumn {
        name: name.to_owned(),
        label: column_label(mission, name).unwrap_or(name).to_owned(),
        kind: kind_from_type(name, info_type),
        sortable: !name.starts_with('_') && info_sortable && verified_server_sort(mission, name),
        unit: unit(mission, name).map(str::to_owned),
    }
}

pub fn has_synthetic(mission: MastMissionId, name: &str) -> bool {
    match name {
        PROG_OBS_COLUMN => matches!(mission, MastMissionId::Jwst | MastMissionId::Hst | MastMissionId::Roman),
        SEP_COLUMN => matches!(mission, MastMissionId::Jwst | MastMissionId::Hst),
        CATEGORY_COLUMN => matches!(mission, MastMissionId::Jwst | MastMissionId::Hst | MastMissionId::Iue),
        _ => false,
    }
}

pub fn synthetic_columns(mission: MastMissionId) -> Vec<MastColumn> {
    [PROG_OBS_COLUMN, SEP_COLUMN, CATEGORY_COLUMN]
        .into_iter()
        .filter(|name| has_synthetic(mission, name))
        .map(|name| column(mission, name, None, false))
        .collect()
}

fn default_column_names(mission: MastMissionId) -> &'static [&'static str] {
    match mission {
        MastMissionId::Jwst => &[
            "fileSetName",
            "productLevel",
            "instrume",
            "filter",
            "exp_type",
            "targname",
            CATEGORY_COLUMN,
            PROG_OBS_COLUMN,
            "date_obs",
            "effexptm",
            SEP_COLUMN,
            "access",
        ],
        MastMissionId::Hst => &[
            "sci_data_set_name",
            "sci_instrume",
            "sci_spec_1234",
            "sci_aper_1234",
            "sci_targname",
            CATEGORY_COLUMN,
            PROG_OBS_COLUMN,
            "sci_start_time",
            "sci_actual_duration",
            SEP_COLUMN,
        ],
        MastMissionId::Roman => &[
            "fileSetName",
            "dataset_category",
            "instrument_name",
            "optical_element",
            "exposure_type",
            PROG_OBS_COLUMN,
            "exposure_start_time",
            "exposure_time",
            "access",
        ],
        MastMissionId::Iue => &["iue_data_id", "iue_target_name", CATEGORY_COLUMN, "iue_disp", "iue_aper", "iue_obs_start_time", "iue_com_exp_time"],
        MastMissionId::Classy => &["Target", "z", "distance", "log_Mstar", "log_SFR"],
        MastMissionId::Ullyses => &["target_name_ullyses", "target_classification", "spectral_type", "instrument", "grating", "filter"],
        MastMissionId::RomanCgi | MastMissionId::RomanSpectra | MastMissionId::Tess | MastMissionId::MultiMission => &[],
    }
}

fn quick(id: QuickFilterId, label: &str, column: &str, options: Option<&[&str]>) -> MastQuickFilter {
    MastQuickFilter {
        id,
        label: label.to_owned(),
        column: column.to_owned(),
        options: options.map(|values| values.iter().map(|v| (*v).to_owned()).collect()),
    }
}

fn quick_filters(mission: MastMissionId) -> Vec<MastQuickFilter> {
    let jwst_filter_column = if JWST_FILTER_WILDCARD_OK { "opticalElements" } else { "filter" };
    match mission {
        MastMissionId::Jwst => vec![
            quick(QuickFilterId::Level, "Level", LEVEL_COLUMN, Some(&["combined", "exposures", "all"])),
            quick(QuickFilterId::Instrument, "Instrument", "instrume", Some(&["NIRCAM", "MIRI", "NIRSPEC", "NIRISS", "FGS"])),
            quick(QuickFilterId::Filter, "Filter", jwst_filter_column, None),
            quick(QuickFilterId::Date, "Date", "date_obs", None),
            quick(QuickFilterId::Public, "Public only", "access", None),
        ],
        MastMissionId::Hst => vec![
            quick(QuickFilterId::Instrument, "Instrument", "sci_instrume", Some(&["WFC3", "ACS", "WFPC2", "NICMOS", "STIS", "COS"])),
            quick(QuickFilterId::Filter, "Filter", "sci_spec_1234", None),
            quick(QuickFilterId::Date, "Date", "sci_start_time", None),
        ],
        MastMissionId::Roman => vec![
            quick(QuickFilterId::Instrument, "Instrument", "instrument_name", None),
            quick(QuickFilterId::Filter, "Filter", "optical_element", None),
            quick(QuickFilterId::Date, "Date", "exposure_start_time", None),
            quick(QuickFilterId::Public, "Public only", "access", None),
        ],
        _ => Vec::new(),
    }
}

fn not_deployed_note(checked_on: &str) -> String {
    format!("not deployed by MAST (checked {checked_on})")
}

fn static_entry(id: MastMissionId) -> (MastAvailability, (bool, bool, bool), Option<String>) {
    match id {
        MastMissionId::Jwst | MastMissionId::Hst => (MastAvailability::Open, (true, true, true), None),
        MastMissionId::Roman => (
            MastAvailability::Token,
            (true, true, true),
            Some("needs a MAST token and STScI authorization; anonymous calls answer 401".to_owned()),
        ),
        MastMissionId::Iue => (
            MastAvailability::Open,
            (true, false, IUE_PRODUCTS_OK),
            Some(format!("no image endpoints at MAST (checked {CHECKED_ON})")),
        ),
        MastMissionId::Ullyses => (
            MastAvailability::Open,
            (true, false, ULLYSES_PRODUCTS_OK),
            Some(format!("no image endpoints at MAST (checked {CHECKED_ON})")),
        ),
        MastMissionId::Classy => (
            MastAvailability::Open,
            (true, false, CLASSY_PRODUCTS_OK),
            Some(format!("rows are targets; no image endpoints at MAST (checked {CHECKED_ON})")),
        ),
        MastMissionId::RomanCgi => (
            MastAvailability::Token,
            (false, false, false),
            Some("needs Roman authorization; coronagraphy is not processed by the app".to_owned()),
        ),
        MastMissionId::RomanSpectra | MastMissionId::Tess | MastMissionId::MultiMission => {
            (MastAvailability::NotDeployed, (false, false, false), Some(not_deployed_note(CHECKED_ON)))
        }
    }
}

pub fn mission(id: MastMissionId) -> MastMission {
    let (availability, (search, images, products), note) = static_entry(id);
    MastMission {
        id,
        label: label(id).to_owned(),
        availability,
        capabilities: MissionCapabilities { search, images, products },
        views: MissionViews { gallery: images, table: search },
        quick_filters: quick_filters(id),
        default_columns: default_column_names(id).iter().map(|name| column(id, name, None, true)).collect(),
        note,
        checked_on: CHECKED_ON.to_owned(),
        filter_groups: filter_groups(id),
    }
}

pub fn missions() -> Vec<MastMission> {
    MISSION_ORDER.into_iter().map(mission).collect()
}

pub fn merge_probe(base: MastMission, availability: MastAvailability, checked_on: &str) -> MastMission {
    let mut merged = base;
    let was_not_deployed = merged.availability == MastAvailability::NotDeployed;
    merged.availability = availability;
    merged.checked_on = checked_on.to_owned();
    if availability == MastAvailability::NotDeployed {
        merged.capabilities = MissionCapabilities { search: false, images: false, products: false };
        merged.views = MissionViews { gallery: false, table: false };
        merged.note = Some(not_deployed_note(checked_on));
        return merged;
    }
    if availability == MastAvailability::Open {
        merged.capabilities.search = true;
        merged.views.table = true;
    }
    if was_not_deployed {
        merged.note = None;
    }
    merged
}

pub fn utc_date(time: SystemTime) -> String {
    let secs = time.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

pub fn utc_datetime(time: SystemTime) -> String {
    let secs = time.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) % 86_400;
    format!("{}T{:02}:{:02}:{:02}Z", utc_date(time), secs / 3600, secs % 3600 / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn caps(m: &MastMission) -> (bool, bool, bool) {
        (m.capabilities.search, m.capabilities.images, m.capabilities.products)
    }

    #[test]
    fn missions_matrix_matches_design() {
        let all = missions();
        assert_eq!(all.len(), 10);
        let ids: Vec<MastMissionId> = all.iter().map(|m| m.id).collect();
        assert_eq!(ids, MISSION_ORDER.to_vec());
        let expected = [
            (MastMissionId::Jwst, "JWST", MastAvailability::Open, (true, true, true)),
            (MastMissionId::Hst, "HST", MastAvailability::Open, (true, true, true)),
            (MastMissionId::Roman, "Roman", MastAvailability::Token, (true, true, true)),
            (MastMissionId::Iue, "IUE", MastAvailability::Open, (true, false, IUE_PRODUCTS_OK)),
            (MastMissionId::Classy, "CLASSY", MastAvailability::Open, (true, false, CLASSY_PRODUCTS_OK)),
            (MastMissionId::Ullyses, "ULLYSES", MastAvailability::Open, (true, false, ULLYSES_PRODUCTS_OK)),
            (MastMissionId::RomanCgi, "Roman CGI", MastAvailability::Token, (false, false, false)),
            (MastMissionId::RomanSpectra, "Roman Spectra", MastAvailability::NotDeployed, (false, false, false)),
            (MastMissionId::Tess, "TESS", MastAvailability::NotDeployed, (false, false, false)),
            (MastMissionId::MultiMission, "All missions", MastAvailability::NotDeployed, (false, false, false)),
        ];
        for (m, (id, name, availability, capabilities)) in all.iter().zip(expected) {
            assert_eq!(m.id, id);
            assert_eq!(m.label, name);
            assert_eq!(m.availability, availability, "{name}");
            assert_eq!(caps(m), capabilities, "{name}");
            assert_eq!(m.checked_on, CHECKED_ON);
            assert_eq!(m.views.gallery, m.capabilities.images, "{name}");
            assert_eq!(m.views.table, m.capabilities.search, "{name}");
        }
        let jwst = &all[0];
        let filters: Vec<(QuickFilterId, &str)> = jwst.quick_filters.iter().map(|f| (f.id, f.column.as_str())).collect();
        let filter_column = if JWST_FILTER_WILDCARD_OK { "opticalElements" } else { "filter" };
        assert_eq!(
            filters,
            vec![
                (QuickFilterId::Level, "productLevel"),
                (QuickFilterId::Instrument, "instrume"),
                (QuickFilterId::Filter, filter_column),
                (QuickFilterId::Date, "date_obs"),
                (QuickFilterId::Public, "access"),
            ]
        );
        assert_eq!(jwst.quick_filters[0].options, Some(["combined", "exposures", "all"].map(str::to_owned).to_vec()));
        assert_eq!(
            jwst.quick_filters[1].options,
            Some(["NIRCAM", "MIRI", "NIRSPEC", "NIRISS", "FGS"].map(str::to_owned).to_vec())
        );
        let hst = &all[1];
        let hst_filters: Vec<(QuickFilterId, &str)> = hst.quick_filters.iter().map(|f| (f.id, f.column.as_str())).collect();
        assert_eq!(
            hst_filters,
            vec![
                (QuickFilterId::Instrument, "sci_instrume"),
                (QuickFilterId::Filter, "sci_spec_1234"),
                (QuickFilterId::Date, "sci_start_time"),
            ]
        );
        assert_eq!(
            hst.quick_filters[0].options,
            Some(["WFC3", "ACS", "WFPC2", "NICMOS", "STIS", "COS"].map(str::to_owned).to_vec())
        );
        let roman_columns: Vec<&str> = all[2].quick_filters.iter().map(|f| f.column.as_str()).collect();
        assert_eq!(roman_columns, vec!["instrument_name", "optical_element", "exposure_start_time", "access"]);
        for m in &all[3..] {
            assert!(m.quick_filters.is_empty(), "{}", m.label);
        }
        let jwst_cols: Vec<&str> = jwst.default_columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            jwst_cols,
            vec!["fileSetName", "productLevel", "instrume", "filter", "exp_type", "targname", "_category", "_prog_obs", "date_obs", "effexptm", "_sep_arcmin", "access"]
        );
        let hst_cols: Vec<&str> = hst.default_columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            hst_cols,
            vec!["sci_data_set_name", "sci_instrume", "sci_spec_1234", "sci_aper_1234", "sci_targname", "_category", "_prog_obs", "sci_start_time", "sci_actual_duration", "_sep_arcmin"]
        );
        let labels: Vec<&str> = jwst.default_columns.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["Dataset", "Lvl", "Instr", "Filter", "Exp type", "Target", "Category", "Prog.Obs", "Date obs (UTC)", "Exp (s)", "Sep (MAST)", "Access"]
        );
        let effexptm = jwst.default_columns.iter().find(|c| c.name == "effexptm").unwrap();
        assert_eq!((effexptm.kind, effexptm.unit.as_deref()), (ColumnKind::Float, Some("s")));
        let date = jwst.default_columns.iter().find(|c| c.name == "date_obs").unwrap();
        assert_eq!(date.kind, ColumnKind::Datetime);
        assert_eq!(date.sortable, JWST_DATE_SORT_OK);
        let hst_sep = hst.default_columns.iter().find(|c| c.name == SEP_COLUMN).unwrap();
        assert_eq!((hst_sep.unit.as_deref(), hst_sep.sortable), (Some("arcmin"), false));
        for m in &all {
            assert!(m.default_columns.iter().all(|c| !c.name.starts_with('_') || !c.sortable));
        }
        let iue_cols: Vec<(&str, &str)> = all[3].default_columns.iter().map(|c| (c.name.as_str(), c.label.as_str())).collect();
        assert_eq!(
            iue_cols,
            vec![
                ("iue_data_id", "Dataset"),
                ("iue_target_name", "Target"),
                ("_category", "Category"),
                ("iue_disp", "Dispersion"),
                ("iue_aper", "Aperture"),
                ("iue_obs_start_time", "Date obs (UTC)"),
                ("iue_com_exp_time", "Exp (s)")
            ]
        );
        for m in &all[4..] {
            assert!(m.default_columns.iter().all(|c| c.name != CATEGORY_COLUMN), "{}", m.label);
        }
        assert!(all[6].note.as_deref().unwrap().contains("coronagraphy"));
        for m in &all[7..] {
            assert_eq!(m.note.as_deref(), Some("not deployed by MAST (checked 2026-10-16)"));
        }
    }

    #[test]
    fn missions_carry_their_filter_groups_and_category_columns() {
        let all = missions();
        let groups: Vec<usize> = all.iter().map(|m| m.filter_groups.len()).collect();
        assert_eq!(groups, [2, 2, 0, 1, 0, 0, 0, 0, 0, 0]);
        for m in &all {
            assert_eq!(m.filter_groups, crate::core::mast::object_filters::filter_groups(m.id), "{}", m.label);
        }
        let jwst = serde_json::to_value(&all[0]).unwrap();
        assert_eq!(jwst["filterGroups"][0]["id"], serde_json::json!("category"));
        assert_eq!(jwst["filterGroups"][1]["options"].as_array().unwrap().len(), 8);
        assert_eq!(all[3].filter_groups[0].options.len(), 5);
        let merged = merge_probe(mission(MastMissionId::Iue), MastAvailability::Open, "2026-11-02");
        assert_eq!(merged.filter_groups, all[3].filter_groups);
        for (mission, name, label) in [
            (MastMissionId::Jwst, "targcat", "Category"),
            (MastMissionId::Jwst, "targdesc", "Target description"),
            (MastMissionId::Hst, "sci_broad_category", "Category"),
            (MastMissionId::Hst, "sci_target_descrip", "Target description"),
            (MastMissionId::Iue, "iue_category", "Object class"),
            (MastMissionId::Jwst, CATEGORY_COLUMN, "Category"),
            (MastMissionId::Iue, CATEGORY_COLUMN, "Category"),
        ] {
            assert_eq!(column(mission, name, Some("char"), true).label, label, "{mission:?} {name}");
        }
        let synthetic = column(MastMissionId::Hst, CATEGORY_COLUMN, None, true);
        assert_eq!((synthetic.kind, synthetic.sortable, synthetic.unit), (ColumnKind::Text, false, None));
        for mission in [MastMissionId::Jwst, MastMissionId::Hst, MastMissionId::Iue] {
            assert!(has_synthetic(mission, CATEGORY_COLUMN), "{mission:?}");
            assert_eq!(synthetic_columns(mission).last().map(|c| c.name.as_str()), Some(CATEGORY_COLUMN), "{mission:?}");
        }
        for mission in [MastMissionId::Roman, MastMissionId::Classy, MastMissionId::Ullyses, MastMissionId::Tess] {
            assert!(!has_synthetic(mission, CATEGORY_COLUMN), "{mission:?}");
        }
        assert_eq!(synthetic_columns(MastMissionId::Iue).len(), 1);
    }

    #[test]
    fn probe_results_merge_into_the_matrix() {
        let tess = merge_probe(mission(MastMissionId::Tess), MastAvailability::Open, "2026-11-02");
        assert_eq!(tess.availability, MastAvailability::Open);
        assert!(tess.capabilities.search && tess.views.table);
        assert!(!tess.capabilities.images);
        assert_eq!(tess.checked_on, "2026-11-02");
        assert_eq!(tess.note, None);
        let jwst = merge_probe(mission(MastMissionId::Jwst), MastAvailability::NotDeployed, "2026-11-02");
        assert_eq!(caps(&jwst), (false, false, false));
        assert!(!jwst.views.gallery && !jwst.views.table);
        assert_eq!(jwst.note.as_deref(), Some("not deployed by MAST (checked 2026-11-02)"));
        let cgi = merge_probe(mission(MastMissionId::RomanCgi), MastAvailability::Token, "2026-11-02");
        assert_eq!(cgi.availability, MastAvailability::Token);
        assert_eq!(caps(&cgi), (false, false, false));
        assert_eq!(cgi.note, mission(MastMissionId::RomanCgi).note);
    }

    #[test]
    fn utc_date_formats_the_civil_day() {
        assert_eq!(utc_date(UNIX_EPOCH), "1970-01-01");
        assert_eq!(utc_date(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29");
        assert_eq!(utc_date(UNIX_EPOCH + Duration::from_secs(1_791_244_799)), "2026-10-05");
        assert_eq!(utc_date(UNIX_EPOCH + Duration::from_secs(1_791_244_800)), "2026-10-06");
    }

    #[test]
    fn utc_datetime_formats_iso_seconds() {
        assert_eq!(utc_datetime(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(utc_datetime(UNIX_EPOCH + Duration::from_secs(1_791_244_799)), "2026-10-05T23:59:59Z");
        assert_eq!(utc_datetime(UNIX_EPOCH + Duration::from_millis(1_791_288_600_900)), "2026-10-06T12:10:00Z");
    }

    #[test]
    fn dataset_columns_follow_the_field_rules() {
        assert_eq!(dataset_column(MastMissionId::Jwst), Some("fileSetName"));
        assert_eq!(dataset_column(MastMissionId::Roman), Some("fileSetName"));
        assert_eq!(dataset_column(MastMissionId::Hst), Some("sci_data_set_name"));
        assert_eq!(dataset_column(MastMissionId::Iue), Some("iue_data_id"));
        assert_eq!(dataset_column(MastMissionId::Classy), Some("Target"));
        assert_eq!(dataset_column(MastMissionId::Ullyses), Some("target_id"));
        assert!(!verified_server_sort(MastMissionId::Jwst, "fileSetName"));
        assert_eq!(verified_server_sort(MastMissionId::Jwst, "date_obs"), JWST_DATE_SORT_OK);
        assert!(!verified_server_sort(MastMissionId::Hst, "date_obs"));
    }
}
