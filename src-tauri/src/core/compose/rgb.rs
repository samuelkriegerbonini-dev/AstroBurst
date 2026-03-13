use anyhow::{bail, Result};
use ndarray::{Array2, s};
use rayon::prelude::*;

use crate::core::imaging::scnr;
use crate::core::imaging::stats;
use crate::core::imaging::stf::{self, AutoStfConfig, StfParams};
use crate::types::image::ImageStats;

pub use crate::types::compose::{
    WhiteBalance, ChannelStats, DimensionCrop,
    RgbComposeConfig, RgbComposeResult,
};

pub struct ProcessedRgb {
    pub r: Array2<f32>,
    pub g: Array2<f32>,
    pub b: Array2<f32>,
    pub rows: usize,
    pub cols: usize,
    pub stf_r: StfParams,
    pub stf_g: StfParams,
    pub stf_b: StfParams,
    pub stats_r: ChannelStats,
    pub stats_g: ChannelStats,
    pub stats_b: ChannelStats,
    pub offset_g: (i32, i32),
    pub offset_b: (i32, i32),
    pub scnr_applied: bool,
    pub dimension_crop: Option<DimensionCrop>,
}

fn crop_to_size(arr: &Array2<f32>, rows: usize, cols: usize) -> Array2<f32> {
    arr.slice(s![..rows, ..cols]).to_owned()
}

pub fn harmonize_dimensions(
    r: Option<&Array2<f32>>,
    g: Option<&Array2<f32>>,
    b: Option<&Array2<f32>>,
    tolerance: usize,
) -> Result<(
    Option<Array2<f32>>,
    Option<Array2<f32>>,
    Option<Array2<f32>>,
    usize,
    usize,
    Option<DimensionCrop>,
)> {
    let dims: Vec<(usize, usize)> = [r, g, b]
        .into_iter()
        .flatten()
        .map(|a| a.dim())
        .collect();

    if dims.is_empty() {
        return Ok((None, None, None, 0, 0, None));
    }

    let min_rows = dims.iter().map(|d| d.0).min().unwrap();
    let min_cols = dims.iter().map(|d| d.1).min().unwrap();
    let max_rows = dims.iter().map(|d| d.0).max().unwrap();
    let max_cols = dims.iter().map(|d| d.1).max().unwrap();

    let row_diff = max_rows - min_rows;
    let col_diff = max_cols - min_cols;

    if row_diff == 0 && col_diff == 0 {
        return Ok((
            r.map(|a| a.clone()),
            g.map(|a| a.clone()),
            b.map(|a| a.clone()),
            min_rows,
            min_cols,
            None,
        ));
    }

    let pct_threshold = (min_rows.max(min_cols) as f64 * 0.01) as usize;
    let effective_tolerance = tolerance.max(pct_threshold);

    if row_diff > effective_tolerance || col_diff > effective_tolerance {
        let mut msg = format!(
            "Channel dimensions differ by more than {}px (rows: {}px, cols: {}px).",
            effective_tolerance, row_diff, col_diff
        );
        if let Some(ra) = r { msg.push_str(&format!(" R={}x{}", ra.dim().1, ra.dim().0)); }
        if let Some(ga) = g { msg.push_str(&format!(" G={}x{}", ga.dim().1, ga.dim().0)); }
        if let Some(ba) = b { msg.push_str(&format!(" B={}x{}", ba.dim().1, ba.dim().0)); }
        msg.push_str(". Use alignment or manually crop.");
        bail!("{}", msg);
    }

    let crop_info = DimensionCrop {
        original_r: r.map(|a| [a.dim().1, a.dim().0]),
        original_g: g.map(|a| [a.dim().1, a.dim().0]),
        original_b: b.map(|a| [a.dim().1, a.dim().0]),
        cropped_to: [min_cols, min_rows],
    };

    let conform = |channel: Option<&Array2<f32>>, rows, cols| {
        channel.map(|a| {
            if a.dim() == (rows, cols) { a.clone() } else { crop_to_size(a, rows, cols) }
        })
    };

    Ok((conform(r, min_rows, min_cols), conform(g, min_rows, min_cols), conform(b, min_rows, min_cols), min_rows, min_cols, Some(crop_info)))
}

fn to_channel_stats(st: &ImageStats) -> ChannelStats {
    ChannelStats { min: st.min, max: st.max, median: st.median, mean: st.mean }
}

