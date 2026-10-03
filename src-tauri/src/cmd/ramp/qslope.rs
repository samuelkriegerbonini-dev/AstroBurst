use std::path::Path;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Deserializer};
use serde_json::json;

use crate::cmd::common::{blocking_cmd, invalidate_written, resolve_output_dir, save_auto_stf_preview_png, write_derived_fits};
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::ramp::compare::{compare_with_rate, ratio_image, read_official_rate, read_qslope};
use crate::core::ramp::irs2::{Irs2Layout, Irs2Sample};
use crate::core::ramp::product::{
    qslope_output_path, qslope_primary_header, sibling_rate_path, write_qslope, CARD_ABPROC, CARD_BUNIT, QSLOPE_SUFFIX,
};
use crate::core::ramp::quick_slope::{
    band_scale, fit_pixel, plan_slope, quick_slope, reference_offsets, science_rows_of_band, slope_bands, QuickSlopeParams,
    RefCorrection, ReferenceOffsets, ERR_TGROUP_MISSING,
};
use crate::infra::fits::reader::read_primary_header;
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_PNG_PATH};
use crate::types::header::HduHeader;
use crate::types::image_ref::source_stem;
use crate::types::ImageRef;

pub const EVENT_QSLOPE_PROGRESS: &str = "qslope-progress";
pub const ABPROC_QSLOPE_RATIO: &str = "qslope-ratio";
pub const RATIO_SUFFIX: &str = "_ratio";
pub const BUNIT_RATIO: &str = "ratio";
pub const CARD_RATIO_QSLOPE: &str = "ABQSLOPE";
pub const CARD_RATIO_RATE: &str = "ABRATE";
pub const STAGE_WRITING: &str = "writing";
pub const PROGRESS_EXTRA_STEPS: u64 = 2;
pub const ERR_NOT_A_RAMP: &str = "not a ramp: the header has no NGROUPS/NINTS/DATAMODL ramp cards or the group count disagrees with the data";

const KEY_INTEGRATION: &str = "integration";
const KEY_NINTS: &str = "nints";
const KEY_NGROUPS: &str = "ngroups";
const KEY_TGROUP_S: &str = "tgroup_s";
const KEY_TGROUP_SOURCE: &str = "tgroup_source";
const KEY_DETECTOR: &str = "detector";
const KEY_READPATT: &str = "readpatt";
const KEY_REF_CORRECTED: &str = "ref_corrected";
const KEY_STRIPPED: &str = "stripped";
const KEY_PARAMS: &str = "params";
const KEY_BAND_SCALES: &str = "band_scales_dn";
const KEY_COUNTS: &str = "counts";
const KEY_RATE_SIBLING: &str = "rate_sibling";
const KEY_WARNINGS: &str = "warnings";
const KEY_X: &str = "x";
const KEY_Y: &str = "y";
const KEY_GROUP_TIMES: &str = "group_times_s";
const KEY_RAW: &str = "raw";
const KEY_CORRECTED: &str = "corrected";
const KEY_REF_OFFSETS: &str = "ref_offsets";
const KEY_BAND_SCALE: &str = "band_scale_dn";
const KEY_FIT: &str = "fit";
const KEY_FLAGGED_DIFFS: &str = "flagged_diffs";
const KEY_AMPLIFIER: &str = "amplifier";
const KEY_SCIENCE_ROW: &str = "science_row";
const KEY_QSLOPE_PATH: &str = "qslope_path";
const KEY_RATE_PATH: &str = "rate_path";
const KEY_RATIO_PNG_PATH: &str = "ratio_png_path";
const KEY_RATIO_FITS_PATH: &str = "ratio_fits_path";

fn deserialize_some<'de, T, D>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct QuickSlopeParamsInput {
    pub sat_dn: Option<f32>,
    pub jump_k: Option<f32>,
    pub scale_floor_dn: Option<f32>,
    pub min_groups_ols: Option<usize>,
    pub ref_correction: Option<RefCorrection>,
    #[serde(deserialize_with = "deserialize_some")]
    pub ref_window_rows: Option<Option<usize>>,
}

