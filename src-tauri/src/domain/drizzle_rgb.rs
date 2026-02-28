use anyhow::{bail, Result};
use image::{Rgb, RgbImage};
use ndarray::Array2;

use crate::domain::drizzle::{self, DrizzleConfig, DrizzleResult};
use crate::domain::fits_writer::{self, FitsWriteConfig};
use crate::domain::rgb_compose::{ChannelStats, WhiteBalance};
use crate::domain::scnr::{self, ScnrConfig};
use crate::domain::stats;
use crate::domain::stf::{self, AutoStfConfig, StfParams};

#[derive(Debug, Clone)]
pub struct DrizzleRgbConfig {
    pub drizzle: DrizzleConfig,
    pub white_balance: WhiteBalance,
    pub auto_stretch: bool,
    pub linked_stf: bool,
    pub scnr: Option<ScnrConfig>,
}

impl Default for DrizzleRgbConfig {
    fn default() -> Self {
        Self {
            drizzle: DrizzleConfig::default(),
            white_balance: WhiteBalance::Auto,
            auto_stretch: true,
            linked_stf: false,
            scnr: None,
        }
    }
}

#[derive(Debug)]
pub struct DrizzleRgbResult {
    pub png_path: String,
    pub fits_path: Option<String>,
    pub input_dims: (usize, usize),
    pub output_dims: (usize, usize),
    pub scale: f64,
    pub frame_count_r: usize,
    pub frame_count_g: usize,
    pub frame_count_b: usize,
    pub rejected_pixels: u64,
    pub stf_r: StfParams,
    pub stf_g: StfParams,
    pub stf_b: StfParams,
    pub stats_r: ChannelStats,
    pub stats_g: ChannelStats,
    pub stats_b: ChannelStats,
    pub scnr_applied: bool,
}

fn drizzle_channel(paths: &[String], config: &DrizzleConfig) -> Result<DrizzleResult> {
    drizzle::drizzle_from_paths(paths, config, None)
}

fn compute_channel_stats(arr: &Array2<f32>) -> ChannelStats {
    let st = stats::compute_image_stats(arr);
    if st.valid_count == 0 {
        return ChannelStats {
            min: 0.0,
            max: 0.0,
            median: 0.0,
            mean: 0.0,
        };
    }
    ChannelStats {
        min: st.min,
        max: st.max,
        median: st.median,
        mean: st.mean,
    }
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

    let r_result = r_paths
        .filter(|p| p.len() >= 2)
        .map(|p| drizzle_channel(p, &config.drizzle))
        .transpose()?;
    let g_result = g_paths
        .filter(|p| p.len() >= 2)
        .map(|p| drizzle_channel(p, &config.drizzle))
        .transpose()?;
    let b_result = b_paths
        .filter(|p| p.len() >= 2)
        .map(|p| drizzle_channel(p, &config.drizzle))
        .transpose()?;

    if r_result.is_none() && g_result.is_none() && b_result.is_none() {
        bail!("All channels failed or have fewer than 2 frames");
    }

    let ref_result = r_result
        .as_ref()
        .or(g_result.as_ref())
        .or(b_result.as_ref())
        .unwrap();
    let (out_rows, out_cols) = ref_result.output_dims;
    let input_dims = ref_result.input_dims;
    let output_dims = ref_result.output_dims;
    let scale = ref_result.output_scale;

    let zeros = Array2::<f32>::zeros((out_rows, out_cols));
    let r_img = r_result.as_ref().map(|r| &r.image).unwrap_or(&zeros);
    let g_img = g_result.as_ref().map(|r| &r.image).unwrap_or(&zeros);
    let b_img = b_result.as_ref().map(|r| &r.image).unwrap_or(&zeros);

    let frame_count_r = r_result.as_ref().map(|r| r.frame_count).unwrap_or(0);
    let frame_count_g = g_result.as_ref().map(|r| r.frame_count).unwrap_or(0);
    let frame_count_b = b_result.as_ref().map(|r| r.frame_count).unwrap_or(0);

    let rejected_pixels = r_result
        .as_ref()
        .map(|r| r.rejected_pixels)
        .unwrap_or(0)
        + g_result
        .as_ref()
        .map(|r| r.rejected_pixels)
        .unwrap_or(0)
        + b_result
        .as_ref()
        .map(|r| r.rejected_pixels)
        .unwrap_or(0);

    let stats_r_raw = compute_channel_stats(r_img);
    let stats_g_raw = compute_channel_stats(g_img);
    let stats_b_raw = compute_channel_stats(b_img);

    let (wb_r, wb_g, wb_b) = match &config.white_balance {
        WhiteBalance::Auto => {
            let ref_med = stats_g_raw.median.max(1e-10);
            (
                (ref_med / stats_r_raw.median.max(1e-10)) as f32,
                1.0f32,
                (ref_med / stats_b_raw.median.max(1e-10)) as f32,
            )
        }
        WhiteBalance::Manual(r, g, b) => (*r as f32, *g as f32, *b as f32),
        WhiteBalance::None => (1.0, 1.0, 1.0),
    };

    let r_wb = r_img.mapv(|v| v * wb_r);
    let g_wb = g_img.mapv(|v| v * wb_g);
    let b_wb = b_img.mapv(|v| v * wb_b);

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

    let r_stretched = stf::apply_stf_f32(&r_wb, &stf_r, &st_r);
    let mut g_stretched = stf::apply_stf_f32(&g_wb, &stf_g, &st_g);
    let b_stretched = stf::apply_stf_f32(&b_wb, &stf_b, &st_b);

    let scnr_applied = if let Some(ref scnr_cfg) = config.scnr {
        scnr::apply_scnr_inplace(&r_stretched, &mut g_stretched, &b_stretched, scnr_cfg);
        true
    } else {
        false
    };

    let mut img = RgbImage::new(out_cols as u32, out_rows as u32);
    for y in 0..out_rows {
        for x in 0..out_cols {
            let r = (r_stretched[[y, x]].clamp(0.0, 1.0) * 255.0) as u8;
            let g = (g_stretched[[y, x]].clamp(0.0, 1.0) * 255.0) as u8;
            let b = (b_stretched[[y, x]].clamp(0.0, 1.0) * 255.0) as u8;
            img.put_pixel(x as u32, y as u32, Rgb([r, g, b]));
        }
    }
    img.save(output_png)
        .map_err(|e| anyhow::anyhow!("Failed to save RGB PNG: {}", e))?;

    let fits_path = if let Some(fits_out) = output_fits {
        let fconfig = FitsWriteConfig {
            software: Some("AstroKit".into()),
            ..Default::default()
        };
        let no_header: Option<&crate::model::HduHeader> = None;
        let written = fits_writer::write_fits_rgb(
            &r_wb,
            &g_wb,
            &b_wb,
            fits_out,
            no_header.as_ref(),
            &fconfig,
        )?;
        Some(written)
    } else {
        None
    };

    Ok(DrizzleRgbResult {
        png_path: output_png.to_string(),
        fits_path,
        input_dims,
        output_dims,
        scale,
        frame_count_r,
        frame_count_g,
        frame_count_b,
        rejected_pixels,
        stf_r,
        stf_g,
        stf_b,
        stats_r: stats_r_raw,
        stats_g: stats_g_raw,
        stats_b: stats_b_raw,
        scnr_applied,
    })
}