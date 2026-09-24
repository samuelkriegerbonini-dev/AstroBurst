use std::sync::Arc;
use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, cached_header, derived_output_header, load_from_cache_or_disk, output_stem, resolve_output_dir,
    write_derived_fits, OutputValues,
};
use crate::core::imaging::cutout::{shift_header, CutoutRect};
use crate::core::imaging::stats::compute_image_stats;
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::types::header::HduHeader;
use crate::types::constants::{
    RES_PATHS, RES_CACHE_KEYS, RES_DIMENSIONS, RES_CROP_TOP, RES_CROP_BOTTOM,
    RES_CROP_LEFT, RES_CROP_RIGHT, RES_AUTO_DETECTED, RES_ELAPSED_MS,
};

const AUTO_THRESHOLD: f32 = 1e-6;
const ABPROC_CROPPED: &str = "cropped";

fn cropped_header(source: Option<&HduHeader>, top: usize, left: usize, dims: (usize, usize)) -> HduHeader {
    let rect = CutoutRect { x0: left as i64, y0: top as i64, width: dims.1, height: dims.0 };
    let shifted = source.map(|h| shift_header(h, &rect));
    derived_output_header(shifted.as_ref(), ABPROC_CROPPED, OutputValues::Linear)
}

fn write_cropped(out_path: &str, cropped: &Array2<f32>, source_path: &str, top: usize, left: usize) -> anyhow::Result<()> {
    let header = cropped_header(cached_header(source_path).ok().as_ref(), top, left, cropped.dim());
    write_derived_fits(out_path, cropped, Some(&header))
}

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

fn manual_crop_bounds(
    rows: usize,
    cols: usize,
    top: usize,
    bottom: usize,
    left: usize,
    right: usize,
) -> Option<(usize, usize, usize, usize)> {
    let crop_top = top;
    let crop_bottom = rows.saturating_sub(bottom);
    let crop_left = left;
    let crop_right = cols.saturating_sub(right);
    if crop_bottom <= crop_top || crop_right <= crop_left {
        None
    } else {
        Some((crop_top, crop_bottom, crop_left, crop_right))
    }
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
            match manual_crop_bounds(rows, cols, top, bottom, left, right) {
                Some(bounds) => bounds,
                None => anyhow::bail!("Crop region is empty (margins exceed image size)"),
            }
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
                    let stem = output_stem(&paths[i]);
                    let out_path = format!("{}/{}_cropped.fits", output_dir, stem);
                    write_cropped(&out_path, &cropped, &paths[i], crop_top, crop_left)?;
                    out_paths.push(out_path);
                } else {
                    out_paths.push(k);
                }
            } else {
                let stem = output_stem(&paths[i]);
                let out_path = format!("{}/{}_cropped.fits", output_dir, stem);
                resolve_output_dir(&output_dir)?;
                write_cropped(&out_path, &cropped, &paths[i], crop_top, crop_left)?;

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
            RES_PATHS: out_paths,
            RES_CACHE_KEYS: cache_keys,
            RES_DIMENSIONS: [out_cols, out_rows],
            RES_CROP_TOP: actual_top,
            RES_CROP_BOTTOM: actual_bottom,
            RES_CROP_LEFT: actual_left,
            RES_CROP_RIGHT: actual_right,
            RES_AUTO_DETECTED: auto,
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::{load_cached_full, HEADER_ABPROC};
    use crate::core::astrometry::wcs::WcsTransform;
    use crate::infra::fits::writer::write_fits_mono;

    fn card(header: &HduHeader, key: &str) -> Option<String> {
        header.get(key).map(|v| v.trim().trim_matches('\'').trim().to_string())
    }

    #[tokio::test]
    async fn a_cropped_file_keeps_the_source_header_with_the_wcs_moved_to_the_crop_origin() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("wide.fits").to_str().unwrap().to_string();
        let out = dir.path().join("out");
        let mut header = HduHeader::empty();
        header.set("CTYPE1", "RA---TAN".to_string());
        header.set("CTYPE2", "DEC--TAN".to_string());
        header.set_f64("CRVAL1", 150.0);
        header.set_f64("CRVAL2", 2.0);
        header.set_f64("CRPIX1", 20.0);
        header.set_f64("CRPIX2", 30.0);
        header.set_f64("CD1_1", -1e-5);
        header.set_f64("CD2_2", 1e-5);
        header.set("BUNIT", "MJy/sr".to_string());
        header.set_f64("PHOTMJSR", 1.5);
        header.set("FILTER", "F200W".to_string());
        write_fits_mono(&src, &Array2::from_elem((40, 50), 5.0f32), Some(&header)).unwrap();

        let res = crop_channels_cmd(vec![src.clone()], out.to_str().unwrap().to_string(), 2, 4, 3, 5, Some(false), None, None)
            .await
            .unwrap();
        let path = res[RES_PATHS][0].as_str().unwrap().to_string();
        let written = load_cached_full(&path).unwrap();
        assert_eq!(written.arr().dim(), (34, 42));
        let h = written.header().expect("header").clone();
        assert!((h.get_f64("CRPIX1").unwrap() - 17.0).abs() < 1e-9, "CRPIX1 {:?}", h.get("CRPIX1"));
        assert!((h.get_f64("CRPIX2").unwrap() - 28.0).abs() < 1e-9, "CRPIX2 {:?}", h.get("CRPIX2"));
        assert_eq!(h.get_f64("LTV1"), Some(-3.0));
        assert_eq!(h.get_f64("LTV2"), Some(-2.0));
        assert_eq!(card(&h, "BUNIT").as_deref(), Some("MJy/sr"));
        assert_eq!(h.get_f64("PHOTMJSR"), Some(1.5));
        assert_eq!(card(&h, "FILTER").as_deref(), Some("F200W"));
        assert_eq!(card(&h, HEADER_ABPROC).as_deref(), Some(ABPROC_CROPPED));

        header.set("NAXIS1", "50".to_string());
        header.set("NAXIS2", "40".to_string());
        let parent = WcsTransform::from_header(&header).unwrap();
        let cropped = WcsTransform::from_header(&h).unwrap();
        for (x, y) in [(0.0, 0.0), (10.5, 20.25), (41.0, 33.0)] {
            let a = parent.pixel_to_world(x + 3.0, y + 2.0);
            let b = cropped.pixel_to_world(x, y);
            assert!((a.ra - b.ra).abs() < 1e-12 && (a.dec - b.dec).abs() < 1e-12, "({x},{y}): {a:?} vs {b:?}");
        }
    }

    #[test]
    fn manual_crop_rejects_excessive_margins() {
        assert_eq!(manual_crop_bounds(100, 100, 10, 10, 10, 10), Some((10, 90, 10, 90)));
        assert_eq!(manual_crop_bounds(100, 100, 60, 60, 0, 0), None);
        assert_eq!(manual_crop_bounds(100, 100, 0, 0, 70, 70), None);
        assert_eq!(manual_crop_bounds(100, 100, 50, 50, 0, 0), None);
    }
}
