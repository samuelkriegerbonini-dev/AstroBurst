use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use anyhow::{anyhow, bail};
use ndarray::{Array2, Zip};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use super::{photometry_context_with, PhotometryContext, UserZeroPoint, MAX_SKY_ANNULUS_RADIUS, PAR_BATCH_POINTS};
use crate::cmd::common::{blocking_cmd, cached_header, load_companions, source_path};
use crate::core::analysis::aperture::circular_aperture;
use crate::core::analysis::forced::{
    ab_from_jy, aperture_correction_from_growth, aperture_coverage, flags_under_aperture, growth_at_radii, growth_ratio,
    masked_fraction, measure_plain_aperture, pixel_scale_from_cd, point_source_locus, select_correction_stars,
    upper_limit, ApertureCorrection, CorrectionStar, PixelScale, FORCED_AXIS_SCALE_TOLERANCE,
    FORCED_DEFAULT_LIMIT_SIGMA, FORCED_DEFAULT_REFERENCE_RADIUS_ARCSEC, FORCED_MIN_CORRECTION_STARS,
    FORCED_STAR_MIN_SNR, FORCED_SUBSAMPLES,
};
use crate::core::analysis::photometry::{
    fwhm_at_position, saturation_verdict, sky_in_annulus, GrowthPoint, MaskedImage, PhotometryConfig,
    MAX_APERTURE_RADIUS, MIN_APERTURE_RADIUS,
};
use crate::core::analysis::star_detection::{detect_stars, DetectedStar};
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::metadata::filter_wavelengths::{FilterEntry, FilterError, FilterSource};
use crate::core::metadata::photcal::{card_text, MagSystem, PhotCal};
use crate::infra::fits::band_planes::loaded_band_plane;
use crate::infra::image_source::ErrOrigin;
use crate::infra::progress::ProgressHandle;
use crate::infra::wcs_source::load_wcs;
use crate::service::cubeprep::filter_registry;
use crate::types::constants::HEADER_BUNIT;
use crate::types::error::AppError;
use crate::types::header::HduHeader;

#[cfg(test)] mod real_legacy;
#[cfg(test)] mod real_surveys;

