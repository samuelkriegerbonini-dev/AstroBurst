use anyhow::{bail, Result};
use ndarray::Array2;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::astrometry::spectral::{
    air_formula_applies, air_to_vacuum_um, velocity_kms, wavelength_um_from_frequency_ghz, AxisKind, SpectralAxis,
    VelocityConvention, AIR_FORMULA_MIN_UM, AIR_VACUUM_FORMULA, SPEED_OF_LIGHT_KMS,
};
use crate::core::cube::lazy::{median_band_rows, LazyCube};
use crate::core::cube::moments::{channel_widths, ContinuumWindows, DEFAULT_SNR_THRESHOLD, VELOCITY_UNIT};
use crate::math::gauss_newton::{fit_least_squares, FitResult};
use crate::math::{exact_mad_mut, exact_median_mut};
use crate::types::constants::MAD_TO_SIGMA;

pub const MIN_FIT_CHANNELS: usize = 5;
pub const MAX_FIT_CHANNELS: usize = 400;
pub const SLOW_FIT_CHANNELS: usize = 200;
pub const MAX_FIT_ITERATIONS: usize = 50;
pub const FWHM_PER_SIGMA: f64 = 2.354_820_045_030_949;
pub const BIC_MARGIN: f64 = 10.0;
pub const COMPONENT_SNR_MIN: f64 = 3.0;
pub const SEPARATION_MIN_CHANNELS: f64 = 1.0;
pub const DQ_EXACT_LIMIT: f32 = 16_777_216.0;
pub const REST_REQUIRED_MESSAGE: &str = "rest wavelength required for line-fit velocities";
pub const SIGMA_OBSERVED_LABEL: &str = "observed (instrumental width not removed)";
pub const MULTI_PEAK_CAVEAT: &str =
    "single Gaussian: multi-peaked profiles return a blended velocity and inflated sigma; inspect chi2_red";

pub const MASK_FITTED: u32 = 1;
pub const MASK_CONST_CONTINUUM: u32 = 2;
pub const MASK_DQ_DROPPED: u32 = 4;
pub const MASK_ERR_DROPPED: u32 = 8;
pub const MASK_UNRESOLVED: u32 = 16;
pub const MASK_NOT_CONVERGED: u32 = 32;
pub const MASK_TWO_REJECTED: u32 = 64;
pub const MASK_TWO_COMPONENTS: u32 = 128;

const BAND_BYTES: usize = 256 << 20;
const MIN_LINE_SAMPLES: usize = 4;
const MIN_WINDOW_SAMPLES: usize = 3;
const SIGMA_LOWER_BOUND_FRACTION: f64 = 0.25;
const SLOPE_CONDITION_EPSILON: f64 = 1e-12;
const PIVOT_EPSILON: f64 = 1e-300;
const MODEL_SUBSAMPLES: usize = 4;
const CONTINUUM_PARAMETERS: usize = 2;
const SINGLE_PARAMETERS: usize = 3;
const PAIR_PARAMETERS: usize = 6;
const MIN_PAIR_SAMPLES: usize = 8;
const MAX_PAIR_ITERATIONS: usize = 200;
const SPLIT_START_FRACTION: f64 = 0.7;
const SIGMA_LOWER_MARGIN: f64 = 1.01;
const COMPONENT_WIDTH_FRACTION: f64 = 0.25;
const COMPONENT_FLUX_FRACTION_MIN: f64 = 0.1;
const SECOND_AMPLITUDE_FRACTION: f64 = 0.1;

