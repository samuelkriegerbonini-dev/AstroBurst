use anyhow::{Context, Result};
use ndarray::{Array2, Zip};
use rayon::prelude::*;
use rustfft::{num_complex::Complex, FftPlanner};
use std::sync::Arc;

use crate::infra::progress::ProgressHandle;
use crate::types::error::AppError;
use crate::types::stacking::{RLConfig, RLResult};

pub fn generate_gaussian_psf(size: usize, sigma: f32) -> Array2<f32> {
    let mut psf = Array2::<f32>::zeros((size, size));
    let center = size as f32 / 2.0;
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
    fft_rows: usize,
    fft_cols: usize,
    psf_freq: Vec<Complex<f32>>,
    psf_conj_freq: Vec<Complex<f32>>,
    fwd_row: Arc<dyn rustfft::Fft<f32>>,
    inv_row: Arc<dyn rustfft::Fft<f32>>,
    fwd_col: Arc<dyn rustfft::Fft<f32>>,
    inv_col: Arc<dyn rustfft::Fft<f32>>,
}

impl FftConvolver {
    fn new(rows: usize, cols: usize, psf: &Array2<f32>) -> Self {
        let fft_rows = (rows + psf.nrows() - 1).next_power_of_two();
        let fft_cols = (cols + psf.ncols() - 1).next_power_of_two();

        let mut planner = FftPlanner::<f32>::new();
        let fwd_row = planner.plan_fft_forward(fft_cols);
        let inv_row = planner.plan_fft_inverse(fft_cols);
        let fwd_col = planner.plan_fft_forward(fft_rows);
        let inv_col = planner.plan_fft_inverse(fft_rows);

        let psf_freq = Self::compute_psf_freq(psf, fft_rows, fft_cols, &fwd_row, &fwd_col);
        let psf_conj_freq: Vec<Complex<f32>> = psf_freq.iter().map(|c| c.conj()).collect();

        Self {
            rows,
            cols,
            fft_rows,
            fft_cols,
            psf_freq,
            psf_conj_freq,
            fwd_row,
            inv_row,
            fwd_col,
            inv_col,
        }
    }

    fn compute_psf_freq(
        psf: &Array2<f32>,
        fft_rows: usize,
        fft_cols: usize,
        fwd_row: &Arc<dyn rustfft::Fft<f32>>,
        fwd_col: &Arc<dyn rustfft::Fft<f32>>,
    ) -> Vec<Complex<f32>> {
        let (pr, pc) = psf.dim();
        let cy = pr / 2;
        let cx = pc / 2;

        let mut buf = vec![Complex::new(0.0f32, 0.0); fft_rows * fft_cols];

        for y in 0..pr {
            for x in 0..pc {
                let dy = (y as isize - cy as isize).rem_euclid(fft_rows as isize) as usize;
                let dx = (x as isize - cx as isize).rem_euclid(fft_cols as isize) as usize;
                buf[dy * fft_cols + dx] = Complex::new(psf[[y, x]], 0.0);
            }
        }

        for r in 0..fft_rows {
            let start = r * fft_cols;
            let row_slice = &mut buf[start..start + fft_cols];
            fwd_row.process(row_slice);
        }

        let mut col_buf = vec![Complex::new(0.0f32, 0.0); fft_rows];
        for c in 0..fft_cols {
            for r in 0..fft_rows {
                col_buf[r] = buf[r * fft_cols + c];
            }
            fwd_col.process(&mut col_buf);
            for r in 0..fft_rows {
                buf[r * fft_cols + c] = col_buf[r];
            }
        }

        buf
    }

    fn forward_2d(&self, image: &Array2<f32>) -> Vec<Complex<f32>> {
        let mut buf = vec![Complex::new(0.0f32, 0.0); self.fft_rows * self.fft_cols];

        for y in 0..self.rows {
            for x in 0..self.cols {
                buf[y * self.fft_cols + x] = Complex::new(image[[y, x]], 0.0);
            }
        }

        buf.par_chunks_mut(self.fft_cols).for_each(|row| {
            self.fwd_row.process(row);
        });

        let mut col_major = vec![Complex::new(0.0f32, 0.0); self.fft_rows * self.fft_cols];
        for r in 0..self.fft_rows {
            for c in 0..self.fft_cols {
                col_major[c * self.fft_rows + r] = buf[r * self.fft_cols + c];
            }
        }

        col_major.par_chunks_mut(self.fft_rows).for_each(|col| {
            self.fwd_col.process(col);
        });

        for c in 0..self.fft_cols {
            for r in 0..self.fft_rows {
                buf[r * self.fft_cols + c] = col_major[c * self.fft_rows + r];
            }
        }

        buf
    }

