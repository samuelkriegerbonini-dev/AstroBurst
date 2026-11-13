use std::f64::consts::PI;

use anyhow::{bail, Result};

use crate::core::analysis::transit_model::{
    check_eclipse_shape, check_transit_shape, eclipse_flux, full_transit_duration_days, sky_separation, transit_duration_days,
    transit_flux, LimbDarkening, TransitShape, ECLIPSE_MODEL_NAME, MAX_RADIUS_RATIO, TRANSIT_MODEL_NAME,
};
use crate::math::gauss_newton::{fit_least_squares, solve_linear};

pub const MAX_TRANSIT_FIT_POINTS: usize = 200_000;
pub const TRANSIT_FIT_MAX_ITERATIONS: usize = 200;
pub const TRANSIT_PARAMETER_NAMES: [&str; 6] = ["t0", "rp", "a_rs", "b", "u1", "u2"];
pub const MIN_TIME_AVERAGING_BINS: usize = 4;
pub const TRANSIT_CURVE_POINTS: usize = 400;
pub const STRONG_CORRELATION: f64 = 0.95;
pub const BETA_WARNING: f64 = 1.5;
pub const GRAVITATIONAL_CONSTANT_SI: f64 = 6.6743e-11;

const SLOT_T0: usize = 0;
const SLOT_RP: usize = 1;
const SLOT_A_RS: usize = 2;
const SLOT_B: usize = 3;
const SLOT_U1: usize = 4;
const SLOT_U2: usize = 5;
const SLOT_FLUX_RATIO: usize = 4;
const ECLIPSE_PARAMETER_NAMES: [&str; 5] = ["t0", "rp", "a_rs", "b", "flux_ratio"];
const MAX_BASELINE_ORDER: u8 = 2;
const T0_STEP_PER_DURATION: f64 = 1e-4;
const DERIVATIVE_STEP: f64 = 1e-5;
const FLUX_RATIO_STEP: f64 = 1e-6;
const TOTAL_OCCULTATION_MARGIN: f64 = 1e-6;
const POOR_REDUCED_CHI2: f64 = 3.0;
const GEOMETRY_NAMES: [&str; 3] = ["rp", "a_rs", "b"];
const GEOMETRY_STEPS: [f64; 3] = [1e-6, 1e-5, 1e-6];
const GEOMETRY_RP: usize = 0;
const GEOMETRY_A_RS: usize = 1;
const GEOMETRY_B: usize = 2;
const SECONDS_PER_DAY: f64 = 86400.0;
const G_PER_CM3_PER_KG_PER_M3: f64 = 1e-3;

