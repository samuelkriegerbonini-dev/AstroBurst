use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, cached_header, derived_output_header, load_from_cache_or_disk, output_stem, resolve_output_dir, write_derived_fits, OutputValues, MAX_PREVIEW_DIM};
use crate::core::imaging::stf::{make_stf_u8_fn, AutoStfConfig};
use crate::cmd::helpers;
use crate::cmd::processing::source_header;
use crate::core::alignment::pair::align_pair_with_label;
use crate::core::compose::rgb::{harmonize_dimensions, align_channels};
use crate::core::analysis::photometry::SATURATION_KEYWORDS;
use crate::core::compose::channel_blend::{blend_channels, BlendWeight};
use crate::core::imaging::resample::{compute_wcs_updates, resample_image};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::metadata::photcal::{GENERIC_ZERO_POINT_KEYS, ROMAN_PIXEL_AREA_SR_KEYS};
use crate::infra::fits::writer::filter_header;
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::types::header::HduHeader;
use crate::types::constants::{MAX_DIMENSION_RATIO, RES_DIMENSIONS, RES_ELAPSED_MS, RES_MAX, RES_MEAN, RES_MEDIAN, RES_MIN, RES_PNG_PATH, RES_STATS_B, RES_STATS_G, RES_STATS_R, ALIGN_METHOD, DIMENSIONS, CHANNELS, RES_CHANNEL, RES_PATH, RES_FILE_SIZE_BYTES, RES_OFFSET, RES_BLEND_PRESET, RES_CHANNEL_COUNT, RES_AUTO_STF, RES_CACHE_KEY, RES_CONFIDENCE, RES_METHOD_USED, RES_MATCHED_STARS, RES_INLIERS, RES_RESIDUAL_PX, RES_REGISTERED};

use super::rgb::{composite_png_path, load_entry};

