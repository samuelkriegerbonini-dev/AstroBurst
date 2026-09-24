use super::fft::FftFloat;

pub fn compute_mean_sigma<T: FftFloat>(data: &[T]) -> (T, T) {
    if data.is_empty() {
        return (T::zero(), T::zero());
    }

    let mut sum = T::zero();
    let mut count = T::zero();
    for &v in data.iter() {
        if v.is_finite_val() {
            sum = sum + v;
            count = count + T::one();
        }
    }

    if count < T::one() {
        return (T::zero(), T::zero());
    }

    let mean = sum / count;

    let mut var_sum = T::zero();
    for &v in data.iter() {
        if v.is_finite_val() {
            let d = v - mean;
            var_sum = var_sum + d * d;
        }
    }

    let n_minus_1 = if count > T::one() {
        count - T::one()
    } else {
        T::one()
    };
    let sigma = (var_sum / n_minus_1).sqrt_val();
    (mean, sigma)
}

pub fn compute_snr<T: FftFloat>(peak: T, mean: T, sigma: T) -> T {
    if sigma.abs_val() < T::epsilon_val() {
        return T::zero();
    }
    (peak - mean) / sigma
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_mean_sigma() {
        let data = vec![2.0f64, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let (mean, sigma) = compute_mean_sigma(&data);
        assert!((mean - 5.0).abs() < 1e-10);
        assert!(sigma > 0.0);
    }

    #[test]
    fn test_compute_mean_sigma_empty() {
        let data: Vec<f64> = vec![];
        let (mean, sigma) = compute_mean_sigma(&data);
        assert!((mean - 0.0).abs() < 1e-10);
        assert!((sigma - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_compute_snr() {
        let snr = compute_snr(10.0f64, 2.0, 4.0);
        assert!((snr - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_compute_snr_zero_sigma() {
        let snr = compute_snr(10.0f64, 2.0, 0.0);
        assert!((snr - 0.0).abs() < 1e-10);
    }
}