impl QuickSlopeParamsInput {
    pub fn resolve(self) -> QuickSlopeParams {
        let defaults = QuickSlopeParams::default();
        QuickSlopeParams {
            sat_dn: self.sat_dn.unwrap_or(defaults.sat_dn),
            jump_k: self.jump_k.unwrap_or(defaults.jump_k),
            scale_floor_dn: self.scale_floor_dn.unwrap_or(defaults.scale_floor_dn),
            min_groups_ols: self.min_groups_ols.unwrap_or(defaults.min_groups_ols),
            ref_correction: self.ref_correction.unwrap_or(defaults.ref_correction),
            ref_window_rows: self.ref_window_rows.unwrap_or(defaults.ref_window_rows),
        }
    }
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path).to_string()
}

fn detector_card(primary: &HduHeader) -> Option<String> {
    primary
        .get("DETECTOR")
        .map(|d| d.trim().trim_matches('\'').trim().to_string())
        .filter(|d| !d.is_empty())
}

fn with_extension(path: &str, extension: &str) -> String {
    match path.rsplit_once('.') {
        Some((stem, _)) if !stem.ends_with(['/', '\\']) => format!("{stem}.{extension}"),
        _ => format!("{path}.{extension}"),
    }
}

fn uncal_primary(path: &str, cube: &crate::core::cube::lazy::LazyCube) -> Result<HduHeader> {
    match &cube.primary_header {
        Some(primary) => Ok(primary.clone()),
        None => read_primary_header(&ImageRef::parse(path).path),
    }
}

pub(crate) fn quick_slope_files(
    path: &str,
    output_dir: &str,
    integration: usize,
    params: &QuickSlopeParams,
    progress: Option<&ProgressHandle>,
) -> Result<serde_json::Value> {
    let started = Instant::now();
    let cube = GLOBAL_CUBE_CACHE.get_or_open(path)?;
    let info = cube.ramp().context(ERR_NOT_A_RAMP)?;
    params.validate()?;
    let plan = plan_slope(&info, params)?;
    let bands = slope_bands(plan.layout.as_ref(), cube.geometry.naxis2).len() as u64;
    if let Some(progress) = progress {
        progress.set_total(bands + PROGRESS_EXTRA_STEPS);
    }
    let tick = |stage: &str| {
        if let Some(progress) = progress {
            progress.tick_with_stage(stage);
        }
    };
    let on_band_done = |band: usize, count: usize| tick(&format!("band {} of {}", band + 1, count));
    let cancelled = || progress.is_some_and(ProgressHandle::is_cancelled);
    let product = quick_slope(&cube, &info, integration, params, &on_band_done, &cancelled)?;

    let out_dir = resolve_output_dir(output_dir)?;
    let fits_path = qslope_output_path(path, &out_dir, info.nints, integration);
    let png_path = with_extension(&fits_path, "png");
    let primary = qslope_primary_header(&uncal_primary(path, &cube)?, &info, params, &product, &file_name(&fits_path));
    tick(STAGE_WRITING);
    GLOBAL_CUBE_CACHE.invalidate(&fits_path);
    invalidate_written(&fits_path);
    let written = write_qslope(&fits_path, &primary, &product);
    invalidate_written(&fits_path);
    written?;
    save_auto_stf_preview_png(&product.sci, &png_path)?;
    let rate_sibling = sibling_rate_path(&ImageRef::parse(path).path);
    if let Some(progress) = progress {
        progress.emit_complete();
    }
    let (height, width) = product.sci.dim();
    Ok(json!({
        RES_FITS_PATH: fits_path,
        RES_PNG_PATH: png_path,
        RES_DIMENSIONS: [width, height],
        KEY_INTEGRATION: integration,
        KEY_NINTS: info.nints,
        KEY_NGROUPS: info.ngroups,
        KEY_TGROUP_S: product.tgroup_s,
        KEY_TGROUP_SOURCE: info.tgroup_source,
        KEY_DETECTOR: info.detector,
        KEY_READPATT: info.readpatt,
        KEY_REF_CORRECTED: product.ref_corrected,
        KEY_STRIPPED: product.layout.is_some(),
        KEY_PARAMS: params,
        KEY_BAND_SCALES: product.band_scales_dn,
        KEY_COUNTS: product.counts,
        KEY_RATE_SIBLING: rate_sibling,
        KEY_WARNINGS: product.warnings,
        RES_ELAPSED_MS: started.elapsed().as_millis() as u64,
    }))
}

