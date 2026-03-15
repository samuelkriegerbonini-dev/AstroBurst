use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::types::HduHeader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NarrowbandFilter {
    #[serde(rename = "Hα (656nm)")]
    Ha,
    #[serde(rename = "[OIII] (502nm)")]
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
            Self::Oiii => write!(f, "[OIII] (502nm)"),
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

fn re_ha() -> &'static Regex {
    RE_HA.get_or_init(|| {
        Regex::new(r"(?i)(\bH[\-_]?(?:alpha|a)\b|656\s*(?:nm|\.?\d)|H_?α)").unwrap()
    })
}

fn re_oiii() -> &'static Regex {
    RE_OIII.get_or_init(|| {
        Regex::new(r"(?i)(\bO\s*III\b|\[?OIII\]?|502\s*(?:nm|\.?\d)|O3\b)").unwrap()
    })
}

fn re_sii() -> &'static Regex {
    RE_SII.get_or_init(|| {
        Regex::new(r"(?i)(\bS\s*II\b|\[?SII\]?|673\s*(?:nm|\.?\d)|S2\b)").unwrap()
    })
}

const FILTER_MATCHERS: [(NarrowbandFilter, fn(&str) -> bool); 3] = [
    (NarrowbandFilter::Ha, |v| re_ha().is_match(v)),
    (NarrowbandFilter::Oiii, |v| re_oiii().is_match(v)),
    (NarrowbandFilter::Sii, |v| re_sii().is_match(v)),
];

const DISCOVERY_KEYWORDS: &[&str] = &[
    "FILTER", "FILTER1", "FILTER2", "FILTER3",
    "INSTRUME", "OBJECT", "IMAGETYP",
    "FILT_ID", "FILTNAM", "FILTNAME",
];

const FILENAME_PATTERNS: &[(NarrowbandFilter, &[&str])] = &[
    (NarrowbandFilter::Ha, &["_HA", "_HALPHA", "-HA", "_H_ALPHA", "656"]),
    (NarrowbandFilter::Oiii, &["_OIII", "-OIII", "_O3", "-O3", "502"]),
    (NarrowbandFilter::Sii, &["_SII", "-SII", "_S2", "-S2", "673"]),
];

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

fn match_filter_value(value: &str, keyword: &str) -> Option<FilterDetection> {
    let confidence = keyword_confidence(keyword);
    for &(filter, matcher) in &FILTER_MATCHERS {
        if matcher(value) {
            return Some(make_detection(filter, confidence, keyword, value));
        }
    }
    None
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
        if key_upper.contains("FILT") || key_upper.contains("BAND") || key_upper.contains("LINE") {
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

pub fn suggest_palette(files: &[(String, HduHeader)]) -> PaletteSuggestion {
    let mut r_file: Option<(Confidence, ChannelSuggestion)> = None;
    let mut g_file: Option<(Confidence, ChannelSuggestion)> = None;
    let mut b_file: Option<(Confidence, ChannelSuggestion)> = None;
    let mut unmapped: Vec<ChannelSuggestion> = Vec::new();

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

        let channel = detection.as_ref().map(|d| (d.hubble_channel, d.confidence));
        match channel {
            Some((HubbleChannel::Red, conf)) => {
                if r_file.as_ref().map_or(true, |(c, _)| conf < *c) {
                    if let Some((_, prev)) = r_file.replace((conf, suggestion)) {
                        unmapped.push(prev);
                    }
                } else {
                    unmapped.push(suggestion);
                }
            }
            Some((HubbleChannel::Green, conf)) => {
                if g_file.as_ref().map_or(true, |(c, _)| conf < *c) {
                    if let Some((_, prev)) = g_file.replace((conf, suggestion)) {
                        unmapped.push(prev);
                    }
                } else {
                    unmapped.push(suggestion);
                }
            }
            Some((HubbleChannel::Blue, conf)) => {
                if b_file.as_ref().map_or(true, |(c, _)| conf < *c) {
                    if let Some((_, prev)) = b_file.replace((conf, suggestion)) {
                        unmapped.push(prev);
                    }
                } else {
                    unmapped.push(suggestion);
                }
            }
            None => unmapped.push(suggestion),
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
        palette_name: "SHO (Hubble Palette)".into(),
    }
}

fn detect_from_filename(name: &str) -> Option<FilterDetection> {
    let upper = name.to_uppercase();

    for &(filter, patterns) in FILENAME_PATTERNS {
        for &pat in patterns {
            if upper.contains(pat) {
                return Some(make_detection(filter, Confidence::Low, "filename", name));
            }
        }
    }

    None
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
        let patterns = ["Ha", "H-alpha", "Halpha", "H_alpha", "H_Alpha", "656nm", "656.3"];
        for p in patterns {
            assert!(re_ha().is_match(p), "Ha regex should match '{p}'");
        }
    }

    #[test]
    fn test_regex_patterns_oiii() {
        let patterns = ["OIII", "[OIII]", "O III", "O3", "502nm"];
        for p in patterns {
            assert!(re_oiii().is_match(p), "OIII regex should match '{p}'");
        }
    }

    #[test]
    fn test_regex_patterns_sii() {
        let patterns = ["SII", "[SII]", "S II", "S2", "673nm"];
        for p in patterns {
            assert!(re_sii().is_match(p), "SII regex should match '{p}'");
        }
    }

    #[test]
    fn test_wavelength_angstrom_normalization() {
        let filter = classify_wavelength_nm(6563.0);
        assert_eq!(filter, Some(NarrowbandFilter::Ha));
    }
}
