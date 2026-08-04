use anyhow::{Context, Result};
use ndarray::{Array2};
use rayon::prelude::*;

use crate::infra::progress::ProgressHandle;
use crate::math::median::{median_f32_mut};
use crate::math::sigma_clipped_stats;
use crate::types::constants::MAD_TO_SIGMA;
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

#[derive(Debug, Clone)]
pub struct LinkedBackgroundResult {
    pub model: Array2<f32>,
    pub corrected: Vec<Array2<f32>>,
    pub sample_count: usize,
    pub rms_residual: f64,
}

#[derive(Debug, Clone)]
pub struct NeutralizeResult {
    pub corrected: Array2<f32>,
    pub level: f32,
    pub sample_count: usize,
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

    if config.poly_degree < crate::types::constants::MIN_POLY_DEGREE
        || config.poly_degree > crate::types::constants::MAX_POLY_DEGREE
    {
        anyhow::bail!(
            "Polynomial degree {} outside supported range [{}, {}]",
            config.poly_degree,
            crate::types::constants::MIN_POLY_DEGREE,
            crate::types::constants::MAX_POLY_DEGREE
        );
    }

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

pub fn extract_background_linked(
    channels: &[&Array2<f32>],
    reference: &Array2<f32>,
    config: &BackgroundConfig,
) -> Result<LinkedBackgroundResult> {
    if config.poly_degree < crate::types::constants::MIN_POLY_DEGREE
        || config.poly_degree > crate::types::constants::MAX_POLY_DEGREE
    {
        anyhow::bail!(
            "Polynomial degree {} outside supported range [{}, {}]",
            config.poly_degree,
            crate::types::constants::MIN_POLY_DEGREE,
            crate::types::constants::MAX_POLY_DEGREE
        );
    }

    let (rows, cols) = reference.dim();
    for ch in channels {
        if ch.dim() != (rows, cols) {
            anyhow::bail!(
                "Linked background: channel dim {:?} does not match reference {:?}",
                ch.dim(),
                (rows, cols)
            );
        }
    }

    let samples = auto_sample_grid(reference, config)?;
    let sample_count = samples.len();
    if sample_count < min_samples_for_degree(config.poly_degree) {
        anyhow::bail!(
            "Not enough background samples ({}) for polynomial degree {}",
            sample_count,
            config.poly_degree
        );
    }

    let coeffs = fit_polynomial_surface(&samples, rows, cols, config)?;
    let model = evaluate_polynomial_surface(&coeffs, rows, cols, config.poly_degree);
    let rms_residual = compute_rms_residual(&samples, &coeffs, rows, cols, config.poly_degree);

    let corrected = channels
        .iter()
        .map(|ch| apply_correction(ch, &model, &config.mode))
        .collect();

    Ok(LinkedBackgroundResult {
        model,
        corrected,
        sample_count,
        rms_residual,
    })
}

pub fn neutralize_background(
    image: &Array2<f32>,
    config: &BackgroundConfig,
) -> Result<NeutralizeResult> {
    let samples = auto_sample_grid(image, config).unwrap_or_default();
    let sample_count = samples.len();

    let level = if samples.is_empty() {
        super::stats::compute_image_stats(image).median as f32
    } else {
        let mut vals: Vec<f32> = samples.iter().map(|s| s.value).collect();
        median_f32_mut(&mut vals)
    };

    let corrected = image.mapv(|v| if v.is_finite() { v - level } else { v });

    Ok(NeutralizeResult {
        corrected,
        level,
        sample_count,
    })
}

#[derive(Debug, Clone, Copy)]
pub enum DebandAxis {
    Rows,
    Columns,
    Both,
}

#[derive(Debug, Clone)]
pub struct DebandConfig {
    pub axis: DebandAxis,
    pub sigma_clip: f32,
    pub iterations: usize,
}

fn robust_line_level(mut vals: Vec<f32>, sigma_clip: f32, iterations: usize) -> f32 {
    if vals.len() < 8 {
        return f32::NAN;
    }
    sigma_clipped_stats(&mut vals, sigma_clip, iterations.max(1)).0 as f32
}

fn deband_rows(image: &mut Array2<f32>, config: &DebandConfig) {
    let (rows, cols) = image.dim();
    if rows == 0 || cols == 0 {
        return;
    }
    let slice = match image.as_slice() {
        Some(s) => s,
        None => return,
    };
    let row_bg: Vec<f32> = slice
        .par_chunks(cols)
        .map(|row| {
            let vals: Vec<f32> = row.iter().copied().filter(|v| v.is_finite() && *v > 1e-7).collect();
            robust_line_level(vals, config.sigma_clip, config.iterations)
        })
        .collect();

    let mut finite: Vec<f32> = row_bg.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return;
    }
    let global = median_f32_mut(&mut finite);

