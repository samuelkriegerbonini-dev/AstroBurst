use anyhow::{bail, Result};
use ndarray::Array2;
use serde::{Deserialize, Serialize};

use super::star_field::{self, FieldConfig, Star};
use super::psf::{self, AiryPsf, GaussianPsf, MoffatPsf, PsfModel};
use super::noise::{self, NoiseParams};
use crate::types::header::HduHeader;

const MAX_SYNTH_DIM: u32 = 16384;
const MAX_SYNTH_STARS: usize = 1_000_000;
const MAX_SYNTH_FRAMES: u32 = 1024;
const FRAME_UNIT: &str = "ADU";

pub type SynthFrame = (Array2<f32>, Array2<f32>, Vec<Star>);
pub type SynthStack = (Vec<Array2<f32>>, Array2<f32>, Vec<Star>);

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

fn require_positive(name: &str, v: f64) -> Result<()> {
    if !(v.is_finite() && v > 0.0) {
        bail!("{name} must be a positive number, got {v}");
    }
    Ok(())
}

fn require_non_negative(name: &str, v: f64) -> Result<()> {
    if !(v.is_finite() && v >= 0.0) {
        bail!("{name} must be zero or positive, got {v}");
    }
    Ok(())
}

impl SynthConfig {
    pub fn validate(&self) -> Result<()> {
        let f = &self.field;
        if f.width == 0 || f.height == 0 || f.width > MAX_SYNTH_DIM || f.height > MAX_SYNTH_DIM {
            bail!("Field size {}x{} is outside 1..{} pixels", f.width, f.height, MAX_SYNTH_DIM);
        }
        if f.n_stars > MAX_SYNTH_STARS {
            bail!("{} stars requested; the limit is {}", f.n_stars, MAX_SYNTH_STARS);
        }
        require_positive("Minimum flux", f.flux_min)?;
        require_positive("Maximum flux", f.flux_max)?;
        if f.flux_max < f.flux_min {
            bail!("Maximum flux {} is below the minimum flux {}", f.flux_max, f.flux_min);
        }
        match &self.field_type {
            FieldType::Uniform => {}
            FieldType::KingCluster { core_radius, tidal_radius } => {
                require_positive("Core radius", *core_radius)?;
                require_positive("Tidal radius", *tidal_radius)?;
            }
            FieldType::ExponentialDisk { scale_length, inclination_deg } => {
                require_positive("Scale length", *scale_length)?;
                if !inclination_deg.is_finite() {
                    bail!("Inclination must be a finite angle, got {inclination_deg}");
                }
            }
        }
        match &self.psf_type {
            PsfType::Gaussian { fwhm } => require_positive("PSF FWHM", *fwhm)?,
            PsfType::Moffat { fwhm, beta } => {
                require_positive("PSF FWHM", *fwhm)?;
                require_positive("Moffat beta", *beta)?;
            }
            PsfType::Airy { lambda_over_d } => require_positive("Airy lambda/D", *lambda_over_d)?,
        }
        let n = &self.noise;
        require_positive("Gain", n.gain)?;
        require_non_negative("Read noise", n.readout_noise)?;
        require_non_negative("Sky background", n.sky_background)?;
        require_non_negative("Dark current", n.dark_current)?;
        require_non_negative("Exposure time", n.exposure_time)?;
        if !n.bias_level.is_finite() {
            bail!("Bias level must be finite, got {}", n.bias_level);
        }
        if self.apply_vignette && !(0.0..=1.0).contains(&self.vignette_strength) {
            bail!("Vignette strength must be within 0..1, got {}", self.vignette_strength);
        }
        Ok(())
    }
}

fn to_frame_units(electrons: &Array2<f32>, noise: &NoiseParams) -> Array2<f32> {
    let gain = noise.gain as f32;
    electrons.mapv(|e| e / gain)
}

pub fn frame_header(noise: &NoiseParams) -> HduHeader {
    let mut header = HduHeader::empty();
    header.set_f64("EXPTIME", noise.exposure_time);
    header.set_f64("GAIN", noise.gain);
    header.set("BUNIT", FRAME_UNIT.to_string());
    header
}

pub fn generate(config: &SynthConfig) -> Result<SynthFrame> {
    config.validate()?;
    let stars = gen_field(config)?;
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
        noise::apply_vignette(&mut image, &flat);
    }

    let noisy = noise::apply_noise(&image, &config.noise);
    Ok((noisy, to_frame_units(&ground_truth, &config.noise), stars))
}

pub fn generate_stack(config: &SynthConfig) -> Result<SynthStack> {
    config.validate()?;
    if config.n_frames == 0 || config.n_frames > MAX_SYNTH_FRAMES {
        bail!("Frame count {} is outside 1..{}", config.n_frames, MAX_SYNTH_FRAMES);
    }
    let stars = gen_field(config)?;
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
                noise::apply_vignette(&mut img, &flat);
            }
            let mut np = config.noise.clone();
            np.seed = config.noise.seed + i as u64 * 7919;
            noise::apply_noise(&img, &np)
        })
        .collect();

    Ok((frames, to_frame_units(&gt, &config.noise), stars))
}

pub fn catalog_csv(stars: &[Star], noise: &NoiseParams) -> String {
    let mut out = String::from("id,x,y,z,flux_e,flux_adu,temperature\n");
    for (i, s) in stars.iter().enumerate() {
        out.push_str(&format!(
            "{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.1}\n",
            i, s.x, s.y, s.z, s.flux, s.flux / noise.gain, s.temperature
        ));
    }
    out
}

