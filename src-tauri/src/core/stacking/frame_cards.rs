use crate::core::metadata::filter_wavelengths::effective_filter;
use crate::core::stacking::cfa_guard::cfa_pattern;
use crate::infra::image_source::load_plane_header;
use crate::types::header::HduHeader;
use crate::types::image_ref::ImageRef;

const EXPOSURE_KEYS: [&str; 3] = ["EXPTIME", "XPOSURE", "EFFEXPTM"];
const EXPOSURE_PRESENCE_KEYS: [&str; 3] = ["XPOSURE", "EFFEXPTM", "DURATION"];
const TEMPERATURE_KEYS: [&str; 2] = ["CCD-TEMP", "SET-TEMP"];
const UNSET_FILTERS: [&str; 2] = ["CLEAR", "NONE"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Bias,
    Dark,
    Flat,
    FlatDark,
    Light,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameCards {
    pub path: String,
    pub exposure_s: Option<f64>,
    pub filter: Option<String>,
    pub imagetyp: Option<String>,
    pub frame_kind: FrameKind,
    pub gain: Option<f64>,
    pub offset: Option<f64>,
    pub xbinning: Option<i64>,
    pub ybinning: Option<i64>,
    pub temp_c: Option<f64>,
    pub cfa: bool,
}

fn positive(v: &f64) -> bool {
    v.is_finite() && *v > 0.0
}

pub fn exposure_seconds(header: &HduHeader) -> Option<f64> {
    if let Some(v) = EXPOSURE_KEYS.iter().filter_map(|key| header.get_f64(key)).find(positive) {
        return Some(v);
    }
    if EXPOSURE_PRESENCE_KEYS.iter().any(|key| header.get(key).is_some()) {
        return None;
    }
    header.get_f64("EXPOSURE").filter(positive)
}

pub fn temperature_c(header: &HduHeader) -> Option<f64> {
    TEMPERATURE_KEYS.iter().filter_map(|key| header.get_f64(key)).find(|v| v.is_finite())
}

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    let text = header.get(key)?.trim().trim_matches('\'').trim();
    (!text.is_empty()).then(|| text.to_string())
}

pub fn frame_filter(header: &HduHeader) -> Option<String> {
    effective_filter(header)
        .map(|v| v.to_ascii_uppercase())
        .filter(|v| !UNSET_FILTERS.contains(&v.as_str()))
}

pub fn classify_imagetyp(imagetyp: Option<&str>) -> FrameKind {
    let Some(text) = imagetyp else {
        return FrameKind::Unknown;
    };
    let text = text.to_ascii_lowercase();
    if text.contains("bias") || text.contains("zero") {
        FrameKind::Bias
    } else if text.contains("flat") && text.contains("dark") {
        FrameKind::FlatDark
    } else if text.contains("flat") {
        FrameKind::Flat
    } else if text.contains("dark") {
        FrameKind::Dark
    } else if text.contains("light") || text.contains("object") {
        FrameKind::Light
    } else {
        FrameKind::Unknown
    }
}

fn integer_card(header: &HduHeader, key: &str) -> Option<i64> {
    header
        .get_i64(key)
        .or_else(|| header.get_f64(key).filter(|v| v.is_finite()).map(|v| v.round() as i64))
}

pub fn frame_cards_from_header(path: &str, header: &HduHeader) -> FrameCards {
    let imagetyp = card_text(header, "IMAGETYP").map(|v| v.to_ascii_lowercase());
    FrameCards {
        path: path.to_string(),
        exposure_s: exposure_seconds(header),
        filter: frame_filter(header),
        frame_kind: classify_imagetyp(imagetyp.as_deref()),
        imagetyp,
        gain: header.get_f64("GAIN").filter(|v| v.is_finite()),
        offset: header.get_f64("OFFSET").filter(|v| v.is_finite()),
        xbinning: integer_card(header, "XBINNING"),
        ybinning: integer_card(header, "YBINNING"),
        temp_c: temperature_c(header),
        cfa: cfa_pattern(header).is_some(),
    }
}

pub fn read_merged_header(path: &str) -> Option<HduHeader> {
    match load_plane_header(&ImageRef::parse(path)) {
        Ok(header) => Some(header),
        Err(e) => {
            log::warn!("Cannot read the header of {}: {:#}", path, e);
            None
        }
    }
}

