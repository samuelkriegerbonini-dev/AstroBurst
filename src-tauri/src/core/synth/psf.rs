use libm::j1 as bessel_j1;
use ndarray::Array2;
use std::f64::consts::PI;

use super::star_field::Star;

pub trait PsfModel: Send + Sync {
    fn evaluate(&self, dx: f64, dy: f64) -> f64;
    fn radius(&self) -> f64;
}

pub struct GaussianPsf {
    pub sigma: f64,
    inv_2sigma_sq: f64,
}

impl GaussianPsf {
    pub fn from_fwhm(fwhm: f64) -> Self {
        let sigma = fwhm / 2.3548;
        Self {
            sigma,
            inv_2sigma_sq: 1.0 / (2.0 * sigma * sigma),
        }
    }
}

impl PsfModel for GaussianPsf {
    fn evaluate(&self, dx: f64, dy: f64) -> f64 {
        (-(dx * dx + dy * dy) * self.inv_2sigma_sq).exp()
    }
    fn radius(&self) -> f64 {
        self.sigma * 4.0
    }
}

pub struct MoffatPsf {
    pub alpha: f64,
    pub beta: f64,
    inv_alpha_sq: f64,
}

impl MoffatPsf {
    pub fn from_fwhm(fwhm: f64, beta: f64) -> Self {
        let alpha = fwhm / (2.0 * (2.0_f64.powf(1.0 / beta) - 1.0).sqrt());
        Self {
            alpha,
            beta,
            inv_alpha_sq: 1.0 / (alpha * alpha),
        }
    }
}

impl PsfModel for MoffatPsf {
    fn evaluate(&self, dx: f64, dy: f64) -> f64 {
        (1.0 + (dx * dx + dy * dy) * self.inv_alpha_sq).powf(-self.beta)
    }
    fn radius(&self) -> f64 {
        self.alpha * 5.0
    }
}

pub struct AiryPsf {
    pub lambda_over_d: f64,
    scale: f64,
}

impl AiryPsf {
    pub fn new(lambda_over_d_pixels: f64) -> Self {
        Self {
            lambda_over_d: lambda_over_d_pixels,
            scale: PI / lambda_over_d_pixels,
        }
    }
}

impl PsfModel for AiryPsf {
    fn evaluate(&self, dx: f64, dy: f64) -> f64 {
        let r = (dx * dx + dy * dy).sqrt();
        if r < 1e-10 {
            return 1.0;
        }
        let x = r * self.scale;
        let j1 = bessel_j1(x);
        let v = 2.0 * j1 / x;
        v * v
    }
    fn radius(&self) -> f64 {
        self.lambda_over_d * 4.0
    }
}

pub fn render_stars(
    stars: &[Star],
    psf: &dyn PsfModel,
    width: u32,
    height: u32,
) -> Array2<f32> {
    let w = width as usize;
    let h = height as usize;
    let mut image = Array2::<f32>::zeros((h, w));
    let psf_r = psf.radius().ceil() as i64;

    for star in stars {
        let (sx, sy, flux) = (star.x, star.y, star.flux);
        let fx0 = (sx - psf_r as f64).floor() as i64;
        let fx1 = (sx + psf_r as f64).ceil() as i64;
        let fy0 = (sy - psf_r as f64).floor() as i64;
        let fy1 = (sy + psf_r as f64).ceil() as i64;

        if fx1 < 0 || fy1 < 0 || fx0 > w as i64 - 1 || fy0 > h as i64 - 1 {
            continue;
        }

        let mut psf_sum = 0.0;
        for py in fy0..=fy1 {
            for px in fx0..=fx1 {
                psf_sum += psf.evaluate(px as f64 - sx, py as f64 - sy);
            }
        }
        if psf_sum < 1e-20 {
            continue;
        }
        let norm = flux / psf_sum;

        let x0 = fx0.max(0) as usize;
        let x1 = (fx1.min(w as i64 - 1)) as usize;
        let y0 = fy0.max(0) as usize;
        let y1 = (fy1.min(h as i64 - 1)) as usize;
        for py in y0..=y1 {
            for px in x0..=x1 {
                image[[py, px]] += (psf.evaluate(px as f64 - sx, py as f64 - sy) * norm) as f32;
            }
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    const J1_REFERENCE: [(f64, f64); 10] = [
        (0.5, 0.2422684576748739),
        (1.0, 0.4400505857449335),
        (2.0, 0.5767248077568734),
        (3.8317059702075125, -6.149807356994906e-17),
        (5.0, -0.32757913759146523),
        (8.0, 0.23463634685391463),
        (10.0, 0.04347274616886144),
        (20.0, 0.06683312417585005),
        (50.0, -0.09751182812517514),
        (100.0, -0.07714535201411216),
    ];

    #[test]
    fn bessel_j1_matches_double_precision_reference() {
        let failures: Vec<String> = J1_REFERENCE
            .iter()
            .filter_map(|&(x, expected)| {
                let got = bessel_j1(x);
                let delta = (got - expected).abs();
                (delta > 1e-12).then(|| {
                    format!("x={x}: got {got:e}, expected {expected:e}, |delta|={delta:e}")
                })
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn airy_render_matches_the_series_reference_within_f32_resolution() {
        let star = Star {
            x: 32.0,
            y: 32.0,
            z: 0.0,
            flux: 1000.0,
            temperature: 5800.0,
        };
        let image = render_stars(&[star], &AiryPsf::new(2.5), 64, 64);
        let samples = [
            ("peak", image[[32, 32]], 131.41490173339844),
            ("r=1", image[[32, 33]], 87.32694244384766),
            ("r=3", image[[32, 35]], 0.02325810119509697),
            ("r=6", image[[32, 38]], 0.19441719353199005),
        ];
        let failures: Vec<String> = samples
            .iter()
            .filter_map(|&(name, got, expected)| {
                let rel = (got as f64 - expected).abs() / expected;
                (rel > 1e-6).then(|| format!("{name}: got {got}, expected {expected}, rel={rel:e}"))
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        let sum: f64 = image.iter().map(|&v| v as f64).sum();
        assert!((sum - 1000.0).abs() / 1000.0 <= 1e-6, "sum {sum}");
    }
}
