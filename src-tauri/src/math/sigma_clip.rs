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
        let before = values.len();
        values.retain(|&v| v >= lo && v <= hi);
        if values.len() == before {
            break;
        }
    }

    if values.is_empty() {
        return (f64::NAN, f64::NAN);
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
    fn test_empty_reports_no_measurement() {
        let mut vals: Vec<f32> = vec![];
        let (med, sig) = sigma_clipped_stats(&mut vals, 3.0, 3);
        assert!(med.is_nan(), "median of nothing must not be a number, got {med}");
        assert!(sig.is_nan(), "sigma of nothing must not be a number, got {sig}");
        assert_eq!(serde_json::json!({ "median": med, "std": sig }), serde_json::json!({ "median": null, "std": null }));
    }

    #[test]
    fn test_no_survivors_reports_no_measurement() {
        for kappa in [-3.0f32, f32::NAN] {
            let mut vals: Vec<f32> = (1..=50).map(|i| i as f32).collect();
            let (med, sig) = sigma_clipped_stats(&mut vals, kappa, 3);
            assert!(vals.is_empty(), "kappa {kappa} should reject every value");
            assert!(med.is_nan() && sig.is_nan(), "kappa {kappa}: got median {med}, sigma {sig}");
        }
    }

    fn ramp_with_outlier() -> Vec<f32> {
        let mut vals: Vec<f32> = (1..=100).map(|i| i as f32).collect();
        vals.push(100_000.0);
        vals
    }

    #[test]
    fn test_stops_once_a_pass_rejects_nothing() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut vals = ramp_with_outlier();
            let result = sigma_clipped_stats(&mut vals, 3.0, usize::MAX);
            let _ = tx.send((result, vals.len()));
        });
        let ((med, sig), survivors) = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("clipping kept iterating after a pass rejected nothing");
        let mut reference = ramp_with_outlier();
        let expected = sigma_clipped_stats(&mut reference, 3.0, 3);
        assert_eq!((med, sig), expected);
        assert_eq!(survivors, reference.len());
        assert_eq!(survivors, 100);
    }

    #[test]
    fn test_clean_data() {
        let mut vals: Vec<f32> = (1..=100).map(|i| i as f32).collect();
        let (med, _sig) = sigma_clipped_stats(&mut vals, 3.0, 3);
        assert!(med > 45.0 && med < 55.0);
    }
}
