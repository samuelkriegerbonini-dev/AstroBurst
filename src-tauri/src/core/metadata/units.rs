use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::core::alignment::wcs_reproject::pixel_area_ratio;
use crate::core::analysis::photometry::SATURATION_KEYWORDS;
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::imaging::push_history;
use crate::core::metadata::photcal::{
    card_text, is_mjy_per_sr, is_rate_unit, normalized_unit, pixel_area_sr, ARCSEC_PER_RADIAN, GENERIC_ZERO_POINT_KEYS,
    JY_PER_MJY, ROMAN_CONVERSION_MJY_KEYS, ROMAN_PIXEL_AREA_SR_KEYS, SURVEY_ZERO_POINT_KEYS,
};
use crate::types::header::HduHeader;

pub const SPEED_OF_LIGHT_ANGSTROM_PER_S: f64 = 2.997_924_58e18;
pub const ERG_S_CM2_HZ_PER_JY: f64 = 1e-23;
pub const JY_PER_NANOMAGGY: f64 = 3.630_780_547_701_014e-6;
pub const BUNIT_JY: &str = "Jy";
pub const BUNIT_MJY_SR: &str = "MJy/sr";
pub const CARD_FLUXCONV: &str = "FLUXCONV";
pub const CARD_BUNIT_IN: &str = "BUNIT_IN";

