use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use super::seed::{derive_seed, SeedPurpose};

const MAX_PLACEMENT_ATTEMPTS_PER_STAR: u64 = 1_000_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Star {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub flux: f64,
    pub temperature: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldConfig {
    pub width: u32,
    pub height: u32,
    pub n_stars: usize,
    pub flux_min: f64,
    pub flux_max: f64,
    pub seed: u64,
}

impl Default for FieldConfig {
    fn default() -> Self {
        Self {
            width: 2048,
            height: 2048,
            n_stars: 500,
            flux_min: 2000.0,
            flux_max: 500_000.0,
            seed: 42,
        }
    }
}

fn rng_from_seed(seed: u64) -> impl Rng {
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    StdRng::seed_from_u64(derive_seed(seed, SeedPurpose::Field, 0))
}

use rand::Rng;
use std::f64::consts::PI;

fn power_law_flux(rng: &mut impl Rng, flux_min: f64, flux_max: f64) -> f64 {
    let alpha = 2.5;
    let f_min_inv = flux_min.powf(1.0 - alpha);
    let f_max_inv = flux_max.powf(1.0 - alpha);
    let u: f64 = rng.gen();
    (f_min_inv + u * (f_max_inv - f_min_inv)).powf(1.0 / (1.0 - alpha))
}

pub fn in_frame(x: f64, y: f64, width: u32, height: u32) -> bool {
    x >= 0.0 && x < width as f64 && y >= 0.0 && y < height as f64
}

pub fn uniform_field(cfg: &FieldConfig) -> Vec<Star> {
    let mut rng = rng_from_seed(cfg.seed);
    (0..cfg.n_stars)
        .map(|_| {
            let flux = power_law_flux(&mut rng, cfg.flux_min, cfg.flux_max);
            Star {
                x: rng.gen::<f64>() * cfg.width as f64,
                y: rng.gen::<f64>() * cfg.height as f64,
                z: 0.0,
                flux,
                temperature: 3000.0 + rng.gen::<f64>() * 27000.0,
            }
        })
        .collect()
}

pub fn king_cluster(cfg: &FieldConfig, core_radius: f64, tidal_radius: f64) -> Result<Vec<Star>> {
    let valid = |v: f64| v.is_finite() && v > 0.0;
    if !valid(core_radius) || !valid(tidal_radius) {
        bail!("King cluster radii must be positive: core {core_radius}, tidal {tidal_radius}");
    }
    let mut rng = rng_from_seed(cfg.seed);
    let cx = cfg.width as f64 * 0.5;
    let cy = cfg.height as f64 * 0.5;
    let c = tidal_radius / core_radius;
    let king_norm = 1.0 / (1.0 + c * c).sqrt();
    let mut stars = Vec::with_capacity(cfg.n_stars);
    let mut misses = 0u64;
    let mut off_frame = 0u64;
    while stars.len() < cfg.n_stars {
        if misses >= MAX_PLACEMENT_ATTEMPTS_PER_STAR {
            bail!(
                "King cluster with core radius {core_radius} and tidal radius {tidal_radius} accepts almost no stars inside the {}x{} frame; placed {} of {} ({off_frame} candidates fell outside the frame). Use a tidal radius closer to or above the core radius and within the frame.",
                cfg.width,
                cfg.height,
                stars.len(),
                cfg.n_stars
            );
        }
        let r = rng.gen::<f64>() * tidal_radius;
        let profile = (1.0 / (1.0 + (r / core_radius).powi(2)).sqrt() - king_norm)
            .max(0.0)
            .powi(2);
        let accept = profile * (r / tidal_radius);
        misses += 1;
        if rng.gen::<f64>() >= accept {
            continue;
        }
        let theta = rng.gen::<f64>() * 2.0 * PI;
        let (x, y) = (cx + r * theta.cos(), cy + r * theta.sin());
        if !in_frame(x, y, cfg.width, cfg.height) {
            off_frame += 1;
            continue;
        }
        misses = 0;
        let flux = power_law_flux(&mut rng, cfg.flux_min, cfg.flux_max);
        stars.push(Star {
            x,
            y,
            z: 0.0,
            flux,
            temperature: 3000.0 + rng.gen::<f64>() * 27000.0,
        });
    }
    Ok(stars)
}

pub fn exponential_disk(
    cfg: &FieldConfig,
    scale_length: f64,
    inclination_deg: f64,
) -> Result<Vec<Star>> {
    let mut rng = rng_from_seed(cfg.seed);
    let cx = cfg.width as f64 * 0.5;
    let cy = cfg.height as f64 * 0.5;
    let cos_i = (inclination_deg * PI / 180.0).cos();
    let mut stars = Vec::with_capacity(cfg.n_stars);
    let mut misses = 0u64;
    while stars.len() < cfg.n_stars {
        if misses >= MAX_PLACEMENT_ATTEMPTS_PER_STAR {
            bail!(
                "Exponential disk with scale length {scale_length} places almost no stars inside the {}x{} frame; placed {} of {}. Use a shorter scale length or a larger frame.",
                cfg.width,
                cfg.height,
                stars.len(),
                cfg.n_stars
            );
        }
        let u1: f64 = rng.gen::<f64>().max(1e-12);
        let u2: f64 = rng.gen::<f64>().max(1e-12);
        let r = -scale_length * (u1 * u2).ln();
        let theta = rng.gen::<f64>() * 2.0 * PI;
        let (x, y) = (cx + r * theta.cos(), cy + r * theta.sin() * cos_i);
        if !in_frame(x, y, cfg.width, cfg.height) {
            misses += 1;
            continue;
        }
        misses = 0;
        let flux = power_law_flux(&mut rng, cfg.flux_min, cfg.flux_max);
        stars.push(Star {
            x,
            y,
            z: rng.gen::<f64>() * scale_length * 0.1,
            flux,
            temperature: 3000.0 + rng.gen::<f64>() * 27000.0,
        });
    }
    Ok(stars)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_field() -> FieldConfig {
        FieldConfig { width: 512, height: 512, n_stars: 500, ..FieldConfig::default() }
    }

    fn assert_all_in_frame(stars: &[Star], cfg: &FieldConfig, label: &str) {
        assert_eq!(stars.len(), cfg.n_stars, "{label}: n_stars not honoured");
        let outside: Vec<&Star> = stars.iter().filter(|s| !in_frame(s.x, s.y, cfg.width, cfg.height)).collect();
        assert!(outside.is_empty(), "{label}: {} of {} stars lie outside the {}x{} frame, first {:?}", outside.len(), stars.len(), cfg.width, cfg.height, outside.first());
    }

    #[test]
    fn every_catalog_star_lies_inside_a_512_frame_with_the_2048_default_radii() {
        let cfg = small_field();
        assert_all_in_frame(&uniform_field(&cfg), &cfg, "uniform");
        assert_all_in_frame(&king_cluster(&cfg, 50.0, 400.0).unwrap(), &cfg, "king");
        assert_all_in_frame(&exponential_disk(&cfg, 200.0, 30.0).unwrap(), &cfg, "disk");
        let few = FieldConfig { n_stars: 7, ..cfg };
        assert_all_in_frame(&exponential_disk(&few, 200.0, 30.0).unwrap(), &few, "disk 7");
        assert_all_in_frame(&king_cluster(&few, 50.0, 400.0).unwrap(), &few, "king 7");
    }

    #[test]
    fn a_disk_that_cannot_fit_the_frame_is_an_error_instead_of_a_hang() {
        let cfg = FieldConfig { width: 4, height: 4, n_stars: 3, ..FieldConfig::default() };
        let err = exponential_disk(&cfg, 1e9, 0.0).expect_err("a 1e9 px scale length on a 4 px frame must give up");
        assert!(err.to_string().contains("inside the 4x4 frame"), "{err}");
    }

    #[test]
    fn the_default_flux_range_is_in_total_electrons_per_frame() {
        let cfg = FieldConfig::default();
        assert_eq!((cfg.flux_min, cfg.flux_max), (2000.0, 500_000.0));
        let stars = uniform_field(&cfg);
        assert!(stars.iter().all(|s| s.flux >= cfg.flux_min && s.flux <= cfg.flux_max));
    }

    #[test]
    fn the_field_stream_is_derived_from_the_seed_not_the_raw_seed() {
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        let cfg = small_field();
        let a = uniform_field(&cfg);
        let first_x = |mut rng: StdRng| {
            rng.gen::<f64>();
            rng.gen::<f64>() * cfg.width as f64
        };
        assert_eq!(a[0].x, first_x(StdRng::seed_from_u64(derive_seed(cfg.seed, SeedPurpose::Field, 0))));
        assert_ne!(a[0].x, first_x(StdRng::seed_from_u64(cfg.seed)));
        let b = uniform_field(&FieldConfig { seed: 43, ..small_field() });
        assert!(a.iter().zip(&b).all(|(s, t)| s.x != t.x || s.y != t.y));
        assert_eq!(uniform_field(&small_field())[0].x, a[0].x);
    }
}
