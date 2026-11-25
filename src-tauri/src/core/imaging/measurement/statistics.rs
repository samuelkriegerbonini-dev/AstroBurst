use anyhow::{bail, Result};
use ndarray::Array2;
use rayon::prelude::*;
use serde::Serialize;

use crate::core::imaging::region::RegionShape;
use crate::core::imaging::stats::is_valid_pixel;
use crate::core::imaging::wavelet::k_sigma_noise;
use crate::math::median::exact_median_mut;

pub const NOISE_METHOD_K_SIGMA_MRS: &str = "k-sigma-mrs";
pub const MIN_NOISE_REGION_PIXELS: usize = 64;

const BIWEIGHT_TUNING_CONSTANT: f64 = 9.0;
const NOISE_CLIP_K: f64 = 3.0;
const NOISE_MAX_ITERATIONS: usize = 10;
const PAR_THRESHOLD: usize = 262_144;
const PAR_CHUNK: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChannelStatistics {
    pub count: u64,
    pub total: u64,
    pub fraction: f64,
    pub mean: f64,
    pub median: f64,
    pub avg_dev: f64,
    pub mad: f64,
    pub bwmv_sqrt: f64,
    pub min: f64,
    pub max: f64,
    pub sum: f64,
    pub variance: f64,
    pub std_dev: f64,
    pub nan_count: u64,
    pub padding: u64,
    pub excluded: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct PixelTally {
    total: u64,
    nan_count: u64,
    padding: u64,
    excluded: u64,
}

impl PixelTally {
    fn take(&mut self, v: f32, values: &mut Vec<f32>) {
        if is_valid_pixel(v) {
            values.push(v);
        } else if v.is_finite() {
            self.padding += 1;
        } else {
            self.nan_count += 1;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoiseEvaluation {
    pub sigma: f64,
    pub fraction: f64,
    pub iterations: usize,
    pub method: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RegionNoise {
    Evaluated(NoiseEvaluation),
    TooSmall { finite_pixels: usize },
}

impl RegionNoise {
    pub fn evaluation(&self) -> Option<&NoiseEvaluation> {
        match self {
            RegionNoise::Evaluated(noise) => Some(noise),
            RegionNoise::TooSmall { .. } => None,
        }
    }

    pub fn note(&self) -> Option<String> {
        match self {
            RegionNoise::Evaluated(_) => None,
            RegionNoise::TooSmall { finite_pixels } => Some(format!(
                "region too small for noise evaluation ({} finite pixels, {} needed)",
                finite_pixels, MIN_NOISE_REGION_PIXELS
            )),
        }
    }
}

impl ChannelStatistics {
    fn empty(tally: PixelTally) -> Self {
        Self {
            count: 0,
            total: tally.total,
            fraction: 0.0,
            mean: 0.0,
            median: 0.0,
            avg_dev: 0.0,
            mad: 0.0,
            bwmv_sqrt: 0.0,
            min: 0.0,
            max: 0.0,
            sum: 0.0,
            variance: 0.0,
            std_dev: 0.0,
            nan_count: tally.nan_count,
            padding: tally.padding,
            excluded: tally.excluded,
        }
    }
}

fn reduce_values<T, M, R>(values: &[f32], identity: T, map: M, reduce: R) -> T
where
    T: Copy + Send + Sync,
    M: Fn(T, f32) -> T + Sync + Send,
    R: Fn(T, T) -> T + Sync + Send,
{
    if values.len() >= PAR_THRESHOLD {
        values
            .par_chunks(PAR_CHUNK)
            .map(|chunk| chunk.iter().fold(identity, |acc, &v| map(acc, v)))
            .reduce(|| identity, &reduce)
    } else {
        values.iter().fold(identity, |acc, &v| map(acc, v))
    }
}

fn biweight_midvariance(values: &[f32], median: f64, mad: f64) -> f64 {
    if !(mad > 0.0) {
        return 0.0;
    }
    let scale = BIWEIGHT_TUNING_CONSTANT * mad;
    let (num, den) = reduce_values(
        values,
        (0.0f64, 0.0f64),
        |(num, den), v| {
            let d = v as f64 - median;
            let u = d / scale;
            if u.abs() < 1.0 {
                let w = 1.0 - u * u;
                (num + d * d * w.powi(4), den + w * (1.0 - 5.0 * u * u))
            } else {
                (num, den)
            }
        },
        |(a, b), (c, d)| (a + c, b + d),
    );
    if den == 0.0 {
        return 0.0;
    }
    (values.len() as f64 * num / (den * den)).max(0.0)
}

pub fn statistics_from_finite(values: Vec<f32>, total: u64, nan_count: u64, excluded: u64) -> ChannelStatistics {
    statistics_from_tally(values, PixelTally { total, nan_count, padding: 0, excluded })
}

fn statistics_from_tally(mut values: Vec<f32>, tally: PixelTally) -> ChannelStatistics {
    let PixelTally { total, nan_count, padding, excluded } = tally;
    let count = values.len() as u64;
    if count == 0 {
        return ChannelStatistics::empty(tally);
    }
    let (min, max, sum) = reduce_values(
        &values,
        (f64::INFINITY, f64::NEG_INFINITY, 0.0f64),
        |(lo, hi, s), v| {
            let vf = v as f64;
            (lo.min(vf), hi.max(vf), s + vf)
        },
        |(lo_a, hi_a, s_a), (lo_b, hi_b, s_b)| (lo_a.min(lo_b), hi_a.max(hi_b), s_a + s_b),
    );
    let n = count as f64;
    let mean = sum / n;
    let median = exact_median_mut(&mut values);
    let (abs_dev_sum, sq_dev_sum) = reduce_values(
        &values,
        (0.0f64, 0.0f64),
        |(a, q), v| {
            let vf = v as f64;
            (a + (vf - median).abs(), q + (vf - mean) * (vf - mean))
        },
        |(a, b), (c, d)| (a + c, b + d),
    );
    let avg_dev = abs_dev_sum / n;
    let variance = if count > 1 { sq_dev_sum / (n - 1.0) } else { 0.0 };
    let std_dev = variance.sqrt();
    let mut deviations: Vec<f32> = values.iter().map(|&v| (v as f64 - median).abs() as f32).collect();
    let mad = exact_median_mut(&mut deviations);
    drop(deviations);
    let bwmv_sqrt = biweight_midvariance(&values, median, mad).sqrt();
    ChannelStatistics {
        count,
        total,
        fraction: if total > 0 { count as f64 / total as f64 } else { 0.0 },
        mean,
        median,
        avg_dev,
        mad,
        bwmv_sqrt,
        min,
        max,
        sum,
        variance,
        std_dev,
        nan_count,
        padding,
        excluded,
    }
}

fn collect_valid(data: &Array2<f32>, excluded: Option<&Array2<u8>>) -> (Vec<f32>, PixelTally) {
    let mut values = Vec::with_capacity(data.len());
    let mut tally = PixelTally { total: data.len() as u64, ..PixelTally::default() };
    match excluded {
        Some(mask) if mask.dim() == data.dim() => {
            for (&v, &m) in data.iter().zip(mask.iter()) {
                if m != 0 {
                    tally.excluded += 1;
                } else {
                    tally.take(v, &mut values);
                }
            }
        }
        _ => {
            for &v in data.iter() {
                tally.take(v, &mut values);
            }
        }
    }
    (values, tally)
}

pub fn exact_statistics(data: &Array2<f32>, excluded: Option<&Array2<u8>>) -> ChannelStatistics {
    let (values, tally) = collect_valid(data, excluded);
    statistics_from_tally(values, tally)
}

pub fn statistics_for_region(
    data: &Array2<f32>,
    shape: &RegionShape,
    excluded: Option<&Array2<u8>>,
) -> Result<ChannelStatistics> {
    shape.validate()?;
    let mv = shape.masked_values(data, excluded);
    if mv.n_inside == 0 {
        bail!("{} region covers no image pixels", shape.kind());
    }
    let mut tally = PixelTally { total: mv.n_inside, nan_count: mv.n_nan, padding: 0, excluded: mv.n_excluded };
    let mut values = Vec::with_capacity(mv.values.len());
    for v in mv.values {
        tally.take(v, &mut values);
    }
    Ok(statistics_from_tally(values, tally))
}

pub fn data_range(data: &Array2<f32>) -> (f64, f64) {
    let contiguous: Vec<f32>;
    let values: &[f32] = match data.as_slice() {
        Some(s) => s,
        None => {
            contiguous = data.iter().copied().collect();
            &contiguous
        }
    };
    let (lo, hi) = reduce_values(
        values,
        (f64::INFINITY, f64::NEG_INFINITY),
        |(lo, hi), v| {
            if is_valid_pixel(v) {
                let vf = v as f64;
                (lo.min(vf), hi.max(vf))
            } else {
                (lo, hi)
            }
        },
        |(lo_a, hi_a), (lo_b, hi_b)| (lo_a.min(lo_b), hi_a.max(hi_b)),
    );
    if lo.is_finite() && hi.is_finite() {
        (lo, hi)
    } else {
        (f64::NAN, f64::NAN)
    }
}

pub fn evaluate_noise(data: &Array2<f32>) -> NoiseEvaluation {
    let estimate = k_sigma_noise(data, NOISE_CLIP_K, NOISE_MAX_ITERATIONS);
    NoiseEvaluation {
        sigma: estimate.sigma,
        fraction: estimate.fraction_used,
        iterations: estimate.iterations,
        method: NOISE_METHOD_K_SIGMA_MRS,
    }
}

fn masked_copy(data: &Array2<f32>, excluded: Option<&Array2<u8>>) -> Option<Array2<f32>> {
    let mask = excluded.filter(|m| m.dim() == data.dim())?;
    Some(Array2::from_shape_fn(data.dim(), |idx| if mask[idx] != 0 { f32::NAN } else { data[idx] }))
}

pub fn evaluate_noise_masked(data: &Array2<f32>, excluded: Option<&Array2<u8>>) -> NoiseEvaluation {
    match masked_copy(data, excluded) {
        Some(masked) => evaluate_noise(&masked),
        None => evaluate_noise(data),
    }
}

pub fn evaluate_noise_window(window: &Array2<f32>) -> RegionNoise {
    let finite_pixels = window.iter().filter(|v| v.is_finite()).count();
    if finite_pixels < MIN_NOISE_REGION_PIXELS {
        return RegionNoise::TooSmall { finite_pixels };
    }
    RegionNoise::Evaluated(evaluate_noise(window))
}

fn clipped_bounds(data_dim: (usize, usize), shape: &RegionShape) -> Option<(i64, i64, i64, i64)> {
    let (rows, cols) = data_dim;
    let b = shape.bounds();
    let x0 = b.x0.max(0);
    let y0 = b.y0.max(0);
    let x1 = b.x1.min(cols as i64 - 1);
    let y1 = b.y1.min(rows as i64 - 1);
    if x0 > x1 || y0 > y1 {
        None
    } else {
        Some((x0, y0, x1, y1))
    }
}

pub fn region_window(data: &Array2<f32>, shape: &RegionShape) -> Option<Array2<f32>> {
    let (x0, y0, x1, y1) = clipped_bounds(data.dim(), shape)?;
    let window = Array2::from_shape_fn(((y1 - y0 + 1) as usize, (x1 - x0 + 1) as usize), |(wy, wx)| {
        let px = x0 + wx as i64;
        let py = y0 + wy as i64;
        if shape.contains(px as f64, py as f64) {
            data[[py as usize, px as usize]]
        } else {
            f32::NAN
        }
    });
    Some(window)
}

fn region_window_mask(mask: &Array2<u8>, shape: &RegionShape) -> Option<Array2<u8>> {
    let (x0, y0, x1, y1) = clipped_bounds(mask.dim(), shape)?;
    Some(Array2::from_shape_fn(((y1 - y0 + 1) as usize, (x1 - x0 + 1) as usize), |(wy, wx)| {
        mask[[(y0 + wy as i64) as usize, (x0 + wx as i64) as usize]]
    }))
}

pub fn evaluate_noise_in_region(
    data: &Array2<f32>,
    shape: &RegionShape,
    excluded: Option<&Array2<u8>>,
) -> Result<RegionNoise> {
    shape.validate()?;
    let window = match region_window(data, shape) {
        Some(w) => w,
        None => bail!("{} region covers no image pixels", shape.kind()),
    };
    let mask_window = excluded
        .filter(|m| m.dim() == data.dim())
        .and_then(|m| region_window_mask(m, shape));
    Ok(match masked_copy(&window, mask_window.as_ref()) {
        Some(masked) => evaluate_noise_window(&masked),
        None => evaluate_noise_window(&window),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::constants::MAD_TO_SIGMA;

    fn arr(rows: usize, cols: usize, values: Vec<f32>) -> Array2<f32> {
        Array2::from_shape_vec((rows, cols), values).unwrap()
    }

    fn gaussian_noise_image(rows: usize, cols: usize, sigma: f64, seed: u64) -> Array2<f32> {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};
        let mut rng = StdRng::seed_from_u64(seed);
        Array2::from_shape_fn((rows, cols), |_| {
            let u1: f64 = rng.gen::<f64>().max(1e-30);
            let u2: f64 = rng.gen::<f64>();
            (sigma * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
        })
    }

    fn hand_biweight_midvariance(values: &[f64], c: f64) -> f64 {
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let n = sorted.len();
        let median = if n % 2 == 0 {
            (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
        } else {
            sorted[n / 2]
        };
        let mut devs: Vec<f64> = values.iter().map(|v| (v - median).abs()).collect();
        devs.sort_by(|a, b| a.total_cmp(b));
        let mad = if n % 2 == 0 {
            (devs[n / 2 - 1] + devs[n / 2]) / 2.0
        } else {
            devs[n / 2]
        };
        let mut num = 0.0;
        let mut den = 0.0;
        for &v in values {
            let u = (v - median) / (c * mad);
            if u.abs() < 1.0 {
                let w = 1.0 - u * u;
                num += (v - median).powi(2) * w.powi(4);
                den += w * (1.0 - 5.0 * u * u);
            }
        }
        n as f64 * num / (den * den)
    }

    #[test]
    fn exact_statistics_on_hand_built_vector_includes_tiny_and_ignores_nan() {
        let data = arr(2, 3, vec![1.0, 2.0, 3.0, 4.0, 100.0, f32::NAN]);
        let s = exact_statistics(&data, None);
        assert_eq!(s.count, 5);
        assert_eq!(s.total, 6);
        assert_eq!(s.nan_count, 1);
        assert_eq!(s.excluded, 0);
        assert!((s.fraction - 5.0 / 6.0).abs() < 1e-12);
        assert_eq!(s.median, 3.0);
        assert_eq!(s.mad, 1.0);
        assert!((s.avg_dev - (2.0 + 1.0 + 0.0 + 1.0 + 97.0) / 5.0).abs() < 1e-9);
        assert_eq!(s.min, 1.0);
        assert_eq!(s.max, 100.0);
        assert!((s.sum - 110.0).abs() < 1e-9);
        assert!((s.mean - 22.0).abs() < 1e-9);
        let expected_var = [1.0f64, 2.0, 3.0, 4.0, 100.0]
            .iter()
            .map(|v| (v - 22.0).powi(2))
            .sum::<f64>()
            / 4.0;
        assert!((s.variance - expected_var).abs() < 1e-6, "variance {}", s.variance);
        assert!((s.std_dev - expected_var.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn exact_statistics_counts_exact_zero_as_padding_and_keeps_tiny_and_negative_values() {
        let data = arr(1, 5, vec![-3.0, 0.0, 1e-9, 2.0, 5.0]);
        let s = exact_statistics(&data, None);
        assert_eq!(s.count, 4);
        assert_eq!(s.padding, 1);
        assert_eq!(s.nan_count, 0);
        assert_eq!(s.total, 5);
        assert!((s.fraction - 0.8).abs() < 1e-12);
        assert_eq!(s.min, -3.0);
        assert_eq!(s.max, 5.0);
        let tiny = 1e-9f32 as f64;
        assert!((s.median - (tiny + 2.0) / 2.0).abs() < 1e-12, "median {}", s.median);
        assert!((s.mean - (-3.0 + tiny + 2.0 + 5.0) / 4.0).abs() < 1e-12, "mean {}", s.mean);
    }

    #[test]
    fn exact_statistics_partitions_every_pixel_into_data_nan_padding_or_excluded() {
        let data = arr(2, 4, vec![0.0, 0.0, f32::NAN, 4.0, -1.0, 0.0, f32::INFINITY, 6.0]);
        let mut mask = Array2::<u8>::zeros((2, 4));
        mask[[0, 1]] = 1;
        mask[[1, 3]] = 1;
        let s = exact_statistics(&data, Some(&mask));
        assert_eq!(s.count, 2);
        assert_eq!(s.padding, 2);
        assert_eq!(s.nan_count, 2);
        assert_eq!(s.excluded, 2);
        assert_eq!(s.count + s.padding + s.nan_count + s.excluded, s.total);
        assert_eq!(s.min, -1.0);
        assert_eq!(s.max, 4.0);
        assert_eq!(s.median, 1.5);
    }

    #[test]
    fn exact_statistics_median_matches_the_histogram_path_on_a_zero_padded_frame() {
        let mut data = Array2::from_shape_fn((20, 20), |(y, x)| (y * 20 + x) as f32 - 150.0 + 0.5);
        for y in 0..20 {
            for x in 0..6 {
                data[[y, x]] = 0.0;
            }
        }
        let exact = exact_statistics(&data, None);
        let histogram = crate::core::imaging::stats::compute_image_stats(&data);
        assert_eq!(exact.padding, 120);
        assert_eq!(exact.count, histogram.valid_count);
        assert!((exact.median - histogram.median).abs() < 1e-6, "{} vs {}", exact.median, histogram.median);
        assert_eq!(exact.min, histogram.min);
        assert_eq!(exact.max, histogram.max);
    }

    #[test]
    fn exact_statistics_honours_the_exclusion_mask() {
        let data = arr(2, 2, vec![1.0, 2.0, 3.0, 1000.0]);
        let mut mask = Array2::<u8>::zeros((2, 2));
        mask[[1, 1]] = 1;
        let s = exact_statistics(&data, Some(&mask));
        assert_eq!(s.count, 3);
        assert_eq!(s.excluded, 1);
        assert_eq!(s.total, 4);
        assert_eq!(s.max, 3.0);
        assert_eq!(s.median, 2.0);
        assert!((s.mean - 2.0).abs() < 1e-12);
    }

    #[test]
    fn exact_statistics_of_an_all_nan_array_is_empty() {
        let data = arr(1, 3, vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY]);
        let s = exact_statistics(&data, None);
        assert_eq!(s.count, 0);
        assert_eq!(s.nan_count, 3);
        assert_eq!(s.fraction, 0.0);
        assert!(s.mean.is_finite());
    }

    #[test]
    fn biweight_midvariance_matches_hand_formula_and_rejects_the_outlier() {
        let values: Vec<f64> = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 100.0];
        let data = arr(1, 11, values.iter().map(|&v| v as f32).collect());
        let s = exact_statistics(&data, None);
        let expected = hand_biweight_midvariance(&values, 9.0).sqrt();
        assert!((s.bwmv_sqrt - expected).abs() < 1e-6, "bwmv_sqrt {} expected {}", s.bwmv_sqrt, expected);
        let inlier_std = (1..=10).map(|v| (v as f64 - 5.5).powi(2)).sum::<f64>() / 9.0;
        let inlier_std = inlier_std.sqrt();
        assert!((s.bwmv_sqrt - inlier_std).abs() / inlier_std < 0.1, "bwmv {} inlier std {}", s.bwmv_sqrt, inlier_std);
        assert!(s.bwmv_sqrt < s.std_dev / 5.0, "bwmv {} std {}", s.bwmv_sqrt, s.std_dev);
        assert!(s.mad * MAD_TO_SIGMA > s.bwmv_sqrt, "uniform inliers make the MAD estimator overshoot: {} vs {}", s.mad * MAD_TO_SIGMA, s.bwmv_sqrt);
    }

    #[test]
    fn biweight_midvariance_tracks_gaussian_sigma_between_mad_sigma_and_std_with_outliers() {
        let mut image = gaussian_noise_image(200, 200, 2.0, 21);
        for i in 0..200 {
            image[[i, (i * 7) % 200]] = 1000.0;
        }
        let s = exact_statistics(&image, None);
        assert!((s.bwmv_sqrt - 2.0).abs() / 2.0 < 0.03, "bwmv {}", s.bwmv_sqrt);
        assert!((s.mad * MAD_TO_SIGMA - 2.0).abs() / 2.0 < 0.03, "mad sigma {}", s.mad * MAD_TO_SIGMA);
        assert!(s.std_dev > 10.0, "std {}", s.std_dev);
        assert!(s.bwmv_sqrt < s.std_dev);
    }

    #[test]
    fn biweight_midvariance_is_zero_when_mad_is_zero() {
        let data = arr(1, 5, vec![4.0, 4.0, 4.0, 4.0, 9.0]);
        let s = exact_statistics(&data, None);
        assert_eq!(s.count, 5);
        assert_eq!(s.median, 4.0);
        assert_eq!(s.mad, 0.0);
        assert_eq!(s.bwmv_sqrt, 0.0);
        assert!(s.std_dev > 0.0);
    }

    #[test]
    fn exact_statistics_matches_on_a_large_array_reduced_in_parallel() {
        let n = 600_000usize;
        let values: Vec<f32> = (0..n).map(|i| ((i * 7919) % 1000) as f32 - 500.0).collect();
        let data = arr(600, 1000, values.clone());
        let s = exact_statistics(&data, None);
        let valid: Vec<f32> = values.iter().copied().filter(|&v| v != 0.0).collect();
        let m = valid.len();
        let sum: f64 = valid.iter().map(|&v| v as f64).sum();
        assert_eq!(s.count, m as u64);
        assert_eq!(s.padding, (n - m) as u64);
        assert!((s.sum - sum).abs() < 1e-3);
        assert_eq!(s.min, -500.0);
        assert_eq!(s.max, 499.0);
        let mut sorted = valid.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let median = (sorted[m / 2 - 1] as f64 + sorted[m / 2] as f64) / 2.0;
        assert_eq!(s.median, median);
    }

    #[test]
    fn statistics_for_region_on_a_circle_matches_masked_values() {
        let data = Array2::from_shape_fn((20, 20), |(y, x)| (y * 20 + x) as f32);
        let shape = RegionShape::Circle { x: 10.0, y: 10.0, r: 3.0 };
        let mv = shape.masked_values(&data, None);
        let s = statistics_for_region(&data, &shape, None).unwrap();
        assert_eq!(s.count, mv.values.len() as u64);
        assert_eq!(s.total, mv.n_inside);
        assert_eq!(s.count, 29);
        let expected_sum: f64 = mv.values.iter().map(|&v| v as f64).sum();
        assert!((s.sum - expected_sum).abs() < 1e-6);
        assert_eq!(s.min, *mv.values.iter().min_by(|a, b| a.total_cmp(b)).unwrap() as f64);
        assert_eq!(s.median, 210.0);
    }

    #[test]
    fn statistics_for_region_rejects_off_image_and_invalid_shapes() {
        let data = Array2::<f32>::from_elem((8, 8), 1.0);
        assert!(statistics_for_region(&data, &RegionShape::Circle { x: 50.0, y: 50.0, r: 2.0 }, None).is_err());
        assert!(statistics_for_region(&data, &RegionShape::Circle { x: 4.0, y: 4.0, r: -1.0 }, None).is_err());
        let ok = statistics_for_region(&data, &RegionShape::Circle { x: 4.0, y: 4.0, r: 1.0 }, None).unwrap();
        assert_eq!(ok.count, 5);
    }

    #[test]
    fn statistics_for_region_counts_exact_zero_as_padding() {
        let mut data = Array2::from_shape_fn((12, 12), |(y, x)| (y * 12 + x) as f32 - 80.0);
        data[[4, 4]] = 0.0;
        data[[5, 4]] = 0.0;
        data[[5, 5]] = f32::NAN;
        let shape = RegionShape::Box { x: 5.0, y: 5.0, width: 3.0, height: 3.0, angle: 0.0 };
        let s = statistics_for_region(&data, &shape, None).unwrap();
        assert_eq!(s.total, 9);
        assert_eq!(s.nan_count, 1);
        assert_eq!(s.padding, 2);
        assert_eq!(s.count, 6);
        assert_eq!(s.count + s.padding + s.nan_count + s.excluded, s.total);
        assert_eq!(s.min, -27.0);
        assert_eq!(s.max, -2.0);

        let blank = Array2::<f32>::zeros((8, 8));
        let empty = statistics_for_region(&blank, &RegionShape::Circle { x: 4.0, y: 4.0, r: 1.0 }, None).unwrap();
        assert_eq!(empty.count, 0);
        assert_eq!(empty.padding, 5);
        assert_eq!(empty.fraction, 0.0);
    }

    #[test]
    fn statistics_for_region_counts_excluded_pixels() {
        let data = Array2::from_elem((8, 8), 2.0f32);
        let mut mask = Array2::<u8>::zeros((8, 8));
        mask[[4, 4]] = 1;
        let shape = RegionShape::Box { x: 4.0, y: 4.0, width: 3.0, height: 3.0, angle: 0.0 };
        let s = statistics_for_region(&data, &shape, Some(&mask)).unwrap();
        assert_eq!(s.excluded, 1);
        assert_eq!(s.count + s.excluded, s.total);
    }

    #[test]
    fn data_range_ignores_non_finite_and_padding_and_keeps_negatives() {
        let data = arr(1, 5, vec![-2.5, f32::NAN, 0.0, 7.5, f32::INFINITY]);
        assert_eq!(data_range(&data), (-2.5, 7.5));
        let padded = arr(1, 5, vec![0.0, 3.0, 0.0, 7.5, -0.0]);
        assert_eq!(data_range(&padded), (3.0, 7.5));
        let negative = arr(1, 3, vec![-4.0, 0.0, -1.0]);
        assert_eq!(data_range(&negative), (-4.0, -1.0));
        for empty in [arr(1, 2, vec![f32::NAN, f32::NAN]), arr(1, 2, vec![0.0, 0.0])] {
            let (lo, hi) = data_range(&empty);
            assert!(lo.is_nan() && hi.is_nan());
        }
    }

    #[test]
    fn evaluate_noise_recovers_gaussian_sigma_within_five_percent() {
        let image = gaussian_noise_image(256, 256, 3.0, 11);
        let n = evaluate_noise(&image);
        assert!((n.sigma - 3.0).abs() / 3.0 < 0.05, "sigma {}", n.sigma);
        assert!(n.fraction > 0.9);
        assert!(n.iterations >= 1);
        assert_eq!(n.method, "k-sigma-mrs");
    }

    #[test]
    fn evaluate_noise_masked_ignores_excluded_hot_pixels() {
        let mut image = gaussian_noise_image(128, 128, 1.0, 5);
        let mut mask = Array2::<u8>::zeros((128, 128));
        for i in 0..128 {
            image[[i, i]] = 5000.0;
            mask[[i, i]] = 1;
        }
        let n = evaluate_noise_masked(&image, Some(&mask));
        assert!((n.sigma - 1.0).abs() < 0.1, "sigma {}", n.sigma);
    }

    #[test]
    fn region_window_keeps_only_pixels_inside_the_shape() {
        let data = Array2::from_elem((10, 10), 1.0f32);
        let shape = RegionShape::Circle { x: 5.0, y: 5.0, r: 2.0 };
        let window = region_window(&data, &shape).unwrap();
        let inside = window.iter().filter(|v| v.is_finite()).count() as u64;
        assert_eq!(inside, shape.masked_values(&data, None).n_inside);
        assert!(window.dim().0 <= 6 && window.dim().1 <= 6, "dim {:?}", window.dim());
        assert!(region_window(&data, &RegionShape::Circle { x: 50.0, y: 50.0, r: 2.0 }).is_none());
    }

    #[test]
    fn evaluate_noise_in_region_reads_the_local_noise_level() {
        let mut image = gaussian_noise_image(160, 160, 1.0, 3);
        let quiet = gaussian_noise_image(160, 160, 0.2, 4);
        for y in 0..80 {
            for x in 0..160 {
                image[[y, x]] = quiet[[y, x]];
            }
        }
        let top = RegionShape::Box { x: 80.0, y: 30.0, width: 120.0, height: 40.0, angle: 0.0 };
        let bottom = RegionShape::Box { x: 80.0, y: 120.0, width: 120.0, height: 40.0, angle: 0.0 };
        let n_top = evaluate_noise_in_region(&image, &top, None).unwrap();
        let n_bottom = evaluate_noise_in_region(&image, &bottom, None).unwrap();
        let n_top = n_top.evaluation().expect("top region is large enough");
        let n_bottom = n_bottom.evaluation().expect("bottom region is large enough");
        assert!((n_top.sigma - 0.2).abs() / 0.2 < 0.1, "top sigma {}", n_top.sigma);
        assert!((n_bottom.sigma - 1.0).abs() < 0.1, "bottom sigma {}", n_bottom.sigma);
    }

    #[test]
    fn evaluate_noise_in_region_declines_windows_with_fewer_than_sixty_four_finite_pixels() {
        let image = gaussian_noise_image(64, 64, 1.0, 9);
        let circle = RegionShape::Circle { x: 20.0, y: 20.0, r: 2.0 };
        assert_eq!(
            evaluate_noise_in_region(&image, &circle, None).unwrap(),
            RegionNoise::TooSmall { finite_pixels: 13 }
        );
        let line = RegionShape::Line { x1: 2.0, y1: 2.0, x2: 40.0, y2: 30.0 };
        assert_eq!(
            evaluate_noise_in_region(&image, &line, None).unwrap(),
            RegionNoise::TooSmall { finite_pixels: 0 }
        );
        let point = RegionShape::Point { x: 5.0, y: 5.0 };
        let too_small = evaluate_noise_in_region(&image, &point, None).unwrap();
        assert!(too_small.evaluation().is_none());
        assert_eq!(
            too_small.note().as_deref(),
            Some("region too small for noise evaluation (0 finite pixels, 64 needed)")
        );

        let square = RegionShape::Box { x: 20.5, y: 20.5, width: 8.0, height: 8.0, angle: 0.0 };
        let evaluated = evaluate_noise_in_region(&image, &square, None).unwrap();
        assert!(evaluated.note().is_none());
        assert_eq!(evaluated.evaluation().unwrap().method, "k-sigma-mrs");

        let mut mask = Array2::<u8>::zeros((64, 64));
        mask[[20, 20]] = 1;
        assert_eq!(
            evaluate_noise_in_region(&image, &square, Some(&mask)).unwrap(),
            RegionNoise::TooSmall { finite_pixels: 63 }
        );
    }
}
