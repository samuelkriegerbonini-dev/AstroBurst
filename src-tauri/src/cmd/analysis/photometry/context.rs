use crate::cmd::analysis::area::AreaWeights;
use serde_json::json;
use crate::cmd::common::{load_cached, load_cached_full};
use crate::types::constants::{RES_RA, RES_DEC, RES_LABEL};
use crate::core::analysis::gain::{gain_model, GainModel};
use crate::core::analysis::photometry::{measure_star_prepared, saturation_level, MaskedImage, PhotometryConfig, StarPhotometry};
use crate::core::astrometry::wcs::WcsTransform;
use crate::infra::wcs_source::load_wcs;
use crate::core::metadata::photcal::{missing_calibration_reason, PhotCal};
use crate::infra::cache::ImageEntry;
use crate::types::header::HduHeader;
use crate::cmd::analysis::{DqMask, LayerReport, PhotometryPlanes, UserZeroPoint, dq_not_applied_warning, layer_report, no_data_exclusion, photometry_planes, resolve_dq_mask, uncalibrated_layer_reason, user_photcal};

pub const PHOTOMETRY_SATURATED_FLAG: &str = "SATURATED";
pub const HEADER_PROCESSING_PROVENANCE: &str = "ABPROC";
pub const RES_PHOTCAL: &str = "photcal";

pub(crate) fn apply_calibration(phot: &mut StarPhotometry, cal: &PhotCal) {
    if let Some(c) = cal.calibrate(phot.net_flux, Some(phot.flux_err)) {
        phot.flux_jy = Some(c.flux_jy);
        phot.flux_err_jy = c.flux_err_jy;
        phot.mag_ab = c.mag_ab;
        phot.mag_ab_err = c.mag_ab_err;
        phot.st_mag = c.st_mag;
    }
    phot.mag_ab_total = phot
        .flux_total
        .and_then(|total| cal.calibrate(total, None))
        .and_then(|c| c.mag_ab);
}

fn photcal_json(cal: &PhotCal) -> anyhow::Result<serde_json::Value> {
    let mut val = serde_json::to_value(cal)?;
    if let Some(obj) = val.as_object_mut() {
        obj.insert(RES_LABEL.to_string(), json!(cal.label()));
    }
    Ok(val)
}

pub(crate) struct PhotometryContext {
    pub entry: ImageEntry,
    pub mask: Option<DqMask>,
    pub planes: PhotometryPlanes,
    pub area: AreaWeights,
    pub wcs: Option<WcsTransform>,
    pub photcal: Option<PhotCal>,
    pub gain_model: GainModel,
    pub warnings: Vec<String>,
    pub layers: LayerReport,
    pub calibration_source: Option<&'static str>,
    pub calibration_gap: Option<String>,
    pub exclusion: Option<ndarray::Array2<u8>>,
}

pub(crate) fn photometry_context(path: &str, exclude_dq: bool) -> anyhow::Result<PhotometryContext> {
    photometry_context_with(path, exclude_dq, None)
}

pub(crate) fn photometry_context_with(
    path: &str,
    exclude_dq: bool,
    zero_point: Option<&UserZeroPoint>,
) -> anyhow::Result<PhotometryContext> {
    let entry = load_cached_full(path).or_else(|_| load_cached(path))?;
    let dims = entry.arr().dim();
    let mask = resolve_dq_mask(path, exclude_dq, dims);
    let planes = photometry_planes(path, dims);
    let layers = layer_report(path, &entry);
    let header = entry.header();
    let wcs = header.and_then(|h| load_wcs(path, h).ok());
    let gain_model = gain_model(header, planes.err.is_some());
    let mut warnings: Vec<String> = Vec::new();
    let header_calibration = match uncalibrated_layer_reason(path, &entry) {
        Some(layer_reason) => Err(layer_reason),
        None => Ok(header.and_then(|h| PhotCal::from_header(h, wcs.as_ref()))),
    };
    let (mut photcal, calibration_source, calibration_gap) = match (header_calibration, zero_point) {
        (Err(layer_reason), _) => (None, None, Some(layer_reason)),
        (Ok(Some(cal)), typed) => {
            warnings.extend(cal.warnings.iter().cloned());
            if typed.is_some() {
                warnings.push(format!("user zero point not applied: the header calibrates this image ({})", cal.origin));
            }
            (Some(cal), Some("header"), None)
        }
        (Ok(None), Some(typed)) => {
            let (cal, _) = user_photcal(header, typed)?;
            warnings.extend(cal.warnings.iter().cloned());
            (Some(cal), Some("user"), None)
        }
        (Ok(None), None) => (None, None, Some(missing_calibration_reason(header))),
    };
    warnings.extend(calibration_gap.clone());
    warnings.extend(layers.err_note.clone());
    warnings.extend(layers.err_gap.as_ref().map(|gap| format!("error layer not used: {gap}")));
    let area = AreaWeights::resolve(path, &entry, photcal.as_mut(), &mut warnings);
    warnings.extend(processed_data_warning(header));
    warnings.extend(wcs.as_ref().and_then(|w| w.pv_warning()).map(str::to_string));
    if exclude_dq && mask.is_none() {
        warnings.extend(dq_not_applied_warning(&layers));
    }
    let exclusion = no_data_exclusion(path, &planes, mask.as_ref());
    Ok(PhotometryContext {
        entry,
        mask,
        planes,
        area,
        wcs,
        photcal,
        gain_model,
        warnings,
        layers,
        calibration_source,
        calibration_gap,
        exclusion,
    })
}

