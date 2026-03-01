use ndarray::Array2;
use rayon::prelude::*;

use crate::domain::stats::{self, ImageStats, Histogram, is_valid_pixel};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct StfParams {
    pub shadow: f64,
    pub midtone: f64,
    pub highlight: f64,
}

impl Default for StfParams {
    fn default() -> Self {
        Self {
            shadow: 0.0,
            midtone: 0.25,
            highlight: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AutoStfConfig {
    pub target_bg: f64,
    pub shadow_k: f64,
}

impl Default for AutoStfConfig {
    fn default() -> Self {
        Self {
            target_bg: 0.25,
            shadow_k: -2.8,
        }
    }
}

pub fn analyze(data: &Array2<f32>) -> (ImageStats, Histogram) {
    let st = stats::compute_image_stats(data);
    let hist = stats::compute_histogram(data, &st);
    (st, hist)
}

pub fn auto_stf(stats: &ImageStats, config: &AutoStfConfig) -> StfParams {
    if stats.valid_count == 0 {
        return StfParams::default();
    }

    let range = (stats.max - stats.min).max(1e-30);
    let median_norm = (stats.median - stats.min) / range;
    let sigma_norm = stats.sigma / range;

    let shadow_norm = (median_norm + config.shadow_k * sigma_norm).max(0.0);
    let highlight_norm = 1.0f64;

    let clip_range = (highlight_norm - shadow_norm).max(1e-15);
    let m_clipped = ((median_norm - shadow_norm) / clip_range).clamp(0.0, 1.0);

    let midtone = if m_clipped <= 0.0 || m_clipped >= 1.0 {
        0.5
    } else {
        mtf_balance(m_clipped, config.target_bg)
    };

    StfParams {
        shadow: shadow_norm,
        midtone,
        highlight: highlight_norm,
    }
}

fn mtf_balance(m: f64, t: f64) -> f64 {
    let denom = 2.0 * t * m - t - m;
    if denom.abs() < 1e-15 {
        return 0.5;
    }
    let b = (m * (t - 1.0)) / denom;
    b.clamp(0.0001, 0.9999)
}

#[inline(always)]
fn mtf(x: f64, m: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    (m - 1.0) * x / ((2.0 * m - 1.0) * x - m)
}

pub fn apply_stf(data: &Array2<f32>, params: &StfParams, stats: &ImageStats) -> Vec<u8> {
    let slice = data.as_slice().expect("contiguous");

    let range = (stats.max - stats.min).max(1e-30);
    let inv_range = 1.0 / range;
    let dmin = stats.min;

    let shadow = params.shadow;
    let highlight = params.highlight;
    let clip_range = (highlight - shadow).max(1e-15);
    let m = params.midtone;

    slice
        .par_iter()
        .map(|&v| {
            if !is_valid_pixel(v) {
                return 0u8;
            }
            let norm = (v as f64 - dmin) * inv_range;
            let clipped = ((norm - shadow) / clip_range).clamp(0.0, 1.0);
            let stretched = mtf(clipped, m);
            (stretched * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

pub fn apply_stf_f32(data: &Array2<f32>, params: &StfParams, stats: &ImageStats) -> Array2<f32> {
    let (rows, cols) = data.dim();
    let slice = data.as_slice().expect("contiguous");

    let range = (stats.max - stats.min).max(1e-30);
    let inv_range = 1.0 / range;
    let dmin = stats.min;

    let shadow = params.shadow;
    let highlight = params.highlight;
    let clip_range = (highlight - shadow).max(1e-15);
    let m = params.midtone;

    let pixels: Vec<f32> = slice
        .par_iter()
        .map(|&v| {
            if !is_valid_pixel(v) {
                return 0.0f32;
            }
            let norm = (v as f64 - dmin) * inv_range;
            let clipped = ((norm - shadow) / clip_range).clamp(0.0, 1.0);
            mtf(clipped, m) as f32
        })
        .collect();

    Array2::from_shape_vec((rows, cols), pixels).unwrap()
}

pub fn downsample_histogram(hist: &Histogram, target_bins: usize) -> Vec<u32> {
    stats::downsample_histogram(hist, target_bins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mtf_identity() {
        let v = mtf(0.5, 0.5);
        assert!((v - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_mtf_boundaries() {
        assert!((mtf(0.0, 0.3) - 0.0).abs() < 1e-10);
        assert!((mtf(1.0, 0.3) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_auto_stf_clean_data() {
        let data = Array2::from_shape_vec(
            (100, 100),
            (1..=10000).map(|i| i as f32 / 10000.0).collect(),
        )
            .unwrap();
        let (st, _hist) = analyze(&data);
        let params = auto_stf(&st, &AutoStfConfig::default());
        assert!(params.shadow >= 0.0);
        assert!(params.highlight <= 1.0);
        assert!(params.midtone > 0.0 && params.midtone < 1.0);
    }

    #[test]
    fn test_auto_stf_with_padding() {
        let mut raw = vec![0.0f32; 10000];
        for i in 0..2500 {
            raw[3750 + i] = (i + 1) as f32 * 0.001;
        }
        let data = Array2::from_shape_vec((100, 100), raw).unwrap();

        let (st, _hist) = analyze(&data);

        assert_eq!(st.valid_count, 2500);
        assert!(st.min > 0.0);

        let params = auto_stf(&st, &AutoStfConfig::default());
        assert!(params.shadow >= 0.0);
        assert!(params.midtone > 0.0);
    }

    #[test]
    fn test_shadow_k_aggressiveness() {
        let data = Array2::from_shape_fn((100, 100), |(r, c)| {
            (r * 100 + c) as f32 * 0.001 + 0.01
        });
        let (st, _) = analyze(&data);

        let gentle = auto_stf(&st, &AutoStfConfig { target_bg: 0.25, shadow_k: -1.5 });
        let aggressive = auto_stf(&st, &AutoStfConfig { target_bg: 0.25, shadow_k: -4.0 });

        assert!(aggressive.shadow <= gentle.shadow);
    }

    #[test]
    fn test_apply_stf_range() {
        let data = Array2::from_shape_vec(
            (4, 4),
            (1..=16).map(|i| i as f32 * 100.0).collect(),
        )
            .unwrap();
        let (st, _) = analyze(&data);
        let params = StfParams { shadow: 0.0, midtone: 0.5, highlight: 1.0 };
        let buf = apply_stf(&data, &params, &st);
        assert_eq!(buf.len(), 16);
        assert_eq!(buf[0], 0);
        assert_eq!(buf[15], 255);
    }

    #[test]
    fn test_padding_pixels_rendered_black() {
        let mut raw = vec![0.0f32; 16];
        raw[8] = 0.5;
        raw[9] = 1.0;
        let data = Array2::from_shape_vec((4, 4), raw).unwrap();
        let (st, _) = analyze(&data);
        let params = StfParams { shadow: 0.0, midtone: 0.5, highlight: 1.0 };
        let buf = apply_stf(&data, &params, &st);
        for i in 0..8 {
            assert_eq!(buf[i], 0, "padding pixel {} should be black", i);
        }
    }
}