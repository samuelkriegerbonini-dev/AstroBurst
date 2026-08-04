use ndarray::Array2;
use rayon::prelude::*;

use crate::core::imaging::star_mask::{generate_star_mask, StarMaskConfig, StarMaskResult};
use crate::core::imaging::stats::is_valid_pixel;

const MINMAX_CHUNK: usize = 1 << 16;

#[derive(Debug, Clone)]
pub struct MaskedStretchConfig {
    pub iterations: usize,
    pub target_background: f64,
    pub mask_growth: f64,
    pub mask_softness: f64,
    pub detection_sigma: f64,
    pub max_eccentricity: f64,
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
            detection_sigma: 8.0,
            max_eccentricity: 0.85,
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
        detection_sigma: config.detection_sigma,
        max_eccentricity: config.max_eccentricity,
        luminance_protect: config.luminance_protect,
        luminance_ceiling: config.luminance_ceiling,
        ..StarMaskConfig::default()
    };

    let normalized = normalize_to_01(image);
    let mask_result = generate_star_mask(&normalized, &mask_config)?;
    masked_stretch_with_mask(&normalized, &mask_result, config)
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

    let mut bg_buf: Vec<f32> = Vec::new();
    let mut scratch = Array2::zeros(working.dim());
    let mut prev_bg = compute_masked_median(&working, mask, &mut bg_buf);
    let mut iterations_run = 0;
    let mut converged = false;

    for iter_idx in 0..config.iterations {
        iterations_run = iter_idx + 1;

        let bg = compute_masked_median(&working, mask, &mut bg_buf);

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
        stretch_blend_with_scratch(&mut working, mask, midtone as f32, protection, &mut scratch);

        prev_bg = bg;
    }

    let final_bg = compute_masked_median(&working, mask, &mut bg_buf);

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

fn compute_luminance_into(
    out: &mut Array2<f32>,
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
) {
    ndarray::Zip::from(out)
        .and(r)
        .and(g)
        .and(b)
        .par_for_each(|o, &rv, &gv, &bv| {
            let rn = if rv.is_finite() { rv } else { 0.0 };
            let gn = if gv.is_finite() { gv } else { 0.0 };
            let bn = if bv.is_finite() { bv } else { 0.0 };
            *o = 0.2126 * rn + 0.7152 * gn + 0.0722 * bn;
        });
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
    compute_luminance_into(&mut out, r, g, b);
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
        detection_sigma: config.detection_sigma,
        max_eccentricity: config.max_eccentricity,
        luminance_protect: config.luminance_protect,
        luminance_ceiling: config.luminance_ceiling,
        ..StarMaskConfig::default()
    };

    let shared_mask = generate_star_mask(&normalize_to_01(&luminance), &mask_config)?;
    let mask = &shared_mask.mask;
    let protection = config.protection_amount as f32;
    let target_bg = config.target_background;

    let (mut wr, mut wg, mut wb) = match shared_min_max(&[r, g, b]) {
        Some((dmin, dmax)) => (
            normalize_to_01_with(r, dmin, dmax),
            normalize_to_01_with(g, dmin, dmax),
            normalize_to_01_with(b, dmin, dmax),
        ),
        None => (
            Array2::zeros(r.dim()),
            Array2::zeros(g.dim()),
            Array2::zeros(b.dim()),
        ),
    };

    let mut bg_buf: Vec<f32> = Vec::new();
    let mut lum = Array2::zeros(wr.dim());
    let mut scratch = Array2::zeros(wr.dim());

    compute_luminance_into(&mut lum, &wr, &wg, &wb);
    let mut prev_bg = compute_masked_median(&lum, mask, &mut bg_buf);
    let mut iterations_run = 0;
    let mut converged = false;

    for iter_idx in 0..config.iterations {
        iterations_run = iter_idx + 1;

        compute_luminance_into(&mut lum, &wr, &wg, &wb);
        let bg = compute_masked_median(&lum, mask, &mut bg_buf);

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

        let midtone = mtf_balance(bg, target_bg) as f32;
        stretch_blend_with_scratch(&mut wr, mask, midtone, protection, &mut scratch);
        stretch_blend_with_scratch(&mut wg, mask, midtone, protection, &mut scratch);
        stretch_blend_with_scratch(&mut wb, mask, midtone, protection, &mut scratch);

        prev_bg = bg;
    }

    compute_luminance_into(&mut lum, &wr, &wg, &wb);
    let final_bg = compute_masked_median(&lum, mask, &mut bg_buf);

    clamp_inplace(&mut wr);
    clamp_inplace(&mut wg);
    clamp_inplace(&mut wb);

    Ok(MaskedStretchRgbResult {
        shared_mask_coverage: shared_mask.coverage_fraction,
        shared_stars_masked: shared_mask.stars_masked,
        r: MaskedStretchResult {
            image: wr,
            iterations_run,
            final_background: final_bg,
            stars_masked: shared_mask.stars_masked,
            mask_coverage: shared_mask.coverage_fraction,
            converged,
        },
        g: MaskedStretchResult {
            image: wg,
            iterations_run,
            final_background: final_bg,
            stars_masked: shared_mask.stars_masked,
            mask_coverage: shared_mask.coverage_fraction,
            converged,
        },
        b: MaskedStretchResult {
            image: wb,
            iterations_run,
            final_background: final_bg,
            stars_masked: shared_mask.stars_masked,
            mask_coverage: shared_mask.coverage_fraction,
            converged,
        },
    })
}

