use anyhow::{bail, Result};
use ndarray::Array2;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use super::noise::{self, NoiseParams};
use super::psf::{self, AiryPsf, GaussianPsf, MoffatPsf, PsfModel};
use super::seed::{combine_seeds, derive_seed, SeedPurpose};
use super::star_field::{self, FieldConfig, Star};
use crate::types::header::HduHeader;

const MAX_SYNTH_DIM: u32 = 16384;
const MAX_SYNTH_STARS: usize = 1_000_000;
const MAX_SYNTH_FRAMES: u32 = 1024;
pub const SYNTH_PIXEL_BUDGET: u64 = 1 << 30;
const FRAME_UNIT: &str = "ADU";
pub const CADENCE_OVERHEAD_SECONDS: f64 = 10.0;
const EPOCH_MJD: f64 = 60310.0;
const EPOCH_DAYS_SINCE_UNIX: i64 = 19723;
const SECONDS_PER_DAY: f64 = 86400.0;
const SECONDS_PER_HOUR: f64 = 3600.0;
const SECONDS_PER_MINUTE: f64 = 60.0;
const UNIX_EPOCH_DAY_OFFSET: i64 = 719_468;
const DAYS_PER_400_YEARS: i64 = 146_097;
const AIRY_FWHM_PER_LAMBDA_OVER_D: f64 = 1.029;
const COSMIC_RAY_MIN_ELECTRONS: f64 = 5000.0;
const COSMIC_RAY_MAX_ELECTRONS: f64 = 50000.0;
const COSMIC_RAY_MAX_PIXELS: u32 = 3;
const PIXELS_PER_MEGAPIXEL: f64 = 1.0e6;
pub const GROUND_TRUTH_NOTE: &str = "noise-free star flux, pre-flat, no sky";
pub const OFFSET_CONVENTION: &str = "star = catalog + (SYNDX, SYNDY) px";

