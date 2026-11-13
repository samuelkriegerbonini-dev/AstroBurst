use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

use crate::core::metadata::photcal::{AB_MAG_ZERO_POINT, MAG_ERR_PER_RELATIVE_FLUX_ERR};

pub const SPEED_OF_LIGHT_M_PER_S: f64 = 299_792_458.0;
pub const FLAMBDA_CGS_PER_JY_UM2: f64 = SPEED_OF_LIGHT_M_PER_S * 1e-21;
pub const FLAMBDA_SI_PER_JY_UM2: f64 = SPEED_OF_LIGHT_M_PER_S * 1e-20;

const MICRO_SIGN: char = '\u{b5}';
const GREEK_MU: char = '\u{3bc}';
const A_RING_UPPER: char = '\u{c5}';
const A_RING_LOWER: char = '\u{e5}';
const MIDDLE_DOT: char = '\u{b7}';

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpectralFluxUnit {
    FluxDensityNu { jy_per_unit: f64 },
    FluxDensityLambda { cgs_per_unit: f64 },
    SurfaceBrightness,
    Rate,
    Unknown,
}

#[derive(Clone, Copy)]
enum Base {
    Jansky,
    Erg,
    Second,
    Centimetre,
    Metre,
    Angstrom,
    Micrometre,
    Nanometre,
    Hertz,
    Watt,
    Count,
    Steradian,
    Arcsecond,
    Pixel,
    Beam,
}

const BASE_COUNT: usize = Base::Beam as usize + 1;
const AREAL_BASES: [Base; 4] = [Base::Steradian, Base::Arcsecond, Base::Pixel, Base::Beam];

type BasePowers = [i64; BASE_COUNT];
type Signature = &'static [(Base, i64)];

const JY_PER_UNIT_BY_SIGNATURE: [(Signature, f64); 3] = [
    (&[(Base::Jansky, 1)], 1.0),
    (&[(Base::Erg, 1), (Base::Second, -1), (Base::Centimetre, -2), (Base::Hertz, -1)], 1e23),
    (&[(Base::Watt, 1), (Base::Metre, -2), (Base::Hertz, -1)], 1e26),
];
const CGS_PER_UNIT_BY_SIGNATURE: [(Signature, f64); 6] = [
    (&[(Base::Erg, 1), (Base::Second, -1), (Base::Centimetre, -2), (Base::Angstrom, -1)], 1.0),
    (&[(Base::Erg, 1), (Base::Second, -1), (Base::Centimetre, -2), (Base::Micrometre, -1)], 1e-4),
    (&[(Base::Erg, 1), (Base::Second, -1), (Base::Centimetre, -2), (Base::Nanometre, -1)], 0.1),
    (&[(Base::Watt, 1), (Base::Metre, -2), (Base::Micrometre, -1)], 0.1),
    (&[(Base::Watt, 1), (Base::Metre, -2), (Base::Nanometre, -1)], 100.0),
    (&[(Base::Watt, 1), (Base::Metre, -2), (Base::Angstrom, -1)], 1000.0),
];
const RATE_SIGNATURE: Signature = &[(Base::Count, 1), (Base::Second, -1)];

#[derive(Clone, Copy, PartialEq)]
enum Group {
    Closed,
    Multiplying,
    Dividing,
}

static RE_POWER_OF_TEN: OnceLock<Regex> = OnceLock::new();
static RE_DECIMAL: OnceLock<Regex> = OnceLock::new();
static RE_EXPONENT: OnceLock<Regex> = OnceLock::new();

fn re_power_of_ten() -> &'static Regex {
    RE_POWER_OF_TEN.get_or_init(|| Regex::new(r"^10 *(?:\*\*|\^) *\(?([+-]?[0-9]+)\)?").unwrap())
}

fn re_decimal() -> &'static Regex {
    RE_DECIMAL.get_or_init(|| Regex::new(r"^[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?").unwrap())
}

fn re_exponent() -> &'static Regex {
    RE_EXPONENT.get_or_init(|| Regex::new(r"(?:\*\*|\^) *\(?([+-]?[0-9]+)\)?").unwrap())
}

fn is_unit_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c, MICRO_SIGN | GREEK_MU | A_RING_UPPER | A_RING_LOWER)
}

