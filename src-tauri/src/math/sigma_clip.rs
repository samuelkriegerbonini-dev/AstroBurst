use super::median::{exact_median_f64, exact_median_mut, median_f32_mut};
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClippedStats {
    pub mean: f64,
    pub median: f64,
    pub std: f64,
    pub count: usize,
}

fn mean_and_std(values: &[f64]) -> (f64, f64) {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n;
    (mean, variance.sqrt())
}

pub fn std_clipped_stats(values: &[f64], sigma: f64, maxiters: usize) -> ClippedStats {
    let mut kept: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    for _ in 0..maxiters {
        if kept.is_empty() {
            break;
        }
        let centre = exact_median_f64(&kept);
        let (_, std) = mean_and_std(&kept);
        let (low, high) = (centre - sigma * std, centre + sigma * std);
        let before = kept.len();
        kept.retain(|&v| v >= low && v <= high);
        if kept.len() == before {
            break;
        }
    }
    if kept.is_empty() {
        return ClippedStats { mean: f64::NAN, median: f64::NAN, std: f64::NAN, count: 0 };
    }
    let (mean, std) = mean_and_std(&kept);
    ClippedStats { mean, median: exact_median_f64(&kept), std, count: kept.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-12 * b.abs().max(1.0)
    }

    #[test]
    fn std_clip_rejects_beyond_sigma_std_of_the_median_and_reports_survivor_stats() {
        let stats = std_clipped_stats(&[1.0, 2.0, 3.0, 4.0, 5.0, 100.0], 2.0, 5);
        assert_eq!(stats.count, 5);
        assert!(close(stats.mean, 3.0) && close(stats.median, 3.0), "{stats:?}");
        assert!(close(stats.std, 2f64.sqrt()), "population std, not sample std: {stats:?}");
        let loose = std_clipped_stats(&[1.0, 2.0, 3.0, 4.0, 5.0, 100.0], 3.0, 5);
        assert_eq!(loose.count, 6);
        assert!(close(loose.mean, 115.0 / 6.0) && close(loose.median, 3.5), "{loose:?}");
    }

    #[test]
    fn std_clip_keeps_values_on_the_bounds_and_ignores_non_finite_values() {
        let flat = std_clipped_stats(&[1.0, 1.0, f64::NAN, 1.0, f64::INFINITY, 1.0], 3.0, 5);
        assert_eq!(flat, ClippedStats { mean: 1.0, median: 1.0, std: 0.0, count: 4 });
        let empty = std_clipped_stats(&[f64::NAN, f64::NEG_INFINITY], 3.0, 5);
        assert_eq!(empty.count, 0);
        assert!(empty.mean.is_nan() && empty.median.is_nan() && empty.std.is_nan(), "{empty:?}");
    }

    #[test]
    fn std_clip_stops_after_maxiters_passes() {
        let mut values = vec![0.0; 10];
        values.extend([5.0, 50.0]);
        let once = std_clipped_stats(&values, 2.0, 1);
        assert_eq!(once.count, 11);
        assert!(close(once.mean, 5.0 / 11.0), "{once:?}");
        let converged = std_clipped_stats(&values, 2.0, 5);
        assert_eq!(converged, ClippedStats { mean: 0.0, median: 0.0, std: 0.0, count: 10 });
    }

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
