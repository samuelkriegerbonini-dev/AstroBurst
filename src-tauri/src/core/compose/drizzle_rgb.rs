use ndarray::{Array2, s};

use crate::core::compose::white_balance;
use crate::core::imaging::scnr;
use crate::core::imaging::stats;
use crate::types::image::{StfParams, AutoStfConfig};
use crate::core::imaging::stf;
pub use crate::types::compose::{
    WhiteBalance, ChannelStats, DrizzleRgbConfig, DrizzleRgbResult,
};

pub struct DrizzleRgbChannels {
    pub r: Option<Array2<f32>>,
    pub g: Option<Array2<f32>>,
    pub b: Option<Array2<f32>>,
    pub frame_count_r: usize,
    pub frame_count_g: usize,
    pub frame_count_b: usize,
    pub rejected_pixels: u64,
    pub input_dims: (usize, usize),
    pub scale: f64,
}

pub struct ProcessedDrizzleRgb {
    pub r_stretched: Array2<f32>,
    pub g_stretched: Array2<f32>,
    pub b_stretched: Array2<f32>,
    pub r_wb: Array2<f32>,
    pub g_wb: Array2<f32>,
    pub b_wb: Array2<f32>,
    pub output_dims: (usize, usize),
    pub stf_r: StfParams,
    pub stf_g: StfParams,
    pub stf_b: StfParams,
    pub stats_r: ChannelStats,
    pub stats_g: ChannelStats,
    pub stats_b: ChannelStats,
    pub scnr_applied: bool,
}

pub fn process_drizzle_rgb(
    channels: &DrizzleRgbChannels,
    config: &DrizzleRgbConfig,
) -> ProcessedDrizzleRgb {
    let dims: Vec<(usize, usize)> = [&channels.r, &channels.g, &channels.b]
        .iter()
        .filter_map(|r| r.as_ref().map(|img| img.dim()))
        .collect();
    let min_rows = dims.iter().map(|d| d.0).min().unwrap_or(0);
    let min_cols = dims.iter().map(|d| d.1).min().unwrap_or(0);
    let out_rows = min_rows;
    let out_cols = min_cols;

    let crop = |img: &Array2<f32>| -> Array2<f32> {
        let (r, c) = img.dim();
        if r == out_rows && c == out_cols {
            img.clone()
        } else {
            img.slice(s![..out_rows, ..out_cols]).to_owned()
        }
    };

    let zeros = Array2::<f32>::zeros((out_rows, out_cols));
    let r_img = channels.r.as_ref().map(|r| crop(r)).unwrap_or_else(|| zeros.clone());
    let g_img = channels.g.as_ref().map(|r| crop(r)).unwrap_or_else(|| zeros.clone());
    let b_img = channels.b.as_ref().map(|r| crop(r)).unwrap_or_else(|| zeros.clone());

    let sr_full = stats::compute_image_stats(&r_img);
    let sg_full = stats::compute_image_stats(&g_img);
    let sb_full = stats::compute_image_stats(&b_img);

    let stats_r_raw = ChannelStats::from(&sr_full);
    let stats_g_raw = ChannelStats::from(&sg_full);
    let stats_b_raw = ChannelStats::from(&sb_full);

    let (wb_r, wb_g, wb_b) = match &config.white_balance {
        WhiteBalance::Auto => white_balance::select_wb_reference(&sr_full, &sg_full, &sb_full),
        WhiteBalance::Manual(r, g, b) => (*r, *g, *b),
        WhiteBalance::None => (1.0, 1.0, 1.0),
    };

    let r_wb = r_img.mapv(|v| v * wb_r as f32);
    let g_wb = g_img.mapv(|v| v * wb_g as f32);
    let b_wb = b_img.mapv(|v| v * wb_b as f32);

    let stf_cfg = AutoStfConfig::default();

    let (stf_r, stf_g, stf_b, st_r, st_g, st_b) = if config.auto_stretch {
        if config.linked_stf {
            let combined = (&r_wb + &g_wb + &b_wb) / 3.0;
            let (st, _) = stf::analyze(&combined);
            let params = stf::auto_stf(&st, &stf_cfg);
            let sr = stats::compute_image_stats(&r_wb);
            let sg = stats::compute_image_stats(&g_wb);
            let sb = stats::compute_image_stats(&b_wb);
            (params, params, params, sr, sg, sb)
        } else {
            let (sr, _) = stf::analyze(&r_wb);
            let (sg, _) = stf::analyze(&g_wb);
            let (sb, _) = stf::analyze(&b_wb);
            let pr = stf::auto_stf(&sr, &stf_cfg);
            let pg = stf::auto_stf(&sg, &stf_cfg);
            let pb = stf::auto_stf(&sb, &stf_cfg);
            (pr, pg, pb, sr, sg, sb)
        }
    } else {
        let sr = stats::compute_image_stats(&r_wb);
        let sg = stats::compute_image_stats(&g_wb);
        let sb = stats::compute_image_stats(&b_wb);
        let default_stf = StfParams {
            shadow: 0.0,
            midtone: 0.5,
            highlight: 1.0,
        };
        (default_stf, default_stf, default_stf, sr, sg, sb)
    };

    let mut r_stretched = stf::apply_stf_f32(&r_wb, &stf_r, &st_r);
    let mut g_stretched = stf::apply_stf_f32(&g_wb, &stf_g, &st_g);
    let mut b_stretched = stf::apply_stf_f32(&b_wb, &stf_b, &st_b);

    let scnr_applied = if let Some(ref scnr_cfg) = config.scnr {
        scnr::apply_scnr_inplace(&mut r_stretched, &mut g_stretched, &mut b_stretched, scnr_cfg);
        true
    } else {
        false
    };

    ProcessedDrizzleRgb {
        r_stretched,
        g_stretched,
        b_stretched,
        r_wb,
        g_wb,
        b_wb,
        output_dims: (out_rows, out_cols),
        stf_r,
        stf_g,
        stf_b,
        stats_r: stats_r_raw,
        stats_g: stats_g_raw,
        stats_b: stats_b_raw,
        scnr_applied,
    }
}

use anyhow::{bail, Result};
use image::RgbImage;
use rayon::prelude::*;

use crate::core::stacking::calibration::drizzle_from_paths;
use crate::infra::fits::writer as fits_writer;
use crate::types::stacking::{DrizzleConfig, DrizzleResult};

fn drizzle_channel(paths: &[String], config: &DrizzleConfig) -> Result<DrizzleResult> {
    drizzle_from_paths(paths, config, None)
}

pub fn drizzle_rgb(
    r_paths: Option<&[String]>,
    g_paths: Option<&[String]>,
    b_paths: Option<&[String]>,
    output_png: &str,
    output_fits: Option<&str>,
    config: &crate::types::compose::DrizzleRgbConfig,
) -> Result<crate::types::compose::DrizzleRgbResult> {
    let channel_count = [r_paths.is_some(), g_paths.is_some(), b_paths.is_some()]
        .iter()
        .filter(|&&b| b)
        .count();
    if channel_count < 2 {
        bail!("Need at least 2 channels for RGB drizzle (got {})", channel_count);
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

    Ok(crate::types::compose::DrizzleRgbResult {
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