pub(crate) fn processed_data_warning(header: Option<&HduHeader>) -> Option<String> {
    let provenance = header?.get(HEADER_PROCESSING_PROVENANCE)?;
    Some(format!("photometry on processed data ({})", provenance.trim().trim_matches('\'').trim()))
}

pub(crate) fn pixel_units(header: Option<&HduHeader>) -> (Option<String>, Option<String>) {
    let Some(header) = header else {
        return (None, None);
    };
    let Some(bunit) = crate::core::metadata::photcal::card_text(header, "BUNIT") else {
        return (None, None);
    };
    let summed_unit = match crate::core::metadata::units::classify_unit(header).class {
        crate::core::metadata::units::UnitClass::SurfaceBrightness { .. } => {
            format!("{bunit}{}", crate::cmd::line_measure::REGION_SUM_UNIT_SUFFIX)
        }
        _ => bunit.clone(),
    };
    (Some(bunit), Some(summed_unit))
}

impl PhotometryContext {
    pub(in crate::cmd::analysis) fn config(&self, aperture_radius: Option<f64>, sky_annulus: Option<(f64, f64)>, gain: Option<f64>) -> PhotometryConfig {
        PhotometryConfig {
            aperture_radius: aperture_radius.filter(|r| r.is_finite() && *r > 0.0),
            saturation: Some(saturation_level(self.entry.header(), self.entry.stats().max)),
            gain: gain.filter(|g| g.is_finite() && *g > 0.0),
            sky_annulus,
            ..PhotometryConfig::default()
        }
    }

    pub(in crate::cmd::analysis) fn masked_image(&self) -> Result<MaskedImage<'_>, String> {
        self.area.masked_image(&self.entry, self.exclusion.as_ref().or(self.mask.as_ref().map(|m| &m.map)))
    }

    pub(in crate::cmd::analysis) fn measure(&self, source: &MaskedImage, x: f64, y: f64, config: &PhotometryConfig) -> Result<StarPhotometry, String> {
        let mut phot = measure_star_prepared(
            source,
            self.planes.err.as_ref().map(|e| e.arr()),
            self.planes.saturated.as_ref(),
            x,
            y,
            config,
        )?;
        if let Some(cal) = &self.photcal {
            apply_calibration(&mut phot, cal);
        }
        Ok(phot)
    }

    pub(in crate::cmd::analysis) fn sky_json(&self, phot: &StarPhotometry) -> serde_json::Value {
        match &self.wcs {
            Some(wcs) => {
                let coord = wcs.pixel_to_world(phot.x, phot.y);
                if coord.ra.is_finite() && coord.dec.is_finite() {
                    json!({ RES_RA: coord.ra, RES_DEC: coord.dec })
                } else {
                    serde_json::Value::Null
                }
            }
            None => serde_json::Value::Null,
        }
    }

    pub(in crate::cmd::analysis) fn photcal_json(&self) -> anyhow::Result<serde_json::Value> {
        match &self.photcal {
            Some(cal) => photcal_json(cal),
            None => Ok(serde_json::Value::Null),
        }
    }
}
