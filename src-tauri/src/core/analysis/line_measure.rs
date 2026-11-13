use serde::Serialize;

use crate::core::astrometry::spectral::{
    velocity_kms, wavelength_um_from_frequency_ghz, AxisKind, VelocityConvention, SPEED_OF_LIGHT_KMS,
};
use crate::core::cube::linefit::{error_scale, Weighting};
use crate::core::cube::moments::channel_widths;
use crate::core::cube::noise_factor::{EXPECTED_FACTOR_RANGE, WARNING_STANDARD_ERRORS};
use crate::math::gauss_newton::{absolute_errors, fit_least_squares};
use crate::math::{exact_mad_mut, exact_median_mut};
use crate::types::constants::MAD_TO_SIGMA;

pub use crate::core::cube::moments::ContinuumWindows;

pub const FWHM_PER_SIGMA: f64 = 2.354_820_045_030_949;
pub const MIN_LINE_CHANNELS: usize = 3;
pub const MAX_FIT_ITERATIONS: usize = 50;
pub const MODEL_NONE: &str = "none";
pub const MODEL_GAUSSIAN: &str = "gaussian";

const SIGMA_LOWER_BOUND_FRACTION: f64 = 0.25;
const SLOPE_CONDITION_EPSILON: f64 = 1e-12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineModel {
    None,
    Gaussian,
}

