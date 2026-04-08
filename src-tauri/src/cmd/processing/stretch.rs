use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, render_and_save, resolve_output_dir, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::imaging::stretch::{arcsinh_stretch, arcsinh_stretch_rgb};
use crate::core::imaging::masked_stretch::{masked_stretch, masked_stretch_rgb_shared, MaskedStretchConfig};
use crate::types::constants::{
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_PNG_PATH,
    RES_STRETCH_FACTOR, RES_ITERATIONS_RUN, RES_STARS_MASKED,
    RES_MASK_COVERAGE, RES_FINAL_BACKGROUND, RES_CONVERGED,
    SUFFIX_MASKED_STRETCH,
};

#[tauri::command]
pub async fn apply_arcsinh_stretch_cmd(
    path: String,
    output_dir: String,
    factor: f64,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let entry = load_from_cache_or_disk(&path)?;
        let image = entry.arr();

        let clamped_factor = (factor as f32).clamp(1.0, 500.0);

        let t0 = std::time::Instant::now();
        let stretched = arcsinh_stretch(image, clamped_factor);
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let ro = render_and_save(&stretched, &path, &output_dir, "arcsinh", true)?;
        let (rows, cols) = ro.dims;

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: ro.fits_path,
            RES_STRETCH_FACTOR: clamped_factor,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

#[tauri::command]
pub async fn masked_stretch_cmd(
    path: String,
    output_dir: String,
    iterations: Option<usize>,
    target_background: Option<f64>,
    mask_growth: Option<f64>,
    mask_softness: Option<f64>,
    protection_amount: Option<f64>,
    luminance_protect: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let entry = load_from_cache_or_disk(&path)?;
        let image = entry.arr();

        let config = MaskedStretchConfig {
            iterations: iterations.unwrap_or(10),
            target_background: target_background.unwrap_or(0.25),
            mask_growth: mask_growth.unwrap_or(2.5),
            mask_softness: mask_softness.unwrap_or(4.0),
            protection_amount: protection_amount.unwrap_or(0.85),
            luminance_protect: luminance_protect.unwrap_or(true),
            ..MaskedStretchConfig::default()
        };

        let t0 = std::time::Instant::now();
        let result = masked_stretch(image, &config).map_err(|e| anyhow::anyhow!(e))?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let ro = render_and_save(&result.image, &path, &output_dir, SUFFIX_MASKED_STRETCH, true)?;
        let (rows, cols) = ro.dims;

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: ro.fits_path,
            RES_ITERATIONS_RUN: result.iterations_run,
            RES_FINAL_BACKGROUND: result.final_background,
            RES_STARS_MASKED: result.stars_masked,
            RES_MASK_COVERAGE: result.mask_coverage,
            RES_CONVERGED: result.converged,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

#[tauri::command]
pub async fn arcsinh_stretch_composite_cmd(
    output_dir: String,
    factor: f64,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let (er, eg, eb) = helpers::load_composite_rgb()?;
        let clamped_factor = (factor as f32).clamp(1.0, 500.0);

        let t0 = std::time::Instant::now();
        let (r, g, b) = arcsinh_stretch_rgb(er.arr(), eg.arr(), eb.arr(), clamped_factor);
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let (rows, cols) = r.dim();

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let png_path = format!("{}/composite_arcsinh_{}.png", output_dir, ts);
        helpers::render_rgb_preview(&r, &g, &b, &png_path, MAX_PREVIEW_DIM)?;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_STRETCH_FACTOR: clamped_factor,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

fn channel_stats_json(r: &crate::core::imaging::masked_stretch::MaskedStretchResult) -> serde_json::Value {
    json!({
        RES_ITERATIONS_RUN: r.iterations_run,
        RES_FINAL_BACKGROUND: r.final_background,
        RES_CONVERGED: r.converged,
    })
}

#[tauri::command]
pub async fn masked_stretch_composite_cmd(
    output_dir: String,
    iterations: Option<usize>,
    target_background: Option<f64>,
    mask_growth: Option<f64>,
    mask_softness: Option<f64>,
    protection_amount: Option<f64>,
    luminance_protect: Option<bool>,
    shared_mask: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let (er, eg, eb) = helpers::load_composite_rgb()?;

        let config = MaskedStretchConfig {
            iterations: iterations.unwrap_or(10),
            target_background: target_background.unwrap_or(0.25),
            mask_growth: mask_growth.unwrap_or(2.5),
            mask_softness: mask_softness.unwrap_or(4.0),
            protection_amount: protection_amount.unwrap_or(0.85),
            luminance_protect: luminance_protect.unwrap_or(true),
            ..MaskedStretchConfig::default()
        };

        let t0 = std::time::Instant::now();
        let use_shared = shared_mask.unwrap_or(false);

        let (r_img, g_img, b_img, per_channel, stars, coverage, mask_mode) = if use_shared {
            let result = masked_stretch_rgb_shared(er.arr(), eg.arr(), eb.arr(), &config)
                .map_err(|e| anyhow::anyhow!(e))?;
            let pc = json!({
                "r": channel_stats_json(&result.r),
                "g": channel_stats_json(&result.g),
                "b": channel_stats_json(&result.b),
            });
            (
                result.r.image, result.g.image, result.b.image,
                pc, result.shared_stars_masked, result.shared_mask_coverage,
                "shared_luminance",
            )
        } else {
            let (res_r, (res_g, res_b)) = rayon::join(
                || masked_stretch(er.arr(), &config),
                || rayon::join(
                    || masked_stretch(eg.arr(), &config),
                    || masked_stretch(eb.arr(), &config),
                ),
            );
            let r = res_r.map_err(|e| anyhow::anyhow!(e))?;
            let g = res_g.map_err(|e| anyhow::anyhow!(e))?;
            let b = res_b.map_err(|e| anyhow::anyhow!(e))?;
            let pc = json!({
                "r": channel_stats_json(&r),
                "g": channel_stats_json(&g),
                "b": channel_stats_json(&b),
            });
            let total_stars = r.stars_masked + g.stars_masked + b.stars_masked;
            let avg_coverage = (r.mask_coverage + g.mask_coverage + b.mask_coverage) / 3.0;
            (
                r.image, g.image, b.image,
                pc, total_stars, avg_coverage,
                "per_channel",
            )
        };

        let elapsed_ms = t0.elapsed().as_millis() as u64;
        let (rows, cols) = r_img.dim();

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let png_path = format!("{}/composite_masked_{}.png", output_dir, ts);
        helpers::render_rgb_preview(&r_img, &g_img, &b_img, &png_path, MAX_PREVIEW_DIM)?;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_STARS_MASKED: stars,
            RES_MASK_COVERAGE: coverage,
            "channels": per_channel,
            "mask_mode": mask_mode,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}
