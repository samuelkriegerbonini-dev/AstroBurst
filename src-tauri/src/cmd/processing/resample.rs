use serde_json::json;

use crate::cmd::common::{blocking_cmd, extract_image_resolved, render_asinh_and_save, resolve_output_dir};
use crate::core::imaging::resample::resample_with_wcs;
use crate::core::imaging::stats::compute_image_stats;
use crate::types::constants::{
    RES_DIMENSIONS, RES_FITS_PATH, RES_MAX, RES_MEAN, RES_MIN,
    RES_ORIGINAL_DIMENSIONS, RES_PNG_PATH, RES_SIGMA, RES_STATS, RES_WCS_UPDATES,
};

#[tauri::command]
pub async fn resample_fits_cmd(
    path: String,
    target_width: usize,
    target_height: usize,
    output_dir: String,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;
        let resolved = extract_image_resolved(&path)?;
        let result = resample_with_wcs(
            &resolved.arr,
            &resolved.header,
            target_height,
            target_width,
        )?;

        let (png_path, fits_path) = render_asinh_and_save(
            &result.image,
            &output_dir,
            &format!("{}_resampled",
                std::path::Path::new(&path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("resampled")),
            true,
        )?;

        let stats = compute_image_stats(&result.image);

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_FITS_PATH: fits_path,
            RES_DIMENSIONS: result.resampled_dims,
            RES_ORIGINAL_DIMENSIONS: result.original_dims,
            RES_WCS_UPDATES: result.header_updates,
            RES_STATS: {
                RES_MIN: stats.min,
                RES_MAX: stats.max,
                RES_MEAN: stats.mean,
                RES_SIGMA: stats.sigma,
            },
        }))
    })
}
