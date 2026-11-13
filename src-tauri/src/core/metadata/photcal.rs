use serde::{Deserialize, Serialize};

use crate::core::analysis::gain::{classify_unit, UnitClass};
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::imaging::asinh::{BOFFSET_CARD, BSOFTEN_CARD};
use crate::core::metadata::units::{self, per_pixel_jansky_scale, JY_PER_NANOMAGGY};
use crate::types::header::HduHeader;

pub const ARCSEC_PER_RADIAN: f64 = 206_264.806_247_096_36;
pub const AB_MAG_ZERO_POINT: f64 = 8.90;
pub const HST_DEFAULT_PHOTZPT: f64 = -21.10;
pub const ST_TO_AB_OFFSET: f64 = 18.692;
pub const MAG_ERR_PER_RELATIVE_FLUX_ERR: f64 = 1.085_736_204_758_129;
pub const JY_PER_MJY: f64 = 1.0e6;

pub const GENERIC_ZERO_POINT_KEYS: [&str; 5] = ["MAGZERO", "MAGZPT", "ZPT", "PHOTZP", "ZEROPT"];
pub const SURVEY_ZERO_POINT_KEYS: [&str; 2] = [FLUXMAG0_KEY, FPA_ZP_KEY];
pub const USER_ZERO_POINT_KEYWORD: &str = "user";
pub const USER_ZERO_POINT_WARNING: &str = "calibrated with a user zero point";
pub const ROMAN_CONVERSION_MJY_KEYS: [&str; 2] = [
    "ROMAN_META_PHOTOMETRY_CONVERSION_MEGAJANSKYS",
    "ROMAN_META_PHOTOMETRY_CONVERSION_MEGAJANSKYS_VALUE",
];
pub const ROMAN_PIXEL_AREA_SR_KEYS: [&str; 2] = [
    "ROMAN_META_PHOTOMETRY_PIXELAREA_STERADIANS",
    "ROMAN_META_PHOTOMETRY_PIXELAREA_STERADIANS_VALUE",
];

const MJY_PER_SR_UNITS: [&str; 6] = ["MJY/SR", "MJYSR-1", "MJY.SR-1", "MJY.SR**-1", "MJYSR**-1", "MJY/STERADIAN"];
const PIXEL_AREA_TOLERANCE_RATIO: f64 = 1.10;
const JWST_MJY_SR_NOTE: &str = "JWST MJy/sr: flux [Jy] = sum(MJy/sr) * PIXAR_SR * 1e6";
const MAG_SYSTEM_CARDS: [&str; 2] = ["PHOTSYS", "MAGSYS"];
const STORED_VALUE_ZERO_POINT_KEYS: [&str; 2] = ["MAGZERO", "PHOTZP"];
const PER_SECOND_ZERO_POINT_KEYS: [&str; 1] = ["MAGZPT"];
const UNIT_ZERO_POINT_TOLERANCE_MAG: f64 = 0.005;
const ZERO_POINT_TEXT_DECIMALS_SCALE: f64 = 1e4;
const FLUXMAG0_KEY: &str = "FLUXMAG0";
const FPA_ZP_KEY: &str = "HIERARCH FPA.ZP";
const HIERARCH_PREFIX: &str = "HIERARCH ";
const FPA_ZP_SOURCE: &str = "FPA.ZP + 2.5 log10(EXPTIME)";
const LEGACY_SURVEY_NAME: &str = "LegacySurvey";
const NANOMAGGY_UNIT: &str = "nanomaggy";
const APP_PRODUCT_CARD: &str = "ABPROC";
const NON_SCIENCE_CARD_VALUES: [(&str, &[&str]); 4] = [
    ("RESAMPT1", &["FLAGS"]),
    ("DES_EXT", &["MASK", "WEIGHT"]),
    ("IMAGETYP", &["INVVAR", "MASKBITS"]),
    ("IMTYPE", &["INVVAR", "MASKBITS"]),
];
const PAN_STARRS_MASK_COUNT_CARD: &str = "MSKNUM";
const NON_SCIENCE_EXTNAMES: [&str; 11] =
    ["MASK", "MSK", "FLAG", "MASKBITS", "DQ", "VARIANCE", "VAR", "IVAR", "INVVAR", "WGT", "WEIGHT"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MagSystem {
    Ab,
    Vega,
    St,
    Unknown,
}

impl MagSystem {
    fn from_card_value(value: &str) -> MagSystem {
        match value.trim().to_uppercase().as_str() {
            "AB" | "ABMAG" | "ABNU" => MagSystem::Ab,
            "VEGA" | "VEGAMAG" => MagSystem::Vega,
            "ST" | "STMAG" => MagSystem::St,
            _ => MagSystem::Unknown,
        }
    }

    fn name(self) -> &'static str {
        match self {
            MagSystem::Ab => "AB",
            MagSystem::Vega => "Vega",
            MagSystem::St => "ST",
            MagSystem::Unknown => "AB assumed",
        }
    }

    fn gives_ab_magnitudes(self) -> bool {
        matches!(self, MagSystem::Ab | MagSystem::Unknown)
    }
}

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
        system: MagSystem,
        exptime_applied: bool,
    },
    FluxDensityPerPixel {
        jy_per_unit: f64,
        unit: String,
    },
    SurveyZeroPoint {
        zp_ab: f64,
        source: String,
        exptime_applied: Option<f64>,
    },
}

impl FluxConvention {
    fn mag_system(&self) -> MagSystem {
        match self {
            FluxConvention::GenericZeroPoint { system, .. } => *system,
            _ => MagSystem::Ab,
        }
    }

    fn rate_per_native_unit(&self) -> Option<f64> {
        match self {
            FluxConvention::HstCounts { per_second: false, exptime, .. } => exptime.map(|t| 1.0 / t),
            FluxConvention::GenericZeroPoint { exptime_applied: true, exptime, .. } => exptime.map(|t| 1.0 / t),
            _ => Some(1.0),
        }
    }

