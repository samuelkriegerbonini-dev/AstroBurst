use rayon::prelude::*;
use rustfft::num_complex::Complex;

use super::fft::FftFloat;

#[inline]
pub fn norm<T: FftFloat>(c: Complex<T>) -> T {
    (c.re * c.re + c.im * c.im).sqrt_val()
}

#[inline]
pub fn norm_sqr<T: FftFloat>(c: Complex<T>) -> T {
    c.re * c.re + c.im * c.im
}

#[inline]
pub fn safe_normalize<T: FftFloat>(c: Complex<T>, epsilon: T) -> Complex<T> {
    let mag = norm(c);
    if mag > epsilon {
        Complex::new(c.re / mag, c.im / mag)
    } else {
        Complex::new(T::zero(), T::zero())
    }
}

#[inline]
pub fn cross_power_element<T: FftFloat>(a: Complex<T>, b: Complex<T>, epsilon: T) -> Complex<T> {
    let product = Complex::new(
        a.re * b.re + a.im * b.im,
        a.im * b.re - a.re * b.im,
    );
    safe_normalize(product, epsilon)
}

pub fn cross_power_spectrum<T: FftFloat>(
    fa: &[Complex<T>],
    fb: &[Complex<T>],
    epsilon: T,
) -> Vec<Complex<T>> {
    fa.par_iter()
        .zip(fb.par_iter())
        .map(|(&a, &b)| cross_power_element(a, b, epsilon))
        .collect()
}

pub fn pointwise_multiply<T: FftFloat>(
    a: &[Complex<T>],
    b: &[Complex<T>],
) -> Vec<Complex<T>> {
    a.par_iter()
        .zip(b.par_iter())
        .map(|(&x, &y)| Complex::new(
            x.re * y.re - x.im * y.im,
            x.re * y.im + x.im * y.re,
        ))
        .collect()
}

pub fn pointwise_multiply_into<T: FftFloat>(
    buf: &mut [Complex<T>],
    freq: &[Complex<T>],
) {
    buf.par_iter_mut()
        .zip(freq.par_iter())
        .for_each(|(b, &f)| {
            let re = b.re * f.re - b.im * f.im;
            let im = b.re * f.im + b.im * f.re;
            b.re = re;
            b.im = im;
        });
}

pub fn conjugate_slice<T: FftFloat>(data: &[Complex<T>]) -> Vec<Complex<T>> {
    data.par_iter()
        .map(|c| Complex::new(c.re, T::zero() - c.im))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_norm_zero() {
        let c = Complex::new(0.0f64, 0.0);
        assert!((norm(c) - 0.0).abs() < 1e-15);
    }

    #[test]
    fn test_norm_unit() {
        let c = Complex::new(3.0f64, 4.0);
        assert!((norm(c) - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_norm_sqr_exact() {
        let c = Complex::new(3.0f64, 4.0);
        assert!((norm_sqr(c) - 25.0).abs() < 1e-10);
    }

    #[test]
    fn test_safe_normalize_normal() {
        let c = Complex::new(3.0f64, 4.0);
        let n = safe_normalize(c, 1e-15);
        let mag = norm(n);
        assert!((mag - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_safe_normalize_near_zero() {
        let c = Complex::new(1e-20f64, 1e-20);
        let n = safe_normalize(c, 1e-15);
        assert!((n.re - 0.0).abs() < 1e-15);
        assert!((n.im - 0.0).abs() < 1e-15);
    }

    #[test]
    fn test_cross_power_element_identity() {
        let a = Complex::new(3.0f64, 4.0);
        let result = cross_power_element(a, a, 1e-15);
        let mag = norm(result);
        assert!((mag - 1.0).abs() < 1e-10);
        assert!(result.im.abs() < 1e-10);
    }

    #[test]
    fn test_cross_power_spectrum_parallel() {
        let fa: Vec<Complex<f64>> = (0..100)
            .map(|i| Complex::new(i as f64, (i as f64) * 0.5))
            .collect();
        let fb: Vec<Complex<f64>> = (0..100)
            .map(|i| Complex::new(i as f64 + 1.0, (i as f64) * 0.3))
            .collect();
        let result = cross_power_spectrum(&fa, &fb, 1e-15);
        assert_eq!(result.len(), 100);
        assert!((result[0].re - 0.0).abs() < 1e-15);
        for c in &result[1..] {
            let mag = norm(*c);
            assert!((mag - 1.0).abs() < 1e-10 || mag < 1e-10);
        }
    }

    #[test]
    fn test_pointwise_multiply() {
        let a = vec![Complex::new(1.0f64, 2.0), Complex::new(3.0, 4.0)];
        let b = vec![Complex::new(5.0f64, 6.0), Complex::new(7.0, 8.0)];
        let result = pointwise_multiply(&a, &b);
        assert!((result[0].re - (-7.0)).abs() < 1e-10);
        assert!((result[0].im - 16.0).abs() < 1e-10);
    }

    #[test]
    fn test_pointwise_multiply_into() {
        let mut buf = vec![Complex::new(1.0f64, 2.0)];
        let freq = vec![Complex::new(3.0f64, 4.0)];
        pointwise_multiply_into(&mut buf, &freq);
        assert!((buf[0].re - (-5.0)).abs() < 1e-10);
        assert!((buf[0].im - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_conjugate_slice() {
        let data = vec![
            Complex::new(1.0f64, 2.0),
            Complex::new(3.0, -4.0),
        ];
        let conj = conjugate_slice(&data);
        assert!((conj[0].re - 1.0).abs() < 1e-10);
        assert!((conj[0].im - (-2.0)).abs() < 1e-10);
        assert!((conj[1].re - 3.0).abs() < 1e-10);
        assert!((conj[1].im - 4.0).abs() < 1e-10);
    }

    #[test]
    fn test_f32_operations() {
        let c = Complex::new(3.0f32, 4.0);
        let n = norm(c);
        assert!((n - 5.0).abs() < 1e-5);
        let normalized = safe_normalize(c, 1e-15f32);
        assert!((norm(normalized) - 1.0).abs() < 1e-5);
    }
}
