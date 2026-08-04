use anyhow::{Context, Result};
use ndarray::{Array2, Zip};
use rayon::prelude::*;
use rustfft::num_complex::Complex;

use crate::infra::progress::ProgressHandle;
use crate::math::complex;
use crate::math::fft::FftEngine2D;
use crate::types::error::AppError;
use crate::types::stacking::{RLConfig, RLResult};

pub fn generate_gaussian_psf(size: usize, sigma: f32) -> Array2<f32> {
    let mut psf = Array2::<f32>::zeros((size, size));
    let center = (size - 1) as f32 / 2.0;
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
    rows: usize,
    cols: usize,
    engine: FftEngine2D<f32>,
    psf_freq: Vec<Complex<f32>>,
    psf_conj_freq: Vec<Complex<f32>>,
    io_buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
}

impl FftConvolver {
    fn new(rows: usize, cols: usize, psf: &Array2<f32>) -> Self {
        let engine = FftEngine2D::<f32>::from_padded_dims(
            rows, cols, psf.nrows() - 1, psf.ncols() - 1,
        );

        let psf_freq = Self::compute_psf_freq(psf, &engine);
        let psf_conj_freq = complex::conjugate_slice(&psf_freq);

        Self {
            rows,
            cols,
            engine,
            psf_freq,
            psf_conj_freq,
            io_buf: Vec::new(),
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

    fn convolve_with_into(&mut self, image: &Array2<f32>, transpose_psf: bool, out: &mut Array2<f32>) {
        let rows = self.rows;
        let cols = self.cols;
        let fft_cols = self.engine.fft_cols;
        let total = self.engine.total_size();
        let zero = Complex::new(0.0f32, 0.0f32);

        if self.io_buf.len() != total {
            self.io_buf.clear();
            self.io_buf.resize(total, zero);
        }

        let img_slice = image.as_slice().expect("contiguous");
        self.io_buf
            .par_chunks_mut(fft_cols)
            .enumerate()
            .for_each(|(y, row)| {
                if y < rows {
                    let src = &img_slice[y * cols..y * cols + cols];
                    for (dst, &v) in row[..cols].iter_mut().zip(src) {
                        *dst = Complex::new(if v.is_finite() { v } else { 0.0 }, 0.0);
                    }
                    for c in row[cols..].iter_mut() {
                        *c = zero;
                    }
                } else {
                    for c in row.iter_mut() {
                        *c = zero;
                    }
                }
            });

        let Self {
            engine,
            io_buf,
            scratch,
            psf_freq,
            psf_conj_freq,
            ..
        } = self;
        engine.forward_2d_with_scratch(io_buf, scratch);
        let freq = if transpose_psf { &**psf_conj_freq } else { &**psf_freq };
        complex::pointwise_multiply_into(io_buf, freq);
        engine.inverse_2d_with_scratch(io_buf, scratch);

        let io_ref: &[Complex<f32>] = io_buf;
        out.as_slice_mut()
            .expect("contiguous")
            .par_chunks_mut(cols)
            .enumerate()
            .for_each(|(y, out_row)| {
                let base = y * fft_cols;
                for x in 0..cols {
                    out_row[x] = io_ref[base + x].re;
                }
            });
    }

    fn convolve_psf_into(&mut self, image: &Array2<f32>, out: &mut Array2<f32>) {
        self.convolve_with_into(image, false, out)
    }

    fn convolve_psf_transpose_into(&mut self, image: &Array2<f32>, out: &mut Array2<f32>) {
        self.convolve_with_into(image, true, out)
    }
}

#[cfg(test)]
fn compute_l2_delta(prev: &Array2<f32>, curr: &Array2<f32>) -> f64 {
    let n = prev.len() as f64;
    let sum_sq: f64 = prev
        .as_slice()
        .unwrap()
        .par_iter()
        .zip(curr.as_slice().unwrap().par_iter())
        .map(|(&a, &b)| {
            let d = (b - a) as f64;
            d * d
        })
        .sum();
    (sum_sq / n).sqrt()
}

pub fn richardson_lucy(
    image: &Array2<f32>,
    psf: &Array2<f32>,
    config: &RLConfig,
    progress: Option<&ProgressHandle>,
) -> Result<RLResult> {
    let start = std::time::Instant::now();
    let (rows, cols) = image.dim();
    let mut estimate = image.clone();

    let mut convolver = FftConvolver::new(rows, cols, psf);

    let image_max = match image.as_slice() {
        Some(s) => s
            .par_chunks(1 << 16)
            .map(|c| c.iter().fold(0.0f32, |a, &b| if b.is_finite() { a.max(b) } else { a }))
            .collect::<Vec<f32>>()
            .into_iter()
            .fold(0.0f32, |a, b| a.max(b)),
        None => image
            .iter()
            .fold(0.0f32, |a, &b| if b.is_finite() { a.max(b) } else { a }),
    };
    let inv_image_max = if image_max > 0.0 { 1.0 / image_max } else { 1.0 };

    let convergence_threshold = 1e-6;
    let mut last_convergence = f64::MAX;
    let mut iterations_run = 0;

    let mut convolved = Array2::<f32>::zeros((rows, cols));
    let mut ratio = Array2::<f32>::zeros((rows, cols));
    let mut correction = Array2::<f32>::zeros((rows, cols));

    for iter in 0..config.iterations {
        if let Some(p) = progress {
            if p.is_cancelled() {
                return Err(AppError::Cancelled.into());
            }
        }

        convolver.convolve_psf_into(&estimate, &mut convolved);

        let epsilon = 1e-6f32;
        let lambda = config.regularization as f32;

        Zip::from(&mut ratio)
            .and(&convolved)
            .and(image)
            .par_for_each(|r, &c, &img| *r = img / (c + epsilon));

        convolver.convolve_psf_transpose_into(&ratio, &mut correction);

        let sum_sq_delta: f64 = estimate
            .as_slice_mut()
            .context("Estimate not contiguous")?
            .par_iter_mut()
            .zip(correction.as_slice().context("Correction not contiguous")?)
            .map(|(est, &cor)| {
                let old = *est;
                let damping = 1.0 + lambda * old * inv_image_max;
                *est = (old * cor / damping).max(0.0);
                let d = (*est - old) as f64;
                d * d
            })
            .sum();

        if config.deringing {
            apply_deringing(&mut estimate, image, config.deringing_threshold);
        }

        iterations_run = iter + 1;
        last_convergence = (sum_sq_delta / (rows * cols) as f64).sqrt();

        if let Some(p) = progress {
            p.tick_with_stage(&format!(
                "iteration {}/{} (delta: {:.2e})",
                iterations_run, config.iterations, last_convergence
            ));
        }

        if last_convergence < convergence_threshold && iterations_run >= 3 {
            if let Some(p) = progress {
                p.tick_with_stage(&format!(
                    "converged at iteration {} (delta: {:.2e})",
                    iterations_run, last_convergence
                ));
            }
            break;
        }
    }

    Ok(RLResult {
        image: estimate,
        iterations_run,
        convergence: last_convergence,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

fn apply_deringing(estimate: &mut Array2<f32>, original: &Array2<f32>, threshold: f32) {
    let cols = estimate.ncols();
    let orig_slice = original.as_slice().unwrap();
    estimate
        .as_slice_mut()
        .unwrap()
        .par_chunks_mut(cols)
        .enumerate()
        .for_each(|(y, row)| {
            let orig_row = &orig_slice[y * cols..(y + 1) * cols];
            for x in 0..cols {
                let orig = orig_row[x];
                let est = row[x];
                let lower = (orig * (1.0 - threshold)).max(0.0);
                if est < lower {
                    row[x] = lower;
                }
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
        let mut convolver = FftConvolver::new(rows, cols, &psf);
        let mut result = Array2::<f32>::zeros((rows, cols));
        convolver.convolve_psf_into(&image, &mut result);

        for y in 1..rows - 1 {
            for x in 1..cols - 1 {
                let diff = (result[[y, x]] - image[[y, x]]).abs();
                assert!(diff < 0.5, "Mismatch at ({},{}): {} vs {}", y, x, result[[y, x]], image[[y, x]]);
            }
        }
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
            psf_sigma: 1.0,
            psf_size: 5,
            regularization: 0.001,
            deringing: false,
            deringing_threshold: 0.1,
        };

        let result = richardson_lucy(&image, &psf, &config, None).unwrap();
        assert!(result.iterations_run > 0);
        assert!(result.iterations_run <= 5);
        assert!(result.convergence.is_finite());
        assert!(result.elapsed_ms > 0);
        assert_eq!(result.image.dim(), (size, size));
    }

    #[test]
    fn test_deringing_limits_undershoot_only() {
        let size = 16;
        let original = Array2::from_elem((size, size), 100.0f32);
        let mut estimate = original.clone();
        estimate[[5, 5]] = 200.0;
        estimate[[8, 8]] = 10.0;

        apply_deringing(&mut estimate, &original, 0.1);

        assert!((estimate[[5, 5]] - 200.0).abs() < 1e-4);
        assert!((estimate[[8, 8]] - 90.0).abs() < 1e-4);
        assert!((estimate[[0, 0]] - 100.0).abs() < 1e-4);
    }

    #[test]
    fn test_l2_delta_identical() {
        let a = Array2::from_elem((10, 10), 1.0f32);
        let delta = compute_l2_delta(&a, &a);
        assert!(delta < 1e-10);
    }

    #[test]
    fn test_l2_delta_different() {
        let a = Array2::from_elem((10, 10), 1.0f32);
        let b = Array2::from_elem((10, 10), 2.0f32);
        let delta = compute_l2_delta(&a, &b);
        assert!((delta - 1.0).abs() < 1e-10);
    }
}
