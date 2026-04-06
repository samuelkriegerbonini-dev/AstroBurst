use std::sync::Arc;
use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_cached, load_from_cache_or_disk, resolve_output_dir, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::compose::rgb::process_rgb;
use crate::core::compose::lrgb::apply_lrgb;
use crate::core::imaging::resample::resample_image;
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{StfParams, apply_stf_f32};
use crate::core::imaging::scnr::apply_scnr_inplace;
use crate::infra::cache::{ImageEntry, GLOBAL_IMAGE_CACHE};
use crate::types::compose::{RgbComposeConfig, RgbComposeResult};
use crate::types::constants::{RES_DIMENSIONS, RES_DIMENSION_INFO, RES_ELAPSED_MS, RES_MAX, RES_MEAN, RES_MEDIAN, RES_MIN, RES_OFFSET_B, RES_OFFSET_G, RES_PNG_PATH, RES_SCNR_APPLIED, RES_STATS_B, RES_STATS_G, RES_STATS_R, RES_SHADOW, RES_MIDTONE, RES_HIGHLIGHT, LRGB_APPLIED, RESAMPLED, STF_G, STF_R, STF_B, COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B, COMPOSITE_ORIG_R, COMPOSITE_ORIG_G, COMPOSITE_ORIG_B};

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
pub async fn compose_rgb_cmd(
    l_path: Option<String>,
    r_path: Option<String>,
    g_path: Option<String>,
    b_path: Option<String>,
    output_dir: String,
    auto_stretch: Option<bool>,
    linked_stf: Option<bool>,
    align: Option<bool>,
    align_method: Option<String>,
    wb_mode: Option<String>,
    wb_r: Option<f64>,
    wb_g: Option<f64>,
    wb_b: Option<f64>,
    scnr_enabled: Option<bool>,
    scnr_method: Option<String>,
    scnr_amount: Option<f64>,
    dimension_tolerance: Option<usize>,
    lrgb_lightness: Option<f64>,
    lrgb_chrominance: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let l_entry = load_entry(&l_path)?;
        let r_entry = load_entry(&r_path)?;
        let g_entry = load_entry(&g_path)?;
        let b_entry = load_entry(&b_path)?;

        let r_ref = r_entry.as_ref().map(|e| e.arr());
        let g_ref = g_entry.as_ref().map(|e| e.arr());
        let b_ref = b_entry.as_ref().map(|e| e.arr());

        let wb = helpers::parse_wb(wb_mode.as_deref(), wb_r, wb_g, wb_b);

        let scnr_cfg = helpers::parse_scnr_config(
            scnr_enabled,
            scnr_method.as_deref(),
            scnr_amount,
            None,
        );

        let align_m = helpers::parse_align_method(align_method.as_deref());

        let config = RgbComposeConfig {
            white_balance: wb,
            auto_stretch: auto_stretch.unwrap_or(true),
            linked_stf: linked_stf.unwrap_or(false),
            align: align.unwrap_or(true),
            align_method: align_m,
            scnr: scnr_cfg,
            dimension_tolerance: dimension_tolerance.unwrap_or(100),
            ..RgbComposeConfig::default()
        };

        let mut processed = process_rgb(
            r_ref,
            g_ref,
            b_ref,
            &config,
        )?;

        if let (Some(pre_r), Some(pre_g), Some(pre_b)) = (
            processed.pre_stretch_r.take(),
            processed.pre_stretch_g.take(),
            processed.pre_stretch_b.take(),
        ) {
            let stats_r = processed.stats_wb_r.clone().unwrap_or_else(|| compute_image_stats(&pre_r));
            let stats_g = processed.stats_wb_g.clone().unwrap_or_else(|| compute_image_stats(&pre_g));
            let stats_b = processed.stats_wb_b.clone().unwrap_or_else(|| compute_image_stats(&pre_b));

            helpers::insert_composite_and_orig(pre_r, pre_g, pre_b, stats_r, stats_g, stats_b);
        }

        let lrgb_applied = if let Some(l_entry_ref) = l_entry.as_ref() {
            let lightness = lrgb_lightness.unwrap_or(1.0) as f32;
            let chrominance = lrgb_chrominance.unwrap_or(1.0) as f32;

            let l_data = l_entry_ref.arr();
            let (lr, lc) = l_data.dim();
            let l_matched = if lr != processed.rows || lc != processed.cols {
                resample_image(l_data, processed.rows, processed.cols)?
            } else {
                l_data.to_owned()
            };

            let l_stretched = if config.auto_stretch {
                use crate::core::imaging::stf::{auto_stf, analyze};
                use crate::types::image::AutoStfConfig;
                let (stats, _) = analyze(&l_matched);
                let stf = auto_stf(&stats, &AutoStfConfig::default());
                apply_stf_f32(&l_matched, &stf, &stats)
            } else {
                l_matched
            };

            apply_lrgb(
                &l_stretched,
                &mut processed.r,
                &mut processed.g,
                &mut processed.b,
                lightness,
                chrominance,
            )?;
            true
        } else {
            false
        };

        let png_path = composite_png_path(&output_dir);

        helpers::render_rgb_preview(
            &processed.r,
            &processed.g,
            &processed.b,
            &png_path,
            MAX_PREVIEW_DIM,
        )?;

        let resampled = processed.dimension_info.as_ref().map_or(false, |d| d.resampled);

        let result = RgbComposeResult {
            png_path: png_path.clone(),
            stf_r: processed.stf_r,
            stf_g: processed.stf_g,
            stf_b: processed.stf_b,
            stats_r: processed.stats_r.clone(),
            stats_g: processed.stats_g.clone(),
            stats_b: processed.stats_b.clone(),
            offset_g: processed.offset_g,
            offset_b: processed.offset_b,
            width: processed.cols,
            height: processed.rows,
            scnr_applied: processed.scnr_applied,
            dimension_info: processed.dimension_info,
        };

        let elapsed = t0.elapsed().as_millis() as u64;

        let stf_r_json = json!({RES_SHADOW: result.stf_r.shadow, RES_MIDTONE: result.stf_r.midtone, RES_HIGHLIGHT: result.stf_r.highlight});
        let stf_g_json = json!({RES_SHADOW: result.stf_g.shadow, RES_MIDTONE: result.stf_g.midtone, RES_HIGHLIGHT: result.stf_g.highlight});
        let stf_b_json = json!({RES_SHADOW: result.stf_b.shadow, RES_MIDTONE: result.stf_b.midtone, RES_HIGHLIGHT: result.stf_b.highlight});

        Ok(json!({
            RES_PNG_PATH: result.png_path,
            RES_DIMENSIONS: [result.width, result.height],
            RES_SCNR_APPLIED: result.scnr_applied,
            RES_OFFSET_G: [result.offset_g.0, result.offset_g.1],
            RES_OFFSET_B: [result.offset_b.0, result.offset_b.1],
            RES_DIMENSION_INFO: result.dimension_info,
            RESAMPLED: resampled,
            LRGB_APPLIED: lrgb_applied,
            STF_R: stf_r_json,
            STF_G: stf_g_json,
            STF_B: stf_b_json,
            RES_STATS_R: { RES_MEDIAN: result.stats_r.median, RES_MEAN: result.stats_r.mean, RES_MIN: result.stats_r.min, RES_MAX: result.stats_r.max },
            RES_STATS_G: { RES_MEDIAN: result.stats_g.median, RES_MEAN: result.stats_g.mean, RES_MIN: result.stats_g.min, RES_MAX: result.stats_g.max },
            RES_STATS_B: { RES_MEDIAN: result.stats_b.median, RES_MEAN: result.stats_b.mean, RES_MIN: result.stats_b.min, RES_MAX: result.stats_b.max },
            RES_ELAPSED_MS: elapsed,
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
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let (entry_r, entry_g, entry_b) = helpers::load_composite_rgb()
            .map_err(|_| anyhow::anyhow!("Composite not in cache. Please recompose first."))?;

        let stf_r = StfParams { shadow: shadow_r, midtone: midtone_r, highlight: highlight_r };
        let stf_g = StfParams { shadow: shadow_g, midtone: midtone_g, highlight: highlight_g };
        let stf_b = StfParams { shadow: shadow_b, midtone: midtone_b, highlight: highlight_b };

        let mut r_stretched = apply_stf_f32(entry_r.arr(), &stf_r, entry_r.stats());
        let mut g_stretched = apply_stf_f32(entry_g.arr(), &stf_g, entry_g.stats());
        let mut b_stretched = apply_stf_f32(entry_b.arr(), &stf_b, entry_b.stats());

        if let Some(cfg) = helpers::parse_scnr_config(scnr_enabled, scnr_method.as_deref(), scnr_amount, None) {
            apply_scnr_inplace(&mut r_stretched, &mut g_stretched, &mut b_stretched, &cfg);
        }

        let png_path = composite_png_path(&output_dir);
        helpers::render_rgb_preview(&r_stretched, &g_stretched, &b_stretched, &png_path, MAX_PREVIEW_DIM)?;

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

        Ok(json!({ "channel": channel, "updated": true }))
    })
}
