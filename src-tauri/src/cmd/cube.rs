use std::time::Instant;

use ndarray::Array2;
use serde::Serialize;
use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, derived_output_header, output_stem, render_named_and_save, resolve_output_dir, write_derived_fits,
    write_derived_mef, OutputValues,
};
use crate::infra::wcs_source::load_wcs;
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::cube::companions::{open_cube_set, CubeSet};
use crate::core::cube::eager::{build_wavelength_axis, classify_spectral_cube, ramp_classification};
use crate::core::cube::lazy::{ApertureSpectrum, CollapseMode, LazyCube};
use crate::core::cube::moments::{moment_maps_set, ContinuumWindows, MomentConfig, DQ_NOT_APPLIED_NOTE};
use crate::core::cube::noise_factor::{noise_factor, NoiseFactor};
use crate::core::imaging::push_history;
use crate::core::imaging::region::RegionShape;
use crate::core::imaging::stats::percentile;
use crate::core::metadata::photcal::{FluxConvention, PhotCal, JY_PER_MJY};
use crate::infra::fits::writer::{HduData, ImageHdu};
use crate::infra::render::grayscale::render_grayscale;
use crate::types::constants::{
    RES_BITPIX, RES_FITS_PATH, RES_FRAME_INDEX, RES_FRAMES, RES_HEIGHT,
    RES_OUTPUT_PATH, RES_SPECTRUM, RES_WIDTH, RES_UNIT,
    RES_SPECTRAL_CLASSIFICATION, RES_IS_SPECTRAL, RES_SPECTRAL_REASON,
    RES_AXIS_TYPE, RES_AXIS_UNIT, RES_CHANNEL_COUNT, RES_WAVELENGTHS, RES_RAMP,
    EXTNAME_DQ, EXTNAME_ERR, EXTNAME_SCI,
};
use crate::types::header::HduHeader;

pub const KEY_SPECTRAL_AXIS: &str = "spectral_axis";
pub const APERTURE_SUBSAMPLES: u8 = 5;

const KEY_AXIS_UNIT_ASSUMED: &str = "axis_unit_assumed";
const CARD_BUNIT: &str = "BUNIT";
const CARD_CHANNEL: &str = "CHANNEL";
const CARD_SPECTRAL_TYPE: &str = "SPECTYPE";
const CARD_SPECTRAL_VALUE: &str = "SPECVAL";
const CARD_SPECTRAL_UNIT: &str = "SPECUNIT";
pub(crate) const CARD_GROUP: &str = "ABGROUP";
pub(crate) const CARD_INTEGRATION: &str = "ABINTEG";
pub(crate) const CARD_NOISE_FACTOR: &str = "ABNOISEF";
pub const ERR_SOURCE_NONE: &str = "none";
const MEDIAN_ERROR_NOTE: &str =
    "the median error is approximate: sqrt(pi/2) = 1.2533 times the error of the mean of the same channels";
const ABPROC_CUBE_FRAME: &str = "cube-frame";
const SUMMED_UNIT_SUFFIX: &str = " x channels";

const PREVIEW_LOW_PERCENTILE: f64 = 0.01;
const PREVIEW_HIGH_PERCENTILE: f64 = 0.99;
const AXIS3_KEYS: [&str; 10] = [
    "NAXIS3", "CTYPE3", "CRVAL3", "CDELT3", "CRPIX3", "CUNIT3", "CNAME3", "CRDER3", "CSYER3", "CROTA3",
];

fn is_axis3_matrix_key(key: &str) -> bool {
    let Some(body) = key.strip_prefix("CD").or_else(|| key.strip_prefix("PC")) else {
        return false;
    };
    body.starts_with("3_") || body.ends_with("_3")
}

pub fn strip_spectral_axis(header: &HduHeader) -> HduHeader {
    let mut out = header.clone();
    let keys: Vec<String> = out.cards.iter().map(|(k, _)| k.clone()).collect();
    for key in keys {
        let upper = key.trim().to_uppercase();
        if AXIS3_KEYS.contains(&upper.as_str()) || is_axis3_matrix_key(&upper) {
            out.remove(&key);
        }
    }
    if out.get("NAXIS").is_some() {
        out.set("NAXIS", "2".to_string());
    }
    if out.get("WCSAXES").is_some() || out.get("CTYPE1").is_some() {
        out.set("WCSAXES", "2".to_string());
    }
    out
}

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

fn plane_header(cube: &LazyCube, abproc: &str, values: OutputValues) -> HduHeader {
    derived_output_header(Some(&strip_spectral_axis(&cube.header)), abproc, values)
}

pub(crate) fn collapse_header(cube: &LazyCube, mode: CollapseMode) -> HduHeader {
    let abproc = format!("collapse-{}", mode.name());
    if mode != CollapseMode::Sum {
        return plane_header(cube, &abproc, OutputValues::Linear);
    }
    let mut header = plane_header(cube, &abproc, OutputValues::Rescaled);
    if let Some(unit) = card_text(&cube.header, CARD_BUNIT) {
        header.set(CARD_BUNIT, format!("{}{}", unit, SUMMED_UNIT_SUFFIX));
    }
    header
}

fn moment_header(cube: &LazyCube, order: u8, unit: &str) -> HduHeader {
    let mut header = plane_header(cube, &format!("moment{}", order), OutputValues::Rescaled);
    header.set(CARD_BUNIT, unit.to_string());
    header
}

pub(crate) fn frame_header(cube: &LazyCube, z: usize) -> HduHeader {
    let mut header = plane_header(cube, ABPROC_CUBE_FRAME, OutputValues::Linear);
    header.set(CARD_CHANNEL, z.to_string());
    if let Some(ramp) = cube.ramp() {
        header.set(CARD_GROUP, (z % ramp.ngroups).to_string());
        header.set(CARD_INTEGRATION, (z / ramp.ngroups).to_string());
    }
    if let Ok(axis) = cube.spectral_axis() {
        if let Some(value) = axis.header_values().get(z).copied().filter(|v| v.is_finite()) {
            header.set_f64(CARD_SPECTRAL_VALUE, value);
            if !axis.ctype.is_empty() {
                header.set(CARD_SPECTRAL_TYPE, axis.ctype.clone());
            }
            if !axis.header_unit.is_empty() {
                header.set(CARD_SPECTRAL_UNIT, axis.header_unit.clone());
            }
        }
    }
    header
}

pub fn linear_preview(arr: &Array2<f32>) -> Array2<f32> {
    let mut finite: Vec<f32> = arr.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return Array2::zeros(arr.dim());
    }
    let mut lo = percentile(&mut finite, PREVIEW_LOW_PERCENTILE);
    let mut hi = percentile(&mut finite, PREVIEW_HIGH_PERCENTILE);
    if hi <= lo {
        lo = finite.iter().copied().fold(f32::INFINITY, f32::min);
        hi = finite.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    }
    let range = (hi - lo).max(1e-10);
    arr.mapv(|v| if v.is_finite() { ((v - lo) / range).clamp(0.0, 1.0) } else { 0.0 })
}

fn mjy_per_sr_to_jy_factor(cube: &LazyCube) -> Option<f64> {
    let header = &cube.header;
    let wcs = load_wcs(&cube.source_path, header).ok();
    let cal = PhotCal::from_header(header, wcs.as_ref())?;
    match cal.convention {
        FluxConvention::JwstMjySr { pixar_sr, .. } => Some(pixar_sr * JY_PER_MJY),
        _ => None,
    }
}

