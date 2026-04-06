use rayon::prelude::*;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftNum, FftPlanner};
use std::sync::Arc;

pub trait FftFloat: FftNum + PartialOrd + Send + Sync + 'static {
    fn from_usize(n: usize) -> Self;
    fn from_f64(v: f64) -> Self;
    fn cos_val(self) -> Self;
    fn sin_val(self) -> Self;
    fn sqrt_val(self) -> Self;
    fn abs_val(self) -> Self;
    fn ln1p_val(self) -> Self;
    fn is_finite_val(self) -> bool;
    fn max_of(self, other: Self) -> Self;
    fn min_of(self, other: Self) -> Self;
    fn neg_infinity_val() -> Self;
    fn infinity_val() -> Self;
    fn epsilon_val() -> Self;
    fn half() -> Self;
    fn two() -> Self;
    fn pi() -> Self;
}

impl FftFloat for f32 {
    #[inline] fn from_usize(n: usize) -> Self { n as f32 }
    #[inline] fn from_f64(v: f64) -> Self { v as f32 }
    #[inline] fn cos_val(self) -> Self { self.cos() }
    #[inline] fn sin_val(self) -> Self { self.sin() }
    #[inline] fn sqrt_val(self) -> Self { self.sqrt() }
    #[inline] fn abs_val(self) -> Self { self.abs() }
    #[inline] fn ln1p_val(self) -> Self { (1.0 + self).ln() }
    #[inline] fn is_finite_val(self) -> bool { self.is_finite() }
    #[inline] fn max_of(self, other: Self) -> Self { self.max(other) }
    #[inline] fn min_of(self, other: Self) -> Self { self.min(other) }
    #[inline] fn neg_infinity_val() -> Self { f32::NEG_INFINITY }
    #[inline] fn infinity_val() -> Self { f32::INFINITY }
    #[inline] fn epsilon_val() -> Self { 1e-15 }
    #[inline] fn half() -> Self { 0.5 }
    #[inline] fn two() -> Self { 2.0 }
    #[inline] fn pi() -> Self { std::f32::consts::PI }
}

impl FftFloat for f64 {
    #[inline] fn from_usize(n: usize) -> Self { n as f64 }
    #[inline] fn from_f64(v: f64) -> Self { v }
    #[inline] fn cos_val(self) -> Self { self.cos() }
    #[inline] fn sin_val(self) -> Self { self.sin() }
    #[inline] fn sqrt_val(self) -> Self { self.sqrt() }
    #[inline] fn abs_val(self) -> Self { self.abs() }
    #[inline] fn ln1p_val(self) -> Self { (1.0 + self).ln() }
    #[inline] fn is_finite_val(self) -> bool { self.is_finite() }
    #[inline] fn max_of(self, other: Self) -> Self { self.max(other) }
    #[inline] fn min_of(self, other: Self) -> Self { self.min(other) }
    #[inline] fn neg_infinity_val() -> Self { f64::NEG_INFINITY }
    #[inline] fn infinity_val() -> Self { f64::INFINITY }
    #[inline] fn epsilon_val() -> Self { 1e-15 }
    #[inline] fn half() -> Self { 0.5 }
    #[inline] fn two() -> Self { 2.0 }
    #[inline] fn pi() -> Self { std::f64::consts::PI }
}

#[inline]
pub fn next_power_of_two(n: usize) -> usize {
    n.next_power_of_two()
}

pub fn transpose<T: FftFloat>(data: &[Complex<T>], rows: usize, cols: usize) -> Vec<Complex<T>> {
    let zero = Complex::new(T::zero(), T::zero());
    let mut out = vec![zero; rows * cols];
    out.par_chunks_mut(rows)
        .enumerate()
        .for_each(|(x, col_buf)| {
            for y in 0..rows {
                col_buf[y] = data[y * cols + x];
            }
        });
    out
}

pub fn transpose_back<T: FftFloat>(
    transposed: &[Complex<T>],
    dst: &mut [Complex<T>],
    orig_rows: usize,
    orig_cols: usize,
) {
    dst.par_chunks_mut(orig_cols)
        .enumerate()
        .for_each(|(y, row_buf)| {
            for x in 0..orig_cols {
                row_buf[x] = transposed[x * orig_rows + y];
            }
        });
}

