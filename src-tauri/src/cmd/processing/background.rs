use std::sync::Arc;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, derived_output_header, load_from_cache_or_disk, output_stem, resolve_output_dir, save_auto_stf_preview_png, write_derived_fits, OutputValues};
use crate::cmd::processing::local_contrast::source_header;
use crate::core::imaging::background::{extract_background, extract_background_linked, neutralize_background, deband, deband_axis_name, detect_band_axis, DebandAxis, DebandConfig, BackgroundConfig, BackgroundMode};
use crate::core::imaging::stats::compute_image_stats;
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    MODE_DIVIDE, PROGRESS_EVENT, PROGRESS_STEPS,
    MIN_GRID_SIZE, MAX_GRID_SIZE, MIN_POLY_DEGREE, MAX_POLY_DEGREE, MIN_ITERATIONS, MAX_ITERATIONS,
    RES_CORRECTED_PNG, RES_MODEL_PNG, RES_CORRECTED_FITS, RES_SAMPLE_COUNT,
    RES_RMS_RESIDUAL, RES_ELAPSED_MS, RES_DIMENSIONS, RES_CACHE_KEY,
};
use crate::types::image_ref::sanitize_fragment;

const ABPROC_BG_CORRECTED: &str = "bg_corrected";

fn background_config(
    grid_size: usize,
    poly_degree: usize,
    sigma_clip: f64,
    iterations: usize,
    mode: &str,
) -> anyhow::Result<BackgroundConfig> {
    let clip = sigma_clip as f32;
    if !(clip.is_finite() && clip > 0.0) {
        anyhow::bail!("sigma_clip must be a finite number greater than 0, got {}", sigma_clip);
    }
    Ok(BackgroundConfig {
        grid_size: grid_size.clamp(MIN_GRID_SIZE, MAX_GRID_SIZE),
        poly_degree: poly_degree.clamp(MIN_POLY_DEGREE, MAX_POLY_DEGREE),
        sigma_clip: clip,
        iterations: iterations.clamp(MIN_ITERATIONS, MAX_ITERATIONS),
        mode: match mode {
            MODE_DIVIDE => BackgroundMode::Divide,
            _ => BackgroundMode::Subtract,
        },
    })
}

fn background_png_paths(output_dir: &str, stem: &str, bin_id: Option<&str>) -> (String, String) {
    let base = match bin_id {
        Some(bid) => format!("{}_wizard_{}", stem, sanitize_fragment(bid)),
        None => stem.to_string(),
    };
    (
        format!("{}/{}_bg_corrected.png", output_dir, base),
        format!("{}/{}_bg_model.png", output_dir, base),
    )
}

fn run_extract_background(
    path: &str,
    output_dir: &str,
    config: &BackgroundConfig,
    bin_id: Option<&str>,
    progress: Option<&ProgressHandle>,
) -> anyhow::Result<serde_json::Value> {
    let entry = load_from_cache_or_disk(path)?;
    let bg_result = extract_background(entry.arr(), config, progress)?;

    let stem = output_stem(path);
    let (corrected_png, model_png) = background_png_paths(output_dir, &stem, bin_id);
    let (rows, cols) = bg_result.corrected.dim();
    save_auto_stf_preview_png(&bg_result.corrected, &corrected_png)?;
    save_auto_stf_preview_png(&bg_result.model, &model_png)?;

    let corrected_fits = format!("{}/{}_bg_corrected.fits", output_dir, stem);
    let cache_key = match bin_id {
        Some(bid) => crate::types::constants::wizard_bg_key(bid),
        None => {
            let header = derived_output_header(source_header(path, &entry).as_ref(), ABPROC_BG_CORRECTED, OutputValues::Linear);
            write_derived_fits(&corrected_fits, &bg_result.corrected, Some(&header))?;
            corrected_fits
        }
    };

    let stats = compute_image_stats(&bg_result.corrected);
    GLOBAL_IMAGE_CACHE.insert_synthetic(&cache_key, Arc::new(bg_result.corrected), stats);

    Ok(json!({
        RES_CORRECTED_PNG: corrected_png,
        RES_MODEL_PNG: model_png,
        RES_CORRECTED_FITS: cache_key,
        RES_CACHE_KEY: cache_key,
        RES_SAMPLE_COUNT: bg_result.sample_count,
        RES_RMS_RESIDUAL: bg_result.rms_residual,
        RES_ELAPSED_MS: bg_result.elapsed_ms,
        RES_DIMENSIONS: [cols, rows],
    }))
}