pub(crate) fn err_source(err: Option<&LazyCube>) -> String {
    match err {
        Some(cube) => format!("{} HDU {}", EXTNAME_ERR, cube.hdu_index),
        None => ERR_SOURCE_NONE.to_string(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RegionSpectrumResponse {
    pub sum: Vec<f32>,
    pub mean: Vec<f32>,
    pub npix: f64,
    pub n_bg: usize,
    pub bg_per_pixel: Option<Vec<f32>>,
    pub wavelengths: Option<Vec<f64>>,
    pub unit: String,
    pub bg_subtracted: bool,
    pub flux_jy: Option<Vec<f64>>,
    pub err: Option<Vec<f32>>,
    pub err_scaled: Option<Vec<f32>>,
    pub mean_err: Option<Vec<f32>>,
    pub used_weight: Vec<f64>,
    pub err_source: String,
    pub bg_err_per_pixel: Option<Vec<f32>>,
    pub flux_jy_err: Option<Vec<f64>>,
    pub n_dq_excluded: Option<Vec<u32>>,
    pub noise_factor: Option<NoiseFactor>,
    pub notes: Vec<String>,
    pub elapsed_ms: u64,
}

fn region_error_notes(set: &CubeSet, spectrum: &ApertureSpectrum, background: bool, notes: &mut Vec<String>) {
    match &set.err {
        Some(err) => notes.push(format!(
            "error propagated from the ERR cube (HDU {}) as sqrt(sum w^2 ERR^2) over the aperture{}",
            err.hdu_index,
            if background { ", plus the variance of the clipped-median sky level, pi/2 times that of a mean" } else { "" }
        )),
        None => notes.push("no ERR cube: the region spectrum has no error column".to_string()),
    }
    if let Some(dq) = &set.dq {
        let dropped: u64 = spectrum.n_dq_excluded.iter().map(|&n| u64::from(n)).sum();
        let channels = spectrum.n_dq_excluded.iter().filter(|&&n| n > 0).count();
        notes.push(format!(
            "{} cube (HDU {}): aperture spaxels with bit 0 (DO_NOT_USE) dropped per channel, {} spaxel-channels over {} channels",
            EXTNAME_DQ, dq.hdu_index, dropped, channels
        ));
    }
}

fn window_channels((start, end): (usize, usize), depth: usize) -> Vec<usize> {
    (start..=end).filter(|&z| z < depth).collect()
}

fn complete_aperture_sums(spectrum: &ApertureSpectrum, windows: ContinuumWindows) -> (Vec<f32>, usize) {
    let mut sums = spectrum.sum.clone();
    let distinct = if windows.0 == windows.1 { vec![windows.0] } else { vec![windows.0, windows.1] };
    let mut skipped = 0;
    for window in distinct {
        let channels = window_channels(window, sums.len());
        let incomplete = spectrum.dropouts.incomplete_channels(&channels);
        for z in channels {
            if incomplete[z] && sums[z].is_finite() {
                sums[z] = f32::NAN;
                skipped += 1;
            }
        }
    }
    (sums, skipped)
}

fn region_noise_factor(
    spectrum: &ApertureSpectrum,
    continuum: Option<ContinuumWindows>,
    notes: &mut Vec<String>,
) -> Option<NoiseFactor> {
    let windows = continuum?;
    let Some(err) = spectrum.err.as_deref() else {
        notes.push("continuum windows given but no ERR cube: the noise factor needs the propagated error".to_string());
        return None;
    };
    let (sums, skipped) = complete_aperture_sums(spectrum, windows);
    if skipped > 0 {
        notes.push(format!(
            "{} continuum-window channels lost aperture spaxels (DQ bit 0 or non-finite flux) that other channels of the same window kept: left out of the noise factor",
            skipped
        ));
    }
    match noise_factor(&sums, err, windows) {
        Ok(factor) => {
            notes.extend(factor.warning.clone());
            Some(factor)
        }
        Err(reason) => {
            notes.push(format!("noise factor not measured: {}", reason));
            None
        }
    }
}

fn mean_errors(err: &[f32], used_weight: &[f64]) -> Vec<f32> {
    err.iter()
        .zip(used_weight)
        .map(|(&e, &w)| if w > 0.0 { (e as f64 / w) as f32 } else { f32::NAN })
        .collect()
}

pub(crate) fn region_spectrum_response(
    set: &CubeSet,
    shape: &RegionShape,
    background: Option<&RegionShape>,
    continuum: Option<ContinuumWindows>,
    t0: Instant,
) -> anyhow::Result<RegionSpectrumResponse> {
    let cube = &set.sci;
    let spectrum = cube.extract_spectrum_aperture_with(
        shape,
        background,
        APERTURE_SUBSAMPLES,
        set.err.as_deref(),
        set.dq.as_deref(),
    )?;
    let axis = cube.spectral_axis().ok();
    let jy_factor = mjy_per_sr_to_jy_factor(cube);
    let in_jy = |values: &[f32]| -> Option<Vec<f64>> {
        jy_factor.map(|factor| values.iter().map(|&v| v as f64 * factor).collect())
    };
    let flux_jy = in_jy(&spectrum.sum);
    let flux_jy_err = spectrum.err.as_deref().and_then(in_jy);
    let mut notes = set.notes.clone();
    region_error_notes(set, &spectrum, background.is_some(), &mut notes);
    let factor = region_noise_factor(&spectrum, continuum, &mut notes);
    let err_scaled = match (&spectrum.err, &factor) {
        (Some(err), Some(f)) => Some(err.iter().map(|&e| (e as f64 * f.factor) as f32).collect()),
        _ => None,
    };
    let mean_err = spectrum.err.as_deref().map(|err| mean_errors(err, &spectrum.used_weight));
    Ok(RegionSpectrumResponse {
        sum: spectrum.sum,
        mean: spectrum.mean,
        npix: spectrum.npix,
        n_bg: spectrum.n_bg,
        bg_subtracted: spectrum.bg_per_pixel.is_some(),
        bg_per_pixel: spectrum.bg_per_pixel,
        wavelengths: axis.as_ref().map(|a| a.values.clone()),
        unit: axis.map(|a| a.unit).unwrap_or_default(),
        flux_jy,
        err: spectrum.err,
        err_scaled,
        mean_err,
        used_weight: spectrum.used_weight,
        err_source: err_source(set.err.as_deref()),
        bg_err_per_pixel: spectrum.bg_err_per_pixel.filter(|_| set.err.is_some()),
        flux_jy_err,
        n_dq_excluded: set.dq.as_ref().map(|_| spectrum.n_dq_excluded),
        noise_factor: factor,
        notes,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

fn write_value_with_error(
    fits_path: &str,
    value: &Array2<f32>,
    header: &HduHeader,
    err: Option<(&Array2<f32>, &HduHeader)>,
) -> anyhow::Result<()> {
    let Some((err, err_header)) = err else {
        return write_derived_fits(fits_path, value, Some(header));
    };
    let hdus = [
        ImageHdu { data: HduData::F32(value), header: Some(header), extname: EXTNAME_SCI, extver: 1 },
        ImageHdu { data: HduData::F32(err), header: Some(err_header), extname: EXTNAME_ERR, extver: 1 },
    ];
    write_derived_mef(fits_path, None, &hdus)
}

#[derive(Debug, Clone, Serialize)]
pub struct CollapseRangeResponse {
    pub png_path: String,
    pub fits_path: String,
    pub z0: usize,
    pub z1: usize,
    pub mode: &'static str,
    pub dimensions: [usize; 2],
    pub has_err: bool,
    pub err_source: String,
    pub notes: Vec<String>,
    pub elapsed_ms: u64,
}

pub(crate) fn collapse_range_files(
    set: &CubeSet,
    path: &str,
    output_dir: &str,
    z0: usize,
    z1: usize,
    mode: CollapseMode,
    t0: Instant,
) -> anyhow::Result<CollapseRangeResponse> {
    let cube = &set.sci;
    let collapsed = cube.collapse_range_with_err(z0, z1, mode, set.err.as_deref())?;
    let arr = &collapsed.value;
    let name = format!("{}_collapse_{}_{}-{}", output_stem(path), mode.name(), z0, z1);
    let (png_path, _) = render_named_and_save(arr, output_dir, &name, false, None)?;
    let fits_path = format!("{}/{}.fits", output_dir, name);
    let header = collapse_header(cube, mode);
    let mut err_header = header.clone();
    let mut notes = set.notes.clone();
    if let (Some(err), Some(_)) = (&set.err, &collapsed.err) {
        notes.push(format!("ERR extension propagated from the ERR cube (HDU {})", err.hdu_index));
        if mode == CollapseMode::Median {
            notes.push(MEDIAN_ERROR_NOTE.to_string());
            push_history(&mut err_header, MEDIAN_ERROR_NOTE);
        }
    }
    if set.dq.is_some() {
        notes.push(DQ_NOT_APPLIED_NOTE.to_string());
    }
    write_value_with_error(&fits_path, arr, &header, collapsed.err.as_ref().map(|e| (e, &err_header)))?;
    let (rows, cols) = arr.dim();
    Ok(CollapseRangeResponse {
        png_path,
        fits_path,
        z0,
        z1,
        mode: mode.name(),
        dimensions: [cols, rows],
        has_err: collapsed.err.is_some(),
        err_source: err_source(set.err.as_deref()),
        notes,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct MapFiles {
    pub png_path: String,
    pub fits_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MomentMapsResponse {
    pub m0: MapFiles,
    pub m1: MapFiles,
    pub m2: MapFiles,
    pub m0_unit: String,
    pub velocity_unit: &'static str,
    pub noise_per_channel: Option<f64>,
    pub n_channels: usize,
    pub notes: Vec<String>,
    pub dimensions: [usize; 2],
    pub has_err: bool,
    pub err_source: String,
    pub noise_factor: Option<f64>,
    pub elapsed_ms: u64,
}

fn save_map(
    arr: &Array2<f32>,
    err: Option<&Array2<f32>>,
    header: &HduHeader,
    noise_factor: Option<f64>,
    output_dir: &str,
    name: &str,
) -> anyhow::Result<MapFiles> {
    let png_path = format!("{}/{}.png", output_dir, name);
    render_grayscale(&linear_preview(arr), &png_path)?;
    let fits_path = format!("{}/{}.fits", output_dir, name);
    match err {
        Some(err) => {
            let mut sci_header = header.clone();
            if let Some(factor) = noise_factor {
                sci_header.set_f64(CARD_NOISE_FACTOR, factor);
            }
            write_value_with_error(&fits_path, arr, &sci_header, Some((err, header)))?;
        }
        None => write_derived_fits(&fits_path, arr, Some(header))?,
    }
    Ok(MapFiles { png_path, fits_path })
}

pub(crate) fn moment_map_files(
    set: &CubeSet,
    path: &str,
    output_dir: &str,
    cfg: &MomentConfig,
    t0: Instant,
) -> anyhow::Result<MomentMapsResponse> {
    let cube = &set.sci;
    let maps = moment_maps_set(set, cfg)?;
    let stem = output_stem(path);
    let range = format!("{}-{}", cfg.z0, cfg.z1);
    let factor = maps.noise_factor;
    let m0_header = moment_header(cube, 0, &maps.m0_unit);
    let velocity_header = |order: u8| moment_header(cube, order, maps.velocity_unit);
    let name = |order: u8| format!("{}_m{}_{}", stem, order, range);
    let m0 = save_map(&maps.m0, maps.m0_err.as_ref(), &m0_header, factor, output_dir, &name(0))?;
    let m1 = save_map(&maps.m1, maps.m1_err.as_ref(), &velocity_header(1), factor, output_dir, &name(1))?;
    let m2 = save_map(&maps.m2, maps.m2_err.as_ref(), &velocity_header(2), factor, output_dir, &name(2))?;
    let (rows, cols) = maps.m0.dim();
    Ok(MomentMapsResponse {
        m0,
        m1,
        m2,
        m0_unit: maps.m0_unit,
        velocity_unit: maps.velocity_unit,
        noise_per_channel: maps.noise_per_channel,
        n_channels: maps.n_channels,
        notes: set.notes.iter().cloned().chain(maps.notes).collect(),
        dimensions: [cols, rows],
        has_err: maps.m0_err.is_some(),
        err_source: err_source(set.err.as_deref()),
        noise_factor: factor,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

fn cube_info_json(path: &str) -> anyhow::Result<serde_json::Value> {
    let cube = GLOBAL_CUBE_CACHE.get_or_open(path)?;
    let geo = &cube.geometry;
    let ramp = cube.ramp();
    let classification = match &ramp {
        Some(r) => ramp_classification(r, geo.naxis3),
        None => classify_spectral_cube(&cube.header, geo.naxis3),
    };
    Ok(json!({
        RES_WIDTH: geo.naxis1,
        RES_HEIGHT: geo.naxis2,
        RES_FRAMES: geo.depth,
        RES_BITPIX: geo.bitpix,
        RES_RAMP: ramp,
        RES_SPECTRAL_CLASSIFICATION: {
            RES_IS_SPECTRAL: classification.is_spectral,
            RES_SPECTRAL_REASON: classification.reason,
            RES_AXIS_TYPE: classification.axis_type,
            RES_AXIS_UNIT: classification.axis_unit,
            KEY_AXIS_UNIT_ASSUMED: classification.axis_unit_assumed,
            RES_CHANNEL_COUNT: classification.channel_count,
        },
        RES_WAVELENGTHS: build_wavelength_axis(&cube.header),
        KEY_SPECTRAL_AXIS: cube.spectral_axis().ok(),
    }))
}

pub(crate) fn cube_frame_json(
    path: &str,
    frame_index: usize,
    output_path: &str,
    output_fits: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let cube = GLOBAL_CUBE_CACHE.get_or_open(path)?;
    let frame = cube.get_frame(frame_index)?;
    cube.save_display_png(&frame, output_path)?;
    if let Some(fits_path) = output_fits {
        write_derived_fits(fits_path, &frame, Some(&frame_header(&cube, frame_index)))?;
    }
    Ok(json!({
        RES_FRAME_INDEX: frame_index,
        RES_OUTPUT_PATH: output_path,
        RES_FITS_PATH: output_fits,
    }))
}

const KEY_FLUX_JY: &str = "flux_jy";

fn cube_spectrum_json(path: &str, x: usize, y: usize) -> anyhow::Result<serde_json::Value> {
    let cube = GLOBAL_CUBE_CACHE.get_or_open(path)?;
    let spectrum = cube.extract_spectrum_at(y, x)?;
    let axis = cube.spectral_axis().ok();
    let classification = classify_spectral_cube(&cube.header, cube.geometry.naxis3);
    let flux_jy = mjy_per_sr_to_jy_factor(&cube).map(|f| spectrum.iter().map(|&v| v as f64 * f).collect::<Vec<f64>>());
    Ok(json!({
        RES_SPECTRUM: spectrum,
        RES_WAVELENGTHS: axis.as_ref().map(|a| a.header_values()),
        RES_UNIT: axis.map(|a| a.header_unit),
        RES_IS_SPECTRAL: classification.is_spectral,
        KEY_FLUX_JY: flux_jy,
    }))
}

fn region_spectrum_json(
    path: &str,
    shape: &RegionShape,
    background: Option<&RegionShape>,
    continuum: Option<ContinuumWindows>,
) -> anyhow::Result<serde_json::Value> {
    let t0 = Instant::now();
    let set = open_cube_set(path)?;
    let response = region_spectrum_response(&set, shape, background, continuum, t0)?;
    Ok(serde_json::to_value(&response)?)
}

fn collapse_range_json(path: &str, output_dir: &str, z0: usize, z1: usize, mode: &str) -> anyhow::Result<serde_json::Value> {
    let t0 = Instant::now();
    let mode = CollapseMode::parse(mode)?;
    let out_dir = resolve_output_dir(output_dir)?;
    let set = open_cube_set(path)?;
    let response = collapse_range_files(&set, path, &out_dir, z0, z1, mode, t0)?;
    Ok(serde_json::to_value(&response)?)
}

fn moment_maps_json(path: &str, output_dir: &str, config: &MomentConfig) -> anyhow::Result<serde_json::Value> {
    let t0 = Instant::now();
    let out_dir = resolve_output_dir(output_dir)?;
    let set = open_cube_set(path)?;
    let response = moment_map_files(&set, path, &out_dir, config, t0)?;
    Ok(serde_json::to_value(&response)?)
}

#[tauri::command]
pub async fn get_cube_info(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!(cube_info_json(&path))
}

#[tauri::command]
pub async fn get_cube_frame(
    path: String,
    frame_index: usize,
    output_path: String,
    output_fits: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!(cube_frame_json(&path, frame_index, &output_path, output_fits.as_deref()))
}

#[tauri::command]
pub async fn get_cube_spectrum(
    path: String,
    x: usize,
    y: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!(cube_spectrum_json(&path, x, y))
}

#[tauri::command]
pub async fn get_cube_spectrum_region_cmd(
    path: String,
    shape: RegionShape,
    background: Option<RegionShape>,
    continuum: Option<ContinuumWindows>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!(region_spectrum_json(&path, &shape, background.as_ref(), continuum))
}

#[tauri::command]
pub async fn collapse_cube_range_cmd(
    path: String,
    output_dir: String,
    z0: usize,
    z1: usize,
    mode: String,
) -> Result<serde_json::Value, String> {
    blocking_cmd!(collapse_range_json(&path, &output_dir, z0, z1, &mode))
}

#[tauri::command]
pub async fn moment_maps_cmd(
    path: String,
    output_dir: String,
    config: MomentConfig,
) -> Result<serde_json::Value, String> {
    blocking_cmd!(moment_maps_json(&path, &output_dir, &config))
}

#[cfg(test)]
mod tests {
    use crate::core::astrometry::wcs::WcsTransform;
    use std::fs::File;

    use super::*;
    use crate::cmd::common::HEADER_ABPROC;
    use crate::core::astrometry::spectral::VelocityConvention;
    use crate::core::cube::companions::open_cube_set;
    use crate::core::cube::lazy::{region_pixels, test_support::*};
    use crate::core::cube::moments::{moment_maps_set, ContinuumWindows, DEFAULT_SNR_THRESHOLD, DQ_NOT_APPLIED_NOTE};
    use crate::core::cube::noise_factor::noise_factor;
    use crate::infra::fits::reader::extract_image_mmap;
    use crate::infra::image_source::load_plane;
    use crate::types::constants::{EXTNAME_ERR, EXTNAME_SCI};
    use crate::types::image_ref::ImageRef;

    fn alone(cube: LazyCube) -> CubeSet {
        CubeSet { sci: std::sync::Arc::new(cube), err: None, dq: None, notes: Vec::new() }
    }

    fn open_cube(dir: &tempfile::TempDir, name: &str, noise: f32) -> (String, CubeSet) {
        let path = dir.path().join(name);
        write_line_cube(&path, noise);
        let text = path.to_str().unwrap().to_string();
        let set = alone(LazyCube::open(&text).unwrap());
        (text, set)
    }

    fn disk() -> RegionShape {
        RegionShape::Circle { x: LINE_DISK_CENTRE, y: LINE_DISK_CENTRE, r: LINE_DISK_RADIUS }
    }

    fn line_moment_config() -> MomentConfig {
        MomentConfig {
            z0: 8,
            z1: 32,
            rest_um: None,
            convention: VelocityConvention::Optical,
            continuum: Some(((0, 5), (34, 39))),
            snr_threshold: DEFAULT_SNR_THRESHOLD,
            mask_below_threshold: true,
        }
    }

    fn reopen(path: &str) -> crate::infra::fits::reader::MmapImageResult {
        extract_image_mmap(&File::open(path).unwrap()).unwrap()
    }

    const GOLDEN_NOISE: f32 = 0.01;
    const GOLDEN_COLLAPSE_RANGE: (usize, usize) = (15, 25);
    const FITS_CARD_BYTES: usize = 80;
    const PRE_ERR_REGION_FIELDS: [&str; 9] =
        ["sum", "mean", "npix", "n_bg", "bg_per_pixel", "wavelengths", "unit", "bg_subtracted", "flux_jy"];
    const GOLDEN_NO_ERR_SHA256: [(&str, &str); 8] = [
        ("region", "a3451f6ef209921f1299e171300d1cc1abdcb570aaf090c91fff77ea9146eb9b"),
        ("region_bg", "d963038ab3b471e8d5c1292fd9e6bd6b1a10e0a64effb13eb4b31eab31d7a53c"),
        ("sum", "68f9c31d54058218d084f698f845d6f1bb5940121648e2732f3a117cdbae48c5"),
        ("mean", "da223b7c0e33a697597cf660de8c3494bb489ab3809ae51e672d7cafc008683b"),
        ("median", "870e6d195ab6da283d16c0a346dd2312d4c3c7cbfde6e1e1225891c0ec5cfdaf"),
        ("m0", "095e193d408e1c40c694c31e0a65f3ea0486777f7a7cc1dfa4c094e4b002a441"),
        ("m1", "ce87c7fd3458b3b33da49031979a939888bd1e449786af563e2a04d69b27aafd"),
        ("m2", "a2be6f5271aff2ce68d86e441966df76e150410a12d04a8ab24d86e982963335"),
    ];

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes).iter().map(|b| format!("{:02x}", b)).collect()
    }

    fn without_version_stamp(bytes: &[u8]) -> Vec<u8> {
        let stamp = format!("AstroBurst {}", env!("CARGO_PKG_VERSION"));
        bytes
            .chunks(FITS_CARD_BYTES)
            .flat_map(|card| {
                if card.windows(stamp.len()).any(|w| w == stamp.as_bytes()) {
                    let text = String::from_utf8_lossy(card).replace(&stamp, "AstroBurst");
                    format!("{:<width$}", text.trim_end(), width = FITS_CARD_BYTES).into_bytes()
                } else {
                    card.to_vec()
                }
            })
            .collect()
    }

    fn fits_sha256(path: &str) -> String {
        sha256_hex(&without_version_stamp(&std::fs::read(path).unwrap()))
    }

    fn pre_err_region_sha256(response: &RegionSpectrumResponse) -> String {
        let value = serde_json::to_value(response).unwrap();
        let fields: Vec<(&str, &serde_json::Value)> = PRE_ERR_REGION_FIELDS.iter().map(|k| (*k, &value[*k])).collect();
        sha256_hex(serde_json::to_string(&fields).unwrap().as_bytes())
    }

    fn no_err_output_hashes(dir: &tempfile::TempDir) -> Vec<(&'static str, String)> {
        let (path, _) = open_cube(dir, "golden.fits", GOLDEN_NOISE);
        let set = open_cube_set(&path).unwrap();
        assert!(set.err.is_none() && set.dq.is_none());
        let out_dir = dir.path().join("golden_out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let out = out_dir.to_str().unwrap();
        let background = RegionShape::Annulus { x: LINE_DISK_CENTRE, y: LINE_DISK_CENTRE, r_inner: 9.0, r_outer: 13.0 };
        let mut hashes = vec![
            ("region", pre_err_region_sha256(&region_spectrum_response(&set, &disk(), None, None, Instant::now()).unwrap())),
            (
                "region_bg",
                pre_err_region_sha256(
                    &region_spectrum_response(&set, &disk(), Some(&background), None, Instant::now()).unwrap(),
                ),
            ),
        ];
        let (z0, z1) = GOLDEN_COLLAPSE_RANGE;
        for mode in [CollapseMode::Sum, CollapseMode::Mean, CollapseMode::Median] {
            let files = collapse_range_files(&set, &path, out, z0, z1, mode, Instant::now()).unwrap();
            assert!(!files.has_err);
            hashes.push((mode.name(), fits_sha256(&files.fits_path)));
        }
        let maps = moment_map_files(&set, &path, out, &line_moment_config(), Instant::now()).unwrap();
        assert!(!maps.has_err && maps.noise_factor.is_none());
        for (name, files) in [("m0", &maps.m0), ("m1", &maps.m1), ("m2", &maps.m2)] {
            hashes.push((name, fits_sha256(&files.fits_path)));
        }
        GLOBAL_CUBE_CACHE.invalidate(&path);
        hashes
    }

    #[test]
    fn no_err_region_collapse_and_moment_outputs_match_their_pre_error_golden_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let expected: Vec<(&str, String)> = GOLDEN_NO_ERR_SHA256.iter().map(|&(k, v)| (k, v.to_string())).collect();
        assert_eq!(no_err_output_hashes(&dir), expected);
    }

    #[test]
    fn the_golden_hash_ignores_only_the_version_stamp_cards() {
        let stamp = format!("AstroBurst {}", env!("CARGO_PKG_VERSION"));
        let card = |text: &str| format!("{:<width$}", text, width = FITS_CARD_BYTES).into_bytes();
        let stamped = [card(&format!("PROGRAM = '{}'", stamp)), card("BUNIT   = 'Jy/beam'")].concat();
        let other = [card("PROGRAM = 'AstroBurst 9.9.99'"), card("BUNIT   = 'Jy/beam'")].concat();
        let normalised = without_version_stamp(&stamped);
        assert_eq!(normalised.len(), stamped.len());
        assert_eq!(&normalised[..FITS_CARD_BYTES], card("PROGRAM = 'AstroBurst'").as_slice());
        assert_eq!(&normalised[FITS_CARD_BYTES..], &stamped[FITS_CARD_BYTES..]);
        assert_eq!(without_version_stamp(&other), other);
    }

    #[test]
    fn open_cube_set_resolves_the_err_and_dq_companions_of_a_mef_cube() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("set_s3d.fits");
        write_mef_cube(&path, 4, 3, 6, &[("SCI", 10.0), ("ERR", 0.5), ("DQ", 0.0)]);
        let key = path.to_str().unwrap();
        let set = open_cube_set(key).unwrap();
        assert_eq!(set.sci.hdu_index, 1);
        assert_eq!(set.err.as_ref().map(|c| c.hdu_index), Some(2));
        assert_eq!(set.dq.as_ref().map(|c| c.hdu_index), Some(3));
        assert_eq!(set.err.as_ref().unwrap().extract_spectrum_at(2, 3).unwrap()[4], 4.5);
        assert_eq!(set.dq.as_ref().unwrap().extract_spectrum_at(0, 1).unwrap()[5], 5.0);
        let refused = open_cube_set(&format!("{}#hdu=3", key)).err().expect("the DQ plane must be refused").to_string();
        assert!(refused.contains("opens the DQ extension (HDU 3): cube products need the SCI cube"), "{}", refused);
        GLOBAL_CUBE_CACHE.invalidate(key);
    }

    const SET_NOISE: f32 = 0.01;
    const SET_ERR: f32 = 0.005_773_5;
    const SET_ERR_SOURCE: &str = "ERR HDU 2";
    const SET_WINDOWS: ContinuumWindows = ((0, 5), (34, 39));
    const REGION_WINDOWS: ContinuumWindows = ((0, 6), (34, 39));
    const RELATIVE_TOLERANCE: f64 = 1e-6;

    fn write_set(dir: &tempfile::TempDir, name: &str, dq: impl Fn(usize, usize, usize) -> i32) -> String {
        let path = dir.path().join(name);
        write_line_cube_mef(&path, SET_NOISE, |_, _, _| SET_ERR, dq);
        path.to_str().unwrap().to_string()
    }

    fn unflagged(_: usize, _: usize, _: usize) -> i32 {
        0
    }

    fn out_dir(dir: &tempfile::TempDir) -> String {
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        out.to_str().unwrap().to_string()
    }

    fn same_bits(a: &Array2<f32>, b: &Array2<f32>) -> bool {
        a.dim() == b.dim() && a.iter().zip(b.iter()).all(|(x, y)| x.to_bits() == y.to_bits())
    }

    fn extname(plane: &crate::infra::image_source::LoadedPlane) -> Option<String> {
        plane.info.extname.as_deref().map(|n| n.trim().trim_matches('\'').trim().to_string())
    }

    fn invalidate_set(key: &str) {
        for hdu in ["", "#hdu=1", "#hdu=2", "#hdu=3"] {
            GLOBAL_CUBE_CACHE.invalidate(&format!("{}{}", key, hdu));
        }
    }

    #[test]
    fn a_collapse_of_a_cube_with_err_writes_sci_and_err_extensions_that_the_viewer_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_set(&dir, "collapse_s3d.fits", unflagged);
        let out = out_dir(&dir);
        let (z0, z1) = (15, 25);
        let n = (z1 - z0 + 1) as f64;
        let sum_err = SET_ERR as f64 * n.sqrt();
        let sci = LazyCube::open(&format!("{}#hdu=1", key)).unwrap();
        for (mode, expected_err) in [
            (CollapseMode::Sum, sum_err),
            (CollapseMode::Mean, sum_err / n),
            (CollapseMode::Median, std::f64::consts::FRAC_PI_2.sqrt() * sum_err / n),
        ] {
            let value = collapse_range_json(&key, &out, z0, z1, mode.name()).unwrap();
            assert_eq!(value["has_err"], true, "{}", value);
            assert_eq!(value["err_source"], SET_ERR_SOURCE, "{}", value);
            let fits = value["fits_path"].as_str().unwrap();
            let plane = load_plane(&ImageRef::parse(fits)).unwrap();
            assert_eq!(extname(&plane).as_deref(), Some(EXTNAME_SCI));
            assert!(same_bits(&plane.arr, &sci.collapse_range(z0, z1, mode).unwrap()), "{} values", mode.name());
            assert_eq!(plane.header.get(HEADER_ABPROC), Some(format!("collapse-{}", mode.name()).as_str()));
            let err_ref = plane.companions.err.clone().expect("the ERR extension is the companion of SCI");
            let err_plane = load_plane(&err_ref).unwrap();
            assert_eq!(extname(&err_plane).as_deref(), Some(EXTNAME_ERR));
            for &e in err_plane.arr.iter() {
                assert!((e as f64 - expected_err).abs() <= RELATIVE_TOLERANCE * expected_err, "{} error {}", mode.name(), e);
            }
            let notes: Vec<&str> = value["notes"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
            assert_eq!(notes.iter().any(|n| n.contains("approximate")), mode == CollapseMode::Median, "{:?}", notes);
            assert!(notes.iter().any(|n| n.contains(DQ_NOT_APPLIED_NOTE)), "{:?}", notes);
        }
        invalidate_set(&key);
    }

    #[test]
    fn moment_maps_of_a_cube_with_err_are_mef_files_whose_sci_carries_abnoisef_and_whose_err_the_viewer_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_set(&dir, "moments_s3d.fits", unflagged);
        let out = out_dir(&dir);
        let value = moment_maps_json(&key, &out, &line_moment_config()).unwrap();
        assert_eq!(value["has_err"], true, "{}", value);
        assert_eq!(value["err_source"], SET_ERR_SOURCE, "{}", value);
        let factor = value["noise_factor"].as_f64().expect("noise factor with continuum windows and ERR");
        let maps = moment_maps_set(&open_cube_set(&key).unwrap(), &line_moment_config()).unwrap();
        assert_eq!(maps.noise_factor, Some(factor));
        let planes = [
            ("m0", &maps.m0, maps.m0_err.as_ref().unwrap()),
            ("m1", &maps.m1, maps.m1_err.as_ref().unwrap()),
            ("m2", &maps.m2, maps.m2_err.as_ref().unwrap()),
        ];
        for (name, map, err_map) in planes {
            let fits = value[name]["fits_path"].as_str().unwrap();
            let plane = load_plane(&ImageRef::parse(fits)).unwrap();
            assert_eq!(extname(&plane).as_deref(), Some(EXTNAME_SCI), "{}", name);
            assert!(same_bits(&plane.arr, map), "{} values", name);
            let card = plane.header.get_f64(CARD_NOISE_FACTOR).unwrap_or_else(|| panic!("{} SCI has no {}", name, CARD_NOISE_FACTOR));
            assert!((card - factor).abs() <= 1e-13 * factor, "{} card {} factor {}", name, card, factor);
            let err_plane = load_plane(&plane.companions.err.clone().expect("ERR companion")).unwrap();
            assert_eq!(extname(&err_plane).as_deref(), Some(EXTNAME_ERR), "{}", name);
            assert!(same_bits(&err_plane.arr, err_map), "{} error values", name);
            assert!(err_plane.header.get(CARD_NOISE_FACTOR).is_none(), "{} ERR carries the factor card", name);
            assert_eq!(err_plane.header.get("BUNIT"), plane.header.get("BUNIT"), "{}", name);
        }
        invalidate_set(&key);
    }

    const TRANSIENT_FLAG_CHANNELS: [usize; 2] = [3, 4];
    const CENTRE_SPAXEL: (usize, usize) = (16, 16);
    const JUMP_ONLY_SPAXEL: (usize, usize) = (16, 17);
    const DO_NOT_USE: i32 = 1;
    const JUMP_DET: i32 = 4;

    fn centre_flagged_in_two_channels(z: usize, y: usize, x: usize) -> i32 {
        if (y, x) == CENTRE_SPAXEL && TRANSIENT_FLAG_CHANNELS.contains(&z) {
            DO_NOT_USE
        } else if (y, x) == JUMP_ONLY_SPAXEL {
            JUMP_DET
        } else {
            0
        }
    }

    fn f32_array(value: &serde_json::Value) -> Vec<f32> {
        value.as_array().unwrap_or_else(|| panic!("not an array: {}", value)).iter().map(|v| v.as_f64().unwrap() as f32).collect()
    }

    fn notes_of(value: &serde_json::Value) -> Vec<String> {
        value["notes"].as_array().unwrap().iter().map(|n| n.as_str().unwrap().to_string()).collect()
    }

    fn aperture_weight_squares(skip: Option<(usize, usize)>) -> f64 {
        region_pixels(&disk(), LINE_CUBE_SIZE, LINE_CUBE_SIZE, APERTURE_SUBSAMPLES)
            .unwrap()
            .iter()
            .filter(|p| Some((p.y, p.x)) != skip)
            .map(|p| (p.weight as f64).powi(2))
            .sum()
    }

    #[test]
    fn a_region_spectrum_of_a_cube_with_err_carries_the_propagated_error_its_source_the_dq_counts_and_the_noise_factor() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_set(&dir, "region_s3d.fits", centre_flagged_in_two_channels);
        let value = region_spectrum_json(&key, &disk(), None, Some(REGION_WINDOWS)).unwrap();
        let set = open_cube_set(&key).unwrap();
        let spectrum = set
            .sci
            .extract_spectrum_aperture_with(&disk(), None, APERTURE_SUBSAMPLES, set.err.as_deref(), set.dq.as_deref())
            .unwrap();
        assert_eq!(value["err_source"], SET_ERR_SOURCE);
        let err = f32_array(&value["err"]);
        assert_eq!(err.len(), LINE_CUBE_DEPTH);
        for (z, &e) in err.iter().enumerate() {
            let skip = TRANSIENT_FLAG_CHANNELS.contains(&z).then_some(CENTRE_SPAXEL);
            let expected = SET_ERR as f64 * aperture_weight_squares(skip).sqrt();
            assert!((e as f64 - expected).abs() <= RELATIVE_TOLERANCE * expected, "channel {}: {} vs {}", z, e, expected);
        }
        assert_eq!(f32_array(&value["sum"]), spectrum.sum);
        let expected_excluded: Vec<u64> =
            (0..LINE_CUBE_DEPTH).map(|z| u64::from(TRANSIENT_FLAG_CHANNELS.contains(&z))).collect();
        let reported_excluded: Vec<u64> = value["n_dq_excluded"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
        assert_eq!(reported_excluded, expected_excluded);
        let mut complete = spectrum.sum.clone();
        for &z in &TRANSIENT_FLAG_CHANNELS {
            complete[z] = f32::NAN;
        }
        let factor = noise_factor(&complete, &err, REGION_WINDOWS).unwrap();
        let reported = &value["noise_factor"];
        assert_eq!(reported["factor"].as_f64(), Some(factor.factor), "{}", reported);
        assert_eq!(reported["n_channels"], 11);
        assert_eq!(reported["windows"], serde_json::json!([[0, 6], [34, 39]]));
        let relative_se = factor.factor_se / factor.factor;
        assert!((factor.factor - 1.0).abs() <= 3.0 * relative_se, "factor {} relative SE {}", factor.factor, relative_se);
        assert!(reported["warning"].is_null(), "{}", reported);
        assert!(
            notes_of(&value).iter().any(|n| n.contains("2 continuum-window channels lost aperture spaxels")),
            "{:?}",
            notes_of(&value)
        );
        let scaled = f32_array(&value["err_scaled"]);
        for z in 0..LINE_CUBE_DEPTH {
            assert_eq!(scaled[z], (err[z] as f64 * factor.factor) as f32, "channel {}", z);
        }
        let used_weight: Vec<f64> =
            value["used_weight"].as_array().expect("used weight per channel").iter().map(|w| w.as_f64().unwrap()).collect();
        assert_eq!(used_weight, spectrum.used_weight);
        let mean_err = f32_array(&value["mean_err"]);
        for z in 0..LINE_CUBE_DEPTH {
            let back = mean_err[z] as f64 * used_weight[z];
            assert!((back - err[z] as f64).abs() <= RELATIVE_TOLERANCE * err[z] as f64, "channel {}: {} vs {}", z, back, err[z]);
        }
        assert!(used_weight[TRANSIENT_FLAG_CHANNELS[0]] < used_weight[0], "{:?}", used_weight);
        assert!(value["flux_jy_err"].is_null());
        assert!(notes_of(&value).iter().any(|n| n.contains("ERR cube (HDU 2)")), "{:?}", notes_of(&value));
        assert!(notes_of(&value).iter().any(|n| n.contains("DQ cube (HDU 3)")), "{:?}", notes_of(&value));

        let windowless = region_spectrum_json(&key, &disk(), None, None).unwrap();
        assert!(windowless["noise_factor"].is_null() && windowless["err_scaled"].is_null(), "{}", windowless);
        assert_eq!(f32_array(&windowless["err"]), err);

        let short = region_spectrum_json(&key, &disk(), None, Some(((0, 3), (34, 39)))).unwrap();
        assert!(short["noise_factor"].is_null() && short["err_scaled"].is_null(), "{}", short);
        assert_eq!(f32_array(&short["sum"]), spectrum.sum);
        assert!(notes_of(&short).iter().any(|n| n.contains("continuum window 0..=3 has 4 channels")), "{:?}", notes_of(&short));
        invalidate_set(&key);
    }

    #[test]
    fn a_region_spectrum_in_mjy_per_sr_converts_its_error_to_jy_and_a_cube_without_err_reports_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mjy_s3d.fits");
        let pixar_sr = 3.0461742e-12;
        write_line_cube_mef_with_cards(&path, &[("BUNIT", "'MJy/sr'"), ("PIXAR_SR", "3.0461742E-12")], SET_NOISE, |_, _, _| SET_ERR, unflagged);
        let key = path.to_str().unwrap();
        let value = region_spectrum_json(key, &disk(), None, None).unwrap();
        let err = f32_array(&value["err"]);
        let flux_jy_err = value["flux_jy_err"].as_array().expect("error in Jy");
        assert_eq!(flux_jy_err.len(), LINE_CUBE_DEPTH);
        for z in 0..LINE_CUBE_DEPTH {
            let expected = err[z] as f64 * pixar_sr * JY_PER_MJY;
            let got = flux_jy_err[z].as_f64().unwrap();
            assert!((got - expected).abs() <= 1e-12 * expected, "channel {}: {} vs {}", z, got, expected);
        }
        invalidate_set(key);

        let (plain, _) = open_cube(&dir, "plain.fits", SET_NOISE);
        let value = region_spectrum_json(&plain, &disk(), None, Some(SET_WINDOWS)).unwrap();
        assert_eq!(value["err_source"], ERR_SOURCE_NONE);
        for key in ["err", "err_scaled", "mean_err", "flux_jy_err", "n_dq_excluded", "noise_factor", "bg_err_per_pixel"] {
            assert!(value[key].is_null(), "{} = {}", key, value[key]);
        }
        assert_eq!(value["used_weight"].as_array().map(Vec::len), Some(LINE_CUBE_DEPTH), "{}", value["used_weight"]);
        assert!(notes_of(&value).iter().any(|n| n.contains("no ERR cube")), "{:?}", notes_of(&value));
        GLOBAL_CUBE_CACHE.invalidate(&plain);
    }

    #[test]
    fn cube_products_of_a_file_cut_short_inside_its_err_extension_still_run_and_say_why_they_have_no_error() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_set(&dir, "cut_s3d.fits", unflagged);
        let file = File::open(&key).unwrap();
        let err_data_start = crate::infra::fits::reader::list_extensions(&file).unwrap()[2].data_start;
        drop(file);
        let mut bytes = std::fs::read(&key).unwrap();
        bytes.truncate(err_data_start + 1000);
        std::fs::write(&key, &bytes).unwrap();
        let reason = |notes: &[String]| {
            notes.iter().any(|n| n.starts_with("ERR extension (HDU 2) could not be read: ") && n.ends_with("products have no error column"))
        };
        let region = region_spectrum_json(&key, &disk(), None, Some(SET_WINDOWS)).unwrap();
        assert_eq!(region["err_source"], ERR_SOURCE_NONE);
        assert!(region["err"].is_null() && region["noise_factor"].is_null(), "{}", region);
        assert!(reason(&notes_of(&region)), "{:?}", notes_of(&region));
        let out = out_dir(&dir);
        let collapse = collapse_range_json(&key, &out, 15, 25, "sum").unwrap();
        assert_eq!(collapse["has_err"], false);
        assert!(reason(&notes_of(&collapse)), "{:?}", notes_of(&collapse));
        let moments = moment_maps_json(&key, &out, &line_moment_config()).unwrap();
        assert_eq!(moments["has_err"], false);
        assert!(reason(&notes_of(&moments)), "{:?}", notes_of(&moments));
        invalidate_set(&key);
    }

    #[test]
    fn strip_spectral_axis_removes_every_third_axis_card_and_keeps_bunit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pc.fits");
        write_line_cube_with_cards(&path, &[("PC3_3", "1.0"), ("PC1_3", "0.0"), ("CD2_3", "0.0"), ("CNAME3", "'wave'")]);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let header = strip_spectral_axis(&cube.header);
        for key in ["NAXIS3", "CTYPE3", "CRVAL3", "CRPIX3", "CUNIT3", "CD3_3", "PC3_3", "PC1_3", "CD2_3", "CNAME3"] {
            assert!(header.get(key).is_none(), "{} survived", key);
            assert!(!header.cards.iter().any(|(k, _)| k == key), "{} card survived", key);
        }
        assert_eq!(header.get("WCSAXES"), Some("2"));
        assert_eq!(header.get("NAXIS"), Some("2"));
        assert_eq!(header.get("BUNIT"), Some("Jy/beam"));
        assert_eq!(header.get("CTYPE1"), Some("RA---TAN"));
        assert_eq!(header.get("CD1_1"), Some("-1.0E-4"));
        assert_eq!(header.get("RESTWAV"), Some("1.020E-6"));
        assert!(is_axis3_matrix_key("PC3_1") && is_axis3_matrix_key("CD1_3") && !is_axis3_matrix_key("CD1_2"));
        assert!(!is_axis3_matrix_key("PCOUNT"));
    }

    #[test]
    fn collapsed_range_fits_reopens_as_a_2d_image_with_the_wcs_intact() {
        let dir = tempfile::tempdir().unwrap();
        let (path, set) = open_cube(&dir, "line.fits", 0.0);
        let out_dir = dir.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let out = collapse_range_files(&set, &path, out_dir.to_str().unwrap(), 15, 25, CollapseMode::Sum, Instant::now()).unwrap();
        assert_eq!(out.mode, "sum");
        assert_eq!(out.dimensions, [LINE_CUBE_SIZE, LINE_CUBE_SIZE]);
        assert!(out.fits_path.ends_with("line_collapse_sum_15-25.fits"));
        assert!(std::path::Path::new(&out.png_path).exists());

        let reopened = reopen(&out.fits_path);
        assert_eq!(reopened.image.dim(), (LINE_CUBE_SIZE, LINE_CUBE_SIZE));
        assert_eq!(reopened.header.get_i64("NAXIS"), Some(2));
        assert!(reopened.header.get("NAXIS3").is_none());
        assert!(reopened.header.get("CTYPE3").is_none());
        assert!(reopened.header.get("CD3_3").is_none());
        assert_eq!(reopened.header.get_i64("WCSAXES"), Some(2));
        assert_eq!(reopened.header.get("BUNIT"), Some("Jy/beam x channels"));
        assert_eq!(reopened.header.get(HEADER_ABPROC), Some("collapse-sum"));
        let n = 11.0f32;
        let line_sum: f32 = (15..=25).map(line_profile).sum();
        let centre = (LINE_DISK_CENTRE as usize, LINE_DISK_CENTRE as usize);
        assert!((reopened.image[centre] - (n + line_sum)).abs() < 1e-3);
        assert!((reopened.image[[0, 0]] - n).abs() < 1e-5);

        let original = WcsTransform::from_header(&set.sci.header).unwrap();
        let written = WcsTransform::from_header(&reopened.header).unwrap();
        let a = original.pixel_to_world(10.0, 10.0);
        let b = written.pixel_to_world(10.0, 10.0);
        assert!((a.ra - b.ra).abs() < 1e-9 && (a.dec - b.dec).abs() < 1e-9, "{:?} vs {:?}", (a.ra, a.dec), (b.ra, b.dec));
    }

    #[test]
    fn a_summed_collapse_drops_the_flux_density_calibration_and_a_mean_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mjy_cube.fits");
        write_line_cube_with_cards(
            &path,
            &[("BUNIT", "'MJy/sr'"), ("PIXAR_SR", "1.0E-13"), ("PIXAR_A2", "0.0042"), ("SATURATE", "5000.0"), ("DATAMAX", "3.0")],
        );
        let key = path.to_str().unwrap();
        let set = alone(LazyCube::open(key).unwrap());
        let out = dir.path().join("out");
        let out = out.to_str().unwrap();
        std::fs::create_dir_all(out).unwrap();

        let sum = collapse_range_files(&set, key, out, 0, 39, CollapseMode::Sum, Instant::now()).unwrap();
        let header = reopen(&sum.fits_path).header;
        assert_eq!(header.get("BUNIT"), Some("MJy/sr x channels"));
        for dropped in ["PIXAR_SR", "PIXAR_A2", "SATURATE", "DATAMAX"] {
            assert!(header.get(dropped).is_none(), "{} survived a channel sum", dropped);
        }
        assert_eq!(header.get(HEADER_ABPROC), Some("collapse-sum"));
        assert!(PhotCal::from_header(&header, None).is_none(), "a channel sum was calibrated as a flux density");

        let mean = collapse_range_files(&set, key, out, 0, 39, CollapseMode::Mean, Instant::now()).unwrap();
        let header = reopen(&mean.fits_path).header;
        assert_eq!(header.get("BUNIT"), Some("MJy/sr"));
        assert!(header.get("PIXAR_SR").is_some());
        assert!(header.get("DATAMAX").is_none());
        assert_eq!(header.get(HEADER_ABPROC), Some("collapse-mean"));
        assert!(matches!(
            PhotCal::from_header(&header, None).map(|c| c.convention),
            Some(FluxConvention::JwstMjySr { .. })
        ));
    }

    #[test]
    fn region_spectrum_reports_flux_in_jy_only_for_mjy_per_sr_cubes() {
        let dir = tempfile::tempdir().unwrap();
        let (_, set) = open_cube(&dir, "line.fits", 0.0);
        let response = region_spectrum_response(&set, &disk(), None, None, Instant::now()).unwrap();
        assert!(response.flux_jy.is_none());
        assert!(!response.bg_subtracted);
        assert_eq!(response.unit, "um");
        let wavelengths = response.wavelengths.as_ref().unwrap();
        assert_eq!(wavelengths.len(), LINE_CUBE_DEPTH);
        assert!((wavelengths[20] - LINE_REST_UM).abs() < 1e-12);
        assert!((response.npix - disk_pixel_count() as f64).abs() < 1.0);

        let path = dir.path().join("mjy.fits");
        write_line_cube_with_cards(&path, &[("BUNIT", "'MJy/sr'"), ("PIXAR_SR", "3.0461742E-12")]);
        let mjy = alone(LazyCube::open(path.to_str().unwrap()).unwrap());
        let background = RegionShape::Annulus { x: LINE_DISK_CENTRE, y: LINE_DISK_CENTRE, r_inner: 9.0, r_outer: 13.0 };
        let response = region_spectrum_response(&mjy, &disk(), Some(&background), None, Instant::now()).unwrap();
        assert!(response.bg_subtracted);
        let flux = response.flux_jy.as_ref().expect("flux in Jy");
        assert_eq!(flux.len(), LINE_CUBE_DEPTH);
        for z in 0..LINE_CUBE_DEPTH {
            let expected = response.sum[z] as f64 * 3.0461742e-12 * JY_PER_MJY;
            assert!((flux[z] - expected).abs() <= 1e-12 * expected.abs().max(1.0), "z={}", z);
        }
        let line_weight: f64 = region_pixels(&disk(), LINE_CUBE_SIZE, LINE_CUBE_SIZE, APERTURE_SUBSAMPLES)
            .unwrap()
            .iter()
            .filter(|p| inside_disk(p.y, p.x))
            .map(|p| p.weight as f64)
            .sum();
        assert!(line_weight < response.npix);
        assert!((response.sum[20] as f64 - line_weight).abs() < 1e-3, "sum={} weight={}", response.sum[20], line_weight);
    }

    #[test]
    fn moment_map_files_write_three_2d_fits_with_their_units() {
        let dir = tempfile::tempdir().unwrap();
        let (path, set) = open_cube(&dir, "line.fits", 0.01);
        let out_dir = dir.path().join("maps");
        std::fs::create_dir_all(&out_dir).unwrap();
        let out = moment_map_files(&set, &path, out_dir.to_str().unwrap(), &line_moment_config(), Instant::now()).unwrap();
        assert_eq!(out.m0_unit, "Jy/beam km/s");
        assert_eq!(out.velocity_unit, "km/s");
        assert_eq!(out.n_channels, 25);
        assert_eq!(out.dimensions, [LINE_CUBE_SIZE, LINE_CUBE_SIZE]);
        assert!(out.m0.fits_path.ends_with("line_m0_8-32.fits"));
        for (files, unit, abproc) in [(&out.m0, "Jy/beam km/s", "moment0"), (&out.m1, "km/s", "moment1"), (&out.m2, "km/s", "moment2")] {
            assert!(std::path::Path::new(&files.png_path).exists());
            let reopened = reopen(&files.fits_path);
            assert_eq!(reopened.header.get_i64("NAXIS"), Some(2));
            assert!(reopened.header.get("CTYPE3").is_none());
            assert_eq!(reopened.header.get("BUNIT"), Some(unit));
            assert_eq!(reopened.header.get(HEADER_ABPROC), Some(abproc));
            assert_eq!(reopened.image.dim(), (LINE_CUBE_SIZE, LINE_CUBE_SIZE));
            assert!(WcsTransform::from_header(&reopened.header).is_ok());
        }
        let m1 = reopen(&out.m1.fits_path).image;
        let centre = (LINE_DISK_CENTRE as usize, LINE_DISK_CENTRE as usize);
        assert!(m1[centre].is_finite());
        assert!(m1[[0, 0]].is_nan());
    }

    #[test]
    fn a_moment_zero_map_of_an_mjy_per_sr_cube_is_not_calibrated_as_a_flux_density() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mjy_line.fits");
        write_line_cube_with_cards(&path, &[("BUNIT", "'MJy/sr'"), ("PIXAR_SR", "1.0E-13"), ("SATURATE", "5000.0")]);
        let key = path.to_str().unwrap();
        let set = alone(LazyCube::open(key).unwrap());
        let out = dir.path().join("maps");
        std::fs::create_dir_all(&out).unwrap();
        let maps = moment_map_files(&set, key, out.to_str().unwrap(), &line_moment_config(), Instant::now()).unwrap();
        assert_eq!(maps.m0_unit, "MJy/sr km/s");
        let header = reopen(&maps.m0.fits_path).header;
        assert_eq!(header.get("BUNIT"), Some("MJy/sr km/s"));
        assert!(header.get("PIXAR_SR").is_none());
        assert!(header.get("SATURATE").is_none());
        let wcs = WcsTransform::from_header(&header).ok();
        assert!(PhotCal::from_header(&header, wcs.as_ref()).is_none(), "M0 was calibrated as MJy/sr");
    }

    #[test]
    fn every_cube_command_accepts_the_plane_ref_the_frontend_sends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("refcube.fits");
        write_line_cube(&path, 0.01);
        let key = format!("{}#hdu=0", path.to_str().unwrap());
        let out = dir.path().join("out");
        let out = out.to_str().unwrap();

        let info = cube_info_json(&key).unwrap();
        assert_eq!(info[RES_FRAMES], LINE_CUBE_DEPTH);
        assert_eq!(info[RES_WIDTH], LINE_CUBE_SIZE);
        let spectrum = cube_spectrum_json(&key, 16, 16).unwrap();
        assert_eq!(spectrum[RES_SPECTRUM].as_array().unwrap().len(), LINE_CUBE_DEPTH);
        let region = region_spectrum_json(&key, &disk(), None, None).unwrap();
        assert_eq!(region["sum"].as_array().unwrap().len(), LINE_CUBE_DEPTH);
        std::fs::create_dir_all(out).unwrap();
        let png = format!("{}/frame.png", out);
        let frame = cube_frame_json(&key, 3, &png, None).unwrap();
        assert_eq!(frame[RES_OUTPUT_PATH], png);
        let collapsed = collapse_range_json(&key, out, 15, 25, "mean").unwrap();
        assert!(collapsed["fits_path"].as_str().unwrap().ends_with("refcube_hdu0_collapse_mean_15-25.fits"));
        let moments = moment_maps_json(&key, out, &line_moment_config()).unwrap();
        assert!(moments["m0"]["fits_path"].as_str().unwrap().contains("refcube_hdu0_m0_8-32"));
        GLOBAL_CUBE_CACHE.invalidate(&key);
    }

    #[test]
    fn collapse_previews_use_the_auto_stf_transfer_of_the_processing_previews() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stf_cube.fits");
        write_line_cube(&path, 0.05);
        let key = path.to_str().unwrap();
        let out = dir.path().join("out");
        let out = out.to_str().unwrap();
        let last = LINE_CUBE_DEPTH - 1;
        let cube = LazyCube::open(key).unwrap();
        for mode in [CollapseMode::Mean, CollapseMode::Median] {
            let collapsed = cube.collapse_range(0, last, mode).unwrap();
            let response = collapse_range_json(key, out, 0, last, mode.name()).unwrap();
            let png = response["png_path"].as_str().unwrap();
            let written = image::open(png).unwrap().to_luma8();
            assert_eq!(written.dimensions(), (LINE_CUBE_SIZE as u32, LINE_CUBE_SIZE as u32));
            assert_eq!(
                written.as_raw(),
                &crate::cmd::common::auto_stretch_preview(&collapsed),
                "{} is not rendered with the auto-STF transfer",
                png
            );
        }
        GLOBAL_CUBE_CACHE.invalidate(key);
    }

    #[test]
    fn an_exported_frame_keeps_the_celestial_wcs_unit_and_its_channel_coordinate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frames.fits");
        write_line_cube(&path, 0.0);
        let key = path.to_str().unwrap();
        let png = dir.path().join("frame_0003.png");
        let fits = dir.path().join("frame_0003.fits");
        let result = cube_frame_json(key, 3, png.to_str().unwrap(), Some(fits.to_str().unwrap())).unwrap();
        assert_eq!(result[RES_FITS_PATH], fits.to_str().unwrap());

        let reopened = reopen(fits.to_str().unwrap());
        let header = &reopened.header;
        assert_eq!(header.get_i64("NAXIS"), Some(2));
        assert!(header.get("NAXIS3").is_none() && header.get("CTYPE3").is_none());
        assert_eq!(header.get("CTYPE1"), Some("RA---TAN"));
        assert_eq!(header.get("BUNIT"), Some("Jy/beam"));
        assert_eq!(header.get_i64(CARD_CHANNEL), Some(3));
        assert!((header.get_f64(CARD_SPECTRAL_VALUE).unwrap() - 1.003).abs() < 1e-9);
        assert_eq!(header.get(CARD_SPECTRAL_UNIT), Some("um"));
        assert_eq!(header.get(CARD_SPECTRAL_TYPE), Some("WAVE"));
        assert_eq!(header.get(HEADER_ABPROC), Some(ABPROC_CUBE_FRAME));
        let original = WcsTransform::from_header(&LazyCube::open(key).unwrap().header).unwrap();
        let written = WcsTransform::from_header(header).unwrap();
        let (a, b) = (original.pixel_to_world(5.0, 7.0), written.pixel_to_world(5.0, 7.0));
        assert!((a.ra - b.ra).abs() < 1e-9 && (a.dec - b.dec).abs() < 1e-9);
        assert_eq!(reopened.image[[0, 0]], LINE_CONTINUUM);
        GLOBAL_CUBE_CACHE.invalidate(key);
    }

    #[test]
    fn a_frame_preview_is_not_crushed_by_one_hot_pixel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hot_frame.fits");
        write_cube(&path, 8, 8, 3, &[], |z, y, x| {
            if z == 2 && y == 0 && x == 0 {
                1.0e6
            } else {
                50.0 + deterministic_noise(0, y, x, 1.0)
            }
        });
        let key = path.to_str().unwrap();
        let png = dir.path().join("hot.png");
        cube_frame_json(key, 2, png.to_str().unwrap(), None).unwrap();
        let pixels = image::open(&png).unwrap().to_luma8();
        let sky = &pixels.as_raw()[1..];
        let lit = sky.iter().filter(|&&v| v > 0).count();
        assert!(lit > sky.len() / 2, "sky rendered black next to a hot pixel: {:?}", sky);
        GLOBAL_CUBE_CACHE.invalidate(key);
    }

    #[test]
    fn cube_info_and_point_spectrum_label_their_axes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("muse.fits");
        let cards = [("CTYPE3", "'AWAV'"), ("CUNIT3", "'Angstrom'"), ("CRVAL3", "4750.0"), ("CD3_3", "1.25"), ("CRPIX3", "1.0")];
        write_cube(&path, 4, 4, 3, &cards, |z, _, _| 1.0 + z as f32);
        let key = path.to_str().unwrap();
        let spectrum = cube_spectrum_json(key, 1, 1).unwrap();
        assert_eq!(spectrum[RES_WAVELENGTHS][1], 4751.25);
        assert_eq!(spectrum[RES_UNIT], "Angstrom");
        let info = cube_info_json(key).unwrap();
        assert_eq!(info[RES_SPECTRAL_CLASSIFICATION][KEY_AXIS_UNIT_ASSUMED], false);
        assert_eq!(info[KEY_SPECTRAL_AXIS]["unit"], "um");

        let bare = dir.path().join("bare_wave.fits");
        write_cube(&bare, 4, 4, 3, &[("CTYPE3", "'WAVE'"), ("CRVAL3", "4.7E-7"), ("CDELT3", "1.0E-10")], |_, _, _| 1.0);
        let info = cube_info_json(bare.to_str().unwrap()).unwrap();
        assert_eq!(info[RES_SPECTRAL_CLASSIFICATION][KEY_AXIS_UNIT_ASSUMED], true);
        assert_eq!(info[RES_SPECTRAL_CLASSIFICATION][RES_AXIS_UNIT], "M");
        GLOBAL_CUBE_CACHE.invalidate(key);
        GLOBAL_CUBE_CACHE.invalidate(bare.to_str().unwrap());
    }

    #[test]
    fn point_spectrum_reports_flux_in_jy_only_for_mjy_per_sr_cubes() {
        let dir = tempfile::tempdir().unwrap();
        let (key, _) = open_cube(&dir, "line.fits", 0.0);
        let plain = cube_spectrum_json(&key, 16, 16).unwrap();
        assert!(plain[KEY_FLUX_JY].is_null(), "Jy reported for a Jy/beam cube: {}", plain[KEY_FLUX_JY]);

        let mjy_path = dir.path().join("mjy.fits");
        write_line_cube_with_cards(&mjy_path, &[("BUNIT", "'MJy/sr'"), ("PIXAR_SR", "3.0461742E-12")]);
        let mjy_key = mjy_path.to_str().unwrap();
        let response = cube_spectrum_json(mjy_key, 16, 16).unwrap();
        let spectrum = response[RES_SPECTRUM].as_array().unwrap();
        let flux = response[KEY_FLUX_JY].as_array().expect("flux in Jy");
        assert_eq!(flux.len(), LINE_CUBE_DEPTH);
        for z in 0..LINE_CUBE_DEPTH {
            let expected = spectrum[z].as_f64().unwrap() * 3.0461742e-12 * JY_PER_MJY;
            let got = flux[z].as_f64().unwrap();
            assert!((got - expected).abs() <= 1e-12 * expected.abs().max(1.0), "z={} got={} expected={}", z, got, expected);
        }
        let pinned = 2.0 * 3.0461742e-12 * JY_PER_MJY;
        let got = flux[20].as_f64().unwrap();
        assert!((got - pinned).abs() <= 1e-12 * pinned, "channel 20: got={} expected={}", got, pinned);
        GLOBAL_CUBE_CACHE.invalidate(&key);
        GLOBAL_CUBE_CACHE.invalidate(mjy_key);
    }

    fn write_ramp_fixture(dir: &tempfile::TempDir, name: &str) -> String {
        let path = dir.path().join(name);
        let primary = [
            ("TELESCOP", "'JWST'"),
            ("NGROUPS", "3"),
            ("NINTS", "2"),
            ("TGROUP", "10.0"),
            ("READPATT", "'NRSIRS2RAPID'"),
            ("DATAMODL", "'Level1bModel'"),
        ];
        write_u16_ramp_mef(&path, 4, 3, 3, 2, &primary, &[("BUNIT", "'DN'")], |i, g, y, x| (30000 + i * 1000 + g * 100 + y * 10 + x) as u16);
        path.to_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn cube_info_json_carries_the_ramp_block_and_the_flattened_frame_count() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_ramp_fixture(&dir, "ramp_info.fits");
        let info = get_cube_info(key.clone()).await.unwrap();
        assert_eq!(info[RES_FRAMES], 6);
        assert_eq!(info[RES_WIDTH], 4);
        assert_eq!(info[RES_HEIGHT], 3);
        assert_eq!(info[RES_RAMP]["ngroups"], 3);
        assert_eq!(info[RES_RAMP]["nints"], 2);
        assert_eq!(info[RES_RAMP]["tgroup_s"], 10.0);
        assert_eq!(info[RES_RAMP]["kind"], "jwst_groups");
        assert_eq!(info[RES_RAMP]["group_times_s"], serde_json::json!([0.0, 10.0, 20.0]));
        assert_eq!(info[RES_SPECTRAL_CLASSIFICATION][RES_IS_SPECTRAL], false);
        let reason = info[RES_SPECTRAL_CLASSIFICATION][RES_SPECTRAL_REASON].as_str().unwrap();
        assert!(reason.starts_with("ramp:"), "{}", reason);
        assert_eq!(info[RES_SPECTRAL_CLASSIFICATION][RES_CHANNEL_COUNT], 3);
        assert!(info[RES_WAVELENGTHS].is_null());
        GLOBAL_CUBE_CACHE.invalidate(&key);

        let plain = dir.path().join("plain.fits");
        write_line_cube(&plain, 0.0);
        let plain_key = plain.to_str().unwrap().to_string();
        let info = get_cube_info(plain_key.clone()).await.unwrap();
        assert!(info[RES_RAMP].is_null(), "{}", info[RES_RAMP]);
        assert_eq!(info[RES_FRAMES], LINE_CUBE_DEPTH);
        assert_eq!(info[RES_SPECTRAL_CLASSIFICATION][RES_IS_SPECTRAL], true);
        GLOBAL_CUBE_CACHE.invalidate(&plain_key);
    }

    #[test]
    fn a_ramp_frame_header_names_group_and_integration() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_ramp_fixture(&dir, "ramp_frame.fits");
        let cube = LazyCube::open(&key).unwrap();
        let header = frame_header(&cube, 4);
        assert_eq!(header.get_i64("ABGROUP"), Some(1));
        assert_eq!(header.get_i64("ABINTEG"), Some(1));
        assert_eq!(header.get_i64(CARD_CHANNEL), Some(4));
        assert_eq!(header.get("TELESCOP"), None, "the primary header must not leak into a derived frame");
        assert_eq!(header.get("BUNIT"), Some("DN"));
        assert_eq!(header.get(HEADER_ABPROC), Some(ABPROC_CUBE_FRAME));
        assert!(header.get("NAXIS4").is_none() && header.get("NAXIS3").is_none(), "{:?}", header.cards);
        let first = frame_header(&cube, 0);
        assert_eq!((first.get_i64("ABGROUP"), first.get_i64("ABINTEG")), (Some(0), Some(0)));

        let png = dir.path().join("ramp_frame.png");
        let fits = dir.path().join("ramp_frame_4.fits");
        cube_frame_json(&key, 4, png.to_str().unwrap(), Some(fits.to_str().unwrap())).unwrap();
        let reopened = reopen(fits.to_str().unwrap());
        assert_eq!(reopened.header.get_i64("ABGROUP"), Some(1));
        assert_eq!(reopened.header.get_i64("ABINTEG"), Some(1));
        assert_eq!(reopened.image[[1, 2]], 31112.0);

        let plain = dir.path().join("plain_frame.fits");
        write_line_cube(&plain, 0.0);
        let plain_cube = LazyCube::open(plain.to_str().unwrap()).unwrap();
        let header = frame_header(&plain_cube, 3);
        assert!(header.get("ABGROUP").is_none() && header.get("ABINTEG").is_none());
        GLOBAL_CUBE_CACHE.invalidate(&key);
    }

    #[test]
    fn linear_preview_scales_percentiles_and_blacks_out_nan() {
        let arr = Array2::from_shape_vec((2, 3), vec![f32::NAN, 0.0, 1.0, 2.0, 3.0, 4.0]).unwrap();
        let out = linear_preview(&arr);
        assert_eq!(out[[0, 0]], 0.0);
        assert_eq!(out[[0, 1]], 0.0);
        assert!((out[[1, 2]] - 1.0).abs() < 1e-6);
        assert!(out[[0, 2]] > 0.0 && out[[0, 2]] < out[[1, 0]] && out[[1, 0]] < out[[1, 1]]);
        let flat = Array2::from_elem((2, 2), 5.0f32);
        assert!(linear_preview(&flat).iter().all(|v| v.is_finite()));
        let empty = Array2::from_elem((2, 2), f32::NAN);
        assert!(linear_preview(&empty).iter().all(|v| *v == 0.0));
    }
}
