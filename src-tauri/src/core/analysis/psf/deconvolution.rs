use std::collections::VecDeque;

use anyhow::{bail, Context, Result};
use ndarray::parallel::prelude::*;
use ndarray::{Array2, Axis, Zip};
use rayon::prelude::*;
use rustfft::num_complex::Complex;

use crate::core::imaging::stats::is_padding;
use crate::infra::progress::ProgressHandle;
use crate::math::complex;
use crate::math::fft::{next_smooth_size, FftEngine2D};
use crate::math::median::median_f32_mut;
use crate::types::constants::{MAX_PROVIDED_PSF_SIZE, MIN_PROVIDED_PSF_SIZE};
use crate::types::error::AppError;
use crate::types::stacking::{RLConfig, RLResult};

pub fn generate_gaussian_psf(size: usize, sigma: f32) -> Array2<f32> {
    let mut psf = Array2::<f32>::zeros((size, size));
    let center = size.saturating_sub(1) as f32 / 2.0;
    let sigma2 = 2.0 * sigma * sigma;
    let mut sum = 0.0f32;

    for y in 0..size {
        for x in 0..size {
            let dy = y as f32 - center;
            let dx = x as f32 - center;
            let val = (-((dx * dx + dy * dy) / sigma2)).exp();
            psf[[y, x]] = val;
            sum += val;
        }
    }

    if sum > 0.0 {
        psf.mapv_inplace(|v| v / sum);
    }

    psf
}

struct FftConvolver {
    fft_cols: usize,
    engine: FftEngine2D<f32>,
    psf_freq: Vec<Complex<f32>>,
    io_buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
}

impl FftConvolver {
    fn new(rows: usize, cols: usize, psf: &Array2<f32>) -> Self {
        let engine = FftEngine2D::<f32>::new(next_smooth_size(rows), next_smooth_size(cols));
        let psf_freq = Self::compute_psf_freq(psf, &engine);
        let io_buf = engine.alloc_buffer();

        Self {
            fft_cols: engine.fft_cols,
            engine,
            psf_freq,
            io_buf,
            scratch: Vec::new(),
        }
    }

    fn compute_psf_freq(psf: &Array2<f32>, engine: &FftEngine2D<f32>) -> Vec<Complex<f32>> {
        let (pr, pc) = psf.dim();
        let cy = pr / 2;
        let cx = pc / 2;
        let fft_rows = engine.fft_rows;
        let fft_cols = engine.fft_cols;

        let mut buf = engine.alloc_buffer();

        for y in 0..pr {
            for x in 0..pc {
                let dy = (y as isize - cy as isize).rem_euclid(fft_rows as isize) as usize;
                let dx = (x as isize - cx as isize).rem_euclid(fft_cols as isize) as usize;
                buf[dy * fft_cols + dx] = Complex::new(psf[[y, x]], 0.0);
            }
        }

        engine.forward_2d(&mut buf);
        buf
    }

    fn load_rows(&mut self, rows: usize, cols: usize, fill: impl Fn(usize, &mut [Complex<f32>]) + Sync) {
        let zero = Complex::new(0.0f32, 0.0f32);
        self.io_buf.par_chunks_mut(self.fft_cols).enumerate().for_each(|(y, row)| {
            let (head, tail) = row.split_at_mut(cols);
            if y < rows {
                fill(y, head);
            } else {
                head.fill(zero);
            }
            tail.fill(zero);
        });
    }

    fn convolve(&mut self, transpose_psf: bool) {
        let Self { engine, io_buf, scratch, psf_freq, .. } = self;
        engine.forward_2d_with_scratch(io_buf, scratch);
        if transpose_psf {
            complex::pointwise_multiply_conj_into(io_buf, psf_freq);
        } else {
            complex::pointwise_multiply_into(io_buf, psf_freq);
        }
        engine.inverse_2d_with_scratch(io_buf, scratch);
    }
}

const PEDESTAL_MARGIN: f32 = 1e-3;
const RELATIVE_EPSILON: f32 = 1e-6;
const RELATIVE_CONVERGENCE: f64 = 1e-6;
const MIN_COVERAGE_FRACTION: f64 = 1e-3;

fn validate_psf_kernel(psf: &Array2<f32>) -> Result<()> {
    let (rows, cols) = psf.dim();
    if rows == 0 || cols == 0 || rows % 2 == 0 || cols % 2 == 0 {
        bail!("The PSF kernel must have an odd size so it has a centre pixel; got {}x{}", cols, rows);
    }
    let mut sum = 0.0f64;
    for &v in psf.iter() {
        if !v.is_finite() {
            bail!("The PSF kernel contains a non-finite value; check the PSF sigma");
        }
        sum += v as f64;
    }
    if sum <= 0.0 {
        bail!("The PSF kernel sums to {sum}; it needs a positive total to deconvolve");
    }
    Ok(())
}

