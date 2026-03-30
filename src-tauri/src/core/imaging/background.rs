use anyhow::{Context, Result};
use ndarray::{Array2};
use rayon::prelude::*;

use crate::infra::progress::ProgressHandle;
use crate::math::median::{median_f32_mut};
use crate::types::error::AppError;

const MAX_POLY_TERMS: usize = 21;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct BackgroundConfig {
    pub grid_size: usize,
    pub poly_degree: usize,
    pub sigma_clip: f32,
    pub iterations: usize,
    pub mode: BackgroundMode,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundMode {
    Subtract,
    Divide,
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self {
            grid_size: 8,
            poly_degree: 3,
            sigma_clip: 2.5,
            iterations: 3,
            mode: BackgroundMode::Subtract,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BackgroundResult {
    pub model: Array2<f32>,
    pub corrected: Array2<f32>,
    pub sample_count: usize,
    pub rms_residual: f64,
    pub elapsed_ms: u64,
}

struct SamplePoint {
    y: f32,
    x: f32,
    value: f32,
}

pub fn extract_background(
    image: &Array2<f32>,
    config: &BackgroundConfig,
    progress: Option<&ProgressHandle>,
) -> Result<BackgroundResult> {
    let start = std::time::Instant::now();
    let (rows, cols) = image.dim();

    if let Some(p) = progress {
        p.set_total(4);
        p.tick_with_stage("sampling background");
    }

    let samples = auto_sample_grid(image, config)?;
    let sample_count = samples.len();

    if sample_count < min_samples_for_degree(config.poly_degree) {
        anyhow::bail!(
            "Not enough background samples ({}) for polynomial degree {}",
            sample_count,
            config.poly_degree
        );
    }

    if let Some(p) = progress {
        if p.is_cancelled() {
            return Err(AppError::Cancelled.into());
        }
        p.tick_with_stage("fitting polynomial surface");
    }

    let coeffs = fit_polynomial_surface(&samples, rows, cols, config)?;

    if let Some(p) = progress {
        if p.is_cancelled() {
            return Err(AppError::Cancelled.into());
        }
        p.tick_with_stage("generating model");
    }

    let model = evaluate_polynomial_surface(&coeffs, rows, cols, config.poly_degree);

    if let Some(p) = progress {
        p.tick_with_stage("applying correction");
    }

    let corrected = apply_correction(image, &model, &config.mode);

    let rms_residual = compute_rms_residual(&samples, &coeffs, rows, cols, config.poly_degree);

    if let Some(p) = progress {
        p.emit_complete();
    }

    Ok(BackgroundResult {
        model,
        corrected,
        sample_count,
        rms_residual,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

fn auto_sample_grid(
    image: &Array2<f32>,
    config: &BackgroundConfig,
) -> Result<Vec<SamplePoint>> {
    let (rows, cols) = image.dim();
    let grid = config.grid_size;
    let cell_h = rows / grid;
    let cell_w = cols / grid;

    if cell_h < 4 || cell_w < 4 {
        anyhow::bail!("Image too small for grid_size={}", grid);
    }

    let margin_h = cell_h / 4;
    let margin_w = cell_w / 4;
    let inner_h = cell_h - 2 * margin_h;
    let inner_w = cell_w - 2 * margin_w;

    let mut all_pixels: Vec<f32> = image
        .as_slice()
        .context("Image not contiguous")?
        .iter()
        .filter(|v| v.is_finite() && **v > 0.0)
        .copied()
        .collect();

    let global_median = median_f32_mut(&mut all_pixels);
    let mut devs: Vec<f32> = all_pixels.iter().map(|v| (v - global_median).abs()).collect();
    let global_mad = median_f32_mut(&mut devs);
    let sigma = global_mad * 1.4826;

    let mut samples = Vec::with_capacity(grid * grid);

    for gy in 0..grid {
        for gx in 0..grid {
            let y0 = gy * cell_h + margin_h;
            let x0 = gx * cell_w + margin_w;

            let mut cell_pixels = Vec::with_capacity(inner_h * inner_w);
            let mut zero_count = 0usize;
            let total_cell = inner_h * inner_w;
            for y in y0..y0 + inner_h {
                for x in x0..x0 + inner_w {
                    if y < rows && x < cols {
                        let v = image[[y, x]];
                        if v.is_finite() && v > 1e-7 {
                            cell_pixels.push(v);
                        } else {
                            zero_count += 1;
                        }
                    }
                }
            }

            if cell_pixels.is_empty() || zero_count as f64 / total_cell as f64 > 0.3 {
                continue;
            }

            let cell_median = median_f32_mut(&mut cell_pixels);

            let lo = global_median - config.sigma_clip * sigma;
            let hi = global_median + config.sigma_clip * sigma;

            if cell_median >= lo && cell_median <= hi {
                let cy = (y0 + inner_h / 2) as f32;
                let cx = (x0 + inner_w / 2) as f32;
                samples.push(SamplePoint {
                    y: cy,
                    x: cx,
                    value: cell_median,
                });
            }
        }
    }

    for _iter in 1..config.iterations {
        if samples.len() < min_samples_for_degree(config.poly_degree) {
            break;
        }

        let mut values: Vec<f32> = samples.iter().map(|s| s.value).collect();
        let med = median_f32_mut(&mut values);
        let mut devs: Vec<f32> = values.iter().map(|v| (v - med).abs()).collect();
        let mad = median_f32_mut(&mut devs);
        let sig = mad * 1.4826;
        let lo = med - config.sigma_clip * sig;
        let hi = med + config.sigma_clip * sig;

        samples.retain(|s| s.value >= lo && s.value <= hi);
    }

    Ok(samples)
}

fn min_samples_for_degree(degree: usize) -> usize {
    let n_terms = (degree + 1) * (degree + 2) / 2;
    n_terms + 2
}

#[inline]
fn poly_basis_into(y: f64, x: f64, degree: usize, out: &mut [f64; MAX_POLY_TERMS]) -> usize {
    let mut idx = 0;
    for total_deg in 0..=degree {
        for y_pow in (0..=total_deg).rev() {
            let x_pow = total_deg - y_pow;
            out[idx] = y.powi(y_pow as i32) * x.powi(x_pow as i32);
            idx += 1;
        }
    }
    idx
}

#[inline]
fn eval_poly_inline(
    _ny: f64,
    _nx: f64,
    degree: usize,
    coeffs: &[f64],
    y_pows: &[f64; 7],
    x_pows: &[f64; 7],
) -> f64 {
    let mut val = 0.0f64;
    let mut idx = 0;
    for total_deg in 0..=degree {
        for y_pow in (0..=total_deg).rev() {
            let x_pow = total_deg - y_pow;
            val += coeffs[idx] * y_pows[y_pow] * x_pows[x_pow];
            idx += 1;
        }
    }
    val
}

fn fit_polynomial_surface(
    samples: &[SamplePoint],
    rows: usize,
    cols: usize,
    config: &BackgroundConfig,
) -> Result<Vec<f64>> {
    let degree = config.poly_degree;
    let n_terms = (degree + 1) * (degree + 2) / 2;

    let row_scale = rows as f64;
    let col_scale = cols as f64;

    let mut ata = vec![0.0f64; n_terms * n_terms];
    let mut atb = vec![0.0f64; n_terms];
    let mut basis_buf = [0.0f64; MAX_POLY_TERMS];

    for sample in samples {
        let ny = sample.y as f64 / row_scale - 0.5;
        let nx = sample.x as f64 / col_scale - 0.5;
        let val = sample.value as f64;

        let count = poly_basis_into(ny, nx, degree, &mut basis_buf);

        for i in 0..count {
            atb[i] += basis_buf[i] * val;
            for j in 0..count {
                ata[i * n_terms + j] += basis_buf[i] * basis_buf[j];
            }
        }
    }

    for i in 0..n_terms {
        ata[i * n_terms + i] += 1e-8;
    }

    solve_linear_system(&mut ata, &mut atb, n_terms)
        .context("Failed to solve polynomial fit")?;

    Ok(atb)
}

#[cfg(test)]
fn poly_basis(y: f64, x: f64, degree: usize) -> Vec<f64> {
    let n_terms = (degree + 1) * (degree + 2) / 2;
    let mut basis = Vec::with_capacity(n_terms);

    for total_deg in 0..=degree {
        for y_pow in (0..=total_deg).rev() {
            let x_pow = total_deg - y_pow;
            basis.push(y.powi(y_pow as i32) * x.powi(x_pow as i32));
        }
    }

    basis
}

fn evaluate_polynomial_surface(
    coeffs: &[f64],
    rows: usize,
    cols: usize,
    degree: usize,
) -> Array2<f32> {
    let row_scale = rows as f64;
    let col_scale = cols as f64;

    let result: Vec<f32> = (0..rows)
        .into_par_iter()
        .flat_map(|y| {
            let ny = y as f64 / row_scale - 0.5;
            let mut y_pows = [0.0f64; 7];
            y_pows[0] = 1.0;
            for i in 1..=degree.min(6) {
                y_pows[i] = y_pows[i - 1] * ny;
            }

            (0..cols)
                .map(|x| {
                    let nx = x as f64 / col_scale - 0.5;
                    let mut x_pows = [0.0f64; 7];
                    x_pows[0] = 1.0;
                    for i in 1..=degree.min(6) {
                        x_pows[i] = x_pows[i - 1] * nx;
                    }
                    eval_poly_inline(ny, nx, degree, coeffs, &y_pows, &x_pows) as f32
                })
                .collect::<Vec<f32>>()
        })
        .collect();

    Array2::from_shape_vec((rows, cols), result).unwrap()
}

fn apply_correction(
    image: &Array2<f32>,
    model: &Array2<f32>,
    mode: &BackgroundMode,
) -> Array2<f32> {
    let (rows, cols) = image.dim();

    let mut finite_vals: Vec<f32> = model
        .as_slice()
        .unwrap()
        .iter()
        .filter(|v| v.is_finite() && **v > 0.0)
        .copied()
        .collect();
    let model_median = if finite_vals.is_empty() {
        0.0f32
    } else {
        median_f32_mut(&mut finite_vals)
    };

    let result: Vec<f32> = image
        .as_slice()
        .unwrap()
        .par_iter()
        .zip(model.as_slice().unwrap().par_iter())
        .map(|(&img, &bg)| match mode {
            BackgroundMode::Subtract => {
                img - bg + model_median
            }
            BackgroundMode::Divide => {
                if bg.abs() > 1e-10 {
                    (img / bg) * model_median
                } else {
                    img
                }
            }
        })
        .collect();

    Array2::from_shape_vec((rows, cols), result).unwrap()
}

fn compute_rms_residual(
    samples: &[SamplePoint],
    coeffs: &[f64],
    rows: usize,
    cols: usize,
    degree: usize,
) -> f64 {
    let row_scale = rows as f64;
    let col_scale = cols as f64;

    let sum_sq: f64 = samples
        .iter()
        .map(|s| {
            let ny = s.y as f64 / row_scale - 0.5;
            let nx = s.x as f64 / col_scale - 0.5;
            let mut y_pows = [0.0f64; 7];
            let mut x_pows = [0.0f64; 7];
            y_pows[0] = 1.0;
            x_pows[0] = 1.0;
            for i in 1..=degree.min(6) {
                y_pows[i] = y_pows[i - 1] * ny;
                x_pows[i] = x_pows[i - 1] * nx;
            }
            let predicted = eval_poly_inline(ny, nx, degree, coeffs, &y_pows, &x_pows);
            let diff = s.value as f64 - predicted;
            diff * diff
        })
        .sum();

    (sum_sq / samples.len() as f64).sqrt()
}

fn solve_linear_system(a: &mut [f64], b: &mut [f64], n: usize) -> Result<()> {
    for col in 0..n {
        let mut max_row = col;
        let mut max_val = a[col * n + col].abs();
        for row in (col + 1)..n {
            let v = a[row * n + col].abs();
            if v > max_val {
                max_val = v;
                max_row = row;
            }
        }

        if max_val < 1e-14 {
            anyhow::bail!("Singular matrix in polynomial fit");
        }

        if max_row != col {
            for k in 0..n {
                a.swap(col * n + k, max_row * n + k);
            }
            b.swap(col, max_row);
        }

        let pivot = a[col * n + col];
        for row in (col + 1)..n {
            let factor = a[row * n + col] / pivot;
            for k in col..n {
                a[row * n + k] -= factor * a[col * n + k];
            }
            b[row] -= factor * b[col];
        }
    }

    for col in (0..n).rev() {
        let mut sum = b[col];
        for k in (col + 1)..n {
            sum -= a[col * n + k] * b[k];
        }
        b[col] = sum / a[col * n + col];
    }

    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_poly_basis_degree_1() {
        let b = poly_basis(0.5, 0.3, 1);
        assert_eq!(b.len(), 3);
        assert!((b[0] - 1.0).abs() < 1e-10);
        assert!((b[1] - 0.5).abs() < 1e-10);
        assert!((b[2] - 0.3).abs() < 1e-10);
    }

    #[test]
    fn test_poly_basis_into_matches_alloc() {
        let y = 0.5;
        let x = 0.3;
        for degree in 1..=5 {
            let alloc = poly_basis(y, x, degree);
            let mut buf = [0.0f64; MAX_POLY_TERMS];
            let mut y_pows = [0.0f64; 7];
            let mut x_pows = [0.0f64; 7];
            y_pows[0] = 1.0;
            x_pows[0] = 1.0;
            for i in 1..=degree {
                y_pows[i] = y_pows[i - 1] * y;
                x_pows[i] = x_pows[i - 1] * x;
            }
            let val_alloc: f64 = alloc.iter().enumerate().map(|(i, &b)| b * (i as f64 + 1.0)).sum();
            let coeffs: Vec<f64> = (0..alloc.len()).map(|i| (i as f64 + 1.0)).collect();
            let val_inline = eval_poly_inline(y, x, degree, &coeffs, &y_pows, &x_pows);
            assert!((val_alloc - val_inline).abs() < 1e-10, "Mismatch at degree {}", degree);
        }
    }

    #[test]
    fn test_poly_basis_degree_2() {
        let b = poly_basis(0.5, 0.3, 2);
        assert_eq!(b.len(), 6);
    }

    #[test]
    fn test_poly_basis_degree_3() {
        let b = poly_basis(0.5, 0.3, 3);
        assert_eq!(b.len(), 10);
    }

    #[test]
    fn test_flat_background_extraction() {
        let rows = 64;
        let cols = 64;
        let bg_level = 100.0f32;
        let image = Array2::from_elem((rows, cols), bg_level);

        let config = BackgroundConfig {
            grid_size: 4,
            poly_degree: 1,
            sigma_clip: 3.0,
            iterations: 2,
            mode: BackgroundMode::Subtract,
        };

        let result = extract_background(&image, &config, None).unwrap();
        assert!(result.sample_count > 0);

        for y in 10..rows - 10 {
            for x in 10..cols - 10 {
                assert!(
                    result.corrected[[y, x]].abs() < 1.0,
                    "Corrected pixel at ({},{}) = {} should be near 0",
                    y, x, result.corrected[[y, x]]
                );
            }
        }
    }

    #[test]
    fn test_gradient_removal() {
        let rows = 128;
        let cols = 128;
        let mut image = Array2::zeros((rows, cols));
        for y in 0..rows {
            for x in 0..cols {
                let gradient = (y as f32 / rows as f32) * 50.0 + 100.0;
                image[[y, x]] = gradient;
            }
        }

        let config = BackgroundConfig {
            grid_size: 6,
            poly_degree: 1,
            sigma_clip: 3.0,
            iterations: 2,
            mode: BackgroundMode::Subtract,
        };

        let result = extract_background(&image, &config, None).unwrap();

        let mut values: Vec<f32> = Vec::new();
        for y in 10..rows - 10 {
            for x in 10..cols - 10 {
                values.push(result.corrected[[y, x]]);
            }
        }
        let mean: f32 = values.iter().sum::<f32>() / values.len() as f32;
        let stddev: f32 = (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>()
            / values.len() as f32)
            .sqrt();

        assert!(
            stddev < 5.0,
            "After gradient removal stddev should be small, got {}",
            stddev
        );
    }

    #[test]
    fn test_solve_linear_2x2() {
        let mut a = vec![2.0, 1.0, 5.0, 7.0];
        let mut b = vec![11.0, 13.0];
        solve_linear_system(&mut a, &mut b, 2).unwrap();
        assert!((b[0] - 7.444).abs() < 0.01);
        assert!((b[1] + 3.888).abs() < 0.01);
    }

    #[test]
    fn test_fast_median() {
        let mut v1 = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((median_f32_mut(&mut v1) - 3.0).abs() < 1e-6);
        let mut v2 = vec![1.0, 2.0, 3.0, 4.0];
        assert!((median_f32_mut(&mut v2) - 2.5).abs() < 1e-6);
    }
}
