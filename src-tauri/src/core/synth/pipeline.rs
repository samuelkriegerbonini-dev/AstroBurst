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
pub const SYNTH_PIXEL_BUDGET: u64 = 1 << 30;
const FRAME_UNIT: &str = "ADU";
const DEFAULT_CADENCE_SECONDS: f64 = 60.0;
const EPOCH_MJD: f64 = 60310.0;
const EPOCH_DAYS_SINCE_UNIX: i64 = 19723;
const SECONDS_PER_DAY: f64 = 86400.0;
const SECONDS_PER_HOUR: f64 = 3600.0;
const SECONDS_PER_MINUTE: f64 = 60.0;
const UNIX_EPOCH_DAY_OFFSET: i64 = 719_468;
const DAYS_PER_400_YEARS: i64 = 146_097;

fn default_cadence() -> f64 {
    DEFAULT_CADENCE_SECONDS
}

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
    #[serde(default = "default_cadence")]
    pub cadence_seconds: f64,
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
            cadence_seconds: DEFAULT_CADENCE_SECONDS,
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
        require_positive("Cadence", self.cadence_seconds)?;
        Ok(())
    }
}

fn to_frame_units(electrons: &Array2<f32>, noise: &NoiseParams) -> Array2<f32> {
    let gain = noise.gain as f32;
    electrons.mapv(|e| e / gain)
}