pub fn save_catalog(stars: &[Star], noise: &NoiseParams, path: &str) -> Result<()> {
    std::fs::write(path, catalog_csv(stars, noise))?;
    Ok(())
}

fn gen_field(config: &SynthConfig) -> Result<Vec<Star>> {
    Ok(match &config.field_type {
        FieldType::Uniform => star_field::uniform_field(&config.field),
        FieldType::KingCluster {
            core_radius,
            tidal_radius,
        } => star_field::king_cluster(&config.field, *core_radius, *tidal_radius)?,
        FieldType::ExponentialDisk {
            scale_length,
            inclination_deg,
        } => star_field::exponential_disk(&config.field, *scale_length, *inclination_deg),
    })
}

fn make_psf(psf_type: &PsfType) -> Box<dyn PsfModel> {
    match psf_type {
        PsfType::Gaussian { fwhm } => Box::new(GaussianPsf::from_fwhm(*fwhm)),
        PsfType::Moffat { fwhm, beta } => Box::new(MoffatPsf::from_fwhm(*fwhm, *beta)),
        PsfType::Airy { lambda_over_d } => Box::new(AiryPsf::new(*lambda_over_d)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet_config() -> SynthConfig {
        let mut config = SynthConfig::default();
        config.field.width = 64;
        config.field.height = 64;
        config.field.n_stars = 3;
        config.noise.readout_noise = 0.0;
        config.noise.sky_background = 0.0;
        config.noise.dark_current = 0.0;
        config.noise.bias_level = 0.0;
        config
    }

    #[test]
    fn ground_truth_and_catalog_are_in_the_units_of_the_noisy_frame() {
        let config = quiet_config();
        let (noisy, truth, stars) = generate(&config).unwrap();
        let noisy_sum: f64 = noisy.iter().map(|v| *v as f64).sum();
        let truth_sum: f64 = truth.iter().map(|v| *v as f64).sum();
        let poisson_adu = (truth_sum * config.noise.gain).sqrt() / config.noise.gain;
        assert!(
            (noisy_sum - truth_sum).abs() < 5.0 * poisson_adu + 1.0,
            "noisy frame holds {noisy_sum} ADU against a ground truth of {truth_sum} ADU"
        );
        let catalog_adu: f64 = stars.iter().map(|s| s.flux / config.noise.gain).sum();
        assert!(truth_sum <= catalog_adu * (1.0 + 1e-4), "{truth_sum} > {catalog_adu}");

        let csv = catalog_csv(&stars, &config.noise);
        let mut lines = csv.lines();
        assert_eq!(lines.next(), Some("id,x,y,z,flux_e,flux_adu,temperature"));
        let first: Vec<f64> = lines.next().unwrap().split(',').map(|v| v.parse().unwrap()).collect();
        assert!((first[5] - first[4] / config.noise.gain).abs() < 1e-3);
    }

    #[test]
    fn invalid_configurations_are_refused_before_any_work() {
        let bad: Vec<(&str, Box<dyn Fn(&mut SynthConfig)>)> = vec![
            ("Field size", Box::new(|c| c.field.width = 0)),
            ("Minimum flux", Box::new(|c| c.field.flux_min = 0.0)),
            ("Maximum flux", Box::new(|c| c.field.flux_max = 1.0)),
            ("Core radius", Box::new(|c| c.field_type = FieldType::KingCluster { core_radius: 0.0, tidal_radius: 100.0 })),
            ("Tidal radius", Box::new(|c| c.field_type = FieldType::KingCluster { core_radius: 5.0, tidal_radius: 0.0 })),
            ("PSF FWHM", Box::new(|c| c.psf_type = PsfType::Gaussian { fwhm: 0.0 })),
            ("Moffat beta", Box::new(|c| c.psf_type = PsfType::Moffat { fwhm: 3.0, beta: -1.0 })),
            ("Airy lambda/D", Box::new(|c| c.psf_type = PsfType::Airy { lambda_over_d: 0.0 })),
            ("Gain", Box::new(|c| c.noise.gain = 0.0)),
            ("Exposure time", Box::new(|c| c.noise.exposure_time = -1.0)),
        ];
        for (expected, corrupt) in bad {
            let mut config = quiet_config();
            corrupt(&mut config);
            let err = generate(&config).expect_err(expected);
            assert!(err.to_string().contains(expected), "{expected}: {err}");
        }
        let mut stack = quiet_config();
        stack.n_frames = 0;
        assert!(generate_stack(&stack).unwrap_err().to_string().contains("Frame count 0"));
        stack.n_frames = 2;
        assert_eq!(generate_stack(&stack).unwrap().0.len(), 2);
    }

    #[test]
    fn a_hopeless_king_profile_stops_instead_of_spinning() {
        let mut config = quiet_config();
        config.field_type = FieldType::KingCluster { core_radius: 200.0, tidal_radius: 1.0 };
        let err = generate(&config).expect_err("an acceptance of about 1e-11 must give up");
        assert!(err.to_string().contains("accepts almost no stars"), "{err}");
        config.field_type = FieldType::KingCluster { core_radius: 20.0, tidal_radius: 60.0 };
        assert_eq!(generate(&config).unwrap().2.len(), 3);
    }
}
