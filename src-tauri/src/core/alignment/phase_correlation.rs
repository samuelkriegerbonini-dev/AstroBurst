use std::borrow::Cow;

use ndarray::Array2;

use super::downsample::area_downsample;
use crate::math::complex;
use crate::math::fft::{self, FftEngine2D};
use crate::math::normalization;
use crate::math::subpixel;
use crate::math::window;

const COARSE_MAX_DIM: usize = 512;
const REFINE_CROP_SIZE: usize = 512;
const CONFIDENCE_THRESHOLD: f64 = 2.0;
const EPSILON: f64 = 1e-15;

#[derive(Debug, Clone)]
pub struct PhaseCorrelationResult {
    pub dx: f64,
    pub dy: f64,
    pub confidence: f64,
}

pub fn phase_correlate(
    reference: &Array2<f32>,
    target: &Array2<f32>,
) -> PhaseCorrelationResult {
    let (ref_rows, ref_cols) = reference.dim();
    let (tgt_rows, tgt_cols) = target.dim();
    let rows = ref_rows.min(tgt_rows);
    let cols = ref_cols.min(tgt_cols);

    let ref_cropped: Cow<Array2<f32>> = if ref_rows != rows || ref_cols != cols {
        Cow::Owned(reference.slice(ndarray::s![..rows, ..cols]).to_owned())
    } else {
        Cow::Borrowed(reference)
    };
    let tgt_cropped: Cow<Array2<f32>> = if tgt_rows != rows || tgt_cols != cols {
        Cow::Owned(target.slice(ndarray::s![..rows, ..cols]).to_owned())
    } else {
        Cow::Borrowed(target)
    };

    if is_constant_or_zero(&ref_cropped) || is_constant_or_zero(&tgt_cropped) {
        return PhaseCorrelationResult {
            dx: 0.0,
            dy: 0.0,
            confidence: 0.0,
        };
    }

    if rows <= COARSE_MAX_DIM && cols <= COARSE_MAX_DIM {
        return correlate_single(&ref_cropped, &tgt_cropped);
    }

    let ds_rows = COARSE_MAX_DIM.min(rows);
    let ds_cols = COARSE_MAX_DIM.min(cols);
    let scale_y = rows as f64 / ds_rows as f64;
    let scale_x = cols as f64 / ds_cols as f64;

    let ref_ds = area_downsample(&ref_cropped, ds_rows, ds_cols);
    let tgt_ds = area_downsample(&tgt_cropped, ds_rows, ds_cols);

    let coarse = correlate_single(&ref_ds, &tgt_ds);
    let coarse_dx = coarse.dx * scale_x;
    let coarse_dy = coarse.dy * scale_y;

    let half = REFINE_CROP_SIZE / 2;
    let ref_cy = rows / 2;
    let ref_cx = cols / 2;
    let tgt_cy = ((ref_cy as f64 + coarse_dy).round() as isize).clamp(0, rows as isize - 1) as usize;
    let tgt_cx = ((ref_cx as f64 + coarse_dx).round() as isize).clamp(0, cols as isize - 1) as usize;

    let (ref_crop, ref_y0, ref_x0) = extract_crop(&ref_cropped, ref_cy, ref_cx, half, rows, cols);
    let (tgt_crop, tgt_y0, tgt_x0) = extract_crop(&tgt_cropped, tgt_cy, tgt_cx, half, rows, cols);

    if ref_crop.dim() != tgt_crop.dim() {
        return PhaseCorrelationResult {
            dx: coarse_dx,
            dy: coarse_dy,
            confidence: coarse.confidence,
        };
    }

    let refine = correlate_single(&ref_crop, &tgt_crop);
    PhaseCorrelationResult {
        dx: (tgt_x0 as f64 - ref_x0 as f64) + refine.dx,
        dy: (tgt_y0 as f64 - ref_y0 as f64) + refine.dy,
        confidence: refine.confidence,
    }
}

fn extract_crop(
    img: &Array2<f32>,
    cy: usize,
    cx: usize,
    half: usize,
    rows: usize,
    cols: usize,
) -> (Array2<f32>, usize, usize) {
    let y0 = cy.saturating_sub(half);
    let y1 = (cy + half).min(rows);
    let x0 = cx.saturating_sub(half);
    let x1 = (cx + half).min(cols);
    (img.slice(ndarray::s![y0..y1, x0..x1]).to_owned(), y0, x0)
}

