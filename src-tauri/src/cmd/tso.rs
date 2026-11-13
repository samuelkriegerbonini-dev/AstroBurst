use serde::Serialize;

use crate::cmd::common::blocking_cmd;
use crate::core::analysis::tso::cache::set_id;
use crate::core::analysis::tso::white_light::{light_curves, CurveRequest, LightCurves};
use crate::core::analysis::tso::whtlt::{compare_whtlt, import_whtlt};
use crate::core::analysis::tso::{
    find_series, open_tso, series_label, series_spectrum, ExposureCompleteness, SeriesKey, TsoFile, TsoPick, TsoSeries,
    TsoSet, TsoTimeScale,
};

pub const EMPTY_PATHS_ERROR: &str = "paths must name at least one x1dints file";
pub const EMPTY_WHTLT_PATH_ERROR: &str = "path must name a whtlt table, got an empty string";

#[derive(Debug, Clone, Serialize)]
pub struct TsoSeriesSummary {
    pub key: SeriesKey,
    pub label: String,
    pub n_integrations: usize,
    pub n_channels: usize,
    pub flux_unit: String,
    pub time_scale: TsoTimeScale,
    pub time_source: String,
    pub time_first: f64,
    pub time_last: f64,
    pub wavelength_min_um: f64,
    pub wavelength_max_um: f64,
    pub common_grid: bool,
    pub outlier_detection: Option<String>,
    pub completeness: Vec<ExposureCompleteness>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TsoOpenResult {
    pub set_id: String,
    pub files: Vec<TsoFile>,
    pub series: Vec<TsoSeriesSummary>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
struct SeriesLightCurves {
    set_id: String,
    label: String,
    #[serde(flatten)]
    curves: LightCurves,
}

fn checked_paths(paths: &[String]) -> Result<(), String> {
    if paths.is_empty() || paths.iter().any(|path| path.trim().is_empty()) {
        return Err(EMPTY_PATHS_ERROR.to_string());
    }
    Ok(())
}

fn series_summary(series: &TsoSeries, keys: &[SeriesKey]) -> TsoSeriesSummary {
    let time = series.time();
    let finite_grid = || series.grid_um.iter().copied().filter(|wavelength| wavelength.is_finite());
    TsoSeriesSummary {
        key: series.key.clone(),
        label: series_label(&series.key, keys),
        n_integrations: series.n_integrations,
        n_channels: series.n_channels,
        flux_unit: series.flux_unit.clone(),
        time_scale: series.time_scale,
        time_source: series.time_source.clone(),
        time_first: time.first().copied().unwrap_or(f64::NAN),
        time_last: time.last().copied().unwrap_or(f64::NAN),
        wavelength_min_um: finite_grid().reduce(f64::min).unwrap_or(f64::NAN),
        wavelength_max_um: finite_grid().reduce(f64::max).unwrap_or(f64::NAN),
        common_grid: series.common_grid,
        outlier_detection: series.outlier_detection.clone(),
        completeness: series.completeness.clone(),
        notes: series.notes.clone(),
        warnings: series.warnings.clone(),
    }
}

fn told_once(sentences: impl Iterator<Item = String>) -> Vec<String> {
    let mut told: Vec<String> = Vec::new();
    for sentence in sentences {
        if !told.contains(&sentence) {
            told.push(sentence);
        }
    }
    told
}

fn open_result(set: &TsoSet) -> TsoOpenResult {
    let keys: Vec<SeriesKey> = set.series.iter().map(|series| series.key.clone()).collect();
    let series: Vec<TsoSeriesSummary> = set.series.iter().map(|one| series_summary(one, &keys)).collect();
    let several = series.len() > 1;
    let of_its_series = |summary: &TsoSeriesSummary, sentence: &String| {
        if several {
            format!("{}: {sentence}", summary.label)
        } else {
            sentence.clone()
        }
    };
    let series_notes = series.iter().flat_map(|summary| summary.notes.iter().map(move |note| of_its_series(summary, note)));
    let series_warnings =
        series.iter().flat_map(|summary| summary.warnings.iter().map(move |warning| of_its_series(summary, warning)));
    TsoOpenResult {
        set_id: set_id(set),
        files: set.files.clone(),
        notes: told_once(set.notes.iter().cloned().chain(series_notes)),
        warnings: told_once(set.warnings.iter().cloned().chain(series_warnings)),
        series,
    }
}

pub(crate) fn tso_open_json(paths: &[String]) -> anyhow::Result<serde_json::Value> {
    let set = open_tso(paths)?;
    Ok(serde_json::to_value(open_result(&set))?)
}

pub(crate) fn tso_spectrum_json(paths: &[String], series: &SeriesKey, pick: &TsoPick) -> anyhow::Result<serde_json::Value> {
    let set = open_tso(paths)?;
    Ok(serde_json::to_value(series_spectrum(&set, series, pick)?)?)
}

pub(crate) fn tso_light_curves_json(paths: &[String], series: &SeriesKey, request: &CurveRequest) -> anyhow::Result<serde_json::Value> {
    let set = open_tso(paths)?;
    let found = find_series(&set, series)?;
    let keys: Vec<SeriesKey> = set.series.iter().map(|one| one.key.clone()).collect();
    Ok(serde_json::to_value(SeriesLightCurves {
        set_id: set_id(&set),
        label: series_label(series, &keys),
        curves: light_curves(&set, found, request)?,
    })?)
}

#[tauri::command]
pub async fn tso_open_cmd(paths: Vec<String>) -> Result<serde_json::Value, String> {
    checked_paths(&paths)?;
    blocking_cmd!(tso_open_json(&paths))
}

#[tauri::command]
pub async fn tso_spectrum_cmd(paths: Vec<String>, series: SeriesKey, pick: TsoPick) -> Result<serde_json::Value, String> {
    checked_paths(&paths)?;
    blocking_cmd!(tso_spectrum_json(&paths, &series, &pick))
}

#[tauri::command]
pub async fn tso_light_curves_cmd(
    paths: Vec<String>,
    series: SeriesKey,
    request: CurveRequest,
) -> Result<serde_json::Value, String> {
    checked_paths(&paths)?;
    blocking_cmd!(tso_light_curves_json(&paths, &series, &request))
}

fn checked_whtlt_path(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err(EMPTY_WHTLT_PATH_ERROR.to_string());
    }
    Ok(())
}

pub(crate) fn whtlt_import_json(path: &str) -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::to_value(import_whtlt(path)?)?)
}

pub(crate) fn tso_whtlt_compare_json(paths: &[String], whtlt_path: &str) -> anyhow::Result<serde_json::Value> {
    let set = open_tso(paths)?;
    let whtlt = import_whtlt(whtlt_path)?;
    Ok(serde_json::to_value(compare_whtlt(&set, &whtlt)?)?)
}

#[tauri::command]
pub async fn whtlt_import_cmd(path: String) -> Result<serde_json::Value, String> {
    checked_whtlt_path(&path)?;
    blocking_cmd!(whtlt_import_json(&path))
}

