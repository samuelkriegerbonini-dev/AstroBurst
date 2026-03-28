use serde::{Deserialize, Serialize};

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
            flux_min: 100.0,
            flux_max: 50000.0,
            seed: 42,
        }
    }
}

fn rng_from_seed(seed: u64) -> impl Rng {
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    StdRng::seed_from_u64(seed)
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

pub fn king_cluster(cfg: &FieldConfig, core_radius: f64, tidal_radius: f64) -> Vec<Star> {
    let mut rng = rng_from_seed(cfg.seed);
    let cx = cfg.width as f64 * 0.5;
    let cy = cfg.height as f64 * 0.5;
    let c = tidal_radius / core_radius;
    let king_norm = 1.0 / (1.0 + c * c).sqrt();
    let mut stars = Vec::with_capacity(cfg.n_stars);
    while stars.len() < cfg.n_stars {
        let r = rng.gen::<f64>() * tidal_radius;
        let profile = (1.0 / (1.0 + (r / core_radius).powi(2)).sqrt() - king_norm)
            .max(0.0)
            .powi(2);
        if rng.gen::<f64>() < profile {
            let theta = rng.gen::<f64>() * 2.0 * PI;
            let flux = power_law_flux(&mut rng, cfg.flux_min, cfg.flux_max);
            stars.push(Star {
                x: cx + r * theta.cos(),
                y: cy + r * theta.sin(),
                z: 0.0,
                flux,
                temperature: 3000.0 + rng.gen::<f64>() * 27000.0,
            });
        }
    }
    stars
}

pub fn exponential_disk(
    cfg: &FieldConfig,
    scale_length: f64,
    inclination_deg: f64,
) -> Vec<Star> {
    let mut rng = rng_from_seed(cfg.seed);
    let cx = cfg.width as f64 * 0.5;
    let cy = cfg.height as f64 * 0.5;
    let cos_i = (inclination_deg * PI / 180.0).cos();
    (0..cfg.n_stars)
        .map(|_| {
            let u: f64 = rng.gen::<f64>().min(1.0 - 1e-10);
            let r = -scale_length * (1.0 - u).ln();
            let theta = rng.gen::<f64>() * 2.0 * PI;
            let flux = power_law_flux(&mut rng, cfg.flux_min, cfg.flux_max);
            Star {
                x: cx + r * theta.cos(),
                y: cy + r * theta.sin() * cos_i,
                z: rng.gen::<f64>() * scale_length * 0.1,
                flux,
                temperature: 3000.0 + rng.gen::<f64>() * 27000.0,
            }
        })
        .collect()
}

pub fn from_particles(
    positions: &[(f64, f64, f64)],
    masses: &[f64],
    image_width: u32,
    image_height: u32,
    fov_scale: f64,
) -> Vec<Star> {
    let cx = image_width as f64 * 0.5;
    let cy = image_height as f64 * 0.5;
    positions
        .iter()
        .zip(masses.iter())
        .map(|(&(px, py, pz), &mass)| {
            let luminosity = mass.abs().powf(3.5);
            Star {
                x: cx + px * fov_scale,
                y: cy + py * fov_scale,
                z: pz,
                flux: (luminosity * fov_scale).max(1.0),
                temperature: (5778.0 * mass.abs().powf(0.505)).clamp(2000.0, 50000.0),
            }
        })
        .collect()
}
