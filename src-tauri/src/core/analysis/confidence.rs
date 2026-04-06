use crate::math::normalization::{compute_mean_sigma, compute_snr};

pub fn compute_detection_snr(peak_above_background: f64, background_sigma: f64) -> f64 {
    if background_sigma <= f64::EPSILON {
        return 0.0;
    }
    peak_above_background / background_sigma
}

pub fn compute_surface_confidence(surface: &[f64], peak_value: f64) -> f64 {
    if surface.is_empty() {
        return 0.0;
    }
    let (mean, sigma) = compute_mean_sigma(surface);
    if sigma <= f64::EPSILON {
        return 0.0;
    }
    compute_snr(peak_value, mean, sigma)
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

    #[test]
    fn test_surface_confidence_basic() {
        let surface = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let conf = compute_surface_confidence(&surface, 10.0);
        assert!(conf > 0.0);
    }

    #[test]
    fn test_surface_confidence_empty() {
        let surface: Vec<f64> = vec![];
        let conf = compute_surface_confidence(&surface, 10.0);
        assert!((conf - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_surface_confidence_constant() {
        let surface = vec![5.0; 100];
        let conf = compute_surface_confidence(&surface, 10.0);
        assert!((conf - 0.0).abs() < 1e-10);
    }
}