    let slice_mut = image.as_slice_mut().expect("contiguous");
    slice_mut
        .par_chunks_mut(cols)
        .zip(row_bg.par_iter())
        .for_each(|(row, &bg)| {
            if !bg.is_finite() {
                return;
            }
            let offset = bg - global;
            if offset.abs() < 1e-12 {
                return;
            }
            for v in row.iter_mut() {
                if v.is_finite() {
                    *v -= offset;
                }
            }
        });
}

fn deband_cols(image: &mut Array2<f32>, config: &DebandConfig) {
    let (rows, cols) = image.dim();
    if rows == 0 || cols == 0 {
        return;
    }

    let col_bg: Vec<f32> = (0..cols)
        .into_par_iter()
        .map(|j| {
            let vals: Vec<f32> = (0..rows)
                .map(|i| image[[i, j]])
                .filter(|v| v.is_finite() && *v > 1e-7)
                .collect();
            robust_line_level(vals, config.sigma_clip, config.iterations)
        })
        .collect();

    let mut finite: Vec<f32> = col_bg.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return;
    }
    let global = median_f32_mut(&mut finite);

    let offsets: Vec<f32> = col_bg
        .iter()
        .map(|&bg| if bg.is_finite() { bg - global } else { 0.0 })
        .collect();

    let slice_mut = image.as_slice_mut().expect("contiguous");
    slice_mut.par_chunks_mut(cols).for_each(|row| {
        for (j, v) in row.iter_mut().enumerate() {
            let offset = offsets[j];
            if offset != 0.0 && v.is_finite() {
                *v -= offset;
            }
        }
    });
}

fn levels_mad(levels: &[f32]) -> f32 {
    let mut finite: Vec<f32> = levels.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.len() < 4 {
        return 0.0;
    }
    let med = median_f32_mut(&mut finite);
    let mut devs: Vec<f32> = finite.iter().map(|v| (v - med).abs()).collect();
    median_f32_mut(&mut devs)
}

pub fn detect_band_axis(image: &Array2<f32>, config: &DebandConfig) -> DebandAxis {
    let (rows, cols) = image.dim();

    let row_lv: Vec<f32> = (0..rows)
        .into_par_iter()
        .map(|i| {
            let vals: Vec<f32> = (0..cols)
                .map(|j| image[[i, j]])
                .filter(|v| v.is_finite() && *v > 1e-7)
                .collect();
            robust_line_level(vals, config.sigma_clip, config.iterations)
        })
        .collect();

    let col_lv: Vec<f32> = (0..cols)
        .into_par_iter()
        .map(|j| {
            let vals: Vec<f32> = (0..rows)
                .map(|i| image[[i, j]])
                .filter(|v| v.is_finite() && *v > 1e-7)
                .collect();
            robust_line_level(vals, config.sigma_clip, config.iterations)
        })
        .collect();

    let mad_r = levels_mad(&row_lv);
    let mad_c = levels_mad(&col_lv);

    if mad_r > 1.5 * mad_c {
        DebandAxis::Rows
    } else if mad_c > 1.5 * mad_r {
        DebandAxis::Columns
    } else {
        DebandAxis::Both
    }
}

pub fn deband_axis_name(axis: DebandAxis) -> &'static str {
    match axis {
        DebandAxis::Rows => "rows",
        DebandAxis::Columns => "columns",
        DebandAxis::Both => "both",
    }
}

