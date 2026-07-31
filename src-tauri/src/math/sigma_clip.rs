use super::median::{exact_median_mut, median_f32_mut};
use crate::types::constants::MAD_TO_SIGMA;

fn robust_or_std_sigma(mad: f64, values: &[f32], center: f64) -> f64 {
    let robust = mad * MAD_TO_SIGMA;
    if robust > 0.0 {
        return robust;
    }
    if values.len() < 2 {
        return 0.0;
    }
    let var = values
        .iter()
        .map(|&v| {
            let d = v as f64 - center;
            d * d
        })
        .sum::<f64>()
        / (values.len() - 1) as f64;
    var.sqrt()
}

pub fn sigma_clipped_stats(values: &mut Vec<f32>, kappa: f32, iterations: usize) -> (f64, f64) {
    let mut devs: Vec<f32> = Vec::with_capacity(values.len());

    for _ in 0..iterations {
        if values.len() < 3 {
            break;
        }

        let median = exact_median_mut(values);

        devs.clear();
        devs.extend(values.iter().map(|&v| (v as f64 - median).abs() as f32));
        let mad = median_f32_mut(&mut devs) as f64;
        let sig = robust_or_std_sigma(mad, values, median);
        if sig <= 0.0 {
            break;
        }

        let lo = (median - kappa as f64 * sig) as f32;
        let hi = (median + kappa as f64 * sig) as f32;
        values.retain(|&v| v >= lo && v <= hi);
    }

    if values.is_empty() {
        return (0.0, 1.0);
    }

    let median = exact_median_mut(values);
    devs.clear();
    devs.extend(values.iter().map(|&v| (v as f64 - median).abs() as f32));
    let mad = median_f32_mut(&mut devs) as f64;
    let sigma = robust_or_std_sigma(mad, values, median);

    (median, sigma)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_with_outliers() {
        let mut vals: Vec<f32> = (1..=100).map(|i| i as f32).collect();
        vals.push(100_000.0);
        let (med, sig) = sigma_clipped_stats(&mut vals, 3.0, 3);
        assert!(med > 40.0 && med < 60.0);
        assert!(sig < 500.0);
    }

    #[test]
    fn test_empty() {
        let mut vals: Vec<f32> = vec![];
        let (med, sig) = sigma_clipped_stats(&mut vals, 3.0, 3);
        assert_eq!(med, 0.0);
        assert_eq!(sig, 1.0);
    }

    #[test]
    fn test_clean_data() {
        let mut vals: Vec<f32> = (1..=100).map(|i| i as f32).collect();
        let (med, _sig) = sigma_clipped_stats(&mut vals, 3.0, 3);
        assert!(med > 45.0 && med < 55.0);
    }
}