fn sqrt_two_pi() -> f64 {
    (2.0 * std::f64::consts::PI).sqrt()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Components {
    One,
    Two,
    Auto,
}

impl Components {
    pub fn parse(name: &str) -> std::result::Result<Components, String> {
        match name.trim().to_lowercase().as_str() {
            "one" | "" => Ok(Components::One),
            "two" => Ok(Components::Two),
            "auto" => Ok(Components::Auto),
            other => Err(format!("unknown components mode '{}': use one, two or auto", other)),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Components::One => "one",
            Components::Two => "two",
            Components::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Weighting {
    Err,
    Continuum,
}

impl Weighting {
    pub fn name(self) -> &'static str {
        match self {
            Weighting::Err => "err",
            Weighting::Continuum => "continuum",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct LineFitConfigWire {
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
    emission_only: Option<bool>,
    #[serde(default)]
    use_err: Option<bool>,
    #[serde(default)]
    use_dq: Option<bool>,
    #[serde(default)]
    resolving_power: Option<f64>,
    #[serde(default)]
    components: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "LineFitConfigWire")]
pub struct LineFitConfig {
    pub z0: usize,
    pub z1: usize,
    pub rest_um: Option<f64>,
    pub convention: VelocityConvention,
    pub continuum: Option<ContinuumWindows>,
    pub snr_threshold: f64,
    pub emission_only: bool,
    pub use_err: bool,
    pub use_dq: bool,
    pub resolving_power: Option<f64>,
    pub components: Components,
}

impl TryFrom<LineFitConfigWire> for LineFitConfig {
    type Error = String;

    fn try_from(wire: LineFitConfigWire) -> std::result::Result<Self, Self::Error> {
        let convention = match wire.convention.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(name) => VelocityConvention::parse(name)?,
            None => VelocityConvention::Optical,
        };
        let snr_threshold = wire.snr_threshold.unwrap_or(DEFAULT_SNR_THRESHOLD);
        if !snr_threshold.is_finite() || snr_threshold < 0.0 {
            return Err(format!("snr_threshold must be a non-negative number, got {}", snr_threshold));
        }
        if let Some(r) = wire.resolving_power {
            if !(r.is_finite() && r > 0.0) {
                return Err(format!("resolving_power must be a positive number, got {}", r));
            }
        }
        let components = match wire.components.as_deref() {
            Some(name) => Components::parse(name)?,
            None => Components::One,
        };
        Ok(LineFitConfig {
            z0: wire.z0,
            z1: wire.z1,
            rest_um: wire.rest_um,
            convention,
            continuum: wire.continuum,
            snr_threshold,
            emission_only: wire.emission_only.unwrap_or(true),
            use_err: wire.use_err.unwrap_or(true),
            use_dq: wire.use_dq.unwrap_or(true),
            resolving_power: wire.resolving_power,
            components,
        })
    }
}

pub struct LineFitInputs<'a> {
    pub sci: &'a LazyCube,
    pub err: Option<&'a LazyCube>,
    pub dq: Option<&'a LazyCube>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ComponentFit {
    pub amplitude: f64,
    pub centre: f64,
    pub sigma: f64,
    pub amplitude_err: f64,
    pub centre_err: f64,
    pub sigma_err: f64,
    pub velocity_kms: f64,
    pub v_err_kms: f64,
    pub sigma_kms: f64,
    pub sigma_err_kms: f64,
    pub sigma_corr_kms: Option<f64>,
    pub flux: f64,
    pub flux_err: f64,
    pub snr: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContinuumParams {
    pub intercept: f64,
    pub slope: f64,
    pub x_ref: f64,
    pub sigma: f64,
    pub linear: bool,
    pub channels: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpaxelModel {
    pub channel: Vec<f64>,
    pub continuum: Vec<f64>,
    pub total: Vec<f64>,
    pub components: Vec<Vec<f64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpaxelFit {
    pub x: usize,
    pub y: usize,
    pub z0: usize,
    pub z1: usize,
    pub continuum_windows: ContinuumWindows,
    pub span: (usize, usize),
    pub axis: Vec<f64>,
    pub axis_unit: String,
    pub flux: Vec<f32>,
    pub err: Option<Vec<f32>>,
    pub channels: Vec<usize>,
    pub dropped_dq: Vec<usize>,
    pub dropped_err: Vec<usize>,
    pub continuum: Option<ContinuumParams>,
    pub weighting: Weighting,
    pub single: Option<ComponentFit>,
    pub chi2: f64,
    pub dof: usize,
    pub chi2_red: f64,
    pub converged: bool,
    pub iterations: usize,
    pub components: Vec<ComponentFit>,
    pub ncomp: u8,
    pub delta_bic: Option<f64>,
    pub mask: u32,
    pub model: Option<SpaxelModel>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ComponentPlanes {
    pub flux: Array2<f32>,
    pub velocity: Array2<f32>,
    pub sigma_obs: Array2<f32>,
    pub flux_err: Array2<f32>,
    pub v_err: Array2<f32>,
    pub sigma_err: Array2<f32>,
}

#[derive(Debug, Clone)]
pub struct LineFitMaps {
    pub flux: Array2<f32>,
    pub velocity: Array2<f32>,
    pub sigma_obs: Array2<f32>,
    pub sigma_corr: Option<Array2<f32>>,
    pub flux_err: Array2<f32>,
    pub v_err: Array2<f32>,
    pub sigma_err: Array2<f32>,
    pub chi2_red: Array2<f32>,
    pub snr: Array2<f32>,
    pub mask: Array2<f32>,
    pub ncomp: Array2<f32>,
    pub components: Option<[ComponentPlanes; 2]>,
    pub continuum_windows: ContinuumWindows,
    pub flux_unit: String,
    pub velocity_unit: &'static str,
    pub sigma_label: String,
    pub weighting: Weighting,
    pub err_hdu: Option<usize>,
    pub dq_hdu: Option<usize>,
    pub rest_um: Option<f64>,
    pub n_channels: usize,
    pub n_fit: usize,
    pub n_masked: usize,
    pub n_const_continuum: usize,
    pub n_two_components: usize,
    pub median_chi2_red: Option<f64>,
    pub notes: Vec<String>,
}

pub fn dq_flagged(value: f32) -> bool {
    !value.is_finite() || value < 0.0 || value >= DQ_EXACT_LIMIT || ((value as u32) & 1) != 0
}

pub fn error_scale(weighting: Weighting, chi2: f64, dof: usize) -> f64 {
    if dof == 0 {
        return 1.0;
    }
    let reduced = chi2 / dof as f64;
    match weighting {
        Weighting::Err => reduced.max(1.0),
        Weighting::Continuum => reduced,
    }
}

pub fn plane_names(cfg: &LineFitConfig, maps: &LineFitMaps) -> Vec<&'static str> {
    let mut names = vec!["flux", "velocity", "sigma_obs"];
    if maps.sigma_corr.is_some() {
        names.push("sigma_corr");
    }
    names.extend(["flux_err", "v_err", "sigma_err", "chi2_red", "snr", "mask", "ncomp"]);
    if cfg.components != Components::One && maps.components.is_some() {
        names.extend([
            "c1_flux",
            "c1_velocity",
            "c1_sigma_obs",
            "c1_flux_err",
            "c1_v_err",
            "c1_sigma_err",
            "c2_flux",
            "c2_velocity",
            "c2_sigma_obs",
            "c2_flux_err",
            "c2_v_err",
            "c2_sigma_err",
        ]);
    }
    names
}

fn bunit(cube: &LazyCube) -> Option<String> {
    cube.header
        .get("BUNIT")
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

fn robust_sigma(residuals: &mut Vec<f32>) -> f64 {
    if residuals.len() < 2 {
        return f64::NAN;
    }
    let median = exact_median_mut(residuals) as f32;
    exact_mad_mut(residuals, median) as f64 * MAD_TO_SIGMA
}

fn measured_axis_values(axis: &SpectralAxis, notes: &mut Vec<String>) -> Vec<f64> {
    if axis.kind != AxisKind::Awav {
        return axis.values.clone();
    }
    if axis.values.iter().any(|&w| w.is_finite() && !air_formula_applies(w)) {
        notes.push(format!(
            "some air wavelengths are below {} um where {} does not apply: left unchanged",
            AIR_FORMULA_MIN_UM, AIR_VACUUM_FORMULA
        ));
    }
    notes.push(format!("air wavelengths converted to vacuum with {}", AIR_VACUUM_FORMULA));
    axis.values.iter().map(|&w| air_to_vacuum_um(w)).collect()
}

fn check_window(window: (usize, usize), depth: usize) -> Result<()> {
    if window.0 > window.1 || window.1 >= depth {
        bail!("continuum window {}..={} is invalid for a cube with {} channels", window.0, window.1, depth);
    }
    Ok(())
}

fn windows_overlap(a: (usize, usize), b: (usize, usize)) -> bool {
    a != b && a.0 <= b.1 && b.0 <= a.1
}

fn automatic_windows(line: (usize, usize), depth: usize, notes: &mut Vec<String>) -> Result<ContinuumWindows> {
    let width = line.1 - line.0 + 1;
    let left = (line.0 > 0).then(|| (line.0.saturating_sub(width), line.0 - 1));
    let right = (line.1 + 1 < depth).then(|| (line.1 + 1, (line.1 + width).min(depth - 1)));
    let windows = match (left, right) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, a),
        (None, Some(b)) => (b, b),
        (None, None) => bail!(
            "no continuum channels outside the line range {}..={} on an axis with {} channels",
            line.0,
            line.1,
            depth
        ),
    };
    notes.push(format!(
        "no continuum windows given: channels {}..={} and {}..={} outside the line were used",
        windows.0 .0, windows.0 .1, windows.1 .0, windows.1 .1
    ));
    Ok(windows)
}

fn resolve_windows(cfg: &LineFitConfig, depth: usize, notes: &mut Vec<String>) -> Result<ContinuumWindows> {
    let Some(windows) = cfg.continuum else {
        return automatic_windows((cfg.z0, cfg.z1), depth, notes);
    };
    check_window(windows.0, depth)?;
    check_window(windows.1, depth)?;
    if windows_overlap(windows.0, windows.1) {
        bail!(
            "continuum windows {}..={} and {}..={} overlap",
            windows.0 .0,
            windows.0 .1,
            windows.1 .0,
            windows.1 .1
        );
    }
    Ok(windows)
}

fn frame_note(axis: &SpectralAxis) -> String {
    match axis.specsys.as_deref() {
        Some(frame) => format!(
            "velocities are in the axis frame {}: no barycentric or heliocentric correction is applied",
            frame
        ),
        None => "velocities are in the axis frame, which the header does not state (no SPECSYS): no barycentric or heliocentric correction is applied".to_string(),
    }
}

struct SpaxelContext {
    axis: Vec<f64>,
    axis_unit: String,
    widths: Vec<f64>,
    channels: Vec<usize>,
    line: Vec<usize>,
    window_a: Vec<usize>,
    window_b: Vec<usize>,
    windows: ContinuumWindows,
    x_ref: f64,
    x_bounds: (f64, f64),
    axis_kind: AxisKind,
    rest_um: Option<f64>,
    convention: VelocityConvention,
    sigma_inst: Option<f64>,
    weighting: Weighting,
    emission_only: bool,
    snr_threshold: f64,
    components: Components,
}

impl SpaxelContext {
    fn to_velocity(&self, x: f64) -> f64 {
        let rest = self.rest_um.unwrap_or(1.0);
        match self.axis_kind {
            AxisKind::Wave | AxisKind::Awav => velocity_kms(x, rest, self.convention),
            AxisKind::Freq => velocity_kms(wavelength_um_from_frequency_ghz(x), rest, self.convention),
            AxisKind::Zopt => velocity_kms(1.0 + x, 1.0, self.convention),
            _ => x,
        }
    }

    fn dv_dx_at(&self, x: f64, dx: f64) -> f64 {
        (self.to_velocity(x + dx) - self.to_velocity(x - dx)).abs() / (2.0 * dx)
    }
}

fn positions_of(channels: &[usize], range: (usize, usize)) -> Vec<usize> {
    channels.iter().enumerate().filter(|(_, &z)| z >= range.0 && z <= range.1).map(|(k, _)| k).collect()
}

fn build_context(cube: &LazyCube, cfg: &LineFitConfig, weighting: Weighting) -> Result<(SpaxelContext, Vec<String>)> {
    let depth = cube.geometry.naxis3;
    let axis = cube.spectral_axis().map_err(anyhow::Error::msg)?;
    if !axis.kind.is_spectral() {
        bail!(
            "CTYPE3 '{}' is not a spectral axis: the line fit needs a wavelength, frequency or velocity axis",
            axis.ctype
        );
    }
    cube.check_channel_range(cfg.z0, cfg.z1)?;
    let n_channels = cfg.z1 - cfg.z0 + 1;
    if n_channels < MIN_FIT_CHANNELS {
        bail!(
            "line-fit window {}..={} has {} channels: at least {} are needed",
            cfg.z0,
            cfg.z1,
            n_channels,
            MIN_FIT_CHANNELS
        );
    }
    if n_channels > MAX_FIT_CHANNELS {
        bail!(
            "line-fit window {}..={} has {} channels: the limit is {} (brush a narrower line)",
            cfg.z0,
            cfg.z1,
            n_channels,
            MAX_FIT_CHANNELS
        );
    }
    let mut notes = Vec::new();
    if n_channels > SLOW_FIT_CHANNELS {
        notes.push(format!("line window has {} channels: expect a slower fit", n_channels));
    }
    let values = measured_axis_values(&axis, &mut notes);
    if values.len() != depth {
        bail!("spectral axis has {} values for a cube with {} channels", values.len(), depth);
    }
    if values.iter().any(|v| !v.is_finite()) {
        bail!("spectral axis contains non-finite values");
    }
    let increasing = values.windows(2).all(|w| w[1] > w[0]);
    let decreasing = values.windows(2).all(|w| w[1] < w[0]);
    if !(increasing || decreasing) {
        bail!("spectral axis is not monotonic: the line fit needs strictly increasing or decreasing values");
    }
    let rest_um = if axis.kind.is_velocity() {
        notes.push(format!(
            "velocity axis {} used as stored in {}: rest wavelength and convention ignored",
            axis.ctype, axis.unit
        ));
        None
    } else if axis.kind == AxisKind::Zopt {
        notes.push(format!(
            "redshift axis {}: velocities from (1 + z) with the {} convention; rest wavelength ignored",
            axis.ctype,
            cfg.convention.name()
        ));
        None
    } else {
        match cfg.rest_um.filter(|r| r.is_finite() && *r > 0.0).or(axis.rest_wavelength_um) {
            Some(rest) => {
                notes.push(format!(
                    "velocities from the {} convention with rest wavelength {} um",
                    cfg.convention.name(),
                    rest
                ));
                Some(rest)
            }
            None => bail!("{}", REST_REQUIRED_MESSAGE),
        }
    };
    notes.push(frame_note(&axis));
    let windows = resolve_windows(cfg, depth, &mut notes)?;
    let mut channels: Vec<usize> = (cfg.z0..=cfg.z1)
        .chain(windows.0 .0..=windows.0 .1)
        .chain(windows.1 .0..=windows.1 .1)
        .collect();
    channels.sort_unstable();
    channels.dedup();
    let line = positions_of(&channels, (cfg.z0, cfg.z1));
    let window_a = positions_of(&channels, windows.0);
    let window_b = positions_of(&channels, windows.1);
    let widths = channel_widths(&values);
    let x_ref = values[(cfg.z0 + cfg.z1) / 2];
    let x_bounds = (values[cfg.z0].min(values[cfg.z1]), values[cfg.z0].max(values[cfg.z1]));
    let sigma_inst = cfg.resolving_power.map(|r| SPEED_OF_LIGHT_KMS / (r * FWHM_PER_SIGMA));
    let ctx = SpaxelContext {
        axis: values,
        axis_unit: axis.unit.clone(),
        widths,
        channels,
        line,
        window_a,
        window_b,
        windows,
        x_ref,
        x_bounds,
        axis_kind: axis.kind,
        rest_um,
        convention: cfg.convention,
        sigma_inst,
        weighting,
        emission_only: cfg.emission_only,
        snr_threshold: cfg.snr_threshold,
        components: cfg.components,
    };
    Ok((ctx, notes))
}

struct ContinuumFit {
    intercept: f64,
    slope: f64,
    cov: [[f64; 2]; 2],
    scatter: f64,
    linear: bool,
    channels: usize,
}

fn fit_continuum(dx: &[f64], ys: &[f64], weights: &[f64], linear: bool) -> ContinuumFit {
    let mut s0 = 0.0;
    let mut s1 = 0.0;
    let mut s2 = 0.0;
    let mut t0 = 0.0;
    let mut t1 = 0.0;
    for ((&x, &y), &w) in dx.iter().zip(ys).zip(weights) {
        s0 += w;
        s1 += w * x;
        s2 += w * x * x;
        t0 += w * y;
        t1 += w * x * y;
    }
    let det = s0 * s2 - s1 * s1;
    let (intercept, slope, cov, linear) = if linear && det.abs() > SLOPE_CONDITION_EPSILON * s0 * s2 {
        let slope = (s0 * t1 - s1 * t0) / det;
        let intercept = (t0 - slope * s1) / s0;
        (intercept, slope, [[s2 / det, -s1 / det], [-s1 / det, s0 / det]], true)
    } else {
        (t0 / s0, 0.0, [[1.0 / s0, 0.0], [0.0, 0.0]], false)
    };
    let mut residuals: Vec<f32> = dx.iter().zip(ys).map(|(x, y)| (y - (intercept + slope * x)) as f32).collect();
    ContinuumFit { intercept, slope, cov, scatter: robust_sigma(&mut residuals), linear, channels: ys.len() }
}

fn gaussian_model(level: f64, slope: f64, x_ref: f64) -> impl Fn(f64, &[f64], &mut [f64]) -> f64 {
    move |x: f64, p: &[f64], jac: &mut [f64]| {
        let d = x - p[1];
        let s2 = p[2] * p[2];
        let e = (-d * d / (2.0 * s2)).exp();
        jac[0] = e;
        jac[1] = p[0] * e * d / s2;
        jac[2] = p[0] * e * d * d / (s2 * p[2]);
        level + slope * (x - x_ref) + p[0] * e
    }
}

fn two_gaussian_model(level: f64, slope: f64, x_ref: f64) -> impl Fn(f64, &[f64], &mut [f64]) -> f64 {
    move |x: f64, p: &[f64], jac: &mut [f64]| {
        let mut total = level + slope * (x - x_ref);
        for c in 0..2 {
            let (a, xc, s) = (p[3 * c], p[3 * c + 1], p[3 * c + 2]);
            let d = x - xc;
            let s2 = s * s;
            let e = (-d * d / (2.0 * s2)).exp();
            jac[3 * c] = e;
            jac[3 * c + 1] = a * e * d / s2;
            jac[3 * c + 2] = a * e * d * d / (s2 * s);
            total += a * e;
        }
        total
    }
}

fn invert_symmetric(m: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = m.len();
    let mut work: Vec<Vec<f64>> = m
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut extended = row.clone();
            extended.extend((0..n).map(|j| if i == j { 1.0 } else { 0.0 }));
            extended
        })
        .collect();
    for col in 0..n {
        let pivot_row = (col..n)
            .filter(|&r| work[r][col].is_finite())
            .max_by(|&a, &b| work[a][col].abs().total_cmp(&work[b][col].abs()))?;
        if work[pivot_row][col].abs() <= PIVOT_EPSILON {
            return None;
        }
        work.swap(col, pivot_row);
        let pivot = work[col][col];
        for k in 0..2 * n {
            work[col][k] /= pivot;
        }
        for row in 0..n {
            if row == col {
                continue;
            }
            let factor = work[row][col];
            if factor == 0.0 {
                continue;
            }
            for k in 0..2 * n {
                work[row][k] -= factor * work[col][k];
            }
        }
    }
    let inverse: Vec<Vec<f64>> = work.into_iter().map(|row| row[n..].to_vec()).collect();
    inverse.iter().flatten().all(|v| v.is_finite()).then_some(inverse)
}

fn parameter_covariance<F>(
    x: &[f64],
    y: &[f64],
    w: &[f64],
    params: &[f64],
    k_rows: &[[f64; 2]],
    cov_c: &[[f64; 2]; 2],
    model: &F,
) -> Option<Vec<Vec<f64>>>
where
    F: Fn(f64, &[f64], &mut [f64]) -> f64,
{
    let np = params.len();
    let mut normal = vec![vec![0.0; np]; np];
    let mut cross = vec![[0.0; CONTINUUM_PARAMETERS]; np];
    let mut row = vec![0.0; np];
    for i in 0..x.len() {
        if !(w[i] > 0.0) || !y[i].is_finite() {
            continue;
        }
        row.iter_mut().for_each(|v| *v = 0.0);
        let value = model(x[i], params, &mut row);
        if !value.is_finite() || row.iter().any(|v| !v.is_finite()) {
            return None;
        }
        for a in 0..np {
            for b in 0..np {
                normal[a][b] += w[i] * row[a] * row[b];
            }
            for c in 0..CONTINUUM_PARAMETERS {
                cross[a][c] += w[i] * row[a] * k_rows[i][c];
            }
        }
    }
    let cov0 = invert_symmetric(&normal)?;
    let mut g = vec![[0.0; CONTINUUM_PARAMETERS]; np];
    for a in 0..np {
        for c in 0..CONTINUUM_PARAMETERS {
            g[a][c] = (0..np).map(|b| cov0[a][b] * cross[b][c]).sum();
        }
    }
    let mut cov = cov0;
    for a in 0..np {
        for b in 0..np {
            let mut extra = 0.0;
            for c in 0..CONTINUUM_PARAMETERS {
                for d in 0..CONTINUUM_PARAMETERS {
                    extra += g[a][c] * cov_c[c][d] * g[b][d];
                }
            }
            cov[a][b] += extra;
        }
    }
    cov.iter().flatten().all(|v| v.is_finite()).then_some(cov)
}

struct SingleFit {
    params: [f64; 3],
    errors: [f64; 3],
    cov_a_sigma: f64,
}

struct SpaxelOutcome {
    attempted: bool,
    mask: u32,
    dropped_dq: Vec<usize>,
    dropped_err: Vec<usize>,
    line_channels: Vec<usize>,
    continuum: Option<ContinuumFit>,
    unit_weights: bool,
    single: Option<ComponentFit>,
    chi2: f64,
    dof: usize,
    iterations: usize,
    converged: bool,
    pair: Option<[ComponentFit; 2]>,
    delta_bic: Option<f64>,
    notes: Vec<String>,
}

impl SpaxelOutcome {
    fn empty(dropped_dq: Vec<usize>, dropped_err: Vec<usize>, line_channels: Vec<usize>) -> SpaxelOutcome {
        SpaxelOutcome {
            attempted: false,
            mask: 0,
            dropped_dq,
            dropped_err,
            line_channels,
            continuum: None,
            unit_weights: false,
            single: None,
            chi2: f64::NAN,
            dof: 0,
            iterations: 0,
            converged: false,
            pair: None,
            delta_bic: None,
            notes: Vec::new(),
        }
    }

    fn chi2_red(&self) -> f64 {
        if self.converged && self.dof > 0 {
            self.chi2 / self.dof as f64
        } else {
            f64::NAN
        }
    }

    fn ncomp(&self) -> u8 {
        match (self.attempted, &self.pair) {
            (false, _) => 0,
            (true, None) => 1,
            (true, Some(_)) => 2,
        }
    }
}

enum PairOutcome {
    Accepted { components: [ComponentFit; 2], delta_bic: f64 },
    Rejected { reason: String, delta_bic: Option<f64> },
}

fn rejected(reason: impl Into<String>, delta_bic: Option<f64>) -> PairOutcome {
    PairOutcome::Rejected { reason: reason.into(), delta_bic }
}

#[derive(Clone, Copy)]
struct PairInput<'a> {
    xs: &'a [f64],
    ys: &'a [f64],
    weights: &'a [f64],
    continuum: &'a ContinuumFit,
    single: &'a SingleFit,
    chi2_single: f64,
    dof_single: usize,
    mean_dx: f64,
    lower: [f64; SINGLE_PARAMETERS],
    upper: [f64; SINGLE_PARAMETERS],
}

fn fit_two(ctx: &SpaxelContext, input: &PairInput) -> PairOutcome {
    let PairInput { xs, ys, weights, continuum, single, chi2_single, dof_single, mean_dx, lower, upper } = *input;
    let model_single = gaussian_model(continuum.intercept, continuum.slope, ctx.x_ref);
    let mut jac = [0.0; SINGLE_PARAMETERS];
    let residuals: Vec<f64> = xs.iter().zip(ys).map(|(&x, &y)| y - model_single(x, &single.params, &mut jac)).collect();
    let [a1, x1, s1] = single.params;
    let sign = if a1 < 0.0 { -1.0 } else { 1.0 };
    let Some((peak, &peak_residual)) = residuals.iter().enumerate().max_by(|a, b| (a.1 * sign).total_cmp(&(b.1 * sign))) else {
        return rejected("no line samples for the second component", None);
    };
    let a2 = sign * (peak_residual * sign).max(SECOND_AMPLITUDE_FRACTION * a1.abs());
    let residual_start = [a1, x1, s1, a2, xs[peak], s1];
    let half = SPLIT_START_FRACTION * s1;
    let split_start = [a1, x1 - half, half, a1, x1 + half, half];
    let lower6 = [lower[0], lower[1], lower[2], lower[0], lower[1], lower[2]];
    let upper6 = [upper[0], upper[1], upper[2], upper[0], upper[1], upper[2]];
    let model = two_gaussian_model(continuum.intercept, continuum.slope, ctx.x_ref);
    let bic_gain = |fit: &FitResult| {
        let n = (fit.dof + PAIR_PARAMETERS) as f64;
        (chi2_single + SINGLE_PARAMETERS as f64 * n.ln()) - (fit.chi2 + PAIR_PARAMETERS as f64 * n.ln())
    };
    let mut converged = None;
    let mut failure = (String::new(), None);
    for start in [residual_start, split_start] {
        match fit_least_squares(xs, ys, weights, &start, &lower6, &upper6, MAX_PAIR_ITERATIONS, &model) {
            Ok(fit) if fit.converged => match sign_rejection([fit.params[0], fit.params[3]], a1) {
                Some(reason) => failure = (reason, Some(bic_gain(&fit))),
                None => {
                    converged = Some(fit);
                    break;
                }
            },
            Ok(fit) => failure = (format!("pair fit did not converge after {} iterations", fit.iterations), None),
            Err(reason) => failure = (format!("pair fit skipped: {}", reason), None),
        }
    }
    let Some(fit) = converged else {
        return rejected(failure.0, failure.1);
    };
    let delta_bic = bic_gain(&fit);
    let p = &fit.params;
    let span = ctx.x_bounds.1 - ctx.x_bounds.0;
    let sigma_lower = SIGMA_LOWER_BOUND_FRACTION * mean_dx;
    if (p[1] - p[4]).abs() < SEPARATION_MIN_CHANNELS * mean_dx {
        return rejected("centres separated by less than one channel", Some(delta_bic));
    }
    for c in 0..2 {
        let (a, xc, s) = (p[3 * c], p[3 * c + 1], p[3 * c + 2]);
        if !(s > SIGMA_LOWER_MARGIN * sigma_lower) {
            return rejected(format!("component {} sigma at the lower bound", c + 1), Some(delta_bic));
        }
        if s > COMPONENT_WIDTH_FRACTION * span {
            return rejected(format!("component {} broader than a quarter of the window", c + 1), Some(delta_bic));
        }
        if xc == ctx.x_bounds.0 || xc == ctx.x_bounds.1 {
            return rejected(format!("component {} centre pinned at a window bound", c + 1), Some(delta_bic));
        }
        if !(a.is_finite() && xc.is_finite()) {
            return rejected(format!("component {} parameters are not finite", c + 1), Some(delta_bic));
        }
    }
    if let Some(reason) = flux_fraction_rejection([p[0] * p[2], p[3] * p[5]]) {
        return rejected(reason, Some(delta_bic));
    }
    let k_rows: Vec<[f64; 2]> = xs.iter().map(|&x| [1.0, x - ctx.x_ref]).collect();
    let Some(cov) = parameter_covariance(xs, ys, weights, &fit.params, &k_rows, &continuum.cov, &model) else {
        return rejected("pair covariance is singular", Some(delta_bic));
    };
    let scale = error_scale(ctx.weighting, fit.chi2, fit.dof);
    let errors: Vec<f64> = (0..PAIR_PARAMETERS).map(|i| (scale * cov[i][i]).max(0.0).sqrt()).collect();
    let fits = [0, 1].map(|c| SingleFit {
        params: [p[3 * c], p[3 * c + 1], p[3 * c + 2]],
        errors: [errors[3 * c], errors[3 * c + 1], errors[3 * c + 2]],
        cov_a_sigma: scale * cov[3 * c][3 * c + 2],
    });
    if ctx.components == Components::Auto {
        let chi2_red_single = if dof_single > 0 { chi2_single / dof_single as f64 } else { 1.0 };
        let amplitude_snrs = [0, 1].map(|c| fits[c].params[0] / fits[c].errors[0]);
        if let Some(reason) = auto_rejection(delta_bic, chi2_red_single, amplitude_snrs) {
            return rejected(reason, Some(delta_bic));
        }
    }
    let mut components: Vec<ComponentFit> = fits.iter().map(|f| component_fit(ctx, f, mean_dx).0).collect();
    components.sort_by(|a, b| a.velocity_kms.total_cmp(&b.velocity_kms));
    let second = components.pop().expect("two components");
    let first = components.pop().expect("two components");
    PairOutcome::Accepted { components: [first, second], delta_bic }
}

fn sign_rejection(amplitudes: [f64; 2], single_amplitude: f64) -> Option<String> {
    let sign = if single_amplitude < 0.0 { -1.0 } else { 1.0 };
    amplitudes
        .iter()
        .position(|a| a * sign < 0.0)
        .map(|c| format!("component {} has the opposite sign to the single fit", c + 1))
}

fn flux_fraction_rejection(fluxes: [f64; 2]) -> Option<String> {
    let total = fluxes[0].abs() + fluxes[1].abs();
    if !(total > 0.0) || fluxes.iter().any(|f| f.abs() / total < COMPONENT_FLUX_FRACTION_MIN) {
        return Some("a component carries less than 10 % of the flux".to_string());
    }
    None
}

fn auto_rejection(delta_bic: f64, chi2_red_single: f64, amplitude_snrs: [f64; 2]) -> Option<String> {
    let margin = delta_bic / chi2_red_single.max(1.0);
    if !(margin >= BIC_MARGIN) {
        return Some(format!("normalised ΔBIC {:.1} below {}", margin, BIC_MARGIN));
    }
    amplitude_snrs
        .iter()
        .position(|snr| !(snr.abs() >= COMPONENT_SNR_MIN))
        .map(|c| format!("component {} amplitude below {} sigma", c + 1, COMPONENT_SNR_MIN))
}

fn component_fit(ctx: &SpaxelContext, fit: &SingleFit, mean_dx: f64) -> (ComponentFit, bool) {
    let [a, xc, sx] = fit.params;
    let [a_err, xc_err, sx_err] = fit.errors;
    let v = ctx.to_velocity(xc);
    let v_err = xc_err * ctx.dv_dx_at(xc, mean_dx);
    let sv = (ctx.to_velocity(xc + sx) - v).abs();
    let k = if sx > 0.0 { sv / sx } else { f64::NAN };
    let sv_err = sx_err * k;
    let flux = a * sv * sqrt_two_pi();
    let cov_a_sv = k * fit.cov_a_sigma;
    let variance = (sv * a_err) * (sv * a_err) + (a * sv_err) * (a * sv_err) + 2.0 * a * sv * cov_a_sv;
    let flux_err = sqrt_two_pi() * variance.max(0.0).sqrt();
    let snr = if flux_err > 0.0 { flux / flux_err } else { f64::NAN };
    let (sigma_corr, unresolved) = match ctx.sigma_inst {
        Some(inst) if sv > inst => (Some((sv * sv - inst * inst).sqrt()), false),
        Some(_) => (Some(f64::NAN), true),
        None => (None, false),
    };
    let component = ComponentFit {
        amplitude: a,
        centre: xc,
        sigma: sx,
        amplitude_err: a_err,
        centre_err: xc_err,
        sigma_err: sx_err,
        velocity_kms: v,
        v_err_kms: v_err,
        sigma_kms: sv,
        sigma_err_kms: sv_err,
        sigma_corr_kms: sigma_corr,
        flux,
        flux_err,
        snr,
    };
    (component, unresolved)
}

struct LineSample {
    x: f64,
    y: f64,
    dx: f64,
    excess: f64,
}

fn start_values(samples: &[LineSample], mean_dx: f64, x_bounds: (f64, f64), emission: bool) -> [f64; 3] {
    let span = x_bounds.1 - x_bounds.0;
    if !emission {
        let trough = samples.iter().min_by(|a, b| a.excess.total_cmp(&b.excess)).map(|s| (s.excess, s.x));
        let (depth, x) = trough.unwrap_or((0.0, x_bounds.0));
        return [depth, x, span / 4.0];
    }
    let peak = samples.iter().max_by(|a, b| a.excess.total_cmp(&b.excess)).map(|s| (s.excess, s.x));
    let (peak_excess, peak_x) = peak.unwrap_or((0.0, x_bounds.0));
    let positive: Vec<&LineSample> = samples.iter().filter(|s| s.excess > 0.0).collect();
    let weight: f64 = positive.iter().map(|s| s.excess * s.dx).sum();
    let (x0, sigma0) = if weight > 0.0 {
        let x0 = positive.iter().map(|s| s.excess * s.x * s.dx).sum::<f64>() / weight;
        let variance = positive.iter().map(|s| s.excess * (s.x - x0) * (s.x - x0) * s.dx).sum::<f64>() / weight;
        let sigma0 = if variance > 0.0 { variance.sqrt() } else { mean_dx };
        (x0, sigma0.clamp(SIGMA_LOWER_BOUND_FRACTION * mean_dx, span.max(mean_dx)))
    } else {
        (peak_x, mean_dx)
    };
    [peak_excess.max(0.0), x0, sigma0]
}

fn fit_one(ctx: &SpaxelContext, sci: &[f32], err: Option<&[f32]>, dq: Option<&[f32]>) -> SpaxelOutcome {
    let n = ctx.channels.len();
    let mut usable = vec![false; n];
    let mut dropped_dq = Vec::new();
    let mut dropped_err = Vec::new();
    for k in 0..n {
        if !sci[k].is_finite() {
            continue;
        }
        let mut ok = true;
        if let Some(e) = err {
            if !(e[k].is_finite() && e[k] > 0.0) {
                ok = false;
                dropped_err.push(ctx.channels[k]);
            }
        }
        if let Some(d) = dq {
            if dq_flagged(d[k]) {
                ok = false;
                dropped_dq.push(ctx.channels[k]);
            }
        }
        usable[k] = ok;
    }
    let line_channels: Vec<usize> = ctx.line.iter().filter(|&&k| usable[k]).map(|&k| ctx.channels[k]).collect();
    let raw_weight = |k: usize| match err {
        Some(e) => 1.0 / (e[k] as f64 * e[k] as f64),
        None => 1.0,
    };

    let cont_a: Vec<usize> = ctx.window_a.iter().copied().filter(|&k| usable[k]).collect();
    let cont_b: Vec<usize> = ctx.window_b.iter().copied().filter(|&k| usable[k]).collect();
    let mut cont_positions: Vec<usize> = cont_a.iter().chain(&cont_b).copied().collect();
    cont_positions.sort_unstable();
    cont_positions.dedup();
    if cont_positions.is_empty() || line_channels.len() < MIN_LINE_SAMPLES {
        return SpaxelOutcome::empty(dropped_dq, dropped_err, line_channels);
    }
    let linear = ctx.windows.0 != ctx.windows.1
        && cont_a.len() >= MIN_WINDOW_SAMPLES
        && cont_b.len() >= MIN_WINDOW_SAMPLES
        && cont_positions.len() >= MIN_WINDOW_SAMPLES;
    let cont_dx: Vec<f64> = cont_positions.iter().map(|&k| ctx.axis[ctx.channels[k]] - ctx.x_ref).collect();
    let cont_y: Vec<f64> = cont_positions.iter().map(|&k| sci[k] as f64).collect();
    let cont_w: Vec<f64> = cont_positions.iter().map(|&k| raw_weight(k)).collect();
    let mut continuum = fit_continuum(&cont_dx, &cont_y, &cont_w, linear);

    let mut outcome = SpaxelOutcome::empty(dropped_dq, dropped_err, line_channels);
    outcome.attempted = true;
    if ctx.components != Components::One {
        outcome.mask |= MASK_TWO_REJECTED;
    }
    if !continuum.linear {
        outcome.mask |= MASK_CONST_CONTINUUM;
    }
    if !outcome.dropped_dq.is_empty() {
        outcome.mask |= MASK_DQ_DROPPED;
    }
    if !outcome.dropped_err.is_empty() {
        outcome.mask |= MASK_ERR_DROPPED;
    }

    let scatter_weight = if ctx.weighting == Weighting::Err || (continuum.scatter.is_finite() && continuum.scatter > 0.0) {
        1.0 / (continuum.scatter * continuum.scatter)
    } else {
        outcome.unit_weights = true;
        outcome.notes.push("continuum scatter is zero or unknown: the Gaussian fit used unit weights".to_string());
        1.0
    };
    if ctx.weighting == Weighting::Continuum {
        for row in continuum.cov.iter_mut() {
            for v in row.iter_mut() {
                *v /= scatter_weight;
            }
        }
    }
    let weight_of = |k: usize| match ctx.weighting {
        Weighting::Err => raw_weight(k),
        Weighting::Continuum => scatter_weight,
    };

    let samples: Vec<LineSample> = ctx
        .line
        .iter()
        .copied()
        .filter(|&k| usable[k])
        .map(|k| {
            let z = ctx.channels[k];
            let x = ctx.axis[z];
            let level = continuum.intercept + continuum.slope * (x - ctx.x_ref);
            LineSample { x, y: sci[k] as f64, dx: ctx.widths[z], excess: sci[k] as f64 - level }
        })
        .collect();
    let weights: Vec<f64> = ctx.line.iter().copied().filter(|&k| usable[k]).map(weight_of).collect();
    let xs: Vec<f64> = samples.iter().map(|s| s.x).collect();
    let ys: Vec<f64> = samples.iter().map(|s| s.y).collect();
    let mean_dx = samples.iter().map(|s| s.dx).sum::<f64>() / samples.len() as f64;
    let span = ctx.x_bounds.1 - ctx.x_bounds.0;
    let flux_total: f64 = samples.iter().map(|s| s.excess * s.dx).sum();
    let emission = ctx.emission_only || flux_total >= 0.0;
    let start = start_values(&samples, mean_dx, ctx.x_bounds, emission);
    let lower = [if emission { 0.0 } else { f64::NEG_INFINITY }, ctx.x_bounds.0, SIGMA_LOWER_BOUND_FRACTION * mean_dx];
    let upper = [f64::INFINITY, ctx.x_bounds.1, span.max(mean_dx)];
    let model = gaussian_model(continuum.intercept, continuum.slope, ctx.x_ref);

    let fit = match fit_least_squares(&xs, &ys, &weights, &start, &lower, &upper, MAX_FIT_ITERATIONS, &model) {
        Ok(fit) => fit,
        Err(reason) => {
            outcome.notes.push(format!("Gaussian fit skipped: {}", reason));
            outcome.mask |= MASK_NOT_CONVERGED;
            outcome.continuum = Some(continuum);
            return outcome;
        }
    };
    outcome.chi2 = fit.chi2;
    outcome.dof = fit.dof;
    outcome.iterations = fit.iterations;
    if !fit.converged {
        outcome.notes.push(format!("Gaussian fit did not converge after {} iterations", fit.iterations));
        outcome.mask |= MASK_NOT_CONVERGED;
        outcome.continuum = Some(continuum);
        return outcome;
    }
    let k_rows: Vec<[f64; 2]> = xs.iter().map(|&x| [1.0, x - ctx.x_ref]).collect();
    let Some(cov) = parameter_covariance(&xs, &ys, &weights, &fit.params, &k_rows, &continuum.cov, &model) else {
        outcome.notes.push("Gaussian fit covariance is singular".to_string());
        outcome.mask |= MASK_NOT_CONVERGED;
        outcome.continuum = Some(continuum);
        return outcome;
    };
    let scale = error_scale(ctx.weighting, fit.chi2, fit.dof);
    let errors = [0, 1, 2].map(|i| (scale * cov[i][i]).max(0.0).sqrt());
    let single = SingleFit {
        params: [fit.params[0], fit.params[1], fit.params[2]],
        errors,
        cov_a_sigma: scale * cov[0][2],
    };
    let (component, unresolved) = component_fit(ctx, &single, mean_dx);
    outcome.converged = true;
    if unresolved {
        outcome.mask |= MASK_UNRESOLVED;
    }
    if component.snr.abs() >= ctx.snr_threshold {
        outcome.mask |= MASK_FITTED;
    }
    if ctx.components != Components::One {
        if outcome.mask & MASK_FITTED == 0 {
            outcome.notes.push(format!(
                "two components not attempted: single fit |S/N| {:.1} below the threshold {}",
                component.snr.abs(),
                ctx.snr_threshold
            ));
        } else if xs.len() < MIN_PAIR_SAMPLES {
            outcome.notes.push(format!(
                "two components not attempted: {} usable line samples, {} needed",
                xs.len(),
                MIN_PAIR_SAMPLES
            ));
        } else {
            let input = PairInput {
                xs: &xs,
                ys: &ys,
                weights: &weights,
                continuum: &continuum,
                single: &single,
                chi2_single: fit.chi2,
                dof_single: fit.dof,
                mean_dx,
                lower,
                upper,
            };
            match fit_two(ctx, &input) {
                PairOutcome::Accepted { components, delta_bic } => {
                    outcome.mask &= !MASK_TWO_REJECTED;
                    outcome.mask |= MASK_TWO_COMPONENTS;
                    outcome.pair = Some(components);
                    outcome.delta_bic = Some(delta_bic);
                }
                PairOutcome::Rejected { reason, delta_bic } => {
                    outcome.notes.push(format!("two components rejected: {}", reason));
                    outcome.delta_bic = delta_bic;
                }
            }
        }
    }
    outcome.single = Some(component);
    outcome.continuum = Some(continuum);
    outcome
}

fn check_companion(name: &str, sci: &LazyCube, cube: Option<&LazyCube>) -> Result<()> {
    if let Some(c) = cube {
        let (a, b) = (&sci.geometry, &c.geometry);
        if (a.naxis1, a.naxis2, a.naxis3) != (b.naxis1, b.naxis2, b.naxis3) {
            bail!(
                "{} cube {}x{}x{} does not match the SCI cube",
                name,
                b.naxis1,
                b.naxis2,
                b.naxis3
            );
        }
    }
    Ok(())
}

struct ResolvedInputs<'a> {
    err: Option<&'a LazyCube>,
    dq: Option<&'a LazyCube>,
    weighting: Weighting,
}

fn resolve_inputs<'a>(inputs: &LineFitInputs<'a>, cfg: &LineFitConfig) -> Result<ResolvedInputs<'a>> {
    check_companion("ERR", inputs.sci, inputs.err)?;
    check_companion("DQ", inputs.sci, inputs.dq)?;
    let err = inputs.err.filter(|_| cfg.use_err);
    let dq = inputs.dq.filter(|_| cfg.use_dq);
    let weighting = if err.is_some() { Weighting::Err } else { Weighting::Continuum };
    Ok(ResolvedInputs { err, dq, weighting })
}