    fn jansky_per_native_unit(&self) -> Option<f64> {
        let factor = match self {
            FluxConvention::JwstMjySr { pixar_sr, .. } => pixar_sr * JY_PER_MJY,
            FluxConvention::JwstDnPerSec { photmjsr, pixar_sr } => photmjsr * pixar_sr * JY_PER_MJY,
            FluxConvention::RomanDnPerSec { mjy_per_dn_s, pixar_sr } => mjy_per_dn_s * pixar_sr * JY_PER_MJY,
            FluxConvention::HstCounts { photflam, photplam, photzpt, .. } => {
                self.rate_per_native_unit()?
                    * photflam
                    * 10f64.powf((AB_MAG_ZERO_POINT - photzpt - ST_TO_AB_OFFSET) / 2.5)
                    * photplam.powi(2)
            }
            FluxConvention::GenericZeroPoint { zp, exptime, system, exptime_applied, .. } => {
                if !system.gives_ab_magnitudes() {
                    return None;
                }
                let per_stored_unit = 10f64.powf((AB_MAG_ZERO_POINT - zp) / 2.5);
                match exptime {
                    Some(t) if *exptime_applied => per_stored_unit / t,
                    _ => per_stored_unit,
                }
            }
            FluxConvention::FluxDensityPerPixel { jy_per_unit, .. } => *jy_per_unit,
            FluxConvention::SurveyZeroPoint { zp_ab, .. } => 10f64.powf((AB_MAG_ZERO_POINT - zp_ab) / 2.5),
        };
        (factor.is_finite() && factor > 0.0).then_some(factor)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PhotCal {
    pub convention: FluxConvention,
    pub bunit: Option<String>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
    pub area_applied: bool,
    pub origin: String,
    pub mag_system: MagSystem,
    pub jy_per_native_unit: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SystemMagnitude {
    pub mag: f64,
    pub mag_err: Option<f64>,
    pub system: MagSystem,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CalibratedFlux {
    pub flux_jy: f64,
    pub flux_err_jy: Option<f64>,
    pub mag_ab: Option<f64>,
    pub mag_ab_err: Option<f64>,
    pub st_mag: Option<f64>,
}

pub(crate) fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    let cleaned = header.get(key)?.trim().trim_matches('\'').trim();
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

fn card_f64(header: &HduHeader, key: &str) -> Option<f64> {
    header.get_f64(key).filter(|v| v.is_finite())
}

fn first_card_f64<'a>(header: &HduHeader, keys: impl IntoIterator<Item = &'a str>) -> Option<(&'a str, f64)> {
    keys.into_iter().find_map(|key| card_f64(header, key).map(|v| (key, v)))
}

pub(crate) fn normalized_unit(bunit: &str) -> String {
    bunit.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase()
}

pub(crate) fn is_mjy_per_sr(unit: &str) -> bool {
    let compact: String = unit.chars().filter(|c| !c.is_whitespace()).collect();
    compact.starts_with('M') && MJY_PER_SR_UNITS.contains(&compact.to_ascii_uppercase().as_str())
}

fn is_dn_per_second(unit: &str) -> bool {
    unit.starts_with("DN/S")
}

pub(crate) fn is_rate_unit(unit: &str) -> bool {
    unit.contains("/S")
}

fn positive_exptime(header: &HduHeader) -> Option<f64> {
    card_f64(header, "EXPTIME").filter(|v| *v > 0.0)
}

fn stated_mag_system(header: &HduHeader) -> Option<(&'static str, String)> {
    MAG_SYSTEM_CARDS.into_iter().find_map(|card| card_text(header, card).map(|value| (card, value)))
}

fn zero_point_formula(subject: &str, zp: f64, exptime_applied: bool) -> String {
    format!(
        "{subject}: mag = {zp:?} - 2.5 log10({})",
        if exptime_applied { "sum / EXPTIME" } else { "sum" }
    )
}

fn flux_density_zero_point(jy_per_unit: f64) -> f64 {
    AB_MAG_ZERO_POINT - 2.5 * jy_per_unit.log10()
}

fn zero_point_text(zp: f64) -> String {
    format!("{:?}", (zp * ZERO_POINT_TEXT_DECIMALS_SCALE).round() / ZERO_POINT_TEXT_DECIMALS_SCALE)
}

fn contradicts_unit_zero_point(card_zp: f64, unit_zp: f64) -> bool {
    (card_zp - unit_zp).abs() > UNIT_ZERO_POINT_TOLERANCE_MAG
}

fn flux_density_formula(unit: &str, jy_per_unit: f64) -> String {
    format!(
        "1 {unit} = {jy_per_unit:e} Jy; AB mag = {} - 2.5 log10(sum)",
        zero_point_text(flux_density_zero_point(jy_per_unit))
    )
}

fn non_science_card(header: &HduHeader, card: &str, values: &[&str]) -> Option<String> {
    let value = card_text(header, card)?;
    values.contains(&value.to_uppercase().as_str()).then(|| format!("{card} = '{value}'"))
}

fn non_science_layer(header: &HduHeader) -> Option<String> {
    NON_SCIENCE_CARD_VALUES
        .iter()
        .find_map(|(card, values)| non_science_card(header, card, values))
        .or_else(|| {
            header
                .get(PAN_STARRS_MASK_COUNT_CARD)
                .map(|_| format!("{PAN_STARRS_MASK_COUNT_CARD} card of a Pan-STARRS variance or mask cutout"))
        })
        .or_else(|| non_science_card(header, "EXTNAME", &NON_SCIENCE_EXTNAMES))
}

fn has_asinh_scaled_pixels(header: &HduHeader) -> bool {
    [BSOFTEN_CARD, BOFFSET_CARD].iter().all(|card| header.get(card).is_some())
}

fn is_legacy_cutout_without_unit(header: &HduHeader, bunit: Option<&str>) -> bool {
    bunit.is_none()
        && card_text(header, "SURVEY").is_some_and(|survey| survey.eq_ignore_ascii_case(LEGACY_SURVEY_NAME))
}

fn exposure_origin(applied_exptime: Option<f64>) -> String {
    applied_exptime.map(|t| format!(", counts divided by EXPTIME = {t:?} s")).unwrap_or_default()
}

fn wcs_pixel_area_sr(wcs: Option<&WcsTransform>) -> Option<f64> {
    let cd = wcs?.raw_params().4;
    let square_degrees = (cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0]).abs();
    let steradians = square_degrees * 1f64.to_radians().powi(2);
    (steradians.is_finite() && steradians > 0.0).then_some(steradians)
}

fn square_arcsec(steradians: f64) -> f64 {
    steradians * ARCSEC_PER_RADIAN.powi(2)
}

fn areas_disagree(card: f64, wcs: f64) -> bool {
    let ratio = card / wcs;
    ratio > PIXEL_AREA_TOLERANCE_RATIO || ratio < PIXEL_AREA_TOLERANCE_RATIO.recip()
}

#[cfg(feature = "tauri")]
pub(crate) fn is_relative_pixel_area(mean: f64) -> bool {
    mean.is_finite() && !areas_disagree(mean, 1.0)
}

pub(crate) struct PixelArea {
    pub(crate) steradians: f64,
    pub(crate) derived: bool,
    replaced_card: bool,
}

pub(crate) fn pixel_area_sr(
    header: &HduHeader,
    keys: &[&str],
    wcs: Option<&WcsTransform>,
    warnings: &mut Vec<String>,
) -> Option<PixelArea> {
    let from_wcs = wcs_pixel_area_sr(wcs);
    if let Some((key, card)) = first_card_f64(header, keys.iter().copied()).filter(|(_, v)| *v > 0.0) {
        return Some(match from_wcs {
            Some(steradians) if areas_disagree(card, steradians) => {
                warnings.push(format!(
                    "{key} = {card:.4e} sr is {:.3}x the WCS pixel area ({steradians:.4e} sr); the image was probably resampled after {key} was written, so the WCS pixel area is used",
                    card / steradians
                ));
                PixelArea { steradians, derived: true, replaced_card: true }
            }
            _ => PixelArea { steradians: card, derived: false, replaced_card: false },
        });
    }
    let steradians = from_wcs?;
    warnings.push(format!(
        "{} missing; pixel area derived from the WCS pixel scale ({:.4} arcsec/px)",
        keys[0],
        square_arcsec(steradians).sqrt()
    ));
    Some(PixelArea { steradians, derived: true, replaced_card: false })
}

impl PhotCal {
    pub fn from_header(header: &HduHeader, wcs: Option<&WcsTransform>) -> Option<PhotCal> {
        let bunit = card_text(header, "BUNIT");
        let unit = bunit.as_deref().map(normalized_unit).unwrap_or_default();
        if is_mjy_per_sr(&unit) && !bunit.as_deref().is_some_and(is_mjy_per_sr) {
            return None;
        }
        let mut notes = Vec::new();
        let mut warnings = Vec::new();
        let unit_text = bunit.as_deref().unwrap_or_default();

        let (convention, origin) = if bunit.as_deref().is_some_and(is_mjy_per_sr) {
            let area = pixel_area_sr(header, &["PIXAR_SR"], wcs, &mut warnings)?;
            let pixar_a2 = match card_f64(header, "PIXAR_A2").filter(|v| *v > 0.0) {
                Some(a2) if !area.replaced_card => Some(a2),
                card => wcs_pixel_area_sr(wcs).map(square_arcsec).map(|a2| {
                    notes.push(format!(
                        "PIXAR_A2 {}; pixel area {:.6} arcsec^2 taken from the WCS",
                        if card.is_some() { "replaced like PIXAR_SR" } else { "missing" },
                        a2
                    ));
                    a2
                }),
            };
            notes.push(JWST_MJY_SR_NOTE.into());
            if header.get("PHOTMJSR").is_some() {
                notes.push("PHOTMJSR present but not applied: the data are already in MJy/sr".into());
            }
            let origin = format!(
                "BUNIT {unit_text} with {} {:.4e} sr",
                if area.derived { "the WCS pixel area" } else { "PIXAR_SR =" },
                area.steradians
            );
            (FluxConvention::JwstMjySr { pixar_sr: area.steradians, pixar_a2, derived_pixar: area.derived }, origin)
        } else if let Some(jy_per_unit) = bunit.as_deref().and_then(per_pixel_jansky_scale) {
            let unit_zp = flux_density_zero_point(jy_per_unit);
            let zp_text = zero_point_text(unit_zp);
            let zero_point_card = first_card_f64(header, GENERIC_ZERO_POINT_KEYS);
            if zero_point_card.is_some_and(|(_, zp)| contradicts_unit_zero_point(zp, unit_zp)) {
                return None;
            }
            notes.push(flux_density_formula(unit_text, jy_per_unit));
            if ["PHOTFLAM", "PHOTPLAM"].iter().all(|key| card_f64(header, key).is_some_and(|v| v > 0.0)) {
                notes.push(format!(
                    "PHOTFLAM and PHOTPLAM are present but not applied: BUNIT '{unit_text}' is already a flux density"
                ));
            }
            let origin = format!(
                "BUNIT {unit_text}, zero point {zp_text}{}",
                zero_point_card.map(|(keyword, zp)| format!(" ({keyword} = {zp:?} agrees)")).unwrap_or_default()
            );
            (FluxConvention::FluxDensityPerPixel { jy_per_unit, unit: unit_text.to_string() }, origin)
        } else if is_dn_per_second(&unit) && first_card_f64(header, ROMAN_CONVERSION_MJY_KEYS).is_some() {
            let (key, mjy_per_dn_s) = first_card_f64(header, ROMAN_CONVERSION_MJY_KEYS)?;
            let mut area_keys: Vec<&str> = ROMAN_PIXEL_AREA_SR_KEYS.to_vec();
            area_keys.push("PIXAR_SR");
            let area = pixel_area_sr(header, &area_keys, wcs, &mut warnings)?;
            notes.push(format!(
                "Roman DN/s: flux [Jy] = sum(DN/s) * {} * pixel area [sr] * 1e6",
                key
            ));
            let origin = format!(
                "BUNIT {unit_text} with {key} = {mjy_per_dn_s:?} and a pixel area of {:.4e} sr",
                area.steradians
            );
            (FluxConvention::RomanDnPerSec { mjy_per_dn_s, pixar_sr: area.steradians }, origin)
        } else if is_dn_per_second(&unit) && card_f64(header, "PHOTMJSR").is_some() {
            let photmjsr = card_f64(header, "PHOTMJSR")?;
            let area = pixel_area_sr(header, &["PIXAR_SR"], wcs, &mut warnings)?;
            notes.push("JWST DN/s: flux [Jy] = sum(DN/s) * PHOTMJSR * PIXAR_SR * 1e6".into());
            let origin = format!(
                "BUNIT {unit_text} with PHOTMJSR = {photmjsr:?} and a pixel area of {:.4e} sr",
                area.steradians
            );
            (FluxConvention::JwstDnPerSec { photmjsr, pixar_sr: area.steradians }, origin)
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
            let exptime = positive_exptime(header);
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
            let origin = format!(
                "PHOTFLAM = {photflam:e} and PHOTPLAM = {photplam:?}, {}",
                match (per_second, exptime) {
                    (true, _) => "count rate".to_string(),
                    (false, Some(t)) => format!("counts divided by EXPTIME = {t:?} s"),
                    (false, None) => "counts without EXPTIME".to_string(),
                }
            );
            (FluxConvention::HstCounts { photflam, photplam, photzpt, per_second, exptime }, origin)
        } else if non_science_layer(header).is_some() || has_asinh_scaled_pixels(header) {
            return None;
        } else if let Some(fluxmag0) = card_f64(header, FLUXMAG0_KEY).filter(|v| *v > 0.0) {
            let zp_ab = 2.5 * fluxmag0.log10();
            notes.push("FLUXMAG0: flux [Jy] = sum x 10^(8.90 / 2.5) / FLUXMAG0".into());
            let origin = format!("FLUXMAG0 = {fluxmag0:.2e}, zero point {zp_ab:.2}");
            (
                FluxConvention::SurveyZeroPoint { zp_ab, source: FLUXMAG0_KEY.to_string(), exptime_applied: None },
                origin,
            )
        } else if let Some(zp) = card_f64(header, FPA_ZP_KEY) {
            let exptime = positive_exptime(header)?;
            let zp_ab = zp + 2.5 * exptime.log10();
            warnings.push(format!(
                "FPA.ZP is defined for one count per second: the stack counts are divided by EXPTIME = {exptime:?} s"
            ));
            notes.push(
                "Pan-STARRS stack: flux [Jy] = sum / EXPTIME x 10^((8.90 - FPA.ZP) / 2.5); EXPTIME is the card, not the sum of the EXP_ cards"
                    .into(),
            );
            let origin =
                format!("FPA.ZP = {zp:?} with EXPTIME = {exptime:?} s, zero point {zp_ab:.2} for one count");
            (
                FluxConvention::SurveyZeroPoint {
                    zp_ab,
                    source: FPA_ZP_SOURCE.to_string(),
                    exptime_applied: Some(exptime),
                },
                origin,
            )
        } else if let Some((keyword, zp)) = first_card_f64(header, GENERIC_ZERO_POINT_KEYS) {
            let per_second = is_rate_unit(&unit);
            let exptime = positive_exptime(header);
            let stated = stated_mag_system(header);
            let system = stated.as_ref().map_or(MagSystem::Unknown, |(_, value)| MagSystem::from_card_value(value));
            let system_origin = match (&stated, system) {
                (Some((card, value)), MagSystem::Unknown) => {
                    warnings.push(format!(
                        "{keyword} = {zp:?}: the magnitude system '{value}' ({card}) is not recognised, so AB is assumed"
                    ));
                    system.name().to_string()
                }
                (Some((card, value)), MagSystem::Vega | MagSystem::St) => {
                    warnings.push(format!(
                        "{keyword} = {zp:?} is a {name} zero point ({card} = '{value}'): magnitudes are in the {name} system and no AB magnitude or flux in Jy is given",
                        name = system.name()
                    ));
                    format!("{} ({card})", system.name())
                }
                (Some((card, _)), MagSystem::Ab) => format!("{} ({card})", system.name()),
                (None, _) => {
                    warnings.push(format!(
                        "{keyword} = {zp:?}: the header states no magnitude system (no PHOTSYS or MAGSYS card), so AB is assumed"
                    ));
                    system.name().to_string()
                }
            };
            let stored_unit = match &bunit {
                Some(text) => format!("BUNIT '{text}'"),
                None => "BUNIT missing".to_string(),
            };
            let count_unit = match &bunit {
                Some(_) => stored_unit.clone(),
                None => format!("{stored_unit}, counts assumed"),
            };
            let holds_counts = matches!(classify_unit(Some(&unit)), UnitClass::Counts | UnitClass::Electrons);
            let applied_exptime = if per_second {
                None
            } else if PER_SECOND_ZERO_POINT_KEYS.contains(&keyword) {
                match exptime.filter(|_| holds_counts) {
                    Some(t) => {
                        warnings.push(format!(
                            "{keyword} is defined for one count per second: the pixel values ({count_unit}) are divided by EXPTIME = {t:?} s"
                        ));
                        Some(t)
                    }
                    None if holds_counts => {
                        warnings.push(format!(
                            "{keyword} is defined for one count per second but the header has no positive EXPTIME: the zero point is applied to the pixel values as stored ({count_unit})"
                        ));
                        None
                    }
                    None => {
                        warnings.push(format!(
                            "{keyword} is defined for one count per second and BUNIT '{unit_text}' is not a count unit: EXPTIME is not applied"
                        ));
                        None
                    }
                }
            } else {
                if !STORED_VALUE_ZERO_POINT_KEYS.contains(&keyword) {
                    warnings.push(format!(
                        "{keyword} is applied to the pixel values as stored ({stored_unit}): whether it is defined per second is not known{}",
                        exptime.map(|t| format!(", so EXPTIME = {t:?} s is not applied")).unwrap_or_default()
                    ));
                }
                None
            };
            let exptime_applied = applied_exptime.is_some();
            notes.push(zero_point_formula(&format!("zero point {keyword}"), zp, exptime_applied));
            let origin = format!("{keyword} = {zp:?}, {system_origin}{}", exposure_origin(applied_exptime));
            (
                FluxConvention::GenericZeroPoint {
                    zp,
                    keyword: keyword.to_string(),
                    per_second,
                    exptime,
                    system,
                    exptime_applied,
                },
                origin,
            )
        } else if is_legacy_cutout_without_unit(header, bunit.as_deref())
            && card_text(header, APP_PRODUCT_CARD).is_none()
        {
            warnings.push(format!(
                "the header has no BUNIT and no zero-point card: {NANOMAGGY_UNIT} (AB zero point {}) is assumed from SURVEY = {LEGACY_SURVEY_NAME}",
                zero_point_text(flux_density_zero_point(JY_PER_NANOMAGGY))
            ));
            notes.push(flux_density_formula(NANOMAGGY_UNIT, JY_PER_NANOMAGGY));
            let origin = format!(
                "{NANOMAGGY_UNIT} assumed from SURVEY = {LEGACY_SURVEY_NAME} (no BUNIT or zero-point card)"
            );
            (
                FluxConvention::FluxDensityPerPixel { jy_per_unit: JY_PER_NANOMAGGY, unit: NANOMAGGY_UNIT.to_string() },
                origin,
            )
        } else {
            return None;
        };

        Some(PhotCal::assemble(convention, bunit, notes, warnings, origin))
    }

    fn assemble(
        convention: FluxConvention,
        bunit: Option<String>,
        notes: Vec<String>,
        warnings: Vec<String>,
        origin: String,
    ) -> PhotCal {
        let mag_system = convention.mag_system();
        let jy_per_native_unit = convention.jansky_per_native_unit();
        PhotCal { convention, bunit, notes, warnings, area_applied: false, origin, mag_system, jy_per_native_unit }
    }

    pub fn from_user_zero_point(zp: f64, system: MagSystem, per_second: bool, exptime: Option<f64>) -> PhotCal {
        PhotCal::user_zero_point(zp, system, None, per_second, exptime)
    }

    pub fn user_zero_point_for(header: Option<&HduHeader>, zp: f64, system: MagSystem, per_second: bool) -> PhotCal {
        let bunit = header.and_then(|cards| card_text(cards, "BUNIT"));
        let exptime = header.and_then(positive_exptime);
        PhotCal::user_zero_point(zp, system, bunit, per_second, exptime)
    }

    fn user_zero_point(
        zp: f64,
        system: MagSystem,
        bunit: Option<String>,
        per_second: bool,
        exptime: Option<f64>,
    ) -> PhotCal {
        let stored_as_rate = bunit.as_deref().map(normalized_unit).is_some_and(|unit| is_rate_unit(&unit));
        let needs_exptime = per_second && !stored_as_rate;
        let applied_exptime = exptime.filter(|t| needs_exptime && t.is_finite() && *t > 0.0);
        let mut warnings = vec![USER_ZERO_POINT_WARNING.to_string()];
        match system {
            MagSystem::Ab => {}
            MagSystem::Vega | MagSystem::St => warnings.push(format!(
                "the user zero point is in the {name} system: magnitudes are in the {name} system and no AB magnitude or flux in Jy is given",
                name = system.name()
            )),
            MagSystem::Unknown => {
                warnings.push("the user zero point has no stated magnitude system, so AB is assumed".to_string())
            }
        }
        match applied_exptime {
            Some(t) => warnings.push(format!(
                "the user zero point is defined for one count per second: the pixel values are divided by EXPTIME = {t:?} s"
            )),
            None if needs_exptime => warnings.push(
                "the user zero point is defined for one count per second but no exposure time is available: it is applied to the pixel values as stored"
                    .to_string(),
            ),
            None => {}
        }
        let exptime_applied = applied_exptime.is_some();
        let notes = vec![zero_point_formula("user zero point", zp, exptime_applied)];
        let origin = format!("user zero point {zp:?} {}{}", system.name(), exposure_origin(applied_exptime));
        PhotCal::assemble(
            FluxConvention::GenericZeroPoint {
                zp,
                keyword: USER_ZERO_POINT_KEYWORD.to_string(),
                per_second: stored_as_rate,
                exptime: applied_exptime,
                system,
                exptime_applied,
            },
            bunit,
            notes,
            warnings,
            origin,
        )
    }

    pub fn pixar_from_header(&self) -> bool {
        matches!(self.convention, FluxConvention::JwstMjySr { derived_pixar: false, .. })
    }

    pub fn apply_area_map(&mut self, hdu: usize, map_mean: f64) {
        self.area_applied = true;
        self.notes.retain(|note| note != JWST_MJY_SR_NOTE);
        self.notes.push(format!(
            "AREA pixel-area map applied (HDU {hdu}, map mean {map_mean:.4}): flux [Jy] = sum((SB - sky) * AREA) * PIXAR_SR * 1e6"
        ));
    }

    pub fn label(&self) -> String {
        match &self.convention {
            FluxConvention::JwstMjySr { derived_pixar, .. } => format!(
                "JWST MJy/sr, PIXAR_SR {}{}",
                if *derived_pixar { "derived from the WCS" } else { "from header" },
                if self.area_applied { ", AREA pixel-area map applied" } else { "" }
            ),
            FluxConvention::JwstDnPerSec { .. } => "JWST DN/s via PHOTMJSR and PIXAR_SR".into(),
            FluxConvention::HstCounts { per_second, .. } => format!(
                "HST PHOTFLAM/PHOTPLAM ({}), STmag to ABmag",
                if *per_second { "count rate" } else { "counts / EXPTIME" }
            ),
            FluxConvention::RomanDnPerSec { .. } => "Roman DN/s via conversion_megajanskys and pixel area".into(),
            FluxConvention::GenericZeroPoint { keyword, system, .. } if keyword == USER_ZERO_POINT_KEYWORD => {
                format!("user zero point ({})", system.name())
            }
            FluxConvention::GenericZeroPoint { keyword, system, .. } => {
                format!("zero point {keyword} ({})", system.name())
            }
            FluxConvention::FluxDensityPerPixel { unit, .. } if self.bunit.is_none() => {
                format!("flux density per pixel ({unit}, assumed)")
            }
            FluxConvention::FluxDensityPerPixel { unit, .. } => format!("flux density per pixel ({unit})"),
            FluxConvention::SurveyZeroPoint { source, .. } => format!("zero point {source}"),
        }
    }

    pub fn jy_per_native_unit(&self) -> Option<f64> {
        self.jy_per_native_unit
    }

    pub fn converts_to_jansky(&self) -> bool {
        self.jy_per_native_unit.is_some()
    }

    pub fn jansky_unavailable_reason(&self) -> Option<String> {
        let a_warning_already_is_the_reason = match &self.convention {
            FluxConvention::HstCounts { per_second: false, exptime: None, .. } => true,
            FluxConvention::GenericZeroPoint { system, .. } => !system.gives_ab_magnitudes(),
            _ => false,
        };
        if self.converts_to_jansky() || a_warning_already_is_the_reason {
            return None;
        }
        Some(format!(
            "{} gives no finite positive conversion to Jy, so the data are treated as uncalibrated",
            self.label()
        ))
    }

    pub fn magnitude(&self, net: f64, net_err: Option<f64>) -> Option<SystemMagnitude> {
        if !(net.is_finite() && net > 0.0) {
            return None;
        }
        let system = self.mag_system;
        match &self.convention {
            FluxConvention::GenericZeroPoint { zp, .. } if !system.gives_ab_magnitudes() => {
                let rate = net * self.convention.rate_per_native_unit()?;
                let mag = zp - 2.5 * rate.log10();
                let mag_err =
                    net_err.filter(|e| e.is_finite() && *e >= 0.0).map(|e| MAG_ERR_PER_RELATIVE_FLUX_ERR * e / net);
                mag.is_finite().then_some(SystemMagnitude { mag, mag_err, system })
            }
            _ => {
                let flux = self.calibrate(net, net_err)?;
                Some(SystemMagnitude { mag: flux.mag_ab?, mag_err: flux.mag_ab_err, system })
            }
        }
    }

    pub fn calibrate(&self, net: f64, net_err: Option<f64>) -> Option<CalibratedFlux> {
        if !net.is_finite() {
            return None;
        }
        let factor = self.jy_per_native_unit?;
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
                let rate = net * self.convention.rate_per_native_unit()?;
                Some(-2.5 * (rate * photflam).log10() + photzpt)
            }
            _ => None,
        };
        Some(CalibratedFlux { flux_jy, flux_err_jy, mag_ab, mag_ab_err, st_mag })
    }
}

