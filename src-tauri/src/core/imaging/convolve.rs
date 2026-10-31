use std::collections::HashMap;
use std::sync::Arc;

use ndarray::Array2;
use rayon::prelude::*;
use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::math::complex::pointwise_multiply_into;
use crate::math::fft::{FftEngine2D, FftFloat};

pub const MAX_NORMALIZATION: f64 = 100.0;
pub const NORMALIZATION_ZERO_TOL: f64 = 1e-8;
pub const UNKNOWN_SUPPORT_FRACTION: f64 = 1e-3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NanTreatment {
    Interpolate,
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvolveOptions {
    pub nan_treatment: NanTreatment,
    pub fill_value: f64,
    pub normalize_kernel: bool,
    pub normalization_zero_tol: f64,
    pub preserve_nan: bool,
    pub min_wt: f64,
}

impl Default for ConvolveOptions {
    fn default() -> Self {
        Self {
            nan_treatment: NanTreatment::Interpolate,
            fill_value: 0.0,
            normalize_kernel: true,
            normalization_zero_tol: NORMALIZATION_ZERO_TOL,
            preserve_nan: false,
            min_wt: 0.0,
        }
    }
}

impl ConvolveOptions {
    pub fn lightstack() -> Self {
        Self { normalize_kernel: false, ..Self::default() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConvolveReport {
    pub kernel_sum: f64,
    pub kernel_negative_fraction: f64,
    pub padded_rows: usize,
    pub padded_cols: usize,
    pub nan_pixels: usize,
    pub filled_nan_pixels: usize,
    pub renormalized_pixels: usize,
    pub weight_above_one_pixels: usize,
    pub zero_weight_pixels: usize,
    pub weight_min: f64,
    pub weight_max: f64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Convolved<T> {
    pub image: Array2<T>,
    pub variance: Option<Array2<T>>,
    pub weight: Option<Array2<T>>,
    pub report: ConvolveReport,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ConvolveError {
    #[error("the image is empty")]
    EmptyImage,
    #[error("the kernel is empty")]
    EmptyKernel,
    #[error("the kernel can't be normalized: its sum {sum} is below {}", 1.0 / MAX_NORMALIZATION)]
    KernelNotNormalizable { sum: f64 },
    #[error("cannot interpolate NaNs with an unnormalizable kernel: its sum {sum} is within {tolerance} of zero")]
    UnnormalizableInterpolation { sum: f64, tolerance: f64 },
    #[error("fill_value must be finite when NaNs are filled")]
    NonFiniteFill,
    #[error("the variance plane is {variance_cols}x{variance_rows} but the image is {cols}x{rows}")]
    VarianceShape { rows: usize, cols: usize, variance_rows: usize, variance_cols: usize },
}

pub struct FftPlanCache<T: FftFloat> {
    engines: HashMap<(usize, usize), Arc<FftEngine2D<T>>>,
}

impl<T: FftFloat> Default for FftPlanCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: FftFloat> FftPlanCache<T> {
    pub fn new() -> Self {
        Self { engines: HashMap::new() }
    }

    pub fn engine(&mut self, rows: usize, cols: usize) -> Arc<FftEngine2D<T>> {
        self.engines.entry((rows, cols)).or_insert_with(|| Arc::new(FftEngine2D::new(rows, cols))).clone()
    }

    pub fn len(&self) -> usize {
        self.engines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.engines.is_empty()
    }
}

fn is_five_smooth(mut n: usize) -> bool {
    if n == 0 {
        return false;
    }
    for prime in [2, 3, 5] {
        while n % prime == 0 {
            n /= prime;
        }
    }
    n == 1
}

pub fn padded_len(len: usize, kernel_len: usize) -> usize {
    let minimum = (len + kernel_len / 2).max(1);
    (minimum..).find(|&n| is_five_smooth(n)).expect("5-smooth numbers are unbounded")
}

struct PreparedKernel {
    normalized: Array2<f64>,
    scale: f64,
    sum: f64,
    negative_fraction: f64,
}

fn prepare_kernel(kernel: &Array2<f64>, options: &ConvolveOptions) -> Result<PreparedKernel, ConvolveError> {
    if kernel.is_empty() {
        return Err(ConvolveError::EmptyKernel);
    }
    let cleaned = kernel.mapv(|v| if v.is_finite() { v } else { 0.0 });
    let sum: f64 = cleaned.iter().sum();
    let negative_fraction = cleaned.iter().filter(|&&v| v < 0.0).count() as f64 / cleaned.len() as f64;
    let (scale, normalized) = if options.normalize_kernel {
        if sum < 1.0 / MAX_NORMALIZATION {
            return Err(ConvolveError::KernelNotNormalizable { sum });
        }
        (1.0, cleaned.mapv(|v| v / sum))
    } else if sum.abs() < options.normalization_zero_tol {
        if options.nan_treatment == NanTreatment::Interpolate {
            return Err(ConvolveError::UnnormalizableInterpolation { sum, tolerance: options.normalization_zero_tol });
        }
        (1.0, cleaned)
    } else {
        (sum, cleaned.mapv(|v| v / sum))
    };
    Ok(PreparedKernel { normalized, scale, sum, negative_fraction })
}

fn kernel_spectrum<T: FftFloat>(kernel: &Array2<f64>, engine: &FftEngine2D<T>, scratch: &mut Vec<Complex<T>>) -> Vec<Complex<T>> {
    let (rows, cols) = (engine.fft_rows as isize, engine.fft_cols as isize);
    let (centre_row, centre_col) = ((kernel.nrows() / 2) as isize, (kernel.ncols() / 2) as isize);
    let mut buffer = engine.alloc_buffer();
    for ((r, c), &value) in kernel.indexed_iter() {
        if value == 0.0 {
            continue;
        }
        let row = (r as isize - centre_row).rem_euclid(rows) as usize;
        let col = (c as isize - centre_col).rem_euclid(cols) as usize;
        let slot = &mut buffer[row * cols as usize + col];
        slot.re = slot.re + <T as FftFloat>::from_f64(value);
    }
    engine.forward_2d_with_scratch(&mut buffer, scratch);
    buffer
}

fn padded_plane<T: FftFloat>(
    engine: &FftEngine2D<T>,
    (rows, cols): (usize, usize),
    inside: impl Fn(usize, usize) -> T + Sync,
    outside: T,
) -> Vec<Complex<T>> {
    let zero = T::zero();
    let mut buffer = vec![Complex::new(outside, zero); engine.total_size()];
    buffer.par_chunks_mut(engine.fft_cols).take(rows).enumerate().for_each(|(r, row)| {
        for (c, slot) in row.iter_mut().take(cols).enumerate() {
            *slot = Complex::new(inside(r, c), zero);
        }
    });
    buffer
}

fn convolved<T: FftFloat>(
    mut buffer: Vec<Complex<T>>,
    spectrum: &[Complex<T>],
    engine: &FftEngine2D<T>,
    scratch: &mut Vec<Complex<T>>,
) -> Vec<Complex<T>> {
    engine.forward_2d_with_scratch(&mut buffer, scratch);
    pointwise_multiply_into(&mut buffer, spectrum);
    engine.inverse_2d_with_scratch(&mut buffer, scratch);
    buffer
}

fn warnings_for(report: &ConvolveReport, options: &ConvolveOptions) -> Vec<String> {
    let mut warnings = Vec::new();
    if report.kernel_negative_fraction > 0.0 && report.renormalized_pixels > 0 {
        warnings.push(format!(
            "the kernel has negative lobes ({:.1} percent of its pixels); {} output pixels near NaN holes were divided by normalization weights from {:.4} to {:.4} ({} above 1), so values there are the least reliable",
            100.0 * report.kernel_negative_fraction,
            report.renormalized_pixels,
            report.weight_min,
            report.weight_max,
            report.weight_above_one_pixels
        ));
    }
    if report.zero_weight_pixels > 0 {
        let outcome = if options.min_wt > 0.0 { format!("set to NaN (min_wt {})", options.min_wt) } else { "set to 0".to_string() };
        warnings.push(format!("{} output pixels had no usable input under the kernel and were {outcome}", report.zero_weight_pixels));
    }
    warnings
}

fn convolve_with<T: FftFloat>(
    image: &Array2<T>,
    variance: Option<&Array2<T>>,
    kernel: &Array2<f64>,
    options: &ConvolveOptions,
    plans: &mut FftPlanCache<T>,
) -> Result<Convolved<T>, ConvolveError> {
    let (rows, cols) = image.dim();
    if rows == 0 || cols == 0 {
        return Err(ConvolveError::EmptyImage);
    }
    if let Some(variance) = variance {
        if variance.dim() != image.dim() {
            let (variance_rows, variance_cols) = variance.dim();
            return Err(ConvolveError::VarianceShape { rows, cols, variance_rows, variance_cols });
        }
    }
    let interpolate = options.nan_treatment == NanTreatment::Interpolate;
    if !interpolate && !options.fill_value.is_finite() {
        return Err(ConvolveError::NonFiniteFill);
    }
    let prepared = prepare_kernel(kernel, options)?;
    let (fft_rows, fft_cols) = (padded_len(rows, kernel.nrows()), padded_len(cols, kernel.ncols()));
    let engine = plans.engine(fft_rows, fft_cols);
    let mut scratch = Vec::new();
    let (zero, one) = (T::zero(), T::one());
    let fill_finite = options.fill_value.is_finite();
    let outside = if fill_finite { <T as FftFloat>::from_f64(options.fill_value) } else { zero };
    let filled = if interpolate { zero } else { <T as FftFloat>::from_f64(options.fill_value) };
    let valid = |r: usize, c: usize| image[[r, c]].is_finite_val();

    let spectrum = kernel_spectrum(&prepared.normalized, &engine, &mut scratch);
    let numerator = convolved(
        padded_plane(&engine, (rows, cols), |r, c| if valid(r, c) { image[[r, c]] } else { filled }, outside),
        &spectrum,
        &engine,
        &mut scratch,
    );
    let weight = interpolate.then(|| {
        let plane = padded_plane(&engine, (rows, cols), |r, c| if valid(r, c) { one } else { zero }, if fill_finite { one } else { zero });
        convolved(plane, &spectrum, &engine, &mut scratch)
    });

    let scale = <T as FftFloat>::from_f64(prepared.scale);
    let min_wt = <T as FftFloat>::from_f64(options.min_wt);
    let tiny = <T as FftFloat>::from_f64(10.0) * T::machine_epsilon();
    let weight_at = |r: usize, c: usize| weight.as_ref().map_or(one, |w| w[r * fft_cols + c].re);
    let starved = |w: T| interpolate && if options.min_wt > 0.0 { w < min_wt } else { w < tiny };
    let mut out = Array2::<T>::zeros((rows, cols));
    out.as_slice_mut().expect("standard layout").par_chunks_mut(cols).enumerate().for_each(|(r, row)| {
        for (c, slot) in row.iter_mut().enumerate() {
            let w = weight_at(r, c);
            let n = numerator[r * fft_cols + c].re;
            *slot = if !interpolate {
                scale * n
            } else if starved(w) {
                if options.min_wt > 0.0 { <T as FftFloat>::from_f64(f64::NAN) } else { zero }
            } else {
                scale * n / w
            };
            if options.preserve_nan && !valid(r, c) {
                *slot = <T as FftFloat>::from_f64(f64::NAN);
            }
        }
    });

    let tolerance = 100.0 * T::machine_epsilon().as_f64();
    let mut report = ConvolveReport {
        kernel_sum: prepared.sum,
        kernel_negative_fraction: prepared.negative_fraction,
        padded_rows: fft_rows,
        padded_cols: fft_cols,
        nan_pixels: 0,
        filled_nan_pixels: 0,
        renormalized_pixels: 0,
        weight_above_one_pixels: 0,
        zero_weight_pixels: 0,
        weight_min: 1.0,
        weight_max: 1.0,
        warnings: Vec::new(),
    };
    let mut extrema: Option<(f64, f64)> = None;
    for ((r, c), value) in out.indexed_iter() {
        if !valid(r, c) {
            report.nan_pixels += 1;
            if value.is_finite_val() {
                report.filled_nan_pixels += 1;
            }
        }
        if interpolate {
            let w = weight_at(r, c);
            let wf = w.as_f64();
            extrema = Some(extrema.map_or((wf, wf), |(low, high)| (low.min(wf), high.max(wf))));
            if (wf - 1.0).abs() > tolerance {
                report.renormalized_pixels += 1;
            }
            if wf > 1.0 + tolerance {
                report.weight_above_one_pixels += 1;
            }
            if starved(w) {
                report.zero_weight_pixels += 1;
            }
        }
    }
    if let Some((low, high)) = extrema {
        report.weight_min = low;
        report.weight_max = high;
    }
    report.warnings = warnings_for(&report, options);

    let variance = variance.map(|variance| {
        let squared = prepared.normalized.mapv(|v| v * v);
        let squared_spectrum = kernel_spectrum(&squared, &engine, &mut scratch);
        let known = |r: usize, c: usize| valid(r, c) && variance[[r, c]].is_finite_val();
        let summed = convolved(
            padded_plane(&engine, (rows, cols), |r, c| if known(r, c) { variance[[r, c]] } else { zero }, zero),
            &squared_spectrum,
            &engine,
            &mut scratch,
        );
        let unknown_support = (0..rows).any(|r| (0..cols).any(|c| valid(r, c) && !known(r, c))).then(|| {
            let magnitude = prepared.normalized.mapv(f64::abs);
            let peak = magnitude.iter().fold(0.0f64, |acc, &v| acc.max(v));
            let support_spectrum = kernel_spectrum(&magnitude, &engine, &mut scratch);
            let plane = padded_plane(&engine, (rows, cols), |r, c| if valid(r, c) && !known(r, c) { one } else { zero }, zero);
            (convolved(plane, &support_spectrum, &engine, &mut scratch), <T as FftFloat>::from_f64(UNKNOWN_SUPPORT_FRACTION * peak))
        });
        let squared_scale = scale * scale;
        Array2::from_shape_fn((rows, cols), |(r, c)| {
            let w = weight_at(r, c);
            let reaches_unknown = unknown_support.as_ref().is_some_and(|(support, floor)| support[r * fft_cols + c].re > *floor);
            if !out[[r, c]].is_finite_val() || starved(w) || reaches_unknown {
                return <T as FftFloat>::from_f64(f64::NAN);
            }
            squared_scale * summed[r * fft_cols + c].re / (w * w)
        })
    });

    Ok(Convolved { image: out, variance, weight: interpolate.then(|| Array2::from_shape_fn((rows, cols), |(r, c)| weight_at(r, c))), report })
}

pub fn convolve_fft<T: FftFloat>(
    image: &Array2<T>,
    kernel: &Array2<f64>,
    options: &ConvolveOptions,
    plans: &mut FftPlanCache<T>,
) -> Result<Convolved<T>, ConvolveError> {
    convolve_with(image, None, kernel, options, plans)
}

pub fn convolve_fft_with_variance<T: FftFloat>(
    image: &Array2<T>,
    variance: &Array2<T>,
    kernel: &Array2<f64>,
    options: &ConvolveOptions,
    plans: &mut FftPlanCache<T>,
) -> Result<Convolved<T>, ConvolveError> {
    convolve_with(image, Some(variance), kernel, options, plans)
}

#[cfg(test)]
pub(crate) mod reference {
    use ndarray::Array2;

    use super::{ConvolveOptions, NanTreatment, MAX_NORMALIZATION};

    pub fn direct_convolve(image: &Array2<f64>, kernel: &Array2<f64>, options: &ConvolveOptions) -> Array2<f64> {
        let (rows, cols) = image.dim();
        let (k_rows, k_cols) = kernel.dim();
        let kernel = kernel.mapv(|v| if v.is_finite() { v } else { 0.0 });
        let sum: f64 = kernel.iter().sum();
        let (scale, normalized) = if options.normalize_kernel {
            assert!(sum >= 1.0 / MAX_NORMALIZATION);
            (1.0, kernel.mapv(|v| v / sum))
        } else if sum.abs() < options.normalization_zero_tol {
            (1.0, kernel.clone())
        } else {
            (sum, kernel.mapv(|v| v / sum))
        };
        let interpolate = options.nan_treatment == NanTreatment::Interpolate;
        let fill_finite = options.fill_value.is_finite();
        let sample = |r: isize, c: isize| -> (f64, f64) {
            if r < 0 || c < 0 || r >= rows as isize || c >= cols as isize {
                return if fill_finite { (options.fill_value, 1.0) } else { (0.0, 0.0) };
            }
            let v = image[[r as usize, c as usize]];
            match (v.is_finite(), interpolate) {
                (true, _) => (v, 1.0),
                (false, true) => (0.0, 0.0),
                (false, false) => (options.fill_value, 1.0),
            }
        };
        let mut out = Array2::<f64>::zeros((rows, cols));
        for r in 0..rows {
            for c in 0..cols {
                let (mut numerator, mut weight) = (0.0, 0.0);
                for kr in 0..k_rows {
                    for kc in 0..k_cols {
                        let k = normalized[[kr, kc]];
                        if k == 0.0 {
                            continue;
                        }
                        let (v, w) = sample(r as isize + (k_rows / 2) as isize - kr as isize, c as isize + (k_cols / 2) as isize - kc as isize);
                        numerator += k * v;
                        weight += k * w;
                    }
                }
                out[[r, c]] = if interpolate {
                    let value = scale * numerator / weight;
                    if options.min_wt > 0.0 {
                        if weight < options.min_wt { f64::NAN } else { value }
                    } else if weight < 10.0 * f64::EPSILON {
                        0.0
                    } else {
                        value
                    }
                } else {
                    scale * numerator
                };
                if options.preserve_nan && !image[[r, c]].is_finite() {
                    out[[r, c]] = f64::NAN;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::reference::direct_convolve;
    use super::*;

    fn row(values: &[f64]) -> Array2<f64> {
        Array2::from_shape_vec((1, values.len()), values.to_vec()).unwrap()
    }

    fn gaussian(size: usize, sigma: f64) -> Array2<f64> {
        let c = (size as f64 - 1.0) / 2.0;
        Array2::from_shape_fn((size, size), |(r, k)| {
            let (dy, dx) = (r as f64 - c, k as f64 - c);
            (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp()
        })
    }

    fn normalized(kernel: Array2<f64>) -> Array2<f64> {
        let sum = kernel.sum();
        kernel.mapv(|v| v / sum)
    }

    fn difference_of_gaussians(size: usize) -> Array2<f64> {
        let core = normalized(gaussian(size, 1.2));
        let wide = normalized(gaussian(size, 3.0));
        normalized(core.mapv(|v| 1.6 * v) - wide.mapv(|v| 0.6 * v))
    }

    fn scene(rows: usize, cols: usize) -> Array2<f64> {
        Array2::from_shape_fn((rows, cols), |(r, c)| {
            let (y, x) = (r as f64, c as f64);
            2.0 + 0.05 * x - 0.03 * y + 40.0 * (-((x - 11.3).powi(2) + (y - 9.6).powi(2)) / 3.1).exp() + 15.0 * (-((x - 25.0).powi(2) + (y - 22.0).powi(2)) / 1.4).exp()
        })
    }

    fn with_holes(mut image: Array2<f64>, holes: &[(usize, usize, usize, usize)]) -> Array2<f64> {
        for &(r0, r1, c0, c1) in holes {
            for r in r0..r1 {
                for c in c0..c1 {
                    image[[r, c]] = f64::NAN;
                }
            }
        }
        image
    }

    fn max_abs(image: &Array2<f64>) -> f64 {
        image.iter().filter(|v| v.is_finite()).fold(0.0f64, |acc, v| acc.max(v.abs()))
    }

    fn assert_same(got: &Array2<f64>, expected: &Array2<f64>, relative: f64, what: &str) -> f64 {
        assert_eq!(got.dim(), expected.dim(), "{what}: shape");
        let scale = max_abs(expected);
        let mut worst = 0.0f64;
        for ((index, g), e) in got.indexed_iter().zip(expected.iter()) {
            assert_eq!(g.is_nan(), e.is_nan(), "{what}: NaN pattern at {index:?}: {g} against {e}");
            if e.is_finite() {
                worst = worst.max((g - e).abs());
            }
        }
        assert!(worst <= relative * scale, "{what}: max difference {worst:e} is above {relative:e} of {scale:e}");
        worst / scale
    }

    fn run(image: &Array2<f64>, kernel: &Array2<f64>, options: &ConvolveOptions) -> Convolved<f64> {
        convolve_fft(image, kernel, options, &mut FftPlanCache::new()).unwrap()
    }

    #[test]
    fn matches_astropy_convolve_fft_with_nan_holes() {
        let defaults = ConvolveOptions::default();
        let documented: [(&[f64], &[f64], ConvolveOptions, &[f64]); 6] = [
            (&[1.0, 0.0, 3.0], &[1.0, 1.0, 1.0], defaults, &[1.0 / 3.0, 4.0 / 3.0, 1.0]),
            (&[1.0, f64::NAN, 3.0], &[1.0, 1.0, 1.0], defaults, &[0.5, 2.0, 1.5]),
            (&[1.0, 0.0, 3.0], &[0.0, 1.0, 0.0], defaults, &[1.0, 0.0, 3.0]),
            (&[1.0, 2.0, 3.0], &[1.0], defaults, &[1.0, 2.0, 3.0]),
            (&[1.0, f64::NAN, 3.0], &[0.0, 1.0, 0.0], defaults, &[1.0, 0.0, 3.0]),
            (&[1.0, f64::NAN, 3.0], &[0.0, 1.0, 0.0], ConvolveOptions { min_wt: 1e-8, ..defaults }, &[1.0, f64::NAN, 3.0]),
        ];
        for (image, kernel, options, expected) in documented {
            let got = run(&row(image), &row(kernel), &options).image;
            for (g, e) in got.iter().zip(expected) {
                assert!(
                    (g.is_nan() && e.is_nan()) || (g - e).abs() < 1e-12,
                    "convolve_fft({image:?}, {kernel:?}) gives {got:?}, astropy documents {expected:?}"
                );
            }
        }

        let image = with_holes(scene(40, 36), &[(9, 12, 10, 14), (30, 32, 3, 5), (0, 2, 20, 23)]);
        for (kernel, what) in [(difference_of_gaussians(15), "odd kernel with negative lobes"), (normalized(gaussian(8, 1.7)), "even kernel")] {
            for options in [ConvolveOptions::lightstack(), ConvolveOptions::default(), ConvolveOptions { nan_treatment: NanTreatment::Fill, fill_value: 0.5, ..ConvolveOptions::lightstack() }] {
                let got = run(&image, &kernel.mapv(|v| 0.97 * v), &options).image;
                let expected = direct_convolve(&image, &kernel.mapv(|v| 0.97 * v), &options);
                assert_same(&got, &expected, 1e-9, &format!("{what}, {options:?}"));
            }
        }
    }

    #[test]
    fn preserve_nan_keeps_holes_nan() {
        let image = with_holes(scene(24, 30), &[(5, 8, 6, 9), (20, 22, 0, 3)]);
        let kernel = normalized(gaussian(7, 1.3));
        let kept = run(&image, &kernel, &ConvolveOptions { preserve_nan: true, ..ConvolveOptions::lightstack() });
        let filled = run(&image, &kernel, &ConvolveOptions::lightstack());
        for ((index, source), (value, interpolated)) in image.indexed_iter().zip(kept.image.iter().zip(filled.image.iter())) {
            assert_eq!(value.is_nan(), source.is_nan(), "preserve_nan at {index:?}");
            assert!(interpolated.is_finite(), "interpolation leaves a NaN at {index:?}");
        }
        assert_eq!(kept.report.nan_pixels, 15);
        assert_eq!(kept.report.filled_nan_pixels, 0);
        assert_eq!(filled.report.filled_nan_pixels, 15);
    }

    #[test]
    fn no_wraparound_at_image_edges() {
        let mut image = Array2::<f64>::zeros((20, 26));
        image[[10, 0]] = 1000.0;
        image[[0, 13]] = 500.0;
        let kernel = normalized(gaussian(11, 2.5));
        let got = run(&image, &kernel, &ConvolveOptions::lightstack()).image;
        for r in 0..20 {
            for c in 20..26 {
                assert!(got[[r, c]].abs() < 1e-9, "light wrapped to the right edge at ({r}, {c}): {}", got[[r, c]]);
            }
        }
        for c in 0..26 {
            assert!(got[[19, c]].abs() < 1e-9, "light wrapped to the bottom edge at column {c}: {}", got[[19, c]]);
        }
        assert_same(&got, &direct_convolve(&image, &kernel, &ConvolveOptions::lightstack()), 1e-12, "edge pixel");

        let narrow = scene(6, 5);
        let large = normalized(gaussian(21, 4.0));
        let got = run(&narrow, &large, &ConvolveOptions::lightstack()).image;
        assert_same(&got, &direct_convolve(&narrow, &large, &ConvolveOptions::lightstack()), 1e-12, "kernel larger than the image");
    }

    #[test]
    fn negative_kernel_lobes_near_mask_edges_are_reported() {
        let hole = (12, 16, 14, 18);
        let image = with_holes(scene(32, 34), &[hole]);
        let lobed = difference_of_gaussians(15);
        assert!(lobed.iter().any(|&v| v < 0.0));
        let result = run(&image, &lobed, &ConvolveOptions::lightstack());
        let report = &result.report;
        assert!(report.kernel_negative_fraction > 0.0);
        assert!(report.weight_above_one_pixels > 0, "{report:?}");
        assert!(report.weight_max > 1.0, "{report:?}");
        assert!(report.renormalized_pixels >= report.weight_above_one_pixels);
        assert!(report.warnings.iter().any(|w| w.contains("negative lobes")), "{:?}", report.warnings);
        let weight = result.weight.as_ref().unwrap();
        let reach = 7;
        for ((r, c), &w) in weight.indexed_iter() {
            if (w - 1.0).abs() > 1e-9 {
                assert!(
                    r + reach >= hole.0 && r < hole.1 + reach && c + reach >= hole.2 && c < hole.3 + reach,
                    "pixel ({r}, {c}) far from the hole was renormalized by {w}"
                );
            }
        }

        let positive = run(&image, &normalized(gaussian(15, 1.5)), &ConvolveOptions::lightstack()).report;
        assert_eq!(positive.weight_above_one_pixels, 0);
        assert!(positive.weight_max <= 1.0 + 1e-12);
        assert!(positive.renormalized_pixels > 0);
        assert!(!positive.warnings.iter().any(|w| w.contains("negative lobes")), "{:?}", positive.warnings);
    }

    #[test]
    fn kernels_that_cannot_be_normalized_are_refused() {
        let image = scene(8, 8);
        let tiny = row(&[0.001, 0.002, 0.001]);
        match convolve_fft(&image, &tiny, &ConvolveOptions::default(), &mut FftPlanCache::new()).unwrap_err() {
            ConvolveError::KernelNotNormalizable { sum } => assert!((sum - 0.004).abs() < 1e-15, "{sum}"),
            other => panic!("{other:?}"),
        }
        let balanced = row(&[-1.0, 2.0, -1.0]);
        let error = convolve_fft(&image, &balanced, &ConvolveOptions::lightstack(), &mut FftPlanCache::new()).unwrap_err();
        assert!(matches!(error, ConvolveError::UnnormalizableInterpolation { .. }), "{error:?}");
        let filled = ConvolveOptions { nan_treatment: NanTreatment::Fill, ..ConvolveOptions::lightstack() };
        let got = run(&image, &balanced, &filled).image;
        assert_same(&got, &direct_convolve(&image, &balanced, &filled), 1e-12, "zero-sum kernel with fill");
    }

    #[test]
    fn plan_cache_reuses_engines_by_padded_shape() {
        let mut plans = FftPlanCache::<f64>::new();
        let kernel = normalized(gaussian(9, 1.5));
        convolve_fft(&scene(30, 40), &kernel, &ConvolveOptions::default(), &mut plans).unwrap();
        let first = plans.engine(padded_len(30, 9), padded_len(40, 9));
        assert_eq!(plans.len(), 1);
        convolve_fft(&scene(30, 40), &kernel, &ConvolveOptions::lightstack(), &mut plans).unwrap();
        assert!(Arc::ptr_eq(&first, &plans.engine(padded_len(30, 9), padded_len(40, 9))));
        assert_eq!(plans.len(), 1);
        convolve_fft(&scene(50, 40), &kernel, &ConvolveOptions::default(), &mut plans).unwrap();
        assert_eq!(plans.len(), 2);
        for len in [1usize, 7, 17, 97, 1225] {
            for kernel_len in [1usize, 2, 21, 101] {
                let padded = padded_len(len, kernel_len);
                assert!(padded >= len + kernel_len / 2, "{len} + {kernel_len}");
                let mut rest = padded;
                for prime in [2, 3, 5] {
                    while rest % prime == 0 {
                        rest /= prime;
                    }
                }
                assert_eq!(rest, 1, "{padded} is not 5-smooth");
            }
        }
    }

    #[test]
    fn float32_path_stays_within_f_class_of_float64() {
        let image = with_holes(scene(48, 52), &[(20, 24, 20, 23), (0, 2, 0, 52)]);
        let kernel = difference_of_gaussians(21);
        let parity = run(&image, &kernel, &ConvolveOptions::lightstack());
        let fast = convolve_fft(&image.mapv(|v| v as f32), &kernel, &ConvolveOptions::lightstack(), &mut FftPlanCache::new()).unwrap();
        assert_same(&fast.image.mapv(f64::from), &parity.image, 2e-6, "float32 against float64");
        assert_eq!(fast.report.nan_pixels, parity.report.nan_pixels);
    }

    #[test]
    fn d1_planes_with_nan_holes_match_direct_summation_in_both_precisions() {
        let d1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lightstack/d1");
        let psf = |name: &str| crate::core::cubeprep::psf_io::load_psf(&d1.join("psf").join(name), None).unwrap().data;
        let (narrow, broad) = (psf("d1_psf_JWST_F277W_40mas.fits"), psf("d1_psf_JWST_F444W_40mas.fits"));
        let lobed = &narrow * 1.6 - &broad * 0.6;
        assert!(lobed.iter().any(|&v| v < 0.0));
        for (file, kernel, what) in [("d1_JWST_F444W.fits", &broad, "F444W PSF"), ("d1_JWST_F277W.fits", &lobed, "lobed kernel")] {
            let read = crate::infra::fits::reader::extract_image_mmap_by_index(&std::fs::File::open(d1.join(file)).unwrap(), 1).unwrap();
            let fast_input = read.image.as_standard_layout().to_owned();
            let image = fast_input.mapv(f64::from);
            assert!(image.iter().filter(|v| v.is_nan()).count() > 400, "{file}: the D1 NaN border and holes");
            let options = ConvolveOptions::lightstack();
            let expected = direct_convolve(&image, kernel, &options);
            let parity = run(&image, kernel, &options);
            let parity_error = assert_same(&parity.image, &expected, 1e-9, &format!("{file} {what}, float64"));
            let fast = convolve_fft(&fast_input, kernel, &options, &mut FftPlanCache::new()).unwrap();
            let fast_error = assert_same(&fast.image.mapv(f64::from), &expected, 2e-6, &format!("{file} {what}, float32"));
            eprintln!("{file} with the {what}: float64 {parity_error:.2e}, float32 {fast_error:.2e} of the max abs value");
            assert_eq!(parity.report.filled_nan_pixels, parity.report.nan_pixels);
        }
    }

    #[test]
    fn variance_is_convolved_with_the_squared_kernel() {
        let image = with_holes(scene(30, 28), &[(10, 13, 9, 12)]);
        let variance = Array2::from_shape_fn((30, 28), |(r, c)| 0.5 + 0.01 * (r + 2 * c) as f64);
        let kernel = normalized(gaussian(9, 2.0));
        let options = ConvolveOptions::lightstack();
        let result = convolve_fft_with_variance(&image, &variance, &kernel, &options, &mut FftPlanCache::new()).unwrap();
        let got = result.variance.unwrap();
        let mask = image.mapv(|v| if v.is_finite() { 1.0 } else { 0.0 });
        let numerator = direct_convolve(&(&variance * &mask), &kernel.mapv(|v| v * v), &ConvolveOptions { nan_treatment: NanTreatment::Fill, ..options });
        let weight = direct_convolve(&mask, &kernel, &ConvolveOptions { nan_treatment: NanTreatment::Fill, fill_value: 1.0, ..options });
        let expected = Array2::from_shape_fn((30, 28), |(r, c)| numerator[[r, c]] / (weight[[r, c]] * weight[[r, c]]));
        assert_same(&got, &expected, 1e-9, "variance");

        let mut unknown = variance.clone();
        unknown[[25, 3]] = f64::NAN;
        let result = convolve_fft_with_variance(&image, &unknown, &kernel, &options, &mut FftPlanCache::new()).unwrap();
        let got = result.variance.unwrap();
        for ((r, c), v) in got.indexed_iter() {
            let near = r.abs_diff(25) <= 4 && c.abs_diff(3) <= 4;
            assert_eq!(v.is_nan(), near, "unknown variance at (25, 3) reaches ({r}, {c}): {v}");
        }

        let error = convolve_fft_with_variance(&image, &variance.slice(ndarray::s![..29, ..]).to_owned(), &kernel, &options, &mut FftPlanCache::new()).unwrap_err();
        assert_eq!(error, ConvolveError::VarianceShape { rows: 30, cols: 28, variance_rows: 29, variance_cols: 28 });
    }
}