fn weighting_note(weighting: Weighting, err: Option<&LazyCube>) -> String {
    match (weighting, err) {
        (Weighting::Err, Some(cube)) => format!(
            "weights: 1/ERR^2 from HDU {} (EXTNAME ERR); absolute-sigma errors, inflated by sqrt(chi2_red) where chi2_red > 1; continuum uncertainty propagated",
            cube.hdu_index
        ),
        _ => "weights: 1/scatter^2 of the continuum residuals per spaxel; formal errors scaled by chi2/dof; continuum uncertainty propagated".to_string(),
    }
}

struct ComponentBuffers {
    flux: Vec<f32>,
    velocity: Vec<f32>,
    sigma_obs: Vec<f32>,
    flux_err: Vec<f32>,
    v_err: Vec<f32>,
    sigma_err: Vec<f32>,
}

impl ComponentBuffers {
    fn new(npix: usize) -> ComponentBuffers {
        ComponentBuffers {
            flux: vec![f32::NAN; npix],
            velocity: vec![f32::NAN; npix],
            sigma_obs: vec![f32::NAN; npix],
            flux_err: vec![f32::NAN; npix],
            v_err: vec![f32::NAN; npix],
            sigma_err: vec![f32::NAN; npix],
        }
    }

    fn set(&mut self, p: usize, c: &ComponentFit) {
        self.flux[p] = c.flux as f32;
        self.velocity[p] = c.velocity_kms as f32;
        self.sigma_obs[p] = c.sigma_kms as f32;
        self.flux_err[p] = c.flux_err as f32;
        self.v_err[p] = c.v_err_kms as f32;
        self.sigma_err[p] = c.sigma_err_kms as f32;
    }

    fn into_planes(self, shape: (usize, usize)) -> Result<ComponentPlanes> {
        Ok(ComponentPlanes {
            flux: Array2::from_shape_vec(shape, self.flux)?,
            velocity: Array2::from_shape_vec(shape, self.velocity)?,
            sigma_obs: Array2::from_shape_vec(shape, self.sigma_obs)?,
            flux_err: Array2::from_shape_vec(shape, self.flux_err)?,
            v_err: Array2::from_shape_vec(shape, self.v_err)?,
            sigma_err: Array2::from_shape_vec(shape, self.sigma_err)?,
        })
    }
}

fn two_component_note(components: Components, accepted: usize) -> String {
    match components {
        Components::Auto => format!(
            "two components: {} spaxels accepted (ΔBIC/max(1, chi2_red) ≥ 10, both components ≥ 3 sigma and ≥ 10 % of the flux, separated by ≥ 1 channel, each narrower than a quarter of the window)",
            accepted
        ),
        _ => format!(
            "two components: {} spaxels accepted (forced; both components ≥ 10 % of the flux, separated by ≥ 1 channel, each narrower than a quarter of the window)",
            accepted
        ),
    }
}

