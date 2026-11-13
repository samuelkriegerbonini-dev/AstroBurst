use std::path::Path;
use std::time::Instant;

use anyhow::{anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::cmd::analysis::{
    forced_aperture_checked, forced_band, photometry_context, ForcedAperture, ForcedBand, ForcedBandInput,
    ForcedBandResult, ForcedCorrection, ForcedOptions, ForcedSkyPosition,
};
use crate::cmd::common::{blocking_cmd, source_path};
use crate::core::analysis::forced::FORCED_SUBSAMPLES;
use crate::core::astrometry::catalog_compare::{
    aperture_inside, catalogue_positions, evaluate_catalogue, resolve_presets, summarise, validate_selection,
    CompareAperture, CompareCut, CompareMapping, CompareRow, CompareSelection, Measured, RejectedCount, ResolvedPreset,
};
use crate::infra::tables::{read_text_table, TableFormat, TextTable};

const CATALOGUE_TABLE_EXTENSIONS: [&str; 2] = ["csv", "ecsv"];
const NO_PRESET_NOTE: &str = "no preset matches the columns of this table: choose the columns by hand";
const RATIO_NOTE: &str = "ratio = measured / catalogue, both in Jy; the statistics are the median and 1.4826 x MAD of the rows used; the error of the median is sqrt(pi/2) x 1.4826 x MAD / sqrt(N)";
const FORMAL_ERROR_NOTE: &str = "formal error of the app: sqrt(sum (w sigma)^2) over the aperture; catalogues that use sqrt(sum w sigma^2), such as Legacy Surveys and DES, are 1.3 to 4.4 percent higher at radii of 13 to 4 px";
const POSITIONS_NOTE: &str =
    "positions are the catalogue's, read as ICRS degrees and placed through the WCS of the image; nothing is recentred";
const EXCLUDED_FLAGS_NOTE: &str =
    "flagged pixels are excluded: an aperture that holds one counts as touching pixels without data";

#[derive(Serialize)]
struct ProbeColumn<'a> {
    name: &'a str,
    numeric: bool,
    unit: Option<&'a str>,
}

#[derive(Serialize)]
struct CompareProbe<'a> {
    table_path: &'a str,
    file_name: String,
    format: TableFormat,
    n_rows: usize,
    columns: Vec<ProbeColumn<'a>>,
    presets: Vec<ResolvedPreset>,
    notes: Vec<String>,
}

pub(crate) fn read_catalogue_table(table_path: &str) -> anyhow::Result<(String, TextTable)> {
    let file_name = table_path.rsplit(['\\', '/']).next().unwrap_or(table_path).to_string();
    let compared = file_name
        .rsplit_once('.')
        .is_some_and(|(_, extension)| CATALOGUE_TABLE_EXTENSIONS.iter().any(|known| extension.eq_ignore_ascii_case(known)));
    if !compared {
        bail!("catalogue table {file_name}: only CSV and ECSV tables can be compared; save the catalogue as CSV");
    }
    let table = read_text_table(table_path).map_err(|error| anyhow!("catalogue table {file_name}: {error:#}"))?;
    Ok((file_name, table))
}

pub(crate) fn compare_probe_json(table_path: &str) -> anyhow::Result<serde_json::Value> {
    let (file_name, table) = read_catalogue_table(table_path)?;
    let names: Vec<String> = table.columns.iter().map(|column| column.name.clone()).collect();
    let presets = resolve_presets(&names);
    let mut notes = table.notes.clone();
    if presets.is_empty() {
        notes.push(NO_PRESET_NOTE.to_string());
    }
    let probe = CompareProbe {
        table_path,
        file_name,
        format: table.format,
        n_rows: table.n_rows,
        columns: table
            .columns
            .iter()
            .map(|column| ProbeColumn { name: &column.name, numeric: column.is_numeric(), unit: column.unit.as_deref() })
            .collect(),
        presets,
        notes,
    };
    Ok(serde_json::to_value(&probe)?)
}

#[tauri::command]
pub async fn catalog_compare_probe_cmd(table_path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!(compare_probe_json(&table_path))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareRequest {
    pub band: ForcedBandInput,
    pub table_path: String,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub catalogue_band: Option<String>,
    pub mapping: CompareMapping,
    pub aperture: CompareAperture,
    #[serde(default)]
    pub cuts: Vec<CompareCut>,
    #[serde(default)]
    pub min_snr: Option<f64>,
    #[serde(default)]
    pub max_snr: Option<f64>,
    #[serde(default)]
    pub min_neighbour_arcsec: Option<f64>,
    #[serde(default)]
    pub exclude_dq: Option<bool>,
}

#[derive(Serialize)]
struct ApertureUsed {
    radius_px: f64,
    radius_arcsec: f64,
    annulus_px: Option<[f64; 2]>,
    pixel_scale_arcsec: f64,
    local_sky: bool,
}

#[derive(Serialize)]
struct ComparedTable<'a> {
    path: &'a str,
    file_name: String,
    format: TableFormat,
    n_rows: usize,
}

#[derive(Serialize)]
struct CompareReport<'a> {
    n_table: usize,
    n_on_image: usize,
    n_after_cuts: usize,
    n_measured: usize,
    n_used: usize,
    ratio_median: Option<f64>,
    ratio_nmad: Option<f64>,
    ratio_median_err: Option<f64>,
    n_ratio: usize,
    ratio_offset_mag: Option<f64>,
    err_ratio_median: Option<f64>,
    err_ratio_nmad: Option<f64>,
    n_err: usize,
    implied_zero_point_ab: Option<f64>,
    implied_zero_point_nmad: Option<f64>,
    n_zero_point: usize,
    rows: Vec<CompareRow>,
    rejected: Vec<RejectedCount>,
    aperture: ApertureUsed,
    band: ForcedBand,
    calibration_origin: Option<String>,
    preset: Option<&'a str>,
    catalogue_band: Option<&'a str>,
    table: ComparedTable<'a>,
    notes: Vec<String>,
    warnings: Vec<String>,
    elapsed_ms: u64,
}

fn image_file_name(path: &str) -> String {
    let source = source_path(path);
    Path::new(&source).file_name().and_then(|name| name.to_str()).map_or_else(|| source.clone(), str::to_string)
}

fn on_pixel_grid(x: f64, y: f64, cols: usize, rows: usize) -> bool {
    x.is_finite() && y.is_finite() && x >= -0.5 && x < cols as f64 - 0.5 && y >= -0.5 && y < rows as f64 - 0.5
}

fn placed_at(x: f64, y: f64, reach_px: f64, cols: usize, rows: usize) -> Measured {
    Measured {
        x,
        y,
        on_image: true,
        inside: aperture_inside(x, y, reach_px, cols, rows),
        error: None,
        flux_native: None,
        flux_err_native: None,
        flux_jy: None,
        flux_err_jy: None,
        masked_fraction: None,
        saturated: false,
    }
}

fn aperture_note(aperture: &ApertureUsed) -> String {
    let circle = format!(
        "aperture: circle of radius {:.2} px ({:.2} arcsec at {:.4} arcsec per px), {FORCED_SUBSAMPLES} x {FORCED_SUBSAMPLES} sub-pixel weights",
        aperture.radius_px, aperture.radius_arcsec, aperture.pixel_scale_arcsec
    );
    match aperture.annulus_px {
        Some([inner, outer]) => format!("{circle}, local sky from the annulus {inner:.2} to {outer:.2} px"),
        None => format!("{circle}, no local sky subtracted"),
    }
}

