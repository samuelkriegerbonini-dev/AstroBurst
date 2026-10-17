use anyhow::{bail, Result};
use ndarray::Array2;
use rayon::prelude::*;
use serde::Deserialize;

use crate::core::astrometry::spectral::{velocity_axis, SpectralAxis, VelocityConvention};
use crate::core::cube::companions::{check_companion, CubeSet};
use crate::core::cube::lazy::{err_variance, median_band_rows, LazyCube};
use crate::core::cube::noise_factor::{residual_dof_correction, EXPECTED_FACTOR_RANGE};
use crate::core::stacking::memory::{effective_memory, format_bytes, MemoryInfo, MEMORY_BUDGET_ENV};
use crate::math::{exact_mad_mut, exact_median_mut};
use crate::types::constants::{EXTNAME_ERR, MAD_TO_SIGMA};

pub const DEFAULT_SNR_THRESHOLD: f64 = 3.0;
pub const VELOCITY_UNIT: &str = "km/s";
pub const REST_REQUIRED_MESSAGE: &str = "rest wavelength required for velocity moments";
pub const DQ_NOT_APPLIED_NOTE: &str =
    "DQ is not applied to moment maps or range collapses: flagged spaxels with finite values are included";

const CONTINUUM_BAND_BYTES: usize = 256 << 20;
const MOMENT_BATCH_SIZE: usize = 16;
const MAX_FACTOR_RESIDUALS: usize = 4_000_000;
const F32_BYTES: u64 = 4;
const MOMENT_PLANES: u64 = 3;

pub type ContinuumWindows = ((usize, usize), (usize, usize));

#[derive(Debug, Clone, Deserialize)]
struct MomentConfigWire {
    z0: usize,
    z1: usize,
    #[serde(default)]
    rest_um: Option<f64>,
    #[serde(default)]
    convention: Option<String>,
    #[serde(default)]
    continuum: Option<ContinuumWindows>,
    #[serde(default)]
    snr_threshold: Option<f64>,
    #[serde(default)]
    mask_below_threshold: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "MomentConfigWire")]
pub struct MomentConfig {
    pub z0: usize,
    pub z1: usize,
    pub rest_um: Option<f64>,
    pub convention: VelocityConvention,
    pub continuum: Option<ContinuumWindows>,
    pub snr_threshold: f64,
    pub mask_below_threshold: bool,
}

impl TryFrom<MomentConfigWire> for MomentConfig {
    type Error = String;

    fn try_from(wire: MomentConfigWire) -> std::result::Result<Self, Self::Error> {
        let convention = match wire.convention.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(name) => VelocityConvention::parse(name)?,
            None => VelocityConvention::Optical,
        };
        let snr_threshold = wire.snr_threshold.unwrap_or(DEFAULT_SNR_THRESHOLD);
        if !snr_threshold.is_finite() || snr_threshold < 0.0 {
            return Err(format!("snr_threshold must be a non-negative number, got {}", snr_threshold));
        }
        Ok(MomentConfig {
            z0: wire.z0,
            z1: wire.z1,
            rest_um: wire.rest_um,
            convention,
            continuum: wire.continuum,
            snr_threshold,
            mask_below_threshold: wire.mask_below_threshold.unwrap_or(true),
        })
    }
}

