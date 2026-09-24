use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::types::HduHeader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NarrowbandFilter {
    #[serde(rename = "Hα (656nm)")]
    Ha,
    #[serde(rename = "[OIII] (501nm)")]
    Oiii,
    #[serde(rename = "[SII] (673nm)")]
    Sii,
}

impl std::fmt::Display for NarrowbandFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ha => write!(f, "Hα (656nm)"),
            Self::Oiii => write!(f, "[OIII] (501nm)"),
            Self::Sii => write!(f, "[SII] (673nm)"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HubbleChannel {
    #[serde(rename = "R")]
    Red,
    #[serde(rename = "G")]
    Green,
    #[serde(rename = "B")]
    Blue,
}

impl std::fmt::Display for HubbleChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Red => write!(f, "R"),
            Self::Green => write!(f, "G"),
            Self::Blue => write!(f, "B"),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FilterDetection {
    pub filter: NarrowbandFilter,
    pub confidence: Confidence,
    pub matched_keyword: String,
    pub matched_value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChannelSuggestion {
    pub file_path: String,
    pub file_name: String,
    pub detection: Option<FilterDetection>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PaletteSuggestion {
    pub r_file: Option<ChannelSuggestion>,
    pub g_file: Option<ChannelSuggestion>,
    pub b_file: Option<ChannelSuggestion>,
    pub unmapped: Vec<ChannelSuggestion>,
    pub is_complete: bool,
    pub palette_name: String,
}

static RE_HA: OnceLock<Regex> = OnceLock::new();
static RE_OIII: OnceLock<Regex> = OnceLock::new();
static RE_SII: OnceLock<Regex> = OnceLock::new();
static RE_FILTER_CODE: OnceLock<Regex> = OnceLock::new();
static RE_WAVELENGTH_WITH_UNIT: OnceLock<Regex> = OnceLock::new();

fn re_ha() -> &'static Regex {
    RE_HA.get_or_init(|| Regex::new(r"(?i)(\bH[\-_]?(?:alpha|a)\b|H_?α)").unwrap())
}

fn re_oiii() -> &'static Regex {
    RE_OIII.get_or_init(|| Regex::new(r"(?i)(\bO\s*III\b|\[?OIII\]?|\bO3\b)").unwrap())
}

fn re_sii() -> &'static Regex {
    RE_SII.get_or_init(|| Regex::new(r"(?i)(\bS\s*II\b|\[?SII\]?|\bS2\b)").unwrap())
}

fn re_filter_code() -> &'static Regex {
    RE_FILTER_CODE.get_or_init(|| Regex::new(r"(?i)\bF\d{3,4}[NWM]\d?\b").unwrap())
}

fn re_wavelength_with_unit() -> &'static Regex {
    RE_WAVELENGTH_WITH_UNIT.get_or_init(|| {
        Regex::new(r"(?i)(?:^|[^0-9A-Za-z.])(\d{3,5}(?:\.\d+)?)\s*(?:nm|angstroms?|Å|A)(?:[^0-9A-Za-z]|$)")
            .unwrap()
    })
}

const FILTER_MATCHERS: [(NarrowbandFilter, fn(&str) -> bool); 3] = [
    (NarrowbandFilter::Ha, |v| re_ha().is_match(v)),
    (NarrowbandFilter::Oiii, |v| re_oiii().is_match(v)),
    (NarrowbandFilter::Sii, |v| re_sii().is_match(v)),
];

const WAVELENGTH_AXIS_CODES: [&str; 2] = ["WAVE", "AWAV"];
const ANGSTROM_HEURISTIC_LIMIT: f64 = 1000.0;

const DISCOVERY_KEYWORDS: &[&str] = &[
    "FILTER", "FILTER1", "FILTER2", "FILTER3",
    "INSTRUME", "IMAGETYP",
    "FILT_ID", "FILTNAM", "FILTNAME",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PaletteType {
    #[serde(rename = "SHO")]
    Sho,
    #[serde(rename = "HOO")]
    Hoo,
    #[serde(rename = "HOS")]
    Hos,
    #[serde(rename = "NaturalColor")]
    NaturalColor,
    #[serde(rename = "Custom")]
    Custom,
}

impl PaletteType {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Sho => "SHO (Hubble Palette)",
            Self::Hoo => "HOO",
            Self::Hos => "HOS",
            Self::NaturalColor => "Natural Color",
            Self::Custom => "Custom",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "SHO" | "HUBBLE" => Self::Sho,
            "HOO" => Self::Hoo,
            "HOS" => Self::Hos,
            "NATURAL" | "NATURALCOLOR" | "NATURAL_COLOR" => Self::NaturalColor,
            "CUSTOM" => Self::Custom,
            _ => Self::Sho,
        }
    }
}

impl Default for PaletteType {
    fn default() -> Self {
        Self::Sho
    }
}

