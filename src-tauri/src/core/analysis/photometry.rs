use ndarray::Array2;
use serde::Serialize;

use crate::core::analysis::aperture::{annulus, circular_aperture, weighted_sum, AperturePixel};
use crate::core::imaging::dq_flags::apply_exclusion;
use crate::types::constants::MAD_TO_SIGMA;

const ANNULUS_MEMBERSHIP_WEIGHT: f32 = 0.5;
pub const SKY_ANNULUS_INNER_FACTOR: f64 = 2.0;
pub const SKY_ANNULUS_OUTER_FACTOR: f64 = 3.0;
pub const GROWTH_CURVE_REACH_FACTOR: f64 = 4.0;
pub const GROWTH_PLATEAU_TOLERANCE: f64 = 0.01;
const IMAGE_MAX_SATURATION_FRACTION: f64 = 0.95;

fn sorted_median(vals: &[f64]) -> f64 {
    let n = vals.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 0 {
        (vals[n / 2 - 1] + vals[n / 2]) / 2.0
    } else {
        vals[n / 2]
    }
}

#[derive(Debug, Clone)]
pub struct PhotometryConfig {
    pub search_radius: usize,
    pub aperture_radius: Option<f64>,
    pub image_max: Option<f64>,
    pub subsamples: u8,
    pub gain: Option<f64>,
}

impl Default for PhotometryConfig {
    fn default() -> Self {
        Self {
            search_radius: 8,
            aperture_radius: None,
            image_max: None,
            subsamples: 5,
            gain: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StarPhotometry {
    pub x: f64,
    pub y: f64,
    pub peak: f64,
    pub net_flux: f64,
    pub flux_err: f64,
    pub mag_inst: f64,
    pub snr: f64,
    pub fwhm: f64,
    pub aperture_radius: f64,
    pub aperture_pixels: u32,
    pub aperture_area: f64,
    pub bg_mean: f64,
    pub bg_sigma: f64,
    pub bg_pixels: u32,
    pub saturated: bool,
    pub n_masked: u32,
    pub n_saturated: u32,
    pub err_used: bool,
    pub aperture_correction: Option<f64>,
    pub flux_total: Option<f64>,
    pub plateau_radius: Option<f64>,
    pub flux_jy: Option<f64>,
    pub flux_err_jy: Option<f64>,
    pub mag_ab: Option<f64>,
    pub mag_ab_err: Option<f64>,
    pub mag_ab_total: Option<f64>,
    pub st_mag: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SkyEstimate {
    mean: f64,
    sigma: f64,
    count: u32,
}

fn annulus_stats(
    image: &Array2<f32>,
    x: f64,
    y: f64,
    inner_r: f64,
    outer_r: f64,
    subsamples: u8,
) -> SkyEstimate {
    let (h, w) = image.dim();
    let mut vals: Vec<f64> = annulus(h, w, x, y, inner_r, outer_r, subsamples)
        .into_iter()
        .filter(|p| p.weight >= ANNULUS_MEMBERSHIP_WEIGHT)
        .map(|p| image[[p.y, p.x]])
        .filter(|v| v.is_finite())
        .map(|v| v as f64)
        .collect();

    if vals.is_empty() {
        return SkyEstimate { mean: 0.0, sigma: 0.0, count: 0 };
    }

    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted_median(&vals);

    let lo = vals.len() / 4;
    let hi = (3 * vals.len() / 4).max(lo + 1).min(vals.len());
    let clipped = &vals[lo..hi];
    let mean = if clipped.is_empty() {
        median
    } else {
        clipped.iter().sum::<f64>() / clipped.len() as f64
    };

    let mut devs: Vec<f64> = vals.iter().map(|v| (v - median).abs()).collect();
    devs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let sigma = sorted_median(&devs) * MAD_TO_SIGMA;

    SkyEstimate { mean, sigma, count: vals.len() as u32 }
}

fn refine_centroid(image: &Array2<f32>, x0: f64, y0: f64, bg: f64, radius: i64) -> (f64, f64) {
    let (h, w) = image.dim();
    let mut x = x0;
    let mut y = y0;
    for _ in 0..2 {
        let cx = x.round() as i64;
        let cy = y.round() as i64;
        let mut sum_x = 0.0f64;
        let mut sum_y = 0.0f64;
        let mut sum_w = 0.0f64;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let ny = cy + dy;
                let nx = cx + dx;
                if ny < 0 || nx < 0 || ny >= h as i64 || nx >= w as i64 {
                    continue;
                }
                let v = image[[ny as usize, nx as usize]];
                if !v.is_finite() {
                    continue;
                }
                let wgt = (v as f64 - bg).max(0.0);
                sum_x += nx as f64 * wgt;
                sum_y += ny as f64 * wgt;
                sum_w += wgt;
            }
        }
        if sum_w <= 0.0 {
            return (x0, y0);
        }
        x = sum_x / sum_w;
        y = sum_y / sum_w;
    }
    (x, y)
}

fn fwhm_truncated_moments(image: &Array2<f32>, x: f64, y: f64, bg: f64) -> f64 {
    let (h, w) = image.dim();
    let ix = x.round().clamp(0.0, (w - 1) as f64) as usize;
    let iy = y.round().clamp(0.0, (h - 1) as f64) as usize;
    let peak = image[[iy, ix]] as f64;
    let net_peak = peak - bg;
    if !net_peak.is_finite() || net_peak <= 0.0 {
        return 0.0;
    }
    let threshold = bg + 0.5 * net_peak;
    let radius = 12i64;

    let mut m2 = 0.0f64;
    let mut sum_w = 0.0f64;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let ny = iy as i64 + dy;
            let nx = ix as i64 + dx;
            if ny < 0 || nx < 0 || ny >= h as i64 || nx >= w as i64 {
                continue;
            }
            let v = image[[ny as usize, nx as usize]] as f64;
            if !v.is_finite() || v < threshold {
                continue;
            }
            let wgt = v - bg;
            let fx = nx as f64 - x;
            let fy = ny as f64 - y;
            m2 += (fx * fx + fy * fy) * wgt;
            sum_w += wgt;
        }
    }
    if sum_w <= 0.0 {
        return 0.0;
    }

