use ndarray::Array2;
use rand::Rng;
use rand::rngs::StdRng;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoiseParams {
    pub gain: f64,
    pub readout_noise: f64,
    pub sky_background: f64,
    pub dark_current: f64,
    pub exposure_time: f64,
    pub bias_level: f64,
    pub seed: u64,
}

impl Default for NoiseParams {
    fn default() -> Self {
        Self {
            gain: 1.5,
            readout_noise: 8.0,
            sky_background: 200.0,
            dark_current: 0.05,
            exposure_time: 300.0,
            bias_level: 1000.0,
            seed: 123,
        }
    }
}

struct BoxMullerNormal(f64, f64);

impl BoxMullerNormal {
    fn sample(&self, rng: &mut impl Rng) -> f64 {
        let u1: f64 = rng.gen::<f64>().max(1e-30);
        let u2: f64 = rng.gen::<f64>();
        self.0 + self.1 * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

fn poisson_sample(rng: &mut impl Rng, lambda: f64) -> u64 {
    if lambda <= 0.0 {
        return 0;
    }
    if lambda < 30.0 {
        let l = (-lambda).exp();
        let (mut k, mut p) = (0u64, 1.0);
        loop {
            k += 1;
            p *= rng.gen::<f64>();
            if p <= l {
                return k - 1;
            }
        }
    } else {
        let sample = lambda + lambda.sqrt() * BoxMullerNormal(0.0, 1.0).sample(rng);
        sample.round().max(0.0) as u64
    }
}

pub fn apply_noise(image: &Array2<f32>, params: &NoiseParams) -> Array2<f32> {
    let mut rng = StdRng::seed_from_u64(params.seed);
    let normal = BoxMullerNormal(0.0, params.readout_noise);
    let (h, w) = image.dim();
    let mut out = Array2::<f32>::zeros((h, w));

    for y in 0..h {
        for x in 0..w {
            let flux = image[[y, x]] as f64;
            let signal_e = (flux + params.sky_background) * params.gain * params.exposure_time
                + params.dark_current * params.exposure_time;
            let photon_e = poisson_sample(&mut rng, signal_e.max(0.0));
            let read_e = normal.sample(&mut rng);
            out[[y, x]] = ((photon_e as f64 + read_e + params.bias_level) / params.gain).max(0.0) as f32;
        }
    }
    out
}

pub fn generate_flat_field(width: u32, height: u32, seed: u64, vignette_strength: f64) -> Array2<f32> {
    let mut rng = StdRng::seed_from_u64(seed);
    let (cx, cy) = (width as f64 * 0.5, height as f64 * 0.5);
    let max_r = (cx * cx + cy * cy).sqrt();
    let h = height as usize;
    let w = width as usize;
    let mut flat = Array2::<f32>::zeros((h, w));

    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f64 - cx, y as f64 - cy);
            let r = (dx * dx + dy * dy).sqrt() / max_r;
            flat[[y, x]] =
                ((1.0 - vignette_strength * r * r) * (1.0 + rng.gen::<f64>() * 0.02 - 0.01))
                    .max(0.01) as f32;
        }
    }
    flat
}

pub fn apply_flat_field(image: &mut Array2<f32>, flat: &Array2<f32>) {
    let (h, w) = image.dim();
    for y in 0..h {
        for x in 0..w {
            let f = flat[[y, x]];
            if f > 1e-6 {
                image[[y, x]] /= f;
            }
        }
    }
}
