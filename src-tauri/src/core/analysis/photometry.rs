use std::f64::consts::LN_2;

use ndarray::Array2;
use serde::Serialize;

use crate::core::analysis::aperture::{annulus, circular_aperture, weighted_sum, AperturePixel, ApertureSum};
use crate::core::imaging::dq_flags::apply_exclusion;
use crate::math::gauss_newton::solve_linear;
use crate::types::constants::MAD_TO_SIGMA;
use crate::types::header::HduHeader;

const ANNULUS_MEMBERSHIP_WEIGHT: f32 = 0.5;
pub const SKY_ANNULUS_INNER_FACTOR: f64 = 2.0;
pub const SKY_ANNULUS_OUTER_FACTOR: f64 = 3.0;
pub const GROWTH_CURVE_REACH_FACTOR: f64 = 4.0;
pub const GROWTH_PLATEAU_TOLERANCE: f64 = 0.01;
pub const GROWTH_CURVE_STEP: f64 = 0.5;
pub const GROWTH_CURVE_MAX_POINTS: usize = 160;
const GROWTH_PLATEAU_LOOKBACK: f64 = 2.0;
const GROWTH_PLATEAU_MIN_RADIUS: f64 = 3.0;
pub const SATURATION_KEYWORDS: [&str; 5] = ["SATURATE", "SATLEVEL", "SATURATION", "MAXLIN", "DATAMAX"];
const DATAMAX_KEYWORD: &str = "DATAMAX";
const DATAMAX_IS_IMAGE_MAX_TOLERANCE: f64 = 1e-6;
const COARSE_SKY_INNER: f64 = 8.0;
const COARSE_SKY_OUTER: f64 = 14.0;
const MIN_REFINED_SKY_PIXELS: u32 = 16;
pub const IMAGE_MAX_SATURATION_SOURCE: &str = "image maximum";
pub const DQ_SATURATION_SOURCE: &str = "DQ SATURATED";
pub const NO_SATURATION_SOURCE: &str = "none";
const HEADER_SATURATION_FRACTION: f64 = 0.95;
const FLAT_TOP_TOLERANCE: f64 = 1e-3;
const FLAT_TOP_MIN_PIXELS: usize = 2;
pub const MIN_APERTURE_RADIUS: f64 = 2.0;
pub const MAX_APERTURE_RADIUS: f64 = 60.0;
pub const AUTO_APERTURE_FWHM_FACTOR: f64 = 1.5;
const FWHM_FIT_REACH_FACTOR: f64 = 1.2;
const FWHM_FIT_MIN_REACH: f64 = 2.5;
const FWHM_FIT_MAX_REACH: f64 = 48.0;
const FWHM_FIT_PASSES: usize = 6;
const FWHM_FIT_RIDGE: f64 = 1e-12;

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
    pub saturation: Option<(f64, &'static str)>,
    pub subsamples: u8,
    pub gain: Option<f64>,
    pub sky_annulus: Option<(f64, f64)>,
    pub fixed_position: bool,
}

impl Default for PhotometryConfig {
    fn default() -> Self {
        Self {
            search_radius: 8,
            aperture_radius: None,
            saturation: None,
            subsamples: 5,
            gain: None,
            sky_annulus: None,
            fixed_position: false,
        }
    }
}

pub fn saturation_level(header: Option<&HduHeader>, image_max: f64) -> (f64, &'static str) {
    if let Some(header) = header {
        for keyword in SATURATION_KEYWORDS {
            if let Some(level) = header.get_f64(keyword).filter(|v| v.is_finite() && *v > 0.0) {
                let records_the_data = keyword == DATAMAX_KEYWORD
                    && (level - image_max).abs() <= DATAMAX_IS_IMAGE_MAX_TOLERANCE * level.abs().max(image_max.abs());
                if records_the_data {
                    return (image_max, IMAGE_MAX_SATURATION_SOURCE);
                }
                return (level, keyword);
            }
        }
    }
    (image_max, IMAGE_MAX_SATURATION_SOURCE)
}

