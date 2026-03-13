use crate::math::median::{exact_median_mut, exact_mad_mut};
use crate::types::image::{Histogram, ImageStats};
use crate::types::constants::{PADDING_THRESHOLD, MAD_TO_SIGMA, HISTOGRAM_BINS};
use ndarray::Array2;
use rayon::prelude::*;

const CHUNK_SIZE: usize = 65536;

#[inline]
pub fn is_valid_pixel(v: f32) -> bool {
    v.is_finite() && v > PADDING_THRESHOLD
}

struct ChunkAccum {
    pixels: Vec<f32>,
    min: f64,
    max: f64,
    sum: f64,
}

pub fn compute_image_stats(data: &Array2<f32>) -> ImageStats {
    let slice = data.as_slice().expect("Array2 must be contiguous");

    let merged = slice
        .par_chunks(CHUNK_SIZE)
        .map(|chunk| {
            let mut acc = ChunkAccum {
                pixels: Vec::with_capacity(chunk.len()),
                min: f64::MAX,
                max: f64::MIN,
                sum: 0.0,
            };
            for &v in chunk {
                if is_valid_pixel(v) {
                    let vf = v as f64;
                    if vf < acc.min {
                        acc.min = vf;
                    }
                    if vf > acc.max {
                        acc.max = vf;
                    }
                    acc.sum += vf;
                    acc.pixels.push(v);
                }
            }
            acc
        })
        .reduce(
            || ChunkAccum {
                pixels: Vec::new(),
                min: f64::MAX,
                max: f64::MIN,
                sum: 0.0,
            },
            |mut a, b| {
                a.pixels.extend_from_slice(&b.pixels);
                a.min = a.min.min(b.min);
                a.max = a.max.max(b.max);
                a.sum += b.sum;
                a
            },
        );

    let mut valid = merged.pixels;
    let n = valid.len() as u64;
    if n == 0 {
        return ImageStats::default();
    }

    let mean = merged.sum / n as f64;
    let median = exact_median_mut(&mut valid);
    let mad_f32 = exact_mad_mut(&mut valid, median as f32);
    let mad = mad_f32 as f64;
    let sigma = (mad * MAD_TO_SIGMA).max(1e-30);

    ImageStats {
        min: merged.min,
        max: merged.max,
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
