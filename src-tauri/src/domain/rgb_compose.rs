use anyhow::{bail, Context, Result};
use ndarray::Array2;
use image::{RgbImage, Rgb};

use crate::domain::scnr::{self, ScnrConfig};
use crate::domain::stats;
use crate::domain::stf::{self, AutoStfConfig, StfParams};

#[derive(Debug, Clone)]
pub struct RgbComposeConfig {
    pub white_balance: WhiteBalance,
    pub auto_stretch: bool,
    pub stf_r: Option<StfParams>,
    pub stf_g: Option<StfParams>,
    pub stf_b: Option<StfParams>,
    pub linked_stf: bool,
    pub align: bool,
    pub scnr: Option<ScnrConfig>,
}

impl Default for RgbComposeConfig {
    fn default() -> Self {
        Self {
            white_balance: WhiteBalance::Auto,
            auto_stretch: true,
            stf_r: None,
            stf_g: None,
            stf_b: None,
            linked_stf: false,
            align: true,
            scnr: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum WhiteBalance {
    Auto,
    Manual(f64, f64, f64),
    None,
}

#[derive(Debug, Clone)]
pub struct RgbComposeResult {
    pub png_path: String,
    pub stf_r: StfParams,
    pub stf_g: StfParams,
    pub stf_b: StfParams,
    pub stats_r: ChannelStats,
    pub stats_g: ChannelStats,
    pub stats_b: ChannelStats,
    pub offset_g: (i32, i32),
    pub offset_b: (i32, i32),
    pub width: usize,
    pub height: usize,
    pub scnr_applied: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChannelStats {
    pub min: f64,
    pub max: f64,
    pub median: f64,
    pub mean: f64,
}

pub fn compose_rgb(
    r_channel: Option<&Array2<f32>>,
    g_channel: Option<&Array2<f32>>,
    b_channel: Option<&Array2<f32>>,
    output_path: &str,
    config: &RgbComposeConfig,
) -> Result<RgbComposeResult> {
    let present = [r_channel.is_some(), g_channel.is_some(), b_channel.is_some()];
    let count = present.iter().filter(|&&b| b).count();
    if count < 2 {
        bail!("Need at least 2 channels for RGB compose (got {})", count);
    }

    let ref_channel = r_channel.or(g_channel).or(b_channel).unwrap();
    let (rows, cols) = ref_channel.dim();

    for (ch, name) in [(r_channel, "R"), (g_channel, "G"), (b_channel, "B")] {
        if let Some(arr) = ch {
            let (cr, cc) = arr.dim();
            if cr != rows || cc != cols {
                bail!(
                    "Channel {} has dimensions {}×{} but expected {}×{}",
                    name, cc, cr, cols, rows
                );
            }
        }
    }

    let (r_aligned, g_aligned, b_aligned, off_g, off_b) = if config.align && count >= 2 {
        align_channels(r_channel, g_channel, b_channel, rows, cols)?
    } else {
        let r = channel_or_synth(r_channel, g_channel, b_channel, rows, cols);
        let g = channel_or_synth(g_channel, r_channel, b_channel, rows, cols);
        let b = channel_or_synth(b_channel, r_channel, g_channel, rows, cols);
        (r, g, b, (0, 0), (0, 0))
    };

    let stats_r = channel_stats(&r_aligned);
    let stats_g = channel_stats(&g_aligned);
    let stats_b = channel_stats(&b_aligned);

    let (wb_r, wb_g, wb_b) = match &config.white_balance {
        WhiteBalance::Auto => {
            let ref_med = stats_g.median.max(1e-10);
            (
                ref_med / stats_r.median.max(1e-10),
                1.0,
                ref_med / stats_b.median.max(1e-10),
            )
        }
        WhiteBalance::Manual(r, g, b) => (*r, *g, *b),
        WhiteBalance::None => (1.0, 1.0, 1.0),
    };

    let r_wb = apply_multiplier(&r_aligned, wb_r as f32);
    let g_wb = apply_multiplier(&g_aligned, wb_g as f32);
    let b_wb = apply_multiplier(&b_aligned, wb_b as f32);

    let stf_config = AutoStfConfig::default();

    let (stf_r_params, stf_g_params, stf_b_params, stats_wb_r, stats_wb_g, stats_wb_b) =
        if config.auto_stretch {
            if config.linked_stf {
                let combined = merge_for_stf(&r_wb, &g_wb, &b_wb);
                let (st, _hist) = stf::analyze(&combined);
                let params = stf::auto_stf(&st, &stf_config);
                let sr = stats::compute_image_stats(&r_wb);
                let sg = stats::compute_image_stats(&g_wb);
                let sb = stats::compute_image_stats(&b_wb);
                (params, params, params, sr, sg, sb)
            } else {
                let (sr, _) = stf::analyze(&r_wb);
                let (sg, _) = stf::analyze(&g_wb);
                let (sb, _) = stf::analyze(&b_wb);
                let pr = stf::auto_stf(&sr, &stf_config);
                let pg = stf::auto_stf(&sg, &stf_config);
                let pb = stf::auto_stf(&sb, &stf_config);
                (pr, pg, pb, sr, sg, sb)
            }
        } else {
            let sr = stats::compute_image_stats(&r_wb);
            let sg = stats::compute_image_stats(&g_wb);
            let sb = stats::compute_image_stats(&b_wb);
            (
                config.stf_r.unwrap_or(StfParams { shadow: 0.0, midtone: 0.5, highlight: 1.0 }),
                config.stf_g.unwrap_or(StfParams { shadow: 0.0, midtone: 0.5, highlight: 1.0 }),
                config.stf_b.unwrap_or(StfParams { shadow: 0.0, midtone: 0.5, highlight: 1.0 }),
                sr,
                sg,
                sb,
            )
        };

    let r_stretched = stf::apply_stf_f32(&r_wb, &stf_r_params, &stats_wb_r);
    let mut g_stretched = stf::apply_stf_f32(&g_wb, &stf_g_params, &stats_wb_g);
    let b_stretched = stf::apply_stf_f32(&b_wb, &stf_b_params, &stats_wb_b);

    if let Some(ref scnr_cfg) = config.scnr {
        scnr::apply_scnr_inplace(&r_stretched, &mut g_stretched, &b_stretched, scnr_cfg);
    }

    let mut img = RgbImage::new(cols as u32, rows as u32);
    for y in 0..rows {
        for x in 0..cols {
            let r = (r_stretched[[y, x]].clamp(0.0, 1.0) * 255.0) as u8;
            let g = (g_stretched[[y, x]].clamp(0.0, 1.0) * 255.0) as u8;
            let b = (b_stretched[[y, x]].clamp(0.0, 1.0) * 255.0) as u8;
            img.put_pixel(x as u32, y as u32, Rgb([r, g, b]));
        }
    }

    img.save(output_path)
        .with_context(|| format!("Failed to save RGB image to {}", output_path))?;

    Ok(RgbComposeResult {
        png_path: output_path.to_string(),
        stf_r: stf_r_params,
        stf_g: stf_g_params,
        stf_b: stf_b_params,
        stats_r,
        stats_g,
        stats_b,
        offset_g: off_g,
        offset_b: off_b,
        width: cols,
        height: rows,
        scnr_applied: config.scnr.is_some(),
    })
}

fn channel_stats(arr: &Array2<f32>) -> ChannelStats {
    let st = stats::compute_image_stats(arr);
    if st.valid_count == 0 {
        return ChannelStats { min: 0.0, max: 0.0, median: 0.0, mean: 0.0 };
    }
    ChannelStats {
        min: st.min,
        max: st.max,
        median: st.median,
        mean: st.mean,
    }
}

fn apply_multiplier(arr: &Array2<f32>, mult: f32) -> Array2<f32> {
    arr.mapv(|v| v * mult)
}

fn channel_or_synth(
    primary: Option<&Array2<f32>>,
    alt1: Option<&Array2<f32>>,
    alt2: Option<&Array2<f32>>,
    rows: usize,
    cols: usize,
) -> Array2<f32> {
    if let Some(ch) = primary {
        return ch.clone();
    }

    let a = alt1.map(|a| a.clone()).unwrap_or_else(|| Array2::zeros((rows, cols)));
    let b = alt2.map(|b| b.clone()).unwrap_or_else(|| Array2::zeros((rows, cols)));

    (&a + &b) / 2.0
}

fn merge_for_stf(r: &Array2<f32>, g: &Array2<f32>, b: &Array2<f32>) -> Array2<f32> {
    (r + g + b) / 3.0
}

fn align_channels(
    r: Option<&Array2<f32>>,
    g: Option<&Array2<f32>>,
    b: Option<&Array2<f32>>,
    rows: usize,
    cols: usize,
) -> Result<(Array2<f32>, Array2<f32>, Array2<f32>, (i32, i32), (i32, i32))> {
    let ref_ch = r.or(g).or(b).unwrap();

    let r_img = channel_or_synth(r, g, b, rows, cols);
    let g_img = channel_or_synth(g, r, b, rows, cols);
    let b_img = channel_or_synth(b, r, g, rows, cols);

    let off_g = if g.is_some() {
        find_offset_pyramid(ref_ch, &g_img)
    } else {
        (0, 0)
    };

    let off_b = if b.is_some() {
        find_offset_pyramid(ref_ch, &b_img)
    } else {
        (0, 0)
    };

    let g_shifted = shift_image(&g_img, off_g.0, off_g.1);
    let b_shifted = shift_image(&b_img, off_b.0, off_b.1);

    Ok((r_img, g_shifted, b_shifted, off_g, off_b))
}

fn downsample_2x(img: &Array2<f32>) -> Array2<f32> {
    let (rows, cols) = img.dim();
    let nr = rows / 2;
    let nc = cols / 2;
    Array2::from_shape_fn((nr, nc), |(r, c)| {
        let r2 = r * 2;
        let c2 = c * 2;
        let a = img[[r2, c2]];
        let b = img[[r2, c2 + 1]];
        let c_val = img[[r2 + 1, c2]];
        let d = img[[r2 + 1, c2 + 1]];
        (a + b + c_val + d) * 0.25
    })
}

fn find_offset_pyramid(reference: &Array2<f32>, target: &Array2<f32>) -> (i32, i32) {
    let ref_2x = downsample_2x(reference);
    let tgt_2x = downsample_2x(target);
    let ref_4x = downsample_2x(&ref_2x);
    let tgt_4x = downsample_2x(&tgt_2x);

    let coarse = find_offset_parallel(&ref_4x, &tgt_4x, 64, 0, 0);
    let mid = find_offset_parallel(&ref_2x, &tgt_2x, 4, coarse.0 * 2, coarse.1 * 2);
    find_offset_parallel(reference, target, 2, mid.0 * 2, mid.1 * 2)
}

fn find_offset_parallel(
    reference: &Array2<f32>,
    target: &Array2<f32>,
    max_shift: i32,
    center_dy: i32,
    center_dx: i32,
) -> (i32, i32) {
    use rayon::prelude::*;

    let (rows, cols) = reference.dim();
    let cy = rows / 2;
    let cx = cols / 2;
    let region = (rows.min(cols) / 4).max(1);

    let y_start = cy.saturating_sub(region);
    let y_end = (cy + region).min(rows);
    let x_start = cx.saturating_sub(region);
    let x_end = (cx + region).min(cols);

    let shifts: Vec<(i32, i32)> = (-max_shift..=max_shift)
        .flat_map(|dy| (-max_shift..=max_shift).map(move |dx| (center_dy + dy, center_dx + dx)))
        .collect();

    let best = shifts
        .par_iter()
        .map(|&(dy, dx)| {
            let mut r_sum = 0.0f64;
            let mut t_sum = 0.0f64;
            let mut count = 0u32;

            for y in y_start..y_end {
                let ty = y as i32 + dy;
                if ty < 0 || ty >= rows as i32 {
                    continue;
                }
                for x in x_start..x_end {
                    let tx = x as i32 + dx;
                    if tx < 0 || tx >= cols as i32 {
                        continue;
                    }
                    let rv = reference[[y, x]] as f64;
                    let tv = target[[ty as usize, tx as usize]] as f64;
                    if rv.is_finite() && rv.abs() > 1e-7 && tv.is_finite() && tv.abs() > 1e-7 {
                        r_sum += rv;
                        t_sum += tv;
                        count += 1;
                    }
                }
            }

            if count < 10 {
                return (dy, dx, f64::NEG_INFINITY);
            }

            let r_mean = r_sum / count as f64;
            let t_mean = t_sum / count as f64;

            let mut num = 0.0f64;
            let mut r_var = 0.0f64;
            let mut t_var = 0.0f64;

            for y in y_start..y_end {
                let ty = y as i32 + dy;
                if ty < 0 || ty >= rows as i32 {
                    continue;
                }
                for x in x_start..x_end {
                    let tx = x as i32 + dx;
                    if tx < 0 || tx >= cols as i32 {
                        continue;
                    }
                    let rv = reference[[y, x]] as f64;
                    let tv = target[[ty as usize, tx as usize]] as f64;
                    if rv.is_finite() && rv.abs() > 1e-7 && tv.is_finite() && tv.abs() > 1e-7 {
                        let rd = rv - r_mean;
                        let td = tv - t_mean;
                        num += rd * td;
                        r_var += rd * rd;
                        t_var += td * td;
                    }
                }
            }

            if r_var > 0.0 && t_var > 0.0 {
                (dy, dx, num / (r_var * t_var).sqrt())
            } else {
                (dy, dx, f64::NEG_INFINITY)
            }
        })
        .reduce(
            || (0i32, 0i32, f64::NEG_INFINITY),
            |a, b| if b.2 > a.2 { b } else { a },
        );

    (best.0, best.1)
}

fn shift_image(image: &Array2<f32>, dy: i32, dx: i32) -> Array2<f32> {
    if dy == 0 && dx == 0 {
        return image.clone();
    }

    let (rows, cols) = image.dim();
    let mut shifted = Array2::zeros((rows, cols));

    for y in 0..rows {
        let sy = y as i32 - dy;
        if sy < 0 || sy >= rows as i32 {
            continue;
        }
        for x in 0..cols {
            let sx = x as i32 - dx;
            if sx < 0 || sx >= cols as i32 {
                continue;
            }
            shifted[[y, x]] = image[[sy as usize, sx as usize]];
        }
    }

    shifted
}

