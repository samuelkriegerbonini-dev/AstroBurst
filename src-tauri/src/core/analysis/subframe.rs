use ndarray::Array2;
use serde::{Deserialize, Serialize};

use super::star_detection::{detect_stars, DetectedStar};

const DETECTION_SIGMA: f64 = 4.0;
const MIN_STARS_FOR_METRICS: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubframeMetrics {
    pub file_path: String,
    pub file_name: String,
    pub star_count: usize,
    pub median_fwhm: f64,
    pub median_eccentricity: f64,
    pub median_snr: f64,
    pub background_median: f64,
    pub background_sigma: f64,
    pub noise_ratio: f64,
    pub weight: f64,
    pub accepted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubframeWeightConfig {
    pub fwhm_weight: f64,
    pub eccentricity_weight: f64,
    pub snr_weight: f64,
    pub noise_weight: f64,
    pub max_fwhm: f64,
    pub max_eccentricity: f64,
    pub min_snr: f64,
    pub min_stars: usize,
}

impl Default for SubframeWeightConfig {
    fn default() -> Self {
        Self {
            fwhm_weight: 1.0,
            eccentricity_weight: 0.5,
            snr_weight: 1.0,
            noise_weight: 0.3,
            max_fwhm: 8.0,
            max_eccentricity: 0.7,
            min_snr: 5.0,
            min_stars: 5,
        }
    }
}

pub fn analyze_subframe(
    image: &Array2<f32>,
    file_path: &str,
    config: &SubframeWeightConfig,
) -> SubframeMetrics {
    let file_name = file_path
        .split(&['/', '\\'][..])
        .last()
        .unwrap_or(file_path)
        .to_string();

    let result = detect_stars(image, DETECTION_SIGMA);
    let stars = &result.stars;

    if stars.len() < MIN_STARS_FOR_METRICS.min(config.min_stars) {
        return SubframeMetrics {
            file_path: file_path.to_string(),
            file_name,
            star_count: stars.len(),
            median_fwhm: 0.0,
            median_eccentricity: 0.0,
            median_snr: 0.0,
            background_median: result.background_median,
            background_sigma: result.background_sigma,
            noise_ratio: 0.0,
            weight: 0.0,
            accepted: false,
        };
    }

    let median_fwhm = median_of(stars, |s| s.fwhm);
    let median_ecc = median_of(stars, |s| s.eccentricity);
    let median_snr = median_of(stars, |s| s.snr);

    let noise_ratio = if result.background_median > 1e-15 {
        result.background_sigma / result.background_median
    } else {
        0.0
    };

    let weight = compute_weight(median_fwhm, median_ecc, median_snr, noise_ratio, config);

    let accepted = stars.len() >= config.min_stars
        && median_fwhm <= config.max_fwhm
        && median_ecc <= config.max_eccentricity
        && median_snr >= config.min_snr;

    SubframeMetrics {
        file_path: file_path.to_string(),
        file_name,
        star_count: stars.len(),
        median_fwhm,
        median_eccentricity: median_ecc,
        median_snr,
        background_median: result.background_median,
        background_sigma: result.background_sigma,
        noise_ratio,
        weight,
        accepted,
    }
}

fn compute_weight(
    fwhm: f64,
    ecc: f64,
    snr: f64,
    noise: f64,
    config: &SubframeWeightConfig,
) -> f64 {
    let fwhm_score = if fwhm > 0.5 { 1.0 / fwhm } else { 0.0 };
    let ecc_score = 1.0 - ecc;
    let snr_score = snr.ln().max(0.0);
    let noise_score = 1.0 / (1.0 + noise * 10.0);

    let total_weight = config.fwhm_weight + config.eccentricity_weight + config.snr_weight + config.noise_weight;
    if total_weight < 1e-15 {
        return 0.0;
    }

    let raw = config.fwhm_weight * fwhm_score
        + config.eccentricity_weight * ecc_score
        + config.snr_weight * snr_score
        + config.noise_weight * noise_score;

    (raw / total_weight).max(0.0)
}

pub fn normalize_weights(metrics: &mut [SubframeMetrics]) {
    let max_w = metrics
        .iter()
        .map(|m| m.weight)
        .fold(0.0f64, f64::max);

    if max_w > 1e-15 {
        for m in metrics.iter_mut() {
            m.weight /= max_w;
        }
    }
}

fn median_of(stars: &[DetectedStar], f: impl Fn(&DetectedStar) -> f64) -> f64 {
    let mut vals: Vec<f64> = stars.iter().map(|s| f(s)).filter(|v| v.is_finite()).collect();
    if vals.is_empty() {
        return 0.0;
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = vals.len() / 2;
    if vals.len() % 2 == 0 {
        (vals[mid - 1] + vals[mid]) / 2.0
    } else {
        vals[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = SubframeWeightConfig::default();
        assert!(cfg.max_fwhm > 0.0);
        assert!(cfg.min_stars > 0);
    }

    #[test]
    fn test_weight_better_fwhm_scores_higher() {
        let cfg = SubframeWeightConfig::default();
        let w1 = compute_weight(2.0, 0.3, 20.0, 0.01, &cfg);
        let w2 = compute_weight(5.0, 0.3, 20.0, 0.01, &cfg);
        assert!(w1 > w2, "w1={} should be > w2={}", w1, w2);
    }

    #[test]
    fn test_weight_better_ecc_scores_higher() {
        let cfg = SubframeWeightConfig::default();
        let w1 = compute_weight(3.0, 0.1, 20.0, 0.01, &cfg);
        let w2 = compute_weight(3.0, 0.6, 20.0, 0.01, &cfg);
        assert!(w1 > w2, "w1={} should be > w2={}", w1, w2);
    }

    #[test]
    fn test_normalize_weights() {
        let mut metrics = vec![
            SubframeMetrics {
                file_path: "a".into(), file_name: "a".into(),
                star_count: 10, median_fwhm: 2.0, median_eccentricity: 0.2,
                median_snr: 20.0, background_median: 0.1, background_sigma: 0.01,
                noise_ratio: 0.1, weight: 0.5, accepted: true,
            },
            SubframeMetrics {
                file_path: "b".into(), file_name: "b".into(),
                star_count: 10, median_fwhm: 3.0, median_eccentricity: 0.4,
                median_snr: 15.0, background_median: 0.1, background_sigma: 0.02,
                noise_ratio: 0.2, weight: 1.0, accepted: true,
            },
        ];
        normalize_weights(&mut metrics);
        assert!((metrics[1].weight - 1.0).abs() < 1e-10);
        assert!((metrics[0].weight - 0.5).abs() < 1e-10);
    }
}
