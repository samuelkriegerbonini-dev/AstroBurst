use super::fft::FftFloat;

pub fn hann_periodic<T: FftFloat>(n: usize) -> Vec<T> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![T::one()];
    }
    let two_pi = T::two() * T::pi();
    let nf = <T as FftFloat>::from_usize(n);
    (0..n)
        .map(|i| {
            let phase = two_pi * <T as FftFloat>::from_usize(i) / nf;
            T::half() * (T::one() - phase.cos_val())
        })
        .collect()
}

pub fn hann_symmetric<T: FftFloat>(n: usize) -> Vec<T> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![T::one()];
    }
    let two_pi = T::two() * T::pi();
    let denom = <T as FftFloat>::from_usize(n - 1).max_of(T::one());
    (0..n)
        .map(|i| {
            let phase = two_pi * <T as FftFloat>::from_usize(i) / denom;
            T::half() * (T::one() - phase.cos_val())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hann_periodic_length() {
        let w: Vec<f64> = hann_periodic(128);
        assert_eq!(w.len(), 128);
    }

    #[test]
    fn test_hann_periodic_first_zero() {
        let w: Vec<f64> = hann_periodic(128);
        assert!(w[0].abs() < 1e-10);
    }

    #[test]
    fn test_hann_periodic_not_zero_at_end() {
        let w: Vec<f64> = hann_periodic(128);
        assert!(w[127] > 1e-5);
    }

    #[test]
    fn test_hann_periodic_peak() {
        let w: Vec<f64> = hann_periodic(128);
        let max_val = w.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!((max_val - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_hann_symmetric_endpoints_zero() {
        let w: Vec<f64> = hann_symmetric(128);
        assert!(w[0].abs() < 1e-10);
        assert!(w[127].abs() < 1e-10);
    }

    #[test]
    fn test_hann_symmetric_peak_at_center() {
        let w: Vec<f64> = hann_symmetric(129);
        let center = 64;
        assert!((w[center] - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_hann_symmetric_symmetry() {
        let w: Vec<f64> = hann_symmetric(128);
        for i in 0..64 {
            assert!(
                (w[i] - w[127 - i]).abs() < 1e-10,
                "asymmetry at i={}: {} vs {}",
                i,
                w[i],
                w[127 - i]
            );
        }
    }

    #[test]
    fn test_hann_periodic_symmetry() {
        let w: Vec<f64> = hann_periodic(128);
        for i in 1..64 {
            assert!(
                (w[i] - w[128 - i]).abs() < 1e-10,
                "asymmetry at i={}: {} vs {}",
                i,
                w[i],
                w[128 - i]
            );
        }
    }

    #[test]
    fn test_hann_periodic_f32() {
        let w: Vec<f32> = hann_periodic(64);
        assert_eq!(w.len(), 64);
        assert!(w[0].abs() < 1e-6);
    }

    #[test]
    fn test_hann_empty() {
        let w: Vec<f64> = hann_periodic(0);
        assert!(w.is_empty());
    }

    #[test]
    fn test_hann_single() {
        let w: Vec<f64> = hann_periodic(1);
        assert_eq!(w.len(), 1);
        assert!((w[0] - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_periodic_vs_symmetric_differ() {
        let p: Vec<f64> = hann_periodic(128);
        let s: Vec<f64> = hann_symmetric(128);
        let mut any_diff = false;
        for (a, b) in p.iter().zip(s.iter()) {
            if (a - b).abs() > 1e-10 {
                any_diff = true;
                break;
            }
        }
        assert!(any_diff);
    }

    #[test]
    fn test_hann_values_in_range() {
        let w: Vec<f64> = hann_periodic(256);
        for v in &w {
            assert!(*v >= -1e-10);
            assert!(*v <= 1.0 + 1e-10);
        }
    }
}
