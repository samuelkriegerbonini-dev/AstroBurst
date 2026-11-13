use std::path::Path;
use std::sync::OnceLock;

use anyhow::{bail, Result};
use regex::Regex;
use serde::Serialize;

use super::series::series_label;
use super::white_light::{light_curves, CurveMode, CurveRequest, LimitsOrigin};
use super::{SeriesKey, TsoSeries, TsoSet, UNKNOWN_UNIT};
use crate::infra::tables::{read_text_table, TextTable};

pub const WHTLT_JOIN_TOLERANCE_D: f64 = 1e-9;
pub const WHTLT_MATCH_TOLERANCE: f64 = 1e-4;

const LEGACY_TIME_COLUMN: &str = "MJD";
const UTC_TIME_COLUMN: &str = "MJD_UTC";
const TDB_TIME_COLUMN: &str = "BJD_TDB";
const META_DETECTOR: &str = "detector";
const ANY: &str = "any";
const LEVEL_2B: u8 = 2;
const LEVEL_2B_NOTE: &str = "these are level 2b spectra: the archived white light comes from the level 3 product, extracted after outlier detection, so a difference is expected";

static FLUX_COLUMN: OnceLock<Regex> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WhtltSchema {
    Legacy,
    Current,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WhtltSeries {
    pub column: String,
    pub detector: Option<String>,
    pub sporder: Option<i64>,
    pub mjd_utc: Vec<f64>,
    pub bmjd_tdb: Vec<f64>,
    pub flux: Vec<f64>,
    pub flux_unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WhtltImport {
    pub path: String,
    pub file_name: String,
    pub schema: WhtltSchema,
    pub series: Vec<WhtltSeries>,
    pub meta: serde_json::Value,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WhtltComparison {
    pub series: Option<SeriesKey>,
    pub label: Option<String>,
    pub column: String,
    pub n_whtlt: usize,
    pub n_joined: usize,
    pub max_rel_diff: f64,
    pub within_1e4: bool,
    pub limits_um: Option<[f64; 2]>,
    pub limits_origin: Option<LimitsOrigin>,
    pub notes: Vec<String>,
}

struct FluxColumn<'a> {
    name: &'a str,
    unit: Option<&'a str>,
    sporder: Option<i64>,
    detector: Option<&'a str>,
}

fn flux_column_pattern() -> &'static Regex {
    FLUX_COLUMN.get_or_init(|| Regex::new(r"^whitelight_flux(?:_order_([0-9]+))?(?:_([A-Za-z0-9]+))?$").unwrap())
}

fn flux_columns(table: &TextTable) -> Vec<FluxColumn<'_>> {
    table
        .columns
        .iter()
        .filter_map(|column| {
            let parts = flux_column_pattern().captures(&column.name)?;
            Some(FluxColumn {
                name: &column.name,
                unit: column.unit.as_deref(),
                sporder: parts.get(1).and_then(|order| order.as_str().parse().ok()),
                detector: parts.get(2).map(|detector| detector.as_str()),
            })
        })
        .collect()
}

pub fn whtlt_from_table(table: &TextTable, path: &str) -> Result<WhtltImport> {
    let file_name = Path::new(path).file_name().map_or_else(|| path.to_string(), |name| name.to_string_lossy().into_owned());
    let names: Vec<&str> = table.columns.iter().map(|column| column.name.as_str()).collect();
    let flux_columns = flux_columns(table);
    if flux_columns.is_empty() {
        bail!("{file_name}: no whitelight_flux column: this is not a white-light table (columns: {})", names.join(", "));
    }
    let has_column = |name: &str| names.contains(&name);
    let schema = if has_column(LEGACY_TIME_COLUMN) && !names.iter().any(|name| name.starts_with(UTC_TIME_COLUMN)) {
        WhtltSchema::Legacy
    } else {
        WhtltSchema::Current
    };
    let meta_detector = table.meta.get(META_DETECTOR).and_then(serde_json::Value::as_str);

    let mut notes = table.notes.clone();
    let mut series = Vec::with_capacity(flux_columns.len());
    for column in flux_columns {
        let of_the_detector =
            |time_column: &str| column.detector.map_or_else(|| time_column.to_string(), |detector| format!("{time_column}_{detector}"));
        let (utc_column, tdb_column) = match schema {
            WhtltSchema::Legacy => (LEGACY_TIME_COLUMN.to_string(), None),
            WhtltSchema::Current => (of_the_detector(UTC_TIME_COLUMN), Some(of_the_detector(TDB_TIME_COLUMN))),
        };
        if !has_column(&utc_column) {
            bail!("{file_name}: column {} has no time column {utc_column}", column.name);
        }
        let utc = table.f64_column(&utc_column)?;
        let tdb = match tdb_column {
            Some(tdb_column) if has_column(&tdb_column) => Some(table.f64_column(&tdb_column)?),
            Some(tdb_column) => {
                notes.push(format!("{file_name}: column {tdb_column} missing: no barycentric time for {}", column.name));
                None
            }
            None => None,
        };
        let flux = table.f64_column(column.name)?;
        let kept: Vec<usize> = (0..table.n_rows).filter(|&row| utc[row].is_finite() && flux[row].is_finite()).collect();
        if kept.len() < table.n_rows {
            notes.push(format!(
                "{} row(s) without a finite time or flux left out of {}",
                table.n_rows - kept.len(),
                column.name
            ));
        }
        series.push(WhtltSeries {
            column: column.name.to_string(),
            detector: column.detector.or(meta_detector).map(str::to_string),
            sporder: column.sporder,
            mjd_utc: kept.iter().map(|&row| utc[row]).collect(),
            bmjd_tdb: kept.iter().map(|&row| tdb.map_or(f64::NAN, |tdb| tdb[row])).collect(),
            flux: kept.iter().map(|&row| flux[row]).collect(),
            flux_unit: column.unit.map(str::to_string),
        });
    }
    Ok(WhtltImport { path: path.to_string(), file_name, schema, series, meta: table.meta.clone(), notes })
}

pub fn import_whtlt(path: &str) -> Result<WhtltImport> {
    whtlt_from_table(&read_text_table(path)?, path)
}

fn nearest_flux(by_time: &[(f64, f64)], time: f64) -> Option<f64> {
    let later = by_time.partition_point(|(app_time, _)| *app_time < time);
    by_time[later.saturating_sub(1)..(later + 1).min(by_time.len())]
        .iter()
        .map(|&(app_time, flux)| ((app_time - time).abs(), flux))
        .min_by(|near, far| near.0.total_cmp(&far.0))
        .filter(|(distance, _)| *distance <= WHTLT_JOIN_TOLERANCE_D)
        .map(|(_, flux)| flux)
}

fn relative_difference(app: f64, archived: f64) -> f64 {
    if archived != 0.0 {
        (app - archived).abs() / archived.abs()
    } else if app == 0.0 {
        0.0
    } else {
        f64::INFINITY
    }
}

fn without_spaces(unit: &str) -> String {
    unit.chars().filter(|symbol| !symbol.is_whitespace()).collect()
}