pub struct FftEngine2D<T: FftFloat> {
    pub fft_rows: usize,
    pub fft_cols: usize,
    fwd_row: Arc<dyn Fft<T>>,
    fwd_col: Arc<dyn Fft<T>>,
    inv_row: Arc<dyn Fft<T>>,
    inv_col: Arc<dyn Fft<T>>,
}

impl<T: FftFloat> FftEngine2D<T> {
    pub fn new(fft_rows: usize, fft_cols: usize) -> Self {
        let mut planner = FftPlanner::<T>::new();
        Self {
            fft_rows,
            fft_cols,
            fwd_row: planner.plan_fft_forward(fft_cols),
            fwd_col: planner.plan_fft_forward(fft_rows),
            inv_row: planner.plan_fft_inverse(fft_cols),
            inv_col: planner.plan_fft_inverse(fft_rows),
        }
    }

    pub fn from_image_dims(rows: usize, cols: usize) -> Self {
        Self::new(next_power_of_two(rows), next_power_of_two(cols))
    }

    pub fn from_padded_dims(rows: usize, cols: usize, pad_rows: usize, pad_cols: usize) -> Self {
        Self::new(
            (rows + pad_rows).next_power_of_two(),
            (cols + pad_cols).next_power_of_two(),
        )
    }

    pub fn total_size(&self) -> usize {
        self.fft_rows * self.fft_cols
    }

    pub fn alloc_buffer(&self) -> Vec<Complex<T>> {
        vec![Complex::new(T::zero(), T::zero()); self.total_size()]
    }

    pub fn forward_2d(&self, buf: &mut [Complex<T>]) {
        buf.par_chunks_mut(self.fft_cols).for_each(|row| {
            self.fwd_row.process(row);
        });

        let mut col_major = transpose(buf, self.fft_rows, self.fft_cols);
        col_major.par_chunks_mut(self.fft_rows).for_each(|col| {
            self.fwd_col.process(col);
        });

        transpose_back(&col_major, buf, self.fft_rows, self.fft_cols);
    }

    pub fn inverse_2d(&self, buf: &mut [Complex<T>]) {
        buf.par_chunks_mut(self.fft_cols).for_each(|row| {
            self.inv_row.process(row);
        });

        let mut col_major = transpose(buf, self.fft_rows, self.fft_cols);
        col_major.par_chunks_mut(self.fft_rows).for_each(|col| {
            self.inv_col.process(col);
        });

        transpose_back(&col_major, buf, self.fft_rows, self.fft_cols);

        let norm = T::one() / <T as FftFloat>::from_usize(self.fft_rows * self.fft_cols);
        buf.par_iter_mut().for_each(|c| {
            c.re = c.re * norm;
            c.im = c.im * norm;
        });
    }

    pub fn forward_2d_alloc(&self, data: &[Complex<T>]) -> Vec<Complex<T>> {
        let mut buf = data.to_vec();
        self.forward_2d(&mut buf);
        buf
    }

    pub fn inverse_2d_alloc(&self, data: &[Complex<T>]) -> Vec<Complex<T>> {
        let mut buf = data.to_vec();
        self.inverse_2d(&mut buf);
        buf
    }

    #[inline]
    pub fn fwd_row_plan(&self) -> &Arc<dyn Fft<T>> {
        &self.fwd_row
    }

    #[inline]
    pub fn fwd_col_plan(&self) -> &Arc<dyn Fft<T>> {
        &self.fwd_col
    }

    #[inline]
    pub fn inv_row_plan(&self) -> &Arc<dyn Fft<T>> {
        &self.inv_row
    }

    #[inline]
    pub fn inv_col_plan(&self) -> &Arc<dyn Fft<T>> {
        &self.inv_col
    }
}

