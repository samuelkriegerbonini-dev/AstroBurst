use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, resolve_output_dir, extract_image_resolved, MAX_PREVIEW_DIM};
use crate::core::imaging::stf::{make_stf_u8_fn, AutoStfConfig};
use crate::cmd::helpers;
use crate::core::alignment::pair::align_pair_with_label;
use crate::core::compose::rgb::{harmonize_dimensions, align_channels};
use crate::core::compose::channel_blend::{blend_channels, BlendWeight};
use crate::core::imaging::resample::resample_image;
use crate::core::imaging::stats::compute_image_stats;
use crate::infra::fits::writer::{write_fits_mono, filter_header};
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::types::constants::{MAX_DIMENSION_RATIO, RES_DIMENSIONS, RES_ELAPSED_MS, RES_MAX, RES_MEAN, RES_MEDIAN, RES_MIN, RES_PNG_PATH, RES_STATS_B, RES_STATS_G, RES_STATS_R, ALIGN_METHOD, DIMENSIONS, CHANNELS, RES_CHANNEL, RES_PATH, RES_FILE_SIZE_BYTES, RES_OFFSET, RES_BLEND_PRESET, RES_CHANNEL_COUNT};

use super::rgb::{composite_png_path, load_entry};

fn update_wcs_for_offset(header: &mut crate::types::header::HduHeader, dy: f64, dx: f64) {
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

fn build_channel_header(
    path: &Option<String>,
    offset: (f64, f64),
    copy_wcs: bool,
    copy_metadata: bool,
) -> Option<crate::types::header::HduHeader> {
    let p = path.as_ref()?;
    let resolved = extract_image_resolved(p).ok()?;
    let mut hdr = filter_header(&resolved.header, copy_wcs, copy_metadata)?;
    if copy_wcs {
        update_wcs_for_offset(&mut hdr, offset.0, offset.1);
    }
    Some(hdr)
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

        let r_entry = load_entry(&r_path)?;
        let g_entry = load_entry(&g_path)?;
        let b_entry = load_entry(&b_path)?;

        let r_ref = r_entry.as_ref().map(|e| e.arr());
        let g_ref = g_entry.as_ref().map(|e| e.arr());
        let b_ref = b_entry.as_ref().map(|e| e.arr());

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

        let channels: Vec<(&str, &Array2<f32>, &Option<String>, (f64, f64))> = vec![
            ("R", &r_aligned, &r_path, (0.0, 0.0)),
            ("G", &g_aligned, &g_path, off_g),
            ("B", &b_aligned, &b_path, off_b),
        ];

        for (label, data, src_path, offset) in &channels {
            if src_path.is_none() {
                continue;
            }
            let stem = src_path.as_ref().unwrap()
                .split(&['/', '\\'][..])
                .last()
                .unwrap_or("channel")
                .replace(".fits", "")
                .replace(".fit", "")
                .replace(".fts", "");
            let out_path = format!("{}/{}_aligned.fits", out_dir, stem);

            let hdr = build_channel_header(src_path, *offset, do_wcs, do_meta);
            write_fits_mono(&out_path, data, hdr.as_ref())?;

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

        let (r, g, b) = blend_channels(&refs, &blend_weights, max_rows, max_cols);

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
        let linked_stf = helpers::compute_linked_stf(er.stats(), eg.stats(), eb.stats(), &stf_config);
        let fn_r = make_stf_u8_fn(&linked_stf, er.stats());
        let fn_g = make_stf_u8_fn(&linked_stf, eg.stats());
        let fn_b = make_stf_u8_fn(&linked_stf, eb.stats());
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
            "auto_stf": stf_json,
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

        let mut channel_results = Vec::new();

        let ref_key = if use_bin_ids {
            let bid = &bin_ids.as_ref().unwrap()[0];
            let k = crate::types::constants::wizard_aligned_key(bid);
            let stats = compute_image_stats(ref_arr);
            GLOBAL_IMAGE_CACHE.insert_synthetic(&k, std::sync::Arc::new(ref_arr.to_owned()), stats);
            k
        } else {
            String::new()
        };

        if write_disk {
            let stem0 = std::path::Path::new(&paths[0])
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("ch0");
            let out0 = format!("{}/{}_aligned.fits", output_dir, stem0);
            crate::infra::fits::writer::write_fits_mono(&out0, ref_arr, None)?;
            channel_results.push(json!({
                RES_OFFSET: [0.0, 0.0],
                RES_PATH: out0,
                "cache_key": ref_key,
            }));
        } else {
            channel_results.push(json!({
                RES_OFFSET: [0.0, 0.0],
                "cache_key": ref_key,
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

            let label = std::path::Path::new(&paths[i])
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("ch");

            let result = align_pair_with_label(
                ref_arr,
                &target_resized,
                method,
                rows,
                cols,
                label,
            )?;

            let cache_key = if use_bin_ids {
                let bid = &bin_ids.as_ref().unwrap()[i];
                let k = crate::types::constants::wizard_aligned_key(bid);
                let stats = compute_image_stats(&result.aligned);
                GLOBAL_IMAGE_CACHE.insert_synthetic(&k, std::sync::Arc::new(result.aligned.clone()), stats);
                k
            } else {
                String::new()
            };

            let mut entry_json = json!({
                RES_OFFSET: [result.offset.0, result.offset.1],
                "confidence": result.confidence,
                "method_used": result.method_used,
                "matched_stars": result.matched_stars,
                "inliers": result.inliers,
                "residual_px": result.residual_px,
                "cache_key": cache_key,
            });

            if write_disk {
                let out_path = format!("{}/{}_aligned.fits", output_dir, label);
                crate::infra::fits::writer::write_fits_mono(&out_path, &result.aligned, None)?;
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
