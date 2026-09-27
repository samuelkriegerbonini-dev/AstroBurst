use ndarray::Array2;
use rayon::prelude::*;

use crate::core::imaging::star_mask::{generate_star_mask, StarMaskConfig, StarMaskResult};
use crate::core::imaging::stats::{is_padding, is_valid_pixel};

const MINMAX_CHUNK: usize = 1 << 16;
const CORRECTION_PASSES: usize = 3;

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

fn star_mask_config(config: &MaskedStretchConfig) -> StarMaskConfig {
    StarMaskConfig {
        growth_factor: config.mask_growth,
        softness: config.mask_softness,
        detection_sigma: config.detection_sigma,
        max_eccentricity: config.max_eccentricity,
        luminance_protect: config.luminance_protect,
        luminance_ceiling: config.luminance_ceiling,
        ..StarMaskConfig::default()
    }
}

pub fn masked_stretch(
    image: &Array2<f32>,
    config: &MaskedStretchConfig,
) -> Result<MaskedStretchResult, String> {
    let normalized = normalize_to_01(image);
    let mask_result = generate_star_mask(&normalized, &star_mask_config(config))?;
    stretch_normalized(normalized, &image.mapv(is_padding), &mask_result, config)
}

struct PassOutcome {
    iterations_run: usize,
    final_background: f64,
    converged: bool,
}

fn within_tolerance(background: f64, config: &MaskedStretchConfig) -> bool {
    (background - config.target_background).abs() < config.convergence_threshold
}

fn log_interpolated_goals(start: f64, target: f64, steps: usize) -> impl Iterator<Item = f64> {
    let log_start = start.max(f64::MIN_POSITIVE).ln();
    let log_target = target.max(f64::MIN_POSITIVE).ln();
    (1..=steps).map(move |k| (log_start + (log_target - log_start) * k as f64 / steps as f64).exp())
}

fn run_stretch_passes(
    initial_background: f64,
    config: &MaskedStretchConfig,
    mut apply_pass_and_measure: impl FnMut(f32, f32) -> f64,
) -> PassOutcome {
    let target = config.target_background;
    let mut background = initial_background;
    let mut converged = within_tolerance(background, config);
    if converged || background >= target {
        return PassOutcome { iterations_run: 0, final_background: background, converged };
    }

    let steps = config.iterations.max(1);
    let mut iterations_run = 0;
    let mut pass_towards = |goal: f64, background: &mut f64| {
        let midtone = mtf_balance(*background, goal) as f32;
        let next = apply_pass_and_measure(midtone, *background as f32);
        iterations_run += 1;
        let moved = (next - *background).abs();
        *background = next;
        moved
    };

    let mut stalled = false;
    for goal in log_interpolated_goals(background, target, steps) {
        stalled = pass_towards(goal, &mut background) == 0.0;
        if stalled {
            break;
        }
    }
    converged = within_tolerance(background, config);

    for _ in 0..CORRECTION_PASSES {
        if converged || stalled {
            break;
        }
        let moved = pass_towards(target, &mut background);
        converged = within_tolerance(background, config);
        stalled = moved < config.convergence_threshold * 0.1;
    }

    PassOutcome { iterations_run, final_background: background, converged }
}

fn channel_result(image: Array2<f32>, outcome: &PassOutcome, mask_result: &StarMaskResult) -> MaskedStretchResult {
    MaskedStretchResult {
        image,
        iterations_run: outcome.iterations_run,
        final_background: outcome.final_background,
        stars_masked: mask_result.stars_masked,
        mask_coverage: mask_result.coverage_fraction,
        converged: outcome.converged,
    }
}

