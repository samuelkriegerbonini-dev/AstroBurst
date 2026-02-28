use std::path::Path;
use std::time::Instant;

use anyhow::Result;

use crate::domain::normalize::asinh_normalize;
use crate::domain::stats;
use crate::domain::stf::{self, StfParams};
use crate::utils::render::render_grayscale;
use crate::utils::tiles::{generate_tile_pyramid, TileParams};

use super::helpers::*;

#[tauri::command]
pub async fn apply_stf_render(
    path: String,
    output_dir: String,
    shadow: f64,
    midtone: f64,
    highlight: f64,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();
        let (_, arr, _tmp) = extract_image_resolved(&path)?;
        let dims = arr.dim();

        let params = StfParams {
            shadow,
            midtone,
            highlight,
        };
        let st = stats::compute_image_stats(&arr);
        let stretched = stf::apply_stf_f32(&arr, &params, &st);

        let stem = Path::new(&path)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let out_dir = resolve_output_dir(&output_dir)?;
        let png_path = out_dir.join(format!("{}_stf.png", stem));
        render_grayscale(&stretched, png_path.to_str().unwrap())?;

        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "png_path": png_path.to_string_lossy(),
            "dimensions": [dims.1, dims.0],
            "stf_params": { "shadow": shadow, "midtone": midtone, "highlight": highlight },
            "elapsed_ms": elapsed,
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn generate_tiles(
    path: String,
    output_dir: String,
    tile_size: Option<usize>,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();
        let (_, arr, _tmp) = extract_image_resolved(&path)?;
        let normalized = asinh_normalize(&arr);

        let params = TileParams {
            tile_size: tile_size.unwrap_or(256),
        };

        let pyramid = generate_tile_pyramid(&normalized, &output_dir, &params)?;
        let elapsed = start.elapsed().as_millis() as u64;

        let levels: Vec<serde_json::Value> = pyramid
            .levels
            .iter()
            .map(|level| {
                serde_json::json!({
                    "level": level.level,
                    "width": level.width,
                    "height": level.height,
                    "cols": level.cols,
                    "rows": level.rows,
                    "scale_factor": level.scale_factor,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "tile_size": pyramid.tile_size,
            "original_width": pyramid.original_width,
            "original_height": pyramid.original_height,
            "num_levels": pyramid.levels.len(),
            "levels": levels,
            "base_dir": pyramid.base_dir,
            "elapsed_ms": elapsed,
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn get_tile(
    path: String,
    output_dir: String,
    level: usize,
    col: usize,
    row: usize,
    tile_size: Option<usize>,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let ts = tile_size.unwrap_or(256);
        let tile_path = format!("{}/{}/{}_{}.png", output_dir, level, col, row);

        if Path::new(&tile_path).exists() {
            return Ok(serde_json::json!({
                "tile_path": tile_path,
                "level": level,
                "col": col,
                "row": row,
                "cached": true,
            }));
        }

        let start = Instant::now();
        let (_, arr, _tmp) = extract_image_resolved(&path)?;
        let normalized = asinh_normalize(&arr);

        let params = TileParams { tile_size: ts };
        let _ = generate_tile_pyramid(&normalized, &output_dir, &params)?;
        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "tile_path": tile_path,
            "level": level,
            "col": col,
            "row": row,
            "cached": false,
            "elapsed_ms": elapsed,
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}
