use std::time::Instant;

use anyhow::{anyhow, bail};
use serde::Deserialize;
use serde_json::{json, Value};

use ndarray::Array2;

use crate::cmd::analysis::{resolve_dq_mask, DqMask, HEADER_PROCESSING_PROVENANCE, RES_PHOTCAL};
use crate::cmd::catalog::uncalibrated_reason;
use crate::cmd::common::{blocking_cmd, load_cached_full, load_companions};
use crate::core::astrometry::wcs::{position_angle_deg, WcsTransform};
use crate::core::imaging::region::{
    ellipse_geometry, elliptical_profile, encircled_radius_from_bins, line_cut, major_axis_angle_deg,
    petrosian_radius, radial_profile, region_data_stats, EllipticalBin, EllipticalProfile, PhysicalMap,
    RegionShape, RegionStats, RegionSystem, SigmaClip, MAX_SB_BIN_WIDTH, MIN_SB_BIN_WIDTH, PETROSIAN_ETA,
};
use crate::core::imaging::region_file::{parse_reg_with_physical, write_reg_with_physical, Region};
use crate::core::metadata::photcal::{missing_calibration_reason, FluxConvention, PhotCal, AB_MAG_ZERO_POINT};
use crate::infra::cache::ImageEntry;
use crate::types::constants::{
    RES_BINS, RES_CALIBRATED, RES_CALIBRATION_WARNINGS, RES_DEC, RES_DQ_EXCLUDED, RES_ELAPSED_MS, RES_ERROR,
    RES_HAS_WCS, RES_ID, RES_LABEL, RES_MAG_AB_CUMULATIVE, RES_MASKED, RES_MU_AB, RES_MU_ERR, RES_NOTES,
    RES_PETROSIAN_RADIUS_PX, RES_PIXEL_AREA_ARCSEC2, RES_PIXEL_SCALE_ARCSEC, RES_R50_PX, RES_R80_PX, RES_R90_PX,
    RES_RA, RES_REGIONS, RES_REG_TEXT, RES_SKY, RES_SKY_PA_DEG, RES_SMA_ARCSEC, RES_STATS, RES_SYSTEM,
    RES_TOTAL_MAG_AB, RES_WARNINGS,
};
use crate::types::header::HduHeader;

const MAX_REGIONS_PER_CALL: usize = 512;
const MAX_REG_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_SIGMA_CLIP_ITERS: usize = 100;
const PIXEL_SCALE_ANISOTROPY_TOLERANCE: f64 = 0.01;
const ARCSEC_PER_DEGREE: f64 = 3600.0;
const POSITION_ANGLE_PROBE_PX: f64 = 1.0;
const POSITION_ANGLE_PERIOD_DEG: f64 = 180.0;
const FLUX_SOURCE_NET: &str = "net";
const FLUX_SOURCE_SUM: &str = "sum";
const RES_FLUX_SOURCE: &str = "flux_source";
const RES_FLUX_NATIVE: &str = "flux_native";
const RES_FLUX_ERR_NATIVE: &str = "flux_err_native";
const RES_FLUX_JY: &str = "flux_jy";
const RES_FLUX_ERR_JY: &str = "flux_err_jy";
const RES_MAG_AB: &str = "mag_ab";
const RES_MAG_AB_ERR: &str = "mag_ab_err";
const RES_ST_MAG: &str = "st_mag";
const RES_AREA_ARCSEC2: &str = "area_arcsec2";
const RES_GEOMETRIC_AREA_ARCSEC2: &str = "geometric_area_arcsec2";
const RES_SB_MAG_ARCSEC2: &str = "sb_mag_arcsec2";
const RES_PA_SKY_DEG: &str = "pa_sky_deg";
const DEFAULT_SB_BIN_WIDTH: f64 = 1.0;
const SB_COARSE_BIN_NOTE: &str = "integer-lattice membership: bins below 3 px are biased";
const SB_NO_BACKGROUND_NOTE: &str = "no background subtracted";
const SB_AXES_SWAPPED_NOTE: &str =
    "ellipse normalised: ry exceeded rx, so the major axis is ry and the position angle was rotated by 90 deg";

#[derive(Deserialize)]
pub struct RegionStatsRequest {
    pub id: String,
    pub shape: RegionShape,
    #[serde(default)]
    pub background: Option<RegionShape>,
}

fn entry_wcs(entry: &ImageEntry) -> Option<WcsTransform> {
    entry.header().and_then(|h| WcsTransform::from_header(h).ok())
}

fn entry_physical(entry: &ImageEntry) -> PhysicalMap {
    entry.header().map(PhysicalMap::from_header).unwrap_or_else(PhysicalMap::identity)
}

fn sigma_clip_params(sigma: Option<f32>, maxiters: Option<usize>) -> anyhow::Result<SigmaClip> {
    let defaults = SigmaClip::default();
    let sigma = sigma.unwrap_or(defaults.sigma);
    let maxiters = maxiters.unwrap_or(defaults.maxiters);
    if !(sigma.is_finite() && sigma > 0.0) {
        bail!("sigma must be a finite number greater than 0, got {}", sigma);
    }
    if !(1..=MAX_SIGMA_CLIP_ITERS).contains(&maxiters) {
        bail!("maxiters must be between 1 and {}, got {}", MAX_SIGMA_CLIP_ITERS, maxiters);
    }
    Ok(SigmaClip { sigma, maxiters })
}

pub(crate) fn companion_err(path: &str, dims: (usize, usize)) -> Option<ImageEntry> {
    match load_companions(path) {
        Ok(comps) => comps.err.filter(|e| e.arr().dim() == dims),
        Err(e) => {
            log::warn!("ERR companion unavailable for {}: {:#}", path, e);
            None
        }
    }
}

pub(crate) struct EntryCalibration {
    pub photcal: Option<PhotCal>,
    pub wcs: Option<WcsTransform>,
    pub pixel_area_arcsec2: Option<f64>,
    pub warnings: Vec<String>,
}

impl EntryCalibration {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn none() -> Self {
        Self { photcal: None, wcs: None, pixel_area_arcsec2: None, warnings: Vec::new() }
    }
}

fn header_card_text(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|v| v.trim().trim_matches('\'').trim().to_string())
        .filter(|v| !v.is_empty())
}

fn axis_pixel_scales_arcsec(wcs: &WcsTransform) -> (f64, f64) {
    let cd = wcs.raw_params().4;
    let scale_x = (cd[0][0].powi(2) + cd[1][0].powi(2)).sqrt() * ARCSEC_PER_DEGREE;
    let scale_y = (cd[0][1].powi(2) + cd[1][1].powi(2)).sqrt() * ARCSEC_PER_DEGREE;
    (scale_x, scale_y)
}

fn anisotropy_warning(wcs: &WcsTransform) -> Option<String> {
    let (scale_x, scale_y) = axis_pixel_scales_arcsec(wcs);
    let larger = scale_x.max(scale_y);
    if !(larger.is_finite() && larger > 0.0) {
        return None;
    }
    ((scale_x - scale_y).abs() / larger > PIXEL_SCALE_ANISOTROPY_TOLERANCE).then(|| {
        format!(
            "pixel scales differ per axis ({scale_x:.5} x {scale_y:.5} arcsec); the pixel area uses their mean"
        )
    })
}

