use std::sync::Mutex;
use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, cached_header, derived_output_header, load_from_cache_or_disk, output_stem, resolve_output_dir, write_derived_fits, OutputValues, MAX_PREVIEW_DIM};
use crate::core::imaging::stf::{auto_stf, make_stf_u8_fn, AutoStfConfig};
use crate::cmd::helpers;
use crate::cmd::processing::source_header;
use crate::core::alignment::pair::{align_pair_with_label, shift_image_subpixel, AlignPairResult};
use crate::core::alignment::wcs_reproject::{pixel_area_ratio, point_source_residual, prefilter_coarse_target, reproject_to_grid, REF_TIE_TOLERANCE};
use crate::core::astrometry::wcs::{WcsKind, WcsTransform};
use crate::core::compose::rgb::{harmonize_dimensions, align_channels};
use crate::core::analysis::photometry::SATURATION_KEYWORDS;
use crate::core::compose::channel_blend::{blend_channels, BlendWeight};
use crate::core::imaging::resample::{compute_wcs_updates, resample_image};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::metadata::photcal::{GENERIC_ZERO_POINT_KEYS, ROMAN_PIXEL_AREA_SR_KEYS};
use crate::infra::fits::writer::filter_header;
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::infra::wcs_source::load_wcs;
use crate::types::compose::AlignMethod;
use crate::types::header::HduHeader;
use crate::types::ImageStats;
use crate::types::constants::{MAX_DIMENSION_RATIO, RES_DIMENSIONS, RES_ELAPSED_MS, RES_MAX, RES_MEAN, RES_MEDIAN, RES_MIN, RES_PNG_PATH, RES_STATS_B, RES_STATS_G, RES_STATS_R, ALIGN_METHOD, DIMENSIONS, CHANNELS, RES_CHANNEL, RES_PATH, RES_FILE_SIZE_BYTES, RES_OFFSET, RES_BLEND_PRESET, RES_CHANNEL_COUNT, RES_AUTO_STF, RES_CACHE_KEY, RES_CONFIDENCE, RES_METHOD_USED, RES_MATCHED_STARS, RES_INLIERS, RES_RESIDUAL_PX, RES_REGISTERED, RES_STF_LINKED, RES_STF_NOTE, STF_B, STF_G, STF_R, RES_REFERENCE_INDEX, RES_REFERENCE_RULE, RES_RUN_TOKEN, RES_REPROJECTED, RES_WCS_SCALE_RATIO, RES_WCS_ROTATION_DEG, RES_PREFILTER_K, RES_WCS_KIND, RES_RESIDUAL_MEASURED, RES_WARNINGS, ALIGN_METHOD_PHASE};

use super::rgb::{composite_png_path, load_entry};

const ABPROC_ALIGNED: &str = "aligned";
const LEVEL_SPREAD_SIGMAS: f64 = 3.0;
const LEVEL_RATIO_LIMIT: f64 = 1.5;

fn sig3(v: f64) -> String {
    if !v.is_finite() || v == 0.0 {
        return format!("{v}");
    }
    let exp = v.abs().log10().floor() as i32;
    let scale = 10f64.powi(2 - exp);
    let rounded = (v * scale).round() / scale;
    let exp = rounded.abs().log10().floor() as i32;
    let decimals = (2 - exp).max(0) as usize;
    format!("{rounded:.decimals$}")
}

fn stf_unlinked_note(medians: [f64; 3]) -> String {
    format!(
        "Preview, Adjust and the PNG export use an unlinked auto-STF because the blended channel medians differ (R {} · G {} · B {}); the composite FITS data are unchanged. To equalise the channels, turn Match levels on (with one filter per channel) or remove each channel's pedestal with Background › Neutralize before blending.",
        sig3(medians[0]),
        sig3(medians[1]),
        sig3(medians[2])
    )
}

pub(crate) fn stf_should_unlink(stats: [&ImageStats; 3]) -> bool {
    let medians = stats.map(|s| s.median);
    let lo = medians.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = medians.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let sigma_max = stats.iter().map(|s| s.sigma).fold(0.0, f64::max);
    let spread_hit = hi - lo > LEVEL_SPREAD_SIGMAS * sigma_max;
    let ratio_hit = lo > 0.0 && hi / lo > LEVEL_RATIO_LIMIT;
    spread_hit || ratio_hit
}
const PIXEL_AREA_CARDS: [&str; 2] = ["PIXAR_SR", "PIXAR_A2"];
const PER_PIXEL_FLUX_CARDS: [&str; 2] = ["PHOTFLAM", "PHOTFNU"];
const CALIBRATION_ZERO_POINT_CARDS: [&str; 2] = ["ZP", "ZPTMAG"];
const PHYSICAL_CARDS: [&str; 6] = ["LTV1", "LTV2", "LTM1_1", "LTM1_2", "LTM2_1", "LTM2_2"];

fn update_wcs_for_offset(header: &mut HduHeader, dy: f64, dx: f64) {
    if dy.abs() < 1e-12 && dx.abs() < 1e-12 {
        return;
    }
    if let Some(crpix1) = header.get_f64("CRPIX1") {
        header.set_f64("CRPIX1", crpix1 - dx);
    }
    if let Some(crpix2) = header.get_f64("CRPIX2") {
        header.set_f64("CRPIX2", crpix2 - dy);
    }
}

pub(crate) fn rescale_per_pixel_calibration(header: &mut HduHeader, area_ratio: f64) {
    if !area_ratio.is_finite() || area_ratio <= 0.0 {
        return;
    }
    let per_pixel = PIXEL_AREA_CARDS.iter().chain(ROMAN_PIXEL_AREA_SR_KEYS.iter()).chain(PER_PIXEL_FLUX_CARDS.iter());
    for &key in per_pixel {
        if let Some(v) = header.get_f64(key) {
            header.set_f64(key, v * area_ratio);
        }
    }
    let zero_point_shift = 2.5 * area_ratio.log10();
    for key in GENERIC_ZERO_POINT_KEYS.into_iter().chain(CALIBRATION_ZERO_POINT_CARDS) {
        if let Some(v) = header.get_f64(key) {
            header.set_f64(key, v - zero_point_shift);
        }
    }
    for key in SATURATION_KEYWORDS {
        header.remove(key);
    }
}

pub(crate) fn rescale_header_to_grid(header: &mut HduHeader, original_dims: (usize, usize), target_dims: (usize, usize)) {
    if original_dims == target_dims
        || original_dims.0 == 0
        || original_dims.1 == 0
        || target_dims.0 == 0
        || target_dims.1 == 0
    {
        return;
    }
    for (key, value) in compute_wcs_updates(header, original_dims, target_dims) {
        if !key.starts_with("NAXIS") {
            header.set_f64(&key, value);
        }
    }
    let area_ratio = (original_dims.0 as f64 / target_dims.0 as f64) * (original_dims.1 as f64 / target_dims.1 as f64);
    rescale_per_pixel_calibration(header, area_ratio);
}

fn aligned_channel_header(
    source: &HduHeader,
    original_dims: (usize, usize),
    target_dims: (usize, usize),
    offset: (f64, f64),
    copy_wcs: bool,
    copy_metadata: bool,
) -> Option<HduHeader> {
    let mut hdr = filter_header(source, copy_wcs, copy_metadata)?;
    rescale_header_to_grid(&mut hdr, original_dims, target_dims);
    if copy_wcs {
        update_wcs_for_offset(&mut hdr, offset.0, offset.1);
    }
    Some(derived_output_header(Some(&hdr), ABPROC_ALIGNED, OutputValues::Linear))
}

fn registered_channel_header(
    target: Option<&HduHeader>,
    reference: Option<&HduHeader>,
    target_dims: (usize, usize),
    grid_dims: (usize, usize),
    registered: bool,
    area_ratio: Option<f64>,
) -> HduHeader {
    if !registered {
        let mut header = target.cloned().unwrap_or_else(HduHeader::empty);
        rescale_header_to_grid(&mut header, target_dims, grid_dims);
        return derived_output_header(Some(&header), ABPROC_ALIGNED, OutputValues::Linear);
    }
    let mut header = target.and_then(|t| filter_header(t, false, true)).unwrap_or_else(HduHeader::empty);
    for key in PHYSICAL_CARDS {
        header.remove(key);
    }
    match area_ratio {
        Some(ratio) => rescale_per_pixel_calibration(&mut header, ratio),
        None if target_dims != grid_dims && grid_dims.0 > 0 && grid_dims.1 > 0 => {
            let ratio = (target_dims.0 as f64 / grid_dims.0 as f64) * (target_dims.1 as f64 / grid_dims.1 as f64);
            rescale_per_pixel_calibration(&mut header, ratio);
        }
        None => {}
    }
    if let Some(reference) = reference {
        let grid_cards = filter_header(reference, true, false).map(|wcs| wcs.cards).unwrap_or_default();
        let physical = PHYSICAL_CARDS.iter().filter_map(|&k| reference.get(k).map(|v| (k.to_string(), v.to_string())));
        for (key, value) in grid_cards.into_iter().chain(physical) {
            header.set(&key, value);
        }
    }
    derived_output_header(Some(&header), ABPROC_ALIGNED, OutputValues::Linear)
}

fn aligned_output_path(out_dir: &str, src_path: &str, label: &str) -> String {
    format!("{}/{}_{}_aligned.fits", out_dir, output_stem(src_path), label)
}