const ABPROC_ALIGNED: &str = "aligned";
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
    if target_dims != grid_dims && grid_dims.0 > 0 && grid_dims.1 > 0 {
        let area_ratio = (target_dims.0 as f64 / grid_dims.0 as f64) * (target_dims.1 as f64 / grid_dims.1 as f64);
        rescale_per_pixel_calibration(&mut header, area_ratio);
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

        let method = helpers::parse_align_method(align_method.as_deref());

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

        helpers::insert_composite_and_orig(r, g, b, stats_r.clone(), stats_g.clone(), stats_b.clone());

        let png_path = composite_png_path(&output_dir);
        let (er, eg, eb) = helpers::load_composite_rgb()?;

        let stf_config = AutoStfConfig::default();
        let (linked_stf, combined_stats) =
            helpers::compute_linked_stf_with_stats(er.stats(), eg.stats(), eb.stats(), &stf_config);
        let fn_r = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_g = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_b = make_stf_u8_fn(&linked_stf, &combined_stats);
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
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

#[tauri::command]
pub async fn align_channels_cmd(
    paths: Vec<String>,
    output_dir: String,
    align_method: Option<String>,
    bin_ids: Option<Vec<String>>,
    persist_to_disk: Option<bool>,
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

        let entries: Vec<_> = paths
            .iter()
            .map(|p| load_from_cache_or_disk(p))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let ref_arr = entries[0].arr();
        let (rows, cols) = ref_arr.dim();

        let method = helpers::parse_align_method(align_method.as_deref());

        let use_bin_ids = bin_ids.as_ref().map(|ids| ids.len() == paths.len()).unwrap_or(false);
        if use_bin_ids {
            GLOBAL_IMAGE_CACHE.remove_prefix(crate::types::constants::WIZARD_CACHE_PREFIX);
        }

        let mut channel_results = Vec::new();

        let ref_header = source_header(&paths[0], &entries[0]);
        let header0 = derived_output_header(ref_header.as_ref(), ABPROC_ALIGNED, OutputValues::Linear);

        let ref_key = if use_bin_ids {
            let bid = &bin_ids.as_ref().unwrap()[0];
            let k = crate::types::constants::wizard_aligned_key(bid);
            let stats = entries[0].stats().clone();
            GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(&k, entries[0].data_arc(), stats, Some(header0.clone()));
            k
        } else {
            String::new()
        };

        if write_disk {
            let stem0 = output_stem(&paths[0]);
            let out0 = format!("{}/{}_aligned.fits", output_dir, stem0);
            write_derived_fits(&out0, ref_arr, Some(&header0))?;
            channel_results.push(json!({
                RES_OFFSET: [0.0, 0.0],
                RES_REGISTERED: true,
                RES_PATH: out0,
                RES_CACHE_KEY: ref_key,
            }));
        } else {
            channel_results.push(json!({
                RES_OFFSET: [0.0, 0.0],
                RES_REGISTERED: true,
                RES_CACHE_KEY: ref_key,
            }));
        }

        for (i, entry) in entries.iter().enumerate().skip(1) {
            let target = entry.arr();
            let (tr, tc) = target.dim();

            let target_resized = if tr != rows || tc != cols {
                resample_image(target, rows, cols)?
            } else {
                target.to_owned()
            };

            let label = output_stem(&paths[i]);

            let result = align_pair_with_label(
                ref_arr,
                &target_resized,
                method,
                rows,
                cols,
                &label,
            )?;

            let header = registered_channel_header(
                source_header(&paths[i], entry).as_ref(),
                ref_header.as_ref(),
                (tr, tc),
                (rows, cols),
                result.registered,
            );

            let cache_key = if use_bin_ids {
                let bid = &bin_ids.as_ref().unwrap()[i];
                let k = crate::types::constants::wizard_aligned_key(bid);
                let stats = compute_image_stats(&result.aligned);
                GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(
                    &k,
                    std::sync::Arc::new(result.aligned.clone()),
                    stats,
                    Some(header.clone()),
                );
                k
            } else {
                String::new()
            };

            let mut entry_json = json!({
                RES_OFFSET: [result.offset.0, result.offset.1],
                RES_REGISTERED: result.registered,
                RES_CONFIDENCE: result.confidence,
                RES_METHOD_USED: result.method_used,
                RES_MATCHED_STARS: result.matched_stars,
                RES_INLIERS: result.inliers,
                RES_RESIDUAL_PX: result.residual_px,
                RES_CACHE_KEY: cache_key,
            });

            if write_disk {
                let out_path = format!("{}/{}_aligned.fits", output_dir, label);
                write_derived_fits(&out_path, &result.aligned, Some(&header))?;
                entry_json.as_object_mut().unwrap().insert(RES_PATH.to_string(), json!(out_path));
            }

            channel_results.push(entry_json);
        }

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            CHANNELS: channel_results,
            ALIGN_METHOD: helpers::align_method_str(method),
            DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::load_cached_full;
    use crate::core::astrometry::wcs::WcsTransform;
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

        let header = registered_channel_header(Some(&target), Some(&reference), (16, 16), (32, 32), true);
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

        let unregistered = registered_channel_header(Some(&target), Some(&reference), (16, 16), (32, 32), false);
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

    fn lone_blob(size: usize, cx: f32) -> Array2<f32> {
        let cy = size as f32 * 0.5;
        Array2::from_shape_fn((size, size), |(y, x)| {
            let (dy, dx) = (y as f32 - cy, x as f32 - cx);
            100.0 + 1000.0 * (-(dy * dy + dx * dx) / 18.0).exp()
        })
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

        let res = align_channels_cmd(vec![reference, shifted], out.clone(), None, None, None).await.unwrap();
        let channels = res[CHANNELS].as_array().unwrap();
        assert_eq!(channels[0][RES_REGISTERED], true, "{res}");
        assert_eq!(channels[1][RES_REGISTERED], true, "{res}");

        let res = align_channels_cmd(vec![far_reference.clone(), far_target.clone()], out.clone(), None, None, None)
            .await
            .unwrap();
        let channels = res[CHANNELS].as_array().unwrap();
        assert_eq!(channels[1][RES_METHOD_USED], "phase_correlation_identity", "{res}");
        assert_eq!(channels[1][RES_REGISTERED], false, "{res}");
        assert_eq!(channels[1][RES_OFFSET], json!([0.0, 0.0]));

        let res = align_channels_cmd(vec![far_reference, far_target], out, Some(ALIGN_METHOD_AFFINE.to_string()), None, None)
            .await
            .unwrap();
        let channels = res[CHANNELS].as_array().unwrap();
        assert_eq!(channels[1][RES_METHOD_USED], "identity", "{res}");
        assert_eq!(channels[1][RES_REGISTERED], false, "{res}");
    }
}
