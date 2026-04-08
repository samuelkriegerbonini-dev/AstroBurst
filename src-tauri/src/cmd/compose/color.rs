use std::time::Instant;

use ndarray::{Array2, Zip};
use serde_json::json;

use crate::cmd::common::{blocking_cmd, resolve_output_dir, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::imaging::stats::{compute_image_stats, compute_image_stats_with_known_range};
use crate::core::imaging::stf::{make_stf_u8_fn, AutoStfConfig};
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::types::image::ImageStats;
use crate::types::constants::{
    RES_ELAPSED_MS, RES_PNG_PATH, RES_WB_APPLIED, RES_R_FACTOR, RES_G_FACTOR, RES_B_FACTOR,
    COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B,
};

use super::rgb::composite_png_path;

const PAR_THRESHOLD: usize = 4_000_000;

fn calibrate_channel(
    orig: &Array2<f32>,
    factor: f32,
    orig_stats: &ImageStats,
) -> (Array2<f32>, ImageStats) {
    let npix = orig.len();

    if npix <= PAR_THRESHOLD {
        let result = orig.mapv(|v| v * factor);
        let stats = compute_image_stats(&result);
        return (result, stats);
    }

    let mut result = Array2::zeros(orig.dim());
    Zip::from(&mut result)
        .and(orig)
        .par_for_each(|o, &v| {
            *o = v * factor;
        });

    let (known_min, known_max) = if factor >= 0.0 {
        (orig_stats.min * factor as f64, orig_stats.max * factor as f64)
    } else {
        (orig_stats.max * factor as f64, orig_stats.min * factor as f64)
    };

    let stats = compute_image_stats_with_known_range(&result, known_min, known_max);
    (result, stats)
}

#[tauri::command]
pub async fn reset_wb_cmd(
    output_dir: String,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let (orig_r, orig_g, orig_b) = helpers::load_composite_orig_rgb()
            .map_err(|_| anyhow::anyhow!("No original composite. Run Blend first."))?;

        let stats_r = orig_r.stats().clone();
        let stats_g = orig_g.stats().clone();
        let stats_b = orig_b.stats().clone();

        let png_path = composite_png_path(&output_dir);

        let stf_config = AutoStfConfig::default();
        let linked_stf = helpers::compute_linked_stf(&stats_r, &stats_g, &stats_b, &stf_config);
        let fn_r = make_stf_u8_fn(&linked_stf, &stats_r);
        let fn_g = make_stf_u8_fn(&linked_stf, &stats_g);
        let fn_b = make_stf_u8_fn(&linked_stf, &stats_b);
        helpers::render_rgb_preview_with_stf(orig_r.arr(), orig_g.arr(), orig_b.arr(), fn_r, fn_g, fn_b, &png_path, MAX_PREVIEW_DIM)?;

        let arc_r = orig_r.data_arc();
        let arc_g = orig_g.data_arc();
        let arc_b = orig_b.data_arc();

        GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_R, arc_r, stats_r);
        GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_G, arc_g, stats_g);
        GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_B, arc_b, stats_b);

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_PNG_PATH: png_path,
            "reset": true,
            RES_R_FACTOR: 1.0,
            RES_G_FACTOR: 1.0,
            RES_B_FACTOR: 1.0,
            "auto_stf": helpers::stf_json(&linked_stf),
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

#[tauri::command]
pub async fn calibrate_and_scnr_cmd(
    output_dir: String,
    r_factor: f64,
    g_factor: f64,
    b_factor: f64,
    scnr_enabled: Option<bool>,
    scnr_method: Option<String>,
    scnr_amount: Option<f64>,
    scnr_preserve_luminance: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let (orig_r, orig_g, orig_b) = helpers::load_composite_orig_rgb()
            .map_err(|_| anyhow::anyhow!("No original composite. Run Blend first."))?;

        let rf = (r_factor as f32).max(1e-6);
        let gf = (g_factor as f32).max(1e-6);
        let bf = (b_factor as f32).max(1e-6);

        let sr = orig_r.stats();
        let sg = orig_g.stats();
        let sb = orig_b.stats();

        let ((mut r, mut stats_r), ((mut g, mut stats_g), (mut b, mut stats_b))) = rayon::join(
            || calibrate_channel(orig_r.arr(), rf, sr),
            || rayon::join(
                || calibrate_channel(orig_g.arr(), gf, sg),
                || calibrate_channel(orig_b.arr(), bf, sb),
            ),
        );

        let scnr_config = helpers::parse_scnr_config(
            scnr_enabled,
            scnr_method.as_deref(),
            scnr_amount,
            scnr_preserve_luminance,
        );

        let scnr_applied = match scnr_config {
            Some(ref cfg) if cfg.amount > 1e-7 => {
                crate::core::imaging::scnr::apply_scnr_inplace(&mut r, &mut g, &mut b, cfg);

                if cfg.preserve_luminance {
                    let (sr2, (sg2, sb2)) = rayon::join(
                        || compute_image_stats(&r),
                        || rayon::join(
                            || compute_image_stats(&g),
                            || compute_image_stats(&b),
                        ),
                    );
                    stats_r = sr2;
                    stats_g = sg2;
                    stats_b = sb2;
                } else {
                    stats_g = compute_image_stats(&g);
                }
                true
            }
            _ => false,
        };

        let png_path = composite_png_path(&output_dir);

        let stf_config = AutoStfConfig::default();
        let linked_stf = helpers::compute_linked_stf(&stats_r, &stats_g, &stats_b, &stf_config);
        let fn_r = make_stf_u8_fn(&linked_stf, &stats_r);
        let fn_g = make_stf_u8_fn(&linked_stf, &stats_g);
        let fn_b = make_stf_u8_fn(&linked_stf, &stats_b);
        helpers::render_rgb_preview_with_stf(&r, &g, &b, fn_r, fn_g, fn_b, &png_path, MAX_PREVIEW_DIM)?;
        helpers::insert_composite_rgb(r, g, b, stats_r, stats_g, stats_b);

        let stf_json = helpers::stf_json(&linked_stf);
        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_WB_APPLIED: true,
            RES_R_FACTOR: r_factor,
            RES_G_FACTOR: g_factor,
            RES_B_FACTOR: b_factor,
            "scnr_applied": scnr_applied,
            "auto_stf": stf_json,
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

#[tauri::command]
pub async fn compute_auto_wb_cmd() -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let (entry_r, entry_g, entry_b) = helpers::load_orig_or_composite()?;

        let sr = entry_r.stats();
        let sg = entry_g.stats();
        let sb = entry_b.stats();

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