fn split_leading_scale(text: &str) -> (f64, &str) {
    let number = re_power_of_ten()
        .captures(text)
        .map(|power| (format!("1e{}", &power[1]), power[0].len()))
        .or_else(|| re_decimal().find(text).map(|decimal| (decimal.as_str().to_string(), decimal.end())));
    let Some((digits, end)) = number else {
        return (1.0, text);
    };
    let after_number = text[end..].trim_start_matches(' ');
    let unit = after_number.strip_prefix('*').unwrap_or(after_number).trim_start_matches(' ');
    match (digits.parse::<f64>(), unit.chars().next()) {
        (Ok(scale), Some(next)) if next == '(' || is_unit_letter(next) => (scale, unit),
        _ => (1.0, text),
    }
}

fn split_exponent(after_name: &str) -> (Option<&str>, &str) {
    let candidate = after_name.strip_prefix('^').unwrap_or(after_name);
    let sign_len = usize::from(candidate.starts_with(['+', '-']));
    let digit_len = candidate[sign_len..].bytes().take_while(u8::is_ascii_digit).count();
    if digit_len == 0 {
        return (None, after_name);
    }
    let (exponent, rest) = candidate.split_at(sign_len + digit_len);
    (Some(exponent), rest)
}

fn jy_per_jansky_spelling(name: &str) -> Option<f64> {
    match name {
        "Jy" | "JY" | "jy" | "Jansky" | "jansky" => Some(1.0),
        "mJy" => Some(1e-3),
        "uJy" | "\u{b5}Jy" | "\u{3bc}Jy" => Some(1e-6),
        "nJy" => Some(1e-9),
        "MJy" => Some(1e6),
        "kJy" => Some(1e3),
        _ => None,
    }
}

fn base_of_lowercase_name(name: &str) -> Option<Base> {
    match name {
        "erg" | "ergs" => Some(Base::Erg),
        "s" | "sec" | "second" => Some(Base::Second),
        "cm" => Some(Base::Centimetre),
        "m" => Some(Base::Metre),
        "a" | "aa" | "ang" | "angstrom" | "angstroms" | "\u{e5}" => Some(Base::Angstrom),
        "um" | "micron" | "microns" | "micrometer" | "\u{b5}m" | "\u{3bc}m" => Some(Base::Micrometre),
        "nm" => Some(Base::Nanometre),
        "hz" => Some(Base::Hertz),
        "w" | "watt" => Some(Base::Watt),
        "sr" | "steradian" => Some(Base::Steradian),
        "arcsec" | "arcsecond" => Some(Base::Arcsecond),
        "pix" | "pixel" | "pixels" => Some(Base::Pixel),
        "beam" => Some(Base::Beam),
        "dn" | "adu" | "count" | "counts" | "ct" | "cts" | "electron" | "electrons" => Some(Base::Count),
        _ => None,
    }
}

fn scan_factors(unit: &str) -> Option<(f64, BasePowers)> {
    let mut scale = 1.0;
    let mut powers = [0; BASE_COUNT];
    let mut divide_next = false;
    let mut group = Group::Closed;
    let mut rest = unit;
    while let Some(c) = rest.chars().next() {
        let after_char = &rest[c.len_utf8()..];
        match c {
            ' ' | '\t' | '.' | '*' | MIDDLE_DOT => rest = after_char,
            '/' if divide_next => return None,
            '/' => {
                divide_next = true;
                rest = after_char;
            }
            '(' if group != Group::Closed => return None,
            '(' => {
                group = if divide_next { Group::Dividing } else { Group::Multiplying };
                divide_next = false;
                rest = after_char;
            }
            ')' if group == Group::Closed || divide_next => return None,
            ')' => {
                group = Group::Closed;
                rest = after_char;
            }
            _ => {
                let name_len = rest.find(|c| !is_unit_letter(c)).unwrap_or(rest.len());
                if name_len == 0 {
                    return None;
                }
                let (name, after_name) = rest.split_at(name_len);
                let (exponent, after_factor) = split_exponent(after_name);
                rest = after_factor;
                if name == "x" && exponent.is_none() {
                    continue;
                }
                let mut power = i64::from(exponent.map_or(Ok(1), str::parse::<i32>).ok()?);
                if group == Group::Dividing {
                    power = -power;
                }
                if divide_next {
                    power = -power;
                    divide_next = false;
                }
                let (base, jy_per_name) = match jy_per_jansky_spelling(name) {
                    Some(jy) => (Base::Jansky, jy),
                    None => (base_of_lowercase_name(&name.to_lowercase())?, 1.0),
                };
                scale *= jy_per_name.powf(power as f64);
                powers[base as usize] += power;
            }
        }
    }
    (group == Group::Closed && !divide_next).then_some((scale, powers))
}

fn has_signature(powers: &BasePowers, signature: Signature) -> bool {
    let mut expected = [0; BASE_COUNT];
    for &(base, power) in signature {
        expected[base as usize] = power;
    }
    *powers == expected
}