const CORRECTION_DETECTION_SIGMA: f64 = 5.0;
const MAX_CORRECTION_CANDIDATES: usize = 500;
const SCATTER_WARNING_FRACTION: f64 = 0.5;
const SCATTER_WARNING_FLOOR: f64 = 0.02;
const ANGSTROM_PER_MICRON: f64 = 1.0e4;
const ERR_ROLE_SIGMA: &str = "sigma";
const ERR_LAYER_SOURCE: &str = "ERR layer of the file";
const FILTER_FROM_FILE_NAME: &str = "file name";
const FILTER_FROM_REQUEST: &str = "request";
const NO_DATA_IN_APERTURE: &str = "the aperture holds no pixel with data";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ForcedBandInput {
    pub path: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub dataset: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ForcedSkyPosition {
    pub id: String,
    pub ra_deg: f64,
    pub dec_deg: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForcedCorrection {
    None,
    GrowthCurve,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ForcedAperture {
    #[serde(default)]
    pub radius_arcsec: Option<f64>,
    #[serde(default)]
    pub radius_px: Option<f64>,
    #[serde(default)]
    pub annulus_arcsec: Option<[f64; 2]>,
    #[serde(default)]
    pub annulus_px: Option<[f64; 2]>,
    pub correction: ForcedCorrection,
    #[serde(default)]
    pub reference_radius_arcsec: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForcedOptions {
    pub exclude_dq: bool,
    pub gain: Option<f64>,
    pub limit_sigma: f64,
    pub f_corr: Option<f64>,
    pub zero_point: Option<UserZeroPoint>,
}

impl Default for ForcedOptions {
    fn default() -> Self {
        Self { exclude_dq: false, gain: None, limit_sigma: FORCED_DEFAULT_LIMIT_SIGMA, f_corr: None, zero_point: None }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ForcedFilter {
    pub dataset: String,
    pub filter: String,
    pub pivot_um: f64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForcedErrOrigin {
    pub role: String,
    pub source: String,
    pub assumed: bool,
    pub n_no_data: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ForcedFlagName {
    pub value: u32,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForcedBand {
    pub index: usize,
    pub path: String,
    pub file_name: String,
    pub label: String,
    pub error: Option<String>,
    pub filter: Option<ForcedFilter>,
    pub photcal: serde_json::Value,
    pub calibration_origin: Option<String>,
    pub jy_per_native_unit: Option<f64>,
    pub bunit: Option<String>,
    pub pixel_scale_arcsec: Option<f64>,
    pub aperture_radius_px: Option<f64>,
    pub annulus_px: Option<[f64; 2]>,
    pub aperture_correction: Option<ApertureCorrection>,
    pub err_origin: Option<ForcedErrOrigin>,
    pub dq_source: Option<String>,
    pub f_corr: Option<f64>,
    pub n_measured: usize,
    pub n_failed: usize,
    pub warnings: Vec<String>,
    pub flag_names: Vec<ForcedFlagName>,
    pub mag_system: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForcedCell {
    pub x: f64,
    pub y: f64,
    pub on_image: bool,
    pub error: Option<String>,
    pub flux_native: Option<f64>,
    pub flux_err_native: Option<f64>,
    pub flux_jy: Option<f64>,
    pub flux_err_jy: Option<f64>,
    pub flux_err_corr_jy: Option<f64>,
    pub flux_total_jy: Option<f64>,
    pub mag_ab: Option<f64>,
    pub mag_ab_err: Option<f64>,
    pub upper_limit_jy: Option<f64>,
    pub upper_limit_ab: Option<f64>,
    pub snr: Option<f64>,
    pub masked_fraction: Option<f64>,
    pub n_nonfinite: u32,
    pub n_masked: u32,
    pub flags_or: Option<u32>,
    pub saturated: bool,
    pub fwhm_px: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForcedBandResult {
    pub band: ForcedBand,
    pub cells: Vec<ForcedCell>,
}

impl ForcedCell {
    fn off_image(x: f64, y: f64) -> Self {
        Self {
            x,
            y,
            on_image: false,
            error: None,
            flux_native: None,
            flux_err_native: None,
            flux_jy: None,
            flux_err_jy: None,
            flux_err_corr_jy: None,
            flux_total_jy: None,
            mag_ab: None,
            mag_ab_err: None,
            upper_limit_jy: None,
            upper_limit_ab: None,
            snr: None,
            masked_fraction: None,
            n_nonfinite: 0,
            n_masked: 0,
            flags_or: None,
            saturated: false,
            fwhm_px: None,
        }
    }

    fn unmeasured(x: f64, y: f64) -> Self {
        Self { on_image: true, ..Self::off_image(x, y) }
    }
}

fn is_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn file_name_of(path: &str) -> String {
    let source = source_path(path);
    Path::new(&source).file_name().and_then(|name| name.to_str()).map_or_else(|| source.clone(), str::to_string)
}

fn requested_radius(aperture: &ForcedAperture) -> anyhow::Result<f64> {
    let radius = match (aperture.radius_arcsec, aperture.radius_px) {
        (Some(_), Some(_)) => bail!("give the aperture radius either in arcsec or in pixels, not both"),
        (None, None) => bail!("the aperture needs a radius in arcsec or in pixels"),
        (Some(radius), None) | (None, Some(radius)) => radius,
    };
    if !is_positive(radius) {
        bail!("aperture radius {radius} must be a positive finite number");
    }
    Ok(radius)
}

pub(crate) fn forced_aperture_checked(aperture: &ForcedAperture) -> anyhow::Result<()> {
    requested_radius(aperture)?;
    if aperture.annulus_arcsec.is_some() && aperture.annulus_px.is_some() {
        bail!("give the sky annulus either in arcsec or in pixels, not both");
    }
    if let Some([inner, outer]) = aperture.annulus_arcsec.or(aperture.annulus_px) {
        if !(is_positive(inner) && is_positive(outer) && outer > inner) {
            bail!("sky annulus [{inner}, {outer}] needs two positive finite radii with the outer one larger");
        }
    }
    if let Some(reference) = aperture.reference_radius_arcsec.filter(|radius| !is_positive(*radius)) {
        bail!("reference radius {reference} arcsec must be a positive finite number");
    }
    Ok(())
}

fn aperture_in_pixels(
    aperture: &ForcedAperture,
    scale_arcsec: f64,
    file_name: &str,
) -> anyhow::Result<(f64, Option<[f64; 2]>)> {
    let requested = requested_radius(aperture)?;
    let radius_px = match aperture.radius_arcsec {
        Some(radius_arcsec) => radius_arcsec / scale_arcsec,
        None => requested,
    };
    if !(MIN_APERTURE_RADIUS..=MAX_APERTURE_RADIUS).contains(&radius_px) {
        match aperture.radius_arcsec {
            Some(radius_arcsec) => bail!(
                "aperture radius {radius_arcsec} arcsec is {radius_px:.2} px on {file_name} ({scale_arcsec:.4} arcsec/px): the limits are {MIN_APERTURE_RADIUS} to {MAX_APERTURE_RADIUS} px"
            ),
            None => bail!(
                "aperture radius {radius_px} px on {file_name}: the limits are {MIN_APERTURE_RADIUS} to {MAX_APERTURE_RADIUS} px"
            ),
        }
    }
    let annulus_px =
        aperture.annulus_px.or(aperture.annulus_arcsec.map(|radii| radii.map(|radius| radius / scale_arcsec)));
    if let Some([inner, outer]) = annulus_px {
        if inner <= radius_px {
            bail!(
                "sky annulus inner radius {inner:.2} px must be larger than the aperture radius {radius_px:.2} px on {file_name}"
            );
        }
        if outer > MAX_SKY_ANNULUS_RADIUS {
            bail!(
                "sky annulus outer radius {outer:.2} px on {file_name} exceeds the limit of {MAX_SKY_ANNULUS_RADIUS} px"
            );
        }
    }
    Ok((radius_px, annulus_px))
}

fn no_celestial_wcs(file_name: &str) -> anyhow::Error {
    anyhow!("{file_name} has no celestial WCS: sky positions cannot be placed on it")
}

fn pixel_scale_of(wcs: &WcsTransform, file_name: &str) -> anyhow::Result<PixelScale> {
    pixel_scale_from_cd(wcs.raw_params().4).ok_or_else(|| anyhow!("{file_name}: the WCS has no usable pixel scale"))
}

pub(crate) fn forced_band_pixel_scale(path: &str) -> anyhow::Result<f64> {
    let file_name = file_name_of(path);
    let header = cached_header(path)?;
    let wcs = load_wcs(path, &header).map_err(|_| no_celestial_wcs(&file_name))?;
    Ok(pixel_scale_of(&wcs, &file_name)?.area_scale_arcsec)
}

fn no_data_map(err: &Array2<f32>, dq_excluded: Option<&Array2<u8>>) -> (Array2<u8>, u64) {
    let mut map = dq_excluded.cloned().unwrap_or_else(|| Array2::zeros(err.dim()));
    let mut n_no_data = 0u64;
    Zip::from(&mut map).and(err).for_each(|mark, &sigma| {
        if !(sigma.is_finite() && sigma > 0.0) {
            *mark = 1;
            n_no_data += 1;
        }
    });
    (map, n_no_data)
}

fn forced_filter(entry: &FilterEntry, source: String) -> ForcedFilter {
    ForcedFilter {
        dataset: entry.dataset.clone(),
        filter: entry.filter.clone(),
        pivot_um: entry.pivot_angstrom / ANGSTROM_PER_MICRON,
        source,
    }
}

fn band_filter(
    band: &ForcedBandInput,
    label: Option<&str>,
    header: Option<&HduHeader>,
) -> Result<(ForcedFilter, Vec<String>), FilterError> {
    let registry = filter_registry();
    match registry.identify(header, &source_path(&band.path), band.dataset.as_deref()) {
        Ok(found) => {
            let source = match &found.source {
                FilterSource::Header { keyword } => format!("header {keyword}"),
                FilterSource::FileName => FILTER_FROM_FILE_NAME.to_string(),
            };
            Ok((forced_filter(&found.entry, source), found.warnings))
        }
        Err(error) => band
            .dataset
            .as_deref()
            .zip(label)
            .and_then(|(dataset, label)| registry.entry(dataset, label))
            .map(|entry| (forced_filter(entry, FILTER_FROM_REQUEST.to_string()), Vec::new()))
            .ok_or(error),
    }
}

fn band_err_origin(origin: Option<&ErrOrigin>, n_no_data: u64) -> ForcedErrOrigin {
    match origin {
        Some(origin) => ForcedErrOrigin {
            role: origin.role.name().to_string(),
            source: origin.source.clone(),
            assumed: origin.assumed,
            n_no_data,
        },
        None => ForcedErrOrigin {
            role: ERR_ROLE_SIGMA.to_string(),
            source: ERR_LAYER_SOURCE.to_string(),
            assumed: false,
            n_no_data,
        },
    }
}

fn mag_system_name(system: MagSystem) -> &'static str {
    match system {
        MagSystem::Ab => "ab",
        MagSystem::Vega => "vega",
        MagSystem::St => "st",
        MagSystem::Unknown => "unknown",
    }
}

fn calibrate_cell(cell: &mut ForcedCell, cal: &PhotCal, options: &ForcedOptions) {
    let Some(calibrated) = cell.flux_native.and_then(|flux| cal.calibrate(flux, cell.flux_err_native)) else {
        return;
    };
    cell.flux_jy = Some(calibrated.flux_jy);
    cell.flux_err_jy = calibrated.flux_err_jy;
    cell.flux_err_corr_jy = calibrated.flux_err_jy.zip(options.f_corr).map(|(err, f_corr)| err * f_corr);
    cell.mag_ab = calibrated.mag_ab;
    cell.mag_ab_err = calibrated.mag_ab_err;
    cell.upper_limit_jy =
        upper_limit(calibrated.flux_jy, calibrated.flux_err_jy, options.f_corr, options.limit_sigma);
    cell.upper_limit_ab = cell.upper_limit_jy.and_then(ab_from_jy);
}

struct BandAperture<'a> {
    ctx: &'a PhotometryContext,
    source: &'a MaskedImage<'a>,
    err: Option<&'a Array2<f32>>,
    flags: Option<&'a Array2<u32>>,
    radius_px: f64,
    config: PhotometryConfig,
    options: &'a ForcedOptions,
}

impl BandAperture<'_> {
    fn is_saturated(&self, x: f64, y: f64, peak: f64) -> bool {
        let (rows, cols) = self.source.image().dim();
        let pixels = circular_aperture(rows, cols, x, y, self.radius_px, FORCED_SUBSAMPLES);
        saturation_verdict(self.source, self.ctx.planes.saturated.as_ref(), &pixels, peak, self.config.saturation)
            .saturated
    }

    fn plain_cell(&self, x: f64, y: f64) -> ForcedCell {
        let unmeasured = ForcedCell::unmeasured(x, y);
        let aperture = match measure_plain_aperture(self.source, self.err, x, y, self.radius_px, FORCED_SUBSAMPLES) {
            Ok(aperture) => aperture,
            Err(reason) => return ForcedCell { error: Some(reason), ..unmeasured },
        };
        let counted = ForcedCell {
            masked_fraction: Some(aperture.masked_fraction),
            n_masked: aperture.n_masked,
            n_nonfinite: aperture.n_nonfinite,
            ..unmeasured
        };
        if aperture.n_used == 0 {
            return ForcedCell { error: Some(NO_DATA_IN_APERTURE.to_string()), ..counted };
        }
        ForcedCell {
            flux_native: Some(aperture.flux),
            flux_err_native: aperture.flux_err,
            snr: aperture.flux_err.filter(|err| *err > 0.0).map(|err| aperture.flux / err),
            fwhm_px: fwhm_at_position(self.source, x, y, FORCED_SUBSAMPLES),
            saturated: self.is_saturated(x, y, aperture.peak),
            ..counted
        }
    }

    fn annulus_cell(&self, x: f64, y: f64) -> ForcedCell {
        let unmeasured = ForcedCell::unmeasured(x, y);
        let phot = match self.ctx.measure(self.source, x, y, &self.config) {
            Ok(phot) => phot,
            Err(reason) => return ForcedCell { error: Some(reason), ..unmeasured },
        };
        let (rows, cols) = self.source.image().dim();
        let coverage = aperture_coverage(rows, cols, x, y, self.radius_px, FORCED_SUBSAMPLES);
        ForcedCell {
            flux_native: Some(phot.net_flux),
            flux_err_native: Some(phot.flux_err),
            snr: Some(phot.snr),
            masked_fraction: Some(masked_fraction(phot.aperture_area, &coverage)),
            n_nonfinite: phot.n_nonfinite,
            n_masked: phot.n_masked,
            saturated: phot.saturated,
            fwhm_px: phot.fwhm,
            ..unmeasured
        }
    }

    fn cell(&self, x: f64, y: f64) -> ForcedCell {
        let mut cell = match self.config.sky_annulus {
            Some(_) => self.annulus_cell(x, y),
            None => self.plain_cell(x, y),
        };
        cell.flags_or = self.flags.map(|bits| flags_under_aperture(bits, x, y, self.radius_px, FORCED_SUBSAMPLES));
        if let Some(cal) = &self.ctx.photcal {
            calibrate_cell(&mut cell, cal, self.options);
        }
        cell
    }

    fn correction_candidate(&self, star: &DetectedStar, reference_px: f64) -> Option<(CorrectionStar, Vec<GrowthPoint>)> {
        let (x, y) = (star.x, star.y);
        let sum_within = |radius_px: f64| {
            measure_plain_aperture(self.source, self.err, x, y, radius_px, FORCED_SUBSAMPLES)
                .ok()
                .filter(|aperture| aperture.n_used > 0)
        };
        let measured = sum_within(self.radius_px)?;
        let reference = sum_within(reference_px)?;
        let sky = match self.config.sky_annulus {
            Some((inner, outer)) => sky_in_annulus(self.source, x, y, inner, outer, FORCED_SUBSAMPLES)?.mean,
            None => 0.0,
        };
        let snr = match measured.flux_err.filter(|err| *err > 0.0) {
            Some(err) => (measured.flux - sky * measured.used_area) / err,
            None => star.snr,
        };
        let candidate = CorrectionStar {
            x,
            y,
            snr,
            saturated: self.is_saturated(x, y, measured.peak),
            masked_fraction: reference.masked_fraction,
        };
        let curve = growth_at_radii(self.source, x, y, &[self.radius_px, reference_px], sky, FORCED_SUBSAMPLES);
        Some((candidate, curve))
    }

    fn growth_correction(&self, reference_px: f64, label: &str, warnings: &mut Vec<String>) -> Option<ApertureCorrection> {
        let radius_px = self.radius_px;
        let refusal = if reference_px <= radius_px {
            Some(format!(
                "the reference radius {reference_px:.2} px is not larger than the aperture radius {radius_px:.2} px"
            ))
        } else if reference_px > MAX_APERTURE_RADIUS {
            Some(format!("the reference radius {reference_px:.1} px is above the {MAX_APERTURE_RADIUS} px limit"))
        } else {
            self.config.sky_annulus.filter(|(inner, _)| *inner < reference_px).map(|(inner, _)| {
                format!("the sky annulus starts at {inner:.2} px, inside the reference radius {reference_px:.2} px")
            })
        };
        if let Some(reason) = refusal {
            warnings.push(format!("no aperture correction for {label}: {reason}"));
            return None;
        }

        let detected = detect_stars(self.source.image(), CORRECTION_DETECTION_SIGMA).stars;
        let (candidates, curves): (Vec<CorrectionStar>, Vec<Vec<GrowthPoint>>) = detected
            .iter()
            .take(MAX_CORRECTION_CANDIDATES)
            .filter_map(|star| self.correction_candidate(star, reference_px))
            .unzip();
        let selected: Vec<Vec<GrowthPoint>> = select_correction_stars(&candidates, reference_px, FORCED_STAR_MIN_SNR)
            .into_iter()
            .map(|index| curves[index].clone())
            .collect();
        let Some(correction) = aperture_correction_from_growth(&selected, radius_px, reference_px) else {
            let ratios: Vec<f64> =
                selected.iter().filter_map(|curve| growth_ratio(curve, radius_px, reference_px)).collect();
            warnings.push(format!(
                "no aperture correction for {label}: {} isolated unsaturated sources above S/N {FORCED_STAR_MIN_SNR}, {} of them on the point-source locus ({FORCED_MIN_CORRECTION_STARS} needed)",
                ratios.len(),
                point_source_locus(&ratios).len()
            ));
            return None;
        };
        if correction.factor < 1.0 {
            warnings.push(format!(
                "aperture correction of {label} is {:.4}, below 1: the reference aperture holds less flux than the measurement aperture",
                correction.factor
            ));
        }
        if correction.scatter > SCATTER_WARNING_FRACTION * (correction.factor - 1.0).abs()
            && correction.scatter > SCATTER_WARNING_FLOOR
        {
            warnings.push(format!(
                "aperture correction of {label} rests on {} sources whose growth ratio scatters by {:.3}: check that they are point sources",
                correction.n_stars, correction.scatter
            ));
        }
        Some(correction)
    }
}

pub(crate) fn forced_band(
    band: &ForcedBandInput,
    positions: &[ForcedSkyPosition],
    aperture: &ForcedAperture,
    options: &ForcedOptions,
) -> anyhow::Result<ForcedBandResult> {
    forced_aperture_checked(aperture)?;
    let path = band.path.as_str();
    let file_name = file_name_of(path);
    let ctx = photometry_context_with(path, options.exclude_dq, options.zero_point.as_ref())?;
    let wcs = ctx.wcs.as_ref().ok_or_else(|| no_celestial_wcs(&file_name))?;
    let scale = pixel_scale_of(wcs, &file_name)?;
    let (radius_px, annulus_px) = aperture_in_pixels(aperture, scale.area_scale_arcsec, &file_name)?;

    let header = ctx.entry.header();
    let companions = load_companions(path).unwrap_or_default();
    let mut warnings = ctx.warnings.clone();
    for reason in [&companions.err_unavailable, &companions.dq_unavailable].into_iter().flatten() {
        if !warnings.iter().any(|warning| warning.contains(reason.as_str())) {
            warnings.push(reason.clone());
        }
    }
    if scale.anisotropy() > FORCED_AXIS_SCALE_TOLERANCE {
        warnings.push(format!(
            "pixel scales differ per axis ({:.5} x {:.5} arcsec); radii in arcsec use the square root of the pixel area ({:.5} arcsec)",
            scale.x_arcsec, scale.y_arcsec, scale.area_scale_arcsec
        ));
    }
    let named = band.label.clone().or_else(|| header.and_then(loaded_band_plane).and_then(|plane| plane.label));
    let (filter, filter_error) = match band_filter(band, named.as_deref(), header) {
        Ok((filter, notes)) => {
            warnings.extend(notes);
            (Some(filter), None)
        }
        Err(error) => (None, Some(error)),
    };
    let label =
        named.or_else(|| filter.as_ref().map(|found| found.filter.clone())).unwrap_or_else(|| file_name.clone());
    if let Some(error) = filter_error {
        warnings.push(format!(
            "filter of {label} not identified ({error}): the band has no pivot wavelength and stays off the SED"
        ));
    }

    let (rows, cols) = ctx.entry.arr().dim();
    let err = ctx.planes.err.as_ref().map(|plane| plane.arr());
    let dq_excluded = ctx.mask.as_ref().map(|mask| &mask.map);
    let no_data = err.map(|err| no_data_map(err, dq_excluded));
    let excluded = no_data.as_ref().map(|(map, _)| map).or(dq_excluded);
    let source = ctx.area.masked_image(&ctx.entry, excluded).map_err(anyhow::Error::msg)?;
    let flags = companions
        .dq
        .as_ref()
        .and_then(|(layer, _)| layer.int_plane())
        .map(|plane| &plane.bits)
        .filter(|bits| bits.dim() == (rows, cols));
    let flag_names = companions.flags.as_ref().map_or_else(Vec::new, |set| {
        set.flags.iter().map(|(value, name)| ForcedFlagName { value: *value, name: name.clone() }).collect()
    });

    let sky_annulus = annulus_px.map(|[inner, outer]| (inner, outer));
    let measurement = BandAperture {
        ctx: &ctx,
        source: &source,
        err,
        flags,
        radius_px,
        config: PhotometryConfig { fixed_position: true, ..ctx.config(Some(radius_px), sky_annulus, options.gain) },
        options,
    };
    let cell_at = |position: &ForcedSkyPosition| {
        let (x, y) = wcs.world_to_pixel(position.ra_deg, position.dec_deg);
        let on_image = x >= -0.5 && x < cols as f64 - 0.5 && y >= -0.5 && y < rows as f64 - 0.5;
        if on_image {
            measurement.cell(x, y)
        } else {
            ForcedCell::off_image(x, y)
        }
    };
    let mut cells: Vec<ForcedCell> = if positions.len() > PAR_BATCH_POINTS {
        positions.par_iter().map(cell_at).collect()
    } else {
        positions.iter().map(cell_at).collect()
    };

    if err.is_none() && annulus_px.is_none() {
        warnings.push(format!("no error layer and no sky annulus: the fluxes of {label} have no error"));
    }
    let n_on_image = cells.iter().filter(|cell| cell.on_image).count();
    let n_incomplete = cells.iter().filter(|cell| cell.masked_fraction.is_some_and(|fraction| fraction > 0.0)).count();
    if n_incomplete > 0 {
        warnings.push(format!(
            "{n_incomplete} of {n_on_image} apertures of {label} hold pixels without data: their flux is a lower limit"
        ));
    }

    let aperture_correction = match aperture.correction {
        ForcedCorrection::GrowthCurve => {
            let reference_arcsec = aperture.reference_radius_arcsec.unwrap_or(FORCED_DEFAULT_REFERENCE_RADIUS_ARCSEC);
            measurement.growth_correction(reference_arcsec / scale.area_scale_arcsec, &label, &mut warnings)
        }
        ForcedCorrection::None => None,
    };
    if let Some(correction) = &aperture_correction {
        for cell in &mut cells {
            cell.flux_total_jy = cell.flux_jy.map(|flux_jy| flux_jy * correction.factor);
        }
    }

    Ok(ForcedBandResult {
        band: ForcedBand {
            index: 0,
            path: band.path.clone(),
            file_name,
            label,
            error: None,
            filter,
            photcal: ctx.photcal_json()?,
            calibration_origin: ctx.photcal.as_ref().map(|cal| cal.origin.clone()),
            jy_per_native_unit: ctx.photcal.as_ref().and_then(PhotCal::jy_per_native_unit),
            bunit: header.and_then(|header| card_text(header, HEADER_BUNIT)),
            pixel_scale_arcsec: Some(scale.area_scale_arcsec),
            aperture_radius_px: Some(radius_px),
            annulus_px,
            aperture_correction,
            err_origin: no_data
                .as_ref()
                .map(|(_, n_no_data)| band_err_origin(companions.err_origin.as_ref(), *n_no_data)),
            dq_source: companions.dq_origin.clone(),
            f_corr: options.f_corr,
            n_measured: cells.iter().filter(|cell| cell.flux_native.is_some()).count(),
            n_failed: cells.iter().filter(|cell| cell.on_image && cell.error.is_some()).count(),
            warnings,
            flag_names,
            mag_system: ctx.photcal.as_ref().map(|cal| mag_system_name(cal.mag_system).to_string()),
        },
        cells,
    })
}

pub const FORCED_PROGRESS_EVENT: &str = "forced-photometry-progress";
pub const FORCED_MAX_BANDS: usize = 12;
pub const FORCED_MAX_POSITIONS: usize = 5000;
const PIXEL_RADIUS_SCALE_TOLERANCE: f64 = 0.01;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForcedRunOptions {
    #[serde(default)]
    pub exclude_dq: Option<bool>,
    #[serde(default)]
    pub gain: Option<f64>,
    #[serde(default)]
    pub limit_sigma: Option<f64>,
    #[serde(default)]
    pub f_corr: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub zero_points: Option<Vec<Option<UserZeroPoint>>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForcedSource {
    pub id: String,
    pub ra_deg: f64,
    pub dec_deg: f64,
    pub cells: Vec<Option<ForcedCell>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForcedPhotometryResult {
    pub bands: Vec<ForcedBand>,
    pub sources: Vec<ForcedSource>,
    pub aperture: ForcedAperture,
    pub limit_sigma: f64,
    pub elapsed_ms: u64,
    pub warnings: Vec<String>,
}

fn forced_request_checked(
    bands: &[ForcedBandInput],
    positions: &[ForcedSkyPosition],
    aperture: &ForcedAperture,
    options: &ForcedRunOptions,
) -> anyhow::Result<()> {
    match bands.len() {
        0 => bail!("forced photometry needs at least one band, got 0"),
        n if n > FORCED_MAX_BANDS => bail!("forced photometry of {n} bands exceeds the limit of {FORCED_MAX_BANDS}"),
        _ => {}
    }
    match positions.len() {
        0 => bail!("forced photometry needs at least one position, got 0"),
        n if n > FORCED_MAX_POSITIONS => {
            bail!("forced photometry of {n} positions exceeds the limit of {FORCED_MAX_POSITIONS}")
        }
        _ => {}
    }
    let mut paths = HashSet::new();
    if let Some(band) = bands.iter().find(|band| !paths.insert(band.path.as_str())) {
        bail!("band {} is listed twice", band.path);
    }
    let mut ids = HashSet::new();
    for (index, position) in positions.iter().enumerate() {
        let ForcedSkyPosition { id, ra_deg, dec_deg } = position;
        if id.is_empty() {
            bail!("position {index} has an empty id");
        }
        if !ids.insert(id.as_str()) {
            bail!("position id '{id}' is used twice");
        }
        if !((0.0..=360.0).contains(ra_deg) && (-90.0..=90.0).contains(dec_deg)) {
            bail!("position '{id}' ({ra_deg}, {dec_deg}) must have RA in 0 to 360 and Dec in -90 to 90 degrees");
        }
    }
    forced_aperture_checked(aperture)?;
    if let Some(limit_sigma) = options.limit_sigma.filter(|value| !is_positive(*value)) {
        bail!("limit_sigma {limit_sigma} must be a positive finite number");
    }
    if let Some(gain) = options.gain.filter(|value| !is_positive(*value)) {
        bail!("gain {gain} must be a positive finite number of e-/ADU");
    }
    if let Some(factors) = &options.f_corr {
        if factors.len() != bands.len() {
            bail!("f_corr has {} values for {} bands", factors.len(), bands.len());
        }
        if let Some((k, factor)) =
            factors.iter().enumerate().find_map(|(k, factor)| factor.filter(|value| !is_positive(*value)).map(|value| (k, value)))
        {
            bail!("f_corr of band {k} is {factor}: it must be a positive finite number");
        }
    }
    if let Some(typed) = options.zero_points.as_ref().filter(|typed| typed.len() != bands.len()) {
        bail!("zero_points has {} values for {} bands", typed.len(), bands.len());
    }
    Ok(())
}

fn pixel_radius_scales_checked(bands: &[ForcedBandInput], aperture: &ForcedAperture) -> anyhow::Result<()> {
    if aperture.radius_px.is_none() {
        return Ok(());
    }
    let scales: Vec<(String, f64)> = bands
        .iter()
        .filter_map(|band| forced_band_pixel_scale(&band.path).ok().map(|scale| (file_name_of(&band.path), scale)))
        .collect();
    for (i, (file_a, a)) in scales.iter().enumerate() {
        for (file_b, b) in &scales[i + 1..] {
            if (a - b).abs() > PIXEL_RADIUS_SCALE_TOLERANCE * a.max(*b) {
                bail!(
                    "an aperture radius in pixels needs bands of one pixel scale: {file_a} has {a:.4} and {file_b} has {b:.4} arcsec/px; give the radius in arcsec"
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn forced_photometry(
    bands: &[ForcedBandInput],
    positions: &[ForcedSkyPosition],
    aperture: &ForcedAperture,
    options: &ForcedRunOptions,
    progress: Option<&ProgressHandle>,
) -> anyhow::Result<ForcedPhotometryResult> {
    let started = Instant::now();
    forced_request_checked(bands, positions, aperture, options)?;
    pixel_radius_scales_checked(bands, aperture)?;

    let limit_sigma = options.limit_sigma.unwrap_or(FORCED_DEFAULT_LIMIT_SIGMA);
    let mut sources: Vec<ForcedSource> = positions
        .iter()
        .map(|position| ForcedSource {
            id: position.id.clone(),
            ra_deg: position.ra_deg,
            dec_deg: position.dec_deg,
            cells: vec![None; bands.len()],
        })
        .collect();
    let mut band_entries = Vec::with_capacity(bands.len());
    let mut first_error = None;
    for (k, band) in bands.iter().enumerate() {
        if progress.is_some_and(|handle| handle.is_cancelled()) {
            return Err(AppError::Cancelled.into());
        }
        let file_name = file_name_of(&band.path);
        let band_options = ForcedOptions {
            exclude_dq: options.exclude_dq.unwrap_or(false),
            gain: options.gain,
            limit_sigma,
            f_corr: options.f_corr.as_ref().and_then(|factors| factors[k]),
            zero_point: options.zero_points.as_ref().and_then(|typed| typed[k].clone()),
        };
        match forced_band(band, positions, aperture, &band_options) {
            Ok(ForcedBandResult { band: measured, cells }) => {
                for (source, cell) in sources.iter_mut().zip(cells) {
                    source.cells[k] = Some(cell);
                }
                band_entries.push(ForcedBand { index: k, ..measured });
            }
            Err(error) => {
                let error = format!("{error:#}");
                first_error.get_or_insert_with(|| error.clone());
                band_entries.push(ForcedBand {
                    index: k,
                    path: band.path.clone(),
                    label: band.label.clone().unwrap_or_else(|| file_name.clone()),
                    file_name: file_name.clone(),
                    error: Some(error),
                    filter: None,
                    photcal: serde_json::Value::Null,
                    calibration_origin: None,
                    jy_per_native_unit: None,
                    bunit: None,
                    pixel_scale_arcsec: None,
                    aperture_radius_px: None,
                    annulus_px: None,
                    aperture_correction: None,
                    err_origin: None,
                    dq_source: None,
                    f_corr: None,
                    n_measured: 0,
                    n_failed: 0,
                    warnings: Vec::new(),
                    flag_names: Vec::new(),
                    mag_system: None,
                });
            }
        }
        if let Some(handle) = progress {
            handle.tick_with_stage(&file_name);
        }
    }

    let n_bands = band_entries.len();
    let measured: Vec<&ForcedBand> = band_entries.iter().filter(|band| band.error.is_none()).collect();
    if measured.is_empty() {
        bail!("no band could be measured: {}", first_error.unwrap_or_default());
    }
    let mut warnings = Vec::new();
    let n_failed = n_bands - measured.len();
    if n_failed > 0 {
        warnings.push(format!("{n_failed} of {n_bands} bands could not be measured"));
    }
    let n_uncalibrated = measured.iter().filter(|band| band.jy_per_native_unit.is_none()).count();
    if n_uncalibrated > 0 {
        warnings.push(format!(
            "{n_uncalibrated} of {n_bands} bands have no calibration to Jy: they stay out of colours and of the SED"
        ));
    }
    let n_unidentified = measured.iter().filter(|band| band.filter.is_none()).count();
    if n_unidentified > 0 {
        warnings.push(format!("{n_unidentified} of {n_bands} bands have no identified filter: they stay off the SED"));
    }

    let mut used_aperture = aperture.clone();
    if aperture.correction == ForcedCorrection::GrowthCurve && aperture.reference_radius_arcsec.is_none() {
        used_aperture.reference_radius_arcsec = Some(FORCED_DEFAULT_REFERENCE_RADIUS_ARCSEC);
    }
    Ok(ForcedPhotometryResult {
        bands: band_entries,
        sources,
        aperture: used_aperture,
        limit_sigma,
        elapsed_ms: started.elapsed().as_millis() as u64,
        warnings,
    })
}

#[tauri::command]
pub async fn forced_photometry_cmd(
    app: tauri::AppHandle,
    bands: Vec<ForcedBandInput>,
    positions: Vec<ForcedSkyPosition>,
    aperture: ForcedAperture,
    options: Option<ForcedRunOptions>,
) -> Result<serde_json::Value, String> {
    let options = options.unwrap_or_default();
    let progress = ProgressHandle::new(&app, FORCED_PROGRESS_EVENT, bands.len() as u64);
    blocking_cmd!({
        let result = forced_photometry(&bands, &positions, &aperture, &options, Some(&progress))?;
        progress.emit_complete();
        Ok(serde_json::to_value(result)?)
    })
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;
    use crate::cmd::analysis::photometry_context;
    use crate::core::analysis::aperture::total_weight;
    use crate::infra::fits::reader::test_fixtures::{write_raw_hdus, write_test_mef, HduData, TestHdu};
    use crate::infra::fits::writer::write_fits_mono;
    use crate::infra::progress::{ProgressPayload, ProgressSink};

    type Source = (f64, f64, f64, f64);

    const BAND_SIZE: usize = 128;
    const BAND_PIXELS: usize = BAND_SIZE * BAND_SIZE;
    const CUBE_SIZE: usize = 64;
    const FIELD_SIZE: usize = 256;
    const TOTAL_FLUX: f64 = 10000.0;
    const ENCLOSED_FLUX: f64 = 9961.340798605272;
    const JY_PER_NANOMAGGY: f64 = 3.630780547701e-6;
    const CLEAN_DQ: i32 = 0i32 - 2147483647 - 1;
    const STAR_A: Source = (80.0, 70.0, TOTAL_FLUX / (2.0 * PI * 9.0), 3.0);
    const STAR_B: Source = (71.75, 66.75, TOTAL_FLUX / (2.0 * PI * 2.25), 1.5);
    const FIELD_STARS: [(f64, f64); 6] =
        [(48.0, 52.0), (120.3, 60.6), (200.7, 55.2), (60.4, 150.9), (140.5, 200.5), (205.2, 190.1)];
    const FIELD_GALAXY: Source = (125.0, 130.0, 3000.0, 4.0);
    const MIRROR_FACTOR: f64 = 1.5057;
    const CONTINUOUS_FACTOR: f64 = 1.4802223878154521;

    fn scene(size: usize, sources: &[Source], background: impl Fn(usize, usize) -> f64) -> Vec<f32> {
        (0..size * size)
            .map(|i| {
                let (x, y) = (i % size, i / size);
                let light: f64 = sources
                    .iter()
                    .map(|&(x0, y0, amplitude, sigma)| {
                        let r2 = (x as f64 - x0).powi(2) + (y as f64 - y0).powi(2);
                        amplitude * (-r2 / (2.0 * sigma * sigma)).exp()
                    })
                    .sum();
                (background(x, y) + light) as f32
            })
            .collect()
    }

    fn wcs_cards(size: usize, scale_arcsec: (f64, f64)) -> Vec<(&'static str, String)> {
        let reference_pixel = size as f64 / 2.0 + 0.5;
        vec![
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
            ("CRPIX1", format!("{:E}", reference_pixel)),
            ("CRPIX2", format!("{:E}", reference_pixel)),
            ("CRVAL1", format!("{:E}", 150.0)),
            ("CRVAL2", format!("{:E}", 2.0)),
            ("CD1_1", format!("{:E}", -scale_arcsec.0 / 3600.0)),
            ("CD1_2", format!("{:E}", 0.0)),
            ("CD2_1", format!("{:E}", 0.0)),
            ("CD2_2", format!("{:E}", scale_arcsec.1 / 3600.0)),
        ]
    }

    fn write_layers(
        dir: &Path,
        name: &str,
        size: usize,
        scale_arcsec: (f64, f64),
        sci: Vec<f32>,
        err: Option<Vec<f32>>,
        dq: Option<Vec<i32>>,
        extra: Vec<(&'static str, String)>,
    ) -> String {
        let mut cards = wcs_cards(size, scale_arcsec);
        cards.extend(extra);
        let layer = |extname: &'static str, data: HduData, extra_cards: Vec<(&'static str, String)>| TestHdu {
            extname: Some(extname),
            extver: Some(1),
            cols: size,
            rows: size,
            data,
            extra_cards,
        };
        let mut hdus = vec![layer("SCI", HduData::F32(sci), cards)];
        if let Some(err) = err {
            hdus.push(layer("ERR", HduData::F32(err), vec![]));
        }
        if let Some(dq) = dq {
            hdus.push(layer("DQ", HduData::I32(dq), vec![("BZERO", "2147483648".into()), ("BSCALE", "1".into())]));
        }
        let path = dir.join(name);
        write_test_mef(&path, &[], &hdus);
        format!("{}#hdu=1", path.to_str().unwrap())
    }

    fn write_band(
        dir: &Path,
        name: &str,
        scale_arcsec: f64,
        stars: &[Source],
        bg: f64,
        err: Option<f32>,
        extra: Vec<(&'static str, String)>,
    ) -> String {
        write_layers(
            dir,
            name,
            BAND_SIZE,
            (scale_arcsec, scale_arcsec),
            scene(BAND_SIZE, stars, |_, _| bg),
            err.map(|sigma| vec![sigma; BAND_SIZE * BAND_SIZE]),
            None,
            extra,
        )
    }

    fn write_field(dir: &Path, name: &str, sources: &[Source], extra: Vec<(&'static str, String)>) -> String {
        let sky = |x: usize, y: usize| 100.0 + ((7 * y + 13 * x) % 5) as f64 - 2.0;
        write_layers(
            dir,
            name,
            FIELD_SIZE,
            (0.25, 0.25),
            scene(FIELD_SIZE, sources, sky),
            Some(vec![2.0; FIELD_SIZE * FIELD_SIZE]),
            None,
            extra,
        )
    }

    fn field_stars(positions: &[(f64, f64)]) -> Vec<Source> {
        positions.iter().map(|&(x, y)| (x, y, 3000.0, 2.0)).collect()
    }

    fn nanomaggy_cards() -> Vec<(&'static str, String)> {
        vec![("BUNIT", "'nanomaggy'".into()), ("MAGZERO", "22.5".into())]
    }

    fn sky_of(key: &str, x: f64, y: f64) -> ForcedSkyPosition {
        let coord = photometry_context(key, false).unwrap().wcs.unwrap().pixel_to_world(x, y);
        ForcedSkyPosition { id: format!("{x},{y}"), ra_deg: coord.ra, dec_deg: coord.dec }
    }

    fn band_input(key: &str) -> ForcedBandInput {
        ForcedBandInput { path: key.to_string(), label: None, dataset: None }
    }

    fn plain(radius_arcsec: f64) -> ForcedAperture {
        ForcedAperture {
            radius_arcsec: Some(radius_arcsec),
            radius_px: None,
            annulus_arcsec: None,
            annulus_px: None,
            correction: ForcedCorrection::None,
            reference_radius_arcsec: None,
        }
    }

    fn growth(reference_radius_arcsec: f64) -> ForcedAperture {
        ForcedAperture {
            radius_arcsec: None,
            radius_px: Some(3.0),
            annulus_arcsec: None,
            annulus_px: Some([20.0, 26.0]),
            correction: ForcedCorrection::GrowthCurve,
            reference_radius_arcsec: Some(reference_radius_arcsec),
        }
    }

    fn measured(key: &str, positions: &[ForcedSkyPosition], aperture: &ForcedAperture) -> ForcedBandResult {
        forced_band(&band_input(key), positions, aperture, &ForcedOptions::default()).unwrap()
    }

    fn refusal(key: &str, aperture: &ForcedAperture) -> String {
        forced_band(&band_input(key), &[], aperture, &ForcedOptions::default()).unwrap_err().to_string()
    }

    fn relative_difference(value: f64, expected: f64) -> f64 {
        ((value - expected) / expected).abs()
    }

    #[test]
    fn one_sky_position_lands_on_each_band_through_its_own_wcs() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_band(dir.path(), "a.fits", 0.2, &[STAR_A], 0.0, None, vec![]);
        let b = write_band(dir.path(), "b.fits", 0.4, &[STAR_B], 0.0, None, vec![]);
        let position = sky_of(&a, 80.0, 70.0);
        let on_a = measured(&a, &[position.clone()], &plain(2.0));
        let on_b = measured(&b, &[position], &plain(2.0));

        assert!((on_a.cells[0].x - 80.0).abs() < 1e-6 && (on_a.cells[0].y - 70.0).abs() < 1e-6, "{:?}", on_a.cells[0]);
        assert!((on_b.cells[0].x - 71.75).abs() < 1e-4 && (on_b.cells[0].y - 66.75).abs() < 1e-4, "{:?}", on_b.cells[0]);
        assert!(on_a.cells[0].on_image && on_b.cells[0].on_image);
        assert!((on_a.band.aperture_radius_px.unwrap() - 10.0).abs() < 1e-9, "{:?}", on_a.band.aperture_radius_px);
        assert!((on_b.band.aperture_radius_px.unwrap() - 5.0).abs() < 1e-9, "{:?}", on_b.band.aperture_radius_px);
        assert!((on_a.band.pixel_scale_arcsec.unwrap() - 0.2).abs() < 1e-9, "{:?}", on_a.band.pixel_scale_arcsec);
        assert!((on_b.band.pixel_scale_arcsec.unwrap() - 0.4).abs() < 1e-9, "{:?}", on_b.band.pixel_scale_arcsec);
        assert_eq!(on_a.band.file_name, "a.fits");
        assert_eq!(on_a.band.path, a);
        assert_eq!(on_a.band.index, 0);
        assert!(on_a.band.error.is_none());
    }

    #[test]
    fn the_same_aperture_in_arcsec_encloses_the_same_fraction_in_both_bands() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_band(dir.path(), "a.fits", 0.2, &[STAR_A], 0.0, None, vec![]);
        let b = write_band(dir.path(), "b.fits", 0.4, &[STAR_B], 0.0, None, vec![]);
        let position = sky_of(&a, 80.0, 70.0);
        let flux_a = measured(&a, &[position.clone()], &plain(2.0)).cells[0].flux_native.unwrap();
        let flux_b = measured(&b, &[position], &plain(2.0)).cells[0].flux_native.unwrap();

        assert!(relative_difference(flux_a, ENCLOSED_FLUX) < 0.01, "flux_a={flux_a}");
        assert!(relative_difference(flux_b, ENCLOSED_FLUX) < 0.01, "flux_b={flux_b}");
        assert!(relative_difference(flux_a, flux_b) < 0.01, "flux_a={flux_a} flux_b={flux_b}");
    }

    #[test]
    fn a_brighter_neighbour_does_not_move_a_forced_position() {
        let dir = tempfile::tempdir().unwrap();
        let neighbour: Source = (84.0, 70.0, 2.0 * STAR_A.2, 3.0);
        let a = write_band(dir.path(), "a.fits", 0.2, &[STAR_A, neighbour], 0.0, None, vec![]);
        let result = measured(&a, &[sky_of(&a, 80.0, 70.0)], &plain(2.0));
        let cell = &result.cells[0];

        assert!((cell.x - 80.0).abs() < 1e-6 && (cell.y - 70.0).abs() < 1e-6, "{cell:?}");
        assert!(cell.flux_native.is_some());
        assert_eq!(result.band.n_measured, 1);
    }

    #[test]
    fn without_an_annulus_the_pedestal_stays_and_with_one_it_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_band(dir.path(), "a.fits", 0.2, &[STAR_A], 5.0, None, vec![]);
        let position = [sky_of(&a, 80.0, 70.0)];
        let with_pedestal = measured(&a, &position, &plain(2.0));
        let annulus = ForcedAperture { annulus_arcsec: Some([4.0, 6.0]), ..plain(2.0) };
        let sky_removed = measured(&a, &position, &annulus);

        let pedestal_flux = with_pedestal.cells[0].flux_native.unwrap();
        let net_flux = sky_removed.cells[0].flux_native.unwrap();
        assert!(relative_difference(pedestal_flux, ENCLOSED_FLUX + 5.0 * 100.0 * PI) < 0.02, "flux={pedestal_flux}");
        assert!(relative_difference(net_flux, ENCLOSED_FLUX) < 0.015, "flux={net_flux}");
        assert_eq!(with_pedestal.band.annulus_px, None);
        let [inner, outer] = sky_removed.band.annulus_px.unwrap();
        assert!((inner - 20.0).abs() < 1e-9 && (outer - 30.0).abs() < 1e-9, "[{inner}, {outer}]");
        assert!(sky_removed.cells[0].flux_err_native.is_some());
        assert_eq!(sky_removed.cells[0].masked_fraction, Some(0.0));
    }

    #[test]
    fn the_formal_error_is_the_error_layer_over_the_aperture_weights() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_band(dir.path(), "a.fits", 0.2, &[STAR_A], 0.0, Some(0.5), vec![]);
        let result = measured(&a, &[sky_of(&a, 80.0, 70.0)], &plain(2.0));
        let cell = &result.cells[0];

        let weights = circular_aperture(BAND_SIZE, BAND_SIZE, cell.x, cell.y, 10.0, 5);
        let expected = 0.5 * weights.iter().map(|p| (p.weight as f64).powi(2)).sum::<f64>().sqrt();
        let flux = cell.flux_native.unwrap();
        let flux_err = cell.flux_err_native.unwrap();
        assert!(relative_difference(flux_err, expected) < 1e-9, "flux_err={flux_err} expected={expected}");
        assert_eq!(cell.snr, Some(flux / flux_err));
        let origin = result.band.err_origin.unwrap();
        assert_eq!(
            (origin.role.as_str(), origin.source.as_str(), origin.assumed, origin.n_no_data),
            ("sigma", "EXTNAME ERR, HDU 2: as is", false, 0)
        );
    }

    #[test]
    fn a_band_without_an_error_layer_or_an_annulus_has_no_error_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_band(dir.path(), "a.fits", 0.2, &[STAR_A], 0.0, None, vec![]);
        let result = measured(&a, &[sky_of(&a, 80.0, 70.0)], &plain(2.0));
        let cell = &result.cells[0];

        assert!(cell.flux_native.is_some());
        assert_eq!(cell.flux_err_native, None);
        assert_eq!(cell.snr, None);
        assert_eq!(result.band.label, "a.fits");
        assert!(
            result
                .band
                .warnings
                .contains(&"no error layer and no sky annulus: the fluxes of a.fits have no error".to_string()),
            "{:?}",
            result.band.warnings
        );
        assert!(result.band.err_origin.is_none());
        assert!(result.band.dq_source.is_none());
    }

    #[test]
    fn fluxes_are_calibrated_to_jy_and_ab_and_a_flux_below_three_sigma_becomes_a_limit() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_band(dir.path(), "a.fits", 0.2, &[STAR_A], 0.0, Some(0.5), nanomaggy_cards());
        let positions = [sky_of(&a, 80.0, 70.0), sky_of(&a, 40.0, 70.0)];
        let result = measured(&a, &positions, &plain(2.0));

        let star = &result.cells[0];
        let flux_native = star.flux_native.unwrap();
        assert!(relative_difference(star.flux_jy.unwrap(), flux_native * JY_PER_NANOMAGGY) < 1e-8, "{star:?}");
        assert!((star.mag_ab.unwrap() - (22.5 - 2.5 * flux_native.log10())).abs() < 1e-6, "{star:?}");
        assert!(star.mag_ab_err.is_some() && star.flux_err_jy.is_some());
        assert_eq!(star.upper_limit_jy, None);
        assert_eq!(star.upper_limit_ab, None);
        assert_eq!(star.flux_err_corr_jy, None);
        assert_eq!(star.flux_total_jy, None);
        assert!(relative_difference(result.band.jy_per_native_unit.unwrap(), JY_PER_NANOMAGGY) < 1e-8);
        assert_eq!(result.band.bunit.as_deref(), Some("nanomaggy"));
        assert!(result.band.calibration_origin.is_some());
        assert!(result.band.photcal["label"].is_string(), "{}", result.band.photcal);

        let empty_sky = &result.cells[1];
        let flux_err_jy = empty_sky.flux_err_jy.unwrap();
        let limit_jy = empty_sky.upper_limit_jy.unwrap();
        assert!(empty_sky.flux_native.unwrap().abs() < 1e-6, "{empty_sky:?}");
        assert!(empty_sky.flux_jy.is_some());
        assert!(relative_difference(limit_jy, 3.0 * flux_err_jy) < 1e-12, "{empty_sky:?}");
        assert!((empty_sky.upper_limit_ab.unwrap() - (-2.5 * limit_jy.log10() + 8.90)).abs() < 1e-9, "{empty_sky:?}");

        let correlated = ForcedOptions { f_corr: Some(1.5), ..ForcedOptions::default() };
        let corrected = forced_band(&band_input(&a), &positions, &plain(2.0), &correlated).unwrap();
        let empty_sky = &corrected.cells[1];
        let flux_err_jy = empty_sky.flux_err_jy.unwrap();
        assert!(relative_difference(empty_sky.flux_err_corr_jy.unwrap(), 1.5 * flux_err_jy) < 1e-12, "{empty_sky:?}");
        assert!(relative_difference(empty_sky.upper_limit_jy.unwrap(), 4.5 * flux_err_jy) < 1e-12, "{empty_sky:?}");
        assert_eq!(corrected.band.f_corr, Some(1.5));
    }

    #[test]
    fn an_uncalibrated_band_keeps_native_fluxes_only() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_band(dir.path(), "a.fits", 0.2, &[STAR_A], 0.0, Some(0.5), vec![]);
        let result = measured(&a, &[sky_of(&a, 80.0, 70.0)], &plain(2.0));
        let cell = &result.cells[0];

        assert!(cell.flux_native.is_some() && cell.flux_err_native.is_some());
        assert_eq!((cell.flux_jy, cell.flux_err_jy, cell.flux_err_corr_jy), (None, None, None));
        assert_eq!((cell.mag_ab, cell.mag_ab_err), (None, None));
        assert_eq!((cell.upper_limit_jy, cell.upper_limit_ab), (None, None));
        assert!(result.band.photcal.is_null());
        assert!(result.band.calibration_origin.is_none());
        assert!(result.band.jy_per_native_unit.is_none());
        assert!(result.band.bunit.is_none());
        assert!(!result.band.warnings.is_empty());
    }

    #[test]
    fn a_position_off_the_image_is_a_cell_without_values() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_band(dir.path(), "a.fits", 0.2, &[STAR_A], 0.0, Some(0.5), vec![]);
        let star = sky_of(&a, 80.0, 70.0);
        let north = ForcedSkyPosition { id: "north".into(), ra_deg: star.ra_deg, dec_deg: star.dec_deg + 1.0 };
        let result = measured(&a, &[star, north], &plain(2.0));
        let cell = &result.cells[1];

        assert!(!cell.on_image);
        assert!(cell.error.is_none());
        assert!(cell.flux_native.is_none() && cell.flux_err_native.is_none() && cell.snr.is_none());
        assert!(cell.masked_fraction.is_none() && cell.flags_or.is_none() && cell.fwhm_px.is_none());
        assert_eq!((cell.n_masked, cell.n_nonfinite, cell.saturated), (0, 0, false));
        assert_eq!(result.band.n_measured, 1);
        assert_eq!(result.band.n_failed, 0);
    }

    #[test]
    fn refusals_name_the_file_and_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_band(dir.path(), "a.fits", 0.2, &[STAR_A], 0.0, None, vec![]);
        let b = write_band(dir.path(), "b.fits", 0.4, &[STAR_B], 0.0, None, vec![]);
        let bare = dir.path().join("bare.fits");
        let bare = bare.to_str().unwrap();
        write_fits_mono(bare, &Array2::from_elem((32, 32), 1.0f32), None).unwrap();
        let in_pixels = |radius_px: f64| ForcedAperture { radius_arcsec: None, radius_px: Some(radius_px), ..plain(2.0) };

        let no_wcs = "bare.fits has no celestial WCS: sky positions cannot be placed on it";
        assert_eq!(refusal(bare, &plain(2.0)), no_wcs);
        assert_eq!(forced_band_pixel_scale(bare).unwrap_err().to_string(), no_wcs);
        assert_eq!(
            refusal(&a, &plain(13.0)),
            "aperture radius 13 arcsec is 65.00 px on a.fits (0.2000 arcsec/px): the limits are 2 to 60 px"
        );
        assert_eq!(
            refusal(&b, &plain(0.3)),
            "aperture radius 0.3 arcsec is 0.75 px on b.fits (0.4000 arcsec/px): the limits are 2 to 60 px"
        );
        assert_eq!(refusal(&a, &in_pixels(61.0)), "aperture radius 61 px on a.fits: the limits are 2 to 60 px");
        assert!(forced_band(&band_input(&a), &[], &in_pixels(2.0), &ForcedOptions::default()).is_ok());
        assert!(forced_band(&band_input(&a), &[], &in_pixels(60.0), &ForcedOptions::default()).is_ok());
        assert_eq!(
            refusal(&a, &ForcedAperture { radius_px: Some(10.0), ..plain(2.0) }),
            "give the aperture radius either in arcsec or in pixels, not both"
        );
        assert_eq!(
            refusal(&a, &ForcedAperture { radius_arcsec: None, ..plain(2.0) }),
            "the aperture needs a radius in arcsec or in pixels"
        );
        assert_eq!(refusal(&a, &plain(-1.0)), "aperture radius -1 must be a positive finite number");
        assert_eq!(refusal(&a, &in_pixels(f64::NAN)), "aperture radius NaN must be a positive finite number");
        assert_eq!(
            refusal(&a, &ForcedAperture { annulus_arcsec: Some([4.0, 6.0]), annulus_px: Some([20.0, 30.0]), ..plain(2.0) }),
            "give the sky annulus either in arcsec or in pixels, not both"
        );
        assert_eq!(
            refusal(&a, &ForcedAperture { annulus_arcsec: Some([6.0, 4.0]), ..plain(2.0) }),
            "sky annulus [6, 4] needs two positive finite radii with the outer one larger"
        );
        assert_eq!(
            refusal(&a, &ForcedAperture { annulus_arcsec: Some([1.0, 6.0]), ..plain(2.0) }),
            "sky annulus inner radius 5.00 px must be larger than the aperture radius 10.00 px on a.fits"
        );
        assert_eq!(
            refusal(&a, &ForcedAperture { annulus_px: Some([20.0, 600.0]), ..plain(2.0) }),
            "sky annulus outer radius 600.00 px on a.fits exceeds the limit of 512 px"
        );
        assert_eq!(
            refusal(&a, &ForcedAperture { reference_radius_arcsec: Some(-2.0), ..plain(2.0) }),
            "reference radius -2 arcsec must be a positive finite number"
        );
        assert_eq!(
            forced_aperture_checked(&ForcedAperture { radius_arcsec: None, ..plain(2.0) }).unwrap_err().to_string(),
            "the aperture needs a radius in arcsec or in pixels"
        );
        assert!(forced_aperture_checked(&plain(2.0)).is_ok());
    }

    #[test]
    fn flag_bits_under_the_aperture_are_reported_and_excluded_pixels_are_counted() {
        let dir = tempfile::tempdir().unwrap();
        let sci = scene(BAND_SIZE, &[STAR_A], |_, _| 0.0);
        let jump_pixel = 70 * BAND_SIZE + 82;
        let bad_pixel = 73 * BAND_SIZE + 80;
        let bad_value = sci[bad_pixel] as f64;
        let mut dq = vec![CLEAN_DQ; BAND_SIZE * BAND_SIZE];
        dq[jump_pixel] = 4 - 2147483647 - 1;
        dq[bad_pixel] = 1 - 2147483647 - 1;
        let a = write_layers(
            dir.path(),
            "a.fits",
            BAND_SIZE,
            (0.2, 0.2),
            sci,
            Some(vec![0.5; BAND_SIZE * BAND_SIZE]),
            Some(dq),
            vec![("TELESCOP", "'JWST'".into())],
        );
        let position = [sky_of(&a, 80.0, 70.0)];
        let kept = measured(&a, &position, &plain(2.0));
        let excluding = ForcedOptions { exclude_dq: true, ..ForcedOptions::default() };
        let excluded = forced_band(&band_input(&a), &position, &plain(2.0), &excluding).unwrap();

        let kept_cell = &kept.cells[0];
        assert_eq!(kept_cell.flags_or, Some(5));
        assert_eq!(kept_cell.n_masked, 0);
        assert_eq!(kept_cell.masked_fraction, Some(0.0));
        assert_eq!(kept.band.dq_source.as_deref(), Some("EXTNAME DQ, HDU 3"));

        let excluded_cell = &excluded.cells[0];
        let aperture_weight =
            total_weight(&circular_aperture(BAND_SIZE, BAND_SIZE, excluded_cell.x, excluded_cell.y, 10.0, 5));
        assert_eq!(excluded_cell.flags_or, Some(5));
        assert_eq!(excluded_cell.n_masked, 1);
        assert!((excluded_cell.masked_fraction.unwrap() - 1.0 / aperture_weight).abs() < 1e-9, "{excluded_cell:?}");
        let drop = kept_cell.flux_native.unwrap() - excluded_cell.flux_native.unwrap();
        assert!(bad_value > 1.0 && (drop - bad_value).abs() < 1e-9 * bad_value, "drop={drop} pixel={bad_value}");
    }

    #[test]
    fn pixels_without_a_usable_error_value_leave_the_aperture() {
        let dir = tempfile::tempdir().unwrap();
        let mut err = vec![0.5f32; BAND_SIZE * BAND_SIZE];
        err[70 * BAND_SIZE + 81] = 0.0;
        err[72 * BAND_SIZE + 80] = 0.0;
        let sci = scene(BAND_SIZE, &[STAR_A], |_, _| 0.0);
        let a = write_layers(dir.path(), "a.fits", BAND_SIZE, (0.2, 0.2), sci.clone(), Some(err), None, vec![]);
        let result = measured(&a, &[sky_of(&a, 80.0, 70.0)], &plain(2.0));
        let cell = &result.cells[0];

        assert_eq!(cell.n_masked, 2);
        assert!(cell.masked_fraction.unwrap() > 0.0, "{cell:?}");
        assert!(cell.flux_native.is_some() && cell.error.is_none());
        assert!(
            result
                .band
                .warnings
                .contains(&"1 of 1 apertures of a.fits hold pixels without data: their flux is a lower limit".to_string()),
            "{:?}",
            result.band.warnings
        );
        assert_eq!(result.band.err_origin.unwrap().n_no_data, 2);

        let hole: Vec<f32> = (0..BAND_SIZE * BAND_SIZE)
            .map(|i| {
                let (dx, dy) = ((i % BAND_SIZE) as f64 - 80.0, (i / BAND_SIZE) as f64 - 70.0);
                if dx.hypot(dy) < 13.0 { f32::NAN } else { 0.5 }
            })
            .collect();
        let holed = write_layers(dir.path(), "holed.fits", BAND_SIZE, (0.2, 0.2), sci, Some(hole), None, vec![]);
        let result = measured(&holed, &[sky_of(&holed, 80.0, 70.0)], &plain(2.0));
        let cell = &result.cells[0];
        assert_eq!(cell.error.as_deref(), Some("the aperture holds no pixel with data"));
        assert_eq!(cell.masked_fraction, Some(1.0));
        assert!(cell.on_image && cell.flux_native.is_none());
        assert_eq!((result.band.n_measured, result.band.n_failed), (0, 1));
    }

    #[test]
    fn the_growth_curve_of_the_band_stars_corrects_to_the_reference_aperture() {
        let dir = tempfile::tempdir().unwrap();
        let stars = field_stars(&FIELD_STARS);
        let first_star = |key: &str| [sky_of(key, FIELD_STARS[0].0, FIELD_STARS[0].1)];

        let field = write_field(dir.path(), "field.fits", &stars, vec![]);
        let result = measured(&field, &first_star(&field), &growth(2.0));
        let correction = result.band.aperture_correction.clone().unwrap();
        assert!(relative_difference(correction.factor, MIRROR_FACTOR) < 0.015, "{correction:?}");
        assert!(relative_difference(correction.factor, CONTINUOUS_FACTOR) < 0.03, "{correction:?}");
        assert_eq!((correction.n_stars, correction.n_candidates), (6, 6));
        assert!((correction.reference_radius_px - 8.0).abs() < 1e-9, "{correction:?}");
        assert!((correction.radius_px - 3.0).abs() < 1e-9, "{correction:?}");
        assert!(result.cells[0].flux_native.is_some());
        assert_eq!(result.cells[0].flux_total_jy, None);

        let calibrated = write_field(dir.path(), "calibrated.fits", &stars, nanomaggy_cards());
        let result = measured(&calibrated, &first_star(&calibrated), &growth(2.0));
        let factor = result.band.aperture_correction.clone().unwrap().factor;
        let cell = &result.cells[0];
        assert_eq!(cell.flux_total_jy, Some(cell.flux_jy.unwrap() * factor));

        let mut with_galaxy = stars.clone();
        with_galaxy.push(FIELD_GALAXY);
        let mixed = write_field(dir.path(), "mixed.fits", &with_galaxy, vec![]);
        let correction = measured(&mixed, &first_star(&mixed), &growth(2.0)).band.aperture_correction.unwrap();
        assert!(relative_difference(correction.factor, MIRROR_FACTOR) < 0.015, "{correction:?}");
        assert_eq!((correction.n_stars, correction.n_candidates), (6, 7));

        let sparse = write_field(dir.path(), "sparse.fits", &field_stars(&FIELD_STARS[..2]), vec![]);
        let result = measured(&sparse, &first_star(&sparse), &growth(2.0));
        assert!(result.band.aperture_correction.is_none());
        assert!(
            result.band.warnings.contains(
                &"no aperture correction for sparse.fits: 2 isolated unsaturated sources above S/N 50, 0 of them on the point-source locus (3 needed)"
                    .to_string()
            ),
            "{:?}",
            result.band.warnings
        );

        let result = measured(&field, &first_star(&field), &growth(0.5));
        assert!(result.band.aperture_correction.is_none());
        assert!(
            result.band.warnings.contains(
                &"no aperture correction for field.fits: the reference radius 2.00 px is not larger than the aperture radius 3.00 px"
                    .to_string()
            ),
            "{:?}",
            result.band.warnings
        );
    }

    #[test]
    fn the_filter_comes_from_the_header_or_from_the_request() {
        let dir = tempfile::tempdir().unwrap();
        let nircam = write_band(
            dir.path(),
            "nircam.fits",
            0.2,
            &[STAR_A],
            0.0,
            None,
            vec![("TELESCOP", "'JWST'".into()), ("INSTRUME", "'NIRCAM'".into()), ("FILTER", "'F200W'".into())],
        );
        let band = measured(&nircam, &[], &plain(2.0)).band;
        let filter = band.filter.unwrap();
        assert_eq!((filter.dataset.as_str(), filter.filter.as_str(), filter.source.as_str()), ("JWST", "F200W", "header FILTER"));
        assert!(filter.pivot_um > 1.9 && filter.pivot_um < 2.1, "{}", filter.pivot_um);
        assert_eq!(band.label, "F200W");

        let plain_file = write_band(dir.path(), "plain.fits", 0.2, &[STAR_A], 0.0, None, vec![]);
        let requested =
            ForcedBandInput { path: plain_file.clone(), label: Some("r".into()), dataset: Some("LSST".into()) };
        let band = forced_band(&requested, &[], &plain(2.0), &ForcedOptions::default()).unwrap().band;
        let filter = band.filter.unwrap();
        assert_eq!((filter.dataset.as_str(), filter.filter.as_str(), filter.source.as_str()), ("LSST", "r", "request"));
        assert!(filter.pivot_um > 0.55 && filter.pivot_um < 0.70, "{}", filter.pivot_um);
        assert_eq!(band.label, "r");
        assert!(!band.warnings.iter().any(|warning| warning.contains("not identified")), "{:?}", band.warnings);

        let band = measured(&plain_file, &[], &plain(2.0)).band;
        assert!(band.filter.is_none());
        assert_eq!(band.label, "plain.fits");
        assert!(
            band.warnings.iter().any(|warning| {
                warning.starts_with("filter of plain.fits not identified (")
                    && warning.ends_with("): the band has no pivot wavelength and stays off the SED")
            }),
            "{:?}",
            band.warnings
        );
    }

    #[test]
    fn the_pixel_scale_helper_reads_the_header_only() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_band(dir.path(), "a.fits", 0.2, &[STAR_A], 0.0, None, vec![]);
        assert!((forced_band_pixel_scale(&a).unwrap() - 0.2).abs() < 1e-9);
        assert!(!measured(&a, &[], &plain(2.0)).band.warnings.iter().any(|warning| warning.contains("per axis")));

        let sci = scene(BAND_SIZE, &[STAR_A], |_, _| 0.0);
        let oblong = write_layers(dir.path(), "oblong.fits", BAND_SIZE, (0.2, 0.21), sci, None, None, vec![]);
        let area_scale = (0.2f64 * 0.21).sqrt();
        assert!((forced_band_pixel_scale(&oblong).unwrap() - area_scale).abs() < 1e-9);
        let band = measured(&oblong, &[], &plain(2.0)).band;
        assert!((band.pixel_scale_arcsec.unwrap() - area_scale).abs() < 1e-9);
        assert!((band.aperture_radius_px.unwrap() - 2.0 / area_scale).abs() < 1e-9);
        assert!(
            band.warnings.contains(
                &"pixel scales differ per axis (0.20000 x 0.21000 arcsec); radii in arcsec use the square root of the pixel area (0.20494 arcsec)"
                    .to_string()
            ),
            "{:?}",
            band.warnings
        );
    }

    #[test]
    fn an_empty_position_list_returns_the_band_facts_without_cells() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_band(dir.path(), "a.fits", 0.2, &[STAR_A], 0.0, Some(0.5), nanomaggy_cards());
        let result = forced_band(&band_input(&a), &[], &plain(2.0), &ForcedOptions::default()).unwrap();

        assert!(result.cells.is_empty());
        assert!((result.band.aperture_radius_px.unwrap() - 10.0).abs() < 1e-9);
        assert!((result.band.pixel_scale_arcsec.unwrap() - 0.2).abs() < 1e-9);
        assert!(relative_difference(result.band.jy_per_native_unit.unwrap(), JY_PER_NANOMAGGY) < 1e-8);
        assert_eq!((result.band.n_measured, result.band.n_failed), (0, 0));
        assert!(
            !result.band.warnings.iter().any(|warning| warning.contains("hold pixels without data")),
            "{:?}",
            result.band.warnings
        );
    }

    fn two_bands(dir: &Path, cards: Vec<(&'static str, String)>) -> (String, String) {
        let a = write_band(dir, "a.fits", 0.2, &[STAR_A], 0.0, Some(0.5), cards.clone());
        let b = write_band(dir, "b.fits", 0.4, &[STAR_B], 0.0, Some(0.5), cards);
        (a, b)
    }

    fn run(
        keys: &[&str],
        positions: &[ForcedSkyPosition],
        aperture: &ForcedAperture,
        options: &ForcedRunOptions,
    ) -> anyhow::Result<ForcedPhotometryResult> {
        let inputs: Vec<ForcedBandInput> = keys.iter().map(|key| band_input(key)).collect();
        forced_photometry(&inputs, positions, aperture, options, None)
    }

    fn sorted_keys(value: &serde_json::Value) -> Vec<String> {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
        names.sort();
        names
    }

    fn stage_recorder() -> (ProgressSink, Arc<Mutex<Vec<String>>>) {
        let stages: Arc<Mutex<Vec<String>>> = Arc::default();
        let seen = Arc::clone(&stages);
        let sink: ProgressSink = Arc::new(move |payload: &ProgressPayload| seen.lock().unwrap().push(payload.stage.clone()));
        (sink, stages)
    }

    #[test]
    fn two_bands_give_one_row_per_position_with_one_cell_per_band() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = two_bands(dir.path(), vec![]);
        let positions = [sky_of(&a, 80.0, 70.0), sky_of(&a, 40.0, 70.0)];
        let result = run(&[&a, &b], &positions, &plain(2.0), &ForcedRunOptions::default()).unwrap();

        assert_eq!(result.bands.iter().map(|band| band.index).collect::<Vec<_>>(), vec![0, 1]);
        assert!(result.bands.iter().all(|band| band.error.is_none()), "{:?}", result.bands);
        assert_eq!(result.sources.len(), 2);
        assert!(result.sources.iter().all(|source| source.cells.len() == 2));
        assert_eq!(result.sources[0].id, positions[0].id);
        assert_eq!((result.sources[1].ra_deg, result.sources[1].dec_deg), (positions[1].ra_deg, positions[1].dec_deg));
        let on_a = result.sources[0].cells[0].as_ref().unwrap();
        let on_b = result.sources[0].cells[1].as_ref().unwrap();
        assert!((on_a.x - 80.0).abs() < 1e-6 && (on_a.y - 70.0).abs() < 1e-6, "{on_a:?}");
        assert!((on_b.x - 71.75).abs() < 1e-4 && (on_b.y - 66.75).abs() < 1e-4, "{on_b:?}");
        assert_eq!(result.limit_sigma, 3.0);
        assert_eq!(result.aperture.radius_arcsec, Some(2.0));
        assert_eq!(result.aperture, plain(2.0));
        assert_eq!(
            result.warnings,
            vec![
                "2 of 2 bands have no calibration to Jy: they stay out of colours and of the SED".to_string(),
                "2 of 2 bands have no identified filter: they stay off the SED".to_string(),
            ]
        );
    }

    #[test]
    fn the_serialised_result_has_the_contract_keys() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = two_bands(dir.path(), vec![]);
        let positions = [sky_of(&a, 80.0, 70.0), sky_of(&a, 40.0, 70.0)];
        let result = run(&[&a, &b], &positions, &plain(2.0), &ForcedRunOptions::default()).unwrap();
        let value = serde_json::to_value(&result).unwrap();

        assert_eq!(sorted_keys(&value), sorted(&["bands", "sources", "aperture", "limit_sigma", "elapsed_ms", "warnings"]));
        assert_eq!(
            sorted_keys(&value["bands"][0]),
            sorted(&[
                "index",
                "path",
                "file_name",
                "label",
                "error",
                "filter",
                "photcal",
                "calibration_origin",
                "jy_per_native_unit",
                "bunit",
                "pixel_scale_arcsec",
                "aperture_radius_px",
                "annulus_px",
                "aperture_correction",
                "err_origin",
                "dq_source",
                "f_corr",
                "n_measured",
                "n_failed",
                "warnings",
                "flag_names",
                "mag_system",
            ])
        );
        assert_eq!(sorted_keys(&value["sources"][0]), sorted(&["id", "ra_deg", "dec_deg", "cells"]));
        assert_eq!(
            sorted_keys(&value["sources"][0]["cells"][1]),
            sorted(&[
                "x",
                "y",
                "on_image",
                "error",
                "flux_native",
                "flux_err_native",
                "flux_jy",
                "flux_err_jy",
                "flux_err_corr_jy",
                "flux_total_jy",
                "mag_ab",
                "mag_ab_err",
                "upper_limit_jy",
                "upper_limit_ab",
                "snr",
                "masked_fraction",
                "n_nonfinite",
                "n_masked",
                "flags_or",
                "saturated",
                "fwhm_px",
            ])
        );
        assert_eq!(
            sorted_keys(&value["aperture"]),
            sorted(&["radius_arcsec", "radius_px", "annulus_arcsec", "annulus_px", "correction", "reference_radius_arcsec"])
        );

        let field = write_field(dir.path(), "field.fits", &field_stars(&FIELD_STARS), vec![]);
        let first_star = [sky_of(&field, FIELD_STARS[0].0, FIELD_STARS[0].1)];
        let grown = run(&[&field], &first_star, &growth(2.0), &ForcedRunOptions::default()).unwrap();
        let value = serde_json::to_value(&grown).unwrap();
        assert_eq!(
            sorted_keys(&value["bands"][0]["aperture_correction"]),
            sorted(&["radius_px", "reference_radius_px", "factor", "scatter", "n_stars", "n_candidates"])
        );
        assert_eq!(value["aperture"]["correction"], "growth_curve");

        let default_reference = ForcedAperture { reference_radius_arcsec: None, ..growth(2.0) };
        let grown = run(&[&field], &first_star, &default_reference, &ForcedRunOptions::default()).unwrap();
        assert_eq!(grown.aperture.reference_radius_arcsec, Some(3.5));
        assert_eq!(grown.aperture.radius_px, Some(3.0));
    }

    #[test]
    fn a_band_that_cannot_be_measured_is_reported_and_the_others_still_run() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = two_bands(dir.path(), vec![]);
        let bare = dir.path().join("bare.fits");
        let bare = bare.to_str().unwrap();
        write_fits_mono(bare, &Array2::from_elem((32, 32), 1.0f32), None).unwrap();
        let positions = [sky_of(&a, 80.0, 70.0)];
        let result = run(&[&a, bare, &b], &positions, &plain(2.0), &ForcedRunOptions::default()).unwrap();

        let failed = &result.bands[1];
        assert!(failed.error.as_deref().is_some_and(|error| error.contains("has no celestial WCS")), "{failed:?}");
        assert_eq!((failed.index, failed.label.as_str(), failed.file_name.as_str()), (1, "bare.fits", "bare.fits"));
        assert_eq!(failed.path, bare);
        assert!(failed.photcal.is_null() && failed.filter.is_none() && failed.pixel_scale_arcsec.is_none());
        assert!(failed.aperture_radius_px.is_none() && failed.aperture_correction.is_none() && failed.f_corr.is_none());
        assert_eq!((failed.n_measured, failed.n_failed), (0, 0));
        assert!(result.sources[0].cells[1].is_none());
        for k in [0, 2] {
            assert!(result.bands[k].error.is_none(), "{:?}", result.bands[k]);
            assert_eq!(result.bands[k].index, k);
            assert!(result.sources[0].cells[k].as_ref().is_some_and(|cell| cell.flux_native.is_some()));
        }
        assert!(result.warnings.contains(&"1 of 3 bands could not be measured".to_string()), "{:?}", result.warnings);
        assert!(
            result
                .warnings
                .contains(&"2 of 3 bands have no calibration to Jy: they stay out of colours and of the SED".to_string()),
            "{:?}",
            result.warnings
        );

        let alone = run(&[bare], &positions, &plain(2.0), &ForcedRunOptions::default()).unwrap_err().to_string();
        assert!(alone.starts_with("no band could be measured:"), "{alone}");
        assert!(alone.ends_with("bare.fits has no celestial WCS: sky positions cannot be placed on it"), "{alone}");
    }

    #[test]
    fn requests_outside_the_limits_are_refused_before_any_file_is_opened() {
        let missing =
            |n: usize| -> Vec<ForcedBandInput> { (0..n).map(|i| band_input(&format!("C:/missing-forced/band{i}.fits"))).collect() };
        let at = |id: &str, ra_deg: f64, dec_deg: f64| ForcedSkyPosition { id: id.to_string(), ra_deg, dec_deg };
        let one = vec![at("a", 150.0, 2.0)];
        let none = ForcedRunOptions::default();
        let refusal = |bands: &[ForcedBandInput], positions: &[ForcedSkyPosition], aperture: &ForcedAperture, options: &ForcedRunOptions| {
            forced_photometry(bands, positions, aperture, options, None).unwrap_err().to_string()
        };

        assert_eq!(refusal(&missing(0), &one, &plain(2.0), &none), "forced photometry needs at least one band, got 0");
        assert_eq!(refusal(&missing(13), &one, &plain(2.0), &none), "forced photometry of 13 bands exceeds the limit of 12");
        assert_eq!(refusal(&missing(1), &[], &plain(2.0), &none), "forced photometry needs at least one position, got 0");
        let many: Vec<ForcedSkyPosition> = (0..5001).map(|i| at(&format!("p{i}"), 150.0, 2.0)).collect();
        assert_eq!(
            refusal(&missing(1), &many, &plain(2.0), &none),
            "forced photometry of 5001 positions exceeds the limit of 5000"
        );
        let twice = [missing(1), missing(1)].concat();
        assert_eq!(refusal(&twice, &one, &plain(2.0), &none), "band C:/missing-forced/band0.fits is listed twice");
        assert!(refusal(&missing(1), &[at("a", 150.0, 2.0), at("", 150.0, 2.0)], &plain(2.0), &none)
            .contains("position 1 has an empty id"));
        assert!(refusal(&missing(1), &[at("a", 150.0, 2.0), at("a", 151.0, 2.0)], &plain(2.0), &none)
            .contains("position id 'a' is used twice"));
        assert!(refusal(&missing(1), &[at("a", 400.0, 2.0)], &plain(2.0), &none)
            .contains("position 'a' (400, 2) must have RA in 0 to 360 and Dec in -90 to 90 degrees"));
        assert!(refusal(&missing(1), &[at("a", 150.0, f64::NAN)], &plain(2.0), &none)
            .contains("position 'a' (150, NaN) must have RA in 0 to 360 and Dec in -90 to 90 degrees"));
        assert!(refusal(&missing(1), &one, &ForcedAperture { radius_arcsec: None, ..plain(2.0) }, &none)
            .contains("the aperture needs a radius in arcsec or in pixels"));
        let options = ForcedRunOptions { limit_sigma: Some(0.0), ..ForcedRunOptions::default() };
        assert!(refusal(&missing(1), &one, &plain(2.0), &options).contains("limit_sigma 0 must be a positive finite number"));
        let options = ForcedRunOptions { gain: Some(-1.0), ..ForcedRunOptions::default() };
        assert!(refusal(&missing(1), &one, &plain(2.0), &options)
            .contains("gain -1 must be a positive finite number of e-/ADU"));
        let options = ForcedRunOptions { f_corr: Some(vec![Some(1.2)]), ..ForcedRunOptions::default() };
        assert!(refusal(&missing(2), &one, &plain(2.0), &options).contains("f_corr has 1 values for 2 bands"));
        let options = ForcedRunOptions { f_corr: Some(vec![Some(0.0), None]), ..ForcedRunOptions::default() };
        assert!(refusal(&missing(2), &one, &plain(2.0), &options)
            .contains("f_corr of band 0 is 0: it must be a positive finite number"));

        let measured_missing = run(&["C:/missing-forced/band0.fits"], &one, &plain(2.0), &none).unwrap_err().to_string();
        assert!(measured_missing.starts_with("no band could be measured:"), "{measured_missing}");
    }

    #[test]
    fn a_pixel_radius_needs_bands_of_one_pixel_scale() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = two_bands(dir.path(), vec![]);
        let in_pixels = ForcedAperture { radius_arcsec: None, radius_px: Some(5.0), ..plain(2.0) };
        let positions = [sky_of(&a, 80.0, 70.0)];

        assert_eq!(
            run(&[&a, &b], &positions, &in_pixels, &ForcedRunOptions::default()).unwrap_err().to_string(),
            "an aperture radius in pixels needs bands of one pixel scale: a.fits has 0.2000 and b.fits has 0.4000 arcsec/px; give the radius in arcsec"
        );

        let copy_path = dir.path().join("copy.fits");
        std::fs::copy(dir.path().join("a.fits"), &copy_path).unwrap();
        let copy = format!("{}#hdu=1", copy_path.to_str().unwrap());
        let result = run(&[&a, &copy], &positions, &in_pixels, &ForcedRunOptions::default()).unwrap();
        assert!(result.bands.iter().all(|band| band.error.is_none()), "{:?}", result.bands);
        assert_eq!(result.bands[1].aperture_radius_px, Some(5.0));
        assert_eq!(result.bands[1].file_name, "copy.fits");
    }

    #[test]
    fn per_band_noise_factors_scale_the_corrected_error_of_their_band_only() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = two_bands(dir.path(), nanomaggy_cards());
        let positions = [sky_of(&a, 80.0, 70.0), sky_of(&a, 40.0, 70.0)];
        let options = ForcedRunOptions { f_corr: Some(vec![Some(1.5), None]), ..ForcedRunOptions::default() };
        let result = run(&[&a, &b], &positions, &plain(2.0), &options).unwrap();

        assert_eq!(result.bands[0].f_corr, Some(1.5));
        assert_eq!(result.bands[1].f_corr, None);
        for source in &result.sources {
            let on_a = source.cells[0].as_ref().unwrap();
            let flux_err_jy = on_a.flux_err_jy.unwrap();
            assert!(relative_difference(on_a.flux_err_corr_jy.unwrap(), 1.5 * flux_err_jy) < 1e-12, "{on_a:?}");
            let on_b = source.cells[1].as_ref().unwrap();
            assert!(on_b.flux_err_jy.is_some(), "{on_b:?}");
            assert_eq!(on_b.flux_err_corr_jy, None);
        }
        assert!(!result.warnings.iter().any(|warning| warning.contains("no calibration to Jy")), "{:?}", result.warnings);
    }

    #[test]
    fn progress_ticks_once_per_band_and_a_cancel_request_stops_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = two_bands(dir.path(), vec![]);
        let inputs = vec![band_input(&a), band_input(&b)];
        let positions = [sky_of(&a, 80.0, 70.0)];
        let options = ForcedRunOptions::default();

        let (sink, stages) = stage_recorder();
        let progress = ProgressHandle::with_sink(sink, 2);
        forced_photometry(&inputs, &positions, &plain(2.0), &options, Some(&progress)).unwrap();
        assert_eq!(*stages.lock().unwrap(), vec!["a.fits".to_string(), "b.fits".to_string()]);

        let (sink, stages) = stage_recorder();
        let cancelled = ProgressHandle::with_sink(sink, 2).with_cancel_source(Arc::new(|| true));
        let error = forced_photometry(&inputs, &positions, &plain(2.0), &options, Some(&cancelled)).unwrap_err();
        assert_eq!(error.to_string(), "Operation cancelled");
        assert!(stages.lock().unwrap().is_empty());
    }

    fn band_layer(extname: &'static str, data: HduData, extra_cards: Vec<(&'static str, String)>) -> TestHdu {
        TestHdu { extname: Some(extname), extver: None, cols: BAND_SIZE, rows: BAND_SIZE, data, extra_cards }
    }

    fn science_layer(extname: &'static str) -> TestHdu {
        band_layer(extname, HduData::F32(scene(BAND_SIZE, &[STAR_A], |_, _| 0.0)), wcs_cards(BAND_SIZE, (0.2, 0.2)))
    }

    fn write_survey_layers(dir: &Path, name: &str, hdus: &[TestHdu]) -> String {
        let path = dir.join(name);
        write_test_mef(&path, &[], hdus);
        format!("{}#hdu=1", path.to_str().unwrap())
    }

    fn pixel_index(x: usize, y: usize) -> usize {
        y * BAND_SIZE + x
    }

    fn mask_on_the_source(value: i32) -> Vec<i32> {
        (0..BAND_PIXELS)
            .map(|i| {
                let (dx, dy) = ((i % BAND_SIZE) as f64 - STAR_A.0, (i / BAND_SIZE) as f64 - STAR_A.1);
                if dx.hypot(dy) < 13.0 { value } else { 0 }
            })
            .collect()
    }

    fn half_sigma_error(cell: &ForcedCell, without_data: &[(usize, usize)]) -> f64 {
        let weights = circular_aperture(BAND_SIZE, BAND_SIZE, cell.x, cell.y, 10.0, FORCED_SUBSAMPLES);
        let sum_of_squares: f64 = weights
            .iter()
            .filter(|pixel| !without_data.contains(&(pixel.x, pixel.y)))
            .map(|pixel| (pixel.weight as f64).powi(2))
            .sum();
        0.5 * sum_of_squares.sqrt()
    }

    fn typed_zero_points(zero_points: serde_json::Value) -> ForcedRunOptions {
        serde_json::from_value(json!({ "zero_points": zero_points })).unwrap()
    }

    fn write_griz_cube(dir: &Path, name: &str, extra: Vec<(&'static str, String)>) -> String {
        let empty = vec![0.0f32; CUBE_SIZE * CUBE_SIZE];
        let r_plane = scene(CUBE_SIZE, &[(30.0, 30.0, TOTAL_FLUX / (2.0 * PI * 9.0), 3.0)], |_, _| 0.0);
        let data: Vec<u8> =
            [&empty, &r_plane, &empty, &empty].into_iter().flatten().flat_map(|value| value.to_be_bytes()).collect();
        let mut cards: Vec<(&'static str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "3".into()),
            ("NAXIS1", CUBE_SIZE.to_string()),
            ("NAXIS2", CUBE_SIZE.to_string()),
            ("NAXIS3", "4".into()),
            ("BANDS", "'griz    '".into()),
            ("BAND0", "'g       '".into()),
            ("BAND1", "'r       '".into()),
            ("BAND2", "'i       '".into()),
            ("BAND3", "'z       '".into()),
        ];
        cards.extend(wcs_cards(CUBE_SIZE, (0.262, 0.262)));
        cards.extend(extra);
        let path = dir.join(name);
        write_raw_hdus(&path, &[(cards, data)]);
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn a_variance_layer_gives_the_same_error_as_its_sigma() {
        let dir = tempfile::tempdir().unwrap();
        let calexp = write_survey_layers(
            dir.path(),
            "calexp.fits",
            &[
                science_layer("IMAGE"),
                band_layer("MASK", HduData::I32(vec![0; BAND_PIXELS]), vec![]),
                band_layer("VARIANCE", HduData::F32(vec![0.25; BAND_PIXELS]), vec![]),
            ],
        );
        let result = measured(&calexp, &[sky_of(&calexp, 80.0, 70.0)], &plain(2.0));
        let cell = &result.cells[0];

        let flux_err = cell.flux_err_native.unwrap();
        let expected = half_sigma_error(cell, &[]);
        assert!(relative_difference(flux_err, expected) < 1e-6, "flux_err={flux_err} expected={expected}");
        let origin = result.band.err_origin.unwrap();
        assert_eq!(
            (origin.role.as_str(), origin.source.as_str(), origin.assumed, origin.n_no_data),
            ("variance", "EXTNAME VARIANCE, HDU 3: square root", false, 0)
        );
        assert_eq!(result.band.dq_source.as_deref(), Some("EXTNAME MASK, HDU 2"));
    }

    #[test]
    fn zero_inverse_variance_is_no_data_not_a_large_error() {
        let dir = tempfile::tempdir().unwrap();
        let without_data = [(81, 70), (80, 72)];
        let mut inverse_variance = vec![4.0f32; BAND_PIXELS];
        for (x, y) in without_data {
            inverse_variance[pixel_index(x, y)] = 0.0;
        }
        let coadd = write_survey_layers(
            dir.path(),
            "coadd.fits",
            &[science_layer("SCI"), band_layer("INVVAR", HduData::F32(inverse_variance), vec![])],
        );
        let result = measured(&coadd, &[sky_of(&coadd, 80.0, 70.0)], &plain(2.0));
        let cell = &result.cells[0];

        assert_eq!(cell.n_masked, 2);
        assert!(cell.masked_fraction.unwrap() > 0.0, "{cell:?}");
        let origin = result.band.err_origin.unwrap();
        assert_eq!(
            (origin.role.as_str(), origin.source.as_str(), origin.assumed, origin.n_no_data),
            ("inverse_variance", "EXTNAME INVVAR, HDU 2: 1 / square root", false, 2)
        );
        let flux_err = cell.flux_err_native.unwrap();
        let expected = half_sigma_error(cell, &without_data);
        assert!(relative_difference(flux_err, expected) < 1e-6, "flux_err={flux_err} expected={expected}");
        assert!(expected < half_sigma_error(cell, &[]));
    }

    #[test]
    fn afw_mask_planes_are_named_from_the_file_and_detected_never_excludes() {
        let dir = tempfile::tempdir().unwrap();
        let saturated_pixel = pixel_index(83, 71);
        let saturated_value = scene(BAND_SIZE, &[STAR_A], |_, _| 0.0)[saturated_pixel] as f64;
        let mut mask = mask_on_the_source(32);
        mask[saturated_pixel] |= 2;
        let mask_planes = vec![("MP_BAD", "0".into()), ("MP_SAT", "1".into()), ("HIERARCH MP_DETECTED", "5".into())];
        let calexp = write_survey_layers(
            dir.path(),
            "calexp.fits",
            &[science_layer("IMAGE"), band_layer("MASK", HduData::I32(mask), mask_planes)],
        );
        let position = [sky_of(&calexp, 80.0, 70.0)];
        let kept = measured(&calexp, &position, &plain(2.0));
        let excluding = ForcedOptions { exclude_dq: true, ..ForcedOptions::default() };
        let excluded = forced_band(&band_input(&calexp), &position, &plain(2.0), &excluding).unwrap();

        let named = |value: u32, name: &str| ForcedFlagName { value, name: name.to_string() };
        assert_eq!(kept.band.flag_names, vec![named(1, "BAD"), named(2, "SAT"), named(32, "DETECTED")]);
        assert_eq!(excluded.band.flag_names, kept.band.flag_names);
        assert_eq!(kept.band.dq_source.as_deref(), Some("EXTNAME MASK, HDU 2"));
        for result in [&kept, &excluded] {
            assert_eq!(result.cells[0].flags_or, Some(34));
            assert!(result.cells[0].saturated, "{:?}", result.cells[0]);
        }
        assert_eq!(kept.cells[0].n_masked, 0);
        assert_eq!(excluded.cells[0].n_masked, 1);
        let drop = kept.cells[0].flux_native.unwrap() - excluded.cells[0].flux_native.unwrap();
        assert!(
            saturated_value > 1.0 && (drop - saturated_value).abs() < 1e-9 * saturated_value,
            "drop={drop} pixel={saturated_value}"
        );
    }

    #[test]
    fn a_flag_layer_without_names_excludes_nothing_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mosaic = write_survey_layers(
            dir.path(),
            "mosaic.fits",
            &[science_layer("SCI"), band_layer("MASK", HduData::I32(mask_on_the_source(1 << 24)), vec![])],
        );
        let excluding = ForcedOptions { exclude_dq: true, ..ForcedOptions::default() };
        let result = forced_band(&band_input(&mosaic), &[sky_of(&mosaic, 80.0, 70.0)], &plain(2.0), &excluding).unwrap();
        let cell = &result.cells[0];

        assert_eq!(cell.n_masked, 0);
        assert_eq!(cell.masked_fraction, Some(0.0));
        assert_eq!(cell.flags_or, Some(1 << 24));
        assert!(relative_difference(cell.flux_native.unwrap(), ENCLOSED_FLUX) < 0.01, "{cell:?}");
        assert!(result.band.flag_names.is_empty(), "{:?}", result.band.flag_names);
        assert_eq!(result.band.dq_source.as_deref(), Some("EXTNAME MASK, HDU 2"));
        let said: Vec<&String> =
            result.band.warnings.iter().filter(|warning| warning.contains("nothing is excluded")).collect();
        assert_eq!(said.len(), 1, "{:?}", result.band.warnings);
    }

    #[test]
    fn physical_units_and_flux_zero_points_calibrate_a_band() {
        let dir = tempfile::tempdir().unwrap();
        let cases: [(&str, (&'static str, String), f64, f64, &str); 3] = [
            ("njy.fits", ("BUNIT", "'nJy'".into()), 1.0e-9, 1e-12, "nJy"),
            ("fluxmag0.fits", ("FLUXMAG0", "63095734448.0194".into()), 5.754399e-8, 1e-6, "FLUXMAG0"),
            ("nanomaggy.fits", ("BUNIT", "'nanomaggy'".into()), 3.630781e-6, 1e-6, "nanomaggy"),
        ];
        for (name, card, jy_per_unit, tolerance, named) in cases {
            let key = write_band(dir.path(), name, 0.2, &[STAR_A], 0.0, Some(0.5), vec![card]);
            let band = measured(&key, &[], &plain(2.0)).band;
            let factor = band.jy_per_native_unit.unwrap();
            assert!(relative_difference(factor, jy_per_unit) < tolerance, "{name}: {factor}");
            let origin = band.calibration_origin.unwrap();
            assert!(origin.contains(named), "{name}: {origin}");
            assert_eq!(band.photcal["origin"].as_str(), Some(origin.as_str()), "{name}");
            assert_eq!(band.mag_system.as_deref(), Some("ab"), "{name}");
        }
    }

    #[test]
    fn a_typed_zero_point_calibrates_a_band_without_header_calibration() {
        let dir = tempfile::tempdir().unwrap();
        let bare = write_band(dir.path(), "bare.fits", 0.2, &[STAR_A], 0.0, Some(0.5), vec![]);
        let position = [sky_of(&bare, 80.0, 70.0)];
        let typed = |key: &str, zero_point: serde_json::Value| {
            run(&[key], &position, &plain(2.0), &typed_zero_points(json!([zero_point]))).unwrap()
        };
        let counted_per_second =
            |text: &String| text.starts_with("the user zero point is defined for one count per second");

        let ab = typed(&bare, json!({ "zp": 25.0, "system": "ab", "per_second": false }));
        let cell = ab.sources[0].cells[0].as_ref().unwrap();
        let (flux_native, flux_jy) = (cell.flux_native.unwrap(), cell.flux_jy.unwrap());
        assert!(relative_difference(flux_jy, flux_native * 3.630780547701e-7) < 1e-8, "{cell:?}");
        assert!((cell.mag_ab.unwrap() - (25.0 - 2.5 * flux_native.log10())).abs() < 1e-9, "{cell:?}");
        let band = &ab.bands[0];
        assert_eq!(
            band.warnings.iter().filter(|warning| warning.as_str() == "calibrated with a user zero point").count(),
            1,
            "{:?}",
            band.warnings
        );
        assert!(!band.warnings.iter().any(|warning| warning.contains("no photometric calibration")), "{:?}", band.warnings);
        assert_eq!(band.calibration_origin.as_deref(), Some("user zero point 25.0 AB"));
        assert_eq!(band.mag_system.as_deref(), Some("ab"));
        assert!(!ab.warnings.iter().any(|warning| warning.contains("no calibration to Jy")), "{:?}", ab.warnings);

        let vega = typed(&bare, json!({ "zp": 25.0, "system": "vega", "per_second": false }));
        let cell = vega.sources[0].cells[0].as_ref().unwrap();
        assert!(cell.flux_native.is_some());
        assert_eq!((cell.flux_jy, cell.mag_ab), (None, None));
        let band = &vega.bands[0];
        assert_eq!(band.mag_system.as_deref(), Some("vega"));
        assert_eq!(band.jy_per_native_unit, None);
        assert_eq!(band.calibration_origin.as_deref(), Some("user zero point 25.0 Vega"));
        assert!(band.warnings.iter().any(|warning| warning.contains("Vega system")), "{:?}", band.warnings);

        for (name, bunit, exptime, zp, jy_per_unit) in [
            ("rate_des.fits", "'COUNTS/S'", "1080.0", 30.0, 3.6307805477010028e-9),
            ("rate_adu.fits", "'ADU/S'", "300.0", 25.0, 3.63078054770101e-7),
        ] {
            let cards = vec![("BUNIT", bunit.to_string()), ("EXPTIME", exptime.to_string())];
            let key = write_band(dir.path(), name, 0.2, &[STAR_A], 0.0, Some(0.5), cards);
            let result = typed(&key, json!({ "zp": zp, "system": "ab", "per_second": true }));
            let band = &result.bands[0];
            let factor = band.jy_per_native_unit.unwrap();
            assert!(relative_difference(factor, jy_per_unit) < 1e-12, "{name}: {factor}");
            assert!(!band.warnings.iter().any(counted_per_second), "{name}: {:?}", band.warnings);
        }

        let cards = vec![("BUNIT", "'adu'".to_string()), ("EXPTIME", "600.0".to_string())];
        let counts = write_band(dir.path(), "counts.fits", 0.2, &[STAR_A], 0.0, Some(0.5), cards);
        let result = typed(&counts, json!({ "zp": 25.0, "system": "ab", "per_second": true }));
        let band = &result.bands[0];
        let factor = band.jy_per_native_unit.unwrap();
        assert!(relative_difference(factor, 6.051300912835017e-10) < 1e-12, "{factor}");
        assert!(
            band.warnings.contains(
                &"the user zero point is defined for one count per second: the pixel values are divided by EXPTIME = 600.0 s"
                    .to_string()
            ),
            "{:?}",
            band.warnings
        );

        let untimed =
            write_band(dir.path(), "untimed.fits", 0.2, &[STAR_A], 0.0, Some(0.5), vec![("BUNIT", "'adu'".to_string())]);
        let a = write_band(dir.path(), "a.fits", 0.2, &[STAR_A], 0.0, None, vec![]);
        let per_second = typed_zero_points(json!([{ "zp": 25.0, "system": "ab", "per_second": true }, null]));
        let result = run(&[&untimed, &a], &position, &plain(2.0), &per_second).unwrap();
        let refused = result.bands[0].error.as_deref().unwrap_or_default();
        assert!(
            refused.contains(
                "a zero point per second needs EXPTIME to turn the pixel values into a rate, and this header has none (BUNIT 'adu')"
            ),
            "{refused}"
        );
        assert!(result.sources[0].cells[0].is_none());
        assert!(result.bands[1].error.is_none(), "{:?}", result.bands[1]);
        assert!(result.sources[0].cells[1].as_ref().is_some_and(|cell| cell.flux_native.is_some()));
    }

    #[test]
    fn a_typed_zero_point_does_not_override_a_header_calibration() {
        let dir = tempfile::tempdir().unwrap();
        let cards = vec![("BUNIT", "'nanomaggy'".to_string())];
        let nanomaggy = write_band(dir.path(), "nanomaggy.fits", 0.2, &[STAR_A], 0.0, Some(0.5), cards);
        let position = [sky_of(&nanomaggy, 80.0, 70.0)];
        let typed = typed_zero_points(json!([{ "zp": 25.0, "system": "ab", "per_second": false }]));
        let result = run(&[&nanomaggy], &position, &plain(2.0), &typed).unwrap();
        let band = &result.bands[0];

        let factor = band.jy_per_native_unit.unwrap();
        assert!(relative_difference(factor, 3.630781e-6) < 1e-6, "{factor}");
        let not_applied = "user zero point not applied: the header calibrates this image (";
        assert_eq!(
            band.warnings.iter().filter(|warning| warning.starts_with(not_applied)).count(),
            1,
            "{:?}",
            band.warnings
        );
        assert!(!band.warnings.iter().any(|warning| warning == "calibrated with a user zero point"), "{:?}", band.warnings);

        let other = write_band(dir.path(), "other.fits", 0.2, &[STAR_A], 0.0, Some(0.5), vec![]);
        assert_eq!(
            run(&[&nanomaggy, &other], &position, &plain(2.0), &typed).unwrap_err().to_string(),
            "zero_points has 1 values for 2 bands"
        );
    }

    #[test]
    fn a_plane_of_a_cutout_cube_is_a_band() {
        let dir = tempfile::tempdir().unwrap();
        let cube = write_griz_cube(dir.path(), "cutout.fits", vec![]);
        let plane = |k: usize| format!("{cube}#hdu=0&plane={k}");
        let decam = |path: String| ForcedBandInput { path, label: None, dataset: Some("DECAM".into()) };
        let position = [sky_of(&plane(1), 30.0, 30.0)];

        let r = forced_band(&decam(plane(1)), &position, &plain(2.0), &ForcedOptions::default()).unwrap();
        assert_eq!(r.band.label, "r");
        assert_eq!(r.band.file_name, "cutout.fits");
        assert_eq!(r.band.path, plane(1));
        let radius_px = r.band.aperture_radius_px.unwrap();
        let enclosed = TOTAL_FLUX * (1.0 - (-radius_px * radius_px / 18.0).exp());
        let flux = r.cells[0].flux_native.unwrap();
        assert!(relative_difference(flux, enclosed) < 0.01, "flux={flux} enclosed={enclosed}");
        let filter = r.band.filter.unwrap();
        assert_eq!((filter.dataset.as_str(), filter.filter.as_str()), ("DECAM", "r"));
        assert!((filter.pivot_um - 0.64176).abs() < 1e-4, "{}", filter.pivot_um);

        let g = forced_band(&decam(plane(0)), &position, &plain(2.0), &ForcedOptions::default()).unwrap();
        assert_eq!(g.band.label, "g");
        assert!(g.cells[0].flux_native.unwrap().abs() < 1e-6, "{:?}", g.cells[0]);

        let no_dataset = measured(&plane(1), &position, &plain(2.0)).band;
        assert_eq!(no_dataset.label, "r");
        assert!(no_dataset.filter.is_none());
        assert!(
            no_dataset.warnings.iter().any(|warning| warning.starts_with("filter of r not identified ('cutout.fits': ")),
            "{:?}",
            no_dataset.warnings
        );

        let foreign = write_griz_cube(dir.path(), "foreign.fits", vec![("INSTRUME", "'SIMCAM  '".into())]);
        let band = forced_band(&decam(format!("{foreign}#hdu=0&plane=1")), &[], &plain(2.0), &ForcedOptions::default())
            .unwrap()
            .band;
        let filter = band.filter.unwrap();
        assert_eq!((filter.dataset.as_str(), filter.filter.as_str(), filter.source.as_str()), ("DECAM", "r", "request"));
        assert_eq!(band.label, "r");
    }

    #[test]
    fn a_layer_that_cannot_be_used_is_told_once_in_the_band_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let small_error = TestHdu {
            extname: Some("ERR"),
            extver: None,
            cols: CUBE_SIZE,
            rows: CUBE_SIZE,
            data: HduData::F32(vec![0.5; CUBE_SIZE * CUBE_SIZE]),
            extra_cards: vec![],
        };
        let float_mask = band_layer("DQ", HduData::F32(vec![0.0; BAND_PIXELS]), vec![]);
        let key = write_survey_layers(dir.path(), "unusable.fits", &[science_layer("SCI"), small_error, float_mask]);
        let position = [sky_of(&key, 80.0, 70.0)];

        for exclude_dq in [false, true] {
            let options = ForcedOptions { exclude_dq, ..ForcedOptions::default() };
            let band = forced_band(&band_input(&key), &position, &plain(2.0), &options).unwrap().band;
            let told = |reason: &str| band.warnings.iter().filter(|warning| warning.contains(reason)).count();
            assert_eq!(told("is 64x64 and does not match the image 128x128: not used"), 1, "{:?}", band.warnings);
            assert_eq!(told("flag bits cannot be read exactly"), 1, "{:?}", band.warnings);
            assert!(band.err_origin.is_none() && band.dq_source.is_none() && band.flag_names.is_empty());
        }
    }

    #[test]
    fn the_command_is_registered() {
        assert!(include_str!("../../lib.rs").contains("cmd::analysis::forced_photometry_cmd,"));
    }
}