fn flat_top_pixel_count(
    image: &Array2<f32>,
    pixels: &[AperturePixel],
    excluded: Option<&Array2<u8>>,
    level: f64,
) -> usize {
    let floor = level * (1.0 - FLAT_TOP_TOLERANCE);
    pixels
        .iter()
        .filter(|p| !excluded.is_some_and(|m| m[[p.y, p.x]] != 0))
        .map(|p| image[[p.y, p.x]] as f64)
        .filter(|v| v.is_finite() && *v >= floor)
        .count()
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
    pub fwhm: Option<f64>,
    pub aperture_radius: f64,
    pub aperture_pixels: u32,
    pub aperture_area: f64,
    pub bg_mean: f64,
    pub bg_sigma: f64,
    pub bg_pixels: u32,
    pub saturated: bool,
    pub saturation_source: String,
    pub n_masked: u32,
    pub n_nonfinite: u32,
    pub n_saturated: u32,
    pub err_used: bool,
    pub aperture_correction: Option<f64>,
    pub ee_in_aperture: Option<f64>,
    pub flux_total: Option<f64>,
    pub plateau_radius: Option<f64>,
    pub flux_jy: Option<f64>,
    pub flux_err_jy: Option<f64>,
    pub mag_ab: Option<f64>,
    pub mag_ab_err: Option<f64>,
    pub mag_ab_total: Option<f64>,
    pub st_mag: Option<f64>,
    pub sky_inner: f64,
    pub sky_outer: f64,
    pub growth_curve: Vec<GrowthPoint>,
    pub ee50_radius: Option<f64>,
    pub ee80_radius: Option<f64>,
    pub area_applied: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct GrowthPoint {
    pub r: f64,
    pub flux: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SkyLevel {
    pub mean: f64,
    pub sigma: f64,
    pub count: u32,
}

fn annulus_stats(
    image: &Array2<f32>,
    x: f64,
    y: f64,
    inner_r: f64,
    outer_r: f64,
    subsamples: u8,
) -> SkyLevel {
    let (h, w) = image.dim();
    let mut vals: Vec<f64> = annulus(h, w, x, y, inner_r, outer_r, subsamples)
        .into_iter()
        .filter(|p| p.weight >= ANNULUS_MEMBERSHIP_WEIGHT)
        .map(|p| image[[p.y, p.x]])
        .filter(|v| v.is_finite())
        .map(|v| v as f64)
        .collect();

    if vals.is_empty() {
        return SkyLevel { mean: 0.0, sigma: 0.0, count: 0 };
    }

    vals.sort_by(|a, b| a.total_cmp(b));
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
    devs.sort_by(|a, b| a.total_cmp(b));
    let sigma = sorted_median(&devs) * MAD_TO_SIGMA;

    SkyLevel { mean, sigma, count: vals.len() as u32 }
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

fn half_maximum_radius_squared(slope: f64, curvature: f64) -> Option<f64> {
    let drop_rate = (slope * slope - 4.0 * curvature * LN_2).sqrt() - slope;
    (drop_rate > 0.0).then(|| 2.0 * LN_2 / drop_rate)
}

fn fwhm_radial_fit(image: &Array2<f32>, x: f64, y: f64, bg: f64, first_guess: f64) -> Option<f64> {
    if !(first_guess > 0.0) {
        return None;
    }
    let (h, w) = image.dim();
    let (cx, cy) = (x.round() as i64, y.round() as i64);
    let mut reach = (FWHM_FIT_REACH_FACTOR * first_guess).clamp(FWHM_FIT_MIN_REACH, FWHM_FIT_MAX_REACH);
    let mut profile: Option<[f64; 3]> = None;
    let mut fwhm = None;
    for _ in 0..FWHM_FIT_PASSES {
        let reach2 = reach * reach;
        let half = reach.ceil() as i64;
        let mut normal = [[0.0f64; 3]; 3];
        let mut rhs = [0.0f64; 3];
        for ny in (cy - half).max(0)..=(cy + half).min(h as i64 - 1) {
            for nx in (cx - half).max(0)..=(cx + half).min(w as i64 - 1) {
                let t = (nx as f64 - x).powi(2) + (ny as f64 - y).powi(2);
                let u = t / reach2;
                let v = image[[ny as usize, nx as usize]] as f64 - bg;
                if u >= 1.0 || !v.is_finite() {
                    continue;
                }
                let (weight, weighted_log) = match profile {
                    None if v > 0.0 => (v * v, v * v * v.ln()),
                    None => continue,
                    Some([level, slope, curvature]) => {
                        let t_model = if curvature > 0.0 { t.min(-slope / (2.0 * curvature)) } else { t };
                        let eta = level + slope * t_model + curvature * t_model * t_model;
                        let mu = eta.exp();
                        (mu * mu, mu * mu * eta + mu * (v - mu))
                    }
                };
                let taper = (1.0 - u) * (1.0 - u);
                let basis = [1.0, u, u * u];
                for i in 0..3 {
                    rhs[i] += taper * weighted_log * basis[i];
                    for j in 0..3 {
                        normal[i][j] += taper * weight * basis[i] * basis[j];
                    }
                }
            }
        }
        normal[1][1] += FWHM_FIT_RIDGE * normal[0][0];
        normal[2][2] += FWHM_FIT_RIDGE * normal[0][0];
        let solution = solve_linear(&mut normal.map(|row| row.to_vec()), &mut rhs)?;
        let (slope, curvature) = (solution[1] / reach2, solution[2] / (reach2 * reach2));
        let t_half = half_maximum_radius_squared(slope, curvature)?;
        profile = Some([solution[0], slope, curvature]);
        let width = 2.0 * t_half.sqrt();
        fwhm = (t_half < reach2 && (0.5..=40.0).contains(&width)).then_some(width);
        reach = (FWHM_FIT_REACH_FACTOR * width).clamp(FWHM_FIT_MIN_REACH, FWHM_FIT_MAX_REACH);
    }
    fwhm
}

pub(crate) fn area_factor(value: f32) -> f64 {
    let area = value as f64;
    if area.is_finite() && area > 0.0 {
        area
    } else {
        1.0
    }
}

pub fn area_map_mean(area: &Array2<f32>) -> f64 {
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for &value in area.iter() {
        let a = value as f64;
        if a.is_finite() && a > 0.0 {
            sum += a;
            count += 1;
        }
    }
    if count == 0 {
        f64::NAN
    } else {
        sum / count as f64
    }
}

fn area_at(p: &AperturePixel, area: Option<&Array2<f32>>) -> f64 {
    area.map_or(1.0, |map| area_factor(map[[p.y, p.x]]))
}

struct ApertureFlux {
    sum: f64,
    weight: f64,
    geometric_weight: f64,
    n_finite: u32,
    n_masked: u32,
    n_nonfinite: u32,
}

impl ApertureFlux {
    fn geometric(ap: &ApertureSum) -> Self {
        Self {
            sum: ap.sum,
            weight: ap.weight,
            geometric_weight: ap.weight,
            n_finite: ap.n_finite,
            n_masked: ap.n_masked,
            n_nonfinite: ap.n_nonfinite,
        }
    }

    fn net(&self, sky_mean: f64) -> f64 {
        self.sum - sky_mean * self.weight
    }
}

fn area_weighted_sum(
    image: &Array2<f32>,
    area: &Array2<f32>,
    pixels: &[AperturePixel],
    excluded: Option<&Array2<u8>>,
) -> ApertureFlux {
    let mask = excluded.filter(|m| m.dim() == image.dim());
    let mut flux = ApertureFlux { sum: 0.0, weight: 0.0, geometric_weight: 0.0, n_finite: 0, n_masked: 0, n_nonfinite: 0 };
    for p in pixels {
        let Some(&value) = image.get((p.y, p.x)) else {
            continue;
        };
        if mask.is_some_and(|m| m[[p.y, p.x]] != 0) {
            flux.n_masked += 1;
            continue;
        }
        if !value.is_finite() {
            flux.n_nonfinite += 1;
            continue;
        }
        let geometric = p.weight as f64;
        let w = geometric * area_factor(area[[p.y, p.x]]);
        flux.sum += w * value as f64;
        flux.weight += w;
        flux.geometric_weight += geometric;
        flux.n_finite += 1;
    }
    flux
}

fn aperture_flux(
    image: &Array2<f32>,
    area: Option<&Array2<f32>>,
    pixels: &[AperturePixel],
    excluded: Option<&Array2<u8>>,
) -> ApertureFlux {
    match area {
        Some(map) => area_weighted_sum(image, map, pixels, excluded),
        None => ApertureFlux::geometric(&weighted_sum(image, pixels, excluded)),
    }
}

fn err_weighted_variance(
    image: &Array2<f32>,
    err: &Array2<f32>,
    area: Option<&Array2<f32>>,
    pixels: &[AperturePixel],
    excluded: Option<&Array2<u8>>,
    model_variance: impl Fn(f64) -> f64,
) -> f64 {
    pixels
        .iter()
        .filter(|p| !excluded.is_some_and(|m| m[[p.y, p.x]] != 0))
        .filter_map(|p| {
            let v = image[[p.y, p.x]] as f64;
            if !v.is_finite() {
                return None;
            }
            let e = err[[p.y, p.x]] as f64;
            let geometric = p.weight as f64;
            let a = area_at(p, area);
            Some(if e.is_finite() { (geometric * a).powi(2) * e * e } else { geometric * geometric * a * model_variance(v) })
        })
        .sum()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct GrowthPlateau {
    radius: f64,
    flux: f64,
}

pub fn growth_curve(
    image: &Array2<f32>,
    excluded: Option<&Array2<u8>>,
    area: Option<&Array2<f32>>,
    x: f64,
    y: f64,
    r_max: f64,
    sky_mean: f64,
    subsamples: u8,
) -> Vec<GrowthPoint> {
    if !r_max.is_finite() || r_max <= 0.0 || !x.is_finite() || !y.is_finite() {
        return Vec::new();
    }
    let (h, w) = image.dim();
    let area = area.filter(|a| a.dim() == image.dim());
    let step = GROWTH_CURVE_STEP.max(r_max / GROWTH_CURVE_MAX_POINTS as f64);
    let n = ((r_max / step).floor() as usize).min(GROWTH_CURVE_MAX_POINTS);
    (1..=n)
        .map(|k| {
            let r = k as f64 * step;
            let flux = aperture_flux(image, area, &circular_aperture(h, w, x, y, r, subsamples), excluded);
            GrowthPoint { r, flux: flux.net(sky_mean) }
        })
        .collect()
}

fn flux_nearest_radius(curve: &[GrowthPoint], before: usize, target: f64) -> f64 {
    let mut best_flux = 0.0;
    let mut best_distance = target.abs();
    for point in &curve[..before] {
        let distance = (point.r - target).abs();
        if distance < best_distance {
            best_distance = distance;
            best_flux = point.flux;
        }
    }
    best_flux
}

fn growth_plateau(curve: &[GrowthPoint], r_ap: f64, sky_inner: f64) -> Option<GrowthPlateau> {
    let start_radius = (r_ap.floor() + 1.0).max(GROWTH_PLATEAU_MIN_RADIUS);
    for (i, point) in curve.iter().enumerate() {
        if point.r < start_radius {
            continue;
        }
        let earlier = flux_nearest_radius(curve, i, point.r - GROWTH_PLATEAU_LOOKBACK);
        if point.flux > 0.0 && (point.flux - earlier).abs() < GROWTH_PLATEAU_TOLERANCE * point.flux {
            if point.r >= sky_inner {
                return None;
            }
            return Some(GrowthPlateau { radius: point.r, flux: point.flux });
        }
    }
    None
}

pub fn encircled_radius(curve: &[GrowthPoint], total: f64, fraction: f64) -> Option<f64> {
    if !(total.is_finite() && total > 0.0 && fraction.is_finite()) {
        return None;
    }
    let target = fraction * total;
    let hit = curve.iter().position(|p| p.flux >= target)?;
    let (r0, f0) = if hit == 0 { (0.0, 0.0) } else { (curve[hit - 1].r, curve[hit - 1].flux) };
    let (r1, f1) = (curve[hit].r, curve[hit].flux);
    let span = f1 - f0;
    if !(span > 0.0) || !span.is_finite() {
        return Some(r1);
    }
    let t = ((target - f0) / span).clamp(0.0, 1.0);
    Some(r0 + t * (r1 - r0))
}

pub struct MaskedImage<'a> {
    image: &'a Array2<f32>,
    excluded: Option<&'a Array2<u8>>,
    masked: Option<Array2<f32>>,
    area: Option<&'a Array2<f32>>,
}

impl<'a> MaskedImage<'a> {
    pub fn new(image: &'a Array2<f32>, excluded: Option<&'a Array2<u8>>) -> Result<Self, String> {
        let excluded = excluded.filter(|m| m.dim() == image.dim());
        let masked = excluded
            .map(|mask| apply_exclusion(image, mask))
            .transpose()
            .map_err(|e| e.to_string())?;
        Ok(Self { image, excluded, masked, area: None })
    }

    pub fn with_area(mut self, area: Option<&'a Array2<f32>>) -> Self {
        self.area = area.filter(|a| a.dim() == self.image.dim());
        self
    }

    fn working(&self) -> &Array2<f32> {
        self.masked.as_ref().unwrap_or(self.image)
    }

    pub(crate) fn image(&self) -> &'a Array2<f32> {
        self.image
    }

    pub(crate) fn excluded(&self) -> Option<&'a Array2<u8>> {
        self.excluded
    }

    pub(crate) fn area(&self) -> Option<&'a Array2<f32>> {
        self.area
    }
}

pub(crate) fn sky_in_annulus(
    source: &MaskedImage,
    x: f64,
    y: f64,
    r_in: f64,
    r_out: f64,
    subsamples: u8,
) -> Option<SkyLevel> {
    let sky = annulus_stats(source.working(), x, y, r_in, r_out, subsamples);
    (sky.count > 0).then_some(sky)
}

fn fwhm_above_coarse_sky(working: &Array2<f32>, x: f64, y: f64, coarse_sky_mean: f64, subsamples: u8) -> Option<f64> {
    let fwhm_above = |sky: f64| fwhm_radial_fit(working, x, y, sky, fwhm_truncated_moments(working, x, y, sky));
    let mut fwhm = fwhm_above(coarse_sky_mean);
    if let Some(coarse_fwhm) = fwhm.filter(|f| SKY_ANNULUS_INNER_FACTOR * f > COARSE_SKY_INNER) {
        let wide_sky = annulus_stats(
            working,
            x,
            y,
            SKY_ANNULUS_INNER_FACTOR * coarse_fwhm,
            SKY_ANNULUS_OUTER_FACTOR * coarse_fwhm,
            subsamples,
        );
        if wide_sky.count >= MIN_REFINED_SKY_PIXELS {
            fwhm = fwhm_above(wide_sky.mean);
        }
    }
    fwhm
}

pub(crate) fn fwhm_at_position(source: &MaskedImage, x: f64, y: f64, subsamples: u8) -> Option<f64> {
    let working = source.working();
    let coarse_sky = annulus_stats(working, x, y, COARSE_SKY_INNER, COARSE_SKY_OUTER, subsamples);
    fwhm_above_coarse_sky(working, x, y, coarse_sky.mean, subsamples)
}

pub(crate) struct SaturationVerdict {
    pub saturated: bool,
    pub source: &'static str,
    pub n_saturated: u32,
}