fn valid_min_max_slice(slice: &[f32], mut dmin: f32, mut dmax: f32) -> (f32, f32) {
    let pairs: Vec<(f32, f32)> = slice
        .par_chunks(MINMAX_CHUNK)
        .map(|chunk| {
            let mut mn = f32::INFINITY;
            let mut mx = f32::NEG_INFINITY;
            for &v in chunk {
                if is_valid_pixel(v) {
                    if v < mn {
                        mn = v;
                    }
                    if v > mx {
                        mx = v;
                    }
                }
            }
            (mn, mx)
        })
        .collect();
    for (mn, mx) in pairs {
        if mn < dmin {
            dmin = mn;
        }
        if mx > dmax {
            dmax = mx;
        }
    }
    (dmin, dmax)
}

fn valid_min_max(image: &Array2<f32>, dmin: f32, dmax: f32) -> (f32, f32) {
    match image.as_slice() {
        Some(s) => valid_min_max_slice(s, dmin, dmax),
        None => {
            let mut mn = dmin;
            let mut mx = dmax;
            for &v in image.iter() {
                if is_valid_pixel(v) {
                    if v < mn {
                        mn = v;
                    }
                    if v > mx {
                        mx = v;
                    }
                }
            }
            (mn, mx)
        }
    }
}

fn normalize_to_01(image: &Array2<f32>) -> Array2<f32> {
    let (dmin, dmax) = valid_min_max(image, f32::INFINITY, f32::NEG_INFINITY);
    let range = dmax - dmin;
    if !range.is_finite() || range < 1e-10 {
        return Array2::zeros(image.dim());
    }
    normalize_to_01_with(image, dmin, dmax)
}

fn shared_min_max(channels: &[&Array2<f32>]) -> Option<(f32, f32)> {
    let mut dmin = f32::INFINITY;
    let mut dmax = f32::NEG_INFINITY;
    for ch in channels {
        let (mn, mx) = valid_min_max(ch, dmin, dmax);
        dmin = mn;
        dmax = mx;
    }
    let range = dmax - dmin;
    if !range.is_finite() || range < 1e-10 {
        None
    } else {
        Some((dmin, dmax))
    }
}

fn normalize_to_01_with(image: &Array2<f32>, dmin: f32, dmax: f32) -> Array2<f32> {
    let inv = 1.0 / (dmax - dmin);
    let mut out = Array2::zeros(image.dim());
    ndarray::Zip::from(&mut out)
        .and(image)
        .par_for_each(|o, &v| {
            *o = if !v.is_finite() || v <= 0.0 {
                0.0
            } else {
                ((v - dmin) * inv).clamp(0.0, 1.0)
            };
        });
    out
}

