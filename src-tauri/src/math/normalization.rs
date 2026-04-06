use super::fft::FftFloat;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NormStrategy {
    MinMax,
    ZScore,
    UnitEnergy,
}

pub fn normalize_strategy<T: FftFloat>(data: &mut [T], strategy: NormStrategy) {
    match strategy {
        NormStrategy::MinMax => min_max_normalize(data),
        NormStrategy::ZScore => z_score_normalize(data),
        NormStrategy::UnitEnergy => unit_energy_normalize(data),
    }
}

pub fn min_max_normalize<T: FftFloat>(data: &mut [T]) {
    if data.is_empty() {
        return;
    }

    let mut min_val = T::infinity_val();
    let mut max_val = T::neg_infinity_val();
    for &v in data.iter() {
        if v.is_finite_val() {
            min_val = min_val.min_of(v);
            max_val = max_val.max_of(v);
        }
    }

    if !min_val.is_finite_val() || !max_val.is_finite_val() {
        return;
    }

    let range = max_val - min_val;
    if range.abs_val() < T::epsilon_val() {
        for v in data.iter_mut() {
            if v.is_finite_val() {
                *v = T::zero();
            }
        }
        return;
    }

    let inv_range = T::one() / range;
    for v in data.iter_mut() {
        if v.is_finite_val() {
            *v = (*v - min_val) * inv_range;
        } else {
            *v = T::zero();
        }
    }
}

pub fn z_score_normalize<T: FftFloat>(data: &mut [T]) {
    if data.len() < 2 {
        return;
    }

    let mut sum = T::zero();
    let mut count = T::zero();
    for &v in data.iter() {
        if v.is_finite_val() {
            sum = sum + v;
            count = count + T::one();
        }
    }

    if count < T::two() {
        return;
    }

    let mean = sum / count;

    let mut var_sum = T::zero();
    for &v in data.iter() {
        if v.is_finite_val() {
            let d = v - mean;
            var_sum = var_sum + d * d;
        }
    }

    let sigma = (var_sum / (count - T::one())).sqrt_val();
    if sigma.abs_val() < T::epsilon_val() {
        for v in data.iter_mut() {
            *v = T::zero();
        }
        return;
    }

    let inv_sigma = T::one() / sigma;
    for v in data.iter_mut() {
        if v.is_finite_val() {
            *v = (*v - mean) * inv_sigma;
        } else {
            *v = T::zero();
        }
    }
}

pub fn unit_energy_normalize<T: FftFloat>(data: &mut [T]) {
    if data.is_empty() {
        return;
    }

    let mut energy = T::zero();
    for &v in data.iter() {
        if v.is_finite_val() {
            energy = energy + v * v;
        }
    }

    if energy.abs_val() < T::epsilon_val() {
        return;
    }

    let inv_norm = T::one() / energy.sqrt_val();
    for v in data.iter_mut() {
        if v.is_finite_val() {
            *v = *v * inv_norm;
        } else {
            *v = T::zero();
        }
    }
}

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
    fn test_min_max_basic() {
        let mut data = vec![1.0f64, 2.0, 3.0, 4.0, 5.0];
        min_max_normalize(&mut data);
        assert!((data[0] - 0.0).abs() < 1e-10);
        assert!((data[4] - 1.0).abs() < 1e-10);
        assert!((data[2] - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_min_max_constant() {
        let mut data = vec![5.0f64; 10];
        min_max_normalize(&mut data);
        for v in &data {
            assert!((v - 0.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_min_max_empty() {
        let mut data: Vec<f64> = vec![];
        min_max_normalize(&mut data);
        assert!(data.is_empty());
    }

    #[test]
    fn test_min_max_with_nan() {
        let mut data = vec![1.0f32, f32::NAN, 3.0, f32::INFINITY, 5.0];
        min_max_normalize(&mut data);
        assert!((data[0] - 0.0).abs() < 1e-5);
        assert!((data[2] - 0.5).abs() < 1e-5);
        assert!((data[4] - 1.0).abs() < 1e-5);
        assert!((data[1] - 0.0).abs() < 1e-5);
        assert!((data[3] - 0.0).abs() < 1e-5);
    }

    #[test]
    fn test_z_score_basic() {
        let mut data = vec![2.0f64, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        z_score_normalize(&mut data);
        let (mean, sigma) = compute_mean_sigma(&data);
        assert!(mean.abs() < 1e-10);
        assert!((sigma - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_z_score_constant() {
        let mut data = vec![5.0f64; 10];
        z_score_normalize(&mut data);
        for v in &data {
            assert!(v.abs() < 1e-10);
        }
    }

    #[test]
    fn test_z_score_too_few() {
        let mut data = vec![5.0f64];
        z_score_normalize(&mut data);
        assert!((data[0] - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_unit_energy_basic() {
        let mut data = vec![3.0f64, 4.0];
        unit_energy_normalize(&mut data);
        let energy: f64 = data.iter().map(|v| v * v).sum();
        assert!((energy - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_unit_energy_zero() {
        let mut data = vec![0.0f64; 10];
        unit_energy_normalize(&mut data);
        for v in &data {
            assert!((v - 0.0).abs() < 1e-10);
        }
    }

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

    #[test]
    fn test_normalize_strategy_dispatch() {
        let mut data = vec![1.0f64, 2.0, 3.0, 4.0, 5.0];
        normalize_strategy(&mut data, NormStrategy::MinMax);
        assert!((data[0] - 0.0).abs() < 1e-10);
        assert!((data[4] - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_f32_min_max() {
        let mut data = vec![10.0f32, 20.0, 30.0];
        min_max_normalize(&mut data);
        assert!((data[0] - 0.0).abs() < 1e-5);
        assert!((data[1] - 0.5).abs() < 1e-5);
        assert!((data[2] - 1.0).abs() < 1e-5);
    }
}