pub(crate) fn pixel_fit_json(path: &str, x: usize, y: usize, integration: usize, params: &QuickSlopeParams) -> Result<serde_json::Value> {
    let cube = GLOBAL_CUBE_CACHE.get_or_open(path)?;
    let info = cube.ramp().context(ERR_NOT_A_RAMP)?;
    let tgroup_s = info.tgroup_s.context(ERR_TGROUP_MISSING)?;
    if integration >= info.nints {
        bail!("integration {integration} out of range (nints={})", info.nints);
    }
    params.validate()?;
    let (width, height) = (cube.geometry.naxis1, cube.geometry.naxis2);
    if x >= width || y >= height {
        bail!("pixel ({x}, {y}) is outside the {width} x {height} frame");
    }
    let plan = plan_slope(&info, params)?;
    let (amplifier, science_row) = match &plan.layout {
        Some(layout) => match layout.sample_kind(y) {
            Irs2Sample::RefOutput => bail!("row {y} lies in the IRS2 reference output: it has no slope in the product"),
            Irs2Sample::Reference => bail!("row {y} is an interleaved IRS2 reference row: it feeds the offset correction and has no slope in the product"),
            Irs2Sample::Science => (layout.amplifier_of(y), layout.science_row_of(y)),
        },
        None => (None, Some(y)),
    };
    let band = slope_bands(plan.layout.as_ref(), height)
        .into_iter()
        .find(|band| band.contains(&y))
        .with_context(|| format!("row {y} belongs to no slope band"))?;
    let z0 = integration * info.ngroups;
    let groups = cube.decode_row_band_batch(z0, z0 + info.ngroups, band.start, band.len())?;
    let science_rows = science_rows_of_band(plan.layout.as_ref(), &band);
    let offsets = match (&plan.layout, plan.apply_correction) {
        (Some(layout), true) => reference_offsets(&groups, width, band.clone(), layout, params.ref_window_rows)?,
        _ => ReferenceOffsets::zero(info.ngroups, width),
    };
    let scale = band_scale(&groups, width, band.clone(), &science_rows, &offsets, params);
    let r = y - band.start;
    let raw: Vec<f32> = groups.iter().map(|plane| plane[r * width + x]).collect();
    let ref_offsets: Vec<f32> = (0..info.ngroups).map(|g| offsets.get(r, g, x)).collect();
    let corrected: Vec<f32> = raw.iter().zip(&ref_offsets).map(|(v, o)| v - o).collect();
    let mut diffs = Vec::new();
    let mut flagged = Vec::new();
    let fit = fit_pixel(&raw, &corrected, tgroup_s, scale, params, &mut diffs, &mut flagged);
    let group_times_s: Vec<f64> = (0..info.ngroups).map(|g| g as f64 * tgroup_s).collect();
    Ok(json!({
        KEY_X: x,
        KEY_Y: y,
        KEY_INTEGRATION: integration,
        KEY_GROUP_TIMES: group_times_s,
        KEY_RAW: raw,
        KEY_CORRECTED: corrected,
        KEY_REF_OFFSETS: plan.apply_correction.then_some(ref_offsets),
        KEY_BAND_SCALE: scale,
        KEY_FIT: fit,
        KEY_FLAGGED_DIFFS: flagged,
        KEY_AMPLIFIER: amplifier,
        KEY_SCIENCE_ROW: science_row,
    }))
}

fn ratio_stem(qslope_path: &str) -> String {
    let stem = source_stem(qslope_path);
    match stem.len().checked_sub(QSLOPE_SUFFIX.len()) {
        Some(cut) if stem.get(cut..).is_some_and(|tail| tail.eq_ignore_ascii_case(QSLOPE_SUFFIX)) => stem[..cut].to_string(),
        _ => stem,
    }
}

