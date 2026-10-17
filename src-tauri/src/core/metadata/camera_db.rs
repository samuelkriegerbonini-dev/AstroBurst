use std::cmp::Reverse;

use crate::core::metadata::photcal::ARCSEC_PER_RADIAN;

const MICRONS_PER_MM: f64 = 1000.0;
const DIGIT_WILDCARD: u8 = b'#';
const ROMAN_ONE: u8 = b'I';
const TOKEN_SEPARATOR: u8 = b' ';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraCfa {
    Mono,
    Bayer,
    XTrans,
    Either,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraEntry {
    pub patterns: &'static [&'static str],
    pub sensor: &'static str,
    pub pitch_um: f64,
    pub alternate_bin1_pitch_um: Option<f64>,
    pub cfa: CameraCfa,
    pub qe_id: Option<&'static str>,
    pub unity_gain_e_per_adu: Option<f64>,
    pub read_noise_e: Option<f64>,
    pub sources: &'static [&'static str],
}

const fn variant(
    family: CameraEntry,
    patterns: &'static [&'static str],
    cfa: CameraCfa,
) -> CameraEntry {
    CameraEntry {
        patterns,
        cfa,
        ..family
    }
}

const fn pitch_from_width(width_mm: f64, width_px: f64) -> f64 {
    width_mm * MICRONS_PER_MM / width_px
}

const ZWO_ASI2600: CameraEntry = CameraEntry {
    patterns: &["ASI2600"],
    sensor: "IMX571",
    pitch_um: 3.76,
    alternate_bin1_pitch_um: None,
    cfa: CameraCfa::Either,
    qe_id: Some("Sony IMX571"),
    unity_gain_e_per_adu: None,
    read_noise_e: None,
    sources: &["https://www.zwoastro.com/product/new-asi2600mm-mc-pro/"],
};

const ZWO_ASI6200: CameraEntry = CameraEntry {
    patterns: &["ASI6200"],
    sensor: "IMX455",
    pitch_um: 3.76,
    alternate_bin1_pitch_um: None,
    cfa: CameraCfa::Either,
    qe_id: Some("Sony IMX455"),
    unity_gain_e_per_adu: None,
    read_noise_e: None,
    sources: &["https://www.zwoastro.com/product/asi6200/"],
};

const ZWO_ASI533: CameraEntry = CameraEntry {
    patterns: &["ASI533"],
    sensor: "IMX533",
    pitch_um: 3.76,
    alternate_bin1_pitch_um: None,
    cfa: CameraCfa::Either,
    qe_id: Some("Sony IMX533"),
    unity_gain_e_per_adu: None,
    read_noise_e: None,
    sources: &["https://www.zwoastro.com/product/asi533-pro-series/"],
};

const ZWO_ASI183: CameraEntry = CameraEntry {
    patterns: &["ASI183"],
    sensor: "IMX183",
    pitch_um: 2.4,
    alternate_bin1_pitch_um: None,
    cfa: CameraCfa::Either,
    qe_id: Some("Sony IMX183"),
    unity_gain_e_per_adu: None,
    read_noise_e: None,
    sources: &["https://www.zwoastro.com/product/asi183/"],
};

const ZWO_ASI585: CameraEntry = CameraEntry {
    patterns: &["ASI585"],
    sensor: "IMX585",
    pitch_um: 2.9,
    alternate_bin1_pitch_um: None,
    cfa: CameraCfa::Either,
    qe_id: Some("Sony IMX585"),
    unity_gain_e_per_adu: None,
    read_noise_e: None,
    sources: &["https://www.zwoastro.com/product/asi585mc-mm-pro/"],
};

const ZWO_ASI678: CameraEntry = CameraEntry {
    patterns: &["ASI678"],
    sensor: "IMX678",
    pitch_um: 2.0,
    alternate_bin1_pitch_um: None,
    cfa: CameraCfa::Either,
    qe_id: Some("Sony IMX678"),
    unity_gain_e_per_adu: None,
    read_noise_e: None,
    sources: &["https://www.zwoastro.com/product/zwo-asi678mc/"],
};

const ZWO_ASI1600: CameraEntry = CameraEntry {
    patterns: &["ASI1600"],
    sensor: "MN34230",
    pitch_um: 3.8,
    alternate_bin1_pitch_um: None,
    cfa: CameraCfa::Either,
    qe_id: Some("Panasonic MN34230"),
    unity_gain_e_per_adu: None,
    read_noise_e: None,
    sources: &["https://i.zwoastro.com/zwo-website/manuals/ASI1600_Manual_EN_V1.5.pdf"],
};

const QHY268: CameraEntry = CameraEntry {
    patterns: &["QHY268"],
    sensor: "IMX571",
    pitch_um: 3.76,
    alternate_bin1_pitch_um: None,
    cfa: CameraCfa::Either,
    qe_id: Some("Sony IMX571"),
    unity_gain_e_per_adu: None,
    read_noise_e: None,
    sources: &["https://www.qhyccd.com/astronomical-camera-qhy268/"],
};