fn civil_from_unix_days(days: i64) -> (i64, u32, u32) {
    let z = days + UNIX_EPOCH_DAY_OFFSET;
    let era = if z >= 0 { z } else { z - (DAYS_PER_400_YEARS - 1) } / DAYS_PER_400_YEARS;
    let day_of_era = z - era * DAYS_PER_400_YEARS;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn observation_date(seconds_since_epoch: f64) -> String {
    let total = if seconds_since_epoch.is_finite() { seconds_since_epoch.max(0.0) } else { 0.0 };
    let days = (total / SECONDS_PER_DAY).floor();
    let remainder = total - days * SECONDS_PER_DAY;
    let hours = (remainder / SECONDS_PER_HOUR).floor();
    let minutes = ((remainder - hours * SECONDS_PER_HOUR) / SECONDS_PER_MINUTE).floor();
    let seconds = remainder - hours * SECONDS_PER_HOUR - minutes * SECONDS_PER_MINUTE;
    let (year, month, day) = civil_from_unix_days(EPOCH_DAYS_SINCE_UNIX + days as i64);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{seconds:06.3}", hours as u32, minutes as u32)
}

pub fn frame_header(noise: &NoiseParams, frame_index: usize, cadence_seconds: f64) -> HduHeader {
    let seconds_since_epoch = frame_index as f64 * cadence_seconds;
    let mut header = HduHeader::empty();
    header.set_f64("EXPTIME", noise.exposure_time);
    header.set_f64("GAIN", noise.gain);
    header.set("BUNIT", FRAME_UNIT.to_string());
    header.set("DATE-OBS", observation_date(seconds_since_epoch));
    header.set_f64("MJD-OBS", EPOCH_MJD + seconds_since_epoch / SECONDS_PER_DAY);
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

pub fn check_stack_pixel_budget(width: u32, height: u32, n_frames: u32) -> Result<()> {
    let total = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(u64::from(n_frames));
    if total > SYNTH_PIXEL_BUDGET {
        bail!(
            "A stack of {n_frames} frames of {width}x{height} pixels holds {total} pixels; the limit is {SYNTH_PIXEL_BUDGET} pixels, so reduce the frame count or the field size."
        );
    }
    Ok(())
}

#[derive(Debug)]
pub struct StackPlan {
    config: SynthConfig,
    ground_truth: Array2<f32>,
    pub stars: Vec<Star>,
}

impl StackPlan {
    pub fn n_frames(&self) -> u32 {
        self.config.n_frames
    }

    pub fn ground_truth_in_frame_units(&self) -> Array2<f32> {
        to_frame_units(&self.ground_truth, &self.config.noise)
    }

    pub fn header(&self, frame_index: u32) -> HduHeader {
        frame_header(&self.config.noise, frame_index as usize, self.config.cadence_seconds)
    }

    pub fn frame(&self, frame_index: u32) -> Array2<f32> {
        let mut img = self.ground_truth.clone();
        if self.config.apply_vignette {
            let flat = noise::generate_flat_field(
                self.config.field.width,
                self.config.field.height,
                self.config.noise.seed.wrapping_add(999).wrapping_add(u64::from(frame_index)),
                self.config.vignette_strength,
            );
            noise::apply_vignette(&mut img, &flat);
        }
        let mut np = self.config.noise.clone();
        np.seed = self.config.noise.seed.wrapping_add(u64::from(frame_index).wrapping_mul(7919));
        noise::apply_noise(&img, &np)
    }
}

pub fn prepare_stack(config: &SynthConfig) -> Result<StackPlan> {
    config.validate()?;
    if config.n_frames == 0 || config.n_frames > MAX_SYNTH_FRAMES {
        bail!("Frame count {} is outside 1..{}", config.n_frames, MAX_SYNTH_FRAMES);
    }
    check_stack_pixel_budget(config.field.width, config.field.height, config.n_frames)?;
    let stars = gen_field(config)?;
    let psf_model = make_psf(&config.psf_type);
    let ground_truth =
        psf::render_stars(&stars, psf_model.as_ref(), config.field.width, config.field.height);
    Ok(StackPlan { config: config.clone(), ground_truth, stars })
}

pub fn generate_stack(config: &SynthConfig) -> Result<SynthStack> {
    let plan = prepare_stack(config)?;
    let frames: Vec<Array2<f32>> = (0..plan.n_frames()).map(|i| plan.frame(i)).collect();
    Ok((frames, plan.ground_truth_in_frame_units(), plan.stars))
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
        stack.cadence_seconds = 0.0;
        assert!(generate_stack(&stack).unwrap_err().to_string().contains("Cadence"));
    }

    #[test]
    fn stack_frame_headers_carry_increasing_observation_times_from_a_fixed_epoch() {
        use crate::core::astrometry::time::{jd_from_mjd, parse_fits_datetime};
        let mut config = quiet_config();
        config.n_frames = 4;
        config.cadence_seconds = 90.0;
        let (frames, _, _) = generate_stack(&config).unwrap();
        let headers: Vec<HduHeader> =
            (0..frames.len()).map(|i| frame_header(&config.noise, i, config.cadence_seconds)).collect();
        assert_eq!(headers[0].get("DATE-OBS"), Some("2024-01-01T00:00:00.000"));
        assert_eq!(headers[0].get_f64("MJD-OBS"), Some(EPOCH_MJD));
        assert_eq!(headers[0].get_f64("EXPTIME"), Some(config.noise.exposure_time));
        for pair in headers.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let step_days = b.get_f64("MJD-OBS").unwrap() - a.get_f64("MJD-OBS").unwrap();
            assert!((step_days - 90.0 / SECONDS_PER_DAY).abs() < 1e-9, "step {step_days}");
            assert!(a.get("DATE-OBS").unwrap() < b.get("DATE-OBS").unwrap());
            let jd_from_date = parse_fits_datetime(b.get("DATE-OBS").unwrap()).unwrap();
            let jd_from_mjd_card = jd_from_mjd(b.get_f64("MJD-OBS").unwrap());
            assert!((jd_from_date - jd_from_mjd_card).abs() < 1e-8, "{jd_from_date} vs {jd_from_mjd_card}");
        }
        assert_eq!(observation_date(25.0 * 3600.0 + 61.5), "2024-01-02T01:01:01.500");
        assert_eq!(observation_date(366.0 * 86400.0), "2025-01-01T00:00:00.000");
        assert_eq!(civil_from_unix_days(0), (1970, 1, 1));
    }

    #[test]
    fn the_pixel_budget_accepts_a_stack_at_the_limit_and_refuses_one_pixel_more() {
        assert_eq!(SYNTH_PIXEL_BUDGET, 1_073_741_824);
        assert!(check_stack_pixel_budget(16384, 16384, 4).is_ok());
        let err = check_stack_pixel_budget(16384, 16384, 5).unwrap_err().to_string();
        assert!(err.contains("5 frames of 16384x16384 pixels"), "{err}");
        assert!(err.contains("the limit is 1073741824 pixels"), "{err}");
        assert!(check_stack_pixel_budget(u32::MAX, u32::MAX, u32::MAX).is_err());
        assert!(check_stack_pixel_budget(0, 16384, 1024).is_ok());
    }

    #[test]
    fn a_stack_over_the_pixel_budget_is_refused_before_any_frame_is_rendered() {
        let mut config = quiet_config();
        config.field.width = 16384;
        config.field.height = 16384;
        config.n_frames = 1024;
        let err = prepare_stack(&config).expect_err("16384x16384x1024 is over the budget").to_string();
        assert!(err.contains("the limit is"), "{err}");
        assert!(generate_stack(&config).unwrap_err().to_string().contains("the limit is"));
    }

    #[test]
    fn a_stack_plan_renders_each_frame_on_demand_and_deterministically() {
        let mut config = quiet_config();
        config.n_frames = 3;
        config.noise.readout_noise = 2.0;
        config.apply_vignette = true;
        let plan = prepare_stack(&config).unwrap();
        assert_eq!(plan.n_frames(), 3);
        assert_eq!(plan.stars.len(), 3);
        assert_eq!(plan.frame(1), plan.frame(1));
        assert_ne!(plan.frame(0), plan.frame(1));
        assert_eq!(plan.frame(2).dim(), (64, 64));
        let (frames, truth, stars) = generate_stack(&config).unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[2], plan.frame(2));
        assert_eq!(truth, plan.ground_truth_in_frame_units());
        assert_eq!(stars.len(), plan.stars.len());
        assert_eq!(plan.header(2).get_f64("MJD-OBS"), frame_header(&config.noise, 2, config.cadence_seconds).get_f64("MJD-OBS"));
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