fn positive_finite(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

pub(crate) fn entry_calibration(entry: &ImageEntry) -> EntryCalibration {
    let wcs = entry_wcs(entry);
    let header = entry.header();
    let mut warnings = Vec::new();
    let photcal = header.and_then(|h| {
        let cal = PhotCal::from_header(h, wcs.as_ref())?;
        match uncalibrated_reason(h) {
            Some(reason) => {
                warnings.push(reason);
                None
            }
            None => {
                warnings.extend(cal.jansky_unavailable_reason());
                warnings.extend(cal.warnings.iter().cloned());
                cal.converts_to_jansky().then_some(cal)
            }
        }
    });
    if photcal.is_none() && warnings.is_empty() {
        warnings.push(missing_calibration_reason(header));
    }
    if let Some(provenance) = header.and_then(|h| header_card_text(h, HEADER_PROCESSING_PROVENANCE)) {
        warnings.push(format!("photometry on processed data ({provenance})"));
    }
    let pixel_area_arcsec2 = match &photcal {
        Some(PhotCal { convention: FluxConvention::JwstMjySr { pixar_a2: Some(a2), .. }, .. }) => positive_finite(*a2),
        _ => wcs.as_ref().and_then(|w| {
            warnings.extend(anisotropy_warning(w));
            positive_finite(w.pixel_scale_arcsec().powi(2))
        }),
    };
    EntryCalibration { photcal, wcs, pixel_area_arcsec2, warnings }
}

fn sky_centre(shape: &RegionShape, wcs: Option<&WcsTransform>) -> (Option<f64>, Option<f64>) {
    let Some(wcs) = wcs else { return (None, None) };
    let (x, y) = shape.centre();
    let coord = wcs.pixel_to_world(x, y);
    if coord.ra.is_finite() && coord.dec.is_finite() {
        (Some(coord.ra), Some(coord.dec))
    } else {
        (None, None)
    }
}

fn sky_position_angle_deg(shape: &RegionShape, wcs: Option<&WcsTransform>) -> Option<f64> {
    let wcs = wcs?;
    let (sin, cos) = major_axis_angle_deg(shape)?.to_radians().sin_cos();
    let (x, y) = shape.centre();
    let from = wcs.pixel_to_world(x, y);
    let to = wcs.pixel_to_world(x + POSITION_ANGLE_PROBE_PX * cos, y + POSITION_ANGLE_PROBE_PX * sin);
    if ![from.ra, from.dec, to.ra, to.dec].iter().all(|v| v.is_finite()) {
        return None;
    }
    let pa = position_angle_deg(from.ra, from.dec, to.ra, to.dec).rem_euclid(POSITION_ANGLE_PERIOD_DEG);
    if !pa.is_finite() {
        return None;
    }
    Some(if pa >= POSITION_ANGLE_PERIOD_DEG { 0.0 } else { pa })
}

fn surface_brightness_mag_arcsec2(flux_jy: f64, area_arcsec2: Option<f64>) -> Option<f64> {
    let area = area_arcsec2.filter(|a| *a > 0.0)?;
    (flux_jy > 0.0).then(|| -2.5 * (flux_jy / area).log10() + AB_MAG_ZERO_POINT)
}

struct RegionSky {
    ra: Option<f64>,
    dec: Option<f64>,
    pa_sky_deg: Option<f64>,
    area_arcsec2: Option<f64>,
    geometric_area_arcsec2: Option<f64>,
}

fn region_sky(stats: &RegionStats, shape: &RegionShape, cal: &EntryCalibration) -> RegionSky {
    let (ra, dec) = sky_centre(shape, cal.wcs.as_ref());
    RegionSky {
        ra,
        dec,
        pa_sky_deg: sky_position_angle_deg(shape, cal.wcs.as_ref()),
        area_arcsec2: cal.pixel_area_arcsec2.map(|a| stats.count as f64 * a),
        geometric_area_arcsec2: cal.pixel_area_arcsec2.map(|a| stats.area * a),
    }
}

fn region_sky_json(stats: &RegionStats, shape: &RegionShape, cal: &EntryCalibration) -> Value {
    if cal.wcs.is_none() && cal.pixel_area_arcsec2.is_none() {
        return Value::Null;
    }
    let sky = region_sky(stats, shape, cal);
    json!({
        RES_RA: sky.ra,
        RES_DEC: sky.dec,
        RES_PA_SKY_DEG: sky.pa_sky_deg,
        RES_AREA_ARCSEC2: sky.area_arcsec2,
        RES_GEOMETRIC_AREA_ARCSEC2: sky.geometric_area_arcsec2,
    })
}

pub(crate) fn calibrated_flux_json(stats: &RegionStats, shape: &RegionShape, cal: &EntryCalibration) -> Value {
    let Some(photcal) = &cal.photcal else { return Value::Null };
    let flux_native = stats.net_sum.unwrap_or(stats.sum);
    let Some(c) = photcal.calibrate(flux_native, stats.sum_err) else { return Value::Null };
    let sky = region_sky(stats, shape, cal);
    json!({
        RES_FLUX_SOURCE: if stats.net_sum.is_some() { FLUX_SOURCE_NET } else { FLUX_SOURCE_SUM },
        RES_FLUX_NATIVE: flux_native,
        RES_FLUX_ERR_NATIVE: stats.sum_err,
        RES_FLUX_JY: c.flux_jy,
        RES_FLUX_ERR_JY: c.flux_err_jy,
        RES_MAG_AB: c.mag_ab,
        RES_MAG_AB_ERR: c.mag_ab_err,
        RES_ST_MAG: c.st_mag,
        RES_AREA_ARCSEC2: sky.area_arcsec2,
        RES_GEOMETRIC_AREA_ARCSEC2: sky.geometric_area_arcsec2,
        RES_SB_MAG_ARCSEC2: surface_brightness_mag_arcsec2(c.flux_jy, sky.area_arcsec2),
        RES_RA: sky.ra,
        RES_DEC: sky.dec,
        RES_PA_SKY_DEG: sky.pa_sky_deg,
    })
}

fn photcal_json(cal: &PhotCal) -> anyhow::Result<Value> {
    let mut val = serde_json::to_value(cal)?;
    if let Some(obj) = val.as_object_mut() {
        obj.insert(RES_LABEL.to_string(), json!(cal.label()));
    }
    Ok(val)
}

fn with_timing(mut body: Value, masked: bool, t0: Instant) -> Value {
    if let Some(obj) = body.as_object_mut() {
        obj.insert(RES_MASKED.to_string(), json!(masked));
        obj.insert(RES_ELAPSED_MS.to_string(), json!(t0.elapsed().as_millis() as u64));
    }
    body
}

pub(crate) fn stats_for_entry(
    entry: &ImageEntry,
    regions: &[RegionStatsRequest],
    mask: Option<&DqMask>,
    err: Option<&Array2<f32>>,
    clip: SigmaClip,
    cal: &EntryCalibration,
) -> Vec<Value> {
    let excluded = mask.map(|m| &m.map);
    regions
        .iter()
        .map(|req| {
            let stats = region_data_stats(entry.arr(), &req.shape, req.background.as_ref(), excluded, err, clip)
                .map_err(|e| e.to_string())
                .and_then(|stats| {
                    let mut value = serde_json::to_value(&stats).map_err(|e| e.to_string())?;
                    if let Some(obj) = value.as_object_mut() {
                        obj.insert(RES_CALIBRATED.to_string(), calibrated_flux_json(&stats, &req.shape, cal));
                        obj.insert(RES_SKY.to_string(), region_sky_json(&stats, &req.shape, cal));
                    }
                    Ok(value)
                });
            match stats {
                Ok(value) => json!({ RES_ID: req.id, RES_STATS: value, RES_ERROR: Value::Null }),
                Err(e) => json!({ RES_ID: req.id, RES_STATS: Value::Null, RES_ERROR: e }),
            }
        })
        .collect()
}

pub(crate) fn import_for_entry(entry: &ImageEntry, reg_text: &str) -> anyhow::Result<Value> {
    let wcs = entry_wcs(entry);
    let parsed = parse_reg_with_physical(reg_text, wcs.as_ref(), &entry_physical(entry))?;
    Ok(json!({
        RES_REGIONS: parsed.regions,
        RES_WARNINGS: parsed.warnings,
        RES_HAS_WCS: wcs.is_some(),
    }))
}

pub(crate) fn export_for_entry(
    entry: &ImageEntry,
    regions: &[Region],
    system: RegionSystem,
    sexagesimal: bool,
) -> anyhow::Result<String> {
    let wcs = entry_wcs(entry);
    Ok(write_reg_with_physical(regions, system, wcs.as_ref(), sexagesimal, &entry_physical(entry))?)
}

#[tauri::command]
pub async fn region_stats_cmd(
    path: String,
    regions: Vec<RegionStatsRequest>,
    exclude_dq: Option<bool>,
    sigma: Option<f32>,
    maxiters: Option<usize>,
) -> Result<Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        if regions.len() > MAX_REGIONS_PER_CALL {
            bail!("too many regions in one call ({}); the limit is {}", regions.len(), MAX_REGIONS_PER_CALL);
        }
        let clip = sigma_clip_params(sigma, maxiters)?;
        let entry = load_cached_full(&path)?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), entry.arr().dim());
        let err_entry = companion_err(&path, entry.arr().dim());
        let cal = entry_calibration(&entry);
        let entries =
            stats_for_entry(&entry, &regions, mask.as_ref(), err_entry.as_ref().map(|e| e.arr()), clip, &cal);
        let photcal = match &cal.photcal {
            Some(photcal) => photcal_json(photcal)?,
            None => Value::Null,
        };
        Ok(json!({
            RES_REGIONS: entries,
            RES_MASKED: mask.is_some(),
            RES_DQ_EXCLUDED: mask.as_ref().map(|m| m.excluded),
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_PHOTCAL: photcal,
            RES_CALIBRATION_WARNINGS: cal.warnings,
            RES_PIXEL_AREA_ARCSEC2: cal.pixel_area_arcsec2,
        }))
    })
}

#[tauri::command]
pub async fn radial_profile_cmd(
    path: String,
    x: f64,
    y: f64,
    max_radius: f64,
    background: Option<[f64; 2]>,
    exclude_dq: Option<bool>,
) -> Result<Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let entry = load_cached_full(&path)?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), entry.arr().dim());
        let profile = radial_profile(
            entry.arr(),
            x,
            y,
            max_radius,
            background.map(|b| (b[0], b[1])),
            mask.as_ref().map(|m| &m.map),
        )?;
        Ok(with_timing(serde_json::to_value(profile)?, mask.is_some(), t0))
    })
}

#[tauri::command]
pub async fn line_cut_cmd(
    path: String,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    exclude_dq: Option<bool>,
) -> Result<Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let entry = load_cached_full(&path)?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), entry.arr().dim());
        let cut = line_cut(entry.arr(), x1, y1, x2, y2, mask.as_ref().map(|m| &m.map))?;
        Ok(with_timing(serde_json::to_value(cut)?, mask.is_some(), t0))
    })
}

fn sb_bin_surface_brightness(bin: &EllipticalBin, photcal: Option<&PhotCal>, pixel_area: Option<f64>) -> (Option<f64>, Option<f64>) {
    let (Some(photcal), Some(mean)) = (photcal, bin.mean) else { return (None, None) };
    let mean_err = bin.std.map(|std| std / (bin.count as f64).sqrt());
    let Some(c) = photcal.calibrate(mean, mean_err) else { return (None, None) };
    let mu_ab = surface_brightness_mag_arcsec2(c.flux_jy, pixel_area);
    (mu_ab, mu_ab.and(c.mag_ab_err))
}

