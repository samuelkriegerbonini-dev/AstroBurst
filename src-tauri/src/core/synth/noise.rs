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
    #[serde(default)]
    pub seed: Option<u64>,
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
            seed: None,
        }
    }
}

impl NoiseParams {
    pub fn background_sigma_adu(&self) -> f64 {
        let sky_e = self.sky_background * self.gain;
        let dark_e = self.dark_current * self.exposure_time;
        (sky_e + dark_e + self.readout_noise * self.readout_noise).sqrt() / self.gain
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

pub fn apply_noise(
    image: &Array2<f32>,
    params: &NoiseParams,
    seed: u64,
    flat: Option<&Array2<f32>>,
) -> Array2<f32> {
    let mut rng = StdRng::seed_from_u64(seed);
    let normal = BoxMullerNormal(0.0, params.readout_noise);
    let (h, w) = image.dim();
    let mut out = Array2::<f32>::zeros((h, w));

    let sky_e = params.sky_background * params.gain;
    let dark_e = params.dark_current * params.exposure_time;
    for y in 0..h {
        for x in 0..w {
            let star_e = image[[y, x]] as f64;
            let response = flat.map_or(1.0, |f| f[[y, x]] as f64);
            let signal_e = (star_e + sky_e) * response + dark_e;
            let photon_e = poisson_sample(&mut rng, signal_e.max(0.0));
            let read_e = normal.sample(&mut rng);
            out[[y, x]] = ((photon_e as f64 + read_e) / params.gain + params.bias_level).max(0.0) as f32;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> NoiseParams {
        NoiseParams {
            gain: 2.0,
            readout_noise: 0.0,
            sky_background: 0.0,
            dark_current: 0.0,
            exposure_time: 300.0,
            bias_level: 100.0,
            seed: None,
        }
    }

    fn box_mean(img: &Array2<f32>, y0: usize, x0: usize, size: usize) -> f64 {
        let mut sum = 0.0;
        for y in y0..y0 + size {
            for x in x0..x0 + size {
                sum += img[[y, x]] as f64;
            }
        }
        sum / (size * size) as f64
    }

    #[test]
    fn star_flux_is_total_electrons_and_bias_is_added_in_adu() {
        let mut image = Array2::<f32>::zeros((16, 16));
        image[[8, 8]] = 10_000.0;
        let out = apply_noise(&image, &params(), 9, None);
        let star_adu = out[[8, 8]] as f64 - 100.0;
        assert!((star_adu - 5000.0).abs() < 250.0, "a 10000 e- star gave {star_adu} ADU at gain 2");
        assert!(out.iter().enumerate().filter(|(i, _)| *i != 8 * 16 + 8).all(|(_, v)| *v == 100.0));
    }

    #[test]
    fn sky_background_is_a_level_in_adu() {
        let sky = NoiseParams { sky_background: 200.0, ..params() };
        let out = apply_noise(&Array2::<f32>::zeros((64, 64)), &sky, 9, None);
        let mean = out.iter().map(|v| *v as f64).sum::<f64>() / out.len() as f64;
        assert!((mean - 300.0).abs() < 1.0, "sky 200 ADU plus bias 100 ADU gave a mean of {mean}");
    }

    #[test]
    fn the_flat_darkens_the_sky_in_the_corners_but_not_dark_or_bias() {
        let size = 128u32;
        let flat = generate_flat_field(size, size, 5, 0.5);
        let sky = NoiseParams { sky_background: 20_000.0, ..params() };
        let out = apply_noise(&Array2::<f32>::zeros((128, 128)), &sky, 9, Some(&flat));
        let centre = box_mean(&out, 60, 60, 8) - 100.0;
        let corner = box_mean(&out, 0, 0, 8) - 100.0;
        let r = ((60.0f64 - 64.0).powi(2) * 2.0).sqrt() / (64.0f64 * 64.0 * 2.0).sqrt();
        let rc = ((4.0f64 - 64.0).powi(2) * 2.0).sqrt() / (64.0f64 * 64.0 * 2.0).sqrt();
        let expected_ratio = (1.0 - 0.5 * rc * rc) / (1.0 - 0.5 * r * r);
        let ratio = corner / centre;
        assert!((ratio - expected_ratio).abs() < 0.02, "corner/centre sky ratio {ratio}, expected {expected_ratio}");

        let dark = NoiseParams { dark_current: 100.0, ..params() };
        let out = apply_noise(&Array2::<f32>::zeros((128, 128)), &dark, 9, Some(&flat));
        let centre = box_mean(&out, 60, 60, 8) - 100.0;
        let corner = box_mean(&out, 0, 0, 8) - 100.0;
        assert!((corner / centre - 1.0).abs() < 0.02, "dark current was flat-fielded: corner {corner} vs centre {centre}");
        assert!((centre - 15_000.0).abs() < 100.0, "dark 100 e-/s over 300 s at gain 2 gave {centre} ADU");
        let bias_only = apply_noise(&Array2::<f32>::zeros((128, 128)), &params(), 9, Some(&flat));
        assert!(bias_only.iter().all(|v| *v == 100.0), "bias was flat-fielded");
    }

    #[test]
    fn the_background_sigma_follows_the_noise_model() {
        let p = NoiseParams::default();
        let expected = (200.0f64 * 1.5 + 0.05 * 300.0 + 64.0).sqrt() / 1.5;
        assert!((p.background_sigma_adu() - expected).abs() < 1e-9);
        assert!((expected - 12.98).abs() < 0.01);
    }
}