fn stretch_normalized(
    mut working: Array2<f32>,
    padding: &Array2<bool>,
    mask_result: &StarMaskResult,
    config: &MaskedStretchConfig,
) -> Result<MaskedStretchResult, String> {
    let protection = config.protection_amount as f32;
    let mask = &mask_result.mask;

    let mut bg_buf: Vec<f32> = Vec::new();
    let mut scratch = Array2::zeros(working.dim());
    let mut weights = Array2::zeros(working.dim());
    let initial_bg = compute_masked_median(&working, mask, padding, &mut bg_buf);

    let outcome = run_stretch_passes(initial_bg, config, |midtone, background| {
        protection_weights_into(&working, mask, background, protection, &mut weights);
        stretch_blend_with_scratch(&mut working, &weights, midtone, &mut scratch);
        compute_masked_median(&working, mask, padding, &mut bg_buf)
    });

    clamp_inplace(&mut working);

    Ok(channel_result(working, &outcome, mask_result))
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

    let shared_mask = generate_star_mask(&normalize_to_01(&luminance), &star_mask_config(config))?;
    let mask = &shared_mask.mask;
    let protection = config.protection_amount as f32;

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

    let padding = ndarray::Zip::from(r)
        .and(g)
        .and(b)
        .map_collect(|&rv, &gv, &bv| is_padding(rv) || is_padding(gv) || is_padding(bv));
    let mut bg_buf: Vec<f32> = Vec::new();
    let mut lum = Array2::zeros(wr.dim());
    let mut scratch = Array2::zeros(wr.dim());
    let mut weights = Array2::zeros(wr.dim());

    compute_luminance_into(&mut lum, &wr, &wg, &wb);
    let initial_bg = compute_masked_median(&lum, mask, &padding, &mut bg_buf);

    let outcome = run_stretch_passes(initial_bg, config, |midtone, background| {
        protection_weights_into(&lum, mask, background, protection, &mut weights);
        stretch_blend_with_scratch(&mut wr, &weights, midtone, &mut scratch);
        stretch_blend_with_scratch(&mut wg, &weights, midtone, &mut scratch);
        stretch_blend_with_scratch(&mut wb, &weights, midtone, &mut scratch);
        compute_luminance_into(&mut lum, &wr, &wg, &wb);
        compute_masked_median(&lum, mask, &padding, &mut bg_buf)
    });

    clamp_inplace(&mut wr);
    clamp_inplace(&mut wg);
    clamp_inplace(&mut wb);

    Ok(MaskedStretchRgbResult {
        shared_mask_coverage: shared_mask.coverage_fraction,
        shared_stars_masked: shared_mask.stars_masked,
        r: channel_result(wr, &outcome, &shared_mask),
        g: channel_result(wg, &outcome, &shared_mask),
        b: channel_result(wb, &outcome, &shared_mask),
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
            *o = if is_padding(v) {
                0.0
            } else {
                ((v - dmin) * inv).clamp(0.0, 1.0)
            };
        });
    out
}

fn protection_weights_into(
    reference: &Array2<f32>,
    mask: &Array2<f32>,
    background: f32,
    protection: f32,
    out: &mut Array2<f32>,
) {
    let inv_headroom = 1.0 / (1.0 - background).max(1e-6);
    ndarray::Zip::from(out)
        .and(reference)
        .and(mask)
        .par_for_each(|w, &value, &m| {
            let brightness_above_sky = ((value - background) * inv_headroom).clamp(0.0, 1.0);
            *w = protection * m * brightness_above_sky;
        });
}

fn stretch_blend_with_scratch(
    working: &mut Array2<f32>,
    weights: &Array2<f32>,
    midtone: f32,
    scratch: &mut Array2<f32>,
) {
    apply_mtf_into(working, midtone, scratch);
    ndarray::Zip::from(working)
        .and(&*scratch)
        .and(weights)
        .par_for_each(|dst, &stretched, &w| {
            *dst = *dst * w + stretched * (1.0 - w);
        });
}

