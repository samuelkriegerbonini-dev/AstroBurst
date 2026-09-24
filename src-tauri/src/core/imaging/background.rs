use anyhow::{Context, Result};
use ndarray::{Array2};
use rayon::prelude::*;

use crate::infra::progress::ProgressHandle;
use crate::core::imaging::stats::is_valid_pixel;
use crate::math::median::{median_f32_mut};
use crate::math::sigma_clipped_stats;
use crate::types::constants::MAD_TO_SIGMA;
use crate::types::error::AppError;

const MAX_POLY_TERMS: usize = 21;
const MIN_SCALED_PIVOT: f64 = 1e-10;
const SKY_LEVEL_MAX_SAMPLES: usize = 262_144;

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

    let corrected = apply_correction(image, &model, &config.mode)?;

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

    let level = model_level_over_data(reference, &model);
    let corrected = channels
        .iter()
        .map(|ch| correct_at_level(ch, &model, &config.mode, level))
        .collect::<Result<Vec<_>>>()?;

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
    let samples = auto_sample_grid(image, config)?;
    let sample_count = samples.len();

    let level = if samples.is_empty() {
        clipped_sky_level(image, config).with_context(|| {
            format!(
                "Neutralize could not measure the sky level: no valid pixels, or none left after a {} sigma clip",
                config.sigma_clip
            )
        })?
    } else {
        let mut vals: Vec<f32> = samples.iter().map(|s| s.value).collect();
        median_f32_mut(&mut vals)
    };

    let corrected = image.mapv(|v| if is_valid_pixel(v) { v - level } else { v });

    Ok(NeutralizeResult {
        corrected,
        sample_count,
    })
}

