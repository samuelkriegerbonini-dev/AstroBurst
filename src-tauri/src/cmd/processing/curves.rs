use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, resolve_output_dir};
use crate::cmd::helpers;
use crate::cmd::processing::local_contrast::store_tone_result;
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
    RES_R, RES_G, RES_B,
};
use crate::types::image::{ScnrConfig, StfParams};

pub const RES_CONTRAST_REAPPLIED: &str = "contrast_reapplied";

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
        let output_dir = resolve_output_dir(&output_dir)?;

        let (mut r_img, mut g_img, mut b_img, stf_applied, stf_r_params, stf_g_params, stf_b_params) =
            if let Some((er, eg, eb)) = helpers::load_composite_stretched() {
                (
                    er.arr().to_owned(),
                    eg.arr().to_owned(),
                    eb.arr().to_owned(),
                    false,
                    StfParams::default(),
                    StfParams::default(),
                    StfParams::default(),
                )
            } else {
                let src_r = helpers::load_composite_channel(COMPOSITE_KEY_R)?;
                let src_g = helpers::load_composite_channel(COMPOSITE_KEY_G)?;
                let src_b = helpers::load_composite_channel(COMPOSITE_KEY_B)?;

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

                let (r_img, (g_img, b_img)) = rayon::join(
                    || apply_stf_f32(src_r.arr(), &stf_r_params, &norm_r),
                    || rayon::join(
                        || apply_stf_f32(src_g.arr(), &stf_g_params, &norm_g),
                        || apply_stf_f32(src_b.arr(), &stf_b_params, &norm_b),
                    ),
                );
                (r_img, g_img, b_img, true, stf_r_params, stf_g_params, stf_b_params)
            };

        let (rows, cols) = r_img.dim();

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
        let contrast_reapplied = store_tone_result(r_img, g_img, b_img, &png_path)?;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_CONTRAST_REAPPLIED: contrast_reapplied,
            RES_DIMENSIONS: [cols, rows],
            RES_COMPOSITE_DIMS: [cols, rows],
            RES_STF_APPLIED: stf_applied,
            RES_LEVELS_APPLIED: levels_applied,
            RES_CURVES_APPLIED: curves_applied,
            RES_SCNR_APPLIED: scnr_applied,
            RES_STF: {
                RES_R: {
                    RES_SHADOW: stf_r_params.shadow,
                    RES_MIDTONE: stf_r_params.midtone,
                    RES_HIGHLIGHT: stf_r_params.highlight,
                },
                RES_G: {
                    RES_SHADOW: stf_g_params.shadow,
                    RES_MIDTONE: stf_g_params.midtone,
                    RES_HIGHLIGHT: stf_g_params.highlight,
                },
                RES_B: {
                    RES_SHADOW: stf_b_params.shadow,
                    RES_MIDTONE: stf_b_params.midtone,
                    RES_HIGHLIGHT: stf_b_params.highlight,
                },
            },
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use ndarray::Array2;

    use crate::cmd::processing::{hdrmt_composite_cmd, lhe_composite_cmd};
    use crate::core::imaging::hdr::{hdrmt_rgb, HdrConfig};
    use crate::core::imaging::local_contrast::{lhe_rgb, LheConfig};

    type Triplet = (Array2<f32>, Array2<f32>, Array2<f32>);

    fn plane(seed: usize) -> Array2<f32> {
        Array2::from_shape_fn((16, 16), |(y, x)| ((y * 16 + x + seed * 5) % 37) as f32 / 40.0 + 0.05)
    }

    fn curve(midpoint: f64) -> ToneCurveInput {
        ToneCurveInput { points: vec![[0.0, 0.0], [0.5, midpoint], [1.0, 1.0]] }
    }

    fn curved(stretched: &[Array2<f32>; 3], input: &ToneCurveInput) -> Triplet {
        let lut = build_spline(input);
        apply_curve_rgb(&stretched[0], &stretched[1], &stretched[2], &lut, &lut, &lut)
    }

    async fn adjust(out: &str, input: &ToneCurveInput) -> Result<serde_json::Value, String> {
        let c = || Some(input.clone());
        apply_tone_composite_cmd(out.to_string(), None, None, None, None, None, None, None, c(), c(), c(), None).await
    }

    fn toned_tier() -> Triplet {
        let (r, g, b) = helpers::load_composite_toned().expect("toned tier");
        (r.arr().clone(), g.arr().clone(), b.arr().clone())
    }

    #[tokio::test]
    async fn new_curves_after_a_composite_contrast_step_re_apply_it_instead_of_dropping_it() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let stretched = [plane(1), plane(2), plane(3)];
        helpers::clear_composite_derived();
        helpers::insert_composite_stretched(stretched[0].clone(), stretched[1].clone(), stretched[2].clone());

        let (c1, c2) = (curve(0.7), curve(0.35));
        let lhe = LheConfig { kernel_radius: 3, ..LheConfig::default() };
        let hdr = HdrConfig { layers: 2, ..HdrConfig::default() };
        let hdr_again = HdrConfig { layers: 3, ..HdrConfig::default() };

        let first = adjust(&out, &c1).await;
        let lhe_run = lhe_composite_cmd(out.clone(), lhe.clone()).await;
        let second = adjust(&out, &c2).await;
        let after_second = helpers::load_composite_toned().map(|_| toned_tier());
        let hdr_run = hdrmt_composite_cmd(out.clone(), hdr.clone()).await;
        let third = adjust(&out, &c1).await;
        let after_third = helpers::load_composite_toned().map(|_| toned_tier());
        let hdr_rerun = hdrmt_composite_cmd(out, hdr_again.clone()).await;
        let after_rerun = helpers::load_composite_toned().map(|_| toned_tier());
        helpers::clear_composite_derived();
        lhe_run.unwrap();
        hdr_run.unwrap();
        hdr_rerun.unwrap();

        let t2 = curved(&stretched, &c2);
        let expected_second = lhe_rgb(&t2.0, &t2.1, &t2.2, &lhe).unwrap();
        assert!(after_second == Some(expected_second), "the new curves dropped the LHE result");

        let t1 = curved(&stretched, &c1);
        let l1 = lhe_rgb(&t1.0, &t1.1, &t1.2, &lhe).unwrap();
        let expected_third = hdrmt_rgb(&l1.0, &l1.1, &l1.2, &hdr).unwrap();
        assert!(after_third == Some(expected_third), "LHE then HDR were not replayed in order on the new curves");

        let expected_rerun = hdrmt_rgb(&l1.0, &l1.1, &l1.2, &hdr_again).unwrap();
        assert!(after_rerun == Some(expected_rerun), "an HDR re-run after Adjust compounded on its own output");

        assert_eq!(first.unwrap()[RES_CONTRAST_REAPPLIED], json!([]));
        assert_eq!(second.unwrap()[RES_CONTRAST_REAPPLIED], json!(["lhe"]));
        assert_eq!(third.unwrap()[RES_CONTRAST_REAPPLIED], json!(["lhe", "hdr"]));
    }
}
