pub const LAMBDA_START: f64 = 1e-3;
pub const LAMBDA_GROWTH: f64 = 10.0;
pub const LAMBDA_MAX: f64 = 1e12;
pub const CHI2_TOLERANCE: f64 = 1e-10;
pub const STEP_TOLERANCE: f64 = 1e-8;

const PIVOT_EPSILON: f64 = 1e-300;

#[derive(Debug, Clone, PartialEq)]
pub struct FitResult {
    pub params: Vec<f64>,
    pub errors: Vec<f64>,
    pub chi2: f64,
    pub dof: usize,
    pub iterations: usize,
    pub converged: bool,
}

fn solve_linear(matrix: &mut [Vec<f64>], rhs: &mut [f64]) -> Option<Vec<f64>> {
    let n = rhs.len();
    for col in 0..n {
        let pivot_row = (col..n)
            .filter(|&r| matrix[r][col].is_finite())
            .max_by(|&a, &b| matrix[a][col].abs().total_cmp(&matrix[b][col].abs()))?;
        if matrix[pivot_row][col].abs() <= PIVOT_EPSILON {
            return None;
        }
        matrix.swap(col, pivot_row);
        rhs.swap(col, pivot_row);
        let pivot = matrix[col][col];
        for row in col + 1..n {
            let factor = matrix[row][col] / pivot;
            if factor == 0.0 {
                continue;
            }
            for k in col..n {
                matrix[row][k] -= factor * matrix[col][k];
            }
            rhs[row] -= factor * rhs[col];
        }
    }
    let mut solution = vec![0.0; n];
    for row in (0..n).rev() {
        let mut acc = rhs[row];
        for k in row + 1..n {
            acc -= matrix[row][k] * solution[k];
        }
        solution[row] = acc / matrix[row][row];
    }
    solution.iter().all(|v| v.is_finite()).then_some(solution)
}

fn invert(matrix: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = matrix.len();
    let mut columns = Vec::with_capacity(n);
    for col in 0..n {
        let mut work: Vec<Vec<f64>> = matrix.to_vec();
        let mut unit = vec![0.0; n];
        unit[col] = 1.0;
        columns.push(solve_linear(&mut work, &mut unit)?);
    }
    let mut inverse = vec![vec![0.0; n]; n];
    for (col, column) in columns.iter().enumerate() {
        for row in 0..n {
            inverse[row][col] = column[row];
        }
    }
    Some(inverse)
}

fn clamp_params(params: &mut [f64], lower: &[f64], upper: &[f64]) {
    for (i, p) in params.iter_mut().enumerate() {
        if *p < lower[i] {
            *p = lower[i];
        }
        if *p > upper[i] {
            *p = upper[i];
        }
    }
}

struct Linearisation {
    chi2: f64,
    normal: Vec<Vec<f64>>,
    gradient: Vec<f64>,
}

fn linearise<F>(x: &[f64], y: &[f64], weights: &[f64], params: &[f64], model: &F) -> Option<Linearisation>
where
    F: Fn(f64, &[f64], &mut [f64]) -> f64,
{
    let np = params.len();
    let mut normal = vec![vec![0.0; np]; np];
    let mut gradient = vec![0.0; np];
    let mut row = vec![0.0; np];
    let mut chi2 = 0.0;
    for i in 0..x.len() {
        let w = weights[i];
        if !(w > 0.0) || !y[i].is_finite() {
            continue;
        }
        row.iter_mut().for_each(|v| *v = 0.0);
        let value = model(x[i], params, &mut row);
        if !value.is_finite() || row.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let r = y[i] - value;
        chi2 += w * r * r;
        for a in 0..np {
            gradient[a] += w * row[a] * r;
            for b in a..np {
                normal[a][b] += w * row[a] * row[b];
            }
        }
    }
    for a in 0..np {
        for b in 0..a {
            normal[a][b] = normal[b][a];
        }
    }
    chi2.is_finite().then_some(Linearisation { chi2, normal, gradient })
}

