use ndarray::{Array2, Array3};
use rayon::prelude::*;

use crate::math::simd::collapse_mean_simd;
use crate::math::median::{f32_cmp};
use crate::types::header::HduHeader;

#[derive(Debug, Clone, serde::Serialize)]
pub struct CubeResult {
    pub dimensions: [usize; 3],
    pub collapsed_path: String,
    pub collapsed_median_path: String,
    pub frames_dir: String,
    pub frame_count: usize,
    pub center_spectrum: Vec<f32>,
    pub wavelengths: Option<Vec<f64>>,
}

pub fn collapse_mean(cube: &Array3<f32>) -> Array2<f32> {
    collapse_mean_simd(cube)
}

pub fn collapse_median(cube: &Array3<f32>) -> Array2<f32> {
    let (depth, rows, cols) = cube.dim();
    let npix = rows * cols;

    let result_data: Vec<f32> = (0..npix)
        .into_par_iter()
        .map(|i| {
            let y = i / cols;
            let x = i % cols;
            let mut vals: Vec<f32> = (0..depth)
                .map(|z| cube[[z, y, x]])
                .filter(|v| v.is_finite() && *v != 0.0)
                .collect();

            if vals.is_empty() {
                return 0.0;
            }

            let mid = vals.len() / 2;
            vals.select_nth_unstable_by(mid, |a, b| {
                f32_cmp(a, b)
            });
            vals[mid]
        })
        .collect();

    Array2::from_shape_vec((rows, cols), result_data).unwrap()
}

pub fn extract_spectrum(cube: &Array3<f32>, y: usize, x: usize) -> Vec<f32> {
    let depth = cube.dim().0;
    (0..depth).map(|z| cube[[z, y, x]]).collect()
}

pub fn build_wavelength_axis(header: &HduHeader) -> Option<Vec<f64>> {
    let crval3 = header.get_f64("CRVAL3")?;
    let cdelt3 = header.get_f64("CDELT3")?;
    let crpix3 = header.get_f64("CRPIX3").unwrap_or(1.0);
    let naxis3 = header.get_i64("NAXIS3")? as usize;

    let axis: Vec<f64> = (0..naxis3)
        .map(|i| crval3 + (i as f64 - crpix3 + 1.0) * cdelt3)
        .collect();

    Some(axis)
}

#[derive(Debug, Clone)]
pub struct GlobalCubeStats {
    pub median: f32,
    pub sigma: f32,
    pub low: f32,
    pub high: f32,
}

pub fn compute_global_stats(cube: &Array3<f32>) -> GlobalCubeStats {
    let mut finite: Vec<f32> = cube
        .iter()
        .filter(|v| v.is_finite() && **v != 0.0)
        .copied()
        .collect();

    if finite.is_empty() {
        return GlobalCubeStats {
            median: 0.0,
            sigma: 1.0,
            low: 0.0,
            high: 1.0,
        };
    }

    let n = finite.len();
    let mid = n / 2;
    finite.select_nth_unstable_by(mid, |a, b| {
        f32_cmp(a, b)
    });
    let median = finite[mid];

    let mut deviations: Vec<f32> = finite.iter().map(|v| (v - median).abs()).collect();
    let dev_mid = deviations.len() / 2;
    deviations.select_nth_unstable_by(dev_mid, |a, b| {
        f32_cmp(a, b)
    });
    let sigma = (deviations[dev_mid] * 1.4826).max(1e-10);

    finite.sort_unstable_by(|a, b| f32_cmp(a, b));
    let low = finite[(n as f64 * 0.01) as usize];
    let high = finite[((n as f64 * 0.999) as usize).min(n - 1)];

    GlobalCubeStats {
        median,
        sigma,
        low,
        high,
    }
}

pub fn normalize_with_global(data: &Array2<f32>, g: &GlobalCubeStats) -> Array2<f32> {
    let alpha: f32 = 10.0;
    let inv_sigma_alpha = alpha / g.sigma;

    data.mapv(|v| {
        if !v.is_finite() {
            return 0.0;
        }
        let clamped = v.clamp(g.low, g.high);
        let scaled = inv_sigma_alpha * (clamped - g.median);
        scaled.asinh()
    })
}
