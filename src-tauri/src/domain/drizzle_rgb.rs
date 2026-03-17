use anyhow::{bail, Result};
use image::RgbImage;
use rayon;
use rayon::prelude::*;

use crate::domain::drizzle::{self, DrizzleConfig, DrizzleResult};
use crate::infra::fits::writer as fits_writer;

pub use crate::core::compose::drizzle_rgb::{
    DrizzleRgbChannels, process_drizzle_rgb,
};
pub use crate::types::compose::{
    DrizzleRgbConfig, DrizzleRgbResult,
};

fn drizzle_channel(paths: &[String], config: &DrizzleConfig) -> Result<DrizzleResult> {
    drizzle::drizzle_from_paths(paths, config, None)
}

pub fn drizzle_rgb(
    r_paths: Option<&[String]>,
    g_paths: Option<&[String]>,
    b_paths: Option<&[String]>,
    output_png: &str,
    output_fits: Option<&str>,
    config: &DrizzleRgbConfig,
) -> Result<DrizzleRgbResult> {
    let channel_count = [r_paths.is_some(), g_paths.is_some(), b_paths.is_some()]
        .iter()
        .filter(|&&b| b)
        .count();
    if channel_count < 2 {
        bail!(
            "Need at least 2 channels for RGB drizzle (got {})",
            channel_count
        );
    }

    let (r_result, (g_result, b_result)) = rayon::join(
        || {
            r_paths
                .filter(|p| p.len() >= 2)
                .map(|p| drizzle_channel(p, &config.drizzle))
                .transpose()
        },
        || {
            rayon::join(
                || {
                    g_paths
                        .filter(|p| p.len() >= 2)
                        .map(|p| drizzle_channel(p, &config.drizzle))
                        .transpose()
                },
                || {
                    b_paths
                        .filter(|p| p.len() >= 2)
                        .map(|p| drizzle_channel(p, &config.drizzle))
                        .transpose()
                },
            )
        },
    );
    let r_result = r_result?;
    let g_result = g_result?;
    let b_result = b_result?;

    if r_result.is_none() && g_result.is_none() && b_result.is_none() {
        bail!("All channels failed or have fewer than 2 frames");
    }

    let ref_result = r_result
        .as_ref()
        .or(g_result.as_ref())
        .or(b_result.as_ref())
        .unwrap();
    let input_dims = ref_result.input_dims;
    let scale = ref_result.output_scale;

    let channels = DrizzleRgbChannels {
        r: r_result.as_ref().map(|r| r.image.clone()),
        g: g_result.as_ref().map(|r| r.image.clone()),
        b: b_result.as_ref().map(|r| r.image.clone()),
        frame_count_r: r_result.as_ref().map(|r| r.frame_count).unwrap_or(0),
        frame_count_g: g_result.as_ref().map(|r| r.frame_count).unwrap_or(0),
        frame_count_b: b_result.as_ref().map(|r| r.frame_count).unwrap_or(0),
        rejected_pixels: r_result.as_ref().map(|r| r.rejected_pixels).unwrap_or(0)
            + g_result.as_ref().map(|r| r.rejected_pixels).unwrap_or(0)
            + b_result.as_ref().map(|r| r.rejected_pixels).unwrap_or(0),
        input_dims,
        scale,
    };

    let processed = process_drizzle_rgb(&channels, config);
    let (out_rows, out_cols) = processed.output_dims;

    let mut pixels = vec![0u8; out_rows * out_cols * 3];
    pixels
        .par_chunks_mut(out_cols * 3)
        .enumerate()
        .for_each(|(y, row_buf)| {
            let r_slice = processed.r_stretched.as_slice().unwrap();
            let g_slice = processed.g_stretched.as_slice().unwrap();
            let b_slice = processed.b_stretched.as_slice().unwrap();
            let base = y * out_cols;
            for x in 0..out_cols {
                let i = base + x;
                let o = x * 3;
                row_buf[o] = (r_slice[i].clamp(0.0, 1.0) * 255.0) as u8;
                row_buf[o + 1] = (g_slice[i].clamp(0.0, 1.0) * 255.0) as u8;
                row_buf[o + 2] = (b_slice[i].clamp(0.0, 1.0) * 255.0) as u8;
            }
        });

    let img = RgbImage::from_raw(out_cols as u32, out_rows as u32, pixels)
        .ok_or_else(|| anyhow::anyhow!("Failed to create RGB image buffer"))?;
    img.save(output_png)
        .map_err(|e| anyhow::anyhow!("Failed to save RGB PNG: {}", e))?;

    let fits_path = if let Some(fits_out) = output_fits {
        fits_writer::write_fits_rgb(
            fits_out,
            &processed.r_wb,
            &processed.g_wb,
            &processed.b_wb,
            None,
        )?;
        Some(fits_out.to_string())
    } else {
        None
    };

    Ok(DrizzleRgbResult {
        png_path: output_png.to_string(),
        fits_path,
        input_dims,
        output_dims: processed.output_dims,
        scale,
        frame_count_r: channels.frame_count_r,
        frame_count_g: channels.frame_count_g,
        frame_count_b: channels.frame_count_b,
        rejected_pixels: channels.rejected_pixels,
        stf_r: processed.stf_r,
        stf_g: processed.stf_g,
        stf_b: processed.stf_b,
        stats_r: processed.stats_r,
        stats_g: processed.stats_g,
        stats_b: processed.stats_b,
        scnr_applied: processed.scnr_applied,
    })
}