fn stretch_blend_with_scratch(
    working: &mut Array2<f32>,
    mask: &Array2<f32>,
    midtone: f32,
    protection: f32,
    scratch: &mut Array2<f32>,
) {
    apply_mtf_into(working, midtone, scratch);
    ndarray::Zip::from(working)
        .and(&*scratch)
        .and(mask)
        .par_for_each(|dst, &stretched, &m| {
            let blend = m * protection;
            *dst = *dst * blend + stretched * (1.0 - blend);
        });
}

fn compute_masked_median(image: &Array2<f32>, mask: &Array2<f32>, bg_vals: &mut Vec<f32>) -> f64 {
    bg_vals.clear();
    match (image.as_slice(), mask.as_slice()) {
        (Some(img), Some(msk)) => {
            let chunks: Vec<Vec<f32>> = img
                .par_chunks(MINMAX_CHUNK)
                .zip(msk.par_chunks(MINMAX_CHUNK))
                .map(|(ic, mc)| {
                    let mut local = Vec::with_capacity(ic.len());
                    for (i, &v) in ic.iter().enumerate() {
                        if mc[i] < 0.5 && v.is_finite() && v > 0.0 {
                            local.push(v);
                        }
                    }
                    local
                })
                .collect();
            let total: usize = chunks.iter().map(|c| c.len()).sum();
            bg_vals.reserve(total);
            for c in &chunks {
                bg_vals.extend_from_slice(c);
            }
        }
        _ => {
            ndarray::Zip::from(image).and(mask).for_each(|&v, &m| {
                if m < 0.5 && v.is_finite() && v > 0.0 {
                    bg_vals.push(v);
                }
            });
        }
    }

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

fn apply_mtf_into(data: &Array2<f32>, m: f32, out: &mut Array2<f32>) {
    ndarray::Zip::from(out)
        .and(data)
        .par_for_each(|o, &x| {
            *o = if x <= 0.0 {
                0.0
            } else if x >= 1.0 {
                1.0
            } else {
                let denom = (2.0 * m - 1.0) * x - m;
                if denom.abs() < 1e-10 {
                    x
                } else {
                    ((m - 1.0) * x / denom).clamp(0.0, 1.0)
                }
            };
        });
}

fn clamp_inplace(data: &mut Array2<f32>) {
    data.par_mapv_inplace(|v| v.clamp(0.0, 1.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(peak: f32) -> Array2<f32> {
        Array2::from_shape_fn((24, 24), |(y, x)| {
            let dy = y as f32 - 12.0;
            let dx = x as f32 - 12.0;
            0.05 + (-(dy * dy + dx * dx) / 2.0).exp() * peak
        })
    }

    #[test]
    fn shared_mask_stretch_produces_valid_rgb() {
        let r = scene(3.0);
        let g = scene(2.0);
        let b = scene(1.0);
        let result = masked_stretch_rgb_shared(&r, &g, &b, &MaskedStretchConfig::default()).unwrap();
        for img in [&result.r.image, &result.g.image, &result.b.image] {
            assert_eq!(img.dim(), (24, 24));
            assert!(img.iter().all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0));
        }
    }

    fn tri_scene() -> (Array2<f32>, Array2<f32>, Array2<f32>) {
        let mk = |peak: f32| {
            Array2::from_shape_fn((24, 24), |(y, x)| {
                if y == 0 && x == 0 {
                    0.0
                } else {
                    let dy = y as f32 - 12.0;
                    let dx = x as f32 - 12.0;
                    0.3 + (-(dy * dy + dx * dx) / 4.0).exp() * peak
                }
            })
        };
        (mk(1.0), mk(2.0), mk(4.0))
    }

    #[test]
    fn shared_mask_preserves_neutral_background() {
        let (r, g, b) = tri_scene();
        let res = masked_stretch_rgb_shared(&r, &g, &b, &MaskedStretchConfig::default()).unwrap();
        let rv = res.r.image[[1, 1]];
        let gv = res.g.image[[1, 1]];
        let bv = res.b.image[[1, 1]];
        assert!((rv - gv).abs() < 1e-3, "R/G diverged at neutral bg: {} vs {}", rv, gv);
        assert!((gv - bv).abs() < 1e-3, "G/B diverged at neutral bg: {} vs {}", gv, bv);
    }
}