#[derive(Debug, Clone)]
pub struct MomentMaps {
    pub m0: Array2<f32>,
    pub m1: Array2<f32>,
    pub m2: Array2<f32>,
    pub m0_unit: String,
    pub velocity_unit: &'static str,
    pub noise_per_channel: Option<f64>,
    pub n_channels: usize,
    pub notes: Vec<String>,
    pub m0_err: Option<Array2<f32>>,
    pub m1_err: Option<Array2<f32>>,
    pub m2_err: Option<Array2<f32>>,
    pub noise_factor: Option<f64>,
    pub err_hdu: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PixelFit {
    intercept: f32,
    slope: f32,
    sigma: f32,
}

impl PixelFit {
    const UNKNOWN: PixelFit = PixelFit { intercept: f32::NAN, slope: f32::NAN, sigma: f32::NAN };
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LineFit {
    intercept: f64,
    slope: f64,
    channels: usize,
    fitted_parameters: usize,
}

fn robust_sigma(residuals: &mut Vec<f32>) -> f32 {
    if residuals.len() < 2 {
        return f32::NAN;
    }
    let median = exact_median_mut(residuals) as f32;
    (exact_mad_mut(residuals, median) as f64 * MAD_TO_SIGMA) as f32
}

fn fits_a_slope(n: usize, sx: f64, sxx: f64, constant_only: bool) -> bool {
    let nf = n as f64;
    let denominator = nf * sxx - sx * sx;
    !(constant_only || n < 3 || denominator.abs() <= 1e-12 * nf * sxx.max(1.0))
}

fn fit_line(xs: &[f64], ys: &[f32], constant_only: bool) -> Option<LineFit> {
    let mut n = 0usize;
    let (mut sx, mut sy, mut sxx, mut sxy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (&x, &y) in xs.iter().zip(ys) {
        if !y.is_finite() {
            continue;
        }
        let y = y as f64;
        n += 1;
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
    }
    if n == 0 {
        return None;
    }
    let nf = n as f64;
    if !fits_a_slope(n, sx, sxx, constant_only) {
        return Some(LineFit { intercept: sy / nf, slope: 0.0, channels: n, fitted_parameters: 1 });
    }
    let denominator = nf * sxx - sx * sx;
    let slope = (nf * sxy - sx * sy) / denominator;
    Some(LineFit { intercept: (sy - slope * sx) / nf, slope, channels: n, fitted_parameters: 2 })
}

fn fit_pixel_line(xs: &[f64], ys: &[f32], constant_only: bool) -> (PixelFit, Option<LineFit>) {
    let Some(line) = fit_line(xs, ys, constant_only) else {
        return (PixelFit::UNKNOWN, None);
    };
    let mut residuals: Vec<f32> = xs
        .iter()
        .zip(ys)
        .filter(|(_, y)| y.is_finite())
        .map(|(&x, &y)| (y as f64 - (line.intercept + line.slope * x)) as f32)
        .collect();
    let fit = PixelFit { intercept: line.intercept as f32, slope: line.slope as f32, sigma: robust_sigma(&mut residuals) };
    (fit, Some(line))
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct BaselineCovariance {
    x_mean: f64,
    level: f64,
    cross: f64,
    slope: f64,
    inv_n: f64,
    inv_sxx: f64,
}

impl BaselineCovariance {
    const UNKNOWN: BaselineCovariance = BaselineCovariance {
        x_mean: f64::NAN,
        level: f64::NAN,
        cross: f64::NAN,
        slope: f64::NAN,
        inv_n: f64::NAN,
        inv_sxx: f64::NAN,
    };

    fn of_weighted_sum(&self, weights: f64, weighted_x: f64) -> f64 {
        let centred = weighted_x - self.x_mean * weights;
        weights * weights * self.level + 2.0 * weights * centred * self.cross + centred * centred * self.slope
    }

    fn shared_with(&self, weights: f64, weighted_x: f64, shared_variance: f64, shared_variance_x: f64) -> f64 {
        let centred = weighted_x - self.x_mean * weights;
        -2.0 * (weights * self.inv_n * shared_variance
            + centred * self.inv_sxx * (shared_variance_x - self.x_mean * shared_variance))
    }
}

fn baseline_covariance(xs: &[f64], ys: &[f32], errs: &[f32], constant_only: bool) -> BaselineCovariance {
    let used: Vec<(f64, f64)> = xs
        .iter()
        .zip(ys)
        .zip(errs)
        .filter(|((_, y), _)| y.is_finite())
        .map(|((&x, _), &e)| (x, err_variance(e)))
        .collect();
    let n = used.len();
    if n == 0 {
        return BaselineCovariance::UNKNOWN;
    }
    let nf = n as f64;
    let sx: f64 = used.iter().map(|u| u.0).sum();
    let sxx: f64 = used.iter().map(|u| u.0 * u.0).sum();
    let total: f64 = used.iter().map(|u| u.1).sum();
    let level_only = BaselineCovariance {
        x_mean: sx / nf,
        level: total / (nf * nf),
        cross: 0.0,
        slope: 0.0,
        inv_n: 1.0 / nf,
        inv_sxx: 0.0,
    };
    if !fits_a_slope(n, sx, sxx, constant_only) {
        return level_only;
    }
    let x_mean = level_only.x_mean;
    let sxx_centred: f64 = used.iter().map(|u| (u.0 - x_mean).powi(2)).sum();
    let cross = used.iter().map(|u| u.1 * (u.0 - x_mean)).sum::<f64>() / (nf * sxx_centred);
    let slope = used.iter().map(|u| u.1 * (u.0 - x_mean).powi(2)).sum::<f64>() / (sxx_centred * sxx_centred);
    BaselineCovariance { cross, slope, inv_sxx: 1.0 / sxx_centred, ..level_only }
}

pub fn channel_widths(velocities: &[f64]) -> Vec<f64> {
    let n = velocities.len();
    (0..n)
        .map(|i| {
            if n < 2 {
                0.0
            } else if i == 0 {
                (velocities[1] - velocities[0]).abs()
            } else if i + 1 == n {
                (velocities[n - 1] - velocities[n - 2]).abs()
            } else {
                (velocities[i + 1] - velocities[i - 1]).abs() / 2.0
            }
        })
        .collect()
}

fn velocity_frame_note(axis: &SpectralAxis) -> String {
    let frame = match axis.specsys.as_deref() {
        Some(frame) => format!("axis frame {}", frame),
        None => "axis frame, which the header does not state (no SPECSYS)".to_string(),
    };
    format!(
        "M1 velocities are in the {}: no barycentric or heliocentric correction is applied to the moment maps",
        frame
    )
}

fn velocities_for(axis: &SpectralAxis, cfg: &MomentConfig, notes: &mut Vec<String>) -> Result<Vec<f64>> {
    if !axis.kind.is_spectral() {
        bail!(
            "CTYPE3 '{}' is not a spectral axis: moments need a wavelength, frequency or velocity axis",
            axis.ctype
        );
    }
    if axis.kind.is_velocity() {
        notes.push(format!(
            "velocity axis {} used as stored in {} (rest wavelength and convention ignored)",
            axis.ctype, axis.unit
        ));
        notes.push(velocity_frame_note(axis));
        return Ok(axis.values.clone());
    }
    let rest_um = match cfg.rest_um.filter(|r| r.is_finite() && *r > 0.0).or(axis.rest_wavelength_um) {
        Some(rest) => rest,
        None => bail!("{}", REST_REQUIRED_MESSAGE),
    };
    let velocity = velocity_axis(axis, rest_um, cfg.convention).map_err(anyhow::Error::msg)?;
    notes.extend(velocity.notes);
    notes.push(format!(
        "velocities from the {} convention with rest wavelength {} um",
        cfg.convention.name(),
        rest_um
    ));
    notes.push(velocity_frame_note(axis));
    Ok(velocity.values_kms)
}

fn check_window(window: (usize, usize), depth: usize) -> Result<()> {
    if window.0 > window.1 || window.1 >= depth {
        bail!(
            "continuum window {}..={} is invalid for a cube with {} channels",
            window.0,
            window.1,
            depth
        );
    }
    Ok(())
}

fn window_channels(windows: ContinuumWindows, depth: usize) -> Result<Vec<usize>> {
    check_window(windows.0, depth)?;
    check_window(windows.1, depth)?;
    let mut channels: Vec<usize> = (windows.0 .0..=windows.0 .1).chain(windows.1 .0..=windows.1 .1).collect();
    channels.sort_unstable();
    channels.dedup();
    Ok(channels)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MapNoiseFactor {
    factor: f64,
    pixels: usize,
    residuals: usize,
    stride: usize,
}

struct ContinuumFit {
    pixels: Vec<PixelFit>,
    linear: bool,
    covariance: Option<Vec<BaselineCovariance>>,
    noise_factor: Option<MapNoiseFactor>,
}

fn decode_window_rows(cube: &LazyCube, channels: &[usize], band_start: usize, row_count: usize) -> Result<Vec<Vec<f32>>> {
    channels.par_iter().map(|&z| cube.decode_rows(z, band_start, row_count)).collect()
}

fn continuum_band_rows(window_channels: usize, cols: usize, rows: usize, has_err: bool) -> usize {
    median_band_rows(window_channels, cols, rows, CONTINUUM_BAND_BYTES / (1 + usize::from(has_err)))
}

fn factor_pixel_stride(npix: usize, window_channels: usize) -> usize {
    npix.saturating_mul(window_channels).div_ceil(MAX_FACTOR_RESIDUALS).max(1)
}

fn normalised_residuals(xs: &[f64], ys: &[f32], errs: &[f32], line: &LineFit) -> Vec<f32> {
    let correction = residual_dof_correction(line.channels, line.fitted_parameters);
    xs.iter()
        .zip(ys)
        .zip(errs)
        .filter(|((_, y), e)| y.is_finite() && e.is_finite() && **e > 0.0)
        .map(|((&x, &y), &e)| ((y as f64 - (line.intercept + line.slope * x)) / e as f64 * correction) as f32)
        .collect()
}

fn pooled_noise_factor(mut pool: Vec<f32>, pixels: usize, stride: usize) -> Option<MapNoiseFactor> {
    if pool.len() < 2 {
        return None;
    }
    let residuals = pool.len();
    let centre = exact_median_mut(&mut pool) as f32;
    let factor = exact_mad_mut(&mut pool, centre) as f64 * MAD_TO_SIGMA;
    Some(MapNoiseFactor { factor, pixels, residuals, stride })
}

fn fit_continuum(
    cube: &LazyCube,
    err: Option<&LazyCube>,
    windows: ContinuumWindows,
    channels: &[usize],
    notes: &mut Vec<String>,
) -> Result<ContinuumFit> {
    let g = &cube.geometry;
    let (rows, cols) = (g.naxis2, g.naxis1);
    let linear = windows.0 != windows.1 && channels.len() >= 3;
    let xs: Vec<f64> = channels.iter().map(|&z| z as f64).collect();
    let band_rows = continuum_band_rows(channels.len(), cols, rows, err.is_some());
    let npix = rows * cols;
    let stride = factor_pixel_stride(npix, channels.len());
    let mut pixels = vec![PixelFit::UNKNOWN; npix];
    let mut covariance = err.map(|_| vec![BaselineCovariance::UNKNOWN; npix]);
    let mut pool: Vec<f32> = Vec::new();
    let mut pooled_pixels = 0usize;

    for band_start in (0..rows).step_by(band_rows) {
        let band_end = (band_start + band_rows).min(rows);
        let row_count = band_end - band_start;
        let planes = decode_window_rows(cube, channels, band_start, row_count)?;
        let err_planes = err.map(|c| decode_window_rows(c, channels, band_start, row_count)).transpose()?;
        let offset = band_start * cols;
        let band = offset..offset + row_count * cols;
        let samples = |planes: &[Vec<f32>], i: usize| -> Vec<f32> { planes.iter().map(|p| p[i]).collect() };
        match (covariance.as_mut(), err_planes.as_ref()) {
            (Some(all), Some(err_planes)) => {
                let sampled: Vec<Vec<f32>> = pixels[band.clone()]
                    .par_iter_mut()
                    .zip(all[band].par_iter_mut())
                    .enumerate()
                    .filter_map(|(i, (fit, pixel_covariance))| {
                        let ys = samples(&planes, i);
                        let errs = samples(err_planes, i);
                        let (pixel, line) = fit_pixel_line(&xs, &ys, !linear);
                        *fit = pixel;
                        *pixel_covariance = baseline_covariance(&xs, &ys, &errs, !linear);
                        let pooled = (offset + i) % stride == 0 && pixel.sigma > 0.0;
                        line.filter(|l| pooled && l.channels > l.fitted_parameters)
                            .map(|l| normalised_residuals(&xs, &ys, &errs, &l))
                            .filter(|r| !r.is_empty())
                    })
                    .collect();
                pooled_pixels += sampled.len();
                pool.extend(sampled.into_iter().flatten());
            }
            _ => pixels[band].par_iter_mut().enumerate().for_each(|(i, fit)| {
                *fit = fit_pixel_line(&xs, &samples(&planes, i), !linear).0;
            }),
        }
    }

    notes.push(format!(
        "continuum: {} fitted per pixel over channels {}..={} and {}..={}, subtracted before the moments",
        if linear { "first-order line" } else { "constant" },
        windows.0 .0,
        windows.0 .1,
        windows.1 .0,
        windows.1 .1
    ));
    let noise_factor = pooled_noise_factor(pool, pooled_pixels, stride);
    Ok(ContinuumFit { pixels, linear, covariance, noise_factor })
}

fn median_finite(values: impl Iterator<Item = f32>) -> Option<f64> {
    let mut finite: Vec<f32> = values.filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        None
    } else {
        Some(exact_median_mut(&mut finite))
    }
}

fn line_free_channel(cfg: &MomentConfig, depth: usize) -> Option<usize> {
    if cfg.z0 > 0 {
        Some(cfg.z0 - 1)
    } else if cfg.z1 + 1 < depth {
        Some(cfg.z1 + 1)
    } else {
        None
    }
}

fn global_noise_from_channel(cube: &LazyCube, cfg: &MomentConfig, notes: &mut Vec<String>) -> Result<Option<f64>> {
    let Some(z) = line_free_channel(cfg, cube.geometry.naxis3) else {
        notes.push("no line-free channel outside the range: noise unknown, SNR mask not applied".to_string());
        return Ok(None);
    };
    let plane = cube.decode_rows(z, 0, cube.geometry.naxis2)?;
    let mut finite: Vec<f32> = plane.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.len() < 2 {
        notes.push(format!("channel {} has no finite pixels: noise unknown, SNR mask not applied", z));
        return Ok(None);
    }
    let median = exact_median_mut(&mut finite) as f32;
    let sigma = exact_mad_mut(&mut finite, median) as f64 * MAD_TO_SIGMA;
    notes.push(format!(
        "no continuum windows: moments use the raw channel values; noise = robust sigma of channel {} ({:.4e})",
        z, sigma
    ));
    Ok(Some(sigma))
}

#[derive(Clone, Copy, Default)]
struct PixelAccumulator {
    m0: f64,
    weight: f64,
    weighted_v: f64,
    weighted_v2: f64,
    count: u32,
}

impl PixelAccumulator {
    fn add(&mut self, value: f64, v: f64, dv: f64) {
        self.count += 1;
        self.m0 += value * dv;
        if value > 0.0 {
            self.weight += value;
            self.weighted_v += value * v;
            self.weighted_v2 += value * v * v;
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ErrorAccumulator {
    m0_variance: f64,
    dv: f64,
    dv_x: f64,
    variance_u: [f64; 5],
    u: [f64; 3],
    x_u: [f64; 3],
}

#[derive(Clone, Copy, Default)]
struct OverlapAccumulator {
    m0: f64,
    m0_x: f64,
    u: [f64; 3],
    x_u: [f64; 3],
}

impl OverlapAccumulator {
    fn add(&mut self, value: f64, variance: f64, u: f64, dv: f64, x: f64) {
        self.m0 += dv * variance;
        self.m0_x += dv * variance * x;
        if value > 0.0 {
            for ((plain, by_x), power) in self.u.iter_mut().zip(self.x_u.iter_mut()).zip([1.0, u, u * u]) {
                *plain += variance * power;
                *by_x += variance * x * power;
            }
        }
    }
}

#[derive(Clone, Copy)]
struct PixelBaseline<'a> {
    covariance: &'a BaselineCovariance,
    overlap: Option<&'a OverlapAccumulator>,
}

impl PixelBaseline<'_> {
    fn variance(&self, weights: f64, weighted_x: f64, shared: impl Fn(&OverlapAccumulator) -> (f64, f64)) -> f64 {
        let fitted = self.covariance.of_weighted_sum(weights, weighted_x);
        let overlap = self.overlap.map_or(0.0, |o| {
            let (variance, variance_x) = shared(o);
            self.covariance.shared_with(weights, weighted_x, variance, variance_x)
        });
        fitted + overlap
    }
}

impl ErrorAccumulator {
    fn add(&mut self, value: f64, variance: f64, u: f64, dv: f64, x: f64) {
        self.m0_variance += dv * dv * variance;
        self.dv += dv;
        self.dv_x += dv * x;
        if value > 0.0 {
            let powers = [1.0, u, u * u, u * u * u, u * u * u * u];
            for (slot, power) in self.variance_u.iter_mut().zip(powers) {
                *slot += variance * power;
            }
            for ((plain, by_x), power) in self.u.iter_mut().zip(self.x_u.iter_mut()).zip(powers) {
                *plain += power;
                *by_x += x * power;
            }
        }
    }

    fn m0_variance(&self, baseline: Option<PixelBaseline>) -> f64 {
        self.m0_variance + baseline.map_or(0.0, |b| b.variance(self.dv, self.dv_x, |o| (o.m0, o.m0_x)))
    }

    fn m1_variance(&self, weight: f64, mu: f64, baseline: Option<PixelBaseline>) -> f64 {
        let [a0, a1, a2, _, _] = self.variance_u;
        let direct = (a2 - 2.0 * mu * a1 + mu * mu * a0) / (weight * weight);
        let g0 = (self.u[1] - mu * self.u[0]) / weight;
        let g1 = (self.x_u[1] - mu * self.x_u[0]) / weight;
        let shared = |o: &OverlapAccumulator| ((o.u[1] - mu * o.u[0]) / weight, (o.x_u[1] - mu * o.x_u[0]) / weight);
        direct + baseline.map_or(0.0, |b| b.variance(g0, g1, shared))
    }

    fn m2_variance(&self, weight: f64, mu: f64, m2: f64, baseline: Option<PixelBaseline>) -> f64 {
        let [a0, a1, a2, a3, a4] = self.variance_u;
        let (p, q) = (-2.0 * mu, mu * mu - m2 * m2);
        let scale = 2.0 * m2 * weight;
        let direct = (a4 + 2.0 * p * a3 + (p * p + 2.0 * q) * a2 + 2.0 * p * q * a1 + q * q * a0) / (scale * scale);
        let g0 = (self.u[2] + p * self.u[1] + q * self.u[0]) / scale;
        let g1 = (self.x_u[2] + p * self.x_u[1] + q * self.x_u[0]) / scale;
        let shared = |o: &OverlapAccumulator| {
            ((o.u[2] + p * o.u[1] + q * o.u[0]) / scale, (o.x_u[2] + p * o.x_u[1] + q * o.x_u[0]) / scale)
        };
        direct + baseline.map_or(0.0, |b| b.variance(g0, g1, shared))
    }
}

fn error_from_variance(variance: f64) -> f32 {
    variance.max(0.0).sqrt() as f32
}

fn error_notes(err_hdu: usize, continuum: Option<&ContinuumFit>, notes: &mut Vec<String>) {
    notes.push(format!(
        "error planes: first-order propagation of the ERR cube (HDU {}){}; M1 and M2 errors use only the positive channels, like the maps, so they are biased in low-S/N wings and degrade below S/N ~5",
        err_hdu,
        if continuum.is_some() { " including the covariance of the fitted continuum" } else { "" }
    ));
    match continuum.map(|fit| fit.noise_factor) {
        None => notes.push("no continuum windows: the noise factor is not measured".to_string()),
        Some(None) => notes.push(
            "no pixel has a non-zero continuum residual scatter and a usable ERR: the noise factor is not measured".to_string(),
        ),
        Some(Some(f)) => {
            let sampling = if f.stride > 1 { format!(", one pixel in {}", f.stride) } else { String::new() };
            notes.push(format!(
                "noise factor {:.3}: robust sigma of the continuum-window residuals divided by the ERR, pooled over {} residuals from {} pixels with non-zero scatter{}; the error planes hold the propagated ERR and the factor applies only on display or export",
                f.factor, f.residuals, f.pixels, sampling
            ));
            let (low, high) = EXPECTED_FACTOR_RANGE;
            if !(low..=high).contains(&f.factor) {
                notes.push(format!(
                    "noise factor {:.3} is outside [{}, {}]: the ERR cube does not describe the scatter of this cube",
                    f.factor, low, high
                ));
            }
        }
    }
}

fn bunit(cube: &LazyCube) -> Option<String> {
    cube.header
        .get("BUNIT")
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

fn moment_batch_size(has_err: bool) -> usize {
    MOMENT_BATCH_SIZE / (1 + usize::from(has_err))
}

fn moment_memory_bytes(rows: usize, cols: usize, window_channels: Option<usize>, has_err: bool, overlap: bool) -> u64 {
    let npix = (rows as u64).saturating_mul(cols as u64);
    let err = u64::from(has_err);
    let planes = 1 + err;
    let accumulating = size_of::<PixelAccumulator>() as u64
        + MOMENT_PLANES * F32_BYTES * planes
        + err * size_of::<ErrorAccumulator>() as u64
        + u64::from(overlap) * size_of::<OverlapAccumulator>() as u64
        + moment_batch_size(has_err) as u64 * F32_BYTES * planes;
    let Some(channels) = window_channels else {
        return npix.saturating_mul(accumulating);
    };
    let channels = channels as u64;
    let continuum = size_of::<PixelFit>() as u64 + err * size_of::<BaselineCovariance>() as u64;
    let band_pixels = (continuum_band_rows(channels as usize, cols, rows, has_err) as u64).saturating_mul(cols as u64);
    let pool = err * npix.saturating_mul(channels).min(MAX_FACTOR_RESIDUALS as u64) * F32_BYTES;
    let fitting = band_pixels.saturating_mul(channels * F32_BYTES * planes).saturating_add(pool);
    npix.saturating_mul(continuum).saturating_add(npix.saturating_mul(accumulating).max(fitting))
}

fn check_moment_memory(required: u64, rows: usize, cols: usize, has_err: bool, info: Option<MemoryInfo>) -> Result<()> {
    let Some(info) = info else {
        return Ok(());
    };
    if required > info.commit_available {
        bail!(
            "moment maps of this {}x{} cube{} need about {} of memory but only {} is available: cut the cube to a smaller field first, close other applications, or, if the system can page this much, set {} to the memory to allow (in MB) and restart",
            cols,
            rows,
            if has_err { " with its ERR cube" } else { "" },
            format_bytes(required),
            format_bytes(info.commit_available),
            MEMORY_BUDGET_ENV
        );
    }
    Ok(())
}

pub fn moment_maps(cube: &LazyCube, cfg: &MomentConfig) -> Result<MomentMaps> {
    moment_maps_within(cube, None, cfg, effective_memory())
}

pub fn moment_maps_set(set: &CubeSet, cfg: &MomentConfig) -> Result<MomentMaps> {
    let mut maps = moment_maps_within(&set.sci, set.err.as_deref(), cfg, effective_memory())?;
    if set.dq.is_some() {
        maps.notes.push(DQ_NOT_APPLIED_NOTE.to_string());
    }
    Ok(maps)
}

fn moment_maps_within(
    cube: &LazyCube,
    err: Option<&LazyCube>,
    cfg: &MomentConfig,
    memory: Option<MemoryInfo>,
) -> Result<MomentMaps> {
    let g = &cube.geometry;
    let (rows, cols, depth) = (g.naxis2, g.naxis1, g.naxis3);
    cube.check_channel_range(cfg.z0, cfg.z1)?;
    check_companion(EXTNAME_ERR, cube, err)?;
    let n_channels = cfg.z1 - cfg.z0 + 1;
    if n_channels < 2 {
        bail!("moment maps need at least two channels in the range, got {}", n_channels);
    }
    let axis = cube.spectral_axis().map_err(anyhow::Error::msg)?;
    let mut notes = Vec::new();
    let velocities = velocities_for(&axis, cfg, &mut notes)?;
    if velocities.len() != depth {
        bail!("spectral axis has {} values for a cube with {} channels", velocities.len(), depth);
    }
    let widths = channel_widths(&velocities);
    let dv_mean = widths[cfg.z0..=cfg.z1].iter().sum::<f64>() / n_channels as f64;
    if !(dv_mean > 0.0) {
        bail!("channel velocity width is zero: the spectral axis has no usable step");
    }
    let windows = cfg.continuum.map(|w| window_channels(w, depth).map(|channels| (w, channels))).transpose()?;
    let mut in_windows = vec![false; depth];
    for &z in windows.iter().flat_map(|(_, channels)| channels) {
        in_windows[z] = true;
    }
    let overlap = err.is_some() && in_windows[cfg.z0..=cfg.z1].iter().any(|&w| w);
    let required = moment_memory_bytes(rows, cols, windows.as_ref().map(|(_, c)| c.len()), err.is_some(), overlap);
    check_moment_memory(required, rows, cols, err.is_some(), memory)?;

    let continuum = match &windows {
        Some((w, channels)) => Some(fit_continuum(cube, err, *w, channels, &mut notes)?),
        None => None,
    };
    let global_noise = match &continuum {
        Some(fit) => {
            let level = median_finite(fit.pixels.iter().map(|p| p.sigma)).filter(|s| *s > 0.0);
            match level {
                Some(s) => notes.push(format!(
                    "noise per channel = robust sigma of the {} residuals per pixel, floored at the map median {:.4e}",
                    if fit.linear { "linear-fit" } else { "constant-fit" },
                    s
                )),
                None => notes.push("continuum residuals have zero scatter: per-pixel noise unavailable".to_string()),
            }
            level
        }
        None => global_noise_from_channel(cube, cfg, &mut notes)?,
    };

    let npix = rows * cols;
    let mut acc = vec![PixelAccumulator::default(); npix];
    let mut err_acc = err.map(|_| vec![ErrorAccumulator::default(); npix]);
    let mut overlap_acc = overlap.then(|| vec![OverlapAccumulator::default(); npix]);
    let v_ref = velocities[(cfg.z0 + cfg.z1) / 2];
    let batch_size = moment_batch_size(err.is_some());
    for batch_start in (cfg.z0..=cfg.z1).step_by(batch_size) {
        let batch_end = (batch_start + batch_size).min(cfg.z1 + 1);
        let frames = cube.decode_frames(batch_start, batch_end)?;
        let err_frames = err.map(|c| c.decode_frames(batch_start, batch_end)).transpose()?;
        for (k, frame) in frames.iter().enumerate() {
            let z = batch_start + k;
            let v = velocities[z];
            let dv = widths[z];
            let xz = z as f64;
            let value_at = |i: usize| -> Option<f64> {
                let raw = frame[i];
                if !raw.is_finite() {
                    return None;
                }
                match &continuum {
                    Some(fit) => {
                        let p = &fit.pixels[i];
                        let baseline = p.intercept as f64 + p.slope as f64 * xz;
                        baseline.is_finite().then_some(raw as f64 - baseline)
                    }
                    None => Some(raw as f64),
                }
            };
            match (err_acc.as_mut(), err_frames.as_ref()) {
                (Some(err_acc), Some(err_frames)) => {
                    let sigma = &err_frames[k];
                    let u = v - v_ref;
                    match overlap_acc.as_mut().filter(|_| in_windows[z]) {
                        Some(overlap_acc) => acc
                            .par_iter_mut()
                            .zip(err_acc.par_iter_mut())
                            .zip(overlap_acc.par_iter_mut())
                            .enumerate()
                            .for_each(|(i, ((a, e), o))| {
                                if let Some(value) = value_at(i) {
                                    let variance = err_variance(sigma[i]);
                                    a.add(value, v, dv);
                                    e.add(value, variance, u, dv, xz);
                                    o.add(value, variance, u, dv, xz);
                                }
                            }),
                        None => acc.par_iter_mut().zip(err_acc.par_iter_mut()).enumerate().for_each(|(i, (a, e))| {
                            if let Some(value) = value_at(i) {
                                a.add(value, v, dv);
                                e.add(value, err_variance(sigma[i]), u, dv, xz);
                            }
                        }),
                    }
                }
                _ => acc.par_iter_mut().enumerate().for_each(|(i, a)| {
                    if let Some(value) = value_at(i) {
                        a.add(value, v, dv);
                    }
                }),
            }
        }
    }

    let sqrt_n = (n_channels as f64).sqrt();
    let threshold = cfg.snr_threshold;
    let mut m0 = vec![f32::NAN; npix];
    let mut m1 = vec![f32::NAN; npix];
    let mut m2 = vec![f32::NAN; npix];
    let mut errors = err_acc.as_ref().map(|_| [vec![f32::NAN; npix], vec![f32::NAN; npix], vec![f32::NAN; npix]]);
    let mut masked = 0usize;
    let mut unmaskable = 0usize;
    for i in 0..npix {
        let a = acc[i];
        if a.count == 0 {
            continue;
        }
        let per_pixel = continuum
            .as_ref()
            .map(|fit| fit.pixels[i].sigma as f64)
            .filter(|s| s.is_finite() && *s > 0.0);
        let noise = match (per_pixel, global_noise) {
            (Some(local), Some(floor)) => Some(local.max(floor)),
            (local, floor) => local.or(floor),
        };
        let passes = match noise {
            Some(sigma) => a.m0 / (sigma * sqrt_n * dv_mean) >= threshold,
            None => {
                unmaskable += 1;
                true
            }
        };
        if !passes {
            masked += 1;
        }
        let baseline = continuum.as_ref().and_then(|fit| fit.covariance.as_ref()).map(|c| PixelBaseline {
            covariance: &c[i],
            overlap: overlap_acc.as_ref().map(|o| &o[i]),
        });
        let pixel_errors = err_acc.as_ref().map(|e| e[i]);
        if passes || !cfg.mask_below_threshold {
            m0[i] = a.m0 as f32;
            if let (Some(e), Some([m0_err, _, _])) = (pixel_errors, errors.as_mut()) {
                m0_err[i] = error_from_variance(e.m0_variance(baseline));
            }
        }
        if passes && a.weight > 0.0 {
            let mean_v = a.weighted_v / a.weight;
            let variance = (a.weighted_v2 / a.weight - mean_v * mean_v).max(0.0);
            m1[i] = mean_v as f32;
            m2[i] = variance.sqrt() as f32;
            if let (Some(e), Some([_, m1_err, m2_err])) = (pixel_errors, errors.as_mut()) {
                let mu = mean_v - v_ref;
                let spread = variance.sqrt();
                m1_err[i] = error_from_variance(e.m1_variance(a.weight, mu, baseline));
                if spread > 0.0 {
                    m2_err[i] = error_from_variance(e.m2_variance(a.weight, mu, spread, baseline));
                }
            }
        }
    }
    notes.push(format!(
        "SNR threshold {}: {} of {} pixels below threshold (M1/M2 set to NaN{})",
        threshold,
        masked,
        npix,
        if cfg.mask_below_threshold { ", M0 too" } else { "" }
    ));
    if unmaskable > 0 {
        notes.push(format!("{} pixels had no noise estimate and were kept unmasked", unmaskable));
    }
    if let Some(e) = err {
        error_notes(e.hdu_index, continuum.as_ref(), &mut notes);
    }

    let m0_unit = match bunit(cube) {
        Some(unit) => format!("{} {}", unit, VELOCITY_UNIT),
        None => VELOCITY_UNIT.to_string(),
    };
    let shape = (rows, cols);
    let [m0_err, m1_err, m2_err] = match errors {
        Some([e0, e1, e2]) => [
            Some(Array2::from_shape_vec(shape, e0)?),
            Some(Array2::from_shape_vec(shape, e1)?),
            Some(Array2::from_shape_vec(shape, e2)?),
        ],
        None => [None, None, None],
    };
    Ok(MomentMaps {
        m0: Array2::from_shape_vec(shape, m0)?,
        m1: Array2::from_shape_vec(shape, m1)?,
        m2: Array2::from_shape_vec(shape, m2)?,
        m0_unit,
        velocity_unit: VELOCITY_UNIT,
        noise_per_channel: global_noise,
        n_channels,
        notes,
        m0_err,
        m1_err,
        m2_err,
        noise_factor: continuum.as_ref().and_then(|fit| fit.noise_factor).map(|f| f.factor),
        err_hdu: err.map(|e| e.hdu_index),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    use super::*;
    use crate::core::astrometry::spectral::SPEED_OF_LIGHT_KMS;
    use crate::core::cube::lazy::test_support::*;

    fn channel_width_kms() -> f64 {
        SPEED_OF_LIGHT_KMS * LINE_CDELT_UM / LINE_REST_UM
    }

    fn fit_pixel(xs: &[f64], ys: &[f32], constant_only: bool) -> PixelFit {
        fit_pixel_line(xs, ys, constant_only).0
    }

    fn line_config() -> MomentConfig {
        MomentConfig {
            z0: 8,
            z1: 32,
            rest_um: None,
            convention: VelocityConvention::Optical,
            continuum: Some(((0, 5), (34, 39))),
            snr_threshold: DEFAULT_SNR_THRESHOLD,
            mask_below_threshold: true,
        }
    }

    fn distance_from_disk_centre(y: usize, x: usize) -> f64 {
        (x as f64 - LINE_DISK_CENTRE).hypot(y as f64 - LINE_DISK_CENTRE)
    }

    #[test]
    fn moment_maps_recover_the_line_centre_and_width_and_mask_the_continuum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line_noisy.fits");
        write_line_cube(&path, 0.01);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let maps = moment_maps(&cube, &line_config()).unwrap();

        assert_eq!(maps.n_channels, 25);
        assert_eq!(maps.m0_unit, "Jy/beam km/s");
        assert_eq!(maps.velocity_unit, "km/s");
        assert_eq!(maps.m0.dim(), (LINE_CUBE_SIZE, LINE_CUBE_SIZE));
        let noise = maps.noise_per_channel.expect("noise estimate");
        assert!(noise > 0.002 && noise < 0.02, "noise={}", noise);

        let dv = channel_width_kms();
        let centre = (LINE_DISK_CENTRE as usize, LINE_DISK_CENTRE as usize);
        let m1 = maps.m1[centre] as f64;
        let m2 = maps.m2[centre] as f64;
        let m0 = maps.m0[centre] as f64;
        assert!(m1.abs() < 0.2 * dv, "m1={} dv={}", m1, dv);
        let expected_m2 = LINE_SIGMA_CHANNELS * dv;
        assert!((m2 - expected_m2).abs() < 0.1 * expected_m2, "m2={} expected={}", m2, expected_m2);
        let expected_m0: f64 = (8..=32).map(|z| line_profile(z) as f64).sum::<f64>() * dv;
        assert!((m0 - expected_m0).abs() < 0.03 * expected_m0, "m0={} expected={}", m0, expected_m0);

        let mut outside = 0usize;
        let mut outside_masked = 0usize;
        for y in 0..LINE_CUBE_SIZE {
            for x in 0..LINE_CUBE_SIZE {
                let d = distance_from_disk_centre(y, x);
                if d <= 4.0 {
                    assert!(maps.m1[[y, x]].is_finite(), "inside pixel ({}, {}) masked", y, x);
                    assert!(maps.m2[[y, x]].is_finite());
                    assert!(maps.m0[[y, x]].is_finite());
                } else if d > 8.0 {
                    outside += 1;
                    let masked = maps.m1[[y, x]].is_nan() && maps.m2[[y, x]].is_nan() && maps.m0[[y, x]].is_nan();
                    if masked {
                        outside_masked += 1;
                    }
                }
            }
        }
        assert!(outside > 500);
        assert!(
            outside_masked as f64 >= 0.95 * outside as f64,
            "masked {} of {} outside pixels",
            outside_masked,
            outside
        );
        assert!(maps.notes.iter().any(|n| n.contains("continuum")));
    }

    #[test]
    fn unmasked_m0_keeps_the_continuum_pixels_when_masking_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line_noisy.fits");
        write_line_cube(&path, 0.01);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let mut cfg = line_config();
        cfg.mask_below_threshold = false;
        let maps = moment_maps(&cube, &cfg).unwrap();
        let corner = maps.m0[[0, 0]];
        assert!(corner.is_finite());
        assert!(corner.abs() < 100.0, "corner m0={}", corner);
        assert!(maps.m1[[0, 0]].is_nan());
    }

    #[test]
    fn moment_maps_require_a_rest_wavelength_when_the_header_has_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no_rest.fits");
        write_line_cube_with_cards(&path, &[("RESTWAV", "")]);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        assert!(cube.spectral_axis().unwrap().rest_wavelength_um.is_none());
        let err = moment_maps(&cube, &line_config()).unwrap_err().to_string();
        assert!(err.contains(REST_REQUIRED_MESSAGE), "{}", err);

        let mut cfg = line_config();
        cfg.rest_um = Some(LINE_REST_UM);
        let maps = moment_maps(&cube, &cfg).unwrap();
        let centre = (LINE_DISK_CENTRE as usize, LINE_DISK_CENTRE as usize);
        assert!((maps.m1[centre] as f64).abs() < 0.2 * channel_width_kms());
    }

    #[test]
    fn a_velocity_axis_is_used_directly_without_a_rest_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vrad.fits");
        let cards = [
            ("CTYPE3", "'VRAD'"),
            ("CUNIT3", "'km/s'"),
            ("CRVAL3", "-300.0"),
            ("CDELT3", "40.0"),
            ("CRPIX3", "1.0"),
            ("BUNIT", "'K'"),
        ];
        write_cube(&path, 8, 8, 16, &cards, |z, _, _| {
            let d = z as f64 - 7.0;
            (2.0 * (-d * d / (2.0 * 1.5 * 1.5)).exp()) as f32
        });
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let cfg = MomentConfig {
            z0: 1,
            z1: 13,
            rest_um: None,
            convention: VelocityConvention::Radio,
            continuum: None,
            snr_threshold: DEFAULT_SNR_THRESHOLD,
            mask_below_threshold: true,
        };
        let maps = moment_maps(&cube, &cfg).unwrap();
        assert_eq!(maps.m0_unit, "K km/s");
        assert!((maps.m1[[3, 3]] as f64 + 20.0).abs() < 0.2 * 40.0, "m1={}", maps.m1[[3, 3]]);
        assert!((maps.m2[[3, 3]] as f64 - 60.0).abs() < 6.0, "m2={}", maps.m2[[3, 3]]);
        assert!(maps.m0[[3, 3]].is_finite());
        assert!(maps.notes.iter().any(|n| n.contains("velocity axis")));
    }

    #[test]
    fn moment_notes_name_the_axis_velocity_frame_and_say_no_correction_is_applied() {
        let dir = tempfile::tempdir().unwrap();
        let topocentric = dir.path().join("topo.fits");
        write_line_cube_with_cards(&topocentric, &[("SPECSYS", "'TOPOCENT'")]);
        let cube = LazyCube::open(topocentric.to_str().unwrap()).unwrap();
        let maps = moment_maps(&cube, &line_config()).unwrap();
        assert!(
            maps.notes
                .iter()
                .any(|n| n.contains("axis frame TOPOCENT") && n.contains("no barycentric or heliocentric correction")),
            "{:?}",
            maps.notes
        );

        let undeclared = dir.path().join("undeclared.fits");
        write_line_cube_with_cards(&undeclared, &[("SPECSYS", "")]);
        let cube = LazyCube::open(undeclared.to_str().unwrap()).unwrap();
        let maps = moment_maps(&cube, &line_config()).unwrap();
        assert!(
            maps.notes.iter().any(|n| n.contains("no SPECSYS") && n.contains("no barycentric or heliocentric correction")),
            "{:?}",
            maps.notes
        );

        let velocity_axis = dir.path().join("vrad.fits");
        let cards = [("CTYPE3", "'VRAD'"), ("CUNIT3", "'km/s'"), ("CRVAL3", "-300.0"), ("CDELT3", "40.0"), ("SPECSYS", "'LSRK'")];
        write_cube(&velocity_axis, 8, 8, 16, &cards, |z, _, _| {
            let d = z as f64 - 7.0;
            (2.0 * (-d * d / (2.0 * 1.5 * 1.5)).exp()) as f32
        });
        let cube = LazyCube::open(velocity_axis.to_str().unwrap()).unwrap();
        let cfg = MomentConfig { z0: 1, z1: 13, continuum: None, ..line_config() };
        let maps = moment_maps(&cube, &cfg).unwrap();
        assert!(maps.notes.iter().any(|n| n.contains("axis frame LSRK")), "{:?}", maps.notes);
    }

    #[test]
    fn moment_config_deserializes_with_defaults_and_rejects_bad_conventions() {
        let cfg: MomentConfig = serde_json::from_str(r#"{"z0":8,"z1":32}"#).unwrap();
        assert_eq!(cfg.convention, VelocityConvention::Optical);
        assert_eq!(cfg.snr_threshold, DEFAULT_SNR_THRESHOLD);
        assert!(cfg.mask_below_threshold);
        assert!(cfg.rest_um.is_none());
        assert!(cfg.continuum.is_none());

        let cfg: MomentConfig = serde_json::from_str(
            r#"{"z0":1,"z1":2,"rest_um":0.65646,"convention":"radio","continuum":[[0,0],[5,6]],"snr_threshold":5,"mask_below_threshold":false}"#,
        )
        .unwrap();
        assert_eq!(cfg.convention, VelocityConvention::Radio);
        assert_eq!(cfg.continuum, Some(((0, 0), (5, 6))));
        assert_eq!(cfg.snr_threshold, 5.0);
        assert!(!cfg.mask_below_threshold);
        assert_eq!(cfg.rest_um, Some(0.65646));

        assert!(serde_json::from_str::<MomentConfig>(r#"{"z0":1,"z1":2,"convention":"sideways"}"#).is_err());
        assert!(serde_json::from_str::<MomentConfig>(r#"{"z0":1,"z1":2,"snr_threshold":-1}"#).is_err());
    }

    #[test]
    fn per_pixel_fit_handles_lines_constants_and_nan_samples() {
        let xs = [0.0, 1.0, 2.0, 10.0, 11.0, 12.0];
        let ys: Vec<f32> = xs.iter().map(|x| (0.5 + 0.02 * x) as f32).collect();
        let fit = fit_pixel(&xs, &ys, false);
        assert!((fit.intercept - 0.5).abs() < 1e-5, "{:?}", fit);
        assert!((fit.slope - 0.02).abs() < 1e-6, "{:?}", fit);
        assert!(fit.sigma.abs() < 1e-5);

        let constant = fit_pixel(&xs, &ys, true);
        let mean_y = ys.iter().map(|v| *v as f64).sum::<f64>() / ys.len() as f64;
        assert!((constant.intercept as f64 - mean_y).abs() < 1e-6);
        assert_eq!(constant.slope, 0.0);
        assert!(constant.sigma > 0.0);

        let mut with_nan = ys.clone();
        with_nan[3] = f32::NAN;
        let fit = fit_pixel(&xs, &with_nan, false);
        assert!((fit.intercept - 0.5).abs() < 1e-5);
        assert!((fit.slope - 0.02).abs() < 1e-6);

        let all_nan = vec![f32::NAN; xs.len()];
        let fit = fit_pixel(&xs, &all_nan, false);
        assert!(fit.intercept.is_nan() && fit.slope.is_nan() && fit.sigma.is_nan());

        let two = fit_pixel(&xs[..2], &ys[..2], false);
        assert!((two.intercept - 0.51).abs() < 1e-5, "{:?}", two);
        assert_eq!(two.slope, 0.0);
    }

    #[test]
    fn channel_widths_use_centred_differences_with_one_sided_edges() {
        let widths = channel_widths(&[0.0, 10.0, 30.0, 60.0]);
        assert_eq!(widths, vec![10.0, 15.0, 25.0, 30.0]);
        let decreasing = channel_widths(&[60.0, 30.0, 10.0, 0.0]);
        assert_eq!(decreasing, vec![30.0, 25.0, 15.0, 10.0]);
        assert_eq!(channel_widths(&[5.0]), vec![0.0]);
        assert!(channel_widths(&[]).is_empty());
    }

    const GOLDEN_MOMENT_NOISE: f32 = 0.01;
    const GOLDEN_MOMENT_SHA256: [(&str, &str); 2] = [
        ("windows", "18371255802c383e4c3361bec9a0fc9e5ea1cdcc062e7e6a4d40fbf916d489e9"),
        ("no_windows", "0ee908c0ca5414d5728f4e53e7285c0145aa19f3c26f31ef79a48f28dea70717"),
    ];

    fn maps_bits_sha256(maps: &MomentMaps) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        for v in maps.m0.iter().chain(maps.m1.iter()).chain(maps.m2.iter()) {
            hasher.update(v.to_bits().to_le_bytes());
        }
        hasher.update(maps.noise_per_channel.unwrap_or(f64::NAN).to_bits().to_le_bytes());
        hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect()
    }

    #[test]
    fn moment_maps_without_err_keep_their_pre_error_bits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line_golden.fits");
        write_line_cube(&path, GOLDEN_MOMENT_NOISE);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let with_windows = moment_maps(&cube, &line_config()).unwrap();
        let without_windows = moment_maps(&cube, &MomentConfig { continuum: None, ..line_config() }).unwrap();
        let hashes = vec![("windows", maps_bits_sha256(&with_windows)), ("no_windows", maps_bits_sha256(&without_windows))];
        let expected: Vec<(&str, String)> = GOLDEN_MOMENT_SHA256.iter().map(|&(k, v)| (k, v.to_string())).collect();
        assert_eq!(hashes, expected);
    }

    #[test]
    fn moment_maps_reject_bad_ranges_and_windows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.0);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let mut cfg = line_config();
        cfg.z1 = LINE_CUBE_DEPTH;
        assert!(moment_maps(&cube, &cfg).unwrap_err().to_string().contains("out of range"));
        let mut cfg = line_config();
        cfg.z0 = 20;
        cfg.z1 = 20;
        assert!(moment_maps(&cube, &cfg).unwrap_err().to_string().contains("two channels"));
        let mut cfg = line_config();
        cfg.continuum = Some(((0, 5), (38, 45)));
        assert!(moment_maps(&cube, &cfg).unwrap_err().to_string().contains("continuum window"));

        let legacy = dir.path().join("legacy.fits");
        write_cube(&legacy, 4, 4, 8, &[("CRVAL3", "1.0"), ("CDELT3", "0.1")], |z, _, _| z as f32);
        let legacy_cube = LazyCube::open(legacy.to_str().unwrap()).unwrap();
        let mut cfg = line_config();
        cfg.z0 = 1;
        cfg.z1 = 6;
        cfg.continuum = None;
        cfg.rest_um = Some(1.3);
        let err = moment_maps(&legacy_cube, &cfg).unwrap_err().to_string();
        assert!(err.contains("not a spectral axis"), "{}", err);
    }

    fn open_arc(path: &std::path::Path) -> Arc<LazyCube> {
        Arc::new(LazyCube::open(path.to_str().unwrap()).unwrap())
    }

    fn bits(map: &Array2<f32>) -> Vec<u32> {
        map.iter().map(|v| v.to_bits()).collect()
    }

    fn assert_same_maps(a: &MomentMaps, b: &MomentMaps) {
        assert_eq!(bits(&a.m0), bits(&b.m0), "M0 bits");
        assert_eq!(bits(&a.m1), bits(&b.m1), "M1 bits");
        assert_eq!(bits(&a.m2), bits(&b.m2), "M2 bits");
        assert_eq!(a.noise_per_channel.map(f64::to_bits), b.noise_per_channel.map(f64::to_bits));
    }

    fn gaussian(rng: &mut StdRng) -> f64 {
        let u1: f64 = rng.gen::<f64>().max(1e-300);
        let u2: f64 = rng.gen::<f64>();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    #[test]
    fn a_cube_set_without_err_gives_todays_maps_and_no_error_planes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line_golden.fits");
        write_line_cube(&path, GOLDEN_MOMENT_NOISE);
        let set = CubeSet { sci: open_arc(&path), err: None, dq: None, notes: Vec::new() };
        let maps = moment_maps_set(&set, &line_config()).unwrap();
        assert_eq!(maps_bits_sha256(&maps), GOLDEN_MOMENT_SHA256[0].1);
        assert!(maps.m0_err.is_none() && maps.m1_err.is_none() && maps.m2_err.is_none());
        assert!(maps.noise_factor.is_none() && maps.err_hdu.is_none());
    }

    const LINE_ERR_SIGMA: f32 = 0.005_773_5;

    #[test]
    fn with_an_err_cube_the_maps_keep_their_bits_and_the_error_planes_follow_their_masks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line_golden.fits");
        write_line_cube(&path, GOLDEN_MOMENT_NOISE);
        let err_path = dir.path().join("line_err.fits");
        write_cube(&err_path, LINE_CUBE_SIZE, LINE_CUBE_SIZE, LINE_CUBE_DEPTH, &[], |_, _, _| LINE_ERR_SIGMA);
        let plain = moment_maps(&LazyCube::open(path.to_str().unwrap()).unwrap(), &line_config()).unwrap();
        let set = CubeSet { sci: open_arc(&path), err: Some(open_arc(&err_path)), dq: None, notes: Vec::new() };
        let maps = moment_maps_set(&set, &line_config()).unwrap();
        assert_same_maps(&maps, &plain);
        assert_eq!(maps.err_hdu, Some(0));
        let planes = [
            ("M0", &maps.m0, maps.m0_err.as_ref().expect("M0 error plane")),
            ("M1", &maps.m1, maps.m1_err.as_ref().expect("M1 error plane")),
            ("M2", &maps.m2, maps.m2_err.as_ref().expect("M2 error plane")),
        ];
        for (name, map, err) in planes {
            assert_eq!(err.dim(), map.dim());
            for (value, sigma) in map.iter().zip(err.iter()) {
                assert_eq!(value.is_finite(), sigma.is_finite(), "{}: value {} error {}", name, value, sigma);
                assert!(!sigma.is_finite() || *sigma > 0.0, "{}: error {}", name, sigma);
            }
            assert!(map.iter().any(|v| v.is_nan()) && map.iter().any(|v| v.is_finite()), "{}", name);
        }
        assert!(maps.notes.iter().any(|n| n.contains("ERR cube (HDU 0)") && n.contains("positive channels")), "{:?}", maps.notes);
    }

    const FLAT_LEVEL: f32 = 2.0;
    const FLAT_ERR: f32 = 0.25;
    const FLAT_SIZE: usize = 6;
    const FLAT_CONSTANT_WINDOW: (usize, usize) = (0, 5);
    const FLAT_LINEAR_WINDOWS: ContinuumWindows = ((0, 5), (34, 39));
    const BASELINE_TOLERANCE: f64 = 1e-6;

    fn flat_set(dir: &tempfile::TempDir) -> CubeSet {
        let path = dir.path().join("flat.fits");
        write_cube(&path, FLAT_SIZE, FLAT_SIZE, LINE_CUBE_DEPTH, &line_cube_cards(), |_, _, _| FLAT_LEVEL);
        let err_path = dir.path().join("flat_err.fits");
        write_cube(&err_path, FLAT_SIZE, FLAT_SIZE, LINE_CUBE_DEPTH, &[], |_, _, _| FLAT_ERR);
        CubeSet { sci: open_arc(&path), err: Some(open_arc(&err_path)), dq: None, notes: Vec::new() }
    }

    fn flat_config(continuum: Option<ContinuumWindows>) -> MomentConfig {
        MomentConfig { continuum, mask_below_threshold: false, ..line_config() }
    }

    fn m0_error_at_centre(maps: &MomentMaps) -> f64 {
        let err = maps.m0_err.as_ref().expect("M0 error plane");
        let centre = err[[FLAT_SIZE / 2, FLAT_SIZE / 2]] as f64;
        assert!(err.iter().all(|e| (*e as f64 - centre).abs() <= 1e-6 * centre), "{:?}", err);
        centre
    }

    #[test]
    fn the_baseline_term_of_the_m0_error_follows_the_least_squares_covariance_on_a_flat_continuum() {
        let dir = tempfile::tempdir().unwrap();
        let set = flat_set(&dir);
        let cfg = line_config();
        let dv = channel_width_kms();
        let n_range = (cfg.z1 - cfg.z0 + 1) as f64;
        let s2 = (FLAT_ERR as f64).powi(2);
        let direct = n_range * dv * dv * s2;

        let raw = moment_maps_set(&set, &flat_config(None)).unwrap();
        assert_within_relative("M0 error without windows", m0_error_at_centre(&raw).powi(2), direct, BASELINE_TOLERANCE);

        let constant = moment_maps_set(&set, &flat_config(Some((FLAT_CONSTANT_WINDOW, FLAT_CONSTANT_WINDOW)))).unwrap();
        let n_cont = (FLAT_CONSTANT_WINDOW.1 - FLAT_CONSTANT_WINDOW.0 + 1) as f64;
        let constant_base = s2 / n_cont * (n_range * dv).powi(2);
        assert!(constant_base > direct, "the baseline term {} must matter against {}", constant_base, direct);
        assert_within_relative("M0 error with a constant fit", m0_error_at_centre(&constant).powi(2), direct + constant_base, BASELINE_TOLERANCE);

        let linear = moment_maps_set(&set, &flat_config(Some(FLAT_LINEAR_WINDOWS))).unwrap();
        let xs: Vec<f64> = (FLAT_LINEAR_WINDOWS.0 .0..=FLAT_LINEAR_WINDOWS.0 .1)
            .chain(FLAT_LINEAR_WINDOWS.1 .0..=FLAT_LINEAR_WINDOWS.1 .1)
            .map(|z| z as f64)
            .collect();
        let (n, sx, sxx) = (xs.len() as f64, xs.iter().sum::<f64>(), xs.iter().map(|x| x * x).sum::<f64>());
        let det = n * sxx - sx * sx;
        let (c00, c01, c11) = (s2 * sxx / det, -s2 * sx / det, s2 * n / det);
        let s0 = n_range * dv;
        let s1: f64 = (cfg.z0..=cfg.z1).map(|z| dv * z as f64).sum();
        let linear_base = s0 * s0 * c00 + 2.0 * s0 * s1 * c01 + s1 * s1 * c11;
        assert!(linear_base > 0.0);
        assert_within_relative("M0 error with a linear fit", m0_error_at_centre(&linear).powi(2), direct + linear_base, BASELINE_TOLERANCE);
    }

    fn assert_within_relative(label: &str, measured: f64, expected: f64, tolerance: f64) {
        let ratio = measured / expected;
        assert!((ratio - 1.0).abs() <= tolerance, "{}: measured {} expected {} ratio {}", label, measured, expected, ratio);
    }

    const AC2_BLOCK_COLS: usize = 12;
    const AC2_BLOCK_ROWS: usize = 16;
    const AC2_REALISATIONS: usize = 300;
    const AC2_SEED: u64 = 20_261_016;
    const AC2_SIGMA: f64 = 1.0;
    const AC2_PEAK_SNR: f64 = 20.0;
    const AC2_CENTRE_SPAN_CHANNELS: f64 = 6.0;
    const AC2_CONTINUUM: (f64, f64) = (5.0, 0.01);
    const AC2_RANGE: (usize, usize) = (13, 27);
    const AC2_WINDOWS: ContinuumWindows = ((0, 6), (33, 39));
    const AC2_TOLERANCE: f64 = 0.05;
    const AC2_CORE_SIGMAS: f64 = 2.0;
    const AC2_MIN_CORE_FRACTION: f64 = 0.8;
    const AC2_MIN_PIXELS: usize = 100;
    const AC2_FACTOR_RANGE: (f64, f64) = (0.97, 1.03);
    const AC2_ROWS: usize = AC2_BLOCK_ROWS * AC2_REALISATIONS;

    fn ac2_centre(x: usize) -> f64 {
        LINE_CENTRE_CHANNEL - AC2_CENTRE_SPAN_CHANNELS / 2.0
            + AC2_CENTRE_SPAN_CHANNELS * x as f64 / (AC2_BLOCK_COLS - 1) as f64
    }

    fn ac2_model(z: usize, x: usize) -> f64 {
        let d = z as f64 - ac2_centre(x);
        let line = AC2_PEAK_SNR * AC2_SIGMA * (-d * d / (2.0 * LINE_SIGMA_CHANNELS * LINE_SIGMA_CHANNELS)).exp();
        AC2_CONTINUUM.0 + AC2_CONTINUUM.1 * z as f64 + line
    }

    fn ac2_noise() -> Vec<f32> {
        let mut rng = StdRng::seed_from_u64(AC2_SEED);
        (0..AC2_BLOCK_COLS * AC2_ROWS * LINE_CUBE_DEPTH).map(|_| (AC2_SIGMA * gaussian(&mut rng)) as f32).collect()
    }

    fn ac2_value(noise: &[f32], z: usize, y: usize, x: usize) -> f32 {
        (ac2_model(z, x) + noise[(z * AC2_ROWS + y) * AC2_BLOCK_COLS + x] as f64) as f32
    }

    fn ac2_config() -> MomentConfig {
        MomentConfig { z0: AC2_RANGE.0, z1: AC2_RANGE.1, continuum: Some(AC2_WINDOWS), ..line_config() }
    }

    fn ac2_set(dir: &tempfile::TempDir, err_scale: f64) -> CubeSet {
        let noise = ac2_noise();
        let path = dir.path().join("ac2_sci.fits");
        write_cube(&path, AC2_BLOCK_COLS, AC2_ROWS, LINE_CUBE_DEPTH, &line_cube_cards(), |z, y, x| ac2_value(&noise, z, y, x));
        let err_path = dir.path().join(format!("ac2_err_{}.fits", err_scale));
        write_cube(&err_path, AC2_BLOCK_COLS, AC2_ROWS, LINE_CUBE_DEPTH, &[], |_, _, _| (AC2_SIGMA * err_scale) as f32);
        CubeSet { sci: open_arc(&path), err: Some(open_arc(&err_path)), dq: None, notes: Vec::new() }
    }

    fn velocities_of(cube: &LazyCube, cfg: &MomentConfig) -> Vec<f64> {
        velocities_for(&cube.spectral_axis().unwrap(), cfg, &mut Vec::new()).unwrap()
    }

    fn ac2_positive_core_fraction(noise: &[f32], m1: &Array2<f32>, velocities: &[f64], block_y: usize, x: usize) -> Option<f64> {
        let window = window_channels(AC2_WINDOWS, LINE_CUBE_DEPTH).unwrap();
        let xs: Vec<f64> = window.iter().map(|&z| z as f64).collect();
        let half_width = AC2_CORE_SIGMAS * LINE_SIGMA_CHANNELS * channel_width_kms();
        let mut fractions = Vec::with_capacity(AC2_REALISATIONS);
        for r in 0..AC2_REALISATIONS {
            let y = r * AC2_BLOCK_ROWS + block_y;
            let centre = m1[[y, x]] as f64;
            let ys: Vec<f32> = window.iter().map(|&z| ac2_value(noise, z, y, x)).collect();
            let fit = fit_pixel(&xs, &ys, false);
            let positive: Vec<usize> = (AC2_RANGE.0..=AC2_RANGE.1)
                .filter(|&z| ac2_value(noise, z, y, x) as f64 - (fit.intercept as f64 + fit.slope as f64 * z as f64) > 0.0)
                .collect();
            if !centre.is_finite() || positive.is_empty() {
                return None;
            }
            let core = positive.iter().filter(|&&z| (velocities[z] - centre).abs() <= half_width).count();
            fractions.push(core as f64 / positive.len() as f64);
        }
        Some(fractions.iter().sum::<f64>() / fractions.len() as f64)
    }

    fn monte_carlo_over_propagated(values: &Array2<f32>, errors: &Array2<f32>, block_y: usize, x: usize) -> Option<f64> {
        let samples: Vec<(f64, f64)> = (0..AC2_REALISATIONS)
            .map(|r| {
                let index = [r * AC2_BLOCK_ROWS + block_y, x];
                (values[index] as f64, errors[index] as f64)
            })
            .collect();
        if samples.iter().any(|(v, e)| !v.is_finite() || !e.is_finite()) {
            return None;
        }
        let n = samples.len() as f64;
        let mean = samples.iter().map(|s| s.0).sum::<f64>() / n;
        let monte_carlo = (samples.iter().map(|s| (s.0 - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
        let propagated = (samples.iter().map(|s| s.1 * s.1).sum::<f64>() / n).sqrt();
        Some(monte_carlo / propagated)
    }

    fn median_ratio(values: &Array2<f32>, errors: &Array2<f32>, keep: impl Fn(usize, usize) -> bool) -> (f64, usize) {
        let ratios: Vec<f64> = (0..AC2_BLOCK_ROWS)
            .flat_map(|y| (0..AC2_BLOCK_COLS).map(move |x| (y, x)))
            .filter(|&(y, x)| keep(y, x))
            .filter_map(|(y, x)| monte_carlo_over_propagated(values, errors, y, x))
            .collect();
        (crate::math::exact_median_f64(&ratios), ratios.len())
    }

    #[test]
    fn moment_errors_match_monte_carlo_within_5_percent_median_over_pixels_300_realisations_seed_20261016() {
        let dir = tempfile::tempdir().unwrap();
        let set = ac2_set(&dir, 1.0);
        let maps = moment_maps_set(&set, &ac2_config()).unwrap();
        assert_same_maps(&maps, &moment_maps(&set.sci, &ac2_config()).unwrap());
        let block = AC2_BLOCK_ROWS * AC2_BLOCK_COLS;
        let m0_err = maps.m0_err.as_ref().expect("M0 error plane");
        let m1_err = maps.m1_err.as_ref().expect("M1 error plane");
        let m2_err = maps.m2_err.as_ref().expect("M2 error plane");

        let (m0, n0) = median_ratio(&maps.m0, m0_err, |_, _| true);
        let (m1, n1) = median_ratio(&maps.m1, m1_err, |_, _| true);
        assert_eq!((n0, n1), (block, block), "every pixel passes the S/N gate in every realisation");
        assert!((m0 - 1.0).abs() <= AC2_TOLERANCE, "M0 median Monte Carlo / propagated {}", m0);
        assert!((m1 - 1.0).abs() <= AC2_TOLERANCE, "M1 median Monte Carlo / propagated {}", m1);

        let noise = ac2_noise();
        let velocities = velocities_of(&set.sci, &ac2_config());
        let line_core: Vec<bool> = (0..AC2_BLOCK_ROWS)
            .flat_map(|y| (0..AC2_BLOCK_COLS).map(move |x| (y, x)))
            .map(|(y, x)| {
                ac2_positive_core_fraction(&noise, &maps.m1, &velocities, y, x).is_some_and(|f| f >= AC2_MIN_CORE_FRACTION)
            })
            .collect();
        let (m2, n2) = median_ratio(&maps.m2, m2_err, |y, x| line_core[y * AC2_BLOCK_COLS + x]);
        assert!(n2 >= AC2_MIN_PIXELS && n2 < block, "{} line-core pixels of {}", n2, block);
        assert!((m2 - 1.0).abs() <= AC2_TOLERANCE, "M2 median Monte Carlo / propagated over {} line-core pixels {}", n2, m2);
    }

    #[test]
    fn the_map_noise_factor_is_near_one_with_the_true_err_and_halving_the_err_doubles_it_and_the_error_planes_halve() {
        let dir = tempfile::tempdir().unwrap();
        let truth = moment_maps_set(&ac2_set(&dir, 1.0), &ac2_config()).unwrap();
        let halved = moment_maps_set(&ac2_set(&dir, 0.5), &ac2_config()).unwrap();
        let factor = truth.noise_factor.expect("noise factor with continuum windows and ERR");
        assert!((AC2_FACTOR_RANGE.0..=AC2_FACTOR_RANGE.1).contains(&factor), "factor {}", factor);
        let doubled = halved.noise_factor.expect("noise factor");
        assert_within_relative("factor with half the ERR", doubled, 2.0 * factor, 1e-5);
        assert!(truth.notes.iter().any(|n| n.contains("noise factor")), "{:?}", truth.notes);
        for (full, half) in [
            (&truth.m0_err, &halved.m0_err),
            (&truth.m1_err, &halved.m1_err),
            (&truth.m2_err, &halved.m2_err),
        ] {
            let (full, half) = (full.as_ref().unwrap(), half.as_ref().unwrap());
            for (f, h) in full.iter().zip(half.iter()) {
                assert_within_relative("error with half the ERR", *h as f64, 0.5 * *f as f64, 1e-5);
            }
        }
        let windowless = moment_maps_set(&ac2_set(&dir, 1.0), &MomentConfig { continuum: None, ..ac2_config() }).unwrap();
        assert!(windowless.noise_factor.is_none());
        assert!(windowless.m0_err.is_some());
    }

    const BLANK_COLS: usize = 10;
    const BLANK_TEST_COLS: usize = 16;
    const BLANK_TEST_ROWS: usize = 64;
    const BLANK_FACTOR_TOLERANCE: f64 = 0.05;

    #[test]
    fn spaxels_with_zero_residual_scatter_stay_out_of_the_map_noise_factor_seed_20261016() {
        let dir = tempfile::tempdir().unwrap();
        let (cols, rows, depth) = (BLANK_TEST_COLS, BLANK_TEST_ROWS, LINE_CUBE_DEPTH);
        let mut rng = StdRng::seed_from_u64(AC2_SEED);
        let noise: Vec<f32> = (0..cols * rows * depth).map(|_| (AC2_SIGMA * gaussian(&mut rng)) as f32).collect();
        let path = dir.path().join("blank_border.fits");
        write_cube(&path, cols, rows, depth, &line_cube_cards(), |z, y, x| {
            if x < BLANK_COLS {
                0.0
            } else {
                (AC2_CONTINUUM.0 + AC2_CONTINUUM.1 * z as f64 + noise[(z * rows + y) * cols + x] as f64) as f32
            }
        });
        let err_path = dir.path().join("blank_border_err.fits");
        write_cube(&err_path, cols, rows, depth, &[], |_, _, _| AC2_SIGMA as f32);
        let set = CubeSet { sci: open_arc(&path), err: Some(open_arc(&err_path)), dq: None, notes: Vec::new() };
        let maps = moment_maps_set(&set, &ac2_config()).unwrap();
        let factor = maps.noise_factor.expect("noise factor");
        assert!((factor - 1.0).abs() <= BLANK_FACTOR_TOLERANCE, "factor {} with {} of {} columns zero-filled", factor, BLANK_COLS, cols);
        let noisy_pixels = (cols - BLANK_COLS) * rows;
        assert!(
            maps.notes.iter().any(|n| n.contains(&format!("from {} pixels", noisy_pixels))),
            "{:?}",
            maps.notes
        );
    }

    const HETERO_SIZE: usize = 4;
    const JACOBIAN_TOLERANCE: f64 = 1e-5;
    const SIGN_MARGIN: f64 = 1e-4;
    const MIN_TERM_SHARE: f64 = 100.0 * JACOBIAN_TOLERANCE;

    fn hetero_err(z: usize) -> f32 {
        (0.1 + 0.02 * z as f64) as f32
    }

    fn hetero_value(z: usize, centre: f64) -> f32 {
        let d = z as f64 - centre;
        (2.0 + 0.01 * z as f64 + (-d * d / (2.0 * LINE_SIGMA_CHANNELS * LINE_SIGMA_CHANNELS)).exp()) as f32
    }

    fn hetero_set(dir: &tempfile::TempDir, centre: f64) -> CubeSet {
        let path = dir.path().join(format!("hetero_{}.fits", centre));
        write_cube(&path, HETERO_SIZE, HETERO_SIZE, LINE_CUBE_DEPTH, &line_cube_cards(), |z, _, _| hetero_value(z, centre));
        let err_path = dir.path().join("hetero_err.fits");
        write_cube(&err_path, HETERO_SIZE, HETERO_SIZE, LINE_CUBE_DEPTH, &[], |z, _, _| hetero_err(z));
        CubeSet { sci: open_arc(&path), err: Some(open_arc(&err_path)), dq: None, notes: Vec::new() }
    }

    #[derive(Debug)]
    struct ExactVariance {
        total: f64,
        cross: f64,
        overlap: f64,
    }

    fn exact_variance(weights: &[f64], errs: &[f64], fitted: &[usize], linear: bool) -> ExactVariance {
        let column = |j: usize| [1.0, if linear { j as f64 } else { 0.0 }];
        let mut normal = [[0.0f64; 2]; 2];
        for &j in fitted {
            let c = column(j);
            for a in 0..2 {
                for b in 0..2 {
                    normal[a][b] += c[a] * c[b];
                }
            }
        }
        let inverse = if linear {
            let det = normal[0][0] * normal[1][1] - normal[0][1] * normal[1][0];
            [[normal[1][1] / det, -normal[0][1] / det], [-normal[1][0] / det, normal[0][0] / det]]
        } else {
            [[1.0 / normal[0][0], 0.0], [0.0, 0.0]]
        };
        let g = weights.iter().enumerate().fold([0.0, 0.0], |[g0, g1], (z, w)| [g0 + w, g1 + w * z as f64]);
        let h = [inverse[0][0] * g[0] + inverse[0][1] * g[1], inverse[1][0] * g[0] + inverse[1][1] * g[1]];
        let baseline_coefficient = |j: usize| h[0] + h[1] * j as f64;
        let mut jacobian = weights.to_vec();
        for &j in fitted {
            jacobian[j] -= baseline_coefficient(j);
        }
        let total = jacobian.iter().zip(errs).map(|(a, s)| a * a * s * s).sum();
        let overlap = -2.0 * fitted.iter().map(|&j| weights[j] * errs[j] * errs[j] * baseline_coefficient(j)).sum::<f64>();
        let cross = if linear {
            let n = fitted.len() as f64;
            let mean = fitted.iter().map(|&j| j as f64).sum::<f64>() / n;
            let sxx: f64 = fitted.iter().map(|&j| (j as f64 - mean).powi(2)).sum();
            let covariance = fitted.iter().map(|&j| errs[j] * errs[j] * (j as f64 - mean)).sum::<f64>() / (n * sxx);
            2.0 * g[0] * (g[1] - mean * g[0]) * covariance
        } else {
            0.0
        };
        ExactVariance { total, cross, overlap }
    }

    fn moment_weights(values: &[f64], velocities: &[f64], cfg: &MomentConfig, fitted: &[usize], linear: bool) -> [Vec<f64>; 3] {
        let n = fitted.len() as f64;
        let mean_x = fitted.iter().map(|&j| j as f64).sum::<f64>() / n;
        let mean_y = fitted.iter().map(|&j| values[j]).sum::<f64>() / n;
        let slope = if linear {
            let sxy: f64 = fitted.iter().map(|&j| (j as f64 - mean_x) * (values[j] - mean_y)).sum();
            let sxx: f64 = fitted.iter().map(|&j| (j as f64 - mean_x).powi(2)).sum();
            sxy / sxx
        } else {
            0.0
        };
        let line: Vec<(usize, f64)> =
            (cfg.z0..=cfg.z1).map(|z| (z, values[z] - mean_y - slope * (z as f64 - mean_x))).collect();
        assert!(line.iter().all(|(_, s)| s.abs() > SIGN_MARGIN), "a channel sits on the sign boundary: {:?}", line);
        let positive: Vec<(usize, f64)> = line.iter().copied().filter(|(_, s)| *s > 0.0).collect();
        let weight: f64 = positive.iter().map(|p| p.1).sum();
        let m1 = positive.iter().map(|&(z, s)| s * velocities[z]).sum::<f64>() / weight;
        let m2 = (positive.iter().map(|&(z, s)| s * velocities[z] * velocities[z]).sum::<f64>() / weight - m1 * m1).sqrt();
        let widths = channel_widths(velocities);
        let mut weights = [vec![0.0; values.len()], vec![0.0; values.len()], vec![0.0; values.len()]];
        for &(z, _) in &line {
            weights[0][z] = widths[z];
        }
        for &(z, _) in &positive {
            weights[1][z] = (velocities[z] - m1) / weight;
            weights[2][z] = ((velocities[z] - m1).powi(2) - m2 * m2) / (2.0 * m2 * weight);
        }
        weights
    }

    fn assert_errors_follow_the_full_jacobian(label: &str, set: &CubeSet, cfg: &MomentConfig) -> [ExactVariance; 3] {
        let maps = moment_maps_set(set, cfg).unwrap();
        let windows = cfg.continuum.unwrap();
        let linear = windows.0 != windows.1;
        let fitted = window_channels(windows, LINE_CUBE_DEPTH).unwrap();
        let values: Vec<f64> = set.sci.extract_spectrum_at(0, 0).unwrap().iter().map(|&v| v as f64).collect();
        let errs: Vec<f64> = set.err.as_ref().unwrap().extract_spectrum_at(0, 0).unwrap().iter().map(|&e| e as f64).collect();
        let velocities = velocities_of(&set.sci, cfg);
        let weights = moment_weights(&values, &velocities, cfg, &fitted, linear);
        let planes = [&maps.m0_err, &maps.m1_err, &maps.m2_err];
        let exact: Vec<ExactVariance> = weights.iter().map(|w| exact_variance(w, &errs, &fitted, linear)).collect();
        for (name, (plane, expected)) in ["M0", "M1", "M2"].iter().zip(planes.iter().zip(&exact)) {
            let plane = plane.as_ref().expect("error plane");
            for e in plane.iter() {
                assert_within_relative(&format!("{} {} error squared", label, name), (*e as f64).powi(2), expected.total, JACOBIAN_TOLERANCE);
            }
        }
        exact.try_into().unwrap()
    }

    fn hetero_config(range: (usize, usize), windows: ContinuumWindows) -> MomentConfig {
        MomentConfig { z0: range.0, z1: range.1, continuum: Some(windows), mask_below_threshold: false, ..line_config() }
    }

    #[test]
    fn a_channel_dependent_err_propagates_through_the_level_slope_cross_covariance_of_the_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let set = hetero_set(&dir, 14.0);
        let exact = assert_errors_follow_the_full_jacobian("disjoint windows", &set, &hetero_config((8, 22), ((0, 3), (30, 39))));
        for (name, e) in ["M0", "M1", "M2"].iter().zip(&exact) {
            assert_eq!(e.overlap, 0.0, "{}", name);
            assert!(e.cross.abs() >= MIN_TERM_SHARE * e.total, "{}: the cross term {} must matter against {}", name, e.cross, e.total);
        }
    }

    #[test]
    fn continuum_windows_inside_the_range_subtract_their_covariance_with_the_fitted_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let set = hetero_set(&dir, LINE_CENTRE_CHANNEL);
        for (label, windows) in [("linear overlap", ((0, 13), (27, 39))), ("constant overlap", ((0, 13), (0, 13)))] {
            let exact = assert_errors_follow_the_full_jacobian(label, &set, &hetero_config((10, 30), windows));
            for (name, e) in ["M0", "M1", "M2"].iter().zip(&exact) {
                assert!(e.overlap.abs() >= MIN_TERM_SHARE * e.total, "{} {}: the overlap term {} must matter against {}", label, name, e.overlap, e.total);
            }
        }

        let flat = flat_set(&dir);
        let cfg = MomentConfig { z0: 10, z1: 30, ..flat_config(Some(((0, 15), (25, 39)))) };
        let maps = moment_maps_set(&flat, &cfg).unwrap();
        let fitted = window_channels(cfg.continuum.unwrap(), LINE_CUBE_DEPTH).unwrap();
        let dv = channel_width_kms();
        let weights: Vec<f64> = (0..LINE_CUBE_DEPTH).map(|z| if (10..=30).contains(&z) { dv } else { 0.0 }).collect();
        let errs = vec![FLAT_ERR as f64; LINE_CUBE_DEPTH];
        let exact = exact_variance(&weights, &errs, &fitted, true);
        let unit = dv * dv * (FLAT_ERR as f64).powi(2);
        assert!((exact.total / unit - 18.9628).abs() < 1e-3, "{}", exact.total / unit);
        assert_within_relative("flat overlap M0 error squared", m0_error_at_centre(&maps).powi(2), exact.total, BASELINE_TOLERANCE);
    }

    #[test]
    fn a_dq_companion_is_named_as_not_applied_to_the_moment_maps() {
        let dir = tempfile::tempdir().unwrap();
        let mut set = flat_set(&dir);
        let cfg = flat_config(Some(FLAT_LINEAR_WINDOWS));
        let plain = moment_maps_set(&set, &cfg).unwrap();
        assert!(!plain.notes.iter().any(|n| n == DQ_NOT_APPLIED_NOTE), "{:?}", plain.notes);
        let dq_path = dir.path().join("flat_dq.fits");
        write_cube(&dq_path, FLAT_SIZE, FLAT_SIZE, LINE_CUBE_DEPTH, &[], |_, _, _| 1.0);
        set.dq = Some(open_arc(&dq_path));
        let with_dq = moment_maps_set(&set, &cfg).unwrap();
        assert!(with_dq.notes.iter().any(|n| n == DQ_NOT_APPLIED_NOTE), "{:?}", with_dq.notes);
        assert_same_maps(&with_dq, &plain);
    }

    #[test]
    fn the_moment_footprint_grows_with_err_and_a_cube_above_the_memory_budget_is_refused_by_name() {
        use crate::core::stacking::memory::{MemoryInfo, MEMORY_BUDGET_ENV};
        let (rows, cols, windows) = (2048, 2048, 14);
        let npix = (rows * cols) as u64;
        let plain = moment_memory_bytes(rows, cols, Some(windows), false, false);
        let with_err = moment_memory_bytes(rows, cols, Some(windows), true, false);
        let overlapping = moment_memory_bytes(rows, cols, Some(windows), true, true);
        let accumulator = std::mem::size_of::<PixelAccumulator>() as u64;
        let error_state = (std::mem::size_of::<ErrorAccumulator>() + std::mem::size_of::<BaselineCovariance>()) as u64;
        assert!(plain >= npix * accumulator, "{}", plain);
        assert!(with_err >= npix * (accumulator + error_state) && with_err > plain, "{} vs {}", with_err, plain);
        assert!(overlapping >= with_err + npix * std::mem::size_of::<OverlapAccumulator>() as u64);

        let budget = MemoryInfo { physical_available: 512 << 20, commit_available: 512 << 20 };
        let refused = check_moment_memory(with_err, rows, cols, true, Some(budget)).unwrap_err().to_string();
        for part in ["moment maps", "2048x2048", "ERR", MEMORY_BUDGET_ENV] {
            assert!(refused.contains(part), "{} missing from: {}", part, refused);
        }
        assert!(check_moment_memory(with_err, rows, cols, true, None).is_ok());
        let enough = MemoryInfo { physical_available: with_err, commit_available: with_err };
        assert!(check_moment_memory(with_err, rows, cols, true, Some(enough)).is_ok());

        let dir = tempfile::tempdir().unwrap();
        let set = flat_set(&dir);
        let tiny = MemoryInfo { physical_available: 1 << 10, commit_available: 1 << 10 };
        let cfg = flat_config(Some(FLAT_LINEAR_WINDOWS));
        let refused = moment_maps_within(&set.sci, set.err.as_deref(), &cfg, Some(tiny)).unwrap_err().to_string();
        assert!(refused.contains("moment maps") && refused.contains(MEMORY_BUDGET_ENV), "{}", refused);
        assert!(moment_maps_within(&set.sci, set.err.as_deref(), &cfg, None).is_ok());
    }
}
