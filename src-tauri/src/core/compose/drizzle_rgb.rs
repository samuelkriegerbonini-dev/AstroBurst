use ndarray::{Array2, s};

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

    let stats_r_raw = compute_channel_stats(&r_img);
    let stats_g_raw = compute_channel_stats(&g_img);
    let stats_b_raw = compute_channel_stats(&b_img);

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
