pub fn compute_detection_snr(peak_above_background: f64, background_sigma: f64) -> f64 {
    if background_sigma <= f64::EPSILON {
        return 0.0;
    }
    peak_above_background / background_sigma
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detection_snr_basic() {
        let snr = compute_detection_snr(100.0, 10.0);
        assert!((snr - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_detection_snr_zero_sigma() {
        let snr = compute_detection_snr(100.0, 0.0);
        assert!((snr - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_detection_snr_epsilon_sigma() {
        let snr = compute_detection_snr(100.0, f64::EPSILON);
        assert!((snr - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_detection_snr_negative_peak() {
        let snr = compute_detection_snr(-5.0, 10.0);
        assert!((snr - (-0.5)).abs() < 1e-10);
    }
}
