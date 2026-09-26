use std::sync::Arc;
use std::time::Instant;

use ndarray::{Array2, Zip};
use serde_json::json;

use crate::cmd::common::{blocking_cmd, resolve_output_dir, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::compose::white_balance::{analyze_wb_reference, validate_wb_factor, WbChannel};
use crate::core::imaging::stats::{compute_image_stats, compute_image_stats_with_known_range};
use crate::core::imaging::stf::{make_stf_u8_fn, AutoStfConfig};
use crate::types::image::ImageStats;
use crate::types::constants::{RES_ELAPSED_MS, RES_PNG_PATH, RES_WB_APPLIED, RES_R_FACTOR, RES_G_FACTOR, RES_B_FACTOR, RES_SCNR_APPLIED, RES_AUTO_STF, RES_STAB_R, RES_STAB_G, RES_STAB_B, RES_REF_CHANNEL, RES_RESET};

use super::rgb::composite_png_path;

const PAR_THRESHOLD: usize = 4_000_000;

pub const RES_EMPTY_CHANNELS: &str = "empty_channels";

fn validated_wb_factors(r_factor: f64, g_factor: f64, b_factor: f64) -> anyhow::Result<(f32, f32, f32)> {
    Ok((
        validate_wb_factor(WbChannel::R, r_factor)? as f32,
        validate_wb_factor(WbChannel::G, g_factor)? as f32,
        validate_wb_factor(WbChannel::B, b_factor)? as f32,
    ))
}

fn auto_wb_payload(sr: &ImageStats, sg: &ImageStats, sb: &ImageStats) -> anyhow::Result<serde_json::Value> {
    let analysis = analyze_wb_reference(sr, sg, sb);
    analysis.median_ratio_is_valid()?;

    let reference = analysis.reference.ok_or_else(|| {
        anyhow::anyhow!("Auto white balance found no usable signal: R, G and B are all empty. Check the blend inputs.")
    })?;

    let (wb_r, wb_g, wb_b) = analysis.factors;
    let empty: Vec<&str> = analysis.empty_channels.iter().map(|c| c.as_str()).collect();

    Ok(json!({
        RES_R_FACTOR: wb_r,
        RES_G_FACTOR: wb_g,
        RES_B_FACTOR: wb_b,
        RES_STAB_R: analysis.stability.0,
        RES_STAB_G: analysis.stability.1,
        RES_STAB_B: analysis.stability.2,
        RES_REF_CHANNEL: reference.as_str(),
        RES_EMPTY_CHANNELS: empty,
    }))
}

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

        let (orig_r, orig_g, orig_b) = helpers::load_composite_orig_rgb()?;

        let stats_r = orig_r.stats().clone();
        let stats_g = orig_g.stats().clone();
        let stats_b = orig_b.stats().clone();

        let png_path = composite_png_path(&output_dir);

        let stf_config = AutoStfConfig::default();
        let (linked_stf, combined_stats) =
            helpers::compute_linked_stf_with_stats(&stats_r, &stats_g, &stats_b, &stf_config);
        let fn_r = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_g = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_b = make_stf_u8_fn(&linked_stf, &combined_stats);
        helpers::render_rgb_preview_with_stf(orig_r.arr(), orig_g.arr(), orig_b.arr(), fn_r, fn_g, fn_b, &png_path, MAX_PREVIEW_DIM)?;

        helpers::insert_composite_white_balanced(
            [
                (orig_r.data_arc(), stats_r),
                (orig_g.data_arc(), stats_g),
                (orig_b.data_arc(), stats_b),
            ],
            [1.0; 3],
        );

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_RESET: true,
            RES_R_FACTOR: 1.0,
            RES_G_FACTOR: 1.0,
            RES_B_FACTOR: 1.0,
            RES_AUTO_STF: helpers::stf_json(&linked_stf),
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

        let (orig_r, orig_g, orig_b) = helpers::load_composite_orig_rgb()?;

        let (rf, gf, bf) = validated_wb_factors(r_factor, g_factor, b_factor)?;

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
        let (linked_stf, combined_stats) =
            helpers::compute_linked_stf_with_stats(&stats_r, &stats_g, &stats_b, &stf_config);
        let fn_r = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_g = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_b = make_stf_u8_fn(&linked_stf, &combined_stats);
        helpers::render_rgb_preview_with_stf(&r, &g, &b, fn_r, fn_g, fn_b, &png_path, MAX_PREVIEW_DIM)?;
        helpers::insert_composite_white_balanced(
            [(Arc::new(r), stats_r), (Arc::new(g), stats_g), (Arc::new(b), stats_b)],
            [rf, gf, bf],
        );

        let stf_json = helpers::stf_json(&linked_stf);
        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_WB_APPLIED: true,
            RES_R_FACTOR: r_factor,
            RES_G_FACTOR: g_factor,
            RES_B_FACTOR: b_factor,
            RES_SCNR_APPLIED: scnr_applied,
            RES_AUTO_STF: stf_json,
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

#[tauri::command]
pub async fn compute_auto_wb_cmd() -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let (entry_r, entry_g, entry_b) = helpers::load_orig_or_composite()?;

        auto_wb_payload(entry_r.stats(), entry_g.stats(), entry_b.stats())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_stats(median: f64, mad: f64) -> ImageStats {
        ImageStats {
            min: 0.0,
            max: 1.0,
            median,
            mad,
            sigma: mad * 1.4826,
            mean: median,
            valid_count: 1000,
        }
    }

    #[test]
    fn auto_wb_payload_reports_empty_channels_as_neutral() {
        let payload = auto_wb_payload(
            &ImageStats::default(),
            &make_stats(0.5, 0.01),
            &ImageStats::default(),
        )
        .unwrap();

        assert_eq!(payload[RES_R_FACTOR], 1.0);
        assert_eq!(payload[RES_G_FACTOR], 1.0);
        assert_eq!(payload[RES_B_FACTOR], 1.0);
        assert_eq!(payload[RES_REF_CHANNEL], "G");
        assert_eq!(payload[RES_EMPTY_CHANNELS], json!(["R", "B"]));
        assert!(payload[RES_STAB_R].is_null());
    }

    #[test]
    fn auto_wb_payload_fails_when_every_channel_is_empty() {
        let err = auto_wb_payload(
            &ImageStats::default(),
            &ImageStats::default(),
            &ImageStats::default(),
        )
        .expect_err("an entirely empty composite must not report success");

        assert!(
            err.to_string().contains("no usable signal"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn auto_wb_payload_keeps_usable_channels_unchanged() {
        let sr = make_stats(0.5, 0.001);
        let sg = make_stats(0.25, 0.02);
        let sb = make_stats(0.125, 0.03);
        let payload = auto_wb_payload(&sr, &sg, &sb).unwrap();

        assert_eq!(payload[RES_REF_CHANNEL], "R");
        assert_eq!(payload[RES_R_FACTOR], 1.0);
        assert_eq!(payload[RES_G_FACTOR], 2.0);
        assert_eq!(payload[RES_B_FACTOR], 4.0);
        assert_eq!(payload[RES_EMPTY_CHANNELS], json!([]));
    }

    use crate::infra::cache::GLOBAL_IMAGE_CACHE;
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::constants::{COMPOSITE_KEY_B, COMPOSITE_KEY_G, COMPOSITE_KEY_R, COMPOSITE_ORIG_R};

    fn plane(seed: f32) -> Array2<f32> {
        Array2::from_shape_fn((16, 16), |(y, x)| 50.0 + seed * 10.0 + ((y * 16 + x) % 23) as f32)
    }

    fn slot(key: &str) -> Array2<f32> {
        GLOBAL_IMAGE_CACHE.get(key).expect(key).arr().clone()
    }

    fn blend(r: Array2<f32>, g: Array2<f32>, b: Array2<f32>) {
        let (sr, sg, sb) = (compute_image_stats(&r), compute_image_stats(&g), compute_image_stats(&b));
        helpers::insert_composite_and_orig(r, g, b, sr, sg, sb);
    }

    fn assert_close(a: &Array2<f32>, b: &Array2<f32>, what: &str) {
        assert_eq!(a.dim(), b.dim(), "{what}");
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() <= 1e-4 * y.abs().max(1e-3), "{what}: {x} vs {y}");
        }
    }

    async fn apply_wb(out: &str, f: (f64, f64, f64)) {
        calibrate_and_scnr_cmd(out.to_string(), f.0, f.1, f.2, None, None, None, None).await.unwrap();
    }

    #[tokio::test]
    async fn color_balance_apply_and_reset_keep_the_lrgb_result() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let l_path = dir.path().join("lum.fits").to_str().unwrap().to_string();
        write_fits_mono(&l_path, &Array2::from_shape_fn((16, 16), |(y, x)| (y * 7 + x * 3) as f32), None).unwrap();

        blend(plane(1.0), plane(2.0), plane(3.0));
        apply_wb(&out, (2.0, 1.0, 0.5)).await;
        assert_eq!(helpers::composite_wb_factors(), [2.0, 1.0, 0.5]);

        super::super::lrgb_combine_composite_cmd(l_path, out.clone(), Some(1.0), Some(1.0)).await.unwrap();
        let lrgb = [slot(COMPOSITE_KEY_R), slot(COMPOSITE_KEY_G), slot(COMPOSITE_KEY_B)];
        assert_close(&slot(COMPOSITE_ORIG_R), &lrgb[0].mapv(|v| v / 2.0), "ORIG after LRGB");

        reset_wb_cmd(out.clone()).await.unwrap();
        assert_close(&slot(COMPOSITE_KEY_R), &lrgb[0].mapv(|v| v / 2.0), "reset dropped the LRGB luminance");
        assert_close(&slot(COMPOSITE_KEY_B), &lrgb[2].mapv(|v| v / 0.5), "reset dropped the LRGB luminance");
        assert_eq!(helpers::composite_wb_factors(), [1.0; 3]);

        apply_wb(&out, (2.0, 1.0, 0.5)).await;
        assert_close(&slot(COMPOSITE_KEY_R), &lrgb[0], "re-applying the same balance changed the LRGB result");
        assert_close(&slot(COMPOSITE_KEY_G), &lrgb[1], "re-applying the same balance changed the LRGB result");
        assert_close(&slot(COMPOSITE_KEY_B), &lrgb[2], "re-applying the same balance changed the LRGB result");
    }

    #[tokio::test]
    async fn a_replaced_channel_is_white_balanced_and_survives_a_reset() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let processed = dir.path().join("r_processed.fits").to_str().unwrap().to_string();
        let new_r = plane(7.0);
        write_fits_mono(&processed, &new_r, None).unwrap();

        blend(plane(1.0), plane(2.0), plane(3.0));
        apply_wb(&out, (2.0, 1.0, 1.0)).await;
        super::super::update_composite_channel_cmd("r".to_string(), processed.clone()).await.unwrap();
        assert_close(&slot(COMPOSITE_KEY_R), &new_r.mapv(|v| v * 2.0), "the synced channel lost its white balance");
        assert_close(&slot(COMPOSITE_ORIG_R), &new_r, "the synced channel is missing from the unbalanced base");

        reset_wb_cmd(out.clone()).await.unwrap();
        assert_close(&slot(COMPOSITE_KEY_R), &new_r, "reset reverted the synced channel to the blend");

        let bigger = dir.path().join("r_full_frame.fits").to_str().unwrap().to_string();
        write_fits_mono(&bigger, &Array2::from_elem((20, 24), 5.0), None).unwrap();
        let err = super::super::update_composite_channel_cmd("r".to_string(), bigger).await.unwrap_err();
        assert!(err.contains("Re-run Blend"), "{err}");
        assert!(err.contains("24x20") && err.contains("16x16"), "{err}");
        assert_close(&slot(COMPOSITE_KEY_R), &new_r, "a refused channel still replaced the composite");
    }

    #[tokio::test]
    async fn reset_and_calibrate_on_a_cleared_composite_say_to_run_blend_again() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        helpers::clear_composite();
        let expected = "The colour composite is no longer in memory; run Blend again.";
        let reset = reset_wb_cmd(out.clone()).await.expect_err("reset on a cleared composite must fail");
        assert_eq!(reset, expected);
        let calibrate = calibrate_and_scnr_cmd(out, 1.0, 1.0, 1.0, None, None, None, None)
            .await
            .expect_err("calibrate on a cleared composite must fail");
        assert_eq!(calibrate, expected);
    }

    #[test]
    fn applied_factors_reject_out_of_range_gain() {
        let err = validated_wb_factors(63_671_007_156.372_07, 1.0, 63_671_007_156.372_07)
            .expect_err("a 6.4e10 gain must not be applied");
        assert!(err.to_string().contains("channel R"), "unexpected error: {}", err);

        let err = validated_wb_factors(1.0, 1.0, 0.0)
            .expect_err("a zero gain must not be applied");
        assert!(err.to_string().contains("channel B"), "unexpected error: {}", err);

        let (r, g, b) = validated_wb_factors(1.3, 1.0, 2.5).unwrap();
        assert_eq!((r, g, b), (1.3f32, 1.0f32, 2.5f32));
    }
}