const PIXEL_AREA_CARDS: [&str; 2] = ["PIXAR_SR", "PIXAR_A2"];
const PER_PIXEL_FLUX_CARDS: [&str; 2] = ["PHOTFLAM", "PHOTFNU"];
const CALIBRATION_ZERO_POINT_CARDS: [&str; 2] = ["ZP", "ZPTMAG"];
const COUNT_CALIBRATION_CARDS: [&str; 3] = ["PHOTFNU", "PHOTFLAM", "PHOTMJSR"];
const PER_STERADIAN_SUFFIXES: [&str; 6] = ["/sr", "/steradian", ".sr**-1", "sr**-1", ".sr-1", "sr-1"];
const PER_ARCSEC2_SUFFIXES: [&str; 5] = ["/arcsec2", "/arcsec**2", "/arcsec^2", ".arcsec-2", ".arcsec**-2"];
const PER_PIXEL_SUFFIXES: [&str; 4] = ["/pixel", "/pix", ".pixel**-1", ".pix**-1"];
const JANSKY_NAMES: [&str; 2] = ["jansky", "jy"];
const FITS_MEGAJANSKY: &str = "MJy";
const NANOMAGGY_NAMES: [&str; 3] = ["nanomaggy", "nanomaggies", "nmgy"];
const LIGHTSTACK_SCALE_RTOL: f64 = 1e-6;
const LIGHTSTACK_SCALE_ATOL: f64 = 1e-8;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "class", rename_all = "snake_case")]
pub enum UnitClass {
    SurfaceBrightness { jy_per_sr_per_unit: f64 },
    PerPixelCalibrated { cards: Vec<String> },
    PerPixelPhysical { jy_per_unit: f64 },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UnitInfo {
    pub bunit: Option<String>,
    pub class: UnitClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetUnit {
    JyPerPixel,
    MjyPerSr,
}

impl TargetUnit {
    pub fn bunit(self) -> &'static str {
        match self {
            Self::JyPerPixel => BUNIT_JY,
            Self::MjyPerSr => BUNIT_MJY_SR,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::JyPerPixel => "Jy/pixel",
            Self::MjyPerSr => "MJy/sr",
        }
    }

    pub fn file_suffix(self) -> &'static str {
        match self {
            Self::JyPerPixel => "Jy",
            Self::MjyPerSr => "MJysr",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaRule {
    Keep,
    Scale,
    Lightstack,
}

impl AreaRule {
    pub fn label(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Scale => "scale",
            Self::Lightstack => "lightstack",
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum UnitError {
    #[error("BUNIT {} has no known area behaviour: it is not a surface brightness, a flux density per pixel or calibrated per-pixel counts; pass area_rule keep, scale or lightstack", quoted(.bunit))]
    UnknownUnit { bunit: Option<String> },
    #[error("cannot convert BUNIT {} to {target}: {reason}", quoted(.bunit))]
    NoConversion { bunit: Option<String>, target: &'static str, reason: String },
    #[error("the pixel geometry is not usable: area ratio {area_ratio}, first-axis scales {scale_in_arcsec} and {scale_out_arcsec} arcsec")]
    InvalidGeometry { area_ratio: f64, scale_in_arcsec: f64, scale_out_arcsec: f64 },
}

fn quoted(bunit: &Option<String>) -> String {
    bunit.as_deref().map_or_else(|| "(missing)".to_string(), |text| format!("'{text}'"))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UnitConversion {
    pub bunit_in: Option<String>,
    pub bunit_out: &'static str,
    pub factor: f64,
    pub method: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PixelGeometry {
    pub area_ratio: f64,
    pub scale_in_arcsec: f64,
    pub scale_out_arcsec: f64,
}

impl PixelGeometry {
    pub fn between(input: &WcsTransform, output: &WcsTransform) -> Self {
        Self {
            area_ratio: pixel_area_ratio(output, input),
            scale_in_arcsec: first_axis_scale_arcsec(input),
            scale_out_arcsec: first_axis_scale_arcsec(output),
        }
    }

    fn is_usable(&self) -> bool {
        [self.area_ratio, self.scale_in_arcsec, self.scale_out_arcsec].iter().all(|value| value.is_finite() && *value > 0.0)
    }

    fn invalid(&self) -> UnitError {
        UnitError::InvalidGeometry {
            area_ratio: self.area_ratio,
            scale_in_arcsec: self.scale_in_arcsec,
            scale_out_arcsec: self.scale_out_arcsec,
        }
    }
}

fn first_axis_scale_arcsec(wcs: &WcsTransform) -> f64 {
    let cd = wcs.raw_params().4;
    cd[0][0].hypot(cd[1][0]) * 3600.0
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AreaCorrection {
    pub rule: AreaRule,
    pub requested: bool,
    pub data_factor: f64,
    pub area_ratio: f64,
}

fn surface_brightness_scale(bunit: &str) -> Option<f64> {
    let unit: String = bunit.chars().filter(|c| !c.is_whitespace()).collect();
    if is_mjy_per_sr(&unit) {
        return Some(JY_PER_MJY);
    }
    [(PER_STERADIAN_SUFFIXES.as_slice(), 1.0), (PER_ARCSEC2_SUFFIXES.as_slice(), ARCSEC_PER_RADIAN * ARCSEC_PER_RADIAN)]
        .into_iter()
        .find_map(|(suffixes, area)| suffixes.iter().find_map(|suffix| {
            let start = unit.len().checked_sub(suffix.len())?;
            unit.get(start..).filter(|tail| tail.eq_ignore_ascii_case(suffix))?;
            Some(jansky_scale(&unit[..start])? * area)
        }))
}

fn strip_per_pixel(unit: &str) -> &str {
    PER_PIXEL_SUFFIXES
        .iter()
        .find_map(|suffix| {
            let start = unit.len().checked_sub(suffix.len())?;
            unit.get(start..).filter(|tail| tail.eq_ignore_ascii_case(suffix)).map(|_| &unit[..start])
        })
        .unwrap_or(unit)
}

fn prefix_scale(prefix: &str) -> Option<f64> {
    match prefix {
        "" => Some(1.0),
        "m" => Some(1e-3),
        "u" | "\u{b5}" | "\u{3bc}" => Some(1e-6),
        "n" => Some(1e-9),
        word => match word.to_ascii_lowercase().as_str() {
            "mega" => Some(1e6),
            "milli" => Some(1e-3),
            "micro" => Some(1e-6),
            "nano" => Some(1e-9),
            _ => None,
        },
    }
}

fn jansky_scale(unit: &str) -> Option<f64> {
    if unit == FITS_MEGAJANSKY {
        return Some(JY_PER_MJY);
    }
    if NANOMAGGY_NAMES.iter().any(|name| unit.eq_ignore_ascii_case(name)) {
        return Some(JY_PER_NANOMAGGY);
    }
    JANSKY_NAMES.iter().find_map(|name| {
        let start = unit.len().checked_sub(name.len())?;
        unit.get(start..).filter(|tail| tail.eq_ignore_ascii_case(name))?;
        prefix_scale(&unit[..start])
    })
}

pub(crate) fn per_pixel_jansky_scale(bunit: &str) -> Option<f64> {
    let compact: String = bunit.chars().filter(|c| !c.is_whitespace()).collect();
    let (multiplier, unit) = match compact.split_once('*').map(|(number, rest)| (number.parse::<f64>(), rest)) {
        Some((Ok(multiplier), rest)) => (multiplier, rest),
        _ => (1.0, compact.as_str()),
    };
    let scale = multiplier * jansky_scale(strip_per_pixel(unit))?;
    (scale.is_finite() && scale > 0.0).then_some(scale)
}

fn calibration_cards(header: &HduHeader) -> Vec<String> {
    COUNT_CALIBRATION_CARDS
        .into_iter()
        .chain(ROMAN_CONVERSION_MJY_KEYS)
        .chain(GENERIC_ZERO_POINT_KEYS)
        .chain(CALIBRATION_ZERO_POINT_CARDS)
        .filter(|key| header.get_f64(key).is_some_and(f64::is_finite))
        .map(String::from)
        .collect()
}

pub fn classify_unit(header: &HduHeader) -> UnitInfo {
    let bunit = card_text(header, "BUNIT");
    let class = if let Some(jy_per_sr_per_unit) = bunit.as_deref().and_then(surface_brightness_scale) {
        UnitClass::SurfaceBrightness { jy_per_sr_per_unit }
    } else if let Some(jy_per_unit) = bunit.as_deref().and_then(per_pixel_jansky_scale) {
        UnitClass::PerPixelPhysical { jy_per_unit }
    } else {
        let cards = calibration_cards(header);
        if cards.is_empty() {
            UnitClass::Unknown
        } else {
            UnitClass::PerPixelCalibrated { cards }
        }
    };
    UnitInfo { bunit, class }
}

fn positive_card(header: &HduHeader, key: &str) -> Option<f64> {
    header.get_f64(key).filter(|value| value.is_finite() && *value > 0.0)
}

struct Factor {
    value: f64,
    method: String,
}

fn count_rate_jansky(header: &HduHeader, bunit: Option<&str>) -> Result<Factor, String> {
    let unit = bunit.map(normalized_unit).unwrap_or_default();
    if !is_rate_unit(&unit) {
        return Err(format!(
            "only count rates convert (LIGHTSTACK counts_s_to_jy); BUNIT {} is not per second",
            bunit.unwrap_or("(missing)")
        ));
    }
    if let Some(photfnu) = positive_card(header, "PHOTFNU") {
        return Ok(Factor { value: photfnu, method: format!("PHOTFNU {photfnu:E} Jy per count/s") });
    }
    match (positive_card(header, "PHOTFLAM"), positive_card(header, "PHOTPLAM")) {
        (Some(photflam), Some(photplam)) => Ok(Factor {
            value: photflam * photplam.powi(2) / SPEED_OF_LIGHT_ANGSTROM_PER_S / ERG_S_CM2_HZ_PER_JY,
            method: format!("PHOTFLAM {photflam:E} * PHOTPLAM {photplam}^2 / c"),
        }),
        _ => Err("a count rate needs PHOTFNU, or PHOTFLAM and PHOTPLAM".into()),
    }
}

fn pixel_area(header: &HduHeader, wcs: Option<&WcsTransform>, warnings: &mut Vec<String>) -> Result<(f64, String), String> {
    let area = pixel_area_sr(header, &["PIXAR_SR"], wcs, warnings).ok_or("there is no PIXAR_SR card and no WCS pixel area")?;
    let source = if area.derived { "WCS pixel area" } else { "PIXAR_SR" };
    Ok((area.steradians, format!("{source} {:E} sr", area.steradians)))
}

fn jansky_per_pixel(info: &UnitInfo, header: &HduHeader, wcs: Option<&WcsTransform>, warnings: &mut Vec<String>) -> Result<Factor, String> {
    match &info.class {
        UnitClass::SurfaceBrightness { jy_per_sr_per_unit } => {
            let (steradians, source) = pixel_area(header, wcs, warnings)?;
            Ok(Factor { value: jy_per_sr_per_unit * steradians, method: format!("{jy_per_sr_per_unit:E} Jy/sr * {source}") })
        }
        UnitClass::PerPixelPhysical { jy_per_unit } => {
            Ok(Factor { value: *jy_per_unit, method: format!("{jy_per_unit:E} Jy per {}", info.bunit.as_deref().unwrap_or("unit")) })
        }
        UnitClass::PerPixelCalibrated { .. } => count_rate_jansky(header, info.bunit.as_deref()),
        UnitClass::Unknown => Err(
            "it is neither a surface brightness, a flux density per pixel nor a count rate with PHOTFNU or PHOTFLAM and PHOTPLAM".into(),
        ),
    }
}

pub fn unit_conversion(header: &HduHeader, wcs: Option<&WcsTransform>, target: TargetUnit) -> Result<UnitConversion, UnitError> {
    let info = classify_unit(header);
    let mut warnings = Vec::new();
    let factor = match (&info.class, target) {
        (UnitClass::SurfaceBrightness { jy_per_sr_per_unit }, TargetUnit::MjyPerSr) => Ok(Factor {
            value: jy_per_sr_per_unit / JY_PER_MJY,
            method: format!("{jy_per_sr_per_unit:E} Jy/sr per unit"),
        }),
        (_, TargetUnit::JyPerPixel) => jansky_per_pixel(&info, header, wcs, &mut warnings),
        (_, TargetUnit::MjyPerSr) => jansky_per_pixel(&info, header, wcs, &mut warnings).and_then(|jansky| {
            let (steradians, source) = pixel_area(header, wcs, &mut warnings)?;
            Ok(Factor { value: jansky.value / (JY_PER_MJY * steradians), method: format!("{} / (1e6 * {source})", jansky.method) })
        }),
    }
    .map_err(|reason| UnitError::NoConversion { bunit: info.bunit.clone(), target: target.label(), reason })?;
    if !(factor.value.is_finite() && factor.value > 0.0) {
        return Err(UnitError::NoConversion {
            bunit: info.bunit,
            target: target.label(),
            reason: format!("the conversion factor {} is not a finite positive number", factor.value),
        });
    }
    Ok(UnitConversion { bunit_in: info.bunit, bunit_out: target.bunit(), factor: factor.value, method: factor.method, warnings })
}

fn scale_planes(factor: f64, sci: &mut Array2<f32>, err: Option<&mut Array2<f32>>) {
    if factor == 1.0 {
        return;
    }
    sci.mapv_inplace(|value| (f64::from(value) * factor) as f32);
    if let Some(err) = err {
        err.mapv_inplace(|value| (f64::from(value) * factor) as f32);
    }
}

fn remove_count_calibration(header: &mut HduHeader) -> Vec<(String, String)> {
    let keys: Vec<&str> = PER_PIXEL_FLUX_CARDS.into_iter().chain(GENERIC_ZERO_POINT_KEYS).chain(CALIBRATION_ZERO_POINT_CARDS).collect();
    keys.into_iter()
        .filter_map(|key| {
            let value = header.get(key)?.trim().to_string();
            header.remove(key);
            Some((key.to_string(), value))
        })
        .collect()
}

fn record_conversion(header: &mut HduHeader, conversion: &UnitConversion) {
    let previous = header.get_f64(CARD_FLUXCONV).filter(|value| value.is_finite() && *value > 0.0).unwrap_or(1.0);
    let original = card_text(header, CARD_BUNIT_IN).or_else(|| conversion.bunit_in.clone()).unwrap_or_default();
    header.set("BUNIT", conversion.bunit_out.to_string());
    header.set_f64(CARD_FLUXCONV, previous * conversion.factor);
    header.set(CARD_BUNIT_IN, original);
}

pub fn apply_conversion(
    conversion: &UnitConversion,
    sci: &mut Array2<f32>,
    err: Option<&mut Array2<f32>>,
    header: &mut HduHeader,
    err_header: Option<&mut HduHeader>,
) {
    scale_planes(conversion.factor, sci, err);
    let bunit_in = conversion.bunit_in.as_deref().unwrap_or("(missing)");
    let removed = remove_count_calibration(header);
    record_conversion(header, conversion);
    if let Some(err_header) = err_header {
        remove_count_calibration(err_header);
        record_conversion(err_header, conversion);
    }
    push_history(
        header,
        &format!("BUNIT {bunit_in} converted to {} by x {:E} ({})", conversion.bunit_out, conversion.factor, conversion.method),
    );
    for (key, value) in removed {
        push_history(header, &format!("{key} = {value} removed: it calibrated {bunit_in}, not {}", conversion.bunit_out));
    }
}

fn lightstack_area_factor(geometry: &PixelGeometry) -> f64 {
    let (scale_in, scale_out) = (geometry.scale_in_arcsec, geometry.scale_out_arcsec);
    if (scale_in - scale_out).abs() <= LIGHTSTACK_SCALE_ATOL + LIGHTSTACK_SCALE_RTOL * scale_out.abs() {
        1.0
    } else {
        (scale_out / scale_in).powi(2)
    }
}

pub fn area_correction(header: &HduHeader, requested: Option<AreaRule>, geometry: &PixelGeometry) -> Result<AreaCorrection, UnitError> {
    if !geometry.is_usable() {
        return Err(geometry.invalid());
    }
    let rule = match requested {
        Some(rule) => rule,
        None => match classify_unit(header) {
            UnitInfo { class: UnitClass::SurfaceBrightness { .. } | UnitClass::PerPixelCalibrated { .. }, .. } => AreaRule::Keep,
            UnitInfo { class: UnitClass::PerPixelPhysical { .. }, .. } => AreaRule::Scale,
            UnitInfo { bunit, class: UnitClass::Unknown } => return Err(UnitError::UnknownUnit { bunit }),
        },
    };
    let data_factor = match rule {
        AreaRule::Keep => 1.0,
        AreaRule::Scale => geometry.area_ratio,
        AreaRule::Lightstack => lightstack_area_factor(geometry),
    };
    Ok(AreaCorrection { rule, requested: requested.is_some(), data_factor, area_ratio: geometry.area_ratio })
}

pub fn apply_area_correction(correction: &AreaCorrection, sci: &mut Array2<f32>, err: Option<&mut Array2<f32>>, header: &mut HduHeader) {
    scale_planes(correction.data_factor, sci, err);
    match correction.rule {
        AreaRule::Keep => rescale_per_pixel_calibration(header, correction.area_ratio),
        AreaRule::Scale | AreaRule::Lightstack => {
            rescale_pixel_area_cards(header, correction.area_ratio);
            remove_saturation_cards(header);
        }
    }
    push_history(
        header,
        &format!(
            "Area rule {}: data x {:.6}, pixel area x {:.6}",
            correction.rule.label(),
            correction.data_factor,
            correction.area_ratio
        ),
    );
}

fn rescale_pixel_area_cards(header: &mut HduHeader, area_ratio: f64) {
    for &key in PIXEL_AREA_CARDS.iter().chain(ROMAN_PIXEL_AREA_SR_KEYS.iter()) {
        if let Some(v) = header.get_f64(key) {
            header.set_f64(key, v * area_ratio);
        }
    }
}

fn rescale_calibration_cards(header: &mut HduHeader, area_ratio: f64) {
    for key in PER_PIXEL_FLUX_CARDS {
        if let Some(v) = header.get_f64(key) {
            header.set_f64(key, v * area_ratio);
        }
    }
    let zero_point_shift = 2.5 * area_ratio.log10();
    for key in GENERIC_ZERO_POINT_KEYS.into_iter().chain(CALIBRATION_ZERO_POINT_CARDS) {
        if let Some(v) = header.get_f64(key) {
            header.set_f64(key, v - zero_point_shift);
        }
    }
    let [fluxmag0, fpa_zp] = SURVEY_ZERO_POINT_KEYS;
    if let Some(v) = header.get_f64(fluxmag0) {
        header.set_f64(fluxmag0, v / area_ratio);
    }
    if let Some(v) = header.get_f64(fpa_zp) {
        header.set_f64(fpa_zp, v - zero_point_shift);
    }
}

fn remove_saturation_cards(header: &mut HduHeader) {
    for key in SATURATION_KEYWORDS {
        header.remove(key);
    }
}

pub fn rescale_per_pixel_calibration(header: &mut HduHeader, area_ratio: f64) {
    if !area_ratio.is_finite() || area_ratio <= 0.0 {
        return;
    }
    let flux_density_unit =
        card_text(header, "BUNIT").filter(|unit| area_ratio != 1.0 && per_pixel_jansky_scale(unit).is_some());
    if let Some(unit) = flux_density_unit {
        header.remove("BUNIT");
        push_history(
            header,
            &format!("BUNIT {unit} removed: the values are per input pixel after a pixel-area change by x {area_ratio:.6}"),
        );
    }
    rescale_pixel_area_cards(header, area_ratio);
    rescale_calibration_cards(header, area_ratio);
    remove_saturation_cards(header);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::region::test_support::{make_header, wcs_cards};
    use crate::core::metadata::photcal::{card_text, FluxConvention, PhotCal, ARCSEC_PER_RADIAN};

    fn geometry_2x() -> PixelGeometry {
        PixelGeometry { area_ratio: 4.0, scale_in_arcsec: 0.02, scale_out_arcsec: 0.04 }
    }

    fn history(header: &HduHeader) -> Vec<String> {
        header.cards.iter().filter(|(key, _)| key == "HISTORY").map(|(_, value)| value.clone()).collect()
    }

    fn history_text(header: &HduHeader) -> String {
        history(header).join(" ")
    }

    fn relative(got: Option<f64>, expected: f64) -> f64 {
        got.map_or(f64::INFINITY, |value| (value - expected).abs() / expected.abs())
    }

    fn header_with_wcs(cd: [[f64; 2]; 2], extra: &[(&str, &str)]) -> (HduHeader, WcsTransform) {
        let mut cards = wcs_cards(cd);
        cards.extend(extra.iter().map(|(key, value)| (key.to_string(), value.to_string())));
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(key, value)| (key.as_str(), value.as_str())).collect();
        let header = make_header(&pairs);
        let wcs = WcsTransform::from_header(&header).unwrap();
        (header, wcs)
    }

    #[test]
    fn mjy_sr_to_jy_matches_lightstack_formula() {
        let mut header = make_header(&[("BUNIT", "MJy/sr"), ("PIXAR_SR", "2.1E-13"), ("PIXAR_A2", "8.9E-4")]);
        let conversion = unit_conversion(&header, None, TargetUnit::JyPerPixel).unwrap();
        let factor = 1e6 * 2.1e-13;
        assert_eq!(conversion.factor, factor);
        assert_eq!(conversion.bunit_in.as_deref(), Some("MJy/sr"));
        assert_eq!(conversion.bunit_out, BUNIT_JY);
        assert!(conversion.method.contains("PIXAR_SR"), "{}", conversion.method);
        assert!(conversion.warnings.is_empty(), "{:?}", conversion.warnings);

        let mut sci = Array2::from_shape_vec((1, 3), vec![1000.0f32, -2.5, f32::NAN]).unwrap();
        let mut err = Array2::from_elem((1, 3), 4.0f32);
        let mut err_header = make_header(&[("BUNIT", "MJy/sr")]);
        apply_conversion(&conversion, &mut sci, Some(&mut err), &mut header, Some(&mut err_header));
        assert_eq!(sci[[0, 0]], (1000.0 * factor) as f32);
        assert_eq!(sci[[0, 1]], (-2.5 * factor) as f32);
        assert!(sci[[0, 2]].is_nan());
        assert!(err.iter().all(|value| *value == (4.0 * factor) as f32));
        for written in [&header, &err_header] {
            assert_eq!(card_text(written, "BUNIT").as_deref(), Some(BUNIT_JY));
            assert!(relative(written.get_f64(CARD_FLUXCONV), factor) < 1e-13);
            assert_eq!(card_text(written, CARD_BUNIT_IN).as_deref(), Some("MJy/sr"));
        }
        assert_eq!(header.get_f64("PIXAR_SR"), Some(2.1e-13), "the pixel area stays, as LIGHTSTACK keeps it");
        let text = history_text(&header);
        assert!(text.contains("BUNIT MJy/sr converted to Jy") && text.contains("PIXAR_SR"), "{text}");
    }

    #[test]
    fn photfnu_takes_precedence_over_photflam() {
        let (photflam, photplam) = (7.0723e-20f64, 8045.53f64);
        let cards = [("BUNIT", "ELECTRONS/S"), ("PHOTFLAM", "7.0723E-20"), ("PHOTPLAM", "8045.53"), ("MAGZERO", "25.0")];
        let mut with_photfnu = make_header(&cards);
        with_photfnu.set("PHOTFNU", "3.0E-07".to_string());
        let conversion = unit_conversion(&with_photfnu, None, TargetUnit::JyPerPixel).unwrap();
        assert_eq!(conversion.factor, 3.0e-7);
        assert!(conversion.method.contains("PHOTFNU"), "{}", conversion.method);

        let from_photflam = unit_conversion(&make_header(&cards), None, TargetUnit::JyPerPixel).unwrap();
        assert_eq!(from_photflam.factor, photflam * photplam.powi(2) / SPEED_OF_LIGHT_ANGSTROM_PER_S / 1e-23);
        assert!((from_photflam.factor / 1.527_036_046_539_967e-7 - 1.0).abs() < 1e-12, "{}", from_photflam.factor);
        assert!(from_photflam.method.contains("PHOTFLAM"), "{}", from_photflam.method);

        let mut sci = Array2::from_elem((2, 2), 10.0f32);
        apply_conversion(&conversion, &mut sci, None, &mut with_photfnu, None);
        assert!(sci.iter().all(|value| *value == (10.0 * 3.0e-7) as f32));
        for removed in ["PHOTFLAM", "PHOTFNU", "MAGZERO"] {
            assert!(with_photfnu.get(removed).is_none(), "{removed} still describes counts");
            assert!(history(&with_photfnu).iter().any(|line| line.starts_with(removed)), "{removed}: {:?}", history(&with_photfnu));
        }
        assert_eq!(with_photfnu.get_f64("PHOTPLAM"), Some(photplam));
        assert_eq!(card_text(&with_photfnu, CARD_BUNIT_IN).as_deref(), Some("ELECTRONS/S"));
        assert!(
            matches!(
                PhotCal::from_header(&with_photfnu, None).map(|cal| cal.convention),
                Some(FluxConvention::FluxDensityPerPixel { jy_per_unit, .. }) if jy_per_unit == 1.0
            ),
            "a Jy/pixel image is calibrated by its unit, not as counts again"
        );
    }

    #[test]
    fn nanomaggy_is_a_flux_density_per_pixel_of_3_63_micro_jansky() {
        let class_of = |bunit: &str| classify_unit(&make_header(&[("BUNIT", bunit)])).class;
        let physical = |bunit: &str| match class_of(bunit) {
            UnitClass::PerPixelPhysical { jy_per_unit } => Some(jy_per_unit),
            _ => None,
        };
        let close = |bunit: &str, expected: f64| {
            let got = physical(bunit).unwrap_or_else(|| panic!("{bunit} is not a flux density per pixel"));
            assert!((got / expected - 1.0).abs() < 1e-12, "{bunit}: {got:e}");
        };
        for unit in ["nanomaggy", "nanomaggies", "NMGY", " nanomaggy / pixel "] {
            close(unit, 3.630780547701014e-6);
        }
        close("10*nanomaggy", 3.630780547701014e-5);
        for unit in ["1/nanomaggy^2", "nanomaggy^2", "maggy", "picomaggy"] {
            assert_eq!(physical(unit), None, "{unit}");
            assert_eq!(class_of(unit), UnitClass::Unknown, "{unit}");
        }
        assert_eq!(
            class_of("nanomaggy/arcsec2"),
            UnitClass::SurfaceBrightness { jy_per_sr_per_unit: JY_PER_NANOMAGGY * ARCSEC_PER_RADIAN * ARCSEC_PER_RADIAN }
        );
        assert!((JY_PER_NANOMAGGY / 10f64.powf((8.90 - 22.5) / 2.5) - 1.0).abs() < 1e-14, "{JY_PER_NANOMAGGY:e}");

        let brick = make_header(&[("BUNIT", "nanomaggy"), ("MAGZERO", "22.5")]);
        assert_eq!(classify_unit(&brick).class, UnitClass::PerPixelPhysical { jy_per_unit: JY_PER_NANOMAGGY });
        let correction = area_correction(&brick, None, &geometry_2x()).unwrap();
        assert_eq!((correction.rule, correction.data_factor), (AreaRule::Scale, 4.0));
        let to_jy = unit_conversion(&brick, None, TargetUnit::JyPerPixel).unwrap();
        assert_eq!(to_jy.factor, JY_PER_NANOMAGGY);
        assert!(to_jy.method.ends_with("Jy per nanomaggy"), "{}", to_jy.method);
    }

    #[test]
    fn a_rescaled_header_drops_a_flux_density_bunit_that_no_longer_describes_its_pixels() {
        let mut brick = make_header(&[("BUNIT", "nanomaggy"), ("MAGZERO", "22.5")]);
        rescale_per_pixel_calibration(&mut brick, 4.0);
        assert!(brick.get("BUNIT").is_none(), "{:?}", brick.get("BUNIT"));
        assert!(
            history_text(&brick)
                .contains("BUNIT nanomaggy removed: the values are per input pixel after a pixel-area change by x 4.000000"),
            "{}",
            history_text(&brick)
        );
        let shifted = brick.get_f64("MAGZERO").unwrap();
        assert!((shifted - 20.994850021680094).abs() < 1e-9, "MAGZERO = {shifted}");
        let cal = PhotCal::from_header(&brick, None).unwrap();
        assert!(matches!(cal.convention, FluxConvention::GenericZeroPoint { .. }), "{:?}", cal.convention);
        assert!(relative(cal.jy_per_native_unit, 1.4523122190804056e-5) < 1e-12, "{:?}", cal.jy_per_native_unit);

        let mut jansky = make_header(&[("BUNIT", "Jy")]);
        rescale_per_pixel_calibration(&mut jansky, 4.0);
        assert!(jansky.get("BUNIT").is_none(), "{:?}", jansky.get("BUNIT"));
        assert_eq!(PhotCal::from_header(&jansky, None), None);

        let mut same_area = make_header(&[("BUNIT", "nanomaggy"), ("MAGZERO", "22.5")]);
        rescale_per_pixel_calibration(&mut same_area, 1.0);
        assert_eq!(card_text(&same_area, "BUNIT").as_deref(), Some("nanomaggy"));
        assert_eq!(same_area.get_f64("MAGZERO"), Some(22.5));
        assert!(history(&same_area).is_empty(), "{:?}", history(&same_area));

        for cards in [&[("BUNIT", "MJy/sr")][..], &[("BUNIT", "ADU"), ("MAGZERO", "25.0")][..]] {
            let mut kept = make_header(cards);
            rescale_per_pixel_calibration(&mut kept, 4.0);
            assert_eq!(card_text(&kept, "BUNIT").as_deref(), Some(cards[0].1));
            assert!(history(&kept).is_empty(), "{:?}", history(&kept));
        }
    }

    #[test]
    fn survey_zero_points_follow_the_pixel_area_of_a_rescaled_header() {
        let mut afw = make_header(&[("BUNIT", "DN"), ("FLUXMAG0", "63095734448.0194")]);
        rescale_per_pixel_calibration(&mut afw, 4.0);
        assert!(relative(afw.get_f64("FLUXMAG0"), 15773933612.00485) < 1e-12, "{:?}", afw.get("FLUXMAG0"));
        assert_eq!(card_text(&afw, "BUNIT").as_deref(), Some("DN"));
        let afw_factor = PhotCal::from_header(&afw, None).and_then(|cal| cal.jy_per_native_unit());
        assert!(relative(afw_factor, 2.3017597493486266e-7) < 1e-12, "{afw_factor:?}");

        let mut stack = make_header(&[
            ("HIERARCH FPA.ZP", "25."),
            ("EXPTIME", "600."),
            ("EXP_0001", "40."),
            ("EXP_0002", "40."),
            ("SCL_0008", "0."),
        ]);
        rescale_per_pixel_calibration(&mut stack, 4.0);
        let shifted = stack.get_f64("HIERARCH FPA.ZP").unwrap();
        assert!((shifted - 23.494850021680094).abs() < 1e-9, "HIERARCH FPA.ZP = {shifted}");
        assert_eq!(stack.get_f64("EXPTIME"), Some(600.0));
        let stack_factor = PhotCal::from_header(&stack, None).and_then(|cal| cal.jy_per_native_unit());
        assert!(relative(stack_factor, 2.4205203651340005e-9) < 1e-12, "{stack_factor:?}");

        let mut same_area = make_header(&[("FLUXMAG0", "63095734448.0194"), ("HIERARCH FPA.ZP", "25.")]);
        rescale_per_pixel_calibration(&mut same_area, 1.0);
        assert_eq!(same_area.get_f64("FLUXMAG0"), Some(63095734448.0194));
        assert_eq!(same_area.get_f64("HIERARCH FPA.ZP"), Some(25.0));
    }

    #[test]
    fn p0_surface_brightness_prefix_case_controls_the_physical_scale() {
        for (unit, jy_per_sr) in [("MJy/sr", 1e6), ("mJy/sr", 1e-3), ("Jy/sr", 1.0), ("uJy/sr", 1e-6), ("nJy/sr", 1e-9), (" mJy / SR ", 1e-3)] {
            let header = make_header(&[("BUNIT", unit), ("PIXAR_SR", "2e-12")]);
            let info = classify_unit(&header);
            assert_eq!(info.class, UnitClass::SurfaceBrightness { jy_per_sr_per_unit: jy_per_sr }, "{unit}");
            let converted = unit_conversion(&header, None, TargetUnit::JyPerPixel).unwrap();
            assert!((converted.factor / (jy_per_sr * 2e-12) - 1.0).abs() < 1e-12, "{unit}: {}", converted.factor);
            let target = unit_conversion(&header, None, TargetUnit::MjyPerSr).unwrap();
            assert!((target.factor / (jy_per_sr / 1e6) - 1.0).abs() < 1e-12, "{unit}: {}", target.factor);
        }
        for (unit, scale) in [("mJy sr-1", 1e-3), ("mJy.sr**-1", 1e-3), ("mJy/STERADIAN", 1e-3), ("µJy/sr", 1e-6), ("mJy/arcsec2", 1e-3 * ARCSEC_PER_RADIAN.powi(2))] {
            assert_eq!(classify_unit(&make_header(&[("BUNIT", unit)])).class,
                UnitClass::SurfaceBrightness { jy_per_sr_per_unit: scale }, "{unit}");
        }
        for unit in ["mJy/sr km/s", "mJy/sr x channels", "mJy/beam", "mJy/sr2"] {
            assert_eq!(classify_unit(&make_header(&[("BUNIT", unit)])).class, UnitClass::Unknown, "{unit}");
        }
    }

    #[test]
    fn grizli_ten_nanojansky_is_per_pixel_physical() {
        let physical = |bunit: &str| match classify_unit(&make_header(&[("BUNIT", bunit)])).class {
            UnitClass::PerPixelPhysical { jy_per_unit } => Some(jy_per_unit),
            _ => None,
        };
        let close = |bunit: &str, expected: f64| {
            let got = physical(bunit).unwrap_or_else(|| panic!("{bunit} is not a flux density per pixel"));
            assert!((got - expected).abs() <= expected * 1e-15, "{bunit}: {got}");
        };
        close("10.0*nanoJansky", 1e-8);
        close("10*nJy", 1e-8);
        close("Jy", 1.0);
        close("JY", 1.0);
        close(" Jy / pixel ", 1.0);
        close("Jy.pix**-1", 1.0);
        close("mJy", 1e-3);
        close("uJy", 1e-6);
        close("\u{b5}Jy", 1e-6);
        close("nJy", 1e-9);
        close("microJansky", 1e-6);
        for not_physical in ["MJy/sr", "MJY", "Jy/beam", "ADU", "mag", "-1*Jy", "x*Jy"] {
            assert_eq!(physical(not_physical), None, "{not_physical}");
        }
        assert_eq!(
            classify_unit(&make_header(&[("BUNIT", "MJy/sr")])).class,
            UnitClass::SurfaceBrightness { jy_per_sr_per_unit: 1e6 }
        );
        assert_eq!(
            classify_unit(&make_header(&[("BUNIT", "Jy/arcsec2")])).class,
            UnitClass::SurfaceBrightness { jy_per_sr_per_unit: ARCSEC_PER_RADIAN * ARCSEC_PER_RADIAN }
        );
        assert_eq!(
            classify_unit(&make_header(&[("BUNIT", "ELECTRONS/S"), ("PHOTFLAM", "1E-19"), ("MAGZERO", "25")])).class,
            UnitClass::PerPixelCalibrated { cards: vec!["PHOTFLAM".into(), "MAGZERO".into()] }
        );
        assert_eq!(classify_unit(&make_header(&[("BUNIT", "ELECTRONS/S")])).class, UnitClass::Unknown);
        assert_eq!(classify_unit(&HduHeader::empty()), UnitInfo { bunit: None, class: UnitClass::Unknown });

        let grizli = make_header(&[("BUNIT", "'10.0*nanoJansky'"), ("PIXAR_SR", "9.4E-15")]);
        let to_jy = unit_conversion(&grizli, None, TargetUnit::JyPerPixel).unwrap();
        assert!((to_jy.factor - 1e-8).abs() < 1e-22, "{}", to_jy.factor);
        assert_eq!(to_jy.bunit_in.as_deref(), Some("10.0*nanoJansky"));
        let to_mjy_sr = unit_conversion(&grizli, None, TargetUnit::MjyPerSr).unwrap();
        assert!((to_mjy_sr.factor - 1e-8 / (1e6 * 9.4e-15)).abs() < 1e-12 * to_mjy_sr.factor, "{}", to_mjy_sr.factor);
    }

    #[test]
    fn fits_megajansky_per_pixel_is_physical_while_all_caps_stays_refused() {
        let physical = |bunit: &str| match classify_unit(&make_header(&[("BUNIT", bunit)])).class {
            UnitClass::PerPixelPhysical { jy_per_unit } => Some(jy_per_unit),
            _ => None,
        };
        for (unit, expected) in [("MJy", 1e6), ("MJy/pixel", 1e6), (" MJy / pix ", 1e6), ("MJy.pix**-1", 1e6), ("10*MJy", 1e7), ("10*MJy/pixel", 1e7), ("megaJansky", 1e6), ("MegaJy", 1e6), ("MEGAJY", 1e6), ("MEGAJANSKY", 1e6), ("mJy", 1e-3)] {
            let got = physical(unit).unwrap_or_else(|| panic!("{unit} is not a flux density per pixel"));
            assert!((got - expected).abs() <= expected * 1e-15, "{unit}: {got}");
        }
        for refused in ["MJY", "Mjy", "MJansky", "MJY/pixel", "MJY/arcsec2", "Mjy/arcsec2", "kJy", "kiloJansky"] {
            assert_eq!(classify_unit(&make_header(&[("BUNIT", refused)])).class, UnitClass::Unknown, "{refused}");
        }
        for (unit, jy_per_sr) in [
            ("MJy/sr", 1e6),
            ("megaJansky/sr", 1e6),
            ("MEGAJY/SR", 1e6),
            ("MJy/arcsec2", 1e6 * ARCSEC_PER_RADIAN.powi(2)),
            ("MJy/arcsec**2", 1e6 * ARCSEC_PER_RADIAN.powi(2)),
            ("MJy/arcsec^2", 1e6 * ARCSEC_PER_RADIAN.powi(2)),
            ("MJy.arcsec-2", 1e6 * ARCSEC_PER_RADIAN.powi(2)),
            ("MJy.arcsec**-2", 1e6 * ARCSEC_PER_RADIAN.powi(2)),
            ("megaJansky/arcsec2", 1e6 * ARCSEC_PER_RADIAN.powi(2)),
        ] {
            assert_eq!(classify_unit(&make_header(&[("BUNIT", unit)])).class, UnitClass::SurfaceBrightness { jy_per_sr_per_unit: jy_per_sr }, "{unit}");
        }
        let header = make_header(&[("BUNIT", "MJy"), ("PIXAR_SR", "2.1E-13")]);
        let to_jy = unit_conversion(&header, None, TargetUnit::JyPerPixel).unwrap();
        assert_eq!(to_jy.factor, 1e6);
        assert_eq!(to_jy.bunit_in.as_deref(), Some("MJy"));
        assert!(to_jy.method.ends_with("Jy per MJy"), "{}", to_jy.method);
        let to_mjy_sr = unit_conversion(&header, None, TargetUnit::MjyPerSr).unwrap();
        assert!((to_mjy_sr.factor * 2.1e-13 - 1.0).abs() < 1e-12, "{}", to_mjy_sr.factor);
        assert_eq!(area_correction(&header, None, &geometry_2x()).unwrap().data_factor, 4.0);
    }

    #[test]
    fn a_megajansky_bunit_wins_over_count_calibration_cards_as_jansky_does() {
        let arcsec2 = ARCSEC_PER_RADIAN * ARCSEC_PER_RADIAN;
        for card in COUNT_CALIBRATION_CARDS.into_iter().chain(ROMAN_CONVERSION_MJY_KEYS).chain(GENERIC_ZERO_POINT_KEYS).chain(CALIBRATION_ZERO_POINT_CARDS) {
            let header = |bunit: &str| make_header(&[("BUNIT", bunit), (card, "0.5")]);
            let default_rule = |bunit: &str| {
                let correction = area_correction(&header(bunit), None, &geometry_2x()).unwrap();
                (correction.rule, correction.data_factor)
            };
            for (unit, jy_per_unit) in [("Jy", 1.0), ("MJy", 1e6), ("MJy/pixel", 1e6), ("10*MJy", 1e7), ("megaJansky", 1e6), ("MegaJy", 1e6)] {
                assert_eq!(classify_unit(&header(unit)).class, UnitClass::PerPixelPhysical { jy_per_unit }, "{unit} with {card}");
                assert_eq!(default_rule(unit), (AreaRule::Scale, 4.0), "{unit} with {card}");
            }
            for (unit, jy_per_sr_per_unit) in [("MJy/sr", 1e6), ("Jy/arcsec2", arcsec2), ("MJy/arcsec2", 1e6 * arcsec2)] {
                assert_eq!(classify_unit(&header(unit)).class, UnitClass::SurfaceBrightness { jy_per_sr_per_unit }, "{unit} with {card}");
                assert_eq!(default_rule(unit), (AreaRule::Keep, 1.0), "{unit} with {card}");
            }
            for unit in ["ELECTRONS/S", "MJY", "Mjy", "MJansky", "MJY/arcsec2"] {
                assert_eq!(classify_unit(&header(unit)).class, UnitClass::PerPixelCalibrated { cards: vec![card.to_string()] }, "{unit} with {card}");
                assert_eq!(default_rule(unit), (AreaRule::Keep, 1.0), "{unit} with {card}");
            }
        }

        for unit in ["Jy", "MJy"] {
            let mut header = make_header(&[("BUNIT", unit), ("PHOTFNU", "3.0E-07"), ("MAGZERO", "25.0"), ("PIXAR_SR", "9.4E-15")]);
            let correction = area_correction(&header, None, &geometry_2x()).unwrap();
            assert_eq!(correction, AreaCorrection { rule: AreaRule::Scale, requested: false, data_factor: 4.0, area_ratio: 4.0 }, "{unit}");
            let mut sci = Array2::from_elem((2, 2), 1.5f32);
            let mut err = Array2::from_elem((2, 2), 0.25f32);
            apply_area_correction(&correction, &mut sci, Some(&mut err), &mut header);
            assert!(sci.iter().all(|value| *value == 6.0) && err.iter().all(|value| *value == 1.0), "{unit}");
            assert_eq!((header.get_f64("PHOTFNU"), header.get_f64("MAGZERO")), (Some(3.0e-7), Some(25.0)), "{unit}");
            assert!(relative(header.get_f64("PIXAR_SR"), 3.76e-14) < 1e-12, "{unit}");
            assert!(history_text(&header).contains("Area rule scale"), "{unit}: {}", history_text(&header));
        }

        let converted = unit_conversion(&make_header(&[("BUNIT", "MJy"), ("PHOTFNU", "3.0E-07")]), None, TargetUnit::JyPerPixel).unwrap();
        assert_eq!((converted.factor, converted.method.as_str()), (1e6, "1E6 Jy per MJy"));
        let refused = unit_conversion(&make_header(&[("BUNIT", "MJY"), ("PHOTFNU", "3.0E-07")]), None, TargetUnit::JyPerPixel).unwrap_err();
        assert!(matches!(&refused, UnitError::NoConversion { reason, .. } if reason.contains("not per second")), "{refused}");
    }

    #[test]
    fn per_pixel_jy_is_scaled_by_area_on_reprojection() {
        let mut header = make_header(&[("BUNIT", "Jy"), ("PIXAR_SR", "9.4E-15"), ("PIXAR_A2", "4.0E-4"), ("PHOTPLAM", "8045.53"), ("SATURATE", "1.0")]);
        let correction = area_correction(&header, None, &geometry_2x()).unwrap();
        assert_eq!(correction, AreaCorrection { rule: AreaRule::Scale, requested: false, data_factor: 4.0, area_ratio: 4.0 });
        let mut sci = Array2::from_elem((2, 2), 1.5f32);
        let mut err = Array2::from_elem((2, 2), 0.25f32);
        apply_area_correction(&correction, &mut sci, Some(&mut err), &mut header);
        assert!(sci.iter().all(|value| *value == 6.0));
        assert!(err.iter().all(|value| *value == 1.0));
        assert!(relative(header.get_f64("PIXAR_SR"), 3.76e-14) < 1e-12);
        assert!(relative(header.get_f64("PIXAR_A2"), 1.6e-3) < 1e-12);
        assert_eq!(header.get_f64("PHOTPLAM"), Some(8045.53));
        assert!(header.get("SATURATE").is_none());
        assert!(history_text(&header).contains("Area rule scale"), "{}", history_text(&header));

        let grizli = make_header(&[("BUNIT", "10.0*nanoJansky")]);
        assert_eq!(area_correction(&grizli, None, &geometry_2x()).unwrap().data_factor, 4.0);
    }

    #[test]
    fn mjy_sr_is_not_scaled_on_reprojection() {
        let mut header = make_header(&[("BUNIT", "MJy/sr"), ("PIXAR_SR", "9.4E-15"), ("PIXAR_A2", "4.0E-4"), ("PHOTMJSR", "1.5")]);
        let correction = area_correction(&header, None, &geometry_2x()).unwrap();
        assert_eq!(correction, AreaCorrection { rule: AreaRule::Keep, requested: false, data_factor: 1.0, area_ratio: 4.0 });
        let mut sci = Array2::from_elem((2, 2), 3.25f32);
        apply_area_correction(&correction, &mut sci, None, &mut header);
        assert!(sci.iter().all(|value| *value == 3.25));
        assert!(relative(header.get_f64("PIXAR_SR"), 3.76e-14) < 1e-12);
        assert!(relative(header.get_f64("PIXAR_A2"), 1.6e-3) < 1e-12);
        assert_eq!(header.get_f64("PHOTMJSR"), Some(1.5));

        let mut counts = make_header(&[("BUNIT", "ELECTRONS/S"), ("PHOTFLAM", "1.0E-19"), ("PHOTPLAM", "8000"), ("MAGZERO", "25.0")]);
        let kept = area_correction(&counts, None, &geometry_2x()).unwrap();
        assert_eq!((kept.rule, kept.data_factor), (AreaRule::Keep, 1.0));
        let mut sci = Array2::from_elem((2, 2), 7.0f32);
        apply_area_correction(&kept, &mut sci, None, &mut counts);
        assert!(sci.iter().all(|value| *value == 7.0));
        assert!(relative(counts.get_f64("PHOTFLAM"), 4.0e-19) < 1e-12);
        assert!(relative(counts.get_f64("MAGZERO"), 25.0 - 2.5 * 4f64.log10()) < 1e-12);
        assert_eq!(counts.get_f64("PHOTPLAM"), Some(8000.0));
    }

    #[test]
    fn unknown_unit_refuses_without_explicit_rule() {
        let adu = make_header(&[("BUNIT", "ADU")]);
        let refused = area_correction(&adu, None, &geometry_2x()).unwrap_err();
        assert_eq!(refused, UnitError::UnknownUnit { bunit: Some("ADU".into()) });
        assert!(refused.to_string().contains("area_rule keep, scale or lightstack"), "{refused}");
        assert_eq!(
            area_correction(&HduHeader::empty(), None, &geometry_2x()).unwrap_err(),
            UnitError::UnknownUnit { bunit: None }
        );
        for (rule, factor) in [(AreaRule::Keep, 1.0), (AreaRule::Scale, 4.0), (AreaRule::Lightstack, 4.0)] {
            let correction = area_correction(&adu, Some(rule), &geometry_2x()).unwrap();
            assert_eq!((correction.rule, correction.requested, correction.data_factor), (rule, true, factor));
        }
        let mjy_sr = make_header(&[("BUNIT", "MJy/sr"), ("PIXAR_SR", "9.4E-15")]);
        assert_eq!(area_correction(&mjy_sr, Some(AreaRule::Lightstack), &geometry_2x()).unwrap().data_factor, 4.0);
        let same_scale = PixelGeometry { area_ratio: 1.000_000_8, scale_in_arcsec: 0.04, scale_out_arcsec: 0.040_000_01 };
        assert_eq!(area_correction(&adu, Some(AreaRule::Lightstack), &same_scale).unwrap().data_factor, 1.0);
        let invalid = PixelGeometry { area_ratio: f64::NAN, ..geometry_2x() };
        assert!(matches!(area_correction(&mjy_sr, None, &invalid), Err(UnitError::InvalidGeometry { .. })));
    }

    #[test]
    fn lightstack_rule_uses_the_squared_first_axis_scale_ratio() {
        let (_, input) = header_with_wcs([[-0.02 / 3600.0, 0.0], [0.0, 0.03 / 3600.0]], &[]);
        let (_, output) = header_with_wcs([[-0.04 / 3600.0, 0.0], [0.0, 0.04 / 3600.0]], &[]);
        let geometry = PixelGeometry::between(&input, &output);
        assert!((geometry.scale_in_arcsec - 0.02).abs() < 1e-12, "{geometry:?}");
        assert!((geometry.scale_out_arcsec - 0.04).abs() < 1e-12, "{geometry:?}");
        assert!((geometry.area_ratio - 0.04 * 0.04 / (0.02 * 0.03)).abs() < 1e-9, "{geometry:?}");
        let adu = make_header(&[("BUNIT", "ADU")]);
        let lightstack = area_correction(&adu, Some(AreaRule::Lightstack), &geometry).unwrap();
        assert!((lightstack.data_factor - 4.0).abs() < 1e-9, "{lightstack:?}");
        let scaled = area_correction(&adu, Some(AreaRule::Scale), &geometry).unwrap();
        assert!((scaled.data_factor - geometry.area_ratio).abs() < 1e-12, "{scaled:?}");
    }

    #[test]
    fn a_jy_image_converts_back_to_mjy_sr_through_its_pixel_area() {
        let mut header = make_header(&[("BUNIT", "MJy/sr"), ("PIXAR_SR", "2.1E-13")]);
        let mut sci = Array2::from_elem((1, 2), 12.5f32);
        let to_jy = unit_conversion(&header, None, TargetUnit::JyPerPixel).unwrap();
        apply_conversion(&to_jy, &mut sci, None, &mut header, None);
        let back = unit_conversion(&header, None, TargetUnit::MjyPerSr).unwrap();
        assert!((back.factor * 1e6 * 2.1e-13 - 1.0).abs() < 1e-12, "{}", back.factor);
        assert_eq!(back.bunit_in.as_deref(), Some(BUNIT_JY));
        apply_conversion(&back, &mut sci, None, &mut header, None);
        assert!(sci.iter().all(|value| (value - 12.5).abs() < 1e-5), "{sci:?}");
        assert_eq!(card_text(&header, "BUNIT").as_deref(), Some(BUNIT_MJY_SR));
        assert_eq!(card_text(&header, CARD_BUNIT_IN).as_deref(), Some("MJy/sr"));
        assert!(relative(header.get_f64(CARD_FLUXCONV), 1.0) < 1e-12);
        assert!(matches!(PhotCal::from_header(&header, None).map(|cal| cal.convention), Some(crate::core::metadata::photcal::FluxConvention::JwstMjySr { .. })));
    }

    #[test]
    fn count_conversion_needs_a_rate_and_photometric_cards() {
        let refusal = |cards: &[(&str, &str)]| unit_conversion(&make_header(cards), None, TargetUnit::JyPerPixel).unwrap_err();
        let not_a_rate = refusal(&[("BUNIT", "ELECTRONS"), ("PHOTFNU", "3E-7")]);
        assert!(matches!(&not_a_rate, UnitError::NoConversion { reason, .. } if reason.contains("rate")), "{not_a_rate}");
        let zero_point_only = refusal(&[("BUNIT", "ELECTRONS/S"), ("MAGZERO", "25")]);
        assert!(matches!(&zero_point_only, UnitError::NoConversion { reason, .. } if reason.contains("PHOTFNU")), "{zero_point_only}");
        let unknown = refusal(&[("BUNIT", "ADU")]);
        assert!(matches!(&unknown, UnitError::NoConversion { bunit: Some(b), .. } if b == "ADU"), "{unknown}");
        let no_area = refusal(&[("BUNIT", "MJy/sr")]);
        assert!(matches!(&no_area, UnitError::NoConversion { reason, .. } if reason.contains("pixel area")), "{no_area}");
        assert!(no_area.to_string().contains("Jy/pixel"), "{no_area}");
    }

    #[test]
    fn a_stale_pixar_sr_gives_way_to_the_wcs_pixel_area() {
        let deg = 0.063 / 3600.0;
        let stale = format!("{:.6e}", (0.031 / ARCSEC_PER_RADIAN).powi(2));
        let (header, wcs) = header_with_wcs([[-deg, 0.0], [0.0, deg]], &[("BUNIT", "MJy/sr"), ("PIXAR_SR", &stale)]);
        let conversion = unit_conversion(&header, Some(&wcs), TargetUnit::JyPerPixel).unwrap();
        let wcs_sr = (0.063 / ARCSEC_PER_RADIAN).powi(2);
        assert!((conversion.factor / (1e6 * wcs_sr) - 1.0).abs() < 1e-6, "{}", conversion.factor);
        assert!(conversion.warnings.iter().any(|warning| warning.starts_with("PIXAR_SR")), "{:?}", conversion.warnings);
        assert!(conversion.method.contains("WCS"), "{}", conversion.method);
    }
}