    fn inverse_2d(&self, buf: &mut [Complex<f32>]) -> Array2<f32> {
        buf.par_chunks_mut(self.fft_cols).for_each(|row| {
            self.inv_row.process(row);
        });

        let fft_rows = self.fft_rows;
        let fft_cols = self.fft_cols;

        let mut col_major = vec![Complex::new(0.0f32, 0.0); fft_rows * fft_cols];
        for r in 0..fft_rows {
            for c in 0..fft_cols {
                col_major[c * fft_rows + r] = buf[r * fft_cols + c];
            }
        }

        col_major.par_chunks_mut(fft_rows).for_each(|col| {
            self.inv_col.process(col);
        });

        for c in 0..fft_cols {
            for r in 0..fft_rows {
                buf[r * fft_cols + c] = col_major[c * fft_rows + r];
            }
        }

        let inv_norm = 1.0 / (fft_rows * fft_cols) as f32;
        let mut result = Array2::<f32>::zeros((self.rows, self.cols));
        for y in 0..self.rows {
            for x in 0..self.cols {
                result[[y, x]] = buf[y * fft_cols + x].re * inv_norm;
            }
        }

        result
    }

    fn convolve_with_freq(&self, image: &Array2<f32>, freq: &[Complex<f32>]) -> Array2<f32> {
        let mut buf = self.forward_2d(image);

        buf.par_iter_mut()
            .zip(freq.par_iter())
            .for_each(|(b, f)| {
                *b = *b * *f;
            });

        self.inverse_2d(&mut buf)
    }

    fn convolve_psf(&self, image: &Array2<f32>) -> Array2<f32> {
        self.convolve_with_freq(image, &self.psf_freq)
    }

    fn convolve_psf_transpose(&self, image: &Array2<f32>) -> Array2<f32> {
        self.convolve_with_freq(image, &self.psf_conj_freq)
    }
}

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

    let convolver = FftConvolver::new(rows, cols, psf);

    let convergence_threshold = 1e-6;
    let mut last_convergence = f64::MAX;
    let mut iterations_run = 0;

    for iter in 0..config.iterations {
        if let Some(p) = progress {
            if p.is_cancelled() {
                return Err(AppError::Cancelled.into());
            }
        }

        let convolved = convolver.convolve_psf(&estimate);

        let epsilon = 1e-6f32;
        let lambda = config.regularization as f32;

        let ratio = if lambda > 0.0 {
            Zip::from(&convolved)
                .and(image)
                .and(&estimate)
                .map_collect(|&c, &img, &est| img / (c + lambda * est + epsilon))
        } else {
            Zip::from(&convolved)
                .and(image)
                .map_collect(|&c, &img| img / (c + epsilon))
        };

        let correction = convolver.convolve_psf_transpose(&ratio);

        let sum_sq_delta: f64 = estimate
            .as_slice_mut()
            .context("Estimate not contiguous")?
            .par_iter_mut()
            .zip(correction.as_slice().context("Correction not contiguous")?)
            .map(|(est, &cor)| {
                let old = *est;
                *est = (old * cor).max(0.0);
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
                let upper = orig * (1.0 + threshold);
                let lower = orig * (1.0 - threshold).max(0.0);
                if est > upper {
                    row[x] = upper;
                } else if est < lower {
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
        let convolver = FftConvolver::new(rows, cols, &psf);
        let result = convolver.convolve_psf(&image);

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
    fn test_deringing_bidirectional() {
        let size = 16;
        let original = Array2::from_elem((size, size), 100.0f32);
        let mut estimate = original.clone();
        estimate[[5, 5]] = 200.0;
        estimate[[8, 8]] = 10.0;

        apply_deringing(&mut estimate, &original, 0.1);

        assert!((estimate[[5, 5]] - 110.0).abs() < 1e-4);
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
