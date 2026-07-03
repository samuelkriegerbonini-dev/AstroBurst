use std::sync::Arc;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, render_and_save, resolve_output_dir, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::imaging::star_removal::{remove_stars, remove_stars_rgb_shared, StarRemovalConfig};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{make_stf_u8_fn, AutoStfConfig};
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::types::constants::{
    COMPOSITE_KEY_B, COMPOSITE_KEY_G, COMPOSITE_KEY_R,
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_MASK_COVERAGE, RES_PNG_PATH,
    RES_STARS_FITS_PATH, RES_STARS_MASKED, RES_STARS_PNG_PATH,
};

fn config_from_args(
    detection_sigma: Option<f64>,
    max_eccentricity: Option<f64>,
    growth_factor: Option<f64>,
    softness: Option<f64>,
    bright_ceiling: Option<f64>,
) -> StarRemovalConfig {
    let defaults = StarRemovalConfig::default();
    StarRemovalConfig {
        detection_sigma: detection_sigma.unwrap_or(defaults.detection_sigma).clamp(2.0, 20.0),
        max_eccentricity: max_eccentricity.unwrap_or(defaults.max_eccentricity).clamp(0.3, 1.0),
        growth_factor: growth_factor.unwrap_or(defaults.growth_factor).clamp(1.0, 10.0),
        softness: softness.unwrap_or(defaults.softness).clamp(0.0, 20.0),
        bright_ceiling: bright_ceiling.unwrap_or(defaults.bright_ceiling).clamp(0.5, 1.0),
    }
}

#[tauri::command]
pub async fn remove_stars_cmd(
    path: String,
    output_dir: String,
    detection_sigma: Option<f64>,
    max_eccentricity: Option<f64>,
    growth_factor: Option<f64>,
    softness: Option<f64>,
    bright_ceiling: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let entry = load_from_cache_or_disk(&path)?;
        let image = entry.arr();

        let config = config_from_args(
            detection_sigma,
            max_eccentricity,
            growth_factor,
            softness,
            bright_ceiling,
        );

        let t0 = std::time::Instant::now();
        let result = remove_stars(image, &config).map_err(|e| anyhow::anyhow!(e))?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let starless_out = render_and_save(&result.starless, &path, &output_dir, "starless", true)?;
        let stars_out = render_and_save(&result.stars, &path, &output_dir, "stars", true)?;
        let (rows, cols) = starless_out.dims;

        Ok(json!({
            RES_PNG_PATH: starless_out.png_path,
            RES_FITS_PATH: starless_out.fits_path,
            RES_STARS_PNG_PATH: stars_out.png_path,
            RES_STARS_FITS_PATH: stars_out.fits_path,
            RES_STARS_MASKED: result.stars_removed,
            RES_MASK_COVERAGE: result.mask_coverage,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

#[tauri::command]
pub async fn remove_stars_composite_cmd(
    output_dir: String,
    detection_sigma: Option<f64>,
    max_eccentricity: Option<f64>,
    growth_factor: Option<f64>,
    softness: Option<f64>,
    bright_ceiling: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let (er, eg, eb) = helpers::load_composite_rgb()?;

        let config = config_from_args(
            detection_sigma,
            max_eccentricity,
            growth_factor,
            softness,
            bright_ceiling,
        );

        let t0 = std::time::Instant::now();
        let result = remove_stars_rgb_shared(er.arr(), eg.arr(), eb.arr(), &config)
            .map_err(|e| anyhow::anyhow!(e))?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let (rows, cols) = result.r.starless.dim();

        let stats_r = compute_image_stats(&result.r.starless);
        let stats_g = compute_image_stats(&result.g.starless);
        let stats_b = compute_image_stats(&result.b.starless);

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let png_path = format!("{}/composite_starless_{}.png", output_dir, ts);
        let stars_png_path = format!("{}/composite_stars_{}.png", output_dir, ts);

        let stf_config = AutoStfConfig::default();
        let (linked_stf, combined_stats) =
            helpers::compute_linked_stf_with_stats(&stats_r, &stats_g, &stats_b, &stf_config);
        let fn_r = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_g = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_b = make_stf_u8_fn(&linked_stf, &combined_stats);
        helpers::render_rgb_preview_with_stf(
            &result.r.starless,
            &result.g.starless,
            &result.b.starless,
            fn_r,
            fn_g,
            fn_b,
            &png_path,
            MAX_PREVIEW_DIM,
        )?;

        helpers::render_rgb_preview(
            &result.r.stars,
            &result.g.stars,
            &result.b.stars,
            &stars_png_path,
            MAX_PREVIEW_DIM,
        )?;

        GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_R, Arc::new(result.r.starless), stats_r);
        GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_G, Arc::new(result.g.starless), stats_g);
        GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_B, Arc::new(result.b.starless), stats_b);

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_STARS_PNG_PATH: stars_png_path,
            RES_STARS_MASKED: result.stars_removed,
            RES_MASK_COVERAGE: result.mask_coverage,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}
