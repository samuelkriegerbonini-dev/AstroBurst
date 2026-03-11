use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, render_and_save, resolve_output_dir};
use crate::core::imaging::stretch::arcsinh_stretch;
use crate::types::constants::{
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_PNG_PATH,
    RES_STRETCH_FACTOR,
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