fn median_finite(values: &[f32]) -> Option<f64> {
    let mut finite: Vec<f32> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        None
    } else {
        Some(exact_median_mut(&mut finite))
    }
}

pub fn fit_cube_lines(inputs: &LineFitInputs, cfg: &LineFitConfig) -> Result<LineFitMaps> {
    let sci = inputs.sci;
    let resolved = resolve_inputs(inputs, cfg)?;
    let (ctx, mut notes) = build_context(sci, cfg, resolved.weighting)?;
    let g = &sci.geometry;
    let (rows, cols) = (g.naxis2, g.naxis1);
    let npix = rows * cols;
    let n_cubes = 1 + resolved.err.is_some() as usize + resolved.dq.is_some() as usize;
    let band_rows = median_band_rows(ctx.channels.len() * n_cubes, cols, rows, BAND_BYTES);

    let mut flux = vec![f32::NAN; npix];
    let mut velocity = vec![f32::NAN; npix];
    let mut sigma_obs = vec![f32::NAN; npix];
    let mut sigma_corr = vec![f32::NAN; npix];
    let mut flux_err = vec![f32::NAN; npix];
    let mut v_err = vec![f32::NAN; npix];
    let mut sigma_err = vec![f32::NAN; npix];
    let mut chi2_red = vec![f32::NAN; npix];
    let mut snr = vec![f32::NAN; npix];
    let mut mask = vec![0.0f32; npix];
    let mut ncomp = vec![0.0f32; npix];
    let two_requested = cfg.components != Components::One;
    let mut component_buffers = two_requested.then(|| [ComponentBuffers::new(npix), ComponentBuffers::new(npix)]);
    let mut n_fit = 0usize;
    let mut n_masked = 0usize;
    let mut n_const_continuum = 0usize;
    let mut n_dq_spaxels = 0usize;
    let mut n_unit_weights = 0usize;
    let mut n_unresolved = 0usize;
    let mut n_two_components = 0usize;

    let decode_band = |cube: &LazyCube, band_start: usize, row_count: usize| -> Result<Vec<Vec<f32>>> {
        ctx.channels.par_iter().map(|&z| cube.decode_rows(z, band_start, row_count)).collect()
    };
    for band_start in (0..rows).step_by(band_rows) {
        let band_end = (band_start + band_rows).min(rows);
        let row_count = band_end - band_start;
        let sci_planes = decode_band(sci, band_start, row_count)?;
        let err_planes = match resolved.err {
            Some(cube) => Some(decode_band(cube, band_start, row_count)?),
            None => None,
        };
        let dq_planes = match resolved.dq {
            Some(cube) => Some(decode_band(cube, band_start, row_count)?),
            None => None,
        };
        let band_npix = row_count * cols;
        let outcomes: Vec<SpaxelOutcome> = (0..band_npix)
            .into_par_iter()
            .map(|i| {
                let s: Vec<f32> = sci_planes.iter().map(|p| p[i]).collect();
                let e: Option<Vec<f32>> = err_planes.as_ref().map(|planes| planes.iter().map(|p| p[i]).collect());
                let d: Option<Vec<f32>> = dq_planes.as_ref().map(|planes| planes.iter().map(|p| p[i]).collect());
                fit_one(&ctx, &s, e.as_deref(), d.as_deref())
            })
            .collect();
        let offset = band_start * cols;
        for (i, o) in outcomes.into_iter().enumerate() {
            let p = offset + i;
            if !o.dropped_dq.is_empty() {
                n_dq_spaxels += 1;
            }
            if !o.attempted {
                continue;
            }
            mask[p] = o.mask as f32;
            ncomp[p] = o.ncomp() as f32;
            if o.pair.is_some() {
                n_two_components += 1;
            }
            if o.unit_weights {
                n_unit_weights += 1;
            }
            if o.mask & MASK_CONST_CONTINUUM != 0 {
                n_const_continuum += 1;
            }
            if o.mask & MASK_UNRESOLVED != 0 {
                n_unresolved += 1;
            }
            if !o.converged {
                continue;
            }
            chi2_red[p] = o.chi2_red() as f32;
            if let Some(c) = &o.single {
                snr[p] = c.snr as f32;
                if o.mask & MASK_FITTED != 0 {
                    n_fit += 1;
                    flux[p] = c.flux as f32;
                    velocity[p] = c.velocity_kms as f32;
                    sigma_obs[p] = c.sigma_kms as f32;
                    sigma_corr[p] = c.sigma_corr_kms.unwrap_or(f64::NAN) as f32;
                    flux_err[p] = c.flux_err as f32;
                    v_err[p] = c.v_err_kms as f32;
                    sigma_err[p] = c.sigma_err_kms as f32;
                    if let (Some(buffers), Some(pair)) = (component_buffers.as_mut(), &o.pair) {
                        buffers[0].set(p, &pair[0]);
                        buffers[1].set(p, &pair[1]);
                    }
                } else {
                    n_masked += 1;
                }
            }
        }
    }

    notes.insert(0, weighting_note(resolved.weighting, resolved.err));
    match (resolved.dq, inputs.dq) {
        (Some(cube), _) => notes.push(format!(
            "DQ HDU {}: channels with DO_NOT_USE (bit 0) dropped in {} spaxels",
            cube.hdu_index, n_dq_spaxels
        )),
        (None, Some(cube)) => {
            notes.push(format!("DQ HDU {} present: channel masking off (use_dq = false)", cube.hdu_index))
        }
        (None, None) => notes.push("no DQ cube: no channel masking".to_string()),
    }
    if n_const_continuum > 0 {
        notes.push(format!(
            "continuum: {} spaxels fitted with a constant (one side window empty or too short)",
            n_const_continuum
        ));
    }
    if n_unit_weights > 0 {
        notes.push(format!(
            "continuum scatter zero or unknown in {} spaxels: the Gaussian fit used unit weights there",
            n_unit_weights
        ));
    }
    notes.push(MULTI_PEAK_CAVEAT.to_string());
    if two_requested {
        notes.push(two_component_note(cfg.components, n_two_components));
    }
    let flux_unit = match bunit(sci) {
        Some(unit) => format!("{} {}", unit, VELOCITY_UNIT),
        None => VELOCITY_UNIT.to_string(),
    };
    notes.push(format!(
        "flux = A·sigma_v·sqrt(2π) in {}; flux_err includes the amplitude-sigma covariance and the continuum uncertainty",
        flux_unit
    ));
    let sigma_label = match (cfg.resolving_power, ctx.sigma_inst) {
        (Some(r), Some(inst)) => {
            notes.push(format!(
                "sigma corrected for the LSF with R = {} (sigma_inst = {:.2} km/s); {} spaxels unresolved (sigma_corr NaN)",
                r, inst, n_unresolved
            ));
            format!("LSF-corrected (R = {})", r)
        }
        _ => {
            notes.push("sigma is observed: instrumental width not removed (no resolving power given)".to_string());
            SIGMA_OBSERVED_LABEL.to_string()
        }
    };
    notes.push(format!(
        "SNR threshold {}: {} spaxels fitted, {} converged fits below threshold",
        cfg.snr_threshold, n_fit, n_masked
    ));

    let shape = (rows, cols);
    let plane = |v: Vec<f32>| Array2::from_shape_vec(shape, v);
    let components = match component_buffers {
        Some([first, second]) => Some([first.into_planes(shape)?, second.into_planes(shape)?]),
        None => None,
    };
    Ok(LineFitMaps {
        flux: plane(flux)?,
        velocity: plane(velocity)?,
        sigma_obs: plane(sigma_obs)?,
        sigma_corr: if ctx.sigma_inst.is_some() { Some(plane(sigma_corr)?) } else { None },
        flux_err: plane(flux_err)?,
        v_err: plane(v_err)?,
        sigma_err: plane(sigma_err)?,
        median_chi2_red: median_finite(&chi2_red),
        chi2_red: plane(chi2_red)?,
        snr: plane(snr)?,
        mask: plane(mask)?,
        ncomp: plane(ncomp)?,
        components,
        continuum_windows: ctx.windows,
        flux_unit,
        velocity_unit: VELOCITY_UNIT,
        sigma_label,
        weighting: resolved.weighting,
        err_hdu: resolved.err.map(|c| c.hdu_index),
        dq_hdu: resolved.dq.map(|c| c.hdu_index),
        rest_um: ctx.rest_um,
        n_channels: cfg.z1 - cfg.z0 + 1,
        n_fit,
        n_masked,
        n_const_continuum,
        n_two_components,
        notes,
    })
}

fn axis_at_fractional(axis: &[f64], channel: f64) -> f64 {
    let n = axis.len();
    let base = channel.floor();
    let idx = (base.max(0.0) as usize).min(n - 1);
    let t = channel - base;
    if t <= 0.0 || idx + 1 >= n {
        axis[idx]
    } else {
        axis[idx] + (axis[idx + 1] - axis[idx]) * t
    }
}

fn spaxel_model(ctx: &SpaxelContext, continuum: &ContinuumFit, components: &[&ComponentFit], span: (usize, usize)) -> SpaxelModel {
    let steps = (span.1 - span.0) * MODEL_SUBSAMPLES;
    let channel: Vec<f64> = (0..=steps).map(|k| span.0 as f64 + k as f64 / MODEL_SUBSAMPLES as f64).collect();
    let xs: Vec<f64> = channel.iter().map(|&c| axis_at_fractional(&ctx.axis, c)).collect();
    let cont: Vec<f64> = xs.iter().map(|&x| continuum.intercept + continuum.slope * (x - ctx.x_ref)).collect();
    let curves: Vec<Vec<f64>> = components
        .iter()
        .map(|c| {
            xs.iter()
                .map(|&x| {
                    let d = x - c.centre;
                    c.amplitude * (-d * d / (2.0 * c.sigma * c.sigma)).exp()
                })
                .collect()
        })
        .collect();
    let total: Vec<f64> = cont
        .iter()
        .enumerate()
        .map(|(k, c)| c + curves.iter().map(|curve| curve[k]).sum::<f64>())
        .collect();
    SpaxelModel { channel, continuum: cont, total, components: curves }
}

