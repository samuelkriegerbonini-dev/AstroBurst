use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_cached, output_stem, resolve_output_dir, save_stf_preview_png};
use crate::cmd::helpers;
use crate::cmd::io::{load_rgb_file_planes, RgbPlanes};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, make_stf_u8_fn, AutoStfConfig, ImageStats, StfParams};
use crate::infra::render::tiles;
use crate::types::constants::{RES_HIGHLIGHT, RES_MIDTONE, RES_PNG_PATH, RES_SHADOW};

#[tauri::command]
pub async fn apply_stf_render(
    path: String,
    output_dir: String,
    shadow: f64,
    midtone: f64,
    highlight: f64,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;

        let cached = load_cached(&path)?;

        let stf_params = StfParams {
            shadow,
            midtone,
            highlight,
        };

        let png_path = format!("{}/{}_stf.png", output_dir, output_stem(&path));
        save_stf_preview_png(cached.arr(), &stf_params, cached.stats(), &png_path)?;

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
        let params = tile_params(tile_size)?;
        let output_dir = resolve_output_dir(&output_dir)?;

        let cached = load_cached(&path)?;

        let stf_params = auto_stf(cached.stats(), &AutoStfConfig::default());
        let normalized = apply_stf_f32(cached.arr(), &stf_params, cached.stats());

        let result = tiles::generate_tile_pyramid(&normalized, &output_dir, &params)?;
        Ok(serde_json::to_value(&result).unwrap_or(json!({})))
    })
}

pub(super) fn tile_params(tile_size: u32) -> anyhow::Result<tiles::TileParams> {
    let size = tile_size as usize;
    if !(tiles::MIN_TILE_SIZE..=tiles::MAX_TILE_SIZE).contains(&size) {
        anyhow::bail!(
            "tile_size must be between {} and {} pixels, got {}",
            tiles::MIN_TILE_SIZE,
            tiles::MAX_TILE_SIZE,
            tile_size
        );
    }
    Ok(tiles::TileParams { tile_size: size })
}

fn tile_rgb_source(path: Option<&str>) -> anyhow::Result<(RgbPlanes, [ImageStats; 3])> {
    match path {
        Some(p) => {
            let (r, g, b) = load_rgb_file_planes(p)?;
            let (stats_r, (stats_g, stats_b)) = rayon::join(
                || compute_image_stats(&r),
                || rayon::join(|| compute_image_stats(&g), || compute_image_stats(&b)),
            );
            Ok(((r, g, b), [stats_r, stats_g, stats_b]))
        }
        None => {
            let (r, g, b) = helpers::load_composite_rgb()?;
            let stats = [r.stats().clone(), g.stats().clone(), b.stats().clone()];
            Ok(((r.data_arc(), g.data_arc(), b.data_arc()), stats))
        }
    }
}

#[tauri::command]
pub async fn generate_tiles_rgb(
    output_dir: String,
    tile_size: u32,
    path: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let params = tile_params(tile_size)?;
        let output_dir = resolve_output_dir(&output_dir)?;

        let ((r, g, b), [stats_r, stats_g, stats_b]) = tile_rgb_source(path.as_deref())?;

        let cfg = AutoStfConfig::default();
        let (stf, combined) = helpers::compute_linked_stf_with_stats(&stats_r, &stats_g, &stats_b, &cfg);

        let fn_r = make_stf_u8_fn(&stf, &combined);
        let fn_g = make_stf_u8_fn(&stf, &combined);
        let fn_b = make_stf_u8_fn(&stf, &combined);

        let result = tiles::generate_tile_pyramid_rgb_stf(&r, &g, &b, &output_dir, &params, fn_r, fn_g, fn_b)?;
        Ok(serde_json::to_value(&result).unwrap_or(json!({})))
    })
}
