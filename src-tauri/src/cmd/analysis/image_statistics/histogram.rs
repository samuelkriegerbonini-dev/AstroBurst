use std::time::Instant;
use serde_json::json;
use rayon::prelude::*;
use crate::cmd::common::{blocking_cmd, cached_header, load_cached, HEADER_DISPLAY_REFERRED};
use crate::types::constants::{HISTOGRAM_BINS, HISTOGRAM_BINS_DISPLAY, RES_BINS, RES_BIN_COUNT, RES_MIN, RES_MAX, RES_DATA_MIN, RES_DATA_MAX, RES_MEDIAN, RES_MEAN, RES_SIGMA, RES_MAD, RES_TOTAL_PIXELS, RES_AUTO_STF, RES_SHADOW, RES_MIDTONE, RES_HIGHLIGHT, RES_ELAPSED_MS, RES_MASKED, RES_DQ_EXCLUDED};
use crate::types::image::{AutoStfConfig, Histogram, ImageStats, StfParams};
use crate::core::imaging::dq_flags::apply_exclusion;
use crate::core::imaging::stats::{build_histogram, compute_histogram_with_stats, compute_image_stats, downsample_histogram, is_valid_pixel};
use crate::core::imaging::stf::auto_stf;
use crate::cmd::analysis::resolve_dq_mask;

const IDENTITY_STF: StfParams = StfParams { shadow: 0.0, midtone: 0.5, highlight: 1.0 };
const HISTOGRAM_CHUNK: usize = 65536;
const MIN_HISTOGRAM_WINDOW: f64 = 1e-10;

fn is_display_referred(path: &str) -> bool {
    cached_header(path)
        .ok()
        .and_then(|h| h.get(HEADER_DISPLAY_REFERRED).map(|v| v.trim().trim_matches('\'').trim() == "T"))
        .unwrap_or(false)
}

fn display_frame(measured: &ImageStats, full: &ImageStats) -> ImageStats {
    let contains = full.min <= measured.min && measured.max <= full.max;
    if full.valid_count == 0 || !contains {
        return measured.clone();
    }
    ImageStats { min: full.min, max: full.max, ..measured.clone() }
}

fn reexpress_stf(stf: &StfParams, from: &ImageStats, to: &ImageStats) -> StfParams {
    let to_range = to.max - to.min;
    let from_range = from.max - from.min;
    let usable = |range: f64| range.is_finite() && range > 0.0;
    let same_frame = from.min == to.min && from.max == to.max;
    if same_frame || !usable(to_range) || !usable(from_range) {
        return *stf;
    }
    let map = |v: f64| ((from.min + v * from_range) - to.min) / to_range;
    StfParams { shadow: map(stf.shadow), midtone: stf.midtone, highlight: map(stf.highlight) }
}

fn histogram_window_arg(lo: Option<f64>, hi: Option<f64>) -> anyhow::Result<Option<(f64, f64)>> {
    let (lo, hi) = match (lo, hi) {
        (None, None) => return Ok(None),
        (Some(lo), Some(hi)) => (lo, hi),
        _ => anyhow::bail!("histogram window needs both lo and hi"),
    };
    if !lo.is_finite() || !hi.is_finite() {
        anyhow::bail!("histogram window ({lo}, {hi}) must be finite");
    }
    if hi <= lo {
        anyhow::bail!("histogram window hi {hi} must be larger than lo {lo}");
    }
    if hi - lo < MIN_HISTOGRAM_WINDOW {
        anyhow::bail!("histogram window from lo {lo} to hi {hi} is narrower than {MIN_HISTOGRAM_WINDOW}");
    }
    Ok(Some((lo, hi)))
}

pub(in crate::cmd::analysis) fn windowed_histogram(arr: &ndarray::Array2<f32>, lo: f64, hi: f64) -> Histogram {
    let mut hist = build_histogram(&[], HISTOGRAM_BINS, lo, hi);
    let range = hi - lo;
    if hist.bins.is_empty() || range < MIN_HISTOGRAM_WINDOW {
        return hist;
    }
    let bins = hist.bins.len();
    let last = bins - 1;
    let inv_bin_width = bins as f64 / range;
    let bin_of = |v: f32| -> Option<usize> {
        let vd = v as f64;
        (is_valid_pixel(v) && vd >= lo && vd <= hi).then(|| (((vd - lo) * inv_bin_width) as usize).min(last))
    };
    let count_into = |mut local: Vec<u32>, v: f32| {
        if let Some(i) = bin_of(v) {
            local[i] += 1;
        }
        local
    };
    hist.bins = match arr.as_slice() {
        Some(slice) => slice
            .par_chunks(HISTOGRAM_CHUNK)
            .fold(|| vec![0u32; bins], |local, chunk| chunk.iter().copied().fold(local, count_into))
            .reduce_with(|mut a, b| {
                for (ai, bi) in a.iter_mut().zip(b.iter()) {
                    *ai += bi;
                }
                a
            })
            .unwrap_or_else(|| vec![0u32; bins]),
        None => arr.iter().copied().fold(vec![0u32; bins], count_into),
    };
    hist
}

#[tauri::command]
pub async fn compute_histogram(
    path: String,
    exclude_dq: Option<bool>,
    lo: Option<f64>,
    hi: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let window = histogram_window_arg(lo, hi)?;
        let cached = load_cached(&path)?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), cached.arr().dim());

        let masked_arr = match &mask {
            Some(m) => Some(apply_exclusion(cached.arr(), &m.map)?),
            None => None,
        };
        let masked_stats = masked_arr.as_ref().map(compute_image_stats);
        let arr = masked_arr.as_ref().unwrap_or(cached.arr());
        let stats = masked_stats.as_ref().unwrap_or(cached.stats());
        let display_referred = is_display_referred(&path);
        let frame = if display_referred {
            ImageStats { min: 0.0, max: 1.0, ..stats.clone() }
        } else {
            display_frame(stats, cached.stats())
        };

        let hist = match window {
            Some((lo, hi)) => windowed_histogram(arr, lo, hi),
            None => compute_histogram_with_stats(arr, &frame),
        };
        let display_bins = downsample_histogram(&hist, HISTOGRAM_BINS_DISPLAY);
        let stf_params = if display_referred {
            IDENTITY_STF
        } else {
            reexpress_stf(&auto_stf(stats, &AutoStfConfig::default()), stats, &frame)
        };

        Ok(json!({
            RES_BINS: display_bins,
            RES_BIN_COUNT: display_bins.len(),
            RES_MIN: hist.min,
            RES_MAX: hist.max,
            RES_DATA_MIN: frame.min,
            RES_DATA_MAX: frame.max,
            RES_MEDIAN: stats.median,
            RES_MEAN: stats.mean,
            RES_SIGMA: stats.sigma,
            RES_MAD: stats.mad,
            RES_TOTAL_PIXELS: stats.valid_count,
            RES_AUTO_STF: {
                RES_SHADOW: stf_params.shadow,
                RES_MIDTONE: stf_params.midtone,
                RES_HIGHLIGHT: stf_params.highlight,
            },
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_MASKED: mask.is_some(),
            RES_DQ_EXCLUDED: mask.as_ref().map(|m| m.excluded),
        }))
    })
}