fn remove_mean(img: &Array2<f32>) -> Array2<f32> {
    let mut sum = 0.0f64;
    let mut cnt = 0u64;
    for &v in img.iter() {
        if v.is_finite() {
            sum += v as f64;
            cnt += 1;
        }
    }
    if cnt == 0 {
        return img.clone();
    }
    let mean = (sum / cnt as f64) as f32;
    img.mapv(|v| if v.is_finite() { v - mean } else { 0.0 })
}

fn correlate_single(a: &Array2<f32>, b: &Array2<f32>) -> PhaseCorrelationResult {
    let (rows, cols) = a.dim();
    let fft_rows = fft::next_power_of_two(rows);
    let fft_cols = fft::next_power_of_two(cols);

    let engine = FftEngine2D::<f64>::new(fft_rows, fft_cols);

    let hann_y = window::hann_periodic::<f64>(rows);
    let hann_x = window::hann_periodic::<f64>(cols);

    let a = remove_mean(a);
    let b = remove_mean(b);

    let mut fa = fft::prepare_windowed_buffer(&a, &hann_y, &hann_x, fft_rows, fft_cols);
    let mut fb = fft::prepare_windowed_buffer(&b, &hann_y, &hann_x, fft_rows, fft_cols);

    engine.forward_2d(&mut fa);
    engine.forward_2d(&mut fb);

    let mut cross = complex::cross_power_spectrum(&fb, &fa, EPSILON);

    engine.inverse_2d(&mut cross);

    let correlation = fft::extract_real(&cross, fft_rows, fft_cols);

    let (peak_y, peak_x, peak_val) = fft::find_peak(&correlation, fft_cols);
    let (mean, sigma) = normalization::compute_mean_sigma(&correlation);
    let confidence = normalization::compute_snr(peak_val, mean, sigma);

    let shift = subpixel::unwrap_and_refine(
        &correlation, fft_rows, fft_cols, peak_y, peak_x,
    );

    PhaseCorrelationResult {
        dx: shift.dx,
        dy: shift.dy,
        confidence,
    }
}

fn is_constant_or_zero(img: &Array2<f32>) -> bool {
    const MIN_FINITE_COUNT: u64 = 16;
    const MIN_FINITE_FRACTION: f64 = 0.25;

    let mut min_val = f32::INFINITY;
    let mut max_val = f32::NEG_INFINITY;
    let mut finite_count = 0u64;

    for &v in img.iter() {
        if v.is_finite() {
            if v < min_val {
                min_val = v;
            }
            if v > max_val {
                max_val = v;
            }
            finite_count += 1;
        }
    }

    let total = img.len() as u64;
    let finite_fraction = if total > 0 {
        finite_count as f64 / total as f64
    } else {
        0.0
    };

    finite_count < MIN_FINITE_COUNT
        || finite_fraction < MIN_FINITE_FRACTION
        || (max_val - min_val).abs() < 1e-10
}