fn palette_channels(palette: &PaletteType, filter: NarrowbandFilter) -> Vec<HubbleChannel> {
    match palette {
        PaletteType::Sho => match filter {
            NarrowbandFilter::Sii => vec![HubbleChannel::Red],
            NarrowbandFilter::Ha => vec![HubbleChannel::Green],
            NarrowbandFilter::Oiii => vec![HubbleChannel::Blue],
        },
        PaletteType::Hoo | PaletteType::NaturalColor => match filter {
            NarrowbandFilter::Ha => vec![HubbleChannel::Red],
            NarrowbandFilter::Oiii => vec![HubbleChannel::Green, HubbleChannel::Blue],
            NarrowbandFilter::Sii => vec![],
        },
        PaletteType::Hos => match filter {
            NarrowbandFilter::Ha => vec![HubbleChannel::Red],
            NarrowbandFilter::Oiii => vec![HubbleChannel::Green],
            NarrowbandFilter::Sii => vec![HubbleChannel::Blue],
        },
        PaletteType::Custom => vec![],
    }
}

pub fn palette_channel(palette: &PaletteType, filter: NarrowbandFilter) -> Option<HubbleChannel> {
    palette_channels(palette, filter).first().copied()
}

fn keyword_confidence(keyword: &str) -> Confidence {
    match keyword.to_uppercase().as_str() {
        "FILTER" | "FILTER1" | "FILTER2" | "FILTER3"
        | "FILT_ID" | "FILTNAM" | "FILTNAME" => Confidence::High,
        "INSTRUME" => Confidence::Medium,
        _ => Confidence::Low,
    }
}

fn make_detection(filter: NarrowbandFilter, confidence: Confidence, keyword: &str, value: &str) -> FilterDetection {
    FilterDetection {
        filter,
        confidence,
        matched_keyword: keyword.to_string(),
        matched_value: value.to_string(),
    }
}

const BARE_WAVELENGTH_KEYWORDS: &[&str] = &[
    "FILTER", "FILTER1", "FILTER2", "FILTER3",
    "FILT_ID", "FILTNAM", "FILTNAME",
    "WAVELEN", "WAVELENG",
];

fn keyword_allows_bare_wavelength(keyword: &str) -> bool {
    let upper = keyword.to_uppercase();
    BARE_WAVELENGTH_KEYWORDS.contains(&upper.as_str())
}

fn narrowband_for_filter_code(code: &str) -> Option<NarrowbandFilter> {
    match code {
        "F656N" | "F657N" => Some(NarrowbandFilter::Ha),
        "F501N" | "F502N" | "F503N" => Some(NarrowbandFilter::Oiii),
        "F673N" => Some(NarrowbandFilter::Sii),
        _ => None,
    }
}

fn filter_code_filter(value: &str) -> Option<NarrowbandFilter> {
    re_filter_code()
        .find_iter(value)
        .find_map(|m| narrowband_for_filter_code(&m.as_str().to_uppercase()))
}

fn unit_wavelength_filter(value: &str) -> Option<NarrowbandFilter> {
    re_wavelength_with_unit()
        .captures_iter(value)
        .filter_map(|c| c[1].parse::<f64>().ok())
        .find_map(classify_wavelength_nm)
}

fn bare_wavelength_filter(value: &str) -> Option<NarrowbandFilter> {
    value
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse::<f64>().ok())
        .find_map(classify_wavelength_nm)
}

fn wavelength_filter_in_value(value: &str, allow_bare: bool) -> Option<NarrowbandFilter> {
    if let Some(filter) = filter_code_filter(value) {
        return Some(filter);
    }
    if let Some(filter) = unit_wavelength_filter(value) {
        return Some(filter);
    }
    if allow_bare && !re_filter_code().is_match(value) {
        return bare_wavelength_filter(value);
    }
    None
}

fn match_filter_value(value: &str, keyword: &str) -> Option<FilterDetection> {
    let confidence = keyword_confidence(keyword);
    for &(filter, matcher) in &FILTER_MATCHERS {
        if matcher(value) {
            return Some(make_detection(filter, confidence, keyword, value));
        }
    }
    wavelength_filter_in_value(value, keyword_allows_bare_wavelength(keyword))
        .map(|filter| make_detection(filter, confidence, keyword, value))
}

pub fn detect_filter(header: &HduHeader) -> Option<FilterDetection> {
    for &keyword in DISCOVERY_KEYWORDS {
        let value = match header.get(keyword) {
            Some(v) => v.to_string(),
            None => continue,
        };
        if let Some(det) = match_filter_value(&value, keyword) {
            return Some(det);
        }
    }

    for (keyword, value) in &header.cards {
        let key_upper = keyword.to_uppercase();
        if key_upper.contains("FILT") || key_upper == "BAND" || key_upper == "LINE" {
            if let Some(det) = match_filter_value(value, keyword) {
                return Some(det);
            }
        }
    }

    detect_from_wavelength_card(header)
}

struct WavelengthCard {
    keyword: &'static str,
    value: f64,
    unit: Option<String>,
}