pub(crate) fn saturation_verdict(
    source: &MaskedImage,
    saturated: Option<&Array2<u8>>,
    pixels: &[AperturePixel],
    peak: f64,
    level: Option<(f64, &'static str)>,
) -> SaturationVerdict {
    let n_saturated = saturated.map_or(0, |map| {
        pixels.iter().filter(|p| map[[p.y, p.x]] != 0).count() as u32
    });
    let level = level.filter(|(level, _)| level.is_finite() && *level > 0.0);
    let (saturated_flag, saturation_source) = match (saturated, level) {
        (Some(_), _) => (n_saturated > 0, DQ_SATURATION_SOURCE),
        (None, Some((level, source_name))) if source_name == IMAGE_MAX_SATURATION_SOURCE => (
            flat_top_pixel_count(source.image, pixels, source.excluded, level) >= FLAT_TOP_MIN_PIXELS,
            source_name,
        ),
        (None, Some((level, source_name))) => (peak >= HEADER_SATURATION_FRACTION * level, source_name),
        (None, None) => (false, NO_SATURATION_SOURCE),
    };
    SaturationVerdict { saturated: saturated_flag, source: saturation_source, n_saturated }
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
    measure_star_prepared(&MaskedImage::new(image, excluded)?, err, saturated, click_x, click_y, config)
}

pub fn measure_star_prepared(
    source: &MaskedImage,
    err: Option<&Array2<f32>>,
    saturated: Option<&Array2<u8>>,
    click_x: f64,
    click_y: f64,
    config: &PhotometryConfig,
) -> Result<StarPhotometry, String> {
    let image = source.image();
    let excluded = source.excluded();
    let area = source.area();
    let working = source.working();
    let dims = image.dim();
    let (h, w) = dims;
    let on_image = click_x >= -0.5 && click_x < w as f64 - 0.5 && click_y >= -0.5 && click_y < h as f64 - 0.5;
    if !on_image {
        return Err(format!("position ({click_x}, {click_y}) lies outside the {w} x {h} image"));
    }
    let err = err.filter(|e| e.dim() == dims);
    let saturated = saturated.filter(|s| s.dim() == dims);

    if h < 16 || w < 16 {
        return Err("Image too small for photometry".into());
    }

    let (x, y, fwhm, r_ap, searched_peak) = if config.fixed_position {
        let radius = config
            .aperture_radius
            .ok_or("a fixed-position measurement needs an explicit aperture radius")?;
        if !(MIN_APERTURE_RADIUS..=MAX_APERTURE_RADIUS).contains(&radius) {
            return Err(format!(
                "aperture radius {radius} px is outside the {MIN_APERTURE_RADIUS} to {MAX_APERTURE_RADIUS} px limits of a fixed-position measurement"
            ));
        }
        (click_x, click_y, fwhm_at_position(source, click_x, click_y, config.subsamples), radius, None)
    } else {
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
            COARSE_SKY_INNER,
            COARSE_SKY_OUTER,
            config.subsamples,
        );
        let (x, y) = refine_centroid(working, best_x as f64, best_y as f64, coarse_sky.mean, 5);
        let fwhm = fwhm_above_coarse_sky(working, x, y, coarse_sky.mean, config.subsamples);
        let r_ap = config
            .aperture_radius
            .unwrap_or(AUTO_APERTURE_FWHM_FACTOR * fwhm.unwrap_or(4.0))
            .clamp(MIN_APERTURE_RADIUS, MAX_APERTURE_RADIUS);
        (x, y, fwhm, r_ap, Some(best_v as f64))
    };
    let mut warnings = Vec::new();
    if fwhm.is_none() && config.aperture_radius.is_none() {
        warnings.push(format!(
            "FWHM not measured: the automatic aperture radius of {r_ap:.1} px is a fallback, not {AUTO_APERTURE_FWHM_FACTOR} x FWHM"
        ));
    }

    let (sky_inner, sky_outer) = match config.sky_annulus {
        Some((r_in, r_out)) => {
            if !(r_in.is_finite() && r_in > r_ap) {
                return Err(format!("sky annulus inner radius {r_in} must be larger than the aperture radius {r_ap}"));
            }
            if !(r_out.is_finite() && r_out > r_in) {
                return Err(format!("sky annulus outer radius {r_out} must be larger than the inner radius {r_in}"));
            }
            (r_in, r_out)
        }
        None => (r_ap * SKY_ANNULUS_INNER_FACTOR, r_ap * SKY_ANNULUS_OUTER_FACTOR),
    };
    let Some(sky) = sky_in_annulus(source, x, y, sky_inner, sky_outer, config.subsamples) else {
        return Err(format!(
            "sky annulus {sky_inner:.1} - {sky_outer:.1} px around ({x:.1}, {y:.1}) has no usable pixel: none with at least half its area inside it is on the image, finite and unmasked"
        ));
    };

    let pixels = circular_aperture(h, w, x, y, r_ap, config.subsamples);
    let ap = aperture_flux(image, area, &pixels, excluded);
    if ap.n_finite == 0 {
        return Err("Aperture contains no finite pixels".into());
    }
    let peak = searched_peak.unwrap_or_else(|| {
        pixels
            .iter()
            .map(|p| working[[p.y, p.x]])
            .filter(|v| v.is_finite())
            .fold(f32::NEG_INFINITY, f32::max) as f64
    });
    if ap.n_nonfinite > 0 {
        warnings.push(format!(
            "{} aperture pixel{} without data (non-finite): the flux is a lower limit",
            ap.n_nonfinite,
            if ap.n_nonfinite == 1 { "" } else { "s" }
        ));
    }

    let net_flux = ap.net(sky.mean);
    let n_ap = ap.weight;
    let n_bg = sky.count as f64;
    let sky_variance = if n_bg > 0.0 {
        n_ap * n_ap * sky.sigma * sky.sigma / n_bg
    } else {
        0.0
    };
    let gain = config.gain.filter(|g| g.is_finite() && *g > 0.0);
    let (variance, err_used) = match err {
        Some(err_plane) => {
            let model_variance = |v: f64| {
                sky.sigma * sky.sigma + gain.map_or(0.0, |g| (v - sky.mean).max(0.0) / g)
            };
            (err_weighted_variance(image, err_plane, area, &pixels, excluded, model_variance) + sky_variance, true)
        }
        None => {
            let poisson = gain.map_or(0.0, |g| net_flux.max(0.0) / g);
            (poisson + n_ap * sky.sigma * sky.sigma + sky_variance, false)
        }
    };
    let flux_err = variance.max(0.0).sqrt();
    let snr = if flux_err > 0.0 {
        net_flux / flux_err
    } else {
        net_flux.max(0.0)
    };
    let mag_inst = if net_flux > 0.0 { -2.5 * net_flux.log10() } else { f64::NAN };

    let saturation = saturation_verdict(source, saturated, &pixels, peak, config.saturation);

    let curve = growth_curve(image, excluded, area, x, y, GROWTH_CURVE_REACH_FACTOR * r_ap, sky.mean, config.subsamples);
    let plateau = growth_plateau(&curve, r_ap, sky_inner)
        .filter(|_| net_flux > 0.0)
        .filter(|p| (net_flux / p.flux).is_finite() && net_flux / p.flux > 0.0);
    let aperture_correction = plateau.map(|p| p.flux / net_flux);
    let ee_in_aperture = plateau.map(|p| net_flux / p.flux);
    let flux_total = plateau.map(|p| p.flux);
    let plateau_radius = plateau.map(|p| p.radius);
    let ee50_radius = flux_total.and_then(|total| encircled_radius(&curve, total, 0.5));
    let ee80_radius = flux_total.and_then(|total| encircled_radius(&curve, total, 0.8));

    Ok(StarPhotometry {
        x,
        y,
        peak,
        net_flux,
        flux_err,
        mag_inst,
        snr,
        fwhm,
        aperture_radius: r_ap,
        aperture_pixels: ap.geometric_weight.round() as u32,
        aperture_area: ap.geometric_weight,
        bg_mean: sky.mean,
        bg_sigma: sky.sigma,
        bg_pixels: sky.count,
        saturated: saturation.saturated,
        saturation_source: saturation.source.to_string(),
        n_masked: ap.n_masked,
        n_nonfinite: ap.n_nonfinite,
        n_saturated: saturation.n_saturated,
        err_used,
        aperture_correction,
        ee_in_aperture,
        flux_total,
        plateau_radius,
        flux_jy: None,
        flux_err_jy: None,
        mag_ab: None,
        mag_ab_err: None,
        mag_ab_total: None,
        st_mag: None,
        sky_inner,
        sky_outer,
        growth_curve: curve,
        ee50_radius,
        ee80_radius,
        area_applied: area.is_some(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure_star(
        image: &Array2<f32>,
        click_x: f64,
        click_y: f64,
        config: &PhotometryConfig,
    ) -> Result<StarPhotometry, String> {
        measure_star_full(image, None, None, None, click_x, click_y, config)
    }

    fn measure_star_masked(
        image: &Array2<f32>,
        click_x: f64,
        click_y: f64,
        config: &PhotometryConfig,
        excluded: Option<&Array2<u8>>,
    ) -> Result<StarPhotometry, String> {
        measure_star_full(image, None, excluded, None, click_x, click_y, config)
    }

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

    fn measured_fwhm(res: &StarPhotometry) -> f64 {
        res.fwhm.expect("the FWHM of a star is measured")
    }

    fn sum_of_squared_weights(res: &StarPhotometry, dims: (usize, usize)) -> f64 {
        circular_aperture(dims.0, dims.1, res.x, res.y, res.aperture_radius, 5)
            .iter()
            .map(|p| (p.weight as f64).powi(2))
            .sum()
    }

    #[test]
    fn a_brighter_neighbour_beyond_the_eight_pixel_search_takes_over_the_measurement() {
        let mut img = gaussian_scene(128, 128, 64.0, 64.0, 1000.0, 2.0, 100.0);
        let row = gaussian_scene(128, 128, 74.0, 64.0, 2000.0, 2.0, 0.0);
        img.zip_mut_with(&row, |a, b| *a += *b);
        let res = measure_star(&img, 64.0, 64.0, &PhotometryConfig::default()).unwrap();
        assert!((res.x - 74.0).abs() < 0.2 && (res.y - 64.0).abs() < 0.2, "({}, {})", res.x, res.y);

        let mut diag = gaussian_scene(128, 128, 64.0, 64.0, 1000.0, 2.0, 100.0);
        let corner = gaussian_scene(128, 128, 73.0, 73.0, 2000.0, 2.0, 0.0);
        diag.zip_mut_with(&corner, |a, b| *a += *b);
        let res = measure_star(&diag, 64.0, 64.0, &PhotometryConfig::default()).unwrap();
        assert!((res.x - 73.0).abs() < 0.2 && (res.y - 73.0).abs() < 0.2, "({}, {})", res.x, res.y);
        assert_eq!(PhotometryConfig::default().search_radius, 8);
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
        let fwhm = measured_fwhm(&result);
        assert!((fwhm - true_fwhm).abs() / true_fwhm < 0.15, "fwhm={} vs {}", fwhm, true_fwhm);

        assert!(result.mag_inst.is_finite());
        assert!(!result.saturated);
        assert_eq!(result.saturation_source, NO_SATURATION_SOURCE);
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
    fn header_saturation_level_flags_a_peak_near_the_level() {
        let img = gaussian_scene(64, 64, 32.0, 32.0, 900.0, 2.0, 100.0);
        let cfg = PhotometryConfig {
            saturation: Some((1000.0, "SATURATE")),
            ..PhotometryConfig::default()
        };
        let result = measure_star(&img, 32.0, 32.0, &cfg).unwrap();
        assert!(result.saturated);
        assert_eq!(result.saturation_source, "SATURATE");

        let cfg_high = PhotometryConfig {
            saturation: Some((100000.0, "SATURATE")),
            ..PhotometryConfig::default()
        };
        let result2 = measure_star(&img, 32.0, 32.0, &cfg_high).unwrap();
        assert!(!result2.saturated);
        assert_eq!(result2.saturation_source, "SATURATE");

        let cfg_invalid = PhotometryConfig {
            saturation: Some((f64::NAN, "SATURATE")),
            ..PhotometryConfig::default()
        };
        let result3 = measure_star(&img, 32.0, 32.0, &cfg_invalid).unwrap();
        assert!(!result3.saturated);
        assert_eq!(result3.saturation_source, NO_SATURATION_SOURCE);
    }

    #[test]
    fn saturation_level_prefers_header_keywords_over_the_image_maximum() {
        let mut header = HduHeader::empty();
        assert_eq!(saturation_level(Some(&header), 70000.0), (70000.0, IMAGE_MAX_SATURATION_SOURCE));
        assert_eq!(saturation_level(None, 70000.0), (70000.0, IMAGE_MAX_SATURATION_SOURCE));

        header.set("SATURATE", "60000".to_string());
        assert_eq!(saturation_level(Some(&header), 70000.0), (60000.0, "SATURATE"));

        header.set("DATAMAX", "65535".to_string());
        assert_eq!(saturation_level(Some(&header), 70000.0), (60000.0, "SATURATE"));

        header.set("SATURATE", "0".to_string());
        assert_eq!(saturation_level(Some(&header), 70000.0), (65535.0, "DATAMAX"));

        header.set("DATAMAX", "'full'".to_string());
        header.set("MAXLIN", "58000.5".to_string());
        assert_eq!(saturation_level(Some(&header), 70000.0), (58000.5, "MAXLIN"));

        header.set("SATLEVEL", "-1".to_string());
        header.set("SATURATION", "59000".to_string());
        assert_eq!(saturation_level(Some(&header), 70000.0), (59000.0, "SATURATION"));
    }

    #[test]
    fn datamax_ranks_below_maxlin_and_a_datamax_equal_to_the_data_maximum_is_not_a_saturation_level() {
        let mut header = HduHeader::empty();
        header.set("DATAMAX", "65535".to_string());
        header.set("MAXLIN", "58000".to_string());
        assert_eq!(saturation_level(Some(&header), 70000.0), (58000.0, "MAXLIN"));

        let mut observed = HduHeader::empty();
        observed.set("DATAMAX", "1000.0".to_string());
        assert_eq!(saturation_level(Some(&observed), 1000.0), (1000.0, IMAGE_MAX_SATURATION_SOURCE));
        assert_eq!(saturation_level(Some(&observed), 1200.0), (1000.0, "DATAMAX"));

        let img = gaussian_scene(64, 64, 32.0, 32.0, 900.0, 2.0, 100.0);
        let cfg = PhotometryConfig {
            saturation: Some(saturation_level(Some(&observed), 1000.0)),
            ..PhotometryConfig::default()
        };
        let result = measure_star(&img, 32.0, 32.0, &cfg).unwrap();
        assert!(!result.saturated, "the brightest star was flagged from a DATAMAX that only records the data maximum");
        assert_eq!(result.saturation_source, IMAGE_MAX_SATURATION_SOURCE);
    }

    #[test]
    fn a_non_positive_net_flux_has_no_instrumental_magnitude() {
        let mut hollow = Array2::from_elem((64, 64), 10.0f32);
        hollow.slice_mut(ndarray::s![20..44, 20..44]).fill(9.0);
        let res = measure_star(&hollow, 32.0, 32.0, &PhotometryConfig::default()).unwrap();
        assert!(res.net_flux <= 0.0, "net_flux={}", res.net_flux);
        assert!(res.mag_inst.is_nan(), "mag_inst={} reported for a net flux of {}", res.mag_inst, res.net_flux);
        assert!(serde_json::to_value(&res).unwrap()["mag_inst"].is_null());

        let img = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        let star = measure_star(&img, 32.0, 32.0, &PhotometryConfig::default()).unwrap();
        assert!((star.mag_inst - (-2.5 * star.net_flux.log10())).abs() < 1e-12);
    }

    #[test]
    fn a_wide_source_is_measured_against_a_sky_outside_its_own_wings() {
        for sigma in [6.0f64, 8.0] {
            let img = gaussian_scene(160, 160, 80.0, 80.0, 1000.0, sigma, 100.0);
            let res = measure_star(&img, 80.0, 80.0, &PhotometryConfig::default()).unwrap();
            let true_fwhm = 2.3548200450309493 * sigma;
            let fwhm = measured_fwhm(&res);
            assert!((fwhm - true_fwhm).abs() / true_fwhm < 0.05, "sigma {sigma}: FWHM {fwhm} against a true {true_fwhm}");
        }
    }

    #[test]
    fn brightest_star_of_an_unsaturated_field_is_not_flagged_from_the_image_maximum() {
        let img = gaussian_scene(64, 64, 32.0, 32.0, 900.0, 2.0, 100.0);
        let image_max = img.iter().cloned().fold(f32::NEG_INFINITY, f32::max) as f64;
        assert_eq!(image_max, 1000.0);
        let cfg = PhotometryConfig {
            saturation: Some(saturation_level(None, image_max)),
            ..PhotometryConfig::default()
        };
        let result = measure_star(&img, 32.0, 32.0, &cfg).unwrap();
        assert_eq!(result.peak, image_max);
        assert!(!result.saturated, "a single peak pixel at the image maximum is not a flat top");
        assert_eq!(result.saturation_source, IMAGE_MAX_SATURATION_SOURCE);
    }

    #[test]
    fn clipped_flat_top_star_is_flagged_from_the_image_maximum() {
        let mut img = gaussian_scene(64, 64, 32.0, 32.0, 1100.0, 2.0, 100.0);
        let clip = 1000.0f32;
        img.mapv_inplace(|v| v.min(clip));
        let clipped: Vec<(usize, usize)> = img
            .indexed_iter()
            .filter(|(_, v)| **v == clip)
            .map(|(idx, _)| idx)
            .collect();
        assert_eq!(clipped.len(), 5, "centre plus its four neighbours sit at the clip level");
        let cfg = PhotometryConfig {
            saturation: Some(saturation_level(None, clip as f64)),
            ..PhotometryConfig::default()
        };
        let result = measure_star(&img, 32.0, 32.0, &cfg).unwrap();
        assert!(result.saturated);
        assert_eq!(result.saturation_source, IMAGE_MAX_SATURATION_SOURCE);
        assert_eq!(result.n_saturated, 0, "n_saturated counts DQ pixels only");

        let mut mask = Array2::<u8>::zeros((64, 64));
        for (y, x) in clipped {
            mask[[y, x]] = 1;
        }
        let masked = measure_star_masked(&img, 32.0, 32.0, &cfg, Some(&mask)).unwrap();
        assert!(!masked.saturated, "excluded pixels do not count toward the flat top");
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
        assert_eq!(result.aperture_pixels, result.aperture_area.round() as u32);
        assert!((result.aperture_area - std::f64::consts::PI * 100.0).abs() < 2.0);
    }

    #[test]
    fn aperture_pixels_is_the_rounded_effective_area_not_the_touched_pixel_count() {
        let img = gaussian_scene(64, 64, 32.3, 31.6, 1000.0, 2.0, 50.0);
        let cfg = PhotometryConfig {
            aperture_radius: Some(3.0),
            ..PhotometryConfig::default()
        };
        let result = measure_star(&img, 32.0, 32.0, &cfg).unwrap();
        let touched = circular_aperture(64, 64, result.x, result.y, 3.0, 5).len() as u32;
        let area = std::f64::consts::PI * 9.0;
        assert_eq!(result.aperture_pixels, result.aperture_area.round() as u32);
        assert!((result.aperture_pixels as f64 - area).abs() <= 1.0, "{} px vs {area}", result.aperture_pixels);
        assert!(touched > result.aperture_pixels, "{touched} touched pixels vs {} effective", result.aperture_pixels);
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
        let fixed = PhotometryConfig {
            fixed_position: true,
            aperture_radius: Some(3.0),
            sky_annulus: Some((30.0, 40.0)),
            ..PhotometryConfig::default()
        };
        assert_eq!(measure_star(&img, 5.0, 5.0, &fixed).unwrap_err(), "Aperture contains no finite pixels");
    }

    #[test]
    fn default_config_uses_five_subsamples_and_no_gain() {
        let cfg = PhotometryConfig::default();
        assert_eq!(cfg.subsamples, 5);
        assert!(cfg.gain.is_none());
        assert!(!cfg.fixed_position);
    }

    fn flat_sky_with_a_hot_pixel_five_pixels_right_of_the_target() -> Array2<f32> {
        let mut img = Array2::from_elem((64, 64), 128.0f32);
        img[[40, 45]] = 5128.0;
        img
    }

    fn faint_target_beside_a_sharp_bright_neighbour() -> Array2<f32> {
        let mut img = gaussian_scene(96, 96, 40.0, 48.0, 400.0, 2.0, 100.0);
        let neighbour = gaussian_scene(96, 96, 46.0, 48.0, 4900.0, 1.0, 0.0);
        img.zip_mut_with(&neighbour, |a, b| *a += *b);
        img
    }

    #[test]
    fn a_fixed_position_measurement_stays_on_the_requested_pixel_next_to_a_brighter_neighbour() {
        let mut img = gaussian_scene(128, 128, 64.0, 64.0, 1000.0, 2.0, 100.0);
        let neighbour = gaussian_scene(128, 128, 70.0, 64.0, 2000.0, 2.0, 0.0);
        img.zip_mut_with(&neighbour, |a, b| *a += *b);
        let searching = PhotometryConfig { aperture_radius: Some(3.0), ..PhotometryConfig::default() };
        let fixed = PhotometryConfig { fixed_position: true, ..searching.clone() };

        let a = measure_star(&img, 64.0, 64.0, &searching).unwrap();
        assert!(a.x > 68.0 && (a.y - 64.0).abs() < 0.1, "({}, {})", a.x, a.y);

        let b = measure_star(&img, 64.0, 64.0, &fixed).unwrap();
        assert_eq!((b.x, b.y), (64.0, 64.0));
        assert_eq!(b.aperture_radius, 3.0);
        let expected = aperture_sum(&img, 64.0, 64.0, 3.0, 5).sum - b.bg_mean * b.aperture_area;
        assert!((b.net_flux - expected).abs() <= 1e-9 * expected.abs(), "net={} expected={expected}", b.net_flux);
        assert!((b.bg_mean - 107.08).abs() < 0.01, "bg_mean={}", b.bg_mean);
        assert!((b.net_flux - 18632.08).abs() < 0.01, "net={}", b.net_flux);
    }

    #[test]
    fn a_fixed_position_on_empty_sky_returns_zero_flux_and_no_fwhm() {
        let img = flat_sky_with_a_hot_pixel_five_pixels_right_of_the_target();
        let searching = PhotometryConfig {
            aperture_radius: Some(3.0),
            sky_annulus: Some((8.0, 12.0)),
            ..PhotometryConfig::default()
        };
        let fixed = PhotometryConfig { fixed_position: true, ..searching.clone() };

        let empty = measure_star(&img, 40.0, 40.0, &fixed).unwrap();
        assert_eq!((empty.x, empty.y), (40.0, 40.0));
        assert_eq!(empty.net_flux, 0.0);
        assert_eq!(empty.flux_err, 0.0);
        assert_eq!(empty.snr, 0.0);
        assert!(empty.mag_inst.is_nan(), "mag_inst={}", empty.mag_inst);
        assert!(empty.fwhm.is_none(), "fwhm={:?}", empty.fwhm);
        assert_eq!(empty.peak, 128.0);
        assert!(empty.warnings.is_empty(), "{:?}", empty.warnings);

        let pulled = measure_star(&img, 40.0, 40.0, &searching).unwrap();
        assert_eq!((pulled.x, pulled.y), (45.0, 40.0));
        assert!((pulled.net_flux - 5000.0).abs() < 1e-6, "net={}", pulled.net_flux);
    }

    #[test]
    fn a_fixed_position_needs_an_explicit_radius_inside_the_aperture_limits() {
        let img = flat_sky_with_a_hot_pixel_five_pixels_right_of_the_target();
        let fixed_with = |aperture_radius: Option<f64>| PhotometryConfig {
            fixed_position: true,
            aperture_radius,
            ..PhotometryConfig::default()
        };
        assert_eq!(
            measure_star(&img, 40.0, 40.0, &fixed_with(None)).unwrap_err(),
            "a fixed-position measurement needs an explicit aperture radius"
        );
        assert_eq!(
            measure_star(&img, 40.0, 40.0, &fixed_with(Some(1.5))).unwrap_err(),
            "aperture radius 1.5 px is outside the 2 to 60 px limits of a fixed-position measurement"
        );
        assert_eq!(
            measure_star(&img, 40.0, 40.0, &fixed_with(Some(60.5))).unwrap_err(),
            "aperture radius 60.5 px is outside the 2 to 60 px limits of a fixed-position measurement"
        );
        assert_eq!(measure_star(&img, 40.0, 40.0, &fixed_with(Some(2.0))).unwrap().aperture_radius, 2.0);

        let searching = PhotometryConfig { aperture_radius: Some(1.0), ..PhotometryConfig::default() };
        assert_eq!(measure_star(&img, 40.0, 40.0, &searching).unwrap().aperture_radius, 2.0);
    }

    #[test]
    fn a_fixed_position_reports_the_brightest_aperture_pixel_as_the_peak() {
        let img = faint_target_beside_a_sharp_bright_neighbour();
        let searching = PhotometryConfig {
            aperture_radius: Some(3.0),
            sky_annulus: Some((10.0, 14.0)),
            saturation: Some((1000.0, "SATURATE")),
            ..PhotometryConfig::default()
        };
        let fixed = PhotometryConfig { fixed_position: true, ..searching.clone() };

        let target = measure_star(&img, 40.0, 48.0, &fixed).unwrap();
        assert!((target.peak - 500.0).abs() < 0.01, "peak={}", target.peak);
        assert!(!target.saturated);
        assert_eq!(target.saturation_source, "SATURATE");

        let pulled = measure_star(&img, 40.0, 48.0, &searching).unwrap();
        assert!(pulled.saturated);
        assert!(pulled.peak > 4900.0, "peak={}", pulled.peak);
    }

    #[test]
    fn a_fixed_position_measures_the_fwhm_at_the_requested_position() {
        let img = gaussian_scene(128, 128, 64.0, 64.0, 1000.0, 3.0, 100.0);
        let fixed = PhotometryConfig { fixed_position: true, aperture_radius: Some(8.0), ..PhotometryConfig::default() };
        let fwhm = measured_fwhm(&measure_star(&img, 64.0, 64.0, &fixed).unwrap());
        assert!((fwhm - 7.0645).abs() / 7.0645 < 0.01, "FWHM {fwhm} against 2.35482 x 3");
    }

    #[test]
    fn the_fwhm_helper_equals_the_fwhm_of_a_fixed_position_measurement() {
        let img = gaussian_scene(128, 128, 64.0, 64.0, 1000.0, 3.0, 100.0);
        let fixed = PhotometryConfig { fixed_position: true, aperture_radius: Some(8.0), ..PhotometryConfig::default() };
        let measured = measured_fwhm(&measure_star(&img, 64.0, 64.0, &fixed).unwrap());
        let helper = fwhm_at_position(&MaskedImage::new(&img, None).unwrap(), 64.0, 64.0, 5)
            .expect("the helper measures the FWHM of a star");
        assert_eq!(helper.to_bits(), measured.to_bits(), "helper {helper} against {measured}");
    }

    #[test]
    fn the_sky_helper_returns_the_annulus_level_and_none_without_pixels() {
        let img = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        let sky = sky_in_annulus(&MaskedImage::new(&img, None).unwrap(), 32.0, 32.0, 8.0, 14.0, 5).unwrap();
        assert!((sky.mean - 100.0).abs() < 0.01, "mean={}", sky.mean);
        assert!(sky.sigma < 0.01, "sigma={}", sky.sigma);
        assert!(sky.count > 0);

        let blank = Array2::from_elem((64, 64), f32::NAN);
        assert!(sky_in_annulus(&MaskedImage::new(&blank, None).unwrap(), 32.0, 32.0, 8.0, 14.0, 5).is_none());
    }

    #[test]
    fn the_saturation_helper_reproduces_the_three_rules() {
        let img = faint_target_beside_a_sharp_bright_neighbour();
        let source = MaskedImage::new(&img, None).unwrap();
        let pixels = circular_aperture(96, 96, 40.0, 48.0, 3.0, 5);
        let header_level = Some((1000.0, "SATURATE"));

        let mut dq = Array2::<u8>::zeros((96, 96));
        dq[[48, 41]] = 1;
        dq[[48, 60]] = 1;
        let flagged = saturation_verdict(&source, Some(&dq), &pixels, 500.0, header_level);
        assert!(flagged.saturated);
        assert_eq!(flagged.source, "DQ SATURATED");
        assert_eq!(flagged.n_saturated, 1);

        let below = saturation_verdict(&source, None, &pixels, 500.0, header_level);
        assert!(!below.saturated);
        assert_eq!(below.source, "SATURATE");
        assert_eq!(below.n_saturated, 0);

        let above = saturation_verdict(&source, None, &pixels, 960.0, header_level);
        assert!(above.saturated);
        assert_eq!(above.source, "SATURATE");

        let unknown = saturation_verdict(&source, None, &pixels, 960.0, None);
        assert!(!unknown.saturated);
        assert_eq!(unknown.source, "none");
        assert_eq!(unknown.n_saturated, 0);
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
            SkyLevel { mean: 0.0, sigma: 0.0, count: 0 }
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
        let partial = measure_star_full(&noisy, Some(&holed), None, None, 32.0, 31.0, &cfg).unwrap();
        assert!(partial.err_used);
        assert_eq!(partial.net_flux, res.net_flux, "the SCI pixel under the ERR hole still counts as flux");
        let pixels = circular_aperture(64, 64, partial.x, partial.y, partial.aperture_radius, 5);
        let hole_w2: f64 = pixels.iter().filter(|p| (p.y, p.x) == (31, 32)).map(|p| (p.weight as f64).powi(2)).sum();
        assert!(hole_w2 > 0.0, "the hole must sit inside the aperture");
        let others_w2: f64 = pixels.iter().filter(|p| (p.y, p.x) != (31, 32)).map(|p| (p.weight as f64).powi(2)).sum();
        let n_ap = partial.aperture_area;
        let n_bg = partial.bg_pixels as f64;
        let sky2 = partial.bg_sigma * partial.bg_sigma;
        let expected = (0.25 * others_w2 + hole_w2 * sky2 + n_ap * n_ap * sky2 / n_bg).sqrt();
        assert!(
            (partial.flux_err - expected).abs() < 1e-6,
            "flux_err={} expected={expected}: the pixel without ERR lost its variance",
            partial.flux_err
        );
    }

    #[test]
    fn an_area_map_enters_the_model_variance_of_a_pixel_without_err_once() {
        let mut img = gaussian_scene(64, 64, 32.0, 32.0, 5000.0, 2.0, 100.0);
        deterministic_noise(&mut img);
        let scale = f64::from(1.02f32);
        let area = Array2::from_elem((64, 64), 1.02f32);
        let all_nan = Array2::from_elem((64, 64), f32::NAN);
        let gain = 2.0;
        let cfg = PhotometryConfig { gain: Some(gain), ..PhotometryConfig::default() };
        let source = MaskedImage::new(&img, None).unwrap().with_area(Some(&area));
        let res = measure_star_prepared(&source, Some(&all_nan), None, 32.0, 32.0, &cfg).unwrap();
        assert!(res.err_used && res.area_applied);
        let sky2 = res.bg_sigma * res.bg_sigma;
        let model: f64 = circular_aperture(64, 64, res.x, res.y, res.aperture_radius, 5)
            .iter()
            .map(|p| {
                let g = p.weight as f64;
                let v = img[[p.y, p.x]] as f64;
                g * g * scale * (sky2 + (v - res.bg_mean).max(0.0) / gain)
            })
            .sum();
        let n_ap = scale * res.aperture_area;
        let expected = (model + n_ap * n_ap * sky2 / res.bg_pixels as f64).sqrt();
        assert!(
            (res.flux_err - expected).abs() < 1e-9 * expected,
            "flux_err={} expected={expected}: a pixel of relative area a carries a times the model variance of a unit pixel, not a^2",
            res.flux_err
        );
    }

    #[test]
    fn a_pixel_without_err_gets_the_background_model_variance_plus_poisson_with_gain() {
        let mut img = gaussian_scene(64, 64, 32.0, 32.0, 5000.0, 2.0, 100.0);
        deterministic_noise(&mut img);
        let all_nan = Array2::from_elem((64, 64), f32::NAN);
        let cfg = PhotometryConfig { gain: Some(2.0), ..PhotometryConfig::default() };
        let with_holes = measure_star_full(&img, Some(&all_nan), None, None, 32.0, 32.0, &cfg).unwrap();
        let without_err = measure_star_full(&img, None, None, None, 32.0, 32.0, &cfg).unwrap();
        assert!(with_holes.err_used);
        assert!(
            with_holes.flux_err > 0.5 * without_err.flux_err,
            "an all-NaN ERR plane reported {} against the model's {}",
            with_holes.flux_err,
            without_err.flux_err
        );
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
        let low_max = PhotometryConfig { saturation: Some((1000.0, "SATURATE")), ..PhotometryConfig::default() };

        let mut sat = Array2::<u8>::zeros((64, 64));
        sat[[32, 33]] = 1;
        let res = measure_star_full(&img, None, None, Some(&sat), 32.0, 32.0, &low_max).unwrap();
        assert!(res.saturated);
        assert_eq!(res.n_saturated, 1);
        assert_eq!(res.saturation_source, DQ_SATURATION_SOURCE);

        let clean = Array2::<u8>::zeros((64, 64));
        let res = measure_star_full(&img, None, None, Some(&clean), 32.0, 32.0, &low_max).unwrap();
        assert!(!res.saturated, "a DQ plane without the bit overrides the header level rule");
        assert_eq!(res.n_saturated, 0);
        assert_eq!(res.saturation_source, DQ_SATURATION_SOURCE);

        let fallback = measure_star_full(&img, None, None, None, 32.0, 32.0, &low_max).unwrap();
        assert!(fallback.saturated);
        assert_eq!(fallback.n_saturated, 0);
        assert_eq!(fallback.saturation_source, "SATURATE");

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
        assert!(corr8 > 1.0 && corr8 < 1.1, "corr8={corr8}");
        assert!(corr10 > 1.0 && corr10 < corr8, "corr10={corr10} corr8={corr8}");
        let total = wide.flux_total.expect("total flux");
        assert!((total - wide.net_flux * corr8).abs() < 1e-6);
        let enclosed8 = serde_json::to_value(&wide).unwrap()["ee_in_aperture"].as_f64().expect("enclosed fraction at r=8");
        assert!(enclosed8 > 0.9 && enclosed8 < 1.0, "enclosed8={enclosed8}");
        assert!((enclosed8 * corr8 - 1.0).abs() < 1e-12);
        let plateau = wide.plateau_radius.expect("plateau radius");
        assert!(plateau > 8.0 && plateau < 16.0, "plateau={plateau}");
        let true_flux = 2.0 * std::f64::consts::PI * 1000.0 * 9.0;
        assert!((total - true_flux).abs() / true_flux < 0.02, "total={total} vs {true_flux}");

        let narrow = at(4.0);
        assert!(narrow.aperture_correction.is_none(), "plateau inside the sky annulus must not be trusted");
        assert!(narrow.flux_total.is_none());
        assert!(serde_json::to_value(&narrow).unwrap()["ee_in_aperture"].is_null());

        let flat = Array2::from_elem((128, 128), 10.0f32);
        let cfg = PhotometryConfig { aperture_radius: Some(8.0), ..PhotometryConfig::default() };
        let none = measure_star(&flat, 64.0, 64.0, &cfg).unwrap();
        assert!(none.aperture_correction.is_none());
    }

    #[test]
    fn an_aperture_enclosing_93_percent_reports_the_multiplicative_correction_above_one() {
        let img = gaussian_scene(128, 128, 64.0, 64.0, 1000.0, 3.0, 10.0);
        let cfg = PhotometryConfig { aperture_radius: Some(6.92), ..PhotometryConfig::default() };
        let res = measure_star(&img, 64.0, 64.0, &cfg).unwrap();
        let total = res.flux_total.expect("total flux");
        let correction = res.aperture_correction.expect("aperture correction");
        assert!(correction > 1.0, "correction {correction} would make the total fainter than the aperture flux");
        assert!((correction - total / res.net_flux).abs() < 1e-12, "correction {correction} against total over net {}", total / res.net_flux);
        let brighter_mag = 2.5 * correction.log10();
        assert!((brighter_mag - 0.0788).abs() < 0.003, "the total is {brighter_mag} mag brighter than the aperture flux");
        let enclosed = serde_json::to_value(&res).unwrap()["ee_in_aperture"].as_f64().expect("enclosed fraction");
        assert!((enclosed - 0.93).abs() < 0.005, "enclosed fraction {enclosed}");
        assert!((enclosed - res.net_flux / total).abs() < 1e-12);
    }

    #[test]
    fn growth_curve_of_a_noiseless_gaussian_is_non_decreasing_and_ends_near_the_total_flux() {
        let sigma = 3.0;
        let amp = 1000.0;
        let img = gaussian_scene(128, 128, 64.0, 64.0, amp, sigma, 10.0);
        let curve = growth_curve(&img, None, None, 64.0, 64.0, 24.0, 10.0, 5);
        assert_eq!(curve.len(), 48);
        assert!((curve[0].r - GROWTH_CURVE_STEP).abs() < 1e-12);
        for pair in curve.windows(2) {
            assert!(pair[1].flux >= pair[0].flux - 1e-6, "flux fell from {} to {}", pair[0].flux, pair[1].flux);
            assert!(pair[1].r > pair[0].r);
        }
        let true_flux = 2.0 * std::f64::consts::PI * amp * sigma * sigma;
        let last = curve.last().unwrap().flux;
        assert!((last - true_flux).abs() / true_flux < 0.01, "last={last} vs {true_flux}");
        assert!(growth_curve(&img, None, None, 64.0, 64.0, 0.0, 10.0, 5).is_empty());
        assert!(growth_curve(&img, None, None, 64.0, 64.0, f64::NAN, 10.0, 5).is_empty());
        let wide = growth_curve(&img, None, None, 64.0, 64.0, 240.0, 10.0, 1);
        assert!(wide.len() <= GROWTH_CURVE_MAX_POINTS);
        assert!((wide[0].r - 1.5).abs() < 1e-12);
    }

    #[test]
    fn encircled_energy_radii_of_a_gaussian_match_the_analytic_values() {
        let sigma = 3.0;
        let img = gaussian_scene(128, 128, 64.0, 64.0, 1000.0, sigma, 10.0);
        let cfg = PhotometryConfig { aperture_radius: Some(8.0), ..PhotometryConfig::default() };
        let res = measure_star(&img, 64.0, 64.0, &cfg).unwrap();
        assert!(res.flux_total.is_some());
        let ee50 = res.ee50_radius.expect("EE50");
        let ee80 = res.ee80_radius.expect("EE80");
        let expected50 = 1.1774 * sigma;
        let expected80 = 1.7941 * sigma;
        assert!((ee50 - expected50).abs() / expected50 < 0.03, "EE50 {ee50} vs {expected50}");
        assert!((ee80 - expected80).abs() / expected80 < 0.03, "EE80 {ee80} vs {expected80}");
        assert!(ee50 < ee80);
        assert_eq!(res.growth_curve.len(), 64);
        assert!((res.sky_inner - 16.0).abs() < 1e-12 && (res.sky_outer - 24.0).abs() < 1e-12);

        let narrow = PhotometryConfig { aperture_radius: Some(4.0), ..PhotometryConfig::default() };
        let no_plateau = measure_star(&img, 64.0, 64.0, &narrow).unwrap();
        assert!(no_plateau.flux_total.is_none());
        assert!(no_plateau.ee50_radius.is_none() && no_plateau.ee80_radius.is_none());
        assert!(!no_plateau.growth_curve.is_empty());
    }

    #[test]
    fn encircled_radius_interpolates_and_rejects_unreachable_fractions() {
        let curve = [
            GrowthPoint { r: 1.0, flux: 10.0 },
            GrowthPoint { r: 2.0, flux: 30.0 },
            GrowthPoint { r: 3.0, flux: 40.0 },
        ];
        assert!((encircled_radius(&curve, 40.0, 0.5).unwrap() - 1.5).abs() < 1e-12);
        assert!((encircled_radius(&curve, 40.0, 0.125).unwrap() - 0.5).abs() < 1e-12);
        assert!((encircled_radius(&curve, 40.0, 1.0).unwrap() - 3.0).abs() < 1e-12);
        assert!(encircled_radius(&curve, 40.0, 1.5).is_none());
        assert!(encircled_radius(&curve, 0.0, 0.5).is_none());
        assert!(encircled_radius(&curve, f64::NAN, 0.5).is_none());
        assert!(encircled_radius(&[], 40.0, 0.5).is_none());
        let flat = [GrowthPoint { r: 1.0, flux: 5.0 }, GrowthPoint { r: 2.0, flux: 5.0 }];
        assert!((encircled_radius(&flat, 5.0, 1.0).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn an_explicit_sky_annulus_equal_to_the_auto_factors_reproduces_the_auto_path() {
        let mut img = gaussian_scene(96, 96, 48.3, 47.6, 2000.0, 2.5, 100.0);
        deterministic_noise(&mut img);
        let r_ap = 6.0;
        let auto = PhotometryConfig { aperture_radius: Some(r_ap), ..PhotometryConfig::default() };
        let explicit = PhotometryConfig {
            aperture_radius: Some(r_ap),
            sky_annulus: Some((SKY_ANNULUS_INNER_FACTOR * r_ap, SKY_ANNULUS_OUTER_FACTOR * r_ap)),
            ..PhotometryConfig::default()
        };
        let a = measure_star(&img, 48.0, 48.0, &auto).unwrap();
        let b = measure_star(&img, 48.0, 48.0, &explicit).unwrap();
        assert!((a.net_flux - b.net_flux).abs() < 1e-9);
        assert!((a.flux_err - b.flux_err).abs() < 1e-9);
        assert!((a.bg_mean - b.bg_mean).abs() < 1e-9);
        assert_eq!(a.plateau_radius, b.plateau_radius);
        assert_eq!(a.sky_inner, b.sky_inner);
        assert_eq!(a.sky_outer, b.sky_outer);

        let wider = PhotometryConfig {
            aperture_radius: Some(r_ap),
            sky_annulus: Some((20.0, 30.0)),
            ..PhotometryConfig::default()
        };
        let c = measure_star(&img, 48.0, 48.0, &wider).unwrap();
        assert_eq!((c.sky_inner, c.sky_outer), (20.0, 30.0));
        assert_ne!(c.bg_pixels, a.bg_pixels);
    }

    #[test]
    fn a_sky_annulus_inside_the_aperture_or_with_no_width_is_refused() {
        let img = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        let inside = PhotometryConfig {
            aperture_radius: Some(5.0),
            sky_annulus: Some((5.0, 10.0)),
            ..PhotometryConfig::default()
        };
        let err = measure_star(&img, 32.0, 32.0, &inside).unwrap_err();
        assert_eq!(err, "sky annulus inner radius 5 must be larger than the aperture radius 5");

        let empty = PhotometryConfig {
            aperture_radius: Some(5.0),
            sky_annulus: Some((8.0, 8.0)),
            ..PhotometryConfig::default()
        };
        let err = measure_star(&img, 32.0, 32.0, &empty).unwrap_err();
        assert_eq!(err, "sky annulus outer radius 8 must be larger than the inner radius 8");

        let nan = PhotometryConfig {
            aperture_radius: Some(5.0),
            sky_annulus: Some((f64::NAN, 12.0)),
            ..PhotometryConfig::default()
        };
        assert!(measure_star(&img, 32.0, 32.0, &nan).is_err());
    }

    #[test]
    fn growth_plateau_keeps_the_integer_step_semantics_of_the_previous_search() {
        let img = gaussian_scene(128, 128, 64.0, 64.0, 1000.0, 3.0, 10.0);
        let r_ap = 8.0;
        let curve = growth_curve(&img, None, None, 64.0, 64.0, GROWTH_CURVE_REACH_FACTOR * r_ap, 10.0, 5);
        let plateau = growth_plateau(&curve, r_ap, SKY_ANNULUS_INNER_FACTOR * r_ap).expect("plateau");
        assert!(plateau.radius >= 11.0 && plateau.radius <= 12.0, "plateau at {}", plateau.radius);
        assert!((plateau.radius / GROWTH_CURVE_STEP).fract().abs() < 1e-12);
        assert!(growth_plateau(&curve, r_ap, plateau.radius).is_none());
        assert!(growth_plateau(&[], r_ap, 16.0).is_none());
    }

    #[test]
    fn a_position_outside_the_image_is_refused_instead_of_measuring_the_nearest_edge_star() {
        let img = gaussian_scene(64, 64, 60.0, 32.0, 1000.0, 2.0, 100.0);
        let cfg = PhotometryConfig::default();
        let inside = measure_star(&img, 60.0, 32.0, &cfg).unwrap();
        assert!((inside.x - 60.0).abs() < 0.5, "x={}", inside.x);
        let err = measure_star(&img, 3000.0, 32.0, &cfg).unwrap_err();
        assert_eq!(err, "position (3000, 32) lies outside the 64 x 64 image");
        let fixed = PhotometryConfig { fixed_position: true, ..PhotometryConfig::default() };
        assert_eq!(measure_star(&img, 3000.0, 32.0, &fixed).unwrap_err(), err);
        assert!(measure_star(&img, -500.0, 10.0, &cfg).is_err());
        assert!(measure_star(&img, 63.5, 32.0, &cfg).is_err());
        assert!(measure_star(&img, 32.0, -0.51, &cfg).is_err());
        assert!(measure_star(&img, f64::NAN, 32.0, &cfg).is_err());
        assert!(measure_star(&img, 63.4, 32.0, &cfg).is_ok());
        assert!(measure_star(&img, -0.5, 32.0, &cfg).is_ok());
    }

    #[test]
    fn an_explicit_sky_annulus_with_no_usable_pixel_is_refused_instead_of_skipping_the_sky() {
        let img = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        let with_annulus = |r_in: f64, r_out: f64| PhotometryConfig {
            aperture_radius: Some(5.0),
            sky_annulus: Some((r_in, r_out)),
            ..PhotometryConfig::default()
        };
        let err = measure_star(&img, 32.0, 32.0, &with_annulus(10.0, 10.3)).unwrap_err();
        assert!(err.starts_with("sky annulus 10.0 - 10.3 px around (32.0, 32.0) has no usable pixel"), "{err}");
        let off_image = measure_star(&img, 32.0, 32.0, &with_annulus(100.0, 110.0)).unwrap_err();
        assert!(off_image.starts_with("sky annulus 100.0 - 110.0 px"), "{off_image}");
        let half_pixel = measure_star(&img, 32.0, 32.0, &with_annulus(10.0, 10.5)).unwrap();
        assert!(half_pixel.bg_pixels > 0);
        assert!((half_pixel.bg_mean - 100.0).abs() < 0.01, "bg_mean={}", half_pixel.bg_mean);
        let enclosed = 2.0 * std::f64::consts::PI * 1000.0 * 4.0 * (1.0 - (-25.0f64 / 8.0).exp());
        assert!((half_pixel.net_flux - enclosed).abs() / enclosed < 0.01, "net={}", half_pixel.net_flux);
    }

    #[test]
    fn an_automatic_sky_annulus_on_masked_or_blank_pixels_is_refused() {
        let mut img = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        for ((y, x), v) in img.indexed_iter_mut() {
            if (x as f64 - 32.0).hypot(y as f64 - 32.0) > 11.0 {
                *v = f32::NAN;
            }
        }
        let cfg = PhotometryConfig { aperture_radius: Some(6.0), ..PhotometryConfig::default() };
        let err = measure_star(&img, 32.0, 32.0, &cfg).unwrap_err();
        assert!(err.starts_with("sky annulus 12.0 - 18.0 px around"), "{err}");

        let clean = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        let ring: Array2<u8> = Array2::from_shape_fn((64, 64), |(y, x)| u8::from((x as f64 - 32.0).hypot(y as f64 - 32.0) > 10.0));
        let masked = measure_star_masked(&clean, 32.0, 32.0, &cfg, Some(&ring)).unwrap_err();
        assert!(masked.starts_with("sky annulus 12.0 - 18.0 px around"), "{masked}");
        assert!(measure_star_masked(&clean, 32.0, 32.0, &cfg, None).unwrap().bg_pixels > 0);
    }

    #[test]
    fn a_prepared_masked_image_is_built_once_and_measures_exactly_like_the_full_call() {
        let mut img = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        deterministic_noise(&mut img);
        let mut mask = Array2::<u8>::zeros((64, 64));
        mask[[31, 33]] = 1;
        mask[[32, 44]] = 1;
        let prepared = MaskedImage::new(&img, Some(&mask)).unwrap();
        let working = prepared.working();
        assert!(working[[31, 33]].is_nan() && working[[32, 44]].is_nan());
        assert_eq!(working[[32, 32]], img[[32, 32]]);
        let cfg = PhotometryConfig { aperture_radius: Some(5.0), sky_annulus: Some((10.0, 15.0)), ..PhotometryConfig::default() };
        for (x, y) in [(32.0, 32.0), (31.2, 33.4), (20.0, 45.0)] {
            let once = measure_star_prepared(&prepared, None, None, x, y, &cfg).unwrap();
            let every_call = measure_star_full(&img, None, Some(&mask), None, x, y, &cfg).unwrap();
            assert_eq!(serde_json::to_value(&once).unwrap(), serde_json::to_value(&every_call).unwrap(), "({x}, {y})");
        }
        assert_eq!(measure_star_prepared(&prepared, None, None, 32.0, 32.0, &cfg).unwrap().n_masked, 1);

        let wrong = Array2::<u8>::ones((8, 8));
        let ignored = MaskedImage::new(&img, Some(&wrong)).unwrap();
        assert!(ignored.masked.is_none() && ignored.excluded.is_none());
    }

    #[test]
    fn non_finite_pixels_inside_the_aperture_are_counted_and_make_the_flux_a_lower_limit() {
        let clean = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        let cfg = PhotometryConfig { aperture_radius: Some(6.0), ..PhotometryConfig::default() };
        let whole = measure_star(&clean, 32.0, 32.0, &cfg).unwrap();

        let mut edge = clean.clone();
        for y in 31..=33 {
            edge[[y, 35]] = f32::NAN;
        }
        let partial = measure_star(&edge, 32.0, 32.0, &cfg).unwrap();
        let json = serde_json::to_value(&partial).unwrap();
        assert_eq!(json["n_nonfinite"], 3, "net flux {} against {} with no count of the missing pixels", partial.net_flux, whole.net_flux);
        assert_eq!(
            json["warnings"],
            serde_json::json!(["3 aperture pixels without data (non-finite): the flux is a lower limit"])
        );
        assert!(partial.net_flux < whole.net_flux && partial.aperture_area < whole.aperture_area);
        assert_eq!(partial.n_masked, 0);

        let whole_json = serde_json::to_value(&whole).unwrap();
        assert_eq!(whole_json["n_nonfinite"], 0);
        assert_eq!(whole_json["warnings"], serde_json::json!([]));

        let area = Array2::from_elem((64, 64), 1.02f32);
        let source = MaskedImage::new(&edge, None).unwrap().with_area(Some(&area));
        let weighted = serde_json::to_value(measure_star_prepared(&source, None, None, 32.0, 32.0, &cfg).unwrap()).unwrap();
        assert_eq!(weighted["area_applied"], true);
        assert_eq!(weighted["n_nonfinite"], 3);
        assert_eq!(weighted["warnings"], json["warnings"]);

        let mut mask = Array2::<u8>::zeros((64, 64));
        mask[[31, 35]] = 1;
        let masked = serde_json::to_value(measure_star_masked(&edge, 32.0, 32.0, &cfg, Some(&mask)).unwrap()).unwrap();
        assert_eq!(masked["n_masked"], 1);
        assert_eq!(masked["n_nonfinite"], 2);

        let mut single = clean.clone();
        single[[32, 35]] = f32::NAN;
        let one = serde_json::to_value(measure_star(&single, 32.0, 32.0, &cfg).unwrap()).unwrap();
        assert_eq!(
            one["warnings"],
            serde_json::json!(["1 aperture pixel without data (non-finite): the flux is a lower limit"])
        );

        let mut hollow = clean.clone();
        hollow.slice_mut(ndarray::s![31..34, 31..34]).fill(f32::NAN);
        let core = measure_star(&hollow, 32.0, 32.0, &PhotometryConfig::default()).unwrap();
        let core_json = serde_json::to_value(&core).unwrap();
        assert_eq!(core.aperture_radius, 6.0);
        assert!(core.net_flux < 0.8 * whole.net_flux, "net flux {} against {}", core.net_flux, whole.net_flux);
        assert_eq!(core_json["n_nonfinite"], 9);
        assert!(core_json["fwhm"].is_null(), "FWHM {} reported for a star with no core", core_json["fwhm"]);
        assert_eq!(
            core_json["warnings"],
            serde_json::json!([
                "FWHM not measured: the automatic aperture radius of 6.0 px is a fallback, not 1.5 x FWHM",
                "9 aperture pixels without data (non-finite): the flux is a lower limit"
            ])
        );
    }

    #[test]
    fn snr_does_not_depend_on_the_unit_of_the_pixel_values() {
        let mut img = gaussian_scene(64, 64, 32.0, 32.0, 5000.0, 2.0, 100.0);
        deterministic_noise(&mut img);
        let reference = measure_star(&img, 32.0, 32.0, &PhotometryConfig::default()).unwrap();
        assert!(reference.snr > 100.0, "snr={}", reference.snr);

        let small_unit = img.mapv(|v| v * 1e-17);
        let scaled = measure_star(&small_unit, 32.0, 32.0, &PhotometryConfig::default()).unwrap();
        assert!(scaled.flux_err > 0.0 && scaled.flux_err < 1e-12, "flux_err={}", scaled.flux_err);
        assert!(
            (scaled.snr - reference.snr).abs() / reference.snr < 1e-3,
            "snr {} in a unit 1e17 times larger against {}",
            scaled.snr,
            reference.snr
        );
        let (scaled_fwhm, reference_fwhm) = (measured_fwhm(&scaled), measured_fwhm(&reference));
        assert!((scaled_fwhm - reference_fwhm).abs() / reference_fwhm < 1e-3, "fwhm {} against {}", scaled_fwhm, reference_fwhm);
        assert!((scaled.aperture_radius - reference.aperture_radius).abs() / reference.aperture_radius < 1e-3);
    }

    #[test]
    fn an_unmeasured_fwhm_is_absent_and_the_automatic_aperture_fallback_is_flagged() {
        let blank = Array2::from_elem((64, 64), 10.0f32);
        let auto = measure_star(&blank, 32.0, 32.0, &PhotometryConfig::default()).unwrap();
        let json = serde_json::to_value(&auto).unwrap();
        assert!(json["fwhm"].is_null(), "FWHM {} reported for a position with no source", json["fwhm"]);
        assert_eq!(auto.fwhm, None, "an unmeasured FWHM is absent, not a NaN in a number");
        assert_eq!(auto.aperture_radius, 6.0);
        assert_eq!(
            json["warnings"],
            serde_json::json!(["FWHM not measured: the automatic aperture radius of 6.0 px is a fallback, not 1.5 x FWHM"])
        );

        let fixed = PhotometryConfig { aperture_radius: Some(5.0), ..PhotometryConfig::default() };
        let json = serde_json::to_value(measure_star(&blank, 32.0, 32.0, &fixed).unwrap()).unwrap();
        assert!(json["fwhm"].is_null());
        assert_eq!(json["aperture_radius"], 5.0);
        assert_eq!(json["warnings"], serde_json::json!([]));

        let mut noisy_sky = Array2::from_elem((64, 64), 100.0f32);
        deterministic_noise(&mut noisy_sky);
        let json = serde_json::to_value(measure_star(&noisy_sky, 32.0, 32.0, &PhotometryConfig::default()).unwrap()).unwrap();
        assert!(json["fwhm"].is_null(), "FWHM {} reported on a noisy sky with no source", json["fwhm"]);
        assert_eq!(json["aperture_radius"], 6.0);

        let star = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        let measured = measure_star(&star, 32.0, 32.0, &PhotometryConfig::default()).unwrap();
        assert!(measured.fwhm.is_some_and(|f| f > 0.0), "a measured FWHM is present: {:?}", measured.fwhm);
        let json = serde_json::to_value(&measured).unwrap();
        assert_eq!(json["fwhm"].as_f64(), measured.fwhm);
        assert_eq!(json["warnings"], serde_json::json!([]));
    }

    const GAUSSIAN_SIGMA_TO_FWHM: f64 = 2.3548200450309493;
    const GAUSS_LEGENDRE_8: [(f64, f64); 8] = [
        (-0.9602898564975363, 0.1012285362903763),
        (-0.7966664774136267, 0.2223810344533745),
        (-0.5255324099163290, 0.3137066458778873),
        (-0.1834346424956498, 0.3626837833783620),
        (0.1834346424956498, 0.3626837833783620),
        (0.5255324099163290, 0.3137066458778873),
        (0.7966664774136267, 0.2223810344533745),
        (0.9602898564975363, 0.1012285362903763),
    ];
    const SUBPIXEL_OFFSETS: [(&str, f64, f64); 3] = [("centre", 0.0, 0.0), ("edge", 0.5, 0.0), ("corner", 0.5, 0.5)];
    const IMAGE_FWHM_ORACLE: [(&str, f64, f64); 8] = [
        ("gaussian", 1.5, 1.661526),
        ("gaussian", 2.0, 2.118745),
        ("gaussian", 3.0, 3.077981),
        ("gaussian", 6.0, 6.038630),
        ("moffat", 1.5, 1.749496),
        ("moffat", 2.0, 2.190178),
        ("moffat", 3.0, 3.128345),
        ("moffat", 6.0, 6.064665),
    ];

    fn pixel_integrated_gaussian(size: usize, star_x: f64, star_y: f64, fwhm: f64, flux: f64, bg: f64) -> Array2<f32> {
        let scale = fwhm / GAUSSIAN_SIGMA_TO_FWHM * std::f64::consts::SQRT_2;
        let column = |i: usize, centre: f64| {
            0.5 * (libm::erf((i as f64 + 0.5 - centre) / scale) - libm::erf((i as f64 - 0.5 - centre) / scale))
        };
        Array2::from_shape_fn((size, size), |(y, x)| (bg + flux * column(x, star_x) * column(y, star_y)) as f32)
    }

    fn pixel_integrated_moffat(size: usize, star_x: f64, star_y: f64, fwhm: f64, beta: f64, flux: f64, bg: f64) -> Array2<f32> {
        let alpha2 = (fwhm / 2.0).powi(2) / (2.0_f64.powf(1.0 / beta) - 1.0);
        let norm = (beta - 1.0) / (std::f64::consts::PI * alpha2);
        Array2::from_shape_fn((size, size), |(y, x)| {
            let mut sum = 0.0;
            for (node_y, weight_y) in GAUSS_LEGENDRE_8 {
                for (node_x, weight_x) in GAUSS_LEGENDRE_8 {
                    let dx = x as f64 + 0.5 * node_x - star_x;
                    let dy = y as f64 + 0.5 * node_y - star_y;
                    sum += 0.25 * weight_x * weight_y * (1.0 + (dx * dx + dy * dy) / alpha2).powf(-beta);
                }
            }
            (bg + flux * norm * sum) as f32
        })
    }

    fn pixel_integrated_star(profile: &str, star_x: f64, star_y: f64, fwhm: f64) -> Array2<f32> {
        match profile {
            "gaussian" => pixel_integrated_gaussian(64, star_x, star_y, fwhm, 20000.0, 100.0),
            _ => pixel_integrated_moffat(64, star_x, star_y, fwhm, 2.5, 20000.0, 100.0),
        }
    }

    #[test]
    fn fwhm_of_a_faint_star_on_a_noisy_sky_stays_near_the_true_width() {
        let true_fwhm = GAUSSIAN_SIGMA_TO_FWHM * 2.0;
        for (amp, tolerance) in [(40.0, 0.05), (8.0, 0.15)] {
            let mut img = gaussian_scene(64, 64, 32.3, 31.6, amp, 2.0, 100.0);
            deterministic_noise(&mut img);
            let res = measure_star(&img, 32.0, 32.0, &PhotometryConfig::default()).unwrap();
            let fwhm = measured_fwhm(&res);
            assert!((fwhm - true_fwhm).abs() / true_fwhm < tolerance, "amplitude {amp}: FWHM {fwhm} against {true_fwhm}");
        }
    }

    #[test]
    fn fwhm_of_a_star_does_not_depend_on_its_subpixel_position_down_to_one_and_a_half_pixels() {
        let mut failures = Vec::new();
        println!("profile   true  image   offset    fwhm  aperture");
        for (profile, fwhm, image_fwhm) in IMAGE_FWHM_ORACLE {
            let tolerance = if fwhm < 2.0 {
                0.04
            } else if fwhm < 3.0 {
                0.025
            } else {
                0.012
            };
            let mut measured = Vec::new();
            for (offset, dx, dy) in SUBPIXEL_OFFSETS {
                let img = pixel_integrated_star(profile, 32.0 + dx, 32.0 + dy, fwhm);
                let res = measure_star(&img, 32.0, 32.0, &PhotometryConfig::default()).unwrap();
                let found = measured_fwhm(&res);
                println!("{profile:8} {fwhm:5.1} {image_fwhm:6.4}  {offset:6} {:7.4}  {:7.4}", found, res.aperture_radius);
                assert!(
                    (res.aperture_radius - (1.5 * found).clamp(MIN_APERTURE_RADIUS, MAX_APERTURE_RADIUS)).abs() < 1e-12,
                    "{profile} {fwhm} {offset}: aperture {} does not follow the FWHM {}",
                    res.aperture_radius,
                    found
                );
                measured.push(found);
            }
            let lowest = measured.iter().cloned().fold(f64::INFINITY, f64::min);
            let highest = measured.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let near_the_image_fwhm = measured.iter().all(|f| ((f - image_fwhm) / image_fwhm).abs() <= tolerance);
            if !near_the_image_fwhm || (highest - lowest) / image_fwhm > tolerance {
                failures.push(format!("{profile} {fwhm}: {lowest:.4} to {highest:.4} against {image_fwhm:.4}"));
            }
        }
        assert!(failures.is_empty(), "the FWHM moves with the subpixel position: {failures:#?}");
    }
}
