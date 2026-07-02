use std::sync::Arc;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, resolve_output_dir, save_preview_png, auto_stretch_preview};
use crate::core::imaging::background::{extract_background, extract_background_linked, neutralize_background, deband, DebandAxis, DebandConfig, BackgroundConfig, BackgroundMode};
use crate::core::imaging::stats::compute_image_stats;
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    MODE_DIVIDE, PROGRESS_EVENT, PROGRESS_STEPS, DEFAULT_STEM,
    MIN_GRID_SIZE, MAX_GRID_SIZE, MIN_POLY_DEGREE, MAX_POLY_DEGREE, MIN_ITERATIONS, MAX_ITERATIONS,
    RES_CORRECTED_PNG, RES_MODEL_PNG, RES_CORRECTED_FITS, RES_SAMPLE_COUNT,
    RES_RMS_RESIDUAL, RES_ELAPSED_MS, RES_DIMENSIONS, RES_CACHE_KEY,
};

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
    persist_to_disk: Option<bool>,
) -> Result<serde_json::Value, String> {
    let progress = ProgressHandle::new(&app, PROGRESS_EVENT, PROGRESS_STEPS as u64);
    let progress_clone = progress.clone();

    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let entry = load_from_cache_or_disk(&path)?;

        let bg_mode = match mode.as_str() {
            MODE_DIVIDE => BackgroundMode::Divide,
            _ => BackgroundMode::Subtract,
        };

        let config = BackgroundConfig {
            grid_size: grid_size.clamp(MIN_GRID_SIZE, MAX_GRID_SIZE),
            poly_degree: poly_degree.clamp(MIN_POLY_DEGREE, MAX_POLY_DEGREE),
            sigma_clip: sigma_clip as f32,
            iterations: iterations.clamp(MIN_ITERATIONS, MAX_ITERATIONS),
            mode: bg_mode,
        };

        let bg_result = extract_background(entry.arr(), &config, Some(&progress_clone))?;

        let rendered = auto_stretch_preview(&bg_result.corrected);
        let model_rendered = auto_stretch_preview(&bg_result.model);

        let stem = std::path::Path::new(&path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(DEFAULT_STEM);

        let corrected_png = format!("{}/{}_bg_corrected.png", output_dir, stem);
        let model_png = format!("{}/{}_bg_model.png", output_dir, stem);

        let (rows, cols) = bg_result.corrected.dim();
        save_preview_png(rendered, cols, rows, &corrected_png)?;
        save_preview_png(model_rendered, cols, rows, &model_png)?;

        let cache_key = match &bin_id {
            Some(bid) => crate::types::constants::wizard_bg_key(bid),
            None => format!("{}/{}_bg_corrected.fits", output_dir, stem),
        };

        let stats = compute_image_stats(&bg_result.corrected);

        let write_disk = persist_to_disk.unwrap_or(false);
        if write_disk && bin_id.is_none() {
            let fits_path = format!("{}/{}_bg_corrected.fits", output_dir, stem);
            crate::infra::fits::writer::write_fits_mono(&fits_path, &bg_result.corrected, None)?;
        }

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
        resolve_output_dir(&output_dir)?;

        if paths.is_empty() || paths.len() != bin_ids.len() {
            anyhow::bail!("paths and bin_ids must be non-empty and of equal length");
        }

        let deband_axis = match mode.as_str() {
            "deband_rows" => Some(DebandAxis::Rows),
            "deband_cols" => Some(DebandAxis::Columns),
            "deband_both" => Some(DebandAxis::Both),
            _ => None,
        };
        let neutralize = mode.as_str() == "neutralize";

        let bg_mode = match mode.as_str() {
            MODE_DIVIDE => BackgroundMode::Divide,
            _ => BackgroundMode::Subtract,
        };

        let config = BackgroundConfig {
            grid_size: grid_size.clamp(MIN_GRID_SIZE, MAX_GRID_SIZE),
            poly_degree: poly_degree.clamp(MIN_POLY_DEGREE, MAX_POLY_DEGREE),
            sigma_clip: sigma_clip as f32,
            iterations: iterations.clamp(MIN_ITERATIONS, MAX_ITERATIONS),
            mode: bg_mode,
        };

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

        let (corrected, sample_counts, rms_residual): (Vec<Array2<f32>>, Vec<usize>, f64) = if let Some(axis) = deband_axis {
            let dcfg = DebandConfig {
                axis,
                sigma_clip: sigma_clip as f32,
                iterations: iterations.clamp(1, 5),
            };
            let out: Vec<Array2<f32>> = loaded.iter().map(|ch| deband(ch, &dcfg)).collect();
            let counts = vec![0usize; loaded.len()];
            (out, counts, 0.0)
        } else if neutralize {
            let mut out = Vec::with_capacity(loaded.len());
            let mut counts = Vec::with_capacity(loaded.len());
            for ch in &loaded {
                let r = neutralize_background(ch, &config)?;
                counts.push(r.sample_count);
                out.push(r.corrected);
            }
            (out, counts, 0.0)
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
            (linked.corrected, vec![count; loaded.len()], linked.rms_residual)
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
