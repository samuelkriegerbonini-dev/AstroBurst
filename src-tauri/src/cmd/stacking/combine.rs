use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, output_stem, render_asinh_and_save, resolve_output_dir};
use crate::core::imaging::stats::compute_image_stats;
use crate::cmd::helpers;
use crate::core::stacking::calibration::calibrate_from_paths;
use crate::core::stacking::calibration::drizzle_from_paths;
use crate::core::stacking::calibration::stack_from_paths;
use crate::infra::fits::writer::write_fits_mono;
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    EVENT_CALIBRATE_PROGRESS, EVENT_STACK_PROGRESS, STAGE_RENDER, STAGE_SAVE,
    RES_DIMENSIONS, RES_DX, RES_DY, RES_FITS_PATH, RES_FRAME_COUNT,
    RES_HAS_BIAS, RES_HAS_DARK, RES_HAS_FLAT, RES_MAX, RES_MEAN, RES_MIN,
    RES_OFFSETS, RES_PNG_PATH, RES_REJECTED_PIXELS, RES_SCALE, RES_SIGMA, RES_STATS,
};
use crate::types::stacking::{DrizzleConfig, StackConfig};

pub const RES_REJECTION: &str = "rejection";
pub const RES_COMBINE: &str = "combine";
pub const RES_NORMALIZATION: &str = "normalization";
pub const RES_REJECTION_NORMALIZATION: &str = "rejection_normalization";
pub const RES_REJECTION_LOW_FITS: &str = "rejection_low_fits";
pub const RES_REJECTION_HIGH_FITS: &str = "rejection_high_fits";
pub const RES_NORMALIZATION_APPLIED: &str = "normalization_applied";

#[tauri::command]
pub async fn calibrate(
    app: tauri::AppHandle,
    science_path: String,
    output_dir: String,
    bias_paths: Option<Vec<String>>,
    dark_paths: Option<Vec<String>>,
    flat_paths: Option<Vec<String>>,
    dark_exposure_ratio: Option<f32>,
) -> Result<serde_json::Value, String> {
    let progress = ProgressHandle::new(&app, EVENT_CALIBRATE_PROGRESS, 4);
    let progress_clone = progress.clone();

    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let calibrated = calibrate_from_paths(
            &science_path,
            bias_paths.as_deref(),
            dark_paths.as_deref(),
            flat_paths.as_deref(),
            dark_exposure_ratio.unwrap_or(1.0),
        )?;

        progress_clone.tick_with_stage(STAGE_RENDER);

        let stem = output_stem(&science_path);

        let (png_path, fits_path) = render_asinh_and_save(
            &calibrated,
            &output_dir,
            &format!("{}_calibrated", stem),
            true,
        )?;

        let (rows, cols) = calibrated.dim();
        let stats = compute_image_stats(&calibrated);

        progress_clone.emit_complete();

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_FITS_PATH: fits_path,
            RES_DIMENSIONS: [cols, rows],
            RES_HAS_BIAS: bias_paths.is_some(),
            RES_HAS_DARK: dark_paths.is_some(),
            RES_HAS_FLAT: flat_paths.is_some(),
            RES_STATS: {
                RES_MIN: stats.min,
                RES_MAX: stats.max,
                RES_MEAN: stats.mean,
                RES_SIGMA: stats.sigma,
            },
        }))
    })
}

fn write_rejection_map(map: &Array2<u16>, output_dir: &str, stem: &str, side: &str) -> anyhow::Result<String> {
    let path = format!("{}/{}_rejection_{}.fits", output_dir, stem, side);
    let counts = map.mapv(|v| v as f32);
    write_fits_mono(&path, &counts, None)?;
    Ok(path)
}

