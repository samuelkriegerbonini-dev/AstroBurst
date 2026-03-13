use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_cached, resolve_output_dir, save_preview_png};
use crate::core::imaging::stf::{apply_stf, apply_stf_f32, auto_stf, AutoStfConfig, StfParams};
use crate::infra::render::tiles;
use crate::types::constants::{RES_HIGHLIGHT, RES_MIDTONE, RES_PNG_PATH, RES_SHADOW, RES_TILE_PATH};

#[tauri::command]
pub async fn apply_stf_render(
    path: String,
    output_dir: String,
    shadow: f64,
    midtone: f64,
    highlight: f64,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let cached = load_cached(&path)?;

        let stf_params = StfParams {
            shadow,
            midtone,
            highlight,
        };

        let rendered = apply_stf(cached.arr(), &stf_params, cached.stats());

        let stem = std::path::Path::new(&path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("stf");
        let png_path = format!("{}/{}_stf.png", output_dir, stem);
        let (rows, cols) = cached.arr().dim();
        save_preview_png(rendered, cols, rows, &png_path)?;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_SHADOW: shadow,
            RES_MIDTONE: midtone,
            RES_HIGHLIGHT: highlight,
        }))
    })
}

#[tauri::command]
pub async fn generate_tiles(
    path: String,
    output_dir: String,
    tile_size: u32,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let cached = load_cached(&path)?;

        let stf_params = auto_stf(cached.stats(), &AutoStfConfig::default());
        let normalized = apply_stf_f32(cached.arr(), &stf_params, cached.stats());

        let params = tiles::TileParams {
            tile_size: tile_size as usize,
        };

        let result = tiles::generate_tile_pyramid(&normalized, &output_dir, &params)?;
        Ok(serde_json::to_value(&result).unwrap_or(json!({})))
    })
}

#[tauri::command]
pub async fn get_tile(
    _path: String,
    output_dir: String,
    level: u32,
    col: u32,
    row: u32,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let tile_path = format!(
            "{}/{}/{}_{}.png",
            output_dir, level, col, row
        );
        Ok(json!({ RES_TILE_PATH: tile_path }))
    })
}
