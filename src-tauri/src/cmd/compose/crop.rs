use std::sync::Arc;
use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, resolve_output_dir};
use crate::core::imaging::stats::compute_image_stats;
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::infra::fits::writer::write_fits_mono;

const AUTO_THRESHOLD: f32 = 1e-6;

fn detect_valid_region(arr: &Array2<f32>, threshold: f32) -> (usize, usize, usize, usize) {
    let (rows, cols) = arr.dim();

    let mut top = 0usize;
    'outer_top: for r in 0..rows {
        for c in 0..cols {
            if arr[[r, c]].abs() > threshold {
                top = r;
                break 'outer_top;
            }
        }
        top = r + 1;
    }

    let mut bottom = rows;
    'outer_bot: for r in (0..rows).rev() {
        for c in 0..cols {
            if arr[[r, c]].abs() > threshold {
                bottom = r + 1;
                break 'outer_bot;
            }
        }
        bottom = r;
    }

    let mut left = 0usize;
    'outer_left: for c in 0..cols {
        for r in 0..rows {
            if arr[[r, c]].abs() > threshold {
                left = c;
                break 'outer_left;
            }
        }
        left = c + 1;
    }

    let mut right = cols;
    'outer_right: for c in (0..cols).rev() {
        for r in 0..rows {
            if arr[[r, c]].abs() > threshold {
                right = c + 1;
                break 'outer_right;
            }
        }
        right = c;
    }

    (top, bottom, left, right)
}

fn crop_array(arr: &Array2<f32>, top: usize, bottom: usize, left: usize, right: usize) -> Array2<f32> {
    let (rows, cols) = arr.dim();
    let t = top.min(rows);
    let b = bottom.min(rows).max(t);
    let l = left.min(cols);
    let r = right.min(cols).max(l);
    arr.slice(ndarray::s![t..b, l..r]).to_owned()
}

#[tauri::command]
pub async fn crop_channels_cmd(
    paths: Vec<String>,
    output_dir: String,
    top: usize,
    bottom: usize,
    left: usize,
    right: usize,
    auto_detect: Option<bool>,
    bin_ids: Option<Vec<String>>,
    persist_to_disk: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let write_disk = persist_to_disk.unwrap_or(false);
        if write_disk {
            resolve_output_dir(&output_dir)?;
        }

        if paths.is_empty() {
            anyhow::bail!("No paths provided for cropping");
        }

        let entries: Vec<_> = paths
            .iter()
            .map(|p| load_from_cache_or_disk(p))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let auto = auto_detect.unwrap_or(true);

        let (crop_top, crop_bottom, crop_left, crop_right) = if auto {
            let mut max_top = 0usize;
            let mut min_bottom = usize::MAX;
            let mut max_left = 0usize;
            let mut min_right = usize::MAX;

            for entry in &entries {
                let arr = entry.arr();
                let (t, b, l, r) = detect_valid_region(arr, AUTO_THRESHOLD);
                max_top = max_top.max(t);
                min_bottom = min_bottom.min(b);
                max_left = max_left.max(l);
                min_right = min_right.min(r);
            }

            if min_bottom <= max_top || min_right <= max_left {
                anyhow::bail!("Auto-crop found no valid overlapping region");
            }

            (max_top, min_bottom, max_left, min_right)
        } else {
            let (rows, cols) = entries[0].arr().dim();
            (top, rows.saturating_sub(bottom), left, cols.saturating_sub(right))
        };

        let use_bin_ids = bin_ids.as_ref().map(|ids| ids.len() == paths.len()).unwrap_or(false);

        let mut out_paths = Vec::new();
        let mut cache_keys = Vec::new();

        for (i, entry) in entries.iter().enumerate() {
            let arr = entry.arr();
            let cropped = crop_array(arr, crop_top, crop_bottom, crop_left, crop_right);

            if use_bin_ids {
                let bid = &bin_ids.as_ref().unwrap()[i];
                let k = crate::types::constants::wizard_cropped_key(bid);
                let stats = compute_image_stats(&cropped);
                GLOBAL_IMAGE_CACHE.insert_synthetic(&k, Arc::new(cropped.clone()), stats);
                cache_keys.push(k.clone());

                if write_disk {
                    let stem = std::path::Path::new(&paths[i])
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("ch");
                    let out_path = format!("{}/{}_cropped.fits", output_dir, stem);
                    write_fits_mono(&out_path, &cropped, None)?;
                    out_paths.push(out_path);
                } else {
                    out_paths.push(k);
                }
            } else {
                let stem = std::path::Path::new(&paths[i])
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("ch");
                let out_path = format!("{}/{}_cropped.fits", output_dir, stem);
                resolve_output_dir(&output_dir)?;
                write_fits_mono(&out_path, &cropped, None)?;

                let stats = compute_image_stats(&cropped);
                GLOBAL_IMAGE_CACHE.insert_synthetic(&out_path, Arc::new(cropped), stats);
                out_paths.push(out_path);
            }
        }

        let (out_rows, out_cols) = if !entries.is_empty() {
            let sample = crop_array(entries[0].arr(), crop_top, crop_bottom, crop_left, crop_right);
            sample.dim()
        } else {
            (0, 0)
        };

        let elapsed = t0.elapsed().as_millis() as u64;

        let first_dim = entries.first().map(|e| e.arr().dim()).unwrap_or((0, 0));
        let actual_top = crop_top;
        let actual_bottom = first_dim.0.saturating_sub(crop_bottom);
        let actual_left = crop_left;
        let actual_right = first_dim.1.saturating_sub(crop_right);

        Ok(json!({
            "paths": out_paths,
            "cache_keys": cache_keys,
            "dimensions": [out_cols, out_rows],
            "crop_top": actual_top,
            "crop_bottom": actual_bottom,
            "crop_left": actual_left,
            "crop_right": actual_right,
            "auto_detected": auto,
            "elapsed_ms": elapsed,
        }))
    })
}