fn compute_masked_median(
    image: &Array2<f32>,
    mask: &Array2<f32>,
    padding: &Array2<bool>,
    bg_vals: &mut Vec<f32>,
) -> f64 {
    bg_vals.clear();
    match (image.as_slice(), mask.as_slice(), padding.as_slice()) {
        (Some(img), Some(msk), Some(pad)) => {
            let chunks: Vec<Vec<f32>> = img
                .par_chunks(MINMAX_CHUNK)
                .zip(msk.par_chunks(MINMAX_CHUNK))
                .zip(pad.par_chunks(MINMAX_CHUNK))
                .map(|((ic, mc), pc)| {
                    let mut local = Vec::with_capacity(ic.len());
                    for (i, &v) in ic.iter().enumerate() {
                        if mc[i] < 0.5 && !pc[i] && v.is_finite() {
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
            ndarray::Zip::from(image).and(mask).and(padding).for_each(|&v, &m, &p| {
                if m < 0.5 && !p && v.is_finite() {
                    bg_vals.push(v);
                }
            });
        }
    }

    if bg_vals.is_empty() {
        return 0.0;
    }

    let mid = bg_vals.len() / 2;
    bg_vals.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    bg_vals[mid] as f64
}

fn mtf_balance(median: f64, target: f64) -> f64 {
    let denom = 2.0 * target * median - target - median;
    if denom.abs() < 1e-15 {
        return 0.5;
    }
    (median * (target - 1.0) / denom).clamp(0.0001, 0.9999)
}

fn mtf(x: f32, m: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let denom = (2.0 * m - 1.0) * x - m;
    if denom.abs() < 1e-10 {
        x
    } else {
        ((m - 1.0) * x / denom).clamp(0.0, 1.0)
    }
}

fn apply_mtf_into(data: &Array2<f32>, m: f32, out: &mut Array2<f32>) {
    ndarray::Zip::from(out)
        .and(data)
        .par_for_each(|o, &x| *o = mtf(x, m));
}

fn clamp_inplace(data: &mut Array2<f32>) {
    data.par_mapv_inplace(|v| v.clamp(0.0, 1.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);

    impl Lcg {
        fn uniform(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }

        fn gaussian(&mut self) -> f32 {
            let s: f32 = (0..4).map(|_| self.uniform()).sum();
            (s - 2.0) * 3.0f32.sqrt()
        }
    }

    const FIELD: usize = 96;
    const NOISE_SIGMA: f32 = 1.0;
    const STAR_PEAK: f32 = 500.0;
    const STAR_SIGMA: f32 = 1.5;
    const FWHM_PER_SIGMA: f32 = 2.354_82;
    const STAR_FWHM: f32 = FWHM_PER_SIGMA * STAR_SIGMA;
    const BLOAT_LEVEL: f32 = 0.6;
    const PADDING_PROBES: [(&str, (usize, usize)); 3] = [("zero", (0, 0)), ("NaN", (0, 1)), ("infinite", (FIELD - 1, FIELD - 1))];

    fn add_stars(img: &mut Array2<f32>, centres: &[(f32, f32)], peak: f32) {
        for &(cy, cx) in centres {
            for ((y, x), v) in img.indexed_iter_mut() {
                let d2 = (y as f32 - cy).powi(2) + (x as f32 - cx).powi(2);
                *v += peak * (-d2 / (2.0 * STAR_SIGMA * STAR_SIGMA)).exp();
            }
        }
    }

    fn mark_padding(img: &mut Array2<f32>) {
        img[[0, 0]] = 0.0;
        img[[0, 1]] = f32::NAN;
        img[[FIELD - 1, FIELD - 1]] = f32::INFINITY;
    }

    fn without_protection(config: &MaskedStretchConfig) -> MaskedStretchConfig {
        MaskedStretchConfig { protection_amount: 0.0, ..config.clone() }
    }

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

    const PROBES: [(usize, f32); 4] = [(10, -2.0), (20, -0.1), (30, 0.1), (40, 2.0)];
    const PROBE_ROW: usize = 32;

    fn zero_mean_sky_with_stars(scale: f32) -> Array2<f32> {
        let mut rng = Lcg(0x9E37_79B9_7F4A_7C15);
        let mut img = Array2::from_shape_fn((64, 64), |_| rng.gaussian());
        add_stars(&mut img, &[(12.0, 12.0), (12.0, 52.0), (52.0, 12.0), (52.0, 52.0)], STAR_PEAK);
        for &(x, v) in &PROBES {
            img[[PROBE_ROW, x]] = v;
        }
        img[[0, 0]] = 0.0;
        img[[0, 1]] = f32::NAN;
        img.mapv_inplace(|v| v * scale);
        img
    }

    fn assert_continuous_ramp(out: &Array2<f32>, label: &str) {
        let p: Vec<f32> = PROBES.iter().map(|&(x, _)| out[[PROBE_ROW, x]]).collect();
        assert!(
            p.windows(2).all(|w| w[0] < w[1]),
            "{}: probes at -2, -0.1, +0.1, +2 sigma not strictly increasing: {:?}",
            label,
            p
        );
        assert!(
            p[2] - p[1] < 0.15 * (p[3] - p[0]),
            "{}: gap across zero {} vs span {}: {:?}",
            label,
            p[2] - p[1],
            p[3] - p[0],
            p
        );
        assert_eq!(out[[0, 0]], 0.0, "{}: zero padding", label);
        assert_eq!(out[[0, 1]], 0.0, "{}: NaN padding", label);
    }

    #[test]
    fn linked_rgb_stretch_is_continuous_across_zero_on_sky_subtracted_data() {
        let r = zero_mean_sky_with_stars(1.2);
        let g = zero_mean_sky_with_stars(1.0);
        let b = zero_mean_sky_with_stars(0.8);
        let res = masked_stretch_rgb_shared(&r, &g, &b, &MaskedStretchConfig::default()).unwrap();
        assert_continuous_ramp(&res.r.image, "R");
        assert_continuous_ramp(&res.g.image, "G");
        assert_continuous_ramp(&res.b.image, "B");
    }

    #[test]
    fn mono_stretch_is_continuous_across_zero_on_sky_subtracted_data() {
        let img = zero_mean_sky_with_stars(1.0);
        let res = masked_stretch(&img, &MaskedStretchConfig::default()).unwrap();
        assert_continuous_ramp(&res.image, "mono");
    }

    #[test]
    fn quantized_levels_above_the_floor_are_not_crushed_to_black() {
        let level = |y: usize, x: usize| 100.0 + ((x + 3 * y) % 4) as f32;
        let mut img = Array2::from_shape_fn((64, 64), |(y, x)| level(y, x));
        for ((y, x), v) in img.indexed_iter_mut() {
            let d2 = (y as i32 - 32).pow(2) + (x as i32 - 32).pow(2);
            *v += if d2 <= 2 { 400.0 } else if d2 <= 8 { 100.0 } else { 0.0 };
        }
        img[[0, 0]] = 0.0;
        let res = masked_stretch(&img, &MaskedStretchConfig::default()).unwrap();
        let out_at = |target: f32| {
            let (y, x) = (2..10)
                .flat_map(|y| (2..10).map(move |x| (y, x)))
                .find(|&(y, x)| level(y, x) == target)
                .unwrap();
            res.image[[y, x]]
        };
        let outs = [out_at(100.0), out_at(101.0), out_at(102.0), out_at(103.0)];
        assert_eq!(outs[0], 0.0, "floor level {:?}", outs);
        assert!(outs.windows(2).all(|w| w[0] < w[1]), "levels merged: {:?}", outs);
        assert_eq!(res.image[[0, 0]], 0.0);
    }

    const STAR_CENTRES: [(f32, f32); 5] = [(24.0, 24.0), (24.0, 72.0), (72.0, 24.0), (72.0, 72.0), (48.0, 48.0)];

    fn faint_sky_with_stars() -> Array2<f32> {
        let mut rng = Lcg(0x2545_F491_4F6C_DD1D);
        let mut img = Array2::from_shape_fn((FIELD, FIELD), |_| 10.0 + NOISE_SIGMA * rng.gaussian());
        add_stars(&mut img, &STAR_CENTRES, STAR_PEAK);
        mark_padding(&mut img);
        img
    }

    fn median_of(mut values: Vec<f32>) -> f32 {
        assert!(!values.is_empty());
        values.sort_by(|a, b| a.total_cmp(b));
        values[values.len() / 2]
    }

    fn split_sky_by_mask(
        mask: &Array2<f32>,
        is_sky_at: impl Fn(usize, usize) -> bool,
    ) -> (Vec<(usize, usize)>, Vec<(usize, usize)>) {
        mask.indexed_iter()
            .map(|(idx, _)| idx)
            .filter(|&(y, x)| is_sky_at(y, x))
            .partition(|&(y, x)| mask[[y, x]] > 0.5)
    }

    fn sky_pixels_split_by_mask(mask: &Array2<f32>, reference: &Array2<f32>) -> (Vec<(usize, usize)>, Vec<(usize, usize)>) {
        let outside: Vec<f32> = reference
            .indexed_iter()
            .filter(|((y, x), v)| mask[[*y, *x]] < 0.5 && is_valid_pixel(**v))
            .map(|(_, v)| *v)
            .collect();
        let sky_median = median_of(outside.clone());
        let sky_sigma = 1.4826 * median_of(outside.iter().map(|v| (v - sky_median).abs()).collect());
        let cutoff = sky_median + 5.0 * sky_sigma;
        split_sky_by_mask(mask, |y, x| is_valid_pixel(reference[[y, x]]) && reference[[y, x]] < cutoff)
    }

    fn sky_pixels_inside_star_masks(img: &Array2<f32>, config: &MaskedStretchConfig) -> Vec<(usize, usize)> {
        let mask = generate_star_mask(&normalize_to_01(img), &star_mask_config(config)).unwrap();
        assert_eq!(mask.stars_masked, STAR_CENTRES.len(), "the synthetic stars were not all masked");
        sky_pixels_split_by_mask(&mask.mask, img).0
    }

    fn lifted_sky(img: &Array2<f32>, offset: f32) -> Array2<f32> {
        img.mapv(|v| if is_padding(v) { v } else { v + offset })
    }

    #[test]
    fn sky_inside_star_masks_is_stretched_to_the_target_background() {
        let img = faint_sky_with_stars();
        let config = MaskedStretchConfig::default();
        let sky_in_masks = sky_pixels_inside_star_masks(&img, &config);
        assert!(sky_in_masks.len() > 300, "only {} sky pixels inside masks", sky_in_masks.len());

        let res = masked_stretch(&img, &config).unwrap();
        let inside = median_of(sky_in_masks.iter().map(|&(y, x)| res.image[[y, x]]).collect());
        let target = config.target_background as f32;
        assert!(
            (inside - target).abs() <= 0.15 * target,
            "sky inside the star masks lands at {} instead of about {}",
            inside,
            target
        );
        assert_eq!(res.image[[0, 0]], 0.0, "zero padding");
        assert_eq!(res.image[[0, 1]], 0.0, "NaN padding");
        assert_eq!(res.image[[95, 95]], 0.0, "infinite padding");
    }

    #[test]
    fn shared_rgb_sky_inside_star_masks_is_stretched_to_the_target_background() {
        let img = faint_sky_with_stars();
        let config = MaskedStretchConfig::default();
        let sky_in_masks = sky_pixels_inside_star_masks(&img, &config);
        let res = masked_stretch_rgb_shared(&img, &img, &img, &config).unwrap();
        let target = config.target_background as f32;
        for (label, plane) in [("R", &res.r.image), ("G", &res.g.image), ("B", &res.b.image)] {
            let inside = median_of(sky_in_masks.iter().map(|&(y, x)| plane[[y, x]]).collect());
            assert!(
                (inside - target).abs() <= 0.15 * target,
                "{}: sky inside the star masks lands at {} instead of about {}",
                label,
                inside,
                target
            );
            assert_eq!(plane[[0, 0]], 0.0, "{}: zero padding", label);
            assert_eq!(plane[[0, 1]], 0.0, "{}: NaN padding", label);
        }
    }

    #[test]
    fn shared_rgb_sky_inside_star_masks_matches_each_channels_own_sky_under_a_colour_cast() {
        let r = faint_sky_with_stars();
        let g = lifted_sky(&r, 30.0);
        let b = r.clone();
        let config = MaskedStretchConfig::default();
        let luminance = compute_luminance(&r, &g, &b).unwrap();
        let mask = generate_star_mask(&normalize_to_01(&luminance), &star_mask_config(&config)).unwrap();
        assert_eq!(mask.stars_masked, STAR_CENTRES.len(), "the synthetic stars were not all masked");
        let (inside, outside) = sky_pixels_split_by_mask(&mask.mask, &r);
        assert!(inside.len() > 300, "only {} sky pixels inside masks", inside.len());

        let res = masked_stretch_rgb_shared(&r, &g, &b, &config).unwrap();
        for (label, plane) in [("R", &res.r.image), ("G", &res.g.image), ("B", &res.b.image)] {
            let inside_median = median_of(inside.iter().map(|&(y, x)| plane[[y, x]]).collect());
            let outside_median = median_of(outside.iter().map(|&(y, x)| plane[[y, x]]).collect());
            assert!(
                (inside_median - outside_median).abs() <= 0.15 * outside_median,
                "{}: sky inside the star masks lands at {} but the sky around them at {}",
                label,
                inside_median,
                outside_median
            );
            assert_eq!(plane[[0, 0]], 0.0, "{}: zero padding", label);
            assert_eq!(plane[[0, 1]], 0.0, "{}: NaN padding", label);
            assert_eq!(plane[[95, 95]], 0.0, "{}: infinite padding", label);
        }
        let g_sky = median_of(outside.iter().map(|&(y, x)| res.g.image[[y, x]]).collect());
        let r_sky = median_of(outside.iter().map(|&(y, x)| res.r.image[[y, x]]).collect());
        assert!(g_sky > r_sky * 1.5, "the colour cast should survive the stretch: G {} vs R {}", g_sky, r_sky);
    }

    const GRADIENT_SKY_RANGE: (f32, f32) = (8.0, 30.0);
    const GRADIENT_STARS: [(f32, f32); 3] = [(20.0, 74.0), (48.0, 80.0), (76.0, 70.0)];

    fn gradient_sky_at(x: usize) -> f32 {
        let (low, high) = GRADIENT_SKY_RANGE;
        low + (high - low) * x as f32 / (FIELD - 1) as f32
    }

    fn gradient_sky_with_stars() -> Array2<f32> {
        let mut rng = Lcg(0x5851_F42D_4C95_7F2D);
        let mut img = Array2::from_shape_fn((FIELD, FIELD), |(_, x)| gradient_sky_at(x) + NOISE_SIGMA * rng.gaussian());
        add_stars(&mut img, &GRADIENT_STARS, STAR_PEAK);
        mark_padding(&mut img);
        img
    }

    fn is_sky(value: f32, local_sky: f32) -> bool {
        is_valid_pixel(value) && value < local_sky + 3.0 * NOISE_SIGMA
    }

    fn gradient_sky_split(img: &Array2<f32>, config: &MaskedStretchConfig) -> (Vec<(usize, usize)>, Vec<(usize, usize)>) {
        let mask = generate_star_mask(&normalize_to_01(img), &star_mask_config(config)).unwrap();
        assert_eq!(mask.stars_masked, GRADIENT_STARS.len(), "the synthetic stars were not all masked");
        split_sky_by_mask(&mask.mask, |y, x| is_sky(img[[y, x]], gradient_sky_at(x)))
    }

    fn worst_relative_deviation(pixels: &[(usize, usize)], actual: &Array2<f32>, reference: &Array2<f32>) -> f32 {
        pixels
            .iter()
            .map(|&(y, x)| (actual[[y, x]] - reference[[y, x]]).abs() / reference[[y, x]].max(1e-6))
            .fold(0.0, f32::max)
    }

    fn pixels_above(image: &Array2<f32>, centre: (f32, f32), radius: f32, level: f32) -> usize {
        image
            .indexed_iter()
            .filter(|((y, x), v)| {
                let d2 = (*y as f32 - centre.0).powi(2) + (*x as f32 - centre.1).powi(2);
                d2 <= radius * radius && **v > level
            })
            .count()
    }

    fn total_bloat(image: &Array2<f32>) -> usize {
        GRADIENT_STARS
            .iter()
            .map(|&centre| pixels_above(image, centre, 3.0 * STAR_FWHM, BLOAT_LEVEL))
            .sum()
    }

    #[test]
    fn sky_inside_star_discs_keeps_its_local_level_under_a_gradient() {
        let img = gradient_sky_with_stars();
        let config = MaskedStretchConfig::default();
        let (inside, _) = gradient_sky_split(&img, &config);
        assert!(inside.len() > 300, "only {} sky pixels inside the discs", inside.len());

        let unprotected = masked_stretch(&img, &without_protection(&config)).unwrap();
        let res = masked_stretch(&img, &config).unwrap();
        let inside_median = median_of(inside.iter().map(|&(y, x)| res.image[[y, x]]).collect());
        let plain_median = median_of(inside.iter().map(|&(y, x)| unprotected.image[[y, x]]).collect());
        let worst = worst_relative_deviation(&inside, &res.image, &unprotected.image);
        assert!(
            worst <= 0.10,
            "sky inside the discs deviates up to {:.1}% from the plain stretch (medians {} vs {}, global target {})",
            worst * 100.0,
            inside_median,
            plain_median,
            config.target_background
        );
    }

    #[test]
    fn stars_bloat_less_than_in_the_unprotected_stretch_and_never_brighten() {
        let img = gradient_sky_with_stars();
        let config = MaskedStretchConfig::default();
        let unprotected = masked_stretch(&img, &without_protection(&config)).unwrap();
        let res = masked_stretch(&img, &config).unwrap();
        let radius = 3.0 * STAR_FWHM;
        for &(cy, cx) in &GRADIENT_STARS {
            let plain = pixels_above(&unprotected.image, (cy, cx), radius, BLOAT_LEVEL);
            let protected = pixels_above(&res.image, (cy, cx), radius, BLOAT_LEVEL);
            assert!(
                protected < plain,
                "star at ({}, {}): {} pixels above {} with protection vs {} without",
                cy,
                cx,
                protected,
                BLOAT_LEVEL,
                plain
            );
            let (y, x) = (cy as usize, cx as usize);
            assert!(
                res.image[[y, x]] <= unprotected.image[[y, x]] + 1e-6,
                "star peak at ({}, {}) is {} but the unprotected stretch gives {}",
                cy,
                cx,
                res.image[[y, x]],
                unprotected.image[[y, x]]
            );
        }
        let brightened = res
            .image
            .iter()
            .zip(unprotected.image.iter())
            .map(|(protected, plain)| protected - plain)
            .fold(f32::MIN, f32::max);
        assert!(brightened <= 1e-4, "protection brightened a pixel by {} over the unprotected stretch", brightened);
    }

    #[test]
    fn padding_stays_exactly_zero_for_zero_nan_and_infinite_inputs() {
        let img = gradient_sky_with_stars();
        let res = masked_stretch(&img, &MaskedStretchConfig::default()).unwrap();
        for (label, idx) in PADDING_PROBES {
            assert_eq!(res.image[idx], 0.0, "mono: {} padding", label);
        }
        let [r, g, b] = colour_cast_field();
        let rgb = masked_stretch_rgb_shared(&r, &g, &b, &MaskedStretchConfig::default()).unwrap();
        for (channel, plane) in [("R", &rgb.r.image), ("G", &rgb.g.image), ("B", &rgb.b.image)] {
            for (label, idx) in PADDING_PROBES {
                assert_eq!(plane[idx], 0.0, "{}: {} padding", channel, label);
            }
        }
    }

    #[test]
    fn more_iterations_reach_the_same_background_with_stronger_star_protection() {
        let img = gradient_sky_with_stars();
        let one = MaskedStretchConfig { iterations: 1, ..MaskedStretchConfig::default() };
        let twenty = MaskedStretchConfig { iterations: 20, ..MaskedStretchConfig::default() };
        let quick = masked_stretch(&img, &one).unwrap();
        let slow = masked_stretch(&img, &twenty).unwrap();
        for (label, res, cfg) in [("1 step", &quick, &one), ("20 steps", &slow, &twenty)] {
            assert!(res.converged, "{}: not converged, background {}", label, res.final_background);
            assert!(
                (res.final_background - cfg.target_background).abs() < cfg.convergence_threshold,
                "{}: background {} is not at the target {}",
                label,
                res.final_background,
                cfg.target_background
            );
        }
        assert_eq!(quick.iterations_run, 1, "1 step: passes applied");
        assert_eq!(slow.iterations_run, 20, "20 steps: passes applied");
        let (quick_bloat, slow_bloat) = (total_bloat(&quick.image), total_bloat(&slow.image));
        assert!(
            slow_bloat < quick_bloat,
            "20 steps leave {} pixels above {} around the stars, 1 step leaves {}",
            slow_bloat,
            BLOAT_LEVEL,
            quick_bloat
        );

        let rgb = masked_stretch_rgb_shared(&img, &img, &img, &twenty).unwrap();
        assert_eq!(rgb.g.iterations_run, 20, "shared RGB: passes applied");
        assert!(rgb.g.converged, "shared RGB: not converged, background {}", rgb.g.final_background);
    }

    const PLANNED_STEPS: usize = 10;
    const START_BACKGROUND: f64 = 0.01;

    fn planned_config() -> MaskedStretchConfig {
        MaskedStretchConfig { iterations: PLANNED_STEPS, ..MaskedStretchConfig::default() }
    }

    fn goal_of(midtone: f32, background: f32) -> f64 {
        mtf(background, midtone) as f64
    }

    #[test]
    fn no_pass_runs_when_the_background_starts_at_or_above_the_target() {
        let config = MaskedStretchConfig::default();
        let (target, tolerance) = (config.target_background, config.convergence_threshold);
        let never = |_: f32, _: f32| -> f64 { panic!("a pass ran on a background that needs none") };

        let above = run_stretch_passes(target + 0.1, &config, never);
        assert_eq!(above.iterations_run, 0, "passes applied to a background above the target");
        assert!(!above.converged, "a background above the target was reported as converged");
        assert_eq!(above.final_background, target + 0.1);

        for start in [target - 0.5 * tolerance, target, target + 0.5 * tolerance] {
            let within = run_stretch_passes(start, &config, never);
            assert_eq!(within.iterations_run, 0, "passes applied to a background of {} at the target", start);
            assert!(within.converged, "a background of {} within tolerance was not reported as converged", start);
            assert_eq!(within.final_background, start);
        }
    }

    #[test]
    fn passes_aim_at_goals_log_interpolated_from_the_background_to_the_target() {
        let config = planned_config();
        let target = config.target_background;
        let mut goals = Vec::new();
        let outcome = run_stretch_passes(START_BACKGROUND, &config, |midtone, background| {
            let goal = goal_of(midtone, background);
            goals.push(goal);
            goal
        });
        assert_eq!(outcome.iterations_run, PLANNED_STEPS, "a closure that lands on every goal needs no correction");
        assert!(outcome.converged, "background {} after the planned passes", outcome.final_background);
        assert_eq!(goals.len(), PLANNED_STEPS);
        for (k, goal) in (1..=PLANNED_STEPS).zip(&goals) {
            let fraction = k as f64 / PLANNED_STEPS as f64;
            let expected = (START_BACKGROUND.ln() + (target.ln() - START_BACKGROUND.ln()) * fraction).exp();
            assert!(
                (goal - expected).abs() <= 1e-4 * expected,
                "pass {}: aimed at {} instead of the log-interpolated goal {}",
                k,
                goal,
                expected
            );
        }
    }

    #[test]
    fn correction_passes_follow_the_planned_steps_until_the_target_is_reached() {
        let config = planned_config();
        let (target, tolerance) = (config.target_background, config.convergence_threshold);

        let mut calls = 0;
        let mut background = START_BACKGROUND;
        let recovered = run_stretch_passes(background, &config, |midtone, measured| {
            calls += 1;
            let goal = goal_of(midtone, measured);
            background = if calls <= PLANNED_STEPS { background + 0.5 * (goal - background) } else { goal };
            background
        });
        assert_eq!(recovered.iterations_run, PLANNED_STEPS + 1, "one correction pass lands on the target");
        assert!(recovered.converged, "background {} after the correction", recovered.final_background);
        assert!((recovered.final_background - target).abs() < tolerance);

        let mut background = START_BACKGROUND;
        let short = run_stretch_passes(background, &config, |midtone, measured| {
            background += 0.5 * (goal_of(midtone, measured) - background);
            background
        });
        assert_eq!(short.iterations_run, PLANNED_STEPS + CORRECTION_PASSES, "corrections stop after the allowed passes");
        assert!(!short.converged, "background {} short of the target was reported as converged", short.final_background);
        assert!(short.final_background < target - tolerance);
    }

    #[test]
    fn passes_stop_when_the_background_stops_moving() {
        let config = planned_config();
        let stuck_level = 0.0625;

        let stuck = run_stretch_passes(stuck_level, &config, |_, _| stuck_level);
        assert_eq!(stuck.iterations_run, 1, "a pass that moves nothing ends the stretch");
        assert!(!stuck.converged);
        assert_eq!(stuck.final_background, stuck_level);

        let mut calls = 0;
        let mut background = START_BACKGROUND;
        let stuck_in_correction = run_stretch_passes(background, &config, |midtone, measured| {
            calls += 1;
            if calls <= PLANNED_STEPS {
                background += 0.5 * (goal_of(midtone, measured) - background);
            }
            background
        });
        assert_eq!(stuck_in_correction.iterations_run, PLANNED_STEPS + 1, "a correction that moves nothing ends the stretch");
        assert!(!stuck_in_correction.converged);
    }

    const CAST_SKY: [f32; 3] = [10.0, 25.0, 5.0];

    fn colour_cast_field() -> [Array2<f32>; 3] {
        let seeds = [0x0DDB_ADC0_FFEE_0001u64, 0x0DDB_ADC0_FFEE_0002, 0x0DDB_ADC0_FFEE_0003];
        let mut planes = [(); 3].map(|_| Array2::zeros((FIELD, FIELD)));
        for (plane, (&sky, seed)) in planes.iter_mut().zip(CAST_SKY.iter().zip(seeds)) {
            let mut rng = Lcg(seed);
            *plane = Array2::from_shape_fn((FIELD, FIELD), |_| sky + NOISE_SIGMA * rng.gaussian());
            add_stars(plane, &STAR_CENTRES, STAR_PEAK);
            mark_padding(plane);
        }
        planes
    }

    #[test]
    fn shared_rgb_keeps_the_sky_and_its_hue_inside_the_discs_under_a_colour_cast() {
        let [r, g, b] = colour_cast_field();
        let config = MaskedStretchConfig::default();
        let luminance = compute_luminance(&r, &g, &b).unwrap();
        let mask = generate_star_mask(&normalize_to_01(&luminance), &star_mask_config(&config)).unwrap();
        assert_eq!(mask.stars_masked, STAR_CENTRES.len(), "the synthetic stars were not all masked");
        let planes = [&r, &g, &b];
        let (inside, outside) = split_sky_by_mask(&mask.mask, |y, x| {
            planes.iter().zip(CAST_SKY).all(|(plane, sky)| is_sky(plane[[y, x]], sky))
        });
        assert!(inside.len() > 300, "only {} sky pixels inside the discs", inside.len());

        let unprotected = masked_stretch_rgb_shared(&r, &g, &b, &without_protection(&config)).unwrap();
        let res = masked_stretch_rgb_shared(&r, &g, &b, &config).unwrap();
        for (label, plane, reference) in [
            ("R", &res.r.image, &unprotected.r.image),
            ("G", &res.g.image, &unprotected.g.image),
            ("B", &res.b.image, &unprotected.b.image),
        ] {
            let worst = worst_relative_deviation(&inside, plane, reference);
            assert!(
                worst <= 0.10,
                "{}: sky inside the discs deviates up to {:.1}% from the unprotected shared stretch",
                label,
                worst * 100.0
            );
        }
        let mean_hue = |pixels: &[(usize, usize)]| {
            pixels.iter().map(|&(y, x)| res.r.image[[y, x]] / res.g.image[[y, x]]).sum::<f32>() / pixels.len() as f32
        };
        let (hue_inside, hue_outside) = (mean_hue(&inside), mean_hue(&outside));
        assert!(
            (hue_inside - hue_outside).abs() <= 0.05 * hue_outside,
            "mean R/G is {} inside the discs but {} outside",
            hue_inside,
            hue_outside
        );
        for (label, result) in [("R", &res.r), ("G", &res.g), ("B", &res.b)] {
            assert!(result.converged, "{}: not converged, background {}", label, result.final_background);
            assert!(
                result.iterations_run > config.iterations,
                "{}: {} passes, but the shared luminance needs corrections after the {} planned ones",
                label,
                result.iterations_run,
                config.iterations
            );
        }
    }

    fn shared_star_mask(r: &Array2<f32>, g: &Array2<f32>, b: &Array2<f32>, config: &MaskedStretchConfig) -> StarMaskResult {
        let luminance = compute_luminance(r, g, b).unwrap();
        generate_star_mask(&normalize_to_01(&luminance), &star_mask_config(config)).unwrap()
    }

    #[test]
    fn shared_rgb_protects_every_channel_with_the_weight_of_its_luminance() {
        let [r, g, b] = colour_cast_field();
        let config = MaskedStretchConfig { iterations: 1, convergence_threshold: 0.1, ..MaskedStretchConfig::default() };
        let res = masked_stretch_rgb_shared(&r, &g, &b, &config).unwrap();
        assert_eq!(res.g.iterations_run, 1, "the weight can only be recovered from a single pass");

        let mask = shared_star_mask(&r, &g, &b, &config);
        let (dmin, dmax) = shared_min_max(&[&r, &g, &b]).unwrap();
        let inputs = [&r, &g, &b].map(|plane| normalize_to_01_with(plane, dmin, dmax));
        let luminance = compute_luminance(&inputs[0], &inputs[1], &inputs[2]).unwrap();
        let padding = ndarray::Zip::from(&r)
            .and(&g)
            .and(&b)
            .map_collect(|&rv, &gv, &bv| is_padding(rv) || is_padding(gv) || is_padding(bv));
        let background = compute_masked_median(&luminance, &mask.mask, &padding, &mut Vec::new());
        let midtone = mtf_balance(background, config.target_background) as f32;
        let background = background as f32;
        let protection = config.protection_amount as f32;
        let outputs = [&res.r.image, &res.g.image, &res.b.image];

        let mut checked = 0;
        let mut strongest = 0.0f32;
        for ((y, x), &m) in mask.mask.indexed_iter() {
            if m <= 0.5 || luminance[[y, x]] < background + 0.1 {
                continue;
            }
            let recovered: Vec<f32> = (0..3)
                .filter_map(|c| {
                    let input = inputs[c][[y, x]];
                    let stretched = mtf(input, midtone);
                    (stretched - input > 0.05).then(|| (stretched - outputs[c][[y, x]]) / (stretched - input))
                })
                .collect();
            let [wr, wg, wb] = match recovered[..] {
                [wr, wg, wb] => [wr, wg, wb],
                _ => continue,
            };
            assert!(
                (wr - wg).abs() <= 1e-3 && (wg - wb).abs() <= 1e-3,
                "({}, {}): R/G/B protected with different weights {} / {} / {}",
                y,
                x,
                wr,
                wg,
                wb
            );
            let expected = protection * m * ((luminance[[y, x]] - background) / (1.0 - background)).clamp(0.0, 1.0);
            assert!(
                (wr - expected).abs() <= 1e-3,
                "({}, {}): protected with weight {} but the luminance gives {}",
                y,
                x,
                wr,
                expected
            );
            checked += 1;
            strongest = strongest.max(wr);
        }
        assert!(checked >= 50, "only {} star pixels could be checked", checked);
        assert!(strongest > 0.5, "the strongest protection weight checked was only {}", strongest);
    }
}
