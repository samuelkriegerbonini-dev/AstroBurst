use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_fits_array, resolve_output_dir};
use crate::domain::rgb_compose::compose_rgb;
use crate::types::compose::{RgbComposeConfig, WhiteBalance};
use crate::types::constants::{
    DEFAULT_DIMENSION_TOLERANCE, DEFAULT_RGB_COMPOSITE_FILENAME, DEFAULT_SCNR_AMOUNT,
    DEFAULT_WB_VALUE, SCNR_METHOD_MAXIMUM, WB_MODE_MANUAL, WB_MODE_NONE,
    RES_DIMENSIONS, RES_DIMENSION_CROP, RES_ELAPSED_MS, RES_MAX, RES_MEAN, RES_MEDIAN,
    RES_MIN, RES_OFFSET_B, RES_OFFSET_G, RES_PNG_PATH, RES_SCNR_APPLIED,
    RES_STATS_B, RES_STATS_G, RES_STATS_R,
};
use crate::types::image::{ScnrConfig, ScnrMethod};

#[tauri::command]
pub async fn compose_rgb_cmd(
    r_path: Option<String>,
    g_path: Option<String>,
    b_path: Option<String>,
    output_dir: String,
    auto_stretch: Option<bool>,
    linked_stf: Option<bool>,
    align: Option<bool>,
    wb_mode: Option<String>,
    wb_r: Option<f64>,
    wb_g: Option<f64>,
    wb_b: Option<f64>,
    scnr_enabled: Option<bool>,
    scnr_method: Option<String>,
    scnr_amount: Option<f64>,
    dimension_tolerance: Option<usize>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let r_arr = r_path.as_ref().map(|p| load_fits_array(p)).transpose()?;
        let g_arr = g_path.as_ref().map(|p| load_fits_array(p)).transpose()?;
        let b_arr = b_path.as_ref().map(|p| load_fits_array(p)).transpose()?;

        let wb = match wb_mode.as_deref() {
            Some(WB_MODE_MANUAL) => WhiteBalance::Manual(
                wb_r.unwrap_or(DEFAULT_WB_VALUE),
                wb_g.unwrap_or(DEFAULT_WB_VALUE),
                wb_b.unwrap_or(DEFAULT_WB_VALUE),
            ),
            Some(WB_MODE_NONE) => WhiteBalance::None,
            _ => WhiteBalance::Auto,
        };

        let scnr_cfg = if scnr_enabled.unwrap_or(false) {
            let method = match scnr_method.as_deref() {
                Some(SCNR_METHOD_MAXIMUM) => ScnrMethod::MaximumNeutral,
                _ => ScnrMethod::AverageNeutral,
            };
            Some(ScnrConfig {
                method,
                amount: scnr_amount.unwrap_or(DEFAULT_SCNR_AMOUNT as f64) as f32,
                preserve_luminance: false,
            })
        } else {
            None
        };

        let config = RgbComposeConfig {
            white_balance: wb,
            auto_stretch: auto_stretch.unwrap_or(true),
            linked_stf: linked_stf.unwrap_or(false),
            align: align.unwrap_or(true),
            scnr: scnr_cfg,
            dimension_tolerance: dimension_tolerance.unwrap_or(DEFAULT_DIMENSION_TOLERANCE),
            ..RgbComposeConfig::default()
        };

        let png_path = format!("{}/{}", output_dir, DEFAULT_RGB_COMPOSITE_FILENAME);

        let result = compose_rgb(
            r_arr.as_ref(),
            g_arr.as_ref(),
            b_arr.as_ref(),
            &png_path,
            &config,
        )?;

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_PNG_PATH: result.png_path,
            RES_DIMENSIONS: [result.width, result.height],
            RES_SCNR_APPLIED: result.scnr_applied,
            RES_OFFSET_G: [result.offset_g.0, result.offset_g.1],
            RES_OFFSET_B: [result.offset_b.0, result.offset_b.1],
            RES_DIMENSION_CROP: result.dimension_crop,
            RES_STATS_R: { RES_MEDIAN: result.stats_r.median, RES_MEAN: result.stats_r.mean, RES_MIN: result.stats_r.min, RES_MAX: result.stats_r.max },
            RES_STATS_G: { RES_MEDIAN: result.stats_g.median, RES_MEAN: result.stats_g.mean, RES_MIN: result.stats_g.min, RES_MAX: result.stats_g.max },
            RES_STATS_B: { RES_MEDIAN: result.stats_b.median, RES_MEAN: result.stats_b.mean, RES_MIN: result.stats_b.min, RES_MAX: result.stats_b.max },
            RES_ELAPSED_MS: elapsed,
        }))
    })
}