pub fn is_low_confidence(confidence: f64) -> bool {
    confidence < CONFIDENCE_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pattern(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            let h = (y.wrapping_mul(73856093) ^ x.wrapping_mul(19349663)) & 0xffff;
            let texture = (h as f32 / 65535.0 - 0.5) * 400.0;
            ((y as f32 * 0.3).sin() * (x as f32 * 0.2).cos() * 1000.0) + 500.0
                + ((y * 7 + x * 13) as f32 * 0.01).sin() * 200.0
                + texture
        })
    }

    fn shift_array(img: &Array2<f32>, dy: i32, dx: i32) -> Array2<f32> {
        let (rows, cols) = img.dim();
        let mut out = Array2::<f32>::zeros((rows, cols));
        for y in 0..rows {
            let sy = y as i32 - dy;
            if sy < 0 || sy >= rows as i32 {
                continue;
            }
            for x in 0..cols {
                let sx = x as i32 - dx;
                if sx < 0 || sx >= cols as i32 {
                    continue;
                }
                out[[y, x]] = img[[sy as usize, sx as usize]];
            }
        }
        out
    }

    #[test]
    fn test_identical_images() {
        let img = make_pattern(128, 128);
        let result = phase_correlate(&img, &img);
        assert!(result.dx.abs() < 0.5, "dx={}", result.dx);
        assert!(result.dy.abs() < 0.5, "dy={}", result.dy);
    }

    #[test]
    fn test_known_integer_shift() {
        let img = make_pattern(256, 256);
        let shifted = shift_array(&img, 10, -5);
        let result = phase_correlate(&img, &shifted);
        assert!(
            (result.dx - (-5.0)).abs() < 1.0,
            "dx={}, expected -5",
            result.dx
        );
        assert!(
            (result.dy - 10.0).abs() < 1.0,
            "dy={}, expected 10",
            result.dy
        );
    }

    #[test]
    fn test_nan_no_panic() {
        let mut img = make_pattern(64, 64);
        img[[10, 10]] = f32::NAN;
        img[[20, 30]] = f32::INFINITY;
        img[[5, 5]] = f32::NEG_INFINITY;
        let result = phase_correlate(&img, &img);
        assert!(result.dx.is_finite());
        assert!(result.dy.is_finite());
    }

    #[test]
    fn test_constant_image() {
        let img = Array2::<f32>::from_elem((64, 64), 100.0);
        let result = phase_correlate(&img, &img);
        assert_eq!(result.dx, 0.0);
        assert_eq!(result.dy, 0.0);
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn test_is_constant_or_zero_rejects_nan_dominated() {
        let mut img = Array2::<f32>::from_elem((64, 64), f32::NAN);
        for i in 0..200usize {
            let y = (i * 7) % 64;
            let x = (i * 11) % 64;
            img[[y, x]] = (i as f32) + 1.0;
        }
        assert!(is_constant_or_zero(&img));
    }

    #[test]
    fn test_is_constant_or_zero_accepts_mostly_finite() {
        let mut img = make_pattern(64, 64);
        for (k, v) in img.iter_mut().enumerate() {
            if k % 5 == 0 {
                *v = f32::NAN;
            }
        }
        assert!(!is_constant_or_zero(&img));
    }

    fn fft_shift(img: &Array2<f32>, dy: f64, dx: f64) -> Array2<f32> {
        use crate::math::fft::{extract_real, prepare_buffer_no_window, FftEngine2D};
        use std::f64::consts::PI;
        let (rows, cols) = img.dim();
        let engine = FftEngine2D::<f64>::new(rows, cols);
        let mut buf = prepare_buffer_no_window::<f64>(img, rows, cols);
        engine.forward_2d(&mut buf);
        for ky in 0..rows {
            let fy = if ky > rows / 2 { ky as f64 - rows as f64 } else { ky as f64 };
            for kx in 0..cols {
                let fx = if kx > cols / 2 { kx as f64 - cols as f64 } else { kx as f64 };
                let ang = -2.0 * PI * (fy * dy / rows as f64 + fx * dx / cols as f64);
                let (s, c) = ang.sin_cos();
                let idx = ky * cols + kx;
                let re = buf[idx].re * c - buf[idx].im * s;
                let im = buf[idx].re * s + buf[idx].im * c;
                buf[idx].re = re;
                buf[idx].im = im;
            }
        }
        engine.inverse_2d(&mut buf);
        let real = extract_real(&buf, rows, cols);
        Array2::from_shape_vec((rows, cols), real.iter().map(|&v| v as f32).collect()).unwrap()
    }

    #[test]
    fn test_subpixel_fractional_recovery() {
        let img = make_pattern(128, 128);
        for &(dy, dx) in &[(0.3f64, -0.2f64), (0.45, 0.15), (-0.35, 0.4)] {
            let shifted = fft_shift(&img, dy, dx);
            let result = phase_correlate(&img, &shifted);
            assert!(
                (result.dy - dy).abs() < 0.05,
                "dy={} expected {}",
                result.dy,
                dy
            );
            assert!(
                (result.dx - dx).abs() < 0.05,
                "dx={} expected {}",
                result.dx,
                dx
            );
        }
    }
}
