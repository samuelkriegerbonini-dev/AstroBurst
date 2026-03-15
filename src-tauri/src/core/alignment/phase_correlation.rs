use ndarray::Array2;
use rayon::prelude::*;
use rustfft::{num_complex::Complex, FftPlanner};

use super::downsample::area_downsample;

const COARSE_MAX_DIM: usize = 512;
const REFINE_CROP_SIZE: usize = 512;
const CONFIDENCE_THRESHOLD: f64 = 2.0;

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

    let ref_cropped = if ref_rows != rows || ref_cols != cols {
        reference.slice(ndarray::s![..rows, ..cols]).to_owned()
    } else {
        reference.clone()
    };
    let tgt_cropped = if tgt_rows != rows || tgt_cols != cols {
        target.slice(ndarray::s![..rows, ..cols]).to_owned()
    } else {
        target.clone()
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

    let scale_y = rows as f64 / COARSE_MAX_DIM as f64;
    let scale_x = cols as f64 / COARSE_MAX_DIM as f64;
    let ds_rows = COARSE_MAX_DIM.min(rows);
    let ds_cols = COARSE_MAX_DIM.min(cols);

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

    let ref_crop = extract_crop(&ref_cropped, ref_cy, ref_cx, half, rows, cols);
    let tgt_crop = extract_crop(&tgt_cropped, tgt_cy, tgt_cx, half, rows, cols);

    if ref_crop.dim() != tgt_crop.dim() {
        return PhaseCorrelationResult {
            dx: coarse_dx,
            dy: coarse_dy,
            confidence: coarse.confidence,
        };
    }

    let refine = correlate_single(&ref_crop, &tgt_crop);
    PhaseCorrelationResult {
        dx: coarse_dx + refine.dx,
        dy: coarse_dy + refine.dy,
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
) -> Array2<f32> {
    let y0 = cy.saturating_sub(half);
    let y1 = (cy + half).min(rows);
    let x0 = cx.saturating_sub(half);
    let x1 = (cx + half).min(cols);
    img.slice(ndarray::s![y0..y1, x0..x1]).to_owned()
}

fn correlate_single(a: &Array2<f32>, b: &Array2<f32>) -> PhaseCorrelationResult {
    let (rows, cols) = a.dim();
    let fft_rows = next_power_of_2(rows);
    let fft_cols = next_power_of_2(cols);

    let hann_y = hann_window(rows);
    let hann_x = hann_window(cols);

    let fa = fft2d(a, &hann_y, &hann_x, fft_rows, fft_cols);
    let fb = fft2d(b, &hann_y, &hann_x, fft_rows, fft_cols);

    let mut cross: Vec<Complex<f64>> = fa
        .iter()
        .zip(fb.iter())
        .map(|(&f1, &f2)| {
            let product = f1 * f2.conj();
            let mag = product.norm();
            if mag > 1e-15 {
                product / mag
            } else {
                Complex::new(0.0, 0.0)
            }
        })
        .collect();

    ifft2d(&mut cross, fft_rows, fft_cols);

    let correlation = build_real_surface(&cross, fft_rows, fft_cols);

    let (peak_y, peak_x, peak_val) = find_peak(&correlation, fft_rows, fft_cols);
    let confidence = compute_confidence(&correlation, fft_rows, fft_cols, peak_val);

    let raw_dy = if peak_y > fft_rows / 2 {
        peak_y as f64 - fft_rows as f64
    } else {
        peak_y as f64
    };
    let raw_dx = if peak_x > fft_cols / 2 {
        peak_x as f64 - fft_cols as f64
    } else {
        peak_x as f64
    };

    let sub_dy = subpixel_refine_1d(&correlation, fft_rows, fft_cols, peak_y, peak_x, true);
    let sub_dx = subpixel_refine_1d(&correlation, fft_rows, fft_cols, peak_y, peak_x, false);

    let dy = raw_dy + sub_dy;
    let dx = raw_dx + sub_dx;

    PhaseCorrelationResult {
        dx,
        dy,
        confidence,
    }
}

fn fft2d(
    img: &Array2<f32>,
    hann_y: &[f64],
    hann_x: &[f64],
    fft_rows: usize,
    fft_cols: usize,
) -> Vec<Complex<f64>> {
    let (rows, cols) = img.dim();
    let mut data = vec![Complex::new(0.0, 0.0); fft_rows * fft_cols];

    for y in 0..rows {
        for x in 0..cols {
            let mut v = img[[y, x]] as f64;
            if !v.is_finite() {
                v = 0.0;
            }
            data[y * fft_cols + x] = Complex::new(v * hann_y[y] * hann_x[x], 0.0);
        }
    }

    let mut planner = FftPlanner::<f64>::new();

    let fft_row = planner.plan_fft_forward(fft_cols);
    data.par_chunks_mut(fft_cols).for_each(|row| {
        fft_row.process(row);
    });

    let fft_col = planner.plan_fft_forward(fft_rows);
    let mut col_buf = vec![Complex::new(0.0, 0.0); fft_rows];
    for x in 0..fft_cols {
        for y in 0..fft_rows {
            col_buf[y] = data[y * fft_cols + x];
        }
        fft_col.process(&mut col_buf);
        for y in 0..fft_rows {
            data[y * fft_cols + x] = col_buf[y];
        }
    }

    data
}

fn ifft2d(data: &mut [Complex<f64>], fft_rows: usize, fft_cols: usize) {
    let mut planner = FftPlanner::<f64>::new();

    let ifft_row = planner.plan_fft_inverse(fft_cols);
    data.par_chunks_mut(fft_cols).for_each(|row| {
        ifft_row.process(row);
    });

    let ifft_col = planner.plan_fft_inverse(fft_rows);
    let mut col_buf = vec![Complex::new(0.0, 0.0); fft_rows];
    for x in 0..fft_cols {
        for y in 0..fft_rows {
            col_buf[y] = data[y * fft_cols + x];
        }
        ifft_col.process(&mut col_buf);
        for y in 0..fft_rows {
            data[y * fft_cols + x] = col_buf[y];
        }
    }

    let norm = 1.0 / (fft_rows * fft_cols) as f64;
    data.iter_mut().for_each(|c| *c *= norm);
}

fn build_real_surface(data: &[Complex<f64>], rows: usize, cols: usize) -> Vec<f64> {
    data.iter().take(rows * cols).map(|c| c.re).collect()
}

fn find_peak(surface: &[f64], rows: usize, cols: usize) -> (usize, usize, f64) {
    let mut best_y = 0;
    let mut best_x = 0;
    let mut best_val = f64::NEG_INFINITY;

    for y in 0..rows {
        for x in 0..cols {
            let v = surface[y * cols + x];
            if v > best_val {
                best_val = v;
                best_y = y;
                best_x = x;
            }
        }
    }

    (best_y, best_x, best_val)
}

fn compute_confidence(surface: &[f64], rows: usize, cols: usize, peak_val: f64) -> f64 {
    let n = rows * cols;
    if n == 0 {
        return 0.0;
    }
    let sum: f64 = surface.iter().sum();
    let mean = sum / n as f64;
    if mean.abs() < 1e-15 {
        return 0.0;
    }
    peak_val / mean
}

fn subpixel_refine_1d(
    surface: &[f64],
    rows: usize,
    cols: usize,
    peak_y: usize,
    peak_x: usize,
    axis_y: bool,
) -> f64 {
    let (center, prev, next) = if axis_y {
        let py = if peak_y == 0 { rows - 1 } else { peak_y - 1 };
        let ny = if peak_y == rows - 1 { 0 } else { peak_y + 1 };
        (
            surface[peak_y * cols + peak_x],
            surface[py * cols + peak_x],
            surface[ny * cols + peak_x],
        )
    } else {
        let px = if peak_x == 0 { cols - 1 } else { peak_x - 1 };
        let nx = if peak_x == cols - 1 { 0 } else { peak_x + 1 };
        (
            surface[peak_y * cols + peak_x],
            surface[peak_y * cols + px],
            surface[peak_y * cols + nx],
        )
    };

    let denom = 2.0 * (2.0 * center - prev - next);
    if denom.abs() < 1e-15 {
        return 0.0;
    }
    ((prev - next) / denom).clamp(-0.5, 0.5)
}

fn hann_window(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let t = std::f64::consts::PI * 2.0 * i as f64 / n as f64;
            0.5 * (1.0 - t.cos())
        })
        .collect()
}

fn next_power_of_2(n: usize) -> usize {
    let mut p = 1;
    while p < n {
        p <<= 1;
    }
    p
}

fn is_constant_or_zero(img: &Array2<f32>) -> bool {
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

    finite_count < 16 || (max_val - min_val).abs() < 1e-10
}

pub fn is_low_confidence(confidence: f64) -> bool {
    confidence < CONFIDENCE_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pattern(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            ((y as f32 * 0.3).sin() * (x as f32 * 0.2).cos() * 1000.0) + 500.0
                + ((y * 7 + x * 13) as f32 * 0.01).sin() * 200.0
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
}
