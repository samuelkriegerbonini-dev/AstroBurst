use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::types::constants::FILTER_WAVELENGTHS_NM;
use crate::types::HduHeader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NarrowbandFilter {
    #[serde(rename = "Hα (656nm)")]
    Ha,
    #[serde(rename = "[OIII] (501nm)")]
    Oiii,
    #[serde(rename = "[SII] (673nm)")]
    Sii,
    #[serde(rename = "Unknown")]
    Unknown,
}

impl std::fmt::Display for NarrowbandFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ha => write!(f, "Hα (656nm)"),
            Self::Oiii => write!(f, "[OIII] (501nm)"),
            Self::Sii => write!(f, "[SII] (673nm)"),
            Self::Unknown => write!(f, "Unknown"),
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
    pub hubble_channel: HubbleChannel,
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
            NarrowbandFilter::Unknown => vec![],
        },
        PaletteType::Hoo | PaletteType::NaturalColor => match filter {
            NarrowbandFilter::Ha => vec![HubbleChannel::Red],
            NarrowbandFilter::Oiii => vec![HubbleChannel::Green, HubbleChannel::Blue],
            NarrowbandFilter::Sii => vec![],
            NarrowbandFilter::Unknown => vec![],
        },
        PaletteType::Hos => match filter {
            NarrowbandFilter::Ha => vec![HubbleChannel::Red],
            NarrowbandFilter::Oiii => vec![HubbleChannel::Green],
            NarrowbandFilter::Sii => vec![HubbleChannel::Blue],
            NarrowbandFilter::Unknown => vec![],
        },
        PaletteType::Custom => vec![],
    }
}

fn filter_to_hubble_channel(filter: NarrowbandFilter) -> HubbleChannel {
    match filter {
        NarrowbandFilter::Sii => HubbleChannel::Red,
        NarrowbandFilter::Ha => HubbleChannel::Green,
        NarrowbandFilter::Oiii => HubbleChannel::Blue,
        NarrowbandFilter::Unknown => HubbleChannel::Green,
    }
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
        hubble_channel: filter_to_hubble_channel(filter),
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

    let wavelength = header.get_f64("WAVELEN")
        .or_else(|| header.get_f64("CRVAL3"))
        .or_else(|| header.get_f64("WAVELENG"))?;

    let filter = classify_wavelength_nm(wavelength)?;
    Some(make_detection(filter, Confidence::Medium, "WAVELEN", &format!("{:.1}nm", wavelength)))
}

fn classify_wavelength_nm(nm: f64) -> Option<NarrowbandFilter> {
    let nm = if nm > 1000.0 { nm / 10.0 } else { nm };

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

pub fn filter_to_wavelength_nm(filter: &str) -> Option<u32> {
    let upper = filter.to_uppercase();
    if let Some(nm) = lookup_filter_nm(upper.trim()) {
        return Some(nm);
    }
    for token in upper.split(|c: char| !c.is_alphanumeric()) {
        if token.is_empty() || token == "CLEAR" {
            continue;
        }
        if let Some(nm) = lookup_filter_nm(token) {
            return Some(nm);
        }
    }
    None
}

fn lookup_filter_nm(key: &str) -> Option<u32> {
    FILTER_WAVELENGTHS_NM
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, nm)| *nm)
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
        HduHeader { cards, index }
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
        assert_eq!(det.hubble_channel, HubbleChannel::Green);
        assert_eq!(det.confidence, Confidence::High);
    }

    #[test]
    fn test_detect_oiii_keyword() {
        let h = header_with(&[("FILTER", "OIII 6nm")]);
        let det = detect_filter(&h).unwrap();
        assert_eq!(det.filter, NarrowbandFilter::Oiii);
        assert_eq!(det.hubble_channel, HubbleChannel::Blue);
    }

    #[test]
    fn test_detect_sii_keyword() {
        let h = header_with(&[("FILTER", "SII narrowband")]);
        let det = detect_filter(&h).unwrap();
        assert_eq!(det.filter, NarrowbandFilter::Sii);
        assert_eq!(det.hubble_channel, HubbleChannel::Red);
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
    fn test_filter_to_wavelength_miri() {
        assert_eq!(filter_to_wavelength_nm("F560W"), Some(5600));
        assert_eq!(filter_to_wavelength_nm("F770W"), Some(7700));
        assert_eq!(filter_to_wavelength_nm("F1000W"), Some(10000));
        assert_eq!(filter_to_wavelength_nm("F1130W"), Some(11300));
        assert_eq!(filter_to_wavelength_nm("F1280W"), Some(12800));
        assert_eq!(filter_to_wavelength_nm("F1500W"), Some(15000));
        assert_eq!(filter_to_wavelength_nm("F1800W"), Some(18000));
        assert_eq!(filter_to_wavelength_nm("F2100W"), Some(21000));
        assert_eq!(filter_to_wavelength_nm("F2550W"), Some(25500));
        assert_eq!(filter_to_wavelength_nm("MIRI/F1000W"), Some(10000));
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
    fn test_filter_to_wavelength_named() {
        assert_eq!(filter_to_wavelength_nm("F656N"), Some(656));
        assert_eq!(filter_to_wavelength_nm("F501N"), Some(501));
        assert_eq!(filter_to_wavelength_nm("F502N"), Some(501));
        assert_eq!(filter_to_wavelength_nm("F673N"), Some(673));
        assert_eq!(filter_to_wavelength_nm("F090W"), Some(900));
        assert_eq!(filter_to_wavelength_nm("F187N"), Some(1870));
        assert_eq!(filter_to_wavelength_nm("F200W"), Some(2000));
        assert_eq!(filter_to_wavelength_nm("F335M"), Some(3350));
        assert_eq!(filter_to_wavelength_nm("F444W"), Some(4440));
    }

    #[test]
    fn test_filter_to_wavelength_normalization() {
        assert_eq!(filter_to_wavelength_nm("f200w"), Some(2000));
        assert_eq!(filter_to_wavelength_nm("F200W-CLEAR"), Some(2000));
        assert_eq!(filter_to_wavelength_nm("CLEAR;F200W"), Some(2000));
        assert_eq!(filter_to_wavelength_nm("F164N+F150W2"), Some(1640));
        assert_eq!(filter_to_wavelength_nm("  F090W  "), Some(900));
    }

    #[test]
    fn test_filter_to_wavelength_aliases_and_unknown() {
        assert_eq!(filter_to_wavelength_nm("Ha"), Some(656));
        assert_eq!(filter_to_wavelength_nm("OIII"), Some(501));
        assert_eq!(filter_to_wavelength_nm("SII"), Some(673));
        assert_eq!(filter_to_wavelength_nm("Luminance"), None);
        assert_eq!(filter_to_wavelength_nm(""), None);
    }

    #[test]
    fn test_filter_wavelength_table_no_duplicate_keys() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for (k, _) in FILTER_WAVELENGTHS_NM {
            assert!(seen.insert(*k), "duplicate filter key in table: {k}");
        }
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
