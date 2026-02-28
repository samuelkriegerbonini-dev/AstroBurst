use ndarray::Array2;
use rayon::prelude::*;

const PADDING_THRESHOLD: f32 = 1e-7;
const MAD_TO_SIGMA: f64 = 1.4826;
const HISTOGRAM_BINS: usize = 65536;

#[inline(always)]
pub fn is_valid_pixel(v: f32) -> bool {
    v.is_finite() && v > PADDING_THRESHOLD
}

#[derive(Debug, Clone)]
pub struct ImageStats {
    pub min: f64,
    pub max: f64,
    pub median: f64,
    pub mad: f64,
    pub sigma: f64,
    pub mean: f64,
    pub valid_count: u64,
}

impl Default for ImageStats {
    fn default() -> Self {
        Self {
            min: 0.0,
            max: 0.0,
            median: 0.0,
            mad: 0.0,
            sigma: 0.0,
            mean: 0.0,
            valid_count: 0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Histogram {
    pub bins: Vec<u32>,
    pub bin_count: usize,
    pub data_min: f64,
    pub data_max: f64,
    pub bin_width: f64,
    pub total_pixels: u64,
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

    ImageStats {
        min: acc.min,
        max: acc.max,
        median,
        mad,
        sigma,
        mean: acc.sum / n as f64,
        valid_count: n,
    }
}

pub fn compute_histogram(data: &Array2<f32>, stats: &ImageStats) -> Histogram {
    if stats.valid_count == 0 {
        return Histogram {
            bins: vec![0u32; HISTOGRAM_BINS],
            bin_count: HISTOGRAM_BINS,
            data_min: 0.0,
            data_max: 1.0,
            bin_width: 1.0 / HISTOGRAM_BINS as f64,
            total_pixels: 0,
        };
    }

    let dmin = stats.min;
    let dmax = stats.max;
    let range = (dmax - dmin).max(1e-30);
    let inv_range = (HISTOGRAM_BINS - 1) as f64 / range;
    let bin_width = range / HISTOGRAM_BINS as f64;

    let slice = data.as_slice().expect("Array2 must be contiguous");

    let chunk_size = (slice.len() / rayon::current_num_threads().max(1)).max(4096);

    let bins = slice
        .par_chunks(chunk_size)
        .fold(
            || vec![0u32; HISTOGRAM_BINS],
            |mut local_bins, chunk| {
                for &v in chunk {
                    if is_valid_pixel(v) {
                        let idx = ((v as f64 - dmin) * inv_range) as usize;
                        let idx = idx.min(HISTOGRAM_BINS - 1);
                        local_bins[idx] += 1;
                    }
                }
                local_bins
            },
        )
        .reduce(
            || vec![0u32; HISTOGRAM_BINS],
            |mut a, b| {
                for (ai, bi) in a.iter_mut().zip(b.iter()) {
                    *ai += bi;
                }
                a
            },
        );

    Histogram {
        bins,
        bin_count: HISTOGRAM_BINS,
        data_min: dmin,
        data_max: dmax,
        bin_width,
        total_pixels: stats.valid_count,
    }
}

pub fn downsample_histogram(hist: &Histogram, target_bins: usize) -> Vec<u32> {
    let ratio = hist.bin_count as f64 / target_bins as f64;
    let mut out = vec![0u32; target_bins];
    for (i, &b) in hist.bins.iter().enumerate() {
        let ti = ((i as f64 / ratio) as usize).min(target_bins - 1);
        out[ti] = out[ti].saturating_add(b);
    }
    out
}

pub fn sigma_clipped_stats(values: &mut Vec<f32>, kappa: f32, iterations: usize) -> (f64, f64) {
    for _ in 0..iterations {
        if values.len() < 3 {
            break;
        }

        let median = exact_median_mut(values);

        let mut devs: Vec<f32> = values.iter().map(|&v| (v as f64 - median).abs() as f32).collect();
        let dev_mid = devs.len() / 2;
        devs.select_nth_unstable_by(dev_mid, |a, b| a.partial_cmp(b).unwrap());
        let mad = devs[dev_mid] as f64;
        let sig = (mad * MAD_TO_SIGMA).max(1e-30);

        let lo = (median - kappa as f64 * sig) as f32;
        let hi = (median + kappa as f64 * sig) as f32;
        values.retain(|&v| v >= lo && v <= hi);
    }

    if values.is_empty() {
        return (0.0, 1.0);
    }

    let median = exact_median_mut(values);
    let mut devs: Vec<f32> = values.iter().map(|&v| (v as f64 - median).abs() as f32).collect();
    let dev_mid = devs.len() / 2;
    devs.select_nth_unstable_by(dev_mid, |a, b| a.partial_cmp(b).unwrap());
    let sigma = (devs[dev_mid] as f64 * MAD_TO_SIGMA).max(1e-30);

    (median, sigma)
}

fn exact_median_mut(data: &mut [f32]) -> f64 {
    let n = data.len();
    if n == 0 {
        return 0.0;
    }
    let mid = n / 2;
    data.select_nth_unstable_by(mid, |a, b| {
        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
    });
    if n % 2 == 0 {
        let right = data[mid] as f64;
        let left = data[..mid]
            .iter()
            .copied()
            .fold(f32::MIN, f32::max) as f64;
        (left + right) / 2.0
    } else {
        data[mid] as f64
    }
}

fn exact_median_f64(data: &[f64]) -> f64 {
    let n = data.len();
    if n == 0 {
        return 0.0;
    }
    let mut buf: Vec<f64> = data.to_vec();
    let mid = n / 2;
    buf.select_nth_unstable_by(mid, |a, b| {
        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
    });
    if n % 2 == 0 {
        let right = buf[mid];
        let left = buf[..mid]
            .iter()
            .copied()
            .fold(f64::MIN, f64::max);
        (left + right) / 2.0
    } else {
        buf[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_pixel() {
        assert!(!is_valid_pixel(0.0));
        assert!(!is_valid_pixel(f32::NAN));
        assert!(!is_valid_pixel(f32::INFINITY));
        assert!(!is_valid_pixel(f32::NEG_INFINITY));
        assert!(!is_valid_pixel(1e-8));
        assert!(is_valid_pixel(0.001));
        assert!(is_valid_pixel(42.0));
    }

    #[test]
    fn test_padding_filtered_out() {
        let mut data_vec = vec![0.0f32; 10000];
        for i in 0..100 {
            data_vec[4950 + i] = (i + 1) as f32 * 0.1;
        }
        let data = Array2::from_shape_vec((100, 100), data_vec).unwrap();
        let stats = compute_image_stats(&data);

        assert_eq!(stats.valid_count, 100);
        assert!(stats.min > 0.0);
    }

    #[test]
    fn test_exact_median_odd() {
        let mut vals = vec![5.0f32, 1.0, 3.0, 2.0, 4.0];
        let m = exact_median_mut(&mut vals);
        assert!((m - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_exact_median_even() {
        let mut vals = vec![1.0f32, 2.0, 3.0, 4.0];
        let m = exact_median_mut(&mut vals);
        assert!((m - 2.5).abs() < 1e-6);
    }

    #[test]
    fn test_mad_known() {
        let data = Array2::from_shape_vec(
            (1, 7),
            vec![1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0],
        )
        .unwrap();
        let stats = compute_image_stats(&data);
        assert!((stats.median - 4.0).abs() < 1e-6);
        assert!((stats.mad - 2.0).abs() < 1e-6);
    }

    #[test]
    fn test_histogram_total_matches() {
        let data = Array2::from_shape_fn((100, 100), |(r, c)| {
            (r * 100 + c) as f32 + 1.0
        });
        let stats = compute_image_stats(&data);
        let hist = compute_histogram(&data, &stats);
        let total: u64 = hist.bins.iter().map(|&b| b as u64).sum();
        assert_eq!(total, stats.valid_count);
    }

    #[test]
    fn test_sigma_clipped_with_outliers() {
        let mut vals: Vec<f32> = (1..=100).map(|i| i as f32).collect();
        vals.push(100_000.0);
        let (med, sig) = sigma_clipped_stats(&mut vals, 3.0, 3);
        assert!(med > 40.0 && med < 60.0);
        assert!(sig < 500.0);
    }

    #[test]
    fn test_empty_data() {
        let data = Array2::from_shape_vec((2, 2), vec![0.0f32; 4]).unwrap();
        let stats = compute_image_stats(&data);
        assert_eq!(stats.valid_count, 0);
        assert_eq!(stats.median, 0.0);
    }
}
