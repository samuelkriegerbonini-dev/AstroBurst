use anyhow::{bail, Result};
use ndarray::{Array2, Zip, s};
use rayon::prelude::*;

use crate::core::alignment::pair::align_pair_with_label;
use crate::core::imaging::scnr;
use crate::core::imaging::stats;
use crate::core::imaging::stf::{self, AutoStfConfig, StfParams};
use crate::types::image::ImageStats;

pub use crate::types::compose::{
    AlignMethod, WhiteBalance, ChannelStats, DimensionCrop,
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
    pub offset_g: (f64, f64),
    pub offset_b: (f64, f64),
    pub scnr_applied: bool,
    pub dimension_crop: Option<DimensionCrop>,
    pub pre_stretch_r: Option<Array2<f32>>,
    pub pre_stretch_g: Option<Array2<f32>>,
    pub pre_stretch_b: Option<Array2<f32>>,
    pub stats_wb_r: Option<ImageStats>,
    pub stats_wb_g: Option<ImageStats>,
    pub stats_wb_b: Option<ImageStats>,
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
    match (alt1, alt2) {
        (Some(a), Some(b)) => {
            let mut out = Array2::zeros((rows, cols));
            Zip::from(&mut out).and(a).and(b)
                .par_for_each(|o, &av, &bv| *o = (av + bv) * 0.5);
            out
        }
        (Some(a), None) => a.clone(),
        (None, Some(b)) => b.clone(),
        (None, None) => Array2::zeros((rows, cols)),
    }
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

pub(crate) fn align_channels(
    r: Option<&Array2<f32>>, g: Option<&Array2<f32>>, b: Option<&Array2<f32>>,
    rows: usize, cols: usize, method: AlignMethod,
) -> Result<(Array2<f32>, Array2<f32>, Array2<f32>, (f64, f64), (f64, f64))> {
    let ref_ch = r.or(g).or(b).unwrap();
    let r_img = channel_or_synth(r, g, b, rows, cols);
    let g_img = channel_or_synth(g, r, b, rows, cols);
    let b_img = channel_or_synth(b, r, g, rows, cols);

    let (g_aligned, off_g) = if g.is_some() {
        let res = align_pair_with_label(ref_ch, &g_img, method, rows, cols, "G")?;
        (res.aligned, res.offset)
    } else {
        (g_img, (0.0, 0.0))
    };

    let (b_aligned, off_b) = if b.is_some() {
        let res = align_pair_with_label(ref_ch, &b_img, method, rows, cols, "B")?;
        (res.aligned, res.offset)
    } else {
        (b_img, (0.0, 0.0))
    };

    Ok((r_img, g_aligned, b_aligned, off_g, off_b))
}

fn select_wb_reference(sr: &ImageStats, sg: &ImageStats, sb: &ImageStats) -> (f64, f64, f64) {
    let stability = |s: &ImageStats| -> f64 {
        if s.median > 1e-10 { s.mad / s.median } else { f64::MAX }
    };
    let stab_r = stability(sr);
    let stab_g = stability(sg);
    let stab_b = stability(sb);
    if stab_r <= stab_g && stab_r <= stab_b {
        let m = sr.median.max(1e-10);
        (1.0, m / sg.median.max(1e-10), m / sb.median.max(1e-10))
    } else if stab_b <= stab_g {
        let m = sb.median.max(1e-10);
        (m / sr.median.max(1e-10), m / sg.median.max(1e-10), 1.0)
    } else {
        let m = sg.median.max(1e-10);
        (m / sr.median.max(1e-10), 1.0, m / sb.median.max(1e-10))
    }
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
            align_channels(r_ref, g_ref, b_ref, rows, cols, config.align_method)?
        } else {
            let r = channel_or_synth(r_ref, g_ref, b_ref, rows, cols);
            let g = channel_or_synth(g_ref, r_ref, b_ref, rows, cols);
            let b = channel_or_synth(b_ref, r_ref, g_ref, rows, cols);
            (r, g, b, (0.0, 0.0), (0.0, 0.0))
        }
    };

    drop(r_harm); drop(g_harm); drop(b_harm);

    let sr_full = stats::compute_image_stats(&r_img);
    let sg_full = stats::compute_image_stats(&g_img);
    let sb_full = stats::compute_image_stats(&b_img);

    let stats_r = ChannelStats::from(&sr_full);
    let stats_g = ChannelStats::from(&sg_full);
    let stats_b = ChannelStats::from(&sb_full);

    let (wb_r, wb_g, wb_b) = match &config.white_balance {
        WhiteBalance::Auto => select_wb_reference(&sr_full, &sg_full, &sb_full),
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

    let pre_r = Some(r_img.clone());
    let pre_g = Some(g_img.clone());
    let pre_b = Some(b_img.clone());
    let pre_sr = Some(stats_wb_r.clone());
    let pre_sg = Some(stats_wb_g.clone());
    let pre_sb = Some(stats_wb_b.clone());

    apply_stf_inplace(&mut r_img, &stf_r_params, &stats_wb_r);
    apply_stf_inplace(&mut g_img, &stf_g_params, &stats_wb_g);
    apply_stf_inplace(&mut b_img, &stf_b_params, &stats_wb_b);

    let scnr_applied = if let Some(ref scnr_cfg) = config.scnr {
        if r_img.dim() == g_img.dim() && g_img.dim() == b_img.dim() {
            scnr::apply_scnr_inplace(&mut r_img, &mut g_img, &mut b_img, scnr_cfg);
            true
        } else { false }
    } else { false };

    Ok(ProcessedRgb {
        r: r_img, g: g_img, b: b_img, rows, cols,
        stf_r: stf_r_params, stf_g: stf_g_params, stf_b: stf_b_params,
        stats_r, stats_g, stats_b,
        offset_g: off_g, offset_b: off_b, scnr_applied, dimension_crop,
        pre_stretch_r: pre_r,
        pre_stretch_g: pre_g,
        pre_stretch_b: pre_b,
        stats_wb_r: pre_sr,
        stats_wb_g: pre_sg,
        stats_wb_b: pre_sb,
    })
}