fn factor_of(powers: &BasePowers, table: &[(Signature, f64)]) -> Option<f64> {
    table.iter().find(|(signature, _)| has_signature(powers, signature)).map(|&(_, factor)| factor)
}

pub fn parse_spectral_flux_unit(text: &str) -> SpectralFluxUnit {
    let (leading_scale, unit) = split_leading_scale(text.trim());
    let unit = re_exponent().replace_all(unit, "^${1}");
    let Some((name_scale, powers)) = scan_factors(&unit) else {
        return SpectralFluxUnit::Unknown;
    };
    let scale = leading_scale * name_scale;
    let areal = AREAL_BASES.iter().any(|&base| powers[base as usize] != 0);
    let mut rest = powers;
    for base in AREAL_BASES {
        rest[base as usize] = 0;
    }
    let density = factor_of(&rest, &JY_PER_UNIT_BY_SIGNATURE)
        .map(|factor| SpectralFluxUnit::FluxDensityNu { jy_per_unit: scale * factor })
        .or_else(|| {
            factor_of(&rest, &CGS_PER_UNIT_BY_SIGNATURE)
                .map(|factor| SpectralFluxUnit::FluxDensityLambda { cgs_per_unit: scale * factor })
        });
    match density {
        Some(_) if areal => SpectralFluxUnit::SurfaceBrightness,
        Some(flux_density) => flux_density,
        None if !areal && has_signature(&rest, RATE_SIGNATURE) => SpectralFluxUnit::Rate,
        None => SpectralFluxUnit::Unknown,
    }
}

fn is_positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

pub fn fnu_jy_to_flambda_cgs(fnu_jy: f64, lambda_um: f64) -> f64 {
    if !is_positive_finite(lambda_um) {
        return f64::NAN;
    }
    fnu_jy * FLAMBDA_CGS_PER_JY_UM2 / (lambda_um * lambda_um)
}

pub fn flambda_cgs_to_fnu_jy(flambda: f64, lambda_um: f64) -> f64 {
    if !is_positive_finite(lambda_um) {
        return f64::NAN;
    }
    flambda * (lambda_um * lambda_um) / FLAMBDA_CGS_PER_JY_UM2
}

pub fn fnu_jy_to_flambda_si_um(fnu_jy: f64, lambda_um: f64) -> f64 {
    if !is_positive_finite(lambda_um) {
        return f64::NAN;
    }
    fnu_jy * FLAMBDA_SI_PER_JY_UM2 / (lambda_um * lambda_um)
}

pub fn fnu_jy_to_ab(fnu_jy: f64) -> Option<f64> {
    is_positive_finite(fnu_jy).then(|| -2.5 * fnu_jy.log10() + AB_MAG_ZERO_POINT)
}

pub fn ab_to_fnu_jy(mag: f64) -> f64 {
    10f64.powf((AB_MAG_ZERO_POINT - mag) / 2.5)
}