fn apply_multiplier_inplace(arr: &mut Array2<f32>, mult: f32) {
    if (mult - 1.0).abs() < 1e-7 { return; }
    arr.par_mapv_inplace(|v| v * mult);
}

fn channel_or_synth(
    primary: Option<&Array2<f32>>,
    alt1: Option<&Array2<f32>>,
    alt2: Option<&Array2<f32>>,
    rows: usize,
    cols: usize,
) -> Array2<f32> {
    if let Some(ch) = primary { return ch.clone(); }
    let a = alt1.map(|a| a.clone()).unwrap_or_else(|| Array2::zeros((rows, cols)));
    let b = alt2.map(|b| b.clone()).unwrap_or_else(|| Array2::zeros((rows, cols)));
    (&a + &b) / 2.0
}

fn merge_for_stf(r: &Array2<f32>, g: &Array2<f32>, b: &Array2<f32>) -> Array2<f32> {
    let (rows, cols) = r.dim();
    let r_s = r.as_slice().expect("contiguous");
    let g_s = g.as_slice().expect("contiguous");
    let b_s = b.as_slice().expect("contiguous");
    let pixels: Vec<f32> = (0..rows * cols)
        .into_par_iter()
        .map(|i| (r_s[i] + g_s[i] + b_s[i]) * (1.0 / 3.0))
        .collect();
    Array2::from_shape_vec((rows, cols), pixels).unwrap()
}

fn downsample_2x(img: &Array2<f32>) -> Array2<f32> {
    let (rows, cols) = img.dim();
    if rows < 2 || cols < 2 { return img.clone(); }
    let src = img.as_slice().expect("contiguous");
    let nr = rows / 2;
    let nc = cols / 2;
    let mut out = vec![0.0f32; nr * nc];
    out.par_chunks_mut(nc).enumerate().for_each(|(r, row)| {
        let r2 = r * 2;
        let b0 = r2 * cols;
        let b1 = (r2 + 1) * cols;
        for c in 0..nc {
            let c2 = c * 2;
            row[c] = (src[b0 + c2] + src[b0 + c2 + 1] + src[b1 + c2] + src[b1 + c2 + 1]) * 0.25;
        }
    });
    Array2::from_shape_vec((nr, nc), out).unwrap()
}

