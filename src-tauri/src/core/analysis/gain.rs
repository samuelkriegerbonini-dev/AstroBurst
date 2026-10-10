use serde::Serialize;

use crate::core::metadata::photcal::{is_mjy_per_sr, normalized_unit};
use crate::types::constants::{
    HEADER_COMBINE_METHOD, HEADER_DISPLAY_REFERRED_CARD, HEADER_DRIZZLE_SCALE, HEADER_NCOMBINE,
};
use crate::types::header::HduHeader;

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PoissonRoute {
    HeaderGain,
    ErrPlane,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UnitClass {
    Counts,
    CountRate,
    Electrons,
    ElectronRate,
    Calibrated,
    Other,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GainModel {
    pub gain_e_per_adu: Option<f64>,
    pub source: Option<String>,
    pub gain_card: Option<f64>,
    pub unit_class: UnitClass,
    pub ncombine: Option<u32>,
    pub combine_method: Option<String>,
    pub drizzle_scale: Option<f64>,
    pub combine_scaled: bool,
    pub effective_gain: Option<f64>,
    pub fallback_gain: Option<f64>,
    pub poisson_route: PoissonRoute,
    pub note: Option<String>,
}

pub const GAIN_KEYWORDS: [&str; 4] = ["EGAIN", "ATODGAIN", "CCDGAIN", "ADCGAIN"];
pub const CAMERA_GAIN_KEYWORD: &str = "GAIN";
pub const SKY_ONLY_ERRORS_WARNING: &str = "flux errors leave out source photon noise: no gain was given and the frames carry no ERR plane; enter the gain in e-/ADU to include it";

const PHOTON_NOISE_PREFIX: &str = "flux errors leave out source photon noise: ";
const ERR_PLANE_NOTE: &str = "Poisson noise from the ERR plane";
const NO_HEADER_GAIN_NOTE: &str = "no gain in the header";
const DISPLAY_REFERRED_NOTE: &str = "display-referred data: no gain applies";
const BUNIT_KEY: &str = "BUNIT";
const EXPTIME_KEY: &str = "EXPTIME";
const COUNT_UNITS: [&str; 4] = ["COUNTS", "COUNT", "DN", "ADU"];
const COUNT_RATE_UNITS: [&str; 4] = ["COUNTS/S", "COUNT/S", "DN/S", "ADU/S"];
const ELECTRON_UNITS: [&str; 3] = ["ELECTRONS", "ELECTRON", "E-"];
const ELECTRON_RATE_UNITS: [&str; 3] = ["ELECTRONS/S", "ELECTRON/S", "E-/S"];
const FLUX_DENSITY_UNITS: [&str; 2] = ["JY", "MJY"];
const COMBINE_MEAN: &str = "mean";
const COMBINE_MEDIAN: &str = "median";
const COMBINE_EXTREMES: [&str; 2] = ["min", "max"];
const SIGNIFICANT_DIGITS: i32 = 4;

pub fn classify_unit(normalized_bunit: Option<&str>) -> UnitClass {
    let unit = match normalized_bunit {
        Some(unit) if !unit.is_empty() => unit,
        _ => return UnitClass::Counts,
    };
    if COUNT_UNITS.contains(&unit) {
        UnitClass::Counts
    } else if COUNT_RATE_UNITS.contains(&unit) {
        UnitClass::CountRate
    } else if ELECTRON_UNITS.contains(&unit) {
        UnitClass::Electrons
    } else if ELECTRON_RATE_UNITS.contains(&unit) {
        UnitClass::ElectronRate
    } else if is_mjy_per_sr(unit) || FLUX_DENSITY_UNITS.contains(&unit) {
        UnitClass::Calibrated
    } else {
        UnitClass::Other
    }
}

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    let cleaned = header.get(key)?.trim().trim_matches('\'').trim();
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

fn positive_card(header: &HduHeader, key: &str) -> Option<f64> {
    header.get_f64(key).filter(|v| v.is_finite() && *v > 0.0)
}

fn is_display_referred(header: &HduHeader) -> bool {
    card_text(header, HEADER_DISPLAY_REFERRED_CARD).is_some_and(|v| v == "T")
}

fn format_sig4(v: f64) -> String {
    if !v.is_finite() || v == 0.0 {
        return format!("{v}");
    }
    let magnitude = v.abs().log10().floor() as i32;
    let decimals = (SIGNIFICANT_DIGITS - 1 - magnitude).max(0) as usize;
    let rounded = if magnitude >= SIGNIFICANT_DIGITS {
        let scale = 10f64.powi(magnitude - SIGNIFICANT_DIGITS + 1);
        (v / scale).round() * scale
    } else {
        v
    };
    let text = format!("{:.*}", decimals, rounded);
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

struct UnitGain {
    gain: f64,
    source: String,
}

fn header_gain(header: &HduHeader) -> Option<(&'static str, f64)> {
    GAIN_KEYWORDS.iter().find_map(|key| positive_card(header, key).map(|g| (*key, g)))
}

fn unit_gain(
    class: UnitClass,
    header: &HduHeader,
    bunit: Option<&str>,
    keyword_gain: Option<(&'static str, f64)>,
) -> (Option<UnitGain>, Option<String>) {
    let exptime = positive_card(header, EXPTIME_KEY);
    let bunit_text = bunit.unwrap_or("");
    match class {
        UnitClass::Counts => (
            keyword_gain.map(|(key, gain)| UnitGain { gain, source: key.to_string() }),
            None,
        ),
        UnitClass::Electrons => (Some(UnitGain { gain: 1.0, source: "BUNIT electrons".to_string() }), None),
        UnitClass::ElectronRate => match exptime {
            Some(t) => (Some(UnitGain { gain: t, source: "BUNIT electrons/s × EXPTIME".to_string() }), None),
            None => (None, Some("BUNIT electrons/s without EXPTIME: gain unavailable".to_string())),
        },
        UnitClass::CountRate => match (keyword_gain, exptime) {
            (Some((key, gain)), Some(t)) => (Some(UnitGain { gain: gain * t, source: format!("{key} × EXPTIME") }), None),
            (Some(_), None) => (None, Some(format!("BUNIT '{bunit_text}' without EXPTIME: gain unavailable"))),
            (None, _) => (None, None),
        },
        UnitClass::Calibrated => (None, Some(format!("JWST/Roman calibrated units ({bunit_text}): no gain applies"))),
        UnitClass::Other => (None, Some(format!("BUNIT '{bunit_text}' is not a count unit: no gain applies"))),
    }
}

struct CombineCards {
    ncombine: Option<u32>,
    method: Option<String>,
    drizzle_scale: Option<f64>,
}

fn combine_cards(header: &HduHeader) -> CombineCards {
    let ncombine = header
        .get_i64(HEADER_NCOMBINE)
        .or_else(|| header.get_f64(HEADER_NCOMBINE).filter(|v| v.is_finite() && v.fract() == 0.0).map(|v| v as i64))
        .filter(|n| *n > 1)
        .map(|n| n as u32);
    let method = card_text(header, HEADER_COMBINE_METHOD).map(|m| m.to_lowercase());
    let drizzle_scale = header.get_f64(HEADER_DRIZZLE_SCALE).filter(|s| s.is_finite() && *s >= 1.0);
    CombineCards { ncombine, method, drizzle_scale }
}

struct ScaledGain {
    effective: f64,
    scaled: bool,
    note: Option<String>,
}

fn scaled_gain(unit: &UnitGain, combine: &CombineCards) -> ScaledGain {
    let g = unit.gain;
    let gain_text = format_sig4(g);
    let Some(n) = combine.ncombine else {
        return ScaledGain { effective: g, scaled: false, note: None };
    };
    if let Some(scale) = combine.drizzle_scale {
        let effective = n as f64 * g / (scale * scale);
        return ScaledGain {
            effective,
            scaled: true,
            note: Some(format!(
                "drizzle of {n} frames at scale {s} (NCOMBINE, ABDRZSCL): effective gain {n} × {gain_text} / {s}² = {eff} e-/ADU; noise is correlated between output pixels, so the Poisson term is approximate",
                s = format_sig4(scale),
                eff = format_sig4(effective)
            )),
        };
    }
    match combine.method.as_deref() {
        Some(COMBINE_MEAN) => {
            let effective = n as f64 * g;
            ScaledGain {
                effective,
                scaled: true,
                note: Some(format!(
                    "mean stack of {n} frames (NCOMBINE, ABCOMB): effective gain {n} × {gain_text} = {} e-/ADU",
                    format_sig4(effective)
                )),
            }
        }
        Some(COMBINE_MEDIAN) => {
            let effective = 2.0 * n as f64 * g / std::f64::consts::PI;
            ScaledGain {
                effective,
                scaled: true,
                note: Some(format!(
                    "median stack of {n} frames: effective gain 2 × {n} × {gain_text} / π = {} e-/ADU (median variance is π/2 of the mean's)",
                    format_sig4(effective)
                )),
            }
        }
        Some(method) if COMBINE_EXTREMES.contains(&method) => ScaledGain {
            effective: g,
            scaled: false,
            note: Some(format!(
                "{method} combine of {n} frames: the gain is not scaled (an extreme-value combine has no Poisson scaling)"
            )),
        },
        _ => ScaledGain {
            effective: g,
            scaled: false,
            note: Some(format!(
                "NCOMBINE={n} in the header: gain not scaled because the combine method is unknown; enter {n} × {gain_text} = {} e-/ADU for a mean stack",
                format_sig4(n as f64 * g)
            )),
        },
    }
}

pub fn gain_model(header: Option<&HduHeader>, has_err_plane: bool) -> GainModel {
    let empty = HduHeader::empty();
    let header = header.unwrap_or(&empty);
    let bunit = card_text(header, BUNIT_KEY);
    let normalized = bunit.as_deref().map(normalized_unit);
    let unit_class = classify_unit(normalized.as_deref());
    let keyword_gain = header_gain(header);
    let gain_card = positive_card(header, CAMERA_GAIN_KEYWORD);
    let (unit, class_note) = unit_gain(unit_class, header, bunit.as_deref(), keyword_gain);
    let combine = combine_cards(header);
    let scaled = unit.as_ref().map(|u| scaled_gain(u, &combine));

    let mut model = GainModel {
        gain_e_per_adu: unit.as_ref().map(|u| u.gain),
        source: unit.as_ref().map(|u| u.source.clone()),
        gain_card,
        unit_class,
        ncombine: combine.ncombine,
        combine_method: combine.method.clone(),
        drizzle_scale: combine.drizzle_scale,
        combine_scaled: false,
        effective_gain: None,
        fallback_gain: None,
        poisson_route: PoissonRoute::Unavailable,
        note: None,
    };

    if is_display_referred(header) {
        model.note = Some(DISPLAY_REFERRED_NOTE.to_string());
        return model;
    }
    if has_err_plane {
        model.poisson_route = PoissonRoute::ErrPlane;
        model.fallback_gain = scaled.as_ref().map(|s| s.effective);
        model.combine_scaled = scaled.as_ref().is_some_and(|s| s.scaled);
        let mut note = ERR_PLANE_NOTE.to_string();
        if let (Some(s), Some(u)) = (&scaled, &unit) {
            note.push_str(&format!(
                "; header gain {} e-/ADU ({}) used only where ERR is not finite",
                format_sig4(s.effective),
                u.source
            ));
        }
        model.note = Some(note);
        return model;
    }
    match scaled {
        Some(s) => {
            model.poisson_route = PoissonRoute::HeaderGain;
            model.effective_gain = Some(s.effective);
            model.combine_scaled = s.scaled;
            model.note = s.note;
        }
        None => {
            model.note = Some(class_note.unwrap_or_else(|| NO_HEADER_GAIN_NOTE.to_string()));
        }
    }
    model
}

pub fn photon_noise_warning(model: &GainModel, gain_given: bool, err_used: bool) -> Option<String> {
    if gain_given || err_used {
        return None;
    }
    if model.note.as_deref() == Some(DISPLAY_REFERRED_NOTE) {
        return Some(format!("{PHOTON_NOISE_PREFIX}{DISPLAY_REFERRED_NOTE}"));
    }
    match model.unit_class {
        UnitClass::Counts | UnitClass::CountRate | UnitClass::Electrons | UnitClass::ElectronRate => {
            Some(SKY_ONLY_ERRORS_WARNING.to_string())
        }
        UnitClass::Calibrated | UnitClass::Other => Some(format!(
            "{PHOTON_NOISE_PREFIX}{}",
            model.note.as_deref().unwrap_or("no gain applies")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (k, v) in cards {
            h.set(k, v.to_string());
        }
        h
    }

    fn model(cards: &[(&str, &str)], has_err: bool) -> GainModel {
        gain_model(Some(&header(cards)), has_err)
    }

    fn note_of(m: &GainModel) -> &str {
        m.note.as_deref().unwrap_or("")
    }

    #[test]
    fn egain_is_read_and_the_camera_gain_setting_is_reported_not_applied() {
        let m = model(&[("GAIN", "120"), ("EGAIN", "0.25")], false);
        assert_eq!(m.gain_e_per_adu, Some(0.25));
        assert_eq!(m.source.as_deref(), Some("EGAIN"));
        assert_eq!(m.gain_card, Some(120.0));
        assert_eq!(m.poisson_route, PoissonRoute::HeaderGain);
        assert_eq!(m.effective_gain, Some(0.25));
        assert_eq!(m.unit_class, UnitClass::Counts);

        let camera_only = model(&[("GAIN", "120")], false);
        assert_eq!(camera_only.gain_e_per_adu, None);
        assert_eq!(camera_only.source, None);
        assert_eq!(camera_only.gain_card, Some(120.0));
        assert_eq!(camera_only.poisson_route, PoissonRoute::Unavailable);
        assert!(note_of(&camera_only).contains("no gain in the header"), "{:?}", camera_only.note);
    }

    #[test]
    fn hst_gain_keywords_and_electron_units() {
        let wfpc2 = model(&[("ATODGAIN", "7")], false);
        assert_eq!(wfpc2.gain_e_per_adu, Some(7.0));
        assert_eq!(wfpc2.source.as_deref(), Some("ATODGAIN"));
        assert_eq!(wfpc2.poisson_route, PoissonRoute::HeaderGain);

        let electrons = model(&[("CCDGAIN", "2"), ("BUNIT", "ELECTRONS")], false);
        assert_eq!(electrons.unit_class, UnitClass::Electrons);
        assert_eq!(electrons.gain_e_per_adu, Some(1.0));
        assert_eq!(electrons.source.as_deref(), Some("BUNIT electrons"));
        assert_eq!(electrons.effective_gain, Some(1.0));

        let rate = model(&[("BUNIT", "ELECTRONS/S"), ("EXPTIME", "1100")], false);
        assert_eq!(rate.unit_class, UnitClass::ElectronRate);
        assert_eq!(rate.gain_e_per_adu, Some(1100.0));
        assert_eq!(rate.effective_gain, Some(1100.0));
        assert_eq!(rate.poisson_route, PoissonRoute::HeaderGain);

        let no_exptime = model(&[("BUNIT", "ELECTRONS/S")], false);
        assert_eq!(no_exptime.poisson_route, PoissonRoute::Unavailable);
        assert_eq!(no_exptime.effective_gain, None);
        assert!(note_of(&no_exptime).contains("EXPTIME"), "{:?}", no_exptime.note);
    }

    #[test]
    fn an_err_plane_wins_over_every_unit() {
        let rate = model(&[("BUNIT", "DN/s"), ("EXPTIME", "100"), ("EGAIN", "2")], true);
        assert_eq!(rate.poisson_route, PoissonRoute::ErrPlane);
        assert_eq!(rate.effective_gain, None);
        assert_eq!(rate.fallback_gain, Some(200.0));
        assert!(note_of(&rate).starts_with("Poisson noise from the ERR plane"), "{:?}", rate.note);
        assert!(note_of(&rate).contains("used only where ERR is not finite"), "{:?}", rate.note);

        let electrons = model(&[("BUNIT", "ELECTRONS")], true);
        assert_eq!(electrons.poisson_route, PoissonRoute::ErrPlane);
        assert_eq!(electrons.fallback_gain, Some(1.0));

        let calibrated = model(&[("BUNIT", "MJy/sr")], true);
        assert_eq!(calibrated.poisson_route, PoissonRoute::ErrPlane);
        assert_eq!(calibrated.fallback_gain, None);
        assert_eq!(calibrated.effective_gain, None);
        assert_eq!(note_of(&calibrated), "Poisson noise from the ERR plane");
    }

    #[test]
    fn calibrated_and_physical_units_have_no_gain() {
        let jwst = model(&[("BUNIT", "MJy/sr")], false);
        assert_eq!(jwst.poisson_route, PoissonRoute::Unavailable);
        assert_eq!(jwst.unit_class, UnitClass::Calibrated);
        assert!(note_of(&jwst).contains("no gain applies"), "{:?}", jwst.note);
        assert_eq!(jwst.effective_gain, None);

        let physical = model(&[("BUNIT", "ERG/S/CM2/A"), ("EGAIN", "2"), ("EXPTIME", "10")], false);
        assert_eq!(physical.poisson_route, PoissonRoute::Unavailable);
        assert_eq!(physical.unit_class, UnitClass::Other);
        assert_eq!(physical.effective_gain, None);
        assert!(note_of(&physical).contains("not a count unit"), "{:?}", physical.note);

        assert_eq!(classify_unit(Some("JY")), UnitClass::Calibrated);
        assert_eq!(classify_unit(Some("MJY")), UnitClass::Calibrated);
        assert_eq!(classify_unit(Some("W/M2/HZ")), UnitClass::Other);
        assert_eq!(classify_unit(None), UnitClass::Counts);
    }

    #[test]
    fn count_rate_units_are_a_whitelist() {
        for unit in ["COUNTS/S", "COUNT/S", "DN/S", "ADU/S"] {
            let m = model(&[("BUNIT", unit), ("EGAIN", "0.5"), ("EXPTIME", "10")], false);
            assert_eq!(m.unit_class, UnitClass::CountRate, "{unit}");
            assert_eq!(m.gain_e_per_adu, Some(5.0), "{unit}");
            assert_eq!(m.source.as_deref(), Some("EGAIN × EXPTIME"), "{unit}");
            assert_eq!(m.poisson_route, PoissonRoute::HeaderGain, "{unit}");
        }
        let no_exptime = model(&[("BUNIT", "DN/S"), ("EGAIN", "0.5")], false);
        assert_eq!(no_exptime.poisson_route, PoissonRoute::Unavailable);
        assert_eq!(no_exptime.effective_gain, None);
    }

    #[test]
    fn combine_cards_scale_the_gain_by_method() {
        let mean = model(&[("NCOMBINE", "16"), ("ABCOMB", "mean"), ("EGAIN", "0.25")], false);
        assert_eq!(mean.effective_gain, Some(4.0));
        assert!(mean.combine_scaled);
        assert_eq!(mean.ncombine, Some(16));
        assert_eq!(mean.combine_method.as_deref(), Some("mean"));
        assert_eq!(mean.gain_e_per_adu, Some(0.25));
        assert!(note_of(&mean).contains("mean stack of 16 frames"), "{:?}", mean.note);

        let median = model(&[("NCOMBINE", "16"), ("ABCOMB", "median"), ("EGAIN", "0.25")], false);
        let expected = 2.0 * 16.0 * 0.25 / std::f64::consts::PI;
        assert!((median.effective_gain.unwrap() - expected).abs() < 1e-9, "{:?}", median.effective_gain);
        assert!(median.combine_scaled);

        let min = model(&[("NCOMBINE", "16"), ("ABCOMB", "min"), ("EGAIN", "0.25")], false);
        assert_eq!(min.effective_gain, Some(0.25));
        assert!(!min.combine_scaled);
        assert!(note_of(&min).contains("extreme-value"), "{:?}", min.note);

        let drizzle = model(&[("NCOMBINE", "16"), ("ABDRZSCL", "2"), ("EGAIN", "0.25")], false);
        assert_eq!(drizzle.effective_gain, Some(1.0));
        assert!(drizzle.combine_scaled);
        assert_eq!(drizzle.drizzle_scale, Some(2.0));
        assert!(note_of(&drizzle).contains("correlated"), "{:?}", drizzle.note);

        let external = model(&[("NCOMBINE", "2"), ("ATODGAIN", "7")], false);
        assert_eq!(external.effective_gain, Some(7.0));
        assert!(!external.combine_scaled);
        assert_eq!(external.ncombine, Some(2));
        assert_eq!(external.combine_method, None);
        assert!(note_of(&external).contains("NCOMBINE=2"), "{:?}", external.note);
        assert!(note_of(&external).contains("14"), "{:?}", external.note);
    }

    #[test]
    fn display_referred_marker_disables_the_gain() {
        let m = model(&[("EGAIN", "0.25"), (HEADER_DISPLAY_REFERRED_CARD, "T")], false);
        assert_eq!(m.poisson_route, PoissonRoute::Unavailable);
        assert_eq!(m.effective_gain, None);
        assert!(note_of(&m).contains("display-referred"), "{:?}", m.note);
        assert_eq!(
            photon_noise_warning(&m, false, false).as_deref(),
            Some("flux errors leave out source photon noise: display-referred data: no gain applies")
        );
        assert_eq!(photon_noise_warning(&m, true, false), None);
    }

    #[test]
    fn photon_noise_warning_picks_the_text_by_unit_class() {
        let counts = model(&[("BUNIT", "ADU")], false);
        assert_eq!(photon_noise_warning(&counts, false, false).as_deref(), Some(SKY_ONLY_ERRORS_WARNING));
        let calibrated = model(&[("BUNIT", "MJy/sr")], false);
        assert_eq!(
            photon_noise_warning(&calibrated, false, false).as_deref(),
            Some("flux errors leave out source photon noise: JWST/Roman calibrated units (MJy/sr): no gain applies")
        );
        assert_eq!(photon_noise_warning(&counts, true, false), None);
        assert_eq!(photon_noise_warning(&counts, false, true), None);
        assert_eq!(photon_noise_warning(&calibrated, false, true), None);
    }

    #[test]
    fn synth_frames_prefill_from_egain() {
        use crate::core::synth::noise::NoiseParams;
        use crate::core::synth::pipeline::frame_header;
        let header = frame_header(&NoiseParams { gain: 1.5, ..NoiseParams::default() }, 0, 1.0);
        let m = gain_model(Some(&header), false);
        assert_eq!(m.source.as_deref(), Some("EGAIN"), "{m:?}");
        assert_eq!(m.gain_e_per_adu, Some(1.5));
        assert_eq!(m.effective_gain, Some(1.5));
        assert_eq!(m.poisson_route, PoissonRoute::HeaderGain);
        assert_eq!(m.unit_class, UnitClass::Counts);
        assert_eq!(m.gain_card, Some(1.5));
    }

    #[test]
    fn notes_format_numbers_with_four_significant_digits_and_no_trailing_zeros() {
        assert_eq!(format_sig4(0.25), "0.25");
        assert_eq!(format_sig4(4.0), "4");
        assert_eq!(format_sig4(1100.0), "1100");
        assert_eq!(format_sig4(14.0), "14");
        assert_eq!(format_sig4(2.5464790894703255), "2.546");
        assert_eq!(format_sig4(3221.04), "3221");
        assert_eq!(format_sig4(12345.0), "12350");
        assert_eq!(format_sig4(0.0), "0");
    }
}
