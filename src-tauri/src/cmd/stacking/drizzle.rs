use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::cmd::common::{blocking_cmd, resolve_output_dir};
use crate::cmd::helpers;
use crate::core::compose::drizzle_rgb::drizzle_rgb_with;
use crate::core::stacking::memory::effective_memory;
use crate::core::stacking::STAGE_LOW_MEMORY;
use crate::infra::progress::ProgressHandle;
use crate::types::compose::{DrizzleRgbConfig, DrizzleRgbResult};
use crate::types::constants::{
    DEFAULT_DRIZZLE_PIXFRAC, DEFAULT_DRIZZLE_SCALE, DEFAULT_DRIZZLE_SIGMA,
    DEFAULT_DRIZZLE_SIGMA_ITERS,
    EVENT_DRIZZLE_RGB_PROGRESS,
    STAGE_SAVE,
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH,
    RES_FRAME_COUNT_B, RES_FRAME_COUNT_G, RES_FRAME_COUNT_R,
    RES_INPUT_DIMS, RES_OUTPUT_DIMS,
    RES_PNG_PATH, RES_REJECTED_PIXELS, RES_SCALE, RES_WARNINGS,
};
use crate::types::stacking::{AlignmentMethod, DrizzleConfig};

const DRIZZLE_RGB_STEM: &str = "drizzle_rgb";

fn parse_alignment_method(name: Option<&str>) -> AlignmentMethod {
    match name.map(|n| n.trim().to_ascii_lowercase()).as_deref() {
        Some("affine") | Some("zncc") => AlignmentMethod::Affine,
        _ => AlignmentMethod::PhaseCorrelation,
    }
}

fn drizzle_rgb_tick_total(counts: &[usize], align: bool) -> u64 {
    let channel_ticks = |n: usize| {
        let aligns = if align { n.saturating_sub(1) } else { 0 };
        n + aligns + usize::from(n > 0)
    };
    (counts.iter().map(|&n| channel_ticks(n)).sum::<usize>() + 1) as u64
}

fn drizzle_rgb_output_paths(dir: &str, stamp_ms: u128, save_fits: bool) -> (String, Option<String>) {
    let stem = format!("{}/{}_{}", dir, DRIZZLE_RGB_STEM, stamp_ms);
    (format!("{}.png", stem), save_fits.then(|| format!("{}.fits", stem)))
}

fn drizzle_rgb_json(result: &DrizzleRgbResult, elapsed_ms: u64) -> serde_json::Value {
    let (out_h, out_w) = result.output_dims;
    let (in_h, in_w) = result.input_dims;
    json!({
        RES_PNG_PATH: result.png_path,
        RES_FITS_PATH: result.fits_path,
        RES_DIMENSIONS: [out_w, out_h],
        RES_OUTPUT_DIMS: [out_w, out_h],
        RES_INPUT_DIMS: [in_w, in_h],
        RES_FRAME_COUNT_R: result.frame_count_r,
        RES_FRAME_COUNT_G: result.frame_count_g,
        RES_FRAME_COUNT_B: result.frame_count_b,
        RES_REJECTED_PIXELS: result.rejected_pixels,
        RES_ELAPSED_MS: elapsed_ms,
        RES_SCALE: result.scale,
        RES_WARNINGS: result.warnings,
    })
}