pub fn provided_psf_kernel(rows: &[Vec<f32>]) -> Result<Array2<f32>> {
    let size = rows.len();
    if !(MIN_PROVIDED_PSF_SIZE..=MAX_PROVIDED_PSF_SIZE).contains(&size) || size % 2 == 0 {
        bail!(
            "The provided PSF kernel must be square with an odd size from {} to {}; it has {} rows",
            MIN_PROVIDED_PSF_SIZE,
            MAX_PROVIDED_PSF_SIZE,
            size
        );
    }
    if let Some((y, row)) = rows.iter().enumerate().find(|(_, row)| row.len() != size) {
        bail!(
            "The provided PSF kernel must be square; row {} has {} values but there are {} rows",
            y,
            row.len(),
            size
        );
    }
    let mut kernel = Array2::<f32>::zeros((size, size));
    let mut sum = 0.0f64;
    for (y, row) in rows.iter().enumerate() {
        for (x, &v) in row.iter().enumerate() {
            if !v.is_finite() {
                bail!("The provided PSF kernel has a non-finite value at row {}, column {}", y, x);
            }
            kernel[[y, x]] = v;
            sum += v as f64;
        }
    }
    if sum <= 0.0 {
        bail!("The provided PSF kernel sums to {}; it needs a positive total to deconvolve", sum);
    }
    let sum = sum as f32;
    kernel.mapv_inplace(|v| v / sum);
    Ok(kernel)
}

fn valid_range(image: &Array2<f32>) -> Option<(f32, f32)> {
    image
        .iter()
        .copied()
        .filter(|v| !is_padding(*v))
        .fold(None, |acc, v| match acc {
            None => Some((v, v)),
            Some((lo, hi)) => Some((lo.min(v), hi.max(v))),
        })
}

fn non_negative_pedestal(image: &Array2<f32>) -> f32 {
    match valid_range(image) {
        Some((lo, hi)) if lo < 0.0 => -lo + PEDESTAL_MARGIN * (hi - lo),
        _ => 0.0,
    }
}

fn valid_median(image: &Array2<f32>) -> f32 {
    let mut valid: Vec<f32> = image.iter().copied().filter(|v| !is_padding(*v)).collect();
    median_f32_mut(&mut valid)
}

pub fn richardson_lucy(
    image: &Array2<f32>,
    psf: &Array2<f32>,
    config: &RLConfig,
    progress: Option<&ProgressHandle>,
) -> Result<RLResult> {
    validate_psf_kernel(psf)?;
    let pedestal = non_negative_pedestal(image);
    let has_padding = image.iter().any(|v| is_padding(*v));
    if pedestal <= 0.0 && !has_padding {
        return richardson_lucy_non_negative(image, psf, config, progress);
    }
    let padding_fill = if pedestal > 0.0 { pedestal } else { valid_median(image) };
    let shifted = image.mapv(|v| if is_padding(v) { padding_fill } else { v + pedestal });
    let mut result = richardson_lucy_non_negative(&shifted, psf, config, progress)?;
    Zip::from(&mut result.image).and(image).par_for_each(|out, &orig| {
        *out = if is_padding(orig) { 0.0 } else { *out - pedestal };
    });
    Ok(result)
}

fn sliding_minimum(src: &[f32], half: usize, dst: &mut [f32]) {
    let len = src.len();
    let mut candidates: VecDeque<usize> = VecDeque::new();
    let mut next = 0usize;
    for (i, out) in dst.iter_mut().enumerate() {
        let last = (i + half).min(len - 1);
        while next <= last {
            while candidates.back().is_some_and(|&j| src[j] >= src[next]) {
                candidates.pop_back();
            }
            candidates.push_back(next);
            next += 1;
        }
        while candidates.front().is_some_and(|&j| j + half < i) {
            candidates.pop_front();
        }
        *out = src[candidates[0]];
    }
}

fn sliding_mean(src: &[f32], half: usize, dst: &mut [f32]) {
    let len = src.len();
    let mut sum = 0.0f64;
    let mut first = 0usize;
    let mut next = 0usize;
    for (i, out) in dst.iter_mut().enumerate() {
        let last = (i + half).min(len - 1);
        while next <= last {
            sum += src[next] as f64;
            next += 1;
        }
        while first + half < i {
            sum -= src[first] as f64;
            first += 1;
        }
        *out = (sum / (next - first) as f64) as f32;
    }
}

fn filter_rows_then_columns(plane: &mut Array2<f32>, half_y: usize, half_x: usize, line: fn(&[f32], usize, &mut [f32])) {
    plane.axis_iter_mut(Axis(0)).into_par_iter().for_each(|mut row| {
        let src = row.to_vec();
        let mut dst = vec![0.0f32; src.len()];
        line(&src, half_x, &mut dst);
        row.iter_mut().zip(&dst).for_each(|(v, &d)| *v = d);
    });
    plane.axis_iter_mut(Axis(1)).into_par_iter().for_each(|mut column| {
        let src = column.to_vec();
        let mut dst = vec![0.0f32; src.len()];
        line(&src, half_y, &mut dst);
        column.iter_mut().zip(&dst).for_each(|(v, &d)| *v = d);
    });
}

