use std::collections::HashMap;

use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::core::analysis::photometry::{measure_star_prepared, MaskedImage, PhotometryConfig};
use crate::core::analysis::star_detection::{detect_stars, DetectedStar};
use crate::core::astrometry::cosmology::{FlatLcdm, PLANCK18_H0_KM_S_MPC, PLANCK18_OMEGA_M};
use crate::math::median::exact_median_f64;
use crate::types::constants::MAD_TO_SIGMA;
use crate::types::header::{parse_fits_float, HduHeader};

pub const LOBANOV_C: f64 = 0.9394372786996513;
pub const DEFAULT_PSF_FWHM_REL_ERR: f64 = 0.03;
pub const DEFAULT_RESOLVED_ELEMENTS: f64 = 3.0;
pub const DEFAULT_MIN_PIXELS_PER_FWHM: f64 = 2.0;
pub const DEFAULT_SCALE_OF_INTEREST_KPC: f64 = 1.0;
pub const HALF_LIGHT_FALLBACK_FLOOR: f64 = 0.92;
pub const AXIS_SCALE_TOLERANCE: f64 = 0.01;

const LN_4: f64 = 2.0 * std::f64::consts::LN_2;
const ARCSEC_PER_DEGREE: f64 = 3600.0;

const NOTE_GAUSSIAN_SIZE: &str =
    "intrinsic size = sqrt(observed FWHM^2 - PSF FWHM^2), exact for Gaussian profiles only";
const NOTE_HALF_LIGHT_SIZE: &str =
    "intrinsic size = 2 sqrt(R_source^2 - R_PSF^2) from half-light radii, a size proxy and not a FWHM";
const NOTE_SIZE_FLOOR: &str =
    "size floor = PSF FWHM x max(L, E): L from Lobanov 2005 Eq. 44 at the signal-to-noise, E = sqrt((1 + eps)^2 - 1)";
const NOTE_NO_SNR: &str = "no signal-to-noise given: the size floor uses the PSF term E only";
const NOTE_NO_PIXEL_SCALE: &str = "no pixel scale given: sampling was not checked";
const NOTE_NO_SOURCE_SIZE: &str = "no source size given: no verdict";
const NOTE_NO_PSF: &str = "no PSF FWHM given: only the physical scale is returned";
const NOTE_FWHM_WINS: &str = "both sizes given: the verdict uses the FWHM";
const NOTE_NOT_WIDER_THAN_PSF: &str = "the observed size does not exceed the PSF: intrinsic size 0";
const WARNING_NO_PSF_HALF_LIGHT_RADIUS: &str = "no PSF half-light radius given: FWHM / 2 is assumed and the size floor is raised to 0.92 of the PSF FWHM, because a star's half-light diameter exceeds its FWHM by up to 36 percent (Moffat beta 2.5)";

pub fn snr_size_limit_fraction(snr: f64) -> Option<f64> {
    if !snr.is_finite() {
        return None;
    }
    let x = 4.0 * LOBANOV_C * snr / (4.0 + LOBANOV_C * snr);
    if x <= 1.0 {
        return None;
    }
    let radicand = LN_4 / x.ln() - 1.0;
    (radicand >= 0.0).then(|| radicand.sqrt())
}

pub fn psf_error_size_floor_fraction(eps: f64) -> f64 {
    ((1.0 + eps) * (1.0 + eps) - 1.0).sqrt()
}

pub fn psf_noise_equivalent_area_px(fwhm_px: f64) -> f64 {
    std::f64::consts::PI * fwhm_px * fwhm_px / (2.0 * std::f64::consts::LN_2)
}

pub fn area_pixel_scale_arcsec(cd: &[[f64; 2]; 2]) -> Option<f64> {
    let scale = ARCSEC_PER_DEGREE * (cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0]).abs().sqrt();
    (scale.is_finite() && scale > 0.0).then_some(scale)
}

pub fn axis_pixel_scales_arcsec(cd: &[[f64; 2]; 2]) -> (f64, f64) {
    (
        cd[0][0].hypot(cd[1][0]) * ARCSEC_PER_DEGREE,
        cd[0][1].hypot(cd[1][1]) * ARCSEC_PER_DEGREE,
    )
}

