use anyhow::{bail, Context, Result};
use ndarray::{Array2, Zip};
use rayon::prelude::*;
use rustfft::num_complex::Complex;

use crate::core::imaging::stats::is_padding;
use crate::infra::progress::ProgressHandle;
use crate::math::complex;
use crate::math::fft::FftEngine2D;
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

const PEDESTAL_MARGIN: f32 = 1e-3;

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

fn richardson_lucy_non_negative(
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