fn clean_card(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

fn bare_wavelength_card(header: &HduHeader, keyword: &'static str) -> Option<WavelengthCard> {
    header.get_f64(keyword).map(|value| WavelengthCard { keyword, value, unit: None })
}

fn spectral_axis_wavelength_card(header: &HduHeader) -> Option<WavelengthCard> {
    let ctype = clean_card(header, "CTYPE3")?.to_uppercase();
    if !WAVELENGTH_AXIS_CODES.iter().any(|code| ctype.starts_with(code)) {
        return None;
    }
    let value = header.get_f64("CRVAL3")?;
    let unit = clean_card(header, "CUNIT3").unwrap_or_else(|| "m".to_string());
    Some(WavelengthCard { keyword: "CRVAL3", value, unit: Some(unit) })
}

fn nm_per_unit(unit: &str) -> Option<f64> {
    match unit.trim().to_lowercase().as_str() {
        "nm" | "nanometer" | "nanometers" | "nanometre" | "nanometres" => Some(1.0),
        "angstrom" | "angstroms" | "a" | "å" | "ang" => Some(0.1),
        "um" | "micron" | "microns" | "micrometer" | "micrometers" | "µm" | "μm" => Some(1.0e3),
        "mm" => Some(1.0e6),
        "cm" => Some(1.0e7),
        "m" | "meter" | "meters" | "metre" | "metres" => Some(1.0e9),
        _ => None,
    }
}

fn heuristic_wavelength_unit(value: f64) -> &'static str {
    if value > ANGSTROM_HEURISTIC_LIMIT { "Angstrom" } else { "nm" }
}

fn heuristic_wavelength_nm(value: f64) -> f64 {
    if value > ANGSTROM_HEURISTIC_LIMIT { value / 10.0 } else { value }
}

fn detect_from_wavelength_card(header: &HduHeader) -> Option<FilterDetection> {
    let card = bare_wavelength_card(header, "WAVELEN")
        .or_else(|| spectral_axis_wavelength_card(header))
        .or_else(|| bare_wavelength_card(header, "WAVELENG"))?;
    let (nm, label) = match card.unit.as_deref() {
        Some(unit) => (card.value * nm_per_unit(unit)?, format!("{} {}", card.value, unit)),
        None => (
            heuristic_wavelength_nm(card.value),
            format!("{} {}", card.value, heuristic_wavelength_unit(card.value)),
        ),
    };
    let filter = narrowband_at_nm(nm)?;
    Some(make_detection(filter, Confidence::Medium, card.keyword, &label))
}

fn classify_wavelength_nm(value: f64) -> Option<NarrowbandFilter> {
    narrowband_at_nm(heuristic_wavelength_nm(value))
}

fn narrowband_at_nm(nm: f64) -> Option<NarrowbandFilter> {
    if (649.0..=663.0).contains(&nm) {
        Some(NarrowbandFilter::Ha)
    } else if (495.0..=510.0).contains(&nm) {
        Some(NarrowbandFilter::Oiii)
    } else if (666.0..=680.0).contains(&nm) {
        Some(NarrowbandFilter::Sii)
    } else {
        None
    }
}

pub fn suggest_palette(files: &[(String, HduHeader)]) -> PaletteSuggestion {
    suggest_palette_with_type(files, &PaletteType::default())
}

pub fn suggest_palette_with_type(files: &[(String, HduHeader)], palette: &PaletteType) -> PaletteSuggestion {
    if *palette == PaletteType::Custom {
        let suggestions: Vec<ChannelSuggestion> = files
            .iter()
            .map(|(path, header)| {
                let file_name = Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                let detection = detect_filter(header)
                    .or_else(|| detect_from_filename(&file_name));
                ChannelSuggestion { file_path: path.clone(), file_name, detection }
            })
            .collect();
        return PaletteSuggestion {
            r_file: None,
            g_file: None,
            b_file: None,
            unmapped: suggestions,
            is_complete: false,
            palette_name: palette.display_name().into(),
        };
    }

    let mut r_file: Option<(Confidence, ChannelSuggestion)> = None;
    let mut g_file: Option<(Confidence, ChannelSuggestion)> = None;
    let mut b_file: Option<(Confidence, ChannelSuggestion)> = None;
    let mut unmapped: Vec<ChannelSuggestion> = Vec::new();

    fn try_assign(
        slot: &mut Option<(Confidence, ChannelSuggestion)>,
        conf: Confidence,
        suggestion: ChannelSuggestion,
    ) -> (bool, Option<ChannelSuggestion>) {
        if slot.as_ref().map_or(true, |(c, _)| conf < *c) {
            let prev = slot.replace((conf, suggestion)).map(|(_, p)| p);
            (true, prev)
        } else {
            (false, None)
        }
    }

    for (path, header) in files {
        let file_name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());

        let detection = detect_filter(header)
            .or_else(|| detect_from_filename(&file_name));

        let suggestion = ChannelSuggestion {
            file_path: path.clone(),
            file_name,
            detection: detection.clone(),
        };

        if let Some(det) = &detection {
            let channels = palette_channels(palette, det.filter);
            if channels.is_empty() {
                unmapped.push(suggestion);
                continue;
            }

            let conf = det.confidence;
            let mut assigned = false;
            let mut displaced: Vec<ChannelSuggestion> = Vec::new();

            for ch in &channels {
                let sug = suggestion.clone();
                let slot = match ch {
                    HubbleChannel::Red => &mut r_file,
                    HubbleChannel::Green => &mut g_file,
                    HubbleChannel::Blue => &mut b_file,
                };
                let (ok, prev) = try_assign(slot, conf, sug);
                if ok {
                    assigned = true;
                }
                if let Some(p) = prev {
                    if !displaced.iter().any(|d| d.file_path == p.file_path) {
                        displaced.push(p);
                    }
                }
            }

            unmapped.extend(displaced);

            if !assigned {
                unmapped.push(suggestion);
            }
        } else {
            unmapped.push(suggestion);
        }
    }

    let r = r_file.map(|(_, s)| s);
    let g = g_file.map(|(_, s)| s);
    let b = b_file.map(|(_, s)| s);
    let is_complete = r.is_some() && g.is_some() && b.is_some();

    PaletteSuggestion {
        r_file: r,
        g_file: g,
        b_file: b,
        unmapped,
        is_complete,
        palette_name: palette.display_name().into(),
    }
}

