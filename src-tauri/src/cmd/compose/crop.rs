use std::sync::Arc;
use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, derived_output_header, output_stem, resolve_output_dir, write_derived_fits,
    OutputValues,
};
use crate::cmd::compose::overlay_preview::load_channel_entry;
use crate::cmd::processing::source_header;
use crate::core::imaging::cutout::{shift_header, CutoutRect};
use crate::core::imaging::stats::compute_image_stats;
use crate::infra::cache::{ImageEntry, GLOBAL_IMAGE_CACHE};
use crate::types::header::HduHeader;
use crate::types::constants::{
    RES_PATHS, RES_CACHE_KEYS, RES_DIMENSIONS, RES_CROP_TOP, RES_CROP_BOTTOM,
    RES_CROP_LEFT, RES_CROP_RIGHT, RES_AUTO_DETECTED, RES_ELAPSED_MS,
};

const AUTO_THRESHOLD: f32 = 1e-6;
const ABPROC_CROPPED: &str = "cropped";
const NO_OVERLAP: &str = "Auto-crop found no valid overlapping region";
const DIFFERENT_SIZES: &str = "Channels have different sizes; run Align first";

fn cropped_header(source: Option<&HduHeader>, top: usize, left: usize, dims: (usize, usize)) -> HduHeader {
    let rect = CutoutRect { x0: left as i64, y0: top as i64, width: dims.1, height: dims.0 };
    let shifted = source.map(|h| shift_header(h, &rect));
    derived_output_header(shifted.as_ref(), ABPROC_CROPPED, OutputValues::Linear)
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

fn auto_crop_bounds(entries: &[ImageEntry]) -> Option<(usize, usize, usize, usize)> {
    let mut max_top = 0usize;
    let mut min_bottom = usize::MAX;
    let mut max_left = 0usize;
    let mut min_right = usize::MAX;

    for entry in entries {
        let (t, b, l, r) = detect_valid_region(entry.arr(), AUTO_THRESHOLD);
        max_top = max_top.max(t);
        min_bottom = min_bottom.min(b);
        max_left = max_left.max(l);
        min_right = min_right.min(r);
    }

    if min_bottom <= max_top || min_right <= max_left {
        None
    } else {
        Some((max_top, min_bottom, max_left, min_right))
    }
}

fn crop_array(arr: &Array2<f32>, top: usize, bottom: usize, left: usize, right: usize) -> Array2<f32> {
    let (rows, cols) = arr.dim();
    let t = top.min(rows);
    let b = bottom.min(rows).max(t);
    let l = left.min(cols);
    let r = right.min(cols).max(l);
    arr.slice(ndarray::s![t..b, l..r]).to_owned()
}

fn cropped_cache_key(run_token: Option<&str>, bin_id: &str) -> String {
    crate::types::constants::wizard_cropped_key_for(run_token, bin_id)
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
    run_token: Option<String>,
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
            .map(|p| load_channel_entry(p))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let auto = auto_detect.unwrap_or(true);

        let (crop_top, crop_bottom, crop_left, crop_right) = if auto {
            match auto_crop_bounds(&entries) {
                Some(bounds) => bounds,
                None => anyhow::bail!(NO_OVERLAP),
            }
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
            let header = cropped_header(source_header(&paths[i], entry).as_ref(), crop_top, crop_left, cropped.dim());

            if use_bin_ids {
                let bid = &bin_ids.as_ref().unwrap()[i];
                let k = cropped_cache_key(run_token.as_deref(), bid);
                let stats = compute_image_stats(&cropped);
                GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(&k, Arc::new(cropped.clone()), stats, Some(header.clone()));
                cache_keys.push(k.clone());

                if write_disk {
                    let stem = output_stem(&paths[i]);
                    let out_path = format!("{}/{}_cropped.fits", output_dir, stem);
                    write_derived_fits(&out_path, &cropped, Some(&header))?;
                    out_paths.push(out_path);
                } else {
                    out_paths.push(k);
                }
            } else {
                let stem = output_stem(&paths[i]);
                let out_path = format!("{}/{}_cropped.fits", output_dir, stem);
                resolve_output_dir(&output_dir)?;
                write_derived_fits(&out_path, &cropped, Some(&header))?;

                let stats = compute_image_stats(&cropped);
                GLOBAL_IMAGE_CACHE.insert_synthetic(&out_path, Arc::new(cropped), stats);
                out_paths.push(out_path);
            }
        }

        if use_bin_ids {
            super::blend::settle_wizard_run(run_token.as_deref().map(str::trim).filter(|t| !t.is_empty()));
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

#[tauri::command]
pub async fn detect_crop_bounds_cmd(paths: Vec<String>) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        if paths.is_empty() {
            anyhow::bail!("No paths provided for crop detection");
        }

        let entries: Vec<_> = paths
            .iter()
            .map(|p| load_channel_entry(p))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let (rows, cols) = entries[0].arr().dim();
        if entries.iter().any(|e| e.arr().dim() != (rows, cols)) {
            anyhow::bail!(DIFFERENT_SIZES);
        }

        let (margins, auto_detected) = match auto_crop_bounds(&entries) {
            Some((top, bottom, left, right)) => ((top, rows - bottom, left, cols - right), true),
            None => ((0, 0, 0, 0), false),
        };

        Ok(json!({
            RES_DIMENSIONS: [cols, rows],
            RES_CROP_TOP: margins.0,
            RES_CROP_BOTTOM: margins.1,
            RES_CROP_LEFT: margins.2,
            RES_CROP_RIGHT: margins.3,
            RES_AUTO_DETECTED: auto_detected,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::{load_cached_full, HEADER_ABPROC};
    use crate::cmd::compose::overlay_preview::ALIGNED_GONE;
    use crate::core::astrometry::wcs::WcsTransform;
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::constants::{wizard_aligned_key, wizard_cropped_key, WIZARD_CACHE_PREFIX};

    fn nan_bordered(rows: usize, cols: usize, top: usize, bottom: usize, left: usize, right: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(r, c)| {
            if r < top || r >= rows - bottom || c < left || c >= cols - right {
                f32::NAN
            } else {
                5.0 + (r * cols + c) as f32 * 1e-3
            }
        })
    }

    fn seed_wizard_entry(bin: &str, arr: &Array2<f32>) -> String {
        let key = wizard_aligned_key(bin);
        GLOBAL_IMAGE_CACHE.insert_synthetic(&key, Arc::new(arr.clone()), compute_image_stats(arr));
        key
    }

    fn margins(res: &serde_json::Value) -> [u64; 4] {
        [RES_CROP_TOP, RES_CROP_BOTTOM, RES_CROP_LEFT, RES_CROP_RIGHT].map(|k| res[k].as_u64().unwrap())
    }

    #[tokio::test]
    async fn detect_crop_bounds_agrees_with_auto_crop_and_crops_nothing() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let r_key = seed_wizard_entry("det_r", &nan_bordered(40, 50, 2, 4, 3, 5));
        let g_key = seed_wizard_entry("det_g", &nan_bordered(40, 50, 6, 1, 0, 9));
        let seeded_r = GLOBAL_IMAGE_CACHE.get(&r_key).unwrap().data_arc();
        let seeded_g = GLOBAL_IMAGE_CACHE.get(&g_key).unwrap().data_arc();
        let test_prefix = format!("{WIZARD_CACHE_PREFIX}det_");

        let detected = detect_crop_bounds_cmd(vec![r_key.clone(), g_key.clone()]).await.unwrap();
        let detect_added_entries = GLOBAL_IMAGE_CACHE.any_key(|k| k.starts_with(&test_prefix) && k != r_key && k != g_key);
        let inputs_untouched = Arc::ptr_eq(&seeded_r, &GLOBAL_IMAGE_CACHE.get(&r_key).unwrap().data_arc())
            && Arc::ptr_eq(&seeded_g, &GLOBAL_IMAGE_CACHE.get(&g_key).unwrap().data_arc());

        let cropped = crop_channels_cmd(
            vec![r_key.clone(), g_key.clone()],
            dir.path().join("out").to_str().unwrap().to_string(),
            0,
            0,
            0,
            0,
            Some(true),
            Some(vec!["det_r".to_string(), "det_g".to_string()]),
            None,
            None,
        )
        .await
        .unwrap();
        for key in [&r_key, &g_key, &wizard_cropped_key("det_r"), &wizard_cropped_key("det_g")] {
            GLOBAL_IMAGE_CACHE.remove(key);
        }

        assert!(!detect_added_entries, "detection must not add entries under {test_prefix} besides its two inputs");
        assert!(inputs_untouched, "detection must not replace the data of the entries it reads");
        assert!(
            !dir.path().join("out").exists(),
            "a wizard crop without persist_to_disk must not create the output dir (detection takes none)"
        );
        assert_eq!(detected[RES_AUTO_DETECTED], true, "{detected}");
        assert_eq!(detected[RES_DIMENSIONS], json!([50, 40]));
        assert_eq!(margins(&detected), [6, 4, 3, 9]);
        assert_eq!(margins(&detected), margins(&cropped), "detect {detected} vs crop {cropped}");
        assert_eq!(cropped[RES_DIMENSIONS], json!([50 - 3 - 9, 40 - 6 - 4]));
    }

    #[tokio::test]
    async fn detect_crop_bounds_without_overlap_still_reports_the_grid() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let upper = seed_wizard_entry("det_upper", &nan_bordered(40, 50, 0, 30, 0, 0));
        let lower = seed_wizard_entry("det_lower", &nan_bordered(40, 50, 30, 0, 0, 0));

        let res = detect_crop_bounds_cmd(vec![upper.clone(), lower.clone()]).await;
        let crop = crop_channels_cmd(
            vec![upper.clone(), lower.clone()],
            "unused".to_string(),
            0,
            0,
            0,
            0,
            Some(true),
            Some(vec!["det_upper".to_string(), "det_lower".to_string()]),
            None,
            None,
        )
        .await;
        GLOBAL_IMAGE_CACHE.remove(&upper);
        GLOBAL_IMAGE_CACHE.remove(&lower);

        let res = res.expect("no overlap is not an error for detection");
        assert_eq!(res[RES_AUTO_DETECTED], false, "{res}");
        assert_eq!(res[RES_DIMENSIONS], json!([50, 40]));
        assert_eq!(margins(&res), [0, 0, 0, 0]);
        assert_eq!(crop.expect_err("auto crop without overlap must still fail"), NO_OVERLAP);
    }

    #[tokio::test]
    async fn detect_crop_bounds_rejects_channels_of_different_sizes() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let wide = seed_wizard_entry("det_wide", &nan_bordered(40, 50, 0, 0, 0, 0));
        let narrow = seed_wizard_entry("det_narrow", &nan_bordered(40, 30, 0, 0, 0, 0));

        let res = detect_crop_bounds_cmd(vec![wide.clone(), narrow.clone()]).await;
        GLOBAL_IMAGE_CACHE.remove(&wide);
        GLOBAL_IMAGE_CACHE.remove(&narrow);

        assert_eq!(res.expect_err("mixed sizes must be rejected"), DIFFERENT_SIZES);
        assert!(detect_crop_bounds_cmd(vec![]).await.is_err());
    }

    #[tokio::test]
    async fn detect_crop_bounds_on_a_missing_wizard_key_says_to_run_align_again() {
        let gone = detect_crop_bounds_cmd(vec![wizard_aligned_key("det_missing_probe")]).await;
        assert_eq!(gone.expect_err("a missing wizard key must fail"), ALIGNED_GONE);

        let missing_file = detect_crop_bounds_cmd(vec!["C:/astrokit-missing/det_probe.fits".to_string()]).await;
        assert_ne!(missing_file.expect_err("a missing file must fail"), ALIGNED_GONE);
    }

    #[tokio::test]
    async fn crop_channels_on_a_missing_wizard_key_says_to_run_align_again() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out").to_str().unwrap().to_string();
        let present = seed_wizard_entry("crop_present", &nan_bordered(40, 50, 2, 4, 3, 5));
        let missing = wizard_aligned_key("crop_missing_probe");
        let cropped_keys = [wizard_cropped_key("crop_present"), wizard_cropped_key("crop_missing_probe")];

        let auto = crop_channels_cmd(
            vec![present.clone(), missing.clone()],
            out.clone(),
            0,
            0,
            0,
            0,
            Some(true),
            Some(vec!["crop_present".to_string(), "crop_missing_probe".to_string()]),
            None,
            None,
        )
        .await;
        let manual = crop_channels_cmd(
            vec![missing.clone()],
            out.clone(),
            2,
            4,
            3,
            5,
            Some(false),
            Some(vec!["crop_missing_probe".to_string()]),
            None,
            None,
        )
        .await;
        let missing_file = crop_channels_cmd(
            vec!["C:/astrokit-missing/crop_probe.fits".to_string()],
            out.clone(),
            2,
            4,
            3,
            5,
            Some(false),
            None,
            None,
            None,
        )
        .await;
        let left_cropped_entries = GLOBAL_IMAGE_CACHE.any_key(|k| cropped_keys.iter().any(|c| c.as_str() == k));
        GLOBAL_IMAGE_CACHE.remove(&present);

        assert_eq!(auto.expect_err("a missing wizard key must fail in auto mode"), ALIGNED_GONE);
        assert_eq!(manual.expect_err("a missing wizard key must fail in manual mode"), ALIGNED_GONE);
        assert_ne!(missing_file.expect_err("a missing file must fail"), ALIGNED_GONE);
        assert!(!left_cropped_entries, "a crop that fails to load must not leave cropped entries behind");
        assert!(!dir.path().join("out").exists(), "a crop that fails to load must not create the output dir");
    }

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

        let res = crop_channels_cmd(vec![src.clone()], out.to_str().unwrap().to_string(), 2, 4, 3, 5, Some(false), None, None, None)
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

    #[tokio::test]
    async fn a_wizard_crop_carries_the_input_header_moved_to_the_crop_origin() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let input = crate::types::constants::wizard_aligned_key("crop_r");
        let mut header = HduHeader::empty();
        header.set("CTYPE1", "RA---TAN".to_string());
        header.set("CTYPE2", "DEC--TAN".to_string());
        header.set_f64("CRVAL1", 150.0);
        header.set_f64("CRVAL2", 2.0);
        header.set_f64("CRPIX1", 20.0);
        header.set_f64("CRPIX2", 30.0);
        header.set_f64("CD1_1", -1e-5);
        header.set_f64("CD2_2", 1e-5);
        header.set("FILTER", "F200W".to_string());
        let arr = Array2::from_elem((40, 50), 5.0f32);
        GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(&input, Arc::new(arr.clone()), compute_image_stats(&arr), Some(header.clone()));

        let res = crop_channels_cmd(
            vec![input.clone()],
            dir.path().join("out").to_str().unwrap().to_string(),
            2,
            4,
            3,
            5,
            Some(false),
            Some(vec!["crop_r".to_string()]),
            None,
            None,
        )
        .await
        .unwrap();
        let key = res[RES_CACHE_KEYS][0].as_str().unwrap().to_string();
        let entry = GLOBAL_IMAGE_CACHE.get(&key);
        GLOBAL_IMAGE_CACHE.remove(&input);
        GLOBAL_IMAGE_CACHE.remove(&key);
        assert_eq!(key, crate::types::constants::wizard_cropped_key("crop_r"));
        assert_eq!(res[RES_PATHS][0], key);
        assert!(!dir.path().join("out").exists(), "a wizard crop must not write to the disk");
        let entry = entry.expect("cropped wizard entry");
        assert_eq!(entry.arr().dim(), (34, 42));
        let h = entry.header().cloned().expect("a cropped wizard channel carries the header of its grid");
        assert!((h.get_f64("CRPIX1").unwrap() - 17.0).abs() < 1e-9, "CRPIX1 {:?}", h.get("CRPIX1"));
        assert!((h.get_f64("CRPIX2").unwrap() - 28.0).abs() < 1e-9, "CRPIX2 {:?}", h.get("CRPIX2"));
        assert_eq!(h.get_f64("LTV1"), Some(-3.0));
        assert_eq!(h.get_f64("LTV2"), Some(-2.0));
        assert_eq!(h.get_i64("NAXIS1"), Some(42));
        assert_eq!(h.get_i64("NAXIS2"), Some(34));
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

    #[tokio::test]
    async fn cropped_keys_carry_the_run_token() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let input = seed_wizard_entry("crop_tok", &nan_bordered(40, 50, 2, 4, 3, 5));
        *super::super::blend::latest_wizard_run() = Some("t1".to_string());

        let res = crop_channels_cmd(
            vec![input.clone()],
            "unused".to_string(),
            0,
            0,
            0,
            0,
            Some(true),
            Some(vec!["crop_tok".to_string()]),
            None,
            Some(" t1 ".to_string()),
        )
        .await
        .unwrap();
        let key = res[RES_CACHE_KEYS][0].as_str().unwrap().to_string();
        let present = GLOBAL_IMAGE_CACHE.contains(&key);
        GLOBAL_IMAGE_CACHE.remove(&input);
        GLOBAL_IMAGE_CACHE.remove(&key);
        GLOBAL_IMAGE_CACHE.remove(&wizard_cropped_key("crop_tok"));

        assert_eq!(key, "__wizard_ch_tt1_crop_tok_cropped");
        assert_eq!(key, crate::types::constants::wizard_cropped_key_for(Some("t1"), "crop_tok"));
        assert_eq!(res[RES_PATHS][0], key);
        assert!(present, "the cropped entry must live under the run key");
    }

    #[tokio::test]
    async fn a_late_crop_removes_its_own_keys_when_a_newer_run_started() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let input = seed_wizard_entry("crop_late", &nan_bordered(40, 50, 2, 4, 3, 5));
        *super::super::blend::latest_wizard_run() = Some("newer".to_string());

        let res = crop_channels_cmd(
            vec![input.clone()],
            "unused".to_string(),
            0,
            0,
            0,
            0,
            Some(true),
            Some(vec!["crop_late".to_string()]),
            None,
            Some("t1".to_string()),
        )
        .await
        .unwrap();
        let key = res[RES_CACHE_KEYS][0].as_str().unwrap().to_string();
        let stale_left = GLOBAL_IMAGE_CACHE.contains(&key);
        GLOBAL_IMAGE_CACHE.remove(&input);
        GLOBAL_IMAGE_CACHE.remove(&key);

        assert_eq!(key, "__wizard_ch_tt1_crop_late_cropped");
        assert!(!stale_left, "a crop that finishes after a newer run started must remove its own keys");
    }

    #[test]
    fn manual_crop_rejects_excessive_margins() {
        assert_eq!(manual_crop_bounds(100, 100, 10, 10, 10, 10), Some((10, 90, 10, 90)));
        assert_eq!(manual_crop_bounds(100, 100, 60, 60, 0, 0), None);
        assert_eq!(manual_crop_bounds(100, 100, 0, 0, 70, 70), None);
        assert_eq!(manual_crop_bounds(100, 100, 50, 50, 0, 0), None);
    }
}