fn deringing_floor(image: &Array2<f32>, psf_rows: usize, psf_cols: usize, threshold: f32) -> Array2<f32> {
    let (half_y, half_x) = (psf_rows / 2, psf_cols / 2);
    let kept = 1.0 - threshold.clamp(0.0, 1.0);
    let mut floor = image.clone();
    filter_rows_then_columns(&mut floor, half_y, half_x, sliding_minimum);
    filter_rows_then_columns(&mut floor, half_y, half_x, sliding_mean);
    floor.par_mapv_inplace(|v| (v * kept).max(0.0));
    floor
}

fn covered_span(position: usize, psf_len: usize, image_len: usize) -> (usize, usize) {
    let reach = psf_len - 1;
    let lo = reach.saturating_sub(position);
    let hi = (reach + image_len).saturating_sub(position).min(psf_len);
    (lo, hi.max(lo))
}

struct PsfCoverage {
    prefix: Vec<f64>,
    stride: usize,
    rows: Vec<(usize, usize)>,
    cols: Vec<(usize, usize)>,
    usable_above: f32,
}

impl PsfCoverage {
    fn new(psf: &Array2<f32>, rows: usize, cols: usize) -> Self {
        let (psf_rows, psf_cols) = psf.dim();
        let stride = psf_cols + 1;
        let mut prefix = vec![0.0f64; (psf_rows + 1) * stride];
        for y in 0..psf_rows {
            let mut along = 0.0f64;
            for x in 0..psf_cols {
                along += psf[[y, x]] as f64;
                prefix[(y + 1) * stride + x + 1] = prefix[y * stride + x + 1] + along;
            }
        }
        let total = prefix[psf_rows * stride + psf_cols];
        Self {
            prefix,
            stride,
            rows: (0..rows + psf_rows - 1).map(|y| covered_span(y, psf_rows, rows)).collect(),
            cols: (0..cols + psf_cols - 1).map(|x| covered_span(x, psf_cols, cols)).collect(),
            usable_above: (MIN_COVERAGE_FRACTION * total) as f32,
        }
    }

    fn weight(&self, y: usize, x: usize) -> f32 {
        let (y0, y1) = self.rows[y];
        let (x0, x1) = self.cols[x];
        let at = |r: usize, c: usize| self.prefix[r * self.stride + c];
        (at(y1, x1) - at(y0, x1) - at(y1, x0) + at(y0, x0)) as f32
    }
}

fn edge_extended(image: &Array2<f32>, half_y: usize, half_x: usize) -> Vec<f32> {
    let (rows, cols) = image.dim();
    let (ext_rows, ext_cols) = (rows + 2 * half_y, cols + 2 * half_x);
    let mut extended = vec![0.0f32; ext_rows * ext_cols];
    extended.par_chunks_mut(ext_cols).enumerate().for_each(|(y, out)| {
        let source = image.row(y.saturating_sub(half_y).min(rows - 1));
        for (x, v) in out.iter_mut().enumerate() {
            *v = source[x.saturating_sub(half_x).min(cols - 1)];
        }
    });
    extended
}

