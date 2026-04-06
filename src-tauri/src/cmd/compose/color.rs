use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, resolve_output_dir, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::imaging::stats::compute_image_stats;
use crate::types::constants::{RES_ELAPSED_MS, RES_PNG_PATH, RES_WB_APPLIED, RES_R_FACTOR, RES_G_FACTOR, RES_B_FACTOR};

use super::rgb::composite_png_path;

#[tauri::command]
pub async fn reset_wb_cmd(
    output_dir: String,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let (orig_r, orig_g, orig_b) = helpers::load_composite_orig_rgb()
            .map_err(|_| anyhow::anyhow!("No original composite. Run Blend first."))?;

        let r = orig_r.arr().to_owned();
        let g = orig_g.arr().to_owned();
        let b = orig_b.arr().to_owned();

        let stats_r = orig_r.stats().clone();
        let stats_g = orig_g.stats().clone();
        let stats_b = orig_b.stats().clone();

        let png_path = composite_png_path(&output_dir);
        helpers::render_rgb_preview(&r, &g, &b, &png_path, MAX_PREVIEW_DIM)?;

        helpers::insert_composite_rgb(r, g, b, stats_r, stats_g, stats_b);

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_PNG_PATH: png_path,
            "reset": true,
            RES_R_FACTOR: 1.0,
            RES_G_FACTOR: 1.0,
            RES_B_FACTOR: 1.0,
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

#[tauri::command]
pub async fn calibrate_composite_cmd(
    output_dir: String,
    r_factor: f64,
    g_factor: f64,
    b_factor: f64,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let (orig_r, orig_g, orig_b) = helpers::load_orig_or_composite()?;

        let rf = r_factor as f32;
        let gf = g_factor as f32;
        let bf = b_factor as f32;

        let r = orig_r.arr().mapv(|v| (v * rf).clamp(0.0, 1.0));
        let g = orig_g.arr().mapv(|v| (v * gf).clamp(0.0, 1.0));
        let b = orig_b.arr().mapv(|v| (v * bf).clamp(0.0, 1.0));

        let stats_r = compute_image_stats(&r);
        let stats_g = compute_image_stats(&g);
        let stats_b = compute_image_stats(&b);

        let png_path = composite_png_path(&output_dir);
        helpers::render_rgb_preview(&r, &g, &b, &png_path, MAX_PREVIEW_DIM)?;

        helpers::insert_composite_rgb(r, g, b, stats_r, stats_g, stats_b);

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_WB_APPLIED: true,
            RES_R_FACTOR: r_factor,
            RES_G_FACTOR: g_factor,
            RES_B_FACTOR: b_factor,
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

#[tauri::command]
pub async fn compute_auto_wb_cmd() -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let (entry_r, entry_g, entry_b) = helpers::load_orig_or_composite()?;

        let sr = compute_image_stats(entry_r.arr());
        let sg = compute_image_stats(entry_g.arr());
        let sb = compute_image_stats(entry_b.arr());

        let stability = |med: f64, mad: f64| -> f64 {
            if med > 1e-10 { mad / med } else { f64::MAX }
        };
        let stab_r = stability(sr.median, sr.mad);
        let stab_g = stability(sg.median, sg.mad);
        let stab_b = stability(sb.median, sb.mad);

        let (wb_r, wb_g, wb_b) = if stab_r <= stab_g && stab_r <= stab_b {
            let m = sr.median.max(1e-10);
            (1.0, m / sg.median.max(1e-10), m / sb.median.max(1e-10))
        } else if stab_b <= stab_g {
            let m = sb.median.max(1e-10);
            (m / sr.median.max(1e-10), m / sg.median.max(1e-10), 1.0)
        } else {
            let m = sg.median.max(1e-10);
            (m / sr.median.max(1e-10), 1.0, m / sb.median.max(1e-10))
        };

        Ok(json!({
            RES_R_FACTOR: wb_r,
            RES_G_FACTOR: wb_g,
            RES_B_FACTOR: wb_b,
            "stab_r": stab_r,
            "stab_g": stab_g,
            "stab_b": stab_b,
            "ref_channel": if stab_r <= stab_g && stab_r <= stab_b { "R" } else if stab_b <= stab_g { "B" } else { "G" },
        }))
    })
}