fn compare_column(set: &TsoSet, keys: &[SeriesKey], whtlt: &WhtltSeries) -> Result<WhtltComparison> {
    let n_whtlt = whtlt.flux.len();
    let candidates: Vec<&TsoSeries> = set
        .series
        .iter()
        .filter(|series| {
            whtlt.detector.as_deref().is_none_or(|detector| series.key.detector.eq_ignore_ascii_case(detector))
                && whtlt.sporder.is_none_or(|order| series.key.sporder == Some(order))
        })
        .collect();
    let &[series] = candidates.as_slice() else {
        let detector = whtlt.detector.as_deref().unwrap_or(ANY);
        let order = whtlt.sporder.map_or_else(|| ANY.to_string(), |order| order.to_string());
        return Ok(WhtltComparison {
            series: None,
            label: None,
            column: whtlt.column.clone(),
            n_whtlt,
            n_joined: 0,
            max_rel_diff: f64::NAN,
            within_1e4: false,
            limits_um: None,
            limits_origin: None,
            notes: vec![format!(
                "column {}: {} series of these files match it (detector {detector}, order {order}); nothing compared",
                whtlt.column,
                candidates.len()
            )],
        });
    };

    let pipeline_defaults = CurveRequest {
        mode: CurveMode::Pipeline,
        wavelength_min_um: None,
        wavelength_max_um: None,
        bins: Vec::new(),
        exclude_um: Vec::new(),
        reject_elements_sigma: None,
    };
    let curves = light_curves(set, series, &pipeline_defaults)?;
    let mut by_time: Vec<(f64, f64)> = curves.mjd_utc.iter().copied().zip(curves.white.flux.iter().copied()).collect();
    by_time.sort_by(|earlier, later| earlier.0.total_cmp(&later.0));
    let differences: Vec<f64> = whtlt
        .mjd_utc
        .iter()
        .zip(&whtlt.flux)
        .filter_map(|(&time, &archived)| nearest_flux(&by_time, time).map(|app| relative_difference(app, archived)))
        .collect();
    let n_joined = differences.len();
    let max_rel_diff = differences.into_iter().reduce(f64::max).unwrap_or(f64::NAN);

    let mut notes = Vec::new();
    if n_joined < n_whtlt {
        notes.push(format!(
            "{} of {n_whtlt} whtlt row(s) have no integration within {WHTLT_JOIN_TOLERANCE_D:e} d of their time",
            n_whtlt - n_joined
        ));
    }
    if series.file_index.first().is_some_and(|&file| set.files[file as usize].level == LEVEL_2B) {
        notes.push(LEVEL_2B_NOTE.to_string());
    }
    let another_unit = |unit: &&str| series.flux_unit != UNKNOWN_UNIT && without_spaces(unit) != without_spaces(&series.flux_unit);
    if let Some(unit) = whtlt.flux_unit.as_deref().filter(another_unit) {
        notes.push(format!("the whtlt column is in {unit} and the spectra are in {}", series.flux_unit));
    }
    notes.extend(curves.warnings);

    Ok(WhtltComparison {
        series: Some(series.key.clone()),
        label: Some(series_label(&series.key, keys)),
        column: whtlt.column.clone(),
        n_whtlt,
        n_joined,
        max_rel_diff,
        within_1e4: n_joined > 0 && max_rel_diff <= WHTLT_MATCH_TOLERANCE,
        limits_um: Some(curves.white.band_um),
        limits_origin: Some(curves.white.limits_origin),
        notes,
    })
}

pub fn compare_whtlt(set: &TsoSet, whtlt: &WhtltImport) -> Result<Vec<WhtltComparison>> {
    let keys: Vec<SeriesKey> = set.series.iter().map(|series| series.key.clone()).collect();
    whtlt.series.iter().map(|column| compare_column(set, &keys, column)).collect()
}

#[cfg(test)]
pub(crate) mod test_support {
    pub fn ecsv_text(columns: &[(&str, Option<&str>)], meta: &[(&str, &str)], rows: &[&str]) -> String {
        let mut lines = vec!["# %ECSV 1.0".to_string(), "# ---".to_string(), "# datatype:".to_string()];
        for (name, unit) in columns {
            let unit = unit.map_or_else(String::new, |unit| format!("unit: {unit}, "));
            lines.push(format!("# - {{name: {name}, {unit}datatype: float64}}"));
        }
        if !meta.is_empty() {
            lines.push("# meta: !!omap".to_string());
            lines.extend(meta.iter().map(|(key, value)| format!("# - {{{key}: {value}}}")));
        }
        lines.push("# schema: astropy-2.0".to_string());
        lines.push(columns.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(" "));
        lines.extend(rows.iter().map(|row| row.to_string()));
        lines.join("\n") + "\n"
    }