pub fn deband(image: &Array2<f32>, config: &DebandConfig) -> Array2<f32> {
    let mut out = if image.is_standard_layout() {
        image.to_owned()
    } else {
        Array2::from_shape_vec(image.raw_dim(), image.iter().copied().collect()).expect("shape matches")
    };
    match config.axis {
        DebandAxis::Rows => deband_rows(&mut out, config),
        DebandAxis::Columns => deband_cols(&mut out, config),
        DebandAxis::Both => {
            deband_rows(&mut out, config);
            deband_cols(&mut out, config);
        }
    }
    out
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

    image
        .as_slice()
        .context("Image not contiguous")?;
    let stats = super::stats::compute_image_stats(image);
    let global_median = stats.median as f32;
    let sigma = stats.sigma as f32;

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
        let sig = mad * MAD_TO_SIGMA as f32;
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
        .flat_map_iter(|y| {
            let ny = y as f64 / row_scale - 0.5;
            let mut y_pows = [0.0f64; 7];
            y_pows[0] = 1.0;
            for i in 1..=degree.min(6) {
                y_pows[i] = y_pows[i - 1] * ny;
            }

            (0..cols).map(move |x| {
                let nx = x as f64 / col_scale - 0.5;
                let mut x_pows = [0.0f64; 7];
                x_pows[0] = 1.0;
                for i in 1..=degree.min(6) {
                    x_pows[i] = x_pows[i - 1] * nx;
                }
                eval_poly_inline(ny, nx, degree, coeffs, &y_pows, &x_pows) as f32
            })
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

    let model_slice = model.as_slice().unwrap();
    let mut finite_vals: Vec<f32> = Vec::with_capacity(model_slice.len());
    for &v in model_slice {
        if v.is_finite() && v > 0.0 {
            finite_vals.push(v);
        }
    }
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
                if bg > 1e-10 {
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
            let mut y_pows = [0.0f64; 7];
            let mut x_pows = [0.0f64; 7];
            y_pows[0] = 1.0;
            x_pows[0] = 1.0;
            for i in 1..=degree {
                y_pows[i] = y_pows[i - 1] * y;
                x_pows[i] = x_pows[i - 1] * x;
            }
            let val_alloc: f64 = alloc.iter().enumerate().map(|(i, &b)| b * (i as f64 + 1.0)).sum();
            let coeffs: Vec<f64> = (0..alloc.len()).map(|i| i as f64 + 1.0).collect();
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
                    (result.corrected[[y, x]] - bg_level).abs() < 1.0,
                    "Corrected pixel at ({},{}) = {} should stay near {}",
                    y, x, result.corrected[[y, x]], bg_level
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

    fn region_mean_sd(img: &Array2<f32>, rows: usize, cols: usize) -> (f32, f32) {
        let mut vals: Vec<f32> = Vec::new();
        for y in 10..rows - 10 {
            for x in 10..cols - 10 {
                vals.push(img[[y, x]]);
            }
        }
        let mean = vals.iter().sum::<f32>() / vals.len() as f32;
        let sd = (vals.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / vals.len() as f32).sqrt();
        (mean, sd)
    }

    #[test]
    fn linked_removes_shared_gradient_preserves_color() {
        let rows = 128;
        let cols = 128;
        let grad = |y: usize| (y as f32 / rows as f32) * 50.0;
        let ch1 = Array2::from_shape_fn((rows, cols), |(y, _)| grad(y) + 100.0);
        let ch2 = Array2::from_shape_fn((rows, cols), |(y, _)| grad(y) + 300.0);
        let reference = Array2::from_shape_fn((rows, cols), |(y, _)| grad(y) + 200.0);

        let config = BackgroundConfig {
            grid_size: 6,
            poly_degree: 1,
            sigma_clip: 3.0,
            iterations: 2,
            mode: BackgroundMode::Subtract,
        };

        let res = extract_background_linked(&[&ch1, &ch2], &reference, &config).unwrap();
        assert_eq!(res.corrected.len(), 2);

        let (m1, sd1) = region_mean_sd(&res.corrected[0], rows, cols);
        let (m2, sd2) = region_mean_sd(&res.corrected[1], rows, cols);

        assert!(sd1 < 2.0, "gradient not removed from ch1: sd={}", sd1);
        assert!(sd2 < 2.0, "gradient not removed from ch2: sd={}", sd2);
        assert!(
            ((m2 - m1) - 200.0).abs() < 5.0,
            "per-channel offset (color) not preserved: {} expected ~200",
            m2 - m1
        );
    }

    #[test]
    fn neutralize_subtracts_sky_level() {
        let rows = 64;
        let cols = 64;
        let image = Array2::from_elem((rows, cols), 100.0f32);

        let config = BackgroundConfig {
            grid_size: 4,
            poly_degree: 1,
            sigma_clip: 3.0,
            iterations: 2,
            mode: BackgroundMode::Subtract,
        };

        let res = neutralize_background(&image, &config).unwrap();
        assert!((res.level - 100.0).abs() < 1.0, "level={}", res.level);
        for y in 10..rows - 10 {
            for x in 10..cols - 10 {
                assert!(
                    res.corrected[[y, x]].abs() < 1.0,
                    "background not neutralized: {}",
                    res.corrected[[y, x]]
                );
            }
        }
    }

    #[test]
    fn deband_rows_flattens_horizontal_stripes() {
        let rows = 64;
        let cols = 64;
        let img = Array2::from_shape_fn((rows, cols), |(y, _x)| {
            100.0f32 + if y % 2 == 0 { 10.0 } else { -10.0 }
        });

        let cfg = DebandConfig { axis: DebandAxis::Rows, sigma_clip: 3.0, iterations: 2 };
        let out = deband(&img, &cfg);

        for y in 0..rows {
            let mut rowvals: Vec<f32> = (0..cols).map(|x| out[[y, x]]).collect();
            let m = median_f32_mut(&mut rowvals);
            assert!((m - 100.0).abs() < 1.0, "row {} median {} not flattened", y, m);
        }
    }

    #[test]
    fn detect_axis_horizontal_stripes() {
        let img = Array2::from_shape_fn((64, 64), |(y, _x)| {
            100.0f32 + if y % 2 == 0 { 8.0 } else { -8.0 }
        });
        let cfg = DebandConfig { axis: DebandAxis::Both, sigma_clip: 3.0, iterations: 2 };
        assert!(matches!(detect_band_axis(&img, &cfg), DebandAxis::Rows));
    }

    #[test]
    fn detect_axis_vertical_stripes() {
        let img = Array2::from_shape_fn((64, 64), |(_y, x)| {
            100.0f32 + if x % 2 == 0 { 8.0 } else { -8.0 }
        });
        let cfg = DebandConfig { axis: DebandAxis::Both, sigma_clip: 3.0, iterations: 2 };
        assert!(matches!(detect_band_axis(&img, &cfg), DebandAxis::Columns));
    }

    #[test]
    fn deband_preserves_overall_level() {
        let rows = 48;
        let cols = 48;
        let img = Array2::from_shape_fn((rows, cols), |(y, _x)| 200.0f32 + (y as f32 % 3.0 - 1.0) * 5.0);
        let cfg = DebandConfig { axis: DebandAxis::Rows, sigma_clip: 3.0, iterations: 2 };
        let out = deband(&img, &cfg);
        let mut before: Vec<f32> = img.iter().copied().collect();
        let mut after: Vec<f32> = out.iter().copied().collect();
        let mb = median_f32_mut(&mut before);
        let ma = median_f32_mut(&mut after);
        assert!((mb - ma).abs() < 1.0, "overall level shifted: {} -> {}", mb, ma);
    }

    #[test]
    fn test_solve_linear_2x2() {
        let mut a = vec![2.0, 1.0, 5.0, 7.0];
        let mut b = vec![11.0, 13.0];
        solve_linear_system(&mut a, &mut b, 2).unwrap();
        // Solution of [[2,1],[5,7]] x = [11,13] is x = [64/9, -29/9].
        assert!((b[0] - 7.1111).abs() < 0.01);
        assert!((b[1] + 3.2222).abs() < 0.01);
    }

    #[test]
    fn test_fast_median() {
        let mut v1 = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((median_f32_mut(&mut v1) - 3.0).abs() < 1e-6);
        let mut v2 = vec![1.0, 2.0, 3.0, 4.0];
        assert!((median_f32_mut(&mut v2) - 2.5).abs() < 1e-6);
    }

    #[test]
    fn divide_mode_skips_nonpositive_background() {
        let image = Array2::from_shape_vec((1, 2), vec![100.0, 100.0]).unwrap();
        let model = Array2::from_shape_vec((1, 2), vec![50.0, -50.0]).unwrap();
        let corrected = apply_correction(&image, &model, &BackgroundMode::Divide);
        assert!(corrected[[0, 0]] > 0.0);
        assert_eq!(corrected[[0, 1]], 100.0);
    }
}