pub fn default_cadence(exposure_time: f64) -> f64 {
    exposure_time + CADENCE_OVERHEAD_SECONDS
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

impl FieldType {
    pub fn name(&self) -> &'static str {
        match self {
            FieldType::Uniform => "uniform",
            FieldType::KingCluster { .. } => "king",
            FieldType::ExponentialDisk { .. } => "disk",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PsfType {
    Gaussian { fwhm: f64 },
    Moffat { fwhm: f64, beta: f64 },
    Airy { lambda_over_d: f64 },
}

impl PsfType {
    pub fn name(&self) -> &'static str {
        match self {
            PsfType::Gaussian { .. } => "gaussian",
            PsfType::Moffat { .. } => "moffat",
            PsfType::Airy { .. } => "airy",
        }
    }

    pub fn fwhm(&self) -> f64 {
        match self {
            PsfType::Gaussian { fwhm } | PsfType::Moffat { fwhm, .. } => *fwhm,
            PsfType::Airy { lambda_over_d } => AIRY_FWHM_PER_LAMBDA_OVER_D * lambda_over_d,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameVariation {
    pub enabled: bool,
    pub dither_px: f64,
    pub fwhm_jitter: f64,
    pub sky_jitter: f64,
    pub transparency_jitter: f64,
    pub cosmic_rays_per_megapixel: f64,
}

impl Default for FrameVariation {
    fn default() -> Self {
        Self {
            enabled: true,
            dither_px: 3.0,
            fwhm_jitter: 0.10,
            sky_jitter: 0.05,
            transparency_jitter: 0.05,
            cosmic_rays_per_megapixel: 25.0,
        }
    }
}

impl FrameVariation {
    pub fn off() -> Self {
        Self { enabled: false, ..Self::default() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(from = "SynthConfigWire")]
pub struct SynthConfig {
    pub field: FieldConfig,
    pub field_type: FieldType,
    pub psf_type: PsfType,
    pub noise: NoiseParams,
    pub apply_vignette: bool,
    pub vignette_strength: f64,
    pub n_frames: u32,
    pub cadence_seconds: f64,
    pub frame_variation: FrameVariation,
}

#[derive(Deserialize)]
struct SynthConfigWire {
    field: FieldConfig,
    field_type: FieldType,
    psf_type: PsfType,
    noise: NoiseParams,
    apply_vignette: bool,
    vignette_strength: f64,
    n_frames: u32,
    cadence_seconds: Option<f64>,
    #[serde(default)]
    frame_variation: FrameVariation,
}

impl From<SynthConfigWire> for SynthConfig {
    fn from(wire: SynthConfigWire) -> Self {
        let cadence_seconds = wire.cadence_seconds.unwrap_or_else(|| default_cadence(wire.noise.exposure_time));
        Self {
            field: wire.field,
            field_type: wire.field_type,
            psf_type: wire.psf_type,
            noise: wire.noise,
            apply_vignette: wire.apply_vignette,
            vignette_strength: wire.vignette_strength,
            n_frames: wire.n_frames,
            cadence_seconds,
            frame_variation: wire.frame_variation,
        }
    }
}

impl Default for SynthConfig {
    fn default() -> Self {
        let noise = NoiseParams::default();
        let cadence_seconds = default_cadence(noise.exposure_time);
        Self {
            field: FieldConfig::default(),
            field_type: FieldType::Uniform,
            psf_type: PsfType::Gaussian { fwhm: 3.0 },
            noise,
            apply_vignette: false,
            vignette_strength: 0.3,
            n_frames: 1,
            cadence_seconds,
            frame_variation: FrameVariation::off(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SynthResult {
    pub width: u32,
    pub height: u32,
    pub star_count: usize,
    pub output_path: Option<String>,
    pub frames_manifest_path: Option<String>,
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

fn require_jitter(name: &str, v: f64) -> Result<()> {
    if !(v.is_finite() && (0.0..1.0).contains(&v)) {
        bail!("{name} must be within 0..1 (below 1), got {v}");
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
        let v = &self.frame_variation;
        if self.n_frames > 1 && v.enabled {
            require_non_negative("Dither", v.dither_px)?;
            require_jitter("FWHM jitter", v.fwhm_jitter)?;
            require_jitter("Sky jitter", v.sky_jitter)?;
            require_jitter("Transparency jitter", v.transparency_jitter)?;
            require_non_negative("Cosmic ray rate", v.cosmic_rays_per_megapixel)?;
        }
        Ok(())
    }

    fn noise_seed(&self, frame_index: u32) -> u64 {
        derive_seed(combine_seeds(self.field.seed, self.noise.seed), SeedPurpose::Noise, frame_index)
    }

    fn flat(&self) -> Option<Array2<f32>> {
        self.apply_vignette.then(|| {
            noise::generate_flat_field(
                self.field.width,
                self.field.height,
                derive_seed(self.field.seed, SeedPurpose::Flat, 0),
                self.vignette_strength,
            )
        })
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

pub fn observation_date(seconds_since_epoch: f64) -> String {
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
    header.set_f64("RDNOISE", noise.readout_noise);
    header.set("BUNIT", FRAME_UNIT.to_string());
    header.set("DATE-OBS", observation_date(seconds_since_epoch));
    header.set_f64("MJD-OBS", EPOCH_MJD + seconds_since_epoch / SECONDS_PER_DAY);
    header
}

pub fn synth_header(config: &SynthConfig, frame_index: u32, frame: Option<&FrameParams>) -> HduHeader {
    let mut header = frame_header(&config.noise, frame_index as usize, config.cadence_seconds);
    header.set("SYNSEED", config.field.seed.to_string());
    header.set("SYNFIELD", config.field_type.name().to_string());
    header.set("SYNPSF", config.psf_type.name().to_string());
    let fwhm_factor = frame.map_or(1.0, |p| p.fwhm_factor);
    header.set_f64("SYNFWHM", config.psf_type.fwhm() * fwhm_factor);
    if let Some(params) = frame {
        header.set("SYNFRAME", frame_index.to_string());
        header.set_f64("SYNDX", params.dx);
        header.set_f64("SYNDY", params.dy);
        header.set("SYNOFFS", OFFSET_CONVENTION.to_string());
    }
    header
}

pub fn ground_truth_header(config: &SynthConfig) -> HduHeader {
    let mut header = synth_header(config, 0, None);
    header.set("SYNTHGT", GROUND_TRUTH_NOTE.to_string());
    header
}

pub fn generate(config: &SynthConfig) -> Result<SynthFrame> {
    config.validate()?;
    let stars = gen_field(config)?;
    let ground_truth = render_stars(config, &stars, &FrameParams::reference());
    let flat = config.flat();
    let noisy = noise::apply_noise(&ground_truth, &config.noise, config.noise_seed(0), flat.as_ref());
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

#[derive(Clone, Debug, PartialEq)]
pub struct CosmicHit {
    pub x: usize,
    pub y: usize,
    pub electrons: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameParams {
    pub dx: f64,
    pub dy: f64,
    pub fwhm_factor: f64,
    pub sky_factor: f64,
    pub transparency: f64,
    pub n_cosmic_rays: usize,
    pub cosmic_hits: Vec<CosmicHit>,
}

impl FrameParams {
    pub fn reference() -> Self {
        Self {
            dx: 0.0,
            dy: 0.0,
            fwhm_factor: 1.0,
            sky_factor: 1.0,
            transparency: 1.0,
            n_cosmic_rays: 0,
            cosmic_hits: Vec::new(),
        }
    }

    fn is_reference(&self) -> bool {
        *self == Self::reference()
    }

    fn draw(config: &SynthConfig, frame_index: u32) -> Self {
        let v = &config.frame_variation;
        if !v.enabled || frame_index == 0 {
            return Self::reference();
        }
        let mut rng = StdRng::seed_from_u64(derive_seed(config.field.seed, SeedPurpose::Frame, frame_index));
        let mut symmetric = |span: f64| (rng.gen::<f64>() * 2.0 - 1.0) * span;
        let dx = symmetric(v.dither_px);
        let dy = symmetric(v.dither_px);
        let fwhm_factor = 1.0 + symmetric(v.fwhm_jitter);
        let sky_factor = 1.0 + symmetric(v.sky_jitter);
        let transparency = 1.0 + symmetric(v.transparency_jitter);
        let megapixels = f64::from(config.field.width) * f64::from(config.field.height) / PIXELS_PER_MEGAPIXEL;
        let n_cosmic_rays = (v.cosmic_rays_per_megapixel * megapixels).round().max(0.0) as usize;
        let mut cosmic_rng = StdRng::seed_from_u64(derive_seed(config.field.seed, SeedPurpose::Cosmic, frame_index));
        let cosmic_hits = cosmic_hits(&mut cosmic_rng, config.field.width, config.field.height, n_cosmic_rays);
        Self { dx, dy, fwhm_factor, sky_factor, transparency, n_cosmic_rays, cosmic_hits }
    }
}

fn cosmic_hits(rng: &mut impl Rng, width: u32, height: u32, n_rays: usize) -> Vec<CosmicHit> {
    let (w, h) = (width as i64, height as i64);
    let mut hits = Vec::new();
    for _ in 0..n_rays {
        let x0 = rng.gen_range(0..w);
        let y0 = rng.gen_range(0..h);
        let length = rng.gen_range(1..=COSMIC_RAY_MAX_PIXELS);
        let (step_x, step_y) = loop {
            let step = (rng.gen_range(-1i64..=1), rng.gen_range(-1i64..=1));
            if step != (0, 0) {
                break step;
            }
        };
        for k in 0..i64::from(length) {
            let (x, y) = (x0 + k * step_x, y0 + k * step_y);
            if x < 0 || y < 0 || x >= w || y >= h {
                break;
            }
            let electrons = rng.gen_range(COSMIC_RAY_MIN_ELECTRONS..=COSMIC_RAY_MAX_ELECTRONS);
            hits.push(CosmicHit { x: x as usize, y: y as usize, electrons });
        }
    }
    hits
}

fn render_stars(config: &SynthConfig, stars: &[Star], params: &FrameParams) -> Array2<f32> {
    let psf_model = make_psf(&config.psf_type, params.fwhm_factor);
    if params.is_reference() {
        return psf::render_stars(stars, psf_model.as_ref(), config.field.width, config.field.height);
    }
    let moved: Vec<Star> = stars
        .iter()
        .map(|s| Star { x: s.x + params.dx, y: s.y + params.dy, flux: s.flux * params.transparency, ..s.clone() })
        .collect();
    psf::render_stars(&moved, psf_model.as_ref(), config.field.width, config.field.height)
}

#[derive(Debug)]
pub struct StackPlan {
    config: SynthConfig,
    ground_truth: Array2<f32>,
    pub stars: Vec<Star>,
    flat: Option<Array2<f32>>,
    frames: Vec<FrameParams>,
}

impl StackPlan {
    pub fn n_frames(&self) -> u32 {
        self.config.n_frames
    }

    pub fn ground_truth_in_frame_units(&self) -> Array2<f32> {
        to_frame_units(&self.ground_truth, &self.config.noise)
    }

    pub fn flat(&self) -> Option<&Array2<f32>> {
        self.flat.as_ref()
    }

    pub fn frame_params(&self, frame_index: u32) -> &FrameParams {
        &self.frames[frame_index as usize]
    }

    pub fn header(&self, frame_index: u32) -> HduHeader {
        synth_header(&self.config, frame_index, Some(self.frame_params(frame_index)))
    }

    pub fn ground_truth_header(&self) -> HduHeader {
        ground_truth_header(&self.config)
    }

    pub fn date_obs(&self, frame_index: u32) -> String {
        observation_date(f64::from(frame_index) * self.config.cadence_seconds)
    }

    pub fn frame(&self, frame_index: u32) -> Array2<f32> {
        let params = self.frame_params(frame_index);
        let rendered;
        let stars_e = if params.is_reference() {
            &self.ground_truth
        } else {
            rendered = render_stars(&self.config, &self.stars, params);
            &rendered
        };
        let mut noise_params = self.config.noise.clone();
        noise_params.sky_background *= params.sky_factor;
        let mut img = noise::apply_noise(stars_e, &noise_params, self.config.noise_seed(frame_index), self.flat.as_ref());
        let gain = self.config.noise.gain;
        for hit in &params.cosmic_hits {
            img[[hit.y, hit.x]] += (hit.electrons / gain) as f32;
        }
        img
    }

    pub fn manifest_csv(&self) -> String {
        let mut out = String::from("frame,dx,dy,fwhm_factor,sky_factor,transparency,n_cosmic_rays,date_obs\n");
        for (i, p) in self.frames.iter().enumerate() {
            out.push_str(&format!(
                "{},{:.4},{:.4},{:.4},{:.4},{:.4},{},{}\n",
                i,
                p.dx,
                p.dy,
                p.fwhm_factor,
                p.sky_factor,
                p.transparency,
                p.n_cosmic_rays,
                self.date_obs(i as u32)
            ));
        }
        out
    }
}

pub fn prepare_stack(config: &SynthConfig) -> Result<StackPlan> {
    config.validate()?;
    if config.n_frames == 0 || config.n_frames > MAX_SYNTH_FRAMES {
        bail!("Frame count {} is outside 1..{}", config.n_frames, MAX_SYNTH_FRAMES);
    }
    check_stack_pixel_budget(config.field.width, config.field.height, config.n_frames)?;
    let stars = gen_field(config)?;
    let ground_truth = render_stars(config, &stars, &FrameParams::reference());
    let flat = config.flat();
    let frames = (0..config.n_frames).map(|i| FrameParams::draw(config, i)).collect();
    Ok(StackPlan { config: config.clone(), ground_truth, stars, flat, frames })
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
        } => star_field::exponential_disk(&config.field, *scale_length, *inclination_deg)?,
    })
}

fn make_psf(psf_type: &PsfType, width_factor: f64) -> Box<dyn PsfModel> {
    match psf_type {
        PsfType::Gaussian { fwhm } => Box::new(GaussianPsf::from_fwhm(*fwhm * width_factor)),
        PsfType::Moffat { fwhm, beta } => Box::new(MoffatPsf::from_fwhm(*fwhm * width_factor, *beta)),
        PsfType::Airy { lambda_over_d } => Box::new(AiryPsf::new(*lambda_over_d * width_factor)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::synth::star_field::in_frame;

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

    fn one_bright_star_config() -> SynthConfig {
        let mut config = quiet_config();
        config.field.n_stars = 1;
        config.field.flux_min = 1.0e7;
        config.field.flux_max = 1.0e7 + 1.0;
        config.field_type = FieldType::KingCluster { core_radius: 1.0, tidal_radius: 4.0 };
        config.n_frames = 4;
        config.frame_variation = FrameVariation { enabled: true, dither_px: 3.0, fwhm_jitter: 0.3, sky_jitter: 0.0, transparency_jitter: 0.0, cosmic_rays_per_megapixel: 0.0 };
        config
    }

    fn moments(img: &Array2<f32>, cx: f64, cy: f64, radius: i64) -> (f64, f64, f64) {
        let (h, w) = img.dim();
        let (mut sum, mut sx, mut sy) = (0.0, 0.0, 0.0);
        let mut sxx = 0.0;
        let mut syy = 0.0;
        let x0 = (cx.round() as i64 - radius).max(0) as usize;
        let x1 = ((cx.round() as i64 + radius) as usize).min(w - 1);
        let y0 = (cy.round() as i64 - radius).max(0) as usize;
        let y1 = ((cy.round() as i64 + radius) as usize).min(h - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let v = img[[y, x]] as f64;
                sum += v;
                sx += v * x as f64;
                sy += v * y as f64;
            }
        }
        let (mx, my) = (sx / sum, sy / sum);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let v = img[[y, x]] as f64;
                sxx += v * (x as f64 - mx).powi(2);
                syy += v * (y as f64 - my).powi(2);
            }
        }
        (mx, my, ((sxx + syy) / (2.0 * sum)).sqrt())
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
        let bad_variation: Vec<(&str, Box<dyn Fn(&mut FrameVariation)>)> = vec![
            ("Dither", Box::new(|v| v.dither_px = -1.0)),
            ("FWHM jitter", Box::new(|v| v.fwhm_jitter = 1.0)),
            ("Sky jitter", Box::new(|v| v.sky_jitter = f64::NAN)),
            ("Transparency jitter", Box::new(|v| v.transparency_jitter = -0.1)),
            ("Cosmic ray rate", Box::new(|v| v.cosmic_rays_per_megapixel = f64::INFINITY)),
        ];
        for (expected, corrupt) in bad_variation {
            let mut config = quiet_config();
            config.n_frames = 2;
            config.frame_variation = FrameVariation::default();
            corrupt(&mut config.frame_variation);
            let err = generate_stack(&config).expect_err(expected);
            assert!(err.to_string().contains(expected), "{expected}: {err}");
            config.n_frames = 1;
            generate(&config).unwrap_or_else(|e| panic!("{expected}: a single image ignores the variation block, got {e}"));
            config.n_frames = 2;
            config.frame_variation.enabled = false;
            generate_stack(&config).unwrap_or_else(|e| panic!("{expected}: a disabled variation block is ignored, got {e}"));
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

    #[test]
    fn at_the_default_configuration_most_stars_are_detectable_and_flux_max_is_not_a_wall() {
        let mut config = SynthConfig::default();
        config.field.width = 512;
        config.field.height = 512;
        let (_, truth, stars) = generate(&config).unwrap();
        let sigma = config.noise.background_sigma_adu();
        assert!((sigma - 12.98).abs() < 0.01, "sigma {sigma}");
        let (h, w) = truth.dim();
        let in_frame_stars: Vec<&Star> = stars.iter().filter(|s| in_frame(s.x, s.y, config.field.width, config.field.height)).collect();
        let detectable = in_frame_stars
            .iter()
            .filter(|s| {
                let (cx, cy) = (s.x.round() as i64, s.y.round() as i64);
                let mut peak = 0.0f32;
                for y in (cy - 1).max(0)..=(cy + 1).min(h as i64 - 1) {
                    for x in (cx - 1).max(0)..=(cx + 1).min(w as i64 - 1) {
                        peak = peak.max(truth[[y as usize, x as usize]]);
                    }
                }
                f64::from(peak) > 5.0 * sigma
            })
            .count();
        assert_eq!(in_frame_stars.len(), 500);
        assert!(
            detectable * 2 >= in_frame_stars.len(),
            "only {detectable} of {} stars peak above 5 sigma ({sigma:.2} ADU) at the defaults",
            in_frame_stars.len()
        );

        let brightest_overall = (0..500u64)
            .map(|seed| {
                let field = FieldConfig { seed, ..config.field.clone() };
                star_field::uniform_field(&field).iter().map(|s| s.flux).fold(0.0, f64::max)
            })
            .fold(0.0, f64::max);
        assert!(brightest_overall > config.field.flux_max * 0.5, "flux_max {} is never approached: brightest {brightest_overall}", config.field.flux_max);
    }

    #[test]
    fn the_sky_darkens_in_the_corners_with_vignetting_and_the_ground_truth_stays_pre_flat() {
        let mut config = quiet_config();
        config.field.width = 128;
        config.field.height = 128;
        config.field.n_stars = 0;
        config.noise.sky_background = 20_000.0;
        config.noise.bias_level = 100.0;
        config.apply_vignette = true;
        config.vignette_strength = 0.5;
        let (noisy, truth, _) = generate(&config).unwrap();
        let mean = |y0: usize, x0: usize| {
            let mut s = 0.0;
            for y in y0..y0 + 8 {
                for x in x0..x0 + 8 {
                    s += noisy[[y, x]] as f64;
                }
            }
            s / 64.0 - 100.0
        };
        let (centre, corner) = (mean(60, 60), mean(0, 0));
        assert!(corner < 0.6 * centre, "corner sky {corner} against centre {centre}");
        assert!(truth.iter().all(|v| *v == 0.0));
        let gt_header = ground_truth_header(&config);
        assert_eq!(gt_header.get("SYNTHGT"), Some(GROUND_TRUTH_NOTE));
        assert_eq!(gt_header.get("SYNFRAME"), None);
    }

    #[test]
    fn frame_zero_equals_the_single_image_and_later_frames_follow_the_manifest_offsets() {
        let config = one_bright_star_config();
        let (single, truth, stars) = generate(&config).unwrap();
        let plan = prepare_stack(&config).unwrap();
        assert_eq!(plan.frame(0), single);
        assert_eq!(plan.ground_truth_in_frame_units(), truth);
        assert_eq!(plan.frame_params(0), &FrameParams::reference());
        let star = &stars[0];
        let (x0, y0, width0) = moments(&plan.frame(0), star.x, star.y, 10);
        assert!((x0 - star.x).abs() < 0.05 && (y0 - star.y).abs() < 0.05, "reference centroid ({x0}, {y0}) vs catalog ({}, {})", star.x, star.y);
        let csv = plan.manifest_csv();
        let rows: Vec<&str> = csv.lines().collect();
        assert_eq!(rows[0], "frame,dx,dy,fwhm_factor,sky_factor,transparency,n_cosmic_rays,date_obs");
        assert_eq!(rows.len(), 5);
        assert!(rows[1].starts_with("0,0.0000,0.0000,1.0000,1.0000,1.0000,0,2024-01-01T00:00:00.000"), "{}", rows[1]);
        let mut moved = 0;
        for i in 1..4u32 {
            let p = plan.frame_params(i);
            let cols: Vec<&str> = rows[1 + i as usize].split(',').collect();
            assert_eq!(cols[0], i.to_string());
            assert!((cols[1].parse::<f64>().unwrap() - p.dx).abs() < 1e-4);
            assert!((cols[2].parse::<f64>().unwrap() - p.dy).abs() < 1e-4);
            assert!((cols[3].parse::<f64>().unwrap() - p.fwhm_factor).abs() < 1e-4);
            assert_eq!(cols[7], plan.date_obs(i));
            assert!(p.dx.abs() <= 3.0 && p.dy.abs() <= 3.0);
            assert!((p.fwhm_factor - 1.0).abs() <= 0.3 && (p.fwhm_factor - 1.0).abs() > 0.01, "frame {i} fwhm factor {}", p.fwhm_factor);
            let (xi, yi, width_i) = moments(&plan.frame(i), star.x + p.dx, star.y + p.dy, 10);
            assert!((xi - x0 - p.dx).abs() < 0.1, "frame {i}: centroid shift {} vs manifest dx {}", xi - x0, p.dx);
            assert!((yi - y0 - p.dy).abs() < 0.1, "frame {i}: centroid shift {} vs manifest dy {}", yi - y0, p.dy);
            let ratio = width_i / width0;
            assert!((ratio - p.fwhm_factor).abs() < 0.03, "frame {i}: measured width ratio {ratio} vs factor {}", p.fwhm_factor);
            if p.dx.hypot(p.dy) > 0.5 {
                moved += 1;
            }
        }
        assert!(moved >= 2, "the dither barely moved the star");
        assert_ne!(plan.frame_params(1).dx, plan.frame_params(2).dx);
    }

    #[test]
    fn cosmic_rays_hit_later_frames_only_and_single_images_never() {
        let mut config = quiet_config();
        config.field.width = 256;
        config.field.height = 256;
        config.field.n_stars = 0;
        config.n_frames = 3;
        config.frame_variation = FrameVariation { enabled: true, cosmic_rays_per_megapixel: 100.0, ..FrameVariation::default() };
        let plan = prepare_stack(&config).unwrap();
        assert_eq!(plan.frame_params(0).n_cosmic_rays, 0);
        assert!(plan.frame(0).iter().all(|v| *v == 0.0));
        assert_eq!(generate(&config).unwrap().0, plan.frame(0));
        for i in 1..3u32 {
            let p = plan.frame_params(i);
            assert_eq!(p.n_cosmic_rays, 7, "frame {i}");
            assert!(p.cosmic_hits.len() >= 7 && p.cosmic_hits.len() <= 21);
            let frame = plan.frame(i);
            let hot: Vec<f32> = frame.iter().copied().filter(|v| *v > 0.0).collect();
            assert_eq!(hot.len(), p.cosmic_hits.iter().map(|h| (h.x, h.y)).collect::<std::collections::HashSet<_>>().len());
            assert!(hot.iter().all(|v| *v >= (COSMIC_RAY_MIN_ELECTRONS / config.noise.gain) as f32));
        }
        assert_ne!(plan.frame_params(1).cosmic_hits, plan.frame_params(2).cosmic_hits);
    }

    #[test]
    fn disabling_the_variation_reproduces_identical_stars_with_noise_only_differences() {
        let mut config = quiet_config();
        config.field.flux_min = 50_000.0;
        config.field.flux_max = 60_000.0;
        config.n_frames = 3;
        config.frame_variation = FrameVariation::off();
        let plan = prepare_stack(&config).unwrap();
        let csv = plan.manifest_csv();
        assert!(csv.lines().skip(1).all(|row| row.contains(",0.0000,0.0000,1.0000,1.0000,1.0000,0,")), "{csv}");
        let (f0, f1) = (plan.frame(0), plan.frame(1));
        assert_ne!(f0, f1);
        let gain = config.noise.gain;
        for ((y, x), a) in f0.indexed_iter() {
            let b = f1[[y, x]];
            let sigma = ((*a as f64).max(1.0) * gain).sqrt() / gain * std::f64::consts::SQRT_2;
            assert!(((*a - b) as f64).abs() < 8.0 * sigma + 1.0, "pixel ({x}, {y}): {a} vs {b}");
        }
        for s in &plan.stars {
            let (x0, y0, _) = moments(&f0, s.x, s.y, 6);
            let (x1, y1, _) = moments(&f1, s.x, s.y, 6);
            assert!((x0 - x1).abs() < 0.05 && (y0 - y1).abs() < 0.05);
        }
    }

    #[test]
    fn the_default_cadence_is_the_exposure_plus_overhead_so_exposures_never_overlap() {
        let defaults = SynthConfig::default();
        assert_eq!(defaults.cadence_seconds, 310.0);
        let json = serde_json::json!({
            "field": {"width": 32, "height": 32, "n_stars": 2, "flux_min": 2000.0, "flux_max": 500000.0, "seed": 5},
            "field_type": "Uniform",
            "psf_type": {"Gaussian": {"fwhm": 3.0}},
            "noise": {"gain": 1.5, "readout_noise": 8.0, "sky_background": 200.0, "dark_current": 0.05, "exposure_time": 45.0, "bias_level": 1000.0},
            "apply_vignette": false,
            "vignette_strength": 0.3,
            "n_frames": 3
        });
        let config: SynthConfig = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(config.cadence_seconds, 55.0);
        assert_eq!(config.noise.seed, None);
        assert_eq!(config.frame_variation, FrameVariation::default());
        assert!(config.frame_variation.enabled);
        let plan = prepare_stack(&config).unwrap();
        for i in 1..3u32 {
            let step = (plan.header(i).get_f64("MJD-OBS").unwrap() - plan.header(i - 1).get_f64("MJD-OBS").unwrap()) * SECONDS_PER_DAY;
            assert!(step >= config.noise.exposure_time, "frames {} and {i} overlap: step {step} s", i - 1);
            assert!((step - 55.0).abs() < 1e-3, "step {step}");
        }
        let mut explicit = json;
        explicit["cadence_seconds"] = serde_json::json!(60.0);
        explicit["noise"]["seed"] = serde_json::json!(1005);
        explicit["frame_variation"] = serde_json::json!({"enabled": false});
        let config: SynthConfig = serde_json::from_value(explicit).unwrap();
        assert_eq!(config.cadence_seconds, 60.0);
        assert_eq!(config.noise.seed, Some(1005));
        assert_eq!(config.frame_variation, FrameVariation::off());
    }

    #[test]
    fn one_flat_serves_every_frame_of_a_stack() {
        let mut config = quiet_config();
        config.field.width = 48;
        config.field.height = 48;
        config.field.n_stars = 0;
        config.noise.sky_background = 1.0e6;
        config.apply_vignette = true;
        config.vignette_strength = 0.5;
        config.n_frames = 3;
        let plan = prepare_stack(&config).unwrap();
        let flat = plan.flat().expect("a vignetted stack keeps its flat");
        assert_eq!(flat.dim(), (48, 48));
        let (f0, f1) = (plan.frame(0), plan.frame(1));
        let ratios: Vec<f64> = f0.iter().zip(f1.iter()).map(|(a, b)| *a as f64 / *b as f64).collect();
        let mean = ratios.iter().sum::<f64>() / ratios.len() as f64;
        let sd = (ratios.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / ratios.len() as f64).sqrt();
        assert!(sd < 0.003, "frame0/frame1 ratio sd {sd}: the flat pattern changed between frames");
        let expected = flat.mapv(|f| (f as f64 * 1.0e6) as f32);
        for ((y, x), v) in f0.indexed_iter() {
            let e = expected[[y, x]] as f64;
            assert!(((*v as f64) - e).abs() < 6.0 * (e * config.noise.gain).sqrt() / config.noise.gain + 1.0, "({x}, {y}) {v} vs {e}");
        }
        let mut single = config.clone();
        single.n_frames = 1;
        assert_eq!(generate(&single).unwrap().0, f0);
    }

    #[test]
    fn seeds_7919_apart_no_longer_share_a_noise_realisation() {
        let mut a = quiet_config();
        a.field.n_stars = 0;
        a.noise.sky_background = 100.0;
        a.n_frames = 2;
        a.field.seed = 0;
        let mut b = a.clone();
        b.field.seed = 7919;
        b.n_frames = 1;
        let stack_a = prepare_stack(&a).unwrap();
        let (single_b, _, _) = generate(&b).unwrap();
        let frame1 = stack_a.frame(1);
        let identical = frame1.iter().zip(single_b.iter()).filter(|(x, y)| x == y).count();
        assert!(identical < frame1.len() / 4, "{identical} of {} background pixels identical between stack(0) frame 1 and single(7919)", frame1.len());
        let mut legacy = a.clone();
        legacy.noise.seed = Some(1000);
        assert_ne!(prepare_stack(&legacy).unwrap().frame(0), stack_a.frame(0));
        assert_eq!(prepare_stack(&legacy).unwrap().frame(0), prepare_stack(&legacy).unwrap().frame(0));
    }

    #[test]
    fn headers_carry_the_provenance_cards() {
        let mut config = one_bright_star_config();
        config.psf_type = PsfType::Airy { lambda_over_d: 2.5 };
        config.field.seed = 77;
        let plan = prepare_stack(&config).unwrap();
        let h = plan.header(2);
        assert_eq!(h.get_i64("SYNSEED"), Some(77));
        assert_eq!(h.get("SYNFIELD"), Some("king"));
        assert_eq!(h.get("SYNPSF"), Some("airy"));
        let factor = plan.frame_params(2).fwhm_factor;
        assert!((factor - 1.0).abs() > 0.01, "frame 2 factor {factor}");
        assert!((h.get_f64("SYNFWHM").unwrap() - 2.5725 * factor).abs() < 1e-9, "SYNFWHM {} vs rendered {}", h.get_f64("SYNFWHM").unwrap(), 2.5725 * factor);
        assert!((plan.header(0).get_f64("SYNFWHM").unwrap() - 2.5725).abs() < 1e-9);
        assert_eq!(h.get("SYNOFFS"), Some("star = catalog + (SYNDX, SYNDY) px"));
        assert_eq!(h.get_i64("SYNFRAME"), Some(2));
        assert!((h.get_f64("SYNDX").unwrap() - plan.frame_params(2).dx).abs() < 1e-9);
        assert!((h.get_f64("SYNDY").unwrap() - plan.frame_params(2).dy).abs() < 1e-9);
        assert_ne!(plan.frame_params(2).dx, 0.0);
        assert_eq!(h.get_f64("RDNOISE"), Some(0.0));
        let single = synth_header(&config, 0, None);
        assert_eq!(single.get("SYNFRAME"), None);
        assert_eq!(single.get("SYNDX"), None);
        assert_eq!(single.get("SYNOFFS"), None);
        assert_eq!(single.get("SYNFIELD"), Some("king"));
        assert_eq!(synth_header(&SynthConfig::default(), 0, None).get_f64("SYNFWHM"), Some(3.0));
        assert_eq!(FieldType::Uniform.name(), "uniform");
        assert_eq!(FieldType::ExponentialDisk { scale_length: 1.0, inclination_deg: 0.0 }.name(), "disk");
        assert_eq!(PsfType::Moffat { fwhm: 2.0, beta: 4.0 }.name(), "moffat");
    }
}
