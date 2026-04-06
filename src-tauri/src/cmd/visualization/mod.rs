use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_cached, resolve_output_dir, save_preview_png};
use crate::cmd::helpers;
use crate::core::imaging::stf::{apply_stf, apply_stf_f32, auto_stf, AutoStfConfig, StfParams};
use crate::infra::render::tiles;
use crate::types::constants::{
    RES_HIGHLIGHT, RES_MIDTONE, RES_PNG_PATH, RES_SHADOW,
};

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
        let (rows, cols) = cached.arr().dim();

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
pub async fn generate_tiles_rgb(
    output_dir: String,
    tile_size: u32,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let (entry_r, entry_g, entry_b) = helpers::load_composite_rgb()
            .map_err(|_| anyhow::anyhow!("RGB composite not available. Run Compose RGB first."))?;

        let cfg = AutoStfConfig::default();
        let stf = helpers::compute_linked_stf(entry_r.stats(), entry_g.stats(), entry_b.stats(), &cfg);

        use crate::core::imaging::stf::make_stf_u8_fn;
        let fn_r = make_stf_u8_fn(&stf, entry_r.stats());
        let fn_g = make_stf_u8_fn(&stf, entry_g.stats());
        let fn_b = make_stf_u8_fn(&stf, entry_b.stats());

        let params = tiles::TileParams {
            tile_size: tile_size as usize,
        };

        let result = tiles::generate_tile_pyramid_rgb_stf(
            entry_r.arr(),
            entry_g.arr(),
            entry_b.arr(),
            &output_dir,
            &params,
            fn_r, fn_g, fn_b,
        )?;
        Ok(serde_json::to_value(&result).unwrap_or(json!({})))
    })
}
