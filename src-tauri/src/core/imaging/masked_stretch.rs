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

    for iter_idx in 0..config.iterations {
        iterations_run = iter_idx + 1;

        let bg = compute_masked_median(&working, mask);

        let at_target = (bg - target_bg).abs() < config.convergence_threshold;
        let stagnated = iter_idx > 0
            && (bg - prev_bg).abs() < config.convergence_threshold * 0.1;

        if at_target {
            converged = true;
            break;
        }

        if stagnated {
            break;
        }

        let midtone = mtf_balance(bg, target_bg);
        let unmasked = apply_mtf(&working, midtone as f32);

        ndarray::Zip::from(&mut working)
            .and(&unmasked)
            .and(mask)
            .par_for_each(|dst, &stretched, &m| {
                let blend = m * protection;
                *dst = *dst * blend + stretched * (1.0 - blend);
            });

        prev_bg = bg;
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

pub struct MaskedStretchRgbResult {
    pub r: MaskedStretchResult,
    pub g: MaskedStretchResult,
    pub b: MaskedStretchResult,
    pub shared_mask_coverage: f64,
    pub shared_stars_masked: usize,
}

fn compute_luminance(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
) -> Result<Array2<f32>, String> {
    let dim = r.dim();
    if g.dim() != dim || b.dim() != dim {
        return Err(format!(
            "Channel dimension mismatch: R={:?} G={:?} B={:?}",
            dim,
            g.dim(),
            b.dim()
        ));
    }

    let mut out = Array2::zeros(dim);
    ndarray::Zip::from(&mut out)
        .and(r)
        .and(g)
        .and(b)
        .par_for_each(|o, &rv, &gv, &bv| {
            let rn = if rv.is_finite() { rv } else { 0.0 };
            let gn = if gv.is_finite() { gv } else { 0.0 };
            let bn = if bv.is_finite() { bv } else { 0.0 };
            *o = 0.2126 * rn + 0.7152 * gn + 0.0722 * bn;
        });
    Ok(out)
}

pub fn masked_stretch_rgb_shared(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    config: &MaskedStretchConfig,
) -> Result<MaskedStretchRgbResult, String> {
    let luminance = compute_luminance(r, g, b)?;

    let mask_config = StarMaskConfig {
        growth_factor: config.mask_growth,
        softness: config.mask_softness,
        luminance_protect: config.luminance_protect,
        luminance_ceiling: config.luminance_ceiling,
        ..StarMaskConfig::default()
    };

    let shared_mask = generate_star_mask(&luminance, &mask_config)?;

    let (res_r, (res_g, res_b)) = rayon::join(
        || masked_stretch_with_mask(r, &shared_mask, config),
        || rayon::join(
            || masked_stretch_with_mask(g, &shared_mask, config),
            || masked_stretch_with_mask(b, &shared_mask, config),
        ),
    );

    Ok(MaskedStretchRgbResult {
        shared_mask_coverage: shared_mask.coverage_fraction,
        shared_stars_masked: shared_mask.stars_masked,
        r: res_r?,
        g: res_g?,
        b: res_b?,
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
    let mut bg_vals: Vec<f32> = Vec::new();
    ndarray::Zip::from(image)
        .and(mask)
        .for_each(|&v, &m| {
            if m < 0.5 && v.is_finite() && v > 0.0 {
                bg_vals.push(v);
            }
        });

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
        let denom = (2.0 * m - 1.0) * x - m;
        if denom.abs() < 1e-10 {
            return x;
        }
        ((m - 1.0) * x / denom).clamp(0.0, 1.0)
    });
    out
}

fn clamp_inplace(data: &mut Array2<f32>) {
    data.par_mapv_inplace(|v| v.clamp(0.0, 1.0));
}