#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct ParameterInput {
    pub value: f64,
    pub free: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransitKind {
    #[default]
    Transit,
    Eclipse,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
pub struct TransitFitRequest {
    pub time: Vec<f64>,
    pub flux: Vec<f64>,
    #[serde(default)]
    pub err: Option<Vec<f64>>,
    pub period: f64,
    pub t0: ParameterInput,
    pub rp: ParameterInput,
    pub a_rs: ParameterInput,
    pub b: ParameterInput,
    pub u1: ParameterInput,
    pub u2: ParameterInput,
    #[serde(default)]
    pub baseline_order: u8,
    #[serde(default)]
    pub scale_errors: bool,
    #[serde(default)]
    pub kind: TransitKind,
    #[serde(default)]
    pub flux_ratio: Option<ParameterInput>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FittedParameter {
    pub name: String,
    pub value: f64,
    pub err: Option<f64>,
    pub free: bool,
    pub at_bound: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FittedBaseline {
    pub order: u8,
    pub reference_time: f64,
    pub coefficients: Vec<f64>,
    pub errors: Vec<Option<f64>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TransitFit {
    pub model: String,
    pub period: f64,
    pub parameters: Vec<FittedParameter>,
    pub baseline: FittedBaseline,
    pub depth: f64,
    pub depth_err: Option<f64>,
    pub chi2: f64,
    pub dof: usize,
    pub reduced_chi2: f64,
    pub weighted: bool,
    pub error_scale: f64,
    pub rms: f64,
    pub rms_ppm: f64,
    pub n_points: usize,
    pub n_in_transit: usize,
    pub iterations: usize,
    pub converged: bool,
    pub free_names: Vec<String>,
    #[serde(skip)]
    pub covariance: Option<Vec<Vec<f64>>>,
    pub time: Vec<f64>,
    pub model_flux: Vec<f64>,
    pub baseline_flux: Vec<f64>,
    pub residual: Vec<f64>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
    pub correlation: Option<Vec<Vec<f64>>>,
    pub derived: DerivedQuantities,
    pub time_averaging: TimeAveraging,
    pub curve_time: Vec<f64>,
    pub curve_flux: Vec<f64>,
    pub curve_baseline: Vec<f64>,
    pub kind: TransitKind,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TimeAveraging {
    pub cadence_days: f64,
    pub bin_points: Vec<usize>,
    pub n_bins: Vec<usize>,
    pub rms_ppm: Vec<f64>,
    pub expected_ppm: Vec<f64>,
    pub beta: f64,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct DerivedQuantities {
    pub inclination_deg: Option<f64>,
    pub inclination_err: Option<f64>,
    pub duration_t14_days: Option<f64>,
    pub duration_t14_err: Option<f64>,
    pub duration_t23_days: Option<f64>,
    pub duration_t23_err: Option<f64>,
    pub stellar_density_cgs: Option<f64>,
    pub stellar_density_err: Option<f64>,
}

fn count_and_first(n: usize, unusable: impl Fn(usize) -> bool) -> Option<(usize, usize)> {
    let first = (0..n).find(|&i| unusable(i))?;
    Some(((first..n).filter(|&i| unusable(i)).count(), first))
}

fn check_series(request: &TransitFitRequest) -> Result<()> {
    let n = request.time.len();
    if request.flux.len() != n {
        bail!("time has {n} values and flux has {}: they must have the same length", request.flux.len());
    }
    if let Some(err) = &request.err {
        if err.len() != n {
            bail!("err has {} values for {n} points", err.len());
        }
    }
    if n == 0 {
        bail!("no point to fit: time and flux are empty");
    }
    if n > MAX_TRANSIT_FIT_POINTS {
        bail!("{n} points exceed the limit of {MAX_TRANSIT_FIT_POINTS}");
    }
    if request.baseline_order > MAX_BASELINE_ORDER {
        bail!("baseline order {} must be 0, 1 or 2", request.baseline_order);
    }
    if request.kind == TransitKind::Eclipse && request.flux_ratio.is_none() {
        bail!("an eclipse fit needs flux_ratio (planet flux over star flux)");
    }
    if let Some((count, first)) = count_and_first(n, |i| !request.time[i].is_finite() || !request.flux[i].is_finite()) {
        bail!("time or flux is not a finite number at {count} of {n} points (first at index {first}): leave those points out before the fit");
    }
    if let Some(err) = &request.err {
        if let Some((count, first)) = count_and_first(n, |i| !(err[i].is_finite() && err[i] > 0.0)) {
            bail!("err is not finite and above 0 at {count} of {n} points (first at index {first}): pass no errors or leave those points out");
        }
    }
    Ok(())
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        0.5 * (sorted[middle - 1] + sorted[middle])
    } else {
        sorted[middle]
    }
}

fn typed_parameters(request: &TransitFitRequest) -> Vec<ParameterInput> {
    let geometry = [request.t0, request.rp, request.a_rs, request.b].into_iter();
    match request.kind {
        TransitKind::Transit => geometry.chain([request.u1, request.u2]).collect(),
        TransitKind::Eclipse => geometry.chain(request.flux_ratio).collect(),
    }
}

fn orbit_of(period: f64, values: &[f64; 6]) -> TransitShape {
    TransitShape { t0: values[SLOT_T0], period, rp: values[SLOT_RP], a_rs: values[SLOT_A_RS], b: values[SLOT_B] }
}

fn shape_flux(kind: TransitKind, x: f64, period: f64, values: &[f64; 6]) -> f64 {
    let shape = orbit_of(period, values);
    match kind {
        TransitKind::Transit => transit_flux(x, &shape, &LimbDarkening { u1: values[SLOT_U1], u2: values[SLOT_U2] }),
        TransitKind::Eclipse => eclipse_flux(x, &shape, values[SLOT_FLUX_RATIO]),
    }
}

fn discs_overlap(x: f64, period: f64, values: &[f64; 6]) -> bool {
    let (z, near) = sky_separation(x, &orbit_of(period, values));
    near && z < 1.0 + values[SLOT_RP]
}

fn baseline_at(coefficients: &[f64], x: f64) -> f64 {
    let mut power = 1.0;
    let mut value = 0.0;
    for coefficient in coefficients {
        value += coefficient * power;
        power *= x;
    }
    value
}

fn root_mean_square(values: impl Iterator<Item = f64>, count: usize) -> f64 {
    (values.map(|value| value * value).sum::<f64>() / count as f64).sqrt()
}

fn scaled_covariance(normal: &[Vec<f64>], error_scale: f64, units: &[f64]) -> Option<Vec<Vec<f64>>> {
    let size = normal.len();
    let mut covariance = vec![vec![0.0; size]; size];
    for col in 0..size {
        let mut work = normal.to_vec();
        let mut unit = vec![0.0; size];
        unit[col] = 1.0;
        let column = solve_linear(&mut work, &mut unit)?;
        for row in 0..size {
            covariance[row][col] = column[row] * error_scale * error_scale * units[row] * units[col];
        }
    }
    (0..size).all(|index| covariance[index][index].is_finite() && covariance[index][index] >= 0.0).then_some(covariance)
}

fn limb_darkening_note(u1: ParameterInput, u2: ParameterInput) -> String {
    match (u1.free, u2.free) {
        (false, false) => {
            format!("limb darkening fixed as typed: u1 = {}, u2 = {} (not taken from a stellar model)", u1.value, u2.value)
        }
        (true, true) => format!("limb darkening fitted: u1 and u2 free, started at {} and {}", u1.value, u2.value),
        (true, false) => format!("limb darkening: u1 fitted, u2 fixed at {}", u2.value),
        (false, true) => format!("limb darkening: u1 fixed at {}, u2 fitted", u1.value),
    }
}

pub fn time_averaging(time: &[f64], relative_residual: &[f64]) -> TimeAveraging {
    let n = time.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&earlier, &later| time[earlier].total_cmp(&time[later]));
    let steps: Vec<f64> = order.windows(2).map(|pair| time[pair[1]] - time[pair[0]]).collect();
    let in_time_order: Vec<f64> = order.iter().map(|&index| relative_residual[index]).collect();
    let single_point_rms = root_mean_square(in_time_order.iter().copied(), n);
    let mut averaging = TimeAveraging {
        cadence_days: if steps.is_empty() { 0.0 } else { median(&steps) },
        bin_points: Vec::new(),
        n_bins: Vec::new(),
        rms_ppm: Vec::new(),
        expected_ppm: Vec::new(),
        beta: 1.0,
    };
    let mut size = 1;
    while n / size >= MIN_TIME_AVERAGING_BINS {
        let bins = n / size;
        let bin_means = in_time_order.chunks_exact(size).map(|bin| bin.iter().sum::<f64>() / size as f64);
        let rms = root_mean_square(bin_means, bins);
        let expected = single_point_rms / (size as f64).sqrt() * (bins as f64 / (bins - 1) as f64).sqrt();
        if size > 1 {
            averaging.beta = averaging.beta.max(rms / expected);
        }
        averaging.bin_points.push(size);
        averaging.n_bins.push(bins);
        averaging.rms_ppm.push(1e6 * rms);
        averaging.expected_ppm.push(1e6 * expected);
        size *= 2;
    }
    averaging
}

fn propagated_error(
    quantity: impl Fn(&[f64; 3]) -> Option<f64>,
    depends_on: &[usize],
    geometry: &[f64; 3],
    columns: &[Option<usize>; 3],
    covariance: Option<&[Vec<f64>]>,
) -> Option<f64> {
    let covariance = covariance?;
    quantity(geometry)?;
    if depends_on.iter().all(|&slot| columns[slot].is_none()) {
        return None;
    }
    let mut gradient = Vec::new();
    for (slot, column) in columns.iter().enumerate() {
        let Some(column) = *column else { continue };
        let (mut raised, mut lowered) = (*geometry, *geometry);
        raised[slot] += GEOMETRY_STEPS[slot];
        lowered[slot] -= GEOMETRY_STEPS[slot];
        gradient.push((column, (quantity(&raised)? - quantity(&lowered)?) / (2.0 * GEOMETRY_STEPS[slot])));
    }
    let mut variance = 0.0;
    for &(row, along_row) in &gradient {
        for &(col, along_col) in &gradient {
            variance += along_row * covariance[row][col] * along_col;
        }
    }
    Some(variance.sqrt())
}

pub fn derived_quantities(
    period: f64,
    rp: f64,
    a_rs: f64,
    b: f64,
    free_names: &[String],
    covariance: Option<&[Vec<f64>]>,
) -> DerivedQuantities {
    let geometry = [rp, a_rs, b];
    let columns = GEOMETRY_NAMES.map(|name| free_names.iter().position(|free| free == name));
    let shape_of = |geometry: &[f64; 3]| TransitShape {
        t0: 0.0,
        period,
        rp: geometry[GEOMETRY_RP],
        a_rs: geometry[GEOMETRY_A_RS],
        b: geometry[GEOMETRY_B],
    };
    let inclination = |geometry: &[f64; 3]| {
        let (a_rs, b) = (geometry[GEOMETRY_A_RS], geometry[GEOMETRY_B]);
        (b <= a_rs).then(|| (b / a_rs).acos().to_degrees())
    };
    let outer_duration = |geometry: &[f64; 3]| transit_duration_days(&shape_of(geometry));
    let inner_duration = |geometry: &[f64; 3]| full_transit_duration_days(&shape_of(geometry));
    let density = |geometry: &[f64; 3]| {
        let seconds = period * SECONDS_PER_DAY;
        Some(
            3.0 * PI * geometry[GEOMETRY_A_RS].powi(3) / (GRAVITATIONAL_CONSTANT_SI * seconds * seconds)
                * G_PER_CM3_PER_KG_PER_M3,
        )
    };
    let shape_and_orbit = [GEOMETRY_RP, GEOMETRY_A_RS, GEOMETRY_B];
    DerivedQuantities {
        inclination_deg: inclination(&geometry),
        inclination_err: propagated_error(inclination, &[GEOMETRY_A_RS, GEOMETRY_B], &geometry, &columns, covariance),
        duration_t14_days: outer_duration(&geometry),
        duration_t14_err: propagated_error(outer_duration, &shape_and_orbit, &geometry, &columns, covariance),
        duration_t23_days: inner_duration(&geometry),
        duration_t23_err: propagated_error(inner_duration, &shape_and_orbit, &geometry, &columns, covariance),
        stellar_density_cgs: density(&geometry),
        stellar_density_err: propagated_error(density, &[GEOMETRY_A_RS], &geometry, &columns, covariance),
    }
}

fn correlation_of(covariance: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let sigma: Vec<f64> = (0..covariance.len()).map(|index| covariance[index][index].sqrt()).collect();
    covariance
        .iter()
        .zip(&sigma)
        .map(|(row, sigma_row)| row.iter().zip(&sigma).map(|(entry, sigma_col)| entry / (sigma_row * sigma_col)).collect())
        .collect()
}

pub fn fit_transit(request: &TransitFitRequest) -> Result<TransitFit> {
    check_series(request)?;
    let n = request.time.len();
    let t_min = request.time.iter().copied().fold(f64::INFINITY, f64::min);
    let t_max = request.time.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if t_min == t_max {
        bail!("every point has the same time: nothing can be fitted against time");
    }
    let period = request.period;
    let kind = request.kind;
    let (names, event): (&[&str], &str) = match kind {
        TransitKind::Transit => (&TRANSIT_PARAMETER_NAMES, "transit"),
        TransitKind::Eclipse => (&ECLIPSE_PARAMETER_NAMES, "eclipse"),
    };
    let typed = typed_parameters(request);
    let typed_shape = TransitShape { t0: request.t0.value, period, rp: request.rp.value, a_rs: request.a_rs.value, b: request.b.value };
    match kind {
        TransitKind::Transit => check_transit_shape(&typed_shape, &LimbDarkening { u1: request.u1.value, u2: request.u2.value }),
        TransitKind::Eclipse => check_eclipse_shape(&typed_shape, typed[SLOT_FLUX_RATIO].value),
    }
    .map_err(anyhow::Error::msg)?;
    let scale = median(&request.flux);
    if scale <= 0.0 {
        bail!("the median flux {scale} is not above 0: the fit needs fluxes on a positive scale");
    }
    let free_slots: Vec<usize> = (0..typed.len()).filter(|&slot| typed[slot].free).collect();
    let n_shape = free_slots.len();
    let n_coefficients = usize::from(request.baseline_order) + 1;
    let n_free = n_shape + n_coefficients;
    if n <= n_free {
        bail!("{n} points for {n_free} free parameters: the fit needs more points than free parameters");
    }

    let reference_time = 0.5 * (t_min + t_max);
    let span = t_max - t_min;
    let epoch = ((request.t0.value - reference_time) / period).round();
    let tau0 = request.t0.value - reference_time - epoch * period;
    let mut start = [0.0; 6];
    for (slot, input) in typed.iter().enumerate() {
        start[slot] = input.value;
    }
    start[SLOT_T0] = tau0;
    let Some(duration) = transit_duration_days(&typed_shape) else {
        bail!("the starting values give no {event} duration: check a_rs and b");
    };
    let x: Vec<f64> = request.time.iter().map(|time| time - reference_time).collect();
    if !x.iter().any(|&x| discs_overlap(x, period, &start)) {
        bail!(
            "no point of the series lies inside the {event} of the starting values (t0 {:.5}, duration {duration:.5} d): check t0 and the period",
            reference_time + tau0
        );
    }
    let start_flux: Vec<f64> = x.iter().map(|&x| shape_flux(kind, x, period, &start)).collect();
    let y: Vec<f64> = request.flux.iter().map(|flux| flux / scale).collect();
    let weights: Vec<f64> = match &request.err {
        Some(err) => err.iter().map(|sigma| (scale / sigma).powi(2)).collect(),
        None => vec![1.0; n],
    };
    let outside: Vec<f64> = (0..n).filter(|&i| start_flux[i] == 1.0).map(|i| y[i]).collect();
    let start_level = if outside.is_empty() { 1.0 } else { median(&outside) };

    let mut steps =
        vec![T0_STEP_PER_DURATION * duration, DERIVATIVE_STEP * start[SLOT_RP], DERIVATIVE_STEP * start[SLOT_A_RS], DERIVATIVE_STEP];
    let mut shape_lower = vec![t_min - reference_time - span / 2.0, 1e-5, 1.000001, 0.0];
    let mut shape_upper = vec![t_max - reference_time + span / 2.0, MAX_RADIUS_RATIO, 1e6, 1.0 + MAX_RADIUS_RATIO];
    match kind {
        TransitKind::Transit => {
            steps.extend([DERIVATIVE_STEP, DERIVATIVE_STEP]);
            shape_lower.extend([0.0, -1.0]);
            shape_upper.extend([2.0, 1.0]);
        }
        TransitKind::Eclipse => {
            steps.push(FLUX_RATIO_STEP);
            shape_lower.push(-0.5);
            shape_upper.push(10.0);
        }
    }
    let vector_of = |shape: &[f64], baseline: [f64; 3]| -> Vec<f64> {
        free_slots.iter().map(|&slot| shape[slot]).chain(baseline.into_iter().take(n_coefficients)).collect()
    };
    let p0 = vector_of(&start, [start_level, 0.0, 0.0]);
    let lower = vector_of(&shape_lower, [0.0, f64::NEG_INFINITY, f64::NEG_INFINITY]);
    let upper = vector_of(&shape_upper, [f64::INFINITY; 3]);

    let shape_values = |params: &[f64]| -> [f64; 6] {
        let mut values = start;
        for (index, &slot) in free_slots.iter().enumerate() {
            values[slot] = params[index];
        }
        values
    };
    let model = |x: f64, params: &[f64], jac: &mut [f64]| -> f64 {
        let values = shape_values(params);
        if kind == TransitKind::Transit && values[SLOT_RP] >= 1.0 + values[SLOT_B] {
            return f64::NAN;
        }
        let shape = shape_flux(kind, x, period, &values);
        let base = baseline_at(&params[n_shape..], x);
        let mut derivative = shape;
        for entry in &mut jac[n_shape..] {
            *entry = derivative;
            derivative *= x;
        }
        for (index, &slot) in free_slots.iter().enumerate() {
            let (mut raised, mut lowered) = (values, values);
            raised[slot] += steps[slot];
            lowered[slot] -= steps[slot];
            jac[index] =
                base * (shape_flux(kind, x, period, &raised) - shape_flux(kind, x, period, &lowered)) / (2.0 * steps[slot]);
        }
        base * shape
    };
    let first = fit_least_squares(&x, &y, &weights, &p0, &lower, &upper, TRANSIT_FIT_MAX_ITERATIONS, &model)
        .map_err(anyhow::Error::msg)?;
    let fit = fit_least_squares(&x, &y, &weights, &first.params, &lower, &upper, TRANSIT_FIT_MAX_ITERATIONS, &model)
        .map_err(anyhow::Error::msg)?;
    let iterations = first.iterations + fit.iterations;

    let mut normal = vec![vec![0.0; n_free]; n_free];
    let mut row = vec![0.0; n_free];
    for i in 0..n {
        model(x[i], &fit.params, &mut row);
        for a in 0..n_free {
            for b in a..n_free {
                normal[a][b] += weights[i] * row[a] * row[b];
            }
        }
    }
    for a in 0..n_free {
        for b in 0..a {
            normal[a][b] = normal[b][a];
        }
    }
    let weighted = request.err.is_some();
    let reduced_chi2 = fit.chi2 / fit.dof as f64;
    let error_scale = if weighted && !request.scale_errors { 1.0 } else { reduced_chi2.sqrt() };
    let units: Vec<f64> = (0..n_free).map(|index| if index < n_shape { 1.0 } else { scale }).collect();
    let covariance = scaled_covariance(&normal, error_scale, &units);
    let error_at = |index: usize| covariance.as_ref().map(|covariance| covariance[index][index].sqrt());
    let correlation = covariance.as_deref().map(correlation_of);
    let free_names: Vec<String> = free_slots
        .iter()
        .map(|&slot| names[slot].to_string())
        .chain((0..n_coefficients).map(|k| format!("c{k}")))
        .collect();

    let best = shape_values(&fit.params);
    let t0 = if request.t0.free { reference_time + best[SLOT_T0] } else { request.t0.value - epoch * period };
    let parameters: Vec<FittedParameter> = (0..typed.len())
        .map(|slot| {
            let position = free_slots.iter().position(|&free_slot| free_slot == slot);
            FittedParameter {
                name: names[slot].to_string(),
                value: if slot == SLOT_T0 { t0 } else { best[slot] },
                err: position.and_then(error_at),
                free: position.is_some(),
                at_bound: position.is_some_and(|index| fit.params[index] == lower[index] || fit.params[index] == upper[index]),
            }
        })
        .collect();
    let coefficients: Vec<f64> = fit.params[n_shape..].iter().map(|coefficient| coefficient * scale).collect();
    let rp = best[SLOT_RP];
    let (depth, depth_err) = match kind {
        TransitKind::Transit => (rp * rp, parameters[SLOT_RP].err.map(|err| 2.0 * rp * err)),
        TransitKind::Eclipse => {
            let star_and_planet = 1.0 + best[SLOT_FLUX_RATIO];
            (
                best[SLOT_FLUX_RATIO] / star_and_planet,
                parameters[SLOT_FLUX_RATIO].err.map(|err| err / (star_and_planet * star_and_planet)),
            )
        }
    };

    let shape_at_best: Vec<f64> = x.iter().map(|&x| shape_flux(kind, x, period, &best)).collect();
    let baseline_flux: Vec<f64> = x.iter().map(|&x| baseline_at(&coefficients, x)).collect();
    let model_flux: Vec<f64> = baseline_flux.iter().zip(&shape_at_best).map(|(base, shape)| base * shape).collect();
    let residual: Vec<f64> = request.flux.iter().zip(&model_flux).map(|(flux, model)| flux - model).collect();
    let n_in_transit = x.iter().filter(|&&x| discs_overlap(x, period, &best)).count();
    let relative_residual: Vec<f64> = residual.iter().zip(&baseline_flux).map(|(residual, base)| residual / base).collect();
    let averaging = time_averaging(&request.time, &relative_residual);
    let beta = averaging.beta;
    let curve_time: Vec<f64> = (0..TRANSIT_CURVE_POINTS)
        .map(|point| t_min + span * point as f64 / (TRANSIT_CURVE_POINTS - 1) as f64)
        .collect();
    let curve_baseline: Vec<f64> = curve_time.iter().map(|time| baseline_at(&coefficients, time - reference_time)).collect();
    let curve_flux: Vec<f64> = curve_time
        .iter()
        .zip(&curve_baseline)
        .map(|(time, base)| base * shape_flux(kind, time - reference_time, period, &best))
        .collect();

    let mut notes = vec![
        match kind {
            TransitKind::Transit => format!("model: {TRANSIT_MODEL_NAME}; flux = baseline polynomial x transit"),
            TransitKind::Eclipse => format!(
                "model: {ECLIPSE_MODEL_NAME}; flux = baseline polynomial x (1 - depth x hidden fraction of the planet), depth = flux_ratio / (1 + flux_ratio)"
            ),
        },
        format!("period {period} d given, not fitted"),
    ];
    if epoch != 0.0 {
        notes.push(format!("t0 moved by {epoch} periods from the typed value to the {event} nearest the middle of the series"));
    }
    notes.push(match kind {
        TransitKind::Transit => limb_darkening_note(request.u1, request.u2),
        TransitKind::Eclipse => {
            "t0 is the mid-eclipse time; on a circular orbit it is the mid-transit time plus half a period".to_string()
        }
    });
    notes.push(format!(
        "baseline: polynomial of order {} in time about {reference_time:.5} (the middle of the series), fitted with the {event}",
        request.baseline_order
    ));
    let listed_when_fixed = match kind {
        TransitKind::Transit => &parameters[..=SLOT_B],
        TransitKind::Eclipse => &parameters[..],
    };
    let fixed: Vec<String> = listed_when_fixed
        .iter()
        .filter(|parameter| !parameter.free)
        .map(|parameter| format!("{} = {}", parameter.name, parameter.value))
        .collect();
    if !fixed.is_empty() {
        notes.push(format!("fixed as given: {}", fixed.join(", ")));
    }
    notes.push(if weighted { "weights 1 / err^2" } else { "equal weights: the points carry no errors" }.to_string());
    notes.push(if !weighted {
        "errors: 1 sigma from the covariance of the fit, scaled by the scatter of the residuals (sqrt(chi2 / dof))".to_string()
    } else if request.scale_errors {
        format!("errors: 1 sigma from the covariance of the fit, multiplied by sqrt(reduced chi2) = {error_scale:.4} (asked)")
    } else {
        format!("errors: 1 sigma from the covariance of the fit, not scaled (reduced chi2 = {reduced_chi2:.3})")
    });
    notes.push(
        match kind {
            TransitKind::Transit => "not modelled: orbital eccentricity, the finite exposure time (the model is evaluated at the given mid-exposure times), limb darkening from stellar models, instrument systematics and correlated noise beyond the baseline polynomial; no MCMC, the errors are the linear estimate of least squares",
            TransitKind::Eclipse => "not modelled: orbital eccentricity (it shifts and stretches the eclipse), the finite exposure time (the model is evaluated at the given mid-exposure times), the phase curve of the planet, limb darkening of the planet, instrument systematics and correlated noise beyond the baseline polynomial; no MCMC, the errors are the linear estimate of least squares",
        }
        .to_string(),
    );
    if let Some(correlation) = &correlation {
        for first in 0..n_shape {
            for second in first + 1..n_shape {
                let r = correlation[first][second];
                if r.abs() >= STRONG_CORRELATION {
                    notes.push(format!(
                        "{} and {} are strongly correlated (r = {r:.3}): their errors are not independent",
                        free_names[first], free_names[second]
                    ));
                }
            }
        }
    }
    notes.push(format!(
        "time-averaging test: binned residuals reach {beta:.2} times the white-noise expectation (beta); the errors are not enlarged by it"
    ));
    notes.push("stellar density from a_rs and the period: circular orbit, planet mass neglected".to_string());

    let mut warnings = Vec::new();
    if !fit.converged {
        warnings.push(format!("the fit did not converge in {iterations} iterations: the values are the last step, not a minimum"));
    }
    for parameter in parameters.iter().filter(|parameter| parameter.at_bound) {
        warnings.push(format!(
            "{} ended at its limit {}: it is not a free fit and its error is not meaningful",
            parameter.name, parameter.value
        ));
    }
    if kind == TransitKind::Eclipse && typed[SLOT_FLUX_RATIO].free && best[SLOT_FLUX_RATIO] < 0.0 {
        warnings.push("the fitted flux ratio is negative: no eclipse is detected at this depth".to_string());
    }
    if kind == TransitKind::Transit
        && (request.rp.free || request.b.free)
        && 1.0 + best[SLOT_B] - rp < TOTAL_OCCULTATION_MARGIN
    {
        warnings.push("the fit stopped where the planet would cover the whole star (rp = 1 + b): the dip is deeper or flatter than a transit of this shape gives".to_string());
    }
    if covariance.is_none() {
        warnings.push("the covariance could not be computed: the errors are missing".to_string());
    }
    let (u1, u2) = (best[SLOT_U1], best[SLOT_U2]);
    if kind == TransitKind::Transit && (request.u1.free || request.u2.free) && (u1 + u2 > 1.0 || u1 + 2.0 * u2 < 0.0) {
        warnings.push(format!(
            "fitted limb darkening is outside the physical range (u1 + u2 <= 1 and u1 + 2 u2 >= 0): u1 = {u1:.3}, u2 = {u2:.3}"
        ));
    }
    if weighted && !request.scale_errors && reduced_chi2 > POOR_REDUCED_CHI2 {
        warnings.push(format!(
            "reduced chi2 = {reduced_chi2:.1}: the model or the errors do not describe the scatter, so the errors of the fit are too small unless they are scaled"
        ));
    }
    if n_in_transit == n {
        warnings.push(format!("every point lies inside the {event}: the baseline is not constrained by points outside it"));
    }
    if beta >= BETA_WARNING {
        warnings.push(format!(
            "correlated noise: binned residuals are {beta:.2} times above the white-noise expectation, so the errors of the fit are too small by about that factor"
        ));
    }

    Ok(TransitFit {
        model: match kind {
            TransitKind::Transit => TRANSIT_MODEL_NAME,
            TransitKind::Eclipse => ECLIPSE_MODEL_NAME,
        }
        .to_string(),
        period,
        depth,
        depth_err,
        baseline: FittedBaseline {
            order: request.baseline_order,
            reference_time,
            errors: (n_shape..n_free).map(error_at).collect(),
            coefficients,
        },
        chi2: fit.chi2,
        dof: fit.dof,
        reduced_chi2,
        weighted,
        error_scale,
        rms: root_mean_square(residual.iter().copied(), n),
        rms_ppm: 1e6 * root_mean_square(relative_residual.iter().copied(), n),
        n_points: n,
        n_in_transit,
        iterations,
        converged: fit.converged,
        derived: derived_quantities(period, rp, best[SLOT_A_RS], best[SLOT_B], &free_names, covariance.as_deref()),
        free_names,
        parameters,
        covariance,
        time: request.time.clone(),
        model_flux,
        baseline_flux,
        residual,
        notes,
        warnings,
        correlation,
        time_averaging: averaging,
        curve_time,
        curve_flux,
        curve_baseline,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use serde_json::Value;
    use std::path::Path;
    use std::time::Instant;

    const NOTE_MODEL: &str =
        "model: Mandel and Agol 2002, quadratic limb darkening, circular orbit; flux = baseline polynomial x transit";
    const NOTE_NOT_MODELLED: &str = "not modelled: orbital eccentricity, the finite exposure time (the model is evaluated at the given mid-exposure times), limb darkening from stellar models, instrument systematics and correlated noise beyond the baseline polynomial; no MCMC, the errors are the linear estimate of least squares";
    const WARNING_NO_COVARIANCE: &str = "the covariance could not be computed: the errors are missing";
    const NOTE_A_RS_WITH_B: &str = "a_rs and b are strongly correlated (r = -0.996): their errors are not independent";
    const NOTE_WHITE_RESIDUALS: &str = "time-averaging test: binned residuals reach 1.00 times the white-noise expectation (beta); the errors are not enlarged by it";
    const NOTE_DENSITY: &str = "stellar density from a_rs and the period: circular orbit, planet mass neglected";
    const WARNING_CORRELATED_NOISE_OF_WD_1856: &str = "correlated noise: binned residuals are 1.55 times above the white-noise expectation, so the errors of the fit are too small by about that factor";
    const NOTE_ECLIPSE_MODEL: &str = "model: eclipse of a uniform planet disc by the star, circular orbit; flux = baseline polynomial x (1 - depth x hidden fraction of the planet), depth = flux_ratio / (1 + flux_ratio)";
    const NOTE_MID_ECLIPSE_TIME: &str = "t0 is the mid-eclipse time; on a circular orbit it is the mid-transit time plus half a period";
    const NOTE_ECLIPSE_NOT_MODELLED: &str = "not modelled: orbital eccentricity (it shifts and stretches the eclipse), the finite exposure time (the model is evaluated at the given mid-exposure times), the phase curve of the planet, limb darkening of the planet, instrument systematics and correlated noise beyond the baseline polynomial; no MCMC, the errors are the linear estimate of least squares";
    const REFUSAL_OF_A_FLUX_RATIO_BELOW_ITS_RANGE: &str = "flux_ratio -0.6 must be above -0.5 and at most 10 (planet flux over star flux)";
    const WARNING_NEGATIVE_FLUX_RATIO: &str = "the fitted flux ratio is negative: no eclipse is detected at this depth";

    fn assert_close(got: f64, want: f64, tolerance: f64, what: &str) {
        assert!((got - want).abs() <= tolerance, "{what}: got {got:?}, want {want:?}, off by {:e}", (got - want).abs());
    }

    fn assert_relative(got: f64, want: f64, tolerance: f64, what: &str) {
        let relative = ((got - want) / want).abs();
        assert!(relative <= tolerance, "{what}: got {got:?}, want {want:?}, relative difference {relative:e}");
    }

    fn assert_near(got: f64, want: f64, what: &str) {
        assert_close(got, want, 1e-12 + 1e-6 * want.abs(), what);
    }

    fn assert_all_near(got: &[f64], want: &[f64], what: &str) {
        assert_eq!(got.len(), want.len(), "{what}: {got:?}");
        for (index, (got, want)) in got.iter().zip(want).enumerate() {
            assert_near(*got, *want, &format!("{what}[{index}]"));
        }
    }

    fn assert_all_relative(got: &[f64], want: &[f64], tolerance: f64, what: &str) {
        assert_eq!(got.len(), want.len(), "{what}: {got:?}");
        for (index, (got, want)) in got.iter().zip(want).enumerate() {
            assert_relative(*got, *want, tolerance, &format!("{what}[{index}]"));
        }
    }

    fn assert_derived(value: Option<f64>, err: Option<f64>, want: f64, want_err: f64, what: &str) {
        assert_close(value.unwrap(), want, 1e-3 * want_err, what);
        assert_relative(err.unwrap(), want_err, 1e-3, &format!("error of {what}"));
    }

    fn derived_pairs(derived: &DerivedQuantities) -> [(&'static str, Option<f64>, Option<f64>); 4] {
        [
            ("inclination_deg", derived.inclination_deg, derived.inclination_err),
            ("duration_t14_days", derived.duration_t14_days, derived.duration_t14_err),
            ("duration_t23_days", derived.duration_t23_days, derived.duration_t23_err),
            ("stellar_density_cgs", derived.stellar_density_cgs, derived.stellar_density_err),
        ]
    }

    fn sorted_keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys
    }

    fn read_text(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    fn white_light_of_wd_1856() -> TransitFitRequest {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lightcurve/e1_white_light.json");
        let series: Value = serde_json::from_str(&read_text(&path)).unwrap();
        TransitFitRequest {
            time: numbers(&series["bmjd_tdb"]),
            flux: numbers(&series["flux"]),
            err: Some(numbers(&series["flux_err"])),
            period: 1.4079405,
            t0: free(60061.5077),
            rp: fixed(7.28),
            a_rs: fixed(336.0),
            b: free(7.16),
            u1: fixed(0.0),
            u2: fixed(0.0),
            baseline_order: 1,
            ..Default::default()
        }
    }

    fn assert_holds(lines: &[String], sentence: &str) {
        assert!(lines.iter().any(|line| line == sentence), "{sentence:?} is missing from {lines:#?}");
    }

    fn fixture() -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transit/fit_series_a.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn numbers(value: &Value) -> Vec<f64> {
        value.as_array().unwrap().iter().map(|member| member.as_f64().unwrap()).collect()
    }

    fn free(value: f64) -> ParameterInput {
        ParameterInput { value, free: true }
    }

    fn fixed(value: f64) -> ParameterInput {
        ParameterInput { value, free: false }
    }

    fn request_of(fixture: &Value, name: &str) -> TransitFitRequest {
        let fit = &fixture["fits"][name];
        let free_names: Vec<&str> = fit["free"].as_array().unwrap().iter().map(|member| member.as_str().unwrap()).collect();
        let input = |parameter: &str| ParameterInput {
            value: fit["start"][parameter].as_f64().unwrap(),
            free: free_names.contains(&parameter),
        };
        TransitFitRequest {
            time: numbers(&fixture["time"]),
            flux: numbers(&fixture["flux"]),
            err: fit["weighted"].as_bool().unwrap().then(|| numbers(&fixture["err"])),
            period: fixture["period"].as_f64().unwrap(),
            t0: input("t0"),
            rp: input("rp"),
            a_rs: input("a_rs"),
            b: input("b"),
            u1: input("u1"),
            u2: input("u2"),
            baseline_order: fit["baseline_order"].as_u64().unwrap() as u8,
            scale_errors: fit["scale_errors"].as_bool().unwrap(),
            ..Default::default()
        }
    }

    fn param<'a>(fit: &'a TransitFit, name: &str) -> &'a FittedParameter {
        fit.parameters.iter().find(|parameter| parameter.name == name).unwrap()
    }

    fn oracle_pairs(fit: &TransitFit, oracle: &Value) -> Vec<(String, f64, Option<f64>, f64, f64)> {
        let mut pairs = Vec::new();
        for name in oracle["free"].as_array().unwrap() {
            let name = name.as_str().unwrap();
            let found = param(fit, name);
            assert!(found.free, "{name} is free in the oracle");
            let value = oracle["values"][name].as_f64().unwrap();
            let err = oracle["errors"][name].as_f64().unwrap();
            pairs.push((name.to_string(), found.value, found.err, value, err));
        }
        let coefficients = numbers(&oracle["baseline"]);
        let errors = numbers(&oracle["baseline_errors"]);
        assert_eq!(fit.baseline.coefficients.len(), coefficients.len());
        assert_eq!(fit.baseline.errors.len(), errors.len());
        for k in 0..coefficients.len() {
            pairs.push((format!("c{k}"), fit.baseline.coefficients[k], fit.baseline.errors[k], coefficients[k], errors[k]));
        }
        pairs
    }

    fn oracle_fit(fixture: &Value, name: &str) -> TransitFit {
        let request = request_of(fixture, name);
        let oracle = &fixture["fits"][name];
        let started = Instant::now();
        let fit = fit_transit(&request).unwrap();
        let wall = started.elapsed();
        let mut worst_sigma = 0.0_f64;
        let mut worst_error = 0.0_f64;
        for (parameter, value, err, oracle_value, oracle_err) in oracle_pairs(&fit, oracle) {
            let sigma = (value - oracle_value).abs() / oracle_err;
            assert!(
                sigma <= 1e-3,
                "fits.{name}, {parameter}: got {value:?}, the oracle has {oracle_value:?} +- {oracle_err:?}, {sigma:e} sigma apart"
            );
            let err = err.unwrap_or_else(|| panic!("fits.{name}, {parameter}: no error"));
            let relative = ((err - oracle_err) / oracle_err).abs();
            assert!(
                relative <= 1e-3,
                "fits.{name}, error of {parameter}: got {err:?}, the oracle has {oracle_err:?}, relative difference {relative:e}"
            );
            worst_sigma = worst_sigma.max(sigma);
            worst_error = worst_error.max(relative);
        }
        let oracle_chi2 = oracle["chi2"].as_f64().unwrap();
        let chi2_gap = ((fit.chi2 - oracle_chi2) / oracle_chi2).abs();
        assert!(chi2_gap <= 1e-6, "fits.{name}, chi2: got {:?}, the oracle has {oracle_chi2:?}", fit.chi2);
        assert_eq!(fit.dof as u64, oracle["dof"].as_u64().unwrap(), "fits.{name}, dof");
        let number = |key: &str| oracle[key].as_f64().unwrap();
        assert_relative(fit.error_scale, number("error_scale"), 1e-6, "error scale");
        assert_close(fit.depth, number("depth"), 1e-3 * number("depth_err"), "depth");
        assert_relative(fit.depth_err.unwrap(), number("depth_err"), 1e-3, "depth error");
        assert_relative(fit.rms, number("rms"), 1e-6, "rms");
        assert_relative(fit.rms_ppm, number("rms_ppm"), 1e-6, "rms in ppm");
        assert_close(fit.baseline.reference_time, number("reference_time"), 1e-9, "reference time");
        assert_eq!(fit.n_in_transit as u64, oracle["n_in_transit"].as_u64().unwrap(), "fits.{name}, points in transit");
        let free_names: Vec<&str> = oracle["free_names"].as_array().unwrap().iter().map(|member| member.as_str().unwrap()).collect();
        assert_eq!(fit.free_names, free_names, "fits.{name}, free names");
        println!(
            "fits.{name}: {} iterations in {:.1} ms; largest difference from the oracle {worst_sigma:e} sigma, largest relative difference of an error {worst_error:e}, chi2 {chi2_gap:e} relative",
            fit.iterations,
            wall.as_secs_f64() * 1e3
        );
        fit
    }

    fn assert_pinned(fit: &TransitFit, name: &str, value: f64, err: f64) {
        let found = param(fit, name);
        assert_close(found.value, value, 1e-3 * err, name);
        assert_relative(found.err.unwrap(), err, 1e-3, &format!("error of {name}"));
    }

    fn assert_pinned_baseline(fit: &TransitFit, k: usize, value: f64, err: f64) {
        assert_close(fit.baseline.coefficients[k], value, 1e-3 * err, &format!("c{k}"));
        assert_relative(fit.baseline.errors[k].unwrap(), err, 1e-3, &format!("error of c{k}"));
    }

    fn noiseless_times() -> Vec<f64> {
        (0..240).map(|i| 10.38 + 0.001 * i as f64).collect()
    }

    fn refusal(request: &TransitFitRequest) -> String {
        fit_transit(request).unwrap_err().to_string()
    }

    fn refusal_after(base: &TransitFitRequest, change: impl FnOnce(&mut TransitFitRequest)) -> String {
        let mut request = base.clone();
        change(&mut request);
        refusal(&request)
    }

    fn eclipse_fixture() -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transit/eclipse_series.json");
        serde_json::from_str(&read_text(&path)).unwrap()
    }

    fn eclipse_request_of(fixture: &Value, name: &str) -> TransitFitRequest {
        let fit = &fixture["fits"][name];
        let free_names: Vec<&str> = fit["free"].as_array().unwrap().iter().map(|member| member.as_str().unwrap()).collect();
        let input = |parameter: &str| ParameterInput {
            value: fit["start"][parameter].as_f64().unwrap(),
            free: free_names.contains(&parameter),
        };
        TransitFitRequest {
            time: numbers(&fixture["time"]),
            flux: numbers(&fixture["flux"]),
            err: Some(numbers(&fixture["err"])),
            period: fixture["period"].as_f64().unwrap(),
            t0: input("t0"),
            rp: input("rp"),
            a_rs: input("a_rs"),
            b: input("b"),
            baseline_order: fit["baseline_order"].as_u64().unwrap() as u8,
            kind: TransitKind::Eclipse,
            flux_ratio: Some(input("flux_ratio")),
            ..Default::default()
        }
    }

    fn eclipse_oracle_fit(fixture: &Value, name: &str) -> TransitFit {
        let oracle = &fixture["fits"][name];
        let fit = fit_transit(&eclipse_request_of(fixture, name)).unwrap();
        let mut worst_sigma = 0.0_f64;
        let mut worst_error = 0.0_f64;
        for (parameter, value, err, oracle_value, oracle_err) in oracle_pairs(&fit, oracle) {
            let sigma = (value - oracle_value).abs() / oracle_err;
            assert!(
                sigma <= 1e-3,
                "fits.{name}, {parameter}: got {value:?}, the oracle has {oracle_value:?} +- {oracle_err:?}, {sigma:e} sigma apart"
            );
            let err = err.unwrap_or_else(|| panic!("fits.{name}, {parameter}: no error"));
            let relative = ((err - oracle_err) / oracle_err).abs();
            assert!(
                relative <= 1e-3,
                "fits.{name}, error of {parameter}: got {err:?}, the oracle has {oracle_err:?}, relative difference {relative:e}"
            );
            worst_sigma = worst_sigma.max(sigma);
            worst_error = worst_error.max(relative);
        }
        let oracle_chi2 = oracle["chi2"].as_f64().unwrap();
        let chi2_gap = ((fit.chi2 - oracle_chi2) / oracle_chi2).abs();
        assert!(chi2_gap <= 1e-6, "fits.{name}, chi2: got {:?}, the oracle has {oracle_chi2:?}", fit.chi2);
        assert_eq!(fit.dof as u64, oracle["dof"].as_u64().unwrap(), "fits.{name}, dof");
        let number = |key: &str| oracle[key].as_f64().unwrap();
        assert_close(fit.depth, number("depth"), 1e-3 * number("depth_err"), "depth");
        assert_relative(fit.depth_err.unwrap(), number("depth_err"), 1e-3, "depth error");
        assert_close(fit.baseline.reference_time, number("reference_time"), 1e-9, "reference time");
        println!(
            "fits.{name}: {} iterations; values {:?}; baseline {:?} +- {:?}; chi2 {:?}; depth {:?} +- {:?}; beta {:?}; largest difference from the oracle {worst_sigma:e} sigma, largest relative difference of an error {worst_error:e}, chi2 {chi2_gap:e} relative",
            fit.iterations,
            fit.parameters.iter().map(|parameter| (parameter.value, parameter.err)).collect::<Vec<_>>(),
            fit.baseline.coefficients,
            fit.baseline.errors,
            fit.chi2,
            fit.depth,
            fit.depth_err,
            fit.time_averaging.beta
        );
        fit
    }

    fn fixed_parameter(name: &str, value: f64) -> FittedParameter {
        FittedParameter { name: name.to_string(), value, err: None, free: false, at_bound: false }
    }

    #[test]
    fn the_fixture_series_has_its_checkpoints() {
        let fixture = fixture();
        let time = numbers(&fixture["time"]);
        let flux = numbers(&fixture["flux"]);
        let err = numbers(&fixture["err"]);
        assert_eq!((time.len(), flux.len(), err.len()), (240, 240, 240));
        assert_close(time[0], 2460310.38, 1e-9, "time[0]");
        assert_close(time[239], 2460310.619, 1e-9, "time[239]");
        assert_close(flux[0], 997.4048558152281, 1e-9, "flux[0]");
        assert_close(flux[120], 988.2609567878745, 1e-9, "flux[120]");
        assert_close(flux[239], 1002.1584460332058, 1e-9, "flux[239]");
        assert!(err.iter().all(|&sigma| sigma == 0.3));
    }

    #[test]
    fn a_weighted_fit_matches_the_oracle() {
        let fixture = fixture();
        let fit = oracle_fit(&fixture, "weighted");
        assert_pinned(&fit, "t0", 2460310.499939434, 8.2774e-05);
        assert_pinned(&fit, "rp", 0.09982034774988176, 0.00047749281707474504);
        assert_pinned(&fit, "a_rs", 8.997425855947435, 0.16873016536633761);
        assert_pinned(&fit, "b", 0.2969973302107353, 0.05840847747413874);
        assert_pinned_baseline(&fit, 0, 999.9775222619853, 0.02647961156016481);
        assert_pinned_baseline(&fit, 1, 19.918566184050015, 0.2899882496347247);
        assert_relative(fit.chi2, 195.2853876168914, 1e-6, "chi2");
        assert_eq!(fit.dof, 234);
        assert_eq!(fit.reduced_chi2, fit.chi2 / 234.0);
        assert!(fit.converged);
        assert!(fit.weighted);
        assert_eq!(fit.error_scale, 1.0);
        assert_close(fit.depth, 0.009964101824907324, 1e-3 * 9.532699809694346e-05, "depth");
        assert_relative(fit.depth_err.unwrap(), 9.532699809694346e-05, 1e-3, "depth error");
        assert_relative(fit.rms, 0.27061415402069433, 1e-6, "rms");
        assert_relative(fit.rms_ppm, 270.70343615597596, 1e-6, "rms in ppm");
        assert_eq!(fit.n_in_transit, 113);
        assert_eq!(fit.n_points, 240);
        assert_eq!(fit.period, 3.0);
        assert_close(fit.baseline.reference_time, 2460310.4995, 1e-9, "reference time");
        assert_eq!(fit.baseline.order, 1);
        let names: Vec<&str> = fit.parameters.iter().map(|parameter| parameter.name.as_str()).collect();
        assert_eq!(names, TRANSIT_PARAMETER_NAMES);
        assert_eq!(*param(&fit, "u1"), FittedParameter { name: "u1".to_string(), value: 0.4, err: None, free: false, at_bound: false });
        assert_eq!(*param(&fit, "u2"), FittedParameter { name: "u2".to_string(), value: 0.25, err: None, free: false, at_bound: false });
        assert!(fit.parameters.iter().all(|parameter| !parameter.at_bound));
        assert_eq!(fit.free_names, ["t0", "rp", "a_rs", "b", "c0", "c1"]);
        let covariance = fit.covariance.as_ref().unwrap();
        assert_eq!(covariance.len(), 6);
        assert!(covariance.iter().all(|row| row.len() == 6));
        let errors: Vec<f64> = ["t0", "rp", "a_rs", "b"]
            .iter()
            .map(|name| param(&fit, name).err.unwrap())
            .chain(fit.baseline.errors.iter().map(|err| err.unwrap()))
            .collect();
        for (index, err) in errors.iter().enumerate() {
            assert_eq!(covariance[index][index].sqrt(), *err, "diagonal {index}");
        }
        let flux = numbers(&fixture["flux"]);
        assert_eq!(fit.time, numbers(&fixture["time"]));
        assert_eq!((fit.model_flux.len(), fit.baseline_flux.len(), fit.residual.len()), (240, 240, 240));
        for i in 0..240 {
            assert_eq!(fit.residual[i], flux[i] - fit.model_flux[i], "residual {i}");
        }
        assert_eq!(fit.model_flux[0], fit.baseline_flux[0]);
        assert!(fit.model_flux[120] < fit.baseline_flux[120]);
        assert!(fit.warnings.is_empty(), "{:?}", fit.warnings);
    }

    #[test]
    fn the_injected_transit_is_recovered_within_three_sigma() {
        let fixture = fixture();
        let fit = fit_transit(&request_of(&fixture, "weighted")).unwrap();
        for name in ["t0", "rp", "a_rs", "b"] {
            let found = param(&fit, name);
            let pull = (found.value - fixture["truth"][name].as_f64().unwrap()) / found.err.unwrap();
            assert!(pull.abs() < 3.0, "{name}: {found:?} is {pull} sigma from the injected value");
        }
        for (k, truth) in [999.99, 20.0].into_iter().enumerate() {
            let pull = (fit.baseline.coefficients[k] - truth) / fit.baseline.errors[k].unwrap();
            assert!(pull.abs() < 3.0, "c{k}: {:?} is {pull} sigma from {truth}", fit.baseline);
        }
    }

    #[test]
    fn an_unweighted_fit_scales_its_errors_by_the_scatter() {
        let fixture = fixture();
        let fit = oracle_fit(&fixture, "unweighted");
        assert_relative(fit.chi2, 1.765257323879983e-05, 1e-6, "chi2");
        assert_eq!(fit.dof, 234);
        assert!(!fit.weighted);
        assert_relative(fit.error_scale, 0.00027466042156517245, 1e-6, "error scale");
        assert_eq!(fit.error_scale, (fit.chi2 / fit.dof as f64).sqrt());
        assert_holds(&fit.notes, "equal weights: the points carry no errors");
        assert_holds(
            &fit.notes,
            "errors: 1 sigma from the covariance of the fit, scaled by the scatter of the residuals (sqrt(chi2 / dof))",
        );
        let weighted = fit_transit(&request_of(&fixture, "weighted")).unwrap();
        assert_relative(fit.rms, weighted.rms, 1e-6, "rms");
    }

    #[test]
    fn a_fit_of_the_radius_alone_keeps_the_fixed_values() {
        let fixture = fixture();
        let fit = oracle_fit(&fixture, "radius_only");
        assert_pinned(&fit, "rp", 0.09985155064081229, 0.00018570663739961966);
        assert_relative(fit.chi2, 196.33875744222533, 1e-6, "chi2");
        assert_eq!(fit.dof, 237);
        for (name, value) in [("t0", 2460310.5), ("a_rs", 9.0), ("b", 0.3)] {
            assert_eq!(*param(&fit, name), FittedParameter { name: name.to_string(), value, err: None, free: false, at_bound: false });
        }
        assert_relative(fit.depth_err.unwrap(), 3.708619141728617e-05, 1e-3, "depth error");
        assert_eq!(fit.free_names, ["rp", "c0", "c1"]);
        assert_holds(&fit.notes, "fixed as given: t0 = 2460310.5, a_rs = 9, b = 0.3");
    }

    #[test]
    fn limb_darkening_can_be_fitted() {
        let fixture = fixture();
        let fit = oracle_fit(&fixture, "all_free");
        assert_pinned(&fit, "u1", 0.34020066278680805, 0.06600150249544098);
        assert_pinned(&fit, "u2", 0.330533251756405, 0.14018619354931677);
        assert_relative(fit.chi2, 193.77186379629956, 1e-6, "chi2");
        assert_eq!(fit.dof, 232);
        assert_eq!(fit.free_names, ["t0", "rp", "a_rs", "b", "u1", "u2", "c0", "c1"]);
        assert_holds(&fit.notes, "limb darkening fitted: u1 and u2 free, started at 0.4 and 0.25");
        assert!(fit.warnings.is_empty(), "{:?}", fit.warnings);
        let mut one_coefficient = request_of(&fixture, "all_free");
        one_coefficient.u2.free = false;
        let fit = fit_transit(&one_coefficient).unwrap();
        assert_holds(&fit.notes, "limb darkening: u1 fitted, u2 fixed at 0.25");
        assert_eq!(fit.free_names, ["t0", "rp", "a_rs", "b", "u1", "c0", "c1"]);
        assert_eq!(*param(&fit, "u2"), FittedParameter { name: "u2".to_string(), value: 0.25, err: None, free: false, at_bound: false });
        let mut other_coefficient = request_of(&fixture, "all_free");
        other_coefficient.u1.free = false;
        let fit = fit_transit(&other_coefficient).unwrap();
        assert_holds(&fit.notes, "limb darkening: u1 fixed at 0.4, u2 fitted");
        assert_eq!(fit.free_names, ["t0", "rp", "a_rs", "b", "u2", "c0", "c1"]);
    }

    #[test]
    fn a_quadratic_baseline_and_scaled_errors() {
        let fixture = fixture();
        let scaled = oracle_fit(&fixture, "quadratic_scaled");
        assert_pinned_baseline(&scaled, 2, -2.132921190747918, 7.42864327433326);
        assert_eq!(scaled.dof, 233);
        assert_eq!(scaled.baseline.order, 2);
        assert_relative(scaled.error_scale, 0.9153364314794327, 1e-6, "error scale");
        assert_holds(
            &scaled.notes,
            "errors: 1 sigma from the covariance of the fit, multiplied by sqrt(reduced chi2) = 0.9153 (asked)",
        );
        let plain = fit_transit(&TransitFitRequest { scale_errors: false, ..request_of(&fixture, "quadratic_scaled") }).unwrap();
        assert_eq!(plain.error_scale, 1.0);
        let pairs = |fit: &TransitFit| -> Vec<(f64, Option<f64>)> {
            fit.parameters
                .iter()
                .map(|parameter| (parameter.value, parameter.err))
                .chain(fit.baseline.coefficients.iter().copied().zip(fit.baseline.errors.iter().copied()))
                .collect()
        };
        let scaled_pairs = pairs(&scaled);
        let plain_pairs = pairs(&plain);
        assert_eq!(scaled_pairs.len(), 9);
        let mut compared = 0;
        for (index, ((scaled_value, scaled_err), (plain_value, plain_err))) in scaled_pairs.iter().zip(&plain_pairs).enumerate() {
            assert_eq!(scaled_value.to_bits(), plain_value.to_bits(), "value {index}");
            assert_eq!(scaled_err.is_some(), plain_err.is_some(), "error {index}");
            if let (Some(scaled_err), Some(plain_err)) = (scaled_err, plain_err) {
                assert_relative(*plain_err, scaled_err / 0.9153364314794327, 1e-9, &format!("error {index}"));
                compared += 1;
            }
        }
        assert_eq!(compared, 7);
    }

    #[test]
    fn a_noiseless_transit_is_recovered_exactly() {
        let shape = TransitShape { t0: 10.5, period: 3.0, rp: 0.1, a_rs: 9.0, b: 0.3 };
        let limb = LimbDarkening { u1: 0.4, u2: 0.25 };
        let time = noiseless_times();
        let flux = time.iter().map(|&t| 500.0 * (1.0 + 0.02 * (t - 10.5)) * transit_flux(t, &shape, &limb)).collect();
        let request = TransitFitRequest {
            time,
            flux,
            period: 3.0,
            t0: free(10.502),
            rp: free(0.09),
            a_rs: free(8.0),
            b: free(0.4),
            u1: fixed(0.4),
            u2: fixed(0.25),
            baseline_order: 1,
            ..Default::default()
        };
        let fit = fit_transit(&request).unwrap();
        assert!(fit.converged, "{:?}", fit.warnings);
        assert_close(param(&fit, "t0").value, 10.5, 1e-9, "t0");
        assert_close(param(&fit, "rp").value, 0.1, 1e-9, "rp");
        assert_close(param(&fit, "a_rs").value, 9.0, 1e-7, "a_rs");
        assert_close(param(&fit, "b").value, 0.3, 1e-7, "b");
        assert!(fit.rms < 1e-9, "rms {:e}", fit.rms);
        assert_close(fit.baseline.reference_time, 10.4995, 1e-12, "reference time");
        assert_close(fit.baseline.coefficients[0], 499.995, 1e-6, "c0");
        assert_close(fit.baseline.coefficients[1], 10.0, 1e-6, "c1");
        assert!(!fit.weighted);
    }

    #[test]
    fn the_fit_does_not_depend_on_the_flux_unit_or_the_time_scale() {
        let fixture = fixture();
        let oracle_errors = &fixture["fits"]["weighted"]["errors"];
        let request = request_of(&fixture, "weighted");
        let reference = fit_transit(&request).unwrap();

        let unit = 1e-20;
        let faint = TransitFitRequest {
            flux: request.flux.iter().map(|flux| flux * unit).collect(),
            err: request.err.as_ref().map(|err| err.iter().map(|sigma| sigma * unit).collect()),
            ..request.clone()
        };
        let fit = fit_transit(&faint).unwrap();
        for name in ["t0", "rp", "a_rs", "b"] {
            let tolerance = 1e-3 * oracle_errors[name].as_f64().unwrap();
            assert_close(param(&fit, name).value, param(&reference, name).value, tolerance, name);
        }
        assert_relative(fit.chi2, reference.chi2, 1e-9, "chi2");
        for k in 0..2 {
            assert_relative(fit.baseline.coefficients[k], reference.baseline.coefficients[k] * unit, 1e-6, &format!("c{k}"));
        }

        let offset = 2400000.5;
        let shifted = TransitFitRequest {
            time: request.time.iter().map(|time| time - offset).collect(),
            t0: free(request.t0.value - offset),
            ..request.clone()
        };
        let fit = fit_transit(&shifted).unwrap();
        let t0_tolerance = 1e-3 * oracle_errors["t0"].as_f64().unwrap();
        assert_close(param(&fit, "t0").value, param(&reference, "t0").value - offset, t0_tolerance, "t0 in the shifted scale");
        for name in ["rp", "a_rs", "b"] {
            let tolerance = 1e-3 * oracle_errors[name].as_f64().unwrap();
            assert_close(param(&fit, name).value, param(&reference, name).value, tolerance, name);
        }
    }

    #[test]
    fn a_t0_of_another_epoch_is_moved_to_the_series() {
        let fixture = fixture();
        let oracle = &fixture["fits"]["weighted"];
        let request = request_of(&fixture, "weighted");
        let moved = TransitFitRequest { t0: free(2460310.502 - 911.0 * 3.0), ..request.clone() };
        let fit = fit_transit(&moved).unwrap();
        assert_close(param(&fit, "t0").value, 2460310.499939434, 1e-3 * 8.2774e-05, "t0");
        for name in ["t0", "rp", "a_rs", "b"] {
            let tolerance = 1e-3 * oracle["errors"][name].as_f64().unwrap();
            assert_close(param(&fit, name).value, oracle["values"][name].as_f64().unwrap(), tolerance, name);
        }
        assert_holds(&fit.notes, "t0 moved by -911 periods from the typed value to the transit nearest the middle of the series");
        let unmoved = fit_transit(&request).unwrap();
        assert!(!unmoved.notes.iter().any(|note| note.starts_with("t0 moved")), "{:?}", unmoved.notes);

        let radius_only = request_of(&fixture, "radius_only");
        let later = TransitFitRequest { t0: fixed(2460310.5 + 5.0 * 3.0), ..radius_only };
        let fit = fit_transit(&later).unwrap();
        assert_eq!(*param(&fit, "t0"), FittedParameter { name: "t0".to_string(), value: 2460310.5, err: None, free: false, at_bound: false });
        assert_pinned(&fit, "rp", 0.09985155064081229, 0.00018570663739961966);
        assert_holds(&fit.notes, "t0 moved by 5 periods from the typed value to the transit nearest the middle of the series");
        assert_holds(&fit.notes, "fixed as given: t0 = 2460310.5, a_rs = 9, b = 0.3");
    }

    #[test]
    fn inputs_that_can_not_be_fitted_are_refused_with_the_reason() {
        let base = request_of(&fixture(), "weighted");
        assert_eq!(
            refusal_after(&base, |request| request.flux.truncate(239)),
            "time has 240 values and flux has 239: they must have the same length"
        );
        assert_eq!(refusal_after(&base, |request| request.err = Some(vec![0.3; 2])), "err has 2 values for 240 points");
        assert_eq!(
            refusal_after(&base, |request| {
                request.time.clear();
                request.flux.clear();
                request.err = None;
            }),
            "no point to fit: time and flux are empty"
        );
        assert_eq!(
            refusal_after(&base, |request| {
                request.time = vec![2460310.5];
                request.flux = vec![1000.0];
                request.err = None;
            }),
            "every point has the same time: nothing can be fitted against time"
        );
        assert_eq!(refusal_after(&base, |request| request.baseline_order = 3), "baseline order 3 must be 0, 1 or 2");
        assert_eq!(
            refusal_after(&base, |request| {
                request.flux[5] = f64::NAN;
                request.time[9] = f64::INFINITY;
            }),
            "time or flux is not a finite number at 2 of 240 points (first at index 5): leave those points out before the fit"
        );
        assert_eq!(
            refusal_after(&base, |request| request.err.as_mut().unwrap()[7] = 0.0),
            "err is not finite and above 0 at 1 of 240 points (first at index 7): pass no errors or leave those points out"
        );
        assert_eq!(
            refusal_after(&base, |request| request.time = vec![2460310.5; 240]),
            "every point has the same time: nothing can be fitted against time"
        );
        assert_eq!(
            refusal_after(&base, |request| request.rp.value = 0.0),
            "rp 0 must be above 0 and at most 100 (planet radius over star radius)"
        );
        assert_eq!(
            refusal_after(&base, |request| request.b.value = 1.5),
            "no transit: b 1.5 is at or above 1 + rp = 1.09, so the planet never crosses the star"
        );
        assert_eq!(refusal_after(&base, |request| request.period = -1.0), "period -1 d must be above 0");
        assert_eq!(
            refusal_after(&base, |request| request.flux = vec![-5.0; 240]),
            "the median flux -5 is not above 0: the fit needs fluxes on a positive scale"
        );
        assert_eq!(
            refusal_after(&base, |request| {
                request.time.truncate(6);
                request.flux.truncate(6);
                request.err.as_mut().unwrap().truncate(6);
            }),
            "6 points for 6 free parameters: the fit needs more points than free parameters"
        );
        assert_eq!(
            refusal_after(&base, |request| request.t0.value = 2460311.5),
            "no point of the series lies inside the transit of the starting values (t0 2460311.50000, duration 0.12151 d): check t0 and the period"
        );
        assert_eq!(
            refusal_after(&base, |request| {
                request.time = (0..200_001).map(|i| 2460310.0 + 1e-6 * i as f64).collect();
                request.flux = vec![1000.0; 200_001];
                request.err = Some(vec![0.3; 200_001]);
            }),
            "200001 points exceed the limit of 200000"
        );
    }

    #[test]
    fn the_notes_state_every_assumption() {
        let fit = fit_transit(&request_of(&fixture(), "weighted")).unwrap();
        assert_eq!(
            fit.notes,
            [
                NOTE_MODEL,
                "period 3 d given, not fitted",
                "limb darkening fixed as typed: u1 = 0.4, u2 = 0.25 (not taken from a stellar model)",
                "baseline: polynomial of order 1 in time about 2460310.49950 (the middle of the series), fitted with the transit",
                "weights 1 / err^2",
                "errors: 1 sigma from the covariance of the fit, not scaled (reduced chi2 = 0.835)",
                NOTE_NOT_MODELLED,
                NOTE_A_RS_WITH_B,
                NOTE_WHITE_RESIDUALS,
                NOTE_DENSITY,
            ]
        );
    }

    #[test]
    fn a_parameter_that_ends_on_its_limit_is_flagged_and_has_no_error() {
        let shape = TransitShape { t0: 10.5, period: 3.0, rp: 0.1, a_rs: 9.0, b: 0.0 };
        let limb = LimbDarkening { u1: 0.4, u2: 0.25 };
        let time = noiseless_times();
        let flux = time.iter().map(|&t| 500.0 * transit_flux(t, &shape, &limb)).collect();
        let request = TransitFitRequest {
            time,
            flux,
            period: 3.0,
            t0: fixed(10.5),
            rp: fixed(0.1),
            a_rs: fixed(9.0),
            b: free(0.0),
            u1: fixed(0.4),
            u2: fixed(0.25),
            ..Default::default()
        };
        let fit = fit_transit(&request).unwrap();
        assert_eq!(*param(&fit, "b"), FittedParameter { name: "b".to_string(), value: 0.0, err: None, free: true, at_bound: true });
        assert!(!fit.converged);
        assert!(fit.covariance.is_none());
        assert_close(fit.baseline.coefficients[0], 500.0, 1e-9, "c0");
        assert_eq!(fit.baseline.errors, [None]);
        assert_eq!(fit.depth_err, None);
        assert_eq!(fit.free_names, ["b", "c0"]);
        assert_eq!(
            fit.warnings,
            [
                format!("the fit did not converge in {} iterations: the values are the last step, not a minimum", fit.iterations).as_str(),
                "b ended at its limit 0: it is not a free fit and its error is not meaningful",
                WARNING_NO_COVARIANCE,
            ]
        );
    }

    #[test]
    fn the_other_warnings_fire_only_when_they_apply() {
        let fixture = fixture();
        let weighted = request_of(&fixture, "weighted");
        let small_errors = TransitFitRequest { err: Some(vec![0.1; 240]), ..weighted };
        let fit = fit_transit(&small_errors).unwrap();
        assert_eq!(
            fit.warnings,
            ["reduced chi2 = 7.5: the model or the errors do not describe the scatter, so the errors of the fit are too small unless they are scaled"]
        );
        let fit = fit_transit(&TransitFitRequest { scale_errors: true, ..small_errors }).unwrap();
        assert!(fit.warnings.is_empty(), "{:?}", fit.warnings);

        let radius_only = request_of(&fixture, "radius_only");
        let inside = TransitFitRequest {
            time: radius_only.time[70..170].to_vec(),
            flux: radius_only.flux[70..170].to_vec(),
            err: Some(vec![0.3; 100]),
            ..radius_only
        };
        let fit = fit_transit(&inside).unwrap();
        assert_eq!((fit.n_in_transit, fit.n_points), (100, 100));
        assert_eq!(fit.warnings, ["every point lies inside the transit: the baseline is not constrained by points outside it"]);

        let time = noiseless_times();
        let steep_limb = LimbDarkening { u1: 0.9, u2: 0.3 };
        let shape = TransitShape { t0: 10.5, period: 3.0, rp: 0.1, a_rs: 9.0, b: 0.3 };
        let noiseless = TransitFitRequest {
            flux: time.iter().map(|&t| 500.0 * transit_flux(t, &shape, &steep_limb)).collect(),
            time: time.clone(),
            period: 3.0,
            t0: fixed(10.5),
            rp: fixed(0.1),
            a_rs: fixed(9.0),
            b: fixed(0.3),
            u1: free(0.4),
            u2: free(0.25),
            ..Default::default()
        };
        let fit = fit_transit(&noiseless).unwrap();
        assert_close(param(&fit, "u1").value, 0.9, 1e-9, "u1");
        assert_close(param(&fit, "u2").value, 0.3, 1e-9, "u2");
        assert_holds(
            &fit.warnings,
            "fitted limb darkening is outside the physical range (u1 + u2 <= 1 and u1 + 2 u2 >= 0): u1 = 0.900, u2 = 0.300",
        );

        let eclipse = TransitFitRequest {
            flux: time.iter().map(|&t| if (t - 10.5).abs() < 0.05 { 1.0 } else { 500.0 }).collect(),
            time,
            period: 3.0,
            t0: fixed(10.5),
            rp: free(0.9),
            a_rs: fixed(9.0),
            b: fixed(0.0),
            u1: fixed(0.4),
            u2: fixed(0.25),
            ..Default::default()
        };
        let fit = fit_transit(&eclipse).unwrap();
        let radius = param(&fit, "rp");
        assert!(radius.value < 1.0 && 1.0 - radius.value < 1e-6 && !radius.at_bound, "{radius:?}");
        assert_holds(
            &fit.warnings,
            "the fit stopped where the planet would cover the whole star (rp = 1 + b): the dip is deeper or flatter than a transit of this shape gives",
        );
    }

    #[test]
    fn time_averaging_of_alternating_residuals_falls_faster_than_white_noise() {
        let time: Vec<f64> = (0..16).map(f64::from).collect();
        let residual: Vec<f64> = (0..16).map(|i| if i % 2 == 0 { 1e-3 } else { -1e-3 }).collect();
        let found = time_averaging(&time, &residual);
        assert_eq!(found.bin_points, [1, 2, 4]);
        assert_eq!(found.n_bins, [16, 8, 4]);
        assert_all_near(&found.rms_ppm, &[1000.0, 0.0, 0.0], "rms");
        assert_all_near(&found.expected_ppm, &[1032.7955589886444, 755.9289460184544, 577.3502691896258], "expected");
        assert_eq!(found.beta, 1.0);
        assert_near(found.cadence_days, 1.0, "cadence");
    }

    #[test]
    fn time_averaging_of_a_constant_offset_shows_correlated_noise() {
        let time: Vec<f64> = (0..16).map(f64::from).collect();
        let found = time_averaging(&time, &[2e-3; 16]);
        assert_eq!(found.bin_points, [1, 2, 4]);
        assert_eq!(found.n_bins, [16, 8, 4]);
        assert_all_near(&found.rms_ppm, &[2000.0; 3], "rms");
        assert_all_near(&found.expected_ppm, &[2065.591117977289, 1511.8578920369089, 1154.7005383792516], "expected");
        assert_near(found.beta, 1.7320508075688772, "beta");
    }

    #[test]
    fn time_averaging_sorts_by_time_and_drops_an_incomplete_last_bin() {
        let time = [8.0, 0.0, 3.0, 1.0, 7.0, 2.0, 6.0, 4.0, 5.0];
        let residual = time.map(|time| (time + 1.0) * 1e-6);
        let found = time_averaging(&time, &residual);
        assert_eq!(found.bin_points, [1, 2]);
        assert_eq!(found.n_bins, [9, 4]);
        assert_all_near(&found.rms_ppm, &[5.627314338711377, 5.024937810560445], "rms");
        assert_all_near(&found.expected_ppm, &[5.9686681931566605, 4.594682917363407], "expected");
        assert_near(found.beta, 1.0936419119524212, "beta");
        assert_near(found.cadence_days, 1.0, "cadence");

        let three_points = time_averaging(&[0.0, 1.0, 3.0], &[1e-6, -2e-6, 3e-6]);
        assert_eq!(
            three_points,
            TimeAveraging {
                cadence_days: 1.5,
                bin_points: Vec::new(),
                n_bins: Vec::new(),
                rms_ppm: Vec::new(),
                expected_ppm: Vec::new(),
                beta: 1.0,
            }
        );
        assert_eq!(time_averaging(&[5.0], &[1e-6]).cadence_days, 0.0);
        assert_eq!(time_averaging(&[0.0, 10.0, 1.0, 2.0], &[0.0; 4]).cadence_days, 1.0);
    }

    #[test]
    fn the_fixture_fits_have_the_time_averaging_rows_of_the_oracle() {
        let fixture = fixture();
        let found = fit_transit(&request_of(&fixture, "weighted")).unwrap().time_averaging;
        let oracle = &fixture["fits"]["weighted"]["time_averaging"];
        assert_eq!(found.bin_points, [1, 2, 4, 8, 16, 32]);
        assert_eq!(found.n_bins, [240, 120, 60, 30, 15, 7]);
        assert_all_near(&found.rms_ppm, &numbers(&oracle["rms_ppm"]), "rms");
        assert_all_near(&found.expected_ppm, &numbers(&oracle["expected_ppm"]), "expected");
        assert_near(found.rms_ppm[0], 270.70343615597596, "rms of single points");
        assert_near(found.expected_ppm[0], 271.26917018194166, "expectation for single points");
        assert_near(found.rms_ppm[5], 40.93712874098216, "rms of bins of 32");
        assert_near(found.expected_ppm[5], 51.6882911278919, "expectation for bins of 32");
        assert_eq!(found.beta, 1.0);
        assert_close(found.cadence_days, 0.0010000001639127731, 1e-9, "cadence");
        let radius_only = fit_transit(&request_of(&fixture, "radius_only")).unwrap().time_averaging;
        assert_near(radius_only.beta, 1.0048589379526505, "beta of the radius-only fit");
    }

    #[test]
    fn derived_quantities_follow_their_closed_forms() {
        let found = derived_quantities(3.0, 0.1, 9.0, 0.3, &[], None);
        assert_near(found.inclination_deg.unwrap(), 88.08978682829007, "inclination");
        assert_near(found.duration_t14_days.unwrap(), 0.1126124414167327, "T14");
        assert_near(found.duration_t23_days.unwrap(), 0.09021583157004577, "T23");
        assert_near(found.stellar_density_cgs.unwrap(), 1.5322263886706138, "density");
        for (key, _, err) in derived_pairs(&found) {
            assert_eq!(err, None, "error of {key}");
        }
        let white_dwarf = derived_quantities(1.4079405, 7.28, 336.0, 7.16, &[], None);
        assert_eq!(white_dwarf.duration_t23_days, None);
        assert_near(white_dwarf.stellar_density_cgs.unwrap(), 361982.1435066046, "density of a white dwarf");
        let never_crossing = derived_quantities(3.0, 0.1, 9.0, 9.5, &[], None);
        assert_eq!(never_crossing.inclination_deg, None);
    }

    #[test]
    fn derived_errors_propagate_the_covariance() {
        let fixture = fixture();
        let found = fit_transit(&request_of(&fixture, "weighted")).unwrap().derived;
        let oracle = &fixture["fits"]["weighted"]["derived"];
        for (key, value, err) in derived_pairs(&found) {
            let pair = numbers(&oracle[key]);
            assert_derived(value, err, pair[0], pair[1], key);
        }
        assert_derived(found.inclination_deg, found.inclination_err, 88.10837170783789, 0.40749544737041427, "inclination");
        assert_derived(found.duration_t14_days, found.duration_t14_err, 0.11271416821362473, 0.0004805759083028952, "T14");
        assert_derived(found.duration_t23_days, found.duration_t23_err, 0.09037364753358537, 0.0005380977600151819, "T23");
        assert_derived(found.stellar_density_cgs, found.stellar_density_err, 1.5309120408516246, 0.08612831467807824, "density");

        let radius_only = fit_transit(&request_of(&fixture, "radius_only")).unwrap().derived;
        let oracle = &fixture["fits"]["radius_only"]["derived"];
        for (key, value, _) in derived_pairs(&radius_only) {
            assert_near(value.unwrap(), numbers(&oracle[key])[0], key);
        }
        assert_relative(radius_only.duration_t14_err.unwrap(), 2.0635361030171306e-05, 1e-3, "error of T14 with rp alone free");
        assert_relative(
            radius_only.duration_t23_err.unwrap(),
            numbers(&oracle["duration_t23_days"])[1],
            1e-3,
            "error of T23 with rp alone free",
        );
        assert_eq!(radius_only.inclination_err, None);
        assert_eq!(radius_only.stellar_density_err, None);
    }

    #[test]
    fn correlations_are_reported_and_strong_ones_noted() {
        let fixture = fixture();
        let fit = fit_transit(&request_of(&fixture, "weighted")).unwrap();
        let correlation = fit.correlation.as_ref().unwrap();
        let oracle = fixture["fits"]["weighted"]["correlation"].as_array().unwrap();
        assert_eq!(correlation.len(), 6);
        for (i, row) in correlation.iter().enumerate() {
            assert_eq!(row.len(), 6);
            assert_close(row[i], 1.0, 1e-12, &format!("diagonal {i}"));
            for (j, want) in numbers(&oracle[i]).into_iter().enumerate() {
                assert_close(row[j], want, 1e-4, &format!("correlation[{i}][{j}]"));
            }
        }
        assert_close(correlation[2][3], -0.9956622594265195, 1e-4, "a_rs with b");
        assert_holds(&fit.notes, NOTE_A_RS_WITH_B);
        assert_eq!(fit.notes.iter().filter(|note| note.contains("strongly correlated")).count(), 1, "{:#?}", fit.notes);
        assert_eq!(fit.notes[fit.notes.len() - 2..], [NOTE_WHITE_RESIDUALS, NOTE_DENSITY]);

        let shape = TransitShape { t0: 10.5, period: 3.0, rp: 0.1, a_rs: 9.0, b: 0.0 };
        let limb = LimbDarkening { u1: 0.4, u2: 0.25 };
        let time = noiseless_times();
        let on_its_limit = TransitFitRequest {
            flux: time.iter().map(|&t| 500.0 * transit_flux(t, &shape, &limb)).collect(),
            time,
            period: 3.0,
            t0: fixed(10.5),
            rp: fixed(0.1),
            a_rs: fixed(9.0),
            b: free(0.0),
            u1: fixed(0.4),
            u2: fixed(0.25),
            ..Default::default()
        };
        let fit = fit_transit(&on_its_limit).unwrap();
        assert!(fit.covariance.is_none());
        assert_eq!(fit.correlation, None);
        for (key, value, err) in derived_pairs(&fit.derived) {
            assert!(value.is_some(), "{key}");
            assert_eq!(err, None, "error of {key}");
        }

        let radius_only = request_of(&fixture, "radius_only");
        let inside_the_transit = TransitFitRequest {
            time: radius_only.time[70..170].to_vec(),
            flux: radius_only.flux[70..170].to_vec(),
            err: Some(vec![0.3; 100]),
            ..radius_only
        };
        let fit = fit_transit(&inside_the_transit).unwrap();
        assert_eq!(fit.free_names, ["rp", "c0", "c1"]);
        let radius_with_level = fit.correlation.as_ref().unwrap()[0][1];
        assert!(radius_with_level >= STRONG_CORRELATION, "rp with c0: {radius_with_level}");
        assert!(!fit.notes.iter().any(|note| note.contains("strongly correlated")), "{:#?}", fit.notes);
    }

    #[test]
    fn the_model_curve_spans_the_series() {
        let fit = fit_transit(&request_of(&fixture(), "weighted")).unwrap();
        assert_eq!((fit.curve_time.len(), fit.curve_flux.len(), fit.curve_baseline.len()), (400, 400, 400));
        assert_close(fit.curve_time[0], fit.time[0], 1e-9, "first time of the curve");
        assert_close(fit.curve_time[399], fit.time[239], 1e-9, "last time of the curve");
        let step = (fit.time[239] - fit.time[0]) / 399.0;
        assert_close(fit.curve_time[133], fit.time[0] + 133.0 * step, 2e-9, "time 133 of the curve");
        assert_eq!(fit.curve_flux[0], fit.curve_baseline[0]);
        let deepest =
            fit.curve_flux.iter().zip(&fit.curve_baseline).map(|(flux, base)| flux / base).fold(f64::INFINITY, f64::min);
        assert!(deepest > 0.9880 && deepest < 0.9884, "deepest point of the curve: {deepest}");
        let (c0, c1) = (fit.baseline.coefficients[0], fit.baseline.coefficients[1]);
        for j in 0..400 {
            let line = c0 + c1 * (fit.curve_time[j] - fit.baseline.reference_time);
            assert_relative(fit.curve_baseline[j], line, 1e-9, &format!("baseline {j} of the curve"));
        }
    }

    #[test]
    fn the_white_light_of_wd_1856_pins_the_code_not_the_planet() {
        let request = white_light_of_wd_1856();
        assert_eq!(request.time.len(), 240);
        let started = Instant::now();
        let fit = fit_transit(&request).unwrap();
        let wall = started.elapsed();
        assert!(fit.converged, "{:?}", fit.warnings);
        assert!(fit.iterations <= 40, "{} iterations", fit.iterations);
        let t0 = param(&fit, "t0");
        assert_close(t0.value, 60061.50767036025, 1e-8, "t0");
        assert_relative(t0.err.unwrap(), 4.389048404765818e-07, 1e-3, "error of t0");
        let b = param(&fit, "b");
        assert_close(b.value, 6.663830956289091, 1e-6, "b");
        assert_relative(b.err.unwrap(), 0.000255485342006647, 1e-3, "error of b");
        assert_relative(fit.chi2, 1332913.8746611457, 1e-6, "chi2");
        assert_eq!(fit.dof, 236);
        assert_close(fit.baseline.reference_time, 60061.48462000021, 1e-9, "reference time");
        assert_relative(fit.baseline.coefficients[0], 0.16440702915045063, 1e-7, "c0");
        assert_relative(fit.baseline.coefficients[1], 0.016867719613871523, 1e-4, "c1");
        assert_relative(fit.rms, 0.006641975265208122, 1e-6, "rms");
        assert_relative(fit.rms_ppm, 40304.60373487816, 1e-6, "rms in ppm");
        assert_eq!(fit.n_in_transit, 18);
        assert_near(fit.depth, 52.998400000000004, "depth");
        assert_eq!(fit.depth_err, None);

        let derived = &fit.derived;
        assert_near(derived.inclination_deg.unwrap(), 88.86358802428627, "inclination");
        assert_relative(derived.inclination_err.unwrap(), 4.357473577781786e-05, 1e-3, "error of the inclination");
        assert_near(derived.duration_t14_days.unwrap(), 0.006556475437561907, "T14");
        assert_relative(derived.duration_t14_err.unwrap(), 4.621146097723114e-07, 1e-3, "error of T14");
        assert_eq!((derived.duration_t23_days, derived.duration_t23_err), (None, None));
        assert_near(derived.stellar_density_cgs.unwrap(), 361982.1435066046, "density");
        assert_eq!(derived.stellar_density_err, None);

        let averaging = &fit.time_averaging;
        assert_eq!(averaging.bin_points, [1, 2, 4, 8, 16, 32]);
        assert_eq!(averaging.n_bins, [240, 120, 60, 30, 15, 7]);
        assert_all_relative(
            &averaging.rms_ppm,
            &[40304.60373487816, 38413.47546521128, 31584.688088208906, 9909.3750823643, 4702.869579369219, 3367.3281022656756],
            1e-5,
            "binned rms",
        );
        assert_all_relative(
            &averaging.expected_ppm,
            &[40388.83497353427, 28619.154562020474, 20322.36650163646, 14493.433778838618, 10429.807095142738, 7695.787394594959],
            1e-5,
            "white-noise expectation",
        );
        assert_relative(averaging.beta, 1.5541835684178582, 1e-5, "beta");
        assert_eq!(
            fit.warnings,
            [
                "reduced chi2 = 5647.9: the model or the errors do not describe the scatter, so the errors of the fit are too small unless they are scaled",
                WARNING_CORRELATED_NOISE_OF_WD_1856,
            ]
        );
        let flux_deficit_centroid = 60061.50769302038;
        assert!((t0.value - flux_deficit_centroid).abs() < 3e-5, "t0 {:?} against the centroid of the dip", t0.value);
        println!(
            "WD 1856+534 b, 240 points: {} iterations in {:.1} ms; t0 {:?} +- {:?}, b {:?} +- {:?}, chi2 {:?}, c0 {:?}, c1 {:?}, rms {:?}, rms_ppm {:?}, inclination {:?} +- {:?}, T14 {:?} +- {:?}, rms rows {:?}, expected rows {:?}, beta {:?}, t0 minus the centroid {:e} d",
            fit.iterations,
            wall.as_secs_f64() * 1e3,
            t0.value,
            t0.err,
            b.value,
            b.err,
            fit.chi2,
            fit.baseline.coefficients[0],
            fit.baseline.coefficients[1],
            fit.rms,
            fit.rms_ppm,
            derived.inclination_deg,
            derived.inclination_err,
            derived.duration_t14_days,
            derived.duration_t14_err,
            averaging.rms_ppm,
            averaging.expected_ppm,
            averaging.beta,
            t0.value - flux_deficit_centroid
        );
    }

    #[test]
    fn scaled_errors_on_the_white_light_of_wd_1856() {
        let plain = fit_transit(&white_light_of_wd_1856()).unwrap();
        let scaled = fit_transit(&TransitFitRequest { scale_errors: true, ..white_light_of_wd_1856() }).unwrap();
        for (scaled, plain) in scaled.parameters.iter().zip(&plain.parameters) {
            assert_eq!(scaled.value.to_bits(), plain.value.to_bits(), "{}", plain.name);
        }
        for k in 0..2 {
            assert_eq!(scaled.baseline.coefficients[k].to_bits(), plain.baseline.coefficients[k].to_bits(), "c{k}");
        }
        assert_eq!(scaled.chi2.to_bits(), plain.chi2.to_bits());
        assert_eq!(scaled.time_averaging, plain.time_averaging);
        assert_eq!(scaled.derived.inclination_deg, plain.derived.inclination_deg);
        assert_eq!(scaled.derived.duration_t14_days, plain.derived.duration_t14_days);
        assert_relative(scaled.error_scale, 75.15277870357995, 1e-6, "error scale");
        assert_relative(param(&scaled, "t0").err.unwrap(), 3.2984918348266615e-05, 1e-3, "error of t0");
        assert_relative(param(&scaled, "b").err.unwrap(), 0.019200433369833982, 1e-3, "error of b");
        assert_relative(scaled.derived.inclination_err.unwrap(), 0.003274762474977313, 1e-3, "error of the inclination");
        assert_eq!(scaled.warnings, [WARNING_CORRELATED_NOISE_OF_WD_1856]);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/whtlt/jw01366-o002_t001_nircam_f322w2-grismr-subgrism256_whtlt.ecsv"]
    fn real_wasp_39_b_nircam_white_light_gives_the_known_geometry() {
        let path = env_dir("ASTROBURST_SCIENCE_DATA_DIR")
            .join("tso/whtlt/jw01366-o002_t001_nircam_f322w2-grismr-subgrism256_whtlt.ecsv");
        let text = read_text(&path);
        let mut lines = text.lines().filter(|line| !line.starts_with('#') && !line.trim().is_empty());
        assert_eq!(lines.next(), Some("MJD_UTC BJD_TDB whitelight_flux"));
        let rows: Vec<Vec<f64>> =
            lines.map(|line| line.split_whitespace().map(|field| field.parse().unwrap()).collect()).collect();
        assert_eq!(rows.len(), 366);
        assert!(rows.iter().all(|row| row.len() == 3));
        let time: Vec<f64> = rows.iter().map(|row| row[1]).collect();
        let flux: Vec<f64> = rows.iter().map(|row| row[2]).collect();
        assert_eq!((time[0], time[365], flux[0]), (59782.811359196894, 59783.152762977756, 709027.0040717721));
        let request = TransitFitRequest {
            time,
            flux,
            period: 4.0552941,
            t0: free(59783.0),
            rp: free(0.145),
            a_rs: free(11.0),
            b: free(0.4),
            u1: fixed(0.1),
            u2: fixed(0.1),
            baseline_order: 1,
            ..Default::default()
        };
        let started = Instant::now();
        let fit = fit_transit(&request).unwrap();
        let wall = started.elapsed();
        let derived = &fit.derived;
        let t0 = param(&fit, "t0");
        let a_rs = param(&fit, "a_rs");
        println!(
            "WASP-39 b, 366 points: {} iterations in {:.1} ms, converged {}",
            fit.iterations,
            wall.as_secs_f64() * 1e3,
            fit.converged
        );
        println!("t0 = {:?} +- {:?} BJD_TDB ({:?} as fitted)", t0.value + 2400000.5, t0.err, t0.value);
        println!("Rp/Rs = {:?} +- {:?}", param(&fit, "rp").value, param(&fit, "rp").err);
        println!("a/Rs = {:?} +- {:?}", a_rs.value, a_rs.err);
        println!("b = {:?} +- {:?}", param(&fit, "b").value, param(&fit, "b").err);
        println!("inclination = {:?} +- {:?} deg", derived.inclination_deg, derived.inclination_err);
        println!(
            "T14 = {:?} +- {:?} d ({:.3} h)",
            derived.duration_t14_days,
            derived.duration_t14_err,
            24.0 * derived.duration_t14_days.unwrap_or(f64::NAN)
        );
        println!(
            "T23 = {:?} +- {:?} d; density = {:?} +- {:?} g cm^-3; depth = {:?} +- {:?}; baseline {:?} +- {:?}; error scale {:?}; rms {:?} ppm; {} points in transit; dof {}; bins {:?}; beta {:?}",
            derived.duration_t23_days,
            derived.duration_t23_err,
            derived.stellar_density_cgs,
            derived.stellar_density_err,
            fit.depth,
            fit.depth_err,
            fit.baseline.coefficients,
            fit.baseline.errors,
            fit.error_scale,
            fit.rms_ppm,
            fit.n_in_transit,
            fit.dof,
            fit.time_averaging.n_bins,
            fit.time_averaging.beta
        );
        println!(
            "against Carter et al. 2024: t0 {:+.2} sigma, a/Rs {:+.2} sigma",
            (t0.value + 2400000.5 - 2459783.50150) / 5.8e-5,
            (a_rs.value - 11.390) / 0.079
        );

        assert!(fit.converged, "{:?}", fit.warnings);
        assert!(fit.iterations <= 40, "{} iterations", fit.iterations);
        assert_pinned(&fit, "t0", 59783.00156844534, 5.797817069603165e-05);
        assert_pinned(&fit, "rp", 0.14631781531618687, 0.00019389151383234117);
        assert_pinned(&fit, "a_rs", 11.305990674886072, 0.07894577446622646);
        assert_pinned(&fit, "b", 0.46324847401642044, 0.01185625989642625);
        assert_eq!(fit.dof, 360);
        assert_relative(fit.error_scale, 0.0004140647100252384, 1e-5, "error scale");
        assert_pinned_baseline(&fit, 0, 709840.7923599984, 19.075554124517577);
        assert_pinned_baseline(&fit, 1, -171.97056510974514, 159.47547983806075);
        assert_derived(Some(fit.depth), fit.depth_err, 0.02140890307890177, 5.6739565424592774e-05, "depth");
        assert_relative(fit.rms_ppm, 410.57681989726063, 1e-5, "rms in ppm");
        assert_eq!(fit.n_in_transit, 128);
        assert_derived(derived.inclination_deg, derived.inclination_err, 87.6517215066731, 0.07634374187832578, "inclination");
        assert_derived(derived.duration_t14_days, derived.duration_t14_err, 0.11998892206595073, 0.0002840126540319784, "T14");
        assert_derived(derived.duration_t23_days, derived.duration_t23_err, 0.08199283618595668, 0.00033735858931842745, "T23");
        assert_derived(derived.stellar_density_cgs, derived.stellar_density_err, 1.6623370172887344, 0.0348225521367094, "density");
        assert_eq!(fit.time_averaging.n_bins, [366, 183, 91, 45, 22, 11, 5]);
        assert_relative(fit.time_averaging.beta, 1.098739151161348, 1e-4, "beta");
        assert_holds(&fit.notes, "a_rs and b are strongly correlated (r = -0.985): their errors are not independent");
        assert!(fit.warnings.is_empty(), "{:?}", fit.warnings);
        assert!((t0.value + 2400000.5 - 2459783.50150).abs() < 3.0 * 5.8e-5, "t0 against Carter et al. 2024");
        assert!((a_rs.value - 11.390).abs() < 3.0 * 0.079, "a/Rs against Carter et al. 2024");
    }

    #[test]
    fn the_result_serialises_with_the_names_the_frontend_reads() {
        let fit = fit_transit(&request_of(&fixture(), "weighted")).unwrap();
        let value = serde_json::to_value(&fit).unwrap();
        let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut expected = [
            "model",
            "period",
            "parameters",
            "baseline",
            "depth",
            "depth_err",
            "chi2",
            "dof",
            "reduced_chi2",
            "weighted",
            "error_scale",
            "rms",
            "rms_ppm",
            "n_points",
            "n_in_transit",
            "iterations",
            "converged",
            "free_names",
            "time",
            "model_flux",
            "baseline_flux",
            "residual",
            "notes",
            "warnings",
            "correlation",
            "derived",
            "time_averaging",
            "curve_time",
            "curve_flux",
            "curve_baseline",
            "kind",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert_eq!(
            sorted_keys(&value["derived"]),
            [
                "duration_t14_days",
                "duration_t14_err",
                "duration_t23_days",
                "duration_t23_err",
                "inclination_deg",
                "inclination_err",
                "stellar_density_cgs",
                "stellar_density_err",
            ]
        );
        assert_eq!(
            sorted_keys(&value["time_averaging"]),
            ["beta", "bin_points", "cadence_days", "expected_ppm", "n_bins", "rms_ppm"]
        );
        assert!(value["derived"]["inclination_err"].is_number());
        assert_eq!(value["correlation"].as_array().unwrap().len(), 6);
        assert_eq!(value["curve_time"].as_array().unwrap().len(), 400);
        let unpropagated = serde_json::to_value(derived_quantities(3.0, 0.1, 9.0, 0.3, &[], None)).unwrap();
        assert!(unpropagated["inclination_deg"].is_number());
        assert!(unpropagated["inclination_err"].is_null());
        let radius = param(&fit, "rp");
        assert_eq!(
            value["parameters"][1],
            serde_json::json!({ "name": "rp", "value": radius.value, "err": radius.err.unwrap(), "free": true, "at_bound": false })
        );
        assert!(value["parameters"][4]["err"].is_null());
        assert_eq!(value["model"], TRANSIT_MODEL_NAME);
        assert_eq!(value["baseline"]["order"], 1);
        assert_eq!(value["baseline"]["coefficients"].as_array().unwrap().len(), 2);
        assert_eq!(value["baseline"]["errors"].as_array().unwrap().len(), 2);

        let request: TransitFitRequest = serde_json::from_str(
            r#"{"time":[1.0,1.5],"flux":[2.0,2.5],"period":3.0,"t0":{"value":1.0,"free":true},"rp":{"value":0.1,"free":true},"a_rs":{"value":9.0,"free":false},"b":{"value":0.3,"free":false},"u1":{"value":0.4,"free":false},"u2":{"value":0.25,"free":false}}"#,
        )
        .unwrap();
        assert_eq!(
            request,
            TransitFitRequest {
                time: vec![1.0, 1.5],
                flux: vec![2.0, 2.5],
                period: 3.0,
                t0: free(1.0),
                rp: free(0.1),
                a_rs: fixed(9.0),
                b: fixed(0.3),
                u1: fixed(0.4),
                u2: fixed(0.25),
                ..Default::default()
            }
        );
        assert_eq!(request.err, None);
        assert_eq!(request.baseline_order, 0);
        assert_eq!(request.scale_errors, false);
    }

    #[test]
    fn the_eclipse_fixture_has_its_checkpoints() {
        let fixture = eclipse_fixture();
        let time = numbers(&fixture["time"]);
        let flux = numbers(&fixture["flux"]);
        let err = numbers(&fixture["err"]);
        assert_eq!((time.len(), flux.len(), err.len()), (240, 240, 240));
        assert_close(time[0], 2460312.38, 1e-9, "time[0]");
        assert_close(flux[0], 1001.1674759706948, 1e-9, "flux[0]");
        assert_close(flux[120], 998.0269076216566, 1e-9, "flux[120]");
        assert_close(flux[239], 998.773074339563, 1e-9, "flux[239]");
        assert!(err.iter().all(|&sigma| sigma == 0.05));
    }

    #[test]
    fn the_flux_ratio_is_recovered_with_the_geometry_fixed() {
        let fixture = eclipse_fixture();
        let fit = eclipse_oracle_fit(&fixture, "ratio_only");
        assert_pinned(&fit, "flux_ratio", 0.0019958550430697096, 6.743964137874127e-06);
        assert_pinned_baseline(&fit, 0, 1000.0025992491796, 0.004297678746576689);
        assert_pinned_baseline(&fit, 1, -10.004260019602778, 0.04659309926636023);
        assert_relative(fit.chi2, 196.60237842009252, 1e-6, "chi2");
        assert_eq!(fit.dof, 237);
        assert_eq!(fit.kind, TransitKind::Eclipse);
        assert_eq!(fit.model, ECLIPSE_MODEL_NAME);
        let names: Vec<&str> = fit.parameters.iter().map(|parameter| parameter.name.as_str()).collect();
        assert_eq!(names, ECLIPSE_PARAMETER_NAMES);
        for (name, value) in [("t0", 2460312.5), ("rp", 0.1), ("a_rs", 9.0), ("b", 0.3)] {
            assert_eq!(*param(&fit, name), fixed_parameter(name, value));
        }
        assert_eq!(fit.free_names, ["flux_ratio", "c0", "c1"]);
        assert_close(fit.depth, 0.001991879540244126, 1e-3 * 6.7171245667462885e-06, "depth");
        assert_relative(fit.depth_err.unwrap(), 6.7171245667462885e-06, 1e-3, "depth error");
        let ratio = param(&fit, "flux_ratio");
        let pull = (ratio.value - fixture["truth"]["flux_ratio"].as_f64().unwrap()) / ratio.err.unwrap();
        assert!(pull.abs() < 3.0, "{ratio:?} is {pull} sigma from the injected flux ratio");
        assert!(!ratio.at_bound);
        assert_eq!((fit.n_in_transit, fit.n_points), (113, 240));
        assert!(fit.converged);
        assert!(fit.weighted);
        assert_eq!(fit.error_scale, 1.0);
        assert!(fit.warnings.is_empty(), "{:?}", fit.warnings);
        assert_eq!(fit.curve_flux[0], fit.curve_baseline[0]);
        assert_relative(fit.curve_flux[200] / fit.curve_baseline[200], 1.0 - fit.depth, 1e-12, "the curve at mid-eclipse");
        println!("ratio_only: flux_ratio {:?} +- {:?}, {pull:+.2} sigma from the injected 0.002", ratio.value, ratio.err);
    }

    #[test]
    fn the_mid_eclipse_time_can_be_fitted_too() {
        let fixture = eclipse_fixture();
        let fit = eclipse_oracle_fit(&fixture, "time_and_ratio");
        assert_pinned(&fit, "t0", 2460312.4999631895, 5.85195236311298e-05);
        assert_pinned(&fit, "flux_ratio", 0.0019958518918608394, 6.743952597016102e-06);
        assert_relative(fit.chi2, 196.21144297572187, 1e-6, "chi2");
        assert_eq!(fit.dof, 236);
        assert_eq!(fit.free_names, ["t0", "flux_ratio", "c0", "c1"]);
        assert_holds(&fit.notes, "fixed as given: rp = 0.1, a_rs = 9, b = 0.3");
        assert!(fit.converged);
        assert!(fit.warnings.is_empty(), "{:?}", fit.warnings);
    }

    #[test]
    fn eclipse_requests_that_can_not_be_fitted_are_refused() {
        let base = eclipse_request_of(&eclipse_fixture(), "ratio_only");
        assert_eq!(
            refusal_after(&base, |request| request.flux_ratio = None),
            "an eclipse fit needs flux_ratio (planet flux over star flux)"
        );
        assert_eq!(
            refusal_after(&base, |request| {
                request.flux_ratio = None;
                request.baseline_order = 3;
            }),
            "baseline order 3 must be 0, 1 or 2"
        );
        assert_eq!(
            refusal_after(&base, |request| {
                request.flux_ratio = None;
                request.flux[5] = f64::NAN;
            }),
            "an eclipse fit needs flux_ratio (planet flux over star flux)"
        );
        assert_eq!(
            refusal_after(&base, |request| request.flux_ratio = Some(free(-0.6))),
            REFUSAL_OF_A_FLUX_RATIO_BELOW_ITS_RANGE
        );
        assert_eq!(
            refusal_after(&base, |request| request.t0.value = 2460311.0),
            "no point of the series lies inside the eclipse of the starting values (t0 2460311.00000, duration 0.11261 d): check t0 and the period"
        );
        assert_eq!(
            refusal_after(&base, |request| request.b.value = 1.5),
            "no eclipse: b 1.5 is at or above 1 + rp = 1.1, so the planet never passes behind the star"
        );
        assert_eq!(
            refusal_after(&base, |request| {
                request.time.truncate(3);
                request.flux.truncate(3);
                request.err.as_mut().unwrap().truncate(3);
            }),
            "3 points for 3 free parameters: the fit needs more points than free parameters"
        );
        let with_limb_darkening_out_of_range = TransitFitRequest { u1: fixed(5.0), u2: free(f64::NAN), ..base };
        let fit = fit_transit(&with_limb_darkening_out_of_range).unwrap();
        assert_eq!(fit.free_names, ["flux_ratio", "c0", "c1"]);
    }

    #[test]
    fn the_notes_of_an_eclipse_state_its_assumptions() {
        let request = eclipse_request_of(&eclipse_fixture(), "ratio_only");
        let fit = fit_transit(&request).unwrap();
        assert_eq!(
            fit.notes,
            [
                NOTE_ECLIPSE_MODEL,
                "period 3 d given, not fitted",
                NOTE_MID_ECLIPSE_TIME,
                "baseline: polynomial of order 1 in time about 2460312.49950 (the middle of the series), fitted with the eclipse",
                "fixed as given: t0 = 2460312.5, rp = 0.1, a_rs = 9, b = 0.3",
                "weights 1 / err^2",
                "errors: 1 sigma from the covariance of the fit, not scaled (reduced chi2 = 0.830)",
                NOTE_ECLIPSE_NOT_MODELLED,
                "time-averaging test: binned residuals reach 1.01 times the white-noise expectation (beta); the errors are not enlarged by it",
                NOTE_DENSITY,
            ]
        );
        assert!(!fit.notes.iter().any(|note| note.contains("limb darkening fixed") || note.contains("u1")), "{:#?}", fit.notes);

        let two_periods_later = TransitFitRequest { t0: fixed(2460312.5 + 2.0 * 3.0), flux_ratio: Some(fixed(0.002)), ..request };
        let fit = fit_transit(&two_periods_later).unwrap();
        assert_eq!(
            fit.notes[..5],
            [
                NOTE_ECLIPSE_MODEL,
                "period 3 d given, not fitted",
                "t0 moved by 2 periods from the typed value to the eclipse nearest the middle of the series",
                NOTE_MID_ECLIPSE_TIME,
                "baseline: polynomial of order 1 in time about 2460312.49950 (the middle of the series), fitted with the eclipse",
            ]
        );
        assert_holds(&fit.notes, "fixed as given: t0 = 2460312.5, rp = 0.1, a_rs = 9, b = 0.3, flux_ratio = 0.002");
        assert_eq!(*param(&fit, "flux_ratio"), fixed_parameter("flux_ratio", 0.002));
        assert_eq!(fit.depth, 0.002 / 1.002);
        assert_eq!(fit.depth_err, None);
        assert_eq!(fit.free_names, ["c0", "c1"]);
    }

    #[test]
    fn a_brightening_gives_a_negative_ratio_and_says_so() {
        let fixture = eclipse_fixture();
        let time = numbers(&fixture["time"]);
        let shape = TransitShape { t0: 2460312.5, period: 3.0, rp: 0.1, a_rs: 9.0, b: 0.3 };
        let brightening = TransitFitRequest {
            flux: time.iter().map(|&t| 1000.0 * (2.0 - eclipse_flux(t, &shape, 0.0005))).collect(),
            time,
            period: 3.0,
            t0: fixed(2460312.5),
            rp: fixed(0.1),
            a_rs: fixed(9.0),
            b: fixed(0.3),
            u1: free(0.4),
            u2: free(0.25),
            kind: TransitKind::Eclipse,
            flux_ratio: Some(free(0.001)),
            ..Default::default()
        };
        let fit = fit_transit(&brightening).unwrap();
        let ratio = param(&fit, "flux_ratio");
        assert!(ratio.value < 0.0, "{ratio:?}");
        assert_close(ratio.value, -0.0005 / 1.001, 1e-9, "flux ratio of the brightening");
        assert_close(fit.depth, -0.0005 / 1.0005, 1e-9, "depth of the brightening");
        assert!(!ratio.at_bound);
        assert_eq!(fit.n_in_transit, 113);
        assert_holds(&fit.warnings, WARNING_NEGATIVE_FLUX_RATIO);
        assert!(!fit.warnings.iter().any(|warning| warning.contains("limb darkening")), "{:?}", fit.warnings);
        assert_eq!(fit.free_names, ["flux_ratio", "c0"]);
        let typed_negative = fit_transit(&TransitFitRequest { flux_ratio: Some(fixed(-0.0005)), ..brightening.clone() }).unwrap();
        assert!(!typed_negative.warnings.iter().any(|warning| warning == WARNING_NEGATIVE_FLUX_RATIO), "{:?}", typed_negative.warnings);

        let brighter_than_the_limit = TransitFitRequest {
            flux: brightening.time.iter().map(|&t| 1000.0 * eclipse_flux(t, &shape, 12.0)).collect(),
            flux_ratio: Some(free(5.0)),
            ..brightening
        };
        let fit = fit_transit(&brighter_than_the_limit).unwrap();
        let ratio = param(&fit, "flux_ratio");
        assert_eq!((ratio.value, ratio.at_bound), (10.0, true));
        assert_holds(&fit.warnings, "flux_ratio ended at its limit 10: it is not a free fit and its error is not meaningful");

        let ratio_only = eclipse_request_of(&fixture, "ratio_only");
        let detected = fit_transit(&ratio_only).unwrap();
        assert!(!detected.warnings.iter().any(|warning| warning == WARNING_NEGATIVE_FLUX_RATIO), "{:?}", detected.warnings);

        let inside = TransitFitRequest {
            time: ratio_only.time[70..170].to_vec(),
            flux: ratio_only.flux[70..170].to_vec(),
            err: Some(vec![0.05; 100]),
            ..ratio_only
        };
        let fit = fit_transit(&inside).unwrap();
        assert_eq!((fit.n_in_transit, fit.n_points), (100, 100));
        assert_holds(&fit.warnings, "every point lies inside the eclipse: the baseline is not constrained by points outside it");

        let larger_than_the_star = TransitShape { rp: 1.5, b: 0.2, ..shape };
        let long_time: Vec<f64> = (0..240).map(|i| 2460312.26 + 0.002 * i as f64).collect();
        let planet_larger_than_the_star = TransitFitRequest {
            flux: long_time.iter().map(|&t| 1000.0 * eclipse_flux(t, &larger_than_the_star, 0.05)).collect(),
            time: long_time,
            period: 3.0,
            t0: fixed(2460312.5),
            rp: fixed(1.5),
            a_rs: fixed(9.0),
            b: free(0.25),
            kind: TransitKind::Eclipse,
            flux_ratio: Some(free(0.04)),
            ..Default::default()
        };
        let fit = fit_transit(&planet_larger_than_the_star).unwrap();
        assert!(fit.converged, "{:?}", fit.warnings);
        assert_close(param(&fit, "flux_ratio").value, 0.05, 1e-7, "flux ratio of a planet larger than the star");
        assert_close(param(&fit, "b").value, 0.2, 1e-5, "b of a planet larger than the star");
        assert!(
            !fit.warnings.iter().any(|warning| warning.contains("cover the whole star") || warning.contains("limb darkening")),
            "{:?}",
            fit.warnings
        );
    }

    #[test]
    fn transit_requests_are_unchanged() {
        assert_eq!(TransitFitRequest::default().kind, TransitKind::Transit);
        assert_eq!(TransitFitRequest::default().flux_ratio, None);
        let fit = fit_transit(&request_of(&fixture(), "weighted")).unwrap();
        assert_eq!(fit.kind, TransitKind::Transit);
        assert_eq!(fit.parameters.len(), 6);
        assert_eq!(fit.model, TRANSIT_MODEL_NAME);
        assert_eq!(serde_json::to_value(&fit).unwrap()["kind"], "transit");
        assert_eq!(serde_json::to_value(TransitKind::Eclipse).unwrap(), "eclipse");
        assert_eq!(serde_json::to_value(TransitKind::Transit).unwrap(), "transit");

        let request: TransitFitRequest = serde_json::from_str(
            r#"{"time":[1.0,1.5],"flux":[2.0,2.5],"period":3.0,"t0":{"value":1.0,"free":false},"rp":{"value":0.1,"free":false},"a_rs":{"value":9.0,"free":false},"b":{"value":0.3,"free":false},"u1":{"value":0.0,"free":false},"u2":{"value":0.0,"free":false},"kind":"eclipse","flux_ratio":{"value":0.001,"free":true}}"#,
        )
        .unwrap();
        assert_eq!(request.kind, TransitKind::Eclipse);
        assert_eq!(request.flux_ratio, Some(free(0.001)));

        let eclipse = fit_transit(&eclipse_request_of(&eclipse_fixture(), "ratio_only")).unwrap();
        let value = serde_json::to_value(&eclipse).unwrap();
        assert_eq!(value["kind"], "eclipse");
        assert_eq!(value["model"], ECLIPSE_MODEL_NAME);
        assert_eq!(value["parameters"].as_array().unwrap().len(), 5);
        let ratio = param(&eclipse, "flux_ratio");
        assert_eq!(
            value["parameters"][4],
            serde_json::json!({ "name": "flux_ratio", "value": ratio.value, "err": ratio.err.unwrap(), "free": true, "at_bound": false })
        );
    }
}