fn clipped_sky_level(image: &Array2<f32>, config: &BackgroundConfig) -> Option<f32> {
    let stride = image.len().div_ceil(SKY_LEVEL_MAX_SAMPLES).max(1);
    let mut vals: Vec<f32> = image.iter().step_by(stride).copied().filter(|&v| is_valid_pixel(v)).collect();
    if vals.is_empty() {
        return None;
    }
    let (median, _) = sigma_clipped_stats(&mut vals, config.sigma_clip, config.iterations.max(1));
    Some(median as f32).filter(|m| m.is_finite())
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
            let vals: Vec<f32> = row.iter().copied().filter(|v| is_valid_pixel(*v)).collect();
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
                if is_valid_pixel(*v) {
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
                .filter(|v| is_valid_pixel(*v))
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
            if offset != 0.0 && is_valid_pixel(*v) {
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
                .filter(|v| is_valid_pixel(*v))
                .collect();
            robust_line_level(vals, config.sigma_clip, config.iterations)
        })
        .collect();

    let col_lv: Vec<f32> = (0..cols)
        .into_par_iter()
        .map(|j| {
            let vals: Vec<f32> = (0..rows)
                .map(|i| image[[i, j]])
                .filter(|v| is_valid_pixel(*v))
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
                        if is_valid_pixel(v) {
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

    if !normal_matrix_has_full_rank(&ata, n_terms) {
        let distinct = |key: fn(&SamplePoint) -> f32| {
            let mut v: Vec<u32> = samples.iter().map(|s| key(s).to_bits()).collect();
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        anyhow::bail!(
            "Background samples do not constrain a degree-{} polynomial: the {} usable samples lie on {} grid rows and {} grid columns. Lower the polynomial degree or the grid size, or crop the padded or empty area",
            degree,
            samples.len(),
            distinct(|s| s.y),
            distinct(|s| s.x)
        );
    }

    for i in 0..n_terms {
        ata[i * n_terms + i] += 1e-8;
    }

    solve_linear_system(&mut ata, &mut atb, n_terms)
        .context("Failed to solve polynomial fit")?;

    Ok(atb)
}

fn normal_matrix_has_full_rank(ata: &[f64], n: usize) -> bool {
    let scale: Vec<f64> = (0..n).map(|i| ata[i * n + i].sqrt()).collect();
    if scale.iter().any(|s| !(s.is_finite() && *s > 0.0)) {
        return false;
    }
    let mut a: Vec<f64> = (0..n * n).map(|k| ata[k] / (scale[k / n] * scale[k % n])).collect();
    for k in 0..n {
        let pivot = a[k * n + k];
        if !(pivot > MIN_SCALED_PIVOT) {
            return false;
        }
        for i in (k + 1)..n {
            let factor = a[i * n + k] / pivot;
            for j in k..n {
                a[i * n + j] -= factor * a[k * n + j];
            }
        }
    }
    true
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

pub(crate) fn apply_correction(
    image: &Array2<f32>,
    model: &Array2<f32>,
    mode: &BackgroundMode,
) -> Result<Array2<f32>> {
    apply_correction_with(image, model, mode, true)
}

pub(crate) fn apply_correction_with(
    image: &Array2<f32>,
    model: &Array2<f32>,
    mode: &BackgroundMode,
    keep_level: bool,
) -> Result<Array2<f32>> {
    check_model_dim(image, model)?;
    let level = if keep_level { model_level_over_data(image, model) } else { None };
    correct_at_level(image, model, mode, level)
}

fn check_model_dim(image: &Array2<f32>, model: &Array2<f32>) -> Result<()> {
    if image.dim() != model.dim() {
        anyhow::bail!(
            "Background model {:?} does not match image {:?}",
            model.dim(),
            image.dim()
        );
    }
    Ok(())
}

fn model_at_data<'a>(image: &'a Array2<f32>, model: &'a Array2<f32>) -> impl Iterator<Item = f32> + 'a {
    image
        .iter()
        .zip(model.iter())
        .filter(|&(&img, &bg)| is_valid_pixel(img) && bg.is_finite())
        .map(|(_, &bg)| bg)
}

fn model_level_over_data(image: &Array2<f32>, model: &Array2<f32>) -> Option<f32> {
    let mut vals: Vec<f32> = model_at_data(image, model).collect();
    if vals.is_empty() {
        None
    } else {
        Some(median_f32_mut(&mut vals))
    }
}

fn correct_at_level(
    image: &Array2<f32>,
    model: &Array2<f32>,
    mode: &BackgroundMode,
    level: Option<f32>,
) -> Result<Array2<f32>> {
    check_model_dim(image, model)?;

    if let BackgroundMode::Divide = mode {
        if let Some(lowest) = model_at_data(image, model).reduce(f32::min) {
            if lowest <= 0.0 {
                anyhow::bail!(
                    "Divide mode needs a background model that stays positive over the image, but the model reaches {}. Use Subtract for sky-subtracted or signed data",
                    lowest
                );
            }
        }
    }

    Ok(ndarray::Zip::from(image).and(model).par_map_collect(|&img, &bg| {
        if !is_valid_pixel(img) {
            return img;
        }
        match mode {
            BackgroundMode::Subtract => img - bg + level.unwrap_or(0.0),
            BackgroundMode::Divide => (img / bg) * level.unwrap_or(1.0),
        }
    }))
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

pub(crate) fn solve_linear_system(a: &mut [f64], b: &mut [f64], n: usize) -> Result<()> {
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
        assert_eq!(res.sample_count, 16);
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
    fn divide_mode_refuses_a_model_that_is_not_positive_over_the_data() {
        let image = Array2::from_shape_vec((1, 2), vec![100.0, 100.0]).unwrap();
        let model = Array2::from_shape_vec((1, 2), vec![50.0, -50.0]).unwrap();
        let err = apply_correction(&image, &model, &BackgroundMode::Divide).unwrap_err();
        assert!(err.to_string().contains("Divide mode"), "{}", err);

        let padded = Array2::from_shape_vec((1, 3), vec![100.0, 0.0, f32::NAN]).unwrap();
        let model = Array2::from_shape_vec((1, 3), vec![50.0, -50.0, 0.0]).unwrap();
        let corrected = apply_correction(&padded, &model, &BackgroundMode::Divide).unwrap();
        assert_eq!(corrected[[0, 0]], 100.0);
        assert_eq!(corrected[[0, 1]], 0.0);
        assert!(corrected[[0, 2]].is_nan());
    }

    struct Noise(u64);

    impl Noise {
        fn uniform(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }

        fn gaussian(&mut self) -> f32 {
            (0..12).map(|_| self.uniform()).sum::<f32>() - 6.0
        }
    }

    fn noisy(rows: usize, cols: usize, seed: u64, sky: impl Fn(usize, usize) -> f32) -> Array2<f32> {
        let mut noise = Noise(seed);
        Array2::from_shape_fn((rows, cols), |(y, x)| sky(y, x) + noise.gaussian())
    }

    fn valid_median(img: &Array2<f32>) -> f32 {
        let mut vals: Vec<f32> = img.iter().copied().filter(|&v| is_valid_pixel(v)).collect();
        median_f32_mut(&mut vals)
    }

    #[test]
    fn polynomial_background_fits_zero_sky_and_signed_gradients() {
        let config = BackgroundConfig::default();
        let flat = noisy(512, 512, 11, |_, _| 0.0);
        let res = extract_background(&flat, &config, None).unwrap();
        assert!(res.sample_count >= 48, "samples {}", res.sample_count);
        let worst = res.model.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        assert!(worst < 0.3, "zero-sky model strays to {}", worst);

        let ramp = |_: usize, x: usize| -3.0 + 6.0 * x as f32 / 511.0;
        let gradient = noisy(512, 512, 12, ramp);
        let res = extract_background(&gradient, &config, None).unwrap();
        assert!(res.sample_count >= 48, "samples {}", res.sample_count);
        for x in [0usize, 64, 128, 256, 384, 511] {
            let err = (res.model[[256, x]] - ramp(256, x)).abs();
            assert!(err < 0.3, "model off by {} sigma at x={}", err, x);
        }
    }

    #[test]
    fn polynomial_fit_refuses_samples_confined_to_one_grid_row() {
        let strip = noisy(800, 800, 13, |y, _| if y >= 700 { 100.0 } else { f32::NAN });
        let config = BackgroundConfig { poly_degree: 1, ..BackgroundConfig::default() };
        let err = extract_background(&strip, &config, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("on 1 grid rows"), "{}", msg);

        let reference = strip.clone();
        assert!(extract_background_linked(&[&strip], &reference, &config).is_err());
    }

    #[test]
    fn neutralize_centres_zero_sky_data_and_keeps_padding() {
        let mut image = noisy(256, 256, 21, |_, _| 0.0);
        for y in 0..24 {
            for x in 0..256 {
                image[[y, x]] = 0.0;
            }
        }
        image[[100, 100]] = f32::NAN;
        let config = BackgroundConfig { grid_size: 8, ..BackgroundConfig::default() };
        let res = neutralize_background(&image, &config).unwrap();
        assert!(res.sample_count > 0);
        let median = valid_median(&res.corrected);
        assert!(median.abs() < 0.1, "neutralized sky median {}", median);
        assert!(res.corrected.slice(ndarray::s![0..24, ..]).iter().all(|&v| v == 0.0));
        assert!(res.corrected[[100, 100]].is_nan());
    }

    #[test]
    fn neutralize_reports_a_grid_that_does_not_fit_the_image() {
        let image = Array2::from_elem((8, 8), 100.0f32);
        let err = neutralize_background(&image, &BackgroundConfig::default()).unwrap_err();
        assert!(err.to_string().contains("too small"), "{}", err);
    }

    #[test]
    fn neutralize_falls_back_to_a_clipped_median_of_all_data_pixels() {
        let mut image = noisy(64, 64, 22, |_, _| -2.0);
        for y in 0..64 {
            for x in 0..64 {
                if (x / 4 + y / 4) % 2 == 0 {
                    image[[y, x]] = 0.0;
                }
            }
        }
        let config = BackgroundConfig { grid_size: 4, ..BackgroundConfig::default() };
        let res = neutralize_background(&image, &config).unwrap();
        assert_eq!(res.sample_count, 0);
        let median = valid_median(&res.corrected);
        assert!(median.abs() < 0.2, "fallback level left median {}", median);

        let empty = Array2::from_elem((64, 64), 0.0f32);
        assert!(neutralize_background(&empty, &config).is_err());
    }

    #[test]
    fn neutralize_refuses_a_sigma_clip_that_rejects_every_pixel_instead_of_filling_nan() {
        let image = noisy(64, 64, 23, |_, _| 5.0);
        for sigma_clip in [-1.0f32, f32::NAN] {
            let config = BackgroundConfig { grid_size: 4, sigma_clip, ..BackgroundConfig::default() };
            let err = neutralize_background(&image, &config).unwrap_err();
            assert!(err.to_string().contains("sigma clip"), "{}", err);
        }
        let config = BackgroundConfig { grid_size: 4, ..BackgroundConfig::default() };
        let res = neutralize_background(&image, &config).unwrap();
        assert!(res.corrected.iter().all(|v| v.is_finite()));
    }

    fn line_level_spread(img: &Array2<f32>, rows: bool) -> f32 {
        let lines: Vec<Vec<f32>> = if rows {
            img.outer_iter().map(|l| l.iter().copied().filter(|&v| is_valid_pixel(v)).collect()).collect()
        } else {
            img.columns().into_iter().map(|l| l.iter().copied().filter(|&v| is_valid_pixel(v)).collect()).collect()
        };
        let mut levels: Vec<f32> = lines.into_iter().map(|mut l| median_f32_mut(&mut l)).collect();
        let centre = median_f32_mut(&mut levels.clone());
        levels.iter_mut().for_each(|v| *v = (*v - centre).abs());
        (levels.iter().map(|v| v * v).sum::<f32>() / levels.len() as f32).sqrt()
    }

    #[test]
    fn deband_removes_stripes_on_zero_median_data_and_keeps_padding() {
        let striped_rows = noisy(64, 2048, 31, |y, _| if y % 2 == 0 { 0.5 } else { -0.5 });
        let striped_cols = noisy(2048, 64, 32, |_, x| if x % 2 == 0 { 0.5 } else { -0.5 });
        for (mut img, axis, rows) in [(striped_rows, DebandAxis::Rows, true), (striped_cols, DebandAxis::Columns, false)] {
            let before = line_level_spread(&img, rows);
            assert!(before > 0.4, "stripes not injected: {}", before);
            if rows {
                img.slice_mut(ndarray::s![.., 0..16]).fill(0.0);
            } else {
                img.slice_mut(ndarray::s![0..16, ..]).fill(0.0);
            }
            let out = deband(&img, &DebandConfig { axis, sigma_clip: 3.0, iterations: 2 });
            let after = line_level_spread(&out, rows);
            assert!(after < 0.1 * before, "{:?}: stripe rms {} left of {}", axis, after, before);
            let pad = if rows { out.slice(ndarray::s![.., 0..16]).to_owned() } else { out.slice(ndarray::s![0..16, ..]).to_owned() };
            assert!(pad.iter().all(|&v| v == 0.0), "{:?}: padding rewritten", axis);
        }
    }

    #[test]
    fn keep_median_re_adds_the_median_of_the_whole_signed_model() {
        let ramp = Array2::from_shape_fn((10, 1000), |(_, x)| -3.0 + 6.0 * x as f32 / 999.0);
        let corrected = apply_correction(&ramp, &ramp, &BackgroundMode::Subtract).unwrap();
        assert!(corrected.iter().all(|v| v.abs() < 1e-3), "signed sky shifted to {}", corrected[[0, 0]]);

        let negative_model = Array2::from_elem((4, 4), -2.0f32);
        let image = Array2::from_elem((4, 4), -1.5f32);
        let corrected = apply_correction(&image, &negative_model, &BackgroundMode::Subtract).unwrap();
        assert!(corrected.iter().all(|&v| (v + 1.5).abs() < 1e-6), "negative pedestal dropped: {}", corrected[[0, 0]]);
    }

    #[test]
    fn subtract_keeps_padding_pixels_as_padding() {
        let mut image = Array2::from_elem((4, 4), 5.0f32);
        image[[0, 0]] = 0.0;
        image[[0, 1]] = f32::NAN;
        let model = Array2::from_shape_fn((4, 4), |(y, _)| 1.0 + y as f32);
        let corrected = apply_correction(&image, &model, &BackgroundMode::Subtract).unwrap();
        assert_eq!(corrected[[0, 0]], 0.0);
        assert!(corrected[[0, 1]].is_nan());
        assert_eq!(corrected[[1, 1]], 5.0 - 2.0 + 3.0);
    }

    #[test]
    fn linked_channels_with_different_footprints_get_one_shared_level() {
        let (rows, cols) = (64, 400);
        let ramp = |_: usize, x: usize| 900.0 + 200.0 * x as f32 / (cols - 1) as f32;
        let green = noisy(rows, cols, 41, |y, x| ramp(y, x) + 50.0);
        let mut red = noisy(rows, cols, 42, ramp);
        red.slice_mut(ndarray::s![.., 360..]).fill(0.0);

        for mode in [BackgroundMode::Subtract, BackgroundMode::Divide] {
            let config = BackgroundConfig { poly_degree: 1, mode: mode.clone(), ..BackgroundConfig::default() };
            let res = extract_background_linked(&[&red, &green], &green, &config).unwrap();
            let (r_out, g_out) = (&res.corrected[0], &res.corrected[1]);
            assert!(r_out.slice(ndarray::s![.., 360..]).iter().all(|&v| v == 0.0), "{:?}: padding rewritten", mode);
            for y in 0..rows {
                for x in 0..360 {
                    let (r, g) = (red[[y, x]], green[[y, x]]);
                    let drift = match mode {
                        BackgroundMode::Subtract => ((g - g_out[[y, x]]) - (r - r_out[[y, x]])).abs(),
                        BackgroundMode::Divide => ((g / g_out[[y, x]]) / (r / r_out[[y, x]]) - 1.0).abs(),
                    };
                    let tolerance = match mode {
                        BackgroundMode::Subtract => 1e-2,
                        BackgroundMode::Divide => 1e-4,
                    };
                    assert!(drift < tolerance, "{:?}: channel correction differs by {} at ({}, {})", mode, drift, y, x);
                }
            }
        }
    }
}
