use crate::math::median::{exact_median_mut, exact_median_f64};
use crate::types::image::{Histogram, ImageStats};
use crate::types::constants::{PADDING_THRESHOLD, MAD_TO_SIGMA, HISTOGRAM_BINS};
use ndarray::Array2;
use rayon::prelude::*;

const CHUNK_SIZE: usize = 65536;

#[inline]
pub fn is_valid_pixel(v: f32) -> bool {
    v.is_finite() && v > PADDING_THRESHOLD
}

pub fn compute_image_stats(data: &Array2<f32>) -> ImageStats {
    let slice = data.as_slice().expect("Array2 must be contiguous");

    let mut valid: Vec<f32> = slice
        .par_iter()
        .copied()
        .filter(|&v| is_valid_pixel(v))
        .collect();

    let n = valid.len() as u64;
    if n == 0 {
        return ImageStats::default();
    }

    let median = exact_median_mut(&mut valid);

    let deviations: Vec<f64> = valid
        .par_iter()
        .map(|&v| (v as f64 - median).abs())
        .collect();
    let mad = exact_median_f64(&deviations);

    let sigma = (mad * MAD_TO_SIGMA).max(1e-30);

    struct Accum {
        min: f64,
        max: f64,
        sum: f64,
    }

    let acc = valid
        .par_iter()
        .fold(
            || Accum {
                min: f64::MAX,
                max: f64::MIN,
                sum: 0.0,
            },
            |mut a, &v| {
                let vf = v as f64;
                if vf < a.min { a.min = vf; }
                if vf > a.max { a.max = vf; }
                a.sum += vf;
                a
            },
        )
        .reduce(
            || Accum {
                min: f64::MAX,
                max: f64::MIN,
                sum: 0.0,
            },
            |a, b| Accum {
                min: a.min.min(b.min),
                max: a.max.max(b.max),
                sum: a.sum + b.sum,
            },
        );

    let mean = acc.sum / n as f64;

    ImageStats {
        min: acc.min,
        max: acc.max,
        mean,
        sigma,
        median,
        mad,
        valid_count: n,
    }
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

    let inv_range = (bins as f64 - 1.0) / range;

    let initial = vec![0u32; bins];
    let histogram = slice
        .par_chunks(CHUNK_SIZE)
        .fold_with(initial.clone(), |mut local_bins, chunk| {
            for &v in chunk {
                if is_valid_pixel(v) {
                    let idx = ((v as f64 - dmin) * inv_range) as usize;
                    local_bins[idx.min(bins - 1)] += 1;
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