pub fn ab_mag_err(fnu_jy: f64, fnu_err_jy: f64) -> Option<f64> {
    (is_positive_finite(fnu_jy) && fnu_err_jy.is_finite() && fnu_err_jy >= 0.0)
        .then(|| MAG_ERR_PER_RELATIVE_FLUX_ERR * fnu_err_jy / fnu_jy)
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    use super::*;

    const GOLDEN_SHA256: &str = "03fb71210bd84141785faf34c2195087b6f3693fdd01705790d93673e0d225f2";
    const RELATIVE_TOLERANCE: f64 = 1e-12;
    const AB_MAG_TOLERANCE: f64 = 1e-12;
    const LINE_FLUX_RELATIVE_TOLERANCE: f64 = 1e-14;

    fn golden_text() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("flux_units")
            .join("astropy_flux_units.json");
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("golden {} unreadable: {}", path.display(), e))
    }

    fn golden() -> Value {
        serde_json::from_str(&golden_text()).unwrap_or_else(|e| panic!("golden is not JSON: {e}"))
    }

    fn rows<'a>(golden: &'a Value, list: &str) -> &'a Vec<Value> {
        golden[list].as_array().unwrap_or_else(|| panic!("golden has no list {list}"))
    }

    fn number(row: &Value, key: &str) -> f64 {
        row[key].as_f64().unwrap_or_else(|| panic!("row {row} has no number {key}"))
    }

    fn rel(a: f64, b: f64) -> f64 {
        (a - b).abs() / b.abs()
    }

    fn kind_and_factor(unit: SpectralFluxUnit) -> (&'static str, Option<f64>) {
        match unit {
            SpectralFluxUnit::FluxDensityNu { jy_per_unit } => ("flux_density_nu", Some(jy_per_unit)),
            SpectralFluxUnit::FluxDensityLambda { cgs_per_unit } => ("flux_density_lambda", Some(cgs_per_unit)),
            SpectralFluxUnit::SurfaceBrightness => ("surface_brightness", None),
            SpectralFluxUnit::Rate => ("rate", None),
            SpectralFluxUnit::Unknown => ("unknown", None),
        }
    }

    fn spellings_parsed_differently(golden: &Value, list: &str) -> Vec<String> {
        rows(golden, list)
            .iter()
            .filter_map(|row| {
                let text = row["text"].as_str().unwrap_or_else(|| panic!("row {row} has no text"));
                let expected_kind = row["kind"].as_str().unwrap_or_else(|| panic!("row {row} has no kind"));
                let expected_factor = match expected_kind {
                    "flux_density_nu" => Some(number(row, "jy_per_unit")),
                    "flux_density_lambda" => Some(number(row, "cgs_per_unit")),
                    _ => None,
                };
                let parsed = parse_spectral_flux_unit(text);
                let (kind, factor) = kind_and_factor(parsed);
                let same_factor = match (factor, expected_factor) {
                    (Some(got), Some(wanted)) => rel(got, wanted) <= RELATIVE_TOLERANCE,
                    (None, None) => true,
                    _ => false,
                };
                (kind != expected_kind || !same_factor)
                    .then(|| format!("{text:?}: parsed {parsed:?}, golden {expected_kind} {expected_factor:?}"))
            })
            .collect()
    }

    #[test]
    fn the_golden_is_the_table_the_generator_writes() {
        let text = golden_text().replace("\r\n", "\n");
        assert_eq!(format!("{:x}", Sha256::digest(text.as_bytes())), GOLDEN_SHA256);
        let golden = golden();
        assert_eq!(golden["kind"], "astropy_flux_unit_goldens");
        assert_eq!(golden["versions"]["astropy"], "8.0.2");
        assert_eq!(rows(&golden, "conversions").len(), 63);
        assert_eq!(rows(&golden, "astropy_spellings").len(), 22);
        assert_eq!(rows(&golden, "defined_spellings").len(), 62);
    }

    #[test]
    fn fnu_to_flambda_and_back_agree_with_astropy_spectral_density() {
        let golden = golden();
        for row in rows(&golden, "conversions") {
            let (lambda_um, fnu_jy) = (number(row, "lambda_um"), number(row, "fnu_jy"));
            let (flambda_cgs, flambda_si_um) = (number(row, "flambda_cgs"), number(row, "flambda_si_um"));
            assert!(rel(fnu_jy_to_flambda_cgs(fnu_jy, lambda_um), flambda_cgs) <= RELATIVE_TOLERANCE, "cgs of {row}");
            assert!(rel(fnu_jy_to_flambda_si_um(fnu_jy, lambda_um), flambda_si_um) <= RELATIVE_TOLERANCE, "SI of {row}");
            assert!(rel(flambda_cgs_to_fnu_jy(flambda_cgs, lambda_um), fnu_jy) <= RELATIVE_TOLERANCE, "Jy of {row}");
        }
        assert!(rel(fnu_jy_to_flambda_cgs(1e-9, 0.1216), 2.0274658744156864e-20) <= RELATIVE_TOLERANCE);
        assert!(rel(fnu_jy_to_flambda_si_um(1e-9, 0.1216), 2.027465874415686e-19) <= RELATIVE_TOLERANCE);
        assert!(rel(fnu_jy_to_flambda_cgs(1e-3, 2.2), 6.194059049586778e-17) <= RELATIVE_TOLERANCE);
    }

    #[test]
    fn jy_to_ab_and_back_agree_with_astropy_abmag() {
        let golden = golden();
        for row in rows(&golden, "conversions") {
            let (fnu_jy, ab_mag) = (number(row, "fnu_jy"), number(row, "ab_mag"));
            let mag = fnu_jy_to_ab(fnu_jy).unwrap_or_else(|| panic!("no magnitude for {row}"));
            assert!((mag - ab_mag).abs() <= AB_MAG_TOLERANCE, "magnitude of {row}: {mag}");
            assert!(rel(ab_to_fnu_jy(ab_mag), fnu_jy) <= RELATIVE_TOLERANCE, "Jy of {row}");
        }
        assert!((fnu_jy_to_ab(1e-3).unwrap() - 16.4).abs() <= AB_MAG_TOLERANCE);
        assert!((fnu_jy_to_ab(3.631e-6).unwrap() - 22.499934377717764).abs() <= AB_MAG_TOLERANCE);
        assert!(fnu_jy_to_ab(3630.7805477010028).unwrap().abs() <= AB_MAG_TOLERANCE);
    }

    #[test]
    fn spellings_astropy_reads_parse_to_the_same_kind_and_factor() {
        let failures = spellings_parsed_differently(&golden(), "astropy_spellings");
        assert!(failures.is_empty(), "{} spellings differ:\n{}", failures.len(), failures.join("\n"));
    }

    #[test]
    fn spellings_defined_by_this_project_parse_as_listed() {
        let failures = spellings_parsed_differently(&golden(), "defined_spellings");
        assert!(failures.is_empty(), "{} spellings differ:\n{}", failures.len(), failures.join("\n"));
    }

    #[test]
    fn magnitudes_and_conversions_refuse_what_has_no_answer() {
        for fnu_jy in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(fnu_jy_to_ab(fnu_jy), None, "magnitude of {fnu_jy} Jy");
        }
        for lambda_um in [0.0, -1.0, f64::NAN] {
            assert!(fnu_jy_to_flambda_cgs(1.0, lambda_um).is_nan(), "cgs at {lambda_um} um");
            assert!(flambda_cgs_to_fnu_jy(1.0, lambda_um).is_nan(), "Jy at {lambda_um} um");
            assert!(fnu_jy_to_flambda_si_um(1.0, lambda_um).is_nan(), "SI at {lambda_um} um");
        }
        assert_eq!(ab_mag_err(1.0, 0.01), Some(MAG_ERR_PER_RELATIVE_FLUX_ERR * 0.01));
        assert!((ab_mag_err(1.0, 0.01).unwrap() - 0.01085736204758129).abs() <= 1e-15);
        assert_eq!(ab_mag_err(0.0, 0.01), None);
        assert_eq!(ab_mag_err(1.0, -0.1), None);
        assert_eq!(ab_mag_err(1.0, f64::NAN), None);
        assert_eq!(ab_mag_err(2.0, 0.0), Some(0.0));
    }

    #[test]
    fn the_line_flux_identity_of_the_contract_holds() {
        let channels_um_jy = [(2.0, 1.0), (2.001, 2.0), (2.002, 1.0)];
        let dlambda_um = 0.001;
        let dlambda_angstrom = 10.0;
        let flux_w_m2: f64 =
            channels_um_jy.iter().map(|&(lambda_um, fnu_jy)| fnu_jy_to_flambda_si_um(fnu_jy, lambda_um) * dlambda_um).sum();
        let flux_erg_s_cm2: f64 = channels_um_jy
            .iter()
            .map(|&(lambda_um, fnu_jy)| fnu_jy_to_flambda_cgs(fnu_jy, lambda_um) * dlambda_angstrom)
            .sum();
        let closed_form_w_m2 = 2.99792458e-12 * 0.001 * (1.0 / 4.0 + 2.0 / 4.004001 + 1.0 / 4.008004);
        assert!(rel(flux_w_m2, closed_form_w_m2) <= LINE_FLUX_RELATIVE_TOLERANCE, "{flux_w_m2} W m^-2");
        assert!(
            rel(flux_w_m2 * 1000.0, flux_erg_s_cm2) <= LINE_FLUX_RELATIVE_TOLERANCE,
            "{flux_erg_s_cm2} erg s^-1 cm^-2"
        );
    }

    #[test]
    fn the_enum_serialises_with_a_kind_tag() {
        assert_eq!(
            serde_json::to_value(SpectralFluxUnit::FluxDensityNu { jy_per_unit: 1e-6 }).unwrap(),
            json!({"kind": "flux_density_nu", "jy_per_unit": 1e-6})
        );
        assert_eq!(
            serde_json::to_value(SpectralFluxUnit::FluxDensityLambda { cgs_per_unit: 1.0 }).unwrap(),
            json!({"kind": "flux_density_lambda", "cgs_per_unit": 1.0})
        );
        assert_eq!(
            serde_json::to_value(SpectralFluxUnit::SurfaceBrightness).unwrap(),
            json!({"kind": "surface_brightness"})
        );
        assert_eq!(serde_json::to_value(SpectralFluxUnit::Rate).unwrap(), json!({"kind": "rate"}));
        assert_eq!(serde_json::to_value(SpectralFluxUnit::Unknown).unwrap(), json!({"kind": "unknown"}));
    }
}