    pub fn two_detectors_ecsv() -> String {
        ecsv_text(
            &[
                ("MJD_UTC_NRS1", None),
                ("BJD_TDB_NRS1", None),
                ("whitelight_flux_NRS1", Some("Jy")),
                ("MJD_UTC_NRS2", None),
                ("BJD_TDB_NRS2", None),
                ("whitelight_flux_NRS2", Some("Jy")),
            ],
            &[("instrument", "NIRSPEC"), ("number_of_integrations", "3")],
            &[
                "60000.1 60000.102 60.0 60000.11 60000.112 6.0",
                "60000.2 60000.202 110.0 60000.21 60000.212 11.0",
                "60000.3 60000.302 160.0 nan nan nan",
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::test_support::{ecsv_text, two_detectors_ecsv};
    use super::*;
    use crate::core::analysis::tso::series::test_support::{file_of, table_of};
    use crate::core::analysis::tso::series::{build_set, read_set};
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::infra::fits::x1dints::test_fixtures::{ramp_row, SynthRow};
    use crate::infra::tables::{parse_text_table, TableFormat};

    const LABEL: &str = "t_whtlt.ecsv";
    const NO_TEXT: [&str; 0] = [];
    const NRS1_ROWS: [&str; 3] = ["60000.1 60000.102 60.0", "60000.2 60000.202 110.0", "60000.3 60000.302 160.0"];
    const UNPAIRED_ROW_NOTE: &str = "1 of 3 whtlt row(s) have no integration within 1e-9 d of their time";
    const LEVEL_2B_NOTE: &str = "these are level 2b spectra: the archived white light comes from the level 3 product, extracted after outlier detection, so a difference is expected";
    const MJY_AGAINST_JY_NOTE: &str = "the whtlt column is in MJy and the spectra are in Jy";
    const SOSS_ORDER_2_WARNING: &str = "the pipeline sums order 2 over the order 1 range 0.93 to 2.82 um, as the archived white light does: this is not a usable light curve of order 2; the masked mode uses the range of the order";
    const SOSS_ORDER_3_WARNING: &str = "the pipeline sums order 3 over the order 1 range 0.93 to 2.82 um, as the archived white light does: this is not a usable light curve of order 3; the masked mode uses the range of the order";

    fn table(text: &str) -> TextTable {
        parse_text_table(text, TableFormat::Ecsv, LABEL).unwrap()
    }

    fn imported(text: &str) -> WhtltImport {
        whtlt_from_table(&table(text), LABEL).unwrap()
    }

    fn refusal(text: &str) -> String {
        whtlt_from_table(&table(text), LABEL).unwrap_err().to_string()
    }

    fn key(detector: &str, sporder: i64) -> SeriesKey {
        SeriesKey { detector: detector.to_string(), sporder: Some(sporder) }
    }

    fn all_nan(values: &[f64], rows: usize) -> bool {
        values.len() == rows && values.iter().all(|value| value.is_nan())
    }

    fn sorted_keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys
    }

    fn legacy_ecsv() -> String {
        ecsv_text(
            &[("MJD", None), ("whitelight_flux", None)],
            &[("instrument", "MIRI"), ("detector", "MIRIMAGE")],
            &["60000.1 60.0", "60000.2 110.0"],
        )
    }

    fn soss_ecsv() -> String {
        ecsv_text(
            &[
                ("MJD_UTC", None),
                ("BJD_TDB", None),
                ("whitelight_flux_order_1", Some("MJy")),
                ("whitelight_flux_order_2", Some("MJy")),
            ],
            &[],
            &["60000.1 60000.102 5.0 7.0"],
        )
    }

    fn bare_ecsv(rows: &[&str]) -> String {
        ecsv_text(&[("MJD_UTC", None), ("BJD_TDB", None), ("whitelight_flux", None)], &[], rows)
    }

    fn nrs1_ecsv(unit: Option<&str>, rows: &[&str]) -> String {
        ecsv_text(&[("MJD_UTC_NRS1", None), ("BJD_TDB_NRS1", None), ("whitelight_flux_NRS1", unit)], &[], rows)
    }

    fn ramp_rows(integrations: i32) -> Vec<SynthRow> {
        (1..=integrations).map(|number| ramp_row(number, 60000.0 + 0.1 * f64::from(number), 5)).collect()
    }

    fn nrs1_set(level: u8) -> TsoSet {
        let spectra = table_of("NRS1", Some(-1), 1, &ramp_rows(3));
        build_set(vec![file_of("s_x1dints.fits", level, Some(3), Some(60000.0), vec![spectra])]).unwrap()
    }

    fn miri_set() -> TsoSet {
        let rows: Vec<SynthRow> = ramp_rows(2)
            .into_iter()
            .map(|row| SynthRow { wavelength: (0..5).map(|channel| 4.0 + 2.0 * f64::from(channel)).collect(), ..row })
            .collect();
        let mut read = file_of("m_x1dints.fits", 3, Some(2), Some(60000.0), vec![table_of("MIRIMAGE", Some(1), 1, &rows)]);
        read.file.exp_type = Some("MIR_LRS-SLITLESS".to_string());
        build_set(vec![read]).unwrap()
    }

    fn soss_set() -> TsoSet {
        let rows = [ramp_row(1, 60000.1, 5)];
        let orders = vec![table_of("NIS", Some(1), 1, &rows), table_of("NIS", Some(2), 2, &rows)];
        let mut read = file_of("n_x1dints.fits", 3, Some(1), Some(60000.0), orders);
        read.file.exp_type = Some("NIS_SOSS".to_string());
        build_set(vec![read]).unwrap()
    }

    fn only_entry(set: &TsoSet, text: &str) -> WhtltComparison {
        let mut entries = compare_whtlt(set, &imported(text)).unwrap();
        assert_eq!(entries.len(), 1);
        entries.remove(0)
    }

    #[test]
    fn a_current_table_gives_one_series_per_flux_column_with_its_own_time_columns() {
        let import = imported(&two_detectors_ecsv());
        assert_eq!(import.schema, WhtltSchema::Current);
        assert_eq!(import.path, LABEL);
        assert_eq!(import.file_name, LABEL);
        assert_eq!(import.meta["instrument"], "NIRSPEC");
        assert_eq!(import.meta["number_of_integrations"], 3);
        assert_eq!(import.series.len(), 2);
        assert_eq!(
            import.series[0],
            WhtltSeries {
                column: "whitelight_flux_NRS1".to_string(),
                detector: Some("NRS1".to_string()),
                sporder: None,
                mjd_utc: vec![60000.1, 60000.2, 60000.3],
                bmjd_tdb: vec![60000.102, 60000.202, 60000.302],
                flux: vec![60.0, 110.0, 160.0],
                flux_unit: Some("Jy".to_string()),
            }
        );
        assert_eq!(
            import.series[1],
            WhtltSeries {
                column: "whitelight_flux_NRS2".to_string(),
                detector: Some("NRS2".to_string()),
                sporder: None,
                mjd_utc: vec![60000.11, 60000.21],
                bmjd_tdb: vec![60000.112, 60000.212],
                flux: vec![6.0, 11.0],
                flux_unit: Some("Jy".to_string()),
            }
        );
        assert_eq!(import.notes, ["1 row(s) without a finite time or flux left out of whitelight_flux_NRS2"]);

        let in_a_folder = whtlt_from_table(&table(&two_detectors_ecsv()), "C:/data/obs/t_whtlt.ecsv").unwrap();
        assert_eq!(in_a_folder.path, "C:/data/obs/t_whtlt.ecsv");
        assert_eq!(in_a_folder.file_name, LABEL);

        let gaps = ["60000.1 nan 60.0", "nan 60000.202 110.0", "60000.3 60000.302 nan", "60000.4 60000.402 210.0"];
        let padded = imported(&nrs1_ecsv(Some("Jy"), &gaps));
        assert_eq!(padded.series[0].mjd_utc, [60000.1, 60000.4]);
        assert_eq!(padded.series[0].flux, [60.0, 210.0]);
        assert!(padded.series[0].bmjd_tdb[0].is_nan());
        assert_eq!(padded.series[0].bmjd_tdb[1], 60000.402);
        assert_eq!(padded.notes, ["2 row(s) without a finite time or flux left out of whitelight_flux_NRS1"]);
    }

    #[test]
    fn a_legacy_table_has_one_utc_time_and_takes_its_detector_from_the_meta() {
        let import = imported(&legacy_ecsv());
        assert_eq!(import.schema, WhtltSchema::Legacy);
        assert_eq!(import.series.len(), 1);
        let series = &import.series[0];
        assert_eq!(series.column, "whitelight_flux");
        assert_eq!(series.detector.as_deref(), Some("MIRIMAGE"));
        assert_eq!(series.sporder, None);
        assert_eq!(series.mjd_utc, [60000.1, 60000.2]);
        assert!(all_nan(&series.bmjd_tdb, 2), "{:?}", series.bmjd_tdb);
        assert_eq!(series.flux, [60.0, 110.0]);
        assert_eq!(series.flux_unit, None);
        assert_eq!(import.notes, NO_TEXT);
        assert_eq!(import.meta, json!({"instrument": "MIRI", "detector": "MIRIMAGE"}));

        let without_meta = imported(&ecsv_text(&[("MJD", None), ("whitelight_flux", None)], &[], &["60000.1 60.0"]));
        assert_eq!(without_meta.schema, WhtltSchema::Legacy);
        assert_eq!(without_meta.series[0].detector, None);
        assert_eq!(without_meta.meta, Value::Null);

        let both_times = ecsv_text(
            &[("MJD", None), ("MJD_UTC", None), ("BJD_TDB", None), ("whitelight_flux", None)],
            &[("detector", "MIRIMAGE")],
            &["1.0 60000.1 60000.102 60.0"],
        );
        let current = imported(&both_times);
        assert_eq!(current.schema, WhtltSchema::Current);
        assert_eq!(current.series[0].mjd_utc, [60000.1]);
        assert_eq!(current.series[0].bmjd_tdb, [60000.102]);
        assert_eq!(current.series[0].detector.as_deref(), Some("MIRIMAGE"));
    }

    #[test]
    fn orders_and_detectors_are_read_from_the_column_names() {
        let soss = imported(&soss_ecsv());
        assert_eq!(soss.schema, WhtltSchema::Current);
        assert_eq!(soss.series.len(), 2);
        for (series, order) in soss.series.iter().zip([1, 2]) {
            assert_eq!(series.column, format!("whitelight_flux_order_{order}"));
            assert_eq!(series.sporder, Some(order));
            assert_eq!(series.detector, None);
            assert_eq!(series.flux_unit.as_deref(), Some("MJy"));
            assert_eq!(series.mjd_utc, [60000.1]);
            assert_eq!(series.bmjd_tdb, [60000.102]);
        }
        assert_eq!(soss.series[0].flux, [5.0]);
        assert_eq!(soss.series[1].flux, [7.0]);
        assert_eq!(soss.notes, NO_TEXT);

        let order_and_detector = ecsv_text(
            &[("MJD_UTC_NRS1", None), ("BJD_TDB_NRS1", None), ("whitelight_flux_order_2_NRS1", Some("Jy"))],
            &[],
            &["60000.1 60000.102 5.0"],
        );
        let both = imported(&order_and_detector);
        assert_eq!(both.series.len(), 1);
        assert_eq!(both.series[0].sporder, Some(2));
        assert_eq!(both.series[0].detector.as_deref(), Some("NRS1"));
        assert_eq!(both.series[0].mjd_utc, [60000.1]);
        assert_eq!(both.series[0].bmjd_tdb, [60000.102]);

        let lookalike = ecsv_text(
            &[("MJD_UTC", None), ("BJD_TDB", None), ("whitelight_flux", None), ("whitelight_fluxes", None)],
            &[],
            &["60000.1 60000.102 5.0 9.0"],
        );
        let one = imported(&lookalike);
        assert_eq!(one.series.len(), 1);
        assert_eq!(one.series[0].column, "whitelight_flux");
        assert_eq!(one.series[0].flux, [5.0]);
        let only_the_lookalike = ecsv_text(&[("MJD_UTC", None), ("whitelight_fluxes", None)], &[], &["60000.1 9.0"]);
        assert_eq!(
            refusal(&only_the_lookalike),
            "t_whtlt.ecsv: no whitelight_flux column: this is not a white-light table (columns: MJD_UTC, whitelight_fluxes)"
        );
    }

    #[test]
    fn tables_that_are_not_white_light_tables_are_refused_by_name() {
        let another_table = ecsv_text(&[("time", None), ("flux", None)], &[], &["60000.1 60.0"]);
        assert_eq!(
            refusal(&another_table),
            "t_whtlt.ecsv: no whitelight_flux column: this is not a white-light table (columns: time, flux)"
        );

        let time_of_no_detector = ecsv_text(
            &[("MJD_UTC", None), ("BJD_TDB", None), ("whitelight_flux_NRS1", Some("Jy"))],
            &[],
            &["60000.1 60000.102 60.0"],
        );
        assert_eq!(
            refusal(&time_of_no_detector),
            "t_whtlt.ecsv: column whitelight_flux_NRS1 has no time column MJD_UTC_NRS1"
        );
        let no_time = ecsv_text(&[("whitelight_flux", None)], &[], &["60.0"]);
        assert_eq!(refusal(&no_time), "t_whtlt.ecsv: column whitelight_flux has no time column MJD_UTC");

        let utc_only = imported(&ecsv_text(&[("MJD_UTC", None), ("whitelight_flux", None)], &[], &["60000.1 60.0", "60000.2 110.0"]));
        assert_eq!(utc_only.schema, WhtltSchema::Current);
        assert_eq!(utc_only.series[0].mjd_utc, [60000.1, 60000.2]);
        assert!(all_nan(&utc_only.series[0].bmjd_tdb, 2), "{:?}", utc_only.series[0].bmjd_tdb);
        assert_eq!(utc_only.notes, ["t_whtlt.ecsv: column BJD_TDB missing: no barycentric time for whitelight_flux"]);

        let utc_and_flux = ecsv_text(&[("MJD_UTC", None), ("whitelight_flux", None)], &[], &["60000.1 60.0"]);
        let text_flux = utc_and_flux.replace("{name: whitelight_flux, datatype: float64}", "{name: whitelight_flux, datatype: string}");
        assert_ne!(text_flux, utc_and_flux);
        assert_eq!(refusal(&text_flux), "column 'whitelight_flux' is text (string), not numeric");
        let text_time = utc_and_flux.replace("{name: MJD_UTC, datatype: float64}", "{name: MJD_UTC, datatype: string}");
        assert_ne!(text_time, utc_and_flux);
        assert_eq!(refusal(&text_time), "column 'MJD_UTC' is text (string), not numeric");
    }

    #[test]
    fn the_comparison_joins_on_time_and_reports_the_largest_relative_difference() {
        let set = nrs1_set(3);
        let g = set.series[0].grid_um.clone();
        let entries = compare_whtlt(&set, &imported(&two_detectors_ecsv())).unwrap();
        assert_eq!(entries.len(), 2);
        let matched = WhtltComparison {
            series: Some(key("NRS1", -1)),
            label: Some("NRS1".to_string()),
            column: "whitelight_flux_NRS1".to_string(),
            n_whtlt: 3,
            n_joined: 3,
            max_rel_diff: 0.0,
            within_1e4: true,
            limits_um: Some([g[0], g[4]]),
            limits_origin: Some(LimitsOrigin::Open),
            notes: Vec::new(),
        };
        assert_eq!(entries[0], matched);
        let unmatched = &entries[1];
        assert_eq!(unmatched.series, None);
        assert_eq!(unmatched.label, None);
        assert_eq!(unmatched.column, "whitelight_flux_NRS2");
        assert_eq!(unmatched.n_whtlt, 2);
        assert_eq!(unmatched.n_joined, 0);
        assert!(unmatched.max_rel_diff.is_nan(), "{}", unmatched.max_rel_diff);
        assert!(!unmatched.within_1e4);
        assert_eq!(unmatched.limits_um, None);
        assert_eq!(unmatched.limits_origin, None);
        assert_eq!(
            unmatched.notes,
            ["column whitelight_flux_NRS2: 0 series of these files match it (detector NRS2, order any); nothing compared"]
        );

        let latest_first = [NRS1_ROWS[2], NRS1_ROWS[0], NRS1_ROWS[1]];
        assert_eq!(only_entry(&set, &nrs1_ecsv(Some("Jy"), &latest_first)), matched);
    }

    #[test]
    fn differences_unpaired_rows_levels_and_units_are_reported() {
        let set = nrs1_set(3);
        let jy = Some("Jy");

        let brighter = only_entry(&set, &nrs1_ecsv(jy, &["60000.1 60000.102 61.0", NRS1_ROWS[1], NRS1_ROWS[2]]));
        assert_eq!(brighter.n_joined, 3);
        assert_eq!(brighter.max_rel_diff, 1.0 / 61.0);
        assert!(!brighter.within_1e4);
        assert_eq!(brighter.notes, NO_TEXT);

        let elsewhere = only_entry(&set, &nrs1_ecsv(jy, &["60000.9 60000.902 60.0", NRS1_ROWS[1], NRS1_ROWS[2]]));
        assert_eq!((elsewhere.n_whtlt, elsewhere.n_joined), (3, 2));
        assert_eq!(elsewhere.max_rel_diff, 0.0);
        assert!(elsewhere.within_1e4);
        assert_eq!(elsewhere.notes, [UNPAIRED_ROW_NOTE]);

        let inside = only_entry(&set, &nrs1_ecsv(jy, &["60000.1000000005 60000.102 60.0", NRS1_ROWS[1], NRS1_ROWS[2]]));
        assert_eq!(inside.n_joined, 3);
        assert_eq!(inside.notes, NO_TEXT);
        let outside = only_entry(&set, &nrs1_ecsv(jy, &["60000.100000002 60000.102 60.0", NRS1_ROWS[1], NRS1_ROWS[2]]));
        assert_eq!(outside.n_joined, 2);
        assert_eq!(outside.notes, [UNPAIRED_ROW_NOTE]);

        let nowhere = only_entry(&set, &nrs1_ecsv(jy, &["60001.1 60001.102 60.0"]));
        assert_eq!((nowhere.n_whtlt, nowhere.n_joined), (1, 0));
        assert!(nowhere.max_rel_diff.is_nan(), "{}", nowhere.max_rel_diff);
        assert!(!nowhere.within_1e4);
        assert_eq!(nowhere.series, Some(key("NRS1", -1)));
        assert_eq!(nowhere.notes, ["1 of 1 whtlt row(s) have no integration within 1e-9 d of their time"]);

        let zero_rows = ["60000.1 60000.102 0.0", NRS1_ROWS[1], NRS1_ROWS[2]];
        let zero = only_entry(&set, &nrs1_ecsv(jy, &zero_rows));
        assert_eq!(zero.n_joined, 3);
        assert_eq!(zero.max_rel_diff, f64::INFINITY);
        assert!(!zero.within_1e4);
        let mut dark = nrs1_set(3);
        for channel in 0..5 {
            let sample = dark.series[0].sample(0, channel);
            dark.series[0].flux[sample] = f64::NAN;
        }
        let both_zero = only_entry(&dark, &nrs1_ecsv(jy, &zero_rows));
        assert_eq!(both_zero.n_joined, 3);
        assert_eq!(both_zero.max_rel_diff, 0.0);
        assert!(both_zero.within_1e4);

        let segments = nrs1_set(2);
        let level_2b = only_entry(&segments, &nrs1_ecsv(jy, &NRS1_ROWS));
        assert_eq!(level_2b.max_rel_diff, 0.0);
        assert_eq!(level_2b.notes, [LEVEL_2B_NOTE]);

        let mut counts = nrs1_set(3);
        counts.series[0].flux_unit = "DN/s".to_string();
        assert_eq!(only_entry(&counts, &nrs1_ecsv(Some("DN / s"), &NRS1_ROWS)).notes, NO_TEXT);
        assert_eq!(only_entry(&set, &nrs1_ecsv(Some("MJy"), &NRS1_ROWS)).notes, [MJY_AGAINST_JY_NOTE]);
        assert_eq!(only_entry(&set, &nrs1_ecsv(None, &NRS1_ROWS)).notes, NO_TEXT);
        let mut unlabelled = nrs1_set(3);
        unlabelled.series[0].flux_unit = "unknown".to_string();
        assert_eq!(only_entry(&unlabelled, &nrs1_ecsv(Some("MJy"), &NRS1_ROWS)).notes, NO_TEXT);

        let everything = only_entry(&segments, &nrs1_ecsv(Some("MJy"), &["60000.9 60000.902 60.0", NRS1_ROWS[1], NRS1_ROWS[2]]));
        assert_eq!(everything.notes, [UNPAIRED_ROW_NOTE, LEVEL_2B_NOTE, MJY_AGAINST_JY_NOTE]);
    }

    #[test]
    fn a_column_without_detector_or_order_matches_the_only_series_and_is_ambiguous_among_several() {
        let legacy = only_entry(&miri_set(), &legacy_ecsv());
        assert_eq!(
            legacy,
            WhtltComparison {
                series: Some(key("MIRIMAGE", 1)),
                label: Some("MIRIMAGE".to_string()),
                column: "whitelight_flux".to_string(),
                n_whtlt: 2,
                n_joined: 2,
                max_rel_diff: 20.0 / 110.0,
                within_1e4: false,
                limits_um: Some([5.0, 12.0]),
                limits_origin: Some(LimitsOrigin::BuiltInDefault),
                notes: Vec::new(),
            }
        );

        let orders = soss_set();
        let entries = compare_whtlt(&orders, &imported(&soss_ecsv())).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].series, Some(key("NIS", 1)));
        assert_eq!(entries[0].label.as_deref(), Some("NIS, order 1"));
        assert_eq!(entries[0].column, "whitelight_flux_order_1");
        assert_eq!(entries[0].max_rel_diff, 55.0 / 5.0);
        assert_eq!(entries[0].notes, [MJY_AGAINST_JY_NOTE]);
        assert_eq!(entries[1].series, Some(key("NIS", 2)));
        assert_eq!(entries[1].label.as_deref(), Some("NIS, order 2"));
        assert_eq!(entries[1].column, "whitelight_flux_order_2");
        assert_eq!(entries[1].max_rel_diff, 53.0 / 7.0);
        assert_eq!(entries[1].notes, [MJY_AGAINST_JY_NOTE, SOSS_ORDER_2_WARNING]);
        for entry in &entries {
            assert_eq!((entry.n_whtlt, entry.n_joined), (1, 1));
            assert_eq!(entry.limits_um, Some([0.93, 2.82]));
            assert_eq!(entry.limits_origin, Some(LimitsOrigin::BuiltInDefault));
        }

        let ambiguous = only_entry(&orders, &bare_ecsv(&["60000.1 60000.102 5.0"]));
        assert_eq!(ambiguous.series, None);
        assert_eq!(ambiguous.n_whtlt, 1);
        assert_eq!(ambiguous.n_joined, 0);
        assert_eq!(
            ambiguous.notes,
            ["column whitelight_flux: 2 series of these files match it (detector any, order any); nothing compared"]
        );

        let lower_case = ecsv_text(
            &[("MJD_UTC_nrs1", None), ("BJD_TDB_nrs1", None), ("whitelight_flux_nrs1", Some("Jy"))],
            &[],
            &NRS1_ROWS,
        );
        let any_case = only_entry(&nrs1_set(3), &lower_case);
        assert_eq!(any_case.series, Some(key("NRS1", -1)));
        assert_eq!(any_case.n_joined, 3);

        let another_order = ecsv_text(
            &[("MJD_UTC_NRS1", None), ("BJD_TDB_NRS1", None), ("whitelight_flux_order_2_NRS1", Some("Jy"))],
            &[],
            &NRS1_ROWS,
        );
        let no_such_order = only_entry(&nrs1_set(3), &another_order);
        assert_eq!(no_such_order.series, None);
        assert_eq!(
            no_such_order.notes,
            ["column whitelight_flux_order_2_NRS1: 0 series of these files match it (detector NRS1, order 2); nothing compared"]
        );
    }