#[tauri::command]
pub async fn drizzle_rgb_cmd(
    app: tauri::AppHandle,
    r_paths: Option<Vec<String>>,
    g_paths: Option<Vec<String>>,
    b_paths: Option<Vec<String>>,
    output_dir: String,
    scale: Option<f64>,
    pixfrac: Option<f64>,
    kernel: Option<String>,
    sigma_low: Option<f32>,
    sigma_high: Option<f32>,
    rejection: Option<String>,
    align: Option<bool>,
    alignment_method: Option<String>,
    wb_mode: Option<String>,
    wb_r: Option<f64>,
    wb_g: Option<f64>,
    wb_b: Option<f64>,
    scnr_enabled: Option<bool>,
    scnr_method: Option<String>,
    scnr_amount: Option<f64>,
    save_fits: Option<bool>,
) -> Result<serde_json::Value, String> {
    let counts = [&r_paths, &g_paths, &b_paths].map(|paths| paths.as_ref().map_or(0, Vec::len));
    let progress = ProgressHandle::new(&app, EVENT_DRIZZLE_RGB_PROGRESS, drizzle_rgb_tick_total(&counts, align.unwrap_or(true)));
    let scale_val = scale.unwrap_or(DEFAULT_DRIZZLE_SCALE);

    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let k = helpers::parse_drizzle_kernel(kernel.as_deref())?;
        let rejection_method = helpers::parse_rejection_method(rejection.as_deref())?;

        let am = parse_alignment_method(alignment_method.as_deref());

        let drizzle_cfg = DrizzleConfig {
            scale: scale_val,
            pixfrac: pixfrac.unwrap_or(DEFAULT_DRIZZLE_PIXFRAC),
            kernel: k,
            sigma_low: sigma_low.unwrap_or(DEFAULT_DRIZZLE_SIGMA),
            sigma_high: sigma_high.unwrap_or(DEFAULT_DRIZZLE_SIGMA),
            sigma_iterations: DEFAULT_DRIZZLE_SIGMA_ITERS,
            align: align.unwrap_or(true),
            alignment_method: am,
            rejection: rejection_method,
        };

        let wb = helpers::parse_wb(wb_mode.as_deref(), wb_r, wb_g, wb_b);

        let scnr_cfg = helpers::parse_scnr_config(
            scnr_enabled,
            scnr_method.as_deref(),
            scnr_amount,
            None,
        );

        let config = DrizzleRgbConfig {
            drizzle: drizzle_cfg,
            white_balance: wb,
            auto_stretch: true,
            linked_stf: false,
            scnr: scnr_cfg,
            align: align.unwrap_or(true),
            align_method: match am {
                AlignmentMethod::Affine => crate::types::compose::AlignMethod::Affine,
                AlignmentMethod::PhaseCorrelation => crate::types::compose::AlignMethod::PhaseCorrelation,
            },
        };

        let stamp_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let (png_path, fits_out) = drizzle_rgb_output_paths(&output_dir, stamp_ms, save_fits.unwrap_or(false));

        let cancelled = || progress.is_cancelled();
        let on_stage = |stage: &str| {
            if stage == STAGE_LOW_MEMORY {
                progress.emit_stage(stage);
            } else {
                progress.tick_with_stage(stage);
            }
        };
        let result = drizzle_rgb_with(
            r_paths.as_deref(),
            g_paths.as_deref(),
            b_paths.as_deref(),
            &png_path,
            fits_out.as_deref(),
            &config,
            &cancelled,
            &on_stage,
            effective_memory(),
        )?;

        progress.tick_with_stage(STAGE_SAVE);
        progress.emit_complete();

        Ok(drizzle_rgb_json(&result, t0.elapsed().as_millis() as u64))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::compose::ChannelStats;
    use crate::types::image::StfParams;

    #[test]
    fn star_based_alignment_accepts_the_affine_name_and_the_legacy_zncc_name() {
        assert_eq!(parse_alignment_method(Some("affine")), AlignmentMethod::Affine);
        assert_eq!(parse_alignment_method(Some("zncc")), AlignmentMethod::Affine);
        assert_eq!(parse_alignment_method(Some(" Affine ")), AlignmentMethod::Affine);
        assert_eq!(parse_alignment_method(Some("phase_correlation")), AlignmentMethod::PhaseCorrelation);
        assert_eq!(parse_alignment_method(None), AlignmentMethod::PhaseCorrelation);
    }

    #[test]
    fn each_drizzle_rgb_run_gets_its_own_output_names() {
        let (first_png, first_fits) = drizzle_rgb_output_paths("C:/out", 1, true);
        let (second_png, second_fits) = drizzle_rgb_output_paths("C:/out", 2, true);
        assert_ne!(first_png, second_png);
        assert_ne!(first_fits, second_fits);
        for path in [&first_png, &second_png, first_fits.as_ref().unwrap(), second_fits.as_ref().unwrap()] {
            let name = path.strip_prefix("C:/out/").expect("inside the output dir");
            assert!(name.starts_with("drizzle_rgb_") && !name.contains('/'), "{path}");
        }
        assert!(first_png.ends_with(".png") && first_fits.as_deref().is_some_and(|f| f.ends_with(".fits")));
        assert_eq!(drizzle_rgb_output_paths("C:/out", 1, false).1, None);
    }

    fn write_channel(dir: &tempfile::TempDir, prefix: &str, count: usize) -> Vec<String> {
        (0..count)
            .map(|i| {
                let path = dir.path().join(format!("{prefix}{i}.fits")).to_str().unwrap().to_string();
                let frame = ndarray::Array2::from_shape_fn((96, 96), |(y, x)| {
                    let d2 = (y as f32 - 40.0).powi(2) + (x as f32 - 52.0).powi(2);
                    100.0 + i as f32 + 1000.0 * (-d2 / 18.0).exp()
                });
                crate::infra::fits::writer::write_fits_mono(&path, &frame, None).unwrap();
                path
            })
            .collect()
    }

    #[test]
    fn drizzle_rgb_tick_total_covers_every_tick() {
        use crate::core::stacking::never_cancelled;
        use crate::types::compose::{AlignMethod, WhiteBalance};
        use std::sync::atomic::{AtomicU64, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let (r, g) = (write_channel(&dir, "r", 2), write_channel(&dir, "g", 2));
        let png = dir.path().join("rgb.png").to_str().unwrap().to_string();
        for align in [true, false] {
            let config = DrizzleRgbConfig {
                drizzle: DrizzleConfig { align, scale: 1.0, ..DrizzleConfig::default() },
                white_balance: WhiteBalance::None,
                auto_stretch: false,
                align: false,
                align_method: AlignMethod::PhaseCorrelation,
                scnr: None,
                ..DrizzleRgbConfig::default()
            };
            let ticks = AtomicU64::new(0);
            let count = |_: &str| {
                ticks.fetch_add(1, Ordering::Relaxed);
            };
            let result = drizzle_rgb_with(Some(&r), Some(&g), None, &png, None, &config, &never_cancelled, &count, None).unwrap();
            assert_eq!((result.frame_count_r, result.frame_count_g), (2, 2), "align {align}");
            assert_eq!(drizzle_rgb_tick_total(&[2, 2, 0], align), ticks.load(Ordering::Relaxed) + 1, "align {align}");
        }
    }

    #[test]
    fn response_reports_the_applied_scale_and_true_input_dims() {
        let stats = || ChannelStats { min: 0.0, max: 1.0, median: 0.5, mean: 0.5 };
        let result = DrizzleRgbResult {
            png_path: "out.png".into(),
            fits_path: None,
            input_dims: (100, 150),
            output_dims: (400, 600),
            scale: 4.0,
            frame_count_r: 2,
            frame_count_g: 2,
            frame_count_b: 0,
            rejected_pixels: 7,
            stf_r: StfParams::default(),
            stf_g: StfParams::default(),
            stf_b: StfParams::default(),
            stats_r: stats(),
            stats_g: stats(),
            stats_b: stats(),
            scnr_applied: false,
            warnings: vec!["R: Frame 3 of 3 was left out of the drizzle".into()],
        };
        let json = drizzle_rgb_json(&result, 5);
        assert_eq!(json[RES_SCALE], 4.0);
        assert_eq!(json[RES_INPUT_DIMS], serde_json::json!([150, 100]));
        assert_eq!(json[RES_OUTPUT_DIMS], serde_json::json!([600, 400]));
        assert_eq!(json[RES_WARNINGS], serde_json::json!(["R: Frame 3 of 3 was left out of the drizzle"]));
    }
}
