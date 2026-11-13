use ndarray::Array2;
use rayon::prelude::*;

use crate::core::analysis::star_detection::{detect_star_extents, StarExtent};

#[derive(Debug, Clone)]
pub struct StarMaskConfig {
    pub growth_factor: f64,
    pub softness: f64,
    pub detection_sigma: f64,
    pub min_fwhm: f64,
    pub max_fwhm: f64,
    pub max_eccentricity: f64,
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
            max_eccentricity: 0.85,
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
    let stars = detect_star_extents(image, config.detection_sigma);
    generate_star_mask_from_detection(image, &stars, config)
}

pub fn generate_star_mask_from_detection(
    image: &Array2<f32>,
    stars: &[StarExtent],
    config: &StarMaskConfig,
) -> Result<StarMaskResult, String> {
    let (h, w) = image.dim();

    let valid_stars: Vec<&StarExtent> = stars
        .iter()
        .filter(|s| {
            s.whole_component_fwhm >= config.min_fwhm
                && s.whole_component_fwhm <= config.max_fwhm
                && s.star.eccentricity <= config.max_eccentricity
        })
        .collect();

    let star_count = valid_stars.len();

    let star_layers: Vec<Vec<(usize, usize, f32)>> = valid_stars
        .par_iter()
        .map(|extent| {
            let star = &extent.star;
            let radius = extent.whole_component_fwhm * config.growth_factor;
            let soft_radius = radius + config.softness;

            let y_min = (star.y - soft_radius).floor().max(0.0) as usize;
            let y_max = ((star.y + soft_radius).ceil() as usize).min(h.saturating_sub(1));
            let x_min = (star.x - soft_radius).floor().max(0.0) as usize;
            let x_max = ((star.x + soft_radius).ceil() as usize).min(w.saturating_sub(1));

            let r2_inner = radius * radius;
            let r2_outer = soft_radius * soft_radius;
            let fade_range = (r2_outer - r2_inner).max(1e-10);

            let mut pixels = Vec::new();
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

                    pixels.push((py, px, val));
                }
            }
            pixels
        })
        .collect();

    let mut mask = Array2::<f32>::zeros((h, w));
    for layer in &star_layers {
        for &(py, px, val) in layer {
            let current = mask[[py, px]];
            if val > current {
                mask[[py, px]] = val;
            }
        }
    }

    if config.luminance_protect {
        let ceiling = config.luminance_ceiling as f32;
        let inv_range = if ceiling < 1.0 { 1.0 / (1.0 - ceiling) } else { 1.0 };
        let img_slice = image.as_slice().unwrap();
        let mask_slice = mask.as_slice_mut().unwrap();

        mask_slice
            .par_iter_mut()
            .zip(img_slice.par_iter())
            .for_each(|(m, &pixel)| {
                if pixel > ceiling && *m < 1.0 {
                    let excess = ((pixel - ceiling) * inv_range).clamp(0.0, 1.0);
                    let smooth = excess * excess * (3.0 - 2.0 * excess);
                    if smooth > *m {
                        *m = smooth;
                    }
                }
            });
    }

    let total = (h * w) as f64;
    let coverage = mask.as_slice().unwrap().par_iter().filter(|&&v| v > 0.01).count() as f64 / total;

    Ok(StarMaskResult {
        mask,
        stars_masked: star_count,
        coverage_fraction: coverage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::analysis::star_detection::DetectedStar;

    fn synthetic_star(x: f64, y: f64, fwhm: f64, ecc: f64) -> StarExtent {
        StarExtent {
            star: DetectedStar {
                x,
                y,
                flux: 100.0,
                fwhm,
                eccentricity: ecc,
                peak: 10.0,
                npix: 9,
                snr: 20.0,
            },
            whole_component_fwhm: fwhm,
        }
    }

    #[test]
    fn elongated_detections_excluded_from_mask() {
        let img = Array2::<f32>::zeros((64, 64));
        let stars = [
            synthetic_star(16.0, 16.0, 3.0, 0.1),
            synthetic_star(48.0, 48.0, 3.0, 0.97),
        ];
        let config = StarMaskConfig::default();
        let res = generate_star_mask_from_detection(&img, &stars, &config).unwrap();
        assert_eq!(res.stars_masked, 1);
        assert!(res.mask[[16, 16]] > 0.5);
        assert!(res.mask[[48, 48]] < 0.01);
    }

    #[test]
    fn fwhm_limits_and_radius_follow_the_whole_component_width_not_the_reported_fwhm() {
        let img = Array2::<f32>::zeros((128, 128));
        let bright = StarExtent { whole_component_fwhm: 6.7, ..synthetic_star(32.0, 32.0, 4.2, 0.1) };
        let clipped = StarExtent { whole_component_fwhm: 8.3, ..synthetic_star(96.0, 32.0, 12.3, 0.1) };
        let faint = StarExtent { whole_component_fwhm: 1.4, ..synthetic_star(64.0, 96.0, 1.6, 0.1) };
        let config = StarMaskConfig { max_fwhm: 10.0, ..StarMaskConfig::default() };
        let res = generate_star_mask_from_detection(&img, &[bright, clipped, faint], &config).unwrap();
        assert_eq!(res.mask[[32, 48]], 1.0, "16 px from a star of whole-component FWHM 6.7 px is inside 2.5 x 6.7 px");
        assert_eq!(res.mask[[32, 54]], 0.0, "22 px from that star is outside its mask and its 4 px soft edge");
        assert_eq!(res.mask[[32, 116]], 1.0, "a star of whole-component FWHM 8.3 px passes the 10 px limit whatever its reported FWHM");
        assert_eq!(res.mask[[96, 64]], 0.0, "a star of whole-component FWHM 1.4 px is below the 1.5 px limit whatever its reported FWHM");
        assert_eq!(res.stars_masked, 2);
    }

    fn uniform_stream(seed: u64) -> impl FnMut() -> f64 {
        let mut state = seed;
        move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        }
    }

    fn moffat_field_with_bright_and_faint_stars() -> Array2<f32> {
        let mut noise = uniform_stream(5);
        let mut field = Array2::from_shape_fn((500, 500), |_| {
            let gaussian = (-2.0 * noise().ln()).sqrt() * (2.0 * std::f64::consts::PI * noise()).cos();
            (10.0 * gaussian) as f32 + 1000.0
        });
        let mut draw = uniform_stream(99);
        for k in 0..60 {
            let (x0, y0) = (20.0 + 460.0 * draw(), 20.0 + 460.0 * draw());
            let amp = 50.0 * 10f64.powf(0.3 + 3.0 * draw());
            let fwhm = 1.2 + 5.0 * draw();
            let beta = if k % 3 == 0 { 2.5 } else { 4.765 };
            let alpha2 = (fwhm / 2.0f64).powi(2) / (2f64.powf(1.0 / beta) - 1.0);
            for ((y, x), value) in field.indexed_iter_mut() {
                let d2 = (x as f64 - x0).powi(2) + (y as f64 - y0).powi(2);
                if d2 < 80.0 * 80.0 {
                    *value += (amp * (1.0 + d2 / alpha2).powf(-beta)) as f32;
                }
            }
        }
        field
    }

    fn bits_checksum(mask: &Array2<f32>) -> u64 {
        mask.iter().fold(0xcbf29ce484222325u64, |acc, v| (acc ^ v.to_bits() as u64).wrapping_mul(0x100000001b3))
    }

    #[test]
    fn star_masks_are_the_masks_made_at_c3f19b3_before_the_fwhm_correction() {
        let field = moffat_field_with_bright_and_faint_stars();
        let clipped = field.mapv(|v| v.min(9000.0));
        let default = StarMaskConfig::default();
        let narrow_limits = StarMaskConfig { min_fwhm: 3.0, max_fwhm: 8.0, ..default.clone() };
        let cases = [
            ("default configuration", &field, &default, (56, 28854, 45863, 0x65197446e685a906u64)),
            ("FWHM limits of 3 to 8 px", &field, &narrow_limits, (43, 23683, 37691, 0x4eb36983b83b2560)),
            ("stars clipped at 9000", &clipped, &default, (57, 32631, 50223, 0x4f429dda78ccee00)),
        ];
        for (name, image, config, at_c3f19b3) in cases {
            let result = generate_star_mask(image, config).unwrap();
            let fully_masked = result.mask.iter().filter(|&&v| v >= 1.0).count();
            let covered = (result.coverage_fraction * image.len() as f64).round() as usize;
            assert_eq!(
                (result.stars_masked, fully_masked, covered, bits_checksum(&result.mask)),
                at_c3f19b3,
                "{name}: stars masked, fully masked pixels, covered pixels and checksum of the mask bits"
            );
        }
    }
}