#[tauri::command]
pub async fn tso_whtlt_compare_cmd(paths: Vec<String>, whtlt_path: String) -> Result<serde_json::Value, String> {
    checked_paths(&paths)?;
    checked_whtlt_path(&whtlt_path)?;
    blocking_cmd!(tso_whtlt_compare_json(&paths, &whtlt_path))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Instant;

    use serde_json::{json, Value};

    use super::*;
    use crate::core::analysis::tso::series::test_support::ramp_set;
    use crate::core::analysis::tso::white_light::CurveMode;
    use crate::core::analysis::tso::whtlt::test_support::two_detectors_ecsv;
    use crate::core::analysis::tso::DATAMODEL_ROWS;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::infra::fits::gzip::test_support::gzip_copy;
    use crate::infra::fits::x1dints::test_fixtures::{ramp_row, rows_hdu, series_primary, write_series_file, SynthRow, SynthTable};

    const DOCUMENT_KEYS: [&str; 13] = [
        "axis",
        "cards",
        "dq",
        "dq_table",
        "err",
        "flux",
        "flux_unit",
        "n_rows",
        "notes",
        "precision",
        "source",
        "wavelength_frame",
        "wavelength_unit_source",
    ];
    const OPEN_KEYS: [&str; 5] = ["files", "notes", "series", "set_id", "warnings"];
    const SERIES_KEYS: [&str; 16] = [
        "common_grid",
        "completeness",
        "flux_unit",
        "key",
        "label",
        "n_channels",
        "n_integrations",
        "notes",
        "outlier_detection",
        "time_first",
        "time_last",
        "time_scale",
        "time_source",
        "warnings",
        "wavelength_max_um",
        "wavelength_min_um",
    ];
    const FILE_KEYS: [&str; 26] = [
        "cal_ver",
        "data_bytes",
        "datamodel",
        "detector",
        "exp_type",
        "expstart",
        "exsegnum",
        "exsegtot",
        "file_name",
        "filter",
        "grating",
        "instrument",
        "intend",
        "intstart",
        "layout",
        "level",
        "n_extract1d",
        "nints",
        "outlier_detection",
        "path",
        "pupil",
        "sha256",
        "subarray",
        "target",
        "whitelight_log",
        "whtlt_path",
    ];
    const NO_USABLE_TIMES: &str = "no usable integration times: TDB-MID repeats values and MJD-AVG is missing; the abscissa is the integration number";

    fn sorted_keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys
    }

    fn texts(value: &Value) -> Vec<&str> {
        value.as_array().unwrap().iter().map(|text| text.as_str().unwrap()).collect()
    }

    fn primary(extra: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        let mut cards = vec![
            ("NINTS", "3".to_string()),
            ("EXPSTART", "60000.0".to_string()),
            ("EXP_TYPE", "'NRS_BRIGHTOBJ'".to_string()),
            ("DETECTOR", "'NRS1'".to_string()),
        ];
        cards.extend(extra.iter().map(|(card, value)| (*card, value.to_string())));
        series_primary(DATAMODEL_ROWS, &cards)
    }

    fn table(extver: i64, detector: &'static str, rows: Vec<SynthRow>) -> SynthTable {
        SynthTable { extver, detector: Some(detector), sporder: Some(-1), flux_unit: Some("Jy"), rows }
    }

    fn three_integrations() -> Vec<SynthRow> {
        (1..=3).map(|number| ramp_row(number, 60000.0 + 0.1 * f64::from(number), 4)).collect()
    }

    fn untimed_integrations() -> Vec<SynthRow> {
        (1..=3).map(|number| SynthRow { mjd_avg: f64::NAN, tdb_mid: 60000.5, ..ramp_row(number, 60000.0, 4) }).collect()
    }

    fn written(dir: &Path, name: &str, extra: &[(&'static str, &str)], tables: &[SynthTable]) -> String {
        let hdus = tables.iter().map(rows_hdu).collect();
        write_series_file(dir, name, primary(extra), hdus).to_str().unwrap().to_string()
    }

    fn three_integration_file(dir: &Path) -> String {
        written(dir, "t_x1dints.fits", &[], &[table(1, "NRS1", three_integrations())])
    }

    fn nrs1() -> SeriesKey {
        SeriesKey { detector: "NRS1".into(), sporder: Some(-1) }
    }

    #[tokio::test]
    async fn tso_open_cmd_describes_files_and_series_without_sample_arrays() {
        let dir = tempfile::tempdir().unwrap();
        let path = three_integration_file(dir.path());
        let value = tso_open_cmd(vec![path.clone()]).await.unwrap();
        assert_eq!(sorted_keys(&value), OPEN_KEYS);
        let set_id = value["set_id"].as_str().unwrap();
        assert_eq!(set_id.len(), 16);
        assert!(set_id.chars().all(|digit| digit.is_ascii_digit() || ('a'..='f').contains(&digit)), "{set_id}");

        let files = value["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(sorted_keys(&files[0]), FILE_KEYS);
        assert_eq!(files[0]["path"], path.as_str());
        assert_eq!(files[0]["file_name"], "t_x1dints.fits");
        assert_eq!(files[0]["level"], 2);
        assert_eq!(files[0]["layout"], "rows");
        assert_eq!(files[0]["nints"], 3);
        assert_eq!(files[0]["sha256"].as_str().unwrap().len(), 64);

        let series = value["series"].as_array().unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(sorted_keys(&series[0]), SERIES_KEYS);
        assert_eq!(
            series[0],
            json!({
                "key": {"detector": "NRS1", "sporder": -1},
                "label": "NRS1",
                "n_integrations": 3,
                "n_channels": 4,
                "flux_unit": "Jy",
                "time_scale": "bmjd_tdb",
                "time_source": "TDB-MID column",
                "time_first": 60000.0 + 0.1 * 1.0 + 0.002,
                "time_last": 60000.0 + 0.1 * 3.0 + 0.002,
                "wavelength_min_um": 1.0,
                "wavelength_max_um": 1.0 + 0.1 * 3.0,
                "common_grid": true,
                "outlier_detection": null,
                "completeness": [{"exposure": "t", "nints": 3, "found": 3, "missing": [], "complete": true}],
                "notes": ["3 integrations from 1 EXTRACT1D table(s) in 1 file(s)"],
                "warnings": [],
            })
        );
        assert_eq!(value["notes"], json!(["3 integrations from 1 EXTRACT1D table(s) in 1 file(s)"]));
        assert_eq!(value["warnings"], json!([]));

        let text = value.to_string();
        assert!(!text.contains("\"flux\":"), "{text}");
        assert!(!text.contains("\"wavelength_um\":"), "{text}");
        assert!(!text.contains("\"dq\":"), "{text}");
    }

    #[tokio::test]
    async fn tso_open_cmd_prefixes_series_sentences_when_the_set_holds_several_series() {
        let dir = tempfile::tempdir().unwrap();
        let both = [table(1, "NRS1", untimed_integrations()), table(2, "NRS2", untimed_integrations())];
        let two = tso_open_cmd(vec![written(dir.path(), "two_x1dints.fits", &[], &both)]).await.unwrap();
        let series = two["series"].as_array().unwrap();
        assert_eq!(series.len(), 2);
        assert!(series[0]["warnings"][0].as_str().unwrap().starts_with("no usable integration times"), "{}", series[0]["warnings"]);
        assert_eq!(series[0]["warnings"], json!([NO_USABLE_TIMES]));
        assert_eq!(series[1]["label"], "NRS2");
        assert_eq!(series[1]["warnings"], json!([NO_USABLE_TIMES]));
        assert_eq!(texts(&two["warnings"]), [format!("NRS1: {NO_USABLE_TIMES}"), format!("NRS2: {NO_USABLE_TIMES}")]);
        assert_eq!(
            texts(&two["notes"]),
            [
                "NRS1: 3 integrations from 1 EXTRACT1D table(s) in 1 file(s)",
                "NRS2: 3 integrations from 1 EXTRACT1D table(s) in 1 file(s)",
            ]
        );
        assert_eq!(series[0]["notes"], json!(["3 integrations from 1 EXTRACT1D table(s) in 1 file(s)"]));
        assert_eq!(series[0]["time_scale"], "index");
        assert_eq!(series[0]["time_source"], "integration number (no usable times)");
        assert_eq!(series[0]["time_first"], 1.0);
        assert_eq!(series[0]["time_last"], 3.0);

        let alone = [table(1, "NRS1", untimed_integrations())];
        let one = tso_open_cmd(vec![written(dir.path(), "one_x1dints.fits", &[], &alone)]).await.unwrap();
        assert_eq!(texts(&one["warnings"]), [NO_USABLE_TIMES]);
        assert_eq!(texts(&one["notes"]), ["3 integrations from 1 EXTRACT1D table(s) in 1 file(s)"]);
    }

    #[test]
    fn the_open_result_joins_set_and_series_sentences_once_and_survives_an_empty_series() {
        let mut set = ramp_set("NRS1", Some(0), 4, 3);
        set.notes = vec!["shared".into(), "of the set".into()];
        set.warnings = vec!["caveat".into()];
        set.series[0].notes = vec!["shared".into(), "of the series".into(), "of the series".into()];
        set.series[0].warnings = vec!["caveat".into(), "another".into()];
        let described = serde_json::to_value(open_result(&set)).unwrap();
        assert_eq!(texts(&described["notes"]), ["shared", "of the set", "of the series"]);
        assert_eq!(texts(&described["warnings"]), ["caveat", "another"]);
        assert_eq!(texts(&described["series"][0]["notes"]), ["shared", "of the series", "of the series"]);
        assert_eq!(described["set_id"], "827d096d92f3deea");
        assert_eq!(described["series"][0]["time_first"], 60000.0 + 0.002);
        assert_eq!(described["series"][0]["time_last"], 60000.0 + 0.01 * 3.0 + 0.002);

        let series = &mut set.series[0];
        series.n_integrations = 0;
        series.int_num.clear();
        series.mjd_utc_mid.clear();
        series.bmjd_tdb_mid.clear();
        series.time_scale = TsoTimeScale::Index;
        series.grid_um = vec![f64::NAN; 3];
        let empty = serde_json::to_value(open_result(&set)).unwrap();
        let summary = &empty["series"][0];
        assert_eq!(summary["n_integrations"], 0);
        assert!(summary["time_first"].is_null() && summary["time_last"].is_null(), "{summary}");
        assert!(summary["wavelength_min_um"].is_null() && summary["wavelength_max_um"].is_null(), "{summary}");
    }

    #[tokio::test]
    async fn tso_spectrum_cmd_returns_a_document_with_an_x1dints_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = three_integration_file(dir.path());
        let mean = tso_spectrum_cmd(vec![path.clone()], nrs1(), TsoPick::Mean { first: 0, last: 1 }).await.unwrap();
        assert_eq!(sorted_keys(&mean), DOCUMENT_KEYS);
        assert_eq!(
            mean["source"],
            json!({
                "kind": "x1dints",
                "paths": [path.as_str()],
                "detector": "NRS1",
                "sporder": -1,
                "pick": {"kind": "mean", "first": 0, "last": 1},
            })
        );
        assert_eq!(mean["flux"], json!([15.0, 16.0, 17.0, 18.0]));
        assert_eq!(mean["err"], json!(vec![0.5f64.sqrt() / 2.0; 4]));
        assert_eq!(mean["dq"], json!([0, 0, 0, 0]));
        assert_eq!(mean["n_rows"], 4);
        assert_eq!(mean["flux_unit"], "Jy");
        assert_eq!(mean["dq_table"], "jwst");
        assert_eq!(mean["precision"], "f64");
        assert_eq!(mean["wavelength_frame"], "vacuum");
        assert_eq!(mean["wavelength_unit_source"], "file");
        assert_eq!(mean["axis"]["unit"], "um");
        assert_eq!(mean["axis"]["values"], json!([1.0, 1.0 + 0.1 * 1.0, 1.0 + 0.1 * 2.0, 1.0 + 0.1 * 3.0]));
        assert_eq!(mean["cards"], json!({"DETECTOR": "NRS1", "EXP_TYPE": "NRS_BRIGHTOBJ"}));
        assert_eq!(mean["notes"][1], "time series NRS1: mean of integrations 1 to 2 of 3");

        let beyond = tso_spectrum_cmd(vec![path.clone()], nrs1(), TsoPick::Integration { index: 7 }).await.unwrap_err();
        assert!(beyond.contains("integration index 7 is out of range: the series NRS1 has 3 integrations (0 to 2)"), "{beyond}");
        let another = SeriesKey { detector: "NRS2".into(), sporder: Some(-1) };
        let absent = tso_spectrum_cmd(vec![path], another, TsoPick::Median).await.unwrap_err();
        assert_eq!(absent, "no series NRS2, order -1 in these files; series found: NRS1");
    }

    #[tokio::test]
    async fn the_commands_refuse_empty_lists_and_pass_the_reader_refusals_through() {
        assert_eq!(tso_open_cmd(vec![]).await.unwrap_err(), EMPTY_PATHS_ERROR);
        assert_eq!(tso_open_cmd(vec!["  ".into()]).await.unwrap_err(), EMPTY_PATHS_ERROR);
        assert_eq!(tso_spectrum_cmd(vec![], nrs1(), TsoPick::Median).await.unwrap_err(), EMPTY_PATHS_ERROR);

        let dir = tempfile::tempdir().unwrap();
        let plain = three_integration_file(dir.path());
        assert_eq!(tso_open_cmd(vec![plain.clone(), String::new()]).await.unwrap_err(), EMPTY_PATHS_ERROR);
        assert_eq!(
            tso_spectrum_cmd(vec![plain.clone(), " ".into()], nrs1(), TsoPick::Median).await.unwrap_err(),
            EMPTY_PATHS_ERROR
        );

        let cube = write_series_file(
            dir.path(),
            "cube.fits",
            series_primary("CubeModel", &[]),
            vec![rows_hdu(&table(1, "NRS1", three_integrations()))],
        );
        let not_a_series = tso_open_cmd(vec![cube.to_str().unwrap().to_string()]).await.unwrap_err();
        assert!(not_a_series.contains("cube.fits: DATAMODL 'CubeModel' is not a time series of spectra"), "{not_a_series}");

        let level_3 = written(dir.path(), "l3_x1dints.fits", &[("S_OUTLIR", "'COMPLETE'")], &[table(1, "NRS1", three_integrations())]);
        let mixed = tso_open_cmd(vec![level_3.clone(), plain.clone()]).await.unwrap_err();
        assert!(mixed.contains("is a level 3 product and"), "{mixed}");
        assert!(mixed.contains("open one level at a time"), "{mixed}");
        let mixed_pick = tso_spectrum_cmd(vec![level_3, plain.clone()], nrs1(), TsoPick::Median).await.unwrap_err();
        assert_eq!(mixed_pick, mixed);

        let packed = gzip_copy(Path::new(&plain), "t_x1dints.fits.gz").to_str().unwrap().to_string();
        let from_gzip = tso_open_cmd(vec![packed.clone()]).await.unwrap();
        let from_plain = tso_open_cmd(vec![plain.clone()]).await.unwrap();
        assert_eq!(from_gzip["series"], from_plain["series"]);
        assert_eq!(from_gzip["files"][0]["file_name"], "t_x1dints.fits.gz");
        assert_ne!(from_gzip["set_id"], from_plain["set_id"]);
        let pick = TsoPick::Integration { index: 2 };
        let gzip_spectrum = tso_spectrum_cmd(vec![packed], nrs1(), pick.clone()).await.unwrap();
        let plain_spectrum = tso_spectrum_cmd(vec![plain], nrs1(), pick).await.unwrap();
        assert_eq!(gzip_spectrum["flux"], json!([30.0, 31.0, 32.0, 33.0]));
        assert_eq!(gzip_spectrum["flux"], plain_spectrum["flux"]);
        assert_eq!(gzip_spectrum["notes"], plain_spectrum["notes"]);
    }

    const E1: &str = "E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits";
    const E1_GOLDEN: &str = "plan/goldens/tso/x1dints_jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048.json";
    const A3: &str = "A3/jw07073-o004_t001_niriss_clear-gr700xd-substrip256_x1dints.fits";
    const A3_FACTS: &str = "plan/data-facts-tso.md 1.2";
    const B1: &str = "B1/jw01274-o013_t008_miri_p750l-slitlessprism_x1dints.fits";
    const B1_GOLDEN: &str = "plan/goldens/tso/x1dints_jw01274-o013_t008_miri_p750l-slitlessprism.json";
    const C_GOLDEN: &str = "plan/goldens/tso/x1dints_jw02327001001_03101_00001-seg00{1..6}_nrs1.json";
    const PICK_ORACLE: &str = "plan/pacotes/TSO-support/tso_checks.py pick";
    const ORACLE_TOLERANCE: f64 = 1e-12;

    fn real(relative: &str) -> String {
        let path = env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("tso").join(relative);
        assert!(path.is_file(), "{} is missing", path.display());
        path.to_str().unwrap().to_string()
    }

    fn only_series(opened: &Value, source: &str) -> Value {
        let series = opened["series"].as_array().unwrap();
        assert_eq!(series.len(), 1, "series count, {source}");
        series[0].clone()
    }

    #[tokio::test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/E1, tso/A3, tso/B1, tso/C"]
    async fn real_sets_open_with_the_series_times_and_identities_of_the_goldens() {
        let golden = E1_GOLDEN;
        let prism = tso_open_cmd(vec![real(E1)]).await.unwrap();
        assert_eq!(sorted_keys(&prism), OPEN_KEYS, "{golden}");
        assert_eq!(prism["set_id"], "4b385da81d5c70b8", "set id, {golden}");
        let file = &prism["files"][0];
        assert_eq!(prism["files"].as_array().unwrap().len(), 1, "{golden}");
        assert_eq!(sorted_keys(file), FILE_KEYS, "{golden}");
        assert_eq!(file["sha256"], "a5e7022d22e779fdf43b14fba6e7d23644c9547c920b6a52c13435c63706bfbc", "produced_by.inputs[0].sha256 of {golden}");
        assert_eq!(file["level"], 3, "{golden}");
        assert_eq!(file["target"], "WD 1856+534 b", "{golden}");
        assert_eq!(file["whitelight_log"], json!({"min_um": null, "max_um": null}), "{golden}");
        let series = only_series(&prism, golden);
        assert_eq!(sorted_keys(&series), SERIES_KEYS, "{golden}");
        assert_eq!(series["key"], json!({"detector": "NRS1", "sporder": 0}), "{golden}");
        assert_eq!(series["label"], "NRS1", "{golden}");
        assert_eq!(series["n_integrations"], 240, "{golden}");
        assert_eq!(series["n_channels"], 431, "{golden}");
        assert_eq!(series["flux_unit"], "Jy", "{golden}");
        assert_eq!(series["time_scale"], "bmjd_tdb", "{golden}");
        assert_eq!(series["time_source"], "TDB-MID column", "{golden}");
        assert_eq!(series["time_first"], 60061.4421733457, "first BMJD (TDB) in days, {golden}");
        assert_eq!(series["time_last"], 60061.52706665472, "last BMJD (TDB) in days, {golden}");
        assert_eq!(series["wavelength_min_um"], 0.5530469695008425, "shortest wavelength in um, {golden}");
        assert_eq!(series["wavelength_max_um"], 5.370352185327354, "longest wavelength in um, {golden}");
        assert_eq!(series["common_grid"], true, "{golden}");
        assert_eq!(series["outlier_detection"], "COMPLETE", "{golden}");
        assert_eq!(
            series["completeness"],
            json!([{
                "exposure": "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048",
                "nints": 240,
                "found": 240,
                "missing": [],
                "complete": true,
            }]),
            "{golden}"
        );
        assert_eq!(prism["warnings"], json!([]), "top-level warnings, {golden}");
        let notes = texts(&prism["notes"]);
        assert!(
            notes.iter().any(|note| note.starts_with("extracted after outlier detection (S_OUTLIR = COMPLETE)")),
            "top-level notes, {golden}: {notes:?}"
        );
        let text = prism.to_string();
        assert!(!text.contains("\"flux\":") && !text.contains("\"wavelength_um\":"), "sample arrays in the answer, {golden}");

        let facts = A3_FACTS;
        let soss = tso_open_cmd(vec![real(A3)]).await.unwrap();
        let orders = soss["series"].as_array().unwrap();
        let expected = [
            ("NIS, order 1", 1, 2048, 0.8473834834322623, 2.8329402405819173),
            ("NIS, order 2", 2, 1783, 0.5878459442296309, 1.4138421961399272),
            ("NIS, order 3", 3, 1134, 0.632646909646051, 0.9556302324967343),
        ];
        assert_eq!(orders.len(), 3, "series count, {facts}");
        let warnings = texts(&soss["warnings"]);
        for (order, (label, sporder, channels, shortest_um, longest_um)) in orders.iter().zip(expected) {
            assert_eq!(order["label"], label, "{facts}");
            assert_eq!(order["key"], json!({"detector": "NIS", "sporder": sporder}), "{label}, {facts}");
            assert_eq!(order["n_channels"], channels, "{label} channels, {facts}");
            assert_eq!(order["n_integrations"], 5, "{label} integrations, {facts}");
            assert_eq!(order["flux_unit"], "MJy", "{label} unit, {facts}");
            assert_eq!(order["time_scale"], "index", "{label} time scale, {facts}");
            assert_eq!(order["time_first"], 1.0, "{label} first integration number, {facts}");
            assert_eq!(order["time_last"], 5.0, "{label} last integration number, {facts}");
            assert_eq!(order["wavelength_min_um"], shortest_um, "{label} shortest wavelength in um, {facts}");
            assert_eq!(order["wavelength_max_um"], longest_um, "{label} longest wavelength in um, {facts}");
            let untimed = format!("{label}: no usable integration times: TDB-MID repeats values and MJD-AVG has NaN values; the abscissa is the integration number");
            let relative = format!("{label}: FLUX is labelled MJy but NIRISS SOSS numbers are not on that scale: treat the flux as relative");
            assert!(warnings.contains(&untimed.as_str()), "{label} time warning, {facts}: {warnings:?}");
            assert!(warnings.contains(&relative.as_str()), "{label} SOSS warning, {facts}: {warnings:?}");
        }

        let golden = B1_GOLDEN;
        let legacy = tso_open_cmd(vec![real(B1)]).await.unwrap();
        assert_eq!(legacy["set_id"], "38d8839dfd3fa542", "set id, {golden}");
        assert_eq!(legacy["files"][0]["layout"], "extensions", "{golden}");
        assert_eq!(legacy["files"][0]["cal_ver"], "1.15.1", "{golden}");
        let series = only_series(&legacy, golden);
        assert_eq!(series["label"], "MIRIMAGE", "{golden}");
        assert_eq!(series["n_integrations"], 10, "{golden}");
        assert_eq!(series["n_channels"], 388, "{golden}");
        assert_eq!(series["time_source"], "TDB-MID keyword", "{golden}");
        assert_eq!(series["time_first"], 60126.80095917577, "first BMJD (TDB) in days, {golden}");
        assert_eq!(series["time_last"], 60126.80143914741, "last BMJD (TDB) in days, {golden}");

        let golden = C_GOLDEN;
        let segment = |number: usize| format!("jw02327001001_03101_00001-seg{number:03}_nrs1_x1dints.fits");
        let reversed: Vec<String> = (1..=6).rev().map(|number| real(&format!("C/{}", segment(number)))).collect();
        let stitched = tso_open_cmd(reversed).await.unwrap();
        assert_eq!(stitched["set_id"], "b31be38139d63dd4", "set id, {golden}");
        let files = stitched["files"].as_array().unwrap();
        let names: Vec<&str> = files.iter().map(|file| file["file_name"].as_str().unwrap()).collect();
        assert_eq!(names, (1..=6).map(segment).collect::<Vec<_>>(), "files in set order, {golden}");
        assert!(files.iter().all(|file| file["level"] == 2), "levels, {golden}");
        let series = only_series(&stitched, golden);
        assert_eq!(series["label"], "NRS1", "{golden}");
        assert_eq!(series["n_integrations"], 44, "{golden}");
        assert_eq!(series["n_channels"], 1299, "{golden}");
        assert_eq!(series["time_first"], 60280.05657544178, "first BMJD (TDB) in days, {golden}");
        assert_eq!(series["time_last"], 60280.50418870245, "last BMJD (TDB) in days, {golden}");
        assert_eq!(
            series["completeness"],
            json!([{"exposure": "jw02327001001_03101_00001", "nints": 44, "found": 44, "missing": [], "complete": true}]),
            "{golden}"
        );
    }

    fn numbers(value: &Value) -> Vec<Option<f64>> {
        value.as_array().unwrap().iter().map(Value::as_f64).collect()
    }

    fn assert_within(measured: Option<f64>, expected: f64, what: &str) {
        let measured = measured.unwrap_or(f64::NAN);
        let relative = ((measured - expected) / expected).abs();
        println!("{what}: measured {measured:?}, oracle {expected:?}, relative difference {relative:.1e}");
        assert!(relative <= ORACLE_TOLERANCE, "{what}: measured {measured:?}, oracle {expected:?}, relative difference {relative:e}, {PICK_ORACLE}");
    }

    #[tokio::test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/E1, tso/A3, tso/B1, tso/C"]
    async fn real_picks_of_the_transit_file_match_the_oracle() {
        let oracle = PICK_ORACLE;
        let path = real(E1);
        let key = SeriesKey { detector: "NRS1".into(), sporder: Some(0) };
        let channels = [15, 16, 200];

        let started = Instant::now();
        let median = tso_spectrum_cmd(vec![path.clone()], key.clone(), TsoPick::Median).await.unwrap();
        let first_call = started.elapsed();
        let started = Instant::now();
        let again = tso_spectrum_cmd(vec![path.clone()], key.clone(), TsoPick::Median).await.unwrap();
        let second_call = started.elapsed();
        println!("tso_spectrum_cmd, median of 240 integrations x 431 channels: first call {first_call:.1?}, second call on the same path {second_call:.1?}");
        assert_eq!(again, median, "the second call gave another document");

        let wavelengths = numbers(&median["axis"]["values"]);
        let (flux, err, dq) = (numbers(&median["flux"]), numbers(&median["err"]), numbers(&median["dq"]));
        assert_eq!(flux.len(), 431, "{oracle}");
        let expected_wavelengths = [0.611371784492148, 0.6159113632376663, 3.260827152963672];
        let expected_flux = [1.1245526920983315e-05, 0.0006430169860636552, 0.000298374341547762];
        let expected_err = [6.185169825052713e-08, 7.120013896551136e-07, 2.718504826611329e-07];
        for (position, channel) in channels.into_iter().enumerate() {
            assert_eq!(wavelengths[channel], Some(expected_wavelengths[position]), "wavelength of channel {channel} in um, {oracle}");
            assert_eq!(flux[channel], Some(expected_flux[position]), "median flux of channel {channel} in Jy, {oracle}");
            assert_within(err[channel], expected_err[position], &format!("median error of channel {channel} in Jy"));
        }
        let empty: Vec<usize> = (0..431).filter(|&channel| flux[channel].is_none()).collect();
        assert_eq!(empty.len(), 24, "channels without a valid sample, {oracle}: {empty:?}");
        for channel in [0, 14, 430] {
            assert!(empty.contains(&channel), "channel {channel} has a median, {oracle}");
        }
        for channel in 0..431 {
            let without_sample = flux[channel].is_none();
            assert_eq!(err[channel].is_none(), without_sample, "error of channel {channel}, {oracle}");
            assert_eq!(dq[channel], Some(if without_sample { 1.0 } else { 0.0 }), "DQ of channel {channel}, {oracle}");
        }
        let notes = texts(&median["notes"]);
        assert_eq!(notes[1], "time series NRS1: median of 240 integrations", "{oracle}");
        assert!(
            notes.contains(&"24 of 431 channels have no valid sample in these integrations and are NaN"),
            "{oracle}: {notes:?}"
        );
        assert_eq!(median["flux_unit"], "Jy", "{oracle}");
        assert_eq!(median["cards"]["TARGNAME"], "WD 1856+534 b", "{oracle}");

        let mean = tso_spectrum_cmd(vec![path.clone()], key.clone(), TsoPick::Mean { first: 0, last: 9 }).await.unwrap();
        let (flux, err) = (numbers(&mean["flux"]), numbers(&mean["err"]));
        let expected_flux = [1.107731528838385e-05, 0.0006433713129263156, 0.0002965090354821721];
        let expected_err = [2.4547227496369594e-07, 2.6302544991248915e-06, 1.0879364125016148e-06];
        for (position, channel) in channels.into_iter().enumerate() {
            assert_within(flux[channel], expected_flux[position], &format!("mean flux of integrations 1 to 10, channel {channel}, in Jy"));
            assert_within(err[channel], expected_err[position], &format!("mean error of integrations 1 to 10, channel {channel}, in Jy"));
        }
        assert_eq!(mean["notes"][1], "time series NRS1: mean of integrations 1 to 10 of 240", "{oracle}");

        let one = tso_spectrum_cmd(vec![path], key, TsoPick::Integration { index: 185 }).await.unwrap();
        let (flux, err, dq) = (numbers(&one["flux"]), numbers(&one["err"]), numbers(&one["dq"]));
        let expected_flux = [1.603036377648799e-07, 3.039650652816377e-05, 8.03992882822244e-06];
        let expected_err = [2.708061726371241e-08, 1.7111405079871057e-05, 9.675339464903363e-07];
        for (position, channel) in channels.into_iter().enumerate() {
            assert_eq!(flux[channel], Some(expected_flux[position]), "flux of integration 186, channel {channel}, in Jy, {oracle}");
            assert_eq!(err[channel], Some(expected_err[position]), "error of integration 186, channel {channel}, in Jy, {oracle}");
        }
        assert_eq!((flux[0], err[0], dq[0]), (None, None, Some(1.0)), "channel 0 of integration 186 (the file stores error 0.0), {oracle}");
        assert_eq!(
            one["notes"][1],
            "time series NRS1: integration 186 of 240 (INT_NUM 186, BMJD (TDB) 60061.50788573973)",
            "{oracle}"
        );
    }

    const LIGHT_CURVE_KEYS: [&str; 16] = [
        "bins",
        "dropped",
        "flux_unit",
        "int_num",
        "label",
        "mjd_utc",
        "mode",
        "notes",
        "rejected_elements",
        "row_index",
        "series",
        "set_id",
        "time",
        "time_scale",
        "warnings",
        "white",
    ];
    const CURVE_KEYS: [&str; 6] = ["band_um", "flux", "flux_err", "limits_origin", "n_used", "npixels_mean"];
    const FLUX_ERR_NOTE: &str =
        "flux_err is the square root of the summed FLUX_ERROR squared: the app's number, not a pipeline product";

    fn five_channel_integrations() -> Vec<SynthRow> {
        (1..=3).map(|number| ramp_row(number, 60000.0 + 0.1 * f64::from(number), 5)).collect()
    }

    fn curve_file(dir: &Path, rows: Vec<SynthRow>) -> String {
        written(dir, "lc_x1dints.fits", &[], &[table(1, "NRS1", rows)])
    }

    fn request(mode: CurveMode) -> CurveRequest {
        CurveRequest {
            mode,
            wavelength_min_um: None,
            wavelength_max_um: None,
            bins: Vec::new(),
            exclude_um: Vec::new(),
            reject_elements_sigma: None,
        }
    }

    #[tokio::test]
    async fn tso_light_curves_cmd_returns_the_white_light_and_the_bins_with_their_limits() {
        let dir = tempfile::tempdir().unwrap();
        let path = curve_file(dir.path(), five_channel_integrations());
        let binned = CurveRequest { bins: vec![[1.05, 1.25], [1.25, 9.0]], ..request(CurveMode::Pipeline) };
        let curves = tso_light_curves_cmd(vec![path.clone()], nrs1(), binned).await.unwrap();
        assert_eq!(sorted_keys(&curves), LIGHT_CURVE_KEYS);
        let opened = tso_open_cmd(vec![path]).await.unwrap();
        assert_eq!(curves["set_id"], opened["set_id"]);
        assert_eq!(curves["label"], "NRS1");
        assert_eq!(curves["series"], json!({"detector": "NRS1", "sporder": -1}));
        assert_eq!(curves["mode"], "pipeline");
        assert_eq!(curves["time_scale"], "bmjd_tdb");
        let mjd: Vec<f64> = (1..=3).map(|number| 60000.0 + 0.1 * f64::from(number)).collect();
        assert_eq!(curves["time"], json!(mjd.iter().map(|time| time + 0.002).collect::<Vec<_>>()));
        assert_eq!(curves["mjd_utc"], json!(mjd));
        assert_eq!(curves["int_num"], json!([1, 2, 3]));
        assert_eq!(curves["row_index"], json!([0, 1, 2]));
        assert_eq!(curves["flux_unit"], "Jy");

        let white = &curves["white"];
        assert_eq!(sorted_keys(white), CURVE_KEYS);
        assert_eq!(white["flux"], json!([60.0, 110.0, 160.0]));
        assert_eq!(white["flux_err"], json!(vec![1.25f64.sqrt(); 3]));
        assert_eq!(white["n_used"], json!([5, 5, 5]));
        assert_eq!(white["npixels_mean"], json!([7.0, 7.0, 7.0]));
        assert_eq!(white["limits_origin"], "open");
        assert_eq!(white["band_um"], json!([1.0, 1.0 + 0.1 * 4.0]));

        let bins = curves["bins"].as_array().unwrap();
        assert_eq!(bins.len(), 2);
        assert_eq!(sorted_keys(&bins[0]), CURVE_KEYS);
        assert_eq!(bins[0]["flux"], json!([23.0, 43.0, 63.0]));
        assert_eq!(bins[1]["flux"], json!([27.0, 47.0, 67.0]));
        assert_eq!(bins[0]["n_used"], json!([2, 2, 2]));
        assert_eq!(bins[0]["limits_origin"], "user");
        assert_eq!(bins[0]["band_um"], json!([1.05, 1.25]));
        assert_eq!(bins[1]["limits_origin"], "user");
        assert_eq!(bins[1]["band_um"], json!([1.25, 9.0]));

        assert_eq!(curves["dropped"], json!({"nan_time": 0, "repeated_time": 0}));
        assert_eq!(curves["rejected_elements"], json!([]));
        assert_eq!(curves["notes"], json!([FLUX_ERR_NOTE]));
        assert_eq!(curves["warnings"], json!([]));
    }

    #[tokio::test]
    async fn the_masked_mode_and_its_options_travel_through_the_request() {
        let dir = tempfile::tempdir().unwrap();
        let mut rows = five_channel_integrations();
        rows[1].flux[2] = f64::NAN;
        let path = curve_file(dir.path(), rows);
        let masked = CurveRequest { exclude_um: vec![[0.95, 1.05]], ..request(CurveMode::Masked) };
        let curves = tso_light_curves_cmd(vec![path.clone()], nrs1(), masked).await.unwrap();
        assert_eq!(curves["mode"], "masked");
        assert_eq!(curves["white"]["flux"], json!([38.0, 68.0, 98.0]));
        assert_eq!(curves["white"]["n_used"], json!([3, 3, 3]));
        assert_eq!(curves["row_index"], json!([0, 1, 2]));
        let notes = texts(&curves["notes"]);
        assert!(notes.contains(&"4 of 5 channels are valid in every integration"), "{notes:?}");
        assert!(notes.contains(&"1 channel(s) removed by the exclusion ranges"), "{notes:?}");

        let pipeline = tso_light_curves_cmd(vec![path], nrs1(), request(CurveMode::Pipeline)).await.unwrap();
        assert_eq!(pipeline["mode"], "pipeline");
        assert_eq!(pipeline["white"]["flux"], json!([60.0, 88.0, 160.0]));
        assert_eq!(pipeline["white"]["n_used"], json!([5, 4, 5]));

        let sent = json!({"mode": "masked", "wavelength_min_um": null, "bins": [[1.0, 2.0]]});
        let read: CurveRequest = serde_json::from_value(sent).unwrap();
        assert_eq!(read, CurveRequest { bins: vec![[1.0, 2.0]], ..request(CurveMode::Masked) });
    }

    #[tokio::test]
    async fn refusals_of_the_set_the_series_and_the_request_reach_the_caller() {
        let nothing = tso_light_curves_cmd(vec![], nrs1(), request(CurveMode::Pipeline)).await.unwrap_err();
        assert_eq!(nothing, EMPTY_PATHS_ERROR);
        let blank = tso_light_curves_cmd(vec![" ".into()], nrs1(), request(CurveMode::Masked)).await.unwrap_err();
        assert_eq!(blank, EMPTY_PATHS_ERROR);

        let dir = tempfile::tempdir().unwrap();
        let path = curve_file(dir.path(), five_channel_integrations());
        let another = SeriesKey { detector: "NRS2".into(), sporder: Some(-1) };
        let absent = tso_light_curves_cmd(vec![path.clone()], another, request(CurveMode::Pipeline)).await.unwrap_err();
        assert!(absent.contains("no series NRS2, order -1 in these files; series found: NRS1"), "{absent}");

        let rejecting = CurveRequest { reject_elements_sigma: Some(5.0), ..request(CurveMode::Pipeline) };
        let refused = tso_light_curves_cmd(vec![path.clone()], nrs1(), rejecting).await.unwrap_err();
        assert!(refused.contains("exclude_um and reject_elements_sigma apply to the masked mode only"), "{refused}");

        let reversed = CurveRequest { wavelength_min_um: Some(3.0), wavelength_max_um: Some(2.0), ..request(CurveMode::Pipeline) };
        let empty = tso_light_curves_cmd(vec![path], nrs1(), reversed).await.unwrap_err();
        assert!(empty.contains("wavelength limits 3 to 2 um are empty"), "{empty}");

        let missing = dir.path().join("absent_x1dints.fits").to_str().unwrap().to_string();
        let unread = tso_light_curves_cmd(vec![missing], nrs1(), request(CurveMode::Pipeline)).await.unwrap_err();
        assert!(unread.contains("absent_x1dints.fits"), "{unread}");
    }

    #[tokio::test]
    async fn non_finite_numbers_arrive_as_null() {
        let dir = tempfile::tempdir().unwrap();
        let mut rows = five_channel_integrations();
        rows[1].mjd_avg = f64::NAN;
        let path = curve_file(dir.path(), rows);
        let outside = CurveRequest { bins: vec![[5.0, 6.0]], ..request(CurveMode::Masked) };
        let masked = tso_light_curves_cmd(vec![path.clone()], nrs1(), outside).await.unwrap();
        assert!(masked["mjd_utc"][1].is_null(), "{}", masked["mjd_utc"]);
        assert_eq!(masked["mjd_utc"].as_array().unwrap().len(), 3);
        assert_eq!(masked["bins"][0]["flux"], json!([null, null, null]));
        assert_eq!(masked["bins"][0]["flux_err"], json!([null, null, null]));
        assert_eq!(masked["bins"][0]["npixels_mean"], json!([null, null, null]));
        assert_eq!(masked["bins"][0]["n_used"], json!([0, 0, 0]));
        assert_eq!(masked["white"]["flux"], json!([60.0, 110.0, 160.0]));

        let pipeline = tso_light_curves_cmd(vec![path], nrs1(), request(CurveMode::Pipeline)).await.unwrap();
        assert_eq!(pipeline["int_num"], json!([1, 3]));
        assert_eq!(pipeline["row_index"], json!([0, 2]));
        assert_eq!(pipeline["dropped"], json!({"nan_time": 1, "repeated_time": 0}));
        assert_eq!(pipeline["white"]["flux"], json!([60.0, 160.0]));
        assert_eq!(pipeline["time"].as_array().unwrap().len(), 2);
        assert_eq!(pipeline["mjd_utc"].as_array().unwrap().len(), 2);
    }

    const A1: &str = "A1/jw02459-o013_t007_nircam_f322w2-grismr-subgrism64_x1dints.fits";
    const A5_NRS1: &str = "A5/jw03375001001_03101_00001-seg001_nrs1_x1dints.fits";
    const A5_NRS2: &str = "A5/jw03375001001_03101_00001-seg001_nrs2_x1dints.fits";
    const CURVES_ORACLE: &str = "plan/pacotes/TSO-support/tso_checks.py curves";
    const SETS_ORACLE: &str = "plan/pacotes/TSO-support/tso_checks.py sets";
    const QUICKLOOK_GOLDEN: &str = "plan/goldens/tso/quicklook_jw02358-o030_nirspec_prism.json";
    const SUM_TOLERANCE: f64 = 1e-12;
    const WINDOW_MEAN_TOLERANCE: f64 = 1e-10;

    fn assert_close(measured: f64, expected: f64, tolerance: f64, what: &str) {
        let relative = ((measured - expected) / expected).abs();
        println!("{what}: measured {measured:?}, expected {expected:?}, relative difference {relative:.1e}");
        assert!(relative <= tolerance, "{what}: measured {measured:?}, expected {expected:?}, relative difference {relative:e}");
    }

    fn finite_numbers(value: &Value) -> Vec<f64> {
        value.as_array().unwrap().iter().map(|number| number.as_f64().unwrap()).collect()
    }

    fn integers(value: &Value) -> Vec<i64> {
        value.as_array().unwrap().iter().map(|number| number.as_i64().unwrap()).collect()
    }

    fn white_sum(curves: &Value) -> f64 {
        finite_numbers(&curves["white"]["flux"]).iter().sum()
    }

    fn white_mean_over(curves: &Value, windows: &[std::ops::RangeInclusive<i64>]) -> f64 {
        let inside: Vec<f64> = finite_numbers(&curves["white"]["flux"])
            .into_iter()
            .zip(integers(&curves["int_num"]))
            .filter(|(_, int_num)| windows.iter().any(|window| window.contains(int_num)))
            .map(|(flux, _)| flux)
            .collect();
        inside.iter().sum::<f64>() / inside.len() as f64
    }

    #[tokio::test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/E1, tso/C, tso/A5, tso/A1"]
    async fn real_light_curves_of_the_transit_file_through_the_command() {
        let oracle = CURVES_ORACLE;
        let golden = QUICKLOOK_GOLDEN;
        let path = real(E1);
        let key = SeriesKey { detector: "NRS1".into(), sporder: Some(0) };
        let bins = vec![[0.6, 1.0], [1.0, 2.0], [2.0, 3.0], [3.0, 4.0], [4.0, 5.3]];
        let pipeline = CurveRequest { bins: bins.clone(), ..request(CurveMode::Pipeline) };

        let started = Instant::now();
        let curves = tso_light_curves_cmd(vec![path.clone()], key.clone(), pipeline.clone()).await.unwrap();
        let first_call = started.elapsed();
        let started = Instant::now();
        let again = tso_light_curves_cmd(vec![path.clone()], key.clone(), pipeline).await.unwrap();
        let second_call = started.elapsed();
        println!("tso_light_curves_cmd on E1, white light and five bins: first call {first_call:.1?}, second call on the same path {second_call:.1?}");
        assert_eq!(again, curves, "the second call gave other curves");

        assert_eq!(curves["set_id"], "4b385da81d5c70b8", "{oracle}");
        assert_eq!(curves["label"], "NRS1", "{oracle}");
        assert_eq!(curves["time_scale"], "bmjd_tdb", "{oracle}");
        for field in ["time", "mjd_utc", "int_num", "row_index"] {
            assert_eq!(curves[field].as_array().unwrap().len(), 240, "{field}, {oracle}");
        }
        let all_curves = std::iter::once(&curves["white"]).chain(curves["bins"].as_array().unwrap());
        for curve in all_curves {
            for field in ["flux", "flux_err", "n_used", "npixels_mean"] {
                assert_eq!(curve[field].as_array().unwrap().len(), 240, "{field} of {}, {oracle}", curve["band_um"]);
            }
        }
        assert_eq!(curves["bins"].as_array().unwrap().len(), 5, "{oracle}");
        assert_eq!(curves["time"][0], 60061.4421733457, "first BMJD (TDB) in days, {oracle}");
        assert_eq!(curves["time"][239], 60061.52706665472, "last BMJD (TDB) in days, {oracle}");
        assert_eq!(curves["mjd_utc"][0], 60061.44133176829, "first MJD (UTC) in days, {oracle}");
        assert_eq!(curves["int_num"][185], 186, "{oracle}");
        let white = &curves["white"];
        assert_eq!(white["band_um"], json!([0.5530469695008425, 5.370352185327354]), "white-light band in um, {oracle}");
        assert_eq!(white["limits_origin"], "open", "{oracle}");
        let flux = finite_numbers(&white["flux"]);
        assert_close(flux[0], 0.16369951048163586, SUM_TOLERANCE, "pipeline white light of integration 1 in Jy");
        assert_close(flux[185], 0.00512653064346067, SUM_TOLERANCE, "pipeline white light of integration 186 in Jy");
        assert_close(white_sum(&curves), 37.761493394107156, SUM_TOLERANCE, "sum of the pipeline white light in Jy");
        assert_close(finite_numbers(&curves["bins"][0]["flux"])[0], 0.03868635075864136, SUM_TOLERANCE, "pipeline bin 0.6 to 1.0 um of integration 1 in Jy");
        let last_bin: f64 = finite_numbers(&curves["bins"][4]["flux"]).iter().sum();
        assert_close(last_bin, 6.568168892743402, SUM_TOLERANCE, "sum of the pipeline bin 4.0 to 5.3 um in Jy");
        let warnings = texts(&curves["warnings"]);
        let pixel_loss = "mean NPIXELS of the summed elements falls below 0.9 of its median in 10 integration(s) (INT_NUM 181 to 190)";
        assert!(warnings.iter().any(|warning| warning.contains(pixel_loss)), "{oracle}: {warnings:?}");
        let transit = [183..=188];
        let baseline = [1..=170, 201..=240];
        assert_close(white_mean_over(&curves, &transit), 0.008133413871789402, WINDOW_MEAN_TOLERANCE, &format!("variants.A_pipeline_sum_nan_skipped.white.F_in of {golden}"));
        assert_close(white_mean_over(&curves, &baseline), 0.1639531116709505, WINDOW_MEAN_TOLERANCE, &format!("variants.A_pipeline_sum_nan_skipped.white.F_out of {golden}"));

        let masked = CurveRequest { bins, ..request(CurveMode::Masked) };
        let curves = tso_light_curves_cmd(vec![path], key, masked).await.unwrap();
        assert_eq!(curves["mode"], "masked", "{oracle}");
        assert_eq!(curves["white"]["n_used"][0], 316, "channels valid in every integration, {oracle}");
        assert_close(white_sum(&curves), 21.09879206422516, SUM_TOLERANCE, "sum of the masked white light in Jy");
        assert_close(white_mean_over(&curves, &transit), 0.0064833549496611355, WINDOW_MEAN_TOLERANCE, &format!("variants.B_elements_finite_in_every_integration.white.F_in of {golden}"));
        assert_close(white_mean_over(&curves, &baseline), 0.091551203985301, WINDOW_MEAN_TOLERANCE, &format!("variants.B_elements_finite_in_every_integration.white.F_out of {golden}"));
    }

    #[tokio::test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/E1, tso/C, tso/A5, tso/A1"]
    async fn real_segments_and_two_detector_sets_give_curves_per_series() {
        let oracle = SETS_ORACLE;
        let segment = |number: usize| format!("C/jw02327001001_03101_00001-seg{number:03}_nrs1_x1dints.fits");
        let reversed: Vec<String> = (1..=6).rev().map(|number| real(&segment(number))).collect();
        let curves = tso_light_curves_cmd(reversed.clone(), nrs1(), request(CurveMode::Pipeline)).await.unwrap();
        assert_eq!(integers(&curves["int_num"]), (1..=44).collect::<Vec<i64>>(), "{oracle}");
        assert_eq!(curves["time"].as_array().unwrap().len(), 44, "{oracle}");
        let flux = finite_numbers(&curves["white"]["flux"]);
        let first_three = [-219.03753027615335, -226.98744321010696, -225.4153096523979];
        for (position, expected) in first_three.into_iter().enumerate() {
            assert_close(flux[position], expected, SUM_TOLERANCE, &format!("C pipeline white light of integration {}", position + 1));
        }
        assert_close(flux[43], -232.74737376121138, SUM_TOLERANCE, "C pipeline white light of integration 44");
        assert_close(white_sum(&curves), -10077.924127637387, SUM_TOLERANCE, "sum of the C pipeline white light");

        let rejecting = CurveRequest { reject_elements_sigma: Some(1000.0), ..request(CurveMode::Masked) };
        let masked = tso_light_curves_cmd(reversed, nrs1(), rejecting).await.unwrap();
        let rejected = masked["rejected_elements"].as_array().unwrap();
        assert_eq!(rejected.len(), 1, "{oracle}: {rejected:?}");
        assert_eq!(rejected[0]["index"], 1289, "{oracle}");
        assert_eq!(rejected[0]["wavelength_um"], 5.16590253862141, "{oracle}");
        assert_eq!(masked["white"]["n_used"][0], 1275, "{oracle}");
        assert_close(finite_numbers(&masked["white"]["flux"])[0], 0.34533711108076626, SUM_TOLERANCE, "C masked white light of integration 1, reject 1000");
        assert_close(white_sum(&masked), 15.310808142350753, SUM_TOLERANCE, "sum of the C masked white light, reject 1000");

        let pair = vec![real(A5_NRS1), real(A5_NRS2)];
        let short = tso_light_curves_cmd(pair.clone(), nrs1(), request(CurveMode::Pipeline)).await.unwrap();
        let flux = finite_numbers(&short["white"]["flux"]);
        let first_three = [-5953.628522647685, -6100.921577118759, -5799.353088876454];
        for (position, expected) in first_three.into_iter().enumerate() {
            assert_close(flux[position], expected, SUM_TOLERANCE, &format!("A5 NRS1 pipeline white light of integration {}", position + 1));
        }
        assert_close(white_sum(&short), -48889.21206983084, SUM_TOLERANCE, "sum of the A5 NRS1 pipeline white light");
        let nrs2 = SeriesKey { detector: "NRS2".into(), sporder: Some(-1) };
        let long = tso_light_curves_cmd(pair, nrs2, request(CurveMode::Pipeline)).await.unwrap();
        assert_close(finite_numbers(&long["white"]["flux"])[0], 3.71370321193536, SUM_TOLERANCE, "A5 NRS2 pipeline white light of integration 1");
        assert_close(white_sum(&long), 29.678629957549255, SUM_TOLERANCE, "sum of the A5 NRS2 pipeline white light");
        assert_eq!(short["time"][0], 60496.522504382556, "first BMJD (TDB) of NRS1 in days, {oracle}");
        assert_eq!(long["time"][0], 60496.52240268599, "first BMJD (TDB) of NRS2 in days, {oracle}");
    }

    #[tokio::test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/E1, tso/C, tso/A5, tso/A1"]
    async fn real_series_without_usable_times_keep_the_integration_number_as_abscissa() {
        let oracle = CURVES_ORACLE;
        let path = real(A1);
        let key = SeriesKey { detector: "NRCALONG".into(), sporder: Some(1) };
        let pipeline = tso_light_curves_cmd(vec![path.clone()], key.clone(), request(CurveMode::Pipeline)).await.unwrap();
        assert_eq!(pipeline["time"], json!([1.0]), "{oracle}");
        assert_eq!(pipeline["time_scale"], "index", "{oracle}");
        assert_eq!(pipeline["white"]["flux"].as_array().unwrap().len(), 1, "{oracle}");
        assert_close(finite_numbers(&pipeline["white"]["flux"])[0], 65197945.38277817, SUM_TOLERANCE, "A1 pipeline white light in DN/s");
        assert_eq!(pipeline["white"]["band_um"], json!([2.47, 3.94]), "{oracle}");
        assert_eq!(pipeline["white"]["limits_origin"], "file log", "{oracle}");
        assert_eq!(pipeline["dropped"], json!({"nan_time": 3, "repeated_time": 0}), "{oracle}");
        assert_eq!(pipeline["flux_unit"], "DN/s", "{oracle}");

        let masked = tso_light_curves_cmd(vec![path], key, request(CurveMode::Masked)).await.unwrap();
        assert_eq!(masked["time"], json!([1.0, 2.0, 3.0, 4.0]), "{oracle}");
        assert_eq!(masked["mjd_utc"], json!([59829.989846412034, null, null, null]), "{oracle}");
        assert_close(finite_numbers(&masked["white"]["flux"])[1], 65301954.81903076, SUM_TOLERANCE, "A1 masked white light of integration 2 in DN/s");
    }

    const WHTLT_IMPORT_KEYS: [&str; 6] = ["file_name", "meta", "notes", "path", "schema", "series"];
    const WHTLT_SERIES_KEYS: [&str; 7] = ["bmjd_tdb", "column", "detector", "flux", "flux_unit", "mjd_utc", "sporder"];
    const WHTLT_COMPARISON_KEYS: [&str; 10] = [
        "column",
        "label",
        "limits_origin",
        "limits_um",
        "max_rel_diff",
        "n_joined",
        "n_whtlt",
        "notes",
        "series",
        "within_1e4",
    ];

    fn text_file(dir: &Path, name: &str, text: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn whtlt_import_cmd_reads_a_table_from_disk_and_tso_whtlt_compare_cmd_pairs_it_with_a_set() {
        let dir = tempfile::tempdir().unwrap();
        let whtlt = text_file(dir.path(), "obs_whtlt.ecsv", &two_detectors_ecsv());
        let level_3 = [("S_OUTLIR", "'COMPLETE'")];
        let spectra = written(dir.path(), "obs_x1dints.fits", &level_3, &[table(1, "NRS1", five_channel_integrations())]);

        let import = whtlt_import_cmd(whtlt.clone()).await.unwrap();
        assert_eq!(sorted_keys(&import), WHTLT_IMPORT_KEYS);
        assert_eq!(import["path"], whtlt.as_str());
        assert_eq!(import["file_name"], "obs_whtlt.ecsv");
        assert_eq!(import["schema"], "current");
        assert_eq!(import["meta"], json!({"instrument": "NIRSPEC", "number_of_integrations": 3}));
        assert_eq!(import["notes"], json!(["1 row(s) without a finite time or flux left out of whitelight_flux_NRS2"]));
        let series = import["series"].as_array().unwrap();
        assert_eq!(series.len(), 2);
        assert_eq!(sorted_keys(&series[0]), WHTLT_SERIES_KEYS);
        assert_eq!(series[0]["column"], "whitelight_flux_NRS1");
        assert_eq!(series[0]["detector"], "NRS1");
        assert!(series[0]["sporder"].is_null(), "{}", series[0]);
        assert_eq!(series[0]["mjd_utc"], json!([60000.1, 60000.2, 60000.3]));
        assert_eq!(series[0]["bmjd_tdb"], json!([60000.102, 60000.202, 60000.302]));
        assert_eq!(series[0]["flux"], json!([60.0, 110.0, 160.0]));
        assert_eq!(series[0]["flux_unit"], "Jy");
        assert_eq!(series[1]["flux"], json!([6.0, 11.0]));

        let compared = tso_whtlt_compare_cmd(vec![spectra.clone()], whtlt).await.unwrap();
        let entries = compared.as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(sorted_keys(&entries[0]), WHTLT_COMPARISON_KEYS);
        assert_eq!(
            entries[0],
            json!({
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
            })
        );
        assert_eq!(sorted_keys(&entries[1]), WHTLT_COMPARISON_KEYS);
        assert!(entries[1]["series"].is_null(), "{}", entries[1]);
        assert!(entries[1]["max_rel_diff"].is_null(), "{}", entries[1]);
        assert_eq!(entries[1]["n_whtlt"], 2);
        assert_eq!(entries[1]["n_joined"], 0);

        let opened = tso_open_cmd(vec![spectra]).await.unwrap();
        assert_eq!(opened["files"][0]["level"], 3);
        let sibling = opened["files"][0]["whtlt_path"].as_str().unwrap();
        assert!(sibling.ends_with("obs_whtlt.ecsv"), "{sibling}");
    }

    #[tokio::test]
    async fn the_whtlt_commands_refuse_empty_arguments_and_files_that_are_not_tables() {
        assert_eq!(whtlt_import_cmd(String::new()).await.unwrap_err(), EMPTY_WHTLT_PATH_ERROR);
        assert_eq!(whtlt_import_cmd("  ".into()).await.unwrap_err(), EMPTY_WHTLT_PATH_ERROR);
        assert_eq!(tso_whtlt_compare_cmd(vec![], "x.ecsv".into()).await.unwrap_err(), EMPTY_PATHS_ERROR);
        assert_eq!(tso_whtlt_compare_cmd(vec!["a.fits".into()], " ".into()).await.unwrap_err(), EMPTY_WHTLT_PATH_ERROR);

        let dir = tempfile::tempdir().unwrap();
        let plain = text_file(dir.path(), "plain_whtlt.ecsv", "MJD_UTC whitelight_flux\n60000.1 60.0\n");
        let not_ecsv = whtlt_import_cmd(plain).await.unwrap_err();
        assert!(not_ecsv.contains("plain_whtlt.ecsv"), "{not_ecsv}");

        let catalogue = text_file(dir.path(), "stars.csv", "ra,dec\n10.5,-20.25\n");
        assert_eq!(
            whtlt_import_cmd(catalogue).await.unwrap_err(),
            "stars.csv: no whitelight_flux column: this is not a white-light table (columns: ra, dec)"
        );

        let spectra = three_integration_file(dir.path());
        let binary = whtlt_import_cmd(spectra.clone()).await.unwrap_err();
        assert!(binary.contains("t_x1dints.fits"), "{binary}");
        assert_eq!(tso_whtlt_compare_cmd(vec![spectra.clone()], spectra.clone()).await.unwrap_err(), binary);

        let absent = dir.path().join("absent_whtlt.ecsv").to_str().unwrap().to_string();
        let unread = tso_whtlt_compare_cmd(vec![spectra], absent).await.unwrap_err();
        assert!(unread.contains("absent_whtlt.ecsv"), "{unread}");
    }

    const A4: &str = "A4/jw04498-o013_t008_miri_p750l-slitlessprism_x1dints.fits";
    const A5_LEVEL_3: &str = "A5/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits";
    const A6: &str = "A6/jw04967-o005_t004_nirspec_clear-prism-s1600a1-sub512_x1dints.fits";
    const C_LEVEL_3: &str = "C/jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits";
    const WHITE_FIXTURE: &str = "tso_quicklook_e1_white.json";
    const BINS_FLUX_FIXTURE: &str = "tso_quicklook_e1_bins_flux.json";
    const BINS_ERR_FIXTURE: &str = "tso_quicklook_e1_bins_err.json";
    const ACCEPTANCE_REJECT_SIGMA: f64 = 100.0;

    fn quick_look_fixture(name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("tso").join(name);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("fixture {} unreadable: {error}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("fixture {} is not JSON: {error}", path.display()))
    }

    fn assert_same_entries(measured: &Value, expected: &Value, what: &str, fixture: &str) {
        let (measured, expected) = (measured.as_array().unwrap(), expected.as_array().unwrap());
        assert_eq!(measured.len(), expected.len(), "entries of {what}, {fixture}");
        if let Some(index) = (0..expected.len()).find(|&index| measured[index] != expected[index]) {
            panic!("{what}[{index}] is {} and {fixture} holds {}", measured[index], expected[index]);
        }
        println!("{what}: {} entries equal to those of {fixture}", expected.len());
    }

    fn assert_entries_within_sum_tolerance(measured: &Value, expected: &Value, what: &str, fixture: &str) {
        let (measured, expected) = (numbers(measured), finite_numbers(expected));
        assert_eq!(measured.len(), expected.len(), "entries of {what}, {fixture}");
        let mut largest = 0.0f64;
        for (index, (ours, &theirs)) in measured.into_iter().zip(&expected).enumerate() {
            let ours = ours.unwrap_or(f64::NAN);
            let relative = if ours == theirs { 0.0 } else { ((ours - theirs) / theirs).abs() };
            assert!(
                relative <= SUM_TOLERANCE,
                "{what}[{index}] is {ours:?} and {fixture} holds {theirs:?}: relative difference {relative:e} is above {SUM_TOLERANCE:e}"
            );
            largest = largest.max(relative);
        }
        println!("{what}: largest relative difference {largest:.1e} over {} values of {fixture}", expected.len());
    }

    #[tokio::test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/*"]
    async fn real_committed_quick_look_fixtures_equal_what_the_command_returns() {
        let white = quick_look_fixture(WHITE_FIXTURE);
        let bins_flux = quick_look_fixture(BINS_FLUX_FIXTURE);
        let bins_err = quick_look_fixture(BINS_ERR_FIXTURE);
        assert_eq!(bins_err["edges_um"], bins_flux["edges_um"], "edges_um of {BINS_ERR_FIXTURE} and of {BINS_FLUX_FIXTURE}");
        let bins: Vec<[f64; 2]> = finite_numbers(&bins_flux["edges_um"]).windows(2).map(|pair| [pair[0], pair[1]]).collect();
        assert_eq!(bins, [[0.6, 1.0], [1.0, 2.0], [2.0, 3.0], [3.0, 4.0], [4.0, 5.3]], "edges_um of {BINS_FLUX_FIXTURE}");
        assert_eq!(white["int_num"].as_array().unwrap().len(), 240, "integrations of {WHITE_FIXTURE}");

        let path = real(E1);
        let opened = tso_open_cmd(vec![path.clone()]).await.unwrap();
        assert_eq!(opened["files"][0]["sha256"], white["sha256"], "SHA-256 of the file the fixtures were made from, {WHITE_FIXTURE}");
        let key = SeriesKey { detector: "NRS1".into(), sporder: Some(0) };
        let binned = CurveRequest { bins: bins.clone(), ..request(CurveMode::Pipeline) };
        let curves = tso_light_curves_cmd(vec![path], key, binned).await.unwrap();
        for field in ["series", "mode", "time_scale", "flux_unit"] {
            assert_eq!(curves[field], white[field], "{field}, {WHITE_FIXTURE}");
        }
        assert_eq!(curves["white"]["limits_origin"], white["limits_origin"], "white.limits_origin, {WHITE_FIXTURE}");
        assert_same_entries(&curves["int_num"], &white["int_num"], "int_num", WHITE_FIXTURE);
        assert_same_entries(&curves["time"], &white["time"], "time", WHITE_FIXTURE);
        assert_same_entries(&curves["mjd_utc"], &white["mjd_utc"], "mjd_utc", WHITE_FIXTURE);
        assert_same_entries(&curves["white"]["n_used"], &white["n_used"], "white.n_used", WHITE_FIXTURE);
        assert_same_entries(&curves["white"]["band_um"], &white["limits_um"], "white.band_um", WHITE_FIXTURE);
        for field in ["flux", "flux_err", "npixels_mean"] {
            assert_entries_within_sum_tolerance(&curves["white"][field], &white[field], &format!("white.{field}"), WHITE_FIXTURE);
        }

        let binned_curves = curves["bins"].as_array().unwrap();
        assert_eq!(binned_curves.len(), bins.len(), "bins of {BINS_FLUX_FIXTURE}");
        for (position, (curve, band)) in binned_curves.iter().zip(&bins).enumerate() {
            assert_eq!(curve["band_um"], json!(band), "bins[{position}].band_um, {BINS_FLUX_FIXTURE}");
            let flux = format!("bins[{position}].flux");
            assert_entries_within_sum_tolerance(&curve["flux"], &bins_flux["flux"][position], &flux, BINS_FLUX_FIXTURE);
            let flux_err = format!("bins[{position}].flux_err");
            assert_entries_within_sum_tolerance(&curve["flux_err"], &bins_err["flux_err"][position], &flux_err, BINS_ERR_FIXTURE);
        }
    }

    type SeriesAcceptance = (&'static str, usize, usize, Vec<u64>, u64, bool);

    fn acceptance_table() -> Vec<(Vec<String>, &'static str, Vec<SeriesAcceptance>)> {
        let segments: Vec<String> =
            (1..=6).map(|number| format!("C/jw02327001001_03101_00001-seg{number:03}_nrs1_x1dints.fits")).collect();
        let orders = ["NIS, order 1", "NIS, order 2", "NIS, order 3"];
        vec![
            (vec![A1.into()], A1, vec![("NRCALONG", 1, 4, vec![], 1, true)]),
            (vec![A3.into()], A3, orders.into_iter().map(|label| (label, 1, 5, vec![], 1, true)).collect()),
            (vec![A4.into()], A4, vec![("MIRIMAGE", 64, 64, vec![], 64, true)]),
            (vec![A5_LEVEL_3.into()], A5_LEVEL_3, vec![("NRS1", 8, 8, vec![230], 8, true), ("NRS2", 8, 8, vec![], 8, true)]),
            (vec![A6.into()], A6, vec![("NRS1", 65, 65, vec![], 65, true)]),
            (vec![B1.into()], B1, vec![("MIRIMAGE", 10, 10, vec![], 10, true)]),
            (vec![C_LEVEL_3.into()], C_LEVEL_3, vec![("NRS1", 44, 44, vec![1289], 44, true)]),
            (vec![E1.into()], E1, vec![("NRS1", 240, 240, vec![], 240, true)]),
            (segments, C_LEVEL_3, vec![("NRS1", 44, 44, vec![1289], 44, false)]),
            (
                vec![A5_NRS1.into(), A5_NRS2.into()],
                A5_LEVEL_3,
                vec![("NRS1", 8, 8, vec![230], 8, true), ("NRS2", 8, 8, vec![], 8, false)],
            ),
        ]
    }

    fn whtlt_table_of(level_3: &str) -> String {
        let file_name = level_3.rsplit('/').next().unwrap();
        let stem = file_name.strip_suffix("_x1dints.fits").unwrap();
        real(&format!("whtlt/{stem}_whtlt.ecsv"))
    }

    fn integrations_of(curves: &Value, what: &str) -> usize {
        let integrations = curves["time"].as_array().unwrap().len();
        for field in ["mjd_utc", "int_num", "row_index"] {
            assert_eq!(curves[field].as_array().unwrap().len(), integrations, "{field}, {what}");
        }
        for field in ["flux", "flux_err", "n_used", "npixels_mean"] {
            assert_eq!(curves["white"][field].as_array().unwrap().len(), integrations, "white.{field}, {what}");
        }
        integrations
    }

    #[tokio::test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/*"]
    async fn real_every_set_opens_gives_curves_in_both_modes_and_matches_its_archived_white_light() {
        for (files, level_3, expected) in acceptance_table() {
            let name = format!("{} ({} file(s))", files[0], files.len());
            let paths: Vec<String> = files.iter().map(|file| real(file)).collect();
            let whtlt = whtlt_table_of(level_3);

            let started = Instant::now();
            let opened = tso_open_cmd(paths.clone()).await.unwrap_or_else(|refusal| panic!("{name}: tso_open_cmd: {refusal}"));
            let open_time = started.elapsed();
            let started = Instant::now();
            let compared = tso_whtlt_compare_cmd(paths.clone(), whtlt)
                .await
                .unwrap_or_else(|refusal| panic!("{name}: tso_whtlt_compare_cmd: {refusal}"));
            let compare_time = started.elapsed();

            let set_id = opened["set_id"].as_str().unwrap();
            let series = opened["series"].as_array().unwrap();
            let labels: Vec<&str> = series.iter().map(|summary| summary["label"].as_str().unwrap()).collect();
            assert_eq!(labels, expected.iter().map(|row| row.0).collect::<Vec<_>>(), "series of {name}");
            let entries = compared.as_array().unwrap();
            assert_eq!(entries.len(), series.len(), "whtlt columns compared with {name}");

            for (summary, row) in series.iter().zip(expected) {
                let (label, kept_by_pipeline, kept_by_masked, rejected_channels, whtlt_rows, within_1e4) = row;
                let what = format!("{name}, {label}");
                let key: SeriesKey = serde_json::from_value(summary["key"].clone()).unwrap();

                let started = Instant::now();
                let pipeline = tso_light_curves_cmd(paths.clone(), key.clone(), request(CurveMode::Pipeline))
                    .await
                    .unwrap_or_else(|refusal| panic!("{what}: tso_light_curves_cmd, pipeline mode: {refusal}"));
                let pipeline_time = started.elapsed();
                let rejecting = CurveRequest { reject_elements_sigma: Some(ACCEPTANCE_REJECT_SIGMA), ..request(CurveMode::Masked) };
                let started = Instant::now();
                let masked = tso_light_curves_cmd(paths.clone(), key.clone(), rejecting)
                    .await
                    .unwrap_or_else(|refusal| panic!("{what}: tso_light_curves_cmd, masked mode: {refusal}"));
                let masked_time = started.elapsed();
                let started = Instant::now();
                let median = tso_spectrum_cmd(paths.clone(), key, TsoPick::Median)
                    .await
                    .unwrap_or_else(|refusal| panic!("{what}: tso_spectrum_cmd, median: {refusal}"));
                let median_time = started.elapsed();

                let kept = integrations_of(&pipeline, &format!("{what}, pipeline mode"));
                assert_eq!(kept, kept_by_pipeline, "integrations kept by the pipeline mode, {what}");
                let kept = integrations_of(&masked, &format!("{what}, masked mode"));
                assert_eq!(kept, kept_by_masked, "integrations kept by the masked mode, {what}");
                let mut summed = integers(&masked["white"]["n_used"]);
                summed.dedup();
                assert_eq!(summed.len(), 1, "elements summed per integration in masked mode, {what}: {summed:?}");
                let rejected: Vec<u64> = masked["rejected_elements"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|element| element["index"].as_u64().unwrap())
                    .collect();
                assert_eq!(rejected, rejected_channels, "channels rejected at {ACCEPTANCE_REJECT_SIGMA} robust sigma, {what}");
                let channels = median["flux"].as_array().unwrap().len();
                assert_eq!(json!(channels), summary["n_channels"], "channels of the median spectrum, {what}");

                let matching: Vec<&Value> = entries.iter().filter(|entry| entry["series"] == summary["key"]).collect();
                assert_eq!(matching.len(), 1, "whtlt columns compared with {what}");
                let entry = matching[0];
                assert_eq!(entry["n_whtlt"], whtlt_rows, "rows of the whtlt column {}, {what}", entry["column"]);
                assert_eq!(entry["n_joined"], whtlt_rows, "whtlt rows joined on time, {what}");
                let max_rel_diff = entry["max_rel_diff"].as_f64().unwrap_or(f64::NAN);
                assert_eq!(entry["within_1e4"], within_1e4, "within 1e-4 of the archived white light, {what}: max_rel_diff {max_rel_diff:e}");

                println!(
                    "{what}: set_id {set_id}; tso_open_cmd {open_time:.1?}; pipeline mode {kept_by_pipeline} integration(s) in {pipeline_time:.1?}; masked mode {kept_by_masked} integration(s) of {} element(s), rejected at {ACCEPTANCE_REJECT_SIGMA} robust sigma {rejected:?}, in {masked_time:.1?}; median of {channels} channel(s) in {median_time:.1?}; {} joined {} of {} row(s), max_rel_diff {max_rel_diff:e}, within 1e-4 {}, tso_whtlt_compare_cmd {compare_time:.1?}",
                    summed[0],
                    entry["column"].as_str().unwrap(),
                    entry["n_joined"],
                    entry["n_whtlt"],
                    entry["within_1e4"]
                );
            }
        }
    }
}
