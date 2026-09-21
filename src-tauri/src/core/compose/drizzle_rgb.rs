use ndarray::{Array2, Zip, s};

use crate::core::alignment::pair::align_pair_with_label;
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

const CHANNEL_LABELS: [&str; 3] = ["R", "G", "B"];

fn mean_of(a: &Array2<f32>, b: &Array2<f32>) -> Array2<f32> {
    let mut out = Array2::<f32>::zeros(a.raw_dim());
    Zip::from(&mut out)
        .and(a)
        .and(b)
        .par_for_each(|o, &av, &bv| *o = (av + bv) * 0.5);
    out
}

pub fn process_drizzle_rgb(
    channels: &DrizzleRgbChannels,
    config: &DrizzleRgbConfig,
) -> Result<ProcessedDrizzleRgb> {
    let dims: Vec<(usize, usize)> = [&channels.r, &channels.g, &channels.b]
        .iter()
        .filter_map(|r| r.as_ref().map(|img| img.dim()))
        .collect();

    if dims.len() < 2 {
        bail!(
            "RGB drizzle needs at least 2 stacked channels, got {} (R: {} frame(s), G: {} frame(s), B: {} frame(s)); a channel with fewer than 2 frames is dropped before this point",
            dims.len(),
            channels.frame_count_r,
            channels.frame_count_g,
            channels.frame_count_b
        );
    }

    let out_rows = dims.iter().map(|d| d.0).min().unwrap_or(0);
    let out_cols = dims.iter().map(|d| d.1).min().unwrap_or(0);

    if out_rows == 0 || out_cols == 0 {
        bail!("RGB drizzle received an empty channel image ({}x{})", out_rows, out_cols);
    }

    let crop = |img: &Array2<f32>| -> Array2<f32> {
        let (r, c) = img.dim();
        if r == out_rows && c == out_cols {
            img.clone()
        } else {
            img.slice(s![..out_rows, ..out_cols]).to_owned()
        }
    };

    let mut planes: [Option<Array2<f32>>; 3] = [
        channels.r.as_ref().map(|img| crop(img)),
        channels.g.as_ref().map(|img| crop(img)),
        channels.b.as_ref().map(|img| crop(img)),
    ];

    if config.align {
        let reference_idx = planes.iter().position(|p| p.is_some()).unwrap_or(0);
        let reference = planes[reference_idx].clone();
        if let Some(reference) = reference {
            for idx in (reference_idx + 1)..planes.len() {
                if let Some(img) = planes[idx].as_mut() {
                    let label = CHANNEL_LABELS[idx];
                    match align_pair_with_label(&reference, img, config.align_method, out_rows, out_cols, label) {
                        Ok(res) => *img = res.aligned,
                        Err(e) => log::warn!("Drizzle RGB: channel '{}' registration failed: {}", label, e),
                    }
                }
            }
        }
    }

    let [r_plane, g_plane, b_plane] = planes;
    let (r_img, g_img, b_img) = match (r_plane, g_plane, b_plane) {
        (Some(r), Some(g), Some(b)) => (r, g, b),
        (None, Some(g), Some(b)) => {
            log::warn!("Drizzle RGB: no R channel, synthesising it as the mean of G and B");
            (mean_of(&g, &b), g, b)
        }
        (Some(r), None, Some(b)) => {
            log::warn!("Drizzle RGB: no G channel, synthesising it as the mean of R and B");
            let g = mean_of(&r, &b);
            (r, g, b)
        }
        (Some(r), Some(g), None) => {
            log::warn!("Drizzle RGB: no B channel, synthesising it as the mean of R and G");
            let b = mean_of(&r, &g);
            (r, g, b)
        }
        _ => bail!("RGB drizzle needs at least 2 stacked channels"),
    };

    let sr_full = stats::compute_image_stats(&r_img);
    let sg_full = stats::compute_image_stats(&g_img);
    let sb_full = stats::compute_image_stats(&b_img);

    let stats_r_raw = ChannelStats::from(&sr_full);
    let stats_g_raw = ChannelStats::from(&sg_full);
    let stats_b_raw = ChannelStats::from(&sb_full);

    let (wb_r, wb_g, wb_b) = match &config.white_balance {
        WhiteBalance::Auto => white_balance::select_wb_reference(&sr_full, &sg_full, &sb_full)?,
        WhiteBalance::Manual(r, g, b) => (
            white_balance::validate_wb_factor(white_balance::WbChannel::R, *r)?,
            white_balance::validate_wb_factor(white_balance::WbChannel::G, *g)?,
            white_balance::validate_wb_factor(white_balance::WbChannel::B, *b)?,
        ),
        WhiteBalance::None => (1.0, 1.0, 1.0),
    };

    let r_wb = r_img.mapv(|v| v * wb_r as f32);
    let g_wb = g_img.mapv(|v| v * wb_g as f32);
    let b_wb = b_img.mapv(|v| v * wb_b as f32);

    let stf_cfg = AutoStfConfig::default();

    let (stf_r, stf_g, stf_b, st_r, st_g, st_b) = if config.auto_stretch {
        if config.linked_stf {
            let sr = stats::compute_image_stats(&r_wb);
            let sg = stats::compute_image_stats(&g_wb);
            let sb = stats::compute_image_stats(&b_wb);
            let combined = stats::combine_channel_stats(&sr, &sg, &sb);
            let params = stf::auto_stf(&combined, &stf_cfg);
            (params, params, params, combined.clone(), combined.clone(), combined)
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

    Ok(ProcessedDrizzleRgb {
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
    })
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

    for (name, paths) in [("R", r_paths), ("G", g_paths), ("B", b_paths)] {
        if let Some(p) = paths {
            if p.len() < 2 {
                bail!(
                    "Channel {} has {} frame(s); drizzle needs at least 2 per channel",
                    name,
                    p.len()
                );
            }
        }
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

    let processed = process_drizzle_rgb(&channels, config)?;
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
                row_buf[o] = (r_slice[i].clamp(0.0, 1.0) * 255.0).round() as u8;
                row_buf[o + 1] = (g_slice[i].clamp(0.0, 1.0) * 255.0).round() as u8;
                row_buf[o + 2] = (b_slice[i].clamp(0.0, 1.0) * 255.0).round() as u8;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::compose::AlignMethod;
    use ndarray::arr2;

    fn config_without_stretch() -> DrizzleRgbConfig {
        DrizzleRgbConfig {
            white_balance: WhiteBalance::None,
            auto_stretch: false,
            align: false,
            align_method: AlignMethod::PhaseCorrelation,
            scnr: None,
            ..DrizzleRgbConfig::default()
        }
    }

    fn channels(
        r: Option<Array2<f32>>,
        g: Option<Array2<f32>>,
        b: Option<Array2<f32>>,
    ) -> DrizzleRgbChannels {
        let frames = |c: &Option<Array2<f32>>| if c.is_some() { 2 } else { 0 };
        let (frame_count_r, frame_count_g, frame_count_b) = (frames(&r), frames(&g), frames(&b));
        DrizzleRgbChannels {
            r,
            g,
            b,
            frame_count_r,
            frame_count_g,
            frame_count_b,
            rejected_pixels: 0,
            input_dims: (2, 2),
            scale: 2.0,
        }
    }

    #[test]
    fn missing_channel_is_synthesised_instead_of_zero_filled() {
        let g = arr2(&[[2.0f32, 2.0], [2.0, 2.0]]);
        let b = arr2(&[[4.0f32, 4.0], [4.0, 4.0]]);

        let processed =
            process_drizzle_rgb(&channels(None, Some(g), Some(b)), &config_without_stretch()).unwrap();

        assert_eq!(processed.r_wb, arr2(&[[3.0f32, 3.0], [3.0, 3.0]]));
        assert!(
            processed.r_wb.iter().all(|v| *v > 0.0),
            "the absent R channel must not come back as a black plane"
        );
    }

    #[test]
    fn a_manual_white_balance_factor_is_bounded_like_the_automatic_one() {
        let plane = arr2(&[[2.0f32, 2.0], [2.0, 2.0]]);
        let with_wb = |wb: WhiteBalance| DrizzleRgbConfig {
            white_balance: wb,
            ..config_without_stretch()
        };

        let err = match process_drizzle_rgb(
            &channels(Some(plane.clone()), Some(plane.clone()), Some(plane.clone())),
            &with_wb(WhiteBalance::Manual(1e10, 1.0, 1.0)),
        ) {
            Ok(_) => panic!("a manual gain of 1e10 must not reach the pixels"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("channel R"), "{}", err);
        assert!(err.contains("outside the usable range"), "{}", err);

        assert!(process_drizzle_rgb(
            &channels(Some(plane.clone()), Some(plane.clone()), Some(plane.clone())),
            &with_wb(WhiteBalance::Manual(1.0, f64::NAN, 1.0)),
        )
        .is_err());

        let ok = process_drizzle_rgb(
            &channels(Some(plane.clone()), Some(plane.clone()), Some(plane)),
            &with_wb(WhiteBalance::Manual(2.0, 1.0, 1.0)),
        )
        .unwrap();
        assert_eq!(ok.r_wb, arr2(&[[4.0f32, 4.0], [4.0, 4.0]]));
    }

    #[test]
    fn a_single_channel_is_rejected_instead_of_yielding_two_black_planes() {
        let g = arr2(&[[2.0f32, 2.0], [2.0, 2.0]]);

        let err = match process_drizzle_rgb(&channels(None, Some(g), None), &config_without_stretch()) {
            Ok(_) => panic!("a lone channel must be rejected, not padded with black planes"),
            Err(e) => e.to_string(),
        };

        assert!(err.contains("at least 2 stacked channels"), "{}", err);
        assert!(err.contains("frame(s)"), "{}", err);
    }
}