    #[test]
    fn the_import_and_the_comparison_serialise_with_the_names_the_frontend_reads() {
        let legacy = serde_json::to_value(imported(&legacy_ecsv())).unwrap();
        assert_eq!(sorted_keys(&legacy), ["file_name", "meta", "notes", "path", "schema", "series"]);
        assert_eq!(legacy["path"], LABEL);
        assert_eq!(legacy["file_name"], LABEL);
        assert_eq!(legacy["schema"], "legacy");
        assert_eq!(legacy["notes"], json!([]));
        assert_eq!(legacy["meta"], json!({"instrument": "MIRI", "detector": "MIRIMAGE"}));
        assert_eq!(
            sorted_keys(&legacy["series"][0]),
            ["bmjd_tdb", "column", "detector", "flux", "flux_unit", "mjd_utc", "sporder"]
        );
        assert_eq!(
            legacy["series"],
            json!([{
                "column": "whitelight_flux",
                "detector": "MIRIMAGE",
                "sporder": null,
                "mjd_utc": [60000.1, 60000.2],
                "bmjd_tdb": [null, null],
                "flux": [60.0, 110.0],
                "flux_unit": null,
            }])
        );
        assert_eq!(serde_json::to_value(WhtltSchema::Current).unwrap(), json!("current"));
        let soss = serde_json::to_value(imported(&soss_ecsv())).unwrap();
        assert_eq!(soss["meta"], Value::Null);
        assert_eq!(soss["series"][1]["sporder"], 2);
        assert_eq!(soss["series"][1]["flux_unit"], "MJy");
        assert_eq!(soss["series"][1]["bmjd_tdb"], json!([60000.102]));

        let set = nrs1_set(3);
        let entries = serde_json::to_value(compare_whtlt(&set, &imported(&two_detectors_ecsv())).unwrap()).unwrap();
        assert_eq!(
            entries,
            json!([
                {
                    "series": {"detector": "NRS1", "sporder": -1},
                    "label": "NRS1",
                    "column": "whitelight_flux_NRS1",
                    "n_whtlt": 3,
                    "n_joined": 3,
                    "max_rel_diff": 0.0,
                    "within_1e4": true,
                    "limits_um": [1.0, 1.0 + 0.1 * 4.0],
                    "limits_origin": "open",
                    "notes": [],
                },
                {
                    "series": null,
                    "label": null,
                    "column": "whitelight_flux_NRS2",
                    "n_whtlt": 2,
                    "n_joined": 0,
                    "max_rel_diff": null,
                    "within_1e4": false,
                    "limits_um": null,
                    "limits_origin": null,
                    "notes": ["column whitelight_flux_NRS2: 0 series of these files match it (detector NRS2, order any); nothing compared"],
                },
            ])
        );
        let zero = only_entry(&set, &nrs1_ecsv(Some("Jy"), &["60000.1 60000.102 0.0"]));
        assert_eq!(serde_json::to_value(zero).unwrap()["max_rel_diff"], Value::Null);
        let limits = serde_json::to_value(only_entry(&miri_set(), &legacy_ecsv())).unwrap();
        assert_eq!(limits["limits_origin"], "built-in default");
        assert_eq!(limits["limits_um"], json!([5.0, 12.0]));
    }

