use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, resolve_output_dir};
use crate::cmd::helpers;
use crate::core::compose::drizzle_rgb::drizzle_rgb;
use crate::infra::progress::ProgressHandle;
use crate::types::compose::{DrizzleRgbConfig, DrizzleRgbResult};
use crate::types::constants::{
    DEFAULT_DRIZZLE_PIXFRAC, DEFAULT_DRIZZLE_SCALE, DEFAULT_DRIZZLE_SIGMA,
    DEFAULT_DRIZZLE_SIGMA_ITERS,
    EVENT_DRIZZLE_RGB_PROGRESS,
    FILE_DRIZZLE_RGB_FITS, FILE_DRIZZLE_RGB_PNG,
    STAGE_SAVE,
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH,
    RES_FRAME_COUNT_B, RES_FRAME_COUNT_G, RES_FRAME_COUNT_R,
    RES_INPUT_DIMS, RES_OUTPUT_DIMS,
    RES_PNG_PATH, RES_REJECTED_PIXELS, RES_SCALE, RES_WARNINGS,
};
use crate::types::stacking::{AlignmentMethod, DrizzleConfig};

fn parse_alignment_method(name: Option<&str>) -> AlignmentMethod {
    match name.map(|n| n.trim().to_ascii_lowercase()).as_deref() {
        Some("affine") | Some("zncc") => AlignmentMethod::Affine,
        _ => AlignmentMethod::PhaseCorrelation,
    }
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
    let total_frames = r_paths.as_ref().map_or(0, |v| v.len())
        + g_paths.as_ref().map_or(0, |v| v.len())
        + b_paths.as_ref().map_or(0, |v| v.len());
    let progress_clone = ProgressHandle::new(&app, EVENT_DRIZZLE_RGB_PROGRESS, total_frames as u64 + 2)
        .clone();
    let scale_val = scale.unwrap_or(DEFAULT_DRIZZLE_SCALE);

    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let k = helpers::parse_drizzle_kernel(kernel.as_deref());
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

        let png_path = format!("{}/{}", output_dir, FILE_DRIZZLE_RGB_PNG);
        let fits_out = if save_fits.unwrap_or(false) {
            Some(format!("{}/{}", output_dir, FILE_DRIZZLE_RGB_FITS))
        } else {
            None
        };

        let result = drizzle_rgb(
            r_paths.as_deref(),
            g_paths.as_deref(),
            b_paths.as_deref(),
            &png_path,
            fits_out.as_deref(),
            &config,
        )?;

        progress_clone.tick_with_stage(STAGE_SAVE);
        progress_clone.emit_complete();

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