impl LineModel {
    pub fn parse(name: &str) -> Result<LineModel, String> {
        match name.trim().to_lowercase().as_str() {
            MODEL_NONE | "" => Ok(LineModel::None),
            MODEL_GAUSSIAN | "gauss" => Ok(LineModel::Gaussian),
            other => Err(format!("unknown line model '{}': use {} or {}", other, MODEL_NONE, MODEL_GAUSSIAN)),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            LineModel::None => MODEL_NONE,
            LineModel::Gaussian => MODEL_GAUSSIAN,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GaussianFit {
    pub amplitude: f64,
    pub centre: f64,
    pub sigma: f64,
    pub amplitude_err: f64,
    pub centre_err: f64,
    pub sigma_err: f64,
    pub chi2: f64,
    pub dof: usize,
    pub iterations: usize,
    pub converged: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LineVelocity {
    pub centroid_kms: f64,
    pub sigma_kms: f64,
    pub fwhm_kms: f64,
    pub rest_um: f64,
    pub convention: VelocityConvention,
    pub shift_applied_kms: f64,
    pub axis_frame: Option<String>,
    pub redshift: f64,
    pub source_centroid_kms: f64,
    pub source_sigma_kms: f64,
    pub source_fwhm_kms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LineMeasurement {
    pub z0: usize,
    pub z1: usize,
    pub n_channels: usize,
    pub n_channels_nominal: usize,
    pub axis_unit: String,
    pub flux_unit: String,
    pub continuum_level: f64,
    pub continuum_slope: f64,
    pub continuum_sigma: f64,
    pub continuum_chi2_red: Option<f64>,
    pub continuum_reference: f64,
    pub continuum_linear: bool,
    pub continuum_channels: usize,
    pub continuum_windows: ContinuumWindows,
    pub flux: f64,
    pub flux_err: f64,
    pub equivalent_width: f64,
    pub centroid: f64,
    pub sigma: f64,
    pub fwhm: f64,
    pub peak: f64,
    pub peak_channel: usize,
    pub snr: f64,
    pub velocity: Option<LineVelocity>,
    pub fit: Option<GaussianFit>,
    pub err_used: bool,
    pub flux_err_terms: Option<FluxErrorTerms>,
    pub n_dq_excluded: usize,
    pub n_nonfinite: usize,
    pub n_bad_err: usize,
    pub notes: Vec<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct FluxErrorTerms {
    pub pixel: f64,
    pub level: f64,
    pub slope: f64,
}

impl FluxErrorTerms {
    pub fn total(&self) -> f64 {
        (self.pixel * self.pixel + self.level * self.level + self.slope * self.slope).sqrt()
    }

    fn scaled(self, factor: f64) -> FluxErrorTerms {
        FluxErrorTerms { pixel: self.pixel * factor, level: self.level * factor, slope: self.slope * factor }
    }
}

struct Channels<'a> {
    flux: &'a [f32],
    err: Option<&'a [f64]>,
    excluded: Option<&'a [bool]>,
}

impl Channels<'_> {
    fn finite(&self, z: usize) -> bool {
        self.flux[z].is_finite()
    }

    fn flagged(&self, z: usize) -> bool {
        self.excluded.is_some_and(|mask| mask[z])
    }

    fn bad_err(&self, z: usize) -> bool {
        self.err.is_some_and(|err| !(err[z].is_finite() && err[z] > 0.0))
    }

    fn usable(&self, z: usize) -> bool {
        self.finite(z) && !self.flagged(z) && !self.bad_err(z)
    }
}

struct Exclusions {
    considered: usize,
    dq: usize,
    nonfinite: usize,
    bad_err: usize,
}

impl Exclusions {
    fn among(channels: &Channels, mut considered: Vec<usize>) -> Exclusions {
        considered.sort_unstable();
        considered.dedup();
        let count = |reason: &dyn Fn(usize) -> bool| considered.iter().filter(|&&z| reason(z)).count();
        Exclusions {
            considered: considered.len(),
            dq: count(&|z| channels.flagged(z)),
            nonfinite: count(&|z| !channels.finite(z)),
            bad_err: count(&|z| channels.bad_err(z)),
        }
    }

    fn reasons(&self, channels: &Channels) -> String {
        let mut reasons = Vec::new();
        if channels.excluded.is_some() {
            reasons.push(format!("{} DQ-flagged", self.dq));
        }
        reasons.push(format!("{} with non-finite flux", self.nonfinite));
        if channels.err.is_some() {
            reasons.push(format!("{} with ERR <= 0 or not finite", self.bad_err));
        }
        reasons.join(", ")
    }

    fn note(&self, channels: &Channels) -> Option<String> {
        if self.dq + self.nonfinite + self.bad_err == 0 {
            return None;
        }
        Some(format!(
            "excluded among the {} line and continuum channels: {}; a channel can count under several reasons",
            self.considered,
            self.reasons(channels)
        ))
    }
}

struct ContinuumCovariance {
    x_mean: f64,
    level_variance: f64,
    slope_variance: f64,
}

struct ContinuumFit {
    x_ref: f64,
    intercept: f64,
    slope: f64,
    sigma: f64,
    channels: usize,
    linear: bool,
    covariance: ContinuumCovariance,
    chi2: f64,
    dof: usize,
}

fn fit_linear(xs: &[f64], ys: &[f64], constant_only: bool) -> (f64, f64, ContinuumCovariance) {
    let n = xs.len() as f64;
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();
    let denominator = n * sxx - sx * sx;
    let x_mean = sx / n;
    if constant_only || xs.len() < 3 || denominator.abs() <= SLOPE_CONDITION_EPSILON * n * sxx.max(1.0) {
        return (sy / n, 0.0, ContinuumCovariance { x_mean, level_variance: 1.0 / n, slope_variance: 0.0 });
    }
    let slope = (n * sxy - sx * sy) / denominator;
    let centred: f64 = xs.iter().map(|x| (x - x_mean) * (x - x_mean)).sum();
    ((sy - slope * sx) / n, slope, ContinuumCovariance { x_mean, level_variance: 1.0 / n, slope_variance: 1.0 / centred })
}

fn fit_weighted_linear(xs: &[f64], ys: &[f64], ws: &[f64], constant_only: bool) -> (f64, f64, ContinuumCovariance) {
    let s: f64 = ws.iter().sum();
    let x_mean = ws.iter().zip(xs).map(|(w, x)| w * x).sum::<f64>() / s;
    let y_mean = ws.iter().zip(ys).map(|(w, y)| w * y).sum::<f64>() / s;
    let sxx: f64 = ws.iter().zip(xs).map(|(w, x)| w * (x - x_mean) * (x - x_mean)).sum();
    let spread: f64 = ws.iter().zip(xs).map(|(w, x)| w * x * x).sum();
    if constant_only || xs.len() < 3 || sxx <= SLOPE_CONDITION_EPSILON * spread {
        return (y_mean, 0.0, ContinuumCovariance { x_mean, level_variance: 1.0 / s, slope_variance: 0.0 });
    }
    let sxy: f64 = ws.iter().zip(xs).zip(ys).map(|((w, x), y)| w * (x - x_mean) * (y - y_mean)).sum();
    let slope = sxy / sxx;
    (y_mean - slope * x_mean, slope, ContinuumCovariance { x_mean, level_variance: 1.0 / s, slope_variance: 1.0 / sxx })
}

fn robust_sigma(residuals: &mut Vec<f32>) -> f64 {
    if residuals.len() < 2 {
        return f64::NAN;
    }
    let median = exact_median_mut(residuals) as f32;
    exact_mad_mut(residuals, median) as f64 * MAD_TO_SIGMA
}

fn check_window(window: (usize, usize), depth: usize) -> Result<(), String> {
    if window.0 > window.1 || window.1 >= depth {
        return Err(format!(
            "continuum window {}..={} is invalid for an axis with {} channels",
            window.0, window.1, depth
        ));
    }
    Ok(())
}

fn windows_overlap(a: (usize, usize), b: (usize, usize)) -> bool {
    a != b && a.0 <= b.1 && b.0 <= a.1
}

fn automatic_windows(line: (usize, usize), depth: usize, notes: &mut Vec<String>) -> Result<ContinuumWindows, String> {
    let width = line.1 - line.0 + 1;
    let left = (line.0 > 0).then(|| (line.0.saturating_sub(width), line.0 - 1));
    let right = (line.1 + 1 < depth).then(|| (line.1 + 1, (line.1 + width).min(depth - 1)));
    let windows = match (left, right) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, a),
        (None, Some(b)) => (b, b),
        (None, None) => {
            return Err(format!(
                "no continuum channels outside the line range {}..={} on an axis with {} channels",
                line.0, line.1, depth
            ))
        }
    };
    notes.push(format!(
        "no continuum windows given: channels {}..={} and {}..={} outside the line were used",
        windows.0 .0, windows.0 .1, windows.1 .0, windows.1 .1
    ));
    Ok(windows)
}

fn fit_continuum(
    axis: &[f64],
    channels: &Channels,
    windows: ContinuumWindows,
    x_ref: f64,
    notes: &mut Vec<String>,
) -> Result<ContinuumFit, String> {
    let flux = channels.flux;
    let mut window_channels: Vec<usize> = (windows.0 .0..=windows.0 .1).chain(windows.1 .0..=windows.1 .1).collect();
    window_channels.sort_unstable();
    window_channels.dedup();
    let usable: Vec<usize> = window_channels.iter().copied().filter(|&z| channels.usable(z)).collect();
    if usable.is_empty() {
        return Err(if window_channels.iter().any(|&z| channels.finite(z)) {
            "the continuum windows contain no usable channels: every finite one is DQ-flagged or has ERR <= 0".to_string()
        } else {
            "the continuum windows contain no finite flux values".to_string()
        });
    }
    let xs: Vec<f64> = usable.iter().map(|&z| axis[z] - x_ref).collect();
    let ys: Vec<f64> = usable.iter().map(|&z| flux[z] as f64).collect();
    let empty_window = [windows.0, windows.1]
        .into_iter()
        .find(|w| !(w.0..=w.1).any(|z| z < flux.len() && channels.usable(z)))
        .filter(|_| windows.0 != windows.1);
    if let Some(w) = empty_window {
        let missing = if (w.0..=w.1).any(|z| z < flux.len() && channels.finite(z)) {
            "no usable channel"
        } else {
            "no finite flux"
        };
        notes.push(format!(
            "continuum window {}..={} has {}: a constant was fitted to the other window instead of extrapolating a slope",
            w.0, w.1, missing
        ));
    }
    let linear = windows.0 != windows.1 && empty_window.is_none() && xs.len() >= 3;
    let weights: Option<Vec<f64>> =
        channels.err.map(|err| usable.iter().map(|&z| 1.0 / (err[z] * err[z])).collect());
    let (intercept, slope, covariance) = match &weights {
        Some(ws) => fit_weighted_linear(&xs, &ys, ws, !linear),
        None => fit_linear(&xs, &ys, !linear),
    };
    let residuals: Vec<f64> = xs.iter().zip(&ys).map(|(x, y)| y - (intercept + slope * x)).collect();
    let chi2: f64 = match &weights {
        Some(ws) => residuals.iter().zip(ws).map(|(r, w)| w * r * r).sum(),
        None => residuals.iter().map(|r| r * r).sum(),
    };
    let fitted_parameters = if covariance.slope_variance > 0.0 { 2 } else { 1 };
    let mut narrowed: Vec<f32> = residuals.iter().map(|&r| r as f32).collect();
    Ok(ContinuumFit {
        x_ref,
        intercept,
        slope,
        sigma: robust_sigma(&mut narrowed),
        channels: xs.len(),
        linear,
        covariance,
        chi2,
        dof: xs.len() - fitted_parameters,
    })
}

fn validate_axis(axis: &[f64], flux: &[f32], line: (usize, usize)) -> Result<(), String> {
    if axis.len() != flux.len() {
        return Err(format!(
            "spectral axis has {} values but the spectrum has {} channels",
            axis.len(),
            flux.len()
        ));
    }
    if line.0 > line.1 || line.1 >= axis.len() {
        return Err(format!(
            "line range {}..={} is invalid for a spectrum with {} channels",
            line.0,
            line.1,
            axis.len()
        ));
    }
    if line.1 - line.0 + 1 < MIN_LINE_CHANNELS {
        return Err(format!(
            "line range {}..={} needs at least {} channels",
            line.0, line.1, MIN_LINE_CHANNELS
        ));
    }
    if axis.iter().any(|v| !v.is_finite()) {
        return Err("spectral axis contains non-finite values".to_string());
    }
    let increasing = axis.windows(2).all(|w| w[1] > w[0]);
    let decreasing = axis.windows(2).all(|w| w[1] < w[0]);
    if !(increasing || decreasing) {
        return Err("spectral axis is not monotonic: the line measurement needs strictly increasing or decreasing values".to_string());
    }
    Ok(())
}

fn check_companion_length(name: &str, len: Option<usize>, channels: usize) -> Result<(), String> {
    match len {
        Some(n) if n != channels => Err(format!("{} has {} values but the spectrum has {} channels", name, n, channels)),
        _ => Ok(()),
    }
}

pub(crate) fn resolve_windows(
    continuum: Option<ContinuumWindows>,
    line: (usize, usize),
    depth: usize,
    notes: &mut Vec<String>,
) -> Result<ContinuumWindows, String> {
    let Some(windows) = continuum else {
        return automatic_windows(line, depth, notes);
    };
    check_window(windows.0, depth)?;
    check_window(windows.1, depth)?;
    if windows_overlap(windows.0, windows.1) {
        return Err(format!(
            "continuum windows {}..={} and {}..={} overlap",
            windows.0 .0, windows.0 .1, windows.1 .0, windows.1 .1
        ));
    }
    Ok(windows)
}

struct LineSample {
    channel: usize,
    x: f64,
    excess: f64,
    continuum: f64,
    dx: f64,
}

fn flux_error_terms(samples: &[LineSample], channel_sigma: impl Fn(usize) -> f64, continuum: &ContinuumFit) -> FluxErrorTerms {
    let pixel = samples
        .iter()
        .map(|s| {
            let e = channel_sigma(s.channel) * s.dx;
            e * e
        })
        .sum::<f64>()
        .sqrt();
    let covariance = &continuum.covariance;
    let width: f64 = samples.iter().map(|s| s.dx).sum();
    let lever: f64 = samples.iter().map(|s| s.dx * (s.x - continuum.x_ref - covariance.x_mean)).sum();
    FluxErrorTerms {
        pixel,
        level: width.abs() * covariance.level_variance.sqrt(),
        slope: lever.abs() * covariance.slope_variance.sqrt(),
    }
}

fn err_scatter_note(chi2_red: f64, dof: usize) -> Option<String> {
    let factor = chi2_red.sqrt();
    let spread = (WARNING_STANDARD_ERRORS / (2.0 * dof as f64).sqrt()).exp();
    let (low, high) = EXPECTED_FACTOR_RANGE;
    let verdict = if factor / spread > high {
        "ERR understates the scatter and the flux error is scaled up to it"
    } else if factor * spread < low {
        "ERR overstates the scatter and the flux error stays at the ERR propagation"
    } else {
        return None;
    };
    Some(format!(
        "continuum residuals scatter {:.2} x ERR over {} degrees of freedom, outside [{}, {}] by more than {} standard errors: {}",
        factor, dof, low, high, WARNING_STANDARD_ERRORS, verdict
    ))
}

fn gaussian_fit(
    samples: &[LineSample],
    continuum: &ContinuumFit,
    err: Option<&[f64]>,
    x_bounds: (f64, f64),
    start: [f64; 3],
    emission: bool,
    notes: &mut Vec<String>,
) -> Option<GaussianFit> {
    let xs: Vec<f64> = samples.iter().map(|s| s.x).collect();
    let ys: Vec<f64> = samples.iter().map(|s| s.excess + s.continuum).collect();
    let weights = match err {
        Some(err) => {
            notes.push(
                "Gaussian fit weighted by 1/ERR^2: absolute-sigma errors, inflated by sqrt(chi2_red) where chi2_red > 1"
                    .to_string(),
            );
            samples.iter().map(|s| 1.0 / (err[s.channel] * err[s.channel])).collect()
        }
        None => {
            let weight = if continuum.sigma.is_finite() && continuum.sigma > 0.0 {
                1.0 / (continuum.sigma * continuum.sigma)
            } else {
                notes.push("continuum scatter is zero or unknown: the Gaussian fit used unit weights".to_string());
                1.0
            };
            vec![weight; xs.len()]
        }
    };
    let mean_dx = samples.iter().map(|s| s.dx).sum::<f64>() / samples.len() as f64;
    let span = x_bounds.1 - x_bounds.0;
    let sigma_bounds = (SIGMA_LOWER_BOUND_FRACTION * mean_dx, span.max(mean_dx));
    let amplitude_lower = if emission { 0.0 } else { f64::NEG_INFINITY };
    let lower = [amplitude_lower, x_bounds.0, sigma_bounds.0];
    let upper = [f64::INFINITY, x_bounds.1, sigma_bounds.1];
    let (level, slope, x_ref) = (continuum.intercept, continuum.slope, continuum.x_ref);
    let model = move |x: f64, p: &[f64], jac: &mut [f64]| {
        let d = x - p[1];
        let s2 = p[2] * p[2];
        let e = (-d * d / (2.0 * s2)).exp();
        jac[0] = e;
        jac[1] = p[0] * e * d / s2;
        jac[2] = p[0] * e * d * d / (s2 * p[2]);
        level + slope * (x - x_ref) + p[0] * e
    };
    match fit_least_squares(&xs, &ys, &weights, &start, &lower, &upper, MAX_FIT_ITERATIONS, model) {
        Ok(fit) => {
            if !fit.converged {
                notes.push(format!(
                    "Gaussian fit did not converge after {} iterations: the non-parametric values stand",
                    fit.iterations
                ));
            }
            let errors = match err {
                Some(_) => {
                    let inflation = error_scale(Weighting::Err, fit.chi2, fit.dof).sqrt();
                    absolute_errors(&xs, &ys, &weights, &fit.params, model).iter().map(|e| e * inflation).collect()
                }
                None => fit.errors.clone(),
            };
            Some(GaussianFit {
                amplitude: fit.params[0],
                centre: fit.params[1],
                sigma: fit.params[2],
                amplitude_err: errors[0],
                centre_err: errors[1],
                sigma_err: errors[2],
                chi2: fit.chi2,
                dof: fit.dof,
                iterations: fit.iterations,
                converged: fit.converged,
            })
        }
        Err(reason) => {
            notes.push(format!("Gaussian fit skipped: {}", reason));
            None
        }
    }
}

pub fn measure_line(
    axis: &[f64],
    flux: &[f32],
    line: (usize, usize),
    continuum: Option<ContinuumWindows>,
    model: LineModel,
) -> Result<LineMeasurement, String> {
    measure_line_with_errors(axis, flux, None, None, line, continuum, model)
}

pub fn measure_line_with_errors(
    axis: &[f64],
    flux: &[f32],
    err: Option<&[f64]>,
    excluded: Option<&[bool]>,
    line: (usize, usize),
    continuum: Option<ContinuumWindows>,
    model: LineModel,
) -> Result<LineMeasurement, String> {
    validate_axis(axis, flux, line)?;
    check_companion_length("ERR", err.map(<[f64]>::len), flux.len())?;
    check_companion_length("DQ exclusion mask", excluded.map(<[bool]>::len), flux.len())?;
    let channels = Channels { flux, err, excluded };
    let depth = axis.len();
    let mut notes = Vec::new();
    let windows = resolve_windows(continuum, line, depth, &mut notes)?;
    let x_ref = axis[(line.0 + line.1) / 2];
    let cont = fit_continuum(axis, &channels, windows, x_ref, &mut notes)?;
    let widths = channel_widths(axis);

    let samples: Vec<LineSample> = (line.0..=line.1)
        .filter(|&z| channels.usable(z))
        .map(|z| {
            let continuum = cont.intercept + cont.slope * (axis[z] - x_ref);
            LineSample { channel: z, x: axis[z], excess: flux[z] as f64 - continuum, continuum, dx: widths[z] }
        })
        .collect();
    if samples.is_empty() {
        return Err(if (line.0..=line.1).any(|z| channels.finite(z)) {
            format!(
                "line range {}..={} contains no usable channels: every finite one is DQ-flagged or has ERR <= 0",
                line.0, line.1
            )
        } else {
            format!("line range {}..={} contains no finite flux values", line.0, line.1)
        });
    }
    let n_channels = samples.len();
    let n_channels_nominal = line.1 - line.0 + 1;
    if n_channels < n_channels_nominal {
        let dropped = Exclusions::among(&channels, (line.0..=line.1).collect());
        notes.push(format!(
            "{} of the {} channels in the line range {}..={} were dropped ({}): flux, equivalent width, centroid and sigma sum the {} remaining channels and miss the share of the dropped ones, and the flux error does not include that loss",
            n_channels_nominal - n_channels,
            n_channels_nominal,
            line.0,
            line.1,
            dropped.reasons(&channels),
            n_channels
        ));
    }
    let exclusions = Exclusions::among(
        &channels,
        (line.0..=line.1).chain(windows.0 .0..=windows.0 .1).chain(windows.1 .0..=windows.1 .1).collect(),
    );
    if err.is_some() || excluded.is_some() {
        notes.extend(exclusions.note(&channels));
    }

    let flux_total: f64 = samples.iter().map(|s| s.excess * s.dx).sum();
    let mean_dx = samples.iter().map(|s| s.dx).sum::<f64>() / n_channels as f64;
    let continuum_crosses_zero =
        samples.iter().any(|s| s.continuum <= 0.0) && samples.iter().any(|s| s.continuum >= 0.0);
    let continuum_negative = samples.iter().all(|s| s.continuum < 0.0);
    let equivalent_width = if continuum_crosses_zero {
        notes.push("continuum reaches or crosses zero inside the line: equivalent width undefined".to_string());
        f64::NAN
    } else if continuum_negative {
        notes.push("continuum is negative across the line: equivalent width undefined".to_string());
        f64::NAN
    } else {
        samples.iter().map(|s| -s.excess / s.continuum * s.dx).sum()
    };
    let peak_sample = samples
        .iter()
        .max_by(|a, b| a.excess.total_cmp(&b.excess))
        .map(|s| (s.channel, s.excess))
        .unwrap_or((line.0, f64::NAN));
    let trough_sample = samples
        .iter()
        .min_by(|a, b| a.excess.total_cmp(&b.excess))
        .map(|s| (s.channel, s.excess))
        .unwrap_or((line.0, f64::NAN));
    let extremum = if flux_total < 0.0 { trough_sample } else { peak_sample };

    let emission = flux_total > 0.0;
    let (centroid, sigma) = if flux_total != 0.0 {
        if !emission {
            notes.push("flux is negative: centroid and sigma are weighted by the absorbed flux".to_string());
        }
        let centroid = samples.iter().map(|s| s.excess * s.x * s.dx).sum::<f64>() / flux_total;
        let variance = samples.iter().map(|s| s.excess * (s.x - centroid) * (s.x - centroid) * s.dx).sum::<f64>() / flux_total;
        if variance < 0.0 {
            notes.push("continuum-subtracted profile has a negative second moment: sigma undefined".to_string());
            (centroid, f64::NAN)
        } else {
            (centroid, variance.sqrt())
        }
    } else {
        notes.push("flux is not positive: centroid set to the peak channel, sigma undefined".to_string());
        (axis[peak_sample.0], f64::NAN)
    };

    let continuum_chi2_red = (err.is_some() && cont.dof > 0).then(|| cont.chi2 / cont.dof as f64);
    let flux_err_terms = match err {
        Some(err) => {
            let scale = error_scale(Weighting::Err, cont.chi2, cont.dof).sqrt();
            let terms = flux_error_terms(&samples, |z| err[z], &cont).scaled(scale);
            let scaling = match continuum_chi2_red {
                Some(chi2_red) if scale > 1.0 => {
                    format!(", each scaled by {:.3}, the square root of the reduced chi2 {:.3} of the continuum fit", scale, chi2_red)
                }
                _ => String::new(),
            };
            notes.push(format!(
                "weights 1/ERR^2 on the continuum fit; flux error from ERR with the pixel, level and slope terms in quadrature: {:.3e}, {:.3e}, {:.3e}{}",
                terms.pixel, terms.level, terms.slope, scaling
            ));
            notes.extend(continuum_chi2_red.and_then(|chi2_red| err_scatter_note(chi2_red, cont.dof)));
            Some(terms)
        }
        None => {
            let residual_rms = (cont.chi2 / cont.dof as f64).sqrt();
            if !(residual_rms.is_finite() && residual_rms > 0.0) {
                notes.push("continuum scatter is zero or unknown: flux error and SNR are not meaningful".to_string());
            }
            residual_rms.is_finite().then(|| flux_error_terms(&samples, |_| 1.0, &cont).scaled(residual_rms))
        }
    };
    let flux_err = flux_err_terms.map_or(f64::NAN, |terms| terms.total());
    let snr = flux_total / flux_err;

    let x_bounds = (axis[line.0].min(axis[line.1]), axis[line.0].max(axis[line.1]));
    let fit = match model {
        LineModel::None => None,
        LineModel::Gaussian => {
            let start = if emission {
                [peak_sample.1, centroid, if sigma.is_finite() { sigma } else { mean_dx }]
            } else {
                [trough_sample.1, axis[trough_sample.0], (x_bounds.1 - x_bounds.0) / 4.0]
            };
            gaussian_fit(&samples, &cont, err, x_bounds, start, emission, &mut notes)
        }
    };

    notes.push(format!(
        "continuum: {} fitted over {} channels in {}..={} and {}..={}, level quoted at the centroid",
        if cont.linear { "first-order line" } else { "constant" },
        cont.channels,
        windows.0 .0,
        windows.0 .1,
        windows.1 .0,
        windows.1 .1
    ));

    Ok(LineMeasurement {
        z0: line.0,
        z1: line.1,
        n_channels,
        n_channels_nominal,
        axis_unit: String::new(),
        flux_unit: String::new(),
        continuum_level: cont.intercept + cont.slope * (centroid - x_ref),
        continuum_slope: cont.slope,
        continuum_sigma: cont.sigma,
        continuum_chi2_red,
        continuum_reference: centroid,
        continuum_linear: cont.linear,
        continuum_channels: cont.channels,
        continuum_windows: windows,
        flux: flux_total,
        flux_err,
        equivalent_width,
        centroid,
        sigma,
        fwhm: sigma * FWHM_PER_SIGMA,
        peak: extremum.1,
        peak_channel: extremum.0,
        snr,
        velocity: None,
        fit,
        err_used: err.is_some(),
        flux_err_terms,
        n_dq_excluded: exclusions.dq,
        n_nonfinite: exclusions.nonfinite,
        n_bad_err: exclusions.bad_err,
        notes,
        elapsed_ms: 0,
    })
}

pub fn with_velocity(
    m: &mut LineMeasurement,
    rest_um: f64,
    redshift: Option<f64>,
    convention: VelocityConvention,
    axis_kind: AxisKind,
    axis_frame: Option<&str>,
    shift_kms: Option<f64>,
) {
    let to_um: fn(f64) -> f64 = match axis_kind {
        AxisKind::Wave | AxisKind::Awav => |x| x,
        AxisKind::Freq => wavelength_um_from_frequency_ghz,
        other => {
            m.notes.push(format!("no line velocity: axis kind {:?} is not a wavelength or frequency axis", other));
            return;
        }
    };
    if !(rest_um.is_finite() && rest_um > 0.0) {
        m.notes.push(format!("no line velocity: rest wavelength {} is not a positive number of micrometres", rest_um));
        return;
    }
    let z = redshift.unwrap_or(0.0);
    if !(z.is_finite() && z > -1.0) {
        m.notes.push(format!("no line velocity: redshift {} is not a finite number above -1", z));
        return;
    }
    let shift = shift_kms.filter(|s| s.is_finite()).unwrap_or(0.0);
    let centroid_kms = velocity_kms(to_um(m.centroid), rest_um, convention);
    let sigma_kms = if m.sigma.is_finite() {
        (velocity_kms(to_um(m.centroid + m.sigma), rest_um, convention) - centroid_kms).abs()
    } else {
        f64::NAN
    };
    let systemic_um = rest_um * (1.0 + z);
    let source_centroid_kms = velocity_kms(to_um(m.centroid), systemic_um, convention);
    let source_sigma_kms = SPEED_OF_LIGHT_KMS * (m.sigma / m.centroid).abs();
    let frame_text = match axis_frame {
        Some(frame) => format!("axis frame {}", frame),
        None => "axis frame not stated in the header (no SPECSYS)".to_string(),
    };
    m.notes.push(format!(
        "line velocity from the {} convention with rest wavelength {} um, {}{}",
        convention.name(),
        rest_um,
        frame_text,
        if shift != 0.0 { format!(", shifted by {:.3} km/s (frame correction)", shift) } else { ", no frame shift applied".to_string() }
    ));
    if redshift.is_some() {
        m.notes.push(format!(
            "redshift {}: source_centroid_kms is taken against {:.7} um, the rest wavelength x (1 + z), and source_sigma_kms is c x sigma / centroid; centroid_kms, sigma_kms and fwhm_kms are observer-frame values against the laboratory rest wavelength",
            z, systemic_um
        ));
    }
    m.velocity = Some(LineVelocity {
        centroid_kms: centroid_kms + shift,
        sigma_kms,
        fwhm_kms: sigma_kms * FWHM_PER_SIGMA,
        rest_um,
        convention,
        shift_applied_kms: shift,
        axis_frame: axis_frame.map(str::to_string),
        redshift: z,
        source_centroid_kms: source_centroid_kms + shift,
        source_sigma_kms,
        source_fwhm_kms: source_sigma_kms * FWHM_PER_SIGMA,
    });
}

#[cfg(test)]
pub(crate) mod golden {
    use std::path::{Path, PathBuf};

    use serde_json::Value;

    pub const ELAPSED_KEY: &str = "elapsed_ms";
    pub const ADDED_KEYS: [&str; 7] = [
        "continuum_chi2_red",
        "err_used",
        "flux_err_terms",
        "n_bad_err",
        "n_channels_nominal",
        "n_dq_excluded",
        "n_nonfinite",
    ];
    const FIT_KEY: &str = "fit";
    const LIBM_EXP_RELATIVE_TOLERANCE: f64 = 1e-12;
    const QUADRATURE_RELATIVE_TOLERANCE: f64 = 1e-12;

    pub fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("line_measure")
    }

    pub fn without_elapsed(mut value: Value) -> Value {
        if let Some(object) = value.as_object_mut() {
            object.remove(ELAPSED_KEY);
        }
        value
    }

    pub fn read_golden(path: &Path) -> Value {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("golden {} unreadable: {}", path.display(), e));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("golden {} is not JSON: {}", path.display(), e))
    }

    fn compare(path: &str, golden: &Value, actual: &Value, in_fit: bool, diffs: &mut Vec<String>) {
        match (golden, actual) {
            (Value::Number(g), Value::Number(a)) if g.is_f64() && a.is_f64() => {
                let (g, a) = (g.as_f64().unwrap_or(f64::NAN), a.as_f64().unwrap_or(f64::NAN));
                let same = if in_fit {
                    (g - a).abs() <= LIBM_EXP_RELATIVE_TOLERANCE * g.abs()
                } else {
                    g.to_bits() == a.to_bits()
                };
                if !same {
                    diffs.push(format!("{path}: golden {g:?}, now {a:?}"));
                }
            }
            (Value::Object(g), Value::Object(a)) => {
                for (key, gv) in g {
                    let child = format!("{path}/{key}");
                    match a.get(key) {
                        Some(av) => compare(&child, gv, av, in_fit || (path.is_empty() && key == FIT_KEY), diffs),
                        None => diffs.push(format!("{child}: missing")),
                    }
                }
            }
            (Value::Array(g), Value::Array(a)) if g.len() == a.len() => {
                for (i, (gv, av)) in g.iter().zip(a).enumerate() {
                    compare(&format!("{path}/{i}"), gv, av, in_fit, diffs);
                }
            }
            _ if golden != actual => diffs.push(format!("{path}: golden {golden}, now {actual}")),
            _ => {}
        }
    }

    pub fn golden_diffs(golden: &Value, actual: &Value) -> Vec<String> {
        let mut diffs = Vec::new();
        compare("", golden, actual, false, &mut diffs);
        let mut added: Vec<&str> = actual
            .as_object()
            .map(|a| a.keys().map(String::as_str).filter(|k| golden.get(k).is_none()).collect())
            .unwrap_or_default();
        added.sort_unstable();
        if added != ADDED_KEYS {
            diffs.push(format!("keys beyond the golden: {:?}, expected {:?}", added, ADDED_KEYS));
        }
        diffs
    }

    pub fn unweighted_diffs(actual: &Value, nonfinite: u64) -> Vec<String> {
        let nominal = actual["z1"].as_u64().zip(actual["z0"].as_u64()).map(|(z1, z0)| z1 - z0 + 1);
        let expected = [
            ("continuum_chi2_red", Value::Null),
            ("err_used", Value::Bool(false)),
            ("n_bad_err", Value::from(0u64)),
            ("n_channels_nominal", nominal.map_or(Value::Null, Value::from)),
            ("n_dq_excluded", Value::from(0u64)),
            ("n_nonfinite", Value::from(nonfinite)),
        ];
        let mut diffs: Vec<String> = expected
            .into_iter()
            .filter(|(key, value)| actual.get(key) != Some(value))
            .map(|(key, value)| {
                let now = actual.get(key).map_or_else(|| "missing".to_string(), Value::to_string);
                format!("/{key}: expected {value}, now {now}")
            })
            .collect();
        let terms = &actual["flux_err_terms"];
        let quadrature =
            ["pixel", "level", "slope"].iter().map(|term| terms[term].as_f64().unwrap_or(f64::NAN).powi(2)).sum::<f64>().sqrt();
        let terms_match_the_flux_error = match actual["flux_err"].as_f64() {
            Some(flux_err) => (quadrature - flux_err).abs() <= QUADRATURE_RELATIVE_TOLERANCE * flux_err,
            None => terms.is_null(),
        };
        if !terms_match_the_flux_error {
            diffs.push(format!("/flux_err_terms: expected the scatter terms of flux_err {}, now {}", actual["flux_err"], terms));
        }
        diffs
    }

    pub fn assert_matches_golden(path: &Path, actual: &Value, nonfinite: u64) {
        let mut diffs = golden_diffs(&read_golden(path), actual);
        diffs.extend(unweighted_diffs(actual, nonfinite));
        assert!(diffs.is_empty(), "{} differs from its golden:\n{}", path.display(), diffs.join("\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::golden::*;
    use super::*;
    use crate::core::astrometry::gwcs::test_support::documents_fits_dir;
    use crate::core::cube::lazy::test_support::*;
    use crate::core::cube::lazy::LazyCube;

    const LINE_RANGE: (usize, usize) = (8, 32);
    const WINDOWS: ContinuumWindows = ((0, 5), (34, 39));
    const EXPECTED_FLUX: f64 = 7.5199e-3;
    const CONTINUUM_LEVEL_TOLERANCE: f64 = 5e-6;

    fn centre_spectrum(noise: f32) -> (Vec<f64>, Vec<f32>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, noise);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let axis = cube.spectral_axis().unwrap();
        let centre = LINE_DISK_CENTRE as usize;
        (axis.values, cube.extract_spectrum_at(centre, centre).unwrap())
    }

    fn close(found: f64, expected: f64, relative: f64) -> bool {
        (found - expected).abs() <= relative * expected.abs()
    }

    #[test]
    fn the_centre_pixel_line_is_measured_analytically() {
        let (axis, flux) = centre_spectrum(0.0);
        let m = measure_line(&axis, &flux, LINE_RANGE, Some(WINDOWS), LineModel::Gaussian).unwrap();
        assert_eq!(m.n_channels, 25);
        assert!((m.centroid - LINE_REST_UM).abs() < 1e-6, "centroid={}", m.centroid);
        assert!(close(m.sigma, LINE_SIGMA_CHANNELS * LINE_CDELT_UM, 0.02), "sigma={}", m.sigma);
        assert!(close(m.fwhm, 0.00706, 0.02), "fwhm={}", m.fwhm);
        assert!(close(m.flux, EXPECTED_FLUX, 0.01), "flux={}", m.flux);
        assert!(close(m.equivalent_width, -EXPECTED_FLUX, 0.01), "ew={}", m.equivalent_width);
        assert!((m.continuum_level - LINE_CONTINUUM as f64).abs() < CONTINUUM_LEVEL_TOLERANCE, "level={}", m.continuum_level);
        assert!(m.continuum_sigma.abs() < 1e-5, "continuum sigma={}", m.continuum_sigma);
        assert!(m.continuum_linear);
        assert_eq!(m.continuum_channels, 12);
        assert_eq!(m.continuum_windows, WINDOWS);
        assert_eq!(m.peak_channel, 20);
        assert!((m.peak - 1.0).abs() < 1e-5);
        assert_eq!(m.continuum_reference, m.centroid);
        let fit = m.fit.as_ref().expect("gaussian fit");
        assert!(fit.converged, "{:?}", fit);
        assert!((fit.amplitude - 1.0).abs() < 1e-4, "{:?}", fit);
        assert!((fit.centre - LINE_REST_UM).abs() < 1e-4, "{:?}", fit);
        assert!((fit.sigma - 0.003).abs() < 1e-4, "{:?}", fit);
        assert!(m.velocity.is_none());
    }

    #[test]
    fn a_noisy_line_is_fitted_within_three_formal_sigma() {
        let (axis, flux) = centre_spectrum(0.05);
        let m = measure_line(&axis, &flux, LINE_RANGE, Some(WINDOWS), LineModel::Gaussian).unwrap();
        assert!(m.continuum_sigma > 0.005 && m.continuum_sigma < 0.08, "sigma={}", m.continuum_sigma);
        assert!(m.snr > 10.0, "snr={}", m.snr);
        let fit = m.fit.as_ref().expect("gaussian fit");
        assert!(fit.converged, "{:?}", fit);
        let pulls = [
            (fit.amplitude - 1.0) / fit.amplitude_err,
            (fit.centre - LINE_REST_UM) / fit.centre_err,
            (fit.sigma - 0.003) / fit.sigma_err,
        ];
        for pull in pulls {
            assert!(pull.is_finite() && pull.abs() < 3.0, "pulls={:?} fit={:?}", pulls, fit);
        }
    }

    #[test]
    fn velocities_follow_the_convention_and_the_frame_shift() {
        let (axis, flux) = centre_spectrum(0.0);
        let mut m = measure_line(&axis, &flux, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        assert!(m.fit.is_none());
        with_velocity(&mut m, LINE_REST_UM, None, VelocityConvention::Optical, AxisKind::Wave, Some("BARYCENT"), None);
        let v = m.velocity.clone().expect("velocity");
        assert!(v.centroid_kms.abs() < 0.1, "v={}", v.centroid_kms);
        let expected_sigma_kms = 299792.458 * 0.003 / LINE_REST_UM;
        assert!(close(v.sigma_kms, expected_sigma_kms, 0.03), "sigma_kms={}", v.sigma_kms);
        assert!(close(v.fwhm_kms, v.sigma_kms * FWHM_PER_SIGMA, 1e-12));
        assert_eq!(v.shift_applied_kms, 0.0);
        assert_eq!(v.axis_frame.as_deref(), Some("BARYCENT"));
        let unshifted_note = m.notes.last().expect("velocity note");
        assert!(unshifted_note.contains("axis frame BARYCENT, no frame shift applied"), "{}", unshifted_note);

        with_velocity(&mut m, LINE_REST_UM, None, VelocityConvention::Radio, AxisKind::Wave, None, Some(12.5));
        let shifted = m.velocity.clone().expect("velocity");
        assert!((shifted.centroid_kms - 12.5).abs() < 0.1, "v={}", shifted.centroid_kms);
        assert_eq!(shifted.shift_applied_kms, 12.5);
        assert_eq!(shifted.convention, VelocityConvention::Radio);
        assert!(shifted.axis_frame.is_none());
        let shifted_note = m.notes.last().expect("velocity note");
        assert!(shifted_note.contains("no SPECSYS") && shifted_note.contains("shifted by 12.500 km/s"), "{}", shifted_note);
        assert!(m.notes.iter().all(|n| !n.contains("topocentric")), "{:?}", m.notes);

        let freq_axis: Vec<f64> = axis.iter().map(|w| 299792.458 / w).collect();
        let mut on_freq = m.clone();
        on_freq.centroid = 299792.458 / LINE_REST_UM;
        on_freq.sigma = f64::NAN;
        with_velocity(&mut on_freq, LINE_REST_UM, None, VelocityConvention::Optical, AxisKind::Freq, Some("LSRK"), None);
        let on_freq_velocity = on_freq.velocity.expect("velocity");
        assert!(on_freq_velocity.centroid_kms.abs() < 1e-6);
        assert!(on_freq_velocity.sigma_kms.is_nan());
        assert!(freq_axis[0] > freq_axis[1]);

        let mut on_velocity_axis = m.clone();
        on_velocity_axis.velocity = None;
        with_velocity(&mut on_velocity_axis, LINE_REST_UM, None, VelocityConvention::Optical, AxisKind::Vrad, None, None);
        assert!(on_velocity_axis.velocity.is_none());
        assert!(on_velocity_axis.notes.iter().any(|n| n.contains("not a wavelength or frequency")));
    }

    #[test]
    fn automatic_windows_and_a_decreasing_axis_still_measure_the_line() {
        let (axis, flux) = centre_spectrum(0.0);
        let m = measure_line(&axis, &flux, LINE_RANGE, None, LineModel::None).unwrap();
        assert_eq!(m.continuum_windows, ((0, 7), (33, 39)));
        assert!(m.notes.iter().any(|n| n.contains("no continuum windows given")));
        assert!(close(m.flux, EXPECTED_FLUX, 0.01), "flux={}", m.flux);

        let reversed_axis: Vec<f64> = axis.iter().rev().copied().collect();
        let reversed_flux: Vec<f32> = flux.iter().rev().copied().collect();
        let r = measure_line(&reversed_axis, &reversed_flux, (7, 31), Some(((0, 5), (34, 39))), LineModel::Gaussian).unwrap();
        assert!(close(r.flux, EXPECTED_FLUX, 0.01), "flux={}", r.flux);
        assert!((r.centroid - LINE_REST_UM).abs() < 1e-6, "centroid={}", r.centroid);
        let fit = r.fit.expect("fit");
        assert!(fit.converged && (fit.centre - LINE_REST_UM).abs() < 1e-4, "{:?}", fit);
    }

    #[test]
    fn a_flat_spectrum_has_no_significant_flux_and_no_panic() {
        let axis: Vec<f64> = (0..40).map(|i| 1.0 + i as f64 * 0.001).collect();
        let flux: Vec<f32> = (0..40).map(|i| 1.0 + deterministic_noise(i, 0, 0, 0.05)).collect();
        let m = measure_line(&axis, &flux, LINE_RANGE, Some(WINDOWS), LineModel::Gaussian).unwrap();
        assert!(m.flux.abs() < 3.0 * m.flux_err, "flux={} err={}", m.flux, m.flux_err);
        assert!(m.snr.abs() < 3.0);
        assert!(m.centroid.is_finite());
        if let Some(fit) = m.fit {
            assert!(fit.amplitude.is_finite() && fit.centre.is_finite() && fit.sigma.is_finite());
        }

        let constant = vec![1.0f32; 40];
        let c = measure_line(&axis, &constant, LINE_RANGE, Some(WINDOWS), LineModel::Gaussian).unwrap();
        assert_eq!(c.flux, 0.0);
        assert!(c.sigma.is_nan());
        assert!(c.notes.iter().any(|n| n.contains("flux is not positive")));

        let mut with_nan = flux.clone();
        with_nan[20] = f32::NAN;
        let n = measure_line(&axis, &with_nan, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        assert_eq!(n.n_channels, 24);
        assert!(n.notes.iter().any(|n| n.contains("non-finite flux")));
    }

    #[test]
    fn invalid_ranges_windows_axes_and_models_are_refused_with_a_sentence() {
        let axis: Vec<f64> = (0..40).map(|i| 1.0 + i as f64 * 0.001).collect();
        let flux = vec![1.0f32; 40];
        let err = |line, windows| measure_line(&axis, &flux, line, windows, LineModel::None).unwrap_err();
        assert!(err((8, 40), Some(WINDOWS)).contains("invalid for a spectrum"));
        assert!(err((10, 8), Some(WINDOWS)).contains("invalid for a spectrum"));
        assert!(err((8, 9), Some(WINDOWS)).contains("at least 3 channels"));
        assert!(err(LINE_RANGE, Some(((0, 5), (38, 45)))).contains("continuum window"));
        assert!(err(LINE_RANGE, Some(((0, 5), (3, 7)))).contains("overlap"));
        assert!(err((0, 39), None).contains("no continuum channels"));
        assert!(measure_line(&axis, &flux[..39], LINE_RANGE, None, LineModel::None).unwrap_err().contains("channels"));
        let mut bumpy = axis.clone();
        bumpy[3] = bumpy[5];
        assert!(measure_line(&bumpy, &flux, LINE_RANGE, None, LineModel::None).unwrap_err().contains("monotonic"));
        let mut broken = axis.clone();
        broken[3] = f64::NAN;
        assert!(measure_line(&broken, &flux, LINE_RANGE, None, LineModel::None).unwrap_err().contains("non-finite"));
        assert!(LineModel::parse("sideways").unwrap_err().contains("unknown line model"));
        assert_eq!(LineModel::parse("Gaussian").unwrap(), LineModel::Gaussian);
        assert_eq!(LineModel::parse("").unwrap(), LineModel::None);
    }

    #[test]
    fn an_unresolved_emission_line_brushed_generously_still_converges() {
        let (axis, flux) = unresolved_line();
        let m = measure_line(&axis, &flux, (10, 50), Some(((0, 7), (53, 59))), LineModel::Gaussian).unwrap();
        let fit = m.fit.expect("gaussian fit");
        assert!(fit.converged, "{:?}", fit);
        assert!((fit.centre - UNRESOLVED_CENTRE_UM).abs() < 3.0 * fit.centre_err, "{:?}", fit);
        assert!((fit.amplitude - 800.0).abs() < 3.0 * fit.amplitude_err, "{:?}", fit);
    }

    #[test]
    fn a_continuum_crossing_zero_between_channels_leaves_the_equivalent_width_undefined() {
        let axis: Vec<f64> = (0..40).map(|i| 1.0 + i as f64 * 0.001).collect();
        for zero_at in [1.0203, 1.0197] {
            let flux: Vec<f32> = axis
                .iter()
                .map(|&x| ((-(x - 1.02) * (x - 1.02) / (2.0 * 0.003 * 0.003)).exp() - 5.0 * (x - zero_at)) as f32)
                .collect();
            let m = measure_line(&axis, &flux, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
            assert!(close(m.flux, EXPECTED_FLUX, 0.01), "flux={}", m.flux);
            assert!(m.equivalent_width.is_nan(), "ew={}", m.equivalent_width);
            assert!(m.notes.iter().any(|n| n.contains("equivalent width undefined")), "{:?}", m.notes);
        }
    }

    #[test]
    fn an_absorption_line_has_its_centroid_and_velocity_at_the_trough() {
        let axis: Vec<f64> = (0..LINE_CUBE_DEPTH).map(|z| 1.0 + z as f64 * LINE_CDELT_UM).collect();
        let flux: Vec<f32> = (0..LINE_CUBE_DEPTH).map(|z| LINE_CONTINUUM - 0.5 * line_profile(z)).collect();
        let mut m = measure_line(&axis, &flux, LINE_RANGE, Some(WINDOWS), LineModel::Gaussian).unwrap();
        assert!(m.flux < 0.0, "flux={}", m.flux);
        assert!((m.centroid - LINE_REST_UM).abs() < 1e-6, "centroid={}", m.centroid);
        assert!(close(m.sigma, LINE_SIGMA_CHANNELS * LINE_CDELT_UM, 0.02), "sigma={}", m.sigma);
        assert!((m.continuum_level - LINE_CONTINUUM as f64).abs() < CONTINUUM_LEVEL_TOLERANCE, "level={}", m.continuum_level);
        assert!(m.notes.iter().any(|n| n.contains("weighted by the absorbed flux")), "{:?}", m.notes);
        let fit = m.fit.as_ref().expect("gaussian fit");
        assert!(fit.converged && (fit.centre - LINE_REST_UM).abs() < 1e-4, "{:?}", fit);
        with_velocity(&mut m, LINE_REST_UM, None, VelocityConvention::Optical, AxisKind::Wave, None, None);
        let v = m.velocity.expect("velocity");
        assert!(v.centroid_kms.abs() < 0.1, "v={}", v.centroid_kms);
        assert!(v.sigma_kms.is_finite(), "sigma_kms={}", v.sigma_kms);
    }

    #[test]
    fn a_continuum_window_without_finite_flux_falls_back_to_a_constant() {
        let (axis, flux) = centre_spectrum(0.05);
        let mut blanked = flux.clone();
        for z in WINDOWS.1 .0..=WINDOWS.1 .1 {
            blanked[z] = f32::NAN;
        }
        let m = measure_line(&axis, &blanked, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        assert!(!m.continuum_linear);
        assert_eq!(m.continuum_slope, 0.0);
        assert_eq!(m.continuum_channels, 6);
        assert!(close(m.flux, EXPECTED_FLUX, 0.02), "flux={}", m.flux);
        assert!(m.notes.iter().any(|n| n.contains("34..=39 has no finite flux")), "{:?}", m.notes);
    }

    #[test]
    fn coinciding_windows_give_a_constant_continuum() {
        let (axis, flux) = centre_spectrum(0.0);
        let m = measure_line(&axis, &flux, LINE_RANGE, Some(((0, 5), (0, 5))), LineModel::None).unwrap();
        assert!(!m.continuum_linear);
        assert_eq!(m.continuum_slope, 0.0);
        assert_eq!(m.continuum_channels, 6);
        assert!(close(m.flux, EXPECTED_FLUX, 0.01), "flux={}", m.flux);
    }

    const G395H_X1D: &str = "jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits";
    const G395H_GOLDEN: &str = "line_measure_goldens/jw01266_g395h_br_alpha.json";
    const BR_ALPHA_LINE: (usize, usize) = (1769, 1786);
    const BR_ALPHA_CONTINUUM: ContinuumWindows = ((1750, 1762), (1789, 1800));

    struct GoldenCase {
        name: &'static str,
        nonfinite: u64,
        measurement: LineMeasurement,
    }

    fn flat_axis() -> Vec<f64> {
        (0..40).map(|i| 1.0 + i as f64 * 0.001).collect()
    }

    const UNRESOLVED_CENTRE_UM: f64 = 0.6528;

    fn unresolved_line() -> (Vec<f64>, Vec<f32>) {
        let step_um = 1.25e-4;
        let axis: Vec<f64> = (0..60).map(|i| UNRESOLVED_CENTRE_UM - 30.0 * step_um + i as f64 * step_um).collect();
        let flux: Vec<f32> = axis
            .iter()
            .enumerate()
            .map(|(z, &x)| {
                let d = (x - UNRESOLVED_CENTRE_UM) / 1.1e-4;
                (300.0 + 800.0 * (-0.5 * d * d).exp()) as f32 + deterministic_noise(z, 0, 9, 20.0 * 3f32.sqrt())
            })
            .collect();
        (axis, flux)
    }

    fn golden_cases() -> Vec<GoldenCase> {
        let (axis, clean) = centre_spectrum(0.0);
        let (_, noisy) = centre_spectrum(0.05);
        let reversed_axis: Vec<f64> = axis.iter().rev().copied().collect();
        let reversed_flux: Vec<f32> = clean.iter().rev().copied().collect();
        let mut blanked = noisy.clone();
        blanked[WINDOWS.1 .0..=WINDOWS.1 .1].fill(f32::NAN);
        let flat_axis = flat_axis();
        let flat: Vec<f32> = (0..40).map(|i| 1.0 + deterministic_noise(i, 0, 0, 0.05)).collect();
        let mut flat_with_nan = flat.clone();
        flat_with_nan[20] = f32::NAN;
        let constant = vec![1.0f32; 40];
        let absorption_axis: Vec<f64> = (0..LINE_CUBE_DEPTH).map(|z| 1.0 + z as f64 * LINE_CDELT_UM).collect();
        let absorption: Vec<f32> = (0..LINE_CUBE_DEPTH).map(|z| LINE_CONTINUUM - 0.5 * line_profile(z)).collect();
        let (unresolved_axis, unresolved) = unresolved_line();
        let run = |axis: &[f64], flux: &[f32], line, windows, model| measure_line(axis, flux, line, windows, model).unwrap();
        let case = |name, nonfinite, measurement| GoldenCase { name, nonfinite, measurement };
        let coinciding = Some(((0, 5), (0, 5)));
        vec![
            case("gaussian_line_noiseless", 0, run(&axis, &clean, LINE_RANGE, Some(WINDOWS), LineModel::Gaussian)),
            case("gaussian_line_noisy", 0, run(&axis, &noisy, LINE_RANGE, Some(WINDOWS), LineModel::Gaussian)),
            case("gaussian_line_automatic_windows", 0, run(&axis, &clean, LINE_RANGE, None, LineModel::None)),
            case(
                "gaussian_line_decreasing_axis",
                0,
                run(&reversed_axis, &reversed_flux, (7, 31), Some(WINDOWS), LineModel::Gaussian),
            ),
            case("constant_continuum_noiseless", 0, run(&axis, &clean, LINE_RANGE, coinciding, LineModel::Gaussian)),
            case("constant_continuum_noisy", 0, run(&axis, &noisy, LINE_RANGE, coinciding, LineModel::Gaussian)),
            case("constant_continuum_blanked_window", 6, run(&axis, &blanked, LINE_RANGE, Some(WINDOWS), LineModel::None)),
            case("flat_noise", 0, run(&flat_axis, &flat, LINE_RANGE, Some(WINDOWS), LineModel::Gaussian)),
            case("flat_noise_with_nan", 1, run(&flat_axis, &flat_with_nan, LINE_RANGE, Some(WINDOWS), LineModel::None)),
            case("flat_constant", 0, run(&flat_axis, &constant, LINE_RANGE, Some(WINDOWS), LineModel::Gaussian)),
            case(
                "absorption_line",
                0,
                run(&absorption_axis, &absorption, LINE_RANGE, Some(WINDOWS), LineModel::Gaussian),
            ),
            case(
                "unresolved_emission_line",
                0,
                run(&unresolved_axis, &unresolved, (10, 50), Some(((0, 7), (53, 59))), LineModel::Gaussian),
            ),
        ]
    }

    fn g395h_measurement(dir: &std::path::Path) -> LineMeasurement {
        let path = dir.join(G395H_X1D);
        let x1d = crate::core::analysis::x1d::read_x1d_spectrum(path.to_str().unwrap(), None).unwrap();
        let flux: Vec<f32> = x1d.flux.iter().map(|&v| v as f32).collect();
        measure_line(&x1d.wavelength_um, &flux, BR_ALPHA_LINE, Some(BR_ALPHA_CONTINUUM), LineModel::Gaussian).unwrap()
    }

    fn golden_value(m: &LineMeasurement) -> serde_json::Value {
        without_elapsed(serde_json::to_value(m).unwrap())
    }

    #[test]
    fn measure_line_is_bit_identical_to_the_goldens_recorded_before_the_error_aware_entry_point() {
        let cases = golden_cases();
        let mut failures = Vec::new();
        for case in &cases {
            let path = fixtures_dir().join(format!("{}.json", case.name));
            let value = golden_value(&case.measurement);
            let mut diffs = golden_diffs(&read_golden(&path), &value);
            diffs.extend(unweighted_diffs(&value, case.nonfinite));
            if !diffs.is_empty() {
                failures.push(format!("{}:\n  {}", case.name, diffs.join("\n  ")));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        assert_eq!(cases.len(), 12);
    }

    #[test]
    #[ignore = "needs ASTROBURST_DOCUMENTS_FITS_DIR with the jw01266 G395H x1d and the local golden recorded before the error-aware entry point"]
    fn the_g395h_br_alpha_measurement_is_bit_identical_to_its_local_golden() {
        let dir = documents_fits_dir();
        let value = golden_value(&g395h_measurement(&dir));
        assert_matches_golden(&dir.join(G395H_GOLDEN), &value, 0);
    }

    const ERR_SIGMA: f64 = 0.05;
    const MC_REALISATIONS: usize = 10_000;
    const MC_SEED: u64 = 20_261_016;
    const MC_RELATIVE_TOLERANCE: f64 = 0.03;
    const BAD_ERR_CHANNELS: usize = 58;

    fn root_sum_squared_widths(axis: &[f64], line: (usize, usize)) -> f64 {
        channel_widths(axis)[line.0..=line.1].iter().map(|w| w * w).sum::<f64>().sqrt()
    }

    fn terms(m: &LineMeasurement) -> FluxErrorTerms {
        m.flux_err_terms.unwrap_or_else(|| panic!("no flux error terms in {:?}", m))
    }

    #[test]
    fn with_a_constant_err_the_pixel_term_is_sigma_times_the_root_sum_of_squared_widths() {
        let (axis, flux) = centre_spectrum(0.05);
        let err = vec![ERR_SIGMA; axis.len()];
        let m = measure_line_with_errors(&axis, &flux, Some(&err), None, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        assert!(m.err_used);
        let t = terms(&m);
        let expected_pixel = ERR_SIGMA * root_sum_squared_widths(&axis, LINE_RANGE);
        assert!(close(t.pixel, expected_pixel, 1e-9), "pixel={} expected={}", t.pixel, expected_pixel);
        let quadrature = (t.pixel * t.pixel + t.level * t.level + t.slope * t.slope).sqrt();
        assert!(close(m.flux_err, quadrature, 1e-12), "flux_err={} terms={:?}", m.flux_err, t);
        let level_ratio = t.level / t.pixel;
        assert!((level_ratio - (25.0f64 / 12.0).sqrt()).abs() < 1e-6, "level/pixel={}", level_ratio);
        assert!(t.slope.is_finite() && t.slope > 0.0 && t.slope < t.pixel, "{:?}", t);
        assert!(close(m.snr, m.flux / m.flux_err, 1e-15));
        assert_eq!((m.n_dq_excluded, m.n_nonfinite, m.n_bad_err), (0, 0, 0));
        assert!(m.notes.iter().any(|n| n.contains("pixel, level and slope terms")), "{:?}", m.notes);

        let unweighted = measure_line(&axis, &flux, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        assert!(close(m.continuum_slope, unweighted.continuum_slope, 1e-9), "{} {}", m.continuum_slope, unweighted.continuum_slope);
        assert!(close(m.flux, unweighted.flux, 1e-9), "{} {}", m.flux, unweighted.flux);
        assert!(close(m.centroid, unweighted.centroid, 1e-12));
    }

    struct FluxMonteCarlo {
        scatter: f64,
        rms_flux_err: f64,
        first: LineMeasurement,
    }

    fn monte_carlo_fluxes(noise: &[f64], err: Option<&[f64]>, windows: ContinuumWindows) -> FluxMonteCarlo {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};
        let axis: Vec<f64> = (0..LINE_CUBE_DEPTH).map(|z| 1.0 + z as f64 * LINE_CDELT_UM).collect();
        let truth: Vec<f64> = (0..LINE_CUBE_DEPTH).map(|z| LINE_CONTINUUM as f64 + line_profile(z) as f64).collect();
        let mut rng = StdRng::seed_from_u64(MC_SEED);
        let mut gauss = move || {
            let u1: f64 = rng.gen::<f64>().max(1e-300);
            let u2: f64 = rng.gen::<f64>();
            (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
        };
        let mut fluxes = Vec::with_capacity(MC_REALISATIONS);
        let mut squared_errors = 0.0;
        let mut first = None;
        for _ in 0..MC_REALISATIONS {
            let flux: Vec<f32> = truth.iter().zip(noise).map(|(t, s)| (t + s * gauss()) as f32).collect();
            let m = measure_line_with_errors(&axis, &flux, err, None, LINE_RANGE, Some(windows), LineModel::None).unwrap();
            fluxes.push(m.flux);
            squared_errors += m.flux_err * m.flux_err;
            first.get_or_insert(m);
        }
        let mean = fluxes.iter().sum::<f64>() / fluxes.len() as f64;
        let variance = fluxes.iter().map(|f| (f - mean) * (f - mean)).sum::<f64>() / (fluxes.len() - 1) as f64;
        FluxMonteCarlo {
            scatter: variance.sqrt(),
            rms_flux_err: (squared_errors / fluxes.len() as f64).sqrt(),
            first: first.unwrap(),
        }
    }

    #[test]
    fn the_flux_error_matches_the_scatter_of_10000_realisations_within_3_percent_seed_20261016() {
        let constant = vec![ERR_SIGMA; LINE_CUBE_DEPTH];
        let ramp: Vec<f64> = (0..LINE_CUBE_DEPTH).map(|z| 0.02 + 0.06 * z as f64 / (LINE_CUBE_DEPTH - 1) as f64).collect();
        for (label, sigma) in [("constant", constant), ("ramp", ramp)] {
            let FluxMonteCarlo { scatter, first: m, rms_flux_err } = monte_carlo_fluxes(&sigma, Some(&sigma), WINDOWS);
            let scale = m.continuum_chi2_red.expect("reduced chi2 of the continuum fit").max(1.0).sqrt();
            let ratio = scatter * scale / m.flux_err;
            assert!((ratio - 1.0).abs() < MC_RELATIVE_TOLERANCE, "{label}: scatter={} flux_err={} ratio={}", scatter, m.flux_err, ratio);
            let t = terms(&m);
            assert!(scatter * scale > 1.5 * t.pixel, "{label}: the level term is needed, scatter={} pixel={}", scatter, t.pixel);
            let conservative = scatter / rms_flux_err;
            assert!(
                conservative > 0.90 && conservative < 0.95,
                "{label}: scaling by the reduced chi2 only when it exceeds one, scatter={} rms flux_err={} ratio={}",
                scatter,
                rms_flux_err,
                conservative
            );
        }
    }

    fn two_window_spectrum() -> (Vec<f64>, Vec<f32>) {
        let axis: Vec<f64> = (0..200).map(|z| 1.0 + z as f64 * 0.001).collect();
        let flux: Vec<f32> = (0..200)
            .map(|z| {
                let d = (z as f64 - 100.0) / 8.0;
                (1.0 + (-0.5 * d * d).exp()) as f32 + deterministic_noise(z, 0, 3, 0.05)
            })
            .collect();
        (axis, flux)
    }

    const WIDE_LINE: (usize, usize) = (60, 140);
    const WIDE_WINDOWS: ContinuumWindows = ((0, 50), (150, 199));

    #[test]
    fn channels_with_err_at_or_below_zero_are_excluded_and_counted_whatever_the_dq_mask_says() {
        let (axis, flux) = two_window_spectrum();
        let bad: Vec<usize> = (70..100).chain(160..188).collect();
        assert_eq!(bad.len(), BAD_ERR_CHANNELS);
        let mut err = vec![ERR_SIGMA; axis.len()];
        for &z in &bad {
            err[z] = 0.0;
        }
        err[bad[0]] = -0.5;
        err[bad[1]] = f64::NAN;
        err[bad[2]] = f64::INFINITY;
        let measure = |excluded: Option<&[bool]>| {
            measure_line_with_errors(&axis, &flux, Some(&err), excluded, WIDE_LINE, Some(WIDE_WINDOWS), LineModel::Gaussian)
                .unwrap()
        };
        let without_dq = measure(None);
        assert_eq!(without_dq.n_bad_err, BAD_ERR_CHANNELS);
        assert_eq!(without_dq.n_dq_excluded, 0);
        assert_eq!(without_dq.n_channels, 81 - 30);
        assert_eq!(without_dq.continuum_channels, 51 + 50 - 28);
        assert!(without_dq.notes.iter().any(|n| n.contains("58 with ERR <= 0 or not finite")), "{:?}", without_dq.notes);

        let mut flagged = vec![false; axis.len()];
        for &z in &bad {
            flagged[z] = true;
        }
        let with_dq = measure(Some(&flagged));
        assert_eq!(with_dq.n_bad_err, BAD_ERR_CHANNELS);
        assert_eq!(with_dq.n_dq_excluded, BAD_ERR_CHANNELS);
        assert_eq!(with_dq.flux.to_bits(), without_dq.flux.to_bits());
        assert_eq!(with_dq.flux_err.to_bits(), without_dq.flux_err.to_bits());

        let mut blanked = flux.clone();
        for &z in &bad {
            blanked[z] = f32::NAN;
        }
        let valid_err = vec![ERR_SIGMA; axis.len()];
        let reference =
            measure_line_with_errors(&axis, &blanked, Some(&valid_err), None, WIDE_LINE, Some(WIDE_WINDOWS), LineModel::Gaussian)
                .unwrap();
        assert_eq!(reference.n_nonfinite, BAD_ERR_CHANNELS);
        assert_eq!(without_dq.flux.to_bits(), reference.flux.to_bits());
        assert_eq!(without_dq.flux_err.to_bits(), reference.flux_err.to_bits());
        assert_eq!(without_dq.centroid.to_bits(), reference.centroid.to_bits());
        assert_eq!(without_dq.continuum_level.to_bits(), reference.continuum_level.to_bits());
    }

    #[test]
    fn dq_flagged_and_nan_channels_are_excluded_and_counted() {
        let (axis, flux) = centre_spectrum(0.05);
        let flagged_channels = [12, 25, 36];
        let nan_channels = [2, 18];
        let mut excluded = vec![false; axis.len()];
        for z in flagged_channels {
            excluded[z] = true;
        }
        let mut with_nan = flux.clone();
        for z in nan_channels {
            with_nan[z] = f32::NAN;
        }
        let err = vec![ERR_SIGMA; axis.len()];
        let m = measure_line_with_errors(&axis, &with_nan, Some(&err), Some(&excluded), LINE_RANGE, Some(WINDOWS), LineModel::None)
            .unwrap();
        assert_eq!((m.n_dq_excluded, m.n_nonfinite, m.n_bad_err), (3, 2, 0));
        assert_eq!(m.n_channels, 25 - 3);
        assert_eq!(m.continuum_channels, 12 - 2);
        assert!(m.notes.iter().any(|n| n.contains("3 DQ-flagged") && n.contains("2 with non-finite flux")), "{:?}", m.notes);

        let mut blanked = with_nan.clone();
        for z in flagged_channels {
            blanked[z] = f32::NAN;
        }
        let reference =
            measure_line_with_errors(&axis, &blanked, Some(&err), None, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        assert_eq!(m.flux.to_bits(), reference.flux.to_bits());
        assert_eq!(m.flux_err.to_bits(), reference.flux_err.to_bits());

        let unweighted =
            measure_line_with_errors(&axis, &with_nan, None, Some(&excluded), LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        assert!(!unweighted.err_used);
        assert!(close(unweighted.flux_err, terms(&unweighted).total(), 1e-15));
        assert_eq!((unweighted.n_dq_excluded, unweighted.n_nonfinite, unweighted.n_bad_err), (3, 2, 0));
        let unweighted_blanked = measure_line(&axis, &blanked, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        assert_eq!(unweighted.flux.to_bits(), unweighted_blanked.flux.to_bits());
        assert_eq!(unweighted.flux_err.to_bits(), unweighted_blanked.flux_err.to_bits());
    }

    #[test]
    fn gaussian_errors_are_absolute_sigma_inflated_only_when_the_reduced_chi2_exceeds_one() {
        let (axis, flux) = centre_spectrum(0.05);
        let fit_with = |sigma: f64| {
            let err = vec![sigma; axis.len()];
            let m = measure_line_with_errors(&axis, &flux, Some(&err), None, LINE_RANGE, Some(WINDOWS), LineModel::Gaussian)
                .unwrap();
            assert!(m.notes.iter().any(|n| n.contains("absolute-sigma")), "{:?}", m.notes);
            m.fit.expect("gaussian fit")
        };
        let overestimated = [fit_with(0.1), fit_with(0.2)];
        for fit in &overestimated {
            assert!(fit.converged && fit.chi2 / (fit.dof as f64) < 1.0, "{:?}", fit);
        }
        assert!(close(overestimated[1].amplitude, overestimated[0].amplitude, 1e-9));
        assert!(close(overestimated[1].amplitude_err, 2.0 * overestimated[0].amplitude_err, 1e-6), "{:?}", overestimated);
        assert!(close(overestimated[1].centre_err, 2.0 * overestimated[0].centre_err, 1e-6), "{:?}", overestimated);

        let underestimated = [fit_with(0.001), fit_with(0.002)];
        for fit in &underestimated {
            assert!(fit.converged && fit.chi2 / (fit.dof as f64) > 1.0, "{:?}", fit);
        }
        assert!(close(underestimated[1].amplitude_err, underestimated[0].amplitude_err, 1e-6), "{:?}", underestimated);
        assert!(close(underestimated[1].sigma_err, underestimated[0].sigma_err, 1e-6), "{:?}", underestimated);
    }

    #[test]
    fn err_and_dq_arrays_of_another_length_are_refused_with_a_sentence() {
        let (axis, flux) = centre_spectrum(0.0);
        let short_err = vec![ERR_SIGMA; axis.len() - 1];
        let err = measure_line_with_errors(&axis, &flux, Some(&short_err), None, LINE_RANGE, Some(WINDOWS), LineModel::None)
            .unwrap_err();
        assert!(err.contains("ERR has 39 values but the spectrum has 40 channels"), "{}", err);
        let short_mask = vec![false; axis.len() + 1];
        let err = measure_line_with_errors(&axis, &flux, None, Some(&short_mask), LINE_RANGE, Some(WINDOWS), LineModel::None)
            .unwrap_err();
        assert!(err.contains("DQ exclusion mask has 41 values but the spectrum has 40 channels"), "{}", err);
        let all_flagged = vec![true; axis.len()];
        let err = measure_line_with_errors(&axis, &flux, None, Some(&all_flagged), LINE_RANGE, Some(WINDOWS), LineModel::None)
            .unwrap_err();
        assert!(err.contains("no usable channels"), "{}", err);
    }

    const UNDERSTATED_ERR: f64 = 0.01;
    const TRUE_NOISE_OVER_ERR: f64 = 5.0;

    #[test]
    fn with_err_understated_five_times_the_flux_error_still_matches_the_scatter_of_10000_realisations_within_3_percent() {
        let err = vec![UNDERSTATED_ERR; LINE_CUBE_DEPTH];
        let noise = vec![UNDERSTATED_ERR * TRUE_NOISE_OVER_ERR; LINE_CUBE_DEPTH];
        let mc = monte_carlo_fluxes(&noise, Some(&err), WINDOWS);
        let ratio = mc.scatter / mc.rms_flux_err;
        assert!(
            (ratio - 1.0).abs() < MC_RELATIVE_TOLERANCE,
            "scatter={} rms flux_err={} ratio={}",
            mc.scatter,
            mc.rms_flux_err,
            ratio
        );
    }

    #[test]
    fn a_constant_err_below_the_continuum_scatter_gives_the_same_flux_error_as_the_scatter_path() {
        let (axis, flux) = centre_spectrum(0.05);
        let err = vec![UNDERSTATED_ERR; axis.len()];
        let weighted =
            measure_line_with_errors(&axis, &flux, Some(&err), None, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        let unweighted = measure_line(&axis, &flux, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        assert!(
            close(weighted.flux_err, unweighted.flux_err, 1e-9),
            "with ERR {} without {}",
            weighted.flux_err,
            unweighted.flux_err
        );
        assert_eq!(unweighted.continuum_chi2_red, None);
        let scale = weighted.continuum_chi2_red.expect("reduced chi2 of the weighted continuum fit").sqrt();
        assert!(scale > 2.5 && scale < 3.5, "scale={}", scale);
        let propagated_pixel = UNDERSTATED_ERR * root_sum_squared_widths(&axis, LINE_RANGE);
        assert!(close(terms(&weighted).pixel, scale * propagated_pixel, 1e-9), "{:?}", terms(&weighted));
        assert!(close(weighted.snr, weighted.flux / weighted.flux_err, 1e-15));
        assert!(weighted.notes.iter().any(|n| n.contains("each scaled by 3.0")), "{:?}", weighted.notes);
        assert!(weighted.notes.iter().any(|n| n.contains("ERR understates the scatter")), "{:?}", weighted.notes);

        let matching = vec![scale * UNDERSTATED_ERR; axis.len()];
        let consistent =
            measure_line_with_errors(&axis, &flux, Some(&matching), None, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        let chi2_red = consistent.continuum_chi2_red.expect("reduced chi2");
        assert!((chi2_red - 1.0).abs() < 1e-9, "chi2_red={}", chi2_red);
        assert!(consistent.notes.iter().all(|n| !n.contains("continuum residuals scatter")), "{:?}", consistent.notes);

        let generous = vec![1.0; axis.len()];
        let overstated =
            measure_line_with_errors(&axis, &flux, Some(&generous), None, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        assert!(overstated.continuum_chi2_red.expect("reduced chi2") < 0.01, "{:?}", overstated.continuum_chi2_red);
        assert!(close(terms(&overstated).pixel, root_sum_squared_widths(&axis, LINE_RANGE), 1e-9), "{:?}", terms(&overstated));
        assert!(overstated.notes.iter().all(|n| !n.contains("each scaled by")), "{:?}", overstated.notes);
        assert!(overstated.notes.iter().any(|n| n.contains("ERR overstates the scatter")), "{:?}", overstated.notes);
    }

    #[test]
    fn without_err_the_flux_error_matches_the_scatter_of_10000_realisations_within_3_percent() {
        let noise = vec![ERR_SIGMA; LINE_CUBE_DEPTH];
        for (label, windows) in [("first-order continuum", WINDOWS), ("constant continuum", ((0, 5), (0, 5)))] {
            let mc = monte_carlo_fluxes(&noise, None, windows);
            let ratio = mc.scatter / mc.rms_flux_err;
            assert!(
                (ratio - 1.0).abs() < MC_RELATIVE_TOLERANCE,
                "{label}: scatter={} rms flux_err={} ratio={}",
                mc.scatter,
                mc.rms_flux_err,
                ratio
            );
        }
    }

    #[test]
    fn without_err_the_flux_error_is_the_residual_rms_times_the_pixel_level_and_slope_geometry() {
        let (axis, flux) = centre_spectrum(0.05);
        let m = measure_line(&axis, &flux, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        let window_channels: Vec<usize> = (WINDOWS.0 .0..=WINDOWS.0 .1).chain(WINDOWS.1 .0..=WINDOWS.1 .1).collect();
        let continuum_at = |z: usize| m.continuum_level + m.continuum_slope * (axis[z] - m.continuum_reference);
        let residual_squares: f64 = window_channels.iter().map(|&z| (flux[z] as f64 - continuum_at(z)).powi(2)).sum();
        let rms = (residual_squares / (window_channels.len() - 2) as f64).sqrt();
        let n = window_channels.len() as f64;
        let x_mean = window_channels.iter().map(|&z| axis[z]).sum::<f64>() / n;
        let sxx: f64 = window_channels.iter().map(|&z| (axis[z] - x_mean).powi(2)).sum();
        let widths = channel_widths(&axis);
        let width: f64 = widths[LINE_RANGE.0..=LINE_RANGE.1].iter().sum();
        let lever: f64 = (LINE_RANGE.0..=LINE_RANGE.1).map(|z| widths[z] * (axis[z] - x_mean)).sum();
        let pixel = rms * root_sum_squared_widths(&axis, LINE_RANGE);
        let level = rms * width / n.sqrt();
        let slope = rms * lever.abs() / sxx.sqrt();
        let expected = (pixel * pixel + level * level + slope * slope).sqrt();
        assert!(close(m.flux_err, expected, 1e-9), "flux_err={} expected={}", m.flux_err, expected);
        assert!(close(m.snr, m.flux / expected, 1e-9), "snr={}", m.snr);
        let t = terms(&m);
        assert!(close(t.pixel, pixel, 1e-9) && close(t.level, level, 1e-9) && close(t.slope, slope, 1e-8), "{:?}", t);
        assert!(!m.err_used);

        let single_channel = measure_line(&axis, &flux, LINE_RANGE, Some(((3, 3), (3, 3))), LineModel::None).unwrap();
        assert!(single_channel.flux_err.is_nan() && single_channel.flux_err_terms.is_none(), "{:?}", single_channel);
        assert!(single_channel.notes.iter().any(|n| n.contains("continuum scatter is zero or unknown")), "{:?}", single_channel.notes);
    }

    const H_ALPHA_UM: f64 = 0.656461;
    const REDSHIFT: f64 = 0.5;
    const SOURCE_SIGMA_KMS: f64 = 100.0;
    const REDSHIFTED_LINE: (usize, usize) = (16, 64);
    const REDSHIFTED_WINDOWS: ContinuumWindows = ((0, 9), (71, 80));

    fn redshifted_h_alpha() -> (Vec<f64>, Vec<f32>) {
        let centre = H_ALPHA_UM * (1.0 + REDSHIFT);
        let sigma_um = centre * SOURCE_SIGMA_KMS / SPEED_OF_LIGHT_KMS;
        let axis: Vec<f64> = (0..81).map(|i| centre + (i as f64 - 40.0) * sigma_um / 4.0).collect();
        let flux = axis
            .iter()
            .map(|&x| {
                let d = (x - centre) / sigma_um;
                (1.0 + (-0.5 * d * d).exp()) as f32
            })
            .collect();
        (axis, flux)
    }

    #[test]
    fn a_redshifted_line_reports_its_source_frame_width_and_its_velocity_relative_to_the_redshift() {
        let (axis, flux) = redshifted_h_alpha();
        let measured = measure_line(&axis, &flux, REDSHIFTED_LINE, Some(REDSHIFTED_WINDOWS), LineModel::None).unwrap();
        let observer_frame = [
            (VelocityConvention::Optical, 149_896.229, 150.0),
            (VelocityConvention::Radio, 99_930.819, 66.644),
            (VelocityConvention::Relativistic, 115_304.792, 85.182),
        ];
        for (convention, observed_centroid_kms, observed_sigma_kms) in observer_frame {
            let mut m = measured.clone();
            with_velocity(&mut m, H_ALPHA_UM, Some(REDSHIFT), convention, AxisKind::Wave, None, None);
            let v = m.velocity.clone().expect("velocity");
            assert_eq!(v.redshift, REDSHIFT);
            assert!(close(v.source_sigma_kms, SOURCE_SIGMA_KMS, 1e-3), "{:?}: source_sigma_kms={}", convention, v.source_sigma_kms);
            assert!(close(v.source_fwhm_kms, SOURCE_SIGMA_KMS * FWHM_PER_SIGMA, 1e-3), "{:?}: {}", convention, v.source_fwhm_kms);
            assert!(v.source_centroid_kms.abs() < 0.5, "{:?}: source_centroid_kms={}", convention, v.source_centroid_kms);
            assert!(close(v.centroid_kms, observed_centroid_kms, 1e-6), "{:?}: centroid_kms={}", convention, v.centroid_kms);
            assert!(close(v.sigma_kms, observed_sigma_kms, 1e-4), "{:?}: sigma_kms={}", convention, v.sigma_kms);
            let note = m.notes.last().expect("redshift note");
            assert!(note.starts_with("redshift 0.5: source_centroid_kms is taken against 0.9846915 um"), "{}", note);
            assert!(note.contains("observer-frame values against the laboratory rest wavelength"), "{}", note);
        }

        let mut without_redshift = measured.clone();
        with_velocity(&mut without_redshift, H_ALPHA_UM, None, VelocityConvention::Optical, AxisKind::Wave, None, Some(12.5));
        let v = without_redshift.velocity.clone().expect("velocity");
        assert_eq!(v.redshift, 0.0);
        assert_eq!(v.source_centroid_kms.to_bits(), v.centroid_kms.to_bits());
        assert!(close(v.source_sigma_kms, SOURCE_SIGMA_KMS, 1e-3), "source_sigma_kms={}", v.source_sigma_kms);
        assert!(without_redshift.notes.last().expect("velocity note").starts_with("line velocity"), "{:?}", without_redshift.notes);

        let mut shifted = measured.clone();
        with_velocity(&mut shifted, H_ALPHA_UM, Some(REDSHIFT), VelocityConvention::Optical, AxisKind::Wave, None, Some(12.5));
        let v = shifted.velocity.expect("velocity");
        assert!((v.source_centroid_kms - 12.5).abs() < 0.5, "source_centroid_kms={}", v.source_centroid_kms);

        let mut on_frequency = measured.clone();
        on_frequency.centroid = SPEED_OF_LIGHT_KMS / measured.centroid;
        on_frequency.sigma = on_frequency.centroid * SOURCE_SIGMA_KMS / SPEED_OF_LIGHT_KMS;
        with_velocity(&mut on_frequency, H_ALPHA_UM, Some(REDSHIFT), VelocityConvention::Radio, AxisKind::Freq, None, None);
        let v = on_frequency.velocity.expect("velocity");
        assert!(close(v.source_sigma_kms, SOURCE_SIGMA_KMS, 1e-9), "source_sigma_kms={}", v.source_sigma_kms);
        assert!(v.source_centroid_kms.abs() < 0.5, "source_centroid_kms={}", v.source_centroid_kms);

        let mut refused = measured.clone();
        with_velocity(&mut refused, H_ALPHA_UM, Some(-1.0), VelocityConvention::Optical, AxisKind::Wave, None, None);
        assert!(refused.velocity.is_none());
        assert!(refused.notes.iter().any(|n| n.contains("redshift -1 is not a finite number above -1")), "{:?}", refused.notes);
    }

    #[test]
    fn a_negative_continuum_across_the_line_leaves_the_equivalent_width_undefined() {
        let axis = flat_axis();
        let (level, line_flux, sigma_um) = (-1e-5, 2.0e-3, 0.002);
        let amplitude = line_flux / (sigma_um * (2.0 * std::f64::consts::PI).sqrt());
        let flux: Vec<f32> = axis
            .iter()
            .map(|&x| (level + amplitude * (-(x - 1.02) * (x - 1.02) / (2.0 * sigma_um * sigma_um)).exp()) as f32)
            .collect();
        let m = measure_line(&axis, &flux, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        assert!(close(m.flux, line_flux, 0.01), "flux={}", m.flux);
        assert!(close(m.continuum_level, level, 0.01), "level={}", m.continuum_level);
        assert!(m.equivalent_width.is_nan(), "ew={}", m.equivalent_width);
        assert!(
            m.notes.iter().any(|n| n.contains("continuum is negative across the line: equivalent width undefined")),
            "{:?}",
            m.notes
        );
    }

    const NARROW_SIGMA_CHANNELS: f64 = 0.94;
    const NARROW_CORE_CHANNEL: usize = 20;
    const NARROW_CORE_SHARE: f64 = 0.405;

    fn narrow_line() -> (Vec<f64>, Vec<f32>) {
        let axis = flat_axis();
        let integral_to = |edge: f64| {
            0.5 * libm::erf((edge - NARROW_CORE_CHANNEL as f64) / (NARROW_SIGMA_CHANNELS * std::f64::consts::SQRT_2))
        };
        let flux = (0..axis.len()).map(|z| (1.0 + integral_to(z as f64 + 0.5) - integral_to(z as f64 - 0.5)) as f32).collect();
        (axis, flux)
    }

    #[test]
    fn a_channel_dropped_inside_the_line_is_reported_as_a_loss_inside_the_line_range() {
        let (axis, flux) = narrow_line();
        let complete = measure_line(&axis, &flux, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        let mut flagged = vec![false; axis.len()];
        flagged[NARROW_CORE_CHANNEL] = true;
        let m = measure_line_with_errors(&axis, &flux, None, Some(&flagged), LINE_RANGE, Some(WINDOWS), LineModel::None)
            .unwrap();
        let lost = 1.0 - m.flux / complete.flux;
        assert!((lost - NARROW_CORE_SHARE).abs() < 0.002, "lost={}", lost);
        assert_eq!(m.n_channels, 24);
        assert!(
            m.notes.iter().any(|n| n.contains("1 of the 25 channels in the line range 8..=32 were dropped (1 DQ-flagged")),
            "{:?}",
            m.notes
        );
        assert_eq!((m.n_channels_nominal, complete.n_channels_nominal), (25, 25));
        assert_eq!(complete.n_channels, 25);
        assert!(complete.notes.iter().all(|n| !n.contains("were dropped")), "{:?}", complete.notes);

        let mut window_flagged = vec![false; axis.len()];
        window_flagged[2] = true;
        let outside =
            measure_line_with_errors(&axis, &flux, None, Some(&window_flagged), LINE_RANGE, Some(WINDOWS), LineModel::None)
                .unwrap();
        assert_eq!((outside.n_channels, outside.n_channels_nominal, outside.n_dq_excluded), (25, 25, 1));
        assert!(outside.notes.iter().all(|n| !n.contains("were dropped")), "{:?}", outside.notes);
        assert_eq!(outside.flux.to_bits(), complete.flux.to_bits());
    }

    #[test]
    fn the_peak_of_an_absorption_line_is_its_deepest_channel() {
        let axis: Vec<f64> = (0..LINE_CUBE_DEPTH).map(|z| 1.0 + z as f64 * LINE_CDELT_UM).collect();
        let absorption: Vec<f32> = (0..LINE_CUBE_DEPTH).map(|z| LINE_CONTINUUM - 0.5 * line_profile(z)).collect();
        let m = measure_line(&axis, &absorption, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        assert!(m.flux < 0.0, "flux={}", m.flux);
        assert_eq!(m.peak_channel, 20, "peak {} at channel {}", m.peak, m.peak_channel);
        assert!((m.peak + 0.5).abs() < 1e-5, "peak={}", m.peak);
        let emission: Vec<f32> = (0..LINE_CUBE_DEPTH).map(|z| LINE_CONTINUUM + 0.5 * line_profile(z)).collect();
        let e = measure_line(&axis, &emission, LINE_RANGE, Some(WINDOWS), LineModel::None).unwrap();
        assert_eq!(e.peak_channel, 20, "peak {} at channel {}", e.peak, e.peak_channel);
        assert!((e.peak - 0.5).abs() < 1e-5, "peak={}", e.peak);
    }
}