fn richardson_lucy_non_negative(
    image: &Array2<f32>,
    psf: &Array2<f32>,
    config: &RLConfig,
    progress: Option<&ProgressHandle>,
) -> Result<RLResult> {
    let start = std::time::Instant::now();
    let (rows, cols) = image.dim();
    if rows == 0 || cols == 0 {
        bail!("The image is empty");
    }
    let (psf_rows, psf_cols) = psf.dim();
    let (half_y, half_x) = (psf_rows / 2, psf_cols / 2);
    let (ext_rows, ext_cols) = (rows + 2 * half_y, cols + 2 * half_x);
    let standard = image.as_standard_layout();
    let image_slice = standard.as_slice().context("Image not contiguous")?;

    let image_max = image_slice
        .par_chunks(1 << 16)
        .map(|c| c.iter().fold(0.0f32, |a, &b| if b.is_finite() { a.max(b) } else { a }))
        .reduce(|| 0.0f32, f32::max);
    let inv_image_max = if image_max > 0.0 { 1.0 / image_max } else { 1.0 };
    let epsilon = if image_max > 0.0 { RELATIVE_EPSILON * image_max } else { RELATIVE_EPSILON };
    let lambda = config.regularization as f32;

    let floor = config
        .deringing
        .then(|| deringing_floor(image, psf_rows, psf_cols, config.deringing_threshold));
    let floor_slice = floor.as_ref().map(|f| f.as_slice().context("Deringing floor not contiguous")).transpose()?;
    let coverage = PsfCoverage::new(psf, rows, cols);
    let mut estimate = edge_extended(image, half_y, half_x);
    let mut convolver = FftConvolver::new(ext_rows, ext_cols, psf);
    let fft_cols = convolver.fft_cols;
    let zero = Complex::new(0.0f32, 0.0f32);
    let image_rows = half_y..half_y + rows;
    let image_cols = half_x..half_x + cols;

    let mut last_convergence = f64::MAX;
    let mut iterations_run = 0;

    for iter in 0..config.iterations {
        if let Some(p) = progress {
            if p.is_cancelled() {
                return Err(AppError::Cancelled.into());
            }
        }

        convolver.load_rows(ext_rows, ext_cols, |y, row| {
            let source = &estimate[y * ext_cols..(y + 1) * ext_cols];
            for (c, &v) in row.iter_mut().zip(source) {
                *c = Complex::new(v, 0.0);
            }
        });
        convolver.convolve(false);

        convolver.io_buf.par_chunks_mut(fft_cols).enumerate().for_each(|(y, row)| {
            if !image_rows.contains(&y) {
                row.fill(zero);
                return;
            }
            let observed = &image_slice[(y - half_y) * cols..(y - half_y + 1) * cols];
            row[..half_x].fill(zero);
            for (c, &img) in row[image_cols.clone()].iter_mut().zip(observed) {
                *c = Complex::new(img / (c.re + epsilon), 0.0);
            }
            row[half_x + cols..].fill(zero);
        });
        convolver.convolve(true);

        let corrections: &[Complex<f32>] = &convolver.io_buf;
        let sum_sq_delta: f64 = estimate
            .par_chunks_mut(ext_cols)
            .enumerate()
            .map(|(y, est_row)| {
                let correction = &corrections[y * fft_cols..y * fft_cols + ext_cols];
                let in_image_row = image_rows.contains(&y);
                let mut sum = 0.0f64;
                for (x, (est, cor)) in est_row.iter_mut().zip(correction).enumerate() {
                    let old = *est;
                    let weight = coverage.weight(y, x);
                    let factor = if weight > coverage.usable_above { cor.re / weight } else { 1.0 };
                    let damping = 1.0 + lambda * old * inv_image_max;
                    *est = (old * factor / damping).max(0.0);
                    if in_image_row && image_cols.contains(&x) {
                        let d = (*est - old) as f64;
                        sum += d * d;
                    }
                }
                sum
            })
            .sum();

        if let Some(floor) = floor_slice {
            return_clamped_flux(&mut estimate, floor, &mut convolver, (rows, cols), (half_y, half_x));
        }

        iterations_run = iter + 1;
        last_convergence = (sum_sq_delta / (rows * cols) as f64).sqrt();

        if let Some(p) = progress {
            p.tick_with_stage(&format!(
                "iteration {}/{} (delta: {:.2e})",
                iterations_run, config.iterations, last_convergence
            ));
        }

        if last_convergence * (inv_image_max as f64) < RELATIVE_CONVERGENCE && iterations_run >= 3 {
            if let Some(p) = progress {
                p.tick_with_stage(&format!(
                    "converged at iteration {} (delta: {:.2e})",
                    iterations_run, last_convergence
                ));
            }
            break;
        }
    }

    let mut result = Array2::<f32>::zeros((rows, cols));
    result
        .as_slice_mut()
        .context("Result not contiguous")?
        .par_chunks_mut(cols)
        .enumerate()
        .for_each(|(y, out)| {
            let source = &estimate[(y + half_y) * ext_cols + half_x..(y + half_y) * ext_cols + half_x + cols];
            out.copy_from_slice(source);
        });

    Ok(RLResult {
        image: result,
        iterations_run,
        convergence: last_convergence,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

fn return_clamped_flux(
    estimate: &mut [f32],
    floor: &[f32],
    convolver: &mut FftConvolver,
    (rows, cols): (usize, usize),
    (half_y, half_x): (usize, usize),
) {
    let (ext_rows, ext_cols) = (rows + 2 * half_y, cols + 2 * half_x);
    let image_rows = half_y..half_y + rows;
    let image_cols = half_x..half_x + cols;
    let floor_row = |y: usize| &floor[(y - half_y) * cols..(y - half_y + 1) * cols];
    let current: &[f32] = estimate;
    let undershoot = |y: usize, x: usize| (floor_row(y)[x] - current[y * ext_cols + half_x + x]).max(0.0);

    let any_below = image_rows.clone().into_par_iter().any(|y| (0..cols).any(|x| undershoot(y, x) > 0.0));
    if !any_below {
        return;
    }

    let zero = Complex::new(0.0f32, 0.0f32);
    convolver.load_rows(ext_rows, ext_cols, |y, row| {
        row.fill(zero);
        if image_rows.contains(&y) {
            for (x, c) in row[image_cols.clone()].iter_mut().enumerate() {
                *c = Complex::new(undershoot(y, x), 0.0);
            }
        }
    });
    convolver.convolve(false);

    let fft_cols = convolver.fft_cols;
    let spreads: &[Complex<f32>] = &convolver.io_buf;
    estimate.par_chunks_mut(ext_cols).enumerate().for_each(|(y, est_row)| {
        let spread = &spreads[y * fft_cols..y * fft_cols + ext_cols];
        let lowest = image_rows.contains(&y).then(|| floor_row(y));
        for (x, (est, back)) in est_row.iter_mut().zip(spread).enumerate() {
            let lifted = match lowest {
                Some(lowest) if image_cols.contains(&x) => est.max(lowest[x - half_x]),
                _ => *est,
            };
            *est = (lifted - back.re).max(0.0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gaussian_psf_normalized() {
        let psf = generate_gaussian_psf(15, 2.0);
        let sum: f32 = psf.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_gaussian_psf_center_peak() {
        let psf = generate_gaussian_psf(15, 2.0);
        let center = 15 / 2;
        let center_val = psf[[center, center]];
        for y in 0..15 {
            for x in 0..15 {
                assert!(psf[[y, x]] <= center_val + 1e-7);
            }
        }
    }

    #[test]
    fn test_fft_convolver_identity() {
        let rows = 64;
        let cols = 64;
        let mut psf = Array2::<f32>::zeros((3, 3));
        psf[[1, 1]] = 1.0;

        let image = Array2::from_shape_fn((rows, cols), |(y, x)| (y * cols + x) as f32);
        let mut convolver = FftConvolver::new(rows + 2, cols + 2, &psf);
        convolver.load_rows(rows, cols, |y, row| {
            for (x, c) in row.iter_mut().enumerate() {
                *c = Complex::new(image[[y, x]], 0.0);
            }
        });
        convolver.convolve(false);

        for y in 0..rows {
            for x in 0..cols {
                let got = convolver.io_buf[y * convolver.fft_cols + x].re;
                assert!((got - image[[y, x]]).abs() < 0.5, "Mismatch at ({},{}): {} vs {}", y, x, got, image[[y, x]]);
            }
        }
    }

    #[test]
    fn the_transposed_convolution_mirrors_the_kernel() {
        let psf = Array2::from_shape_fn((3, 5), |(y, x)| (1 + y * 5 + x) as f32);
        let (rows, cols) = (12usize, 14usize);
        let mut convolver = FftConvolver::new(rows, cols, &psf);
        let impulse = (6usize, 7usize);
        for transpose in [false, true] {
            convolver.load_rows(rows, cols, |y, row| {
                for (x, c) in row.iter_mut().enumerate() {
                    *c = Complex::new(if (y, x) == impulse { 1.0 } else { 0.0 }, 0.0);
                }
            });
            convolver.convolve(transpose);
            for ((u, v), &weight) in psf.indexed_iter() {
                let (dy, dx) = (u as isize - 1, v as isize - 2);
                let (y, x) = if transpose {
                    (impulse.0 as isize - dy, impulse.1 as isize - dx)
                } else {
                    (impulse.0 as isize + dy, impulse.1 as isize + dx)
                };
                let got = convolver.io_buf[y as usize * convolver.fft_cols + x as usize].re;
                assert!((got - weight).abs() < 1e-4, "transpose={transpose}: kernel ({u},{v}) landed as {got}");
            }
        }
    }

    #[test]
    fn psf_coverage_is_the_kernel_sum_that_lands_on_the_image() {
        let psf = Array2::from_shape_fn((3, 5), |(y, x)| (1 + y * 5 + x) as f32);
        let (rows, cols) = (4usize, 6usize);
        let (half_y, half_x) = (1usize, 2usize);
        let coverage = PsfCoverage::new(&psf, rows, cols);
        for y in 0..rows + 2 * half_y {
            for x in 0..cols + 2 * half_x {
                let mut expected = 0.0f32;
                for ((u, v), &weight) in psf.indexed_iter() {
                    let (ty, tx) = (y + u, x + v);
                    let on_image = (2 * half_y..2 * half_y + rows).contains(&ty) && (2 * half_x..2 * half_x + cols).contains(&tx);
                    if on_image {
                        expected += weight;
                    }
                }
                assert_eq!(coverage.weight(y, x), expected, "extended pixel ({y},{x})");
            }
        }
        assert_eq!(coverage.weight(3, 5), 120.0);
        assert_eq!(coverage.weight(0, 0), 15.0);
    }

    #[test]
    fn test_rl_returns_result() {
        let size = 32;
        let psf = generate_gaussian_psf(5, 1.0);
        let image = Array2::from_shape_fn((size, size), |(y, x)| {
            ((y * size + x) as f32 / (size * size) as f32) + 0.01
        });

        let config = RLConfig {
            iterations: 5,
            deringing: false,
            ..RLConfig::default()
        };

        let result = richardson_lucy(&image, &psf, &config, None).unwrap();
        assert!(result.iterations_run > 0);
        assert!(result.iterations_run <= 5);
        assert!(result.convergence.is_finite());
        assert!(result.elapsed_ms > 0);
        assert_eq!(result.image.dim(), (size, size));
    }

    #[test]
    fn the_deringing_floor_follows_the_local_sky_and_stays_below_the_image() {
        let centre = (32.0, 32.0);
        let image = Array2::from_shape_fn((64, 64), |(y, x)| {
            (100.0 + ((y * 7 + x * 3) % 5) as f64 + gaussian_star(y, x, centre, 5.0e4, 2.0)) as f32
        });
        let floor = deringing_floor(&image, 15, 15, 0.1);
        for ((y, x), &lowest) in floor.indexed_iter() {
            assert!(lowest <= 0.9 * image[[y, x]] * (1.0 + 1e-6), "the floor at ({y},{x}) is {lowest} above the image");
            assert!((89.999..91.0).contains(&lowest), "the floor at ({y},{x}) is {lowest}, not 90 percent of the sky");
        }
        let open = deringing_floor(&image, 15, 15, 1.0);
        assert!(open.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn sliding_windows_clip_at_the_ends() {
        let src = [5.0f32, 3.0, 8.0, 1.0, 9.0, 7.0];
        let mut lowest = [0.0f32; 6];
        sliding_minimum(&src, 1, &mut lowest);
        assert_eq!(lowest, [3.0, 3.0, 1.0, 1.0, 1.0, 7.0]);
        let mut mean = [0.0f32; 6];
        sliding_mean(&src, 1, &mut mean);
        for (got, expected) in mean.iter().zip([4.0f32, 16.0 / 3.0, 4.0, 6.0, 17.0 / 3.0, 8.0]) {
            assert!((got - expected).abs() < 1e-6, "{mean:?}");
        }
        sliding_minimum(&src, 9, &mut lowest);
        assert_eq!(lowest, [1.0; 6]);
    }

    #[test]
    fn deringing_gives_back_the_flux_it_lifts() {
        let centre = (64.3, 63.8);
        let image = Array2::from_shape_fn((128, 128), |(y, x)| (100.0 + gaussian_star(y, x, centre, 5.0e4, 2.0)) as f32);
        let psf = generate_gaussian_psf(15, 2.0);
        let ringing = richardson_lucy(&image, &psf, &rl_config(20, false), None).unwrap();
        let deringed = richardson_lucy(&image, &psf, &rl_config(20, true), None).unwrap();
        let lowest = |result: &RLResult| result.image.iter().fold(f32::MAX, |a, &b| a.min(b));
        assert!(lowest(&ringing) < 60.0, "the plain result should ring below the sky, lowest {}", lowest(&ringing));
        assert!(lowest(&deringed) > 75.0, "the deringed result still dips to {}", lowest(&deringed));
        let total = |result: &RLResult| net_flux_within(&result.image, centre, 40.0, 100.0);
        assert!((total(&deringed) / total(&ringing) - 1.0).abs() < 0.002, "deringing changed the flux within 40 px by {:+.3} percent", 100.0 * (total(&deringed) / total(&ringing) - 1.0));
    }

    fn rl_config(iterations: usize, deringing: bool) -> RLConfig {
        RLConfig {
            iterations,
            deringing,
            ..RLConfig::default()
        }
    }

    fn sky_subtracted_field(size: usize, seed: u64) -> Array2<f32> {
        let mut state = seed;
        let mut uniform = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        let stars = [(30.0f64, 30.0f64, 200.0f64), (60.0, 70.0, 80.0)];
        Array2::from_shape_fn((size, size), |(y, x)| {
            let noise = (-2.0 * uniform().ln()).sqrt() * (2.0 * std::f64::consts::PI * uniform()).cos();
            let signal: f64 = stars
                .iter()
                .map(|(sy, sx, a)| a * (-((x as f64 - sx).powi(2) + (y as f64 - sy).powi(2)) / 8.0).exp())
                .sum();
            (noise + signal) as f32
        })
    }

    fn gaussian_stream(seed: u64) -> impl FnMut() -> f64 {
        let mut state = seed;
        let mut uniform = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        move || (-2.0 * uniform().ln()).sqrt() * (2.0 * std::f64::consts::PI * uniform()).cos()
    }

    fn gaussian_star(y: usize, x: usize, centre: (f64, f64), total: f64, sigma: f64) -> f64 {
        let d2 = (x as f64 - centre.0).powi(2) + (y as f64 - centre.1).powi(2);
        total / (2.0 * std::f64::consts::PI * sigma * sigma) * (-d2 / (2.0 * sigma * sigma)).exp()
    }

    fn three_stars_on_a_unit_sky(scale: f64) -> Array2<f32> {
        let mut noise = gaussian_stream(5);
        let stars = [((40.0, 40.0), 300.0), ((90.0, 70.0), 1000.0), ((64.0, 100.0), 100.0)];
        Array2::from_shape_fn((128, 128), |(y, x)| {
            let signal: f64 = stars.iter().map(|&(centre, total)| gaussian_star(y, x, centre, total, 2.0)).sum();
            ((1.0 + signal + 0.05 * noise()) * scale) as f32
        })
    }

    #[test]
    fn richardson_lucy_is_invariant_to_the_flux_unit() {
        let psf = generate_gaussian_psf(15, 2.0);
        let config = RLConfig::default();
        let reference = richardson_lucy(&three_stars_on_a_unit_sky(1.0), &psf, &config, None).unwrap();
        let reference_max = reference.image.iter().fold(0.0f32, |a, &b| a.max(b)) as f64;
        for scale in [2.26e-9f64, 1e-19, 1e6] {
            let scaled = richardson_lucy(&three_stars_on_a_unit_sky(scale), &psf, &config, None).unwrap();
            assert_eq!(scaled.iterations_run, reference.iterations_run, "scale {scale:e}: iterations");
            let worst = scaled
                .image
                .iter()
                .zip(reference.image.iter())
                .map(|(&s, &r)| (s as f64 / scale - r as f64).abs())
                .fold(0.0f64, f64::max);
            assert!(worst / reference_max < 1e-4, "scale {scale:e}: the result differs by {:.2e} of the peak", worst / reference_max);
        }
    }

    fn net_flux_within(image: &Array2<f32>, centre: (f64, f64), radius: f64, sky: f64) -> f64 {
        image
            .indexed_iter()
            .filter(|((y, x), _)| (*x as f64 - centre.0).hypot(*y as f64 - centre.1) <= radius)
            .map(|(_, &v)| v as f64 - sky)
            .sum()
    }

    #[test]
    fn default_deconvolution_keeps_the_flux_of_a_star() {
        let centre = (64.3, 63.8);
        let sky = 100.0;
        let image = Array2::from_shape_fn((128, 128), |(y, x)| (sky + gaussian_star(y, x, centre, 5.0e4, 2.0)) as f32);
        let psf = generate_gaussian_psf(15, 2.0);
        let result = richardson_lucy(&image, &psf, &RLConfig::default(), None).unwrap();
        for radius in [12.0, 30.0] {
            let before = net_flux_within(&image, centre, radius, sky);
            let after = net_flux_within(&result.image, centre, radius, sky);
            assert!(
                (after / before - 1.0).abs() < 0.01,
                "net flux within {radius} px changed by {:+.2} percent",
                100.0 * (after / before - 1.0)
            );
        }
    }

    #[test]
    fn a_flat_sky_keeps_its_border_rows_through_richardson_lucy() {
        let image = Array2::from_elem((64, 64), 100.0f32);
        let psf = generate_gaussian_psf(15, 2.0);
        for deringing in [true, false] {
            let result = richardson_lucy(&image, &psf, &rl_config(20, deringing), None).unwrap();
            let (lowest, highest) = result.image.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
            assert!(
                lowest > 99.0 && highest < 101.0,
                "deringing={deringing}: a flat sky of 100 came back between {lowest} and {highest}"
            );
        }
    }

    fn far_from_the_stars(y: usize, x: usize) -> bool {
        let d1 = (y as f64 - 30.0).hypot(x as f64 - 30.0);
        let d2 = (y as f64 - 60.0).hypot(x as f64 - 70.0);
        d1 > 12.0 && d2 > 12.0
    }

    #[test]
    fn a_sky_subtracted_frame_keeps_its_background_through_richardson_lucy() {
        let image = sky_subtracted_field(96, 11);
        let psf = generate_gaussian_psf(15, 2.0);
        for deringing in [true, false] {
            let result = richardson_lucy(&image, &psf, &rl_config(20, deringing), None).unwrap();
            let zeros = result.image.iter().filter(|v| **v == 0.0).count() as f64 / result.image.len() as f64;
            assert!(zeros < 0.01, "deringing={deringing}: {:.0}% of the frame clamped to 0", zeros * 100.0);
            let mut sky: Vec<f32> = result
                .image
                .indexed_iter()
                .filter(|((y, x), _)| far_from_the_stars(*y, *x))
                .map(|(_, v)| *v)
                .collect();
            sky.sort_by(|a, b| a.total_cmp(b));
            let median = sky[sky.len() / 2];
            assert!(median.abs() < 0.3, "deringing={deringing}: sky median moved to {median}");
            assert!(sky[0] < 0.0, "deringing={deringing}: no negative sky pixel survived");
        }
    }

    #[test]
    fn padding_stays_padding_when_a_pedestal_is_added() {
        let mut image = sky_subtracted_field(64, 5);
        image.slice_mut(ndarray::s![.., ..8]).fill(0.0);
        image[[40, 40]] = f32::NAN;
        let psf = generate_gaussian_psf(7, 1.5);
        let result = richardson_lucy(&image, &psf, &rl_config(5, true), None).unwrap();
        assert!(result.image.slice(ndarray::s![.., ..8]).iter().all(|v| *v == 0.0));
        assert_eq!(result.image[[40, 40]], 0.0);
    }

    fn flat_sky_with_l_shaped_padding() -> Array2<f32> {
        let mut image = Array2::from_elem((96, 96), 20.0f32);
        image.slice_mut(ndarray::s![..48, ..24]).fill(0.0);
        image.slice_mut(ndarray::s![..24, ..48]).fill(0.0);
        image
    }

    fn away_from_the_array_border(y: usize, x: usize) -> bool {
        (12..84).contains(&y) && (12..84).contains(&x)
    }

    fn touches_padding_within(image: &Array2<f32>, y: usize, x: usize, radius: usize) -> bool {
        let (rows, cols) = image.dim();
        let y1 = (y + radius + 1).min(rows);
        let x1 = (x + radius + 1).min(cols);
        image
            .slice(ndarray::s![y.saturating_sub(radius)..y1, x.saturating_sub(radius)..x1])
            .iter()
            .any(|v| is_padding(*v))
    }

    #[test]
    fn a_flat_sky_beside_l_shaped_padding_keeps_its_level_through_richardson_lucy() {
        let image = flat_sky_with_l_shaped_padding();
        let psf = generate_gaussian_psf(15, 2.0);
        for deringing in [true, false] {
            let result = richardson_lucy(&image, &psf, &rl_config(20, deringing), None).unwrap();
            let mut checked = 0usize;
            for ((y, x), &v) in result.image.indexed_iter() {
                if is_padding(image[[y, x]]) {
                    assert_eq!(v, 0.0, "deringing={deringing}: padding at ({y},{x}) became {v}");
                } else if touches_padding_within(&image, y, x, 4) && away_from_the_array_border(y, x) {
                    checked += 1;
                    assert!(
                        (v - 20.0).abs() <= 1.0,
                        "deringing={deringing}: sky at ({y},{x}) next to padding moved to {v}"
                    );
                }
            }
            assert!(checked > 250, "only {checked} pixels lie within 4 px of the padding");
        }
    }

    #[test]
    fn a_positive_image_is_deconvolved_without_a_pedestal() {
        let image = Array2::from_shape_fn((32, 32), |(y, x)| ((y * 32 + x) as f32 / 1024.0) + 0.01);
        assert_eq!(non_negative_pedestal(&image), 0.0);
        let psf = generate_gaussian_psf(5, 1.0);
        let result = richardson_lucy(&image, &psf, &rl_config(3, false), None).unwrap();
        assert!(result.image.iter().all(|v| v.is_finite() && *v >= 0.0));
    }

    fn square_kernel(size: usize, value: f32) -> Vec<Vec<f32>> {
        vec![vec![value; size]; size]
    }

    #[test]
    fn a_provided_kernel_is_normalised_to_unit_sum() {
        let rows = vec![vec![1.0f32, 2.0, 1.0], vec![2.0, 4.0, 2.0], vec![1.0, 2.0, 1.0]];
        let kernel = provided_psf_kernel(&rows).unwrap();
        assert_eq!(kernel.dim(), (3, 3));
        assert!((kernel.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert!((kernel[[1, 1]] - 0.25).abs() < 1e-6);
        assert!((kernel[[0, 0]] - 0.0625).abs() < 1e-6);
        assert_eq!(provided_psf_kernel(&square_kernel(129, 1.0)).unwrap().dim(), (129, 129));
    }

    #[test]
    fn a_malformed_provided_kernel_is_refused_with_the_reason() {
        let even = provided_psf_kernel(&square_kernel(4, 1.0)).unwrap_err().to_string();
        assert!(even.contains("odd size") && even.contains("4 rows"), "{even}");
        let tiny = provided_psf_kernel(&square_kernel(1, 1.0)).unwrap_err().to_string();
        assert!(tiny.contains("from 3 to 129"), "{tiny}");
        let huge = provided_psf_kernel(&square_kernel(131, 1.0)).unwrap_err().to_string();
        assert!(huge.contains("131 rows"), "{huge}");
        let empty = provided_psf_kernel(&[]).unwrap_err().to_string();
        assert!(empty.contains("0 rows"), "{empty}");

        let mut ragged = square_kernel(3, 1.0);
        ragged[1].pop();
        let ragged = provided_psf_kernel(&ragged).unwrap_err().to_string();
        assert!(ragged.contains("row 1 has 2 values"), "{ragged}");

        let mut nan = square_kernel(3, 1.0);
        nan[2][0] = f32::NAN;
        let nan = provided_psf_kernel(&nan).unwrap_err().to_string();
        assert!(nan.contains("non-finite value at row 2, column 0"), "{nan}");

        let zero = provided_psf_kernel(&square_kernel(3, 0.0)).unwrap_err().to_string();
        assert!(zero.contains("sums to 0"), "{zero}");
        let negative = provided_psf_kernel(&square_kernel(5, -1.0)).unwrap_err().to_string();
        assert!(negative.contains("positive total"), "{negative}");
    }

    #[test]
    fn a_degenerate_psf_is_rejected_instead_of_blanking_or_shifting_the_image() {
        let image = Array2::from_elem((33, 33), 1.0f32);
        let config = rl_config(3, true);
        for (size, sigma) in [(0usize, 2.0f32), (4, 2.0), (5, 0.0), (5, f32::NAN)] {
            let psf = generate_gaussian_psf(size, sigma);
            assert!(
                richardson_lucy(&image, &psf, &config, None).is_err(),
                "psf size {size} sigma {sigma} was accepted"
            );
        }
        assert!(richardson_lucy(&image, &generate_gaussian_psf(5, 1.0), &config, None).is_ok());
    }
}