pub fn read_frame_cards(path: &str) -> FrameCards {
    let header = read_merged_header(path).unwrap_or_else(HduHeader::empty);
    frame_cards_from_header(path, &header)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (k, v) in cards {
            h.set(k, (*v).to_string());
        }
        h
    }

    #[test]
    fn exposure_prefers_exptime_then_xposure_then_effexptm() {
        let jwst = header(&[("EXPOSURE", "1"), ("XPOSURE", "3221.04"), ("DURATION", "3221.04")]);
        assert_eq!(exposure_seconds(&jwst), Some(3221.04));
        assert_eq!(exposure_seconds(&header(&[("EXPOSURE", "300")])), Some(300.0));
        assert_eq!(exposure_seconds(&header(&[("EXPTIME", "120"), ("XPOSURE", "3221.04")])), Some(120.0));
        assert_eq!(exposure_seconds(&header(&[("EFFEXPTM", "5797.86"), ("EXPOSURE", "1")])), Some(5797.86));
        assert_eq!(exposure_seconds(&header(&[("EXPOSURE", "1"), ("DURATION", "7.5")])), None);
        assert_eq!(exposure_seconds(&header(&[("EXPTIME", "0")])), None);
        assert_eq!(exposure_seconds(&HduHeader::empty()), None);
    }

    #[test]
    fn jwst_mef_exposure_is_read_from_the_sci_hdu() {
        use crate::infra::fits::reader::test_fixtures::{write_test_mef, HduData, TestHdu};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jwst_i2d.fits");
        write_test_mef(
            &path,
            &[("EXPOSURE", "1".to_string())],
            &[TestHdu {
                extname: Some("SCI"),
                extver: Some(1),
                cols: 2,
                rows: 2,
                data: HduData::F32(vec![1.0, 2.0, 3.0, 4.0]),
                extra_cards: vec![("XPOSURE", "3221.04".to_string()), ("BUNIT", "'MJy/sr'".to_string())],
            }],
        );
        let path = path.to_str().unwrap();
        assert_eq!(crate::core::stacking::calibration::read_exposure_seconds(path), Some(3221.04));
        let cards = read_frame_cards(path);
        assert_eq!(cards.exposure_s, Some(3221.04));
        assert_eq!(cards.frame_kind, FrameKind::Unknown);
        assert!(!cards.cfa);
    }

    const REAL_DATA_EXPOSURES: [(&str, f64); 3] = [
        ("C:/astrokit/exampleFits/sample-data/heavyTest/jw02739-o001_t001_nircam_clear-f200w_i2d.fits", 3221.04),
        ("C:/astrokit/exampleFits/sample-data/heavyTest/jw02739-o001_t001_nircam_clear-f187n_i2d.fits", 5797.86),
        ("C:/astrokit/exampleFits/sample-data/502nmos.fits", 1100.0),
    ];

    #[test]
    #[ignore]
    fn real_data_exposure_reads_xposure_from_jwst_sci() {
        for (path, expected) in REAL_DATA_EXPOSURES {
            if !std::path::Path::new(path).exists() {
                continue;
            }
            let seconds = crate::core::stacking::calibration::read_exposure_seconds(path);
            assert!(
                seconds.is_some_and(|s| (s - expected).abs() <= 0.01),
                "{path}: {seconds:?} vs {expected}"
            );
            assert_eq!(read_frame_cards(path).exposure_s, seconds);
        }
        let wfpc2 = read_frame_cards(REAL_DATA_EXPOSURES[2].0);
        if wfpc2.exposure_s.is_some() {
            assert_eq!(wfpc2.imagetyp.as_deref(), Some("ext"));
            assert_eq!(wfpc2.frame_kind, FrameKind::Unknown);
            assert_eq!(wfpc2.filter.as_deref(), Some("F502N"));
            assert_eq!(wfpc2.temp_c, None);
            assert!(!wfpc2.cfa);
        }
    }

    #[test]
    fn temperature_falls_back_to_set_temp() {
        assert_eq!(temperature_c(&header(&[("SET-TEMP", "-10")])), Some(-10.0));
        assert_eq!(temperature_c(&header(&[("CCD-TEMP", "-9.8"), ("SET-TEMP", "-10")])), Some(-9.8));
        assert_eq!(temperature_c(&HduHeader::empty()), None);
    }

    #[test]
    fn frame_cards_classify_imagetyp_and_normalise_the_filter() {
        let h = header(&[
            ("IMAGETYP", "'Flat Dark'"),
            ("FILTNAM1", "'F502N'"),
            ("GAIN", "100"),
            ("OFFSET", "30"),
            ("XBINNING", "2"),
            ("YBINNING", "2"),
            ("BAYERPAT", "'RGGB'"),
            ("CCD-TEMP", "-9.96"),
            ("EXPTIME", "2.5"),
        ]);
        let cards = frame_cards_from_header("flatdark.fits", &h);
        assert_eq!(cards.frame_kind, FrameKind::FlatDark);
        assert_eq!(cards.imagetyp.as_deref(), Some("flat dark"));
        assert_eq!(cards.filter.as_deref(), Some("F502N"));
        assert_eq!((cards.gain, cards.offset), (Some(100.0), Some(30.0)));
        assert_eq!((cards.xbinning, cards.ybinning), (Some(2), Some(2)));
        assert!(cards.cfa);
        assert_eq!(cards.temp_c, Some(-9.96));
        assert_eq!(cards.exposure_s, Some(2.5));

        for (value, kind) in [
            ("Bias", FrameKind::Bias),
            ("ZERO", FrameKind::Bias),
            ("DARK", FrameKind::Dark),
            ("Dark Frame", FrameKind::Dark),
            ("Flat Field", FrameKind::Flat),
            ("Light Frame", FrameKind::Light),
            ("object", FrameKind::Light),
            ("ext", FrameKind::Unknown),
        ] {
            assert_eq!(classify_imagetyp(Some(value)), kind, "{value}");
        }
        assert_eq!(classify_imagetyp(None), FrameKind::Unknown);
        assert_eq!(frame_filter(&header(&[("FILTER", "CLEAR")])), None);
        assert_eq!(frame_filter(&header(&[("FILTER", "'ha '")])), Some("HA".to_string()));
        assert_eq!(frame_filter(&header(&[("FILTER", "'CLEAR'"), ("PUPIL", "'F162M'")])), Some("F162M".to_string()));
        assert_eq!(frame_filter(&header(&[("FILTER", "'F200W'"), ("PUPIL", "'CLEAR'")])), Some("F200W".to_string()));
        assert_eq!(frame_filter(&header(&[("FILTER1", "'CLEAR1L'"), ("FILTER2", "'F814W'")])), Some("F814W".to_string()));
        assert_eq!(read_frame_cards("C:/definitely/missing/frame.fits").frame_kind, FrameKind::Unknown);
    }
}