const QHY600: CameraEntry = CameraEntry {
    patterns: &["QHY600"],
    sensor: "IMX455",
    pitch_um: 3.76,
    alternate_bin1_pitch_um: None,
    cfa: CameraCfa::Either,
    qe_id: Some("Sony IMX455"),
    unity_gain_e_per_adu: None,
    read_noise_e: None,
    sources: &["https://www.qhyccd.com/astronomical-camera-qhy600/"],
};

const QHY183: CameraEntry = CameraEntry {
    patterns: &["QHY183"],
    sensor: "IMX183",
    pitch_um: 2.4,
    alternate_bin1_pitch_um: None,
    cfa: CameraCfa::Either,
    qe_id: Some("Sony IMX183"),
    unity_gain_e_per_adu: None,
    read_noise_e: None,
    sources: &["https://www.qhyccd.com/astronomical-camera-qhy183/"],
};

const PLAYER_ONE_POSEIDON: CameraEntry = CameraEntry {
    patterns: &["POSEIDON"],
    sensor: "IMX571",
    pitch_um: 3.76,
    alternate_bin1_pitch_um: None,
    cfa: CameraCfa::Either,
    qe_id: Some("Sony IMX571"),
    unity_gain_e_per_adu: None,
    read_noise_e: None,
    sources: &[
        "https://player-one-astronomy.com/product/poseidon-c-pro-imx571-usb3-0-color-cooled-camera/",
        "https://player-one-astronomy.com/product/poseidon-m-pro-imx571-usb3-0-mono-cooled-camera/",
    ],
};

const PLAYER_ONE_URANUS: CameraEntry = CameraEntry {
    patterns: &["URANUS"],
    sensor: "IMX585",
    pitch_um: 2.9,
    alternate_bin1_pitch_um: None,
    cfa: CameraCfa::Either,
    qe_id: Some("Sony IMX585"),
    unity_gain_e_per_adu: None,
    read_noise_e: None,
    sources: &[
        "https://player-one-astronomy.com/product/uranus-c-usb3-0-color-camera-imx585/",
        "https://player-one-astronomy.com/product/uranus-m-pro-usb3-0-mono-cooled-camera-imx585/",
    ],
};