fn find_offset_parallel(
    reference: &Array2<f32>, target: &Array2<f32>,
    max_shift: i32, center_dy: i32, center_dx: i32,
) -> (i32, i32) {
    let (rows, cols) = reference.dim();
    let ref_s = reference.as_slice().expect("contiguous");
    let tgt_s = target.as_slice().expect("contiguous");
    let cy = rows / 2;
    let cx = cols / 2;
    let region = (rows.min(cols) / 4).max(1);
    let y_start = cy.saturating_sub(region);
    let y_end = (cy + region).min(rows);
    let x_start = cx.saturating_sub(region);
    let x_end = (cx + region).min(cols);
    let rows_i = rows as i32;
    let cols_i = cols as i32;

    let shifts: Vec<(i32, i32)> = (-max_shift..=max_shift)
        .flat_map(|dy| (-max_shift..=max_shift).map(move |dx| (center_dy + dy, center_dx + dx)))
        .collect();

    let best = shifts.par_iter().map(|&(dy, dx)| {
        let mut r_sum = 0.0f64;
        let mut t_sum = 0.0f64;
        let mut count = 0u32;
        for y in y_start..y_end {
            let ty = y as i32 + dy;
            if ty < 0 || ty >= rows_i { continue; }
            let rr = y * cols;
            let tr = ty as usize * cols;
            for x in x_start..x_end {
                let tx = x as i32 + dx;
                if tx < 0 || tx >= cols_i { continue; }
                let rv = ref_s[rr + x] as f64;
                let tv = tgt_s[tr + tx as usize] as f64;
                if rv.is_finite() && rv.abs() > 1e-7 && tv.is_finite() && tv.abs() > 1e-7 {
                    r_sum += rv; t_sum += tv; count += 1;
                }
            }
        }
        if count < 10 { return (dy, dx, f64::NEG_INFINITY); }
        let r_mean = r_sum / count as f64;
        let t_mean = t_sum / count as f64;
        let mut num = 0.0f64;
        let mut r_var = 0.0f64;
        let mut t_var = 0.0f64;
        for y in y_start..y_end {
            let ty = y as i32 + dy;
            if ty < 0 || ty >= rows_i { continue; }
            let rr = y * cols;
            let tr = ty as usize * cols;
            for x in x_start..x_end {
                let tx = x as i32 + dx;
                if tx < 0 || tx >= cols_i { continue; }
                let rv = ref_s[rr + x] as f64;
                let tv = tgt_s[tr + tx as usize] as f64;
                if rv.is_finite() && rv.abs() > 1e-7 && tv.is_finite() && tv.abs() > 1e-7 {
                    let rd = rv - r_mean;
                    let td = tv - t_mean;
                    num += rd * td;
                    r_var += rd * rd;
                    t_var += td * td;
                }
            }
        }
        if r_var > 0.0 && t_var > 0.0 { (dy, dx, num / (r_var * t_var).sqrt()) }
        else { (dy, dx, f64::NEG_INFINITY) }
    }).reduce(|| (0i32, 0i32, f64::NEG_INFINITY), |a, b| if b.2 > a.2 { b } else { a });
    (best.0, best.1)
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

fn shift_image(image: &Array2<f32>, dy: i32, dx: i32) -> Array2<f32> {
    if dy == 0 && dx == 0 { return image.clone(); }
    let (rows, cols) = image.dim();
    let src = image.as_slice().expect("contiguous");
    let mut out = vec![0.0f32; rows * cols];
    out.par_chunks_mut(cols).enumerate().for_each(|(y, row)| {
        let sy = y as i32 - dy;
        if sy < 0 || sy >= rows as i32 { return; }
        let sr = sy as usize * cols;
        for x in 0..cols {
            let sx = x as i32 - dx;
            if sx >= 0 && sx < cols as i32 { row[x] = src[sr + sx as usize]; }
        }
    });
    Array2::from_shape_vec((rows, cols), out).unwrap()
}

fn align_channels(
    r: Option<&Array2<f32>>, g: Option<&Array2<f32>>, b: Option<&Array2<f32>>,
    rows: usize, cols: usize,
) -> Result<(Array2<f32>, Array2<f32>, Array2<f32>, (i32, i32), (i32, i32))> {
    let ref_ch = r.or(g).or(b).unwrap();
    let r_img = channel_or_synth(r, g, b, rows, cols);
    let g_img = channel_or_synth(g, r, b, rows, cols);
    let b_img = channel_or_synth(b, r, g, rows, cols);
    let off_g = if g.is_some() { find_offset_pyramid(ref_ch, &g_img) } else { (0, 0) };
    let off_b = if b.is_some() { find_offset_pyramid(ref_ch, &b_img) } else { (0, 0) };
    let g_shifted = shift_image(&g_img, off_g.0, off_g.1);
    let b_shifted = shift_image(&b_img, off_b.0, off_b.1);
    Ok((r_img, g_shifted, b_shifted, off_g, off_b))
}

fn apply_stf_inplace(data: &mut Array2<f32>, params: &StfParams, st: &ImageStats) {
    let range = (st.max - st.min).max(1e-30);
    let inv_range = 1.0 / range;
    let dmin = st.min;
    let shadow = params.shadow;
    let highlight = params.highlight;
    let clip_range = (highlight - shadow).max(1e-15);
    let m = params.midtone;
    data.par_mapv_inplace(|v| {
        if !v.is_finite() || v <= 1e-7 { return 0.0; }
        let norm = (v as f64 - dmin) * inv_range;
        let clipped = ((norm - shadow) / clip_range).clamp(0.0, 1.0);
        if clipped <= 0.0 { return 0.0; }
        if clipped >= 1.0 { return 1.0; }
        ((m - 1.0) * clipped / ((2.0 * m - 1.0) * clipped - m)) as f32
    });
}

pub fn process_rgb(
    r_channel: Option<&Array2<f32>>,
    g_channel: Option<&Array2<f32>>,
    b_channel: Option<&Array2<f32>>,
    config: &RgbComposeConfig,
) -> Result<ProcessedRgb> {
    let present = [r_channel.is_some(), g_channel.is_some(), b_channel.is_some()];
    let count = present.iter().filter(|&&b| b).count();
    if count < 2 { bail!("Need at least 2 channels for RGB compose (got {})", count); }

    let (r_harm, g_harm, b_harm, rows, cols, dimension_crop) =
        harmonize_dimensions(r_channel, g_channel, b_channel, config.dimension_tolerance)?;

    let (mut r_img, mut g_img, mut b_img, off_g, off_b) = {
        let r_ref = r_harm.as_ref().or(r_channel);
        let g_ref = g_harm.as_ref().or(g_channel);
        let b_ref = b_harm.as_ref().or(b_channel);
        if config.align && count >= 2 {
            align_channels(r_ref, g_ref, b_ref, rows, cols)?
        } else {
            let r = channel_or_synth(r_ref, g_ref, b_ref, rows, cols);
            let g = channel_or_synth(g_ref, r_ref, b_ref, rows, cols);
            let b = channel_or_synth(b_ref, r_ref, g_ref, rows, cols);
            (r, g, b, (0, 0), (0, 0))
        }
    };

    drop(r_harm); drop(g_harm); drop(b_harm);

    let sr_full = stats::compute_image_stats(&r_img);
    let sg_full = stats::compute_image_stats(&g_img);
    let sb_full = stats::compute_image_stats(&b_img);

    let stats_r = to_channel_stats(&sr_full);
    let stats_g = to_channel_stats(&sg_full);
    let stats_b = to_channel_stats(&sb_full);

    let (wb_r, wb_g, wb_b) = match &config.white_balance {
        WhiteBalance::Auto => {
            let ref_med = stats_g.median.max(1e-10);
            (ref_med / stats_r.median.max(1e-10), 1.0_f64, ref_med / stats_b.median.max(1e-10))
        }
        WhiteBalance::Manual(r, g, b) => (*r, *g, *b),
        WhiteBalance::None => (1.0, 1.0, 1.0),
    };

    apply_multiplier_inplace(&mut r_img, wb_r as f32);
    apply_multiplier_inplace(&mut g_img, wb_g as f32);
    apply_multiplier_inplace(&mut b_img, wb_b as f32);

    let stf_config = AutoStfConfig::default();

    let (stf_r_params, stf_g_params, stf_b_params, stats_wb_r, stats_wb_g, stats_wb_b) =
        if config.auto_stretch {
            if config.linked_stf {
                let combined = merge_for_stf(&r_img, &g_img, &b_img);
                let (st, _hist) = stf::analyze(&combined);
                drop(combined);
                let params = stf::auto_stf(&st, &stf_config);
                let sr = stats::compute_image_stats(&r_img);
                let sg = stats::compute_image_stats(&g_img);
                let sb = stats::compute_image_stats(&b_img);
                (params, params, params, sr, sg, sb)
            } else {
                let (sr, _) = stf::analyze(&r_img);
                let (sg, _) = stf::analyze(&g_img);
                let (sb, _) = stf::analyze(&b_img);
                let pr = stf::auto_stf(&sr, &stf_config);
                let pg = stf::auto_stf(&sg, &stf_config);
                let pb = stf::auto_stf(&sb, &stf_config);
                (pr, pg, pb, sr, sg, sb)
            }
        } else {
            let sr = stats::compute_image_stats(&r_img);
            let sg = stats::compute_image_stats(&g_img);
            let sb = stats::compute_image_stats(&b_img);
            (
                config.stf_r.unwrap_or(StfParams { shadow: 0.0, midtone: 0.5, highlight: 1.0 }),
                config.stf_g.unwrap_or(StfParams { shadow: 0.0, midtone: 0.5, highlight: 1.0 }),
                config.stf_b.unwrap_or(StfParams { shadow: 0.0, midtone: 0.5, highlight: 1.0 }),
                sr, sg, sb,
            )
        };

    apply_stf_inplace(&mut r_img, &stf_r_params, &stats_wb_r);
    apply_stf_inplace(&mut g_img, &stf_g_params, &stats_wb_g);
    apply_stf_inplace(&mut b_img, &stf_b_params, &stats_wb_b);

    let scnr_applied = if let Some(ref scnr_cfg) = config.scnr {
        if r_img.dim() == g_img.dim() && g_img.dim() == b_img.dim() {
            scnr::apply_scnr_inplace(&r_img, &mut g_img, &b_img, scnr_cfg);
            true
        } else { false }
    } else { false };

    Ok(ProcessedRgb {
        r: r_img, g: g_img, b: b_img, rows, cols,
        stf_r: stf_r_params, stf_g: stf_g_params, stf_b: stf_b_params,
        stats_r, stats_g, stats_b,
        offset_g: off_g, offset_b: off_b, scnr_applied, dimension_crop,
    })
}