    let half_max_truncation = 1.0 - std::f64::consts::LN_2;
    let sigma2 = (m2 / sum_w / 2.0) / half_max_truncation;
    let fwhm = 2.0 * (2.0_f64.ln() * 2.0).sqrt() * sigma2.max(0.0).sqrt();
    fwhm.clamp(0.5, 40.0)
}

fn err_weighted_variance(
    image: &Array2<f32>,
    err: &Array2<f32>,
    pixels: &[AperturePixel],
    excluded: Option<&Array2<u8>>,
) -> f64 {
    pixels
        .iter()
        .filter(|p| !excluded.is_some_and(|m| m[[p.y, p.x]] != 0))
        .filter(|p| image[[p.y, p.x]].is_finite())
        .map(|p| (p.weight as f64, err[[p.y, p.x]] as f64))
        .filter(|(_, e)| e.is_finite())
        .map(|(w, e)| (w * e).powi(2))
        .sum()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct GrowthPlateau {
    radius: f64,
    flux: f64,
}

fn growth_curve_plateau(
    image: &Array2<f32>,
    excluded: Option<&Array2<u8>>,
    x: f64,
    y: f64,
    r_ap: f64,
    sky_mean: f64,
    subsamples: u8,
) -> Option<GrowthPlateau> {
    let (h, w) = image.dim();
    let r_max = (GROWTH_CURVE_REACH_FACTOR * r_ap).ceil().max(0.0) as usize;
    let annulus_inner = SKY_ANNULUS_INNER_FACTOR * r_ap;
    let mut cumulative = Vec::with_capacity(r_max + 1);
    cumulative.push(0.0);
    for r in 1..=r_max {
        let ring = weighted_sum(image, &circular_aperture(h, w, x, y, r as f64, subsamples), excluded);
        cumulative.push(ring.sum - sky_mean * ring.weight);
    }
    let start = (r_ap.floor() as usize + 1).max(3);
    for r in start..=r_max {
        let flux = cumulative[r];
        let two_steps_back = cumulative[r - 2];
        if flux > 0.0 && (flux - two_steps_back).abs() < GROWTH_PLATEAU_TOLERANCE * flux {
            if r as f64 >= annulus_inner {
                return None;
            }
            return Some(GrowthPlateau { radius: r as f64, flux });
        }
    }
    None
}

pub fn measure_star_full(
    image: &Array2<f32>,
    err: Option<&Array2<f32>>,
    excluded: Option<&Array2<u8>>,
    saturated: Option<&Array2<u8>>,
    click_x: f64,
    click_y: f64,
    config: &PhotometryConfig,
) -> Result<StarPhotometry, String> {
    let dims = image.dim();
    let err = err.filter(|e| e.dim() == dims);
    let excluded = excluded.filter(|m| m.dim() == dims);
    let saturated = saturated.filter(|s| s.dim() == dims);
    let masked_owned = match excluded {
        Some(mask) => Some(apply_exclusion(image, mask).map_err(|e| e.to_string())?),
        None => None,
    };
    let working = masked_owned.as_ref().unwrap_or(image);

    let (h, w) = dims;
    if h < 16 || w < 16 {
        return Err("Image too small for photometry".into());
    }

    let cx = click_x.round().clamp(0.0, (w - 1) as f64) as usize;
    let cy = click_y.round().clamp(0.0, (h - 1) as f64) as usize;

    let r = config.search_radius.max(1);
    let mut best_x = cx;
    let mut best_y = cy;
    let mut best_v = f32::NEG_INFINITY;
    for ny in cy.saturating_sub(r)..=(cy + r).min(h - 1) {
        for nx in cx.saturating_sub(r)..=(cx + r).min(w - 1) {
            let v = working[[ny, nx]];
            if v.is_finite() && v > best_v {
                best_v = v;
                best_y = ny;
                best_x = nx;
            }
        }
    }
    if !best_v.is_finite() {
        return Err("No finite pixels near the clicked position".into());
    }

    let coarse_sky = annulus_stats(
        working,
        best_x as f64,
        best_y as f64,
        8.0,
        14.0,
        config.subsamples,
    );
    let (x, y) = refine_centroid(working, best_x as f64, best_y as f64, coarse_sky.mean, 5);

    let fwhm = fwhm_truncated_moments(working, x, y, coarse_sky.mean);
    let fwhm_eff = if fwhm > 0.0 { fwhm } else { 4.0 };

    let r_ap = config
        .aperture_radius
        .unwrap_or(1.5 * fwhm_eff)
        .clamp(2.0, 60.0);

    let sky = annulus_stats(
        working,
        x,
        y,
        r_ap * SKY_ANNULUS_INNER_FACTOR,
        r_ap * SKY_ANNULUS_OUTER_FACTOR,
        config.subsamples,
    );

    let pixels = circular_aperture(h, w, x, y, r_ap, config.subsamples);
    let ap = weighted_sum(image, &pixels, excluded);
    if ap.n_finite == 0 {
        return Err("Aperture contains no finite pixels".into());
    }

    let net_flux = ap.sum - sky.mean * ap.weight;
    let n_ap = ap.weight;
    let n_bg = sky.count as f64;
    let sky_variance = if n_bg > 0.0 {
        n_ap * n_ap * sky.sigma * sky.sigma / n_bg
    } else {
        0.0
    };
    let (variance, err_used) = match err {
        Some(err_plane) => (err_weighted_variance(image, err_plane, &pixels, excluded) + sky_variance, true),
        None => {
            let poisson = config
                .gain
                .filter(|g| g.is_finite() && *g > 0.0)
                .map_or(0.0, |g| net_flux.max(0.0) / g);
            (poisson + n_ap * sky.sigma * sky.sigma + sky_variance, false)
        }
    };
    let flux_err = variance.max(0.0).sqrt();
    let snr = if flux_err > 1e-12 {
        net_flux / flux_err
    } else {
        net_flux.max(0.0)
    };
    let mag_inst = -2.5 * net_flux.max(1e-12).log10();

    let n_saturated = saturated.map_or(0, |map| {
        pixels.iter().filter(|p| map[[p.y, p.x]] != 0).count() as u32
    });
    let saturated_flag = match saturated {
        Some(_) => n_saturated > 0,
        None => config
            .image_max
            .map_or(false, |mx| mx > 0.0 && best_v as f64 >= IMAGE_MAX_SATURATION_FRACTION * mx),
    };

    let plateau = growth_curve_plateau(image, excluded, x, y, r_ap, sky.mean, config.subsamples)
        .filter(|_| net_flux > 0.0)
        .filter(|p| (net_flux / p.flux).is_finite() && net_flux / p.flux > 0.0);
    let aperture_correction = plateau.map(|p| net_flux / p.flux);
    let flux_total = plateau.map(|p| p.flux);
    let plateau_radius = plateau.map(|p| p.radius);

    Ok(StarPhotometry {
        x,
        y,
        peak: best_v as f64,
        net_flux,
        flux_err,
        mag_inst,
        snr,
        fwhm,
        aperture_radius: r_ap,
        aperture_pixels: ap.n_finite,
        aperture_area: ap.weight,
        bg_mean: sky.mean,
        bg_sigma: sky.sigma,
        bg_pixels: sky.count,
        saturated: saturated_flag,
        n_masked: ap.n_masked,
        n_saturated,
        err_used,
        aperture_correction,
        flux_total,
        plateau_radius,
        flux_jy: None,
        flux_err_jy: None,
        mag_ab: None,
        mag_ab_err: None,
        mag_ab_total: None,
        st_mag: None,
    })
}

pub fn measure_star(
    image: &Array2<f32>,
    click_x: f64,
    click_y: f64,
    config: &PhotometryConfig,
) -> Result<StarPhotometry, String> {
    measure_star_full(image, None, None, None, click_x, click_y, config)
}

pub fn measure_star_masked(
    image: &Array2<f32>,
    click_x: f64,
    click_y: f64,
    config: &PhotometryConfig,
    excluded: Option<&Array2<u8>>,
) -> Result<StarPhotometry, String> {
    measure_star_full(image, None, excluded, None, click_x, click_y, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::analysis::aperture::ApertureSum;

    fn aperture_sum(image: &Array2<f32>, x: f64, y: f64, radius: f64, subsamples: u8) -> ApertureSum {
        let (h, w) = image.dim();
        let pixels = circular_aperture(h, w, x, y, radius, subsamples);
        weighted_sum(image, &pixels, None)
    }

    fn gaussian_scene(
        h: usize,
        w: usize,
        star_x: f64,
        star_y: f64,
        amp: f64,
        sigma: f64,
        bg: f64,
    ) -> Array2<f32> {
        Array2::from_shape_fn((h, w), |(y, x)| {
            let dx = x as f64 - star_x;
            let dy = y as f64 - star_y;
            (bg + amp * (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp()) as f32
        })
    }

    fn deterministic_noise(img: &mut Array2<f32>) {
        for ((y, x), v) in img.indexed_iter_mut() {
            let n = (((y * 7 + x * 13) % 5) as f32) - 2.0;
            *v += n;
        }
    }

    fn sum_of_squared_weights(res: &StarPhotometry, dims: (usize, usize)) -> f64 {
        circular_aperture(dims.0, dims.1, res.x, res.y, res.aperture_radius, 5)
            .iter()
            .map(|p| (p.weight as f64).powi(2))
            .sum()
    }

    #[test]
    fn test_measure_recovers_centroid_and_flux() {
        let star_x = 32.4;
        let star_y = 30.7;
        let amp = 1000.0;
        let sigma = 2.0;
        let bg = 100.0;
        let img = gaussian_scene(64, 64, star_x, star_y, amp, sigma, bg);

        let result = measure_star(&img, 31.0, 32.0, &PhotometryConfig::default()).unwrap();

        assert!((result.x - star_x).abs() < 0.1, "x={} vs {}", result.x, star_x);
        assert!((result.y - star_y).abs() < 0.1, "y={} vs {}", result.y, star_y);

        let true_flux = 2.0 * std::f64::consts::PI * amp * sigma * sigma;
        let rel_err = (result.net_flux - true_flux).abs() / true_flux;
        assert!(rel_err < 0.03, "flux={} vs {} ({:.1}%)", result.net_flux, true_flux, rel_err * 100.0);

        let true_fwhm = 2.3548 * sigma;
        assert!(
            (result.fwhm - true_fwhm).abs() / true_fwhm < 0.15,
            "fwhm={} vs {}",
            result.fwhm,
            true_fwhm
        );

        assert!(result.mag_inst.is_finite());
        assert!(!result.saturated);
        assert!(!result.err_used);
        assert_eq!(result.n_masked, 0);
        assert_eq!(result.n_saturated, 0);
        assert!(result.flux_jy.is_none() && result.mag_ab.is_none());
    }

    #[test]
    fn test_snr_with_noisy_background() {
        let mut img = gaussian_scene(64, 64, 32.0, 32.0, 5000.0, 2.0, 100.0);
        deterministic_noise(&mut img);
        let result = measure_star(&img, 32.0, 32.0, &PhotometryConfig::default()).unwrap();
        assert!(result.bg_sigma > 0.5, "bg_sigma={}", result.bg_sigma);
        assert!(result.snr > 100.0, "snr={}", result.snr);
        assert!(result.bg_pixels > 0);
    }

    #[test]
    fn test_saturation_flag() {
        let img = gaussian_scene(64, 64, 32.0, 32.0, 900.0, 2.0, 100.0);
        let cfg = PhotometryConfig {
            image_max: Some(1000.0),
            ..PhotometryConfig::default()
        };
        let result = measure_star(&img, 32.0, 32.0, &cfg).unwrap();
        assert!(result.saturated);

        let cfg_high = PhotometryConfig {
            image_max: Some(100000.0),
            ..PhotometryConfig::default()
        };
        let result2 = measure_star(&img, 32.0, 32.0, &cfg_high).unwrap();
        assert!(!result2.saturated);
    }

    #[test]
    fn test_manual_aperture_radius() {
        let img = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 50.0);
        let cfg = PhotometryConfig {
            aperture_radius: Some(10.0),
            ..PhotometryConfig::default()
        };
        let result = measure_star(&img, 32.0, 32.0, &cfg).unwrap();
        assert!((result.aperture_radius - 10.0).abs() < 1e-9);
        assert!(result.aperture_pixels > 300);
        assert!((result.aperture_area - std::f64::consts::PI * 100.0).abs() < 2.0);
    }

    #[test]
    fn test_masked_peak_moves_measurement_to_next_pixel() {
        let img = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        let plain = measure_star(&img, 32.0, 32.0, &PhotometryConfig::default()).unwrap();
        assert_eq!(plain.peak, img[[32, 32]] as f64);

        let mut mask = Array2::<u8>::zeros((64, 64));
        mask[[32, 32]] = 1;
        let masked = measure_star_masked(&img, 32.0, 32.0, &PhotometryConfig::default(), Some(&mask)).unwrap();
        assert!(masked.peak < plain.peak, "peak {} should drop below {}", masked.peak, plain.peak);
        assert!((masked.peak - img[[32, 33]] as f64).abs() < 1e-6 || (masked.peak - img[[33, 32]] as f64).abs() < 1e-6);

        let none = measure_star_masked(&img, 32.0, 32.0, &PhotometryConfig::default(), None).unwrap();
        assert_eq!(none.peak, plain.peak);
        assert_eq!(none.net_flux, plain.net_flux);

        let all = Array2::<u8>::ones((64, 64));
        assert!(measure_star_masked(&img, 32.0, 32.0, &PhotometryConfig::default(), Some(&all)).is_err());

        let wrong = Array2::<u8>::ones((8, 8));
        let ignored = measure_star_masked(&img, 32.0, 32.0, &PhotometryConfig::default(), Some(&wrong)).unwrap();
        assert_eq!(ignored.peak, plain.peak);
    }

    #[test]
    fn test_rejects_all_nan_region() {
        let mut img = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        for y in 0..20 {
            for x in 0..20 {
                img[[y, x]] = f32::NAN;
            }
        }
        assert!(measure_star(&img, 5.0, 5.0, &PhotometryConfig::default()).is_err());
    }

    #[test]
    fn default_config_uses_five_subsamples_and_no_gain() {
        let cfg = PhotometryConfig::default();
        assert_eq!(cfg.subsamples, 5);
        assert!(cfg.gain.is_none());
    }

    #[test]
    fn flat_scene_aperture_sum_matches_area_with_subsampling() {
        let img = Array2::from_elem((64, 64), 1.0f32);
        let r = 3.0;
        let area = std::f64::consts::PI * r * r;

        let fine = aperture_sum(&img, 32.3, 31.6, r, 5);
        assert!((fine.sum - fine.weight).abs() < 1e-9);
        assert!(
            (fine.sum - area).abs() / area < 0.02,
            "sum={} vs {}",
            fine.sum,
            area
        );
        assert!(fine.n_finite as f64 > fine.weight);

        let coarse = aperture_sum(&img, 32.3, 31.6, r, 1);
        let mut centre_rule_count = 0u32;
        for y in 0..64 {
            for x in 0..64 {
                let dx = x as f64 - 32.3;
                let dy = y as f64 - 31.6;
                if dx * dx + dy * dy <= r * r {
                    centre_rule_count += 1;
                }
            }
        }
        assert_eq!(coarse.n_finite, centre_rule_count);
        assert_eq!(coarse.sum, centre_rule_count as f64);
        assert!((fine.sum - area).abs() < (coarse.sum - area).abs());
    }

    #[test]
    fn aperture_sum_ignores_nan_pixels_and_reports_them() {
        let mut img = Array2::from_elem((64, 64), 2.0f32);
        img[[32, 32]] = f32::NAN;
        let ap = aperture_sum(&img, 32.0, 32.0, 4.0, 5);
        assert_eq!(ap.n_nonfinite, 1);
        assert!((ap.sum - 2.0 * ap.weight).abs() < 1e-9);
        let all_nan = Array2::from_elem((64, 64), f32::NAN);
        let empty = aperture_sum(&all_nan, 32.0, 32.0, 4.0, 5);
        assert_eq!(empty.n_finite, 0);
        assert_eq!(empty.weight, 0.0);
    }

    #[test]
    fn annulus_stats_on_flat_scene_returns_level_and_zero_sigma() {
        let img = Array2::from_elem((64, 64), 7.0f32);
        let sky = annulus_stats(&img, 32.4, 31.7, 8.0, 14.0, 5);
        assert!((sky.mean - 7.0).abs() < 1e-9);
        assert_eq!(sky.sigma, 0.0);
        assert!(sky.count > 0);
        let all_nan = Array2::from_elem((64, 64), f32::NAN);
        assert_eq!(
            annulus_stats(&all_nan, 32.0, 32.0, 8.0, 14.0, 5),
            SkyEstimate { mean: 0.0, sigma: 0.0, count: 0 }
        );
    }

    #[test]
    fn annulus_stats_excludes_star_core_inside_inner_radius() {
        let img = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        let sky = annulus_stats(&img, 32.0, 32.0, 8.0, 14.0, 5);
        assert!((sky.mean - 100.0).abs() < 0.01, "mean={}", sky.mean);
        assert!(sky.sigma < 0.01, "sigma={}", sky.sigma);
    }

    #[test]
    fn subsampled_flux_stays_within_tolerance_of_analytic_gaussian_flux() {
        let amp = 1000.0;
        let sigma = 2.0;
        let img = gaussian_scene(64, 64, 32.4, 30.7, amp, sigma, 100.0);
        let true_flux = 2.0 * std::f64::consts::PI * amp * sigma * sigma;
        for subsamples in [1u8, 5] {
            let cfg = PhotometryConfig {
                subsamples,
                ..PhotometryConfig::default()
            };
            let result = measure_star(&img, 32.0, 31.0, &cfg).unwrap();
            let rel = (result.net_flux - true_flux).abs() / true_flux;
            assert!(
                rel < 0.03,
                "subsamples={subsamples} flux={} vs {true_flux}",
                result.net_flux
            );
            assert!(result.aperture_pixels > 0);
        }
    }

    #[test]
    fn constant_err_plane_propagates_through_the_aperture_weights() {
        let img = gaussian_scene(64, 64, 32.4, 30.7, 1000.0, 2.0, 100.0);
        let err = Array2::from_elem((64, 64), 0.5f32);
        let cfg = PhotometryConfig::default();
        let res = measure_star_full(&img, Some(&err), None, None, 32.0, 31.0, &cfg).unwrap();
        assert!(res.err_used);
        assert_eq!(res.bg_sigma, 0.0);
        let expected = 0.5 * sum_of_squared_weights(&res, img.dim()).sqrt();
        assert!(
            (res.flux_err - expected).abs() < 1e-6,
            "flux_err={} expected={expected}",
            res.flux_err
        );
        assert!((res.snr - res.net_flux / expected).abs() < 1e-6);

        let mut noisy = img.clone();
        deterministic_noise(&mut noisy);
        let res = measure_star_full(&noisy, Some(&err), None, None, 32.0, 31.0, &cfg).unwrap();
        assert!(res.bg_sigma > 0.5);
        let n_ap = res.aperture_area;
        let n_bg = res.bg_pixels as f64;
        let expected = (0.25 * sum_of_squared_weights(&res, noisy.dim())
            + n_ap * n_ap * res.bg_sigma * res.bg_sigma / n_bg)
            .sqrt();
        assert!(
            (res.flux_err - expected).abs() < 1e-6,
            "flux_err={} expected={expected}",
            res.flux_err
        );

        let wrong = Array2::from_elem((8, 8), 0.5f32);
        let ignored = measure_star_full(&img, Some(&wrong), None, None, 32.0, 31.0, &cfg).unwrap();
        assert!(!ignored.err_used);

        let mut holed = err.clone();
        holed[[31, 32]] = f32::NAN;
        let partial = measure_star_full(&img, Some(&holed), None, None, 32.0, 31.0, &cfg).unwrap();
        assert!(partial.err_used);
        let first = measure_star_full(&img, Some(&err), None, None, 32.0, 31.0, &cfg).unwrap();
        assert!(partial.flux_err < first.flux_err);
    }

    #[test]
    fn background_only_error_model_uses_sky_noise_and_optional_gain() {
        let mut img = gaussian_scene(64, 64, 32.0, 32.0, 5000.0, 2.0, 100.0);
        deterministic_noise(&mut img);
        let cfg = PhotometryConfig::default();
        let res = measure_star(&img, 32.0, 32.0, &cfg).unwrap();
        assert!(!res.err_used);
        let n_ap = res.aperture_area;
        let n_bg = res.bg_pixels as f64;
        let expected = (n_ap * res.bg_sigma * res.bg_sigma * (1.0 + n_ap / n_bg)).sqrt();
        assert!((res.flux_err - expected).abs() < 1e-6, "flux_err={} expected={expected}", res.flux_err);
        assert!((res.snr - res.net_flux / expected).abs() < 1e-6);

        let with_gain = PhotometryConfig { gain: Some(2.0), ..PhotometryConfig::default() };
        let res_gain = measure_star(&img, 32.0, 32.0, &with_gain).unwrap();
        let expected_gain = (res_gain.net_flux / 2.0 + expected * expected).sqrt();
        assert!(
            (res_gain.flux_err - expected_gain).abs() < 1e-6,
            "flux_err={} expected={expected_gain}",
            res_gain.flux_err
        );
        assert!(res_gain.flux_err > res.flux_err);
    }

    #[test]
    fn dq_saturated_bit_inside_the_aperture_flags_the_star() {
        let img = gaussian_scene(64, 64, 32.0, 32.0, 900.0, 2.0, 100.0);
        let low_max = PhotometryConfig { image_max: Some(1000.0), ..PhotometryConfig::default() };

        let mut sat = Array2::<u8>::zeros((64, 64));
        sat[[32, 33]] = 1;
        let res = measure_star_full(&img, None, None, Some(&sat), 32.0, 32.0, &low_max).unwrap();
        assert!(res.saturated);
        assert_eq!(res.n_saturated, 1);

        let clean = Array2::<u8>::zeros((64, 64));
        let res = measure_star_full(&img, None, None, Some(&clean), 32.0, 32.0, &low_max).unwrap();
        assert!(!res.saturated, "a DQ plane without the bit overrides the image_max rule");
        assert_eq!(res.n_saturated, 0);

        let fallback = measure_star_full(&img, None, None, None, 32.0, 32.0, &low_max).unwrap();
        assert!(fallback.saturated);
        assert_eq!(fallback.n_saturated, 0);

        let mut far = Array2::<u8>::zeros((64, 64));
        far[[5, 5]] = 1;
        let res = measure_star_full(&img, None, None, Some(&far), 32.0, 32.0, &PhotometryConfig::default()).unwrap();
        assert!(!res.saturated);
        assert_eq!(res.n_saturated, 0);
    }

    #[test]
    fn excluded_pixels_inside_the_aperture_are_counted() {
        let img = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        let mut mask = Array2::<u8>::zeros((64, 64));
        mask[[30, 30]] = 1;
        mask[[30, 31]] = 1;
        mask[[5, 5]] = 1;
        let cfg = PhotometryConfig { aperture_radius: Some(6.0), ..PhotometryConfig::default() };
        let res = measure_star_full(&img, None, Some(&mask), None, 32.0, 32.0, &cfg).unwrap();
        assert_eq!(res.n_masked, 2);
        let plain = measure_star(&img, 32.0, 32.0, &cfg).unwrap();
        assert_eq!(plain.n_masked, 0);
        assert!(res.aperture_pixels < plain.aperture_pixels);
        assert!(res.aperture_area < plain.aperture_area);
        assert!(res.net_flux < plain.net_flux);
    }

    #[test]
    fn aperture_correction_from_the_growth_curve_approaches_one_with_radius() {
        let img = gaussian_scene(128, 128, 64.0, 64.0, 1000.0, 3.0, 10.0);
        let at = |r: f64| {
            let cfg = PhotometryConfig { aperture_radius: Some(r), ..PhotometryConfig::default() };
            measure_star(&img, 64.0, 64.0, &cfg).unwrap()
        };
        let wide = at(8.0);
        let wider = at(10.0);
        let corr8 = wide.aperture_correction.expect("correction at r=8");
        let corr10 = wider.aperture_correction.expect("correction at r=10");
        assert!(corr8 > 0.9 && corr8 < 1.0, "corr8={corr8}");
        assert!(corr10 > corr8 && corr10 < 1.0, "corr10={corr10} corr8={corr8}");
        let total = wide.flux_total.expect("total flux");
        assert!((total - wide.net_flux / corr8).abs() < 1e-6);
        let plateau = wide.plateau_radius.expect("plateau radius");
        assert!(plateau > 8.0 && plateau < 16.0, "plateau={plateau}");
        let true_flux = 2.0 * std::f64::consts::PI * 1000.0 * 9.0;
        assert!((total - true_flux).abs() / true_flux < 0.02, "total={total} vs {true_flux}");

        let narrow = at(4.0);
        assert!(narrow.aperture_correction.is_none(), "plateau inside the sky annulus must not be trusted");
        assert!(narrow.flux_total.is_none());

        let flat = Array2::from_elem((128, 128), 10.0f32);
        let cfg = PhotometryConfig { aperture_radius: Some(8.0), ..PhotometryConfig::default() };
        let none = measure_star(&flat, 64.0, 64.0, &cfg).unwrap();
        assert!(none.aperture_correction.is_none());
    }
}