pub(crate) fn compare_json(request: &CompareRequest) -> anyhow::Result<serde_json::Value> {
    let started = Instant::now();
    let requested = &request.aperture;
    let aperture = ForcedAperture {
        radius_arcsec: requested.radius_arcsec,
        radius_px: requested.radius_px,
        annulus_arcsec: requested.annulus_arcsec,
        annulus_px: requested.annulus_px,
        correction: ForcedCorrection::None,
        reference_radius_arcsec: None,
    };
    forced_aperture_checked(&aperture)?;
    let (file_name, table) = read_catalogue_table(&request.table_path)?;
    let selection = CompareSelection {
        mapping: request.mapping.clone(),
        cuts: request.cuts.clone(),
        min_snr: request.min_snr,
        max_snr: request.max_snr,
        min_neighbour_arcsec: request.min_neighbour_arcsec,
    };
    validate_selection(&table, &selection).map_err(anyhow::Error::msg)?;

    let ctx = photometry_context(&request.band.path, false)?;
    let wcs = ctx.wcs.as_ref().ok_or_else(|| {
        anyhow!(
            "{} has no celestial WCS: catalogue positions cannot be placed on it",
            image_file_name(&request.band.path)
        )
    })?;
    let (rows, cols) = ctx.entry.arr().dim();
    let sky: Vec<(f64, f64)> = catalogue_positions(&table, &selection.mapping)
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .map(|position| position.unwrap_or((f64::NAN, f64::NAN)))
        .collect();
    let pixels = wcs.world_to_pixel_batch(&sky);
    let on_image: Vec<bool> = pixels.iter().map(|&(x, y)| on_pixel_grid(x, y, cols, rows)).collect();
    let side = evaluate_catalogue(&table, &selection, &on_image).map_err(anyhow::Error::msg)?;

    let positions: Vec<ForcedSkyPosition> = side
        .entries
        .iter()
        .filter(|entry| entry.rejection.is_none())
        .map(|entry| ForcedSkyPosition { id: entry.row.to_string(), ra_deg: entry.ra_deg, dec_deg: entry.dec_deg })
        .collect();
    let exclude_dq = request.exclude_dq.unwrap_or(false);
    let options = ForcedOptions { exclude_dq, ..ForcedOptions::default() };
    let ForcedBandResult { band, cells } = forced_band(&request.band, &positions, &aperture, &options)?;
    let (Some(radius_px), Some(pixel_scale_arcsec)) = (band.aperture_radius_px, band.pixel_scale_arcsec) else {
        bail!("{}: the measurement returned no aperture radius", band.file_name);
    };
    let reach_px = band.annulus_px.map_or(radius_px, |[_, outer]| outer);

    let mut cells = cells.into_iter();
    let measured: Vec<Option<Measured>> = side
        .entries
        .iter()
        .map(|entry| match entry.rejection {
            Some(_) => {
                let (x, y) = pixels[entry.row - 1];
                Some(placed_at(x, y, reach_px, cols, rows))
            }
            None => cells.next().map(|cell| Measured {
                on_image: cell.on_image,
                error: cell.error,
                flux_native: cell.flux_native,
                flux_err_native: cell.flux_err_native,
                flux_jy: cell.flux_jy,
                flux_err_jy: cell.flux_err_jy,
                masked_fraction: cell.masked_fraction,
                saturated: cell.saturated,
                ..placed_at(cell.x, cell.y, reach_px, cols, rows)
            }),
        })
        .collect();
    let summary = summarise(side, &measured).map_err(anyhow::Error::msg)?;

    let aperture_used = ApertureUsed {
        radius_px,
        radius_arcsec: radius_px * pixel_scale_arcsec,
        annulus_px: band.annulus_px,
        pixel_scale_arcsec,
        local_sky: band.annulus_px.is_some(),
    };
    let mut notes = vec![
        RATIO_NOTE.to_string(),
        aperture_note(&aperture_used),
        FORMAL_ERROR_NOTE.to_string(),
        POSITIONS_NOTE.to_string(),
    ];
    if exclude_dq {
        notes.push(EXCLUDED_FLAGS_NOTE.to_string());
    }
    if let Some(class) = &request.mapping.class {
        notes.push(format!("class is the catalogue column {class}: the catalogue's own value, not a resolution verdict"));
    }

    let mut warnings = summary.warnings;
    warnings.extend(band.warnings.iter().cloned());
    if let (Some(catalogue_band), Some(filter)) = (&request.catalogue_band, &band.filter) {
        if !catalogue_band.eq_ignore_ascii_case(&filter.filter) {
            warnings.push(format!(
                "the catalogue band is {catalogue_band} and the image filter is {} ({}): the ratio compares different bands",
                filter.filter, filter.source
            ));
        }
    }

    let report = CompareReport {
        n_table: summary.n_table,
        n_on_image: summary.n_on_image,
        n_after_cuts: summary.n_after_cuts,
        n_measured: summary.n_measured,
        n_used: summary.n_used,
        ratio_median: summary.ratio.map(|stat| stat.median),
        ratio_nmad: summary.ratio.map(|stat| stat.nmad),
        ratio_median_err: summary.ratio.and_then(|stat| stat.median_err),
        n_ratio: summary.ratio.map_or(0, |stat| stat.n),
        ratio_offset_mag: summary.ratio_offset_mag,
        err_ratio_median: summary.err_ratio.map(|stat| stat.median),
        err_ratio_nmad: summary.err_ratio.map(|stat| stat.nmad),
        n_err: summary.err_ratio.map_or(0, |stat| stat.n),
        implied_zero_point_ab: summary.implied_zero_point_ab.map(|stat| stat.median),
        implied_zero_point_nmad: summary.implied_zero_point_ab.map(|stat| stat.nmad),
        n_zero_point: summary.implied_zero_point_ab.map_or(0, |stat| stat.n),
        rows: summary.rows,
        rejected: summary.rejected,
        aperture: aperture_used,
        calibration_origin: band.calibration_origin.clone(),
        band,
        preset: request.preset.as_deref(),
        catalogue_band: request.catalogue_band.as_deref(),
        table: ComparedTable { path: &request.table_path, file_name, format: table.format, n_rows: table.n_rows },
        notes,
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    Ok(serde_json::to_value(&report)?)
}

#[tauri::command]
pub async fn catalog_compare_cmd(request: CompareRequest) -> Result<serde_json::Value, String> {
    blocking_cmd!(compare_json(&request))
}

#[cfg(test)]
mod tests {
    use ndarray::Array2;
    use serde_json::json;

    use super::*;
    use crate::cmd::companions::set_companion_json;
    use crate::core::astrometry::catalog_compare::{CatalogueFluxUnit, CutOp, CutValue};
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::infra::fits::reader::test_fixtures::{write_test_mef, HduData, TestHdu};
    use crate::infra::fits::writer::write_fits_mono;

    const SCIENCE_DATA_DIR_VAR: &str = "ASTROBURST_SCIENCE_DATA_DIR";
    const LEGACY_HEADER: &str = "ls_id,ra,dec,type,maskbits,fracflux_r,apflux_r_5,apflux_ivar_r_5\n";
    const LEGACY_ROWS: &str = "10995300315432397,150.0,2.0,PSF,0,0.001,79.09336,1275.3608\n\
10995300315432398,150.001,2.0,REX,0,0.002,12.5,300.0\n";
    const SAVED_CATALOGUES: [(&str, u64, usize, &[(&str, &str)]); 6] = [
        ("survey/catalogs/C1_ls_dr10_box_A.csv", 297, 17, &[("legacy_dr10", "r")]),
        (
            "survey/catalogs/C1b_ls_dr10_brick_0532m280.csv",
            69,
            30,
            &[("legacy_dr10", "g"), ("legacy_dr10", "r"), ("legacy_dr10", "z")],
        ),
        ("survey/catalogs/C2_des_dr2_box_A.csv", 148, 18, &[("des_dr2", "r")]),
        ("survey/catalogs/C3_euclid_q1_mer_cone_A.csv", 692, 21, &[("euclid_q1", "VIS")]),
        ("survey/S1/C4_sdss_dr17_field_1462_4_165.csv", 808, 16, &[("sdss_dr17", "r")]),
        ("survey/P1/C5_ps1_dr2_cone_A.csv", 75, 13, &[("ps1_dr2", "r")]),
    ];
    const PRESET_CUT_COUNTS: [(&str, usize); 5] =
        [("legacy_dr10", 3), ("des_dr2", 2), ("sdss_dr17", 4), ("ps1_dr2", 1), ("euclid_q1", 2)];
    const PRESET_ERROR_KEYS: [(&str, &str); 3] =
        [("legacy_dr10", "flux_ivar"), ("des_dr2", "flux_err"), ("ps1_dr2", "flux_err")];
    const NUMERIC_COLUMNS: [(&str, &str, bool); 9] = [
        ("survey/catalogs/C1_ls_dr10_box_A.csv", "ls_id", false),
        ("survey/catalogs/C1b_ls_dr10_brick_0532m280.csv", "ls_id", false),
        ("survey/catalogs/C1b_ls_dr10_brick_0532m280.csv", "type", false),
        ("survey/catalogs/C1b_ls_dr10_brick_0532m280.csv", "brickname", false),
        ("survey/catalogs/C2_des_dr2_box_A.csv", "coadd_object_id", true),
        ("survey/catalogs/C3_euclid_q1_mer_cone_A.csv", "object_id", false),
        ("survey/S1/C4_sdss_dr17_field_1462_4_165.csv", "objID", false),
        ("survey/S1/C4_sdss_dr17_field_1462_4_165.csv", "type", true),
        ("survey/P1/C5_ps1_dr2_cone_A.csv", "objid", false),
    ];

    fn write_text(dir: &tempfile::TempDir, name: &str, text: &str) -> String {
        let path = dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn presets_of(value: &serde_json::Value) -> Vec<(&str, &str)> {
        value["presets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|preset| (preset["id"].as_str().unwrap(), preset["band"].as_str().unwrap()))
            .collect()
    }

    fn numeric_of<'a>(value: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
        let columns = value["columns"].as_array().unwrap();
        let column = columns.iter().find(|column| column["name"] == name).unwrap_or_else(|| panic!("no column {name}"));
        &column["numeric"]
    }

    #[test]
    fn the_probe_lists_the_columns_and_the_presets_of_a_saved_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_text(&dir, "cat.csv", &format!("{LEGACY_HEADER}{LEGACY_ROWS}"));
        let value = compare_probe_json(&path).unwrap();
        assert_eq!(value["table_path"], path.as_str());
        assert_eq!(value["file_name"], "cat.csv");
        assert_eq!(value["format"], "csv");
        assert_eq!(value["n_rows"], 2);
        assert_eq!(
            value["columns"],
            json!([
                {"name": "ls_id", "numeric": false, "unit": null},
                {"name": "ra", "numeric": true, "unit": null},
                {"name": "dec", "numeric": true, "unit": null},
                {"name": "type", "numeric": false, "unit": null},
                {"name": "maskbits", "numeric": true, "unit": null},
                {"name": "fracflux_r", "numeric": true, "unit": null},
                {"name": "apflux_r_5", "numeric": true, "unit": null},
                {"name": "apflux_ivar_r_5", "numeric": true, "unit": null},
            ])
        );
        assert_eq!(presets_of(&value), vec![("legacy_dr10", "r")]);
        assert_eq!(value["presets"][0]["mapping"]["flux_ivar"], "apflux_ivar_r_5");
        assert_eq!(value["presets"][0]["cuts"][0], json!({"column": "type", "op": "==", "value": "PSF"}));
        assert_eq!(value["notes"], json!(read_text_table(&path).unwrap().notes));
        assert_eq!(value.as_object().unwrap().len(), 7);

        let (file_name, table) = read_catalogue_table(&path).unwrap();
        assert_eq!(file_name, "cat.csv");
        assert_eq!(table.n_rows, 2);

        let unit_path = write_text(&dir, "units.CSV", &format!("# unit ra: deg\n{LEGACY_HEADER}{LEGACY_ROWS}"));
        let with_unit = compare_probe_json(&unit_path).unwrap();
        assert_eq!(with_unit["file_name"], "units.CSV");
        assert_eq!(with_unit["columns"][1], json!({"name": "ra", "numeric": true, "unit": "deg"}));
    }

    #[test]
    fn the_probe_refuses_what_it_cannot_compare() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            compare_probe_json(&dir.path().join("cat.fits").to_str().unwrap().to_string()).unwrap_err().to_string(),
            "catalogue table cat.fits: only CSV and ECSV tables can be compared; save the catalogue as CSV"
        );
        assert_eq!(
            format!("{:#}", read_catalogue_table("C:/d\\sub/catalogue").unwrap_err()),
            "catalogue table catalogue: only CSV and ECSV tables can be compared; save the catalogue as CSV"
        );
        let gone = format!("{:#}", compare_probe_json(&dir.path().join("gone.csv").to_str().unwrap().to_string()).unwrap_err());
        assert!(gone.starts_with("catalogue table gone.csv: "), "{gone}");

        let header_only = compare_probe_json(&write_text(&dir, "empty.csv", LEGACY_HEADER)).unwrap();
        assert_eq!(header_only["n_rows"], 0);
        assert_eq!(presets_of(&header_only), vec![("legacy_dr10", "r")]);
        let notes = header_only["notes"].as_array().unwrap();
        assert!(notes.iter().any(|note| note.as_str().unwrap().contains("no data rows")), "{notes:?}");
        assert!(!notes.iter().any(|note| note == NO_PRESET_NOTE));

        let plain = compare_probe_json(&write_text(&dir, "plain.csv", "a,b\n1,2\n")).unwrap();
        assert_eq!(plain["presets"], json!([]));
        assert_eq!(plain["notes"], json!([NO_PRESET_NOTE]));
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/catalogs, survey/S1, survey/P1 (CSV)"]
    fn real_presets_resolve_on_the_six_saved_catalogues() {
        let data_dir = env_dir(SCIENCE_DATA_DIR_VAR);
        let probes: Vec<(&str, serde_json::Value)> = SAVED_CATALOGUES
            .iter()
            .map(|(file, ..)| (*file, compare_probe_json(data_dir.join(file).to_str().unwrap()).unwrap()))
            .collect();
        for (file, value) in &probes {
            println!(
                "{file}: {} rows, {} columns, presets {:?}, notes {}",
                value["n_rows"],
                value["columns"].as_array().unwrap().len(),
                presets_of(value),
                value["notes"]
            );
        }

        for ((file, value), (_, n_rows, n_columns, presets)) in probes.iter().zip(SAVED_CATALOGUES) {
            assert_eq!(value["format"], "csv", "{file}");
            assert_eq!(value["n_rows"], n_rows, "{file}");
            assert_eq!(value["columns"].as_array().unwrap().len(), n_columns, "{file}");
            assert_eq!(presets_of(value), presets.to_vec(), "{file}");
            for preset in value["presets"].as_array().unwrap() {
                let id = preset["id"].as_str().unwrap();
                let notes = preset["notes"].as_array().unwrap();
                assert!(!notes.iter().any(|note| note.as_str().unwrap().contains("left out")), "{file} {id}: {notes:?}");
                let (_, n_cuts) = PRESET_CUT_COUNTS.iter().find(|(preset_id, _)| *preset_id == id).unwrap();
                assert_eq!(preset["cuts"].as_array().unwrap().len(), *n_cuts, "{file} {id}");
                if let Some((_, key)) = PRESET_ERROR_KEYS.iter().find(|(preset_id, _)| *preset_id == id) {
                    assert!(preset["mapping"][key].is_string(), "{file} {id}: {key}");
                }
            }
        }

        for (file, column, numeric) in NUMERIC_COLUMNS {
            let (_, value) = probes.iter().find(|(name, _)| *name == file).unwrap();
            assert_eq!(numeric_of(value, column), numeric, "{file} {column}");
        }
    }

    const FIELD_SIZE: usize = 200;
    const STAR_SIGMA_PX: f64 = 2.5;
    const FIELD_STARS: [(f64, f64, f64); 6] = [
        (40.0, 50.0, 400.0),
        (120.3, 60.7, 250.0),
        (160.0, 150.5, 150.0),
        (70.25, 140.75, 90.0),
        (100.0, 100.0, 60.0),
        (30.5, 170.5, 40.0),
    ];
    const STAR_IDS: [&str; 6] = ["s1", "s2", "s3", "s4", "s5", "s6"];
    const CATALOGUE_APFLUX: [f64; 6] = [
        15614.09211982579,
        9758.807574891118,
        5855.284544934671,
        3513.1707269608023,
        2342.113817973868,
        1561.409211982579,
    ];
    const CATALOGUE_APFLUX_ERR: f64 = 7.0898154036220635;
    const ORACLE_RATIO_MEDIAN: f64 = 0.99958;
    const ORACLE_ERR_RATIO_MEDIAN: f64 = 0.97918;
    const ORACLE_ZERO_POINT_AB: f64 = 22.49954;
    const S1_FLUX_CAT_JY: f64 = 0.05669134193867522;
    const MEDIAN_ERR_PER_NMAD: f64 = 1.2533141373155001;
    const NOTE_RATIO: &str = "ratio = measured / catalogue, both in Jy; the statistics are the median and 1.4826 x MAD of the rows used; the error of the median is sqrt(pi/2) x 1.4826 x MAD / sqrt(N)";
    const NOTE_APERTURE_8_PX: &str = "aperture: circle of radius 8.00 px (4.00 arcsec at 0.5000 arcsec per px), 5 x 5 sub-pixel weights, no local sky subtracted";
    const NOTE_ANNULUS_20_30_PX: &str = "aperture: circle of radius 8.00 px (4.00 arcsec at 0.5000 arcsec per px), 5 x 5 sub-pixel weights, local sky from the annulus 20.00 to 30.00 px";
    const NOTE_FORMAL_ERROR: &str = "formal error of the app: sqrt(sum (w sigma)^2) over the aperture; catalogues that use sqrt(sum w sigma^2), such as Legacy Surveys and DES, are 1.3 to 4.4 percent higher at radii of 13 to 4 px";
    const NOTE_POSITIONS: &str = "positions are the catalogue's, read as ICRS degrees and placed through the WCS of the image; nothing is recentred";
    const NOTE_EXCLUDED_FLAGS: &str =
        "flagged pixels are excluded: an aperture that holds one counts as touching pixels without data";
    const NOTE_CLASS_TYPE: &str = "class is the catalogue column type: the catalogue's own value, not a resolution verdict";
    const WARNING_NO_JY: &str = "the image has no flux calibration in Jy: the ratio is not available; the zero point implied by the catalogue is given instead";
    const EDGE_REASON: &str = "aperture crosses the image edge";

    struct TableRow {
        id: &'static str,
        sky: (f64, f64),
        apflux: Option<f64>,
        class: &'static str,
    }

    fn field_pixels() -> Vec<f32> {
        (0..FIELD_SIZE * FIELD_SIZE)
            .map(|index| {
                let (column, row) = ((index % FIELD_SIZE) as f64, (index / FIELD_SIZE) as f64);
                FIELD_STARS
                    .iter()
                    .map(|&(x, y, amplitude)| {
                        let r2 = (column - x).powi(2) + (row - y).powi(2);
                        amplitude * (-r2 / (2.0 * STAR_SIGMA_PX * STAR_SIGMA_PX)).exp()
                    })
                    .sum::<f64>() as f32
            })
            .collect()
    }

    fn write_field(dir: &Path, name: &str, err: Option<f32>, extra: Vec<(&'static str, String)>) -> String {
        let mut cards = vec![
            ("CTYPE1", "'RA---TAN'".to_string()),
            ("CTYPE2", "'DEC--TAN'".to_string()),
            ("CRPIX1", format!("{:E}", 100.5)),
            ("CRPIX2", format!("{:E}", 100.5)),
            ("CRVAL1", format!("{:E}", 150.0)),
            ("CRVAL2", format!("{:E}", 2.0)),
            ("CD1_1", format!("{:E}", -0.5 / 3600.0)),
            ("CD1_2", format!("{:E}", 0.0)),
            ("CD2_1", format!("{:E}", 0.0)),
            ("CD2_2", format!("{:E}", 0.5 / 3600.0)),
            ("SATURATE", format!("{:E}", 100000.0)),
        ];
        cards.extend(extra);
        let layer = |extname: &'static str, data: HduData, extra_cards: Vec<(&'static str, String)>| TestHdu {
            extname: Some(extname),
            extver: Some(1),
            cols: FIELD_SIZE,
            rows: FIELD_SIZE,
            data,
            extra_cards,
        };
        let mut hdus = vec![layer("SCI", HduData::F32(field_pixels()), cards)];
        if let Some(sigma) = err {
            hdus.push(layer("ERR", HduData::F32(vec![sigma; FIELD_SIZE * FIELD_SIZE]), vec![]));
        }
        let path = dir.join(name);
        write_test_mef(&path, &[], &hdus);
        format!("{}#hdu=1", path.to_str().unwrap())
    }

    fn nanomaggy_cards() -> Vec<(&'static str, String)> {
        vec![("BUNIT", "'nanomaggy'".into()), ("MAGZERO", "22.5".into())]
    }

    fn sky_at(key: &str, x: f64, y: f64) -> (f64, f64) {
        let coord = photometry_context(key, false).unwrap().wcs.unwrap().pixel_to_world(x, y);
        (coord.ra, coord.dec)
    }

    fn field_rows(key: &str) -> Vec<TableRow> {
        let mut rows: Vec<TableRow> = FIELD_STARS
            .iter()
            .zip(CATALOGUE_APFLUX)
            .zip(STAR_IDS)
            .map(|((&(x, y, _), apflux), id)| TableRow { id, sky: sky_at(key, x, y), apflux: Some(apflux), class: "PSF" })
            .collect();
        rows.extend([
            TableRow { id: "far", sky: (150.0, 3.0), apflux: Some(100.0), class: "PSF" },
            TableRow { id: "rex", sky: sky_at(key, 100.0, 30.0), apflux: Some(100.0), class: "REX" },
            TableRow { id: "blank", sky: sky_at(key, 130.0, 130.0), apflux: None, class: "PSF" },
            TableRow { id: "edge", sky: sky_at(key, 3.0, 100.0), apflux: Some(10.0), class: "PSF" },
        ]);
        rows
    }

    fn write_rows(dir: &Path, rows: &[TableRow]) -> String {
        let mut text = String::from("id,ra,dec,apflux,apflux_err,type\n");
        for row in rows {
            let apflux = row.apflux.map_or_else(String::new, |flux| flux.to_string());
            text.push_str(&format!(
                "{},{:.13},{:.13},{apflux},{CATALOGUE_APFLUX_ERR},{}\n",
                row.id, row.sky.0, row.sky.1, row.class
            ));
        }
        let path = dir.join("cat.csv");
        std::fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn write_catalogue(dir: &Path, key: &str) -> String {
        write_rows(dir, &field_rows(key))
    }

    fn request(key: &str, table: &str) -> CompareRequest {
        CompareRequest {
            band: ForcedBandInput { path: key.to_string(), label: None, dataset: None },
            table_path: table.to_string(),
            preset: None,
            catalogue_band: None,
            mapping: CompareMapping {
                id: Some("id".into()),
                ra: "ra".into(),
                dec: "dec".into(),
                flux: "apflux".into(),
                flux_unit: CatalogueFluxUnit::Nanomaggy,
                zero_point: None,
                flux_err: Some("apflux_err".into()),
                flux_ivar: None,
                class: Some("type".into()),
                missing_value: None,
            },
            aperture: CompareAperture { radius_px: Some(8.0), ..CompareAperture::default() },
            cuts: vec![CompareCut { column: "type".into(), op: CutOp::Eq, value: CutValue::Text("PSF".into()) }],
            min_snr: None,
            max_snr: None,
            min_neighbour_arcsec: None,
            exclude_dq: None,
        }
    }

    fn compared(request: &CompareRequest) -> serde_json::Value {
        compare_json(request).unwrap()
    }

    fn refusal(request: &CompareRequest) -> String {
        format!("{:#}", compare_json(request).unwrap_err())
    }

    fn number(value: &serde_json::Value) -> f64 {
        value.as_f64().unwrap_or_else(|| panic!("not a number: {value}"))
    }

    fn row_named<'a>(report: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
        report["rows"].as_array().unwrap().iter().find(|row| row["id"] == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    fn texts(value: &serde_json::Value) -> Vec<String> {
        value.as_array().unwrap().iter().map(|text| text.as_str().unwrap().to_string()).collect()
    }

    fn calibrated_field(dir: &Path) -> (String, String) {
        let key = write_field(dir, "field.fits", Some(0.5), nanomaggy_cards());
        let table = write_catalogue(dir, &key);
        (key, table)
    }

    #[test]
    fn the_report_compares_the_app_aperture_with_the_catalogue_column() {
        let dir = tempfile::tempdir().unwrap();
        let (key, table) = calibrated_field(dir.path());
        let report = compared(&request(&key, &table));

        for (member, count) in [
            ("n_table", 10),
            ("n_on_image", 9),
            ("n_after_cuts", 7),
            ("n_measured", 6),
            ("n_used", 6),
            ("n_ratio", 6),
            ("n_err", 6),
            ("n_zero_point", 6),
        ] {
            assert_eq!(report[member], count, "{member}");
        }
        let ratio_median = number(&report["ratio_median"]);
        let ratio_nmad = number(&report["ratio_nmad"]);
        assert!((ratio_median - ORACLE_RATIO_MEDIAN).abs() < 5e-4, "{ratio_median}");
        assert!(ratio_nmad < 2e-4, "{ratio_nmad}");
        assert!((number(&report["ratio_offset_mag"]) + 2.5 * ratio_median.log10()).abs() < 1e-12);
        let median_err = MEDIAN_ERR_PER_NMAD * ratio_nmad / 6f64.sqrt();
        assert!((number(&report["ratio_median_err"]) - median_err).abs() <= 1e-12 * median_err);
        let err_ratio_median = number(&report["err_ratio_median"]);
        assert!((err_ratio_median - ORACLE_ERR_RATIO_MEDIAN).abs() < 2e-3, "{err_ratio_median}");
        let zero_point = number(&report["implied_zero_point_ab"]);
        assert!((zero_point - ORACLE_ZERO_POINT_AB).abs() < 1e-3, "{zero_point}");
        assert_eq!(
            report["rejected"],
            json!([
                {"reason": "off the image", "count": 1},
                {"reason": "missing value in apflux", "count": 1},
                {"reason": "cut type == PSF", "count": 1},
                {"reason": EDGE_REASON, "count": 1},
            ])
        );

        let s1 = row_named(&report, "s1");
        assert!((number(&s1["x"]) - 40.0).abs() < 1e-6, "{s1}");
        assert!((number(&s1["y"]) - 50.0).abs() < 1e-6, "{s1}");
        assert!((number(&s1["flux_cat_jy"]) - S1_FLUX_CAT_JY).abs() <= 1e-12 * S1_FLUX_CAT_JY, "{s1}");
        assert_eq!(s1["used"], true);
        assert_eq!(s1["class"], "PSF");
        assert_eq!(s1["masked_fraction"], 0.0);
        let edge = row_named(&report, "edge");
        assert_eq!(edge["used"], false);
        assert_eq!(edge["reason"], EDGE_REASON);
        let (edge_ra, edge_dec) = (number(&edge["ra_deg"]), number(&edge["dec_deg"]));
        let edge_cell = forced_band(
            &request(&key, &table).band,
            &[ForcedSkyPosition { id: "edge".into(), ra_deg: edge_ra, dec_deg: edge_dec }],
            &ForcedAperture {
                radius_arcsec: None,
                radius_px: Some(8.0),
                annulus_arcsec: None,
                annulus_px: None,
                correction: ForcedCorrection::None,
                reference_radius_arcsec: None,
            },
            &ForcedOptions::default(),
        )
        .unwrap()
        .cells
        .remove(0);
        assert_eq!(edge["flux_jy"], json!(edge_cell.flux_jy));

        let aperture = &report["aperture"];
        for (member, expected) in [("radius_px", 8.0), ("radius_arcsec", 4.0), ("pixel_scale_arcsec", 0.5)] {
            assert!((number(&aperture[member]) - expected).abs() < 1e-9, "{member}: {aperture}");
        }
        assert_eq!(aperture["annulus_px"], serde_json::Value::Null);
        assert_eq!(aperture["local_sky"], false);
        assert!(report["calibration_origin"].as_str().is_some_and(|origin| !origin.is_empty()), "{}", report["calibration_origin"]);
        assert_eq!(report["calibration_origin"], report["band"]["calibration_origin"]);
        assert_eq!(report["table"], json!({"path": table, "file_name": "cat.csv", "format": "csv", "n_rows": 10}));
        assert_eq!(
            texts(&report["notes"]),
            vec![NOTE_RATIO, NOTE_APERTURE_8_PX, NOTE_FORMAL_ERROR, NOTE_POSITIONS, NOTE_CLASS_TYPE]
        );
        let warnings = texts(&report["warnings"]);
        assert!(!warnings.iter().any(|warning| warning.starts_with("only")), "{warnings:?}");
        assert_eq!(report["preset"], serde_json::Value::Null);
        assert_eq!(report["catalogue_band"], serde_json::Value::Null);
        assert!(report["elapsed_ms"].is_u64());
        let mut members: Vec<&str> = report.as_object().unwrap().keys().map(String::as_str).collect();
        members.sort_unstable();
        assert_eq!(
            members,
            vec![
                "aperture", "band", "calibration_origin", "catalogue_band", "elapsed_ms", "err_ratio_median",
                "err_ratio_nmad", "implied_zero_point_ab", "implied_zero_point_nmad", "n_after_cuts", "n_err",
                "n_measured", "n_on_image", "n_ratio", "n_table", "n_used", "n_zero_point", "notes", "preset",
                "ratio_median", "ratio_median_err", "ratio_nmad", "ratio_offset_mag", "rejected", "rows", "table",
                "warnings",
            ]
        );
        for member in [
            "file_name", "label", "filter", "calibration_origin", "jy_per_native_unit", "bunit", "pixel_scale_arcsec",
            "aperture_radius_px", "annulus_px", "err_origin", "dq_source", "warnings",
        ] {
            assert!(report["band"].get(member).is_some(), "band.{member}");
        }
        assert_eq!(report["band"]["err_origin"]["role"], "sigma");
    }

    #[test]
    fn a_radius_in_arcsec_gives_the_same_report() {
        let dir = tempfile::tempdir().unwrap();
        let (key, table) = calibrated_field(dir.path());
        let in_pixels = compared(&request(&key, &table));
        let mut arcsec_request = request(&key, &table);
        arcsec_request.aperture = CompareAperture { radius_arcsec: Some(4.0), ..CompareAperture::default() };
        let in_arcsec = compared(&arcsec_request);

        assert!((number(&in_pixels["ratio_median"]) - number(&in_arcsec["ratio_median"])).abs() < 1e-12);
        assert_eq!(in_pixels["n_used"], in_arcsec["n_used"]);
        for member in ["radius_px", "radius_arcsec", "pixel_scale_arcsec"] {
            let (a, b) = (number(&in_pixels["aperture"][member]), number(&in_arcsec["aperture"][member]));
            assert!((a - b).abs() < 1e-12, "{member}: {a} {b}");
        }
        assert_eq!(in_pixels["aperture"]["annulus_px"], in_arcsec["aperture"]["annulus_px"]);
        assert_eq!(in_pixels["aperture"]["local_sky"], in_arcsec["aperture"]["local_sky"]);
    }

    #[test]
    fn a_local_sky_annulus_is_measured_and_its_reach_counts_for_the_edge() {
        let dir = tempfile::tempdir().unwrap();
        let (key, table) = calibrated_field(dir.path());
        let mut annulus_request = request(&key, &table);
        annulus_request.aperture.annulus_px = Some([20.0, 30.0]);
        let report = compared(&annulus_request);

        assert_eq!(report["aperture"]["local_sky"], true);
        assert_eq!(report["aperture"]["annulus_px"], json!([20.0, 30.0]));
        assert_eq!(report["notes"][1], NOTE_ANNULUS_20_30_PX);
        assert_eq!(row_named(&report, "s6")["reason"], EDGE_REASON);
        assert_eq!(report["n_used"], 5);
        let ratio_median = number(&report["ratio_median"]);
        assert!((ratio_median - ORACLE_RATIO_MEDIAN).abs() < 2e-3, "{ratio_median}");
    }

    #[test]
    fn an_image_without_calibration_gives_the_implied_zero_point_only() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_field(dir.path(), "plain.fits", Some(0.5), vec![]);
        let table = write_catalogue(dir.path(), &key);
        let report = compared(&request(&key, &table));

        assert_eq!(report["ratio_median"], serde_json::Value::Null);
        assert_eq!(report["n_ratio"], 0);
        let zero_point = number(&report["implied_zero_point_ab"]);
        assert!((zero_point - ORACLE_ZERO_POINT_AB).abs() < 1e-3, "{zero_point}");
        assert!(texts(&report["warnings"]).iter().any(|warning| warning == WARNING_NO_JY), "{}", report["warnings"]);
        assert_eq!(report["calibration_origin"], serde_json::Value::Null);
    }

    #[test]
    fn an_image_without_an_error_layer_has_no_error_ratio() {
        let dir = tempfile::tempdir().unwrap();
        let (key, table) = calibrated_field(dir.path());
        let with_error = compared(&request(&key, &table));
        let bare_dir = tempfile::tempdir().unwrap();
        let bare_key = write_field(bare_dir.path(), "field.fits", None, nanomaggy_cards());
        let bare_table = write_catalogue(bare_dir.path(), &bare_key);
        let report = compared(&request(&bare_key, &bare_table));

        assert_eq!(report["err_ratio_median"], serde_json::Value::Null);
        assert_eq!(report["n_err"], 0);
        assert!((number(&report["ratio_median"]) - number(&with_error["ratio_median"])).abs() < 1e-12);
        let warnings = texts(&report["warnings"]);
        assert!(
            warnings.iter().any(|warning| warning.starts_with("no error layer and no sky annulus: the fluxes of")),
            "{warnings:?}"
        );
    }

    #[test]
    fn a_filter_that_differs_from_the_catalogue_band_is_said() {
        let dir = tempfile::tempdir().unwrap();
        let cards = vec![
            ("TELESCOP", "'JWST'".to_string()),
            ("INSTRUME", "'NIRCAM'".to_string()),
            ("FILTER", "'F200W'".to_string()),
        ];
        let key = write_field(dir.path(), "nircam.fits", Some(0.5), cards);
        let table = write_catalogue(dir.path(), &key);
        let mut other_band = request(&key, &table);
        other_band.catalogue_band = Some("r".into());
        let warnings = texts(&compared(&other_band)["warnings"]);
        assert!(
            warnings.iter().any(|warning| warning.starts_with("the catalogue band is r and the image filter is F200W")),
            "{warnings:?}"
        );

        let mut same_band = request(&key, &table);
        same_band.catalogue_band = Some("f200w".into());
        let report = compared(&same_band);
        let warnings = texts(&report["warnings"]);
        assert!(!warnings.iter().any(|warning| warning.starts_with("the catalogue band")), "{warnings:?}");
        assert_eq!(report["catalogue_band"], "f200w");
    }

    #[test]
    fn refusals_come_before_any_measurement() {
        let dir = tempfile::tempdir().unwrap();
        let (key, table) = calibrated_field(dir.path());
        let missing_image = dir.path().join("missing.fits").to_str().unwrap().to_string();

        let mut both_radii = request(&missing_image, "nowhere.fits");
        both_radii.aperture.radius_arcsec = Some(4.0);
        assert_eq!(refusal(&both_radii), "give the aperture radius either in arcsec or in pixels, not both");

        let fits_table = dir.path().join("cat.fits").to_str().unwrap().to_string();
        assert_eq!(
            refusal(&request(&missing_image, &fits_table)),
            "catalogue table cat.fits: only CSV and ECSV tables can be compared; save the catalogue as CSV"
        );

        let mut unknown_column = request(&missing_image, &table);
        unknown_column.mapping.flux = "nope".into();
        let text = refusal(&unknown_column);
        assert!(text.starts_with("column 'nope' is not in the table"), "{text}");

        let bare = dir.path().join("bare.fits");
        write_fits_mono(bare.to_str().unwrap(), &Array2::from_elem((32, 32), 1.0f32), None).unwrap();
        assert_eq!(
            refusal(&request(bare.to_str().unwrap(), &table)),
            "bare.fits has no celestial WCS: catalogue positions cannot be placed on it"
        );

        let mut too_large = request(&key, &table);
        too_large.aperture.radius_px = Some(100.0);
        assert_eq!(refusal(&too_large), "aperture radius 100 px on field.fits: the limits are 2 to 60 px");
    }

    #[test]
    fn the_request_is_read_from_the_json_the_front_sends() {
        let front = json!({
            "band": {"path": "C:/d/legacy_r.fits", "label": null, "dataset": null},
            "table_path": "C:/d/c1b.csv",
            "preset": null,
            "catalogue_band": null,
            "mapping": {
                "id": null, "ra": "ra", "dec": "dec", "flux": "apflux_r_5", "flux_unit": "nanomaggy",
                "zero_point": null, "flux_err": null, "flux_ivar": null, "class": null, "missing_value": null
            },
            "aperture": {"radius_arcsec": 2.0, "radius_px": null, "annulus_arcsec": null, "annulus_px": null},
            "cuts": [],
            "min_snr": null,
            "max_snr": null,
            "min_neighbour_arcsec": null,
            "exclude_dq": null
        });
        let parsed: CompareRequest = serde_json::from_value(front.clone()).unwrap();
        assert_eq!(parsed.band.path, "C:/d/legacy_r.fits");
        assert_eq!(parsed.band.label, None);
        assert_eq!(parsed.preset, None);
        assert_eq!(parsed.aperture, CompareAperture { radius_arcsec: Some(2.0), ..CompareAperture::default() });
        assert_eq!(parsed.mapping.flux_unit, CatalogueFluxUnit::Nanomaggy);
        assert_eq!(parsed.exclude_dq, None);
        assert!(parsed.cuts.is_empty());

        let mut unknown = front.clone();
        unknown["radius"] = json!(3.0);
        assert!(serde_json::from_value::<CompareRequest>(unknown).is_err());

        let mut with_cuts = front;
        with_cuts["cuts"] = json!([
            {"column": "type", "op": "==", "value": "PSF"},
            {"column": "flag", "op": "<", "value": 4}
        ]);
        let parsed: CompareRequest = serde_json::from_value(with_cuts).unwrap();
        assert_eq!(
            parsed.cuts,
            vec![
                CompareCut { column: "type".into(), op: CutOp::Eq, value: CutValue::Text("PSF".into()) },
                CompareCut { column: "flag".into(), op: CutOp::Lt, value: CutValue::Number(4.0) },
            ]
        );
    }

    #[test]
    fn limits_of_the_comparison() {
        let dir = tempfile::tempdir().unwrap();
        let (key, table) = calibrated_field(dir.path());

        let far_dir = tempfile::tempdir().unwrap();
        let far_rows: Vec<TableRow> = field_rows(&key)
            .into_iter()
            .map(|row| TableRow { sky: (row.sky.0, row.sky.1 + 1.0), ..row })
            .collect();
        let far = compared(&request(&key, &write_rows(far_dir.path(), &far_rows)));
        assert_eq!(far["n_on_image"], 0);
        assert_eq!(far["n_used"], 0);
        assert_eq!(far["rows"], json!([]));
        assert_eq!(far["aperture"]["radius_px"], 8.0);
        assert!(
            texts(&far["warnings"]).iter().any(|warning| warning == "no row is left to compare: see the rejected rows"),
            "{}",
            far["warnings"]
        );

        let single_dir = tempfile::tempdir().unwrap();
        let single_rows: Vec<TableRow> = field_rows(&key).into_iter().take(1).collect();
        let single = compared(&request(&key, &write_rows(single_dir.path(), &single_rows)));
        assert_eq!(single["n_used"], 1);
        assert_eq!(single["ratio_nmad"], 0.0);
        assert_eq!(single["ratio_median_err"], serde_json::Value::Null);
        assert!(
            texts(&single["warnings"])
                .iter()
                .any(|warning| warning == "only 1 row is used: the median of fewer than 5 rows is indicative"),
            "{}",
            single["warnings"]
        );

        let plain = compared(&request(&key, &table));
        let mut excluding = request(&key, &table);
        excluding.exclude_dq = Some(true);
        let excluded = compared(&excluding);
        for member in [
            "n_on_image",
            "n_after_cuts",
            "n_measured",
            "n_used",
            "ratio_median",
            "ratio_nmad",
            "err_ratio_median",
            "implied_zero_point_ab",
            "rejected",
            "aperture",
        ] {
            assert_eq!(plain[member], excluded[member], "{member}");
        }
        assert!(!texts(&plain["notes"]).iter().any(|note| note == NOTE_EXCLUDED_FLAGS));
        assert_eq!(excluded["notes"][4], NOTE_EXCLUDED_FLAGS);
        assert_eq!(excluded["notes"][5], NOTE_CLASS_TYPE);
    }

    const LEGACY_BRICK_TABLE: &str = "survey/catalogs/C1b_ls_dr10_brick_0532m280.csv";
    const LEGACY_REFERENCE_STAR: &str = "10995300315432397";
    const LEGACY_REFERENCE_STAR_JY: f64 = 2.8717063294031377e-04;
    const LEGACY_TWO_ARCSEC_PX: f64 = 7.633587786259541;
    const DES_R_IMAGE: &str = "survey/D1/des-dr2_A_r_extn1.fits";
    const DES_R_WEIGHT: &str = "survey/D1/des-dr2_A_r_extn3.fits";
    const DES_TABLE: &str = "survey/catalogs/C2_des_dr2_box_A.csv";
    const SDSS_R_FRAME: &str = "survey/S1/frame-r-001462-4-0165.fits.bz2";
    const SDSS_TABLE: &str = "survey/S1/C4_sdss_dr17_field_1462_4_165.csv";
    const PS1_R_STACK: &str = "survey/P1/ps1_A_r_stack_480px.fits";
    const PS1_TABLE: &str = "survey/P1/C5_ps1_dr2_cone_A.csv";
    const EUCLID_VIS_MOSAIC: &str =
        "survey/E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits";
    const EUCLID_TABLE: &str = "survey/catalogs/C3_euclid_q1_mer_cone_A.csv";
    const EUCLID_ISOLATED_STAR: &str = "-531333416280977000";
    const EUCLID_CROWDED_STAR: &str = "-531288437281048064";
    const NO_DATA_REASON: &str = "aperture touches pixels without data";
    const SATURATED_REASON: &str = "saturated in the image";
    const FEW_ROWS_WARNING_4: &str = "only 4 rows are used: the median of fewer than 5 rows is indicative";
    const COUNT_MEMBERS: [&str; 5] = ["n_table", "n_on_image", "n_after_cuts", "n_measured", "n_used"];

    fn survey_file(rel: &str) -> String {
        let path = env_dir(SCIENCE_DATA_DIR_VAR).join(rel);
        assert!(path.is_file(), "{} is missing", path.display());
        path.to_str().unwrap().to_string()
    }

    fn legacy_brick_image(band: &str) -> String {
        survey_file(&format!("survey/L2/legacysurvey-0532m280-image-{band}.fits.fz"))
    }

    fn preset_request(image_key: &str, table_rel: &str, preset_id: &str, band: &str) -> CompareRequest {
        let table = survey_file(table_rel);
        let probe = compare_probe_json(&table).unwrap();
        let preset = probe["presets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|preset| preset["id"] == preset_id && preset["band"] == band)
            .unwrap_or_else(|| panic!("{table_rel}: no preset {preset_id} {band}"));
        serde_json::from_value(json!({
            "band": {"path": image_key, "label": null, "dataset": null},
            "table_path": table,
            "preset": preset_id,
            "catalogue_band": band,
            "mapping": preset["mapping"],
            "aperture": preset["aperture"],
            "cuts": preset["cuts"],
            "min_snr": preset["min_snr"],
            "max_snr": preset["max_snr"],
            "min_neighbour_arcsec": preset["min_neighbour_arcsec"],
            "exclude_dq": null
        }))
        .unwrap()
    }

    fn rejected_of(report: &serde_json::Value) -> Vec<(String, u64)> {
        report["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| (entry["reason"].as_str().unwrap().to_string(), entry["count"].as_u64().unwrap()))
            .collect()
    }

    fn rejected_count(report: &serde_json::Value, reason: &str) -> u64 {
        rejected_of(report).into_iter().find(|(text, _)| text == reason).map_or(0, |(_, count)| count)
    }

    fn survey_report(set: &str, request: &CompareRequest) -> serde_json::Value {
        let report = compared(request);
        let counts: Vec<String> = COUNT_MEMBERS.iter().map(|member| report[*member].to_string()).collect();
        println!(
            "{set}, preset {} {}: {} = {}; ratio_median {} +- {} (nmad {}, N {}); err_ratio_median {} (nmad {}, N {}); implied_zero_point_ab {} (nmad {}, N {}); rejected {:?}",
            request.preset.as_deref().unwrap_or("none"),
            request.catalogue_band.as_deref().unwrap_or("none"),
            COUNT_MEMBERS.join(" / "),
            counts.join(" / "),
            report["ratio_median"],
            report["ratio_median_err"],
            report["ratio_nmad"],
            report["n_ratio"],
            report["err_ratio_median"],
            report["err_ratio_nmad"],
            report["n_err"],
            report["implied_zero_point_ab"],
            report["implied_zero_point_nmad"],
            report["n_zero_point"],
            rejected_of(&report)
        );
        report
    }

    fn assert_counts(report: &serde_json::Value, expected: [u64; 5]) {
        let counts: Vec<u64> = COUNT_MEMBERS.iter().map(|member| report[*member].as_u64().unwrap()).collect();
        assert_eq!(counts, expected, "{}, against CAT-oracle-cascade.py", COUNT_MEMBERS.join(" / "));
    }

    fn assert_rejected(report: &serde_json::Value, expected: &[(&str, u64)]) {
        let expected: Vec<(String, u64)> = expected.iter().map(|(reason, count)| (reason.to_string(), *count)).collect();
        assert_eq!(rejected_of(report), expected, "rejected, against CAT-oracle-cascade.py");
    }

    fn assert_near(what: &str, measured: &serde_json::Value, target: f64, tolerance: f64) {
        let value = measured.as_f64().unwrap_or(f64::NAN);
        assert!((value - target).abs() <= tolerance, "{what}: {measured} is not within {tolerance} of {target}");
    }

    fn used_rows(report: &serde_json::Value) -> Vec<&serde_json::Value> {
        report["rows"].as_array().unwrap().iter().filter(|row| row["used"] == true).collect()
    }

    fn rows_left_out_for<'a>(report: &'a serde_json::Value, reason: &str) -> Vec<&'a serde_json::Value> {
        report["rows"].as_array().unwrap().iter().filter(|row| row["reason"] == reason).collect()
    }

    fn print_rows(set: &str, rows: &[&serde_json::Value]) {
        for row in rows {
            println!(
                "{set} row {}: used {} x {} y {} flux_cat_jy {} ratio {} masked_fraction {} reason {}",
                row["id"],
                row["used"],
                row["x"],
                row["y"],
                row["flux_cat_jy"],
                row["ratio"],
                row["masked_fraction"],
                row["reason"]
            );
        }
    }

    fn assert_legacy_cascade(report: &serde_json::Value, band: &str) {
        assert_counts(report, [69, 69, 21, 21, 21]);
        let flux_fraction_cut = format!("cut fracflux_{band} < 0.01");
        assert_rejected(report, &[("cut type == PSF", 33), ("cut maskbits == 0", 14), (&flux_fraction_cut, 1)]);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2 and survey/catalogs"]
    fn real_legacy_l2_r_report_matches_apflux_r_5_golden_phot_legacy_l2_brick() {
        let request = preset_request(&legacy_brick_image("r"), LEGACY_BRICK_TABLE, "legacy_dr10", "r");
        let report = survey_report("L2", &request);
        let star = row_named(&report, LEGACY_REFERENCE_STAR);
        print_rows("L2 r", &[star]);

        assert_legacy_cascade(&report, "r");
        assert_near("ratio_median, research target", &report["ratio_median"], 1.0, 0.01);
        assert_near(
            "ratio_median, golden phot_legacy_l2_brick section_7_F1 1.0000028, mirror 1.0000182",
            &report["ratio_median"],
            1.0,
            3e-4,
        );
        let ratio_nmad = number(&report["ratio_nmad"]);
        assert!(ratio_nmad < 5e-4, "ratio_nmad {ratio_nmad}: golden 1.6e-5, mirror 5.2e-5");
        assert_near(
            "err_ratio_median, research target (null: the -invvar- sibling was not paired, SURVEY-LAYERS)",
            &report["err_ratio_median"],
            1.0,
            0.05,
        );
        assert_eq!(report["n_err"], 21);
        assert_near(
            "err_ratio_median, golden phot_legacy_l2_brick rows err_w2_r_2.0 x sqrt(apflux_ivar_r_5) 0.98652",
            &report["err_ratio_median"],
            0.9865,
            0.005,
        );
        assert_near("implied_zero_point_ab, header zero point", &report["implied_zero_point_ab"], 22.5, 0.011);

        assert_eq!(star["used"], true);
        assert_near("x, golden reference row 3333.8115", &star["x"], 3333.8115, 0.01);
        assert_near("y, golden reference row 625.4192", &star["y"], 625.4192, 0.01);
        assert_near(
            "flux_cat_jy, 79.09336 nanomaggy",
            &star["flux_cat_jy"],
            LEGACY_REFERENCE_STAR_JY,
            1e-9 * LEGACY_REFERENCE_STAR_JY,
        );
        assert_near("ratio, mirror 79.09526 over 79.09336", &star["ratio"], 1.000024, 0.001);

        assert!(report["calibration_origin"].is_string(), "{}", report["calibration_origin"]);
        assert_near("aperture.radius_px, 2.0 arcsec", &report["aperture"]["radius_px"], LEGACY_TWO_ARCSEC_PX, 1e-9);
        assert_eq!(report["aperture"]["local_sky"], false);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2 and survey/catalogs"]
    fn real_legacy_l2_g_and_z_reports_match_their_catalogue_apertures() {
        for (band, golden) in [("g", "1.0000049"), ("z", "1.0000008")] {
            let request = preset_request(&legacy_brick_image(band), LEGACY_BRICK_TABLE, "legacy_dr10", band);
            let report = survey_report("L2", &request);
            assert_legacy_cascade(&report, band);
            assert_near(
                &format!("ratio_median of {band}, research target (golden phot_legacy_l2_brick rows {golden})"),
                &report["ratio_median"],
                1.0,
                0.01,
            );
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2 and survey/catalogs"]
    fn real_legacy_l2_r_without_cuts_leaves_out_the_two_apertures_without_data() {
        let mut request = preset_request(&legacy_brick_image("r"), LEGACY_BRICK_TABLE, "legacy_dr10", "r");
        request.cuts.clear();
        let report = survey_report("L2 without cuts", &request);
        let without_data = [("10995300315434171", 0.704, 0.005), ("10995300315434517", 0.00546, 5e-4)];
        print_rows("L2 r without cuts", &without_data.map(|(id, ..)| row_named(&report, id)));
        print_rows("L2 r without cuts", &rows_left_out_for(&report, SATURATED_REASON));

        assert_eq!(report["n_after_cuts"], 69, "against CAT-oracle-cascade.py");
        for (id, masked_fraction, tolerance) in without_data {
            let row = row_named(&report, id);
            assert_eq!(row["reason"], NO_DATA_REASON, "{id}");
            assert_near(
                &format!("masked_fraction of {id}, golden apertures_touching_invvar_zero"),
                &row["masked_fraction"],
                masked_fraction,
                tolerance,
            );
        }
        let n_used = report["n_used"].as_u64().unwrap();
        let n_saturated = rejected_count(&report, SATURATED_REASON);
        assert_eq!(
            n_used + n_saturated,
            67,
            "n_used {n_used} plus {n_saturated} saturated: golden phot_legacy_l2_brick keeps the 67 rows without a pixel of zero inverse variance; rejected {:?}",
            rejected_of(&report)
        );
        assert_near(
            "ratio_median, research target (golden 1.0000003 over the 67, mirror 1.000024)",
            &report["ratio_median"],
            1.0,
            0.01,
        );
        assert_near("err_ratio_median, research target (mirror 0.98524)", &report["err_ratio_median"], 1.0, 0.05);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1 and survey/catalogs"]
    fn real_des_d1_r_report_matches_flux_aper_8() {
        let request = preset_request(&survey_file(DES_R_IMAGE), DES_TABLE, "des_dr2", "r");
        let report = survey_report("D1", &request);
        let at_the_edge = rows_left_out_for(&report, EDGE_REASON);
        print_rows("D1 r", &at_the_edge);

        assert_counts(&report, [148, 148, 14, 13, 13]);
        assert_rejected(&report, &[("catalogue signal-to-noise below 50", 134), (EDGE_REASON, 1)]);
        assert_near("x of the row that crosses the edge", &at_the_edge[0]["x"], 5.7, 0.1);
        assert_near("ratio_median, research target", &report["ratio_median"], 1.0, 0.03);
        assert_near(
            "ratio_median, golden phot_des_d1 flux_aper_8_r 0.9963756, mirror 0.9964117",
            &report["ratio_median"],
            0.9964,
            0.003,
        );
        assert_near("aperture.radius_px", &report["aperture"]["radius_px"], 11.11, 1e-9);
        assert_near("implied_zero_point_ab, MAGZERO", &report["implied_zero_point_ab"], 30.0, 0.033);
        assert_eq!(
            report["err_ratio_median"],
            serde_json::Value::Null,
            "the weight file is a separate file and is not assigned to the image of the data folder"
        );
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1 and survey/catalogs"]
    fn real_des_d1_r_faint_errors_match_fluxerr_aper_8() {
        let dir = tempfile::tempdir().unwrap();
        let [image, weight] = [DES_R_IMAGE, DES_R_WEIGHT].map(|rel| {
            let source = survey_file(rel);
            let copy = dir.path().join(Path::new(&source).file_name().unwrap());
            std::fs::copy(&source, &copy).unwrap();
            copy.to_str().unwrap().to_string()
        });
        set_companion_json(&image, "err", &weight, None).unwrap();
        let mut request = preset_request(&image, DES_TABLE, "des_dr2", "r");
        request.cuts = ["flags_r", "imaflags_iso_r"]
            .map(|column| CompareCut { column: column.into(), op: CutOp::Eq, value: CutValue::Number(0.0) })
            .to_vec();
        request.min_snr = Some(5.0);
        request.max_snr = Some(20.0);
        let report = survey_report("D1 faint, weight assigned", &request);
        print_rows("D1 r faint", &rows_left_out_for(&report, EDGE_REASON));

        assert_eq!(report["n_after_cuts"], 32, "against CAT-oracle-cascade.py");
        assert_eq!(rejected_count(&report, EDGE_REASON), 3);
        assert_eq!(report["n_used"], 29);
        assert_near("err_ratio_median, research target", &report["err_ratio_median"], 1.0, 0.20);
        assert_near(
            "err_ratio_median, golden phot_des_d1 0.98194, mirror 0.98195",
            &report["err_ratio_median"],
            0.9819,
            0.005,
        );
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/S1"]
    fn real_sdss_s1_r_report_matches_aperflux7() {
        let request = preset_request(&survey_file(SDSS_R_FRAME), SDSS_TABLE, "sdss_dr17", "r");
        let report = survey_report("S1", &request);

        assert_counts(&report, [808, 808, 31, 31, 31]);
        print_rows("S1 r, first row used", &used_rows(&report)[..1]);
        assert_rejected(
            &report,
            &[("cut type == 6", 496), ("cut clean == 1", 107), ("cut psfMag_r >= 15", 11), ("cut psfMag_r <= 18", 163)],
        );
        assert_near(
            "ratio_median, research target (golden phot_sdss_s1 1.0064, mirror 1.0030)",
            &report["ratio_median"],
            1.0,
            0.03,
        );
        let first_id = used_rows(&report)[0]["id"].as_str().unwrap();
        assert_eq!(first_id.len(), 19, "{first_id}");
        assert!(first_id.starts_with("12376517539972"), "{first_id}");
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1"]
    fn real_ps1_p1_r_report_matches_rflxr5() {
        let request = preset_request(&survey_file(PS1_R_STACK), PS1_TABLE, "ps1_dr2", "r");
        let report = survey_report("P1", &request);
        print_rows("P1 r", &used_rows(&report));

        assert_counts(&report, [75, 47, 4, 4, 4]);
        assert_rejected(
            &report,
            &[
                ("off the image", 28),
                ("missing value (-999) in rflxr5", 1),
                ("catalogue signal-to-noise below 50", 42),
            ],
        );
        let warnings = texts(&report["warnings"]);
        assert!(warnings.iter().any(|warning| warning == FEW_ROWS_WARNING_4), "{warnings:?}");
        assert_near(
            "ratio_median, research target (golden phot_ps1_p1 1.0169, mirror 1.0167; far from 1: the PC001001 cards of the cutout were not read, SURVEY-PHOT)",
            &report["ratio_median"],
            1.0,
            0.05,
        );
        assert_near("aperture.radius_px, 3.0 arcsec", &report["aperture"]["radius_px"], 12.0, 1e-3);

        let mut with_local_sky = request;
        with_local_sky.aperture.annulus_arcsec = Some([5.0, 7.5]);
        let report = survey_report("P1 with the annulus 5.0 to 7.5 arcsec", &with_local_sky);
        let at_the_edge = rows_left_out_for(&report, EDGE_REASON);
        print_rows("P1 r with the annulus", &used_rows(&report));
        print_rows("P1 r with the annulus", &at_the_edge);
        assert_eq!(report["n_used"], 3);
        assert_eq!(at_the_edge.len(), 1);
        assert_near("y of the row whose annulus crosses the edge", &at_the_edge[0]["y"], 452.3, 0.1);
        assert_near(
            "ratio_median with the annulus, research target (golden phot_ps1_p1 0.9907, mirror 0.9870)",
            &report["ratio_median"],
            1.0,
            0.05,
        );
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1 and survey/catalogs"]
    fn real_euclid_e1_vis_report_matches_flux_detection_total() {
        let request = preset_request(&survey_file(EUCLID_VIS_MOSAIC), EUCLID_TABLE, "euclid_q1", "VIS");
        let report = survey_report("E1", &request);
        let used = used_rows(&report);
        print_rows("E1 VIS", &used);

        assert_counts(&report, [692, 112, 1, 1, 1]);
        assert_rejected(
            &report,
            &[("off the image", 580), ("cut point_like_prob > 0.7", 110), ("neighbour within 2 arcsec", 1)],
        );
        assert_eq!(used[0]["id"], EUCLID_ISOLATED_STAR);
        assert_near(
            "ratio, first-run target (golden phot_euclid_e1 0.9909 at 1.25 arcsec, mirror 0.9905)",
            &used[0]["ratio"],
            1.0,
            0.03,
        );
        assert_near("aperture.radius_px, 1.25 arcsec", &report["aperture"]["radius_px"], 12.5, 1e-6);
        assert_near("implied_zero_point_ab, MAGZERO", &report["implied_zero_point_ab"], 24.6, 0.033);

        let mut without_isolation = request;
        without_isolation.min_neighbour_arcsec = None;
        let report = survey_report("E1 without the neighbour rule", &without_isolation);
        let used = used_rows(&report);
        print_rows("E1 VIS without the neighbour rule", &used);
        assert_eq!(report["n_used"], 2);
        assert_eq!(used[0]["id"], EUCLID_ISOLATED_STAR);
        assert_eq!(used[1]["id"], EUCLID_CROWDED_STAR);
        for row in used {
            assert_near(
                &format!("ratio of {}, first-run target (golden phot_euclid_e1 0.9909 and 1.0060)", row["id"]),
                &row["ratio"],
                1.0,
                0.03,
            );
        }

        let mut against_psf_flux = without_isolation;
        against_psf_flux.mapping.flux = "flux_vis_psf".into();
        let report = survey_report("E1 against flux_vis_psf, not asserted", &against_psf_flux);
        print_rows(
            "E1 VIS against flux_vis_psf (golden phot_euclid_e1 curve_of_growth[9] 1.1268 and 1.1914)",
            &[EUCLID_ISOLATED_STAR, EUCLID_CROWDED_STAR].map(|id| row_named(&report, id)),
        );
    }
}