fn chi2_of<F>(x: &[f64], y: &[f64], weights: &[f64], params: &[f64], model: &F) -> f64
where
    F: Fn(f64, &[f64], &mut [f64]) -> f64,
{
    let mut row = vec![0.0; params.len()];
    let mut chi2 = 0.0;
    for i in 0..x.len() {
        let w = weights[i];
        if !(w > 0.0) || !y[i].is_finite() {
            continue;
        }
        let r = y[i] - model(x[i], params, &mut row);
        chi2 += w * r * r;
    }
    chi2
}

fn norm(values: &[f64]) -> f64 {
    values.iter().map(|v| v * v).sum::<f64>().sqrt()
}

fn formal_errors(normal: &[Vec<f64>], chi2: f64, dof: usize) -> Vec<f64> {
    let np = normal.len();
    let scale = if dof > 0 { chi2 / dof as f64 } else { 1.0 };
    match invert(normal) {
        Some(inverse) => (0..np)
            .map(|i| {
                let variance = inverse[i][i] * scale;
                if variance >= 0.0 {
                    variance.sqrt()
                } else {
                    f64::NAN
                }
            })
            .collect(),
        None => vec![f64::NAN; np],
    }
}

fn validate(x: &[f64], y: &[f64], weights: &[f64], p0: &[f64], lower: &[f64], upper: &[f64]) -> Result<usize, String> {
    let np = p0.len();
    if np == 0 {
        return Err("least-squares fit needs at least one parameter".to_string());
    }
    if x.len() != y.len() || x.len() != weights.len() {
        return Err(format!(
            "least-squares fit needs x, y and weights of the same length, got {}, {} and {}",
            x.len(),
            y.len(),
            weights.len()
        ));
    }
    if lower.len() != np || upper.len() != np {
        return Err(format!(
            "least-squares bounds must have one entry per parameter, got {} lower and {} upper for {} parameters",
            lower.len(),
            upper.len(),
            np
        ));
    }
    if p0.iter().any(|p| !p.is_finite()) {
        return Err(format!("least-squares initial parameters must be finite, got {:?}", p0));
    }
    if lower.iter().zip(upper).any(|(lo, hi)| lo.is_nan() || hi.is_nan() || lo > hi) {
        return Err(format!("least-squares bounds must satisfy lower <= upper, got {:?} and {:?}", lower, upper));
    }
    let usable = x
        .iter()
        .zip(y)
        .zip(weights)
        .filter(|((xi, yi), w)| xi.is_finite() && yi.is_finite() && **w > 0.0 && w.is_finite())
        .count();
    if usable <= np {
        return Err(format!(
            "least-squares fit needs more than {} usable samples for {} parameters, got {}",
            np, np, usable
        ));
    }
    Ok(usable)
}

