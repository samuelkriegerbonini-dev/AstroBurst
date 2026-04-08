use crate::math::median::{exact_median_mut, exact_mad_mut};
use crate::types::image::{Histogram, ImageStats};
use crate::types::constants::{PADDING_THRESHOLD, MAD_TO_SIGMA, HISTOGRAM_BINS};
use ndarray::Array2;
use rayon::prelude::*;

const CHUNK_SIZE: usize = 65536;
const HIST_BINS: usize = 65536;

#[inline]
pub fn is_valid_pixel(v: f32) -> bool {
    v.is_finite() && v > PADDING_THRESHOLD
}

pub fn compute_image_stats(data: &Array2<f32>) -> ImageStats {
    let slice = data.as_slice().expect("Array2 must be contiguous");

    if slice.len() > 4_000_000 {
        return compute_image_stats_hist(slice);
    }

    compute_image_stats_exact(slice)
}

pub fn compute_image_stats_with_known_range(
    data: &Array2<f32>,
    known_min: f64,
    known_max: f64,
) -> ImageStats {
    let slice = data.as_slice().expect("Array2 must be contiguous");

    if slice.len() <= 4_000_000 {
        return compute_image_stats_exact(slice);
    }

    if !known_min.is_finite() || !known_max.is_finite() || known_min >= known_max {
        return compute_image_stats_hist(slice);
    }

    compute_stats_hist_core(slice, known_min, known_max)
}

fn compute_image_stats_exact(slice: &[f32]) -> ImageStats {
    let (global_min, global_max, global_sum, total_valid) = scan_stats(slice);

    if total_valid == 0 {
        return ImageStats::default();
    }

    let mut valid = Vec::with_capacity(total_valid);
    for &v in slice {
        if is_valid_pixel(v) {
            valid.push(v);
        }
    }

    let n = valid.len() as u64;
    let mean = global_sum / n as f64;
    let median = exact_median_mut(&mut valid);
    let mad_f32 = exact_mad_mut(&mut valid, median as f32);
    let mad = mad_f32 as f64;
    let sigma = (mad * MAD_TO_SIGMA).max(1e-30);

    ImageStats {
        min: global_min,
        max: global_max,
        mean,
        sigma,
        median,
        mad,
        valid_count: n,
    }
}

fn compute_image_stats_hist(slice: &[f32]) -> ImageStats {
    let (global_min, global_max) = scan_minmax(slice);

    if global_min == f64::MAX {
        return ImageStats::default();
    }

    compute_stats_hist_core(slice, global_min, global_max)
}

