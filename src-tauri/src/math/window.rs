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

pub fn tukey<T: FftFloat>(n: usize, alpha: T) -> Vec<T> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![T::one()];
    }
    let nf = <T as FftFloat>::from_usize(n - 1);
    let half_alpha_n = alpha * nf * T::half();
    (0..n)
        .map(|i| {
            let x = <T as FftFloat>::from_usize(i);
            if alpha <= T::zero() {
                T::one()
            } else if x < half_alpha_n {
                let phase = T::pi() * x / half_alpha_n;
                T::half() * (T::one() - phase.cos_val())
            } else if x > nf - half_alpha_n {
                let phase = T::pi() * (nf - x) / half_alpha_n;
                T::half() * (T::one() - phase.cos_val())
            } else {
                T::one()
            }
        })
        .collect()
}

pub fn cosine_bell<T: FftFloat>(n: usize) -> Vec<T> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![T::one()];
    }
    let nf_m1 = <T as FftFloat>::from_usize(n - 1).max_of(T::one());
    (0..n)
        .map(|i| {
            let phase = T::pi() * <T as FftFloat>::from_usize(i) / nf_m1;
            phase.sin_val()
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowType {
    HannPeriodic,
    HannSymmetric,
    Tukey,
    CosineBell,
    None,
}

pub fn generate_window<T: FftFloat>(window_type: WindowType, n: usize, param: T) -> Vec<T> {
    match window_type {
        WindowType::HannPeriodic => hann_periodic(n),
        WindowType::HannSymmetric => hann_symmetric(n),
        WindowType::Tukey => tukey(n, param),
        WindowType::CosineBell => cosine_bell(n),
        WindowType::None => vec![T::one(); n],
    }
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
    fn test_tukey_alpha_zero_is_rectangular() {
        let w: Vec<f64> = tukey(64, 0.0);
        for v in &w {
            assert!((v - 1.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_tukey_alpha_one_is_hann_like() {
        let w: Vec<f64> = tukey(64, 1.0);
        assert!(w[0].abs() < 1e-10);
        assert!(w[32] > 0.99);
    }

    #[test]
    fn test_cosine_bell_endpoints() {
        let w: Vec<f64> = cosine_bell(64);
        assert!(w[0].abs() < 1e-10);
        assert!(w[63].abs() < 1e-10);
    }

    #[test]
    fn test_cosine_bell_peak() {
        let w: Vec<f64> = cosine_bell(65);
        assert!((w[32] - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_generate_window_dispatch() {
        let w: Vec<f64> = generate_window(WindowType::None, 10, 0.0);
        for v in &w {
            assert!((v - 1.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_generate_window_hann_periodic() {
        let w1: Vec<f64> = generate_window(WindowType::HannPeriodic, 64, 0.0);
        let w2: Vec<f64> = hann_periodic(64);
        for (a, b) in w1.iter().zip(w2.iter()) {
            assert!((a - b).abs() < 1e-15);
        }
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