pub fn fit_least_squares<F>(
    x: &[f64],
    y: &[f64],
    weights: &[f64],
    p0: &[f64],
    lower: &[f64],
    upper: &[f64],
    max_iter: usize,
    model: F,
) -> Result<FitResult, String>
where
    F: Fn(f64, &[f64], &mut [f64]) -> f64,
{
    let usable = validate(x, y, weights, p0, lower, upper)?;
    let np = p0.len();
    let dof = usable - np;
    let mut params = p0.to_vec();
    clamp_params(&mut params, lower, upper);
    let mut lambda = LAMBDA_START;
    let mut iterations = 0usize;
    let mut converged = false;

    let mut current = match linearise(x, y, weights, &params, &model) {
        Some(lin) => lin,
        None => {
            return Ok(FitResult {
                params,
                errors: vec![f64::NAN; np],
                chi2: f64::NAN,
                dof,
                iterations,
                converged,
            })
        }
    };

    while iterations < max_iter && lambda <= LAMBDA_MAX {
        iterations += 1;
        let mut damped = current.normal.clone();
        for (i, row) in damped.iter_mut().enumerate() {
            row[i] *= 1.0 + lambda;
        }
        let mut rhs = current.gradient.clone();
        let Some(step) = solve_linear(&mut damped, &mut rhs) else {
            lambda *= LAMBDA_GROWTH;
            continue;
        };
        let mut trial: Vec<f64> = params.iter().zip(&step).map(|(p, s)| p + s).collect();
        clamp_params(&mut trial, lower, upper);
        let trial_chi2 = chi2_of(x, y, weights, &trial, &model);
        let accepted = (trial_chi2.is_finite() && trial_chi2 <= current.chi2)
            .then(|| linearise(x, y, weights, &trial, &model))
            .flatten();
        let Some(next) = accepted else {
            lambda *= LAMBDA_GROWTH;
            continue;
        };
        let actual_step: Vec<f64> = trial.iter().zip(&params).map(|(t, p)| t - p).collect();
        let improvement = current.chi2 - trial_chi2;
        let small_chi2 = improvement <= CHI2_TOLERANCE * current.chi2;
        let small_step = norm(&actual_step) <= STEP_TOLERANCE * norm(&trial);
        params = trial;
        current = next;
        lambda /= LAMBDA_GROWTH;
        if small_chi2 || small_step {
            converged = true;
            break;
        }
    }

    let errors = formal_errors(&current.normal, current.chi2, dof);
    Ok(FitResult { params, errors, chi2: current.chi2, dof, iterations, converged })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRUE_PARAMS: [f64; 3] = [1.0, 1.020, 0.003];
    const START_PARAMS: [f64; 3] = [0.8, 1.018, 0.004];

    fn gaussian(x: f64, p: &[f64], jac: &mut [f64]) -> f64 {
        let d = x - p[1];
        let s2 = p[2] * p[2];
        let e = (-d * d / (2.0 * s2)).exp();
        jac[0] = e;
        jac[1] = p[0] * e * d / s2;
        jac[2] = p[0] * e * d * d / (s2 * p[2]);
        p[0] * e
    }

    fn sample_axis() -> Vec<f64> {
        (0..40).map(|i| 1.0 + i as f64 * 0.001).collect()
    }

    fn sample_noise(i: usize, amplitude: f64) -> f64 {
        crate::core::cube::lazy::test_support::deterministic_noise(i, 3, 5, amplitude as f32) as f64
    }

    fn fit_gaussian(y: &[f64], weights: &[f64], lower: [f64; 3], upper: [f64; 3]) -> FitResult {
        let x = sample_axis();
        fit_least_squares(&x, y, weights, &START_PARAMS, &lower, &upper, 50, gaussian).unwrap()
    }

    #[test]
    fn a_noiseless_gaussian_is_recovered_exactly() {
        let x = sample_axis();
        let mut row = [0.0; 3];
        let y: Vec<f64> = x.iter().map(|&xi| gaussian(xi, &TRUE_PARAMS, &mut row)).collect();
        let weights = vec![1.0; y.len()];
        let fit = fit_gaussian(&y, &weights, [0.0, 1.0, 0.0002], [f64::INFINITY, 1.04, 0.05]);
        assert!(fit.converged, "{:?}", fit);
        for (found, expected) in fit.params.iter().zip(TRUE_PARAMS) {
            assert!((found - expected).abs() < 1e-6, "{:?}", fit);
        }
        assert!(fit.chi2 < 1e-12);
        assert_eq!(fit.dof, 37);
        assert!(fit.iterations < 50);
        assert!(fit.errors.iter().all(|e| e.is_finite() && *e < 1e-5));
    }

    #[test]
    fn a_noisy_gaussian_is_recovered_within_three_formal_sigma() {
        let x = sample_axis();
        let mut row = [0.0; 3];
        let amplitude = 0.05;
        let y: Vec<f64> = x
            .iter()
            .enumerate()
            .map(|(i, &xi)| gaussian(xi, &TRUE_PARAMS, &mut row) + sample_noise(i, amplitude))
            .collect();
        let sigma = amplitude / 3f64.sqrt();
        let weights = vec![1.0 / (sigma * sigma); y.len()];
        let fit = fit_gaussian(&y, &weights, [0.0, 1.0, 0.0002], [f64::INFINITY, 1.04, 0.05]);
        assert!(fit.converged, "{:?}", fit);
        for k in 0..3 {
            let pull = (fit.params[k] - TRUE_PARAMS[k]).abs() / fit.errors[k];
            assert!(fit.errors[k] > 0.0 && pull < 3.0, "parameter {} pull {} in {:?}", k, pull, fit);
        }
        let reduced = fit.chi2 / fit.dof as f64;
        assert!(reduced > 0.3 && reduced < 3.0, "reduced chi2 {}", reduced);
    }

    #[test]
    fn a_flat_input_reports_no_convergence_without_panicking() {
        let x = sample_axis();
        let y = vec![1.0; x.len()];
        let weights = vec![1.0; x.len()];
        let fit = fit_least_squares(
            &x,
            &y,
            &weights,
            &[0.0, 1.018, 0.004],
            &[0.0, 1.0, 0.0002],
            &[f64::INFINITY, 1.04, 0.05],
            50,
            gaussian,
        )
        .unwrap();
        assert!(!fit.converged, "{:?}", fit);
        assert!(fit.iterations <= 50);
        assert!(fit.params.iter().all(|p| p.is_finite()));
    }

    #[test]
    fn bounds_are_respected_on_every_parameter() {
        let x = sample_axis();
        let mut row = [0.0; 3];
        let y: Vec<f64> = x.iter().map(|&xi| gaussian(xi, &TRUE_PARAMS, &mut row)).collect();
        let weights = vec![1.0; y.len()];
        let fit = fit_gaussian(&y, &weights, [0.0, 1.0, 0.0035], [0.9, 1.019, 0.05]);
        assert!(fit.params[0] <= 0.9 + 1e-12, "{:?}", fit);
        assert!(fit.params[1] <= 1.019 + 1e-12, "{:?}", fit);
        assert!(fit.params[2] >= 0.0035 - 1e-12, "{:?}", fit);
        assert!(fit.params.iter().all(|p| p.is_finite()));
    }

    #[test]
    fn invalid_inputs_are_refused_with_a_sentence() {
        let x = sample_axis();
        let y = vec![1.0; 3];
        let w = vec![1.0; x.len()];
        let err = fit_least_squares(&x, &y, &w, &START_PARAMS, &[0.0; 3], &[1.0; 3], 10, gaussian).unwrap_err();
        assert!(err.contains("same length"), "{}", err);
        let y = vec![1.0; x.len()];
        let err = fit_least_squares(&x, &y, &w, &START_PARAMS, &[2.0; 3], &[1.0; 3], 10, gaussian).unwrap_err();
        assert!(err.contains("lower <= upper"), "{}", err);
        let err = fit_least_squares(&x[..2], &y[..2], &w[..2], &START_PARAMS, &[0.0; 3], &[2.0; 3], 10, gaussian)
            .unwrap_err();
        assert!(err.contains("usable samples"), "{}", err);
        let err = fit_least_squares(&x, &y, &w, &[f64::NAN, 1.0, 1.0], &[0.0; 3], &[2.0; 3], 10, gaussian).unwrap_err();
        assert!(err.contains("finite"), "{}", err);
    }

    #[test]
    fn the_linear_solver_pivots_and_detects_singular_systems() {
        let mut matrix = vec![vec![0.0, 2.0], vec![3.0, 1.0]];
        let mut rhs = vec![4.0, 7.0];
        let solution = solve_linear(&mut matrix, &mut rhs).unwrap();
        assert!((solution[0] - 5.0 / 3.0).abs() < 1e-12 && (solution[1] - 2.0).abs() < 1e-12, "{:?}", solution);
        let mut singular = vec![vec![1.0, 2.0], vec![2.0, 4.0]];
        let mut rhs = vec![1.0, 2.0];
        assert!(solve_linear(&mut singular, &mut rhs).is_none());
        let inverse = invert(&[vec![4.0, 7.0], vec![2.0, 6.0]]).unwrap();
        assert!((inverse[0][0] - 0.6).abs() < 1e-12 && (inverse[1][0] + 0.2).abs() < 1e-12, "{:?}", inverse);
    }
}
