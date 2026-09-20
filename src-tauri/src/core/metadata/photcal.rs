use serde::Serialize;

use crate::core::astrometry::wcs::WcsTransform;
use crate::types::header::HduHeader;

pub const ARCSEC_PER_RADIAN: f64 = 206_264.806_247_096_36;
pub const AB_MAG_ZERO_POINT: f64 = 8.90;
pub const HST_DEFAULT_PHOTZPT: f64 = -21.10;
pub const ST_TO_AB_OFFSET: f64 = 18.692;
pub const MAG_ERR_PER_RELATIVE_FLUX_ERR: f64 = 1.085_736_204_758_129;
pub const JY_PER_MJY: f64 = 1.0e6;

pub const GENERIC_ZERO_POINT_KEYS: [&str; 5] = ["MAGZERO", "MAGZPT", "ZPT", "PHOTZP", "ZEROPT"];
pub const ROMAN_CONVERSION_MJY_KEYS: [&str; 2] = [
    "ROMAN_META_PHOTOMETRY_CONVERSION_MEGAJANSKYS",
    "ROMAN_META_PHOTOMETRY_CONVERSION_MEGAJANSKYS_VALUE",
];
pub const ROMAN_PIXEL_AREA_SR_KEYS: [&str; 2] = [
    "ROMAN_META_PHOTOMETRY_PIXELAREA_STERADIANS",
    "ROMAN_META_PHOTOMETRY_PIXELAREA_STERADIANS_VALUE",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FluxConvention {
    JwstMjySr {
        pixar_sr: f64,
        pixar_a2: Option<f64>,
        derived_pixar: bool,
    },
    JwstDnPerSec {
        photmjsr: f64,
        pixar_sr: f64,
    },
    HstCounts {
        photflam: f64,
        photplam: f64,
        photzpt: f64,
        per_second: bool,
        exptime: Option<f64>,
    },
    RomanDnPerSec {
        mjy_per_dn_s: f64,
        pixar_sr: f64,
    },
    GenericZeroPoint {
        zp: f64,
        keyword: String,
        per_second: bool,
        exptime: Option<f64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PhotCal {
    pub convention: FluxConvention,
    pub bunit: Option<String>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CalibratedFlux {
    pub flux_jy: f64,
    pub flux_err_jy: Option<f64>,
    pub mag_ab: Option<f64>,
    pub mag_ab_err: Option<f64>,
    pub st_mag: Option<f64>,
}

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    let cleaned = header.get(key)?.trim().trim_matches('\'').trim();
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

fn card_f64(header: &HduHeader, key: &str) -> Option<f64> {
    header.get_f64(key).filter(|v| v.is_finite())
}

fn first_card_f64<'a>(header: &HduHeader, keys: impl IntoIterator<Item = &'a str>) -> Option<(&'a str, f64)> {
    keys.into_iter().find_map(|key| card_f64(header, key).map(|v| (key, v)))
}

fn normalized_unit(bunit: &str) -> String {
    bunit.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase()
}

fn is_mjy_per_sr(unit: &str) -> bool {
    unit.contains("MJY/SR")
}

fn is_dn_per_second(unit: &str) -> bool {
    unit.starts_with("DN/S")
}

fn is_rate_unit(unit: &str) -> bool {
    unit.contains("/S")
}

fn wcs_pixel_scale_arcsec(wcs: Option<&WcsTransform>) -> Option<f64> {
    wcs.map(WcsTransform::pixel_scale_arcsec).filter(|s| s.is_finite() && *s > 0.0)
}

fn pixel_area_sr_from_scale(scale_arcsec: f64) -> f64 {
    (scale_arcsec / ARCSEC_PER_RADIAN).powi(2)
}

struct PixelArea {
    steradians: f64,
    derived: bool,
}

fn pixel_area_sr(
    header: &HduHeader,
    keys: &[&str],
    wcs: Option<&WcsTransform>,
    warnings: &mut Vec<String>,
) -> Option<PixelArea> {
    if let Some((_, sr)) = first_card_f64(header, keys.iter().copied()).filter(|(_, v)| *v > 0.0) {
        return Some(PixelArea { steradians: sr, derived: false });
    }
    let scale = wcs_pixel_scale_arcsec(wcs)?;
    warnings.push(format!(
        "{} missing; pixel area derived from the WCS pixel scale ({:.4} arcsec/px)",
        keys[0], scale
    ));
    Some(PixelArea { steradians: pixel_area_sr_from_scale(scale), derived: true })
}

impl PhotCal {
    pub fn from_header(header: &HduHeader, wcs: Option<&WcsTransform>) -> Option<PhotCal> {
        let bunit = card_text(header, "BUNIT");
        let unit = bunit.as_deref().map(normalized_unit).unwrap_or_default();
        let mut notes = Vec::new();
        let mut warnings = Vec::new();

        let convention = if is_mjy_per_sr(&unit) {
            let area = pixel_area_sr(header, &["PIXAR_SR"], wcs, &mut warnings)?;
            let pixar_a2 = match card_f64(header, "PIXAR_A2").filter(|v| *v > 0.0) {
                Some(a2) => Some(a2),
                None => wcs_pixel_scale_arcsec(wcs).map(|s| {
                    notes.push(format!("PIXAR_A2 missing; pixel area {:.6} arcsec^2 taken from the WCS scale", s * s));
                    s * s
                }),
            };
            notes.push("JWST MJy/sr: flux [Jy] = sum(MJy/sr) * PIXAR_SR * 1e6".into());
            if header.get("PHOTMJSR").is_some() {
                notes.push("PHOTMJSR present but not applied: the data are already in MJy/sr".into());
            }
            FluxConvention::JwstMjySr { pixar_sr: area.steradians, pixar_a2, derived_pixar: area.derived }
        } else if is_dn_per_second(&unit) && first_card_f64(header, ROMAN_CONVERSION_MJY_KEYS).is_some() {
            let (key, mjy_per_dn_s) = first_card_f64(header, ROMAN_CONVERSION_MJY_KEYS)?;
            let mut area_keys: Vec<&str> = ROMAN_PIXEL_AREA_SR_KEYS.to_vec();
            area_keys.push("PIXAR_SR");
            let area = pixel_area_sr(header, &area_keys, wcs, &mut warnings)?;
            notes.push(format!(
                "Roman DN/s: flux [Jy] = sum(DN/s) * {} * pixel area [sr] * 1e6",
                key
            ));
            FluxConvention::RomanDnPerSec { mjy_per_dn_s, pixar_sr: area.steradians }
        } else if is_dn_per_second(&unit) && card_f64(header, "PHOTMJSR").is_some() {
            let photmjsr = card_f64(header, "PHOTMJSR")?;
            let area = pixel_area_sr(header, &["PIXAR_SR"], wcs, &mut warnings)?;
            notes.push("JWST DN/s: flux [Jy] = sum(DN/s) * PHOTMJSR * PIXAR_SR * 1e6".into());
            FluxConvention::JwstDnPerSec { photmjsr, pixar_sr: area.steradians }
        } else if let (Some(photflam), Some(photplam)) = (
            card_f64(header, "PHOTFLAM").filter(|v| *v > 0.0),
            card_f64(header, "PHOTPLAM").filter(|v| *v > 0.0),
        ) {
            let photzpt = match card_f64(header, "PHOTZPT") {
                Some(z) => z,
                None => {
                    notes.push(format!("PHOTZPT missing; using the HST default {HST_DEFAULT_PHOTZPT}"));
                    HST_DEFAULT_PHOTZPT
                }
            };
            let per_second = is_rate_unit(&unit);
            let exptime = card_f64(header, "EXPTIME").filter(|v| *v > 0.0);
            if bunit.is_none() {
                notes.push("BUNIT missing; HST counts are assumed not to be a rate".into());
            }
            if !per_second && exptime.is_none() {
                warnings.push(format!(
                    "EXPTIME missing; counts in '{}' cannot be converted to a rate, magnitudes are unavailable",
                    bunit.as_deref().unwrap_or("?")
                ));
            }
            notes.push("HST: STmag = -2.5 log10(rate * PHOTFLAM) + PHOTZPT; ABmag = STmag - 5 log10(PHOTPLAM) + 18.692".into());
            FluxConvention::HstCounts { photflam, photplam, photzpt, per_second, exptime }
        } else if let Some((keyword, zp)) = first_card_f64(header, GENERIC_ZERO_POINT_KEYS) {
            let per_second = is_rate_unit(&unit);
            let exptime = card_f64(header, "EXPTIME").filter(|v| *v > 0.0);
            let mut note = format!(
                "zero point {keyword} = {zp} assumed to apply to the image's native units (BUNIT '{}')",
                bunit.as_deref().unwrap_or("unknown")
            );
            if let (false, Some(t)) = (per_second, exptime) {
                note.push_str(&format!("; EXPTIME {t} s is not applied"));
            }
            notes.push(note);
            FluxConvention::GenericZeroPoint { zp, keyword: keyword.to_string(), per_second, exptime }
        } else {
            return None;
        };

        Some(PhotCal { convention, bunit, notes, warnings })
    }

    pub fn label(&self) -> String {
        match &self.convention {
            FluxConvention::JwstMjySr { derived_pixar, .. } => format!(
                "JWST MJy/sr, PIXAR_SR {}",
                if *derived_pixar { "derived from the WCS" } else { "from header" }
            ),
            FluxConvention::JwstDnPerSec { .. } => "JWST DN/s via PHOTMJSR and PIXAR_SR".into(),
            FluxConvention::HstCounts { per_second, .. } => format!(
                "HST PHOTFLAM/PHOTPLAM ({}), STmag to ABmag",
                if *per_second { "count rate" } else { "counts / EXPTIME" }
            ),
            FluxConvention::RomanDnPerSec { .. } => "Roman DN/s via conversion_megajanskys and pixel area".into(),
            FluxConvention::GenericZeroPoint { keyword, .. } => format!("zero point {keyword} in native units"),
        }
    }

    fn jansky_per_native_unit(&self) -> Option<f64> {
        let factor = match &self.convention {
            FluxConvention::JwstMjySr { pixar_sr, .. } => pixar_sr * JY_PER_MJY,
            FluxConvention::JwstDnPerSec { photmjsr, pixar_sr } => photmjsr * pixar_sr * JY_PER_MJY,
            FluxConvention::RomanDnPerSec { mjy_per_dn_s, pixar_sr } => mjy_per_dn_s * pixar_sr * JY_PER_MJY,
            FluxConvention::HstCounts { photflam, photplam, photzpt, .. } => {
                self.rate_per_native_unit()?
                    * photflam
                    * 10f64.powf((AB_MAG_ZERO_POINT - photzpt - ST_TO_AB_OFFSET) / 2.5)
                    * photplam.powi(2)
            }
            FluxConvention::GenericZeroPoint { zp, .. } => 10f64.powf((AB_MAG_ZERO_POINT - zp) / 2.5),
        };
        (factor.is_finite() && factor > 0.0).then_some(factor)
    }

    fn rate_per_native_unit(&self) -> Option<f64> {
        match &self.convention {
            FluxConvention::HstCounts { per_second: true, .. } => Some(1.0),
            FluxConvention::HstCounts { per_second: false, exptime, .. } => exptime.map(|t| 1.0 / t),
            _ => Some(1.0),
        }
    }

    fn mjy_per_sr_per_native_unit(&self) -> Option<f64> {
        match &self.convention {
            FluxConvention::JwstMjySr { .. } => Some(1.0),
            FluxConvention::JwstDnPerSec { photmjsr, .. } => Some(*photmjsr),
            FluxConvention::RomanDnPerSec { mjy_per_dn_s, .. } => Some(*mjy_per_dn_s),
            _ => None,
        }
    }

    pub fn calibrate(&self, net: f64, net_err: Option<f64>) -> Option<CalibratedFlux> {
        if !net.is_finite() {
            return None;
        }
        let factor = self.jansky_per_native_unit()?;
        let flux_jy = net * factor;
        let flux_err_jy = net_err.filter(|e| e.is_finite() && *e >= 0.0).map(|e| e * factor);
        let positive = flux_jy > 0.0;
        let mag_ab = positive.then(|| -2.5 * flux_jy.log10() + AB_MAG_ZERO_POINT);
        let mag_ab_err = match (positive, flux_err_jy) {
            (true, Some(e)) => Some(MAG_ERR_PER_RELATIVE_FLUX_ERR * e / flux_jy),
            _ => None,
        };
        let st_mag = match (&self.convention, positive) {
            (FluxConvention::HstCounts { photflam, photzpt, .. }, true) => {
                let rate = net * self.rate_per_native_unit()?;
                Some(-2.5 * (rate * photflam).log10() + photzpt)
            }
            _ => None,
        };
        Some(CalibratedFlux { flux_jy, flux_err_jy, mag_ab, mag_ab_err, st_mag })
    }

    pub fn surface_brightness_ab_per_arcsec2(&self, pixel_value: f64) -> Option<f64> {
        if !pixel_value.is_finite() || pixel_value <= 0.0 {
            return None;
        }
        let mjy_per_sr = pixel_value * self.mjy_per_sr_per_native_unit()?;
        let jy_per_arcsec2 = match &self.convention {
            FluxConvention::JwstMjySr { pixar_sr, pixar_a2: Some(a2), .. } if *a2 > 0.0 => {
                mjy_per_sr * pixar_sr * JY_PER_MJY / a2
            }
            _ => mjy_per_sr * JY_PER_MJY / (ARCSEC_PER_RADIAN * ARCSEC_PER_RADIAN),
        };
        (jy_per_arcsec2 > 0.0).then(|| -2.5 * jy_per_arcsec2.log10() + AB_MAG_ZERO_POINT)
    }
}

pub fn missing_calibration_reason(header: Option<&HduHeader>) -> String {
    let Some(header) = header else {
        return "no header available: photometric calibration requires BUNIT and zero-point cards".into();
    };
    let bunit = card_text(header, "BUNIT");
    let unit = bunit.as_deref().map(normalized_unit).unwrap_or_default();
    if is_mjy_per_sr(&unit) {
        return "BUNIT is MJy/sr but PIXAR_SR is missing and no WCS pixel scale is available".into();
    }
    if is_dn_per_second(&unit) {
        return format!(
            "BUNIT '{}' needs PHOTMJSR (JWST) or {} (Roman) plus a pixel area to calibrate",
            bunit.as_deref().unwrap_or(""),
            ROMAN_CONVERSION_MJY_KEYS[0]
        );
    }
    if card_f64(header, "PHOTFLAM").is_some() || card_f64(header, "PHOTPLAM").is_some() {
        return "HST calibration needs both PHOTFLAM and PHOTPLAM".into();
    }
    format!(
        "no photometric calibration in the header (BUNIT '{}'): none of {} / PHOTFLAM+PHOTPLAM / PHOTMJSR / MJy/sr found",
        bunit.as_deref().unwrap_or("missing"),
        GENERIC_ZERO_POINT_KEYS.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::region::test_support::{make_header, wcs_cards};

    fn jwst_mjy_sr_header() -> HduHeader {
        make_header(&[
            ("TELESCOP", "JWST"),
            ("INSTRUME", "NIRCAM"),
            ("BUNIT", "MJy/sr"),
            ("PIXAR_SR", "2.1E-13"),
            ("PIXAR_A2", "8.9E-4"),
            ("PHOTMJSR", "0.5"),
        ])
    }

    fn ab_from_jy(flux_jy: f64) -> f64 {
        -2.5 * flux_jy.log10() + AB_MAG_ZERO_POINT
    }

    #[test]
    fn jwst_mjy_sr_sums_pixel_area_and_ignores_photmjsr() {
        let cal = PhotCal::from_header(&jwst_mjy_sr_header(), None).expect("jwst calibration");
        match &cal.convention {
            FluxConvention::JwstMjySr { pixar_sr, pixar_a2, derived_pixar } => {
                assert!((pixar_sr - 2.1e-13).abs() < 1e-25);
                assert_eq!(*pixar_a2, Some(8.9e-4));
                assert!(!derived_pixar);
            }
            other => panic!("unexpected convention {other:?}"),
        }
        assert_eq!(cal.bunit.as_deref(), Some("MJy/sr"));
        assert!(cal.warnings.is_empty(), "{:?}", cal.warnings);
        assert!(cal.notes.iter().any(|n| n.contains("PHOTMJSR")), "{:?}", cal.notes);
        assert!(cal.label().contains("JWST MJy/sr"), "{}", cal.label());
        assert!(cal.label().contains("from header"), "{}", cal.label());

        let flux = cal.calibrate(1000.0, None).expect("calibrated");
        assert!((flux.flux_jy - 2.1e-4).abs() < 1e-12, "flux_jy={}", flux.flux_jy);
        let mag = flux.mag_ab.expect("mag");
        assert!((mag - 18.09).abs() < 0.01, "mag_ab={mag}");
        assert!(flux.flux_err_jy.is_none() && flux.mag_ab_err.is_none() && flux.st_mag.is_none());
    }

    #[test]
    fn jwst_dn_per_second_applies_photmjsr_before_pixel_area() {
        let header = make_header(&[
            ("BUNIT", "DN/s"),
            ("PHOTMJSR", "0.5"),
            ("PIXAR_SR", "2.1E-13"),
        ]);
        let cal = PhotCal::from_header(&header, None).expect("dn/s calibration");
        assert!(matches!(
            cal.convention,
            FluxConvention::JwstDnPerSec { photmjsr, pixar_sr } if photmjsr == 0.5 && pixar_sr == 2.1e-13
        ));
        let flux = cal.calibrate(1000.0, None).unwrap();
        assert!((flux.flux_jy - 1.05e-4).abs() < 1e-12, "flux_jy={}", flux.flux_jy);
        assert!((flux.mag_ab.unwrap() - ab_from_jy(1.05e-4)).abs() < 1e-9);
        assert!(cal.label().contains("PHOTMJSR"));
    }

    #[test]
    fn hst_electrons_with_exptime_matches_hand_computed_st_and_ab_mags() {
        let header = make_header(&[
            ("TELESCOP", "HST"),
            ("INSTRUME", "WFC3"),
            ("BUNIT", "ELECTRONS"),
            ("PHOTFLAM", "1.9E-19"),
            ("PHOTPLAM", "5300"),
            ("EXPTIME", "600"),
        ]);
        let cal = PhotCal::from_header(&header, None).expect("hst calibration");
        match &cal.convention {
            FluxConvention::HstCounts { photflam, photplam, photzpt, per_second, exptime } => {
                assert_eq!(*photflam, 1.9e-19);
                assert_eq!(*photplam, 5300.0);
                assert_eq!(*photzpt, HST_DEFAULT_PHOTZPT);
                assert!(!per_second);
                assert_eq!(*exptime, Some(600.0));
            }
            other => panic!("unexpected convention {other:?}"),
        }
        assert!(cal.warnings.is_empty(), "{:?}", cal.warnings);

        let flux = cal.calibrate(60_000.0, None).expect("calibrated");
        let st = flux.st_mag.expect("st mag");
        assert!((st - 20.7031).abs() < 0.001, "st_mag={st}");
        let ab = flux.mag_ab.expect("ab mag");
        assert!((ab - 20.7737).abs() < 0.001, "mag_ab={ab}");
        assert!((flux.flux_jy - 1.7804e-5).abs() / 1.7804e-5 < 1e-3, "flux_jy={}", flux.flux_jy);
        assert!((ab_from_jy(flux.flux_jy) - ab).abs() < 1e-9);

        let negative = cal.calibrate(-600.0, Some(60.0)).expect("negative net still calibrates the flux");
        assert!(negative.flux_jy < 0.0);
        assert!(negative.mag_ab.is_none() && negative.st_mag.is_none() && negative.mag_ab_err.is_none());
        assert!((negative.flux_err_jy.unwrap() - flux.flux_jy * 60.0 / 60_000.0).abs() < 1e-15);
    }

    #[test]
    fn hst_count_rate_units_skip_exptime_and_missing_exptime_warns() {
        let per_second = make_header(&[
            ("BUNIT", "ELECTRONS/S"),
            ("PHOTFLAM", "1.9E-19"),
            ("PHOTPLAM", "5300"),
            ("PHOTZPT", "-21.1"),
            ("EXPTIME", "600"),
        ]);
        let cal = PhotCal::from_header(&per_second, None).unwrap();
        assert!(matches!(cal.convention, FluxConvention::HstCounts { per_second: true, .. }));
        let rate = cal.calibrate(100.0, None).unwrap();
        assert!((rate.st_mag.unwrap() - 20.7031).abs() < 0.001);

        let counts_per_s = make_header(&[("BUNIT", "COUNTS/S"), ("PHOTFLAM", "1.9E-19"), ("PHOTPLAM", "5300")]);
        assert!(matches!(
            PhotCal::from_header(&counts_per_s, None).unwrap().convention,
            FluxConvention::HstCounts { per_second: true, .. }
        ));

        let no_exptime = make_header(&[("BUNIT", "ELECTRONS"), ("PHOTFLAM", "1.9E-19"), ("PHOTPLAM", "5300")]);
        let cal = PhotCal::from_header(&no_exptime, None).unwrap();
        assert!(matches!(cal.convention, FluxConvention::HstCounts { per_second: false, exptime: None, .. }));
        assert!(cal.warnings.iter().any(|w| w.contains("EXPTIME")), "{:?}", cal.warnings);
        assert!(cal.calibrate(100.0, None).is_none());
    }

    #[test]
    fn roman_mangled_keys_from_the_asdf_bridge_are_recognised() {
        let plain = make_header(&[
            ("BUNIT", "DN / s"),
            ("ROMAN_META_TELESCOPE", "ROMAN"),
            ("ROMAN_META_PHOTOMETRY_CONVERSION_MEGAJANSKYS", "0.3324"),
            ("ROMAN_META_PHOTOMETRY_PIXELAREA_STERADIANS", "2.8E-13"),
        ]);
        let cal = PhotCal::from_header(&plain, None).expect("roman calibration");
        assert!(matches!(
            cal.convention,
            FluxConvention::RomanDnPerSec { mjy_per_dn_s, pixar_sr } if mjy_per_dn_s == 0.3324 && pixar_sr == 2.8e-13
        ));
        let flux = cal.calibrate(1000.0, None).unwrap();
        assert!((flux.flux_jy - 1000.0 * 0.3324 * 2.8e-13 * 1e6).abs() < 1e-15);
        assert!(cal.label().to_lowercase().contains("roman"));

        let quantity = make_header(&[
            ("BUNIT", "DN / s"),
            ("ROMAN_META_PHOTOMETRY_CONVERSION_MEGAJANSKYS_UNIT", "MJy.sr**-1"),
            ("ROMAN_META_PHOTOMETRY_CONVERSION_MEGAJANSKYS_VALUE", "0.3324"),
            ("ROMAN_META_PHOTOMETRY_PIXELAREA_STERADIANS_UNIT", "sr"),
            ("ROMAN_META_PHOTOMETRY_PIXELAREA_STERADIANS_VALUE", "2.8E-13"),
        ]);
        assert!(matches!(
            PhotCal::from_header(&quantity, None).unwrap().convention,
            FluxConvention::RomanDnPerSec { mjy_per_dn_s, pixar_sr } if mjy_per_dn_s == 0.3324 && pixar_sr == 2.8e-13
        ));

        let wrong_unit = make_header(&[
            ("BUNIT", "MJy/sr"),
            ("ROMAN_META_PHOTOMETRY_CONVERSION_MEGAJANSKYS", "0.3324"),
            ("PIXAR_SR", "2.8E-13"),
        ]);
        assert!(matches!(
            PhotCal::from_header(&wrong_unit, None).unwrap().convention,
            FluxConvention::JwstMjySr { .. }
        ));
    }

    #[test]
    fn roman_keys_match_the_names_produced_by_the_asdf_bridge() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"#ASDF 1.0.0\n#ASDF_STANDARD 1.5.0\n%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n");
        bytes.extend_from_slice(
            b"roman:\n  meta:\n    telescope: ROMAN\n    bunit: DN / s\n    photometry:\n      conversion_megajanskys: !unit/quantity-1.1.0\n        value: 0.3324\n        unit: !unit/unit-1.0.0 MJy.sr**-1\n      pixelarea_steradians: !unit/quantity-1.1.0\n        value: 2.8e-13\n        unit: !unit/unit-1.0.0 sr\n  data: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n",
        );
        bytes.extend_from_slice(b"...\n");
        let path = dir.path().join("roman.asdf");
        std::fs::write(&path, bytes).unwrap();
        let loaded = crate::infra::asdf_bridge::extract_image_from_asdf(&path).unwrap();
        let cal = PhotCal::from_header(&loaded.header, None).expect("roman calibration from the bridge header");
        match cal.convention {
            FluxConvention::RomanDnPerSec { mjy_per_dn_s, pixar_sr } => {
                assert!((mjy_per_dn_s - 0.3324).abs() < 1e-12);
                assert!((pixar_sr - 2.8e-13).abs() < 1e-25);
            }
            other => panic!("unexpected convention {other:?}"),
        }
    }

    #[test]
    fn generic_zero_point_keywords_note_the_native_unit_assumption() {
        let header = make_header(&[("BUNIT", "ADU"), ("EXPTIME", "300"), ("MAGZPT", "25.0")]);
        let cal = PhotCal::from_header(&header, None).expect("generic calibration");
        match &cal.convention {
            FluxConvention::GenericZeroPoint { zp, keyword, per_second, exptime } => {
                assert_eq!(*zp, 25.0);
                assert_eq!(keyword, "MAGZPT");
                assert!(!per_second);
                assert_eq!(*exptime, Some(300.0));
            }
            other => panic!("unexpected convention {other:?}"),
        }
        assert!(cal.notes.iter().any(|n| n.contains("MAGZPT") && n.contains("native")), "{:?}", cal.notes);
        let flux = cal.calibrate(10_000.0, None).unwrap();
        assert!((flux.mag_ab.unwrap() - 15.0).abs() < 1e-9);
        assert!((flux.flux_jy - 10f64.powf((8.90 - 15.0) / 2.5)).abs() < 1e-12);

        let per_second = make_header(&[("BUNIT", "e-/s"), ("ZEROPT", "24.5")]);
        assert!(matches!(
            PhotCal::from_header(&per_second, None).unwrap().convention,
            FluxConvention::GenericZeroPoint { per_second: true, exptime: None, .. }
        ));

        let first_found = make_header(&[("ZPT", "20.0"), ("MAGZERO", "21.0")]);
        assert!(matches!(
            PhotCal::from_header(&first_found, None).unwrap().convention,
            FluxConvention::GenericZeroPoint { zp, .. } if zp == 21.0
        ));

        assert!(PhotCal::from_header(&make_header(&[("BUNIT", "ADU")]), None).is_none());
        assert!(PhotCal::from_header(&HduHeader::empty(), None).is_none());
        assert!(missing_calibration_reason(Some(&make_header(&[("BUNIT", "ADU")]))).contains("ADU"));
        assert!(!missing_calibration_reason(None).is_empty());
    }

    #[test]
    fn missing_pixar_sr_is_derived_from_the_wcs_pixel_scale_with_a_warning() {
        let scale_deg = 0.031 / 3600.0;
        let mut cards = wcs_cards([[-scale_deg, 0.0], [0.0, scale_deg]]);
        cards.push(("BUNIT".into(), "MJy/sr".into()));
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let header = make_header(&pairs);
        let wcs = WcsTransform::from_header(&header).unwrap();
        assert!((wcs.pixel_scale_arcsec() - 0.031).abs() < 1e-9);

        let cal = PhotCal::from_header(&header, Some(&wcs)).expect("derived calibration");
        let expected_sr = (0.031 / ARCSEC_PER_RADIAN).powi(2);
        match &cal.convention {
            FluxConvention::JwstMjySr { pixar_sr, pixar_a2, derived_pixar } => {
                assert!((pixar_sr - expected_sr).abs() / expected_sr < 1e-6, "pixar_sr={pixar_sr}");
                assert!(*derived_pixar);
                let a2 = pixar_a2.expect("pixel area from the WCS");
                assert!((a2 - 0.031 * 0.031).abs() < 1e-9);
            }
            other => panic!("unexpected convention {other:?}"),
        }
        assert!(cal.warnings.iter().any(|w| w.contains("PIXAR_SR")), "{:?}", cal.warnings);
        assert!(cal.label().contains("derived"), "{}", cal.label());

        assert!(PhotCal::from_header(&make_header(&[("BUNIT", "MJy/sr")]), None).is_none());
        assert!(missing_calibration_reason(Some(&make_header(&[("BUNIT", "MJy/sr")]))).contains("PIXAR_SR"));
    }

    #[test]
    fn magnitude_error_follows_the_relative_flux_error() {
        let cal = PhotCal::from_header(&jwst_mjy_sr_header(), None).unwrap();
        let flux = cal.calibrate(1000.0, Some(100.0)).unwrap();
        let err_jy = flux.flux_err_jy.unwrap();
        assert!((err_jy - 2.1e-5).abs() < 1e-15);
        let mag_err = flux.mag_ab_err.unwrap();
        assert!((mag_err - MAG_ERR_PER_RELATIVE_FLUX_ERR * 0.1).abs() < 1e-9, "mag_err={mag_err}");
        assert!((mag_err - 0.10857).abs() < 1e-4);

        let zero = cal.calibrate(0.0, Some(1.0)).unwrap();
        assert_eq!(zero.flux_jy, 0.0);
        assert!(zero.mag_ab.is_none() && zero.mag_ab_err.is_none());
        assert!(cal.calibrate(f64::NAN, None).is_none());
        assert!(cal.calibrate(1.0, Some(f64::NAN)).unwrap().flux_err_jy.is_none());
    }

    #[test]
    fn surface_brightness_uses_the_pixel_area_in_arcsec2() {
        let cal = PhotCal::from_header(&jwst_mjy_sr_header(), None).unwrap();
        let sb = cal.surface_brightness_ab_per_arcsec2(1000.0).expect("surface brightness");
        let expected = ab_from_jy(1000.0 * 2.1e-13 * 1e6 / 8.9e-4);
        assert!((sb - expected).abs() < 1e-9, "sb={sb} expected={expected}");

        let no_a2 = make_header(&[("BUNIT", "MJy/sr"), ("PIXAR_SR", "2.1E-13")]);
        let cal = PhotCal::from_header(&no_a2, None).unwrap();
        let sb = cal.surface_brightness_ab_per_arcsec2(1000.0).unwrap();
        let exact = ab_from_jy(1000.0 * 1e6 / (ARCSEC_PER_RADIAN * ARCSEC_PER_RADIAN));
        assert!((sb - exact).abs() < 1e-9, "sb={sb} exact={exact}");
        assert!(cal.surface_brightness_ab_per_arcsec2(0.0).is_none());
        assert!(cal.surface_brightness_ab_per_arcsec2(-1.0).is_none());

        let hst = make_header(&[("BUNIT", "ELECTRONS/S"), ("PHOTFLAM", "1.9E-19"), ("PHOTPLAM", "5300")]);
        assert!(PhotCal::from_header(&hst, None).unwrap().surface_brightness_ab_per_arcsec2(1.0).is_none());
    }

    #[test]
    fn serialization_tags_the_convention_kind() {
        let cal = PhotCal::from_header(&jwst_mjy_sr_header(), None).unwrap();
        let json = serde_json::to_value(&cal).unwrap();
        assert_eq!(json["convention"]["kind"], "jwst_mjy_sr");
        assert_eq!(json["convention"]["derived_pixar"], false);
        assert_eq!(json["bunit"], "MJy/sr");
        assert!(json["notes"].is_array() && json["warnings"].is_array());
    }
}