#[tauri::command]
pub async fn export_aligned_channels_cmd(
    r_path: Option<String>,
    g_path: Option<String>,
    b_path: Option<String>,
    output_dir: String,
    align_method: Option<String>,
    copy_wcs: Option<bool>,
    copy_metadata: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let out_dir = resolve_output_dir(&output_dir)?;

        if r_path.is_none() && g_path.is_none() && b_path.is_none() {
            anyhow::bail!("No channel to export: assign at least one of R, G or B.");
        }

        let r_entry = load_entry(&r_path)?;
        let g_entry = load_entry(&g_path)?;
        let b_entry = load_entry(&b_path)?;

        let r_ref = r_entry.as_ref().map(|e| e.arr());
        let g_ref = g_entry.as_ref().map(|e| e.arr());
        let b_ref = b_entry.as_ref().map(|e| e.arr());
        let original_dims = [r_ref, g_ref, b_ref].map(|a| a.map(|a| a.dim()));

        let (r_harm, g_harm, b_harm, rows, cols, _info) =
            harmonize_dimensions(r_ref, g_ref, b_ref, MAX_DIMENSION_RATIO)?;

        let rh = r_harm.as_ref().or(r_ref);
        let gh = g_harm.as_ref().or(g_ref);
        let bh = b_harm.as_ref().or(b_ref);

        let method = helpers::parse_align_method_checked(align_method.as_deref())?;

        let (r_aligned, g_aligned, b_aligned, off_g, off_b) =
            align_channels(rh, gh, bh, rows, cols, method)?;

        let do_wcs = copy_wcs.unwrap_or(true);
        let do_meta = copy_metadata.unwrap_or(true);

        let mut exported = Vec::new();

        let channels = [
            ("R", &r_aligned, &r_path, (0.0, 0.0), original_dims[0]),
            ("G", &g_aligned, &g_path, off_g, original_dims[1]),
            ("B", &b_aligned, &b_path, off_b, original_dims[2]),
        ];

        for (label, data, src_path, offset, dims) in &channels {
            let (Some(src_path), Some(dims)) = (src_path.as_deref(), *dims) else {
                continue;
            };
            let out_path = aligned_output_path(&out_dir, src_path, label);

            let hdr = cached_header(src_path).ok().and_then(|source| {
                aligned_channel_header(&source, dims, (rows, cols), *offset, do_wcs, do_meta)
            });
            write_derived_fits(&out_path, data, hdr.as_ref())?;

            let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
            exported.push(json!({
                RES_CHANNEL: label,
                RES_PATH: out_path,
                RES_FILE_SIZE_BYTES: size,
                RES_OFFSET: [offset.0, offset.1],
            }));
        }

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            CHANNELS: exported,
            ALIGN_METHOD: helpers::align_method_str(method),
            DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

#[tauri::command]
pub async fn blend_channels_cmd(
    channel_paths: Vec<String>,
    weights: Vec<serde_json::Value>,
    output_dir: String,
    preset: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        if channel_paths.is_empty() {
            anyhow::bail!("No channel paths provided");
        }

        let entries: Vec<_> = channel_paths
            .iter()
            .map(|p| load_from_cache_or_disk(p))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let dims: Vec<(usize, usize)> = entries.iter().map(|e| e.arr().dim()).collect();
        let max_rows = dims.iter().map(|d| d.0).max().unwrap();
        let max_cols = dims.iter().map(|d| d.1).max().unwrap();

        let needs_resample = dims.iter().any(|&(r, c)| r != max_rows || c != max_cols);

        let arrays: Vec<std::borrow::Cow<Array2<f32>>> = if needs_resample {
            entries
                .iter()
                .map(|e| {
                    let arr = e.arr();
                    let (r, c) = arr.dim();
                    if r != max_rows || c != max_cols {
                        Ok(std::borrow::Cow::Owned(resample_image(arr, max_rows, max_cols)?))
                    } else {
                        Ok(std::borrow::Cow::Borrowed(arr))
                    }
                })
                .collect::<anyhow::Result<Vec<_>>>()?
        } else {
            entries.iter().map(|e| std::borrow::Cow::Borrowed(e.arr())).collect()
        };

        let refs: Vec<&Array2<f32>> = arrays.iter().map(|a| a.as_ref()).collect();

        let blend_weights: Vec<BlendWeight> = weights
            .iter()
            .filter_map(|w| {
                Some(BlendWeight {
                    channel_idx: w.get("channelIdx")?.as_u64()? as usize,
                    r_weight: w.get("r")?.as_f64()?,
                    g_weight: w.get("g")?.as_f64()?,
                    b_weight: w.get("b")?.as_f64()?,
                    offset: w.get("offset").and_then(|v| v.as_f64()).unwrap_or(0.0),
                })
            })
            .collect();

        if blend_weights.len() < weights.len() {
            log::warn!(
                "blend: dropped {} malformed weight(s) of {}",
                weights.len() - blend_weights.len(),
                weights.len()
            );
        }

        let (r, g, b) = blend_channels(&refs, &blend_weights, max_rows, max_cols)?;

        let (stats_r, (stats_g, stats_b)) = rayon::join(
            || compute_image_stats(&r),
            || rayon::join(
                || compute_image_stats(&g),
                || compute_image_stats(&b),
            ),
        );

        let unlinked = stf_should_unlink([&stats_r, &stats_g, &stats_b]);
        let stf_note = unlinked.then(|| stf_unlinked_note([stats_r.median, stats_g.median, stats_b.median]));

        helpers::insert_composite_and_orig(r, g, b, stats_r.clone(), stats_g.clone(), stats_b.clone());

        let png_path = composite_png_path(&output_dir);
        let (er, eg, eb) = helpers::load_composite_rgb()?;

        let stf_config = AutoStfConfig::default();
        let (linked_stf, combined_stats) =
            helpers::compute_linked_stf_with_stats(er.stats(), eg.stats(), eb.stats(), &stf_config);
        let (stf_r, stf_g, stf_b) = if unlinked {
            (auto_stf(er.stats(), &stf_config), auto_stf(eg.stats(), &stf_config), auto_stf(eb.stats(), &stf_config))
        } else {
            (linked_stf, linked_stf, linked_stf)
        };
        let (norm_r, norm_g, norm_b) = if unlinked {
            (er.stats(), eg.stats(), eb.stats())
        } else {
            (&combined_stats, &combined_stats, &combined_stats)
        };
        let fn_r = make_stf_u8_fn(&stf_r, norm_r);
        let fn_g = make_stf_u8_fn(&stf_g, norm_g);
        let fn_b = make_stf_u8_fn(&stf_b, norm_b);
        helpers::render_rgb_preview_with_stf(er.arr(), eg.arr(), eb.arr(), fn_r, fn_g, fn_b, &png_path, MAX_PREVIEW_DIM)?;

        let stf_json = helpers::stf_json(&linked_stf);

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_DIMENSIONS: [max_cols, max_rows],
            RES_CHANNEL_COUNT: channel_paths.len(),
            RES_BLEND_PRESET: preset.unwrap_or_default(),
            RES_STATS_R: { RES_MEDIAN: stats_r.median, RES_MEAN: stats_r.mean, RES_MIN: stats_r.min, RES_MAX: stats_r.max },
            RES_STATS_G: { RES_MEDIAN: stats_g.median, RES_MEAN: stats_g.mean, RES_MIN: stats_g.min, RES_MAX: stats_g.max },
            RES_STATS_B: { RES_MEDIAN: stats_b.median, RES_MEAN: stats_b.mean, RES_MIN: stats_b.min, RES_MAX: stats_b.max },
            RES_AUTO_STF: stf_json,
            RES_STF_LINKED: !unlinked,
            STF_R: helpers::stf_json(&stf_r),
            STF_G: helpers::stf_json(&stf_g),
            STF_B: helpers::stf_json(&stf_b),
            RES_STF_NOTE: stf_note,
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

static LATEST_WIZARD_RUN: Mutex<Option<String>> = Mutex::new(None);

const REFERENCE_RULE_FINEST_WCS: &str = "finest_wcs";
const REFERENCE_RULE_SELECTED: &str = "selected";
const REFERENCE_RULE_FIRST: &str = "first";

pub(crate) fn latest_wizard_run() -> std::sync::MutexGuard<'static, Option<String>> {
    LATEST_WIZARD_RUN.lock().unwrap_or_else(|e| e.into_inner())
}

fn choose_reference(
    paths: &[String],
    headers: &[Option<HduHeader>],
    requested: Option<usize>,
) -> anyhow::Result<(usize, &'static str)> {
    if let Some(i) = requested {
        if i >= headers.len() {
            anyhow::bail!("Reference index {} is out of range for {} channels", i, headers.len());
        }
        return Ok((i, REFERENCE_RULE_SELECTED));
    }
    let scales: Vec<Option<f64>> = headers
        .iter()
        .zip(paths)
        .map(|(h, path)| {
            h.as_ref()
                .and_then(|h| load_wcs(path, h).ok())
                .map(|w| w.pixel_scale_arcsec())
                .filter(|s| s.is_finite() && *s > 0.0)
        })
        .collect();
    let Some(min_scale) = scales.iter().flatten().copied().reduce(f64::min) else {
        return Ok((0, REFERENCE_RULE_FIRST));
    };
    let idx = scales
        .iter()
        .position(|s| s.is_some_and(|s| s <= min_scale * (1.0 + REF_TIE_TOLERANCE)))
        .unwrap_or(0);
    Ok((idx, REFERENCE_RULE_FINEST_WCS))
}

fn run_aligned_key(token: Option<&str>, bin_id: &str) -> String {
    crate::types::constants::wizard_aligned_key_for(token, bin_id)
}

fn claim_wizard_run(token: &str) {
    *latest_wizard_run() = Some(token.to_string());
}

fn start_wizard_run(token: Option<&str>) {
    use crate::types::constants::{wizard_run_prefix, WIZARD_CACHE_PREFIX};
    match token {
        Some(t) => {
            if latest_wizard_run().as_deref() == Some(t) {
                GLOBAL_IMAGE_CACHE.remove_prefix_except(WIZARD_CACHE_PREFIX, &wizard_run_prefix(t));
            }
        }
        None => GLOBAL_IMAGE_CACHE.remove_prefix(WIZARD_CACHE_PREFIX),
    }
}

pub(crate) fn settle_wizard_run(token: Option<&str>) {
    let Some(t) = token else {
        return;
    };
    let superseded = latest_wizard_run().as_deref() != Some(t);
    if superseded {
        GLOBAL_IMAGE_CACHE.remove_prefix(&crate::types::constants::wizard_run_prefix(t));
    }
}

const ALIGN_METHOD_WCS: &str = "wcs";
const WCS_RESIDUAL_WARN_PX: f64 = 5.0;
const WCS_RESIDUAL_FLOOR_PX: f64 = 1.0;

fn wcs_residual_warning(channel: &str, px: f64) -> String {
    format!("{channel}: the WCS of this channel and the reference disagree by {px:.1} px after reprojection; the stored channel is shifted by that residual, but check the plate solutions before trusting the overlay.")
}

fn no_overlap_warning(channel: &str) -> String {
    format!("{channel}: its WCS footprint does not overlap the reference grid; the channel is empty after reprojection.")
}

#[derive(Clone, Copy)]
struct WcsGeometry {
    scale_ratio: f64,
    rotation_deg: f64,
    area_ratio: f64,
    prefilter_k: usize,
    wcs_kind: WcsKind,
}

struct WcsReprojection {
    image: Array2<f32>,
    coverage: f64,
    geometry: WcsGeometry,
}

struct RegisteredChannel {
    result: AlignPairResult,
    header: HduHeader,
    geometry: Option<WcsGeometry>,
    residual_measured: bool,
    warnings: Vec<String>,
}

fn reproject_channel(
    target: &Array2<f32>,
    target_path: &str,
    target_header: Option<&HduHeader>,
    reference_path: &str,
    reference_header: Option<&HduHeader>,
    rows: usize,
    cols: usize,
) -> Option<WcsReprojection> {
    let target_header = target_header?;
    let reference_wcs = load_wcs(reference_path, reference_header?).ok()?;
    let target_wcs = load_wcs(target_path, target_header).ok()?;
    let ratio = reference_wcs.pixel_scale_arcsec() / target_wcs.pixel_scale_arcsec();
    let reduced = prefilter_coarse_target(target, target_header, ratio)
        .and_then(|(arr, header, k)| WcsTransform::from_header(&header).ok().map(|wcs| (arr, wcs, k)));
    let (image, report) = match &reduced {
        Some((arr, wcs, _)) => reproject_to_grid(arr, wcs, &reference_wcs, rows, cols),
        None => reproject_to_grid(target, &target_wcs, &reference_wcs, rows, cols),
    };
    Some(WcsReprojection {
        image,
        coverage: report.coverage,
        geometry: WcsGeometry {
            scale_ratio: target_wcs.pixel_scale_arcsec() / reference_wcs.pixel_scale_arcsec(),
            rotation_deg: report.rotation_deg,
            area_ratio: pixel_area_ratio(&reference_wcs, &target_wcs),
            prefilter_k: reduced.map_or(1, |(_, _, k)| k),
            wcs_kind: report.wcs_kind,
        },
    })
}

fn wcs_only_result(image: Array2<f32>, registered: bool, confidence: f64) -> AlignPairResult {
    AlignPairResult {
        aligned: image,
        offset: (0.0, 0.0),
        center_offset: (0.0, 0.0),
        registered,
        confidence,
        method_used: ALIGN_METHOD_WCS.to_string(),
        matched_stars: 0,
        inliers: 0,
        residual_px: 0.0,
    }
}

fn verified_residual(reference: &Array2<f32>, reprojection: Array2<f32>, measured: AlignPairResult) -> (AlignPairResult, bool) {
    if measured.method_used != ALIGN_METHOD_PHASE {
        return (measured, true);
    }
    let Some(stars) = point_source_residual(reference, &reprojection, &[(0.0, 0.0), measured.offset]) else {
        return (wcs_only_result(reprojection, true, measured.confidence), false);
    };
    let offset = (stars.dy, stars.dx);
    let applied = offset.0.hypot(offset.1) > WCS_RESIDUAL_FLOOR_PX;
    let result = AlignPairResult {
        aligned: if applied { shift_image_subpixel(&reprojection, offset.0, offset.1) } else { reprojection },
        offset,
        center_offset: offset,
        registered: true,
        confidence: measured.confidence,
        method_used: if applied { measured.method_used } else { ALIGN_METHOD_WCS.to_string() },
        matched_stars: stars.matched,
        inliers: stars.agreeing,
        residual_px: stars.rms_px,
    };
    (result, true)
}

#[allow(clippy::too_many_arguments)]
fn register_channel(
    reference: &Array2<f32>,
    reference_path: &str,
    reference_header: Option<&HduHeader>,
    target: &Array2<f32>,
    target_path: &str,
    target_header: Option<&HduHeader>,
    method: AlignMethod,
    label: &str,
) -> anyhow::Result<RegisteredChannel> {
    let (rows, cols) = reference.dim();
    let (tr, tc) = target.dim();
    let Some(reprojection) = reproject_channel(target, target_path, target_header, reference_path, reference_header, rows, cols) else {
        let resized = if (tr, tc) != (rows, cols) { resample_image(target, rows, cols)? } else { target.to_owned() };
        let result = align_pair_with_label(reference, &resized, method, rows, cols, label)?;
        let header = registered_channel_header(target_header, reference_header, (tr, tc), (rows, cols), result.registered, None);
        return Ok(RegisteredChannel { result, header, geometry: None, residual_measured: false, warnings: Vec::new() });
    };
    let geometry = reprojection.geometry;
    let header = registered_channel_header(target_header, reference_header, (tr, tc), (rows, cols), true, Some(geometry.area_ratio));
    let mut warnings = Vec::new();
    let overlaps = reprojection.coverage > 0.0;
    if !overlaps {
        warnings.push(no_overlap_warning(label));
    }
    let measured = overlaps
        .then(|| align_pair_with_label(reference, &reprojection.image, method, rows, cols, label))
        .transpose()?;
    let (result, residual_measured) = match measured {
        Some(result) if result.registered => verified_residual(reference, reprojection.image, result),
        other => (wcs_only_result(reprojection.image, overlaps, other.map_or(0.0, |r| r.confidence)), false),
    };
    let px = result.offset.0.hypot(result.offset.1);
    if residual_measured && px > WCS_RESIDUAL_WARN_PX {
        warnings.push(wcs_residual_warning(label, px));
    }
    Ok(RegisteredChannel { result, header, geometry: Some(geometry), residual_measured, warnings })
}

fn channel_json(channel: &RegisteredChannel, cache_key: &str) -> serde_json::Value {
    let r = &channel.result;
    let g = channel.geometry;
    json!({
        RES_OFFSET: [r.offset.0, r.offset.1],
        RES_REGISTERED: r.registered,
        RES_CONFIDENCE: r.confidence,
        RES_METHOD_USED: r.method_used,
        RES_MATCHED_STARS: r.matched_stars,
        RES_INLIERS: r.inliers,
        RES_RESIDUAL_PX: r.residual_px,
        RES_CACHE_KEY: cache_key,
        RES_REPROJECTED: g.is_some(),
        RES_WCS_SCALE_RATIO: g.map(|g| g.scale_ratio),
        RES_WCS_ROTATION_DEG: g.map(|g| g.rotation_deg),
        RES_PREFILTER_K: g.map(|g| g.prefilter_k),
        RES_WCS_KIND: g.map(|g| g.wcs_kind),
        RES_RESIDUAL_MEASURED: channel.residual_measured,
    })
}

#[cfg(test)]
static BEFORE_RUN_SETTLES: Mutex<Option<fn()>> = Mutex::new(None);

#[cfg(test)]
fn before_run_settles() {
    if let Some(hook) = BEFORE_RUN_SETTLES.lock().unwrap_or_else(|e| e.into_inner()).take() {
        hook();
    }
}

#[cfg(not(test))]
fn before_run_settles() {}

#[tauri::command]
pub async fn align_channels_cmd(
    paths: Vec<String>,
    output_dir: String,
    align_method: Option<String>,
    bin_ids: Option<Vec<String>>,
    persist_to_disk: Option<bool>,
    reference_index: Option<usize>,
    run_token: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let write_disk = persist_to_disk.unwrap_or(false);
        if write_disk {
            resolve_output_dir(&output_dir)?;
        }

        if paths.len() < 2 {
            anyhow::bail!("Need at least 2 channels to align");
        }
        let method = helpers::parse_align_method_checked(align_method.as_deref())?;

        let token = run_token.as_deref().map(str::trim).filter(|t| !t.is_empty());
        let use_bin_ids = bin_ids.as_ref().map(|ids| ids.len() == paths.len()).unwrap_or(false);
        let wizard_token = if use_bin_ids { token } else { None };
        if let Some(t) = wizard_token {
            claim_wizard_run(t);
        }

        let entries: Vec<_> = paths
            .iter()
            .map(|p| load_from_cache_or_disk(p))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let headers: Vec<Option<HduHeader>> =
            paths.iter().zip(&entries).map(|(p, e)| source_header(p, e)).collect();
        let (ref_idx, reference_rule) = choose_reference(&paths, &headers, reference_index)?;

        let ref_arr = entries[ref_idx].arr();
        let (rows, cols) = ref_arr.dim();

        if use_bin_ids {
            start_wizard_run(wizard_token);
        }

        let mut channel_results = Vec::with_capacity(entries.len());
        let mut warnings: Vec<String> = Vec::new();

        let ref_header = headers[ref_idx].as_ref();
        let header0 = derived_output_header(ref_header, ABPROC_ALIGNED, OutputValues::Linear);

        for (i, entry) in entries.iter().enumerate() {
            let cache_key = if use_bin_ids { run_aligned_key(wizard_token, &bin_ids.as_ref().unwrap()[i]) } else { String::new() };
            if i == ref_idx {
                if use_bin_ids {
                    let stats = entry.stats().clone();
                    GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(&cache_key, entry.data_arc(), stats, Some(header0.clone()));
                }
                let mut entry_json = json!({
                    RES_OFFSET: [0.0, 0.0],
                    RES_REGISTERED: true,
                    RES_CACHE_KEY: cache_key,
                    RES_REPROJECTED: false,
                    RES_WCS_SCALE_RATIO: null,
                    RES_WCS_ROTATION_DEG: null,
                    RES_PREFILTER_K: null,
                    RES_WCS_KIND: null,
                    RES_RESIDUAL_MEASURED: false,
                });
                if write_disk {
                    let out0 = format!("{}/{}_aligned.fits", output_dir, output_stem(&paths[i]));
                    write_derived_fits(&out0, ref_arr, Some(&header0))?;
                    entry_json.as_object_mut().unwrap().insert(RES_PATH.to_string(), json!(out0));
                }
                channel_results.push(entry_json);
                continue;
            }

            let label = output_stem(&paths[i]);
            let channel =
                register_channel(ref_arr, &paths[ref_idx], ref_header, entry.arr(), &paths[i], headers[i].as_ref(), method, &label)?;
            let mut entry_json = channel_json(&channel, &cache_key);
            let RegisteredChannel { result, header, warnings: channel_warnings, .. } = channel;
            warnings.extend(channel_warnings);
            let aligned = std::sync::Arc::new(result.aligned);

            if use_bin_ids {
                let stats = compute_image_stats(&aligned);
                GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(&cache_key, aligned.clone(), stats, Some(header.clone()));
            }

            if write_disk {
                let out_path = format!("{}/{}_aligned.fits", output_dir, label);
                write_derived_fits(&out_path, &aligned, Some(&header))?;
                entry_json.as_object_mut().unwrap().insert(RES_PATH.to_string(), json!(out_path));
            }

            channel_results.push(entry_json);
        }

        if wizard_token.is_some() {
            before_run_settles();
            settle_wizard_run(wizard_token);
        }

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            CHANNELS: channel_results,
            ALIGN_METHOD: helpers::align_method_str(method),
            DIMENSIONS: [cols, rows],
            RES_REFERENCE_INDEX: ref_idx,
            RES_REFERENCE_RULE: reference_rule,
            RES_RUN_TOKEN: token,
            RES_WARNINGS: warnings,
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::load_cached_full;
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::constants::ALIGN_METHOD_AFFINE;

    fn blobs(size: usize, scale: f64) -> Array2<f32> {
        Array2::from_shape_fn((size, size), |(y, x)| {
            let (yf, xf) = (y as f64 / scale, x as f64 / scale);
            let blob = |cy: f64, cx: f64| 1000.0 * (-((yf - cy).powi(2) + (xf - cx).powi(2)) / 4.0).exp();
            (10.0 + blob(10.0, 20.0) + blob(22.0, 8.0) + blob(25.0, 25.0)) as f32
        })
    }

    fn wcs_header(crpix: f64, cd: f64, pixar_sr: f64) -> HduHeader {
        let mut h = HduHeader::empty();
        h.set("CTYPE1", "RA---TAN".to_string());
        h.set("CTYPE2", "DEC--TAN".to_string());
        h.set_f64("CRVAL1", 150.0);
        h.set_f64("CRVAL2", 2.0);
        h.set_f64("CRPIX1", crpix);
        h.set_f64("CRPIX2", crpix);
        h.set_f64("CD1_1", -cd);
        h.set_f64("CD2_2", cd);
        h.set_f64("PIXAR_SR", pixar_sr);
        h.set("EXTNAME", "SCI".to_string());
        h
    }

    #[test]
    fn an_upsampled_channel_gets_the_wcs_and_pixel_area_of_its_new_grid() {
        let source = wcs_header(10.0, 1e-5, 4e-14);
        let header = aligned_channel_header(&source, (16, 16), (32, 32), (0.0, 0.0), true, true).unwrap();
        assert!((header.get_f64("CD1_1").unwrap() + 0.5e-5).abs() < 1e-18, "CD kept the native pixel scale");
        assert!((header.get_f64("CD2_2").unwrap() - 0.5e-5).abs() < 1e-18);
        assert!((header.get_f64("CRPIX1").unwrap() - 19.5).abs() < 1e-9, "CRPIX not moved to the new grid");
        assert!((header.get_f64("PIXAR_SR").unwrap() - 1e-14).abs() < 1e-24, "PIXAR_SR kept the native pixel area");
        assert!(header.get("EXTNAME").is_none(), "structural cards of the source HDU were copied");
        assert_eq!(header.get(crate::cmd::common::HEADER_ABPROC), Some(ABPROC_ALIGNED));

        let shifted = aligned_channel_header(&source, (32, 32), (32, 32), (1.5, -2.0), true, true).unwrap();
        assert!((shifted.get_f64("CRPIX1").unwrap() - 12.0).abs() < 1e-9);
        assert!((shifted.get_f64("CRPIX2").unwrap() - 8.5).abs() < 1e-9);
        assert!((shifted.get_f64("CD1_1").unwrap() + 1e-5).abs() < 1e-18);
        assert!(aligned_channel_header(&source, (16, 16), (32, 32), (0.0, 0.0), false, false).is_none());
    }

    fn close(actual: Option<f64>, expected: f64) -> bool {
        actual.is_some_and(|v| (v - expected).abs() <= expected.abs() * 1e-9)
    }

    fn calibrated_header() -> HduHeader {
        let mut h = wcs_header(10.0, 1e-5, 4e-14);
        h.set_f64("PHOTFLAM", 1e-19);
        h.set_f64("PHOTFNU", 4e-7);
        h.set_f64("PHOTMJSR", 1.5);
        h.set_f64("MAGZERO", 25.0);
        h.set_f64("ZPT", 30.0);
        h.set_f64("ZP", 26.0);
        h.set_f64("ZPTMAG", 24.0);
        h.set_f64(ROMAN_PIXEL_AREA_SR_KEYS[0], 2e-13);
        h.set_f64("SATURATE", 60000.0);
        h
    }

    #[test]
    fn an_upsampled_channel_gets_the_photometric_calibration_of_its_new_grid() {
        let header = aligned_channel_header(&calibrated_header(), (16, 16), (32, 32), (0.0, 0.0), true, true).unwrap();
        let shift = 2.5 * 4f64.log10();
        assert!(close(header.get_f64("PHOTFLAM"), 2.5e-20), "PHOTFLAM {:?}", header.get("PHOTFLAM"));
        assert!(close(header.get_f64("PHOTFNU"), 1e-7), "PHOTFNU {:?}", header.get("PHOTFNU"));
        assert!(close(header.get_f64(ROMAN_PIXEL_AREA_SR_KEYS[0]), 5e-14));
        assert!(close(header.get_f64("MAGZERO"), 25.0 + shift), "MAGZERO {:?}", header.get("MAGZERO"));
        assert!(close(header.get_f64("ZPT"), 30.0 + shift));
        assert!(close(header.get_f64("ZP"), 26.0 + shift), "ZP {:?}", header.get("ZP"));
        assert!(close(header.get_f64("ZPTMAG"), 24.0 + shift), "ZPTMAG {:?}", header.get("ZPTMAG"));
        assert!(close(header.get_f64("PHOTMJSR"), 1.5), "a surface-brightness conversion does not depend on the pixel size");
        assert!(header.get("SATURATE").is_none(), "an interpolated pixel no longer saturates at the source level");

        let shifted = aligned_channel_header(&calibrated_header(), (32, 32), (32, 32), (1.5, -2.0), true, true).unwrap();
        assert!(close(shifted.get_f64("PHOTFLAM"), 1e-19));
        assert!(close(shifted.get_f64("MAGZERO"), 25.0));
        assert!(close(shifted.get_f64("SATURATE"), 60000.0));
    }

    fn sip_header() -> HduHeader {
        let mut h = wcs_header(100.0, 1e-4, 4e-14);
        h.set("CTYPE1", "RA---TAN-SIP".to_string());
        h.set("CTYPE2", "DEC--TAN-SIP".to_string());
        h.set("A_ORDER", "2".to_string());
        h.set("B_ORDER", "2".to_string());
        h.set_f64("A_2_0", 2e-5);
        h.set_f64("A_1_1", -1e-5);
        h.set_f64("B_0_2", 3e-5);
        h.set_f64("A_DMAX", 1.5);
        h
    }

    #[test]
    fn rescaling_to_a_new_grid_keeps_the_sip_distortion() {
        let mut source = sip_header();
        source.set("NAXIS1", "200".to_string());
        source.set("NAXIS2", "200".to_string());
        let mut header = source.clone();
        rescale_header_to_grid(&mut header, (200, 200), (400, 400));
        header.set("NAXIS1", "400".to_string());
        header.set("NAXIS2", "400".to_string());
        assert_eq!(header.get("CTYPE1"), Some("RA---TAN-SIP"));
        assert_eq!(header.get("A_ORDER"), Some("2"));
        assert!(close(header.get_f64("A_2_0"), 1e-5), "A_2_0 {:?}", header.get("A_2_0"));
        assert!(close(header.get_f64("A_1_1"), -5e-6), "A_1_1 {:?}", header.get("A_1_1"));
        assert!(close(header.get_f64("B_0_2"), 1.5e-5), "B_0_2 {:?}", header.get("B_0_2"));
        assert!(close(header.get_f64("A_DMAX"), 3.0), "A_DMAX {:?}", header.get("A_DMAX"));

        let before = WcsTransform::from_header(&source).unwrap();
        let after = WcsTransform::from_header(&header).unwrap();
        for (x, y) in [(10.0, 20.0), (150.0, 30.0), (190.0, 185.0)] {
            let a = before.pixel_to_world(x, y);
            let b = after.pixel_to_world(2.0 * x + 0.5, 2.0 * y + 0.5);
            assert!((a.ra - b.ra).abs() < 1e-9 && (a.dec - b.dec).abs() < 1e-9, "({x},{y}): {a:?} vs {b:?}");
        }
    }

    #[test]
    fn a_registered_channel_takes_the_reference_grid_and_keeps_its_own_metadata() {
        let mut reference = wcs_header(16.0, 1e-5, 1e-14);
        reference.set("FILTER", "F444W".to_string());
        reference.set_f64("LTV1", -4.0);
        let mut target = calibrated_header();
        target.set("FILTER", "F200W".to_string());
        target.set_f64("CRVAL1", 151.0);
        target.set_f64("LTV2", -9.0);

        let header = registered_channel_header(Some(&target), Some(&reference), (16, 16), (32, 32), true, None);
        assert_eq!(header.get("FILTER"), Some("F200W"));
        assert!(close(header.get_f64("CRVAL1"), 150.0), "CRVAL1 {:?}", header.get("CRVAL1"));
        assert!(close(header.get_f64("CRPIX1"), 16.0));
        assert!(close(header.get_f64("CD2_2"), 1e-5));
        assert!(close(header.get_f64("PHOTFLAM"), 2.5e-20), "PHOTFLAM {:?}", header.get("PHOTFLAM"));
        assert!(close(header.get_f64("PIXAR_SR"), 1e-14));
        assert!(close(header.get_f64("LTV1"), -4.0));
        assert!(header.get("LTV2").is_none(), "the physical map of the native grid was kept");
        assert!(header.get("EXTNAME").is_none());
        assert_eq!(header.get(crate::cmd::common::HEADER_ABPROC), Some(ABPROC_ALIGNED));

        let unregistered = registered_channel_header(Some(&target), Some(&reference), (16, 16), (32, 32), false, None);
        assert!(close(unregistered.get_f64("CRVAL1"), 151.0), "an unregistered channel claims the reference WCS");
        assert!(close(unregistered.get_f64("CD2_2"), 0.5e-5));
        assert_eq!(unregistered.get("FILTER"), Some("F200W"));
        assert_eq!(unregistered.get(crate::cmd::common::HEADER_ABPROC), Some(ABPROC_ALIGNED));
    }

    fn star_field(dy: f64, dx: f64) -> Array2<f32> {
        Array2::from_shape_fn((64, 64), |(y, x)| {
            let (yf, xf) = (y as f64 - dy, x as f64 - dx);
            let blob = |cy: f64, cx: f64| 1000.0 * (-((yf - cy).powi(2) + (xf - cx).powi(2)) / 4.0).exp();
            (10.0 + blob(20.0, 30.0) + blob(40.0, 14.0) + blob(45.0, 45.0) + blob(12.0, 50.0)) as f32
        })
    }

    fn card(header: &HduHeader, key: &str) -> Option<String> {
        header.get(key).map(|v| v.trim().trim_matches('\'').trim().to_string())
    }

    #[tokio::test]
    async fn aligned_channels_written_to_disk_carry_the_reference_grid_and_their_own_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let reference_path = dir.path().join("ref.fits").to_str().unwrap().to_string();
        let target_path = dir.path().join("tgt.fits").to_str().unwrap().to_string();
        let mut ref_header = wcs_header(16.0, 1e-5, 1e-14);
        ref_header.set("FILTER", "F444W".to_string());
        ref_header.set("BUNIT", "MJy/sr".to_string());
        let mut tgt_header = wcs_header(18.0, 1e-5, 1e-14);
        tgt_header.set("FILTER", "F200W".to_string());
        write_fits_mono(&reference_path, &star_field(0.0, 0.0), Some(&ref_header)).unwrap();
        write_fits_mono(&target_path, &star_field(2.0, -3.0), Some(&tgt_header)).unwrap();

        let res = align_channels_cmd(
            vec![reference_path, target_path],
            out.to_str().unwrap().to_string(),
            None,
            None,
            Some(true),
            None,
            None,
        )
        .await
        .unwrap();
        let channels = res[CHANNELS].as_array().unwrap();
        assert_eq!(channels[1][RES_METHOD_USED], "phase_correlation", "{res}");
        let headers: Vec<HduHeader> = channels
            .iter()
            .map(|c| load_cached_full(c[RES_PATH].as_str().unwrap()).unwrap().header().cloned().expect("header"))
            .collect();
        for h in &headers {
            assert_eq!(card(h, crate::cmd::common::HEADER_ABPROC).as_deref(), Some(ABPROC_ALIGNED));
            assert!((h.get_f64("CRPIX1").unwrap() - 16.0).abs() < 1e-9, "CRPIX1 {:?}", h.get("CRPIX1"));
            assert!((h.get_f64("CD2_2").unwrap() - 1e-5).abs() < 1e-18);
        }
        assert_eq!(card(&headers[0], "FILTER").as_deref(), Some("F444W"));
        assert_eq!(card(&headers[0], "BUNIT").as_deref(), Some("MJy/sr"));
        assert_eq!(card(&headers[1], "FILTER").as_deref(), Some("F200W"));
    }

    #[tokio::test]
    async fn wizard_aligned_entries_carry_the_header_of_the_reference_grid() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let reference_path = dir.path().join("ref.fits").to_str().unwrap().to_string();
        let target_path = dir.path().join("tgt.fits").to_str().unwrap().to_string();
        let mut ref_header = wcs_header(16.0, 1e-5, 1e-14);
        ref_header.set("FILTER", "F444W".to_string());
        let mut tgt_header = wcs_header(18.0, 1e-5, 1e-14);
        tgt_header.set("FILTER", "F200W".to_string());
        write_fits_mono(&reference_path, &star_field(0.0, 0.0), Some(&ref_header)).unwrap();
        write_fits_mono(&target_path, &star_field(2.0, -3.0), Some(&tgt_header)).unwrap();

        let res = align_channels_cmd(
            vec![reference_path.clone(), target_path],
            dir.path().join("out").to_str().unwrap().to_string(),
            None,
            Some(vec!["align_r".to_string(), "align_g".to_string()]),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let channels = res[CHANNELS].as_array().unwrap();
        assert_eq!(channels[1][RES_METHOD_USED], "phase_correlation", "{res}");
        let keys = [
            crate::types::constants::wizard_aligned_key("align_r"),
            crate::types::constants::wizard_aligned_key("align_g"),
        ];
        assert_eq!(channels[0][RES_CACHE_KEY], keys[0]);
        assert_eq!(channels[1][RES_CACHE_KEY], keys[1]);
        let headers: Vec<Option<HduHeader>> = keys
            .iter()
            .map(|k| GLOBAL_IMAGE_CACHE.get(k).unwrap().header().cloned())
            .collect();
        for k in &keys {
            GLOBAL_IMAGE_CACHE.remove(k);
        }
        assert!(!dir.path().join("out").exists(), "a wizard alignment must not write to the disk");
        let reference_wcs =
            WcsTransform::from_header(load_cached_full(&reference_path).unwrap().header().expect("header")).unwrap();
        let headers: Vec<HduHeader> = headers
            .into_iter()
            .map(|h| h.expect("a wizard channel carries the header of its grid"))
            .collect();
        for h in &headers {
            assert_eq!(card(h, crate::cmd::common::HEADER_ABPROC).as_deref(), Some(ABPROC_ALIGNED));
            assert!((h.get_f64("CRPIX1").unwrap() - 16.0).abs() < 1e-9, "CRPIX1 {:?}", h.get("CRPIX1"));
            assert!((h.get_f64("CD2_2").unwrap() - 1e-5).abs() < 1e-18);
            assert_eq!(h.get_i64("NAXIS1"), Some(64));
            assert_eq!(h.get_i64("NAXIS2"), Some(64));
            let wcs = WcsTransform::from_header(h).unwrap();
            for (x, y) in [(0.0, 0.0), (30.5, 20.25), (63.0, 1.0)] {
                let a = reference_wcs.pixel_to_world(x, y);
                let b = wcs.pixel_to_world(x, y);
                assert!((a.ra - b.ra).abs() < 1e-12 && (a.dec - b.dec).abs() < 1e-12, "({x},{y}): {a:?} vs {b:?}");
            }
        }
        assert_eq!(card(&headers[0], "FILTER").as_deref(), Some("F444W"));
        assert_eq!(card(&headers[1], "FILTER").as_deref(), Some("F200W"));
    }

    #[tokio::test]
    async fn exported_channels_have_distinct_readable_names_and_rescaled_headers() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let (night1, night2) = (dir.path().join("night1"), dir.path().join("night2"));
        std::fs::create_dir_all(&night1).unwrap();
        std::fs::create_dir_all(&night2).unwrap();
        let r = night1.join("img.fits").to_str().unwrap().to_string();
        let g = night2.join("img.fits").to_str().unwrap().to_string();
        let mut r_header = wcs_header(8.0, 2e-5, 4e-14);
        r_header.set_f64("PHOTFLAM", 1e-19);
        r_header.set_f64("MAGZERO", 25.0);
        write_fits_mono(&r, &blobs(16, 0.5), Some(&r_header)).unwrap();
        write_fits_mono(&g, &blobs(32, 1.0), Some(&wcs_header(16.0, 1e-5, 1e-14))).unwrap();

        let res = export_aligned_channels_cmd(
            Some(r.clone()),
            Some(g.clone()),
            Some(format!("{}#hdu=0", g)),
            out.to_str().unwrap().to_string(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let paths: Vec<String> = res[CHANNELS]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c[RES_PATH].as_str().unwrap().to_string())
            .collect();
        assert_eq!(paths.len(), 3);
        assert!(paths[0] != paths[1] && paths[1] != paths[2] && paths[0] != paths[2], "{paths:?}");
        for p in &paths {
            let name = std::path::Path::new(p).file_name().unwrap().to_str().unwrap();
            assert!(!name.contains('#') && !name.contains(".fits_"), "{name}");
            assert!(std::path::Path::new(p).exists());
        }
        assert!(paths[0].ends_with("img_R_aligned.fits"), "{}", paths[0]);

        let written = load_cached_full(&paths[0]).unwrap();
        assert_eq!(written.arr().dim(), (32, 32));
        let header = written.header().expect("header");
        assert!((header.get_f64("CD2_2").unwrap() - 1e-5).abs() < 1e-18, "R claims its native pixel scale");
        assert!((header.get_f64("PIXAR_SR").unwrap() - 1e-14).abs() < 1e-24);
        assert!((header.get_f64("CRPIX1").unwrap() - 15.5).abs() < 1e-9);
        assert!(close(header.get_f64("PHOTFLAM"), 2.5e-20), "PHOTFLAM {:?}", header.get("PHOTFLAM"));
        assert!(close(header.get_f64("MAGZERO"), 25.0 + 2.5 * 4f64.log10()), "MAGZERO {:?}", header.get("MAGZERO"));
    }

    #[tokio::test]
    async fn exporting_without_any_channel_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = export_aligned_channels_cmd(None, None, None, dir.path().to_str().unwrap().to_string(), None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("No channel to export"), "{err}");
    }

    #[tokio::test]
    async fn both_align_commands_refuse_an_unknown_align_method() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.fits").to_str().unwrap().to_string();
        let second = dir.path().join("second.fits").to_str().unwrap().to_string();
        write_fits_mono(&first, &blobs(32, 1.0), None).unwrap();
        write_fits_mono(&second, &blobs(32, 1.0), None).unwrap();
        let out = dir.path().join("out").to_str().unwrap().to_string();

        let err = align_channels_cmd(
            vec![first.clone(), second.clone()],
            out.clone(),
            Some("bogus".to_string()),
            None,
            None,
            None,
            None,
        )
        .await
        .expect_err("align_channels_cmd ran an unknown method as phase correlation");
        assert!(err.contains("unknown align method 'bogus'"), "{err}");

        let err = export_aligned_channels_cmd(Some(first), Some(second), None, out, Some("bogus".to_string()), None, None)
            .await
            .expect_err("export_aligned_channels_cmd ran an unknown method as phase correlation");
        assert!(err.contains("unknown align method 'bogus'"), "{err}");
    }

    fn lone_blob(size: usize, cx: f32) -> Array2<f32> {
        let cy = size as f32 * 0.5;
        Array2::from_shape_fn((size, size), |(y, x)| {
            let (dy, dx) = (y as f32 - cy, x as f32 - cx);
            100.0 + 1000.0 * (-(dy * dy + dx * dx) / 18.0).exp()
        })
    }

    struct Lcg(u64);

    impl Lcg {
        fn uniform(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
        }

        fn gauss(&mut self) -> f64 {
            let u1 = self.uniform().max(1e-12);
            let u2 = self.uniform();
            (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
        }
    }

    fn gaussian_plane(size: usize, median: f64, sigma: f64, seed: u64) -> Array2<f32> {
        let mut rng = Lcg(seed);
        Array2::from_shape_fn((size, size), |_| (median + sigma * rng.gauss()) as f32)
    }

    fn write_plane(dir: &std::path::Path, name: &str, data: &Array2<f32>) -> String {
        let path = dir.join(name).to_str().unwrap().to_string();
        write_fits_mono(&path, data, None).unwrap();
        path
    }

    fn weight_row(idx: usize, r: f64, g: f64, b: f64, offset: f64) -> serde_json::Value {
        json!({ "channelIdx": idx, "r": r, "g": g, "b": b, "offset": offset })
    }

    fn diagonal_weights() -> Vec<serde_json::Value> {
        vec![weight_row(0, 1.0, 0.0, 0.0, 0.0), weight_row(1, 0.0, 1.0, 0.0, 0.0), weight_row(2, 0.0, 0.0, 1.0, 0.0)]
    }

    fn png_channel_means(path: &str) -> [f64; 3] {
        let img = image::open(path).unwrap().to_rgb8();
        let n = (img.width() * img.height()) as f64;
        let mut sums = [0.0f64; 3];
        for px in img.pixels() {
            for c in 0..3 {
                sums[c] += px.0[c] as f64;
            }
        }
        sums.map(|s| s / n)
    }

    fn fake_stats(median: f64, sigma: f64) -> ImageStats {
        ImageStats {
            min: median - 5.0 * sigma,
            max: median + 5.0 * sigma,
            median,
            mad: sigma / 1.4826,
            sigma,
            mean: median,
            valid_count: 4096,
        }
    }

    #[test]
    fn stf_unlinks_on_median_spread_or_ratio() {
        assert!(stf_should_unlink([&fake_stats(5.0, 0.5), &fake_stats(40.0, 0.5), &fake_stats(2.5, 0.5)]), "spread");
        assert!(stf_should_unlink([&fake_stats(10.0, 2.0), &fake_stats(5.0, 2.0), &fake_stats(5.0, 2.0)]), "ratio 2 > 1.5");
        assert!(!stf_should_unlink([&fake_stats(100.0, 2.0), &fake_stats(101.0, 2.0), &fake_stats(99.0, 2.0)]), "one camera");
        assert!(!stf_should_unlink([&fake_stats(0.01, 0.5), &fake_stats(-0.02, 0.5), &fake_stats(0.0, 0.5)]), "BG-subtracted");
        assert!(!stf_should_unlink([&fake_stats(0.0, 0.5), &fake_stats(0.3, 0.5), &fake_stats(0.2, 0.5)]), "zero median: ratio clause off");
        assert_eq!(sig3(5.1880), "5.19");
        assert_eq!(sig3(40.710), "40.7");
        assert_eq!(sig3(2.5893), "2.59");
        assert_eq!(sig3(-13.174), "-13.2");
        assert_eq!(sig3(0.012345), "0.0123");
        assert_eq!(sig3(9.996), "10.0");
    }

    #[tokio::test]
    async fn blend_preview_goes_unlinked_when_channel_medians_differ() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out").to_str().unwrap().to_string();
        let r = write_plane(dir.path(), "r.fits", &gaussian_plane(64, 5.0, 0.5, 11));
        let g = write_plane(dir.path(), "g.fits", &gaussian_plane(64, 40.0, 0.5, 12));
        let b = write_plane(dir.path(), "b.fits", &gaussian_plane(64, 2.5, 0.5, 13));

        let res = blend_channels_cmd(vec![r, g, b], diagonal_weights(), out.clone(), None).await.unwrap();
        assert_eq!(res[RES_STF_LINKED], false, "{res}");
        let note = res[RES_STF_NOTE].as_str().expect("an unlinked blend carries the STF note");
        assert!(note.starts_with("Preview, Adjust and the PNG export use an unlinked auto-STF"), "{note}");
        assert!(note.contains("(R 5.0") && note.contains(" · G 40.0 · B 2.5") && note.contains("); the composite FITS data are unchanged."), "{note}");
        assert!(
            note.ends_with("; the composite FITS data are unchanged. To equalise the channels, turn Match levels on (with one filter per channel) or remove each channel's pedestal with Background › Neutralize before blending."),
            "{note}"
        );
        assert_ne!(res[STF_G][crate::types::constants::RES_SHADOW], res[STF_R][crate::types::constants::RES_SHADOW], "{res}");
        assert_eq!(res[RES_STATS_G][RES_MEDIAN].as_f64().unwrap().round(), 40.0);

        let r2 = write_plane(dir.path(), "r2.fits", &gaussian_plane(64, 5.0, 0.5, 21));
        let g2 = write_plane(dir.path(), "g2.fits", &gaussian_plane(64, 5.2, 0.5, 22));
        let b2 = write_plane(dir.path(), "b2.fits", &gaussian_plane(64, 4.9, 0.5, 23));
        let res = blend_channels_cmd(vec![r2, g2, b2], diagonal_weights(), out, None).await.unwrap();
        assert_eq!(res[RES_STF_LINKED], true, "{res}");
        assert!(res[RES_STF_NOTE].is_null(), "{res}");
        assert_eq!(res[STF_G], res[STF_R], "{res}");
        assert_eq!(res[STF_B], res[RES_AUTO_STF], "{res}");
    }

    #[tokio::test]
    async fn bicolor_matrix_with_match_on_is_unlinked_or_neutral() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out").to_str().unwrap().to_string();
        let a = write_plane(dir.path(), "a.fits", &gaussian_plane(64, 10.0, 0.5, 31));
        let b = write_plane(dir.path(), "b.fits", &gaussian_plane(64, 10.0, 0.5, 32));
        let weights = vec![weight_row(0, 0.0, 0.5, 0.5, 0.0), weight_row(1, 1.0, 0.0, 0.0, 0.0)];

        let res = blend_channels_cmd(vec![a, b], weights, out, None).await.unwrap();

        let medians = [RES_STATS_R, RES_STATS_G, RES_STATS_B].map(|k| res[k][RES_MEDIAN].as_f64().unwrap());
        assert!((medians[0] - 10.0).abs() < 0.1 && (medians[1] - 5.0).abs() < 0.1 && (medians[2] - 5.0).abs() < 0.1, "{medians:?}");
        assert_eq!(res[RES_STF_LINKED], false, "{res}");
        assert!(res[RES_STF_NOTE].as_str().unwrap().contains("(R 10.0 · G 5.00 · B 5.00)"), "{res}");
    }

    #[tokio::test]
    async fn jwst_like_trio_with_level_match_keeps_every_channel() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out").to_str().unwrap().to_string();
        let raw = [(5.1880, 0.66491, 41u64), (40.710, 6.3634, 42), (2.5893, 0.30578, 43)];
        let stars = [(10usize, 10usize), (20, 70), (50, 30), (70, 80), (85, 15), (40, 60)];
        let mut planes: Vec<Array2<f32>> = raw
            .iter()
            .map(|&(median, mad, seed)| gaussian_plane(96, median, 1.4826 * mad, seed))
            .collect();
        for idx in [0usize, 2] {
            let sigma = 1.4826 * raw[idx].1;
            for &(y, x) in &stars {
                planes[idx][[y, x]] = (raw[idx].0 + 50.0 * sigma) as f32;
            }
        }
        let levels: Vec<ImageStats> = planes.iter().map(compute_image_stats).collect();
        let ref_idx = (0..3).max_by(|&a, &b| levels[a].mad.partial_cmp(&levels[b].mad).unwrap()).unwrap();
        assert_eq!(ref_idx, 1, "the reference is the channel of largest MAD");
        let med_ref = levels[ref_idx].median;
        let k: Vec<f64> = levels.iter().map(|l| levels[ref_idx].mad / l.mad).collect();
        let p: Vec<f64> = levels.iter().zip(&k).map(|(l, k)| med_ref / k - l.median).collect();
        assert!((k[0] - 9.57).abs() < 0.5 && (k[2] - 20.81).abs() < 1.5, "{k:?}");
        let paths: Vec<String> = planes
            .iter()
            .enumerate()
            .map(|(i, plane)| write_plane(dir.path(), &format!("ch{i}.fits"), plane))
            .collect();
        let weights = vec![
            weight_row(0, k[0], 0.0, 0.0, p[0]),
            weight_row(1, 0.0, k[1], 0.0, p[1]),
            weight_row(2, 0.0, 0.0, k[2], p[2]),
        ];

        let res = blend_channels_cmd(paths, weights, out, None).await.unwrap();

        let medians = [RES_STATS_R, RES_STATS_G, RES_STATS_B].map(|key| res[key][RES_MEDIAN].as_f64().unwrap());
        let (lo, hi) = (medians.iter().cloned().fold(f64::INFINITY, f64::min), medians.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
        assert!(hi - lo <= 0.01 * hi, "output medians must agree within 1%: {medians:?} ({res})");
        let means = png_channel_means(res[RES_PNG_PATH].as_str().unwrap());
        let (lo, hi) = (means.iter().cloned().fold(f64::INFINITY, f64::min), means.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
        assert!(lo / hi >= 0.5, "preview channel means {means:?} must satisfy min/max >= 0.5");
    }

    fn arcsec_header(crpix: f64, arcsec_per_px: f64) -> HduHeader {
        wcs_header(crpix, arcsec_per_px / 3600.0, 1e-14)
    }

    async fn align(paths: Vec<String>, bin_ids: Option<Vec<&str>>, reference_index: Option<usize>, run_token: Option<&str>) -> serde_json::Value {
        align_channels_cmd(
            paths,
            "unused".to_string(),
            None,
            bin_ids.map(|ids| ids.into_iter().map(str::to_string).collect()),
            None,
            reference_index,
            run_token.map(str::to_string),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn reference_defaults_to_the_finest_pixel_scale_with_a_tie_tolerance() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, data: &Array2<f32>, header: Option<&HduHeader>| {
            let path = dir.path().join(name).to_str().unwrap().to_string();
            write_fits_mono(&path, data, header).unwrap();
            path
        };
        let coarse = write("coarse.fits", &blobs(32, 0.5), Some(&arcsec_header(16.0, 0.2)));
        let fine = write("fine.fits", &blobs(64, 1.0), Some(&arcsec_header(32.0, 0.1)));
        let near = write("near.fits", &star_field(0.0, 0.0), Some(&arcsec_header(32.0, 0.10004)));
        let exact = write("exact.fits", &star_field(2.0, -3.0), Some(&arcsec_header(32.0, 0.1)));
        let plain_a = write("plain_a.fits", &star_field(0.0, 0.0), None);
        let plain_b = write("plain_b.fits", &star_field(2.0, -3.0), None);

        let res = align(vec![coarse.clone(), fine.clone()], None, None, None).await;
        assert_eq!(res[RES_REFERENCE_INDEX], 1, "{res}");
        assert_eq!(res[RES_REFERENCE_RULE], "finest_wcs", "{res}");
        assert_eq!(res[DIMENSIONS], json!([64, 64]), "{res}");
        let channels = res[CHANNELS].as_array().unwrap();
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[1][RES_OFFSET], json!([0.0, 0.0]), "the reference entry stays in input order: {res}");
        assert!(channels[0][RES_METHOD_USED].is_string(), "the non-reference entry was registered: {res}");

        let res = align(vec![near, exact], None, None, None).await;
        assert_eq!(res[RES_REFERENCE_INDEX], 0, "a 0.04% scale difference is a tie that the first bin wins: {res}");
        assert_eq!(res[RES_REFERENCE_RULE], "finest_wcs", "{res}");

        let res = align(vec![coarse.clone(), fine.clone()], None, Some(0), None).await;
        assert_eq!(res[RES_REFERENCE_INDEX], 0, "{res}");
        assert_eq!(res[RES_REFERENCE_RULE], "selected", "{res}");
        assert_eq!(res[DIMENSIONS], json!([32, 32]), "{res}");

        let res = align(vec![plain_a, plain_b], None, None, None).await;
        assert_eq!(res[RES_REFERENCE_INDEX], 0, "{res}");
        assert_eq!(res[RES_REFERENCE_RULE], "first", "{res}");
        assert!(res[RES_RUN_TOKEN].is_null(), "{res}");

        let err = align_channels_cmd(vec![coarse, fine], "unused".to_string(), None, None, None, Some(7), None).await.unwrap_err();
        assert!(err.contains("7"), "{err}");
    }

    fn seed_key(key: &str) {
        let arr = star_field(0.0, 0.0);
        GLOBAL_IMAGE_CACHE.insert_synthetic(key, std::sync::Arc::new(arr.clone()), compute_image_stats(&arr));
    }

    fn write_pair(dir: &std::path::Path) -> (String, String) {
        let reference_path = dir.join("ref.fits").to_str().unwrap().to_string();
        let target_path = dir.join("tgt.fits").to_str().unwrap().to_string();
        write_fits_mono(&reference_path, &star_field(0.0, 0.0), Some(&wcs_header(16.0, 1e-5, 1e-14))).unwrap();
        write_fits_mono(&target_path, &star_field(2.0, -3.0), Some(&wcs_header(16.0, 1e-5, 1e-14))).unwrap();
        (reference_path, target_path)
    }

    #[tokio::test]
    async fn wizard_keys_carry_the_run_token_and_other_runs_keys_are_removed() {
        use crate::types::constants::{wizard_aligned_key_for, WIZARD_CACHE_PREFIX};
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let (reference_path, target_path) = write_pair(dir.path());
        let seeded = ["__wizard_ch_ta_r_aligned", "__wizard_ch_b_aligned", "__wizard_ch_tc_r_aligned"];
        for key in seeded {
            seed_key(key);
        }

        let res = align(vec![reference_path, target_path], Some(vec!["r", "g"]), None, Some("b")).await;

        let keys = [wizard_aligned_key_for(Some("b"), "r"), wizard_aligned_key_for(Some("b"), "g")];
        let present: Vec<bool> = keys.iter().map(|k| GLOBAL_IMAGE_CACHE.contains(k)).collect();
        let seeded_left: Vec<&str> = seeded.iter().copied().filter(|k| GLOBAL_IMAGE_CACHE.contains(k)).collect();
        let other_wizard_keys = GLOBAL_IMAGE_CACHE.any_key(|k| k.starts_with(WIZARD_CACHE_PREFIX) && !k.starts_with("__wizard_ch_tb_"));
        for k in &keys {
            GLOBAL_IMAGE_CACHE.remove(k);
        }
        for k in seeded {
            GLOBAL_IMAGE_CACHE.remove(k);
        }

        assert_eq!(keys[0], "__wizard_ch_tb_r_aligned");
        let channels = res[CHANNELS].as_array().unwrap();
        assert_eq!(channels[0][RES_CACHE_KEY], keys[0], "{res}");
        assert_eq!(channels[1][RES_CACHE_KEY], keys[1], "{res}");
        assert_eq!(res[RES_RUN_TOKEN], "b", "{res}");
        assert_eq!(present, [true, true], "the run's own keys must be in the cache");
        assert!(seeded_left.is_empty(), "keys of other runs and legacy bin keys must be removed: {seeded_left:?}");
        assert!(!other_wizard_keys, "only the tb keys may remain");
    }

    fn simulate_newer_run() {
        *latest_wizard_run() = Some("newer".to_string());
        seed_key("__wizard_ch_tnewer_r_aligned");
    }

    #[tokio::test]
    async fn a_late_run_removes_its_own_keys_when_a_newer_run_started() {
        use crate::types::constants::wizard_aligned_key_for;
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let (reference_path, target_path) = write_pair(dir.path());
        *BEFORE_RUN_SETTLES.lock().unwrap() = Some(simulate_newer_run);

        let res = align(vec![reference_path, target_path], Some(vec!["r", "g"]), None, Some("old")).await;

        let old_keys = [wizard_aligned_key_for(Some("old"), "r"), wizard_aligned_key_for(Some("old"), "g")];
        let old_left: Vec<&String> = old_keys.iter().filter(|k| GLOBAL_IMAGE_CACHE.contains(k)).collect();
        let newer_present = GLOBAL_IMAGE_CACHE.contains("__wizard_ch_tnewer_r_aligned");
        let latest = latest_wizard_run().clone();
        let hook_consumed = BEFORE_RUN_SETTLES.lock().unwrap().is_none();
        for k in &old_keys {
            GLOBAL_IMAGE_CACHE.remove(k);
        }
        GLOBAL_IMAGE_CACHE.remove("__wizard_ch_tnewer_r_aligned");

        assert!(hook_consumed, "the run must reach the point where its keys are written");
        let channels = res[CHANNELS].as_array().unwrap();
        assert_eq!(channels[0][RES_CACHE_KEY], old_keys[0], "{res}");
        assert_eq!(channels[1][RES_CACHE_KEY], old_keys[1], "{res}");
        assert!(old_left.is_empty(), "a run that finishes after a newer one started must remove its own keys: {old_left:?}");
        assert!(newer_present, "the newer run's keys must stay");
        assert_eq!(latest.as_deref(), Some("newer"));
    }

    #[tokio::test]
    async fn an_untokened_run_keeps_its_keys_when_a_tokened_run_is_latest() {
        use crate::types::constants::wizard_aligned_key;
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let (reference_path, target_path) = write_pair(dir.path());
        *latest_wizard_run() = Some("t2".to_string());

        let res = align(vec![reference_path, target_path], Some(vec!["ul_r", "ul_g"]), None, None).await;

        let keys = [wizard_aligned_key("ul_r"), wizard_aligned_key("ul_g")];
        let present: Vec<bool> = keys.iter().map(|k| GLOBAL_IMAGE_CACHE.contains(k)).collect();
        let latest = latest_wizard_run().clone();
        for k in &keys {
            GLOBAL_IMAGE_CACHE.remove(k);
        }

        assert_eq!(res[CHANNELS][0][RES_CACHE_KEY], keys[0], "{res}");
        assert!(res[RES_RUN_TOKEN].is_null(), "{res}");
        assert_eq!(present, [true, true], "a legacy run never applies the late-finisher removal");
        assert_eq!(latest.as_deref(), Some("t2"), "a legacy run never touches LATEST_WIZARD_RUN");
    }

    #[test]
    fn a_superseded_run_does_not_wipe_the_newer_runs_keys_at_start() {
        use crate::types::constants::wizard_aligned_key_for;
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let newer_key = wizard_aligned_key_for(Some("newer"), "r");
        seed_key(&newer_key);
        *latest_wizard_run() = Some("newer".to_string());

        start_wizard_run(Some("older"));

        let newer_present = GLOBAL_IMAGE_CACHE.contains(&newer_key);
        let latest = latest_wizard_run().clone();
        GLOBAL_IMAGE_CACHE.remove(&newer_key);

        assert!(newer_present, "a run that is no longer the latest must not wipe the newer run's keys");
        assert_eq!(latest.as_deref(), Some("newer"), "the start of a run never claims LATEST_WIZARD_RUN");
    }

    #[test]
    fn a_registered_header_uses_the_wcs_area_ratio() {
        let reference = wcs_header(16.0, 1e-5, 1e-14);
        let target = calibrated_header();

        let header = registered_channel_header(Some(&target), Some(&reference), (16, 16), (32, 32), true, Some(0.246403));
        assert!(close(header.get_f64("PHOTFLAM"), 1e-19 * 0.246403), "PHOTFLAM {:?}", header.get("PHOTFLAM"));
        assert!(close(header.get_f64("PIXAR_SR"), 4e-14 * 0.246403), "PIXAR_SR {:?}", header.get("PIXAR_SR"));
        assert!(close(header.get_f64("MAGZERO"), 25.0 - 2.5 * 0.246403f64.log10()), "MAGZERO {:?}", header.get("MAGZERO"));
        assert!(close(header.get_f64("CRPIX1"), 16.0), "the grid WCS is the reference's");

        let by_dims = registered_channel_header(Some(&target), Some(&reference), (16, 16), (32, 32), true, None);
        assert!(close(by_dims.get_f64("PHOTFLAM"), 2.5e-20), "without a WCS ratio the dims rule stays: {:?}", by_dims.get("PHOTFLAM"));
    }

    #[tokio::test]
    async fn each_aligned_channel_says_whether_it_was_registered() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out").to_str().unwrap().to_string();
        let write = |name: &str, data: &Array2<f32>| {
            let path = dir.path().join(name).to_str().unwrap().to_string();
            write_fits_mono(&path, data, None).unwrap();
            path
        };
        let reference = write("ref.fits", &star_field(0.0, 0.0));
        let shifted = write("shifted.fits", &star_field(2.0, -3.0));
        let far_reference = write("far_ref.fits", &lone_blob(300, 50.0));
        let far_target = write("far_tgt.fits", &lone_blob(300, 250.0));

        let res = align_channels_cmd(vec![reference, shifted], out.clone(), None, None, None, None, None).await.unwrap();
        let channels = res[CHANNELS].as_array().unwrap();
        assert_eq!(channels[0][RES_REGISTERED], true, "{res}");
        assert_eq!(channels[1][RES_REGISTERED], true, "{res}");

        let res = align_channels_cmd(vec![far_reference.clone(), far_target.clone()], out.clone(), None, None, None, None, None)
            .await
            .unwrap();
        let channels = res[CHANNELS].as_array().unwrap();
        assert_eq!(channels[1][RES_METHOD_USED], "phase_correlation_identity", "{res}");
        assert_eq!(channels[1][RES_REGISTERED], false, "{res}");
        assert_eq!(channels[1][RES_OFFSET], json!([0.0, 0.0]));

        let res = align_channels_cmd(vec![far_reference, far_target], out, Some(ALIGN_METHOD_AFFINE.to_string()), None, None, None, None)
            .await
            .unwrap();
        let channels = res[CHANNELS].as_array().unwrap();
        assert_eq!(channels[1][RES_METHOD_USED], "identity", "{res}");
        assert_eq!(channels[1][RES_REGISTERED], false, "{res}");
    }

    const ARCSEC_DEG: f64 = 1.0 / 3600.0;
    const FWHM_TO_SIGMA: f64 = 0.424_660_900_144_009_5;
    const STAR_REF_POSITIONS: [(f64, f64); 12] = [
        (90.3, 100.7),
        (420.6, 95.2),
        (95.9, 415.4),
        (410.2, 430.8),
        (256.4, 256.6),
        (150.25, 300.75),
        (330.5, 160.5),
        (200.0, 420.0),
        (380.3, 300.1),
        (120.8, 210.9),
        (300.7, 390.3),
        (240.1, 130.6),
    ];

    fn tan_header(size: usize, crpix: (f64, f64), crval: (f64, f64), cd: [[f64; 2]; 2]) -> HduHeader {
        let mut h = HduHeader::empty();
        h.set("NAXIS1", size.to_string());
        h.set("NAXIS2", size.to_string());
        h.set("CTYPE1", "RA---TAN".to_string());
        h.set("CTYPE2", "DEC--TAN".to_string());
        h.set_f64("CRPIX1", crpix.0);
        h.set_f64("CRPIX2", crpix.1);
        h.set_f64("CRVAL1", crval.0);
        h.set_f64("CRVAL2", crval.1);
        h.set_f64("CD1_1", cd[0][0]);
        h.set_f64("CD1_2", cd[0][1]);
        h.set_f64("CD2_1", cd[1][0]);
        h.set_f64("CD2_2", cd[1][1]);
        h
    }

    fn rotated_cd(scale_arcsec: f64, angle_deg: f64) -> [[f64; 2]; 2] {
        let s = scale_arcsec * ARCSEC_DEG;
        let (sin, cos) = angle_deg.to_radians().sin_cos();
        [[-s * cos, s * sin], [s * sin, s * cos]]
    }

    fn render_stars(size: usize, pedestal: f32, stars: &[(f64, f64)], fwhm_px: f64) -> Array2<f32> {
        let sigma = fwhm_px * FWHM_TO_SIGMA;
        Array2::from_shape_fn((size, size), |(y, x)| {
            let signal: f64 = stars
                .iter()
                .map(|&(sx, sy)| {
                    let d2 = (x as f64 - sx).powi(2) + (y as f64 - sy).powi(2);
                    1000.0 * (-d2 / (2.0 * sigma * sigma)).exp()
                })
                .sum();
            pedestal + signal as f32
        })
    }

    fn stars_mapped(stars: &[(f64, f64)], from: &HduHeader, to: &HduHeader) -> Vec<(f64, f64)> {
        let (from, to) = (WcsTransform::from_header(from).unwrap(), WcsTransform::from_header(to).unwrap());
        stars
            .iter()
            .map(|&(x, y)| {
                let sky = from.pixel_to_world(x, y);
                to.world_to_pixel(sky.ra, sky.dec)
            })
            .collect()
    }

    fn finite_median(img: &Array2<f32>) -> f64 {
        let mut values: Vec<f32> = img.iter().copied().filter(|v| v.is_finite()).collect();
        assert!(!values.is_empty(), "no finite pixel");
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        values[values.len() / 2] as f64
    }

    fn finite_fraction(img: &Array2<f32>) -> f64 {
        img.iter().filter(|v| v.is_finite()).count() as f64 / img.len() as f64
    }

    fn centroid_near(img: &Array2<f32>, start: (f64, f64), pedestal: f64) -> (f64, f64) {
        const HALF: i64 = 6;
        let (rows, cols) = (img.dim().0 as i64, img.dim().1 as i64);
        let mut centre = start;
        for _ in 0..2 {
            let (cx, cy) = (centre.0.round() as i64, centre.1.round() as i64);
            let (mut sum, mut sx, mut sy) = (0.0, 0.0, 0.0);
            for y in (cy - HALF).max(0)..=(cy + HALF).min(rows - 1) {
                for x in (cx - HALF).max(0)..=(cx + HALF).min(cols - 1) {
                    let v = img[(y as usize, x as usize)] as f64;
                    if !v.is_finite() {
                        continue;
                    }
                    let w = (v - pedestal).max(0.0);
                    sum += w;
                    sx += w * x as f64;
                    sy += w * y as f64;
                }
            }
            assert!(sum > 0.0, "no flux around {start:?}");
            centre = (sx / sum, sy / sum);
        }
        centre
    }

    fn worst_star_offset(img: &Array2<f32>, stars: &[(f64, f64)]) -> f64 {
        stars
            .iter()
            .map(|&(x, y)| {
                let (cx, cy) = centroid_near(img, (x, y), 100.0);
                (cx - x).hypot(cy - y)
            })
            .fold(0.0, f64::max)
    }

    fn write_channel(dir: &std::path::Path, name: &str, data: &Array2<f32>, header: &HduHeader) -> String {
        let path = dir.join(name).to_str().unwrap().to_string();
        write_fits_mono(&path, data, Some(header)).unwrap();
        path
    }

    async fn align_to_disk(paths: Vec<String>, out: &std::path::Path, reference_index: Option<usize>) -> serde_json::Value {
        align_to_disk_with(paths, out, reference_index, None).await
    }

    async fn align_to_disk_with(paths: Vec<String>, out: &std::path::Path, reference_index: Option<usize>, method: Option<&str>) -> serde_json::Value {
        align_channels_cmd(paths, out.to_str().unwrap().to_string(), method.map(str::to_string), None, Some(true), reference_index, None)
            .await
            .unwrap()
    }

    fn residual_px(channel: &serde_json::Value) -> f64 {
        let offset = channel[RES_OFFSET].as_array().unwrap();
        offset[0].as_f64().unwrap().hypot(offset[1].as_f64().unwrap())
    }

    #[tokio::test]
    async fn align_channels_reprojects_through_wcs_and_reports_the_residual() {
        let dir = tempfile::tempdir().unwrap();
        let ref_header = tan_header(512, (256.5, 256.5), (83.0, 22.0), rotated_cd(0.1, 0.0));
        let mut tgt_header = tan_header(256, (128.5, 128.5), (83.0, 22.0 + 3.0 * ARCSEC_DEG), rotated_cd(0.2, 0.5));
        tgt_header.set_f64("PIXAR_SR", 4e-14);
        tgt_header.set("BUNIT", "MJy/sr".to_string());
        tgt_header.set("FILTER", "F444W".to_string());
        let tgt_stars = stars_mapped(&STAR_REF_POSITIONS, &ref_header, &tgt_header);
        let reference = write_channel(dir.path(), "ref.fits", &render_stars(512, 100.0, &STAR_REF_POSITIONS, 5.0), &ref_header);
        let target_data = render_stars(256, 100.0, &tgt_stars, 2.5);
        let target = write_channel(dir.path(), "tgt.fits", &target_data, &tgt_header);

        let res = align_to_disk_with(vec![reference.clone(), target.clone()], &dir.path().join("out"), None, Some(ALIGN_METHOD_AFFINE)).await;

        assert_eq!(res[RES_REFERENCE_INDEX], 0, "{res}");
        assert_eq!(res[RES_REFERENCE_RULE], "finest_wcs", "{res}");
        assert_eq!(res[DIMENSIONS], json!([512, 512]), "{res}");
        assert_eq!(res[RES_WARNINGS], json!([]), "{res}");
        let ch = &res[CHANNELS][1];
        assert_eq!(ch[RES_REPROJECTED], true, "{res}");
        assert!((ch[RES_WCS_SCALE_RATIO].as_f64().unwrap() - 2.0).abs() < 1e-6, "{res}");
        assert!((ch[RES_WCS_ROTATION_DEG].as_f64().unwrap() - 0.5).abs() < 1e-6, "{res}");
        assert_eq!(ch[RES_PREFILTER_K], 1, "{res}");
        assert_eq!(ch[RES_RESIDUAL_MEASURED], true, "{res}");
        assert_eq!(ch[RES_REGISTERED], true, "{res}");
        assert_eq!(ch[RES_METHOD_USED], "affine", "{res}");
        assert!(residual_px(ch) < 0.3, "residual {} px: {res}", residual_px(ch));
        let reference_entry = &res[CHANNELS][0];
        assert_eq!(reference_entry[RES_REPROJECTED], false, "{res}");
        assert!(reference_entry[RES_WCS_SCALE_RATIO].is_null() && reference_entry[RES_PREFILTER_K].is_null(), "{res}");

        let written = load_cached_full(ch[RES_PATH].as_str().unwrap()).unwrap();
        assert_eq!(written.arr().dim(), (512, 512));
        let header = written.header().expect("header");
        assert!(close(header.get_f64("CRPIX1"), 256.5), "CRPIX1 {:?}", header.get("CRPIX1"));
        assert!(close(header.get_f64("CD2_2"), 0.1 * ARCSEC_DEG), "CD2_2 {:?}", header.get("CD2_2"));
        assert!((header.get_f64("PIXAR_SR").unwrap() - 1e-14).abs() < 1e-18, "PIXAR_SR {:?}", header.get("PIXAR_SR"));
        assert_eq!(card(header, "BUNIT").as_deref(), Some("MJy/sr"));
        assert_eq!(card(header, "FILTER").as_deref(), Some("F444W"));
        let (out_median, target_median) = (finite_median(written.arr()), finite_median(&target_data));
        assert!((out_median - target_median).abs() < 0.01, "values must not be scaled: {out_median} vs {target_median}");
    }

    #[tokio::test]
    async fn a_sub_pixel_phase_correlation_residual_leaves_the_reprojection_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let ref_header = tan_header(512, (256.5, 256.5), (83.0, 22.0), rotated_cd(0.1, 0.0));
        let tgt_header = tan_header(256, (128.5, 128.5), (83.0, 22.0 + 3.0 * ARCSEC_DEG), rotated_cd(0.2, 0.5));
        let tgt_stars = stars_mapped(&STAR_REF_POSITIONS, &ref_header, &tgt_header);
        let reference = write_channel(dir.path(), "ref.fits", &render_stars(512, 100.0, &STAR_REF_POSITIONS, 5.0), &ref_header);
        let target = write_channel(dir.path(), "tgt.fits", &render_stars(256, 100.0, &tgt_stars, 2.5), &tgt_header);

        let res = align_to_disk(vec![reference, target], &dir.path().join("out"), None).await;

        let ch = &res[CHANNELS][1];
        let written = load_cached_full(ch[RES_PATH].as_str().unwrap()).unwrap();
        let worst = worst_star_offset(written.arr(), &STAR_REF_POSITIONS);
        assert!(worst < 0.1, "the stored channel is {worst:.3} px away from the reference: {res}");
        assert_eq!(res[ALIGN_METHOD], ALIGN_METHOD_PHASE, "{res}");
        assert_eq!(ch[RES_REPROJECTED], true, "{res}");
        assert_eq!(ch[RES_REGISTERED], true, "{res}");
        assert_eq!(ch[RES_METHOD_USED], ALIGN_METHOD_WCS, "{res}");
        assert_eq!(ch[RES_RESIDUAL_MEASURED], true, "the point sources measure the residual: {res}");
        assert!(residual_px(ch) < 0.1, "residual {} px: {res}", residual_px(ch));
        assert_eq!(res[RES_WARNINGS], json!([]), "{res}");
    }

    #[test]
    fn a_phase_correlation_residual_the_point_sources_contradict_leaves_the_reprojection_untouched() {
        let reference = render_stars(512, 100.0, &STAR_REF_POSITIONS, 2.5);
        let reprojection = render_stars(512, 100.0, &STAR_REF_POSITIONS, 5.0);
        let spurious = (-1.694, -1.309);
        let measured = AlignPairResult {
            aligned: shift_image_subpixel(&reprojection, spurious.0, spurious.1),
            offset: spurious,
            center_offset: spurious,
            registered: true,
            confidence: 12.0,
            method_used: ALIGN_METHOD_PHASE.to_string(),
            matched_stars: 0,
            inliers: 0,
            residual_px: 0.0,
        };

        let (result, residual_measured) = verified_residual(&reference, reprojection, measured);

        assert!(residual_measured);
        assert!(result.registered);
        assert_eq!(result.method_used, ALIGN_METHOD_WCS);
        assert!(result.offset.0.hypot(result.offset.1) < 0.05, "offset {:?}", result.offset);
        assert_eq!(result.matched_stars, STAR_REF_POSITIONS.len());
        assert_eq!(result.inliers, STAR_REF_POSITIONS.len());
        let worst = worst_star_offset(&result.aligned, &STAR_REF_POSITIONS);
        assert!(worst < 0.05, "the stored channel is {worst:.3} px away from the reference");
    }

    #[test]
    fn a_phase_correlation_residual_without_point_sources_is_not_applied() {
        let flat = Array2::from_elem((128, 128), 100.0f32);
        let measured = AlignPairResult {
            aligned: Array2::from_elem((128, 128), 7.0f32),
            offset: (2.0, -1.5),
            center_offset: (2.0, -1.5),
            registered: true,
            confidence: 9.0,
            method_used: ALIGN_METHOD_PHASE.to_string(),
            matched_stars: 0,
            inliers: 0,
            residual_px: 0.0,
        };

        let (result, residual_measured) = verified_residual(&flat, flat.clone(), measured);

        assert!(!residual_measured);
        assert_eq!(result.method_used, ALIGN_METHOD_WCS);
        assert_eq!(result.offset, (0.0, 0.0));
        assert_eq!(result.confidence, 9.0);
        assert!(result.aligned.iter().all(|&v| v == 100.0), "the stored channel is the reprojection");
    }

    #[tokio::test]
    async fn a_phase_correlation_residual_above_the_floor_is_applied() {
        let dir = tempfile::tempdir().unwrap();
        let ref_header = tan_header(512, (256.5, 256.5), (83.0, 22.0), rotated_cd(0.1, 0.0));
        let tgt_header = tan_header(256, (128.5, 128.5), (83.0, 22.0 + 3.0 * ARCSEC_DEG), rotated_cd(0.2, 0.5));
        let misplaced: Vec<(f64, f64)> = stars_mapped(&STAR_REF_POSITIONS, &ref_header, &tgt_header).iter().map(|&(x, y)| (x + 1.5, y)).collect();
        let reference = write_channel(dir.path(), "ref.fits", &render_stars(512, 100.0, &STAR_REF_POSITIONS, 5.0), &ref_header);
        let target = write_channel(dir.path(), "tgt.fits", &render_stars(256, 100.0, &misplaced, 2.5), &tgt_header);

        let res = align_to_disk(vec![reference, target], &dir.path().join("out"), None).await;

        let ch = &res[CHANNELS][1];
        assert_eq!(ch[RES_METHOD_USED], ALIGN_METHOD_PHASE, "{res}");
        assert_eq!(ch[RES_RESIDUAL_MEASURED], true, "{res}");
        let px = residual_px(ch);
        assert!(px > WCS_RESIDUAL_FLOOR_PX && px < WCS_RESIDUAL_WARN_PX, "residual {px:.3} px: {res}");
        assert_eq!(res[RES_WARNINGS], json!([]), "{res}");
        let written = load_cached_full(ch[RES_PATH].as_str().unwrap()).unwrap();
        let worst = worst_star_offset(written.arr(), &STAR_REF_POSITIONS);
        assert!(worst < 1.0, "the shifted channel is {worst:.3} px away from the reference: {res}");
    }

    #[tokio::test]
    async fn a_wcs_residual_above_five_px_is_warned_and_applied() {
        let dir = tempfile::tempdir().unwrap();
        let ref_header = tan_header(512, (256.5, 256.5), (83.0, 22.0), rotated_cd(0.1, 0.0));
        let tgt_header = tan_header(256, (128.5, 128.5), (83.0, 22.0 + 3.0 * ARCSEC_DEG), rotated_cd(0.2, 0.5));
        let misplaced: Vec<(f64, f64)> = stars_mapped(&STAR_REF_POSITIONS, &ref_header, &tgt_header).iter().map(|&(x, y)| (x + 4.0, y)).collect();
        let reference = write_channel(dir.path(), "ref.fits", &render_stars(512, 100.0, &STAR_REF_POSITIONS, 5.0), &ref_header);
        let target = write_channel(dir.path(), "tgt.fits", &render_stars(256, 100.0, &misplaced, 2.5), &tgt_header);

        let res = align_to_disk(vec![reference, target], &dir.path().join("out"), None).await;

        let ch = &res[CHANNELS][1];
        assert_eq!(ch[RES_RESIDUAL_MEASURED], true, "{res}");
        let px = residual_px(ch);
        assert!(px > WCS_RESIDUAL_WARN_PX && px < 12.0, "residual {px:.3} px: {res}");
        let warnings = res[RES_WARNINGS].as_array().unwrap();
        assert_eq!(warnings.len(), 1, "{res}");
        let warning = warnings[0].as_str().unwrap();
        let prefix = "tgt: the WCS of this channel and the reference disagree by ";
        let suffix = " px after reprojection; the stored channel is shifted by that residual, but check the plate solutions before trusting the overlay.";
        assert!(warning.starts_with(prefix), "{warning}");
        assert!(warning.ends_with(suffix), "{warning}");
        assert_eq!(&warning[prefix.len()..warning.len() - suffix.len()], format!("{px:.1}"), "{warning}");
        let written = load_cached_full(ch[RES_PATH].as_str().unwrap()).unwrap();
        let worst = worst_star_offset(written.arr(), &STAR_REF_POSITIONS);
        assert!(worst < 1.0, "the shifted channel is {worst:.3} px away from the reference: {res}");
    }

    #[tokio::test]
    async fn selecting_a_coarser_reference_prefilters_the_finer_channel() {
        let dir = tempfile::tempdir().unwrap();
        let coarse_header = tan_header(128, (67.5, 62.5), (83.0, 22.0), rotated_cd(0.2, 0.0));
        let mut fine_header = tan_header(256, (128.5, 128.5), (83.0, 22.0), rotated_cd(0.1, 0.0));
        fine_header.set_f64("PIXAR_SR", 4e-14);
        let fine_stars: Vec<(f64, f64)> = STAR_REF_POSITIONS.iter().map(|&(x, y)| (x / 2.0, y / 2.0)).collect();
        let coarse_stars = stars_mapped(&fine_stars, &fine_header, &coarse_header);
        let coarse = write_channel(dir.path(), "coarse.fits", &render_stars(128, 100.0, &coarse_stars, 1.5), &coarse_header);
        let fine = write_channel(dir.path(), "fine.fits", &render_stars(256, 100.0, &fine_stars, 3.0), &fine_header);

        let res = align_to_disk(vec![coarse.clone(), fine.clone()], &dir.path().join("out"), Some(0)).await;

        assert_eq!(res[RES_REFERENCE_INDEX], 0, "{res}");
        assert_eq!(res[RES_REFERENCE_RULE], "selected", "{res}");
        assert_eq!(res[DIMENSIONS], json!([128, 128]), "{res}");
        let ch = &res[CHANNELS][1];
        assert_eq!(ch[RES_REPROJECTED], true, "{res}");
        assert_eq!(ch[RES_PREFILTER_K], 2, "{res}");
        assert_eq!(ch[RES_WCS_KIND], "header", "{res}");
        assert!(res[CHANNELS][0][RES_WCS_KIND].is_null(), "{res}");
        assert!((ch[RES_WCS_SCALE_RATIO].as_f64().unwrap() - 0.5).abs() < 1e-6, "the scale ratio is the original channel's: {res}");
        assert!(ch[RES_WCS_ROTATION_DEG].as_f64().unwrap().abs() < 1e-9, "{res}");
        let written = load_cached_full(ch[RES_PATH].as_str().unwrap()).unwrap();
        assert_eq!(written.arr().dim(), (128, 128));
        let coverage = finite_fraction(written.arr());
        assert!(coverage > 0.9, "coverage {coverage}");
        let header = written.header().expect("header");
        assert!((header.get_f64("PIXAR_SR").unwrap() - 1.6e-13).abs() < 1e-17, "the area ratio uses the original CD: {:?}", header.get("PIXAR_SR"));
        assert!(close(header.get_f64("CRPIX1"), 67.5), "CRPIX1 {:?}", header.get("CRPIX1"));

        let res = align_to_disk(vec![coarse, fine], &dir.path().join("out2"), None).await;
        assert_eq!(res[RES_REFERENCE_INDEX], 1, "{res}");
        assert_eq!(res[CHANNELS][0][RES_PREFILTER_K], 1, "a finer reference never pre-reduces: {res}");
        assert_eq!(res[DIMENSIONS], json!([256, 256]), "{res}");
    }

    #[tokio::test]
    async fn a_channel_without_overlap_is_empty_and_warned() {
        let dir = tempfile::tempdir().unwrap();
        let ref_header = tan_header(64, (32.5, 32.5), (83.0, 22.0), rotated_cd(0.2, 0.0));
        let far_header = tan_header(64, (32.5, 32.5), (83.0, 23.0), rotated_cd(0.2, 0.0));
        let reference = write_channel(dir.path(), "ref.fits", &star_field(0.0, 0.0), &ref_header);
        let far = write_channel(dir.path(), "far.fits", &star_field(2.0, -3.0), &far_header);

        let res = align_to_disk(vec![reference, far], &dir.path().join("out"), None).await;

        assert_eq!(
            res[RES_WARNINGS],
            json!(["far: its WCS footprint does not overlap the reference grid; the channel is empty after reprojection."]),
            "{res}"
        );
        let ch = &res[CHANNELS][1];
        assert_eq!(ch[RES_REPROJECTED], true, "{res}");
        assert_eq!(ch[RES_REGISTERED], false, "{res}");
        assert_eq!(ch[RES_METHOD_USED], "wcs", "{res}");
        assert_eq!(ch[RES_RESIDUAL_MEASURED], false, "{res}");
        assert_eq!(ch[RES_OFFSET], json!([0.0, 0.0]), "{res}");
        assert!(std::path::Path::new(ch[RES_PATH].as_str().unwrap()).exists(), "{res}");
    }

    #[tokio::test]
    async fn a_gwcs_channel_reports_the_wcs_kind_it_was_reprojected_with() {
        use crate::core::astrometry::gwcs::test_support::{fixture_pipeline, fixtures_dir};
        use crate::infra::fits::asdf_hdu::test_fixtures::write_fits_with_asdf_cell;

        let dir = tempfile::tempdir().unwrap();
        let centre = fixture_pipeline("wcs_jwst_nircam_cal300.asdf").forward(1.5, 1.5, false);
        let nircam_scale_deg = 0.0312 / 3600.0;
        let cards: Vec<(&'static str, String)> = vec![
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
            ("CRPIX1", "2.5".into()),
            ("CRPIX2", "2.5".into()),
            ("CRVAL1", centre[0].to_string()),
            ("CRVAL2", centre[1].to_string()),
            ("CDELT1", (-nircam_scale_deg).to_string()),
            ("CDELT2", nircam_scale_deg.to_string()),
        ];
        let channel = dir.path().join("nircam_cal.fits");
        write_fits_with_asdf_cell(&channel, &cards, &std::fs::read(fixtures_dir().join("wcs_jwst_nircam_cal300.asdf")).unwrap());
        let channel = channel.to_str().unwrap().to_string();
        let reference = |name: &str, arcsec_per_px: f64| {
            let mut h = HduHeader::empty();
            h.set("CTYPE1", "RA---TAN".to_string());
            h.set("CTYPE2", "DEC--TAN".to_string());
            h.set_f64("CRPIX1", 2.5);
            h.set_f64("CRPIX2", 2.5);
            h.set_f64("CRVAL1", centre[0]);
            h.set_f64("CRVAL2", centre[1]);
            h.set_f64("CD1_1", -arcsec_per_px / 3600.0);
            h.set_f64("CD2_2", arcsec_per_px / 3600.0);
            let path = dir.path().join(name).to_str().unwrap().to_string();
            write_fits_mono(&path, &Array2::<f32>::ones((4, 4)), Some(&h)).unwrap();
            path
        };

        let res = align(vec![reference("ref_same_scale.fits", 0.0312), channel.clone()], None, Some(0), None).await;
        let ch = &res[CHANNELS][1];
        assert_eq!(ch[RES_REPROJECTED], true, "{res}");
        assert_eq!(ch[RES_PREFILTER_K], 1, "{res}");
        assert_eq!(ch[RES_WCS_KIND], "gwcs", "an unreduced gWCS channel is reprojected through its gWCS: {res}");
        assert!(res[CHANNELS][0][RES_WCS_KIND].is_null(), "{res}");

        let res = align(vec![reference("ref_coarse.fits", 0.1), channel], None, Some(0), None).await;
        let ch = &res[CHANNELS][1];
        assert_eq!(ch[RES_REPROJECTED], true, "{res}");
        assert_eq!(ch[RES_PREFILTER_K], 3, "{res}");
        assert_eq!(ch[RES_WCS_KIND], "header", "the box-reduced channel is reprojected through its rebuilt header: {res}");
    }

    #[tokio::test]
    #[ignore]
    async fn gwcs_real_a4_cal_channel_onto_the_f200w_i2d() {
        use crate::core::astrometry::gwcs::test_support::{heavy_test_dir, real_data_dir, skip_if_absent};

        let i2d = heavy_test_dir().join("jw02739-o001_t001_nircam_clear-f200w_i2d.fits");
        let cal = real_data_dir().join("jw02739001001_02105_00001_nrca1_cal.fits");
        if skip_if_absent(&i2d) || skip_if_absent(&cal) {
            return;
        }
        let paths = [i2d, cal].map(|p| p.to_string_lossy().into_owned()).to_vec();
        let res = align(paths, None, Some(0), None).await;
        let ch = &res[CHANNELS][1];
        println!(
            "A4 real data: elapsed_ms {} prefilter_k {} wcs_kind {} wcs_scale_ratio {} wcs_rotation_deg {} dimensions {}",
            res[RES_ELAPSED_MS], ch[RES_PREFILTER_K], ch[RES_WCS_KIND], ch[RES_WCS_SCALE_RATIO], ch[RES_WCS_ROTATION_DEG], res[DIMENSIONS]
        );
        assert_eq!(res[RES_REFERENCE_RULE], "selected", "{res}");
        assert_eq!(ch[RES_REPROJECTED], true, "{ch}");
        assert_eq!(ch[RES_WCS_KIND], "gwcs", "{ch}");
        assert_eq!(ch[RES_PREFILTER_K], 1, "{ch}");
        assert!(res[RES_ELAPSED_MS].as_u64().unwrap() < 20_000, "elapsed_ms {}", res[RES_ELAPSED_MS]);
    }

    fn finite_box_centroid(img: &Array2<f32>, start: (f64, f64)) -> Option<(f64, f64)> {
        const HALF: usize = 12;
        let (cx, cy) = (start.0.round() as usize, start.1.round() as usize);
        let (rows, cols) = img.dim();
        if cx < HALF || cy < HALF || cx + HALF >= cols || cy + HALF >= rows {
            return None;
        }
        let mut values: Vec<f32> = img.slice(ndarray::s![cy - HALF..=cy + HALF, cx - HALF..=cx + HALF]).iter().copied().collect();
        if values.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let pedestal = crate::math::median_f32_mut(&mut values) as f64;
        Some(centroid_near(img, start, pedestal))
    }

    fn star_offsets_in_five_windows(reference: &Array2<f32>, aligned: &Array2<f32>) -> Vec<f64> {
        use crate::core::analysis::star_detection::detect_stars;
        const WINDOW: usize = 700;
        const INSET: usize = 300;
        let (rows, cols) = reference.dim();
        let origins = [
            (INSET, INSET),
            (INSET, cols - INSET - WINDOW),
            (rows - INSET - WINDOW, INSET),
            (rows - INSET - WINDOW, cols - INSET - WINDOW),
            ((rows - WINDOW) / 2, (cols - WINDOW) / 2),
        ];
        let mut offsets = Vec::new();
        for (r0, c0) in origins {
            let window = reference.slice(ndarray::s![r0..r0 + WINDOW, c0..c0 + WINDOW]).to_owned();
            let stars = detect_stars(&window, 20.0).stars;
            let isolated = stars.iter().filter(|s| stars.iter().filter(|o| (o.x - s.x).hypot(o.y - s.y) < 12.0).count() == 1);
            let mut taken = 0;
            for s in isolated {
                if taken == 4 {
                    break;
                }
                let start = (c0 as f64 + s.x, r0 as f64 + s.y);
                let (Some(a), Some(b)) = (finite_box_centroid(reference, start), finite_box_centroid(aligned, start)) else {
                    continue;
                };
                offsets.push((b.0 - a.0).hypot(b.1 - a.1));
                taken += 1;
            }
        }
        offsets
    }

    #[tokio::test]
    #[ignore]
    async fn real_f444w_onto_f200w_keeps_the_wcs_registration_when_phase_correlation_is_biased() {
        use crate::core::astrometry::gwcs::test_support::{heavy_test_dir, skip_if_absent};

        let f200w = heavy_test_dir().join("jw02739-o001_t001_nircam_clear-f200w_i2d.fits");
        let f444w = heavy_test_dir().join("jw02739-o001_t001_nircam_clear-f444w_i2d.fits");
        if skip_if_absent(&f200w) || skip_if_absent(&f444w) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let paths = [f200w, f444w].map(|p| p.to_string_lossy().into_owned()).to_vec();

        let res = align_to_disk(paths.clone(), dir.path(), None).await;

        assert_eq!(res[RES_REFERENCE_INDEX], 0, "{res}");
        assert_eq!(res[DIMENSIONS], json!([14344, 8589]), "{res}");
        assert_eq!(res[RES_WARNINGS], json!([]), "{res}");
        let ch = &res[CHANNELS][1];
        assert_eq!(ch[RES_REPROJECTED], true, "{ch}");
        let reference = load_cached_full(&paths[0]).unwrap();
        let aligned = load_cached_full(ch[RES_PATH].as_str().unwrap()).unwrap();
        let mut offsets = star_offsets_in_five_windows(reference.arr(), aligned.arr());
        assert!(offsets.len() >= 10, "{} stars measured: {offsets:?}", offsets.len());
        let median = crate::math::exact_median_f64(&offsets);
        offsets.sort_by(f64::total_cmp);
        assert!(median <= 1.0, "persisted F444W stars sit a median {median:.3} px from F200W: {offsets:?} {ch}");
        assert_eq!(ch[RES_RESIDUAL_MEASURED], true, "the Align note must show a measured residual: {ch}");
        let offset = ch[RES_OFFSET].as_array().unwrap();
        assert!(offset.iter().all(|v| v.as_f64().unwrap().abs() <= 1.0), "residual {offset:?}: {ch}");
    }
}
