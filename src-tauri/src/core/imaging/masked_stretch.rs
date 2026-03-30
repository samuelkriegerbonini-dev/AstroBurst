use ndarray::Array2;
use rayon::prelude::*;

use crate::core::imaging::star_mask::{generate_star_mask, StarMaskConfig, StarMaskResult};
use crate::core::imaging::stats::compute_image_stats;

#[derive(Debug, Clone)]
pub struct MaskedStretchConfig {
    pub iterations: usize,
    pub target_background: f64,
    pub mask_growth: f64,
    pub mask_softness: f64,
    pub luminance_protect: bool,
    pub luminance_ceiling: f64,
    pub protection_amount: f64,
    pub convergence_threshold: f64,
}

impl Default for MaskedStretchConfig {
    fn default() -> Self {
        Self {
            iterations: 10,
            target_background: 0.25,
            mask_growth: 2.5,
            mask_softness: 4.0,
            luminance_protect: true,
            luminance_ceiling: 0.85,
            protection_amount: 0.85,
            convergence_threshold: 1e-5,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MaskedStretchResult {
    pub image: Array2<f32>,
    pub iterations_run: usize,
    pub final_background: f64,
    pub stars_masked: usize,
    pub mask_coverage: f64,
    pub converged: bool,
}

pub fn masked_stretch(
    image: &Array2<f32>,
    config: &MaskedStretchConfig,
) -> Result<MaskedStretchResult, String> {
    let mask_config = StarMaskConfig {
        growth_factor: config.mask_growth,
        softness: config.mask_softness,
        luminance_protect: config.luminance_protect,
        luminance_ceiling: config.luminance_ceiling,
        ..StarMaskConfig::default()
    };

    let mask_result = generate_star_mask(image, &mask_config)?;
    masked_stretch_with_mask(image, &mask_result, config)
}

pub fn masked_stretch_with_mask(
    image: &Array2<f32>,
    mask_result: &StarMaskResult,
    config: &MaskedStretchConfig,
) -> Result<MaskedStretchResult, String> {
    let mut working = normalize_to_01(image);
    let protection = config.protection_amount as f32;
    let mask = &mask_result.mask;
    let target_bg = config.target_background;

    let mut prev_bg = compute_masked_median(&working, mask);
    let mut iterations_run = 0;
    let mut converged = false;

    for _iter in 0..config.iterations {
        let bg = compute_masked_median(&working, mask);

        if (bg - target_bg).abs() < config.convergence_threshold {
            converged = true;
            break;
        }

        let midtone = mtf_balance(bg, target_bg);

        let unmasked = apply_mtf(&working, midtone as f32);

        let (h, w) = working.dim();
        let work_slice = working.as_slice_mut().unwrap();
        let unmask_slice = unmasked.as_slice().unwrap();
        let mask_slice = mask.as_slice().unwrap();

        work_slice
            .par_iter_mut()
            .zip(unmask_slice.par_iter())
            .zip(mask_slice.par_iter())
            .for_each(|((dst, &stretched), &m)| {
                let blend = m * protection;
                *dst = *dst * blend + stretched * (1.0 - blend);
            });

        let new_bg = compute_masked_median(&working, mask);
        if (new_bg - prev_bg).abs() < config.convergence_threshold * 0.1 {
            converged = true;
            iterations_run = _iter + 1;
            break;
        }
        prev_bg = new_bg;
        iterations_run = _iter + 1;
    }

    let final_bg = compute_masked_median(&working, mask);

    clamp_inplace(&mut working);

    Ok(MaskedStretchResult {
        image: working,
        iterations_run,
        final_background: final_bg,
        stars_masked: mask_result.stars_masked,
        mask_coverage: mask_result.coverage_fraction,
        converged,
    })
}

fn normalize_to_01(image: &Array2<f32>) -> Array2<f32> {
    let stats = compute_image_stats(image);
    let range = (stats.max - stats.min) as f32;
    if range < 1e-10 {
        return Array2::zeros(image.dim());
    }
    let dmin = stats.min as f32;
    let inv = 1.0 / range;
    let mut out = image.clone();
    out.par_mapv_inplace(|v| {
        if !v.is_finite() || v <= 0.0 {
            0.0
        } else {
            ((v - dmin) * inv).clamp(0.0, 1.0)
        }
    });
    out
}

fn compute_masked_median(image: &Array2<f32>, mask: &Array2<f32>) -> f64 {
    let img_slice = image.as_slice().unwrap();
    let mask_slice = mask.as_slice().unwrap();

    let mut bg_vals: Vec<f32> = img_slice
        .iter()
        .zip(mask_slice.iter())
        .filter_map(|(&v, &m)| {
            if m < 0.5 && v.is_finite() && v > 0.0 {
                Some(v)
            } else {
                None
            }
        })
        .collect();

    if bg_vals.is_empty() {
        return 0.0;
    }

    let mid = bg_vals.len() / 2;
    bg_vals.select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    bg_vals[mid] as f64
}

fn mtf_balance(median: f64, target: f64) -> f64 {
    let denom = 2.0 * target * median - target - median;
    if denom.abs() < 1e-15 {
        return 0.5;
    }
    (median * (target - 1.0) / denom).clamp(0.0001, 0.9999)
}

fn apply_mtf(data: &Array2<f32>, m: f32) -> Array2<f32> {
    let mut out = data.clone();
    out.par_mapv_inplace(|x| {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        (m - 1.0) * x / ((2.0 * m - 1.0) * x - m)
    });
    out
}

fn clamp_inplace(data: &mut Array2<f32>) {
    data.par_mapv_inplace(|v| v.clamp(0.0, 1.0));
}