fn compute_stats_hist_core(slice: &[f32], global_min: f64, global_max: f64) -> ImageStats {
    let range = (global_max - global_min).max(1e-30);
    let bin_width = range / HIST_BINS as f64;
    let inv_bin = HIST_BINS as f64 / range;
    let last_bin = HIST_BINS - 1;

    let (global_sum, total_valid, value_hist) =
        scan_sum_and_hist(slice, global_min, inv_bin, last_bin);

    if total_valid == 0 {
        return ImageStats::default();
    }

    let n = total_valid as u64;
    let mean = global_sum / n as f64;
    let half_count = (total_valid as f64 * 0.5).ceil() as u64;

    let median_bin = find_percentile_bin(&value_hist, total_valid, 0.5);
    let count_before_median: u64 = value_hist[..median_bin].iter().sum();
    let median_bin_lo = global_min + median_bin as f64 * bin_width;
    let median_bin_hi = median_bin_lo + bin_width;

    let coarse_median = interpolate_percentile(
        &value_hist, total_valid, 0.5, global_min, bin_width,
    );

    let dev_range = range;
    let dev_bw = dev_range / HIST_BINS as f64;
    let dev_inv = HIST_BINS as f64 / dev_range;
    let coarse_med_f32 = coarse_median as f32;

    let refine_range = (median_bin_hi - median_bin_lo).max(1e-30);
    let refine_inv = HIST_BINS as f64 / refine_range;

    let (median_refine, dev_hist) = slice
        .par_chunks(CHUNK_SIZE)
        .fold(
            || (vec![0u64; HIST_BINS], vec![0u64; HIST_BINS]),
            |(mut refine, mut dev), chunk| {
                for &v in chunk {
                    if is_valid_pixel(v) {
                        let vf = v as f64;
                        if vf >= median_bin_lo && vf < median_bin_hi {
                            let idx = ((vf - median_bin_lo) * refine_inv) as usize;
                            refine[idx.min(last_bin)] += 1;
                        }
                        let d = (v - coarse_med_f32).abs();
                        let didx = (d as f64 * dev_inv) as usize;
                        dev[didx.min(last_bin)] += 1;
                    }
                }
                (refine, dev)
            },
        )
        .reduce(
            || (vec![0u64; HIST_BINS], vec![0u64; HIST_BINS]),
            |(mut ar, mut ad), (br, bd)| {
                for (a, b) in ar.iter_mut().zip(br.iter()) { *a += b; }
                for (a, b) in ad.iter_mut().zip(bd.iter()) { *a += b; }
                (ar, ad)
            },
        );

    let median_rank_in_bin = half_count.saturating_sub(count_before_median);
    let median_refine_bw = refine_range / HIST_BINS as f64;
    let median = resolve_rank_in_hist(
        &median_refine, median_rank_in_bin, median_bin_lo, median_refine_bw,
    );

    let mad_bin = find_percentile_bin(&dev_hist, total_valid, 0.5);
    let expand_lo = if mad_bin > 0 { mad_bin - 1 } else { 0 };
    let expand_hi = (mad_bin + 2).min(HIST_BINS);
    let mad_region_lo = expand_lo as f64 * dev_bw;
    let mad_region_hi = expand_hi as f64 * dev_bw;

    let exact_med_f32 = median as f32;
    let mad_refine_range = (mad_region_hi - mad_region_lo).max(1e-30);
    let mad_refine_inv = HIST_BINS as f64 / mad_refine_range;
    let mad_lo_f32 = mad_region_lo as f32;
    let mad_hi_f32 = mad_region_hi as f32;

    let (count_below, mad_refine) = slice
        .par_chunks(CHUNK_SIZE)
        .fold(
            || (0u64, vec![0u64; HIST_BINS]),
            |(mut below, mut h), chunk| {
                for &v in chunk {
                    if is_valid_pixel(v) {
                        let dev = (v - exact_med_f32).abs();
                        if dev < mad_lo_f32 {
                            below += 1;
                        } else if dev < mad_hi_f32 {
                            let idx = ((dev as f64 - mad_region_lo) * mad_refine_inv) as usize;
                            h[idx.min(last_bin)] += 1;
                        }
                    }
                }
                (below, h)
            },
        )
        .reduce(
            || (0u64, vec![0u64; HIST_BINS]),
            |(cb1, mut h1), (cb2, h2)| {
                for (a, b) in h1.iter_mut().zip(h2.iter()) { *a += b; }
                (cb1 + cb2, h1)
            },
        );

    let mad_rank_in_region = half_count.saturating_sub(count_below);
    let mad_refine_bw = mad_refine_range / HIST_BINS as f64;
    let mad = resolve_rank_in_hist(
        &mad_refine, mad_rank_in_region, mad_region_lo, mad_refine_bw,
    );

    let sigma = (mad * MAD_TO_SIGMA).max(1e-30);

    ImageStats {
        min: global_min,
        max: global_max,
        mean,
        sigma,
        median,
        mad,
        valid_count: n,
    }
}

fn scan_minmax(slice: &[f32]) -> (f64, f64) {
    slice
        .par_chunks(CHUNK_SIZE)
        .map(|chunk| {
            let mut mn = f64::MAX;
            let mut mx = f64::MIN;
            for &v in chunk {
                if is_valid_pixel(v) {
                    let vf = v as f64;
                    if vf < mn { mn = vf; }
                    if vf > mx { mx = vf; }
                }
            }
            (mn, mx)
        })
        .reduce(
            || (f64::MAX, f64::MIN),
            |(mn1, mx1), (mn2, mx2)| (mn1.min(mn2), mx1.max(mx2)),
        )
}

fn scan_stats(slice: &[f32]) -> (f64, f64, f64, usize) {
    slice
        .par_chunks(CHUNK_SIZE)
        .map(|chunk| {
            let mut mn = f64::MAX;
            let mut mx = f64::MIN;
            let mut s = 0.0f64;
            let mut cnt = 0usize;
            for &v in chunk {
                if is_valid_pixel(v) {
                    let vf = v as f64;
                    if vf < mn { mn = vf; }
                    if vf > mx { mx = vf; }
                    s += vf;
                    cnt += 1;
                }
            }
            (mn, mx, s, cnt)
        })
        .reduce(
            || (f64::MAX, f64::MIN, 0.0, 0usize),
            |(mn1, mx1, s1, c1), (mn2, mx2, s2, c2)| {
                (mn1.min(mn2), mx1.max(mx2), s1 + s2, c1 + c2)
            },
        )
}

fn scan_sum_and_hist(
    slice: &[f32],
    data_min: f64,
    inv_bin: f64,
    last_bin: usize,
) -> (f64, usize, Vec<u64>) {
    struct Acc {
        sum: f64,
        cnt: usize,
        hist: Vec<u64>,
    }

    let result = slice
        .par_chunks(CHUNK_SIZE)
        .fold(
            || Acc { sum: 0.0, cnt: 0, hist: vec![0u64; HIST_BINS] },
            |mut acc, chunk| {
                for &v in chunk {
                    if is_valid_pixel(v) {
                        let vf = v as f64;
                        acc.sum += vf;
                        acc.cnt += 1;
                        let idx = ((vf - data_min) * inv_bin) as usize;
                        acc.hist[idx.min(last_bin)] += 1;
                    }
                }
                acc
            },
        )
        .reduce(
            || Acc { sum: 0.0, cnt: 0, hist: vec![0u64; HIST_BINS] },
            |mut a, b| {
                a.sum += b.sum;
                a.cnt += b.cnt;
                for (ai, bi) in a.hist.iter_mut().zip(b.hist.iter()) { *ai += bi; }
                a
            },
        );

    (result.sum, result.cnt, result.hist)
}

