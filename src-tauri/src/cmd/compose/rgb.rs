use std::sync::Arc;
use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_cached, load_from_cache_or_disk, resolve_output_dir, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::compose::lrgb::lrgb_combine_normalized;
use crate::core::imaging::resample::resample_image;
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{make_stf_u8_fn, AutoStfConfig, StfParams, apply_stf_f32};
use crate::core::imaging::scnr::apply_scnr_inplace;
use crate::infra::cache::{ImageEntry, GLOBAL_IMAGE_CACHE};
use crate::types::constants::{RES_DIMENSIONS, RES_ELAPSED_MS, RES_PNG_PATH, LRGB_APPLIED, COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B, COMPOSITE_ORIG_R, COMPOSITE_ORIG_G, COMPOSITE_ORIG_B, RES_CHANNEL, RES_UPDATED};

pub(super) fn composite_png_path(output_dir: &str) -> String {
    if let Ok(entries) = std::fs::read_dir(output_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("rgb_composite") && name_str.ends_with(".png") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{}/rgb_composite_{}.png", output_dir, ts)
}

pub(super) fn load_entry(path: &Option<String>) -> anyhow::Result<Option<ImageEntry>> {
    match path {
        Some(p) => Ok(Some(load_cached(p)?)),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn lrgb_combine_composite_cmd(
    l_path: String,
    output_dir: String,
    lightness: Option<f64>,
    chrominance: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let (er, eg, eb) = helpers::load_composite_rgb()
            .map_err(|_| anyhow::anyhow!("Composite not in cache. Run Blend first."))?;

        let l_entry = load_from_cache_or_disk(&l_path)?;
        let (rows, cols) = er.arr().dim();
        let l_data = l_entry.arr();
        let l_matched = if l_data.dim() != (rows, cols) {
            resample_image(l_data, rows, cols)?
        } else {
            l_data.to_owned()
        };

        let lightness_w = (lightness.unwrap_or(1.0) as f32).clamp(0.0, 1.0);
        let chrominance_w = (chrominance.unwrap_or(1.0) as f32).clamp(0.0, 1.0);

        let (r, g, b) = lrgb_combine_normalized(
            &l_matched,
            er.arr(),
            eg.arr(),
            eb.arr(),
            lightness_w,
            chrominance_w,
        )?;

        let stats_r = compute_image_stats(&r);
        let stats_g = compute_image_stats(&g);
        let stats_b = compute_image_stats(&b);

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let png_path = format!("{}/composite_lrgb_{}.png", output_dir, ts);

        let stf_config = AutoStfConfig::default();
        let (linked_stf, combined_stats) =
            helpers::compute_linked_stf_with_stats(&stats_r, &stats_g, &stats_b, &stf_config);
        let fn_r = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_g = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_b = make_stf_u8_fn(&linked_stf, &combined_stats);
        helpers::render_rgb_preview_with_stf(&r, &g, &b, fn_r, fn_g, fn_b, &png_path, MAX_PREVIEW_DIM)?;

        helpers::insert_composite_rgb(r, g, b, stats_r, stats_g, stats_b);

        Ok(json!({
            RES_PNG_PATH: png_path,
            LRGB_APPLIED: true,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn restretch_composite_cmd(
    output_dir: String,
    shadow_r: f64, midtone_r: f64, highlight_r: f64,
    shadow_g: f64, midtone_g: f64, highlight_g: f64,
    shadow_b: f64, midtone_b: f64, highlight_b: f64,
    scnr_enabled: Option<bool>,
    scnr_method: Option<String>,
    scnr_amount: Option<f64>,
    cache_result: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let (entry_r, entry_g, entry_b) = helpers::load_composite_rgb()
            .map_err(|_| anyhow::anyhow!("Composite not in cache. Please recompose first."))?;

        let stf_r = StfParams { shadow: shadow_r, midtone: midtone_r, highlight: highlight_r };
        let stf_g = StfParams { shadow: shadow_g, midtone: midtone_g, highlight: highlight_g };
        let stf_b = StfParams { shadow: shadow_b, midtone: midtone_b, highlight: highlight_b };

        let identical_params = shadow_r == shadow_g
            && shadow_g == shadow_b
            && midtone_r == midtone_g
            && midtone_g == midtone_b
            && highlight_r == highlight_g
            && highlight_g == highlight_b;

        let linked_stats = if identical_params {
            Some(crate::core::imaging::stats::combine_channel_stats(
                entry_r.stats(),
                entry_g.stats(),
                entry_b.stats(),
            ))
        } else {
            None
        };

        let (mut r_stretched, mut g_stretched, mut b_stretched) = match &linked_stats {
            Some(combined) => (
                apply_stf_f32(entry_r.arr(), &stf_r, combined),
                apply_stf_f32(entry_g.arr(), &stf_g, combined),
                apply_stf_f32(entry_b.arr(), &stf_b, combined),
            ),
            None => (
                apply_stf_f32(entry_r.arr(), &stf_r, entry_r.stats()),
                apply_stf_f32(entry_g.arr(), &stf_g, entry_g.stats()),
                apply_stf_f32(entry_b.arr(), &stf_b, entry_b.stats()),
            ),
        };

        if let Some(cfg) = helpers::parse_scnr_config(scnr_enabled, scnr_method.as_deref(), scnr_amount, None) {
            apply_scnr_inplace(&mut r_stretched, &mut g_stretched, &mut b_stretched, &cfg);
        }

        let png_path = composite_png_path(&output_dir);
        helpers::render_rgb_preview(&r_stretched, &g_stretched, &b_stretched, &png_path, MAX_PREVIEW_DIM)?;
        if cache_result.unwrap_or(false) {
            helpers::insert_composite_stretched(r_stretched, g_stretched, b_stretched);
        }

        Ok(json!({ RES_PNG_PATH: png_path, RES_ELAPSED_MS: t0.elapsed().as_millis() as u64 }))
    })
}

#[tauri::command]
pub async fn clear_composite_cache_cmd() -> Result<(), String> {
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_KEY_R);
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_KEY_G);
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_KEY_B);
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_ORIG_R);
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_ORIG_G);
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_ORIG_B);
    helpers::clear_composite_derived();
    Ok(())
}

#[tauri::command]
pub async fn update_composite_channel_cmd(
    channel: String,
    path: String,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let key = match channel.to_lowercase().as_str() {
            "r" => COMPOSITE_KEY_R,
            "g" => COMPOSITE_KEY_G,
            "b" => COMPOSITE_KEY_B,
            _ => anyhow::bail!("Invalid channel: {}. Must be r, g, or b.", channel),
        };

        let has_composite = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_R).is_some()
            && GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_G).is_some()
            && GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_B).is_some();

        if !has_composite {
            anyhow::bail!("No active composite. Compose RGB first.");
        }

        let target_dim = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_R)
            .map(|e| e.arr().dim())
            .ok_or_else(|| anyhow::anyhow!("Reference channel not found in cache"))?;

        let entry = load_from_cache_or_disk(&path)?;
        let (arr_arc, stats) = if entry.arr().dim() != target_dim {
            let resampled = crate::core::imaging::resample::resample_image(entry.arr(), target_dim.0, target_dim.1)?;
            let s = compute_image_stats(&resampled);
            (Arc::new(resampled), s)
        } else {
            let s = entry.stats().clone();
            (entry.data_arc(), s)
        };

        GLOBAL_IMAGE_CACHE.insert_synthetic(key, arr_arc, stats);
        helpers::clear_composite_derived();

        Ok(json!({ RES_CHANNEL: channel, RES_UPDATED: true }))
    })
}