fn filename_token_filter(token: &str, next: Option<&str>) -> Option<NarrowbandFilter> {
    match token {
        "HA" | "HALPHA" => Some(NarrowbandFilter::Ha),
        "OIII" | "O3" => Some(NarrowbandFilter::Oiii),
        "SII" | "S2" => Some(NarrowbandFilter::Sii),
        "H" if next == Some("ALPHA") => Some(NarrowbandFilter::Ha),
        _ if re_filter_code().is_match(token) => narrowband_for_filter_code(token),
        _ => None,
    }
}

fn detect_from_filename(name: &str) -> Option<FilterDetection> {
    let upper = name.to_uppercase();
    let tokens: Vec<&str> = upper
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();

    let from_token = tokens
        .iter()
        .enumerate()
        .find_map(|(i, token)| filename_token_filter(token, tokens.get(i + 1).copied()));

    from_token
        .or_else(|| unit_wavelength_filter(&upper))
        .map(|filter| make_detection(filter, Confidence::Low, "filename", name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn header_with(pairs: &[(&str, &str)]) -> HduHeader {
        let cards: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let index: HashMap<String, String> = cards.iter().cloned().collect();
        HduHeader { cards, index, string_keys: None }
    }

    #[test]
    fn test_detect_ha_filter_keyword() {
        let h = header_with(&[
            ("BITPIX", "16"),
            ("NAXIS", "2"),
            ("FILTER", "H-alpha 7nm"),
        ]);
        let det = detect_filter(&h).unwrap();
        assert_eq!(det.filter, NarrowbandFilter::Ha);
        assert_eq!(palette_channel(&PaletteType::Sho, det.filter), Some(HubbleChannel::Green));
        assert_eq!(det.confidence, Confidence::High);
    }

    #[test]
    fn test_detect_oiii_keyword() {
        let h = header_with(&[("FILTER", "OIII 6nm")]);
        let det = detect_filter(&h).unwrap();
        assert_eq!(det.filter, NarrowbandFilter::Oiii);
        assert_eq!(palette_channel(&PaletteType::Sho, det.filter), Some(HubbleChannel::Blue));
    }

    #[test]
    fn test_detect_sii_keyword() {
        let h = header_with(&[("FILTER", "SII narrowband")]);
        let det = detect_filter(&h).unwrap();
        assert_eq!(det.filter, NarrowbandFilter::Sii);
        assert_eq!(palette_channel(&PaletteType::Sho, det.filter), Some(HubbleChannel::Red));
    }

    #[test]
    fn test_detect_by_wavelength_656nm() {
        let h = header_with(&[("FILTER", "Narrowband"), ("WAVELEN", "656.3")]);
        let det = detect_filter(&h).unwrap();
        assert_eq!(det.filter, NarrowbandFilter::Ha);
    }

    #[test]
    fn test_detect_by_wavelength_502nm() {
        let h = header_with(&[("WAVELEN", "502.0")]);
        let det = detect_filter(&h).unwrap();
        assert_eq!(det.filter, NarrowbandFilter::Oiii);
    }

    #[test]
    fn test_detect_by_wavelength_673nm() {
        let h = header_with(&[("WAVELEN", "673.0")]);
        let det = detect_filter(&h).unwrap();
        assert_eq!(det.filter, NarrowbandFilter::Sii);
    }

    #[test]
    fn test_fallback_wildcard_keyword() {
        let h = header_with(&[("MYFILTER", "Ha 7nm")]);
        let det = detect_filter(&h).unwrap();
        assert_eq!(det.filter, NarrowbandFilter::Ha);
    }

    #[test]
    fn test_unknown_returns_none() {
        let h = header_with(&[("FILTER", "Luminance")]);
        let det = detect_filter(&h);
        assert!(det.is_none());
    }

    #[test]
    fn test_filename_fallback() {
        let det = detect_from_filename("M42_Ha_300s.fits").unwrap();
        assert_eq!(det.filter, NarrowbandFilter::Ha);
        assert_eq!(det.confidence, Confidence::Low);
    }

    #[test]
    fn test_filename_oiii() {
        let det = detect_from_filename("NGC7000-OIII-120s.fits").unwrap();
        assert_eq!(det.filter, NarrowbandFilter::Oiii);
    }

    #[test]
    fn test_filename_sii() {
        let det = detect_from_filename("IC1396_SII_600s.fits").unwrap();
        assert_eq!(det.filter, NarrowbandFilter::Sii);
    }

    #[test]
    fn test_suggest_palette_complete() {
        let files = vec![
            ("eagle_sii.fits".into(), header_with(&[("FILTER", "SII")])),
            ("eagle_ha.fits".into(), header_with(&[("FILTER", "H-alpha")])),
            ("eagle_oiii.fits".into(), header_with(&[("FILTER", "OIII")])),
        ];

        let palette = suggest_palette(&files);
        assert!(palette.is_complete);
        assert_eq!(palette.r_file.as_ref().unwrap().file_path, "eagle_sii.fits");
        assert_eq!(palette.g_file.as_ref().unwrap().file_path, "eagle_ha.fits");
        assert_eq!(palette.b_file.as_ref().unwrap().file_path, "eagle_oiii.fits");
        assert!(palette.unmapped.is_empty());
    }

    #[test]
    fn test_suggest_palette_partial() {
        let files = vec![
            ("img_ha.fits".into(), header_with(&[("FILTER", "Ha")])),
            ("img_lum.fits".into(), header_with(&[("FILTER", "Luminance")])),
        ];

        let palette = suggest_palette(&files);
        assert!(!palette.is_complete);
        assert!(palette.g_file.is_some());
        assert_eq!(palette.unmapped.len(), 1);
    }

    #[test]
    fn test_suggest_palette_prefers_higher_confidence() {
        let files = vec![
            ("file1.fits".into(), header_with(&[("FILTER", "H-alpha")])),
            ("file2_Ha.fits".into(), header_with(&[("OBJECT", "M42")])),
        ];

        let palette = suggest_palette(&files);
        assert_eq!(
            palette.g_file.as_ref().unwrap().file_path,
            "file1.fits",
        );
        assert_eq!(palette.unmapped.len(), 1);
    }

    #[test]
    fn test_regex_patterns_ha() {
        let patterns = ["Ha", "H-alpha", "Halpha", "H_alpha", "H_Alpha"];
        for p in patterns {
            assert!(re_ha().is_match(p), "Ha regex should match '{p}'");
        }
    }

    #[test]
    fn test_regex_patterns_oiii() {
        let patterns = ["OIII", "[OIII]", "O III", "O3"];
        for p in patterns {
            assert!(re_oiii().is_match(p), "OIII regex should match '{p}'");
        }
    }

    #[test]
    fn test_regex_patterns_sii() {
        let patterns = ["SII", "[SII]", "S II", "S2"];
        for p in patterns {
            assert!(re_sii().is_match(p), "SII regex should match '{p}'");
        }
    }

    #[test]
    fn test_numeric_filter_values_still_detected() {
        let cases = [
            ("656nm", NarrowbandFilter::Ha),
            ("656.3", NarrowbandFilter::Ha),
            ("6563 A", NarrowbandFilter::Ha),
            ("F656N", NarrowbandFilter::Ha),
            ("502nm", NarrowbandFilter::Oiii),
            ("500.7nm", NarrowbandFilter::Oiii),
            ("5007", NarrowbandFilter::Oiii),
            ("F502N", NarrowbandFilter::Oiii),
            ("F501N", NarrowbandFilter::Oiii),
            ("673nm", NarrowbandFilter::Sii),
            ("673.1", NarrowbandFilter::Sii),
            ("F673N", NarrowbandFilter::Sii),
        ];
        for (value, expected) in cases {
            let h = header_with(&[("FILTER", value)]);
            let det = detect_filter(&h).unwrap_or_else(|| panic!("FILTER='{value}' should be detected"));
            assert_eq!(det.filter, expected, "FILTER='{value}'");
        }
    }

    #[test]
    fn test_object_catalogue_designation_is_not_a_filter() {
        let designations = [
            "NGC 6565",
            "NGC 5007",
            "NGC 6730",
            "HD 165634",
            "SDSS J165604.5+000000",
            "M 5012",
            "NGC 6563",
        ];
        for name in designations {
            let h = header_with(&[("OBJECT", name), ("TARGNAME", name)]);
            assert!(
                detect_filter(&h).is_none(),
                "OBJECT='{name}' must not be read as a filter, got {:?}",
                detect_filter(&h).map(|d| d.filter)
            );
        }
    }

    #[test]
    fn test_instrume_digits_are_not_a_filter() {
        let h = header_with(&[("INSTRUME", "WFC3 6563"), ("IMAGETYP", "SCI 6565")]);
        assert!(
            detect_filter(&h).is_none(),
            "digits in INSTRUME/IMAGETYP must not be read as a wavelength, got {:?}",
            detect_filter(&h).map(|d| d.filter)
        );
    }

    #[test]
    fn test_mast_filename_is_not_narrowband() {
        let names = [
            "jw06565-o001_t001_nircam_clear-f444w_i2d.fits",
            "jw06565-o002_t002_nircam_f200w-clear_i2d.fits",
            "jw06565-o003_t003_miri_f1130w_i2d.fits",
            "jw01234-o001_t001_nircam_clear-f480m_i2d.fits",
        ];
        for name in names {
            assert!(
                detect_from_filename(name).is_none(),
                "'{name}' must not be detected as narrowband, got {:?}",
                detect_from_filename(name).map(|d| d.filter)
            );
        }
    }

    #[test]
    fn test_filename_digits_are_not_wavelengths() {
        let names = [
            "jw06565_exp_300s.fits",
            "NGC5007_L_120s.fits",
            "target_673_survey_20240502.fits",
        ];
        for name in names {
            assert!(
                detect_from_filename(name).is_none(),
                "'{name}' must not be detected, got {:?}",
                detect_from_filename(name).map(|d| d.filter)
            );
        }
    }

    #[test]
    fn test_filename_narrowband_code_and_unit() {
        assert_eq!(
            detect_from_filename("m16_f656n_600s.fits").unwrap().filter,
            NarrowbandFilter::Ha
        );
        assert_eq!(
            detect_from_filename("M42_656nm_120s.fits").unwrap().filter,
            NarrowbandFilter::Ha
        );
        assert_eq!(
            detect_from_filename("NGC7000_H_alpha.fits").unwrap().filter,
            NarrowbandFilter::Ha
        );
    }

    #[test]
    fn test_suggest_palette_jwst_broadband_stays_unmapped() {
        let files = vec![
            (
                "jw06565-o001_t001_nircam_clear-f444w_i2d.fits".to_string(),
                header_with(&[("TARGNAME", "NGC 6565"), ("FILTER", "F444W")]),
            ),
            (
                "jw06565-o002_t001_nircam_clear-f200w_i2d.fits".to_string(),
                header_with(&[("TARGNAME", "NGC 6565"), ("FILTER", "F200W")]),
            ),
            (
                "jw06565-o003_t001_miri_f1130w_i2d.fits".to_string(),
                header_with(&[("TARGNAME", "NGC 6565"), ("FILTER", "F1130W")]),
            ),
        ];

        let palette = suggest_palette(&files);
        assert!(!palette.is_complete);
        assert!(palette.r_file.is_none());
        assert!(palette.g_file.is_none());
        assert!(palette.b_file.is_none());
        assert_eq!(palette.unmapped.len(), 3);
        assert!(palette.unmapped.iter().all(|s| s.detection.is_none()));
    }

    #[test]
    fn a_wavelength_fallback_names_the_card_it_read_and_that_card_unit() {
        let cube = header_with(&[("CTYPE3", "AWAV"), ("CUNIT3", "Angstrom"), ("CRVAL3", "6563.0")]);
        let det = detect_filter(&cube).expect("an Angstrom H-alpha axis");
        assert_eq!(det.filter, NarrowbandFilter::Ha);
        assert_eq!(det.matched_keyword, "CRVAL3");
        assert_eq!(det.matched_value, "6563 Angstrom");

        let microns = header_with(&[("CTYPE3", "WAVE"), ("CUNIT3", "um"), ("CRVAL3", "0.5007")]);
        let det = detect_filter(&microns).expect("an [OIII] axis in microns");
        assert_eq!(det.filter, NarrowbandFilter::Oiii);
        assert_eq!(det.matched_value, "0.5007 um");

        let leng = header_with(&[("WAVELENG", "6731")]);
        let det = detect_filter(&leng).expect("WAVELENG in Angstrom");
        assert_eq!(det.filter, NarrowbandFilter::Sii);
        assert_eq!(det.matched_keyword, "WAVELENG");
        assert_eq!(det.matched_value, "6731 Angstrom");

        let wavelen = header_with(&[("WAVELEN", "656.3")]);
        let det = detect_filter(&wavelen).unwrap();
        assert_eq!((det.matched_keyword.as_str(), det.matched_value.as_str()), ("WAVELEN", "656.3 nm"));
    }

    #[test]
    fn crval3_of_a_non_wavelength_axis_is_not_a_filter() {
        for ctype in ["TIME", "FREQ", "VRAD", "STOKES"] {
            let h = header_with(&[("CTYPE3", ctype), ("CRVAL3", "656.0")]);
            assert!(detect_filter(&h).is_none(), "CTYPE3={} CRVAL3=656 read as {:?}", ctype, detect_filter(&h).map(|d| d.filter));
        }
        assert!(detect_filter(&header_with(&[("CRVAL3", "6563.0")])).is_none());
    }

    #[test]
    fn a_spectral_axis_with_a_unit_is_not_rescaled_as_angstrom() {
        for (crval3, cunit3) in [("6.53", "um"), ("5.0", "um"), ("6.7", "um"), ("6.53e-6", "m"), ("6530", "nm")] {
            let h = header_with(&[("CTYPE3", "WAVE"), ("CUNIT3", cunit3), ("CRVAL3", crval3)]);
            assert!(detect_filter(&h).is_none(), "CRVAL3={} {} read as {:?}", crval3, cunit3, detect_filter(&h).map(|d| d.filter));
        }
        let unknown = header_with(&[("CTYPE3", "WAVE"), ("CUNIT3", "pixel"), ("CRVAL3", "656.3")]);
        assert!(detect_filter(&unknown).is_none());

        let default_metres = header_with(&[("CTYPE3", "WAVE"), ("CRVAL3", "6.563e-7")]);
        let det = detect_filter(&default_metres).expect("a WAVE axis without CUNIT3 is in metres");
        assert_eq!(det.filter, NarrowbandFilter::Ha);
        assert_eq!(det.matched_value, "0.0000006563 m");
        assert!(detect_filter(&header_with(&[("CTYPE3", "WAVE"), ("CRVAL3", "6563")])).is_none());
    }

    #[test]
    fn the_channel_follows_the_requested_palette() {
        assert_eq!(palette_channel(&PaletteType::Hoo, NarrowbandFilter::Ha), Some(HubbleChannel::Red));
        assert_eq!(palette_channel(&PaletteType::Hoo, NarrowbandFilter::Oiii), Some(HubbleChannel::Green));
        assert_eq!(palette_channel(&PaletteType::Hoo, NarrowbandFilter::Sii), None);
        assert_eq!(palette_channel(&PaletteType::Hos, NarrowbandFilter::Sii), Some(HubbleChannel::Blue));
        assert_eq!(palette_channel(&PaletteType::Sho, NarrowbandFilter::Ha), Some(HubbleChannel::Green));
        assert_eq!(palette_channel(&PaletteType::Custom, NarrowbandFilter::Ha), None);
    }

    #[test]
    fn test_bare_number_needs_a_filter_keyword() {
        let cases = [
            ("R_LINEAR", "crds://jwst_nircam_linearity_0656.fits"),
            ("S_LINEAR", "crds://jwst_miri_linearity_0501.fits"),
            ("PIPELINE", "calwebb 6563"),
            ("BASELINE", "5007"),
            ("SUBLINE", "6730"),
        ];
        for (keyword, value) in cases {
            let h = header_with(&[(keyword, value)]);
            assert!(
                detect_filter(&h).is_none(),
                "{keyword}='{value}' must not yield a filter, got {:?}",
                detect_filter(&h).map(|d| d.filter)
            );
        }
    }

    #[test]
    fn test_nii_filter_code_is_not_ha() {
        let h = header_with(&[("FILTER", "F658N")]);
        assert!(
            detect_filter(&h).is_none(),
            "FILTER='F658N' is [NII] and must not be read as Ha, got {:?}",
            detect_filter(&h).map(|d| d.filter)
        );
        assert!(
            detect_from_filename("m16_f658n_600s.fits").is_none(),
            "f658n filename must not be read as Ha, got {:?}",
            detect_from_filename("m16_f658n_600s.fits").map(|d| d.filter)
        );
    }

    #[test]
    fn test_nii_frame_does_not_complete_palette() {
        let files = vec![
            ("m16_f673n.fits".to_string(), header_with(&[("FILTER", "F673N")])),
            ("m16_f658n.fits".to_string(), header_with(&[("FILTER", "F658N")])),
            ("m16_f502n.fits".to_string(), header_with(&[("FILTER", "F502N")])),
        ];
        let palette = suggest_palette(&files);
        assert!(
            !palette.is_complete,
            "an [NII] frame must not complete an SHO palette, green={:?}",
            palette.g_file.as_ref().map(|s| s.file_path.clone())
        );
        assert!(palette.g_file.is_none());
    }

    #[test]
    fn test_unlisted_filter_code_agrees_between_header_and_filename() {
        let h = header_with(&[("FILTER", "F680N")]);
        assert!(
            detect_filter(&h).is_none(),
            "FILTER='F680N' is not a tracked narrowband code, got {:?}",
            detect_filter(&h).map(|d| d.filter)
        );
        assert!(detect_from_filename("NGC6543_F680N.fits").is_none());
    }

    #[test]
    fn test_narrowband_filter_codes_are_detected() {
        let cases = [
            ("F656N", NarrowbandFilter::Ha),
            ("F657N", NarrowbandFilter::Ha),
            ("F501N", NarrowbandFilter::Oiii),
            ("F502N", NarrowbandFilter::Oiii),
            ("F503N", NarrowbandFilter::Oiii),
            ("F673N", NarrowbandFilter::Sii),
        ];
        for (code, expected) in cases {
            let h = header_with(&[("FILTER", code)]);
            let det = detect_filter(&h).unwrap_or_else(|| panic!("FILTER='{code}' should be detected"));
            assert_eq!(det.filter, expected, "FILTER='{code}'");
            assert_eq!(det.confidence, Confidence::High, "FILTER='{code}'");
        }
    }

    #[test]
    fn test_miri_broadband_filter_values_are_not_narrowband() {
        let codes = [
            "F560W", "F770W", "F1000W", "F1130W", "F1280W", "F1500W", "F1800W", "F2100W", "F2550W",
        ];
        for code in codes {
            let h = header_with(&[("FILTER", code)]);
            assert!(
                detect_filter(&h).is_none(),
                "FILTER='{code}' must not be narrowband, got {:?}",
                detect_filter(&h).map(|d| d.filter)
            );
        }
    }

    #[test]
    fn test_regex_patterns_oiii_numeric_values() {
        for value in ["501nm", "5007", "500.7nm"] {
            let h = header_with(&[("FILTER", value)]);
            let det = detect_filter(&h).unwrap_or_else(|| panic!("FILTER='{value}' should be detected"));
            assert_eq!(det.filter, NarrowbandFilter::Oiii, "FILTER='{value}'");
        }
    }

    #[test]
    fn test_regex_patterns_sii_numeric_values() {
        for value in ["671nm", "673nm", "673.1"] {
            let h = header_with(&[("FILTER", value)]);
            let det = detect_filter(&h).unwrap_or_else(|| panic!("FILTER='{value}' should be detected"));
            assert_eq!(det.filter, NarrowbandFilter::Sii, "FILTER='{value}'");
        }
    }

    #[test]
    fn test_wavelength_angstrom_normalization() {
        let filter = classify_wavelength_nm(6563.0);
        assert_eq!(filter, Some(NarrowbandFilter::Ha));
    }

    #[test]
    fn test_suggest_palette_hoo() {
        let files = vec![
            ("eagle_ha.fits".into(), header_with(&[("FILTER", "H-alpha")])),
            ("eagle_oiii.fits".into(), header_with(&[("FILTER", "OIII")])),
            ("eagle_sii.fits".into(), header_with(&[("FILTER", "SII")])),
        ];

        let p = suggest_palette_with_type(&files, &PaletteType::Hoo);
        assert!(p.is_complete);
        assert_eq!(p.r_file.as_ref().unwrap().file_path, "eagle_ha.fits");
        assert_eq!(p.g_file.as_ref().unwrap().file_path, "eagle_oiii.fits");
        assert_eq!(p.b_file.as_ref().unwrap().file_path, "eagle_oiii.fits");
        assert_eq!(p.unmapped.len(), 1);
        assert_eq!(p.unmapped[0].file_path, "eagle_sii.fits");
    }

    #[test]
    fn test_suggest_palette_hos() {
        let files = vec![
            ("eagle_ha.fits".into(), header_with(&[("FILTER", "H-alpha")])),
            ("eagle_oiii.fits".into(), header_with(&[("FILTER", "OIII")])),
            ("eagle_sii.fits".into(), header_with(&[("FILTER", "SII")])),
        ];

        let p = suggest_palette_with_type(&files, &PaletteType::Hos);
        assert!(p.is_complete);
        assert_eq!(p.r_file.as_ref().unwrap().file_path, "eagle_ha.fits");
        assert_eq!(p.g_file.as_ref().unwrap().file_path, "eagle_oiii.fits");
        assert_eq!(p.b_file.as_ref().unwrap().file_path, "eagle_sii.fits");
        assert!(p.unmapped.is_empty());
    }

    #[test]
    fn test_suggest_palette_custom_all_unmapped() {
        let files = vec![
            ("eagle_ha.fits".into(), header_with(&[("FILTER", "H-alpha")])),
            ("eagle_oiii.fits".into(), header_with(&[("FILTER", "OIII")])),
        ];

        let p = suggest_palette_with_type(&files, &PaletteType::Custom);
        assert!(!p.is_complete);
        assert!(p.r_file.is_none());
        assert!(p.g_file.is_none());
        assert!(p.b_file.is_none());
        assert_eq!(p.unmapped.len(), 2);
    }

    #[test]
    fn test_palette_type_from_str() {
        assert_eq!(PaletteType::from_str_loose("SHO"), PaletteType::Sho);
        assert_eq!(PaletteType::from_str_loose("hoo"), PaletteType::Hoo);
        assert_eq!(PaletteType::from_str_loose("HOS"), PaletteType::Hos);
        assert_eq!(PaletteType::from_str_loose("natural"), PaletteType::NaturalColor);
        assert_eq!(PaletteType::from_str_loose("custom"), PaletteType::Custom);
        assert_eq!(PaletteType::from_str_loose("unknown"), PaletteType::Sho);
    }
}