pub fn anisotropy_warning(cd: &[[f64; 2]; 2]) -> Option<String> {
    let (x, y) = axis_pixel_scales_arcsec(cd);
    ((x - y).abs() / x.max(y) > AXIS_SCALE_TOLERANCE).then(|| {
        format!("pixel scales differ per axis ({x:.5} x {y:.5} arcsec); radii in arcsec use the square root of the pixel area")
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Unresolved,
    Marginal,
    Resolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeBasis {
    Fwhm,
    HalfLightRadius,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ResolutionInput {
    pub psf_fwhm_arcsec: Option<f64>,
    pub psf_reference: Option<String>,
    pub pixel_scale_arcsec: Option<f64>,
    pub source_fwhm_arcsec: Option<f64>,
    pub source_half_light_radius_arcsec: Option<f64>,
    pub psf_half_light_radius_arcsec: Option<f64>,
    pub snr: Option<f64>,
    pub redshift: Option<f64>,
    pub psf_fwhm_rel_err: Option<f64>,
    pub resolved_elements: Option<f64>,
    pub min_pixels_per_fwhm: Option<f64>,
    pub scale_of_interest_kpc: Option<f64>,
    pub h0_km_s_mpc: Option<f64>,
    pub omega_m: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolutionCheck {
    pub verdict: Option<Verdict>,
    pub size_basis: Option<SizeBasis>,
    pub psf_fwhm_arcsec: Option<f64>,
    pub psf_reference: Option<String>,
    pub pixels_per_fwhm: Option<f64>,
    pub nyquist_sampled: Option<bool>,
    pub psf_area_px: Option<f64>,
    pub intrinsic_size_arcsec: Option<f64>,
    pub snr_floor_fraction: Option<f64>,
    pub psf_floor_fraction: f64,
    pub floor_fraction: Option<f64>,
    pub size_floor_arcsec: Option<f64>,
    pub resolution_elements: Option<f64>,
    pub kpc_per_arcsec: Option<f64>,
    pub kpc_per_pixel: Option<f64>,
    pub angular_diameter_distance_mpc: Option<f64>,
    pub psf_fwhm_kpc: Option<f64>,
    pub intrinsic_size_kpc: Option<f64>,
    pub size_floor_kpc: Option<f64>,
    pub scale_of_interest_in_psf: Option<f64>,
    pub snr: Option<f64>,
    pub redshift: Option<f64>,
    pub psf_fwhm_rel_err: f64,
    pub resolved_elements: f64,
    pub min_pixels_per_fwhm: f64,
    pub scale_of_interest_kpc: f64,
    pub h0_km_s_mpc: f64,
    pub omega_m: f64,
    pub omega_nu_massive: f64,
    pub omega_rad: f64,
    pub cosmology: String,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

fn refuse_unless_positive(value: Option<f64>, refusal: &str) -> Result<(), String> {
    match value {
        Some(given) if !(given.is_finite() && given > 0.0) => Err(refusal.into()),
        _ => Ok(()),
    }
}

fn excess_in_quadrature(observed: f64, psf: f64) -> Option<f64> {
    (observed > psf).then(|| (observed * observed - psf * psf).sqrt())
}

pub fn classify(input: &ResolutionInput) -> Result<ResolutionCheck, String> {
    let cosmology = FlatLcdm::with(
        input.h0_km_s_mpc.unwrap_or(PLANCK18_H0_KM_S_MPC),
        input.omega_m.unwrap_or(PLANCK18_OMEGA_M),
    )?;
    refuse_unless_positive(input.psf_fwhm_arcsec, "PSF FWHM must be a positive number of arcsec")?;
    refuse_unless_positive(input.pixel_scale_arcsec, "Pixel scale must be a positive number of arcsec")?;
    refuse_unless_positive(input.source_fwhm_arcsec, "Source FWHM must be a positive number of arcsec")?;
    refuse_unless_positive(
        input.source_half_light_radius_arcsec,
        "Half-light radius must be a positive number of arcsec",
    )?;
    refuse_unless_positive(
        input.psf_half_light_radius_arcsec,
        "PSF half-light radius must be a positive number of arcsec",
    )?;
    if input.snr.is_some_and(|snr| !snr.is_finite()) {
        return Err("Signal-to-noise must be a finite number".into());
    }
    refuse_unless_positive(input.redshift, "Redshift must be above 0")?;
    let psf_fwhm_rel_err = input.psf_fwhm_rel_err.unwrap_or(DEFAULT_PSF_FWHM_REL_ERR);
    if !(0.0..1.0).contains(&psf_fwhm_rel_err) {
        return Err("PSF FWHM relative error must be at least 0 and below 1".into());
    }
    refuse_unless_positive(
        input.resolved_elements,
        "The resolved threshold must be a positive number of resolution elements",
    )?;
    refuse_unless_positive(
        input.min_pixels_per_fwhm,
        "The sampling limit must be a positive number of pixels per FWHM",
    )?;
    refuse_unless_positive(
        input.scale_of_interest_kpc,
        "The scale of interest must be a positive number of kpc",
    )?;
    let resolved_elements = input.resolved_elements.unwrap_or(DEFAULT_RESOLVED_ELEMENTS);
    let min_pixels_per_fwhm = input.min_pixels_per_fwhm.unwrap_or(DEFAULT_MIN_PIXELS_PER_FWHM);
    let scale_of_interest_kpc = input.scale_of_interest_kpc.unwrap_or(DEFAULT_SCALE_OF_INTEREST_KPC);

    let psf_floor_fraction = psf_error_size_floor_fraction(psf_fwhm_rel_err);
    let kpc_per_arcsec = input.redshift.and_then(|z| cosmology.kpc_per_arcsec(z));
    let mut check = ResolutionCheck {
        verdict: None,
        size_basis: None,
        psf_fwhm_arcsec: input.psf_fwhm_arcsec,
        psf_reference: input.psf_reference.clone(),
        pixels_per_fwhm: None,
        nyquist_sampled: None,
        psf_area_px: None,
        intrinsic_size_arcsec: None,
        snr_floor_fraction: None,
        psf_floor_fraction,
        floor_fraction: None,
        size_floor_arcsec: None,
        resolution_elements: None,
        kpc_per_arcsec,
        kpc_per_pixel: input.pixel_scale_arcsec.zip(kpc_per_arcsec).map(|(pixel, scale)| pixel * scale),
        angular_diameter_distance_mpc: input
            .redshift
            .and_then(|z| cosmology.angular_diameter_distance_mpc(z)),
        psf_fwhm_kpc: None,
        intrinsic_size_kpc: None,
        size_floor_kpc: None,
        scale_of_interest_in_psf: None,
        snr: input.snr,
        redshift: input.redshift,
        psf_fwhm_rel_err,
        resolved_elements,
        min_pixels_per_fwhm,
        scale_of_interest_kpc,
        h0_km_s_mpc: cosmology.h0_km_s_mpc,
        omega_m: cosmology.omega_m,
        omega_nu_massive: cosmology.omega_nu_massive,
        omega_rad: cosmology.omega_rad,
        cosmology: cosmology.description(),
        notes: Vec::new(),
        warnings: Vec::new(),
    };
    let Some(psf_fwhm) = input.psf_fwhm_arcsec else {
        check.notes.push(NOTE_NO_PSF.into());
        return Ok(check);
    };
    check.notes.push(NOTE_SIZE_FLOOR.into());
    check.psf_fwhm_kpc = kpc_per_arcsec.map(|scale| psf_fwhm * scale);
    check.scale_of_interest_in_psf = check.psf_fwhm_kpc.map(|psf_kpc| scale_of_interest_kpc / psf_kpc);

    match input.pixel_scale_arcsec {
        Some(pixel_scale) => {
            let pixels_per_fwhm = psf_fwhm / pixel_scale;
            check.pixels_per_fwhm = Some(pixels_per_fwhm);
            check.nyquist_sampled = Some(pixels_per_fwhm >= min_pixels_per_fwhm);
            check.psf_area_px = Some(psf_noise_equivalent_area_px(pixels_per_fwhm));
        }
        None => check.notes.push(NOTE_NO_PIXEL_SCALE.into()),
    }

    let mut floor_fraction = match input.snr {
        None => {
            check.notes.push(NOTE_NO_SNR.into());
            psf_floor_fraction
        }
        Some(snr) => match snr_size_limit_fraction(snr) {
            Some(snr_limit) => {
                check.snr_floor_fraction = Some(snr_limit);
                snr_limit.max(psf_floor_fraction)
            }
            None => {
                check.warnings.push(format!(
                    "signal-to-noise {snr:.2} is below 1.42: Lobanov 2005 Eq. 44 has no real value and no size can be measured"
                ));
                f64::INFINITY
            }
        },
    };

    let measured = match (input.source_fwhm_arcsec, input.source_half_light_radius_arcsec) {
        (Some(source_fwhm), half_light_radius) => {
            if half_light_radius.is_some() {
                check.notes.push(NOTE_FWHM_WINS.into());
            }
            Some((SizeBasis::Fwhm, NOTE_GAUSSIAN_SIZE, excess_in_quadrature(source_fwhm, psf_fwhm)))
        }
        (None, Some(source_radius)) => {
            let psf_radius = match input.psf_half_light_radius_arcsec {
                Some(psf_radius) => psf_radius,
                None => {
                    check.warnings.push(WARNING_NO_PSF_HALF_LIGHT_RADIUS.into());
                    floor_fraction = floor_fraction.max(HALF_LIGHT_FALLBACK_FLOOR);
                    psf_fwhm / 2.0
                }
            };
            let diameter = excess_in_quadrature(source_radius, psf_radius).map(|radius| 2.0 * radius);
            Some((SizeBasis::HalfLightRadius, NOTE_HALF_LIGHT_SIZE, diameter))
        }
        (None, None) => None,
    };

    let size_floor = psf_fwhm * floor_fraction;
    if floor_fraction.is_finite() {
        check.floor_fraction = Some(floor_fraction);
        check.size_floor_arcsec = Some(size_floor);
        check.size_floor_kpc = kpc_per_arcsec.map(|scale| size_floor * scale);
    }

    let Some((size_basis, size_note, excess)) = measured else {
        check.notes.push(NOTE_NO_SOURCE_SIZE.into());
        return Ok(check);
    };
    let intrinsic_size = excess.unwrap_or_else(|| {
        check.notes.push(NOTE_NOT_WIDER_THAN_PSF.into());
        0.0
    });
    check.notes.push(size_note.into());
    let resolution_elements = intrinsic_size / psf_fwhm;
    let undersampled_pixels_per_fwhm = check
        .pixels_per_fwhm
        .filter(|_| check.nyquist_sampled == Some(false));
    let verdict = if intrinsic_size < size_floor {
        Verdict::Unresolved
    } else if resolution_elements < resolved_elements {
        Verdict::Marginal
    } else if let Some(pixels_per_fwhm) = undersampled_pixels_per_fwhm {
        check.warnings.push(format!(
            "the PSF spans {pixels_per_fwhm:.2} pixels per FWHM, below the sampling limit of {min_pixels_per_fwhm}: no source is classed resolved"
        ));
        Verdict::Marginal
    } else {
        Verdict::Resolved
    };
    check.verdict = Some(verdict);
    check.size_basis = Some(size_basis);
    check.intrinsic_size_arcsec = Some(intrinsic_size);
    check.intrinsic_size_kpc = kpc_per_arcsec.map(|scale| intrinsic_size * scale);
    check.resolution_elements = Some(resolution_elements);
    Ok(check)
}

pub const IMAGE_PSF_DETECT_SIGMA: f64 = 5.0;
pub const IMAGE_PSF_MIN_PEAK_SNR: f64 = 50.0;
pub const IMAGE_PSF_MAX_ECCENTRICITY: f64 = 0.6;
pub const IMAGE_PSF_EDGE_MARGIN_PX: f64 = 16.0;
pub const IMAGE_PSF_ISOLATION_PX: f64 = 12.0;
pub const IMAGE_PSF_NEIGHBOUR_FLUX_FRACTION: f64 = 0.05;
pub const IMAGE_PSF_DEFAULT_MAX_STARS: usize = 200;
pub const IMAGE_PSF_MAX_STARS_LIMIT: usize = 2000;
pub const IMAGE_PSF_SEARCH_RADIUS_PX: usize = 2;
pub const IMAGE_PSF_MAX_RECENTRE_PX: f64 = 2.0;
pub const IMAGE_PSF_LOCUS_MIN_STARS: usize = 4;
pub const IMAGE_PSF_LOCUS_LOW: f64 = 0.80;
pub const IMAGE_PSF_LOCUS_HIGH: f64 = 1.10;
pub const IMAGE_PSF_SCATTER_WARNING: f64 = 0.10;
pub const IMAGE_PSF_MODEL: &str =
    "width at half maximum of a radial fit to the core of each star, the FWHM of the Photometry panel";
pub const SEEING_CARDS_ARCSEC: [&str; 3] = ["SEEING", "PSF_FWHM", "PSFFWHM"];

const IMAGE_PSF_SCATTER_MIN_STARS: usize = 3;
const SEEING_CARD_MAX_ARCSEC: f64 = 60.0;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImagePsfStar {
    pub x: f64,
    pub y: f64,
    pub fwhm_px: f64,
    pub half_light_radius_px: Option<f64>,
    pub peak_snr: f64,
    pub on_locus: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImagePsf {
    pub fwhm_px: f64,
    pub fwhm_px_scatter: Option<f64>,
    pub half_light_radius_px: Option<f64>,
    pub n_stars: usize,
    pub n_measured: usize,
    pub n_candidates: usize,
    pub n_detected: usize,
    pub stars: Vec<ImagePsfStar>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeeingCard {
    pub key: String,
    pub value_arcsec: f64,
}

pub fn stellar_locus(fwhm_px: &[f64]) -> Vec<bool> {
    if fwhm_px.len() < IMAGE_PSF_LOCUS_MIN_STARS {
        return vec![true; fwhm_px.len()];
    }
    let mut sorted = fwhm_px.to_vec();
    sorted.sort_by(f64::total_cmp);
    let p25 = sorted[(sorted.len() - 1) / 4];
    let locus = IMAGE_PSF_LOCUS_LOW * p25..=IMAGE_PSF_LOCUS_HIGH * p25;
    fwhm_px.iter().map(|width| width.is_finite() && locus.contains(width)).collect()
}

fn isolation_cell(star: &DetectedStar) -> (i64, i64) {
    (
        (star.x / IMAGE_PSF_ISOLATION_PX).floor() as i64,
        (star.y / IMAGE_PSF_ISOLATION_PX).floor() as i64,
    )
}

fn psf_candidates(detected: &[DetectedStar], rows: usize, cols: usize, max_stars: usize) -> Vec<&DetectedStar> {
    let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::with_capacity(detected.len());
    for (index, star) in detected.iter().enumerate() {
        cells.entry(isolation_cell(star)).or_default().push(index);
    }
    let inside_margin = |position: f64, extent: usize| {
        (IMAGE_PSF_EDGE_MARGIN_PX..=extent as f64 - 1.0 - IMAGE_PSF_EDGE_MARGIN_PX).contains(&position)
    };
    let crowded = |index: usize, star: &DetectedStar| {
        let (cell_x, cell_y) = isolation_cell(star);
        (cell_x - 1..=cell_x + 1)
            .flat_map(|x| (cell_y - 1..=cell_y + 1).map(move |y| (x, y)))
            .filter_map(|cell| cells.get(&cell))
            .flatten()
            .filter(|&&other| other != index)
            .map(|&other| &detected[other])
            .any(|other| {
                (other.x - star.x).powi(2) + (other.y - star.y).powi(2)
                    < IMAGE_PSF_ISOLATION_PX * IMAGE_PSF_ISOLATION_PX
                    && other.flux >= IMAGE_PSF_NEIGHBOUR_FLUX_FRACTION * star.flux
            })
    };
    detected
        .iter()
        .enumerate()
        .filter(|(index, star)| {
            star.snr >= IMAGE_PSF_MIN_PEAK_SNR
                && star.eccentricity <= IMAGE_PSF_MAX_ECCENTRICITY
                && inside_margin(star.x, cols)
                && inside_margin(star.y, rows)
                && !crowded(*index, star)
        })
        .map(|(_, star)| star)
        .take(max_stars)
        .collect()
}

pub fn image_psf(
    image: &Array2<f32>,
    excluded: Option<&Array2<u8>>,
    saturated: Option<&Array2<u8>>,
    saturation: Option<(f64, &'static str)>,
    max_stars: usize,
) -> Result<ImagePsf, String> {
    let detected = detect_stars(image, IMAGE_PSF_DETECT_SIGMA).stars;
    let n_detected = detected.len();
    if n_detected == 0 {
        return Err("No star was detected in this image".into());
    }
    let (rows, cols) = image.dim();
    let candidates = psf_candidates(&detected, rows, cols, max_stars);
    let n_candidates = candidates.len();
    if n_candidates == 0 {
        return Err(format!(
            "No detection is bright, round and isolated enough to measure the PSF ({n_detected} detected)"
        ));
    }

    let source = MaskedImage::new(image, excluded)?;
    let config = PhotometryConfig {
        search_radius: IMAGE_PSF_SEARCH_RADIUS_PX,
        saturation,
        ..PhotometryConfig::default()
    };
    let mut stars: Vec<ImagePsfStar> = candidates
        .iter()
        .filter_map(|candidate| {
            let measured = measure_star_prepared(&source, None, saturated, candidate.x, candidate.y, &config).ok()?;
            let fwhm_px = measured.fwhm?;
            let usable = !measured.saturated
                && measured.n_nonfinite == 0
                && measured.n_masked == 0
                && (measured.x - candidate.x).hypot(measured.y - candidate.y) <= IMAGE_PSF_MAX_RECENTRE_PX;
            usable.then_some(ImagePsfStar {
                x: measured.x,
                y: measured.y,
                fwhm_px,
                half_light_radius_px: measured.ee50_radius,
                peak_snr: candidate.snr,
                on_locus: false,
            })
        })
        .collect();
    let n_measured = stars.len();
    if n_measured == 0 {
        return Err(format!(
            "None of the {n_candidates} candidate stars could be measured (saturated, on masked pixels or without a measurable FWHM)"
        ));
    }

    let widths: Vec<f64> = stars.iter().map(|star| star.fwhm_px).collect();
    for (star, on_locus) in stars.iter_mut().zip(stellar_locus(&widths)) {
        star.on_locus = on_locus;
    }
    let locus_widths: Vec<f64> = stars.iter().filter(|star| star.on_locus).map(|star| star.fwhm_px).collect();
    let locus_radii: Vec<f64> = stars
        .iter()
        .filter(|star| star.on_locus)
        .filter_map(|star| star.half_light_radius_px)
        .collect();
    let n_stars = locus_widths.len();
    let fwhm_px = exact_median_f64(&locus_widths);
    let fwhm_px_scatter = (n_stars >= IMAGE_PSF_SCATTER_MIN_STARS).then(|| {
        let deviations: Vec<f64> = locus_widths.iter().map(|width| (width - fwhm_px).abs()).collect();
        MAD_TO_SIGMA * exact_median_f64(&deviations)
    });

    let mut warnings = Vec::new();
    if n_stars < IMAGE_PSF_SCATTER_MIN_STARS {
        warnings.push(format!(
            "the PSF FWHM rests on {n_stars} star{}: no scatter can be given",
            if n_stars == 1 { "" } else { "s" }
        ));
    }
    if let Some(scatter) = fwhm_px_scatter.filter(|scatter| scatter / fwhm_px > IMAGE_PSF_SCATTER_WARNING) {
        warnings.push(format!(
            "the selected stars differ in width by {:.0} percent (1.4826 x MAD over the median): the field may hold few stars or a PSF that varies; check the value or type the PSF FWHM",
            100.0 * scatter / fwhm_px
        ));
    }
    if fwhm_px < DEFAULT_MIN_PIXELS_PER_FWHM {
        warnings.push(format!(
            "the PSF spans {:.2} pixels per FWHM, below {}: the image is undersampled and the measured FWHM depends on the pixel phase of each star",
            fwhm_px, DEFAULT_MIN_PIXELS_PER_FWHM
        ));
    }

    Ok(ImagePsf {
        fwhm_px,
        fwhm_px_scatter,
        half_light_radius_px: (!locus_radii.is_empty()).then(|| exact_median_f64(&locus_radii)),
        n_stars,
        n_measured,
        n_candidates,
        n_detected,
        stars,
        warnings,
    })
}

pub fn seeing_cards(header: &HduHeader) -> Vec<SeeingCard> {
    SEEING_CARDS_ARCSEC
        .iter()
        .filter_map(|key| {
            let text = header.get(key)?.trim().trim_matches('\'');
            let value_arcsec = parse_fits_float(text)
                .filter(|value| value.is_finite() && *value > 0.0 && *value <= SEEING_CARD_MAX_ARCSEC)?;
            Some(SeeingCard { key: key.to_string(), value_arcsec })
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod test_support {
    use ndarray::Array2;

    const FIELD_SIDE: usize = 400;
    const FIELD_PEDESTAL: f32 = 100.0;
    const FIELD_NOISE_SIGMA: f64 = 1.0;
    const FIELD_NOISE_SEED: u64 = 7;
    const SOURCE_REACH_SIGMAS: f64 = 6.0;
    const GRID_SIDE: usize = 5;
    const GRID_PITCH_PX: f64 = 64.0;
    const GALAXY_AMPLITUDE: f64 = 400.0;
    const GALAXY_WIDTH_FACTOR: f64 = 2.0;
    pub(super) const GRID_STARS: usize = 25;

    pub(super) struct GaussianSource {
        pub x: f64,
        pub y: f64,
        pub amplitude: f64,
        pub sigma_px: f64,
    }

    pub(crate) fn gaussian_noise(rows: usize, cols: usize, sigma: f64, seed: u64) -> Array2<f32> {
        let mut state = seed;
        let mut uniform = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        Array2::from_shape_fn((rows, cols), |_| {
            let g = (-2.0 * uniform().ln()).sqrt() * (2.0 * std::f64::consts::PI * uniform()).cos();
            (sigma * g) as f32
        })
    }

    pub(super) fn grid_star(k: usize, sigma_px: f64) -> GaussianSource {
        let (i, j) = ((k % GRID_SIDE) as f64, (k / GRID_SIDE) as f64);
        GaussianSource {
            x: 72.0 + GRID_PITCH_PX * i + 0.3 + 0.1 * j,
            y: 72.0 + GRID_PITCH_PX * j + 0.6 - 0.1 * i,
            amplitude: 300.0 + 60.0 * k as f64,
            sigma_px,
        }
    }

    fn grid_galaxy(g: usize, sigma_px: f64) -> GaussianSource {
        GaussianSource {
            x: 40.0 + GRID_PITCH_PX * (g % GRID_SIDE) as f64 + 0.5,
            y: 104.0 + GRID_PITCH_PX * (g / GRID_SIDE) as f64 + 0.25,
            amplitude: GALAXY_AMPLITUDE,
            sigma_px: GALAXY_WIDTH_FACTOR * sigma_px,
        }
    }

    pub(super) fn field_of(sources: &[GaussianSource]) -> Array2<f32> {
        let mut field = gaussian_noise(FIELD_SIDE, FIELD_SIDE, FIELD_NOISE_SIGMA, FIELD_NOISE_SEED)
            .mapv(|noise| FIELD_PEDESTAL + noise);
        for source in sources {
            let reach = SOURCE_REACH_SIGMAS * source.sigma_px;
            let first = |centre: f64| (centre - reach).ceil().max(0.0) as usize;
            let last = |centre: f64| ((centre + reach).floor() as usize).min(FIELD_SIDE - 1);
            for py in first(source.y)..=last(source.y) {
                for px in first(source.x)..=last(source.x) {
                    let d2 = (px as f64 - source.x).powi(2) + (py as f64 - source.y).powi(2);
                    field[[py, px]] +=
                        (source.amplitude * (-d2 / (2.0 * source.sigma_px * source.sigma_px)).exp()) as f32;
                }
            }
        }
        field
    }

    pub(crate) fn star_field(sigma_px: f64, n_galaxies: usize, clip: Option<f32>) -> Array2<f32> {
        let sources: Vec<GaussianSource> = (0..GRID_STARS)
            .map(|k| grid_star(k, sigma_px))
            .chain((0..n_galaxies).map(|g| grid_galaxy(g, sigma_px)))
            .collect();
        let field = field_of(&sources);
        match clip {
            Some(level) => field.mapv(|value| value.min(level)),
            None => field,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test_support::{field_of, grid_star, star_field, GaussianSource, GRID_STARS};
    use crate::core::analysis::photometry::saturation_level;

    const N1: &str = "intrinsic size = sqrt(observed FWHM^2 - PSF FWHM^2), exact for Gaussian profiles only";
    const N2: &str = "intrinsic size = 2 sqrt(R_source^2 - R_PSF^2) from half-light radii, a size proxy and not a FWHM";
    const N3: &str = "size floor = PSF FWHM x max(L, E): L from Lobanov 2005 Eq. 44 at the signal-to-noise, E = sqrt((1 + eps)^2 - 1)";
    const N4: &str = "no signal-to-noise given: the size floor uses the PSF term E only";
    const N6: &str = "no pixel scale given: sampling was not checked";
    const N7: &str = "no source size given: no verdict";
    const N8: &str = "no PSF FWHM given: only the physical scale is returned";
    const N9: &str = "both sizes given: the verdict uses the FWHM";
    const N10: &str = "the observed size does not exceed the PSF: intrinsic size 0";
    const W2: &str = "no PSF half-light radius given: FWHM / 2 is assumed and the size floor is raised to 0.92 of the PSF FWHM, because a star's half-light diameter exceeds its FWHM by up to 36 percent (Moffat beta 2.5)";
    const PLANCK18_DESCRIPTION: &str =
        "flat LambdaCDM, H0 67.66, Omega_m 0.30966, plus Planck 2018 neutrinos and radiation";
    const SDSS_R_CD: [[f64; 2]; 2] = [
        [-2.70219937496e-6, 0.000109992169645],
        [0.000109959802344, 2.76135555586e-6],
    ];

    fn assert_relative(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance * expected.abs(),
            "{actual} differs from {expected} by more than {tolerance} relative"
        );
    }

    fn has(texts: &[String], text: &str) -> bool {
        texts.iter().any(|entry| entry == text)
    }

    fn gaussian_source(source_fwhm: f64, psf_fwhm: f64, snr: f64, pixel_scale: f64) -> ResolutionInput {
        ResolutionInput {
            psf_fwhm_arcsec: Some(psf_fwhm),
            pixel_scale_arcsec: Some(pixel_scale),
            source_fwhm_arcsec: Some(source_fwhm),
            snr: Some(snr),
            psf_fwhm_rel_err: Some(0.03),
            ..Default::default()
        }
    }

    fn source_at_redshift_1() -> ResolutionInput {
        ResolutionInput {
            psf_fwhm_arcsec: Some(1.14),
            pixel_scale_arcsec: Some(0.2),
            source_fwhm_arcsec: Some(1.20),
            snr: Some(50.0),
            redshift: Some(1.0),
            ..Default::default()
        }
    }

    fn half_light_source(source_radius: f64, psf_radius: Option<f64>) -> ResolutionInput {
        ResolutionInput {
            psf_fwhm_arcsec: Some(1.0),
            pixel_scale_arcsec: Some(0.25),
            snr: Some(50.0),
            source_half_light_radius_arcsec: Some(source_radius),
            psf_half_light_radius_arcsec: psf_radius,
            ..Default::default()
        }
    }

    #[test]
    fn lobanov_eq_44_matches_the_golden_table() {
        let golden = [
            (3.0, 1.3255324159833237),
            (5.0, 0.8943031484404383),
            (10.0, 0.5864023473339853),
            (20.0, 0.402176133512142),
            (50.0, 0.250290234995889),
            (100.0, 0.1760990674503604),
        ];
        for (snr, expected) in golden {
            assert_relative(snr_size_limit_fraction(snr).unwrap(), expected, 1e-12);
        }
        let from_definition = 2.0 * (std::f64::consts::LN_2 / std::f64::consts::PI).sqrt();
        assert!((LOBANOV_C - from_definition).abs() <= 1e-15);
    }

    #[test]
    fn lobanov_limit_has_no_value_at_or_below_snr_1_419() {
        for snr in [1.4192, 1.0, 0.0, -3.0, f64::NAN] {
            assert_eq!(snr_size_limit_fraction(snr), None, "snr {snr}");
        }
        assert!(snr_size_limit_fraction(1.42).unwrap() > 50.0);
    }

    #[test]
    fn psf_floor_follows_the_relative_error() {
        let golden = [
            (0.01, 0.14177446878757827),
            (0.02, 0.20099751242241778),
            (0.03, 0.24677925358506123),
            (0.05, 0.3201562118716425),
            (0.10, 0.4582575694955842),
        ];
        for (eps, expected) in golden {
            assert_relative(psf_error_size_floor_fraction(eps), expected, 1e-12);
        }
    }

    #[test]
    fn psf_area_is_the_sagui_expression_to_the_bit() {
        let f = 0.1 / 0.036;
        let sagui = std::f64::consts::PI * f * f / (2.0 * std::f64::consts::LN_2);
        assert_eq!(psf_noise_equivalent_area_px(f).to_bits(), sagui.to_bits());
        assert_relative(psf_noise_equivalent_area_px(f), 17.485957337296277, 1e-12);
        assert_relative(psf_noise_equivalent_area_px(3.0), 20.395620638222372, 1e-12);
    }

    #[test]
    fn the_nine_worked_examples_of_the_golden_keep_their_class() {
        let examples = [
            (1.004987562112089, 1.0, 20.0, 0.25, Verdict::Unresolved),
            (1.4142135623730951, 1.0, 20.0, 0.25, Verdict::Marginal),
            (4.123105625617661, 1.0, 20.0, 0.25, Verdict::Resolved),
            (1.077032961426901, 1.0, 20.0, 0.25, Verdict::Unresolved),
            (1.0807867504739312, 1.0, 20.0, 0.25, Verdict::Marginal),
            (3.1622776601683795, 1.0, 20.0, 0.25, Verdict::Resolved),
            (4.123105625617661, 1.0, 20.0, 0.6, Verdict::Marginal),
            (0.5768960044930109, 0.203, 50.0, 0.1, Verdict::Marginal),
            (1.2110557377759292, 1.084, 50.0, 0.262, Verdict::Marginal),
        ];
        let checks: Vec<ResolutionCheck> = examples
            .iter()
            .map(|(source, psf, snr, pixel, _)| classify(&gaussian_source(*source, *psf, *snr, *pixel)).unwrap())
            .collect();
        for (index, (check, example)) in checks.iter().zip(examples).enumerate() {
            assert_eq!(check.verdict, Some(example.4), "example {}", index + 1);
            assert_eq!(check.size_basis, Some(SizeBasis::Fwhm));
            assert!(has(&check.notes, N1));
            assert!(has(&check.notes, N3));
            assert_eq!(check.redshift, None);
            assert_eq!(check.kpc_per_arcsec, None);
        }
        assert_eq!(checks[5].resolution_elements, Some(3.0000000000000004));
        assert_eq!(checks[2].nyquist_sampled, Some(true));
        assert!(checks[2].warnings.is_empty());
        assert_eq!(checks[6].nyquist_sampled, Some(false));
        assert_eq!(
            checks[6].warnings,
            vec!["the PSF spans 1.67 pixels per FWHM, below the sampling limit of 2: no source is classed resolved".to_string()]
        );
        assert_relative(checks[7].resolution_elements.unwrap(), 2.6600985221674875, 1e-9);
        assert_relative(checks[8].size_floor_arcsec.unwrap(), 0.2713146147355437, 1e-12);
    }

    #[test]
    fn a_source_with_a_redshift_reports_sizes_in_kpc() {
        let check = classify(&source_at_redshift_1()).unwrap();
        assert_eq!(check.verdict, Some(Verdict::Marginal));
        assert_relative(check.intrinsic_size_arcsec.unwrap(), 0.374699879903904, 1e-12);
        assert_relative(check.size_floor_arcsec.unwrap(), 0.2853308678953134, 1e-12);
        assert_relative(check.resolution_elements.unwrap(), 0.3286841051788632, 1e-12);
        assert_relative(check.pixels_per_fwhm.unwrap(), 5.7, 1e-12);
        assert_relative(check.psf_area_px.unwrap(), 73.62819050398275, 1e-9);
        assert_relative(check.kpc_per_arcsec.unwrap(), 8.23125144264795, 1e-9);
        assert_relative(check.kpc_per_pixel.unwrap(), 1.6462502885295902, 1e-9);
        assert_relative(check.psf_fwhm_kpc.unwrap(), 9.383626644618662, 1e-9);
        assert_relative(check.intrinsic_size_kpc.unwrap(), 3.084248927019024, 1e-9);
        assert_relative(check.size_floor_kpc.unwrap(), 2.3486301179952904, 1e-9);
        assert_relative(check.scale_of_interest_in_psf.unwrap(), 0.10656860485529672, 1e-9);
        assert_relative(check.angular_diameter_distance_mpc.unwrap(), 1697.8174839889118, 1e-9);
        assert_eq!(check.nyquist_sampled, Some(true));
        assert_eq!(check.snr, Some(50.0));
        assert_eq!(check.redshift, Some(1.0));
        assert_relative(check.snr_floor_fraction.unwrap(), 0.250290234995889, 1e-12);
        assert_relative(check.floor_fraction.unwrap(), 0.250290234995889, 1e-12);
    }

    #[test]
    fn a_low_signal_to_noise_source_is_unresolved_by_the_lobanov_term() {
        let check = classify(&ResolutionInput {
            psf_fwhm_arcsec: Some(1.30),
            pixel_scale_arcsec: Some(0.396),
            source_fwhm_arcsec: Some(1.32),
            snr: Some(10.0),
            redshift: Some(0.1),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(check.verdict, Some(Verdict::Unresolved));
        assert_relative(check.floor_fraction.unwrap(), 0.5864023473339853, 1e-12);
        assert_relative(check.size_floor_arcsec.unwrap(), 0.762323051534181, 1e-12);
        assert_relative(check.intrinsic_size_arcsec.unwrap(), 0.22891046284519195, 1e-12);
        assert_relative(check.psf_fwhm_kpc.unwrap(), 2.478435460478056, 1e-9);
    }

    #[test]
    fn a_source_not_wider_than_the_psf_is_unresolved_with_size_zero() {
        let check = classify(&ResolutionInput {
            psf_fwhm_arcsec: Some(1.0),
            source_fwhm_arcsec: Some(0.9),
            snr: Some(20.0),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(check.verdict, Some(Verdict::Unresolved));
        assert_eq!(check.intrinsic_size_arcsec, Some(0.0));
        assert_eq!(check.resolution_elements, Some(0.0));
        assert!(has(&check.notes, N10));
        assert!(has(&check.notes, N6));
        assert_eq!(check.pixels_per_fwhm, None);
        assert_eq!(check.nyquist_sampled, None);
        assert_eq!(check.psf_area_px, None);
    }

    #[test]
    fn without_a_signal_to_noise_the_floor_is_the_psf_term() {
        let without_snr = |source_fwhm: f64| {
            classify(&ResolutionInput {
                psf_fwhm_arcsec: Some(1.0),
                pixel_scale_arcsec: Some(0.25),
                source_fwhm_arcsec: Some(source_fwhm),
                ..Default::default()
            })
            .unwrap()
        };
        let wider = without_snr(1.04);
        assert_eq!(wider.verdict, Some(Verdict::Marginal));
        assert_relative(wider.intrinsic_size_arcsec.unwrap(), 0.2856571371417142, 1e-12);
        assert_relative(wider.size_floor_arcsec.unwrap(), 0.24677925358506123, 1e-12);
        assert!(has(&wider.notes, N4));
        assert_eq!(wider.snr_floor_fraction, None);
        assert_eq!(wider.snr, None);
        let narrower = without_snr(1.02);
        assert_eq!(narrower.verdict, Some(Verdict::Unresolved));
        assert_relative(narrower.intrinsic_size_arcsec.unwrap(), 0.20099751242241778, 1e-12);
        assert!(has(&narrower.notes, N4));
        assert_eq!(narrower.snr_floor_fraction, None);
    }

    #[test]
    fn a_signal_to_noise_below_1_42_measures_no_size() {
        let check = classify(&ResolutionInput {
            psf_fwhm_arcsec: Some(1.0),
            source_fwhm_arcsec: Some(5.0),
            snr: Some(1.2),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(check.verdict, Some(Verdict::Unresolved));
        assert_eq!(check.floor_fraction, None);
        assert_eq!(check.size_floor_arcsec, None);
        assert_eq!(check.snr_floor_fraction, None);
        assert_eq!(
            check.warnings,
            vec!["signal-to-noise 1.20 is below 1.42: Lobanov 2005 Eq. 44 has no real value and no size can be measured".to_string()]
        );
    }

    #[test]
    fn half_light_radii_are_compared_like_with_like() {
        let check = classify(&half_light_source(0.9, Some(0.55))).unwrap();
        assert_eq!(check.size_basis, Some(SizeBasis::HalfLightRadius));
        assert_relative(check.intrinsic_size_arcsec.unwrap(), 1.4247806848775009, 1e-12);
        assert_relative(check.size_floor_arcsec.unwrap(), 0.250290234995889, 1e-12);
        assert_eq!(check.verdict, Some(Verdict::Marginal));
        assert!(check.warnings.is_empty());
        assert!(has(&check.notes, N2));
        assert!(!has(&check.notes, N1));
    }

    #[test]
    fn without_a_psf_half_light_radius_a_moffat_star_stays_unresolved() {
        let star = classify(&half_light_source(0.678, None)).unwrap();
        assert_relative(star.intrinsic_size_arcsec.unwrap(), 0.9158253108535492, 1e-12);
        assert_eq!(star.floor_fraction, Some(0.92));
        assert_eq!(star.verdict, Some(Verdict::Unresolved));
        assert_eq!(star.warnings, vec![W2.to_string()]);
        let galaxy = classify(&half_light_source(2.0, None)).unwrap();
        assert_relative(galaxy.intrinsic_size_arcsec.unwrap(), 3.872983346207417, 1e-12);
        assert_eq!(galaxy.verdict, Some(Verdict::Resolved));
        assert_eq!(galaxy.warnings, vec![W2.to_string()]);
    }

    #[test]
    fn a_fwhm_wins_when_both_sizes_are_given() {
        let check = classify(&ResolutionInput {
            source_half_light_radius_arcsec: Some(0.1),
            ..gaussian_source(4.123105625617661, 1.0, 20.0, 0.25)
        })
        .unwrap();
        assert_eq!(check.verdict, Some(Verdict::Resolved));
        assert_eq!(check.size_basis, Some(SizeBasis::Fwhm));
        assert!(has(&check.notes, N9));
        assert!(check.warnings.is_empty());
    }

    #[test]
    fn without_a_psf_only_the_scale_is_returned() {
        let scale_only = classify(&ResolutionInput {
            redshift: Some(1.0),
            pixel_scale_arcsec: Some(0.262),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(scale_only.verdict, None);
        assert_eq!(scale_only.psf_fwhm_arcsec, None);
        assert_eq!(scale_only.size_floor_arcsec, None);
        assert_eq!(scale_only.psf_fwhm_kpc, None);
        assert_relative(scale_only.kpc_per_arcsec.unwrap(), 8.23125144264795, 1e-9);
        assert_relative(scale_only.kpc_per_pixel.unwrap(), 2.156587877973763, 1e-9);
        assert_relative(
            scale_only.kpc_per_pixel.unwrap(),
            0.262 * scale_only.kpc_per_arcsec.unwrap(),
            1e-12,
        );
        assert_eq!(scale_only.notes, vec![N8.to_string()]);

        let empty = classify(&ResolutionInput::default()).unwrap();
        assert_eq!(empty.verdict, None);
        assert_eq!(empty.size_basis, None);
        assert_eq!(empty.psf_fwhm_arcsec, None);
        assert_eq!(empty.psf_reference, None);
        assert_eq!(empty.pixels_per_fwhm, None);
        assert_eq!(empty.nyquist_sampled, None);
        assert_eq!(empty.psf_area_px, None);
        assert_eq!(empty.intrinsic_size_arcsec, None);
        assert_eq!(empty.snr_floor_fraction, None);
        assert_eq!(empty.floor_fraction, None);
        assert_eq!(empty.size_floor_arcsec, None);
        assert_eq!(empty.resolution_elements, None);
        assert_eq!(empty.kpc_per_arcsec, None);
        assert_eq!(empty.kpc_per_pixel, None);
        assert_eq!(empty.angular_diameter_distance_mpc, None);
        assert_eq!(empty.psf_fwhm_kpc, None);
        assert_eq!(empty.intrinsic_size_kpc, None);
        assert_eq!(empty.size_floor_kpc, None);
        assert_eq!(empty.scale_of_interest_in_psf, None);
        assert_eq!(empty.snr, None);
        assert_eq!(empty.redshift, None);
        assert_relative(empty.psf_floor_fraction, 0.24677925358506123, 1e-12);
        assert_eq!(empty.psf_fwhm_rel_err, 0.03);
        assert_eq!(empty.resolved_elements, 3.0);
        assert_eq!(empty.min_pixels_per_fwhm, 2.0);
        assert_eq!(empty.scale_of_interest_kpc, 1.0);
        assert_eq!(empty.h0_km_s_mpc, 67.66);
        assert_eq!(empty.omega_m, 0.30966);
        assert_eq!(empty.omega_nu_massive, 0.0014147613306646316);
        assert_eq!(empty.omega_rad, 7.893312479128647e-5);
        assert_eq!(empty.cosmology, PLANCK18_DESCRIPTION);
        assert_eq!(empty.notes, vec![N8.to_string()]);
        assert!(empty.warnings.is_empty());
    }

    #[test]
    fn a_psf_without_a_source_gives_the_floor_and_the_psf_in_kpc() {
        let check = classify(&ResolutionInput {
            psf_fwhm_arcsec: Some(0.203),
            psf_reference: Some("Euclid VIS mosaic".into()),
            pixel_scale_arcsec: Some(0.1),
            redshift: Some(1.0),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(check.verdict, None);
        assert_eq!(check.size_basis, None);
        assert_eq!(check.intrinsic_size_arcsec, None);
        assert_eq!(check.resolution_elements, None);
        assert_eq!(check.intrinsic_size_kpc, None);
        assert!(has(&check.notes, N7));
        assert!(has(&check.notes, N3));
        assert!(has(&check.notes, N4));
        assert_eq!(check.psf_fwhm_arcsec, Some(0.203));
        assert_eq!(check.psf_reference.as_deref(), Some("Euclid VIS mosaic"));
        assert_relative(check.pixels_per_fwhm.unwrap(), 2.03, 1e-12);
        assert_eq!(check.nyquist_sampled, Some(true));
        assert_relative(check.psf_area_px.unwrap(), 9.33870145422784, 1e-12);
        assert_relative(check.size_floor_arcsec.unwrap(), 0.05009618847776743, 1e-12);
        assert_relative(check.psf_fwhm_kpc.unwrap(), 1.6709440428575342, 1e-9);
        assert_relative(check.psf_fwhm_kpc.unwrap(), 1.6709437983763447, 1e-6);
        assert_relative(check.scale_of_interest_in_psf.unwrap(), 0.5984640863794987, 1e-9);
        assert_relative(
            check.size_floor_kpc.unwrap(),
            0.05009618847776743 * 8.23125144264795,
            1e-9,
        );
    }

    #[test]
    fn another_cosmology_changes_the_scale_and_is_echoed() {
        let check = classify(&ResolutionInput {
            psf_fwhm_arcsec: Some(1.0),
            redshift: Some(1.0),
            h0_km_s_mpc: Some(70.0),
            omega_m: Some(0.3),
            ..Default::default()
        })
        .unwrap();
        assert_relative(check.kpc_per_arcsec.unwrap(), 8.001803902976224, 1e-9);
        assert_eq!(check.h0_km_s_mpc, 70.0);
        assert_eq!(check.omega_m, 0.3);
        assert_relative(check.omega_nu_massive, 0.0013217553827496221, 1e-12);
        assert_relative(check.omega_rad, 7.374408694158886e-5, 1e-12);
        assert_eq!(
            check.cosmology,
            "flat LambdaCDM, H0 70, Omega_m 0.3, plus Planck 2018 neutrinos and radiation"
        );
    }

    #[test]
    fn every_threshold_is_an_input_and_is_echoed() {
        let lower_threshold = classify(&ResolutionInput {
            resolved_elements: Some(0.9),
            ..gaussian_source(1.4142135623730951, 1.0, 20.0, 0.25)
        })
        .unwrap();
        assert_eq!(lower_threshold.verdict, Some(Verdict::Resolved));
        assert_eq!(lower_threshold.resolved_elements, 0.9);

        let poorly_known_psf = classify(&ResolutionInput {
            psf_fwhm_rel_err: Some(0.5),
            ..gaussian_source(1.4142135623730951, 1.0, 1000.0, 0.25)
        })
        .unwrap();
        assert_eq!(poorly_known_psf.verdict, Some(Verdict::Unresolved));
        assert_eq!(poorly_known_psf.psf_fwhm_rel_err, 0.5);
        assert_relative(poorly_known_psf.psf_floor_fraction, 1.118033988749895, 1e-12);
        assert_relative(poorly_known_psf.floor_fraction.unwrap(), 1.118033988749895, 1e-12);

        let stricter_sampling = classify(&ResolutionInput {
            min_pixels_per_fwhm: Some(5.0),
            ..gaussian_source(4.123105625617661, 1.0, 20.0, 0.25)
        })
        .unwrap();
        assert_eq!(stricter_sampling.verdict, Some(Verdict::Marginal));
        assert_eq!(stricter_sampling.min_pixels_per_fwhm, 5.0);
        assert_eq!(
            stricter_sampling.warnings,
            vec!["the PSF spans 4.00 pixels per FWHM, below the sampling limit of 5: no source is classed resolved".to_string()]
        );

        let all_four = classify(&ResolutionInput {
            psf_fwhm_rel_err: Some(0.05),
            resolved_elements: Some(2.5),
            min_pixels_per_fwhm: Some(2.5),
            scale_of_interest_kpc: Some(0.5),
            redshift: Some(1.0),
            ..gaussian_source(1.20, 1.14, 50.0, 0.2)
        })
        .unwrap();
        assert_eq!(all_four.psf_fwhm_rel_err, 0.05);
        assert_eq!(all_four.resolved_elements, 2.5);
        assert_eq!(all_four.min_pixels_per_fwhm, 2.5);
        assert_eq!(all_four.scale_of_interest_kpc, 0.5);
        assert_relative(all_four.psf_floor_fraction, 0.3201562118716425, 1e-12);
        assert_relative(all_four.scale_of_interest_in_psf.unwrap(), 0.5 / 9.383626644618662, 1e-9);
    }

    #[test]
    fn classify_refuses_inputs_that_are_not_physical() {
        let refusals = [
            (
                ResolutionInput { psf_fwhm_arcsec: Some(0.0), ..Default::default() },
                "PSF FWHM must be a positive number of arcsec",
            ),
            (
                ResolutionInput { pixel_scale_arcsec: Some(-1.0), ..Default::default() },
                "Pixel scale must be a positive number of arcsec",
            ),
            (
                ResolutionInput { source_fwhm_arcsec: Some(f64::NAN), ..Default::default() },
                "Source FWHM must be a positive number of arcsec",
            ),
            (
                ResolutionInput { source_half_light_radius_arcsec: Some(0.0), ..Default::default() },
                "Half-light radius must be a positive number of arcsec",
            ),
            (
                ResolutionInput { psf_half_light_radius_arcsec: Some(-2.0), ..Default::default() },
                "PSF half-light radius must be a positive number of arcsec",
            ),
            (
                ResolutionInput { snr: Some(f64::INFINITY), ..Default::default() },
                "Signal-to-noise must be a finite number",
            ),
            (
                ResolutionInput { redshift: Some(0.0), ..Default::default() },
                "Redshift must be above 0",
            ),
            (
                ResolutionInput { redshift: Some(-0.1), ..Default::default() },
                "Redshift must be above 0",
            ),
            (
                ResolutionInput { psf_fwhm_rel_err: Some(-0.01), ..Default::default() },
                "PSF FWHM relative error must be at least 0 and below 1",
            ),
            (
                ResolutionInput { psf_fwhm_rel_err: Some(1.0), ..Default::default() },
                "PSF FWHM relative error must be at least 0 and below 1",
            ),
            (
                ResolutionInput { resolved_elements: Some(0.0), ..Default::default() },
                "The resolved threshold must be a positive number of resolution elements",
            ),
            (
                ResolutionInput { min_pixels_per_fwhm: Some(0.0), ..Default::default() },
                "The sampling limit must be a positive number of pixels per FWHM",
            ),
            (
                ResolutionInput { scale_of_interest_kpc: Some(0.0), ..Default::default() },
                "The scale of interest must be a positive number of kpc",
            ),
            (
                ResolutionInput { h0_km_s_mpc: Some(5.0), ..Default::default() },
                "H0 must be between 20 and 200 km/s/Mpc",
            ),
            (
                ResolutionInput { omega_m: Some(1.2), ..Default::default() },
                "Omega_m must be above 0 and below 1",
            ),
        ];
        for (input, expected) in refusals {
            assert_eq!(classify(&input).unwrap_err(), expected);
        }
        let first_refusal_wins = ResolutionInput {
            psf_fwhm_arcsec: Some(0.0),
            pixel_scale_arcsec: Some(-1.0),
            ..Default::default()
        };
        assert_eq!(
            classify(&first_refusal_wins).unwrap_err(),
            "PSF FWHM must be a positive number of arcsec"
        );
    }

    #[test]
    fn the_request_deserialises_with_any_field_missing_and_the_result_uses_snake_case() {
        let empty: ResolutionInput = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.psf_fwhm_arcsec, None);
        let with_unknown: ResolutionInput =
            serde_json::from_str(r#"{"psf_fwhm_arcsec":1.0,"unknown":3}"#).unwrap();
        assert_eq!(with_unknown.psf_fwhm_arcsec, Some(1.0));
        assert_eq!(with_unknown.snr, None);

        let json = serde_json::to_value(classify(&source_at_redshift_1()).unwrap()).unwrap();
        assert_eq!(json["verdict"], "marginal");
        assert_eq!(json["size_basis"], "fwhm");
        assert!(json["psf_reference"].is_null());
        assert!(json.as_object().unwrap().contains_key("psf_reference"));
        assert_eq!(json["nyquist_sampled"], true);
        assert_eq!(json["psf_fwhm_arcsec"], 1.14);
        let half_light = serde_json::to_value(classify(&half_light_source(0.9, Some(0.55))).unwrap()).unwrap();
        assert_eq!(half_light["size_basis"], "half_light_radius");
    }

    #[test]
    fn area_pixel_scale_is_the_square_root_of_the_cd_determinant() {
        assert_relative(area_pixel_scale_arcsec(&SDSS_R_CD).unwrap(), 0.39603565421543135, 1e-12);
        assert_eq!(anisotropy_warning(&SDSS_R_CD), None);

        let anisotropic = [[0.29 / 3600.0, 0.0], [0.0, 0.68 / 3600.0]];
        assert_relative(area_pixel_scale_arcsec(&anisotropic).unwrap(), 0.4440720662234904, 1e-12);
        let (x, y) = axis_pixel_scales_arcsec(&anisotropic);
        assert_relative(x, 0.29, 1e-12);
        assert_relative(y, 0.68, 1e-12);
        assert_eq!(
            anisotropy_warning(&anisotropic).as_deref(),
            Some("pixel scales differ per axis (0.29000 x 0.68000 arcsec); radii in arcsec use the square root of the pixel area")
        );

        assert_eq!(area_pixel_scale_arcsec(&[[0.0; 2]; 2]), None);
        assert_eq!(area_pixel_scale_arcsec(&[[f64::NAN; 2]; 2]), None);
        assert_eq!(anisotropy_warning(&[[0.0; 2]; 2]), None);
        assert_eq!(anisotropy_warning(&[[f64::NAN; 2]; 2]), None);
    }

    const FWHM_OF_SIGMA_1_8: f64 = 4.238676081055709;
    const HALF_LIGHT_RADIUS_OF_SIGMA_1_8: f64 = 2.1193;
    const LEGACY_PIXEL_SCALE_ARCSEC: f64 = 0.262;

    fn largest_pixel(image: &Array2<f32>) -> f64 {
        image.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64
    }

    fn psf_of(image: &Array2<f32>) -> Result<ImagePsf, String> {
        image_psf(image, None, None, Some(saturation_level(None, largest_pixel(image))), 200)
    }

    fn dimmed_grid(divisor: f64) -> Array2<f32> {
        let stars: Vec<GaussianSource> = (0..GRID_STARS)
            .map(|k| {
                let star = grid_star(k, 1.8);
                GaussianSource { amplitude: star.amplitude / divisor, ..star }
            })
            .collect();
        field_of(&stars)
    }

    fn header_of(cards: &[(&str, &str)]) -> HduHeader {
        let mut header = HduHeader::empty();
        for (key, value) in cards {
            header.set(key, value.to_string());
        }
        header
    }

    fn legacy_fitted_rows() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("survey_qa")
            .join("legacy_l2_r_fitted_rows.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn verdict_name(row: &serde_json::Value, psf_fwhm_arcsec: f64) -> serde_json::Value {
        let check = classify(&ResolutionInput {
            psf_fwhm_arcsec: Some(psf_fwhm_arcsec),
            pixel_scale_arcsec: Some(LEGACY_PIXEL_SCALE_ARCSEC),
            source_fwhm_arcsec: row["fwhm_arcsec"].as_f64(),
            snr: row["snr"].as_f64(),
            ..Default::default()
        })
        .unwrap();
        serde_json::to_value(check.verdict.unwrap()).unwrap()
    }

    #[test]
    fn stellar_locus_keeps_the_values_near_the_25th_percentile() {
        assert_eq!(
            stellar_locus(&[3.0, 3.1, 3.05, 2.95, 5.0, 6.0, 1.0]),
            vec![true, true, true, true, false, false, false]
        );
        assert_eq!(stellar_locus(&[4.0, 9.0, 4.1]), vec![true, true, true]);
        assert_eq!(
            stellar_locus(&[2.0, 2.0, 2.0, 2.0, f64::NAN]),
            vec![true, true, true, true, false]
        );
        assert_eq!(stellar_locus(&[]), Vec::<bool>::new());
    }

    #[test]
    fn gaussian_stars_report_their_fwhm() {
        let psf = psf_of(&star_field(1.8, 0, None)).unwrap();
        assert_relative(psf.fwhm_px, FWHM_OF_SIGMA_1_8, 0.01);
        assert!((23..=25).contains(&psf.n_stars), "n_stars {}", psf.n_stars);
        assert!(psf.fwhm_px_scatter.unwrap() < 0.05, "scatter {:?}", psf.fwhm_px_scatter);
        assert_eq!(psf.n_measured, psf.n_stars);
        assert_eq!(psf.stars.len(), psf.n_measured);
        assert!(psf.warnings.is_empty(), "{:?}", psf.warnings);
        assert!(psf.stars.iter().all(|star| star.on_locus));
    }

    #[test]
    fn broader_sources_are_left_off_the_stellar_locus() {
        let psf = psf_of(&star_field(1.8, 8, None)).unwrap();
        assert_relative(psf.fwhm_px, FWHM_OF_SIGMA_1_8, 0.01);
        assert!(
            psf.n_measured >= psf.n_stars + 6,
            "{} measured, {} on the locus",
            psf.n_measured,
            psf.n_stars
        );
        for star in psf.stars.iter().filter(|star| !star.on_locus) {
            assert!(star.fwhm_px > 7.0, "off the locus with {} px", star.fwhm_px);
        }
    }

    #[test]
    fn clipped_stars_are_not_used() {
        let unclipped = psf_of(&star_field(1.8, 0, None)).unwrap();
        let clipped = psf_of(&star_field(1.8, 0, Some(1200.0))).unwrap();
        assert!(clipped.n_measured >= 12, "n_measured {}", clipped.n_measured);
        assert!(clipped.n_measured < unclipped.n_measured);
        assert_relative(clipped.fwhm_px, FWHM_OF_SIGMA_1_8, 0.02);
    }

    #[test]
    fn a_star_on_excluded_pixels_is_not_used() {
        let image = star_field(1.8, 0, None);
        let mut excluded = Array2::<u8>::zeros(image.dim());
        for y in 71..=75 {
            for x in 70..=74 {
                excluded[[y, x]] = 1;
            }
        }
        let saturation = Some(saturation_level(None, largest_pixel(&image)));
        let masked = image_psf(&image, Some(&excluded), None, saturation, 200).unwrap();
        assert_eq!(masked.n_measured + 1, psf_of(&image).unwrap().n_measured);
        assert!(masked.stars.iter().all(|star| (star.x - 72.3).hypot(star.y - 72.6) > 3.0));
    }

    #[test]
    fn an_undersampled_field_says_so() {
        let psf = psf_of(&star_field(0.68, 0, None)).unwrap();
        assert!(psf.fwhm_px < 2.0, "fwhm {}", psf.fwhm_px);
        let undersampled: Vec<&String> =
            psf.warnings.iter().filter(|warning| warning.contains("undersampled")).collect();
        assert_eq!(undersampled.len(), 1, "{:?}", psf.warnings);
        assert_eq!(
            undersampled[0],
            &format!(
                "the PSF spans {:.2} pixels per FWHM, below 2: the image is undersampled and the measured FWHM depends on the pixel phase of each star",
                psf.fwhm_px
            )
        );
    }

    #[test]
    fn few_stars_give_no_scatter() {
        let psf = psf_of(&field_of(&[grid_star(6, 1.8), grid_star(18, 1.8)])).unwrap();
        assert_eq!(psf.n_stars, 2);
        assert_eq!(psf.fwhm_px_scatter, None);
        assert_eq!(
            psf.warnings,
            vec!["the PSF FWHM rests on 2 stars: no scatter can be given".to_string()]
        );
    }

    #[test]
    fn an_image_without_usable_stars_is_refused_with_the_reason() {
        assert_eq!(psf_of(&field_of(&[])).unwrap_err(), "No star was detected in this image");
        let faint = psf_of(&dimmed_grid(100.0)).unwrap_err();
        assert!(
            faint.starts_with("No detection is bright, round and isolated enough to measure the PSF ("),
            "{faint}"
        );
        let mixed = psf_of(&dimmed_grid(20.0)).unwrap();
        assert!((8..=14).contains(&mixed.n_candidates), "n_candidates {}", mixed.n_candidates);
        assert!(mixed.n_candidates < mixed.n_detected);
    }

    #[test]
    fn the_half_light_radius_of_gaussian_stars_is_1_18_sigma() {
        let psf = psf_of(&star_field(1.8, 0, None)).unwrap();
        assert_relative(psf.half_light_radius_px.unwrap(), HALF_LIGHT_RADIUS_OF_SIGMA_1_8, 0.08);
    }

    #[test]
    fn seeing_cards_read_the_three_arcsec_cards_and_skip_fwhm() {
        let header = header_of(&[
            ("SEEING", "0.85"),
            ("PSF_FWHM", "'1.1'"),
            ("FWHM", "4.5"),
            ("PSFFWHM", "-1.0"),
        ]);
        assert_eq!(
            seeing_cards(&header),
            vec![
                SeeingCard { key: "SEEING".into(), value_arcsec: 0.85 },
                SeeingCard { key: "PSF_FWHM".into(), value_arcsec: 1.1 },
            ]
        );
        assert!(seeing_cards(&header_of(&[("SEEING", "'NaN'")])).is_empty());
        assert!(seeing_cards(&header_of(&[("SEEING", "75.0")])).is_empty());
    }

    #[test]
    fn the_rule_reproduces_the_oracle_classes_of_the_55_legacy_rows() {
        let fixture = legacy_fitted_rows();
        assert_eq!(fixture["pixel_scale_arcsec"].as_f64(), Some(LEGACY_PIXEL_SCALE_ARCSEC));
        let image_psf_fwhm_arcsec = fixture["image_psf_fwhm_arcsec"].as_f64().unwrap();
        assert_eq!(image_psf_fwhm_arcsec, 0.9760859756254778);
        let rows = fixture["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 55);

        let mut counts = std::collections::BTreeMap::<(bool, &str, String), usize>::new();
        for row in rows {
            let is_psf_type = row["type"] == "PSF";
            let with_catalogue_psf = verdict_name(row, row["psfsize_r_arcsec"].as_f64().unwrap());
            let with_image_psf = verdict_name(row, image_psf_fwhm_arcsec);
            assert_eq!(with_catalogue_psf, row["class_with_psfsize_r"], "row {}", row["id"]);
            assert_eq!(with_image_psf, row["class_with_image_psf"], "row {}", row["id"]);
            for (reference, verdict) in [("psfsize_r", with_catalogue_psf), ("image", with_image_psf)] {
                *counts
                    .entry((is_psf_type, reference, verdict.as_str().unwrap().to_string()))
                    .or_default() += 1;
            }
        }
        let counted: Vec<((bool, &str, &str), usize)> = counts
            .iter()
            .map(|((is_psf_type, reference, verdict), count)| ((*is_psf_type, *reference, verdict.as_str()), *count))
            .collect();
        assert_eq!(
            counted,
            vec![
                ((false, "image", "marginal"), 31),
                ((false, "image", "resolved"), 1),
                ((false, "psfsize_r", "marginal"), 31),
                ((false, "psfsize_r", "resolved"), 1),
                ((true, "image", "marginal"), 9),
                ((true, "image", "unresolved"), 14),
                ((true, "psfsize_r", "unresolved"), 23),
            ]
        );
    }
}
