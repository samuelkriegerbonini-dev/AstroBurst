use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, resolve_output_dir, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::imaging::curves::{
    apply_curve_rgb, apply_levels_rgb, LevelsParams, SplineLut,
};
use crate::core::imaging::scnr;
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, AutoStfConfig};
use crate::types::constants::{
    COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B,
    RES_COMPOSITE_DIMS, RES_CURVES_APPLIED, RES_DIMENSIONS,
    RES_ELAPSED_MS, RES_LEVELS_APPLIED, RES_PNG_PATH,
    RES_SCNR_APPLIED, RES_STF_APPLIED, RES_STF,
    RES_SHADOW, RES_MIDTONE, RES_HIGHLIGHT,
};
use crate::types::image::{ScnrConfig, StfParams};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ToneLevelsInput {
    pub black: f64,
    pub gamma: f64,
    pub white: f64,
}

impl From<&ToneLevelsInput> for LevelsParams {
    fn from(input: &ToneLevelsInput) -> Self {
        Self {
            black: input.black,
            gamma: input.gamma,
            white: input.white,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ToneCurveInput {
    pub points: Vec<[f64; 2]>,
}

fn build_spline(input: &ToneCurveInput) -> SplineLut {
    let pts: Vec<(f64, f64)> = input.points.iter().map(|p| (p[0], p[1])).collect();
    SplineLut::from_points(&pts)
}

fn is_curve_identity(input: &ToneCurveInput) -> bool {
    let pts: Vec<(f64, f64)> = input.points.iter().map(|p| (p[0], p[1])).collect();
    SplineLut::is_identity(&pts)
}

fn identity_lut() -> SplineLut {
    SplineLut::from_points(&[(0.0, 0.0), (1.0, 1.0)])
}

#[tauri::command]
pub async fn apply_tone_composite_cmd(
    output_dir: String,
    stf_r: Option<[f64; 3]>,
    stf_g: Option<[f64; 3]>,
    stf_b: Option<[f64; 3]>,
    linked_stf: Option<bool>,
    levels_r: Option<ToneLevelsInput>,
    levels_g: Option<ToneLevelsInput>,
    levels_b: Option<ToneLevelsInput>,
    curves_r: Option<ToneCurveInput>,
    curves_g: Option<ToneCurveInput>,
    curves_b: Option<ToneCurveInput>,
    scnr: Option<ScnrConfig>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let src_r = helpers::load_composite_channel(COMPOSITE_KEY_R)?;
        let src_g = helpers::load_composite_channel(COMPOSITE_KEY_G)?;
        let src_b = helpers::load_composite_channel(COMPOSITE_KEY_B)?;

        let (rows, cols) = src_r.arr().dim();

        let stats_r = src_r.stats().clone();
        let stats_g = src_g.stats().clone();
        let stats_b = src_b.stats().clone();

        let stf_config = AutoStfConfig::default();
        let linked = linked_stf.unwrap_or(false);

        let (auto_r, auto_g, auto_b, norm_r, norm_g, norm_b) = if linked {
            let (p, combined) = helpers::compute_linked_stf_with_stats(&stats_r, &stats_g, &stats_b, &stf_config);
            (p, p, p, combined.clone(), combined.clone(), combined)
        } else {
            (
                auto_stf(&stats_r, &stf_config),
                auto_stf(&stats_g, &stf_config),
                auto_stf(&stats_b, &stf_config),
                stats_r.clone(),
                stats_g.clone(),
                stats_b.clone(),
            )
        };

        let stf_r_params = stf_r
            .map(|a| StfParams { shadow: a[0], midtone: a[1], highlight: a[2] })
            .unwrap_or(auto_r);
        let stf_g_params = stf_g
            .map(|a| StfParams { shadow: a[0], midtone: a[1], highlight: a[2] })
            .unwrap_or(auto_g);
        let stf_b_params = stf_b
            .map(|a| StfParams { shadow: a[0], midtone: a[1], highlight: a[2] })
            .unwrap_or(auto_b);

        let (mut r_img, (mut g_img, mut b_img)) = rayon::join(
            || apply_stf_f32(src_r.arr(), &stf_r_params, &norm_r),
            || rayon::join(
                || apply_stf_f32(src_g.arr(), &stf_g_params, &norm_g),
                || apply_stf_f32(src_b.arr(), &stf_b_params, &norm_b),
            ),
        );

        let lr = levels_r.as_ref().map(LevelsParams::from).unwrap_or_default();
        let lg = levels_g.as_ref().map(LevelsParams::from).unwrap_or_default();
        let lb = levels_b.as_ref().map(LevelsParams::from).unwrap_or_default();

        let levels_applied = !lr.is_identity() || !lg.is_identity() || !lb.is_identity();
        if levels_applied {
            let (nr, ng, nb) = apply_levels_rgb(&r_img, &g_img, &b_img, &lr, &lg, &lb);
            r_img = nr;
            g_img = ng;
            b_img = nb;
        }

        let curves_id_r = curves_r.as_ref().map_or(true, is_curve_identity);
        let curves_id_g = curves_g.as_ref().map_or(true, is_curve_identity);
        let curves_id_b = curves_b.as_ref().map_or(true, is_curve_identity);
        let curves_applied = !curves_id_r || !curves_id_g || !curves_id_b;

        if curves_applied {
            let lut_r = curves_r.as_ref().map(build_spline).unwrap_or_else(identity_lut);
            let lut_g = curves_g.as_ref().map(build_spline).unwrap_or_else(identity_lut);
            let lut_b = curves_b.as_ref().map(build_spline).unwrap_or_else(identity_lut);
            let (nr, ng, nb) = apply_curve_rgb(&r_img, &g_img, &b_img, &lut_r, &lut_g, &lut_b);
            r_img = nr;
            g_img = ng;
            b_img = nb;
        }

        let scnr_applied = match scnr {
            Some(ref cfg) if cfg.amount > 1e-7 && r_img.dim() == g_img.dim() && g_img.dim() == b_img.dim() => {
                scnr::apply_scnr_inplace(&mut r_img, &mut g_img, &mut b_img, cfg);
                true
            }
            _ => false,
        };

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let png_path = format!("{}/composite_tone_{}.png", output_dir, ts);
        helpers::render_rgb_preview(&r_img, &g_img, &b_img, &png_path, MAX_PREVIEW_DIM)?;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_DIMENSIONS: [cols, rows],
            RES_COMPOSITE_DIMS: [cols, rows],
            RES_STF_APPLIED: true,
            RES_LEVELS_APPLIED: levels_applied,
            RES_CURVES_APPLIED: curves_applied,
            RES_SCNR_APPLIED: scnr_applied,
            RES_STF: {
                "r": {
                    RES_SHADOW: stf_r_params.shadow,
                    RES_MIDTONE: stf_r_params.midtone,
                    RES_HIGHLIGHT: stf_r_params.highlight,
                },
                "g": {
                    RES_SHADOW: stf_g_params.shadow,
                    RES_MIDTONE: stf_g_params.midtone,
                    RES_HIGHLIGHT: stf_g_params.highlight,
                },
                "b": {
                    RES_SHADOW: stf_b_params.shadow,
                    RES_MIDTONE: stf_b_params.midtone,
                    RES_HIGHLIGHT: stf_b_params.highlight,
                },
            },
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}
