use serde::Serialize;

use crate::core::astrometry::spectral::{velocity_kms, wavelength_um_from_frequency_ghz, AxisKind, VelocityConvention};
use crate::core::cube::moments::channel_widths;
use crate::math::gauss_newton::fit_least_squares;
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
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LineMeasurement {
    pub z0: usize,
    pub z1: usize,
    pub n_channels: usize,
    pub axis_unit: String,
    pub flux_unit: String,
    pub continuum_level: f64,
    pub continuum_slope: f64,
    pub continuum_sigma: f64,
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
    pub notes: Vec<String>,
    pub elapsed_ms: u64,
}

struct ContinuumFit {
    intercept: f64,
    slope: f64,
    sigma: f64,
    channels: usize,
    linear: bool,
}

fn fit_linear(xs: &[f64], ys: &[f64], constant_only: bool) -> (f64, f64) {
    let n = xs.len() as f64;
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();
    let denominator = n * sxx - sx * sx;
    if constant_only || xs.len() < 3 || denominator.abs() <= SLOPE_CONDITION_EPSILON * n * sxx.max(1.0) {
        return (sy / n, 0.0);
    }
    let slope = (n * sxy - sx * sy) / denominator;
    ((sy - slope * sx) / n, slope)
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

fn fit_continuum(axis: &[f64], flux: &[f32], windows: ContinuumWindows, x_ref: f64) -> Result<ContinuumFit, String> {
    let mut channels: Vec<usize> = (windows.0 .0..=windows.0 .1).chain(windows.1 .0..=windows.1 .1).collect();
    channels.sort_unstable();
    channels.dedup();
    let samples: Vec<(f64, f64)> = channels
        .iter()
        .filter(|&&z| flux[z].is_finite())
        .map(|&z| (axis[z] - x_ref, flux[z] as f64))
        .collect();
    if samples.is_empty() {
        return Err("the continuum windows contain no finite flux values".to_string());
    }
    let xs: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let ys: Vec<f64> = samples.iter().map(|s| s.1).collect();
    let linear = windows.0 != windows.1 && samples.len() >= 3;
    let (intercept, slope) = fit_linear(&xs, &ys, !linear);
    let mut residuals: Vec<f32> = xs.iter().zip(&ys).map(|(x, y)| (y - (intercept + slope * x)) as f32).collect();
    Ok(ContinuumFit { intercept, slope, sigma: robust_sigma(&mut residuals), channels: samples.len(), linear })
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

fn resolve_windows(
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

fn gaussian_fit(
    samples: &[LineSample],
    continuum: &ContinuumFit,
    x_ref: f64,
    x_bounds: (f64, f64),
    start: [f64; 3],
    emission: bool,
    notes: &mut Vec<String>,
) -> Option<GaussianFit> {
    let xs: Vec<f64> = samples.iter().map(|s| s.x).collect();
    let ys: Vec<f64> = samples.iter().map(|s| s.excess + s.continuum).collect();
    let weight = if continuum.sigma.is_finite() && continuum.sigma > 0.0 {
        1.0 / (continuum.sigma * continuum.sigma)
    } else {
        notes.push("continuum scatter is zero or unknown: the Gaussian fit used unit weights".to_string());
        1.0
    };
    let weights = vec![weight; xs.len()];
    let mean_dx = samples.iter().map(|s| s.dx).sum::<f64>() / samples.len() as f64;
    let span = x_bounds.1 - x_bounds.0;
    let sigma_bounds = (SIGMA_LOWER_BOUND_FRACTION * mean_dx, span.max(mean_dx));
    let amplitude_lower = if emission { 0.0 } else { f64::NEG_INFINITY };
    let lower = [amplitude_lower, x_bounds.0, sigma_bounds.0];
    let upper = [f64::INFINITY, x_bounds.1, sigma_bounds.1];
    let (level, slope) = (continuum.intercept, continuum.slope);
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
            Some(GaussianFit {
                amplitude: fit.params[0],
                centre: fit.params[1],
                sigma: fit.params[2],
                amplitude_err: fit.errors[0],
                centre_err: fit.errors[1],
                sigma_err: fit.errors[2],
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
    validate_axis(axis, flux, line)?;
    let depth = axis.len();
    let mut notes = Vec::new();
    let windows = resolve_windows(continuum, line, depth, &mut notes)?;
    let x_ref = axis[(line.0 + line.1) / 2];
    let cont = fit_continuum(axis, flux, windows, x_ref)?;
    let widths = channel_widths(axis);

    let samples: Vec<LineSample> = (line.0..=line.1)
        .filter(|&z| flux[z].is_finite())
        .map(|z| {
            let continuum = cont.intercept + cont.slope * (axis[z] - x_ref);
            LineSample { channel: z, x: axis[z], excess: flux[z] as f64 - continuum, continuum, dx: widths[z] }
        })
        .collect();
    let n_requested = line.1 - line.0 + 1;
    if samples.is_empty() {
        return Err(format!("line range {}..={} contains no finite flux values", line.0, line.1));
    }
    if samples.len() < n_requested {
        notes.push(format!("{} channels with non-finite flux were skipped", n_requested - samples.len()));
    }
    let n_channels = samples.len();

    let flux_total: f64 = samples.iter().map(|s| s.excess * s.dx).sum();
    let mean_dx = samples.iter().map(|s| s.dx).sum::<f64>() / n_channels as f64;
    let continuum_crosses_zero = samples.iter().any(|s| s.continuum == 0.0);
    let equivalent_width = if continuum_crosses_zero {
        notes.push("continuum is zero inside the line: equivalent width undefined".to_string());
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
        .map(|s| (s.x, s.excess))
        .unwrap_or((axis[line.0], f64::NAN));

    let emission = flux_total > 0.0;
    let (centroid, sigma) = if emission {
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

    let flux_err = cont.sigma * (n_channels as f64).sqrt() * mean_dx;
    let snr = flux_total / flux_err;
    if !(cont.sigma.is_finite() && cont.sigma > 0.0) {
        notes.push("continuum scatter is zero or unknown: flux error and SNR are not meaningful".to_string());
    }

    let x_bounds = (axis[line.0].min(axis[line.1]), axis[line.0].max(axis[line.1]));
    let fit = match model {
        LineModel::None => None,
        LineModel::Gaussian => {
            let start = if emission {
                [peak_sample.1, centroid, if sigma.is_finite() { sigma } else { mean_dx }]
            } else {
                [trough_sample.1, trough_sample.0, (x_bounds.1 - x_bounds.0) / 4.0]
            };
            gaussian_fit(&samples, &cont, x_ref, x_bounds, start, emission, &mut notes)
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
        axis_unit: String::new(),
        flux_unit: String::new(),
        continuum_level: cont.intercept + cont.slope * (centroid - x_ref),
        continuum_slope: cont.slope,
        continuum_sigma: cont.sigma,
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
        peak: peak_sample.1,
        peak_channel: peak_sample.0,
        snr,
        velocity: None,
        fit,
        notes,
        elapsed_ms: 0,
    })
}

pub fn with_velocity(
    m: &mut LineMeasurement,
    rest_um: f64,
    convention: VelocityConvention,
    axis_kind: AxisKind,
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
    let shift = shift_kms.filter(|s| s.is_finite()).unwrap_or(0.0);
    let centroid_kms = velocity_kms(to_um(m.centroid), rest_um, convention);
    let sigma_kms = if m.sigma.is_finite() {
        (velocity_kms(to_um(m.centroid + m.sigma), rest_um, convention) - centroid_kms).abs()
    } else {
        f64::NAN
    };
    m.notes.push(format!(
        "line velocity from the {} convention with rest wavelength {} um{}",
        convention.name(),
        rest_um,
        if shift != 0.0 { format!(", shifted by {:.3} km/s (frame correction)", shift) } else { ", topocentric".to_string() }
    ));
    m.velocity = Some(LineVelocity {
        centroid_kms: centroid_kms + shift,
        sigma_kms,
        fwhm_kms: sigma_kms * FWHM_PER_SIGMA,
        rest_um,
        convention,
        shift_applied_kms: shift,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
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
        with_velocity(&mut m, LINE_REST_UM, VelocityConvention::Optical, AxisKind::Wave, None);
        let v = m.velocity.clone().expect("velocity");
        assert!(v.centroid_kms.abs() < 0.1, "v={}", v.centroid_kms);
        let expected_sigma_kms = 299792.458 * 0.003 / LINE_REST_UM;
        assert!(close(v.sigma_kms, expected_sigma_kms, 0.03), "sigma_kms={}", v.sigma_kms);
        assert!(close(v.fwhm_kms, v.sigma_kms * FWHM_PER_SIGMA, 1e-12));
        assert_eq!(v.shift_applied_kms, 0.0);

        with_velocity(&mut m, LINE_REST_UM, VelocityConvention::Radio, AxisKind::Wave, Some(12.5));
        let shifted = m.velocity.clone().expect("velocity");
        assert!((shifted.centroid_kms - 12.5).abs() < 0.1, "v={}", shifted.centroid_kms);
        assert_eq!(shifted.shift_applied_kms, 12.5);
        assert_eq!(shifted.convention, VelocityConvention::Radio);

        let freq_axis: Vec<f64> = axis.iter().map(|w| 299792.458 / w).collect();
        let mut on_freq = m.clone();
        on_freq.centroid = 299792.458 / LINE_REST_UM;
        on_freq.sigma = f64::NAN;
        with_velocity(&mut on_freq, LINE_REST_UM, VelocityConvention::Optical, AxisKind::Freq, None);
        let on_freq_velocity = on_freq.velocity.expect("velocity");
        assert!(on_freq_velocity.centroid_kms.abs() < 1e-6);
        assert!(on_freq_velocity.sigma_kms.is_nan());
        assert!(freq_axis[0] > freq_axis[1]);

        let mut on_velocity_axis = m.clone();
        on_velocity_axis.velocity = None;
        with_velocity(&mut on_velocity_axis, LINE_REST_UM, VelocityConvention::Optical, AxisKind::Vrad, None);
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
    fn coinciding_windows_give_a_constant_continuum() {
        let (axis, flux) = centre_spectrum(0.0);
        let m = measure_line(&axis, &flux, LINE_RANGE, Some(((0, 5), (0, 5))), LineModel::None).unwrap();
        assert!(!m.continuum_linear);
        assert_eq!(m.continuum_slope, 0.0);
        assert_eq!(m.continuum_channels, 6);
        assert!(close(m.flux, EXPECTED_FLUX, 0.01), "flux={}", m.flux);
    }
}