pub fn missing_calibration_reason(header: Option<&HduHeader>) -> String {
    let Some(header) = header else {
        return "no header available: photometric calibration requires BUNIT and zero-point cards".into();
    };
    let bunit = card_text(header, "BUNIT");
    let unit = bunit.as_deref().map(normalized_unit).unwrap_or_default();
    if bunit.as_deref().is_some_and(is_mjy_per_sr) {
        return "BUNIT is MJy/sr but PIXAR_SR is missing and no WCS pixel scale is available".into();
    }
    if is_mjy_per_sr(&unit) {
        return "BUNIT has a milli prefix (m), not mega (M); convert this surface brightness with Cube Prep before using this photometry calibration".into();
    }
    if let Some(jy_per_unit) = bunit.as_deref().and_then(per_pixel_jansky_scale) {
        let unit_zp = flux_density_zero_point(jy_per_unit);
        let contradicting_card =
            first_card_f64(header, GENERIC_ZERO_POINT_KEYS).filter(|(_, zp)| contradicts_unit_zero_point(*zp, unit_zp));
        if let Some((keyword, zp)) = contradicting_card {
            return format!(
                "BUNIT '{}' means a zero point of {} but {keyword} = {zp:?}: the header contradicts itself, so no calibration is applied",
                bunit.as_deref().unwrap_or(""),
                zero_point_text(unit_zp)
            );
        }
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
    if let Some(evidence) = non_science_layer(header) {
        return format!("this layer is not a science image ({evidence}): no zero point is read for it");
    }
    if has_asinh_scaled_pixels(header) {
        return format!(
            "Pan-STARRS asinh-scaled pixels ({BSOFTEN_CARD} and {BOFFSET_CARD} cards): the values are not linear counts, so no zero point is applied"
        );
    }
    if let Some(zp) = card_f64(header, FPA_ZP_KEY).filter(|_| positive_exptime(header).is_none()) {
        return format!(
            "{FPA_ZP_KEY} = {zp:?} needs the EXPTIME card of the stack and the header has no positive EXPTIME: no zero point is applied"
        );
    }
    let processed_legacy_cutout =
        card_text(header, APP_PRODUCT_CARD).filter(|_| is_legacy_cutout_without_unit(header, bunit.as_deref()));
    if let Some(processed) = processed_legacy_cutout {
        return format!(
            "SURVEY = {LEGACY_SURVEY_NAME} without a unit card: {NANOMAGGY_UNIT} is assumed only for a file as the survey wrote it, and this one was processed ({APP_PRODUCT_CARD} = '{processed}')"
        );
    }
    let zero_point_keys = GENERIC_ZERO_POINT_KEYS
        .into_iter()
        .chain(SURVEY_ZERO_POINT_KEYS)
        .map(|key| key.trim_start_matches(HIERARCH_PREFIX))
        .collect::<Vec<_>>()
        .join(", ");
    let Some(bunit) = bunit else {
        return format!(
            "no photometric calibration in the header: BUNIT is missing and there is no zero-point card ({zero_point_keys})"
        );
    };
    if matches!(units::classify_unit(header).class, units::UnitClass::SurfaceBrightness { .. }) {
        return format!(
            "BUNIT '{bunit}' is a surface brightness and only MJy/sr is read here; convert it to Jy per pixel with Cube Prep"
        );
    }
    match classify_unit(Some(&unit)) {
        UnitClass::Counts | UnitClass::CountRate | UnitClass::Electrons | UnitClass::ElectronRate => format!(
            "no photometric calibration in the header (BUNIT '{bunit}'): a count unit needs a zero-point card ({zero_point_keys}) or PHOTFLAM with PHOTPLAM"
        ),
        UnitClass::Calibrated | UnitClass::Other => format!(
            "no photometric calibration in the header (BUNIT '{bunit}'): the unit is not one this calibration reads (a Jy-family or nanomaggy unit per pixel, or MJy/sr) and there is no zero-point card ({zero_point_keys})"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::core::imaging::region::test_support::{make_header, wcs_cards};
    use crate::core::ops::OutputValues;
    use crate::infra::image_source::load_plane_header;
    use crate::service::common::derived_output_header;
    use crate::types::image_ref::ImageRef;

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
    fn only_a_calibration_with_a_finite_positive_jansky_factor_converts_to_jansky() {
        let no_exptime = make_header(&[("BUNIT", "COUNTS"), ("PHOTFLAM", "1.5E-19"), ("PHOTPLAM", "5921")]);
        assert!(!PhotCal::from_header(&no_exptime, None).unwrap().converts_to_jansky());
        let with_exptime =
            make_header(&[("BUNIT", "COUNTS"), ("PHOTFLAM", "1.5E-19"), ("PHOTPLAM", "5921"), ("EXPTIME", "500")]);
        assert!(PhotCal::from_header(&with_exptime, None).unwrap().converts_to_jansky());
        let underflowing_zero_point = make_header(&[("BUNIT", "ADU"), ("MAGZPT", "1000")]);
        assert!(!PhotCal::from_header(&underflowing_zero_point, None).unwrap().converts_to_jansky());
        assert!(PhotCal::from_header(&jwst_mjy_sr_header(), None).unwrap().converts_to_jansky());
    }

    #[test]
    fn a_calibration_without_a_jansky_factor_gives_its_own_reason_unless_the_exptime_warning_already_does() {
        let no_exptime = make_header(&[("BUNIT", "COUNTS"), ("PHOTFLAM", "1.5E-19"), ("PHOTPLAM", "5921")]);
        assert_eq!(PhotCal::from_header(&no_exptime, None).unwrap().jansky_unavailable_reason(), None);
        assert_eq!(PhotCal::from_header(&jwst_mjy_sr_header(), None).unwrap().jansky_unavailable_reason(), None);
        let zero_photmjsr = make_header(&[("BUNIT", "DN/s"), ("PHOTMJSR", "0"), ("PIXAR_SR", "2.1E-13")]);
        let reason = PhotCal::from_header(&zero_photmjsr, None).unwrap().jansky_unavailable_reason().unwrap();
        assert!(reason.starts_with("JWST DN/s via PHOTMJSR and PIXAR_SR gives no finite positive conversion to Jy"), "{reason}");
        let underflowing_zero_point = make_header(&[("BUNIT", "ADU"), ("MAGZPT", "1000")]);
        let reason = PhotCal::from_header(&underflowing_zero_point, None).unwrap().jansky_unavailable_reason().unwrap();
        assert!(reason.contains("MAGZPT"), "{reason}");
        let negative_roman = make_header(&[
            ("BUNIT", "DN / s"),
            ("ROMAN_META_PHOTOMETRY_CONVERSION_MEGAJANSKYS", "-0.3324"),
            ("ROMAN_META_PHOTOMETRY_PIXELAREA_STERADIANS", "2.8E-13"),
        ]);
        let reason = PhotCal::from_header(&negative_roman, None).unwrap().jansky_unavailable_reason().unwrap();
        assert!(reason.starts_with("Roman DN/s"), "{reason}");
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
    fn the_first_generic_keyword_wins_and_a_header_without_cards_is_uncalibrated() {
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

    fn header_with_wcs(cd: [[f64; 2]; 2], extra: &[(&str, String)]) -> (HduHeader, WcsTransform) {
        let mut cards = wcs_cards(cd);
        cards.extend(extra.iter().map(|(k, v)| (k.to_string(), v.clone())));
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let header = make_header(&pairs);
        let wcs = WcsTransform::from_header(&header).unwrap();
        (header, wcs)
    }

    fn arcsec_to_sr(square_arcsec: f64) -> f64 {
        square_arcsec / ARCSEC_PER_RADIAN.powi(2)
    }

    #[test]
    fn a_pixar_sr_left_over_from_before_a_resample_gives_way_to_the_wcs_pixel_area() {
        let (native, resampled) = (0.031, 0.063);
        let deg = resampled / 3600.0;
        let stale_sr = arcsec_to_sr(native * native);
        let wcs_sr = arcsec_to_sr(resampled * resampled);
        let (header, wcs) = header_with_wcs(
            [[-deg, 0.0], [0.0, deg]],
            &[
                ("BUNIT", "MJy/sr".into()),
                ("PIXAR_SR", format!("{:.6e}", stale_sr)),
                ("PIXAR_A2", format!("{:.6e}", native * native)),
            ],
        );
        let cal = PhotCal::from_header(&header, Some(&wcs)).expect("calibration");
        match &cal.convention {
            FluxConvention::JwstMjySr { pixar_sr, pixar_a2, derived_pixar } => {
                assert!((pixar_sr - wcs_sr).abs() / wcs_sr < 1e-6, "pixar_sr={pixar_sr} wcs={wcs_sr}");
                assert!(*derived_pixar);
                assert!((pixar_a2.unwrap() - resampled * resampled).abs() < 1e-9, "{pixar_a2:?}");
            }
            other => panic!("unexpected convention {other:?}"),
        }
        assert!(
            cal.warnings.iter().any(|w| w.starts_with("PIXAR_SR") && w.contains("WCS pixel area is used")),
            "{:?}",
            cal.warnings
        );
        assert!(cal.label().contains("derived from the WCS"), "{}", cal.label());
        let true_flux_jy = 1.0e-4;
        let flux = cal.calibrate(true_flux_jy / (wcs_sr * JY_PER_MJY), None).unwrap();
        assert!((flux.flux_jy - true_flux_jy).abs() / true_flux_jy < 1e-6, "flux_jy={}", flux.flux_jy);

        let (dn, dn_wcs) = header_with_wcs(
            [[-deg, 0.0], [0.0, deg]],
            &[("BUNIT", "DN/s".into()), ("PHOTMJSR", "0.5".into()), ("PIXAR_SR", format!("{:.6e}", stale_sr))],
        );
        match PhotCal::from_header(&dn, Some(&dn_wcs)).unwrap().convention {
            FluxConvention::JwstDnPerSec { pixar_sr, .. } => assert!((pixar_sr - wcs_sr).abs() / wcs_sr < 1e-6),
            other => panic!("unexpected convention {other:?}"),
        }
    }

    #[test]
    fn a_pixar_sr_within_ten_percent_of_the_wcs_area_is_kept() {
        let deg = 0.063 / 3600.0;
        let wcs_sr = arcsec_to_sr(0.063 * 0.063);
        let (header, wcs) = header_with_wcs(
            [[-deg, 0.0], [0.0, deg]],
            &[("BUNIT", "MJy/sr".into()), ("PIXAR_SR", format!("{:.6e}", wcs_sr * 1.05))],
        );
        let cal = PhotCal::from_header(&header, Some(&wcs)).unwrap();
        match cal.convention {
            FluxConvention::JwstMjySr { pixar_sr, derived_pixar, .. } => {
                assert!((pixar_sr - wcs_sr * 1.05).abs() / wcs_sr < 1e-6);
                assert!(!derived_pixar);
            }
            other => panic!("unexpected convention {other:?}"),
        }
        assert!(cal.warnings.is_empty(), "{:?}", cal.warnings);
    }

    #[test]
    fn the_wcs_pixel_area_of_rectangular_pixels_is_the_cd_determinant() {
        let (sx, sy) = (0.1, 0.2);
        let cd = [[-sx / 3600.0, 0.0], [0.0, sy / 3600.0]];
        let true_sr = arcsec_to_sr(sx * sy);
        let (derived, wcs) = header_with_wcs(cd, &[("BUNIT", "MJy/sr".into())]);
        match PhotCal::from_header(&derived, Some(&wcs)).unwrap().convention {
            FluxConvention::JwstMjySr { pixar_sr, pixar_a2, derived_pixar } => {
                assert!((pixar_sr - true_sr).abs() / true_sr < 1e-6, "pixar_sr={pixar_sr} true={true_sr}");
                assert!((pixar_a2.unwrap() - sx * sy).abs() < 1e-9);
                assert!(derived_pixar);
            }
            other => panic!("unexpected convention {other:?}"),
        }

        let (with_card, wcs) = header_with_wcs(cd, &[("BUNIT", "MJy/sr".into()), ("PIXAR_SR", format!("{:.6e}", true_sr))]);
        let cal = PhotCal::from_header(&with_card, Some(&wcs)).unwrap();
        assert!(matches!(cal.convention, FluxConvention::JwstMjySr { derived_pixar: false, .. }));
        assert!(cal.warnings.is_empty(), "{:?}", cal.warnings);
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
    fn p0_millijansky_must_not_use_megajansky_photometry() {
        for unit in ["mJy/sr", " mjy/SR ", "mJy sr-1"] {
            let header = make_header(&[("BUNIT", unit), ("PIXAR_SR", "2.1E-13")]);
            let cal = PhotCal::from_header(&header, None);
            assert!(cal.is_none(), "{unit} must not use the megajansky multiplier: {cal:?}");
            assert!(!missing_calibration_reason(Some(&header)).contains("BUNIT is MJy/sr"));
            let with_count_calibration = make_header(&[("BUNIT", unit), ("PIXAR_SR", "2.1E-13"), ("PHOTFLAM", "1e-19"), ("PHOTPLAM", "15000")]);
            assert!(PhotCal::from_header(&with_count_calibration, None).is_none());
        }
    }

    #[test]
    fn only_a_plain_mjy_per_sr_unit_is_calibrated_as_a_flux_density() {
        for unit in ["MJy/sr", "MJy / sr", "MJy sr-1", "MJy.sr**-1", " MJY/SR "] {
            let header = make_header(&[("BUNIT", unit), ("PIXAR_SR", "2.1E-13")]);
            assert!(
                matches!(PhotCal::from_header(&header, None).map(|c| c.convention), Some(FluxConvention::JwstMjySr { .. })),
                "BUNIT '{unit}' was not recognised as MJy/sr"
            );
        }
        for unit in ["MJy/sr km/s", "MJy/sr x channels", "MJy/sr.km/s", "10 MJy/sr"] {
            let header = make_header(&[("BUNIT", unit), ("PIXAR_SR", "2.1E-13")]);
            assert!(PhotCal::from_header(&header, None).is_none(), "BUNIT '{unit}' was calibrated as MJy/sr");
            assert!(!missing_calibration_reason(Some(&header)).contains("PIXAR_SR"), "{unit}");
        }
    }

    #[test]
    fn serialization_tags_the_convention_kind() {
        let cal = PhotCal::from_header(&jwst_mjy_sr_header(), None).unwrap();
        let json = serde_json::to_value(&cal).unwrap();
        assert_eq!(json["convention"]["kind"], "jwst_mjy_sr");
        assert_eq!(json["convention"]["derived_pixar"], false);
        assert_eq!(json["bunit"], "MJy/sr");
        assert!(json["notes"].is_array() && json["warnings"].is_array());
        assert!(json["origin"].is_string(), "{json}");
        assert_eq!(json["mag_system"], "ab");
        assert!((json["jy_per_native_unit"].as_f64().unwrap() - 2.1e-7).abs() / 2.1e-7 < 1e-12, "{json}");

        let zero_point = serde_json::to_value(PhotCal::from_header(&casu_like_header(), None).unwrap()).unwrap();
        assert_eq!(zero_point["convention"]["system"], "unknown");
        assert_eq!(zero_point["convention"]["exptime_applied"], true);
    }

    fn per_second_zero_point_on_counts_header() -> HduHeader {
        make_header(&[("BUNIT", "ADU"), ("EXPTIME", "10"), ("MAGZPT", "23.0")])
    }

    fn vega_zero_point_header() -> HduHeader {
        make_header(&[("BUNIT", "ADU"), ("PHOTZP", "23.0"), ("PHOTSYS", "VEGA")])
    }

    fn casu_like_header() -> HduHeader {
        make_header(&[("BUNIT", "ADU"), ("EXPTIME", "300"), ("MAGZPT", "25.0")])
    }

    #[test]
    fn a_per_second_zero_point_on_counts_is_divided_by_the_exposure_time() {
        let cal = PhotCal::from_header(&per_second_zero_point_on_counts_header(), None).unwrap();
        let flux = cal.calibrate(10_000.0, None).unwrap();
        let mag_ab = flux.mag_ab.unwrap();
        assert!((mag_ab - 15.5).abs() < 1e-9, "mag_ab={mag_ab}");
        let expected_jy = 2.2908676527677745e-3;
        assert!((flux.flux_jy - expected_jy).abs() / expected_jy < 1e-12, "flux_jy={:e}", flux.flux_jy);
    }

    #[test]
    fn a_vega_zero_point_is_not_reported_as_an_ab_magnitude_or_as_jansky() {
        let cal = PhotCal::from_header(&vega_zero_point_header(), None).unwrap();
        let flux = cal.calibrate(10_000.0, None);
        assert!(flux.is_none(), "{flux:?}");
    }

    #[test]
    fn the_assumptions_of_a_header_zero_point_reach_the_warnings() {
        let cal = PhotCal::from_header(&casu_like_header(), None).unwrap();
        assert_eq!(
            cal.warnings,
            vec![
                "MAGZPT = 25.0: the header states no magnitude system (no PHOTSYS or MAGSYS card), so AB is assumed".to_string(),
                "MAGZPT is defined for one count per second: the pixel values (BUNIT 'ADU') are divided by EXPTIME = 300.0 s".to_string(),
            ]
        );
        assert!(!cal.notes.iter().any(|note| note.contains("assumed")), "{:?}", cal.notes);
    }

    #[test]
    fn the_serialised_calibration_names_its_magnitude_system() {
        let vega = serde_json::to_value(PhotCal::from_header(&vega_zero_point_header(), None).unwrap()).unwrap();
        assert_eq!(vega["mag_system"], "vega", "{vega}");
        assert!(vega.as_object().unwrap().contains_key("jy_per_native_unit"), "{vega}");
        assert!(vega["jy_per_native_unit"].is_null(), "{vega}");
        let unstated = serde_json::to_value(PhotCal::from_header(&casu_like_header(), None).unwrap()).unwrap();
        assert_eq!(unstated["mag_system"], "unknown", "{unstated}");
        assert!(unstated["origin"].is_string(), "{unstated}");
    }

    const FACTOR_TOLERANCE: f64 = 1e-12;
    const MAGNITUDE_TOLERANCE: f64 = 1e-9;
    const ZERO_POINT_25_JY: f64 = 3.63078054770101e-7;
    const ZERO_POINT_25_OVER_300_S_JY: f64 = 1.2102601825670033e-9;

    fn relative_difference(actual: f64, expected: f64) -> f64 {
        (actual - expected).abs() / expected.abs()
    }

    fn no_system_warning(keyword: &str, zp: &str) -> String {
        format!("{keyword} = {zp}: the header states no magnitude system (no PHOTSYS or MAGSYS card), so AB is assumed")
    }

    fn des_cutout_header() -> HduHeader {
        make_header(&[("BUNIT", "COUNTS/S"), ("MAGZERO", "30."), ("EXPTIME", "1080.")])
    }

    fn native_zero_point_on_counts_header() -> HduHeader {
        make_header(&[("BUNIT", "ADU"), ("EXPTIME", "300"), ("MAGZERO", "25.0")])
    }

    fn hst_counts_header() -> HduHeader {
        make_header(&[
            ("TELESCOP", "HST"),
            ("INSTRUME", "WFC3"),
            ("BUNIT", "ELECTRONS"),
            ("PHOTFLAM", "1.9E-19"),
            ("PHOTPLAM", "5300"),
            ("EXPTIME", "600"),
        ])
    }

    fn jwst_dn_per_second_header() -> HduHeader {
        make_header(&[("BUNIT", "DN/s"), ("PHOTMJSR", "0.5"), ("PIXAR_SR", "2.1E-13")])
    }

    struct ExposureCase {
        cards: &'static [(&'static str, &'static str)],
        factor: f64,
        exptime_applied: bool,
        per_second: bool,
        exptime: Option<f64>,
        warnings: Vec<String>,
    }

    #[test]
    fn the_keyword_family_decides_whether_the_exposure_time_applies() {
        let cases = [
            ExposureCase {
                cards: &[("BUNIT", "COUNTS/S"), ("MAGZERO", "30."), ("EXPTIME", "1080.")],
                factor: 3.6307805477010028e-9,
                exptime_applied: false,
                per_second: true,
                exptime: Some(1080.0),
                warnings: vec![no_system_warning("MAGZERO", "30.0")],
            },
            ExposureCase {
                cards: &[("MAGZERO", "24.6"), ("FILTER", "VIS")],
                factor: 5.248074602497723e-7,
                exptime_applied: false,
                per_second: false,
                exptime: None,
                warnings: vec![no_system_warning("MAGZERO", "24.6")],
            },
            ExposureCase {
                cards: &[("BUNIT", "ADU"), ("EXPTIME", "300"), ("MAGZERO", "25.0")],
                factor: ZERO_POINT_25_JY,
                exptime_applied: false,
                per_second: false,
                exptime: Some(300.0),
                warnings: vec![no_system_warning("MAGZERO", "25.0")],
            },
            ExposureCase {
                cards: &[("BUNIT", "ADU"), ("EXPTIME", "300"), ("PHOTZP", "25.0")],
                factor: ZERO_POINT_25_JY,
                exptime_applied: false,
                per_second: false,
                exptime: Some(300.0),
                warnings: vec![no_system_warning("PHOTZP", "25.0")],
            },
            ExposureCase {
                cards: &[("BUNIT", "ADU"), ("EXPTIME", "300"), ("MAGZPT", "25.0")],
                factor: ZERO_POINT_25_OVER_300_S_JY,
                exptime_applied: true,
                per_second: false,
                exptime: Some(300.0),
                warnings: vec![
                    no_system_warning("MAGZPT", "25.0"),
                    "MAGZPT is defined for one count per second: the pixel values (BUNIT 'ADU') are divided by EXPTIME = 300.0 s".to_string(),
                ],
            },
            ExposureCase {
                cards: &[("BUNIT", "ADU/S"), ("EXPTIME", "300"), ("MAGZPT", "25.0")],
                factor: ZERO_POINT_25_JY,
                exptime_applied: false,
                per_second: true,
                exptime: Some(300.0),
                warnings: vec![no_system_warning("MAGZPT", "25.0")],
            },
            ExposureCase {
                cards: &[("BUNIT", "ADU"), ("MAGZPT", "25.0")],
                factor: ZERO_POINT_25_JY,
                exptime_applied: false,
                per_second: false,
                exptime: None,
                warnings: vec![
                    no_system_warning("MAGZPT", "25.0"),
                    "MAGZPT is defined for one count per second but the header has no positive EXPTIME: the zero point is applied to the pixel values as stored (BUNIT 'ADU')".to_string(),
                ],
            },
            ExposureCase {
                cards: &[("EXPTIME", "300"), ("MAGZPT", "25.0")],
                factor: ZERO_POINT_25_OVER_300_S_JY,
                exptime_applied: true,
                per_second: false,
                exptime: Some(300.0),
                warnings: vec![
                    no_system_warning("MAGZPT", "25.0"),
                    "MAGZPT is defined for one count per second: the pixel values (BUNIT missing, counts assumed) are divided by EXPTIME = 300.0 s".to_string(),
                ],
            },
            ExposureCase {
                cards: &[("BUNIT", "photon"), ("EXPTIME", "300"), ("MAGZPT", "25.0")],
                factor: ZERO_POINT_25_JY,
                exptime_applied: false,
                per_second: false,
                exptime: Some(300.0),
                warnings: vec![
                    no_system_warning("MAGZPT", "25.0"),
                    "MAGZPT is defined for one count per second and BUNIT 'photon' is not a count unit: EXPTIME is not applied".to_string(),
                ],
            },
            ExposureCase {
                cards: &[("BUNIT", "ADU"), ("EXPTIME", "300"), ("ZPT", "25.0")],
                factor: ZERO_POINT_25_JY,
                exptime_applied: false,
                per_second: false,
                exptime: Some(300.0),
                warnings: vec![
                    no_system_warning("ZPT", "25.0"),
                    "ZPT is applied to the pixel values as stored (BUNIT 'ADU'): whether it is defined per second is not known, so EXPTIME = 300.0 s is not applied".to_string(),
                ],
            },
            ExposureCase {
                cards: &[("BUNIT", "ADU"), ("ZPT", "25.0")],
                factor: ZERO_POINT_25_JY,
                exptime_applied: false,
                per_second: false,
                exptime: None,
                warnings: vec![
                    no_system_warning("ZPT", "25.0"),
                    "ZPT is applied to the pixel values as stored (BUNIT 'ADU'): whether it is defined per second is not known".to_string(),
                ],
            },
            ExposureCase {
                cards: &[("BUNIT", "e-/s"), ("ZEROPT", "24.5")],
                factor: 5.754399373371567e-7,
                exptime_applied: false,
                per_second: true,
                exptime: None,
                warnings: vec![no_system_warning("ZEROPT", "24.5")],
            },
        ];
        for case in cases {
            let cal = PhotCal::from_header(&make_header(case.cards), None).unwrap();
            let factor = cal.jy_per_native_unit().unwrap();
            assert!(relative_difference(factor, case.factor) < FACTOR_TOLERANCE, "{:?}: {factor:e}", case.cards);
            assert_eq!(cal.jy_per_native_unit, Some(factor), "{:?}", case.cards);
            match &cal.convention {
                FluxConvention::GenericZeroPoint { per_second, exptime, exptime_applied, .. } => {
                    assert_eq!(*exptime_applied, case.exptime_applied, "{:?}", case.cards);
                    assert_eq!(*per_second, case.per_second, "{:?}", case.cards);
                    assert_eq!(*exptime, case.exptime, "{:?}", case.cards);
                }
                other => panic!("unexpected convention {other:?}"),
            }
            assert_eq!(cal.warnings, case.warnings, "{:?}", case.cards);
        }
    }

    fn zero_point_with_system_cards(system_cards: &[(&str, &str)]) -> PhotCal {
        let mut cards = vec![("BUNIT", "ADU"), ("MAGZERO", "25.0")];
        cards.extend_from_slice(system_cards);
        PhotCal::from_header(&make_header(&cards), None).unwrap()
    }

    #[test]
    fn the_magnitude_system_is_read_from_photsys_then_magsys() {
        let ab = zero_point_with_system_cards(&[("PHOTSYS", "AB")]);
        assert_eq!(ab.mag_system, MagSystem::Ab);
        assert!(ab.warnings.is_empty(), "{:?}", ab.warnings);
        assert_eq!(zero_point_with_system_cards(&[("PHOTSYS", "ABMAG")]).mag_system, MagSystem::Ab);

        let vega = zero_point_with_system_cards(&[("MAGSYS", "VEGA")]);
        assert_eq!(vega.mag_system, MagSystem::Vega);
        assert_eq!(
            vega.warnings,
            vec!["MAGZERO = 25.0 is a Vega zero point (MAGSYS = 'VEGA'): magnitudes are in the Vega system and no AB magnitude or flux in Jy is given".to_string()]
        );
        assert_eq!(zero_point_with_system_cards(&[("PHOTSYS", "vega")]).mag_system, MagSystem::Vega);

        let st = zero_point_with_system_cards(&[("PHOTSYS", "STMAG")]);
        assert_eq!(st.mag_system, MagSystem::St);
        assert_eq!(
            st.warnings,
            vec!["MAGZERO = 25.0 is a ST zero point (PHOTSYS = 'STMAG'): magnitudes are in the ST system and no AB magnitude or flux in Jy is given".to_string()]
        );

        let unrecognised = zero_point_with_system_cards(&[("PHOTSYS", "JOHNSON")]);
        assert_eq!(unrecognised.mag_system, MagSystem::Unknown);
        assert_eq!(
            unrecognised.warnings,
            vec!["MAGZERO = 25.0: the magnitude system 'JOHNSON' (PHOTSYS) is not recognised, so AB is assumed".to_string()]
        );

        let both = zero_point_with_system_cards(&[("PHOTSYS", "AB"), ("MAGSYS", "VEGA")]);
        assert_eq!(both.mag_system, MagSystem::Ab);
        assert!(both.warnings.is_empty(), "{:?}", both.warnings);

        let neither = zero_point_with_system_cards(&[]);
        assert_eq!(neither.mag_system, MagSystem::Unknown);
        assert_eq!(neither.warnings, vec![no_system_warning("MAGZERO", "25.0")]);
    }

    #[test]
    fn the_notes_of_a_zero_point_keep_the_formula_only() {
        let divided = PhotCal::from_header(&casu_like_header(), None).unwrap();
        assert_eq!(divided.notes, vec!["zero point MAGZPT: mag = 25.0 - 2.5 log10(sum / EXPTIME)".to_string()]);
        let as_stored = PhotCal::from_header(&native_zero_point_on_counts_header(), None).unwrap();
        assert_eq!(as_stored.notes, vec!["zero point MAGZERO: mag = 25.0 - 2.5 log10(sum)".to_string()]);
    }

    #[test]
    fn origin_and_label_name_the_card_the_value_and_the_system() {
        let des = PhotCal::from_header(&des_cutout_header(), None).unwrap();
        assert_eq!(des.origin, "MAGZERO = 30.0, AB assumed");
        assert_eq!(des.label(), "zero point MAGZERO (AB assumed)");

        let divided = PhotCal::from_header(&per_second_zero_point_on_counts_header(), None).unwrap();
        assert_eq!(divided.origin, "MAGZPT = 23.0, AB assumed, counts divided by EXPTIME = 10.0 s");

        let vega = PhotCal::from_header(&vega_zero_point_header(), None).unwrap();
        assert_eq!(vega.origin, "PHOTZP = 23.0, Vega (PHOTSYS)");
        assert_eq!(vega.label(), "zero point PHOTZP (Vega)");

        let stated_ab =
            PhotCal::from_header(&make_header(&[("MAGZERO", "30.0"), ("PHOTSYS", "AB")]), None).unwrap();
        assert_eq!(stated_ab.origin, "MAGZERO = 30.0, AB (PHOTSYS)");
        assert_eq!(stated_ab.label(), "zero point MAGZERO (AB)");

        let jwst = PhotCal::from_header(&jwst_mjy_sr_header(), None).unwrap();
        assert_eq!(jwst.origin, "BUNIT MJy/sr with PIXAR_SR = 2.1000e-13 sr");
        assert_eq!(jwst.label(), "JWST MJy/sr, PIXAR_SR from header");

        let hst = PhotCal::from_header(&hst_counts_header(), None).unwrap();
        assert_eq!(hst.origin, "PHOTFLAM = 1.9e-19 and PHOTPLAM = 5300.0, counts divided by EXPTIME = 600.0 s");

        let dn_per_second = PhotCal::from_header(&jwst_dn_per_second_header(), None).unwrap();
        assert_eq!(dn_per_second.origin, "BUNIT DN/s with PHOTMJSR = 0.5 and a pixel area of 2.1000e-13 sr");
    }

    #[test]
    fn every_convention_reports_its_jansky_factor_and_the_ab_system() {
        let jwst = PhotCal::from_header(&jwst_mjy_sr_header(), None).unwrap();
        assert_eq!(jwst.jy_per_native_unit, Some(2.1e-13 * 1e6));
        assert_eq!(jwst.jy_per_native_unit(), Some(2.1e-13 * 1e6));
        assert_eq!(jwst.mag_system, MagSystem::Ab);

        let dn_per_second = PhotCal::from_header(&jwst_dn_per_second_header(), None).unwrap();
        assert_eq!(dn_per_second.jy_per_native_unit, Some(0.5 * 2.1e-13 * 1e6));
        assert_eq!(dn_per_second.jy_per_native_unit(), Some(0.5 * 2.1e-13 * 1e6));
        assert_eq!(dn_per_second.mag_system, MagSystem::Ab);

        let hst = PhotCal::from_header(&hst_counts_header(), None).unwrap();
        let hst_unit_flux = hst.calibrate(1.0, None).unwrap().flux_jy;
        assert_eq!(hst.jy_per_native_unit, Some(hst_unit_flux));
        assert_eq!(hst.jy_per_native_unit(), Some(hst_unit_flux));
        assert_eq!(hst.mag_system, MagSystem::Ab);

        let hst_without_exptime = PhotCal::from_header(
            &make_header(&[("BUNIT", "ELECTRONS"), ("PHOTFLAM", "1.9E-19"), ("PHOTPLAM", "5300")]),
            None,
        )
        .unwrap();
        assert_eq!(hst_without_exptime.jy_per_native_unit, None);
        assert_eq!(hst_without_exptime.jy_per_native_unit(), None);
        assert_eq!(hst_without_exptime.origin, "PHOTFLAM = 1.9e-19 and PHOTPLAM = 5300.0, counts without EXPTIME");

        let vega = PhotCal::from_header(&vega_zero_point_header(), None).unwrap();
        assert_eq!(vega.jy_per_native_unit, None);
        assert_eq!(vega.jy_per_native_unit(), None);
        assert_eq!(vega.mag_system, MagSystem::Vega);
        assert_eq!(vega.jansky_unavailable_reason(), None);
    }

    #[test]
    fn magnitude_follows_the_system_of_the_zero_point() {
        let vega = PhotCal::from_header(&vega_zero_point_header(), None).unwrap();
        let vega_mag = vega.magnitude(10_000.0, Some(100.0)).unwrap();
        assert!((vega_mag.mag - 13.0).abs() < MAGNITUDE_TOLERANCE, "{vega_mag:?}");
        assert!((vega_mag.mag_err.unwrap() - 0.010857362047581288).abs() < MAGNITUDE_TOLERANCE, "{vega_mag:?}");
        assert_eq!(vega_mag.system, MagSystem::Vega);

        let jwst = PhotCal::from_header(&jwst_mjy_sr_header(), None).unwrap();
        let jwst_flux = jwst.calibrate(1000.0, Some(100.0)).unwrap();
        let jwst_mag = jwst.magnitude(1000.0, Some(100.0)).unwrap();
        assert!((jwst_mag.mag - jwst_flux.mag_ab.unwrap()).abs() < MAGNITUDE_TOLERANCE, "{jwst_mag:?}");
        assert!((jwst_mag.mag_err.unwrap() - jwst_flux.mag_ab_err.unwrap()).abs() < MAGNITUDE_TOLERANCE, "{jwst_mag:?}");
        assert_eq!(jwst_mag.system, MagSystem::Ab);

        let unstated = PhotCal::from_header(&native_zero_point_on_counts_header(), None).unwrap();
        let unstated_mag = unstated.magnitude(10_000.0, None).unwrap();
        let unstated_ab = unstated.calibrate(10_000.0, None).unwrap().mag_ab.unwrap();
        assert!((unstated_mag.mag - unstated_ab).abs() < MAGNITUDE_TOLERANCE, "{unstated_mag:?}");
        assert_eq!(unstated_mag.mag_err, None);
        assert_eq!(unstated_mag.system, MagSystem::Unknown);

        for cal in [&vega, &jwst, &unstated] {
            assert_eq!(cal.magnitude(0.0, None), None);
            assert_eq!(cal.magnitude(-5.0, None), None);
            assert_eq!(cal.magnitude(f64::NAN, None), None);
        }

        let vega_per_second = PhotCal::from_header(
            &make_header(&[("MAGZPT", "23.0"), ("PHOTSYS", "VEGA"), ("BUNIT", "ADU"), ("EXPTIME", "10")]),
            None,
        )
        .unwrap();
        let divided = vega_per_second.magnitude(10_000.0, None).unwrap();
        assert!((divided.mag - 15.5).abs() < MAGNITUDE_TOLERANCE, "{divided:?}");
        assert_eq!(divided.system, MagSystem::Vega);
    }

    #[test]
    fn a_zero_point_without_exposure_card_keeps_mag_inst_plus_zero_point() {
        let cal = PhotCal::from_header(&make_header(&[("MAGZPT", "25.0"), ("BUNIT", "ADU")]), None).unwrap();
        for net in [4.0f64, 10_000.0] {
            let mag_ab = cal.calibrate(net, None).unwrap().mag_ab.unwrap();
            assert!((mag_ab - (25.0 - 2.5 * net.log10())).abs() < MAGNITUDE_TOLERANCE, "net={net} mag_ab={mag_ab}");
        }
        assert!(cal.label().starts_with("zero point MAGZPT"), "{}", cal.label());
    }

    const USER_VEGA_WARNING: &str = "the user zero point is in the Vega system: magnitudes are in the Vega system and no AB magnitude or flux in Jy is given";
    const USER_UNSTATED_SYSTEM_WARNING: &str = "the user zero point has no stated magnitude system, so AB is assumed";
    const USER_DIVIDED_BY_300_S_WARNING: &str = "the user zero point is defined for one count per second: the pixel values are divided by EXPTIME = 300.0 s";
    const USER_NO_EXPTIME_WARNING: &str = "the user zero point is defined for one count per second but no exposure time is available: it is applied to the pixel values as stored";

    #[test]
    fn a_user_zero_point_is_a_calibration_with_its_own_origin() {
        let ab = PhotCal::from_user_zero_point(25.0, MagSystem::Ab, false, None);
        assert!(relative_difference(ab.jy_per_native_unit.unwrap(), ZERO_POINT_25_JY) < FACTOR_TOLERANCE);
        assert!((ab.calibrate(10_000.0, None).unwrap().mag_ab.unwrap() - 15.0).abs() < MAGNITUDE_TOLERANCE);
        assert_eq!(ab.warnings, vec![USER_ZERO_POINT_WARNING.to_string()]);
        assert_eq!(USER_ZERO_POINT_WARNING, "calibrated with a user zero point");
        assert_eq!(ab.origin, "user zero point 25.0 AB");
        assert_eq!(ab.label(), "user zero point (AB)");
        assert_eq!(ab.notes, vec!["user zero point: mag = 25.0 - 2.5 log10(sum)".to_string()]);
        assert!(
            matches!(
                &ab.convention,
                FluxConvention::GenericZeroPoint {
                    keyword,
                    system: MagSystem::Ab,
                    per_second: false,
                    exptime: None,
                    exptime_applied: false,
                    ..
                } if keyword == "user"
            ),
            "{:?}",
            ab.convention
        );
        assert_eq!(ab.bunit, None);

        let vega = PhotCal::from_user_zero_point(25.0, MagSystem::Vega, false, None);
        assert_eq!(vega.jy_per_native_unit, None);
        assert_eq!(vega.calibrate(10_000.0, None), None);
        assert_eq!(vega.warnings, vec![USER_ZERO_POINT_WARNING.to_string(), USER_VEGA_WARNING.to_string()]);
        let vega_mag = vega.magnitude(10_000.0, None).unwrap();
        assert!((vega_mag.mag - 15.0).abs() < MAGNITUDE_TOLERANCE, "{vega_mag:?}");
        assert_eq!(vega_mag.system, MagSystem::Vega);

        let divided = PhotCal::from_user_zero_point(25.0, MagSystem::Ab, true, Some(300.0));
        let divided_mag = divided.calibrate(10_000.0, None).unwrap().mag_ab.unwrap();
        assert!((divided_mag - 21.192803136799157).abs() < MAGNITUDE_TOLERANCE, "{divided_mag}");
        assert!(relative_difference(divided.jy_per_native_unit.unwrap(), ZERO_POINT_25_OVER_300_S_JY) < FACTOR_TOLERANCE);
        assert_eq!(
            divided.warnings,
            vec![USER_ZERO_POINT_WARNING.to_string(), USER_DIVIDED_BY_300_S_WARNING.to_string()]
        );
        assert_eq!(divided.origin, "user zero point 25.0 AB, counts divided by EXPTIME = 300.0 s");
        assert_eq!(divided.notes, vec!["user zero point: mag = 25.0 - 2.5 log10(sum / EXPTIME)".to_string()]);

        let no_exptime = PhotCal::from_user_zero_point(25.0, MagSystem::Ab, true, None);
        assert!((no_exptime.calibrate(10_000.0, None).unwrap().mag_ab.unwrap() - 15.0).abs() < MAGNITUDE_TOLERANCE);
        assert_eq!(
            no_exptime.warnings,
            vec![USER_ZERO_POINT_WARNING.to_string(), USER_NO_EXPTIME_WARNING.to_string()]
        );

        let unstated = PhotCal::from_user_zero_point(25.0, MagSystem::Unknown, false, None);
        assert!(relative_difference(unstated.jy_per_native_unit.unwrap(), ZERO_POINT_25_JY) < FACTOR_TOLERANCE);
        assert_eq!(
            unstated.warnings,
            vec![USER_ZERO_POINT_WARNING.to_string(), USER_UNSTATED_SYSTEM_WARNING.to_string()]
        );
        assert_eq!(unstated.label(), "user zero point (AB assumed)");
        assert_eq!(unstated.origin, "user zero point 25.0 AB assumed");

        assert_eq!(PhotCal::from_user_zero_point(f64::NAN, MagSystem::Ab, false, None).jy_per_native_unit, None);
        assert_eq!(PhotCal::from_user_zero_point(f64::NAN, MagSystem::Vega, false, None).magnitude(10_000.0, None), None);
    }

    #[test]
    fn a_user_zero_point_reads_unit_and_exposure_from_the_header() {
        let counts = make_header(&[("BUNIT", "ADU"), ("EXPTIME", "300")]);
        let divided = PhotCal::user_zero_point_for(Some(&counts), 25.0, MagSystem::Ab, true);
        assert!(matches!(
            divided.convention,
            FluxConvention::GenericZeroPoint { exptime_applied: true, per_second: false, exptime: Some(t), .. } if t == 300.0
        ));
        assert!(relative_difference(divided.jy_per_native_unit.unwrap(), ZERO_POINT_25_OVER_300_S_JY) < FACTOR_TOLERANCE);
        assert_eq!(divided.bunit.as_deref(), Some("ADU"));
        assert_eq!(
            divided.warnings,
            vec![USER_ZERO_POINT_WARNING.to_string(), USER_DIVIDED_BY_300_S_WARNING.to_string()]
        );

        let rate = make_header(&[("BUNIT", "ADU/S"), ("EXPTIME", "300")]);
        let as_stored = PhotCal::user_zero_point_for(Some(&rate), 25.0, MagSystem::Ab, true);
        assert!(matches!(
            as_stored.convention,
            FluxConvention::GenericZeroPoint { exptime_applied: false, per_second: true, .. }
        ));
        assert!(relative_difference(as_stored.jy_per_native_unit.unwrap(), ZERO_POINT_25_JY) < FACTOR_TOLERANCE);
        assert_eq!(as_stored.warnings, vec![USER_ZERO_POINT_WARNING.to_string()]);

        let no_exptime = make_header(&[("BUNIT", "ADU")]);
        let not_divided = PhotCal::user_zero_point_for(Some(&no_exptime), 25.0, MagSystem::Ab, true);
        assert!(matches!(not_divided.convention, FluxConvention::GenericZeroPoint { exptime_applied: false, .. }));
        assert_eq!(
            not_divided.warnings,
            vec![USER_ZERO_POINT_WARNING.to_string(), USER_NO_EXPTIME_WARNING.to_string()]
        );

        let without_header = PhotCal::user_zero_point_for(None, 25.0, MagSystem::Ab, false);
        assert_eq!(without_header.warnings, vec![USER_ZERO_POINT_WARNING.to_string()]);
        assert_eq!(without_header, PhotCal::from_user_zero_point(25.0, MagSystem::Ab, false, None));
    }

    const JY_PER_NANOMAGGY_VALUE: f64 = 3.630780547701014e-6;

    fn flux_density_parts(cal: &PhotCal) -> (f64, &str) {
        match &cal.convention {
            FluxConvention::FluxDensityPerPixel { jy_per_unit, unit } => (*jy_per_unit, unit.as_str()),
            other => panic!("unexpected convention {other:?}"),
        }
    }

    fn calibrated_by_unit(cards: &[(&str, &str)]) -> PhotCal {
        PhotCal::from_header(&make_header(cards), None).unwrap_or_else(|| panic!("{cards:?} is not calibrated"))
    }

    #[test]
    fn jansky_family_units_are_calibrated_by_the_unit_itself() {
        for (bunit, expected) in [
            ("nJy", 1e-9),
            ("uJy", 1e-6),
            ("mJy", 1e-3),
            ("Jy", 1.0),
            ("Jy/pixel", 1.0),
            ("10.0*nanoJansky", 1e-8),
            ("MJy", 1e6),
        ] {
            let cal = calibrated_by_unit(&[("BUNIT", bunit)]);
            let (jy_per_unit, unit) = flux_density_parts(&cal);
            assert!(relative_difference(jy_per_unit, expected) < FACTOR_TOLERANCE, "{bunit}: {jy_per_unit:e}");
            assert_eq!(unit, bunit);
            assert_eq!(cal.mag_system, MagSystem::Ab, "{bunit}");
            assert!(cal.warnings.is_empty(), "{bunit}: {:?}", cal.warnings);
            assert_eq!(cal.jy_per_native_unit, Some(jy_per_unit), "{bunit}");
            assert_eq!(cal.bunit.as_deref(), Some(bunit));
        }

        let rubin = calibrated_by_unit(&[("BUNIT", "nJy")]);
        let unit_sum = rubin.calibrate(1.0, None).unwrap().mag_ab.unwrap();
        assert!((unit_sum - 31.4).abs() < MAGNITUDE_TOLERANCE, "mag_ab={unit_sum}");
        let flux = rubin.calibrate(1000.0, Some(10.0)).unwrap();
        assert!(relative_difference(flux.flux_jy, 1e-6) < FACTOR_TOLERANCE, "flux_jy={:e}", flux.flux_jy);
        assert!((flux.mag_ab.unwrap() - 23.9).abs() < MAGNITUDE_TOLERANCE, "{flux:?}");
        assert!((flux.mag_ab_err.unwrap() - 0.01085736204758129).abs() < MAGNITUDE_TOLERANCE, "{flux:?}");
    }

    #[test]
    fn nanomaggy_is_zero_point_22_5_by_definition() {
        let plain = calibrated_by_unit(&[("BUNIT", "nanomaggy")]);
        let (jy_per_unit, unit) = flux_density_parts(&plain);
        assert!(relative_difference(jy_per_unit, JY_PER_NANOMAGGY_VALUE) < FACTOR_TOLERANCE, "{jy_per_unit:e}");
        assert_eq!(unit, "nanomaggy");
        let unit_sum = plain.calibrate(1.0, None).unwrap().mag_ab.unwrap();
        assert!((unit_sum - 22.5).abs() < MAGNITUDE_TOLERANCE, "mag_ab={unit_sum}");
        let legacy_star = plain.calibrate(87.19277, None).unwrap().mag_ab.unwrap();
        assert!((legacy_star - 17.648798812877367).abs() < MAGNITUDE_TOLERANCE, "mag_ab={legacy_star}");
        assert_eq!(plain.origin, "BUNIT nanomaggy, zero point 22.5");

        let sdss_frame = calibrated_by_unit(&[
            ("BUNIT", "nanomaggy"),
            ("NMGY", "0.00509103"),
            ("NMGYIVAR", "0.148005"),
            ("EXPTIME", "53.907456"),
            ("TELESCOP", "2.5m"),
            ("ORIGIN", "SDSS"),
            ("FILTER", "r"),
        ]);
        let (frame_factor, _) = flux_density_parts(&sdss_frame);
        assert!(relative_difference(frame_factor, JY_PER_NANOMAGGY_VALUE) < FACTOR_TOLERANCE, "{frame_factor:e}");
        assert_eq!(sdss_frame.origin, "BUNIT nanomaggy, zero point 22.5");
        assert!(sdss_frame.warnings.is_empty(), "{:?}", sdss_frame.warnings);
        assert!(!sdss_frame.origin.contains("NMGY"), "{}", sdss_frame.origin);
        assert!(!sdss_frame.notes.iter().any(|note| note.contains("NMGY")), "{:?}", sdss_frame.notes);

        let legacy_brick = calibrated_by_unit(&[("BUNIT", "nanomaggy"), ("MAGZERO", "22.5")]);
        assert_eq!(legacy_brick.origin, "BUNIT nanomaggy, zero point 22.5 (MAGZERO = 22.5 agrees)");
        assert!(legacy_brick.warnings.is_empty(), "{:?}", legacy_brick.warnings);
    }

    #[test]
    fn a_unit_and_a_zero_point_card_that_disagree_give_no_calibration() {
        let resampled_by_0_8_1 = make_header(&[("BUNIT", "nanomaggy"), ("MAGZERO", "20.99485")]);
        assert_eq!(PhotCal::from_header(&resampled_by_0_8_1, None), None);
        assert_eq!(
            missing_calibration_reason(Some(&resampled_by_0_8_1)),
            "BUNIT 'nanomaggy' means a zero point of 22.5 but MAGZERO = 20.99485: the header contradicts itself, so no calibration is applied"
        );

        let same_zero_point = calibrated_by_unit(&[("BUNIT", "uJy"), ("MAGZERO", "23.9")]);
        assert_eq!(same_zero_point.origin, "BUNIT uJy, zero point 23.9 (MAGZERO = 23.9 agrees)");
        let within_tolerance = calibrated_by_unit(&[("BUNIT", "uJy"), ("ZPT", "23.904")]);
        assert_eq!(within_tolerance.origin, "BUNIT uJy, zero point 23.9 (ZPT = 23.904 agrees)");
        assert_eq!(flux_density_parts(&within_tolerance).0, 1e-6);

        let beyond_tolerance = make_header(&[("BUNIT", "uJy"), ("ZPT", "23.91")]);
        assert_eq!(PhotCal::from_header(&beyond_tolerance, None), None);
        assert_eq!(
            missing_calibration_reason(Some(&beyond_tolerance)),
            "BUNIT 'uJy' means a zero point of 23.9 but ZPT = 23.91: the header contradicts itself, so no calibration is applied"
        );
    }

    #[test]
    fn the_unit_of_the_pixels_wins_over_count_calibration_cards() {
        let cal = calibrated_by_unit(&[
            ("BUNIT", "10.0*nanoJansky"),
            ("PHOTFLAM", "7.0723E-20"),
            ("PHOTPLAM", "8045.53"),
            ("EXPTIME", "1000"),
        ]);
        let (jy_per_unit, unit) = flux_density_parts(&cal);
        assert!(relative_difference(jy_per_unit, 1e-8) < FACTOR_TOLERANCE, "{jy_per_unit:e}");
        assert_eq!(unit, "10.0*nanoJansky");
        assert_eq!(cal.origin, "BUNIT 10.0*nanoJansky, zero point 28.9");
        assert!(
            cal.notes.iter().any(|note| note
                == "PHOTFLAM and PHOTPLAM are present but not applied: BUNIT '10.0*nanoJansky' is already a flux density"),
            "{:?}",
            cal.notes
        );
        assert!(cal.calibrate(1.0, None).unwrap().st_mag.is_none());

        let without_count_cards = calibrated_by_unit(&[("BUNIT", "10.0*nanoJansky")]);
        assert_eq!(without_count_cards.notes.len(), 1, "{:?}", without_count_cards.notes);
    }

    #[test]
    fn units_that_only_look_like_a_flux_density_stay_uncalibrated() {
        for bunit in ["Jy/beam", "Jy x channels", "Jy km/s", "MJY", "1/nanomaggy^2", "nanomaggy/arcsec2", "mag"] {
            assert_eq!(PhotCal::from_header(&make_header(&[("BUNIT", bunit)]), None), None, "{bunit}");
        }
    }

    #[test]
    fn refusals_are_worded_by_cause() {
        let reason = |cards: &[(&str, &str)]| missing_calibration_reason(Some(&make_header(cards)));
        assert_eq!(
            reason(&[]),
            "no photometric calibration in the header: BUNIT is missing and there is no zero-point card (MAGZERO, MAGZPT, ZPT, PHOTZP, ZEROPT, FLUXMAG0, FPA.ZP)"
        );
        assert_eq!(
            reason(&[("BUNIT", "ADU")]),
            "no photometric calibration in the header (BUNIT 'ADU'): a count unit needs a zero-point card (MAGZERO, MAGZPT, ZPT, PHOTZP, ZEROPT, FLUXMAG0, FPA.ZP) or PHOTFLAM with PHOTPLAM"
        );
        assert_eq!(
            reason(&[("BUNIT", "COUNTS/S")]),
            "no photometric calibration in the header (BUNIT 'COUNTS/S'): a count unit needs a zero-point card (MAGZERO, MAGZPT, ZPT, PHOTZP, ZEROPT, FLUXMAG0, FPA.ZP) or PHOTFLAM with PHOTPLAM"
        );
        assert_eq!(
            reason(&[("BUNIT", "Jy/arcsec2")]),
            "BUNIT 'Jy/arcsec2' is a surface brightness and only MJy/sr is read here; convert it to Jy per pixel with Cube Prep"
        );
        assert_eq!(
            reason(&[("BUNIT", "W/m2/Hz")]),
            "no photometric calibration in the header (BUNIT 'W/m2/Hz'): the unit is not one this calibration reads (a Jy-family or nanomaggy unit per pixel, or MJy/sr) and there is no zero-point card (MAGZERO, MAGZPT, ZPT, PHOTZP, ZEROPT, FLUXMAG0, FPA.ZP)"
        );

        assert_eq!(
            missing_calibration_reason(None),
            "no header available: photometric calibration requires BUNIT and zero-point cards"
        );
        assert_eq!(
            reason(&[("BUNIT", "MJy/sr")]),
            "BUNIT is MJy/sr but PIXAR_SR is missing and no WCS pixel scale is available"
        );
        assert_eq!(
            reason(&[("BUNIT", "mJy/sr")]),
            "BUNIT has a milli prefix (m), not mega (M); convert this surface brightness with Cube Prep before using this photometry calibration"
        );
        assert_eq!(
            reason(&[("BUNIT", "DN/s")]),
            "BUNIT 'DN/s' needs PHOTMJSR (JWST) or ROMAN_META_PHOTOMETRY_CONVERSION_MEGAJANSKYS (Roman) plus a pixel area to calibrate"
        );
        assert_eq!(reason(&[("BUNIT", "ELECTRONS"), ("PHOTFLAM", "1.9E-19")]), "HST calibration needs both PHOTFLAM and PHOTPLAM");
    }

    #[test]
    fn a_flux_density_per_pixel_serialises_its_unit() {
        let cal = calibrated_by_unit(&[("BUNIT", "nJy")]);
        let json = serde_json::to_value(&cal).unwrap();
        assert_eq!(json["convention"]["kind"], "flux_density_per_pixel", "{json}");
        assert_eq!(json["convention"]["jy_per_unit"], 1e-9, "{json}");
        assert_eq!(json["convention"]["unit"], "nJy", "{json}");
        assert_eq!(json["mag_system"], "ab", "{json}");
        assert_eq!(cal.label(), "flux density per pixel (nJy)");
        assert_eq!(cal.notes[0], "1 nJy = 1e-9 Jy; AB mag = 31.4 - 2.5 log10(sum)");
    }

    const FLUXMAG0_OF_ZERO_POINT_27: &str = "63095734448.0194";
    const ZERO_POINT_27_JY: f64 = 5.7543993733715665e-8;
    const FPA_ZP_25_OVER_600_S_JY: f64 = 6.051300912835001e-10;
    const FPA_ZP_SOURCE: &str = "FPA.ZP + 2.5 log10(EXPTIME)";
    const FPA_ZP_600_S_WARNING: &str =
        "FPA.ZP is defined for one count per second: the stack counts are divided by EXPTIME = 600.0 s";
    const FPA_ZP_WITHOUT_EXPTIME_REASON: &str = "HIERARCH FPA.ZP = 25.0 needs the EXPTIME card of the stack and the header has no positive EXPTIME: no zero point is applied";
    const ASINH_PIXELS_REASON: &str = "Pan-STARRS asinh-scaled pixels (BSOFTEN and BOFFSET cards): the values are not linear counts, so no zero point is applied";
    const PAN_STARRS_MASK_EVIDENCE: &str = "MSKNUM card of a Pan-STARRS variance or mask cutout";
    const LEGACY_ASSUMED_ORIGIN: &str = "nanomaggy assumed from SURVEY = LegacySurvey (no BUNIT or zero-point card)";
    const LEGACY_ASSUMED_WARNING: &str = "the header has no BUNIT and no zero-point card: nanomaggy (AB zero point 22.5) is assumed from SURVEY = LegacySurvey";
    const LEGACY_ASSUMED_LABEL: &str = "flux density per pixel (nanomaggy, assumed)";
    const LEGACY_CROPPED_REASON: &str = "SURVEY = LegacySurvey without a unit card: nanomaggy is assumed only for a file as the survey wrote it, and this one was processed (ABPROC = 'cropped')";
    const SURVEY_ZERO_POINT_KEY_LIST: &str = "MAGZERO, MAGZPT, ZPT, PHOTZP, ZEROPT, FLUXMAG0, FPA.ZP";

    fn fluxmag0_header() -> HduHeader {
        make_header(&[("BUNIT", "DN"), ("FLUXMAG0", FLUXMAG0_OF_ZERO_POINT_27)])
    }

    fn pan_starrs_stack_header(exptime: &str) -> HduHeader {
        make_header(&[
            ("HIERARCH FPA.ZP", "25."),
            ("EXPTIME", exptime),
            ("EXP_0001", "40."),
            ("EXP_0002", "40."),
            ("SCL_0008", "0."),
        ])
    }

    fn legacy_cutout_header(imagetyp: &str) -> HduHeader {
        make_header(&[("SURVEY", "LegacySurvey"), ("VERSION", "DR10"), ("IMAGETYP", imagetyp), ("BANDS", "griz")])
    }

    fn survey_zero_point_parts(cal: &PhotCal) -> (f64, &str, Option<f64>) {
        match &cal.convention {
            FluxConvention::SurveyZeroPoint { zp_ab, source, exptime_applied } => {
                (*zp_ab, source.as_str(), *exptime_applied)
            }
            other => panic!("unexpected convention {other:?}"),
        }
    }

    fn not_a_science_image(evidence: &str) -> String {
        format!("this layer is not a science image ({evidence}): no zero point is read for it")
    }

    fn refusal_of(header: &HduHeader) -> String {
        assert_eq!(PhotCal::from_header(header, None), None);
        missing_calibration_reason(Some(header))
    }

    fn refusal(cards: &[(&str, &str)]) -> String {
        refusal_of(&make_header(cards))
    }

    fn jansky_factor(cards: &[(&str, &str)]) -> f64 {
        calibrated_by_unit(cards).jy_per_native_unit().unwrap_or_else(|| panic!("{cards:?} has no Jy factor"))
    }

    #[test]
    fn fluxmag0_of_a_synthetic_afw_header_is_zero_point_27() {
        let without_bunit = make_header(&[("FLUXMAG0", FLUXMAG0_OF_ZERO_POINT_27)]);
        for header in [fluxmag0_header(), without_bunit] {
            let cal = PhotCal::from_header(&header, None).expect("FLUXMAG0 calibrates");
            let (zp_ab, source, exptime_applied) = survey_zero_point_parts(&cal);
            assert!((zp_ab - 27.0).abs() < MAGNITUDE_TOLERANCE, "zp_ab={zp_ab}");
            assert_eq!(source, "FLUXMAG0");
            assert_eq!(exptime_applied, None);
            let factor = cal.jy_per_native_unit().unwrap();
            assert!(relative_difference(factor, ZERO_POINT_27_JY) < FACTOR_TOLERANCE, "{factor:e}");
            let unit_sum = cal.calibrate(1.0, None).unwrap().mag_ab.unwrap();
            assert!((unit_sum - 27.0).abs() < MAGNITUDE_TOLERANCE, "mag_ab={unit_sum}");
            assert_eq!(cal.origin, "FLUXMAG0 = 6.31e10, zero point 27.00");
            assert_eq!(cal.label(), "zero point FLUXMAG0");
            assert_eq!(cal.notes, vec!["FLUXMAG0: flux [Jy] = sum x 10^(8.90 / 2.5) / FLUXMAG0".to_string()]);
            assert!(cal.warnings.is_empty(), "{:?}", cal.warnings);
            assert_eq!(cal.mag_system, MagSystem::Ab);
            assert_eq!(cal.bunit, card_text(&header, "BUNIT"));
        }
        for not_positive in ["0", "-5"] {
            let header = make_header(&[("BUNIT", "DN"), ("FLUXMAG0", not_positive)]);
            assert_eq!(PhotCal::from_header(&header, None), None, "FLUXMAG0 = {not_positive}");
        }
    }

    #[test]
    fn pan_starrs_adds_2_5_log10_of_the_exptime_card() {
        for (exptime, seconds, expected_zp_ab, expected_jy_per_count) in [
            ("602.", 602.0, 31.94899122814456, 6.031196923091406e-10),
            ("600.", 600.0, 31.94537812595911, FPA_ZP_25_OVER_600_S_JY),
            ("1710.", 1710.0, 33.08249027598038, 2.1232634781877284e-10),
        ] {
            let cal = PhotCal::from_header(&pan_starrs_stack_header(exptime), None).expect("FPA.ZP calibrates");
            let (zp_ab, source, exptime_applied) = survey_zero_point_parts(&cal);
            assert!((zp_ab - expected_zp_ab).abs() < MAGNITUDE_TOLERANCE, "EXPTIME {exptime}: zp_ab={zp_ab}");
            assert_eq!(source, FPA_ZP_SOURCE);
            assert_eq!(exptime_applied, Some(seconds));
            let factor = cal.jy_per_native_unit().unwrap();
            assert!(
                relative_difference(factor, expected_jy_per_count) < FACTOR_TOLERANCE,
                "EXPTIME {exptime}: {factor:e}"
            );
            assert_eq!(cal.mag_system, MagSystem::Ab);
            assert_eq!(cal.bunit, None);
        }

        let r_stack = PhotCal::from_header(&pan_starrs_stack_header("600."), None).unwrap();
        assert_eq!(r_stack.warnings, vec![FPA_ZP_600_S_WARNING.to_string()]);
        assert_eq!(r_stack.origin, "FPA.ZP = 25.0 with EXPTIME = 600.0 s, zero point 31.95 for one count");
        assert_eq!(r_stack.label(), "zero point FPA.ZP + 2.5 log10(EXPTIME)");
        assert_eq!(
            r_stack.notes,
            vec!["Pan-STARRS stack: flux [Jy] = sum / EXPTIME x 10^((8.90 - FPA.ZP) / 2.5); EXPTIME is the card, not the sum of the EXP_ cards".to_string()]
        );
        let ten_counts_per_second = r_stack.calibrate(6000.0, None).unwrap().mag_ab.unwrap();
        assert!((ten_counts_per_second - 22.5).abs() < MAGNITUDE_TOLERANCE, "mag_ab={ten_counts_per_second}");
    }

    #[test]
    fn pan_starrs_without_exposure_or_with_asinh_pixels_gets_no_zero_point() {
        assert_eq!(refusal(&[("HIERARCH FPA.ZP", "25.")]), FPA_ZP_WITHOUT_EXPTIME_REASON);
        assert_eq!(refusal(&[("HIERARCH FPA.ZP", "25."), ("EXPTIME", "0.")]), FPA_ZP_WITHOUT_EXPTIME_REASON);
        assert_eq!(refusal(&[("HIERARCH FPA.ZP", "25."), ("MAGZERO", "30.")]), FPA_ZP_WITHOUT_EXPTIME_REASON);

        assert_eq!(
            refusal(&[("HIERARCH FPA.ZP", "25."), ("EXPTIME", "600."), ("BSOFTEN", "99.5"), ("BOFFSET", "3.2")]),
            ASINH_PIXELS_REASON
        );
        let only_bsoften = jansky_factor(&[("HIERARCH FPA.ZP", "25."), ("EXPTIME", "600."), ("BSOFTEN", "99.5")]);
        assert!(relative_difference(only_bsoften, FPA_ZP_25_OVER_600_S_JY) < FACTOR_TOLERANCE, "{only_bsoften:e}");

        assert_eq!(
            refusal(&[("HIERARCH FPA.ZP", "25."), ("EXPTIME", "600."), ("MSKNUM", "18"), ("MSKNAM00", "DETECTOR")]),
            not_a_science_image(PAN_STARRS_MASK_EVIDENCE)
        );
    }

    #[test]
    fn a_legacy_cutout_without_unit_cards_is_nanomaggy_by_assumption_and_says_so() {
        let cal = PhotCal::from_header(&legacy_cutout_header("IMAGE"), None).expect("the Legacy cutout calibrates");
        let (jy_per_unit, unit) = flux_density_parts(&cal);
        assert_eq!(jy_per_unit, JY_PER_NANOMAGGY_VALUE);
        assert_eq!(unit, "nanomaggy");
        assert_eq!(cal.bunit, None);
        assert_eq!(cal.origin, LEGACY_ASSUMED_ORIGIN);
        assert_eq!(cal.warnings, vec![LEGACY_ASSUMED_WARNING.to_string()]);
        assert_eq!(cal.label(), LEGACY_ASSUMED_LABEL);
        assert_eq!(cal.notes, vec!["1 nanomaggy = 3.630780547701014e-6 Jy; AB mag = 22.5 - 2.5 log10(sum)".to_string()]);
        assert_eq!(cal.mag_system, MagSystem::Ab);
        assert_eq!(cal.jy_per_native_unit(), Some(JY_PER_NANOMAGGY_VALUE));
        let unit_sum = cal.calibrate(1.0, None).unwrap().mag_ab.unwrap();
        assert!((unit_sum - 22.5).abs() < MAGNITUDE_TOLERANCE, "mag_ab={unit_sum}");

        let lower_case = PhotCal::from_header(&make_header(&[("SURVEY", "legacysurvey")]), None);
        assert_eq!(lower_case.map(|cal| cal.origin).as_deref(), Some(LEGACY_ASSUMED_ORIGIN));

        assert_eq!(refusal_of(&legacy_cutout_header("INVVAR")), not_a_science_image("IMAGETYP = 'INVVAR'"));
        assert_eq!(refusal_of(&legacy_cutout_header("MASKBITS")), not_a_science_image("IMAGETYP = 'MASKBITS'"));

        let mut cropped = legacy_cutout_header("IMAGE");
        cropped.set("ABPROC", "cropped".to_string());
        assert_eq!(refusal_of(&cropped), LEGACY_CROPPED_REASON);

        assert_eq!(
            refusal(&[("SURVEY", "DECaLS+BASS+MzLS")]),
            format!(
                "no photometric calibration in the header: BUNIT is missing and there is no zero-point card ({SURVEY_ZERO_POINT_KEY_LIST})"
            )
        );
    }

    #[test]
    fn zero_point_cards_on_mask_and_weight_layers_are_not_a_calibration() {
        assert_eq!(
            refusal(&[("MAGZERO", "24.6"), ("FILTER", "VIS"), ("COMBINET", "OR"), ("RESAMPT1", "FLAGS")]),
            "this layer is not a science image (RESAMPT1 = 'FLAGS'): no zero point is read for it"
        );

        let des_weight = [("BUNIT", "electrons"), ("EXPTIME", "0."), ("DES_EXT", "WEIGHT"), ("EXTNAME", "WGT")];
        assert_eq!(refusal(&des_weight), not_a_science_image("DES_EXT = 'WEIGHT'"));
        let mut with_zero_point = des_weight.to_vec();
        with_zero_point.push(("MAGZERO", "30."));
        assert_eq!(refusal(&with_zero_point), not_a_science_image("DES_EXT = 'WEIGHT'"));

        assert_eq!(refusal(&[("EXTNAME", "MASK"), ("MAGZPT", "25.0")]), not_a_science_image("EXTNAME = 'MASK'"));
        assert_eq!(refusal(&[("IMTYPE", "maskbits")]), not_a_science_image("IMTYPE = 'maskbits'"));
        assert_eq!(
            refusal(&[("EXTNAME", " ivar "), ("FLUXMAG0", FLUXMAG0_OF_ZERO_POINT_27)]),
            not_a_science_image("EXTNAME = 'ivar'")
        );
    }

    #[test]
    fn sigma_layers_and_physical_units_keep_their_factor() {
        let sigma = jansky_factor(&[("EXTNAME", "ERR"), ("MAGZERO", "30.")]);
        assert!(relative_difference(sigma, 3.6307805477010028e-9) < FACTOR_TOLERANCE, "{sigma:e}");
        let rms = jansky_factor(&[("EXTNAME", "RMS"), ("MAGZERO", "30.")]);
        assert!(relative_difference(rms, 3.6307805477010028e-9) < FACTOR_TOLERANCE, "{rms:e}");

        let euclid_rms =
            jansky_factor(&[("MAGZERO", "24.6"), ("FILTER", "VIS"), ("COMBINET", "MEDIAN"), ("RESAMPT1", "BILINEAR")]);
        assert!(relative_difference(euclid_rms, 5.248074602497723e-7) < FACTOR_TOLERANCE, "{euclid_rms:e}");

        let variance_in_njy = calibrated_by_unit(&[("EXTNAME", "VARIANCE"), ("BUNIT", "nJy")]);
        assert_eq!(flux_density_parts(&variance_in_njy), (1e-9, "nJy"));

        let mut jwst_dq = jwst_mjy_sr_header();
        jwst_dq.set("EXTNAME", "DQ".to_string());
        let cal = PhotCal::from_header(&jwst_dq, None).expect("MJy/sr is decided before the layer guard");
        assert!(matches!(cal.convention, FluxConvention::JwstMjySr { .. }), "{:?}", cal.convention);
    }

    #[test]
    fn a_rescaled_or_display_referred_product_of_a_survey_image_is_not_calibrated() {
        for source in [fluxmag0_header(), pan_starrs_stack_header("600.")] {
            let source_factor = PhotCal::from_header(&source, None).unwrap().jy_per_native_unit();
            assert!(source_factor.is_some());
            for values in [OutputValues::Rescaled, OutputValues::DisplayReferred] {
                let product = derived_output_header(Some(&source), "pixelmath", values);
                assert_eq!(PhotCal::from_header(&product, None), None, "{values:?}");
            }
            let cropped = derived_output_header(Some(&source), "cropped", OutputValues::Linear);
            let kept = PhotCal::from_header(&cropped, None).expect("a linear product keeps its calibration");
            assert_eq!(kept.jy_per_native_unit(), source_factor);
        }

        let rubin = make_header(&[("BUNIT", "nJy")]);
        let rescaled_rubin = derived_output_header(Some(&rubin), "pixelmath", OutputValues::Rescaled);
        assert_eq!(PhotCal::from_header(&rescaled_rubin, None), None);

        let legacy = legacy_cutout_header("IMAGE");
        let rescaled_legacy = derived_output_header(Some(&legacy), "pixelmath", OutputValues::Rescaled);
        assert_eq!(PhotCal::from_header(&rescaled_legacy, None), None);
        let cropped_legacy = derived_output_header(Some(&legacy), "cropped", OutputValues::Linear);
        assert_eq!(refusal_of(&cropped_legacy), LEGACY_CROPPED_REASON);
    }

    #[test]
    fn the_decision_order_puts_fluxmag0_and_fpa_zp_before_the_generic_keys() {
        let with_generic = calibrated_by_unit(&[("FLUXMAG0", FLUXMAG0_OF_ZERO_POINT_27), ("MAGZERO", "25.0")]);
        assert_eq!(survey_zero_point_parts(&with_generic).1, "FLUXMAG0");

        let stack_with_generic =
            calibrated_by_unit(&[("HIERARCH FPA.ZP", "25."), ("EXPTIME", "600."), ("MAGZERO", "30.")]);
        assert_eq!(survey_zero_point_parts(&stack_with_generic).1, FPA_ZP_SOURCE);

        let both = calibrated_by_unit(&[
            ("HIERARCH FPA.ZP", "25."),
            ("EXPTIME", "600."),
            ("FLUXMAG0", FLUXMAG0_OF_ZERO_POINT_27),
        ]);
        assert_eq!(survey_zero_point_parts(&both).1, "FLUXMAG0");

        let unit_first = calibrated_by_unit(&[("BUNIT", "nJy"), ("FLUXMAG0", FLUXMAG0_OF_ZERO_POINT_27)]);
        assert_eq!(flux_density_parts(&unit_first), (1e-9, "nJy"));

        let legacy_with_generic = calibrated_by_unit(&[("SURVEY", "LegacySurvey"), ("MAGZERO", "22.5")]);
        assert!(
            matches!(legacy_with_generic.convention, FluxConvention::GenericZeroPoint { .. }),
            "{:?}",
            legacy_with_generic.convention
        );
    }

    #[test]
    fn a_survey_zero_point_serialises_its_source() {
        let stack = serde_json::to_value(calibrated_by_unit(&[("HIERARCH FPA.ZP", "25."), ("EXPTIME", "600.")])).unwrap();
        assert_eq!(stack["convention"]["kind"], "survey_zero_point", "{stack}");
        assert_eq!(stack["convention"]["source"], FPA_ZP_SOURCE, "{stack}");
        assert_eq!(stack["convention"]["exptime_applied"], 600.0, "{stack}");
        assert!((stack["convention"]["zp_ab"].as_f64().unwrap() - 31.94537812595911).abs() < MAGNITUDE_TOLERANCE, "{stack}");
        assert_eq!(stack["mag_system"], "ab", "{stack}");

        let afw = serde_json::to_value(PhotCal::from_header(&fluxmag0_header(), None).unwrap()).unwrap();
        assert_eq!(afw["convention"]["kind"], "survey_zero_point", "{afw}");
        assert_eq!(afw["convention"]["source"], "FLUXMAG0", "{afw}");
        assert!(afw["convention"].as_object().unwrap().contains_key("exptime_applied"), "{afw}");
        assert!(afw["convention"]["exptime_applied"].is_null(), "{afw}");
    }

    const SCIENCE_DATA_DIR_VAR: &str = "ASTROBURST_SCIENCE_DATA_DIR";
    const REAL_FACTOR_TOLERANCE: f64 = 5e-7;
    const MICROJANSKY_PER_JANSKY: f64 = 1e6;

    fn science_header(relative: &str) -> HduHeader {
        let path = env_dir(SCIENCE_DATA_DIR_VAR).join(relative);
        load_plane_header(&ImageRef::parse(&path.to_string_lossy()))
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1/des-dr2_A_{g,r,i}_extn1.fits"]
    fn real_des_d1_sci_cutouts_take_magzero_30_without_an_exposure_term() {
        let golden_ujy_per_pixel_unit = 0.0036307805477010066;
        for band in ["g", "r", "i"] {
            let relative = format!("survey/D1/des-dr2_A_{band}_extn1.fits");
            let cal = PhotCal::from_header(&science_header(&relative), None).unwrap();
            let ujy_per_pixel_unit = cal.jy_per_native_unit().unwrap() * MICROJANSKY_PER_JANSKY;
            println!("{relative}: {ujy_per_pixel_unit:e} uJy per pixel unit (golden {golden_ujy_per_pixel_unit:e})");
            assert!(
                relative_difference(ujy_per_pixel_unit, golden_ujy_per_pixel_unit) < REAL_FACTOR_TOLERANCE,
                "{relative}: {ujy_per_pixel_unit:e}"
            );
            assert!(
                matches!(
                    &cal.convention,
                    FluxConvention::GenericZeroPoint {
                        keyword,
                        per_second: true,
                        exptime: Some(t),
                        exptime_applied: false,
                        ..
                    } if keyword == "MAGZERO" && *t == 1080.0
                ),
                "{relative}: {:?}",
                cal.convention
            );
            assert_eq!(cal.origin, "MAGZERO = 30.0, AB assumed", "{relative}");
            assert!(!cal.warnings.iter().any(|warning| warning.contains("EXPTIME")), "{relative}: {:?}", cal.warnings);
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits"]
    fn real_euclid_e1_vis_science_cutout_takes_magzero_24_6() {
        let relative =
            "survey/E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits";
        let golden_ujy_per_pixel_unit = 0.5248074602497712;
        let cal = PhotCal::from_header(&science_header(relative), None).unwrap();
        let ujy_per_pixel_unit = cal.jy_per_native_unit().unwrap() * MICROJANSKY_PER_JANSKY;
        println!("{relative}: {ujy_per_pixel_unit:e} uJy per pixel unit (golden {golden_ujy_per_pixel_unit:e})");
        assert!(
            relative_difference(ujy_per_pixel_unit, golden_ujy_per_pixel_unit) < REAL_FACTOR_TOLERANCE,
            "{ujy_per_pixel_unit:e}"
        );
        assert_eq!(cal.origin, "MAGZERO = 24.6, AB assumed");
        assert_eq!(cal.bunit, None);
    }

    const GOLDEN_UJY_PER_NANOMAGGY: f64 = 3.630780547701009;

    fn real_nanomaggy_calibration(relative: &str) -> PhotCal {
        let cal = PhotCal::from_header(&science_header(relative), None)
            .unwrap_or_else(|| panic!("{relative} is not calibrated"));
        let ujy_per_pixel_unit = cal.jy_per_native_unit().unwrap() * MICROJANSKY_PER_JANSKY;
        println!("{relative}: {ujy_per_pixel_unit:e} uJy per pixel unit (golden {GOLDEN_UJY_PER_NANOMAGGY:e})");
        assert!(
            relative_difference(ujy_per_pixel_unit, GOLDEN_UJY_PER_NANOMAGGY) < REAL_FACTOR_TOLERANCE,
            "{relative}: {ujy_per_pixel_unit:e}"
        );
        assert_eq!(flux_density_parts(&cal).1, "nanomaggy", "{relative}");
        cal
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-image-{g,r,z}.fits.fz"]
    fn real_legacy_l2_brick_images_are_nanomaggy_with_an_agreeing_magzero() {
        for band in ["g", "r", "z"] {
            let relative = format!("survey/L2/legacysurvey-0532m280-image-{band}.fits.fz");
            let cal = real_nanomaggy_calibration(&relative);
            assert_eq!(cal.origin, "BUNIT nanomaggy, zero point 22.5 (MAGZERO = 22.5 agrees)", "{relative}");
            assert!(cal.warnings.is_empty(), "{relative}: {:?}", cal.warnings);
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-invvar-r.fits.fz"]
    fn real_legacy_l2_inverse_variance_is_not_a_flux_density() {
        let header = science_header("survey/L2/legacysurvey-0532m280-invvar-r.fits.fz");
        assert_eq!(card_text(&header, "BUNIT").as_deref(), Some("1/nanomaggy^2"));
        assert_eq!(PhotCal::from_header(&header, None), None);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/S1/frame-{g,r,i}-001462-4-0165.fits.bz2"]
    fn real_sdss_s1_frames_are_nanomaggy_and_the_nmgy_card_is_not_read() {
        for band in ["g", "r", "i"] {
            let relative = format!("survey/S1/frame-{band}-001462-4-0165.fits.bz2");
            let cal = real_nanomaggy_calibration(&relative);
            assert_eq!(cal.origin, "BUNIT nanomaggy, zero point 22.5", "{relative}");
        }
    }

    fn real_refusal(relative: &str) -> String {
        let reason = refusal_of(&science_header(relative));
        println!("{relative}: no factor ({reason})");
        reason
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_{g,r,i}_stack_480px.fits"]
    fn real_ps1_p1_stack_cutouts_use_fpa_zp_and_the_exptime_card() {
        for (band, golden_ujy_per_count, exptime) in [
            ("g", 6.031196923091381e-4, 602.0),
            ("r", 6.051300912835008e-4, 600.0),
            ("i", 2.1232634781877263e-4, 1710.0),
        ] {
            let relative = format!("survey/P1/ps1_A_{band}_stack_480px.fits");
            let cal = PhotCal::from_header(&science_header(&relative), None)
                .unwrap_or_else(|| panic!("{relative} is not calibrated"));
            let ujy_per_count = cal.jy_per_native_unit().unwrap() * MICROJANSKY_PER_JANSKY;
            println!(
                "{relative}: {ujy_per_count:e} uJy per pixel unit (golden {golden_ujy_per_count:e}), origin {}",
                cal.origin
            );
            assert!(
                relative_difference(ujy_per_count, golden_ujy_per_count) < REAL_FACTOR_TOLERANCE,
                "{relative}: {ujy_per_count:e}"
            );
            let (_, source, exptime_applied) = survey_zero_point_parts(&cal);
            assert_eq!(source, FPA_ZP_SOURCE, "{relative}");
            assert_eq!(exptime_applied, Some(exptime), "{relative}");
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_r_{wt,mask}_480px.fits"]
    fn real_ps1_p1_variance_and_mask_cutouts_get_no_zero_point() {
        for layer in ["wt", "mask"] {
            let relative = format!("survey/P1/ps1_A_r_{layer}_480px.fits");
            assert_eq!(real_refusal(&relative), not_a_science_image(PAN_STARRS_MASK_EVIDENCE), "{relative}");
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1/EUC_MER_MOSAIC-VIS-{FLAG,RMS}_TILE102044185-*_cut60_A.fits"]
    fn real_euclid_e1_flag_cutout_ignores_its_magzero_card() {
        let flag =
            "survey/E1/EUC_MER_MOSAIC-VIS-FLAG_TILE102044185-A61C42_20241021T010718.208008Z_00.00_cut60_A.fits";
        assert_eq!(science_header(flag).get_f64("MAGZERO"), Some(24.6));
        assert_eq!(real_refusal(flag), not_a_science_image("RESAMPT1 = 'FLAGS'"));

        let rms = "survey/E1/EUC_MER_MOSAIC-VIS-RMS_TILE102044185-8BF2FC_20241021T010718.207983Z_00.00_cut60_A.fits";
        let golden_ujy_per_pixel_unit = 0.5248074602497712;
        let cal = PhotCal::from_header(&science_header(rms), None).unwrap_or_else(|| panic!("{rms} is not calibrated"));
        let ujy_per_pixel_unit = cal.jy_per_native_unit().unwrap() * MICROJANSKY_PER_JANSKY;
        println!(
            "{rms}: {ujy_per_pixel_unit:e} uJy per pixel unit (golden {golden_ujy_per_pixel_unit:e}), origin {}",
            cal.origin
        );
        assert!(
            relative_difference(ujy_per_pixel_unit, golden_ujy_per_pixel_unit) < REAL_FACTOR_TOLERANCE,
            "{ujy_per_pixel_unit:e}"
        );
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1/des-dr2_A_r_extn{2,3}.fits"]
    fn real_des_d1_mask_and_weight_cutouts_are_not_calibrated() {
        for (extension, evidence) in [("extn2", "DES_EXT = 'MASK'"), ("extn3", "DES_EXT = 'WEIGHT'")] {
            let relative = format!("survey/D1/des-dr2_A_r_{extension}.fits");
            assert_eq!(real_refusal(&relative), not_a_science_image(evidence), "{relative}");
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L1/ls-dr10_A_griz_invvar_maskbits_458px.fits"]
    fn real_legacy_l1_cutout_is_nanomaggy_by_assumption() {
        let file = "survey/L1/ls-dr10_A_griz_invvar_maskbits_458px.fits";
        let image = real_nanomaggy_calibration(&format!("{file}#hdu=0"));
        println!("{file}#hdu=0: origin {}", image.origin);
        assert_eq!(image.origin, LEGACY_ASSUMED_ORIGIN);
        assert_eq!(image.warnings, vec![LEGACY_ASSUMED_WARNING.to_string()]);
        assert_eq!(image.label(), LEGACY_ASSUMED_LABEL);
        assert_eq!(image.bunit, None);

        for (hdu, evidence) in [(1, "IMAGETYP = 'INVVAR'"), (2, "IMAGETYP = 'MASKBITS'")] {
            assert_eq!(real_refusal(&format!("{file}#hdu={hdu}")), not_a_science_image(evidence), "HDU {hdu}");
        }
    }
}
