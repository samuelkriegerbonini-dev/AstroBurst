use std::sync::Arc;
use std::time::Instant;

use ndarray::{Array2, Zip};
use serde_json::json;

use crate::cmd::common::{blocking_cmd, resolve_output_dir, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::compose::white_balance::{analyze_wb_reference, validate_wb_factor, WbChannel};
use crate::core::imaging::stats::{compute_image_stats, compute_image_stats_with_known_range};
use crate::core::imaging::stf::{auto_stf, make_stf_u8_fn, AutoStfConfig, StfParams};
use crate::types::image::ImageStats;
use crate::types::constants::{RES_ELAPSED_MS, RES_PNG_PATH, RES_WB_APPLIED, RES_R_FACTOR, RES_G_FACTOR, RES_B_FACTOR, RES_SCNR_APPLIED, RES_AUTO_STF, RES_STAB_R, RES_STAB_G, RES_STAB_B, RES_REF_CHANNEL, RES_RESET, RES_STF_LINKED, STF_B, STF_G, STF_R};

use super::blend::stf_should_unlink;
use super::rgb::composite_png_path;

const PAR_THRESHOLD: usize = 4_000_000;
const IDENTITY_FACTOR_TOLERANCE: f64 = 1e-6;

pub const RES_EMPTY_CHANNELS: &str = "empty_channels";
pub const RES_BLEND_STF_LINKED: &str = "blend_stf_linked";

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

fn content_wants_unlinked(stf_linked: Option<bool>, orig: [&ImageStats; 3]) -> bool {
    stf_linked == Some(false) || stf_should_unlink(orig)
}

fn keeps_unlinked_stretch(content_unlinked: bool, factors: [f64; 3]) -> bool {
    content_unlinked && factors.iter().all(|f| (f - 1.0).abs() <= IDENTITY_FACTOR_TOLERANCE)
}

pub(crate) struct PreviewStf {
    linked: StfParams,
    channels: [StfParams; 3],
    unlinked: bool,
}

impl PreviewStf {
    pub(crate) fn add_to(&self, mut response: serde_json::Value) -> serde_json::Value {
        if let Some(fields) = response.as_object_mut() {
            fields.insert(RES_AUTO_STF.to_string(), helpers::stf_json(&self.linked));
            fields.insert(RES_STF_LINKED.to_string(), json!(!self.unlinked));
            fields.insert(STF_R.to_string(), helpers::stf_json(&self.channels[0]));
            fields.insert(STF_G.to_string(), helpers::stf_json(&self.channels[1]));
            fields.insert(STF_B.to_string(), helpers::stf_json(&self.channels[2]));
        }
        response
    }
}

pub(crate) fn render_balanced_preview(
    planes: [&Array2<f32>; 3],
    stats: [&ImageStats; 3],
    unlinked: bool,
    png_path: &str,
) -> anyhow::Result<PreviewStf> {
    let stf_config = AutoStfConfig::default();
    let (linked, combined_stats) = helpers::compute_linked_stf_with_stats(stats[0], stats[1], stats[2], &stf_config);
    let (channels, norms) = if unlinked {
        (stats.map(|s| auto_stf(s, &stf_config)), stats)
    } else {
        ([linked; 3], [&combined_stats; 3])
    };
    helpers::render_rgb_preview_with_stf(
        planes[0],
        planes[1],
        planes[2],
        make_stf_u8_fn(&channels[0], norms[0]),
        make_stf_u8_fn(&channels[1], norms[1]),
        make_stf_u8_fn(&channels[2], norms[2]),
        png_path,
        MAX_PREVIEW_DIM,
    )?;
    Ok(PreviewStf { linked, channels, unlinked })
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
    stf_linked: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let (orig_r, orig_g, orig_b) = helpers::load_composite_orig_rgb()?;

        let stats_r = orig_r.stats().clone();
        let stats_g = orig_g.stats().clone();
        let stats_b = orig_b.stats().clone();

        let png_path = composite_png_path(&output_dir);
        let stf = render_balanced_preview(
            [orig_r.arr(), orig_g.arr(), orig_b.arr()],
            [&stats_r, &stats_g, &stats_b],
            content_wants_unlinked(stf_linked, [&stats_r, &stats_g, &stats_b]),
            &png_path,
        )?;

        helpers::insert_composite_white_balanced(
            [
                (orig_r.data_arc(), stats_r),
                (orig_g.data_arc(), stats_g),
                (orig_b.data_arc(), stats_b),
            ],
            [1.0; 3],
        );

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(stf.add_to(json!({
            RES_PNG_PATH: png_path,
            RES_RESET: true,
            RES_R_FACTOR: 1.0,
            RES_G_FACTOR: 1.0,
            RES_B_FACTOR: 1.0,
            RES_ELAPSED_MS: elapsed,
        })))
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
    stf_linked: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let (orig_r, orig_g, orig_b) = helpers::load_composite_orig_rgb()?;

        let (rf, gf, bf) = validated_wb_factors(r_factor, g_factor, b_factor)?;

        let sr = orig_r.stats();
        let sg = orig_g.stats();
        let sb = orig_b.stats();
        let content_unlinked = content_wants_unlinked(stf_linked, [sr, sg, sb]);

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
        let stf = render_balanced_preview(
            [&r, &g, &b],
            [&stats_r, &stats_g, &stats_b],
            keeps_unlinked_stretch(content_unlinked, [r_factor, g_factor, b_factor]),
            &png_path,
        )?;
        helpers::insert_composite_white_balanced(
            [(Arc::new(r), stats_r), (Arc::new(g), stats_g), (Arc::new(b), stats_b)],
            [rf, gf, bf],
        );

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(stf.add_to(json!({
            RES_PNG_PATH: png_path,
            RES_WB_APPLIED: true,
            RES_R_FACTOR: r_factor,
            RES_G_FACTOR: g_factor,
            RES_B_FACTOR: b_factor,
            RES_SCNR_APPLIED: scnr_applied,
            RES_BLEND_STF_LINKED: !content_unlinked,
            RES_ELAPSED_MS: elapsed,
        })))
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
        calibrate_and_scnr_cmd(out.to_string(), f.0, f.1, f.2, None, None, None, None, None).await.unwrap();
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

        super::super::lrgb_combine_composite_cmd(l_path, out.clone(), Some(1.0), Some(1.0), None).await.unwrap();
        let lrgb = [slot(COMPOSITE_KEY_R), slot(COMPOSITE_KEY_G), slot(COMPOSITE_KEY_B)];
        assert_close(&slot(COMPOSITE_ORIG_R), &lrgb[0].mapv(|v| v / 2.0), "ORIG after LRGB");

        reset_wb_cmd(out.clone(), None).await.unwrap();
        assert_close(&slot(COMPOSITE_KEY_R), &lrgb[0].mapv(|v| v / 2.0), "reset dropped the LRGB luminance");
        assert_close(&slot(COMPOSITE_KEY_B), &lrgb[2].mapv(|v| v / 0.5), "reset dropped the LRGB luminance");
        assert_eq!(helpers::composite_wb_factors(), [1.0; 3]);

        apply_wb(&out, (2.0, 1.0, 0.5)).await;
        assert_close(&slot(COMPOSITE_KEY_R), &lrgb[0], "re-applying the same balance changed the LRGB result");
        assert_close(&slot(COMPOSITE_KEY_G), &lrgb[1], "re-applying the same balance changed the LRGB result");
        assert_close(&slot(COMPOSITE_KEY_B), &lrgb[2], "re-applying the same balance changed the LRGB result");
    }

    #[tokio::test]
    async fn reset_and_calibrate_on_a_cleared_composite_say_to_run_blend_again() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        helpers::clear_composite();
        let expected = "The colour composite is no longer in memory; run Blend again.";
        let reset = reset_wb_cmd(out.clone(), None).await.expect_err("reset on a cleared composite must fail");
        assert_eq!(reset, expected);
        let calibrate = calibrate_and_scnr_cmd(out, 1.0, 1.0, 1.0, None, None, None, None, None)
            .await
            .expect_err("calibrate on a cleared composite must fail");
        assert_eq!(calibrate, expected);
    }

    fn unbalanced_planes() -> [Array2<f32>; 3] {
        [
            plane(1.0),
            Array2::from_shape_fn((16, 16), |(y, x)| 200.0 + ((y * 16 + x) % 7) as f32 * 30.0 + if x == 3 { 900.0 } else { 0.0 }),
            Array2::from_shape_fn((16, 16), |(y, x)| 40.0 + ((y * 5 + x * 3) % 31) as f32 * 0.5),
        ]
    }

    fn per_channel_png(planes: &[Array2<f32>; 3], path: &str) {
        let config = AutoStfConfig::default();
        let stats = planes.each_ref().map(compute_image_stats);
        let stfs = stats.each_ref().map(|s| auto_stf(s, &config));
        helpers::render_rgb_preview_with_stf(
            &planes[0],
            &planes[1],
            &planes[2],
            make_stf_u8_fn(&stfs[0], &stats[0]),
            make_stf_u8_fn(&stfs[1], &stats[1]),
            make_stf_u8_fn(&stfs[2], &stats[2]),
            path,
            MAX_PREVIEW_DIM,
        )
        .unwrap();
    }

    fn png_of(res: &serde_json::Value) -> Vec<u8> {
        std::fs::read(res[RES_PNG_PATH].as_str().expect("png path")).unwrap()
    }

    #[tokio::test]
    async fn identity_white_balance_keeps_an_unlinked_stretch() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let planes = unbalanced_planes();
        let config = AutoStfConfig::default();
        let per_channel =
            planes.each_ref().map(|p| helpers::stf_json(&auto_stf(&compute_image_stats(p), &config)));
        let expected_png = dir.path().join("expected.png").to_str().unwrap().to_string();
        per_channel_png(&planes, &expected_png);
        blend(planes[0].clone(), planes[1].clone(), planes[2].clone());

        let mut unlinked = Vec::new();
        let applied = calibrate_and_scnr_cmd(out.clone(), 1.0, 1.0, 1.0, None, None, None, None, Some(false)).await;
        if let Ok(res) = &applied {
            unlinked.push((res.clone(), png_of(res)));
        }
        let reset = reset_wb_cmd(out.clone(), Some(false)).await;
        if let Ok(res) = &reset {
            unlinked.push((res.clone(), png_of(res)));
        }
        helpers::clear_composite();
        applied.unwrap();
        reset.unwrap();

        for (res, png) in unlinked {
            assert_eq!(res[RES_STF_LINKED], false, "{res}");
            assert_eq!([&res[STF_R], &res[STF_G], &res[STF_B]], per_channel.each_ref(), "{res}");
            assert_ne!(res[STF_R], res[STF_G], "{res}");
            assert!(png == std::fs::read(&expected_png).unwrap(), "the preview was not stretched per channel: {res}");
        }
    }

    #[tokio::test]
    async fn reset_after_a_white_balance_restores_the_unlinked_blend_stretch() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let planes = unbalanced_planes();
        let config = AutoStfConfig::default();
        let per_channel =
            planes.each_ref().map(|p| helpers::stf_json(&auto_stf(&compute_image_stats(p), &config)));
        blend(planes[0].clone(), planes[1].clone(), planes[2].clone());

        let balanced = calibrate_and_scnr_cmd(out.clone(), 1.2, 1.0, 0.8, None, None, None, None, Some(false)).await;
        let reset = reset_wb_cmd(out.clone(), Some(true)).await;
        let neutral = calibrate_and_scnr_cmd(out.clone(), 1.0, 1.0, 1.0, None, None, None, None, Some(true)).await;
        blend(plane(1.0), plane(1.1), plane(1.2));
        let even = reset_wb_cmd(out.clone(), Some(true)).await;
        let even_neutral = calibrate_and_scnr_cmd(out.clone(), 1.0, 1.0, 1.0, None, None, None, None, None).await;
        helpers::clear_composite();
        let (balanced, reset, neutral) = (balanced.unwrap(), reset.unwrap(), neutral.unwrap());
        let (even, even_neutral) = (even.unwrap(), even_neutral.unwrap());

        assert_eq!(balanced[RES_STF_LINKED], true, "{balanced}");
        assert_eq!(balanced[RES_BLEND_STF_LINKED], false, "{balanced}");
        for res in [&reset, &neutral] {
            assert_eq!(res[RES_STF_LINKED], false, "{res}");
            assert_eq!([&res[STF_R], &res[STF_G], &res[STF_B]], per_channel.each_ref(), "{res}");
        }
        assert_eq!(neutral[RES_BLEND_STF_LINKED], false, "{neutral}");
        for res in [&even, &even_neutral] {
            assert_eq!(res[RES_STF_LINKED], true, "{res}");
            assert_eq!(res[STF_R], res[RES_AUTO_STF], "{res}");
            assert_eq!(res[STF_B], res[RES_AUTO_STF], "{res}");
        }
        assert_eq!(even_neutral[RES_BLEND_STF_LINKED], true, "{even_neutral}");
    }

    #[tokio::test]
    async fn non_identity_white_balance_is_linked() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let [r, g, b] = unbalanced_planes();
        blend(r, g, b);

        let res = calibrate_and_scnr_cmd(out, 2.0, 1.0, 0.5, None, None, None, None, Some(false)).await;
        let balanced = [COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B].map(|k| GLOBAL_IMAGE_CACHE.get(k));
        helpers::clear_composite();
        let res = res.unwrap();

        let [br, bg, bb] = balanced.map(|e| e.expect("balanced channel"));
        let (expected, _) = helpers::compute_linked_stf_with_stats(br.stats(), bg.stats(), bb.stats(), &AutoStfConfig::default());
        let expected = helpers::stf_json(&expected);
        assert_eq!(res[RES_STF_LINKED], true, "{res}");
        assert_eq!(res[RES_AUTO_STF], expected, "{res}");
        for key in [STF_R, STF_G, STF_B] {
            assert_eq!(res[key], expected, "{key}: {res}");
        }
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