fn sb_bin_json(bin: &EllipticalBin, cal: &EntryCalibration, pixel_scale: Option<f64>) -> anyhow::Result<Value> {
    let mut value = serde_json::to_value(bin)?;
    let obj = value.as_object_mut().ok_or_else(|| anyhow!("profile bin did not serialise to an object"))?;
    let (mu_ab, mu_err) = sb_bin_surface_brightness(bin, cal.photcal.as_ref(), cal.pixel_area_arcsec2);
    let cumulative = cal.photcal.as_ref().and_then(|p| p.calibrate(bin.cumulative_sum, None)).and_then(|c| c.mag_ab);
    obj.insert(RES_SMA_ARCSEC.to_string(), json!(pixel_scale.map(|s| bin.sma * s)));
    obj.insert(RES_MU_AB.to_string(), json!(mu_ab));
    obj.insert(RES_MU_ERR.to_string(), json!(mu_err));
    obj.insert(RES_MAG_AB_CUMULATIVE.to_string(), json!(cumulative));
    Ok(value)
}

fn sb_profile_notes(shape: &RegionShape, background: Option<&RegionShape>) -> Vec<String> {
    let mut notes = vec![SB_COARSE_BIN_NOTE.to_string()];
    notes.push(match background {
        Some(bg) => format!("background from {} region", bg.kind()),
        None => SB_NO_BACKGROUND_NOTE.to_string(),
    });
    if let RegionShape::Ellipse { rx, ry, .. } = shape {
        if rx < ry {
            notes.push(SB_AXES_SWAPPED_NOTE.to_string());
        }
    }
    notes
}

pub(crate) fn sb_profile_json(
    profile: &EllipticalProfile,
    shape: &RegionShape,
    background: Option<&RegionShape>,
    cal: &EntryCalibration,
    masked: bool,
    t0: Instant,
) -> anyhow::Result<Value> {
    let pixel_scale = cal.wcs.as_ref().map(|w| w.pixel_scale_arcsec()).and_then(positive_finite);
    let bins = profile.bins.iter().map(|b| sb_bin_json(b, cal, pixel_scale)).collect::<anyhow::Result<Vec<Value>>>()?;
    let total_mag_ab = profile
        .bins
        .last()
        .and_then(|b| cal.photcal.as_ref().and_then(|p| p.calibrate(b.cumulative_sum, None)))
        .and_then(|c| c.mag_ab);
    let photcal = match &cal.photcal {
        Some(photcal) => photcal_json(photcal)?,
        None => Value::Null,
    };
    let mut body = serde_json::to_value(profile)?;
    let obj = body.as_object_mut().ok_or_else(|| anyhow!("profile did not serialise to an object"))?;
    obj.insert(RES_BINS.to_string(), Value::Array(bins));
    obj.insert(RES_R50_PX.to_string(), json!(encircled_radius_from_bins(&profile.bins, 0.5)));
    obj.insert(RES_R80_PX.to_string(), json!(encircled_radius_from_bins(&profile.bins, 0.8)));
    obj.insert(RES_R90_PX.to_string(), json!(encircled_radius_from_bins(&profile.bins, 0.9)));
    obj.insert(RES_PETROSIAN_RADIUS_PX.to_string(), json!(petrosian_radius(&profile.bins, PETROSIAN_ETA)));
    obj.insert(RES_PIXEL_SCALE_ARCSEC.to_string(), json!(pixel_scale));
    obj.insert(RES_PIXEL_AREA_ARCSEC2.to_string(), json!(cal.pixel_area_arcsec2));
    obj.insert(RES_SKY_PA_DEG.to_string(), json!(sky_position_angle_deg(shape, cal.wcs.as_ref())));
    obj.insert(RES_PHOTCAL.to_string(), photcal);
    obj.insert(RES_CALIBRATION_WARNINGS.to_string(), json!(cal.warnings));
    obj.insert(RES_TOTAL_MAG_AB.to_string(), json!(total_mag_ab));
    obj.insert(RES_NOTES.to_string(), json!(sb_profile_notes(shape, background)));
    Ok(with_timing(body, masked, t0))
}

#[tauri::command]
pub async fn sb_profile_cmd(
    path: String,
    shape: RegionShape,
    bin_width: Option<f64>,
    background: Option<RegionShape>,
    exclude_dq: Option<bool>,
) -> Result<Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let bin_width = bin_width.unwrap_or(DEFAULT_SB_BIN_WIDTH);
        if !bin_width.is_finite() || !(MIN_SB_BIN_WIDTH..=MAX_SB_BIN_WIDTH).contains(&bin_width) {
            bail!("bin_width must be between {} and {} px, got {}", MIN_SB_BIN_WIDTH, MAX_SB_BIN_WIDTH, bin_width);
        }
        let (x, y, sma_max, ellipticity, angle_deg) = ellipse_geometry(&shape)?;
        if let Some(bg) = &background {
            bg.validate()?;
        }
        let entry = load_cached_full(&path)?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), entry.arr().dim());
        let profile = elliptical_profile(
            entry.arr(),
            x,
            y,
            sma_max,
            ellipticity,
            angle_deg,
            bin_width,
            background.as_ref(),
            mask.as_ref().map(|m| &m.map),
        )?;
        let cal = entry_calibration(&entry);
        sb_profile_json(&profile, &shape, background.as_ref(), &cal, mask.is_some(), t0)
    })
}

#[tauri::command]
pub async fn regions_import_cmd(path: String, reg_text: String) -> Result<Value, String> {
    blocking_cmd!({
        if reg_text.len() > MAX_REG_TEXT_BYTES {
            bail!("region file is too large ({} bytes); the limit is {} bytes", reg_text.len(), MAX_REG_TEXT_BYTES);
        }
        let entry = load_cached_full(&path)?;
        import_for_entry(&entry, &reg_text)
    })
}