    type GoldenSeries = (&'static str, Option<&'static str>, Option<i64>, Option<&'static str>, usize, f64, f64, f64, f64);
    type MatchedColumn = (&'static str, &'static str, i64, usize, [f64; 2], LimitsOrigin);

    const SCIENCE_DATA_ENV: &str = "ASTROBURST_SCIENCE_DATA_DIR";
    const WHTLT_ORACLE: &str = "python -I plan/pacotes/TSO-support/tso_checks.py whtlt <data root>/tso";
    const SUM_TOLERANCE: f64 = 1e-12;
    const LEVEL_2B_TOLERANCE: f64 = 1e-6;
    const JY: Option<&str> = Some("Jy");
    const MJY: Option<&str> = Some("MJy");
    const DN_PER_S: Option<&str> = Some("DN / s");
    const NRS1: Option<&str> = Some("NRS1");
    const NRS2: Option<&str> = Some("NRS2");
    const MIRIMAGE: Option<&str> = Some("MIRIMAGE");
    const A1_STEM: &str = "jw02459-o013_t007_nircam_f322w2-grismr-subgrism64";
    const A5_STEM: &str = "jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048";
    const C1_STEM: &str = "jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048";
    const E1_STEM: &str = "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048";
    const WHTLT_GOLDENS: [(&str, WhtltSchema, &[GoldenSeries]); 16] = [
        (
            A1_STEM,
            WhtltSchema::Current,
            &[("whitelight_flux", None, None, DN_PER_S, 1, 59829.989846412034, 59829.99303618102, 65197945.38277817, 65197945.38277817)],
        ),
        (
            "jw07073-o004_t001_niriss_clear-gr700xd-substrip256",
            WhtltSchema::Current,
            &[
                ("whitelight_flux_order_1", None, Some(1), MJY, 1, 61043.21309070602, 61043.21592608525, 1503.7119146038247, 1503.7119146038247),
                ("whitelight_flux_order_2", None, Some(2), MJY, 1, 61043.21309070602, 61043.21592608525, 14055.279010227225, 14055.279010227225),
                ("whitelight_flux_order_3", None, Some(3), MJY, 1, 61043.21309070602, 61043.21592608525, 37.554793231367526, 37.554793231367526),
            ],
        ),
        (
            "jw04498-o013_t008_miri_p750l-slitlessprism",
            WhtltSchema::Current,
            &[("whitelight_flux", None, None, JY, 64, 60463.27446656057, 60463.278435296284, 105.1244014781455, 7420.285204061691)],
        ),
        (
            A5_STEM,
            WhtltSchema::Current,
            &[
                ("whitelight_flux_NRS1", NRS1, None, JY, 8, 60496.52120635712, 60496.522504382556, -5953.629162915719, -48889.218080477076),
                ("whitelight_flux_NRS2", NRS2, None, JY, 8, 60496.52110465069, 60496.52240268599, 3.7127694054398646, 29.66909685181698),
            ],
        ),
        (
            "jw04967-o005_t004_nirspec_clear-prism-s1600a1-sub512",
            WhtltSchema::Current,
            &[("whitelight_flux_NRS1", NRS1, None, JY, 65, 60796.95485544182, 60796.95842671788, 0.016091166200930186, 1.0538909536835777)],
        ),
        (
            "jw06828-o003_t001_nirspec_clear-prism-s1600a1-sub512",
            WhtltSchema::Current,
            &[("whitelight_flux_NRS1", NRS1, None, None, 1, 60599.140564409725, 60599.14203399092, 2.988304267333806, 2.988304267333806)],
        ),
        (
            "jw01274-o013_t008_miri_p750l-slitlessprism",
            WhtltSchema::Legacy,
            &[("whitelight_flux", MIRIMAGE, None, None, 10, 60126.800615279506, f64::NAN, 2.163616608616668, 23.347534963816415)],
        ),
        (
            "jw02965-o002_t001_miri_p750l-slitlessprism",
            WhtltSchema::Legacy,
            &[("whitelight_flux", MIRIMAGE, None, None, 10, 60133.89834643888, f64::NAN, 2.2626413563627708, 23.27636993162461)],
        ),
        (
            C1_STEM,
            WhtltSchema::Current,
            &[("whitelight_flux_NRS1", NRS1, None, JY, 44, 60280.05386059504, 60280.05657544178, -219.0388902270913, -8701.993488389966)],
        ),
        (
            "jw07849-o002_t002_miri_p750l-slitlessprism",
            WhtltSchema::Current,
            &[("whitelight_flux", None, None, JY, 100, 61207.50402118654, 61207.50626390643, 0.0727906475348244, 16.080810437960725)],
        ),
        (
            E1_STEM,
            WhtltSchema::Current,
            &[("whitelight_flux_NRS1", NRS1, None, JY, 240, 60061.44133176829, 60061.4421733457, 0.16369951048163586, 37.761493394107156)],
        ),
        (
            "jw01366-o002_t001_nircam_f322w2-grismr-subgrism256",
            WhtltSchema::Current,
            &[("whitelight_flux", None, None, DN_PER_S, 366, 59782.80985660909, 59782.811359196894, 709027.0040717721, 258151412.92282438)],
        ),
        (
            "jw01366-o003_t001_nirspec_f290lp-g395h-s1600a1-sub2048",
            WhtltSchema::Current,
            &[
                ("whitelight_flux_NRS1", NRS1, None, JY, 465, 59790.91978428565, 59790.920506999464, -4420.80089171448, -2359131.007886567),
                ("whitelight_flux_NRS2", NRS2, None, JY, 465, 59790.919780242635, 59790.92050295683, 32.41469297700512, 14981.129925355544),
            ],
        ),
        (
            "jw01366-o001_t001_niriss_clear-gr700xd-substrip256",
            WhtltSchema::Current,
            &[
                ("whitelight_flux_order_1", None, Some(1), MJY, 537, 59786.86524502978, 59786.866358341525, 124.48996614090491, 66482.16624101387),
                ("whitelight_flux_order_2", None, Some(2), MJY, 537, 59786.86524502978, 59786.866358341525, 211.59072077224457, 112855.14104485876),
                ("whitelight_flux_order_3", None, Some(3), MJY, 537, 59786.86524502978, 59786.866358341525, 4.210519392499863, 2254.123068953902),
            ],
        ),
        (
            "jw01201-o103_t012_niriss_clear-gr700xd-substrip256",
            WhtltSchema::Current,
            &[
                ("whitelight_flux_order_1", None, Some(1), MJY, 121, 60147.41328937095, 60147.418097181064, 78.29289877934431, 9474.363690825465),
                ("whitelight_flux_order_2", None, Some(2), MJY, 121, 60147.41328937095, 60147.418097181064, 169.93709557642907, 20557.021117534627),
                ("whitelight_flux_order_3", None, Some(3), MJY, 121, 60147.41328937095, 60147.418097181064, 1.079644070946319, 128.93039747958858),
            ],
        ),
        (
            "jw01538-o067_t013_niriss_clear-gr700xd-substrip96",
            WhtltSchema::Current,
            &[
                ("whitelight_flux_order_1", None, Some(1), MJY, 1, 59805.322598425926, 59805.32648102644, 1644.594783923854, 1644.594783923854),
                ("whitelight_flux_order_2", None, Some(2), MJY, 1, 59805.322598425926, 59805.32648102644, 1525.0404687929974, 1525.0404687929974),
            ],
        ),
    ];
    const LEVEL_3_PAIRS: [(&str, &[MatchedColumn]); 8] = [
        (
            "A1/jw02459-o013_t007_nircam_f322w2-grismr-subgrism64_x1dints.fits",
            &[("whitelight_flux", "NRCALONG", 1, 1, [2.47, 3.94], LimitsOrigin::FileLog)],
        ),
        (
            "A3/jw07073-o004_t001_niriss_clear-gr700xd-substrip256_x1dints.fits",
            &[
                ("whitelight_flux_order_1", "NIS", 1, 1, [0.93, 2.82], LimitsOrigin::BuiltInDefault),
                ("whitelight_flux_order_2", "NIS", 2, 1, [0.93, 2.82], LimitsOrigin::BuiltInDefault),
                ("whitelight_flux_order_3", "NIS", 3, 1, [0.93, 2.82], LimitsOrigin::BuiltInDefault),
            ],
        ),
        (
            "A4/jw04498-o013_t008_miri_p750l-slitlessprism_x1dints.fits",
            &[("whitelight_flux", "MIRIMAGE", 1, 64, [5.0, 12.0], LimitsOrigin::FileLog)],
        ),
        (
            "A5/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits",
            &[
                ("whitelight_flux_NRS1", "NRS1", -1, 8, [2.8617736352602416, 3.7204518540407396], LimitsOrigin::Open),
                ("whitelight_flux_NRS2", "NRS2", -1, 8, [3.8194005627692933, 5.177174531991892], LimitsOrigin::Open),
            ],
        ),
        (
            "A6/jw04967-o005_t004_nirspec_clear-prism-s1600a1-sub512_x1dints.fits",
            &[("whitelight_flux_NRS1", "NRS1", 0, 65, [0.5524153961529221, 5.368596776621393], LimitsOrigin::Open)],
        ),
        (
            "B1/jw01274-o013_t008_miri_p750l-slitlessprism_x1dints.fits",
            &[("whitelight_flux", "MIRIMAGE", 1, 10, [5.0, 12.0], LimitsOrigin::BuiltInDefault)],
        ),
        (
            "C/jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits",
            &[("whitelight_flux_NRS1", "NRS1", -1, 44, [2.848429652675818, 5.182052338975267], LimitsOrigin::Open)],
        ),
        (
            "E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits",
            &[("whitelight_flux_NRS1", "NRS1", 0, 240, [0.5530469695008425, 5.370352185327354], LimitsOrigin::Open)],
        ),
    ];

    fn tso_path(relative: &str) -> String {
        let path = env_dir(SCIENCE_DATA_ENV).join("tso").join(relative);
        assert!(path.is_file(), "{} is missing", path.display());
        path.to_string_lossy().into_owned()
    }

    fn real_whtlt(stem: &str) -> WhtltImport {
        import_whtlt(&tso_path(&format!("whtlt/{stem}_whtlt.ecsv"))).unwrap()
    }

    fn relative_gap(measured: f64, expected: f64) -> f64 {
        ((measured - expected) / expected).abs()
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/whtlt and tso/<sets>"]
    fn real_whtlt_tables_import_with_the_schema_columns_and_values_of_the_goldens() {
        for (stem, schema, expected) in WHTLT_GOLDENS {
            let golden = format!("series[] of plan/goldens/tso/whtlt_{stem}.json");
            let import = real_whtlt(stem);
            assert_eq!(import.schema, schema, "{golden}");
            assert_eq!(import.file_name, format!("{stem}_whtlt.ecsv"), "{golden}");
            assert_eq!(import.notes, NO_TEXT, "rows left out or columns missing, {golden}");
            assert_eq!(import.series.len(), expected.len(), "flux columns, {golden}");
            let found = import.series.iter().zip(expected);
            for (series, &(column, detector, sporder, flux_unit, rows, mjd_utc, bmjd_tdb, flux, flux_sum)) in found {
                assert_eq!(series.column, column, "flux_column, {golden}");
                assert_eq!(series.detector.as_deref(), detector, "detector of {column}, {golden}");
                assert_eq!(series.sporder, sporder, "order_suffix of {column}, {golden}");
                assert_eq!(series.flux_unit.as_deref(), flux_unit, "flux_unit of {column}, {golden}");
                assert_eq!(series.mjd_utc.len(), rows, "rows_with_finite_time of {column}, {golden}");
                assert_eq!(series.bmjd_tdb.len(), rows, "barycentric times of {column}, {golden}");
                assert_eq!(series.flux.len(), rows, "fluxes of {column}, {golden}");
                assert_eq!(series.mjd_utc[0], mjd_utc, "time_first of {column} in MJD (UTC) days, {golden}");
                if bmjd_tdb.is_nan() {
                    assert!(all_nan(&series.bmjd_tdb, rows), "a legacy table has no barycentric time, {golden}");
                } else {
                    assert_eq!(series.bmjd_tdb[0], bmjd_tdb, "bjd_first of {column} in BMJD (TDB) days, {golden}");
                    assert!(series.bmjd_tdb.iter().all(|time| time.is_finite()), "barycentric times of {column}, {golden}");
                }
                assert_eq!(series.flux[0], flux, "flux_first3[0] of {column}, {golden}");
                let sum: f64 = series.flux.iter().sum();
                let gap = relative_gap(sum, flux_sum);
                println!("{stem} {column}: {rows} row(s), sum {sum:?}, flux_sum_fsum {flux_sum:?}, relative difference {gap:.1e}");
                assert!(gap <= SUM_TOLERANCE, "flux_sum_fsum of {column}: measured {sum:?}, expected {flux_sum:?}, {golden}");
            }
        }
        assert_eq!(real_whtlt(E1_STEM).meta["target_name"], "WD 1856+534 b", "meta of plan/goldens/tso/whtlt_{E1_STEM}.json");
        assert_eq!(real_whtlt(A1_STEM).meta["number_of_integrations"], 1, "meta of plan/goldens/tso/whtlt_{A1_STEM}.json");
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/whtlt and tso/<sets>"]
    fn real_level_3_products_reproduce_their_archived_white_light() {
        let oracle = WHTLT_ORACLE;
        let mut largest = 0.0f64;
        for (relative, expected) in LEVEL_3_PAIRS {
            let stem = relative.rsplit('/').next().unwrap().trim_end_matches("_x1dints.fits");
            let set = read_set(&[tso_path(relative)]).unwrap();
            let entries = compare_whtlt(&set, &real_whtlt(stem)).unwrap();
            assert_eq!(entries.len(), expected.len(), "columns of {stem}, {oracle}");
            for (entry, &(column, detector, sporder, rows, limits_um, limits_origin)) in entries.iter().zip(expected) {
                println!(
                    "{relative} {column}: {} of {} row(s) joined, largest relative difference {:e}, limits {:?} um ({:?})",
                    entry.n_joined, entry.n_whtlt, entry.max_rel_diff, entry.limits_um, entry.limits_origin
                );
                assert_eq!(entry.column, column, "{stem}, {oracle}");
                assert_eq!(entry.series, Some(key(detector, sporder)), "series of {column}, {stem}, {oracle}");
                assert_eq!(entry.n_whtlt, rows, "rows of {column}, {stem}, {oracle}");
                assert_eq!(entry.n_joined, rows, "joined rows of {column}, {stem}, {oracle}");
                assert!(entry.within_1e4, "{column} of {stem} differs by {:e}, {oracle}", entry.max_rel_diff);
                assert!(entry.max_rel_diff <= SUM_TOLERANCE, "{column} of {stem} differs by {:e}, {oracle}", entry.max_rel_diff);
                assert_eq!(entry.limits_um, Some(limits_um), "limits of {column} in um, {stem}, {oracle}");
                assert_eq!(entry.limits_origin, Some(limits_origin), "origin of the limits of {column}, {stem}, {oracle}");
                let reused_range = match (detector, sporder) {
                    ("NIS", 2) => Some(SOSS_ORDER_2_WARNING),
                    ("NIS", 3) => Some(SOSS_ORDER_3_WARNING),
                    _ => None,
                };
                if let Some(warning) = reused_range {
                    assert!(entry.notes.iter().any(|note| note == warning), "order 1 range note of {column}: {:?}", entry.notes);
                }
                let unexpected = |note: &&String| {
                    note.contains("have no integration within") || note.as_str() == LEVEL_2B_NOTE || note.starts_with("the whtlt column is in")
                };
                assert_eq!(entry.notes.iter().find(unexpected), None, "{column} of {stem}, {oracle}");
                largest = largest.max(entry.max_rel_diff);
            }
        }
        println!("largest relative difference over the eight level 3 products: {largest:e}");
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/whtlt and tso/<sets>"]
    fn real_level_2b_segments_join_on_time_and_differ_from_the_level_3_white_light() {
        let oracle = format!("{WHTLT_ORACLE}; plan/data-facts-tso.md 5.5");
        let detectors = [
            tso_path("A5/jw03375001001_03101_00001-seg001_nrs1_x1dints.fits"),
            tso_path("A5/jw03375001001_03101_00001-seg001_nrs2_x1dints.fits"),
        ];
        let entries = compare_whtlt(&read_set(&detectors).unwrap(), &real_whtlt(A5_STEM)).unwrap();
        let expected = [("NRS1", 7.38980487916417e-07, true), ("NRS2", 0.0011169888054512525, false)];
        assert_eq!(entries.len(), expected.len(), "{oracle}");
        for (entry, (detector, max_rel_diff, within_1e4)) in entries.iter().zip(expected) {
            let gap = relative_gap(entry.max_rel_diff, max_rel_diff);
            println!(
                "A5 level 2b {detector}: {} of {} row(s) joined, largest relative difference {:e}, oracle {max_rel_diff:e}, relative gap {gap:.1e}",
                entry.n_joined, entry.n_whtlt, entry.max_rel_diff
            );
            assert_eq!(entry.series, Some(key(detector, -1)), "{oracle}");
            assert_eq!((entry.n_whtlt, entry.n_joined), (8, 8), "rows of {detector} joined on time, {oracle}");
            assert!(gap <= LEVEL_2B_TOLERANCE, "{detector}: measured {:e}, oracle {max_rel_diff:e}, {oracle}", entry.max_rel_diff);
            assert_eq!(entry.within_1e4, within_1e4, "{detector}, {oracle}");
            assert!(entry.notes.iter().any(|note| note == LEVEL_2B_NOTE), "{detector}: {:?}", entry.notes);
        }

        let segments: Vec<String> = (1..=6)
            .map(|number| tso_path(&format!("C/jw02327001001_03101_00001-seg{number:03}_nrs1_x1dints.fits")))
            .collect();
        let entries = compare_whtlt(&read_set(&segments).unwrap(), &real_whtlt(C1_STEM)).unwrap();
        assert_eq!(entries.len(), 1, "{oracle}");
        let entry = &entries[0];
        let gap = relative_gap(entry.max_rel_diff, 723.837953015402);
        println!(
            "C level 2b NRS1: {} of {} row(s) joined, largest relative difference {:e}, oracle 7.23837953015402e2, relative gap {gap:.1e}",
            entry.n_joined, entry.n_whtlt, entry.max_rel_diff
        );
        assert_eq!(entry.series, Some(key("NRS1", -1)), "{oracle}");
        assert_eq!((entry.n_whtlt, entry.n_joined), (44, 44), "rows joined on time, {oracle}");
        assert!(gap <= LEVEL_2B_TOLERANCE, "measured {:e}, oracle 723.837953015402, {oracle}", entry.max_rel_diff);
        assert!(!entry.within_1e4, "{oracle}");
        assert!(entry.notes.iter().any(|note| note == LEVEL_2B_NOTE), "{:?}", entry.notes);
    }
}