#[tauri::command]
pub async fn extract_background_cmd(
    app: tauri::AppHandle,
    path: String,
    output_dir: String,
    grid_size: usize,
    poly_degree: usize,
    sigma_clip: f64,
    iterations: usize,
    mode: String,
    bin_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let progress = ProgressHandle::new(&app, PROGRESS_EVENT, PROGRESS_STEPS as u64);

    blocking_cmd!({
        let config = background_config(grid_size, poly_degree, sigma_clip, iterations, &mode)?;
        let output_dir = resolve_output_dir(&output_dir)?;
        run_extract_background(&path, &output_dir, &config, bin_id.as_deref(), Some(&progress))
    })
}

fn mean_reference(channels: &[Array2<f32>]) -> Array2<f32> {
    let (rows, cols) = channels[0].dim();
    Array2::from_shape_fn((rows, cols), |(y, x)| {
        let mut sum = 0.0f32;
        let mut cnt = 0u32;
        for ch in channels {
            let v = ch[[y, x]];
            if v.is_finite() {
                sum += v;
                cnt += 1;
            }
        }
        if cnt > 0 {
            sum / cnt as f32
        } else {
            f32::NAN
        }
    })
}

#[tauri::command]
pub async fn extract_background_batch_cmd(
    paths: Vec<String>,
    bin_ids: Vec<String>,
    output_dir: String,
    grid_size: usize,
    poly_degree: usize,
    sigma_clip: f64,
    iterations: usize,
    mode: String,
    reference_bin: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = std::time::Instant::now();
        let config = background_config(grid_size, poly_degree, sigma_clip, iterations, &mode)?;
        resolve_output_dir(&output_dir)?;

        if paths.is_empty() || paths.len() != bin_ids.len() {
            anyhow::bail!("paths and bin_ids must be non-empty and of equal length");
        }

        let deband_mode = matches!(
            mode.as_str(),
            "deband_rows" | "deband_cols" | "deband_both" | "deband_auto"
        );
        let neutralize = mode.as_str() == "neutralize";

        let mut loaded: Vec<Array2<f32>> = Vec::with_capacity(paths.len());
        for p in &paths {
            let entry = load_from_cache_or_disk(p)?;
            loaded.push(entry.arr().to_owned());
        }

        let (rows, cols) = loaded[0].dim();
        for ch in loaded.iter_mut().skip(1) {
            if ch.dim() != (rows, cols) {
                *ch = crate::core::imaging::resample::resample_image(ch, rows, cols)?;
            }
        }

        let mut axes: Vec<Option<&'static str>> = vec![None; loaded.len()];

        let (corrected, sample_counts, rms_residual): (Vec<Array2<f32>>, Vec<Option<usize>>, Option<f64>) = if deband_mode {
            let base_cfg = DebandConfig {
                axis: DebandAxis::Both,
                sigma_clip: config.sigma_clip,
                iterations: iterations.clamp(1, 5),
            };
            let mut out = Vec::with_capacity(loaded.len());
            for (i, ch) in loaded.iter().enumerate() {
                let axis = match mode.as_str() {
                    "deband_rows" => DebandAxis::Rows,
                    "deband_cols" => DebandAxis::Columns,
                    "deband_both" => DebandAxis::Both,
                    _ => detect_band_axis(ch, &base_cfg),
                };
                axes[i] = Some(deband_axis_name(axis));
                let dcfg = DebandConfig { axis, ..base_cfg.clone() };
                out.push(deband(ch, &dcfg));
            }
            let counts = vec![None; loaded.len()];
            (out, counts, None)
        } else if neutralize {
            let mut out = Vec::with_capacity(loaded.len());
            let mut counts = Vec::with_capacity(loaded.len());
            for ch in &loaded {
                let r = neutralize_background(ch, &config)?;
                counts.push(Some(r.sample_count));
                out.push(r.corrected);
            }
            (out, counts, None)
        } else {
            let reference = match reference_bin
                .as_ref()
                .and_then(|rb| bin_ids.iter().position(|b| b == rb))
            {
                Some(idx) => loaded[idx].clone(),
                None => mean_reference(&loaded),
            };
            let refs: Vec<&Array2<f32>> = loaded.iter().collect();
            let linked = extract_background_linked(&refs, &reference, &config)?;
            let count = linked.sample_count;
            (linked.corrected, vec![Some(count); loaded.len()], Some(linked.rms_residual))
        };

        let mut results = Vec::with_capacity(paths.len());
        for (i, bin_id) in bin_ids.iter().enumerate() {
            let img = &corrected[i];
            let cache_key = crate::types::constants::wizard_bg_key(bin_id);
            let stats = compute_image_stats(img);
            GLOBAL_IMAGE_CACHE.insert_synthetic(&cache_key, Arc::new(img.clone()), stats);
            results.push(json!({
                "bin_id": bin_id,
                RES_CACHE_KEY: cache_key,
                RES_SAMPLE_COUNT: sample_counts[i],
                "axis": axes[i],
            }));
        }

        Ok(json!({
            "results": results,
            "mode": mode,
            RES_RMS_RESIDUAL: rms_residual,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::constants::wizard_bg_key;

    fn sky(seed: usize) -> Array2<f32> {
        Array2::from_shape_fn((64, 64), |(y, x)| {
            let noise = ((y * 31 + x * 17 + seed * 7) % 23) as f32 - 11.0;
            100.0 + 0.5 * x as f32 + 0.25 * y as f32 + noise
        })
    }

    async fn run_batch(mode: &str, tag: &str) -> serde_json::Value {
        let dir = tempfile::tempdir().unwrap();
        let bins: Vec<String> = ["r", "g"].iter().map(|c| format!("bg_batch_{tag}_{c}")).collect();
        let inputs: Vec<String> = bins.iter().map(|b| format!("__bg_batch_input_{b}")).collect();
        for (i, key) in inputs.iter().enumerate() {
            let arr = sky(i);
            GLOBAL_IMAGE_CACHE.insert_synthetic(key, Arc::new(arr.clone()), compute_image_stats(&arr));
        }
        let value = extract_background_batch_cmd(
            inputs.clone(),
            bins.clone(),
            dir.path().to_str().unwrap().to_string(),
            8,
            2,
            3.0,
            3,
            mode.to_string(),
            None,
        )
        .await;
        for key in inputs.iter().cloned().chain(bins.iter().map(|b| wizard_bg_key(b))) {
            GLOBAL_IMAGE_CACHE.remove(&key);
        }
        value.unwrap()
    }

    fn sky_file(dir: &tempfile::TempDir, name: &str) -> String {
        let path = dir.path().join(name).to_str().unwrap().to_string();
        let mut header = crate::types::header::HduHeader::empty();
        for (k, v) in [("BUNIT", "'MJy/sr'"), ("PHOTMJSR", "1.5"), ("CTYPE1", "'RA---TAN'"), ("CRVAL1", "83.8")] {
            header.set(k, v.to_string());
        }
        crate::infra::fits::writer::write_fits_mono(&path, &sky(0), Some(&header)).unwrap();
        path
    }

    fn config(grid_size: usize) -> BackgroundConfig {
        background_config(grid_size, 2, 3.0, 3, "subtract").unwrap()
    }

    #[tokio::test]
    async fn sigma_clip_values_that_would_poison_the_sky_level_are_refused() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e300, 1e-60] {
            let err = background_config(8, 2, bad, 3, "subtract").unwrap_err().to_string();
            assert!(err.contains("sigma_clip"), "{bad}: {err}");
        }
        assert_eq!(background_config(8, 2, 2.5, 3, "divide").unwrap().sigma_clip, 2.5);

        let dir = tempfile::tempdir().unwrap();
        let key = "__bg_bad_sigma_input".to_string();
        let arr = sky(3);
        GLOBAL_IMAGE_CACHE.insert_synthetic(&key, Arc::new(arr.clone()), compute_image_stats(&arr));
        let refused = extract_background_batch_cmd(
            vec![key.clone()],
            vec!["bg_bad_sigma".to_string()],
            dir.path().to_str().unwrap().to_string(),
            8,
            2,
            0.0,
            3,
            "subtract".to_string(),
            None,
        )
        .await;
        GLOBAL_IMAGE_CACHE.remove(&key);
        GLOBAL_IMAGE_CACHE.remove(&wizard_bg_key("bg_bad_sigma"));
        assert!(refused.unwrap_err().contains("sigma_clip"));
    }

    #[test]
    fn the_background_corrected_fits_keeps_the_source_calibration_and_wcs() {
        let dir = tempfile::tempdir().unwrap();
        let src = sky_file(&dir, "bg_calibrated.fits");
        let value = run_extract_background(&src, dir.path().to_str().unwrap(), &config(8), None, None).unwrap();
        let fits = value[RES_CORRECTED_FITS].as_str().unwrap().to_string();
        GLOBAL_IMAGE_CACHE.remove(&fits);
        let header = crate::cmd::common::cached_header(&fits).unwrap();
        assert_eq!(header.get("BUNIT").map(|v| v.trim().trim_matches('\'').trim()), Some("MJy/sr"));
        assert_eq!(header.get_f64("PHOTMJSR"), Some(1.5));
        assert_eq!(header.get_f64("CRVAL1"), Some(83.8));
        assert_eq!(
            header.get(crate::cmd::common::HEADER_ABPROC).map(|v| v.trim().trim_matches('\'').trim()),
            Some(ABPROC_BG_CORRECTED)
        );
    }

    #[test]
    fn a_wizard_extraction_leaves_the_processing_previews_of_the_same_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap();
        let src = sky_file(&dir, "M42.fits");
        let processing = run_extract_background(&src, out, &config(16), None, None).unwrap();
        let corrected_png = processing[RES_CORRECTED_PNG].as_str().unwrap().to_string();
        let model_png = processing[RES_MODEL_PNG].as_str().unwrap().to_string();
        let before = (std::fs::read(&corrected_png).unwrap(), std::fs::read(&model_png).unwrap());

        let wizard = run_extract_background(&src, out, &config(8), Some("r"), None).unwrap();
        GLOBAL_IMAGE_CACHE.remove(processing[RES_CORRECTED_FITS].as_str().unwrap());
        GLOBAL_IMAGE_CACHE.remove(&wizard_bg_key("r"));

        for key in [RES_CORRECTED_PNG, RES_MODEL_PNG] {
            let path = wizard[key].as_str().unwrap();
            assert_ne!(path, processing[key].as_str().unwrap(), "{key}");
            assert!(path.contains("_wizard_r_"), "{path}");
            assert!(std::path::Path::new(path).exists(), "{path}");
        }
        let after = (std::fs::read(&corrected_png).unwrap(), std::fs::read(&model_png).unwrap());
        assert!(before == after, "the wizard run overwrote the Processing Background previews");
    }

    #[test]
    fn background_previews_of_a_large_image_match_the_gpu_view_of_the_written_result() {
        use crate::cmd::io::test_support::{assert_matches_gpu_view, wide_sky};
        use crate::core::imaging::stf::{auto_stf, AutoStfConfig};
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("wide_bg.fits").to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&src, &wide_sky(32), None).unwrap();

        let value = run_extract_background(&src, dir.path().to_str().unwrap(), &config(8), None, None).unwrap();
        let fits = value[RES_CORRECTED_FITS].as_str().unwrap().to_string();
        GLOBAL_IMAGE_CACHE.remove(&fits);
        let corrected = crate::cmd::common::extract_image_resolved(&fits).unwrap().arr;
        let stats = compute_image_stats(&corrected);
        let stf = auto_stf(&stats, &AutoStfConfig::default());
        assert_matches_gpu_view(value[RES_CORRECTED_PNG].as_str().unwrap(), &corrected, &stf, &stats);
    }

    #[tokio::test]
    async fn modes_that_fit_no_model_report_no_residual_instead_of_a_perfect_one() {
        let deband = run_batch("deband_rows", "deband").await;
        assert!(deband[RES_RMS_RESIDUAL].is_null(), "{deband}");
        for r in deband["results"].as_array().unwrap() {
            assert!(r[RES_SAMPLE_COUNT].is_null(), "de-band measured no samples: {r}");
        }

        let neutralize = run_batch("neutralize", "neutralize").await;
        assert!(neutralize[RES_RMS_RESIDUAL].is_null(), "{neutralize}");
        for r in neutralize["results"].as_array().unwrap() {
            assert!(r[RES_SAMPLE_COUNT].as_u64().is_some(), "{r}");
        }

        let fitted = run_batch("subtract", "fitted").await;
        assert!(fitted[RES_RMS_RESIDUAL].as_f64().is_some_and(|v| v > 0.0), "{fitted}");
    }
}