#[tauri::command]
pub async fn regions_export_cmd(
    path: String,
    regions: Vec<Region>,
    system: String,
    sexagesimal: Option<bool>,
) -> Result<Value, String> {
    blocking_cmd!({
        let system = RegionSystem::parse(&system).map_err(|e| anyhow!(e))?;
        let entry = load_cached_full(&path)?;
        let text = export_for_entry(&entry, &regions, system, sexagesimal.unwrap_or(true))?;
        Ok(json!({ RES_REG_TEXT: text, RES_SYSTEM: system.name() }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::region::test_support::{header_with_cd, north_up_cd};
    use crate::core::imaging::region_file::RegionProperties;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef;
    use crate::infra::fits::writer::write_fits_mono;
    use ndarray::Array2;

    fn req(id: &str, shape: RegionShape) -> RegionStatsRequest {
        RegionStatsRequest { id: id.into(), shape, background: None }
    }

    #[test]
    fn stats_for_entry_reports_dq_exclusion_and_isolates_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("regions.fits");
        let mut dq = vec![-2147483648i32; 16];
        dq[1] = -2147483647;
        dq[6] = -2147483645;
        dq[9] = -2147483646;
        sci_err_dq_mef(&path, 4, 4, dq);
        let key = format!("{}#hdu=1", path.to_str().unwrap());
        let entry = load_cached_full(&key).unwrap();
        let mask = resolve_dq_mask(&key, true, entry.arr().dim()).expect("mask");
        assert_eq!(mask.excluded, 2);

        let whole = RegionShape::Box { x: 1.5, y: 1.5, width: 4.0, height: 4.0, angle: 0.0 };
        let regions = vec![
            req("all", whole.clone()),
            req("empty", RegionShape::Circle { x: 100.0, y: 100.0, r: 2.0 }),
        ];
        let out = stats_for_entry(&entry, &regions, Some(&mask), None, SigmaClip::default(), &EntryCalibration::none());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0][RES_ID], "all");
        assert!(out[0][RES_ERROR].is_null());
        assert_eq!(out[0][RES_STATS]["n_excluded"], 2);
        assert_eq!(out[0][RES_STATS]["n_padding"], 1);
        assert_eq!(out[0][RES_STATS]["count"], 13);
        assert_eq!(out[1][RES_ID], "empty");
        assert!(out[1][RES_STATS].is_null());
        assert!(out[1][RES_ERROR].as_str().unwrap().contains("no finite pixels"));

        let unmasked = stats_for_entry(&entry, &regions[..1], None, None, SigmaClip::default(), &EntryCalibration::none());
        assert_eq!(unmasked[0][RES_STATS]["n_excluded"], 0);
        assert_eq!(unmasked[0][RES_STATS]["n_padding"], 1);
        assert_eq!(unmasked[0][RES_STATS]["count"], 15);
        assert_eq!(unmasked[0][RES_STATS]["sum"], 120.0);

        let invalid = stats_for_entry(
            &entry,
            &[req("bad", RegionShape::Circle { x: 1.0, y: 1.0, r: -1.0 })],
            None,
            None,
            SigmaClip::default(),
            &EntryCalibration::none(),
        );
        assert!(invalid[0][RES_ERROR].as_str().unwrap().contains("invalid region"));
    }

    #[test]
    fn stats_for_entry_propagates_the_err_companion_of_a_mef() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("err_regions.fits");
        let mut dq = vec![-2147483648i32; 16];
        dq[1] = -2147483647;
        dq[6] = -2147483645;
        sci_err_dq_mef(&path, 4, 4, dq);
        let key = format!("{}#hdu=1", path.to_str().unwrap());
        let entry = load_cached_full(&key).unwrap();
        let err = companion_err(&key, entry.arr().dim()).expect("ERR companion");
        assert_eq!(err.arr()[[0, 2]], 1.0);

        let whole = RegionShape::Box { x: 1.5, y: 1.5, width: 4.0, height: 4.0, angle: 0.0 };
        let regions = vec![req("all", whole)];
        let out = stats_for_entry(&entry, &regions, None, Some(err.arr()), SigmaClip::default(), &EntryCalibration::none());
        let sum_err = out[0][RES_STATS]["sum_err"].as_f64().unwrap();
        let expected: f64 = (0..16).map(|i| (0.5 * i as f64).powi(2)).sum::<f64>().sqrt();
        assert!((sum_err - expected).abs() < 1e-6, "sum_err={sum_err} expected={expected}");
        assert!(out[0][RES_STATS]["weighted_mean"].as_f64().unwrap().is_finite());

        let mask = resolve_dq_mask(&key, true, entry.arr().dim()).expect("mask");
        let masked = stats_for_entry(&entry, &regions, Some(&mask), Some(err.arr()), SigmaClip::default(), &EntryCalibration::none());
        let expected_masked: f64 = (0..16)
            .filter(|i| *i != 1 && *i != 6)
            .map(|i| (0.5 * i as f64).powi(2))
            .sum::<f64>()
            .sqrt();
        let masked_err = masked[0][RES_STATS]["sum_err"].as_f64().unwrap();
        assert!((masked_err - expected_masked).abs() < 1e-6, "sum_err={masked_err} expected={expected_masked}");

        let without = stats_for_entry(&entry, &regions, None, None, SigmaClip::default(), &EntryCalibration::none());
        assert!(without[0][RES_STATS]["sum_err"].is_null());
        assert!(without[0][RES_STATS]["weighted_mean"].is_null());

        assert!(companion_err(&key, (8, 8)).is_none());
        assert!(companion_err(&format!("{}#hdu=4", path.to_str().unwrap()), (4, 4)).is_none());
    }

    #[test]
    fn region_stats_agree_with_the_statistics_region_mode_on_a_zero_padded_frame() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drz_edge.fits");
        let arr = Array2::from_shape_fn((12, 12), |(y, x)| if x < 5 { 0.0 } else { ((x + 3 * y) % 7) as f32 - 3.5 });
        write_fits_mono(path.to_str().unwrap(), &arr, None).unwrap();
        let entry = load_cached_full(path.to_str().unwrap()).unwrap();
        let shape = RegionShape::Box { x: 5.5, y: 5.5, width: 12.0, height: 12.0, angle: 0.0 };

        let out = stats_for_entry(&entry, &[req("frame", shape.clone())], None, None, SigmaClip::default(), &EntryCalibration::none());
        let region = &out[0][RES_STATS];
        let statistics = crate::core::imaging::statistics::statistics_for_region(entry.arr(), &shape, None).unwrap();
        assert_eq!(region["count"], statistics.count);
        assert_eq!(region["n_padding"], statistics.padding);
        assert_eq!(region["count"], 84);
        assert_eq!(region["median"].as_f64().unwrap(), statistics.median);
        assert_eq!(region["min"].as_f64().unwrap(), statistics.min);
        assert_eq!(region["max"].as_f64().unwrap(), statistics.max);
        assert!((region["mean"].as_f64().unwrap() - statistics.mean).abs() < 1e-12);
        assert_eq!(statistics.min, -3.5);
    }

    #[test]
    fn export_then_import_round_trips_in_image_and_fk5() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wcs_regions.fits");
        let arr = Array2::<f32>::from_elem((100, 100), 3.0);
        let header = header_with_cd(north_up_cd());
        write_fits_mono(path.to_str().unwrap(), &arr, Some(&header)).unwrap();
        let key = path.to_str().unwrap().to_string();
        let entry = load_cached_full(&key).unwrap();
        assert!(entry_wcs(&entry).is_some(), "header should carry the WCS cards");

        let regions = vec![
            Region {
                shape: RegionShape::Circle { x: 49.5, y: 49.5, r: 3.5 },
                props: RegionProperties { color: Some("red".into()), text: Some("star A".into()), ..Default::default() },
            },
            Region {
                shape: RegionShape::Box { x: 20.0, y: 30.0, width: 10.0, height: 4.0, angle: 25.0 },
                props: RegionProperties { include: false, ..Default::default() },
            },
        ];

        let text = export_for_entry(&entry, &regions, RegionSystem::Image, true).unwrap();
        assert!(text.lines().nth(3).unwrap().starts_with("circle(50.5,50.5,3.5)"));
        let back = import_for_entry(&entry, &text).unwrap();
        assert_eq!(back[RES_HAS_WCS], true);
        assert!(back[RES_WARNINGS].as_array().unwrap().is_empty());
        let shapes: Vec<Region> = serde_json::from_value(back[RES_REGIONS].clone()).unwrap();
        assert_eq!(shapes[0].shape, regions[0].shape);
        assert_eq!(shapes[0].props.color.as_deref(), Some("red"));
        assert_eq!(shapes[0].props.text.as_deref(), Some("star A"));
        assert!(!shapes[1].props.include);
        assert_eq!(shapes[1].shape, regions[1].shape);

        let text = export_for_entry(&entry, &regions, RegionSystem::Fk5, true).unwrap();
        assert_eq!(text.lines().nth(2), Some("fk5"));
        let back = import_for_entry(&entry, &text).unwrap();
        let shapes: Vec<Region> = serde_json::from_value(back[RES_REGIONS].clone()).unwrap();
        let (x, y) = shapes[0].shape.centre();
        assert!((x - 49.5).abs() < 1e-2 && (y - 49.5).abs() < 1e-2, "({x},{y})");
        match &shapes[0].shape {
            RegionShape::Circle { r, .. } => assert!((r - 3.5).abs() < 1e-6),
            other => panic!("{other:?}"),
        }
        match &shapes[1].shape {
            RegionShape::Box { x, y, width, height, angle } => {
                assert!((x - 20.0).abs() < 1e-2 && (y - 30.0).abs() < 1e-2);
                assert!((width - 10.0).abs() < 1e-6 && (height - 4.0).abs() < 1e-6);
                assert!((angle - 25.0).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn import_without_wcs_rejects_sky_regions_and_reports_has_wcs_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.fits");
        let arr = Array2::<f32>::from_elem((8, 8), 1.0);
        write_fits_mono(path.to_str().unwrap(), &arr, None).unwrap();
        let entry = load_cached_full(path.to_str().unwrap()).unwrap();
        let back = import_for_entry(&entry, "image\ncircle(4,4,2)\n").unwrap();
        assert_eq!(back[RES_HAS_WCS], false);
        assert_eq!(back[RES_REGIONS].as_array().unwrap().len(), 1);
        let err = import_for_entry(&entry, "fk5\ncircle(150,2,3\")\n").unwrap_err();
        assert!(err.to_string().contains("requires a WCS"));
        assert!(export_for_entry(&entry, &[], RegionSystem::Icrs, true).is_err());
    }

    #[test]
    fn physical_regions_follow_the_ltv_offset_of_a_cutout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cutout.fits");
        let arr = Array2::<f32>::from_elem((64, 64), 1.0);
        let mut header = crate::types::header::HduHeader::empty();
        header.set_f64("LTV1", -10.0);
        header.set_f64("LTV2", -20.0);
        write_fits_mono(path.to_str().unwrap(), &arr, Some(&header)).unwrap();
        let entry = load_cached_full(path.to_str().unwrap()).unwrap();

        let back = import_for_entry(&entry, "physical\ncircle(31,51,2)\n").unwrap();
        let shapes: Vec<Region> = serde_json::from_value(back[RES_REGIONS].clone()).unwrap();
        assert_eq!(shapes[0].shape, RegionShape::Circle { x: 20.0, y: 30.0, r: 2.0 });

        let text = export_for_entry(&entry, &shapes, RegionSystem::Physical, true).unwrap();
        assert_eq!(text.lines().nth(2), Some("physical"));
        assert!(text.lines().nth(3).unwrap().starts_with("circle(31,51,2)"), "{text}");
        let image = export_for_entry(&entry, &shapes, RegionSystem::Image, true).unwrap();
        assert!(image.lines().nth(3).unwrap().starts_with("circle(21,31,2)"), "{image}");
    }

    #[tokio::test]
    async fn region_stats_cmd_rejects_a_non_positive_sigma_and_an_unbounded_maxiters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.fits");
        let data = Array2::<f32>::from_shape_fn((8, 8), |(y, x)| (y * 8 + x) as f32 + 1.0);
        write_fits_mono(path.to_str().unwrap(), &data, None).unwrap();
        let key = path.to_str().unwrap().to_string();
        let regions = || vec![req("all", RegionShape::Box { x: 3.5, y: 3.5, width: 8.0, height: 8.0, angle: 0.0 })];

        for sigma in [-1.0f32, 0.0, f32::NAN, f32::INFINITY] {
            let err = region_stats_cmd(key.clone(), regions(), None, Some(sigma), None).await.unwrap_err();
            assert!(err.contains("sigma must be a finite number greater than 0"), "{err}");
        }
        for maxiters in [0usize, MAX_SIGMA_CLIP_ITERS + 1] {
            let err = region_stats_cmd(key.clone(), regions(), None, None, Some(maxiters)).await.unwrap_err();
            assert!(err.contains("maxiters must be between 1 and 100"), "{err}");
        }
        let ok = region_stats_cmd(key, regions(), None, Some(2.5), Some(MAX_SIGMA_CLIP_ITERS)).await.unwrap();
        assert!(ok[RES_REGIONS][0][RES_ERROR].is_null(), "{ok}");
        assert_eq!(ok[RES_REGIONS][0][RES_STATS]["count"], 64);
    }

    const JWST_PIXAR_SR: f64 = 2.0e-14;
    const JWST_PIXAR_A2: f64 = 8.5e-4;
    const DISC_RADIUS: f64 = 10.0;
    const DISC_VALUE: f32 = 4.0;
    const FLOOR_VALUE: f32 = 1.0;
    const DISC_CENTRE: f64 = 50.0;

    fn jwst_north_up_cd() -> [[f64; 2]; 2] {
        let s = JWST_PIXAR_SR.sqrt().to_degrees();
        [[-s, 0.0], [0.0, s]]
    }

    fn jwst_header(extra: &[(&str, &str)]) -> crate::types::header::HduHeader {
        let mut header = header_with_cd(jwst_north_up_cd());
        header.set("BUNIT", "MJy/sr".into());
        header.set_f64("PIXAR_SR", JWST_PIXAR_SR);
        header.set_f64("PIXAR_A2", JWST_PIXAR_A2);
        for (k, v) in extra {
            header.set(k, (*v).to_string());
        }
        header
    }

    fn disc_frame() -> Array2<f32> {
        Array2::from_shape_fn((100, 100), |(y, x)| {
            let dx = x as f64 - DISC_CENTRE;
            let dy = y as f64 - DISC_CENTRE;
            if dx * dx + dy * dy <= DISC_RADIUS * DISC_RADIUS { DISC_VALUE } else { FLOOR_VALUE }
        })
    }

    fn lattice_count(shape: &RegionShape) -> u64 {
        (0..100).flat_map(|y| (0..100).map(move |x| (x, y))).filter(|(x, y)| shape.contains(*x as f64, *y as f64)).count() as u64
    }

    fn write_jwst_disc(dir: &std::path::Path, name: &str, extra: &[(&str, &str)]) -> String {
        let path = dir.join(name);
        write_fits_mono(path.to_str().unwrap(), &disc_frame(), Some(&jwst_header(extra))).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn circle(r: f64) -> RegionShape {
        RegionShape::Circle { x: DISC_CENTRE, y: DISC_CENTRE, r }
    }

    #[tokio::test]
    async fn region_stats_calibrate_a_jwst_mjy_sr_disc_to_jansky_ab_and_surface_brightness() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_jwst_disc(dir.path(), "jwst_disc.fits", &[]);
        let annulus = RegionShape::Annulus { x: DISC_CENTRE, y: DISC_CENTRE, r_inner: 14.0, r_outer: 18.0 };
        let ellipse = RegionShape::Ellipse { x: DISC_CENTRE, y: DISC_CENTRE, rx: 6.0, ry: 3.0, angle: 0.0 };
        let regions = vec![
            req("r4", circle(4.0)),
            req("r6", circle(6.0)),
            RegionStatsRequest { id: "net".into(), shape: circle(4.0), background: Some(annulus) },
            req("ellipse", ellipse.clone()),
            req("polygon", RegionShape::Polygon { points: vec![[45.0, 45.0], [55.0, 45.0], [50.0, 55.0]] }),
        ];
        let out = region_stats_cmd(key.clone(), regions, None, None, None).await.unwrap();
        assert!(out[RES_PHOTCAL][RES_LABEL].as_str().unwrap().contains("JWST MJy/sr"), "{out}");
        assert_eq!(out[RES_PHOTCAL]["convention"]["kind"], "jwst_mjy_sr");
        assert!(out[RES_CALIBRATION_WARNINGS].as_array().unwrap().is_empty(), "{out}");
        assert!((out[RES_PIXEL_AREA_ARCSEC2].as_f64().unwrap() - JWST_PIXAR_A2).abs() < 1e-15);

        let entries = out[RES_REGIONS].as_array().unwrap();
        let r4 = &entries[0][RES_STATS];
        let count = lattice_count(&circle(4.0));
        assert_eq!(count, 49);
        assert_eq!(r4["count"], count);
        let cal = &r4[RES_CALIBRATED];
        assert_eq!(cal["flux_source"], "sum");
        let expected_jy = DISC_VALUE as f64 * count as f64 * JWST_PIXAR_SR * 1e6;
        let flux_jy = cal["flux_jy"].as_f64().unwrap();
        assert!((flux_jy - expected_jy).abs() / expected_jy < 1e-9, "flux_jy={flux_jy} expected={expected_jy}");
        assert!((cal["flux_native"].as_f64().unwrap() - DISC_VALUE as f64 * count as f64).abs() < 1e-6);
        assert!(cal["flux_err_native"].is_null() && cal["flux_err_jy"].is_null() && cal["mag_ab_err"].is_null());
        let mag_ab = cal["mag_ab"].as_f64().unwrap();
        assert!((mag_ab - (-2.5 * flux_jy.log10() + 8.90)).abs() < 1e-9, "mag_ab={mag_ab}");
        assert!(cal["st_mag"].is_null());
        let area = cal["area_arcsec2"].as_f64().unwrap();
        assert!((area - count as f64 * JWST_PIXAR_A2).abs() < 1e-12, "area={area}");
        let geometric = cal["geometric_area_arcsec2"].as_f64().unwrap();
        assert!((geometric - std::f64::consts::PI * 16.0 * JWST_PIXAR_A2).abs() < 1e-12, "geometric={geometric}");
        let sb = cal["sb_mag_arcsec2"].as_f64().unwrap();
        assert!((sb - (-2.5 * (flux_jy / area).log10() + 8.90)).abs() < 1e-9, "sb={sb}");

        let r6 = &entries[1][RES_STATS][RES_CALIBRATED];
        assert_eq!(entries[1][RES_STATS]["count"], lattice_count(&circle(6.0)));
        let sb6 = r6["sb_mag_arcsec2"].as_f64().unwrap();
        assert!((sb6 - sb).abs() < 1e-6, "sb r4={sb} r6={sb6}");
        assert!(r6["flux_jy"].as_f64().unwrap() > flux_jy);

        let net = &entries[2][RES_STATS][RES_CALIBRATED];
        assert_eq!(net["flux_source"], "net");
        let net_jy = net["flux_jy"].as_f64().unwrap();
        let expected_net = (DISC_VALUE - FLOOR_VALUE) as f64 * count as f64 * JWST_PIXAR_SR * 1e6;
        assert!((net_jy - expected_net).abs() / expected_net < 1e-9, "net_jy={net_jy} expected={expected_net}");

        let entry = load_cached_full(&key).unwrap();
        let wcs = entry_wcs(&entry).expect("wcs");
        let centre = wcs.pixel_to_world(DISC_CENTRE, DISC_CENTRE);
        for e in entries {
            let cal = &e[RES_STATS][RES_CALIBRATED];
            assert!(cal.is_object(), "{e}");
            for key in [RES_RA, RES_DEC, RES_PA_SKY_DEG, RES_AREA_ARCSEC2, RES_GEOMETRIC_AREA_ARCSEC2] {
                assert_eq!(e[RES_STATS][RES_SKY][key], cal[key], "{key}: {e}");
            }
            if e[RES_ID] == "polygon" {
                assert!(cal["pa_sky_deg"].is_null());
                continue;
            }
            assert!((cal["ra"].as_f64().unwrap() - centre.ra).abs() < 1e-9, "{e}");
            assert!((cal["dec"].as_f64().unwrap() - centre.dec).abs() < 1e-9, "{e}");
        }
        let pa = entries[3][RES_STATS][RES_CALIBRATED]["pa_sky_deg"].as_f64().unwrap();
        assert!((pa - 90.0).abs() < 1e-6, "an east-west major axis on a north-up frame has PA 90, got {pa}");
        assert!(entries[0][RES_STATS][RES_CALIBRATED]["pa_sky_deg"].is_null());

        let rotated = RegionShape::Box { x: DISC_CENTRE, y: DISC_CENTRE, width: 8.0, height: 4.0, angle: 30.0 };
        let cal = entry_calibration(&entry);
        let stats = region_data_stats(entry.arr(), &rotated, None, None, None, SigmaClip::default()).unwrap();
        let value = calibrated_flux_json(&stats, &rotated, &cal);
        let pa = value["pa_sky_deg"].as_f64().unwrap();
        assert!((pa - 120.0).abs() < 1e-6, "a box rotated 30 deg counter-clockwise from east-west has PA 120, got {pa}");
        assert_eq!(value["flux_source"], "sum");
    }

    #[tokio::test]
    async fn region_stats_refuse_calibration_on_display_referred_or_processed_data() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_jwst_disc(dir.path(), "stretched.fits", &[("ABDISP", "T"), ("ABPROC", "arcsinh_stretch")]);
        let out = region_stats_cmd(key, vec![req("r4", circle(4.0))], None, None, None).await.unwrap();
        assert!(out[RES_PHOTCAL].is_null(), "{out}");
        let warnings: Vec<&str> = out[RES_CALIBRATION_WARNINGS].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
        assert!(warnings[0].contains("display-referred"), "{warnings:?}");
        assert!(warnings.iter().any(|w| *w == "photometry on processed data (arcsinh_stretch)"), "{warnings:?}");
        let stats = &out[RES_REGIONS][0][RES_STATS];
        assert!(stats[RES_CALIBRATED].is_null(), "{stats}");
        assert_eq!(stats["count"], 49);
        let wcs_area = JWST_PIXAR_SR * crate::core::metadata::photcal::ARCSEC_PER_RADIAN.powi(2);
        let area = out[RES_PIXEL_AREA_ARCSEC2].as_f64().unwrap();
        assert!((area - wcs_area).abs() / wcs_area < 1e-9, "refused calibration falls back to the WCS pixel area: {area} vs {wcs_area}");
        assert!(stats[RES_SKY][RES_RA].is_f64() && stats[RES_SKY][RES_DEC].is_f64(), "{stats}");
        assert!((stats[RES_SKY][RES_AREA_ARCSEC2].as_f64().unwrap() - 49.0 * area).abs() < 1e-12, "{stats}");
    }

    #[tokio::test]
    async fn region_stats_without_calibration_cards_report_null_photcal_and_the_missing_reason() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain_disc.fits");
        write_fits_mono(path.to_str().unwrap(), &disc_frame(), None).unwrap();
        let key = path.to_str().unwrap().to_string();
        let out = region_stats_cmd(key.clone(), vec![req("r4", circle(4.0))], None, None, None).await.unwrap();
        assert!(out[RES_PHOTCAL].is_null());
        assert!(out[RES_PIXEL_AREA_ARCSEC2].is_null());
        let warnings = out[RES_CALIBRATION_WARNINGS].as_array().unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].as_str().unwrap().contains("BUNIT"), "{warnings:?}");
        let stats = &out[RES_REGIONS][0][RES_STATS];
        assert!(stats.get(RES_CALIBRATED).is_some_and(Value::is_null), "{stats}");
        assert!(stats.get(RES_SKY).is_some_and(Value::is_null), "{stats}");
        assert_eq!(stats["count"], 49);

        let entry = load_cached_full(&key).unwrap();
        let cal = entry_calibration(&entry);
        assert!(cal.photcal.is_none() && cal.wcs.is_none() && cal.pixel_area_arcsec2.is_none());
        let regions = vec![req("r4", circle(4.0)), req("r6", circle(6.0))];
        let with_none = stats_for_entry(&entry, &regions, None, None, SigmaClip::default(), &EntryCalibration::none());
        let with_entry = stats_for_entry(&entry, &regions, None, None, SigmaClip::default(), &cal);
        assert_eq!(with_none, with_entry);
        assert_eq!(with_none[0][RES_STATS]["sum"], DISC_VALUE as f64 * 49.0);
    }

    #[test]
    fn entry_calibration_warns_about_anisotropic_pixel_scales_and_uses_their_mean() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("anisotropic.fits");
        let s = 1.0 / 3600.0;
        let header = header_with_cd([[-s, 0.0], [0.0, 1.05 * s]]);
        write_fits_mono(path.to_str().unwrap(), &disc_frame(), Some(&header)).unwrap();
        let entry = load_cached_full(path.to_str().unwrap()).unwrap();
        let cal = entry_calibration(&entry);
        assert!(cal.photcal.is_none());
        assert!(cal.warnings.iter().any(|w| w.contains("pixel scales differ per axis")), "{:?}", cal.warnings);
        let expected = (1.025f64).powi(2);
        assert!((cal.pixel_area_arcsec2.unwrap() - expected).abs() < 1e-9, "{:?}", cal.pixel_area_arcsec2);

        let isotropic = header_with_cd(north_up_cd());
        let path = dir.path().join("isotropic.fits");
        write_fits_mono(path.to_str().unwrap(), &disc_frame(), Some(&isotropic)).unwrap();
        let entry = load_cached_full(path.to_str().unwrap()).unwrap();
        let cal = entry_calibration(&entry);
        assert!(!cal.warnings.iter().any(|w| w.contains("pixel scales differ")), "{:?}", cal.warnings);
        assert!((cal.pixel_area_arcsec2.unwrap() - 1.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn export_cmd_rejects_unsupported_system() {
        let err = regions_export_cmd("nowhere.fits".into(), vec![], "galactic".into(), None)
            .await
            .unwrap_err();
        assert!(err.contains("unsupported coordinate system 'galactic'"), "{err}");
    }

    const SB_DISC_VALUE: f32 = 3.0;
    const SB_DISC_RADIUS: f64 = 10.0;
    const SB_DISC_FLOOR: f32 = -0.01;
    const SB_PIXEL_SCALE_ARCSEC: f64 = 0.03;

    fn square_arcsec_sr() -> f64 {
        (1.0f64 / ARCSEC_PER_DEGREE).to_radians().powi(2)
    }

    fn sb_north_up_cd() -> [[f64; 2]; 2] {
        let s = SB_PIXEL_SCALE_ARCSEC / ARCSEC_PER_DEGREE;
        [[-s, 0.0], [0.0, s]]
    }

    fn write_flat_sb_disc(dir: &std::path::Path) -> String {
        let arr = Array2::from_shape_fn((100, 100), |(y, x)| {
            let dx = x as f64 - DISC_CENTRE;
            let dy = y as f64 - DISC_CENTRE;
            if dx * dx + dy * dy <= SB_DISC_RADIUS * SB_DISC_RADIUS { SB_DISC_VALUE } else { SB_DISC_FLOOR }
        });
        let mut header = header_with_cd(sb_north_up_cd());
        header.set("BUNIT", "MJy/sr".into());
        header.set_f64("PIXAR_SR", square_arcsec_sr() * SB_PIXEL_SCALE_ARCSEC.powi(2));
        header.set_f64("PIXAR_A2", SB_PIXEL_SCALE_ARCSEC.powi(2));
        let path = dir.join("sb_disc.fits");
        write_fits_mono(path.to_str().unwrap(), &arr, Some(&header)).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn axis_angle_gap_deg(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(POSITION_ANGLE_PERIOD_DEG);
        d.min(POSITION_ANGLE_PERIOD_DEG - d)
    }

    #[tokio::test]
    async fn sb_profile_cmd_calibrates_a_flat_jwst_disc_to_a_constant_surface_brightness() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_flat_sb_disc(dir.path());
        let shape = RegionShape::Ellipse { x: DISC_CENTRE, y: DISC_CENTRE, rx: 24.0, ry: 12.0, angle: 30.0 };
        let out = sb_profile_cmd(key.clone(), shape, None, None, None).await.unwrap();
        assert!(out[RES_PHOTCAL][RES_LABEL].as_str().unwrap().contains("JWST MJy/sr"), "{out}");
        assert!(out[RES_CALIBRATION_WARNINGS].as_array().unwrap().is_empty(), "{out}");
        assert!((out[RES_PIXEL_AREA_ARCSEC2].as_f64().unwrap() - SB_PIXEL_SCALE_ARCSEC.powi(2)).abs() < 1e-15);
        assert!((out[RES_PIXEL_SCALE_ARCSEC].as_f64().unwrap() - SB_PIXEL_SCALE_ARCSEC).abs() < 1e-9);
        assert_eq!(out["bin_width"], 1.0);
        assert_eq!(out["ellipticity"], 0.5);
        assert_eq!(out["angle_deg"], 30.0);
        assert_eq!(out[RES_MASKED], false);
        assert!(out["background"].is_null());

        let expected_mu = -2.5 * (SB_DISC_VALUE as f64 * 1e6 * square_arcsec_sr()).log10() + 8.90;
        assert!((expected_mu - 19.279_32).abs() < 1e-4, "3 MJy/sr is 19.279 mag/arcsec^2, got {expected_mu}");
        let bins = out[RES_BINS].as_array().unwrap();
        assert_eq!(bins.len(), 24);
        let mut inside = 0;
        for bin in bins {
            let sma = bin["sma"].as_f64().unwrap();
            assert!((bin[RES_SMA_ARCSEC].as_f64().unwrap() - sma * SB_PIXEL_SCALE_ARCSEC).abs() < 1e-9, "{bin}");
            if bin["sma_outer"].as_f64().unwrap() <= SB_DISC_RADIUS {
                inside += 1;
                let mu = bin[RES_MU_AB].as_f64().unwrap();
                assert!((mu - expected_mu).abs() < 1e-6, "sma {sma}: mu {mu} expected {expected_mu}");
                if bin["count"] == 1 {
                    assert!(bin[RES_MU_ERR].is_null(), "{bin}");
                } else {
                    assert_eq!(bin[RES_MU_ERR], 0.0, "{bin}");
                }
                assert!(bin[RES_MAG_AB_CUMULATIVE].as_f64().unwrap().is_finite());
            }
        }
        assert_eq!(inside, 10);
        assert!(bins.last().unwrap()[RES_MU_AB].is_null(), "a negative outer bin has no surface brightness");
        assert!(bins.last().unwrap()[RES_MU_ERR].is_null());
        assert!((bins.last().unwrap()["mean"].as_f64().unwrap() - SB_DISC_FLOOR as f64).abs() < 1e-9);
        assert!(out[RES_TOTAL_MAG_AB].as_f64().unwrap().is_finite());
        assert!(out[RES_R50_PX].as_f64().unwrap() > 0.0);
        assert!(out[RES_R80_PX].as_f64().unwrap() > out[RES_R50_PX].as_f64().unwrap());
        assert!(out[RES_R90_PX].as_f64().unwrap() > out[RES_R80_PX].as_f64().unwrap());
        assert!(out[RES_PETROSIAN_RADIUS_PX].as_f64().unwrap() > 0.0);
        let notes: Vec<&str> = out[RES_NOTES].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
        assert_eq!(notes, vec![SB_COARSE_BIN_NOTE, SB_NO_BACKGROUND_NOTE]);
    }

    #[tokio::test]
    async fn sb_profile_sky_position_angle_matches_the_calibrated_region_block_on_a_north_up_frame() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_flat_sb_disc(dir.path());
        let wide = RegionShape::Ellipse { x: DISC_CENTRE, y: DISC_CENTRE, rx: 16.0, ry: 8.0, angle: 30.0 };
        let out = sb_profile_cmd(key.clone(), wide.clone(), Some(2.0), None, None).await.unwrap();
        let pa = out[RES_SKY_PA_DEG].as_f64().unwrap();
        assert!((pa - 120.0).abs() < 1e-6, "major axis 30 deg from the x axis on a north-up frame has PA 120, got {pa}");

        let entry = load_cached_full(&key).unwrap();
        let cal = entry_calibration(&entry);
        let stats = region_data_stats(entry.arr(), &wide, None, None, None, SigmaClip::default()).unwrap();
        let region_pa = calibrated_flux_json(&stats, &wide, &cal)["pa_sky_deg"].as_f64().unwrap();
        assert!((pa - region_pa).abs() < 1e-9, "profile {pa} vs region block {region_pa}");

        let tall = RegionShape::Ellipse { x: DISC_CENTRE, y: DISC_CENTRE, rx: 8.0, ry: 16.0, angle: -60.0 };
        let swapped = sb_profile_cmd(key.clone(), tall, Some(2.0), None, None).await.unwrap();
        assert!((swapped[RES_SKY_PA_DEG].as_f64().unwrap() - 120.0).abs() < 1e-6, "{swapped}");
        assert_eq!(swapped["angle_deg"], 30.0);
        assert_eq!(swapped[RES_BINS], out[RES_BINS]);
        let notes = swapped[RES_NOTES].as_array().unwrap();
        assert_eq!(notes.last().unwrap(), SB_AXES_SWAPPED_NOTE);

        let annulus = RegionShape::Annulus { x: DISC_CENTRE, y: DISC_CENTRE, r_inner: 20.0, r_outer: 30.0 };
        let with_bg = sb_profile_cmd(key, circle(12.0), Some(1.0), Some(annulus), None).await.unwrap();
        assert!((with_bg["background"]["median"].as_f64().unwrap() - SB_DISC_FLOOR as f64).abs() < 1e-9, "{with_bg}");
        assert!(with_bg[RES_SKY_PA_DEG].is_null(), "a circle has no major axis: {with_bg}");
        assert_eq!(with_bg[RES_NOTES][1], "background from annulus region");
    }

    #[tokio::test]
    async fn region_stats_report_sky_centre_pa_and_area_on_a_wcs_only_frame_without_flux_calibration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wcs_only.fits");
        write_fits_mono(path.to_str().unwrap(), &disc_frame(), Some(&header_with_cd(jwst_north_up_cd()))).unwrap();
        let key = path.to_str().unwrap().to_string();
        let ellipse = RegionShape::Ellipse { x: DISC_CENTRE, y: DISC_CENTRE, rx: 6.0, ry: 3.0, angle: 0.0 };
        let out = region_stats_cmd(key.clone(), vec![req("r4", circle(4.0)), req("e", ellipse)], None, None, None)
            .await
            .unwrap();
        assert!(out[RES_PHOTCAL].is_null(), "{out}");
        let pixel_area = out[RES_PIXEL_AREA_ARCSEC2].as_f64().unwrap();
        let expected_area = JWST_PIXAR_SR * crate::core::metadata::photcal::ARCSEC_PER_RADIAN.powi(2);
        assert!((pixel_area - expected_area).abs() / expected_area < 1e-9, "{pixel_area} vs {expected_area}");

        let centre = entry_wcs(&load_cached_full(&key).unwrap()).unwrap().pixel_to_world(DISC_CENTRE, DISC_CENTRE);
        let r4 = &out[RES_REGIONS][0][RES_STATS];
        assert!(r4[RES_CALIBRATED].is_null(), "{r4}");
        let sky = &r4[RES_SKY];
        assert!((sky[RES_RA].as_f64().unwrap() - centre.ra).abs() < 1e-9, "{sky}");
        assert!((sky[RES_DEC].as_f64().unwrap() - centre.dec).abs() < 1e-9, "{sky}");
        assert!(sky[RES_PA_SKY_DEG].is_null(), "{sky}");
        assert!((sky[RES_AREA_ARCSEC2].as_f64().unwrap() - 49.0 * pixel_area).abs() < 1e-12, "{sky}");
        let geometric = std::f64::consts::PI * 16.0 * pixel_area;
        assert!((sky[RES_GEOMETRIC_AREA_ARCSEC2].as_f64().unwrap() - geometric).abs() < 1e-12, "{sky}");

        let pa = out[RES_REGIONS][1][RES_STATS][RES_SKY][RES_PA_SKY_DEG].as_f64().unwrap();
        assert!((pa - 90.0).abs() < 1e-6, "an east-west ellipse on a north-up frame has PA 90, got {pa}");
    }

    #[tokio::test]
    async fn region_position_angle_follows_the_major_axis_of_tall_ellipses_and_boxes_like_the_sb_profile() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_flat_sb_disc(dir.path());
        let entry = load_cached_full(&key).unwrap();
        let cal = entry_calibration(&entry);
        let region_pa = |shape: &RegionShape| {
            let stats = region_data_stats(entry.arr(), shape, None, None, None, SigmaClip::default()).unwrap();
            calibrated_flux_json(&stats, shape, &cal)[RES_PA_SKY_DEG].as_f64().unwrap()
        };
        let ellipse = |rx: f64, ry: f64, angle: f64| RegionShape::Ellipse { x: DISC_CENTRE, y: DISC_CENTRE, rx, ry, angle };
        let tall_ellipses = [
            (ellipse(8.0, 16.0, -60.0), 120.0),
            (ellipse(8.0, 16.0, 300.0), 120.0),
            (ellipse(3.0, 8.0, 0.0), 0.0),
        ];
        for (tall, expected) in tall_ellipses {
            let pa = region_pa(&tall);
            assert!(axis_angle_gap_deg(pa, expected) < 1e-6, "{tall:?}: major-axis PA {expected}, got {pa}");
            let profile = sb_profile_cmd(key.clone(), tall.clone(), Some(2.0), None, None).await.unwrap();
            assert_eq!(profile[RES_SKY_PA_DEG].as_f64().unwrap(), pa, "{tall:?}");
        }
        let boxed = |width: f64, height: f64, angle: f64| RegionShape::Box { x: DISC_CENTRE, y: DISC_CENTRE, width, height, angle };
        let boxes = [(boxed(4.0, 8.0, 0.0), 0.0), (boxed(8.0, 4.0, 30.0), 120.0), (boxed(4.0, 8.0, -60.0), 120.0)];
        for (shape, expected) in boxes {
            let pa = region_pa(&shape);
            assert!(axis_angle_gap_deg(pa, expected) < 1e-6, "{shape:?}: major-axis PA {expected}, got {pa}");
        }
    }

    #[tokio::test]
    async fn position_angles_on_galactic_and_ecliptic_images_are_measured_from_equatorial_north() {
        use crate::core::astrometry::frames::{ecliptic_j2000_to_icrs, galactic_to_icrs};
        let dir = tempfile::tempdir().unwrap();
        let reference_pixel = 49.5;
        let along_x = RegionShape::Ellipse { x: reference_pixel, y: reference_pixel, rx: 10.0, ry: 3.0, angle: 0.0 };
        let cases = [
            ("GLON-TAN", "GLAT-TAN", galactic_to_icrs(0.0, 90.0), 31.40),
            ("ELON-TAN", "ELAT-TAN", ecliptic_j2000_to_icrs(0.0, 90.0), 90.0 - 23.4393),
        ];
        for (ctype1, ctype2, pole, approximate) in cases {
            let mut header = header_with_cd(north_up_cd());
            header.set("CTYPE1", ctype1.into());
            header.set("CTYPE2", ctype2.into());
            header.set_f64("CRVAL1", 0.0);
            header.set_f64("CRVAL2", 0.0);
            header.set("BUNIT", "MJy/sr".into());
            let path = dir.path().join(format!("{ctype1}.fits"));
            write_fits_mono(path.to_str().unwrap(), &disc_frame(), Some(&header)).unwrap();
            let key = path.to_str().unwrap().to_string();
            let wcs = entry_wcs(&load_cached_full(&key).unwrap()).expect("wcs");
            let centre = wcs.pixel_to_world(reference_pixel, reference_pixel);
            let native_north = position_angle_deg(centre.ra, centre.dec, pole.0, pole.1);
            let expected = (native_north + 90.0).rem_euclid(POSITION_ANGLE_PERIOD_DEG);
            assert!((expected - approximate).abs() < 0.01, "{ctype1}: native equator at the reference point has PA {expected}");

            let out = region_stats_cmd(key.clone(), vec![req("e", along_x.clone())], None, None, None).await.unwrap();
            let stats = &out[RES_REGIONS][0][RES_STATS];
            let pa = stats[RES_CALIBRATED][RES_PA_SKY_DEG].as_f64().unwrap();
            assert!(axis_angle_gap_deg(pa, expected) < 1e-6, "{ctype1}: the pixel x axis has equatorial PA {expected}, got {pa}");
            assert_eq!(stats[RES_SKY][RES_PA_SKY_DEG], stats[RES_CALIBRATED][RES_PA_SKY_DEG]);

            let far = wcs.pixel_to_world(reference_pixel + 20.0, reference_pixel);
            let line_pa = position_angle_deg(centre.ra, centre.dec, far.ra, far.dec);
            assert!(axis_angle_gap_deg(pa, line_pa) < 1e-6, "{ctype1}: region {pa} vs a line along the same axis {line_pa}");

            let sb = sb_profile_cmd(key, along_x.clone(), None, None, None).await.unwrap();
            assert_eq!(sb[RES_SKY_PA_DEG].as_f64().unwrap(), pa, "{ctype1}");
        }
    }

    #[tokio::test]
    async fn a_circle_a_round_ellipse_or_a_square_box_has_no_sky_position_angle_in_either_panel() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_flat_sb_disc(dir.path());
        let round = RegionShape::Ellipse { x: DISC_CENTRE, y: DISC_CENTRE, rx: 10.0, ry: 10.0, angle: 37.0 };
        let square = RegionShape::Box { x: DISC_CENTRE, y: DISC_CENTRE, width: 8.0, height: 8.0, angle: 20.0 };
        for shape in [circle(10.0), round.clone()] {
            let out = sb_profile_cmd(key.clone(), shape.clone(), Some(2.0), None, None).await.unwrap();
            assert!(out[RES_SKY_PA_DEG].is_null(), "{shape:?}: {}", out[RES_SKY_PA_DEG]);
        }
        let regions = vec![req("circle", circle(10.0)), req("round", round), req("square", square)];
        let out = region_stats_cmd(key, regions, None, None, None).await.unwrap();
        for entry in out[RES_REGIONS].as_array().unwrap() {
            assert!(entry[RES_STATS][RES_CALIBRATED].is_object(), "{entry}");
            assert!(entry[RES_STATS][RES_CALIBRATED][RES_PA_SKY_DEG].is_null(), "{entry}");
            assert!(entry[RES_STATS][RES_SKY][RES_PA_SKY_DEG].is_null(), "{entry}");
            assert!(entry[RES_STATS][RES_SKY][RES_RA].is_f64(), "{entry}");
        }
    }

    #[tokio::test]
    async fn region_stats_and_sb_profile_treat_hst_counts_without_exptime_as_uncalibrated() {
        let dir = tempfile::tempdir().unwrap();
        let mut header = header_with_cd(north_up_cd());
        header.set("BUNIT", "COUNTS".into());
        header.set_f64("PHOTFLAM", 1.5e-19);
        header.set_f64("PHOTPLAM", 5921.0);
        let path = dir.path().join("hst_counts_no_exptime.fits");
        write_fits_mono(path.to_str().unwrap(), &disc_frame(), Some(&header)).unwrap();
        let key = path.to_str().unwrap().to_string();
        let out = region_stats_cmd(key.clone(), vec![req("r4", circle(4.0))], None, None, None).await.unwrap();
        assert!(out[RES_PHOTCAL].is_null(), "a PhotCal that cannot reach Jy must not be reported as calibration: {out}");
        assert!(out[RES_REGIONS][0][RES_STATS][RES_CALIBRATED].is_null(), "{out}");
        let first = out[RES_CALIBRATION_WARNINGS][0].as_str().unwrap();
        assert!(first.starts_with("EXPTIME missing"), "{first}");
        assert!((out[RES_PIXEL_AREA_ARCSEC2].as_f64().unwrap() - 1.0).abs() < 1e-9, "{out}");
        let sb = sb_profile_cmd(key, circle(8.0), None, None, None).await.unwrap();
        assert!(sb[RES_PHOTCAL].is_null(), "{sb}");
        assert!(sb[RES_CALIBRATION_WARNINGS][0].as_str().unwrap().starts_with("EXPTIME missing"), "{sb}");

        header.set_f64("EXPTIME", 500.0);
        let path = dir.path().join("hst_counts_with_exptime.fits");
        write_fits_mono(path.to_str().unwrap(), &disc_frame(), Some(&header)).unwrap();
        let out = region_stats_cmd(path.to_str().unwrap().into(), vec![req("r4", circle(4.0))], None, None, None).await.unwrap();
        assert!(out[RES_PHOTCAL].is_object(), "{out}");
        assert!(out[RES_REGIONS][0][RES_STATS][RES_CALIBRATED]["flux_jy"].as_f64().is_some(), "{out}");
    }

    #[tokio::test]
    async fn a_refused_calibration_names_its_missing_jansky_factor_before_the_derived_pixel_area_warning() {
        let dir = tempfile::tempdir().unwrap();
        for (name, photmjsr) in [("photmjsr_zero", 0.0), ("photmjsr_negative", -1.2)] {
            let mut header = header_with_cd(north_up_cd());
            header.set("BUNIT", "DN/s".into());
            header.set_f64("PHOTMJSR", photmjsr);
            let path = dir.path().join(format!("{name}.fits"));
            write_fits_mono(path.to_str().unwrap(), &disc_frame(), Some(&header)).unwrap();
            let key = path.to_str().unwrap().to_string();
            let out = region_stats_cmd(key.clone(), vec![req("r4", circle(4.0))], None, None, None).await.unwrap();
            assert!(out[RES_PHOTCAL].is_null(), "{name}: {out}");
            let warnings: Vec<&str> =
                out[RES_CALIBRATION_WARNINGS].as_array().unwrap().iter().filter_map(|w| w.as_str()).collect();
            assert!(warnings[0].contains("no finite positive conversion to Jy"), "{name}: {warnings:?}");
            assert!(warnings.iter().any(|w| w.contains("pixel area derived from the WCS")), "{name}: {warnings:?}");
            let sb = sb_profile_cmd(key, circle(8.0), None, None, None).await.unwrap();
            assert!(sb[RES_PHOTCAL].is_null(), "{name}: {sb}");
            let first = sb[RES_CALIBRATION_WARNINGS][0].as_str().unwrap();
            assert!(first.contains("no finite positive conversion to Jy"), "{name}: {first}");
        }
    }

    #[tokio::test]
    async fn sb_profile_single_pixel_bin_keeps_its_surface_brightness_but_has_no_error() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_flat_sb_disc(dir.path());
        let out = sb_profile_cmd(key, circle(8.0), Some(1.0), None, None).await.unwrap();
        let first = &out[RES_BINS][0];
        assert_eq!(first["count"], 1, "{first}");
        assert!(first["std"].is_null(), "{first}");
        assert!(first[RES_MU_ERR].is_null(), "{first}");
        assert!(first[RES_MU_AB].as_f64().unwrap().is_finite(), "{first}");
        let second = &out[RES_BINS][1];
        assert!(second["count"].as_u64().unwrap() > 1, "{second}");
        assert_eq!(second["std"], 0.0, "{second}");
        assert_eq!(second[RES_MU_ERR], 0.0, "{second}");
    }

    #[tokio::test]
    async fn sb_profile_cmd_rejects_too_elongated_or_too_large_regions_before_loading() {
        let thin = RegionShape::Ellipse { x: 5.0, y: 5.0, rx: 100.0, ry: 4.0, angle: 0.0 };
        let err = sb_profile_cmd("nowhere.fits".into(), thin, None, None, None).await.unwrap_err();
        assert!(err.contains("rx = 100, ry = 4") && err.contains("too elongated"), "{err}");
        let tall = RegionShape::Ellipse { x: 5.0, y: 5.0, rx: 4.0, ry: 100.0, angle: 0.0 };
        let err = sb_profile_cmd("nowhere.fits".into(), tall, None, None, None).await.unwrap_err();
        assert!(err.contains("rx = 4, ry = 100") && err.contains("too elongated"), "{err}");
        let huge = RegionShape::Circle { x: 5.0, y: 5.0, r: 5000.0 };
        let err = sb_profile_cmd("nowhere.fits".into(), huge, None, None, None).await.unwrap_err();
        assert!(err.contains("circle radius r must be at most 4096 px") && err.contains("got 5000 px"), "{err}");
        let wide = RegionShape::Ellipse { x: 5.0, y: 5.0, rx: 3000.0, ry: 5000.0, angle: 0.0 };
        let err = sb_profile_cmd("nowhere.fits".into(), wide, None, None, None).await.unwrap_err();
        assert!(err.contains("semi-major axis") && err.contains("got 5000 px"), "{err}");
        let at_limit = RegionShape::Ellipse { x: 5.0, y: 5.0, rx: 100.0, ry: 5.0, angle: 0.0 };
        assert_eq!(ellipse_geometry(&at_limit).unwrap().3, 0.95);
    }

    #[tokio::test]
    async fn sb_profile_cmd_rejects_bad_bin_widths_and_non_elliptical_shapes_before_loading() {
        let boxed = RegionShape::Box { x: 5.0, y: 5.0, width: 4.0, height: 2.0, angle: 0.0 };
        let err = sb_profile_cmd("nowhere.fits".into(), boxed, None, None, None).await.unwrap_err();
        assert!(err.contains("needs a circle or ellipse region, got box"), "{err}");
        for bin_width in [0.25, 100.0, f64::NAN] {
            let err = sb_profile_cmd("nowhere.fits".into(), circle(4.0), Some(bin_width), None, None).await.unwrap_err();
            assert!(err.contains("bin_width must be between 0.5 and 64 px"), "{err}");
        }
        let bad_bg = RegionShape::Circle { x: 1.0, y: 1.0, r: -2.0 };
        let err = sb_profile_cmd("nowhere.fits".into(), circle(4.0), None, Some(bad_bg), None).await.unwrap_err();
        assert!(err.contains("invalid region"), "{err}");
    }
}