pub(crate) fn compare_rate_files(
    qslope_path: &str,
    rate_path: &str,
    output_dir: &str,
    uncal_rows: Option<[usize; 2]>,
) -> Result<serde_json::Value> {
    let started = Instant::now();
    let qslope_primary = read_primary_header(qslope_path)?;
    if let (Some(quick_detector), Some(rate_detector)) = (detector_card(&qslope_primary), detector_card(&read_primary_header(rate_path)?)) {
        if !quick_detector.eq_ignore_ascii_case(&rate_detector) {
            bail!(
                "the quick slope {} is DETECTOR {quick_detector} but the rate {} is DETECTOR {rate_detector}: compare products of the same detector",
                file_name(qslope_path),
                file_name(rate_path)
            );
        }
    }
    let quick = read_qslope(qslope_path)?;
    let official = read_official_rate(rate_path)?;
    let layout = Irs2Layout::from_product_header(&qslope_primary)?;
    let science_rows = match (uncal_rows, &layout) {
        (None, _) => None,
        (Some([y0, y1]), Some(layout)) => Some(
            layout
                .uncal_rows_to_science_rows(y0, y1)
                .with_context(|| format!("uncal rows {y0}..{y1} hold no IRS2 science rows"))?,
        ),
        (Some([y0, y1]), None) => Some((y0, y1)),
    };
    let comparison = compare_with_rate(&quick, &official, science_rows, layout.as_ref())?;
    let ratio = ratio_image(&quick.sci, &official);
    let out_dir = resolve_output_dir(output_dir)?;
    let base = format!("{}/{}{RATIO_SUFFIX}", out_dir.trim_end_matches(['/', '\\']), ratio_stem(qslope_path));
    let ratio_fits_path = format!("{base}.fits");
    let ratio_png_path = format!("{base}.png");
    let mut header = HduHeader::empty();
    header.set(CARD_ABPROC, ABPROC_QSLOPE_RATIO.to_string());
    header.set(CARD_BUNIT, BUNIT_RATIO.to_string());
    header.set(CARD_RATIO_QSLOPE, file_name(qslope_path));
    header.set(CARD_RATIO_RATE, file_name(rate_path));
    write_derived_fits(&ratio_fits_path, &ratio, Some(&header))?;
    save_auto_stf_preview_png(&ratio, &ratio_png_path)?;
    let mut value = serde_json::to_value(&comparison)?;
    let object = value.as_object_mut().context("the comparison serialises to an object")?;
    object.insert(KEY_QSLOPE_PATH.to_string(), json!(qslope_path));
    object.insert(KEY_RATE_PATH.to_string(), json!(rate_path));
    object.insert(KEY_RATIO_PNG_PATH.to_string(), json!(ratio_png_path));
    object.insert(KEY_RATIO_FITS_PATH.to_string(), json!(ratio_fits_path));
    object.insert(RES_ELAPSED_MS.to_string(), json!(started.elapsed().as_millis() as u64));
    Ok(value)
}

#[tauri::command]
pub async fn ramp_quick_slope_cmd(
    app: tauri::AppHandle,
    path: String,
    output_dir: String,
    integration: Option<usize>,
    params: Option<QuickSlopeParamsInput>,
) -> std::result::Result<serde_json::Value, String> {
    let params = params.unwrap_or_default().resolve();
    let progress = ProgressHandle::new(&app, EVENT_QSLOPE_PROGRESS, 1);
    blocking_cmd!(quick_slope_files(&path, &output_dir, integration.unwrap_or(0), &params, Some(&progress)))
}

#[tauri::command]
pub async fn ramp_pixel_fit_cmd(
    path: String,
    x: usize,
    y: usize,
    integration: Option<usize>,
    params: Option<QuickSlopeParamsInput>,
) -> std::result::Result<serde_json::Value, String> {
    let params = params.unwrap_or_default().resolve();
    blocking_cmd!(pixel_fit_json(&path, x, y, integration.unwrap_or(0), &params))
}

#[tauri::command]
pub async fn ramp_compare_rate_cmd(
    qslope_path: String,
    rate_path: String,
    output_dir: String,
    uncal_rows: Option<[usize; 2]>,
) -> std::result::Result<serde_json::Value, String> {
    blocking_cmd!(compare_rate_files(&qslope_path, &rate_path, &output_dir, uncal_rows))
}

#[cfg(test)]
mod tests {
    use ndarray::Array2;

    use super::*;
    use crate::core::ramp::compare::read_qslope;
    use crate::core::ramp::irs2::IRS2_ALREADY_STRIPPED_NOTE;
    use crate::core::ramp::test_support::{
        irs2_synthetic_value, irs2_truth_slope, owner_like_irs2, truth_rate_planes, write_synthetic_rate, write_synthetic_uncal,
        SyntheticUncal, SYNTHETIC_ERR, SYNTHETIC_PEDESTAL_DN, SYNTHETIC_TGROUP_S,
    };

    const NRS1_NAME: &str = "jw01266005001_02103_00001_nrs1_uncal.fits";
    const NRS2_NAME: &str = "jw01266005001_02103_00001_nrs2_uncal.fits";

