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

fn bessel_j1(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 8.0 {
        let y = x * x;
        let num = x
            * (72362614232.0
                + y * (-7895059235.0
                    + y * (242396853.1
                        + y * (-2972611.439 + y * (15704.4826 + y * (-30.16036606))))));
        let den = 144725228442.0
            + y * (2300535178.0
                + y * (18583304.74 + y * (99447.43394 + y * (376.9991397 + y))));
        num / den
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 2.356194491;
        let p = 1.0
            + y * (0.183105e-2
                + y * (-0.3516396496e-4 + y * (0.2457520174e-5 + y * (-0.240337019e-6))));
        let q = 0.04687499995
            + y * (-0.2002690873e-3
                + y * (0.8449199096e-5 + y * (-0.88228987e-6 + y * 0.105787412e-6)));
        let ans = (0.5641895835 / ax.sqrt()) * (xx.cos() * p - z * xx.sin() * q);
        if x < 0.0 {
            -ans
        } else {
            ans
        }
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
        let x0 = ((sx - psf_r as f64).floor() as i64).max(0) as usize;
        let x1 = ((sx + psf_r as f64).ceil() as i64).min(w as i64 - 1) as usize;
        let y0 = ((sy - psf_r as f64).floor() as i64).max(0) as usize;
        let y1 = ((sy + psf_r as f64).ceil() as i64).min(h as i64 - 1) as usize;

        let mut psf_sum = 0.0;
        for py in y0..=y1 {
            for px in x0..=x1 {
                psf_sum += psf.evaluate(px as f64 - sx, py as f64 - sy);
            }
        }
        if psf_sum < 1e-20 {
            continue;
        }
        let norm = flux / psf_sum;
        for py in y0..=y1 {
            for px in x0..=x1 {
                image[[py, px]] += (psf.evaluate(px as f64 - sx, py as f64 - sy) * norm) as f32;
            }
        }
    }
    image
}