pub fn prepare_windowed_buffer<T: FftFloat>(
    image: &ndarray::Array2<f32>,
    win_y: &[T],
    win_x: &[T],
    fft_rows: usize,
    fft_cols: usize,
) -> Vec<Complex<T>> {
    let (rows, cols) = image.dim();
    let zero = Complex::new(T::zero(), T::zero());
    let mut buf = vec![zero; fft_rows * fft_cols];
    for y in 0..rows {
        let wy = win_y[y];
        let base = y * fft_cols;
        for x in 0..cols {
            let v = <T as FftFloat>::from_f64(image[[y, x]] as f64);
            let windowed = if v.is_finite_val() {
                v * wy * win_x[x]
            } else {
                T::zero()
            };
            buf[base + x] = Complex::new(windowed, T::zero());
        }
    }
    buf
}

pub fn prepare_buffer_no_window<T: FftFloat>(
    image: &ndarray::Array2<f32>,
    fft_rows: usize,
    fft_cols: usize,
) -> Vec<Complex<T>> {
    let (rows, cols) = image.dim();
    let zero = Complex::new(T::zero(), T::zero());
    let mut buf = vec![zero; fft_rows * fft_cols];
    for y in 0..rows {
        let base = y * fft_cols;
        for x in 0..cols {
            let v = <T as FftFloat>::from_f64(image[[y, x]] as f64);
            let val = if v.is_finite_val() { v } else { T::zero() };
            buf[base + x] = Complex::new(val, T::zero());
        }
    }
    buf
}

pub fn extract_real<T: FftFloat>(data: &[Complex<T>], rows: usize, cols: usize) -> Vec<T> {
    data.iter().take(rows * cols).map(|c| c.re).collect()
}

pub fn shifted_log_magnitude<T: FftFloat>(
    data: &[Complex<T>],
    rows: usize,
    cols: usize,
) -> Vec<T> {
    let half_r = rows / 2;
    let half_c = cols / 2;
    (0..rows * cols)
        .into_par_iter()
        .map(|idx| {
            let r = idx / cols;
            let c = idx % cols;
            let sr = (r + half_r) % rows;
            let sc = (c + half_c) % cols;
            let mag = super::complex::norm(data[sr * cols + sc]);
            T::ln1p_val(mag)
        })
        .collect()
}