#[tauri::command]
pub async fn stack(
    app: tauri::AppHandle,
    paths: Vec<String>,
    output_dir: String,
    sigma_low: Option<f32>,
    sigma_high: Option<f32>,
    max_iterations: Option<usize>,
    align: Option<bool>,
    align_method: Option<String>,
    name: Option<String>,
    weights: Option<Vec<f64>>,
    rejection: Option<String>,
    combine: Option<String>,
    normalization: Option<String>,
    rejection_normalization: Option<String>,
    winsor_cutoff: Option<f32>,
    percentile_low: Option<f32>,
    percentile_high: Option<f32>,
    minmax_low: Option<usize>,
    minmax_high: Option<usize>,
    rejection_maps: Option<bool>,
) -> Result<serde_json::Value, String> {
    let frame_count = paths.len() as u64;
    let progress = ProgressHandle::new(&app, EVENT_STACK_PROGRESS, frame_count + 2);
    let progress_clone = progress.clone();

    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let defaults = StackConfig::default();
        let config = StackConfig {
            sigma_low: sigma_low.unwrap_or(defaults.sigma_low),
            sigma_high: sigma_high.unwrap_or(defaults.sigma_high),
            max_iterations: max_iterations.unwrap_or(defaults.max_iterations),
            align: align.unwrap_or(true),
            align_method: helpers::parse_align_method(align_method.as_deref()),
            weights,
            rejection: helpers::parse_rejection_method(rejection.as_deref())?,
            combine: helpers::parse_combine_method(combine.as_deref())?,
            normalization: helpers::parse_normalization_method(normalization.as_deref())?,
            rejection_normalization: helpers::parse_rejection_normalization(rejection_normalization.as_deref())?,
            winsor_cutoff: winsor_cutoff.unwrap_or(defaults.winsor_cutoff),
            percentile_low: percentile_low.unwrap_or(defaults.percentile_low),
            percentile_high: percentile_high.unwrap_or(defaults.percentile_high),
            minmax_low: minmax_low.unwrap_or(defaults.minmax_low),
            minmax_high: minmax_high.unwrap_or(defaults.minmax_high),
            rejection_maps: rejection_maps.unwrap_or(false),
        };

        let result = stack_from_paths(&paths, &config, None)?;

        progress_clone.tick_with_stage(STAGE_RENDER);

        let stem = name.as_deref().unwrap_or("stacked");

        let (png_path, fits_path) = render_asinh_and_save(
            &result.image,
            &output_dir,
            stem,
            true,
        )?;

        let rejection_low_fits = match result.rejection_low.as_ref() {
            Some(map) => Some(write_rejection_map(map, &output_dir, stem, "low")?),
            None => None,
        };
        let rejection_high_fits = match result.rejection_high.as_ref() {
            Some(map) => Some(write_rejection_map(map, &output_dir, stem, "high")?),
            None => None,
        };

        let (rows, cols) = result.image.dim();
        let stats = compute_image_stats(&result.image);

        progress_clone.tick_with_stage(STAGE_SAVE);
        progress_clone.emit_complete();

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_FITS_PATH: fits_path,
            RES_DIMENSIONS: [cols, rows],
            RES_FRAME_COUNT: result.frame_count,
            RES_REJECTED_PIXELS: result.rejected_pixels,
            RES_OFFSETS: result.offsets.iter().map(|(dy, dx)| json!({RES_DY: dy, RES_DX: dx})).collect::<Vec<_>>(),
            RES_REJECTION: config.rejection.name(),
            RES_COMBINE: config.combine.name(),
            RES_NORMALIZATION: config.normalization.name(),
            RES_REJECTION_NORMALIZATION: config.rejection_normalization.name(),
            RES_NORMALIZATION_APPLIED: result.normalization_applied.iter().map(|(offset, scale)| json!({"offset": offset, "scale": scale})).collect::<Vec<_>>(),
            RES_REJECTION_LOW_FITS: rejection_low_fits,
            RES_REJECTION_HIGH_FITS: rejection_high_fits,
            RES_STATS: {
                RES_MIN: stats.min,
                RES_MAX: stats.max,
                RES_MEAN: stats.mean,
                RES_SIGMA: stats.sigma,
            },
        }))
    })
}

#[tauri::command]
pub async fn drizzle_stack(
    app: tauri::AppHandle,
    paths: Vec<String>,
    output_dir: String,
    scale: Option<f64>,
    pixfrac: Option<f64>,
    kernel: Option<String>,
    align: Option<bool>,
    name: Option<String>,
) -> Result<serde_json::Value, String> {
    let frame_count = paths.len() as u64;
    let progress = ProgressHandle::new(&app, EVENT_STACK_PROGRESS, frame_count + 2);
    let progress_clone = progress.clone();

    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let config = DrizzleConfig {
            scale: scale.unwrap_or(2.0),
            pixfrac: pixfrac.unwrap_or(0.7),
            kernel: helpers::parse_drizzle_kernel(kernel.as_deref()),
            align: align.unwrap_or(true),
            ..DrizzleConfig::default()
        };

        let result = drizzle_from_paths(&paths, &config, None)?;

        progress_clone.tick_with_stage(STAGE_RENDER);

        let stem = name.as_deref().unwrap_or("drizzled");

        let (png_path, fits_path) = render_asinh_and_save(
            &result.image,
            &output_dir,
            stem,
            true,
        )?;

        let (rows, cols) = result.image.dim();
        let stats = compute_image_stats(&result.image);

        progress_clone.tick_with_stage(STAGE_SAVE);
        progress_clone.emit_complete();

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_FITS_PATH: fits_path,
            RES_DIMENSIONS: [cols, rows],
            RES_FRAME_COUNT: result.frame_count,
            RES_REJECTED_PIXELS: result.rejected_pixels,
            RES_SCALE: result.output_scale,
            RES_STATS: {
                RES_MIN: stats.min,
                RES_MAX: stats.max,
                RES_MEAN: stats.mean,
                RES_SIGMA: stats.sigma,
            },
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_map_is_written_as_float_counts_next_to_the_stack() {
        let dir = tempfile::tempdir().unwrap();
        let map = Array2::from_shape_vec((2, 3), vec![0u16, 1, 2, 65535, 4, 5]).unwrap();
        let out = dir.path().to_str().unwrap().replace('\\', "/");
        let path = write_rejection_map(&map, &out, "stacked", "high").unwrap();
        assert!(path.ends_with("/stacked_rejection_high.fits"));
        let back = crate::infra::fits::reader::load_fits_image(&path).unwrap();
        assert_eq!(back.dim(), (2, 3));
        let mut values: Vec<f32> = back.iter().copied().collect();
        values.sort_by(crate::math::median::f32_cmp);
        assert_eq!(values, vec![0.0, 1.0, 2.0, 4.0, 5.0, 65535.0]);
    }
}
