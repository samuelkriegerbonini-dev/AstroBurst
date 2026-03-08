use anyhow::{Context, Result};
use ndarray::Array2;
pub use crate::infra::render::rgb::render_rgb;
pub use crate::core::compose::rgb::process_rgb;
pub use crate::types::compose::{
    RgbComposeConfig, RgbComposeResult,
};

pub fn compose_rgb(
    r_channel: Option<&Array2<f32>>,
    g_channel: Option<&Array2<f32>>,
    b_channel: Option<&Array2<f32>>,
    output_path: &str,
    config: &RgbComposeConfig,
) -> Result<RgbComposeResult> {
    let processed = process_rgb(r_channel, g_channel, b_channel, config)?;

    render_rgb(&processed.r, &processed.g, &processed.b, output_path)
        .with_context(|| format!("Failed to save RGB image to {}", output_path))?;

    Ok(RgbComposeResult {
        png_path: output_path.to_string(),
        stf_r: processed.stf_r,
        stf_g: processed.stf_g,
        stf_b: processed.stf_b,
        stats_r: processed.stats_r,
        stats_g: processed.stats_g,
        stats_b: processed.stats_b,
        offset_g: processed.offset_g,
        offset_b: processed.offset_b,
        width: processed.cols,
        height: processed.rows,
        scnr_applied: processed.scnr_applied,
        dimension_crop: processed.dimension_crop,
    })
}