fn find_percentile_bin(hist: &[u64], total: usize, pct: f64) -> usize {
    let target = (total as f64 * pct).ceil() as u64;
    let mut cum = 0u64;
    for (i, &count) in hist.iter().enumerate() {
        cum += count;
        if cum >= target {
            return i;
        }
    }
    hist.len() - 1
}

fn interpolate_percentile(
    hist: &[u64],
    total: usize,
    pct: f64,
    data_min: f64,
    bin_width: f64,
) -> f64 {
    let target = (total as f64 * pct).ceil() as u64;
    let mut cum = 0u64;
    for (i, &count) in hist.iter().enumerate() {
        cum += count;
        if cum >= target {
            let overshoot = cum - target;
            let frac = if count > 0 { 1.0 - (overshoot as f64 / count as f64) } else { 0.5 };
            return data_min + (i as f64 + frac) * bin_width;
        }
    }
    data_min + hist.len() as f64 * bin_width
}

fn resolve_rank_in_hist(
    hist: &[u64],
    rank: u64,
    region_lo: f64,
    sub_bin_width: f64,
) -> f64 {
    if rank == 0 {
        return region_lo;
    }
    let mut cum = 0u64;
    for (i, &count) in hist.iter().enumerate() {
        cum += count;
        if cum >= rank {
            let overshoot = cum - rank;
            let frac = if count > 0 { 1.0 - (overshoot as f64 / count as f64) } else { 0.5 };
            return region_lo + (i as f64 + frac) * sub_bin_width;
        }
    }
    region_lo + hist.len() as f64 * sub_bin_width
}

pub fn compute_histogram(data: &Array2<f32>, bins: usize) -> Histogram {
    let slice = data.as_slice().expect("Array2 must be contiguous");

    let (dmin, dmax) = slice
        .par_iter()
        .filter(|v| is_valid_pixel(**v))
        .fold(
            || (f64::MAX, f64::MIN),
            |(mn, mx), &v| (mn.min(v as f64), mx.max(v as f64)),
        )
        .reduce(
            || (f64::MAX, f64::MIN),
            |(mn1, mx1), (mn2, mx2)| (mn1.min(mn2), mx1.max(mx2)),
        );

    build_histogram(slice, bins, dmin, dmax)
}

pub fn compute_histogram_with_stats(data: &Array2<f32>, stats: &ImageStats) -> Histogram {
    let slice = data.as_slice().expect("Array2 must be contiguous");
    build_histogram(slice, HISTOGRAM_BINS, stats.min, stats.max)
}

fn build_histogram(slice: &[f32], bins: usize, dmin: f64, dmax: f64) -> Histogram {
    let range = dmax - dmin;
    if range < 1e-10 {
        return Histogram {
            bins: vec![0u32; bins],
            bin_edges: vec![dmin; bins + 1],
            min: dmin,
            max: dmax,
        };
    }

    let inv_bin_width = bins as f64 / range;

    let initial = vec![0u32; bins];
    let last = bins - 1;
    let histogram = slice
        .par_chunks(CHUNK_SIZE)
        .fold_with(initial.clone(), |mut local_bins, chunk| {
            for &v in chunk {
                if is_valid_pixel(v) {
                    let idx = ((v as f64 - dmin) * inv_bin_width) as usize;
                    local_bins[idx.min(last)] += 1;
                }
            }
            local_bins
        })
        .reduce_with(|mut a, b| {
            for (ai, bi) in a.iter_mut().zip(b.iter()) {
                *ai += bi;
            }
            a
        })
        .unwrap_or(initial);

    let step = range / bins as f64;
    let bin_edges: Vec<f64> = (0..=bins).map(|i| dmin + i as f64 * step).collect();

    Histogram {
        bins: histogram,
        bin_edges,
        min: dmin,
        max: dmax,
    }
}

pub fn downsample_histogram(hist: &Histogram, target_bins: usize) -> Vec<u32> {
    let src = &hist.bins;
    let src_len = src.len();
    if target_bins >= src_len {
        return src.clone();
    }

    let mut result = vec![0u32; target_bins];
    let ratio = src_len as f64 / target_bins as f64;

    for i in 0..target_bins {
        let start = (i as f64 * ratio) as usize;
        let end = (((i + 1) as f64 * ratio) as usize).min(src_len);
        let mut sum = 0u32;
        for j in start..end {
            sum = sum.saturating_add(src[j]);
        }
        result[i] = sum;
    }

    result
}