pub static CAMERAS: &[CameraEntry] = &[
    ZWO_ASI2600,
    variant(ZWO_ASI2600, &["ASI2600MC"], CameraCfa::Bayer),
    variant(ZWO_ASI2600, &["ASI2600MM"], CameraCfa::Mono),
    ZWO_ASI6200,
    variant(ZWO_ASI6200, &["ASI6200MC"], CameraCfa::Bayer),
    variant(ZWO_ASI6200, &["ASI6200MM"], CameraCfa::Mono),
    ZWO_ASI533,
    variant(ZWO_ASI533, &["ASI533MC"], CameraCfa::Bayer),
    variant(ZWO_ASI533, &["ASI533MM"], CameraCfa::Mono),
    ZWO_ASI183,
    variant(ZWO_ASI183, &["ASI183MC"], CameraCfa::Bayer),
    variant(ZWO_ASI183, &["ASI183MM"], CameraCfa::Mono),
    ZWO_ASI585,
    variant(ZWO_ASI585, &["ASI585MC"], CameraCfa::Bayer),
    variant(ZWO_ASI585, &["ASI585MM"], CameraCfa::Mono),
    ZWO_ASI678,
    variant(ZWO_ASI678, &["ASI678MC"], CameraCfa::Bayer),
    variant(ZWO_ASI678, &["ASI678MM"], CameraCfa::Mono),
    ZWO_ASI1600,
    variant(ZWO_ASI1600, &["ASI1600MC"], CameraCfa::Bayer),
    variant(ZWO_ASI1600, &["ASI1600MM"], CameraCfa::Mono),
    CameraEntry {
        patterns: &["ASI294MC"],
        sensor: "IMX294",
        pitch_um: 4.63,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: Some("Sony IMX294"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://www.zwoastro.com/product/asi294/"],
    },
    CameraEntry {
        patterns: &["ASI294MM"],
        sensor: "IMX492",
        pitch_um: 2.315,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Mono,
        qe_id: Some("Sony IMX492"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &[
            "https://www.zwoastro.com/product/asi294/",
            "https://www.qhyccd.com/astronomical-camera-qhy294/",
        ],
    },
    CameraEntry {
        patterns: &["ASI290MM"],
        sensor: "IMX290",
        pitch_um: 2.9,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Mono,
        qe_id: Some("Sony IMX290"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://i.zwoastro.com/zwo-website/manuals/ASI290_Manual_EN_V1.3.pdf"],
    },
    CameraEntry {
        patterns: &["ASI290MC"],
        sensor: "IMX291",
        pitch_um: 2.9,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: Some("Sony IMX291"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://i.zwoastro.com/zwo-website/manuals/ASI290_Manual_EN_V1.3.pdf"],
    },
    CameraEntry {
        patterns: &["ASI462MC"],
        sensor: "IMX462",
        pitch_um: 2.9,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: Some("Sony IMX462"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://i.zwoastro.com/zwo-website/manuals/ASI462_Manual_EN_V1.1.pdf"],
    },
    CameraEntry {
        patterns: &["ASI071MC"],
        sensor: "IMX071",
        pitch_um: 4.78,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: Some("Sony IMX071"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://i.zwoastro.com/zwo-website/manuals/ASI071MC_Pro_Manual_EN_V1.5.pdf"],
    },
    CameraEntry {
        patterns: &["ASI2400MC"],
        sensor: "IMX410",
        pitch_um: 5.94,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: Some("Sony IMX410"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://www.zwoastro.com/product/asi2400mc-pro/"],
    },
    CameraEntry {
        patterns: &["ASI224MC"],
        sensor: "IMX224",
        pitch_um: 3.75,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: Some("Sony IMX224"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://i.zwoastro.com/zwo-website/manuals/ASI224_Manual_EN_V1.4.pdf"],
    },
    QHY268,
    variant(QHY268, &["QHY268C"], CameraCfa::Bayer),
    variant(QHY268, &["QHY268M"], CameraCfa::Mono),
    QHY600,
    variant(QHY600, &["QHY600C"], CameraCfa::Bayer),
    variant(QHY600, &["QHY600M"], CameraCfa::Mono),
    QHY183,
    variant(QHY183, &["QHY183C"], CameraCfa::Bayer),
    variant(QHY183, &["QHY183M"], CameraCfa::Mono),
    CameraEntry {
        patterns: &["QHY294M"],
        sensor: "IMX492",
        pitch_um: 2.315,
        alternate_bin1_pitch_um: Some(4.63),
        cfa: CameraCfa::Mono,
        qe_id: Some("Sony IMX492"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://www.qhyccd.com/astronomical-camera-qhy294/"],
    },
    CameraEntry {
        patterns: &["QHY294C"],
        sensor: "IMX294",
        pitch_um: 4.63,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: Some("Sony IMX294"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://www.qhyccd.com/uploadfile/2019/0122/20190122041107961.pdf"],
    },
    PLAYER_ONE_POSEIDON,
    variant(PLAYER_ONE_POSEIDON, &["POSEIDONC"], CameraCfa::Bayer),
    variant(PLAYER_ONE_POSEIDON, &["POSEIDONM"], CameraCfa::Mono),
    PLAYER_ONE_URANUS,
    variant(PLAYER_ONE_URANUS, &["URANUSC"], CameraCfa::Bayer),
    variant(PLAYER_ONE_URANUS, &["URANUSM"], CameraCfa::Mono),
    CameraEntry {
        patterns: &["SEESTARS50"],
        sensor: "IMX462",
        pitch_um: 2.9,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: Some("Sony IMX462"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &[
            "https://www.seestar.com/products/seestar-s50",
            "https://i.zwoastro.com/zwo-website/manuals/ASI462_Manual_EN_V1.1.pdf",
        ],
    },
    CameraEntry {
        patterns: &["SEESTARS50PRO"],
        sensor: "OS08B10",
        pitch_um: 2.9,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: Some("OmniVision OS08B10"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://us.zwoastro.com/products/seestar-s50-pro"],
    },
    CameraEntry {
        patterns: &["SEESTARS30"],
        sensor: "IMX662",
        pitch_um: 2.9,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: Some("Sony IMX662"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &[
            "https://www.seestar.com/products/seestar-s30-all-in-one-smart-telescope",
            "https://www.zwoastro.com/product/asi662mc/",
        ],
    },
    CameraEntry {
        patterns: &["SEESTARS30PRO"],
        sensor: "IMX585",
        pitch_um: 2.9,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: Some("Sony IMX585"),
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &[
            "https://www.zwoastro.com/product/seestar-s30-pro/",
            "https://www.zwoastro.com/product/asi585mc-mm-pro/",
        ],
    },
    CameraEntry {
        patterns: &["EOSR6M2", "EOSR6MARKII", "EOSR6MKII", "EOSR6II"],
        sensor: "Canon full-frame CMOS 24.2 MP",
        pitch_um: 6.0,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: None,
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://downloads.canon.com/nw/camera/products/eos/r6-markii/R6-MarkII-SpecSheet.pdf"],
    },
    CameraEntry {
        patterns: &["EOS6DMARKII", "EOS6DMKII", "EOS6DII"],
        sensor: "Canon full-frame CMOS 26.2 MP",
        pitch_um: 5.67,
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: None,
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://downloads.canon.com/nw/camera/products/eos/6d-mark-ii/specifications/canon-eos-6d-mark-ii-specifications-chart.pdf"],
    },
    CameraEntry {
        patterns: &["NIKONZ6"],
        sensor: "Nikon FX CMOS 24.5 MP",
        pitch_um: pitch_from_width(35.9, 6048.0),
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: None,
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://imaging.nikon.com/imaging/lineup/mirrorless/z_6/"],
    },
    CameraEntry {
        patterns: &["ILCE7M3", "A7III", "A7M3"],
        sensor: "Sony 35 mm full-frame CMOS 24.2 MP",
        pitch_um: pitch_from_width(35.6, 6000.0),
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::Bayer,
        qe_id: None,
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &[
            "https://helpguide.sony.net/ilc/1720/v1/en/contents/TP0001667693.html",
            "https://helpguide.sony.net/ilc/1720/v1/en/contents/TP0001667675.html",
        ],
    },
    CameraEntry {
        patterns: &["XT5"],
        sensor: "X-Trans CMOS 5 HR",
        pitch_um: pitch_from_width(23.5, 7728.0),
        alternate_bin1_pitch_um: None,
        cfa: CameraCfa::XTrans,
        qe_id: None,
        unity_gain_e_per_adu: None,
        read_noise_e: None,
        sources: &["https://www.fujifilm-x.com/global/products/cameras/x-t5/specifications/"],
    },
];

fn normalized_model(text: &str) -> Vec<u8> {
    let mut model = Vec::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() {
            model.push(byte.to_ascii_uppercase());
        } else if model.last().is_some_and(|&last| last != TOKEN_SEPARATOR) {
            model.push(TOKEN_SEPARATOR);
        }
    }
    if model.last() == Some(&TOKEN_SEPARATOR) {
        model.pop();
    }
    model
}

fn is_numeric_pattern_byte(byte: u8) -> bool {
    byte == DIGIT_WILDCARD || byte.is_ascii_digit()
}

fn pattern_byte_matches(pattern_byte: u8, model_byte: u8) -> bool {
    if pattern_byte == DIGIT_WILDCARD {
        model_byte.is_ascii_digit()
    } else {
        pattern_byte == model_byte
    }
}

fn is_roman_one_token(token: &[u8]) -> bool {
    !token.is_empty() && token.iter().all(|&byte| byte == ROMAN_ONE)
}

fn number_continues_before(first_pattern_byte: u8, model_before: &[u8]) -> bool {
    let previous = model_before
        .iter()
        .rev()
        .find(|&&byte| byte != TOKEN_SEPARATOR);
    is_numeric_pattern_byte(first_pattern_byte) && previous.is_some_and(u8::is_ascii_digit)
}

fn number_continues_after(last_pattern_byte: u8, model_after: &[u8]) -> bool {
    let numeric_end = is_numeric_pattern_byte(last_pattern_byte);
    let roman_end = numeric_end || last_pattern_byte == ROMAN_ONE;
    match model_after.split_first() {
        Some((&TOKEN_SEPARATOR, after_separator)) => {
            let next_token = after_separator
                .split(|&byte| byte == TOKEN_SEPARATOR)
                .next()
                .unwrap_or_default();
            (numeric_end && next_token.first().is_some_and(u8::is_ascii_digit))
                || (roman_end && is_roman_one_token(next_token))
        }
        Some((&next_byte, _)) => {
            (numeric_end && next_byte.is_ascii_digit()) || (roman_end && next_byte == ROMAN_ONE)
        }
        None => false,
    }
}

fn match_end(model: &[u8], start: usize, pattern: &[u8]) -> Option<usize> {
    let mut position = start;
    for (index, &pattern_byte) in pattern.iter().enumerate() {
        if index > 0 && model.get(position) == Some(&TOKEN_SEPARATOR) {
            position += 1;
        }
        let model_byte = *model.get(position)?;
        if !pattern_byte_matches(pattern_byte, model_byte) {
            return None;
        }
        position += 1;
    }
    Some(position)
}

fn pattern_found_in(model: &[u8], pattern: &str) -> bool {
    let pattern = pattern.as_bytes();
    let (Some(&first), Some(&last)) = (pattern.first(), pattern.last()) else {
        return false;
    };
    (0..model.len()).any(|start| {
        match_end(model, start, pattern).is_some_and(|end| {
            !number_continues_before(first, model.get(..start).unwrap_or_default())
                && !number_continues_after(last, model.get(end..).unwrap_or_default())
        })
    })
}

fn longest_match<T: Copy>(
    model: &[u8],
    rows: impl IntoIterator<Item = (&'static str, T)>,
) -> Option<T> {
    rows.into_iter()
        .filter(|(pattern, _)| pattern_found_in(model, pattern))
        .min_by_key(|(pattern, _)| Reverse(pattern.len()))
        .map(|(_, value)| value)
}

pub fn lookup(instrume: &str) -> Option<&'static CameraEntry> {
    let model = normalized_model(instrume);
    longest_match(
        &model,
        CAMERAS
            .iter()
            .flat_map(|entry| entry.patterns.iter().map(move |pattern| (*pattern, entry))),
    )
}

pub fn scale_arcsec_per_px(pitch_um: f64, focal_mm: f64) -> Option<f64> {
    let positive = |value: f64| value.is_finite() && value > 0.0;
    (positive(pitch_um) && positive(focal_mm))
        .then(|| ARCSEC_PER_RADIAN * pitch_um / (focal_mm * MICRONS_PER_MM))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatClass {
    ApsC,
    FullFrame,
    FourThirds,
    ApsH,
    OneInch,
    MediumFormat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FormatEstimate {
    pub class: FormatClass,
    pub sensor_width_mm: f64,
    pub pitch_um: f64,
    pub window: (f64, f64),
    pub label: &'static str,
}

pub const FORMAT_ESTIMATE_LABEL: &str = "format class estimate";
pub const FORMAT_ESTIMATE_TOLERANCE: f64 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq)]
struct SensorFormat {
    class: FormatClass,
    width_mm: f64,
}

const APS_C: SensorFormat = SensorFormat {
    class: FormatClass::ApsC,
    width_mm: 23.6,
};
const CANON_APS_C: SensorFormat = SensorFormat {
    class: FormatClass::ApsC,
    width_mm: 22.3,
};
const FULL_FRAME: SensorFormat = SensorFormat {
    class: FormatClass::FullFrame,
    width_mm: 36.0,
};
const SONY_FULL_FRAME: SensorFormat = SensorFormat {
    class: FormatClass::FullFrame,
    width_mm: 35.6,
};
const FOUR_THIRDS: SensorFormat = SensorFormat {
    class: FormatClass::FourThirds,
    width_mm: 17.3,
};
const APS_H: SensorFormat = SensorFormat {
    class: FormatClass::ApsH,
    width_mm: 28.7,
};
const ONE_INCH: SensorFormat = SensorFormat {
    class: FormatClass::OneInch,
    width_mm: 13.2,
};
const MEDIUM_FORMAT: SensorFormat = SensorFormat {
    class: FormatClass::MediumFormat,
    width_mm: 44.0,
};

const FORMAT_RULES: &[(&str, SensorFormat)] = &[
    ("APSC", APS_C),
    ("APSH", APS_H),
    ("FULLFRAME", FULL_FRAME),
    ("FOURTHIRDS", FOUR_THIRDS),
    ("MEDIUMFORMAT", MEDIUM_FORMAT),
    ("1INCH", ONE_INCH),
    ("EOS5D", FULL_FRAME),
    ("EOS6D", FULL_FRAME),
    ("EOS1DS", FULL_FRAME),
    ("EOS1DX", FULL_FRAME),
    ("EOS1DC", FULL_FRAME),
    ("EOSR", FULL_FRAME),
    ("EOS1D", APS_H),
    ("EOS7D", CANON_APS_C),
    ("EOS##D", CANON_APS_C),
    ("EOS###D", CANON_APS_C),
    ("EOS####D", CANON_APS_C),
    ("EOSR7", CANON_APS_C),
    ("EOSR10", CANON_APS_C),
    ("EOSR50", CANON_APS_C),
    ("EOSR100", CANON_APS_C),
    ("EOSREBEL", CANON_APS_C),
    ("EOSKISS", CANON_APS_C),
    ("EOSM", CANON_APS_C),
    ("NIKONZ5", FULL_FRAME),
    ("NIKONZ6", FULL_FRAME),
    ("NIKONZ7", FULL_FRAME),
    ("NIKONZ8", FULL_FRAME),
    ("NIKONZ9", FULL_FRAME),
    ("NIKONZF", FULL_FRAME),
    ("NIKONZFC", APS_C),
    ("NIKONZ50", APS_C),
    ("NIKONZ30", APS_C),
    ("NIKOND3", FULL_FRAME),
    ("NIKOND4", FULL_FRAME),
    ("NIKOND5", FULL_FRAME),
    ("NIKOND6", FULL_FRAME),
    ("NIKOND6##", FULL_FRAME),
    ("NIKOND7##", FULL_FRAME),
    ("NIKOND8##", FULL_FRAME),
    ("NIKONDF", FULL_FRAME),
    ("NIKOND##", APS_C),
    ("NIKOND1##", APS_C),
    ("NIKOND2##", APS_C),
    ("NIKOND3##", APS_C),
    ("NIKOND500", APS_C),
    ("NIKOND3###", APS_C),
    ("NIKOND5###", APS_C),
    ("NIKOND7###", APS_C),
    ("ILCE7", SONY_FULL_FRAME),
    ("ILCE9", SONY_FULL_FRAME),
    ("ILCE1", SONY_FULL_FRAME),
    ("ILCE6###", APS_C),
    ("DSCRX1", SONY_FULL_FRAME),
    ("DSCRX10", ONE_INCH),
    ("DSCRX100", ONE_INCH),
    ("GFX", MEDIUM_FORMAT),
    ("FUJIFILMXT", APS_C),
    ("FUJIFILMXS##", APS_C),
    ("FUJIFILMXE", APS_C),
    ("FUJIFILMXH#", APS_C),
    ("FUJIFILMXPRO", APS_C),
    ("FUJIFILMX100", APS_C),
    ("PANASONICDCG", FOUR_THIRDS),
    ("PANASONICDMCG", FOUR_THIRDS),
];

pub fn format_class(instrume: &str, width_px: usize, height_px: usize) -> Option<FormatEstimate> {
    let long_side_px = width_px.max(height_px);
    if long_side_px == 0 {
        return None;
    }
    let format = longest_match(&normalized_model(instrume), FORMAT_RULES.iter().copied())?;
    let pitch_um = format.width_mm * MICRONS_PER_MM / long_side_px as f64;
    Some(FormatEstimate {
        class: format.class,
        sensor_width_mm: format.width_mm,
        pitch_um,
        window: (
            pitch_um * (1.0 - FORMAT_ESTIMATE_TOLERANCE),
            pitch_um * (1.0 + FORMAT_ESTIMATE_TOLERANCE),
        ),
        label: FORMAT_ESTIMATE_LABEL,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(instrume: &str) -> (&'static str, f64, CameraCfa) {
        let entry = lookup(instrume).unwrap_or_else(|| panic!("{instrume} did not resolve"));
        (entry.sensor, entry.pitch_um, entry.cfa)
    }

    #[test]
    fn lookup_matches_vendor_name_variants() {
        assert_eq!(
            resolved("ZWO ASI2600MC Pro"),
            ("IMX571", 3.76, CameraCfa::Bayer)
        );
        assert_eq!(resolved("ASI2600MC-P"), ("IMX571", 3.76, CameraCfa::Bayer));
        assert_eq!(
            resolved("ZWO ASI 2600 MM"),
            ("IMX571", 3.76, CameraCfa::Mono)
        );
        assert_eq!(resolved("zwo asi2600"), ("IMX571", 3.76, CameraCfa::Either));
        assert_eq!(
            resolved("ZWO ASI1600MM-Cool"),
            ("MN34230", 3.8, CameraCfa::Mono)
        );
        assert_eq!(
            resolved("ZWO ASI294MM Pro"),
            ("IMX492", 2.315, CameraCfa::Mono)
        );
        assert_eq!(
            resolved("ZWO ASI294MC Pro"),
            ("IMX294", 4.63, CameraCfa::Bayer)
        );
        assert_eq!(
            resolved("QHY268C-1a2b3c4d5e6f7a8b"),
            ("IMX571", 3.76, CameraCfa::Bayer)
        );
        assert_eq!(resolved("QHY600PH"), ("IMX455", 3.76, CameraCfa::Either));
        assert_eq!(
            resolved("Poseidon-M PRO"),
            ("IMX571", 3.76, CameraCfa::Mono)
        );
        assert_eq!(resolved("Uranus-C"), ("IMX585", 2.9, CameraCfa::Bayer));
        assert_eq!(resolved("Seestar S50"), ("IMX462", 2.9, CameraCfa::Bayer));
        assert_eq!(
            resolved("Seestar S50 Pro"),
            ("OS08B10", 2.9, CameraCfa::Bayer)
        );
        assert_eq!(resolved("Seestar S30"), ("IMX662", 2.9, CameraCfa::Bayer));
        assert_eq!(
            resolved("Seestar S30 Pro"),
            ("IMX585", 2.9, CameraCfa::Bayer)
        );
        assert_eq!(resolved("Canon EOS R6m2").1, 6.0);
        assert_eq!(resolved("Canon EOS R6 Mark II").1, 6.0);
        assert_eq!(resolved("Canon EOS 6D Mark II").1, 5.67);
        assert_eq!(resolved("ILCE-7M3").2, CameraCfa::Bayer);
        assert_eq!(resolved("Sony A7 III").0, resolved("ILCE-7M3").0);
        assert_eq!(resolved("NIKON Z 6").0, "Nikon FX CMOS 24.5 MP");
        assert_eq!(resolved("FUJIFILM X-T5").2, CameraCfa::XTrans);
    }

    #[test]
    fn unknown_model_returns_none() {
        for instrume in [
            "",
            "   ",
            "Canon EOS 90D",
            "Canon EOS R6",
            "Canon EOS R6 Mark III",
            "Canon EOS 6D",
            "NIKON Z 6_2",
            "Nikon Z6II",
            "FUJIFILM X-T50",
            "ZWO ASI26000MC",
            "QHYCCD-Cameras-Capture",
            "Atik 460EX",
        ] {
            assert_eq!(lookup(instrume), None, "{instrume:?}");
        }
    }

    #[test]
    fn a_separate_letter_token_ends_the_model_number() {
        assert_eq!(
            resolved("ZWO ASI2600 IMX571"),
            ("IMX571", 3.76, CameraCfa::Either)
        );
        assert_eq!(
            resolved("ZWO ASI294MM IMX492"),
            ("IMX492", 2.315, CameraCfa::Mono)
        );
        assert_eq!(resolved("X-T5 IR").2, CameraCfa::XTrans);
        assert_eq!(resolved("FUJIFILM X-T5 (IR modified)").2, CameraCfa::XTrans);
        for instrume in [
            "Nikon Z6 II",
            "Nikon Z 6 2",
            "NIKON Z 6_2",
            "FUJIFILM X-T5 II",
            "Canon EOS R6 Mark II I",
            "ZWO ASI2600 2",
        ] {
            assert_eq!(lookup(instrume), None, "{instrume:?}");
        }
    }

    #[test]
    fn qhy294m_offers_its_locked_bin2_pitch_at_bin1() {
        let qhy = lookup("QHY294M Pro").expect("QHY294M resolves");
        assert_eq!(
            (qhy.pitch_um, qhy.alternate_bin1_pitch_um),
            (2.315, Some(4.63))
        );
        let zwo = lookup("ZWO ASI294MM Pro").expect("ASI294MM resolves");
        assert_eq!((zwo.pitch_um, zwo.alternate_bin1_pitch_um), (2.315, None));
        for entry in CAMERAS {
            if let Some(alternate) = entry.alternate_bin1_pitch_um {
                assert!(
                    alternate.is_finite() && alternate > 0.0 && alternate != entry.pitch_um,
                    "{entry:?}"
                );
            }
        }
    }

    #[test]
    fn rows_cite_the_page_that_states_the_stored_pitch() {
        for (instrume, pitch_page) in [
            (
                "Seestar S50",
                "https://i.zwoastro.com/zwo-website/manuals/ASI462_Manual_EN_V1.1.pdf",
            ),
            ("Seestar S30", "https://www.zwoastro.com/product/asi662mc/"),
            (
                "Seestar S30 Pro",
                "https://www.zwoastro.com/product/asi585mc-mm-pro/",
            ),
            (
                "ZWO ASI294MM Pro",
                "https://www.qhyccd.com/astronomical-camera-qhy294/",
            ),
            (
                "Sony A7 III",
                "https://helpguide.sony.net/ilc/1720/v1/en/contents/TP0001667675.html",
            ),
            (
                "Poseidon-M Pro",
                "https://player-one-astronomy.com/product/poseidon-m-pro-imx571-usb3-0-mono-cooled-camera/",
            ),
            (
                "Uranus-M",
                "https://player-one-astronomy.com/product/uranus-m-pro-usb3-0-mono-cooled-camera-imx585/",
            ),
        ] {
            let entry = lookup(instrume).unwrap_or_else(|| panic!("{instrume} did not resolve"));
            assert!(
                entry.sources.contains(&pitch_page),
                "{instrume}: {:?}",
                entry.sources
            );
        }
    }

    #[test]
    fn scale_from_pitch_and_focal_matches_the_formula() {
        let arcsec_per_um_per_mm = ARCSEC_PER_RADIAN / 1000.0;
        for (pitch_um, focal_mm) in [(3.76, 530.0), (2.9, 250.0), (5.94, 135.0), (2.315, 2000.0)] {
            let scale = scale_arcsec_per_px(pitch_um, focal_mm).expect("valid optics");
            assert!((scale - arcsec_per_um_per_mm * pitch_um / focal_mm).abs() < 1e-9);
            assert!((scale / (206.265 * pitch_um / focal_mm) - 1.0).abs() < 1e-6);
        }
        assert!((scale_arcsec_per_px(3.76, 530.0).unwrap() - 1.463_312_587_7).abs() < 1e-9);
        for (pitch_um, focal_mm) in [
            (0.0, 500.0),
            (3.76, 0.0),
            (-3.76, 500.0),
            (3.76, -500.0),
            (f64::NAN, 500.0),
            (3.76, f64::INFINITY),
        ] {
            assert_eq!(scale_arcsec_per_px(pitch_um, focal_mm), None);
        }
    }

    #[test]
    fn format_class_estimate_is_labelled_and_windowed() {
        let estimate = format_class("Canon EOS 90D", 6960, 4640).expect("Canon APS-C body");
        assert_eq!(estimate.class, FormatClass::ApsC);
        assert_eq!(estimate.sensor_width_mm, 22.3);
        assert!((estimate.pitch_um - 22.3e3 / 6960.0).abs() < 1e-12);
        assert_eq!(estimate.label, "format class estimate");
        assert!((estimate.window.0 - estimate.pitch_um * 0.85).abs() < 1e-12);
        assert!((estimate.window.1 - estimate.pitch_um * 1.15).abs() < 1e-12);

        for (instrume, width_px, class, width_mm) in [
            ("Canon EOS Rebel T7i", 6000, FormatClass::ApsC, 22.3),
            ("Canon EOS R7", 6960, FormatClass::ApsC, 22.3),
            ("Canon EOS R100", 6000, FormatClass::ApsC, 22.3),
            ("Canon EOS 5D Mark IV", 6720, FormatClass::FullFrame, 36.0),
            ("Canon EOS R6", 5472, FormatClass::FullFrame, 36.0),
            ("Canon EOS-1D Mark IV", 4896, FormatClass::ApsH, 28.7),
            (
                "Canon EOS-1D X Mark III",
                5472,
                FormatClass::FullFrame,
                36.0,
            ),
            ("NIKON D850", 8256, FormatClass::FullFrame, 36.0),
            ("NIKON D7500", 5568, FormatClass::ApsC, 23.6),
            ("NIKON D500", 5568, FormatClass::ApsC, 23.6),
            ("NIKON Z 50", 5568, FormatClass::ApsC, 23.6),
            ("NIKON Z 7", 8256, FormatClass::FullFrame, 36.0),
            ("ILCE-7RM4", 9504, FormatClass::FullFrame, 35.6),
            ("ILCE-6400", 6000, FormatClass::ApsC, 23.6),
            ("DSC-RX100M7", 5472, FormatClass::OneInch, 13.2),
            ("Panasonic DC-GH5", 5184, FormatClass::FourThirds, 17.3),
            ("FUJIFILM GFX100S", 11648, FormatClass::MediumFormat, 44.0),
            ("FUJIFILM X-S20", 6240, FormatClass::ApsC, 23.6),
            ("FUJIFILM X-H2S", 6240, FormatClass::ApsC, 23.6),
            ("FUJIFILM X-H1", 6000, FormatClass::ApsC, 23.6),
            ("Generic APS-C camera", 6000, FormatClass::ApsC, 23.6),
            (
                "Micro Four Thirds body",
                5184,
                FormatClass::FourThirds,
                17.3,
            ),
            ("1 inch sensor camera", 5472, FormatClass::OneInch, 13.2),
        ] {
            let estimate = format_class(instrume, width_px, width_px * 2 / 3)
                .unwrap_or_else(|| panic!("{instrume} has a known format class"));
            assert_eq!(
                (estimate.class, estimate.sensor_width_mm),
                (class, width_mm),
                "{instrume}"
            );
            assert!((estimate.pitch_um - width_mm * 1000.0 / width_px as f64).abs() < 1e-12);
            assert_eq!(estimate.label, FORMAT_ESTIMATE_LABEL);
        }

        for (instrume, width_px) in [
            ("Canon EOS 90D", 0),
            ("ZWO ASI2600MC Pro", 6248),
            ("FUJIFILM X-S1", 3936),
            ("FUJIFILM X-HF1", 3864),
            ("NIKON Z 6_2", 6048),
            ("Celestron NexImage 5", 2592),
            ("", 6000),
        ] {
            assert_eq!(
                format_class(instrume, width_px, width_px),
                None,
                "{instrume:?}"
            );
        }
    }

    #[test]
    fn format_class_uses_the_long_side_of_a_portrait_frame() {
        let landscape = format_class("Canon EOS 90D", 6960, 4640).expect("landscape frame");
        let portrait = format_class("Canon EOS 90D", 4640, 6960).expect("portrait frame");
        assert_eq!(portrait, landscape);
        assert!((portrait.pitch_um - 22.3e3 / 6960.0).abs() < 1e-12);
        assert!(portrait.window.0 < 3.204 && 3.204 < portrait.window.1);
        assert_eq!(format_class("Canon EOS 90D", 6960, 0), Some(landscape));
        assert_eq!(format_class("Canon EOS 90D", 0, 6960), Some(landscape));
    }

    #[test]
    fn every_format_rule_is_normalised_and_resolves_to_itself() {
        assert!(FORMAT_RULES.len() >= 40);
        for (pattern, format) in FORMAT_RULES {
            assert!(
                !pattern.is_empty()
                    && pattern
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'#'),
                "pattern {pattern:?} is not in normalised form"
            );
            assert!(format.width_mm.is_finite() && format.width_mm > 0.0);
            let sample = pattern.replace('#', "0");
            let estimate = format_class(&sample, 6000, 4000).expect("rule resolves");
            assert_eq!(
                (estimate.class, estimate.sensor_width_mm),
                (format.class, format.width_mm),
                "pattern {pattern:?} resolves to another rule"
            );
        }
    }

    #[test]
    fn every_entry_has_a_source_url_and_a_positive_pitch() {
        assert!(CAMERAS.len() >= 25);
        for entry in CAMERAS {
            assert!(
                !entry.sources.is_empty()
                    && entry.sources.iter().all(|url| url.starts_with("https://")),
                "{entry:?}"
            );
            assert!(
                entry.pitch_um.is_finite() && entry.pitch_um > 0.0,
                "{entry:?}"
            );
            assert!(!entry.sensor.is_empty(), "{entry:?}");
            assert!(!entry.patterns.is_empty(), "{entry:?}");
            for pattern in entry.patterns {
                assert!(
                    !pattern.is_empty()
                        && pattern
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()),
                    "pattern {pattern:?} is not in normalised form"
                );
                assert!(
                    std::ptr::eq(lookup(pattern).expect("pattern resolves"), entry),
                    "pattern {pattern:?} resolves to another entry"
                );
            }
        }
    }
}