pub fn fit_spaxel(inputs: &LineFitInputs, cfg: &LineFitConfig, x: usize, y: usize) -> Result<SpaxelFit> {
    let sci = inputs.sci;
    let g = &sci.geometry;
    if x >= g.naxis1 || y >= g.naxis2 {
        bail!("pixel ({}, {}) is outside the {}x{} cube", x, y, g.naxis1, g.naxis2);
    }
    let resolved = resolve_inputs(inputs, cfg)?;
    let (ctx, mut notes) = build_context(sci, cfg, resolved.weighting)?;
    let sci_spectrum = sci.extract_spectrum_at(y, x)?;
    let err_spectrum = match resolved.err {
        Some(cube) => Some(cube.extract_spectrum_at(y, x)?),
        None => None,
    };
    let dq_spectrum = match resolved.dq {
        Some(cube) => Some(cube.extract_spectrum_at(y, x)?),
        None => None,
    };
    let gather = |spectrum: &Vec<f32>| -> Vec<f32> { ctx.channels.iter().map(|&z| spectrum[z]).collect() };
    let s = gather(&sci_spectrum);
    let e = err_spectrum.as_ref().map(gather);
    let d = dq_spectrum.as_ref().map(gather);
    let outcome = fit_one(&ctx, &s, e.as_deref(), d.as_deref());

    let span = (
        cfg.z0.min(ctx.windows.0 .0).min(ctx.windows.1 .0),
        cfg.z1.max(ctx.windows.0 .1).max(ctx.windows.1 .1),
    );
    notes.insert(0, weighting_note(resolved.weighting, resolved.err));
    notes.extend(outcome.notes.iter().cloned());
    let model = match (&outcome.continuum, &outcome.single, &outcome.pair) {
        (Some(c), _, Some(pair)) => Some(spaxel_model(&ctx, c, &[&pair[0], &pair[1]], span)),
        (Some(c), Some(single), None) => Some(spaxel_model(&ctx, c, &[single], span)),
        _ => None,
    };
    let continuum = outcome.continuum.as_ref().map(|c| ContinuumParams {
        intercept: c.intercept,
        slope: c.slope,
        x_ref: ctx.x_ref,
        sigma: c.scatter,
        linear: c.linear,
        channels: c.channels,
    });
    Ok(SpaxelFit {
        x,
        y,
        z0: cfg.z0,
        z1: cfg.z1,
        continuum_windows: ctx.windows,
        span,
        axis: ctx.axis[span.0..=span.1].to_vec(),
        axis_unit: ctx.axis_unit.clone(),
        flux: sci_spectrum[span.0..=span.1].to_vec(),
        err: err_spectrum.as_ref().map(|e| e[span.0..=span.1].to_vec()),
        channels: outcome.line_channels.clone(),
        dropped_dq: outcome.dropped_dq.clone(),
        dropped_err: outcome.dropped_err.clone(),
        continuum,
        weighting: resolved.weighting,
        chi2: outcome.chi2,
        dof: outcome.dof,
        chi2_red: outcome.chi2_red(),
        converged: outcome.converged,
        iterations: outcome.iterations,
        ncomp: outcome.ncomp(),
        components: outcome.pair.map(|pair| pair.to_vec()).unwrap_or_default(),
        delta_bic: outcome.delta_bic,
        mask: outcome.mask,
        single: outcome.single,
        model,
        notes,
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::core::cube::lazy::test_support::*;

    pub const SMALL_DISK_CENTRE: f64 = 8.0;
    pub const SMALL_DISK_RADIUS: f64 = 5.0;
    pub const SMALL_CUBE_SIZE: usize = 16;

    pub fn write_linefit_mef(
        path: &std::path::Path,
        cols: usize,
        rows: usize,
        depth: usize,
        sci: impl Fn(usize, usize, usize) -> f32,
        err: Option<impl Fn(usize, usize, usize) -> f32>,
        dq: Option<impl Fn(usize, usize, usize) -> i32>,
    ) {
        let axes = [cols, rows, depth];
        let mut bytes = image_hdu("SIMPLE", &[], 8, &[("EXTEND", "T")], Vec::new());
        let mut sci_cards = line_cube_cards();
        sci_cards.push(("EXTNAME", "'SCI'"));
        bytes.extend(image_hdu("XTENSION", &axes, -32, &sci_cards, f32_samples(cols, rows, depth, sci)));
        if let Some(err) = err {
            bytes.extend(image_hdu("XTENSION", &axes, -32, &[("EXTNAME", "'ERR'")], f32_samples(cols, rows, depth, err)));
        }
        if let Some(dq) = dq {
            let mut data = Vec::with_capacity(cols * rows * depth * 4);
            for z in 0..depth {
                for y in 0..rows {
                    for x in 0..cols {
                        data.extend_from_slice(&dq(z, y, x).to_be_bytes());
                    }
                }
            }
            bytes.extend(image_hdu("XTENSION", &axes, 32, &[("EXTNAME", "'DQ'")], data));
        }
        write_bytes(path, &bytes);
    }

    pub fn gaussian(z: f64, centre: f64, sigma: f64) -> f64 {
        let d = z - centre;
        (-d * d / (2.0 * sigma * sigma)).exp()
    }

    pub fn line_sci(z: usize, y: usize, x: usize) -> f32 {
        if inside_disk(y, x) {
            LINE_CONTINUUM + line_profile(z)
        } else {
            LINE_CONTINUUM
        }
    }

    pub fn noise_amplitude(sigma: f32) -> f32 {
        sigma * 3f32.sqrt()
    }

    pub fn inside_small_disk(y: usize, x: usize) -> bool {
        let dx = x as f64 - SMALL_DISK_CENTRE;
        let dy = y as f64 - SMALL_DISK_CENTRE;
        dx * dx + dy * dy <= SMALL_DISK_RADIUS * SMALL_DISK_RADIUS
    }

    pub fn small_disk_pixel_count() -> usize {
        (0..SMALL_CUBE_SIZE)
            .flat_map(|y| (0..SMALL_CUBE_SIZE).map(move |x| (y, x)))
            .filter(|&(y, x)| inside_small_disk(y, x))
            .count()
    }

    pub fn line_cfg() -> LineFitConfig {
        LineFitConfig {
            z0: 8,
            z1: 32,
            rest_um: None,
            convention: VelocityConvention::Optical,
            continuum: Some(((0, 5), (34, 39))),
            snr_threshold: DEFAULT_SNR_THRESHOLD,
            emission_only: true,
            use_err: true,
            use_dq: true,
            resolving_power: None,
            components: Components::One,
        }
    }

    pub fn wide_cfg() -> LineFitConfig {
        LineFitConfig { z0: 12, z1: 46, continuum: Some(((0, 9), (50, 59))), ..line_cfg() }
    }

    pub fn channel_kms() -> f64 {
        SPEED_OF_LIGHT_KMS * LINE_CDELT_UM / LINE_REST_UM
    }

    pub fn v_of_channel(z: f64) -> f64 {
        velocity_kms(1.0 + LINE_CDELT_UM * z, LINE_REST_UM, VelocityConvention::Optical)
    }

    pub fn centre_channel(fit: &ComponentFit) -> f64 {
        (fit.centre - 1.0) / LINE_CDELT_UM
    }

    pub fn sigma_channels(fit: &ComponentFit) -> f64 {
        fit.sigma / LINE_CDELT_UM
    }

    pub fn centre_err_channels(fit: &ComponentFit) -> f64 {
        fit.centre_err / LINE_CDELT_UM
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::core::astrometry::spectral::velocity_axis;
    use crate::core::cube::lazy::test_support::*;

    fn open(path: &std::path::Path) -> LazyCube {
        LazyCube::open(path.to_str().unwrap()).unwrap()
    }

    fn open_hdu(path: &std::path::Path, hdu: usize) -> LazyCube {
        LazyCube::open(&format!("{}#hdu={}", path.to_str().unwrap(), hdu)).unwrap()
    }

    fn only_sci(sci: &LazyCube) -> LineFitInputs<'_> {
        LineFitInputs { sci, err: None, dq: None }
    }

    fn bit(mask: f32, flag: u32) -> bool {
        (mask as u32) & flag != 0
    }

    fn distance_from_disk_centre(y: usize, x: usize) -> f64 {
        (x as f64 - LINE_DISK_CENTRE).hypot(y as f64 - LINE_DISK_CENTRE)
    }

    struct PullStats {
        mean: f64,
        std: f64,
        within_two: f64,
    }

    fn pull_stats(pulls: &[f64]) -> PullStats {
        let n = pulls.len() as f64;
        let mean = pulls.iter().sum::<f64>() / n;
        let variance = pulls.iter().map(|p| (p - mean) * (p - mean)).sum::<f64>() / (n - 1.0);
        let within_two = pulls.iter().filter(|p| p.abs() < 2.0).count() as f64 / n;
        PullStats { mean, std: variance.sqrt(), within_two }
    }

    #[test]
    fn zero_noise_line_cube_is_recovered_analytically_on_every_disk_spaxel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.0);
        let cube = open(&path);
        let maps = fit_cube_lines(&only_sci(&cube), &line_cfg()).unwrap();
        let dv = channel_kms();
        let expected_flux = 1.0 * 3.0 * dv * sqrt_two_pi();
        for y in 0..LINE_CUBE_SIZE {
            for x in 0..LINE_CUBE_SIZE {
                let p = [y, x];
                if inside_disk(y, x) {
                    assert!(maps.velocity[p].abs() < (0.01 * dv) as f32, "v({}, {})={}", x, y, maps.velocity[p]);
                    assert!((maps.sigma_obs[p] as f64 - 3.0 * dv).abs() < 0.01 * dv, "sigma={}", maps.sigma_obs[p]);
                    assert!((maps.flux[p] as f64 - expected_flux).abs() < 1e-4 * expected_flux, "flux={}", maps.flux[p]);
                    assert!(bit(maps.mask[p], MASK_FITTED), "mask({}, {})={}", x, y, maps.mask[p]);
                } else if distance_from_disk_centre(y, x) > 8.0 {
                    assert!(maps.velocity[p].is_nan(), "v({}, {})={}", x, y, maps.velocity[p]);
                    assert!(!bit(maps.mask[p], MASK_FITTED), "mask({}, {})={}", x, y, maps.mask[p]);
                }
            }
        }
        assert_eq!(maps.n_fit, disk_pixel_count());
        assert_eq!(maps.weighting, Weighting::Continuum);
        assert_eq!(maps.flux_unit, "Jy/beam km/s");
        assert_eq!(maps.velocity_unit, "km/s");
        assert_eq!(maps.n_channels, 25);
        assert!(maps.notes.iter().any(|n| n.starts_with("weights: 1/scatter^2 of the continuum residuals per spaxel")), "{:?}", maps.notes);
        assert!(maps.notes.iter().any(|n| n == MULTI_PEAK_CAVEAT));
        assert!(maps.notes.iter().any(|n| n.contains("axis frame BARYCENT")), "{:?}", maps.notes);
        assert!(maps.notes.iter().any(|n| n == "no DQ cube: no channel masking"));
    }

    #[test]
    fn err_weighted_pulls_are_unit_normal_with_absolute_sigma_errors() {
        let dir = tempfile::tempdir().unwrap();
        let dv = channel_kms();
        let truth_flux = 3.0 * dv * sqrt_two_pi();
        let mut pulls: [Vec<f64>; 4] = Default::default();
        let mut chi2_reds = Vec::new();
        for seed in 0..3usize {
            let path = dir.path().join(format!("pulls_{}.fits", seed));
            write_linefit_mef(
                &path,
                24,
                24,
                40,
                |z, y, x| {
                    let centre = 20.0 + 0.15 * (x as f64 - 12.0);
                    (1.0 + gaussian(z as f64, centre, 3.0)) as f32
                        + deterministic_noise(z + seed * 1000, y, x, noise_amplitude(0.05))
                },
                Some(|_, _, _| 0.05f32),
                None::<fn(usize, usize, usize) -> i32>,
            );
            let sci = open_hdu(&path, 1);
            let err = open_hdu(&path, 2);
            let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
            let maps = fit_cube_lines(&inputs, &line_cfg()).unwrap();
            assert_eq!(maps.weighting, Weighting::Err);
            for y in 0..24 {
                for x in 0..24 {
                    let (fx, fy) = (x as f64 - 12.0, y as f64 - 12.0);
                    if fx * fx + fy * fy > 64.0 {
                        continue;
                    }
                    assert!(bit(maps.mask[[y, x]], MASK_FITTED), "seed {} ({}, {}) mask {}", seed, x, y, maps.mask[[y, x]]);
                    let fit = fit_spaxel(&inputs, &line_cfg(), x, y).unwrap();
                    let single = fit.single.expect("single fit");
                    let truth_centre = 20.0 + 0.15 * fx;
                    pulls[0].push((centre_channel(&single) - truth_centre) / centre_err_channels(&single));
                    pulls[1].push((sigma_channels(&single) - 3.0) / (single.sigma_err / LINE_CDELT_UM));
                    pulls[2].push((single.amplitude - 1.0) / single.amplitude_err);
                    pulls[3].push((single.flux - truth_flux) / single.flux_err);
                    chi2_reds.push(fit.chi2_red);
                }
            }
        }
        for (name, values) in ["centre", "sigma", "amplitude", "flux"].iter().zip(&pulls) {
            let s = pull_stats(values);
            println!("{} pull: n={} mean={:.3} std={:.3} within2={:.3}", name, values.len(), s.mean, s.std, s.within_two);
            assert!(s.mean.abs() < 0.2, "{} pull mean {}", name, s.mean);
            assert!(s.std >= 0.8 && s.std <= 1.25, "{} pull std {}", name, s.std);
            assert!(s.within_two >= 0.92, "{} within 2 sigma {}", name, s.within_two);
        }
        let mut sorted: Vec<f32> = chi2_reds.iter().map(|v| *v as f32).collect();
        let median = exact_median_mut(&mut sorted);
        println!("median chi2_red {:.3}", median);
        assert!(median >= 0.8 && median <= 1.3, "median chi2_red {}", median);
    }

    const BAD_HALF_NOISE: f32 = 0.01;

    #[test]
    fn err_down_weighting_keeps_the_centre_where_the_well_measured_half_puts_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad_half.fits");
        write_linefit_mef(
            &path,
            LINE_CUBE_SIZE,
            LINE_CUBE_SIZE,
            LINE_CUBE_DEPTH,
            |z, y, x| {
                let bump = if (21..=24).contains(&z) { 1.0 } else { 0.0 };
                line_sci(z, y, x) + deterministic_noise(z, y, x, noise_amplitude(BAD_HALF_NOISE)) + bump
            },
            Some(|z, _, _| if (21..=32).contains(&z) { 5.0f32 } else { BAD_HALF_NOISE }),
            None::<fn(usize, usize, usize) -> i32>,
        );
        let sci = open_hdu(&path, 1);
        let err = open_hdu(&path, 2);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
        let on_cfg = line_cfg();
        let off_cfg = LineFitConfig { use_err: false, ..line_cfg() };
        let mut total = 0usize;
        let mut passed = 0usize;
        let (mut centre_sum, mut centre_sq_sum, mut err_sum, mut delta_sum) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for y in 0..LINE_CUBE_SIZE {
            for x in 0..LINE_CUBE_SIZE {
                if !inside_disk(y, x) {
                    continue;
                }
                let on = fit_spaxel(&inputs, &on_cfg, x, y).unwrap();
                let off = fit_spaxel(&inputs, &off_cfg, x, y).unwrap();
                assert!(on.dropped_err.is_empty() && off.dropped_err.is_empty());
                assert!(on.mask & MASK_ERR_DROPPED == 0 && off.mask & MASK_ERR_DROPPED == 0);
                assert!(on.notes.iter().any(|n| n.contains("HDU 2")), "{:?}", on.notes);
                assert_eq!(off.weighting, Weighting::Continuum);
                assert_eq!(on.weighting, Weighting::Err);
                let (Some(c_on), Some(c_off)) = (on.single, off.single) else {
                    total += 1;
                    continue;
                };
                total += 1;
                let err_on = centre_err_channels(&c_on);
                let delta = centre_channel(&c_off) - centre_channel(&c_on);
                centre_sum += centre_channel(&c_on);
                centre_sq_sum += centre_channel(&c_on) * centre_channel(&c_on);
                err_sum += err_on;
                delta_sum += delta;
                if (centre_channel(&c_on) - 20.0).abs() < 3.0 * err_on && delta > 3.0 * err_on {
                    passed += 1;
                }
            }
        }
        let n = total as f64;
        let centre_mean = centre_sum / n;
        let centre_std = (centre_sq_sum / n - centre_mean * centre_mean).max(0.0).sqrt();
        println!(
            "down-weighting: {} of {} disk spaxels pass; c_on mean {:.3} std {:.3}, mean err_on {:.3}, mean red shift {:.3} ch",
            passed,
            total,
            centre_mean,
            centre_std,
            err_sum / n,
            delta_sum / n
        );
        assert!(passed as f64 >= 0.95 * total as f64, "{} of {}", passed, total);
    }

    #[test]
    fn dq_do_not_use_channels_are_dropped_and_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dq.fits");
        let hot = |z: usize, y: usize| (19..=21).contains(&z) && (y == 16 || y == 20);
        write_linefit_mef(
            &path,
            LINE_CUBE_SIZE,
            LINE_CUBE_SIZE,
            LINE_CUBE_DEPTH,
            move |z, y, x| line_sci(z, y, x) + if hot(z, y) { 5.0 } else { 0.0 },
            Some(|_, _, _| 0.05f32),
            Some(move |z, y, _| if hot(z, y) { if y == 16 { 1 } else { 513 } } else { 0 }),
        );
        let sci = open_hdu(&path, 1);
        let err = open_hdu(&path, 2);
        let dq = open_hdu(&path, 3);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: Some(&dq) };
        let on = fit_cube_lines(&inputs, &line_cfg()).unwrap();
        let off_cfg = LineFitConfig { use_dq: false, ..line_cfg() };
        let off = fit_cube_lines(&inputs, &off_cfg).unwrap();
        let dv = channel_kms();
        assert_eq!(on.n_fit, disk_pixel_count());
        assert_eq!(on.dq_hdu, Some(3));
        assert_eq!(off.dq_hdu, None);
        assert!(
            on.notes.iter().any(|n| n == "DQ HDU 3: channels with DO_NOT_USE (bit 0) dropped in 64 spaxels"),
            "{:?}",
            on.notes
        );
        assert!(!on.notes.iter().any(|n| n.contains("unit weights")), "{:?}", on.notes);
        assert!(
            off.notes.iter().any(|n| n == "DQ HDU 3 present: channel masking off (use_dq = false)"),
            "{:?}",
            off.notes
        );
        for y in 0..LINE_CUBE_SIZE {
            for x in 0..LINE_CUBE_SIZE {
                let p = [y, x];
                if y == 16 || y == 20 {
                    if !inside_disk(y, x) {
                        continue;
                    }
                    assert!(bit(on.mask[p], MASK_DQ_DROPPED), "mask({}, {})={}", x, y, on.mask[p]);
                    assert!(on.velocity[p].abs() < (0.01 * dv) as f32, "v({}, {})={}", x, y, on.velocity[p]);
                    let spaxel = fit_spaxel(&inputs, &line_cfg(), x, y).unwrap();
                    assert_eq!(spaxel.dropped_dq, vec![19, 20, 21]);
                    assert!(!spaxel.notes.iter().any(|n| n.contains("unit weights")), "{:?}", spaxel.notes);
                    let single = spaxel.single.expect("single");
                    assert!((single.amplitude - 1.0).abs() < 1e-3, "amplitude {}", single.amplitude);
                    let unmasked = fit_spaxel(&inputs, &off_cfg, x, y).unwrap();
                    let biased = unmasked.single.map_or(true, |s| (s.amplitude - 1.0).abs() > 0.1);
                    assert!(biased || unmasked.mask & MASK_NOT_CONVERGED != 0, "({}, {}) unaffected by the hot pixel", x, y);
                } else {
                    assert!(!bit(on.mask[p], MASK_DQ_DROPPED));
                    assert_eq!(on.velocity[p].to_bits(), off.velocity[p].to_bits(), "velocity ({}, {})", x, y);
                    assert_eq!(on.flux[p].to_bits(), off.flux[p].to_bits(), "flux ({}, {})", x, y);
                    assert_eq!(on.sigma_obs[p].to_bits(), off.sigma_obs[p].to_bits(), "sigma ({}, {})", x, y);
                }
            }
        }
    }

    #[test]
    fn a_blank_continuum_window_falls_back_to_a_constant_per_spaxel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cut.fits");
        write_cube(&path, LINE_CUBE_SIZE, LINE_CUBE_SIZE, LINE_CUBE_DEPTH, &line_cube_cards(), |z, y, x| {
            if x < 16 && (34..=39).contains(&z) {
                f32::NAN
            } else {
                line_sci(z, y, x)
            }
        });
        let cube = open(&path);
        let inputs = only_sci(&cube);
        let maps = fit_cube_lines(&inputs, &line_cfg()).unwrap();
        let dv = channel_kms();
        let mut constant = 0usize;
        for y in 0..LINE_CUBE_SIZE {
            for x in 0..LINE_CUBE_SIZE {
                let p = [y, x];
                assert_ne!(maps.mask[p], 0.0, "({}, {}) not attempted", x, y);
                let const_bit = bit(maps.mask[p], MASK_CONST_CONTINUUM);
                assert_eq!(const_bit, x < 16, "mask({}, {})={}", x, y, maps.mask[p]);
                constant += const_bit as usize;
                if x < 16 && inside_disk(y, x) {
                    assert!(bit(maps.mask[p], MASK_FITTED), "mask({}, {})={}", x, y, maps.mask[p]);
                    assert!(maps.velocity[p].abs() < (0.1 * dv) as f32, "v({}, {})={}", x, y, maps.velocity[p]);
                }
            }
        }
        assert_eq!(constant, 16 * LINE_CUBE_SIZE);
        assert_eq!(maps.n_const_continuum, 512);
        assert!(
            maps.notes.iter().any(|n| n == "continuum: 512 spaxels fitted with a constant (one side window empty or too short)"),
            "{:?}",
            maps.notes
        );
        let cut = fit_spaxel(&inputs, &line_cfg(), 12, 16).unwrap().continuum.expect("continuum");
        assert!(!cut.linear);
        assert_eq!(cut.channels, 6);
        assert_eq!(cut.slope, 0.0);
        let whole = fit_spaxel(&inputs, &line_cfg(), 20, 16).unwrap().continuum.expect("continuum");
        assert!(whole.linear);
        assert_eq!(whole.channels, 12);
    }

    #[test]
    fn sigma_is_corrected_for_the_lsf_only_when_r_is_given_and_flagged_when_unresolved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.0);
        let cube = open(&path);
        let inputs = only_sci(&cube);
        let dv = channel_kms();
        let centre = [LINE_DISK_CENTRE as usize, LINE_DISK_CENTRE as usize];

        let resolved_cfg = LineFitConfig { resolving_power: Some(SPEED_OF_LIGHT_KMS / (FWHM_PER_SIGMA * 2.0 * dv)), ..line_cfg() };
        let maps = fit_cube_lines(&inputs, &resolved_cfg).unwrap();
        let corr = maps.sigma_corr.as_ref().expect("sigma_corr plane")[centre] as f64;
        let expected = ((3.0 * dv).powi(2) - (2.0 * dv).powi(2)).sqrt();
        assert!((corr - expected).abs() < 1e-3 * expected, "corr={} expected={}", corr, expected);
        assert!(!bit(maps.mask[centre], MASK_UNRESOLVED));
        assert!(maps.sigma_label.starts_with("LSF-corrected (R = "), "{}", maps.sigma_label);
        assert_eq!(plane_names(&resolved_cfg, &maps)[3], "sigma_corr");

        let unresolved_cfg = LineFitConfig { resolving_power: Some(SPEED_OF_LIGHT_KMS / (FWHM_PER_SIGMA * 4.0 * dv)), ..line_cfg() };
        let maps = fit_cube_lines(&inputs, &unresolved_cfg).unwrap();
        assert!(maps.sigma_corr.as_ref().unwrap()[centre].is_nan());
        assert!(bit(maps.mask[centre], MASK_UNRESOLVED), "mask={}", maps.mask[centre]);
        assert!(bit(maps.mask[centre], MASK_FITTED));
        let spaxel = fit_spaxel(&inputs, &unresolved_cfg, centre[1], centre[0]).unwrap();
        assert!(spaxel.single.unwrap().sigma_corr_kms.unwrap().is_nan());

        let maps = fit_cube_lines(&inputs, &line_cfg()).unwrap();
        assert!(maps.sigma_corr.is_none());
        assert_eq!(maps.sigma_label, SIGMA_OBSERVED_LABEL);
        assert!(!plane_names(&line_cfg(), &maps).contains(&"sigma_corr"));
        assert_eq!(plane_names(&line_cfg(), &maps).len(), 10);
        assert!(maps.notes.iter().any(|n| n.starts_with("sigma is observed")));
    }

    fn timing_sci(z: usize, y: usize, x: usize) -> f32 {
        let dx = x as f64 - 26.0;
        let dy = y as f64 - 27.0;
        let line = if dx * dx + dy * dy <= 400.0 { gaussian(z as f64, 250.0, 3.0) } else { 0.0 };
        (1.0 + line) as f32 + deterministic_noise(z, y, x, noise_amplitude(0.01))
    }

    fn wide_window_cfg() -> LineFitConfig {
        LineFitConfig { z0: 150, z1: 350, continuum: Some(((100, 140), (360, 400))), ..line_cfg() }
    }

    #[test]
    fn window_guards_refuse_short_and_long_windows_and_note_slow_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.0);
        let cube = open(&path);
        let short = LineFitConfig { z0: 20, z1: 23, ..line_cfg() };
        let err = fit_cube_lines(&only_sci(&cube), &short).unwrap_err().to_string();
        assert!(err.contains("at least 5"), "{}", err);

        let deep = dir.path().join("deep.fits");
        write_cube(&deep, 16, 16, 500, &line_cube_cards(), timing_sci);
        let deep_cube = open(&deep);
        let long = LineFitConfig { z0: 50, z1: 450, continuum: Some(((10, 40), (460, 490))), ..line_cfg() };
        let err = fit_cube_lines(&only_sci(&deep_cube), &long).unwrap_err().to_string();
        assert!(err.contains("limit is 400"), "{}", err);

        let maps = fit_cube_lines(&only_sci(&deep_cube), &wide_window_cfg()).unwrap();
        assert_eq!(maps.n_channels, 201);
        assert!(maps.notes.iter().any(|n| n.contains("expect a slower fit")), "{:?}", maps.notes);
        assert!(maps.notes.iter().any(|n| n == "line window has 201 channels: expect a slower fit"));
    }

    #[test]
    #[ignore]
    fn linefit_timing_201_channel_window_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timing.fits");
        write_cube(&path, 53, 55, 500, &line_cube_cards(), timing_sci);
        let cube = open(&path);
        let t0 = std::time::Instant::now();
        let maps = fit_cube_lines(&only_sci(&cube), &wide_window_cfg()).unwrap();
        let elapsed = t0.elapsed();
        println!(
            "201-channel window on 53x55x500: {:?} ({} fitted, {} masked, debug={})",
            elapsed,
            maps.n_fit,
            maps.n_masked,
            cfg!(debug_assertions)
        );
        assert!(maps.notes.iter().any(|n| n.contains("expect a slower fit")));
        assert!(maps.n_fit > 1000, "n_fit={}", maps.n_fit);
        let budget = if cfg!(debug_assertions) { 90.0 } else { 30.0 };
        assert!(elapsed.as_secs_f64() < budget, "{:?} exceeds {} s", elapsed, budget);
    }

    #[test]
    fn config_deserializes_with_the_documented_defaults_and_rejects_bad_values() {
        let cfg: LineFitConfig = serde_json::from_str(r#"{"z0":8,"z1":32}"#).unwrap();
        assert_eq!(cfg.z0, 8);
        assert_eq!(cfg.z1, 32);
        assert!(cfg.rest_um.is_none());
        assert_eq!(cfg.convention, VelocityConvention::Optical);
        assert!(cfg.continuum.is_none());
        assert_eq!(cfg.snr_threshold, DEFAULT_SNR_THRESHOLD);
        assert!(cfg.emission_only && cfg.use_err && cfg.use_dq);
        assert!(cfg.resolving_power.is_none());
        assert_eq!(cfg.components, Components::One);

        let cfg: LineFitConfig = serde_json::from_str(
            r#"{"z0":530,"z1":560,"rest_um":1.875613,"convention":"radio","continuum":[[500,520],[568,588]],"snr_threshold":5,"emission_only":false,"use_err":false,"use_dq":false,"resolving_power":2700,"components":"auto"}"#,
        )
        .unwrap();
        assert_eq!(cfg.convention, VelocityConvention::Radio);
        assert_eq!(cfg.continuum, Some(((500, 520), (568, 588))));
        assert_eq!(cfg.snr_threshold, 5.0);
        assert!(!cfg.emission_only && !cfg.use_err && !cfg.use_dq);
        assert_eq!(cfg.resolving_power, Some(2700.0));
        assert_eq!(cfg.components, Components::Auto);
        assert_eq!(cfg.rest_um, Some(1.875613));

        for bad in [
            r#"{"z0":1,"z1":9,"components":"three"}"#,
            r#"{"z0":1,"z1":9,"resolving_power":0}"#,
            r#"{"z0":1,"z1":9,"snr_threshold":-1}"#,
            r#"{"z0":1,"z1":9,"convention":"sideways"}"#,
        ] {
            assert!(serde_json::from_str::<LineFitConfig>(bad).is_err(), "{} accepted", bad);
        }
        assert_eq!(Components::parse("Two").unwrap(), Components::Two);
        assert_eq!(Components::Auto.name(), "auto");
    }

    #[test]
    fn velocity_and_flux_conversions_follow_the_shipped_formulas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.0);
        let cube = open(&path);
        let inputs = only_sci(&cube);
        let centre = LINE_DISK_CENTRE as usize;
        let optical = fit_spaxel(&inputs, &line_cfg(), centre, centre).unwrap().single.expect("single");
        let rest = LINE_REST_UM;
        assert_eq!(optical.velocity_kms, velocity_kms(optical.centre, rest, VelocityConvention::Optical));
        assert_eq!(
            optical.sigma_kms,
            (velocity_kms(optical.centre + optical.sigma, rest, VelocityConvention::Optical) - optical.velocity_kms).abs()
        );
        assert_eq!(optical.flux, optical.amplitude * optical.sigma_kms * (2.0 * std::f64::consts::PI).sqrt());
        assert!(optical.velocity_kms.abs() < 0.01 * channel_kms());

        let radio_cfg = LineFitConfig { convention: VelocityConvention::Radio, ..line_cfg() };
        let radio = fit_spaxel(&inputs, &radio_cfg, centre, centre).unwrap().single.expect("single");
        let expected_radio_sigma =
            (velocity_kms(radio.centre + radio.sigma, rest, VelocityConvention::Radio) - velocity_kms(radio.centre, rest, VelocityConvention::Radio)).abs();
        assert_eq!(radio.sigma_kms, expected_radio_sigma);
        let ratio = radio.sigma_kms / optical.sigma_kms;
        let expected_ratio = rest / (radio.centre + radio.sigma);
        assert!((ratio - expected_ratio).abs() < 1e-6, "ratio={} expected={}", ratio, expected_ratio);

        assert_eq!(error_scale(Weighting::Err, 4.0, 16), 1.0);
        assert_eq!(error_scale(Weighting::Err, 64.0, 16), 4.0);
        assert_eq!(error_scale(Weighting::Continuum, 4.0, 16), 0.25);
        assert_eq!(error_scale(Weighting::Continuum, 64.0, 16), 4.0);
        assert_eq!(error_scale(Weighting::Err, 1.0, 0), 1.0);

        let vrad = dir.path().join("vrad.fits");
        write_line_cube_with_cards(&vrad, &[("CTYPE3", "'VRAD'"), ("CUNIT3", "'km/s'"), ("RESTWAV", "")]);
        let vrad_cube = open(&vrad);
        let maps = fit_cube_lines(&only_sci(&vrad_cube), &line_cfg()).unwrap();
        assert_eq!(maps.rest_um, None);
        assert!(maps.notes.iter().any(|n| n.contains("rest wavelength") && n.contains("ignored")), "{:?}", maps.notes);
        let v = maps.velocity[[centre, centre]] as f64;
        assert!((v - (1.0 + 0.001 * 20.0)).abs() < 1e-4, "v={}", v);

        let no_rest = dir.path().join("no_rest.fits");
        write_line_cube_with_cards(&no_rest, &[("RESTWAV", "")]);
        let err = fit_cube_lines(&only_sci(&open(&no_rest)), &line_cfg()).unwrap_err().to_string();
        assert!(err.contains(REST_REQUIRED_MESSAGE), "{}", err);
        let err = fit_spaxel(&inputs, &line_cfg(), 99, 0).unwrap_err().to_string();
        assert!(err.contains("outside"), "{}", err);
    }

    #[test]
    fn zopt_axis_velocities_match_the_shipped_redshift_conversion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zopt.fits");
        write_line_cube_with_cards(
            &path,
            &[("CTYPE3", "'ZOPT'"), ("CUNIT3", ""), ("RESTWAV", ""), ("CRVAL3", "0.001"), ("CD3_3", "1.0E-5")],
        );
        let cube = open(&path);
        let inputs = only_sci(&cube);
        let centre = LINE_DISK_CENTRE as usize;
        let axis = cube.spectral_axis().unwrap();
        assert_eq!(axis.kind, AxisKind::Zopt);
        let shipped = velocity_axis(&axis, 1.0, VelocityConvention::Optical).unwrap().values_kms;
        let dz_kms = shipped[21] - shipped[20];
        let maps = fit_cube_lines(&inputs, &line_cfg()).unwrap();
        assert_eq!(maps.rest_um, None);
        assert!(maps.notes.iter().any(|n| n.contains("rest wavelength") && n.contains("ignored")), "{:?}", maps.notes);
        assert!(bit(maps.mask[[centre, centre]], MASK_FITTED), "mask={}", maps.mask[[centre, centre]]);
        let v = maps.velocity[[centre, centre]] as f64;
        assert!((v - shipped[20]).abs() < 0.01 * dz_kms, "v={} expected={}", v, shipped[20]);
        let sigma = maps.sigma_obs[[centre, centre]] as f64;
        assert!((sigma - 3.0 * dz_kms).abs() < 0.01 * dz_kms, "sigma={} expected={}", sigma, 3.0 * dz_kms);
        let flux = maps.flux[[centre, centre]] as f64;
        let expected_flux = 3.0 * dz_kms * sqrt_two_pi();
        assert!((flux - expected_flux).abs() < 1e-4 * expected_flux, "flux={} expected={}", flux, expected_flux);
        let with_rest = LineFitConfig { rest_um: Some(1.875613), ..line_cfg() };
        let single = fit_spaxel(&inputs, &with_rest, centre, centre).unwrap().single.expect("single");
        assert_eq!(single.velocity_kms, velocity_kms(1.0 + single.centre, 1.0, VelocityConvention::Optical));
        assert!((single.centre - 0.0012).abs() < 1e-7, "centre z={}", single.centre);
    }

    #[test]
    fn large_err_values_keep_the_linear_continuum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large_err.fits");
        write_linefit_mef(
            &path,
            LINE_CUBE_SIZE,
            LINE_CUBE_SIZE,
            LINE_CUBE_DEPTH,
            line_sci,
            Some(|_, _, _| 1.0e5f32),
            None::<fn(usize, usize, usize) -> i32>,
        );
        let sci = open_hdu(&path, 1);
        let err = open_hdu(&path, 2);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
        let maps = fit_cube_lines(&inputs, &line_cfg()).unwrap();
        assert_eq!(maps.weighting, Weighting::Err);
        assert_eq!(maps.n_const_continuum, 0, "{:?}", maps.notes);
        let centre = LINE_DISK_CENTRE as usize;
        assert!(!bit(maps.mask[[centre, centre]], MASK_CONST_CONTINUUM), "mask={}", maps.mask[[centre, centre]]);
        let continuum = fit_spaxel(&inputs, &line_cfg(), centre, centre).unwrap().continuum.expect("continuum");
        assert!(continuum.linear);
        assert_eq!(continuum.channels, 12);
    }

    #[test]
    fn fixture_helpers_describe_the_small_disk_and_the_wide_window() {
        assert_eq!(small_disk_pixel_count(), 81);
        assert!(inside_small_disk(8, 8) && inside_small_disk(8, 13) && !inside_small_disk(8, 14));
        assert_eq!(small_disk_pixel_count(), (0..SMALL_CUBE_SIZE * SMALL_CUBE_SIZE).filter(|i| inside_small_disk(i / 16, i % 16)).count());
        let wide = wide_cfg();
        assert_eq!((wide.z0, wide.z1), (12, 46));
        assert_eq!(wide.continuum, Some(((0, 9), (50, 59))));
        assert_eq!(wide.snr_threshold, line_cfg().snr_threshold);
        assert!(v_of_channel(20.0).abs() < 1e-9);
        assert!((v_of_channel(21.0) - channel_kms()).abs() < 1e-9);
        assert!((noise_amplitude(0.05) / 0.05 - 3f32.sqrt()).abs() < 1e-6);
        assert_eq!(gaussian(5.0, 5.0, 2.0), 1.0);
        assert!((gaussian(20.0, 20.0, 3.0) - line_profile(20) as f64).abs() < 1e-7);
    }

    const DOUBLE_PEAK_DEPTH: usize = 60;
    const DOUBLE_PEAK_AMPLITUDES: (f64, f64) = (0.8, 0.6);

    fn write_double_peak_mef(path: &std::path::Path, centres: (f64, f64), sigma: f64, noise: f32) {
        write_pair_mef(path, centres, sigma, noise, 1.0);
    }

    fn write_pair_mef(path: &std::path::Path, centres: (f64, f64), sigma: f64, noise: f32, sign: f64) {
        write_profile_mef(path, noise, noise, move |z| {
            sign * (DOUBLE_PEAK_AMPLITUDES.0 * gaussian(z, centres.0, sigma)
                + DOUBLE_PEAK_AMPLITUDES.1 * gaussian(z, centres.1, sigma))
        });
    }

    fn write_profile_mef(path: &std::path::Path, noise: f32, err: f32, profile: impl Fn(f64) -> f64) {
        write_linefit_mef(
            path,
            SMALL_CUBE_SIZE,
            SMALL_CUBE_SIZE,
            DOUBLE_PEAK_DEPTH,
            move |z, y, x| {
                let line = if inside_small_disk(y, x) { profile(z as f64) } else { 0.0 };
                (1.0 + line) as f32 + deterministic_noise(z, y, x, noise_amplitude(noise))
            },
            Some(move |_, _, _| err),
            None::<fn(usize, usize, usize) -> i32>,
        );
    }

    fn open_pair(path: &std::path::Path) -> (LazyCube, LazyCube) {
        (open_hdu(path, 1), open_hdu(path, 2))
    }

    fn pair_notes(spaxel: &SpaxelFit) -> Vec<&String> {
        spaxel.notes.iter().filter(|n| n.contains("two components")).collect()
    }

    fn small_disk_pixels() -> Vec<(usize, usize)> {
        (0..SMALL_CUBE_SIZE)
            .flat_map(|y| (0..SMALL_CUBE_SIZE).map(move |x| (x, y)))
            .filter(|&(x, y)| inside_small_disk(y, x))
            .collect()
    }

    #[test]
    fn a_double_peaked_spaxel_is_split_into_two_ordered_components() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("double.fits");
        write_double_peak_mef(&path, (24.0, 32.0), 2.5, 0.02);
        let sci = open_hdu(&path, 1);
        let err = open_hdu(&path, 2);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
        let cfg = LineFitConfig { components: Components::Auto, ..wide_cfg() };
        let maps = fit_cube_lines(&inputs, &cfg).unwrap();
        let planes = maps.components.as_ref().expect("component planes");
        let disk = small_disk_pixels();
        let (mut split, mut c1_ok, mut c2_ok, mut amp_ok, mut ordered) = (0usize, 0usize, 0usize, 0usize, 0usize);
        for &(x, y) in &disk {
            let p = [y, x];
            if maps.ncomp[p] != 2.0 {
                let d = fit_spaxel(&inputs, &cfg, x, y).unwrap();
                println!(
                    "double peak ({}, {}) not split: chi2_red {:.2} delta_bic {:?} {:?}",
                    x, y, d.chi2_red, d.delta_bic,
                    d.notes.iter().filter(|n| n.contains("two components")).collect::<Vec<_>>()
                );
                continue;
            }
            split += 1;
            assert!(bit(maps.mask[p], MASK_TWO_COMPONENTS), "mask({}, {})={}", x, y, maps.mask[p]);
            assert!(!bit(maps.mask[p], MASK_TWO_REJECTED), "mask({}, {})={}", x, y, maps.mask[p]);
            let (v1, v2) = (planes[0].velocity[p] as f64, planes[1].velocity[p] as f64);
            assert!(v1 < v2, "({}, {}) c1 {} c2 {}", x, y, v1, v2);
            if (v1 - v_of_channel(24.0)).abs() < 3.0 * planes[0].v_err[p] as f64 {
                c1_ok += 1;
            }
            if (v2 - v_of_channel(32.0)).abs() < 3.0 * planes[1].v_err[p] as f64 {
                c2_ok += 1;
            }
            let spaxel = fit_spaxel(&inputs, &cfg, x, y).unwrap();
            assert_eq!(spaxel.components.len(), 2);
            assert_eq!(spaxel.ncomp, 2);
            let (a1, a2) = (&spaxel.components[0], &spaxel.components[1]);
            assert_eq!(a1.velocity_kms as f32, planes[0].velocity[p]);
            if (a1.amplitude - DOUBLE_PEAK_AMPLITUDES.0).abs() < 3.0 * a1.amplitude_err
                && (a2.amplitude - DOUBLE_PEAK_AMPLITUDES.1).abs() < 3.0 * a2.amplitude_err
            {
                amp_ok += 1;
            }
            let v = maps.velocity[p] as f64;
            if v > v1 && v < v2 {
                ordered += 1;
            }
        }
        println!(
            "double peak: {} of {} disk spaxels split; c1 ok {}, c2 ok {}, amplitudes ok {}, ordered {} of {}; n_two_components {}",
            split, disk.len(), c1_ok, c2_ok, amp_ok, ordered, split, maps.n_two_components
        );
        let floor = 0.95 * disk.len() as f64;
        assert!(split as f64 >= floor, "split {} of {}", split, disk.len());
        assert!(c1_ok as f64 >= floor, "c1 within 3 sigma on {} of {}", c1_ok, disk.len());
        assert!(c2_ok as f64 >= floor, "c2 within 3 sigma on {} of {}", c2_ok, disk.len());
        assert!(amp_ok as f64 >= floor, "amplitudes within 3 sigma on {} of {}", amp_ok, disk.len());
        assert!(maps.n_two_components as f64 >= 0.95 * small_disk_pixel_count() as f64, "n_two_components {}", maps.n_two_components);
        assert_eq!(maps.n_two_components, split);
        assert!(ordered as f64 >= 0.97 * split as f64, "ordered {} of {}", ordered, split);
        for y in 0..SMALL_CUBE_SIZE {
            for x in 0..SMALL_CUBE_SIZE {
                if inside_small_disk(y, x) {
                    continue;
                }
                assert_eq!(maps.ncomp[[y, x]], 1.0, "ncomp({}, {})", x, y);
                assert!(planes[0].velocity[[y, x]].is_nan(), "c1_velocity({}, {})", x, y);
            }
        }
        let expected_note = format!(
            "two components: {} spaxels accepted (ΔBIC/max(1, chi2_red) ≥ 10, both components ≥ 3 sigma and ≥ 10 % of the flux, separated by ≥ 1 channel, each narrower than a quarter of the window)",
            split
        );
        assert!(maps.notes.contains(&expected_note), "{:?}", maps.notes);
        assert!(maps.notes.iter().any(|n| n == MULTI_PEAK_CAVEAT));
    }

    #[test]
    fn a_single_gaussian_spaxel_is_not_split_in_auto_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("single.fits");
        write_linefit_mef(
            &path,
            LINE_CUBE_SIZE,
            LINE_CUBE_SIZE,
            LINE_CUBE_DEPTH,
            |z, y, x| line_sci(z, y, x) + deterministic_noise(z, y, x, noise_amplitude(0.02)),
            Some(|_, _, _| 0.02f32),
            None::<fn(usize, usize, usize) -> i32>,
        );
        let sci = open_hdu(&path, 1);
        let err = open_hdu(&path, 2);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
        let cfg = LineFitConfig { components: Components::Auto, ..line_cfg() };
        let maps = fit_cube_lines(&inputs, &cfg).unwrap();
        let planes = maps.components.as_ref().expect("component planes");
        let mut kept_single = 0usize;
        for y in 0..LINE_CUBE_SIZE {
            for x in 0..LINE_CUBE_SIZE {
                if !inside_disk(y, x) {
                    continue;
                }
                let p = [y, x];
                let m = maps.mask[p];
                if maps.ncomp[p] == 1.0 && bit(m, MASK_TWO_REJECTED) && !bit(m, MASK_TWO_COMPONENTS) {
                    kept_single += 1;
                    assert!(planes[0].velocity[p].is_nan() && planes[1].velocity[p].is_nan(), "({}, {})", x, y);
                }
            }
        }
        println!("single Gaussian in auto mode: {} of {} disk spaxels kept single; n_two_components {}", kept_single, disk_pixel_count(), maps.n_two_components);
        assert!(kept_single as f64 >= 0.95 * disk_pixel_count() as f64, "{} of {}", kept_single, disk_pixel_count());
        assert!(maps.n_two_components as f64 <= 0.05 * disk_pixel_count() as f64, "n_two_components {}", maps.n_two_components);
    }

    #[test]
    fn forced_two_components_recover_a_blended_pair() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = LineFitConfig { components: Components::Two, ..wide_cfg() };
        let disk = small_disk_pixels();

        let blended = dir.path().join("blended.fits");
        write_double_peak_mef(&blended, (26.0, 29.0), 2.0, 0.01);
        let sci = open_hdu(&blended, 1);
        let err = open_hdu(&blended, 2);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
        let maps = fit_cube_lines(&inputs, &cfg).unwrap();
        let planes = maps.components.as_ref().expect("component planes");
        let mut split = 0usize;
        let mut near_truth = 0usize;
        let mut separations: Vec<f64> = Vec::new();
        let mut centre_stats: [(Vec<f64>, Vec<f64>); 2] = Default::default();
        for &(x, y) in &disk {
            let p = [y, x];
            if maps.ncomp[p] != 2.0 {
                let d = fit_spaxel(&inputs, &cfg, x, y).unwrap();
                println!(
                    "forced pair ({}, {}) not split: delta_bic {:?} {:?}",
                    x, y, d.delta_bic,
                    d.notes.iter().filter(|n| n.contains("two components")).collect::<Vec<_>>()
                );
                continue;
            }
            split += 1;
            assert!(planes[0].velocity[p] < planes[1].velocity[p], "({}, {})", x, y);
            let spaxel = fit_spaxel(&inputs, &cfg, x, y).unwrap();
            let separation = centre_channel(&spaxel.components[1]) - centre_channel(&spaxel.components[0]);
            separations.push(separation);
            for (c, centres) in spaxel.components.iter().zip(centre_stats.iter_mut()) {
                centres.0.push(centre_channel(c));
                centres.1.push(centre_err_channels(c));
            }
            assert!(separation >= 1.0, "({}, {}) separation {}", x, y, separation);
            if (separation - 3.0).abs() < 0.5 {
                near_truth += 1;
            }
        }
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let std = |v: &[f64]| {
            let m = mean(v);
            (v.iter().map(|a| (a - m) * (a - m)).sum::<f64>() / (v.len() as f64 - 1.0)).sqrt()
        };
        let mut sorted = separations.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        println!(
            "forced pair 26/29: {} of {} disk spaxels split; separation median {:.3} mean {:.3} std {:.3} ch, within 0.5 ch of 3.0 on {} of {}; c1 centre std {:.3} vs mean err {:.3}; c2 centre std {:.3} vs mean err {:.3}",
            split,
            disk.len(),
            sorted[sorted.len() / 2],
            mean(&separations),
            std(&separations),
            near_truth,
            split,
            std(&centre_stats[0].0),
            mean(&centre_stats[0].1),
            std(&centre_stats[1].0),
            mean(&centre_stats[1].1)
        );
        assert!(split as f64 >= 0.9 * disk.len() as f64, "split {} of {}", split, disk.len());
        assert!((sorted[sorted.len() / 2] - 3.0).abs() < 0.25, "median separation {}", sorted[sorted.len() / 2]);
        assert!(near_truth as f64 >= 0.9 * disk.len() as f64, "within 0.5 ch on {} of {} disk spaxels", near_truth, disk.len());
        assert!(maps.notes.iter().any(|n| n.starts_with(&format!("two components: {} spaxels accepted (forced;", split))), "{:?}", maps.notes);

        let unresolved = dir.path().join("unresolved.fits");
        write_double_peak_mef(&unresolved, (27.0, 27.5), 2.0, 0.01);
        let sci = open_hdu(&unresolved, 1);
        let err = open_hdu(&unresolved, 2);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
        let maps = fit_cube_lines(&inputs, &cfg).unwrap();
        let mut kept_single = 0usize;
        for &(x, y) in &disk {
            let p = [y, x];
            if maps.ncomp[p] == 1.0 && bit(maps.mask[p], MASK_TWO_REJECTED) {
                kept_single += 1;
                assert!(bit(maps.mask[p], MASK_FITTED), "mask({}, {})={}", x, y, maps.mask[p]);
                assert!(maps.velocity[p].is_finite(), "({}, {})", x, y);
            }
        }
        println!("forced pair 27.0/27.5: {} of {} disk spaxels kept single", kept_single, disk.len());
        assert!(kept_single as f64 >= 0.9 * disk.len() as f64, "{} of {}", kept_single, disk.len());
    }

    #[test]
    fn two_component_runs_keep_the_single_fit_in_the_main_planes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("double.fits");
        write_double_peak_mef(&path, (24.0, 32.0), 2.5, 0.02);
        let sci = open_hdu(&path, 1);
        let err = open_hdu(&path, 2);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
        let one_cfg = wide_cfg();
        let auto_cfg = LineFitConfig { components: Components::Auto, ..wide_cfg() };
        let one = fit_cube_lines(&inputs, &one_cfg).unwrap();
        let auto = fit_cube_lines(&inputs, &auto_cfg).unwrap();
        assert!(one.components.is_none());
        assert!(auto.components.is_some());
        assert_eq!(one.n_two_components, 0);
        assert_eq!(plane_names(&one_cfg, &one).len(), 10);
        assert_eq!(plane_names(&auto_cfg, &auto).len(), 22);
        assert_eq!(plane_names(&auto_cfg, &auto)[9], "ncomp");
        assert_eq!(plane_names(&auto_cfg, &auto)[10], "c1_flux");
        assert_eq!(plane_names(&auto_cfg, &auto)[21], "c2_sigma_err");
        let mut below_threshold = 0usize;
        for y in 0..SMALL_CUBE_SIZE {
            for x in 0..SMALL_CUBE_SIZE {
                let p = [y, x];
                assert_eq!(one.velocity[p].to_bits(), auto.velocity[p].to_bits(), "velocity ({}, {})", x, y);
                assert_eq!(one.flux[p].to_bits(), auto.flux[p].to_bits(), "flux ({}, {})", x, y);
                assert_eq!(one.sigma_obs[p].to_bits(), auto.sigma_obs[p].to_bits(), "sigma ({}, {})", x, y);
                assert!(!bit(one.mask[p], MASK_TWO_REJECTED) && !bit(one.mask[p], MASK_TWO_COMPONENTS));
                if one.mask[p] != 0.0 {
                    assert_eq!(one.ncomp[p], 1.0, "ncomp ({}, {})", x, y);
                }
                if one.ncomp[p] == 0.0 {
                    assert_eq!(one.mask[p], 0.0, "mask ({}, {})", x, y);
                }
                if one.mask[p] == 0.0 && one.ncomp[p] == 1.0 {
                    below_threshold += 1;
                }
                assert!(one.ncomp[p] == 0.0 || one.ncomp[p] == 1.0);
            }
        }
        println!("one-mode run: {} spaxels converged below threshold (mask 0, ncomp 1)", below_threshold);
    }

    #[test]
    fn fit_spaxel_matches_the_map_at_that_pixel_and_returns_the_model_curves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("double.fits");
        write_double_peak_mef(&path, (24.0, 32.0), 2.5, 0.02);
        let sci = open_hdu(&path, 1);
        let err = open_hdu(&path, 2);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
        let cfg = LineFitConfig { components: Components::Auto, ..wide_cfg() };
        let maps = fit_cube_lines(&inputs, &cfg).unwrap();
        let centre = SMALL_DISK_CENTRE as usize;
        let spaxel = fit_spaxel(&inputs, &cfg, centre, centre).unwrap();
        let single = spaxel.single.as_ref().expect("single fit");
        assert_eq!(single.velocity_kms as f32, maps.velocity[[centre, centre]]);
        assert_eq!(spaxel.components.len(), 2);
        assert_eq!(spaxel.ncomp, 2);
        assert!(spaxel.delta_bic.is_some());
        assert_eq!(spaxel.span, (0, DOUBLE_PEAK_DEPTH - 1));
        assert_eq!(spaxel.flux.len(), DOUBLE_PEAK_DEPTH);
        assert_eq!(spaxel.axis.len(), DOUBLE_PEAK_DEPTH);
        let model = spaxel.model.as_ref().expect("model");
        assert_eq!(model.channel.len(), 237);
        assert_eq!(model.channel[0], 0.0);
        assert_eq!(model.channel[236], 59.0);
        for k in 1..model.channel.len() {
            assert!((model.channel[k] - model.channel[k - 1] - 0.25).abs() < 1e-12);
        }
        assert_eq!(model.total.len(), model.channel.len());
        assert_eq!(model.continuum.len(), model.channel.len());
        assert_eq!(model.components.len(), 2);
        for k in 0..model.channel.len() {
            let expected = model.continuum[k] + model.components[0][k] + model.components[1][k];
            assert!((model.total[k] - expected).abs() < 1e-9, "k={} total={} expected={}", k, model.total[k], expected);
        }
        let err = fit_spaxel(&inputs, &cfg, 99, 0).unwrap_err().to_string();
        assert!(err.contains("outside"), "{}", err);
    }

    #[test]
    fn dq_values_beyond_f32_precision_are_treated_as_flagged() {
        assert!(!dq_flagged(0.0));
        assert!(dq_flagged(1.0));
        assert!(dq_flagged(513.0));
        assert!(!dq_flagged(512.0));
        assert!(dq_flagged(16_777_216.0));
        assert!(dq_flagged(2_147_483_648.0));
        assert!(dq_flagged(f32::NAN));
        assert!(dq_flagged(-1.0));
    }

    const ABSORPTION_DEPTH: f32 = 0.5;

    #[test]
    fn absorption_lines_are_mapped_when_emission_only_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absorption.fits");
        write_linefit_mef(
            &path,
            LINE_CUBE_SIZE,
            LINE_CUBE_SIZE,
            LINE_CUBE_DEPTH,
            |z, y, x| if inside_disk(y, x) { LINE_CONTINUUM - ABSORPTION_DEPTH * line_profile(z) } else { LINE_CONTINUUM },
            Some(|_, _, _| 0.01f32),
            None::<fn(usize, usize, usize) -> i32>,
        );
        let (sci, err) = open_pair(&path);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
        let cfg = LineFitConfig { emission_only: false, ..line_cfg() };
        let maps = fit_cube_lines(&inputs, &cfg).unwrap();
        let dv = channel_kms();
        let expected_flux = -(ABSORPTION_DEPTH as f64) * LINE_SIGMA_CHANNELS * dv * sqrt_two_pi();
        for y in 0..LINE_CUBE_SIZE {
            for x in 0..LINE_CUBE_SIZE {
                let p = [y, x];
                if inside_disk(y, x) {
                    assert!(bit(maps.mask[p], MASK_FITTED), "mask({}, {})={}", x, y, maps.mask[p]);
                    assert_eq!(maps.ncomp[p], 1.0);
                    assert!((maps.velocity[p] as f64).abs() < 0.01 * dv, "v({}, {})={}", x, y, maps.velocity[p]);
                    assert!((maps.sigma_obs[p] as f64 - LINE_SIGMA_CHANNELS * dv).abs() < 0.01 * dv, "sigma({}, {})={}", x, y, maps.sigma_obs[p]);
                    assert!((maps.flux[p] as f64 - expected_flux).abs() < 1e-4 * expected_flux.abs(), "flux({}, {})={}", x, y, maps.flux[p]);
                    assert!(maps.snr[p] <= -(DEFAULT_SNR_THRESHOLD as f32), "snr({}, {})={}", x, y, maps.snr[p]);
                } else if distance_from_disk_centre(y, x) > 8.0 {
                    assert!(!bit(maps.mask[p], MASK_FITTED), "mask({}, {})={}", x, y, maps.mask[p]);
                    assert!(maps.velocity[p].is_nan());
                }
            }
        }
        assert_eq!(maps.n_fit, disk_pixel_count());
        assert_eq!(maps.n_masked, 0);
        let centre = LINE_DISK_CENTRE as usize;
        let spaxel = fit_spaxel(&inputs, &cfg, centre, centre).unwrap();
        let single = spaxel.single.as_ref().expect("single fit");
        assert!((single.amplitude + ABSORPTION_DEPTH as f64).abs() < 1e-3, "amplitude {}", single.amplitude);
        assert!(single.flux < 0.0 && single.snr < 0.0, "flux {} snr {}", single.flux, single.snr);
        assert_eq!(spaxel.mask & MASK_FITTED, MASK_FITTED);
        let emission_only = fit_cube_lines(&inputs, &line_cfg()).unwrap();
        assert_eq!(emission_only.n_fit, 0);
        assert!(emission_only.velocity[[centre, centre]].is_nan());
    }

    #[test]
    fn an_absorption_pair_is_split_with_both_components_negative() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absorption_pair.fits");
        write_pair_mef(&path, (24.0, 32.0), 2.5, 0.02, -1.0);
        let (sci, err) = open_pair(&path);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
        let cfg = LineFitConfig { emission_only: false, components: Components::Auto, ..wide_cfg() };
        let maps = fit_cube_lines(&inputs, &cfg).unwrap();
        let planes = maps.components.as_ref().expect("component planes");
        let disk = small_disk_pixels();
        let (mut split, mut amp_ok) = (0usize, 0usize);
        for &(x, y) in &disk {
            let p = [y, x];
            assert!(bit(maps.mask[p], MASK_FITTED), "mask({}, {})={}", x, y, maps.mask[p]);
            assert!(maps.flux[p] < 0.0, "flux({}, {})={}", x, y, maps.flux[p]);
            if maps.ncomp[p] != 2.0 {
                let d = fit_spaxel(&inputs, &cfg, x, y).unwrap();
                println!("absorption pair ({}, {}) not split: delta_bic {:?} {:?}", x, y, d.delta_bic, pair_notes(&d));
                continue;
            }
            split += 1;
            assert!(planes[0].velocity[p] < planes[1].velocity[p], "({}, {})", x, y);
            assert!(planes[0].flux[p] < 0.0 && planes[1].flux[p] < 0.0, "({}, {}) c1 {} c2 {}", x, y, planes[0].flux[p], planes[1].flux[p]);
            let spaxel = fit_spaxel(&inputs, &cfg, x, y).unwrap();
            let (a1, a2) = (&spaxel.components[0], &spaxel.components[1]);
            if (a1.amplitude + DOUBLE_PEAK_AMPLITUDES.0).abs() < 3.0 * a1.amplitude_err
                && (a2.amplitude + DOUBLE_PEAK_AMPLITUDES.1).abs() < 3.0 * a2.amplitude_err
            {
                amp_ok += 1;
            }
        }
        println!("absorption pair: {} of {} disk spaxels split; amplitudes ok {}; n_two_components {}", split, disk.len(), amp_ok, maps.n_two_components);
        let floor = 0.95 * disk.len() as f64;
        assert!(split as f64 >= floor, "split {} of {}", split, disk.len());
        assert!(amp_ok as f64 >= floor, "amplitudes ok {} of {}", amp_ok, disk.len());
        assert_eq!(maps.n_two_components, split);
        assert_eq!(maps.n_fit, disk.len());

        let mixed = dir.path().join("mixed.fits");
        write_profile_mef(&mixed, 0.0, 0.02, |z| -0.8 * gaussian(z, 24.0, 2.5) + 0.5 * gaussian(z, 34.0, 2.5));
        let (sci, err) = open_pair(&mixed);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
        let centre = SMALL_DISK_CENTRE as usize;
        let forced = fit_spaxel(&inputs, &LineFitConfig { emission_only: false, components: Components::Two, ..wide_cfg() }, centre, centre).unwrap();
        println!("mixed profile, forced: ncomp {} delta_bic {:?} {:?}", forced.ncomp, forced.delta_bic, pair_notes(&forced));
        assert!(forced.single.as_ref().expect("single fit").amplitude < 0.0);
        assert!(forced.components.iter().all(|c| c.amplitude < 0.0 && c.flux < 0.0), "{:?}", forced.components);
        let auto = fit_spaxel(&inputs, &cfg, centre, centre).unwrap();
        println!("mixed profile, auto: ncomp {} chi2_red {:.1} delta_bic {:?} {:?}", auto.ncomp, auto.chi2_red, auto.delta_bic, pair_notes(&auto));
        assert_eq!(auto.ncomp, 1);
        assert_eq!(auto.mask & (MASK_FITTED | MASK_TWO_REJECTED), MASK_FITTED | MASK_TWO_REJECTED, "mask {}", auto.mask);
        assert!(auto.notes.iter().any(|n| n.contains("normalised ΔBIC")), "{:?}", auto.notes);
    }

    #[test]
    fn auto_rejection_and_flux_fraction_guards_have_the_documented_boundaries() {
        let strong = [10.0, 10.0];
        assert_eq!(auto_rejection(50.0, 10.0, strong).as_deref(), Some("normalised ΔBIC 5.0 below 10"));
        assert_eq!(auto_rejection(150.0, 10.0, strong), None);
        assert_eq!(auto_rejection(9.9, 0.5, strong).as_deref(), Some("normalised ΔBIC 9.9 below 10"));
        assert_eq!(auto_rejection(10.1, 0.5, strong), None);
        assert_eq!(auto_rejection(10.0, 1.0, strong), None);
        assert!(auto_rejection(f64::NAN, 1.0, strong).is_some());
        assert_eq!(auto_rejection(100.0, 1.0, [2.9, 10.0]).as_deref(), Some("component 1 amplitude below 3 sigma"));
        assert_eq!(auto_rejection(100.0, 1.0, [10.0, 2.9]).as_deref(), Some("component 2 amplitude below 3 sigma"));
        assert_eq!(auto_rejection(100.0, 1.0, [3.1, 3.1]), None);
        assert_eq!(auto_rejection(100.0, 1.0, [-3.1, -3.1]), None);
        assert_eq!(auto_rejection(100.0, 1.0, [-2.9, -3.1]).as_deref(), Some("component 1 amplitude below 3 sigma"));
        assert!(auto_rejection(100.0, 1.0, [f64::NAN, 10.0]).is_some());
        assert!(auto_rejection(5.0, 1.0, [2.9, 2.9]).unwrap().contains("ΔBIC"));

        assert_eq!(flux_fraction_rejection([0.1, 0.9]), None);
        assert_eq!(flux_fraction_rejection([0.09, 0.91]).as_deref(), Some("a component carries less than 10 % of the flux"));
        assert_eq!(flux_fraction_rejection([0.91, 0.09]).as_deref(), Some("a component carries less than 10 % of the flux"));
        assert_eq!(flux_fraction_rejection([-0.1, -0.9]), None);
        assert!(flux_fraction_rejection([0.0, 1.0]).is_some());
        assert!(flux_fraction_rejection([0.0, 0.0]).is_some());

        assert_eq!(sign_rejection([0.5, 0.5], 1.0), None);
        assert_eq!(sign_rejection([-0.5, -0.5], -1.0), None);
        assert_eq!(sign_rejection([0.5, -0.5], 1.0).as_deref(), Some("component 2 has the opposite sign to the single fit"));
        assert_eq!(sign_rejection([-0.5, 0.5], 1.0).as_deref(), Some("component 1 has the opposite sign to the single fit"));
        assert_eq!(sign_rejection([0.5, 0.5], -1.0).as_deref(), Some("component 1 has the opposite sign to the single fit"));
        assert_eq!(sign_rejection([-0.5, 0.5], -1.0).as_deref(), Some("component 2 has the opposite sign to the single fit"));
        assert_eq!(sign_rejection([0.0, 0.5], 1.0), None);
    }

    fn forced_pair_at_centre(dir: &tempfile::TempDir, name: &str, profile: impl Fn(f64) -> f64) -> SpaxelFit {
        let path = dir.path().join(format!("{}.fits", name));
        write_profile_mef(&path, 0.0, 0.01, profile);
        let (sci, err) = open_pair(&path);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
        let cfg = LineFitConfig { components: Components::Two, ..wide_cfg() };
        let centre = SMALL_DISK_CENTRE as usize;
        let spaxel = fit_spaxel(&inputs, &cfg, centre, centre).unwrap();
        println!(
            "{}: ncomp {} delta_bic {:?} components {:?} {:?}",
            name,
            spaxel.ncomp,
            spaxel.delta_bic,
            spaxel.components.iter().map(|c| (c.amplitude, centre_channel(c), sigma_channels(c))).collect::<Vec<_>>(),
            pair_notes(&spaxel)
        );
        spaxel
    }

    fn assert_pair_rejected(spaxel: &SpaxelFit, reason: &str) {
        assert_eq!(spaxel.ncomp, 1, "{:?}", spaxel.notes);
        let bits = MASK_FITTED | MASK_TWO_REJECTED | MASK_TWO_COMPONENTS;
        assert_eq!(spaxel.mask & bits, MASK_FITTED | MASK_TWO_REJECTED, "mask {}", spaxel.mask);
        assert!(spaxel.components.is_empty());
        assert!(spaxel.delta_bic.is_some(), "{:?}", spaxel.notes);
        assert!(
            spaxel.notes.iter().any(|n| n.starts_with("two components rejected: ") && n.contains(reason)),
            "{:?}",
            spaxel.notes
        );
    }

    #[test]
    fn a_broad_pedestal_is_rejected_by_the_width_guard() {
        let dir = tempfile::tempdir().unwrap();
        let spaxel = forced_pair_at_centre(&dir, "pedestal", |z| gaussian(z, 26.0, 2.0) + 0.4 * gaussian(z, 34.0, 14.0));
        assert_pair_rejected(&spaxel, "broader than a quarter of the window");
    }

    #[test]
    fn a_component_pinned_at_the_window_edge_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let spaxel = forced_pair_at_centre(&dir, "pinned", |z| gaussian(z, 26.0, 2.5) + 0.6 * gaussian(z, 47.5, 2.0));
        assert_pair_rejected(&spaxel, "centre pinned at a window bound");
    }

    #[test]
    fn a_sub_channel_feature_is_rejected_at_the_sigma_lower_bound() {
        let dir = tempfile::tempdir().unwrap();
        let spaxel = forced_pair_at_centre(&dir, "narrow", |z| gaussian(z, 26.0, 1.5) + gaussian(z, 36.0, 0.24));
        assert_pair_rejected(&spaxel, "sigma at the lower bound");
    }

    #[test]
    fn a_pair_closer_than_one_channel_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let spaxel = forced_pair_at_centre(&dir, "close", |z| gaussian(z, 27.0, 3.0) + gaussian(z, 27.8, 1.0));
        assert_pair_rejected(&spaxel, "centres separated by less than one channel");
    }

    #[test]
    fn a_component_below_ten_percent_of_the_flux_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let spaxel = forced_pair_at_centre(&dir, "faint", |z| gaussian(z, 26.0, 2.5) + 0.05 * gaussian(z, 36.0, 2.5));
        assert_pair_rejected(&spaxel, "a component carries less than 10 % of the flux");
    }

    #[test]
    fn the_pair_needs_eight_usable_samples() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.fits");
        write_linefit_mef(
            &path,
            LINE_CUBE_SIZE,
            LINE_CUBE_SIZE,
            LINE_CUBE_DEPTH,
            line_sci,
            Some(|_, _, _| 0.01f32),
            Some(|z, _, _| if z == 20 { 1 } else { 0 }),
        );
        let sci = open_hdu(&path, 1);
        let err = open_hdu(&path, 2);
        let dq = open_hdu(&path, 3);
        let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: Some(&dq) };
        let centre = LINE_DISK_CENTRE as usize;
        let not_attempted = "two components not attempted: 7 usable line samples, 8 needed";
        let seven = fit_spaxel(&inputs, &LineFitConfig { z0: 17, z1: 24, components: Components::Two, ..line_cfg() }, centre, centre).unwrap();
        assert_eq!(seven.channels.len(), 7);
        assert_eq!(seven.dropped_dq, vec![20]);
        assert_eq!(seven.mask & (MASK_FITTED | MASK_TWO_REJECTED), MASK_FITTED | MASK_TWO_REJECTED, "mask {}", seven.mask);
        assert_eq!(seven.ncomp, 1);
        assert_eq!(seven.delta_bic, None);
        assert!(seven.notes.iter().any(|n| n == not_attempted), "{:?}", seven.notes);
        let eight = fit_spaxel(&inputs, &LineFitConfig { z0: 17, z1: 25, components: Components::Two, ..line_cfg() }, centre, centre).unwrap();
        assert_eq!(eight.channels.len(), 8);
        assert!(!eight.notes.iter().any(|n| n.contains("not attempted")), "{:?}", eight.notes);
        assert!(eight.ncomp == 2 || eight.notes.iter().any(|n| n.starts_with("two components rejected: ")), "{:?}", eight.notes);
        let no_dq = fit_spaxel(&inputs, &LineFitConfig { z0: 17, z1: 23, use_dq: false, components: Components::Two, ..line_cfg() }, centre, centre).unwrap();
        assert_eq!(no_dq.channels.len(), 7);
        assert!(no_dq.dropped_dq.is_empty());
        assert!(no_dq.notes.iter().any(|n| n == not_attempted), "{:?}", no_dq.notes);
    }

    #[test]
    fn component_errors_are_inflated_by_chi2_red_when_err_is_underestimated() {
        let dir = tempfile::tempdir().unwrap();
        let truthful = dir.path().join("truthful.fits");
        write_pair_mef(&truthful, (24.0, 32.0), 2.5, 0.02, 1.0);
        let underestimated = dir.path().join("underestimated.fits");
        write_profile_mef(&underestimated, 0.02, 0.004, |z| {
            DOUBLE_PEAK_AMPLITUDES.0 * gaussian(z, 24.0, 2.5) + DOUBLE_PEAK_AMPLITUDES.1 * gaussian(z, 32.0, 2.5)
        });
        let cfg = LineFitConfig { components: Components::Auto, ..wide_cfg() };
        let centre = SMALL_DISK_CENTRE as usize;
        let fit_at = |path: &std::path::Path| {
            let (sci, err) = open_pair(path);
            let inputs = LineFitInputs { sci: &sci, err: Some(&err), dq: None };
            let maps = fit_cube_lines(&inputs, &cfg).unwrap();
            (maps.n_two_components, fit_spaxel(&inputs, &cfg, centre, centre).unwrap())
        };
        let (n_true, a) = fit_at(&truthful);
        let (n_under, b) = fit_at(&underestimated);
        assert_eq!(a.ncomp, 2, "{:?}", a.notes);
        assert_eq!(b.ncomp, 2, "{:?}", b.notes);
        let chi2_ratio = b.chi2_red / a.chi2_red;
        assert!((20.0..30.0).contains(&chi2_ratio), "chi2_red ratio {}", chi2_ratio);
        for (ca, cb) in a.components.iter().zip(&b.components) {
            let amplitude_ratio = cb.amplitude_err / ca.amplitude_err;
            let centre_ratio = cb.centre_err / ca.centre_err;
            println!(
                "inflation: amplitude_err truthful {:.4} underestimated {:.4} ratio {:.3}; centre_err ratio {:.3}; chi2_red ratio {:.2}",
                ca.amplitude_err, cb.amplitude_err, amplitude_ratio, centre_ratio, chi2_ratio
            );
            assert!((cb.amplitude - ca.amplitude).abs() < 1e-4, "{} vs {}", cb.amplitude, ca.amplitude);
            assert!((0.7..1.3).contains(&amplitude_ratio), "amplitude_err ratio {}", amplitude_ratio);
            assert!((0.7..1.3).contains(&centre_ratio), "centre_err ratio {}", centre_ratio);
        }
        println!("inflation: n_two_components truthful {} underestimated {}", n_true, n_under);
        assert!(n_under as f64 >= 0.95 * small_disk_pixel_count() as f64, "{} vs {}", n_under, n_true);
    }
}
