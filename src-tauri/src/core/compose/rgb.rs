use anyhow::{bail, Result};
use ndarray::{Array2, Zip};
use rayon::prelude::*;

use crate::core::alignment::pair::align_pair_with_label;
use crate::core::compose::white_balance;
use crate::core::imaging::resample::resample_image;
use crate::core::imaging::scnr;
use crate::core::imaging::stats;
use crate::core::imaging::stf::{self, AutoStfConfig, StfParams};
use crate::types::image::ImageStats;

pub use crate::types::compose::{
    AlignMethod, WhiteBalance, ChannelStats, DimensionHarmonize,
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
    pub dimension_info: Option<DimensionHarmonize>,
    pub pre_stretch_r: Option<Array2<f32>>,
    pub pre_stretch_g: Option<Array2<f32>>,
    pub pre_stretch_b: Option<Array2<f32>>,
    pub stats_wb_r: Option<ImageStats>,
    pub stats_wb_g: Option<ImageStats>,
    pub stats_wb_b: Option<ImageStats>,
}

pub fn harmonize_dimensions(
    r: Option<&Array2<f32>>,
    g: Option<&Array2<f32>>,
    b: Option<&Array2<f32>>,
    max_ratio: f64,
) -> Result<(
    Option<Array2<f32>>,
    Option<Array2<f32>>,
    Option<Array2<f32>>,
    usize,
    usize,
    Option<DimensionHarmonize>,
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

    if max_rows == min_rows && max_cols == min_cols {
        return Ok((None, None, None, max_rows, max_cols, None));
    }

    let ratio_rows = max_rows as f64 / min_rows.max(1) as f64;
    let ratio_cols = max_cols as f64 / min_cols.max(1) as f64;
    let ratio = ratio_rows.max(ratio_cols);

    if ratio > max_ratio {
        let mut msg = format!(
            "Channel dimension ratio {:.1}x exceeds {:.0}x limit.",
            ratio, max_ratio
        );
        if let Some(ra) = r { msg.push_str(&format!(" R={}x{}", ra.dim().1, ra.dim().0)); }
        if let Some(ga) = g { msg.push_str(&format!(" G={}x{}", ga.dim().1, ga.dim().0)); }
        if let Some(ba) = b { msg.push_str(&format!(" B={}x{}", ba.dim().1, ba.dim().0)); }
        msg.push_str(". Check channel assignments.");
        bail!("{}", msg);
    }

    log::info!(
        "harmonize_dimensions: resampling channels to {}x{} (ratio {:.2}x)",
        max_cols, max_rows, ratio
    );

    let info = DimensionHarmonize {
        original_r: r.map(|a| [a.dim().1, a.dim().0]),
        original_g: g.map(|a| [a.dim().1, a.dim().0]),
        original_b: b.map(|a| [a.dim().1, a.dim().0]),
        target: [max_cols, max_rows],
        resampled: true,
    };

    let resample_ch = |channel: Option<&Array2<f32>>| -> Result<Option<Array2<f32>>> {
        match channel {
            Some(a) => {
                let (rows, cols) = a.dim();
                if rows == max_rows && cols == max_cols {
                    Ok(Some(a.clone()))
                } else {
                    Ok(Some(resample_image(a, max_rows, max_cols)?))
                }
            }
            None => Ok(None),
        }
    };

    Ok((
        resample_ch(r)?,
        resample_ch(g)?,
        resample_ch(b)?,
        max_rows,
        max_cols,
        Some(info),
    ))
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
    let mut out = Array2::zeros((rows, cols));

    Zip::from(&mut out).and(r).and(g).and(b)
        .par_for_each(|o, &rv, &gv, &bv| {
            *o = (rv + gv + bv) * (1.0 / 3.0);
        });

    out
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

    let max_ratio = crate::types::constants::MAX_DIMENSION_RATIO;

    let (r_harm, g_harm, b_harm, rows, cols, dimension_info) =
        harmonize_dimensions(r_channel, g_channel, b_channel, max_ratio)?;

    let r_eff = r_harm.as_ref().or(r_channel);
    let g_eff = g_harm.as_ref().or(g_channel);
    let b_eff = b_harm.as_ref().or(b_channel);

    let (mut r_img, mut g_img, mut b_img, off_g, off_b) = {
        if config.align && count >= 2 {
            align_channels(r_eff, g_eff, b_eff, rows, cols, config.align_method)?
        } else {
            let r = channel_or_synth(r_eff, g_eff, b_eff, rows, cols);
            let g = channel_or_synth(g_eff, r_eff, b_eff, rows, cols);
            let b = channel_or_synth(b_eff, r_eff, g_eff, rows, cols);
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
        WhiteBalance::Auto => white_balance::select_wb_reference(&sr_full, &sg_full, &sb_full),
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
        offset_g: off_g, offset_b: off_b, scnr_applied, dimension_info,
        pre_stretch_r: pre_r,
        pre_stretch_g: pre_g,
        pre_stretch_b: pre_b,
        stats_wb_r: pre_sr,
        stats_wb_g: pre_sg,
        stats_wb_b: pre_sb,
    })
}
