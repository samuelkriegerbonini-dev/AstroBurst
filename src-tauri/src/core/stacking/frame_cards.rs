use crate::core::imaging::cfa::CfaPattern;
use crate::core::metadata::filter_wavelengths::effective_filter;
use crate::core::metadata::header_aliases::{
    first_f64, first_i64, first_text, focal_length_mm, pixel_size_um, BINNING_KEYS, EXPOSURE_KEYS,
    EXPOSURE_PRESENCE_KEYS, IMAGETYP_KEYS, TEMPERATURE_KEYS, Y_BINNING_KEYS,
};
use crate::core::stacking::cfa_guard::cfa_pattern;
use crate::infra::image_source::load_plane_header;
use crate::types::header::HduHeader;
use crate::types::image_ref::ImageRef;

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
    pub imagetyp: Option<(String, &'static str)>,
    pub frame_kind: FrameKind,
    pub gain: Option<f64>,
    pub offset: Option<f64>,
    pub xbinning: Option<i64>,
    pub ybinning: Option<i64>,
    pub temp_c: Option<f64>,
    pub cfa: bool,
    pub cfa_pattern: Option<CfaPattern>,
    pub pixel_size_um: Option<(f64, &'static str)>,
    pub focal_mm: Option<(f64, &'static str)>,
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
    first_f64(header, &TEMPERATURE_KEYS).map(|(celsius, _)| celsius)
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

pub fn frame_cards_from_header(path: &str, header: &HduHeader) -> FrameCards {
    let imagetyp = first_text(header, &IMAGETYP_KEYS);
    FrameCards {
        path: path.to_string(),
        exposure_s: exposure_seconds(header),
        filter: frame_filter(header),
        frame_kind: classify_imagetyp(imagetyp.as_ref().map(|(text, _)| text.as_str())),
        imagetyp,
        gain: header.get_f64("GAIN").filter(|v| v.is_finite()),
        offset: header.get_f64("OFFSET").filter(|v| v.is_finite()),
        xbinning: first_i64(header, &BINNING_KEYS).map(|(factor, _)| factor),
        ybinning: first_i64(header, &Y_BINNING_KEYS).map(|(factor, _)| factor),
        temp_c: temperature_c(header),
        cfa: cfa_pattern(header).is_some(),
        cfa_pattern: CfaPattern::detect(header),
        pixel_size_um: pixel_size_um(header),
        focal_mm: focal_length_mm(header),
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
    use crate::core::imaging::cfa::test_fixtures::XTRANS_LAYOUT;
    use crate::core::imaging::debayer::BayerPattern;

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
            assert_eq!(wfpc2.imagetyp, Some(("EXT".to_string(), "IMAGETYP")));
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
        assert_eq!(cards.imagetyp, Some(("Flat Dark".to_string(), "IMAGETYP")));
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

    #[test]
    fn maxim_header_fills_pixel_size_and_focal_with_their_keys() {
        let maxim = header(&[
            ("SWCREATE", "'MaxIm DL Version 6.40'"),
            ("IMAGETYP", "'Light Frame'"),
            ("EXPOSURE", "300."),
            ("CCD-TEMP", "-15.03"),
            ("SET-TEMP", "-15."),
            ("XBINNING", "1"),
            ("YBINNING", "1"),
            ("XPIXSZ", "5.4"),
            ("YPIXSZ", "5.4"),
            ("FOCALLEN", "530."),
            ("FILTER", "'Ha'"),
            ("GAIN", "139"),
            ("OFFSET", "21"),
            ("BAYERPAT", "'RGGB'"),
        ]);
        let expected = FrameCards {
            path: "maxim_light.fit".to_string(),
            exposure_s: Some(300.0),
            filter: Some("HA".to_string()),
            imagetyp: Some(("Light Frame".to_string(), "IMAGETYP")),
            frame_kind: FrameKind::Light,
            gain: Some(139.0),
            offset: Some(21.0),
            xbinning: Some(1),
            ybinning: Some(1),
            temp_c: Some(-15.03),
            cfa: true,
            cfa_pattern: Some(CfaPattern::Bayer(BayerPattern::Rggb)),
            pixel_size_um: Some((5.4, "XPIXSZ")),
            focal_mm: Some((530.0, "FOCALLEN")),
        };
        assert_eq!(frame_cards_from_header("maxim_light.fit", &maxim), expected);
    }

    #[test]
    fn nina_header_reports_xpixsz_after_binning_and_focallen() {
        let nina = header(&[
            ("SWCREATE", "'N.I.N.A. 3.1.2.9001 (x64)'"),
            ("IMAGETYP", "'LIGHT'"),
            ("EXPOSURE", "120."),
            ("EXPTIME", "120."),
            ("XBINNING", "2"),
            ("YBINNING", "2"),
            ("GAIN", "100"),
            ("OFFSET", "50"),
            ("XPIXSZ", "7.52"),
            ("YPIXSZ", "7.52"),
            ("INSTRUME", "'ZWO ASI2600MM Pro'"),
            ("SET-TEMP", "-10."),
            ("CCD-TEMP", "-10.1"),
            ("FOCALLEN", "400."),
            ("FOCRATIO", "5."),
            ("FILTER", "'L'"),
        ]);
        let cards = frame_cards_from_header("nina_light.fits", &nina);
        assert_eq!(cards.pixel_size_um, Some((7.52, "XPIXSZ")), "XPIXSZ is the binned pitch (3.76 um x 2) and is reported as written");
        assert_eq!((cards.xbinning, cards.ybinning), (Some(2), Some(2)));
        assert_eq!(cards.focal_mm, Some((400.0, "FOCALLEN")));
        assert_eq!(cards.frame_kind, FrameKind::Light);
        assert_eq!((cards.exposure_s, cards.temp_c), (Some(120.0), Some(-10.1)));
        assert_eq!((cards.gain, cards.offset), (Some(100.0), Some(50.0)));
        assert_eq!(cards.filter.as_deref(), Some("L"));
        assert!(!cards.cfa);
    }

    #[test]
    fn sharpcap_header_reports_binx_and_pixel_size() {
        let sharpcap = header(&[
            ("SWCREATE", "'SharpCap v4.1.12155.0, 64 bit'"),
            ("EXPTIME", "30."),
            ("BINX", "2"),
            ("BINY", "2"),
            ("XPIXSZ", "5.8"),
            ("YPIXSZ", "5.8"),
            ("CCD-TEMP", "-9.8"),
            ("GAIN", "200"),
            ("BAYERPAT", "'RGGB'"),
        ]);
        let cards = frame_cards_from_header("sharpcap_capture.fits", &sharpcap);
        assert_eq!((cards.xbinning, cards.ybinning), (Some(2), Some(2)));
        assert_eq!(cards.pixel_size_um, Some((5.8, "XPIXSZ")));
        assert_eq!(cards.focal_mm, None);
        assert_eq!((cards.exposure_s, cards.temp_c, cards.gain), (Some(30.0), Some(-9.8), Some(200.0)));
        assert!(cards.cfa);
        let xbinning_wins = header(&[("XBINNING", "1"), ("BINX", "2"), ("YBINNING", "1"), ("BINY", "2")]);
        let cards = frame_cards_from_header("both.fits", &xbinning_wins);
        assert_eq!((cards.xbinning, cards.ybinning), (Some(1), Some(1)));
    }

    #[test]
    fn flength_in_metres_is_converted_to_mm() {
        let focal = |cards: &[(&str, &str)]| frame_cards_from_header("focal.fits", &header(cards)).focal_mm;
        let from_metres = focal(&[("FLENGTH", "0.53")]);
        assert!(matches!(from_metres, Some((mm, "FLENGTH")) if (mm - 530.0).abs() < 1e-9), "{from_metres:?}");
        assert_eq!(focal(&[("FOCAL", "1000")]), Some((1000.0, "FOCAL")));
        assert_eq!(focal(&[("FOCALLEN", "800"), ("FLENGTH", "0.53")]), Some((800.0, "FOCALLEN")));
        let unset_focallen = focal(&[("FOCALLEN", "0"), ("FLENGTH", "0.53")]);
        assert!(matches!(unset_focallen, Some((mm, "FLENGTH")) if (mm - 530.0).abs() < 1e-9), "{unset_focallen:?}");
        assert_eq!(focal(&[("FLENGTH", "'unknown'")]), None);
    }

    #[test]
    fn ccd_temp_variants_and_frametyp_are_read() {
        for (key, value, expected) in [("CCD_TEMP", "-5.5", -5.5), ("CCDTEMP", "-6.5", -6.5), ("TEMPERAT", "-7.5", -7.5), ("CAMTCCD", "-8.5", -8.5)] {
            assert_eq!(temperature_c(&header(&[(key, value)])), Some(expected), "{key}");
            assert_eq!(temperature_c(&header(&[("SET-TEMP", "-20"), (key, value)])), Some(expected), "{key} is measured and wins over SET-TEMP");
        }
        assert_eq!(temperature_c(&header(&[("CCD-TEMP", "-1"), ("CCD_TEMP", "-2")])), Some(-1.0));
        let dark = frame_cards_from_header("dark.fits", &header(&[("FRAMETYP", "'Dark'"), ("EXPTIME", "300")]));
        assert_eq!(dark.imagetyp, Some(("Dark".to_string(), "FRAMETYP")));
        assert_eq!(dark.frame_kind, FrameKind::Dark);
        let both = frame_cards_from_header("flat.fits", &header(&[("IMAGETYP", "'Flat Field'"), ("FRAMETYP", "'Dark'")]));
        assert_eq!((both.imagetyp, both.frame_kind), (Some(("Flat Field".to_string(), "IMAGETYP")), FrameKind::Flat));
    }

    #[test]
    fn xtrans_bayerpat_sets_cfa_pattern_and_leaves_the_bayer_flag_off() {
        let quoted = format!("'{XTRANS_LAYOUT}'");
        let cards = frame_cards_from_header("xtrans.fits", &header(&[("BAYERPAT", &quoted), ("EXPTIME", "30")]));
        assert!(cards.cfa_pattern.is_some_and(|p| p.is_xtrans()), "{:?}", cards.cfa_pattern);
        assert_eq!(cards.cfa_pattern.map(|p| p.name()).as_deref(), Some(XTRANS_LAYOUT));
        assert!(!cards.cfa, "the bool still means a 2x2 Bayer lattice applies");
    }

    #[test]
    fn cfa_pattern_is_detected_with_offsets_and_row_order() {
        let bottom_up = header(&[("BAYERPAT", "'RGGB'"), ("ROWORDER", "'BOTTOM-UP'"), ("NAXIS2", "3520")]);
        let cards = frame_cards_from_header("osc.fits", &bottom_up);
        assert_eq!(cards.cfa_pattern, Some(CfaPattern::Bayer(BayerPattern::Gbrg)));
        assert!(cards.cfa);
        let offset = header(&[("BAYERPAT", "'RGGB'"), ("XBAYROFF", "1")]);
        assert_eq!(frame_cards_from_header("offset.fits", &offset).cfa_pattern, Some(CfaPattern::Bayer(BayerPattern::Grbg)));
        let mono = frame_cards_from_header("mono.fits", &header(&[("EXPTIME", "1")]));
        assert_eq!((mono.cfa, mono.cfa_pattern), (false, None));
    }

    #[test]
    fn pixel_size_aliases_are_read_in_table_order_and_blank_cards_are_skipped() {
        let pixel = |cards: &[(&str, &str)]| frame_cards_from_header("pixel.fits", &header(cards)).pixel_size_um;
        for key in ["XPIXSZ", "XPIXELSZ", "PIXSIZE1", "PIXSIZEX", "XPIXSIZE"] {
            assert_eq!(pixel(&[(key, "4.63")]), Some((4.63, key)), "{key}");
        }
        assert_eq!(pixel(&[("XPIXSIZE", "9"), ("PIXSIZE1", "3.76"), ("XPIXSZ", "5.4")]), Some((5.4, "XPIXSZ")));
        assert_eq!(pixel(&[("XPIXSZ", "0"), ("PIXSIZE1", "3.76")]), Some((3.76, "PIXSIZE1")));
        assert_eq!(pixel(&[("XPIXSZ", "'n/a'")]), None);
        let blank_imagetyp = frame_cards_from_header("blank.fits", &header(&[("IMAGETYP", "''"), ("FRAMETYP", "'Bias'")]));
        assert_eq!((blank_imagetyp.imagetyp, blank_imagetyp.frame_kind), (Some(("Bias".to_string(), "FRAMETYP")), FrameKind::Bias));
    }
}