pub fn find_peak<T: FftFloat>(surface: &[T], cols: usize) -> (usize, usize, T) {
    if surface.is_empty() {
        return (0, 0, T::zero());
    }
    let (best_idx, best_val) = surface
        .par_iter()
        .enumerate()
        .reduce_with(|a, b| if *b.1 > *a.1 { b } else { a })
        .map(|(i, v)| (i, *v))
        .unwrap_or((0, surface[0]));
    (best_idx / cols, best_idx % cols, best_val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_power_of_two() {
        assert_eq!(next_power_of_two(1), 1);
        assert_eq!(next_power_of_two(3), 4);
        assert_eq!(next_power_of_two(128), 128);
        assert_eq!(next_power_of_two(129), 256);
    }

    #[test]
    fn test_transpose_square() {
        let data: Vec<Complex<f64>> = (0..9)
            .map(|i| Complex::new(i as f64, 0.0))
            .collect();
        let t = transpose(&data, 3, 3);
        assert!((t[0].re - 0.0).abs() < 1e-10);
        assert!((t[1].re - 3.0).abs() < 1e-10);
        assert!((t[2].re - 6.0).abs() < 1e-10);
        assert!((t[3].re - 1.0).abs() < 1e-10);
        assert!((t[4].re - 4.0).abs() < 1e-10);
        assert!((t[5].re - 7.0).abs() < 1e-10);
    }

    #[test]
    fn test_transpose_rect() {
        let data: Vec<Complex<f64>> = (0..6)
            .map(|i| Complex::new(i as f64, 0.0))
            .collect();
        let t = transpose(&data, 2, 3);
        assert_eq!(t.len(), 6);
        assert!((t[0].re - 0.0).abs() < 1e-10);
        assert!((t[1].re - 3.0).abs() < 1e-10);
        assert!((t[2].re - 1.0).abs() < 1e-10);
        assert!((t[3].re - 4.0).abs() < 1e-10);
        assert!((t[4].re - 2.0).abs() < 1e-10);
        assert!((t[5].re - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_transpose_back_roundtrip() {
        let data: Vec<Complex<f64>> = (0..12)
            .map(|i| Complex::new(i as f64, (i as f64) * 0.1))
            .collect();
        let t = transpose(&data, 3, 4);
        let mut recovered = vec![Complex::new(0.0, 0.0); 12];
        transpose_back(&t, &mut recovered, 3, 4);
        for (a, b) in data.iter().zip(recovered.iter()) {
            assert!((a.re - b.re).abs() < 1e-10);
            assert!((a.im - b.im).abs() < 1e-10);
        }
    }

    #[test]
    fn test_fft_ifft_roundtrip_f64() {
        let engine = FftEngine2D::<f64>::new(8, 8);
        let original: Vec<Complex<f64>> = (0..64)
            .map(|i| Complex::new((i as f64 * 0.1).sin(), 0.0))
            .collect();
        let mut buf = original.clone();
        engine.forward_2d(&mut buf);
        engine.inverse_2d(&mut buf);
        for (a, b) in original.iter().zip(buf.iter()) {
            assert!(
                (a.re - b.re).abs() < 1e-10,
                "re mismatch: {} vs {}",
                a.re,
                b.re
            );
            assert!(
                (a.im - b.im).abs() < 1e-10,
                "im mismatch: {} vs {}",
                a.im,
                b.im
            );
        }
    }

    #[test]
    fn test_fft_ifft_roundtrip_f32() {
        let engine = FftEngine2D::<f32>::new(16, 16);
        let original: Vec<Complex<f32>> = (0..256)
            .map(|i| Complex::new((i as f32 * 0.1).sin(), 0.0))
            .collect();
        let mut buf = original.clone();
        engine.forward_2d(&mut buf);
        engine.inverse_2d(&mut buf);
        for (a, b) in original.iter().zip(buf.iter()) {
            assert!(
                (a.re - b.re).abs() < 1e-4,
                "re mismatch: {} vs {}",
                a.re,
                b.re
            );
        }
    }

    #[test]
    fn test_fft_ifft_rectangular() {
        let engine = FftEngine2D::<f64>::new(8, 16);
        let original: Vec<Complex<f64>> = (0..128)
            .map(|i| Complex::new(i as f64 * 0.01, 0.0))
            .collect();
        let mut buf = original.clone();
        engine.forward_2d(&mut buf);
        engine.inverse_2d(&mut buf);
        for (a, b) in original.iter().zip(buf.iter()) {
            assert!(
                (a.re - b.re).abs() < 1e-10,
                "rect roundtrip: {} vs {}",
                a.re,
                b.re
            );
        }
    }

    #[test]
    fn test_from_image_dims() {
        let engine = FftEngine2D::<f64>::from_image_dims(100, 200);
        assert_eq!(engine.fft_rows, 128);
        assert_eq!(engine.fft_cols, 256);
    }

    #[test]
    fn test_find_peak() {
        let mut surface = vec![0.0f64; 16];
        surface[5] = 10.0;
        let (r, c, val) = find_peak(&surface, 4);
        assert_eq!(r, 1);
        assert_eq!(c, 1);
        assert!((val - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_alloc_buffer() {
        let engine = FftEngine2D::<f32>::new(4, 8);
        let buf = engine.alloc_buffer();
        assert_eq!(buf.len(), 32);
        assert!((buf[0].re - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_prepare_windowed_buffer() {
        let img = ndarray::Array2::from_shape_fn((4, 4), |(r, c)| (r * 4 + c) as f32);
        let win_y = vec![1.0f64; 4];
        let win_x = vec![1.0f64; 4];
        let buf = prepare_windowed_buffer(&img, &win_y, &win_x, 8, 8);
        assert_eq!(buf.len(), 64);
        assert!((buf[0].re - 0.0).abs() < 1e-10);
        assert!((buf[1].re - 1.0).abs() < 1e-10);
        assert!((buf[8 + 2].re - 6.0).abs() < 1e-10);
        assert!((buf[4 * 8].re - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_extract_real() {
        let data = vec![
            Complex::new(1.0f64, 0.5),
            Complex::new(2.0, 0.3),
            Complex::new(3.0, 0.1),
            Complex::new(4.0, 0.2),
        ];
        let real = extract_real(&data, 2, 2);
        assert_eq!(real.len(), 4);
        assert!((real[0] - 1.0).abs() < 1e-10);
        assert!((real[3] - 4.0).abs() < 1e-10);
    }
}
