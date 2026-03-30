use ndarray::Array2;

use crate::core::analysis::star_detection::{detect_stars, DetectedStar, DetectionResult};

#[derive(Debug, Clone)]
pub struct StarMaskConfig {
    pub growth_factor: f64,
    pub softness: f64,
    pub detection_sigma: f64,
    pub min_fwhm: f64,
    pub max_fwhm: f64,
    pub luminance_protect: bool,
    pub luminance_ceiling: f64,
}

impl Default for StarMaskConfig {
    fn default() -> Self {
        Self {
            growth_factor: 2.5,
            softness: 4.0,
            detection_sigma: 5.0,
            min_fwhm: 1.5,
            max_fwhm: 30.0,
            luminance_protect: false,
            luminance_ceiling: 0.85,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StarMaskResult {
    pub mask: Array2<f32>,
    pub stars_masked: usize,
    pub coverage_fraction: f64,
}

pub fn generate_star_mask(
    image: &Array2<f32>,
    config: &StarMaskConfig,
) -> Result<StarMaskResult, String> {
    let detection = detect_stars(image, config.detection_sigma);

    generate_star_mask_from_detection(image, &detection, config)
}

pub fn generate_star_mask_from_detection(
    image: &Array2<f32>,
    detection: &DetectionResult,
    config: &StarMaskConfig,
) -> Result<StarMaskResult, String> {
    let (h, w) = image.dim();
    let mut mask = Array2::<f32>::zeros((h, w));

    let valid_stars: Vec<&DetectedStar> = detection
        .stars
        .iter()
        .filter(|s| s.fwhm >= config.min_fwhm && s.fwhm <= config.max_fwhm)
        .collect();

    let star_count = valid_stars.len();

    for star in &valid_stars {
        let radius = star.fwhm * config.growth_factor;
        let soft_radius = radius + config.softness;

        let y_min = (star.y - soft_radius).floor().max(0.0) as usize;
        let y_max = ((star.y + soft_radius).ceil() as usize).min(h.saturating_sub(1));
        let x_min = (star.x - soft_radius).floor().max(0.0) as usize;
        let x_max = ((star.x + soft_radius).ceil() as usize).min(w.saturating_sub(1));

        let r2_inner = radius * radius;
        let r2_outer = soft_radius * soft_radius;
        let fade_range = (r2_outer - r2_inner).max(1e-10);

        for py in y_min..=y_max {
            for px in x_min..=x_max {
                let dx = px as f64 - star.x;
                let dy = py as f64 - star.y;
                let d2 = dx * dx + dy * dy;

                let val = if d2 <= r2_inner {
                    1.0f32
                } else if d2 <= r2_outer {
                    let t = ((d2 - r2_inner) / fade_range) as f32;
                    let smooth = t * t * (3.0 - 2.0 * t);
                    1.0 - smooth
                } else {
                    continue;
                };

                let current = mask[[py, px]];
                if val > current {
                    mask[[py, px]] = val;
                }
            }
        }
    }

    if config.luminance_protect {
        let ceiling = config.luminance_ceiling as f32;
        let inv_range = if ceiling < 1.0 { 1.0 / (1.0 - ceiling) } else { 1.0 };
        let (h, w) = image.dim();
        for py in 0..h {
            for px in 0..w {
                let pixel = image[[py, px]];
                if pixel > ceiling && mask[[py, px]] < 1.0 {
                    let excess = ((pixel - ceiling) * inv_range).clamp(0.0, 1.0);
                    let smooth = excess * excess * (3.0 - 2.0 * excess);
                    mask[[py, px]] = mask[[py, px]].max(smooth);
                }
            }
        }
    }

    let total = (h * w) as f64;
    let coverage = mask.iter().filter(|&&v| v > 0.01).count() as f64 / total;

    Ok(StarMaskResult {
        mask,
        stars_masked: star_count,
        coverage_fraction: coverage,
    })
}

pub fn invert_mask(mask: &Array2<f32>) -> Array2<f32> {
    let mut inv = mask.clone();
    inv.par_mapv_inplace(|v| 1.0 - v);
    inv
}