    fn write_irs2(dir: &tempfile::TempDir, name: &str, cols: usize, data_reversed: bool, detector: &'static str, write_fastaxis: bool) -> (String, Irs2Layout) {
        let layout = Irs2Layout::new(3200, 16, 4, 5, data_reversed).unwrap();
        let mut spec: SyntheticUncal = owner_like_irs2(cols, data_reversed);
        spec.tgroup_s = SYNTHETIC_TGROUP_S;
        spec.detector = detector;
        spec.write_fastaxis = write_fastaxis;
        let path = write_synthetic_uncal(&dir.path().join(name), &spec, irs2_synthetic_value(&layout, SYNTHETIC_TGROUP_S));
        (path, layout)
    }

    fn out_dir(dir: &tempfile::TempDir, name: &str) -> String {
        dir.path().join(name).to_str().unwrap().to_string()
    }

    fn sorted_keys(value: &serde_json::Value) -> Vec<String> {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    fn sorted(list: &[&str]) -> Vec<String> {
        let mut keys: Vec<String> = list.iter().map(|k| k.to_string()).collect();
        keys.sort();
        keys
    }

    #[test]
    fn quick_slope_files_writes_the_product_and_returns_the_contract_keys() {
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let (uncal, _) = write_irs2(&data, NRS1_NAME, 8, false, "NRS1", true);
        let result = quick_slope_files(&uncal, &out_dir(&out, "products"), 0, &QuickSlopeParams::default(), None).unwrap();
        let expected = [
            "fits_path", "png_path", "dimensions", "integration", "nints", "ngroups", "tgroup_s", "tgroup_source", "detector", "readpatt",
            "ref_corrected", "stripped", "params", "band_scales_dn", "counts", "rate_sibling", "warnings", "elapsed_ms",
        ];
        assert_eq!(sorted_keys(&result), sorted(&expected));
        assert!(result["rate_sibling"].is_null());
        let fits_path = result["fits_path"].as_str().unwrap();
        assert!(fits_path.ends_with("jw01266005001_02103_00001_nrs1_qslope.fits"), "{fits_path}");
        assert!(Path::new(fits_path).exists());
        assert!(Path::new(result["png_path"].as_str().unwrap()).exists());
        assert_eq!(result["dimensions"], json!([8, 2048]));
        assert_eq!((result["integration"].as_u64(), result["nints"].as_u64(), result["ngroups"].as_u64()), (Some(0), Some(1), Some(10)));
        assert_eq!(result["tgroup_s"].as_f64(), Some(SYNTHETIC_TGROUP_S));
        assert_eq!(result["tgroup_source"], json!("tgroup"));
        assert_eq!((result["detector"].as_str(), result["readpatt"].as_str()), (Some("NRS1"), Some("NRSIRS2RAPID")));
        assert_eq!((result["ref_corrected"].as_bool(), result["stripped"].as_bool()), (Some(true), Some(true)));
        assert_eq!(result["params"]["ref_window_rows"], json!(200));
        assert_eq!(result["params"]["ref_correction"], json!("auto"));
        assert_eq!(result["band_scales_dn"].as_array().unwrap().len(), 4);
        assert_eq!(result["counts"], json!({ "saturated": 0, "jump_det": 0, "do_not_use": 0 }));
        assert_eq!(result["warnings"], json!([]));
        assert!(result["elapsed_ms"].is_u64());
        let planes = read_qslope(fits_path).unwrap();
        assert_eq!(planes.sci.dim(), (2048, 8));
        let primary = read_primary_header(fits_path).unwrap();
        assert_eq!(primary.get("ABPROC"), Some("qslope"));
        assert_eq!(primary.get_i64("ABNFAST"), Some(3200));
    }

    #[test]
    fn quick_slope_files_reports_the_sibling_rate_of_the_uncal_when_present() {
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let (uncal, layout) = write_irs2(&data, NRS1_NAME, 8, false, "NRS1", true);
        let (sci, err, dq) = truth_rate_planes(&layout, 8);
        let rate = write_synthetic_rate(&data.path().join("jw01266005001_02103_00001_nrs1_rate.fits"), &sci, &err, &dq, "NRS1");
        let result = quick_slope_files(&uncal, &out_dir(&out, "products"), 0, &QuickSlopeParams::default(), None).unwrap();
        assert_eq!(Path::new(result["rate_sibling"].as_str().unwrap()), Path::new(&rate));
    }

    #[test]
    fn compare_rate_files_returns_the_contract_keys_and_maps_uncal_rows() {
        let data = tempfile::tempdir().unwrap();
        let products = tempfile::tempdir().unwrap();
        let compare = tempfile::tempdir().unwrap();
        let (uncal, layout) = write_irs2(&data, NRS2_NAME, 32, true, "NRS2", true);
        let (sci, err, dq) = truth_rate_planes(&layout, 32);
        let rate = write_synthetic_rate(&data.path().join("jw01266005001_02103_00001_nrs2_rate.fits"), &sci, &err, &dq, "NRS2");
        let slope = quick_slope_files(&uncal, &out_dir(&products, "products"), 0, &QuickSlopeParams::default(), None).unwrap();
        assert_eq!(Path::new(slope["rate_sibling"].as_str().unwrap()), Path::new(&rate));
        let qslope = slope["fits_path"].as_str().unwrap();
        assert!(!qslope.starts_with(data.path().to_str().unwrap()), "the product sits outside the uncal directory");

        let windowed = compare_rate_files(qslope, &rate, &out_dir(&compare, "cmp"), Some([1300, 1500])).unwrap();
        let expected = [
            "detector", "good_pixels", "quick_flagged_in_good", "bins", "amp_bins", "zero_shift", "jump", "saturated", "saturated_rule",
            "saturated_any_group", "rel_hist", "delta_hist", "science_rows", "qslope_path", "rate_path", "ratio_png_path", "ratio_fits_path",
            "elapsed_ms",
        ];
        assert_eq!(sorted_keys(&windowed), sorted(&expected));
        assert_eq!(windowed["science_rows"], json!([1040, 1200]));
        assert_eq!(windowed["zero_shift"]["passed"], json!(true), "{}", windowed["zero_shift"]);
        assert_eq!(windowed["zero_shift"]["best_dy"], json!(0));
        assert_eq!(windowed["amp_bins"].as_array().unwrap().len(), 4);
        assert_eq!(windowed["amp_bins"][1]["science_rows"], json!([1040, 1200]));
        assert_eq!(windowed["good_pixels"].as_u64(), Some(160 * 32));
        assert_eq!(windowed["detector"], json!("NRS2"));
        assert_eq!(windowed["saturated_rule"], json!("all_groups (stcal 1.15.2)"));
        for bin in windowed["bins"].as_array().unwrap() {
            assert!(bin["median_delta"].as_f64().unwrap().abs() < 1e-3, "{bin}");
        }
        assert!(Path::new(windowed["ratio_png_path"].as_str().unwrap()).exists());
        let ratio_fits = windowed["ratio_fits_path"].as_str().unwrap();
        assert!(ratio_fits.ends_with("jw01266005001_02103_00001_nrs2_ratio.fits"), "{ratio_fits}");
        assert!(Path::new(ratio_fits).exists());

        let full = compare_rate_files(qslope, &rate, &out_dir(&compare, "cmp"), None).unwrap();
        assert!(full["science_rows"].is_null());
        assert_eq!(full["good_pixels"].as_u64(), Some(2048 * 32));
        assert_eq!(full["zero_shift"]["passed"], json!(true), "{}", full["zero_shift"]);
        assert!(full["zero_shift"]["corr_at_zero"].as_f64().unwrap() > 0.999);
        let amp_rows: Vec<serde_json::Value> = full["amp_bins"].as_array().unwrap().iter().map(|a| a["science_rows"].clone()).collect();
        assert_eq!(amp_rows, vec![json!([1536, 2048]), json!([1024, 1536]), json!([512, 1024]), json!([0, 512])]);
        assert_eq!(full["quick_flagged_in_good"].as_u64(), Some(0));
    }

    #[test]
    fn compare_rate_files_fails_zero_shift_when_an_nrs2_frame_is_processed_with_the_nrs1_orientation() {
        let data = tempfile::tempdir().unwrap();
        let products = tempfile::tempdir().unwrap();
        let (mislabelled, nrs2_layout) = write_irs2(&data, "mislabelled_nrs2_as_nrs1_uncal.fits", 32, true, "NRS1", false);
        let (sci, err, dq) = truth_rate_planes(&nrs2_layout, 32);
        let rate = write_synthetic_rate(&data.path().join("nrs2_truth_rate.fits"), &sci, &err, &dq, "NRS1");
        let slope = quick_slope_files(&mislabelled, &out_dir(&products, "products"), 0, &QuickSlopeParams::default(), None).unwrap();
        assert_eq!(slope["dimensions"], json!([32, 2048]));
        let cmp = compare_rate_files(slope["fits_path"].as_str().unwrap(), &rate, &out_dir(&products, "cmp"), None).unwrap();
        assert_eq!(cmp["zero_shift"]["passed"], json!(false), "{}", cmp["zero_shift"]);
        let corr_at_zero = cmp["zero_shift"]["corr_at_zero"].as_f64().unwrap_or(f64::NAN);
        assert!(corr_at_zero.is_nan() || corr_at_zero < 0.99, "{}", cmp["zero_shift"]);
    }

    #[test]
    fn compare_rate_files_refuses_a_rate_of_another_detector() {
        let data = tempfile::tempdir().unwrap();
        let products = tempfile::tempdir().unwrap();
        let (uncal, layout) = write_irs2(&data, NRS1_NAME, 8, false, "NRS1", true);
        let (sci, err, dq) = truth_rate_planes(&layout, 8);
        let rate = write_synthetic_rate(&data.path().join("other_detector_rate.fits"), &sci, &err, &dq, "NRS2");
        let slope = quick_slope_files(&uncal, &out_dir(&products, "products"), 0, &QuickSlopeParams::default(), None).unwrap();
        let error = compare_rate_files(slope["fits_path"].as_str().unwrap(), &rate, &out_dir(&products, "cmp"), None).unwrap_err().to_string();
        assert!(error.contains("NRS1") && error.contains("NRS2"), "{error}");
    }

    #[test]
    fn compare_rate_files_succeeds_on_an_already_stripped_product_with_no_amplifier_bins() {
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let cols = 16;
        let mut spec = owner_like_irs2(cols, false);
        spec.rows = 2048;
        spec.tgroup_s = SYNTHETIC_TGROUP_S;
        let uncal = write_synthetic_uncal(&data.path().join("stripped_nrs1_uncal.fits"), &spec, |_, g, y, x| {
            SYNTHETIC_PEDESTAL_DN + irs2_truth_slope(y, x) * (SYNTHETIC_TGROUP_S * g as f64) as f32
        });
        let sci = Array2::from_shape_fn((2048, cols), |(y, x)| irs2_truth_slope(y, x));
        let err = Array2::from_elem((2048, cols), SYNTHETIC_ERR);
        let dq = Array2::zeros((2048, cols));
        let rate = write_synthetic_rate(&data.path().join("stripped_nrs1_rate.fits"), &sci, &err, &dq, "NRS1");
        let slope = quick_slope_files(&uncal, &out_dir(&out, "products"), 0, &QuickSlopeParams::default(), None).unwrap();
        assert_eq!(slope["dimensions"], json!([cols, 2048]));
        assert_eq!((slope["stripped"].as_bool(), slope["ref_corrected"].as_bool()), (Some(false), Some(false)));
        assert_eq!(slope["warnings"], json!([IRS2_ALREADY_STRIPPED_NOTE]));
        let qslope = slope["fits_path"].as_str().unwrap();
        let primary = read_primary_header(qslope).unwrap();
        assert_eq!((primary.get_i64("NRS_NORM"), primary.get_i64("ABNFAST")), (Some(16), None));
        assert_eq!(Irs2Layout::from_product_header(&primary).unwrap(), None);

        let full = compare_rate_files(qslope, &rate, &out_dir(&out, "cmp"), None).unwrap();
        assert_eq!(full["amp_bins"], json!([]));
        assert!(full["science_rows"].is_null());
        assert_eq!(full["good_pixels"].as_u64(), Some(2048 * cols as u64));
        assert_eq!(full["zero_shift"]["passed"], json!(true), "{}", full["zero_shift"]);
        for bin in full["bins"].as_array().unwrap() {
            assert!(bin["median_delta"].as_f64().unwrap().abs() < 1e-3, "{bin}");
        }
        let windowed = compare_rate_files(qslope, &rate, &out_dir(&out, "cmp"), Some([100, 300])).unwrap();
        assert_eq!(windowed["science_rows"], json!([100, 300]));
        assert_eq!(windowed["good_pixels"].as_u64(), Some(200 * cols as u64));
        assert_eq!(windowed["amp_bins"], json!([]));
    }

    #[test]
    fn pixel_fit_json_returns_raw_corrected_offsets_and_the_fit_and_matches_the_product_sci() {
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let (uncal, layout) = write_irs2(&data, NRS1_NAME, 8, false, "NRS1", true);
        let params = QuickSlopeParams::default();
        let slope = quick_slope_files(&uncal, &out_dir(&out, "products"), 0, &params, None).unwrap();
        let product = read_qslope(slope["fits_path"].as_str().unwrap()).unwrap();
        for (x, y) in [(3usize, 700usize), (0, 640), (7, 3199), (5, 1500)] {
            let fit = pixel_fit_json(&uncal, x, y, 0, &params).unwrap();
            let expected = [
                "x", "y", "integration", "group_times_s", "raw", "corrected", "ref_offsets", "band_scale_dn", "fit", "flagged_diffs", "amplifier",
                "science_row",
            ];
            assert_eq!(sorted_keys(&fit), sorted(&expected));
            let science_row = layout.science_row_of(y).unwrap();
            assert_eq!(fit["science_row"].as_u64(), Some(science_row as u64));
            assert_eq!(fit["amplifier"].as_u64(), layout.amplifier_of(y).map(|a| a as u64));
            assert_eq!(fit["fit"]["slope"].as_f64(), Some(product.sci[[science_row, x]] as f64), "pixel ({x}, {y})");
            assert_eq!(fit["fit"]["ngood"].as_u64(), Some(product.ngood[[science_row, x]] as u64));
            assert_eq!(fit["fit"]["dq"].as_u64(), Some(product.dq[[science_row, x]] as u64));
            assert_eq!(fit["raw"].as_array().unwrap().len(), 10);
            assert_eq!(fit["corrected"].as_array().unwrap().len(), 10);
            assert_eq!(fit["ref_offsets"].as_array().unwrap().len(), 10);
            assert_eq!(fit["group_times_s"][3].as_f64(), Some(3.0 * SYNTHETIC_TGROUP_S));
            assert_eq!(fit["flagged_diffs"], json!([]));
            let amp = layout.amplifier_of(y).unwrap();
            assert_eq!(fit["band_scale_dn"].as_f64(), slope["band_scales_dn"][amp].as_f64());
            let raw0 = fit["raw"][0].as_f64().unwrap();
            let offset0 = fit["ref_offsets"][0].as_f64().unwrap();
            assert_eq!(fit["corrected"][0].as_f64(), Some(((raw0 as f32) - (offset0 as f32)) as f64));
        }
        assert!(pixel_fit_json(&uncal, 3, 648, 0, &params).unwrap_err().to_string().contains("reference row"));
        assert!(pixel_fit_json(&uncal, 3, 10, 0, &params).unwrap_err().to_string().contains("reference output"));
        assert!(pixel_fit_json(&uncal, 8, 700, 0, &params).is_err());
        let off = QuickSlopeParams { ref_correction: RefCorrection::Off, ..Default::default() };
        let uncorrected = pixel_fit_json(&uncal, 3, 700, 0, &off).unwrap();
        assert!(uncorrected["ref_offsets"].is_null());
        assert_eq!(uncorrected["raw"], uncorrected["corrected"]);
    }

    #[test]
    fn params_input_resolves_missing_fields_to_the_defaults_and_rejects_invalid_values() {
        let absent: QuickSlopeParamsInput = serde_json::from_value(json!({})).unwrap();
        assert_eq!(absent.resolve(), QuickSlopeParams::default());
        let whole_band: QuickSlopeParamsInput = serde_json::from_value(json!({ "ref_window_rows": null, "jump_k": 7.5 })).unwrap();
        let resolved = whole_band.resolve();
        assert_eq!((resolved.ref_window_rows, resolved.jump_k), (None, 7.5));
        assert!(resolved.validate().is_ok());
        let narrow: QuickSlopeParamsInput = serde_json::from_value(json!({ "ref_window_rows": 30 })).unwrap();
        let resolved = narrow.resolve();
        assert_eq!(resolved.ref_window_rows, Some(30));
        assert!(resolved.validate().unwrap_err().to_string().contains("ref_window_rows"));
        let off: QuickSlopeParamsInput = serde_json::from_value(json!({ "ref_correction": "off" })).unwrap();
        assert_eq!(off.resolve().ref_correction, RefCorrection::Off);
        let amplifier: QuickSlopeParamsInput = serde_json::from_value(json!({ "ref_correction": "amplifier", "sat_dn": 50000 })).unwrap();
        let resolved = amplifier.resolve();
        assert_eq!((resolved.ref_correction, resolved.sat_dn), (RefCorrection::Amplifier, 50000.0));
        assert!(serde_json::from_value::<QuickSlopeParamsInput>(json!({ "ref_correction": "none" })).is_err());
        assert_eq!(EVENT_QSLOPE_PROGRESS, "qslope-progress");
    }
}
