use anyhow::Result;
use ndarray::Array2;
use serde::{Deserialize, Serialize};

use super::star_field::{self, FieldConfig, Star};
use super::psf::{self, AiryPsf, GaussianPsf, MoffatPsf, PsfModel};
use super::noise::{self, NoiseParams};
use crate::infra::fits::writer as fits_writer;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FieldType {
    Uniform,
    KingCluster {
        core_radius: f64,
        tidal_radius: f64,
    },
    ExponentialDisk {
        scale_length: f64,
        inclination_deg: f64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PsfType {
    Gaussian { fwhm: f64 },
    Moffat { fwhm: f64, beta: f64 },
    Airy { lambda_over_d: f64 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SynthConfig {
    pub field: FieldConfig,
    pub field_type: FieldType,
    pub psf_type: PsfType,
    pub noise: NoiseParams,
    pub apply_vignette: bool,
    pub vignette_strength: f64,
    pub n_frames: u32,
}

impl Default for SynthConfig {
    fn default() -> Self {
        Self {
            field: FieldConfig::default(),
            field_type: FieldType::Uniform,
            psf_type: PsfType::Gaussian { fwhm: 3.0 },
            noise: NoiseParams::default(),
            apply_vignette: false,
            vignette_strength: 0.3,
            n_frames: 1,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SynthResult {
    pub width: u32,
    pub height: u32,
    pub star_count: usize,
    pub output_path: Option<String>,
}

pub fn generate(config: &SynthConfig) -> (Array2<f32>, Array2<f32>, Vec<Star>) {
    let stars = gen_field(config);
    let psf_model = make_psf(&config.psf_type);
    let ground_truth =
        psf::render_stars(&stars, psf_model.as_ref(), config.field.width, config.field.height);

    let mut image = ground_truth.clone();
    if config.apply_vignette {
        let flat = noise::generate_flat_field(
            config.field.width,
            config.field.height,
            config.noise.seed + 999,
            config.vignette_strength,
        );
        noise::apply_flat_field(&mut image, &flat);
    }

    let noisy = noise::apply_noise(&image, &config.noise);
    (noisy, ground_truth, stars)
}

pub fn generate_stack(config: &SynthConfig) -> (Vec<Array2<f32>>, Array2<f32>, Vec<Star>) {
    let stars = gen_field(config);
    let psf_model = make_psf(&config.psf_type);
    let gt = psf::render_stars(&stars, psf_model.as_ref(), config.field.width, config.field.height);

    let frames: Vec<Array2<f32>> = (0..config.n_frames)
        .map(|i| {
            let mut img = gt.clone();
            if config.apply_vignette {
                let flat = noise::generate_flat_field(
                    config.field.width,
                    config.field.height,
                    config.noise.seed + 999 + i as u64,
                    config.vignette_strength,
                );
                noise::apply_flat_field(&mut img, &flat);
            }
            let mut np = config.noise.clone();
            np.seed = config.noise.seed + i as u64 * 7919;
            noise::apply_noise(&img, &np)
        })
        .collect();

    (frames, gt, stars)
}

pub fn save_fits(image: &Array2<f32>, path: &str) -> Result<()> {
    fits_writer::write_fits_mono(path, image, None)
}

pub fn save_catalog(stars: &[Star], path: &str) -> Result<()> {
    let mut out = String::from("id,x,y,z,flux,temperature\n");
    for (i, s) in stars.iter().enumerate() {
        out.push_str(&format!(
            "{},{:.4},{:.4},{:.4},{:.4},{:.1}\n",
            i, s.x, s.y, s.z, s.flux, s.temperature
        ));
    }
    std::fs::write(path, out)?;
    Ok(())
}

fn gen_field(config: &SynthConfig) -> Vec<Star> {
    match &config.field_type {
        FieldType::Uniform => star_field::uniform_field(&config.field),
        FieldType::KingCluster {
            core_radius,
            tidal_radius,
        } => star_field::king_cluster(&config.field, *core_radius, *tidal_radius),
        FieldType::ExponentialDisk {
            scale_length,
            inclination_deg,
        } => star_field::exponential_disk(&config.field, *scale_length, *inclination_deg),
    }
}

fn make_psf(psf_type: &PsfType) -> Box<dyn PsfModel> {
    match psf_type {
        PsfType::Gaussian { fwhm } => Box::new(GaussianPsf::from_fwhm(*fwhm)),
        PsfType::Moffat { fwhm, beta } => Box::new(MoffatPsf::from_fwhm(*fwhm, *beta)),
        PsfType::Airy { lambda_over_d } => Box::new(AiryPsf::new(*lambda_over_d)),
    }
}
