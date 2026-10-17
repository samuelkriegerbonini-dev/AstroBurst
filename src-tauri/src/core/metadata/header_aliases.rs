use crate::types::header::HduHeader;

pub const EXPOSURE_KEYS: [&str; 3] = ["EXPTIME", "XPOSURE", "EFFEXPTM"];
pub const EXPOSURE_PRESENCE_KEYS: [&str; 3] = ["XPOSURE", "EFFEXPTM", "DURATION"];
pub const TEMPERATURE_KEYS: [&str; 6] = [
    "CCD-TEMP", "CCD_TEMP", "CCDTEMP", "TEMPERAT", "CAMTCCD", "SET-TEMP",
];
pub const PIXEL_SIZE_UM_KEYS: [&str; 5] =
    ["XPIXSZ", "XPIXELSZ", "PIXSIZE1", "PIXSIZEX", "XPIXSIZE"];
pub const Y_PIXEL_SIZE_UM_KEYS: [&str; 5] =
    ["YPIXSZ", "YPIXELSZ", "PIXSIZE2", "PIXSIZEY", "YPIXSIZE"];
pub const FOCAL_MM_KEYS: [&str; 3] = ["FOCALLEN", "FOCAL", "FLENGTH"];
pub const BINNING_KEYS: [&str; 2] = ["XBINNING", "BINX"];
pub const Y_BINNING_KEYS: [&str; 2] = ["YBINNING", "BINY"];
pub const IMAGETYP_KEYS: [&str; 2] = ["IMAGETYP", "FRAMETYP"];
pub const OFFSET_KEYS: [&str; 2] = ["OFFSET", "BLKLEVEL"];
pub const STACK_COUNT_KEYS: [&str; 2] = ["NCOMBINE", "STACKCNT"];
pub const ALTITUDE_KEYS: [&str; 2] = ["CENTALT", "OBJCTALT"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SexagesimalRaUnit {
    Hours,
    Degrees,
}

pub const POINTING_DEGREE_PAIRS: [(&str, &str, &str, SexagesimalRaUnit); 6] = [
    (
        "RA_TARG",
        "DEC_TARG",
        "RA_TARG/DEC_TARG",
        SexagesimalRaUnit::Hours,
    ),
    (
        "TARG_RA",
        "TARG_DEC",
        "TARG_RA/TARG_DEC",
        SexagesimalRaUnit::Hours,
    ),
    ("RA_D", "DEC_D", "RA_D/DEC_D", SexagesimalRaUnit::Degrees),
    ("RA", "DEC", "RA/DEC", SexagesimalRaUnit::Hours),
    ("OBJRA", "OBJDEC", "OBJRA/OBJDEC", SexagesimalRaUnit::Hours),
    (
        "RA_OBJ",
        "DEC_OBJ",
        "RA_OBJ/DEC_OBJ",
        SexagesimalRaUnit::Hours,
    ),
];

const FOCAL_LENGTH_IN_METRES_KEY: &str = "FLENGTH";
const MILLIMETRES_PER_METRE: f64 = 1000.0;

pub(crate) fn first_f64(header: &HduHeader, keys: &[&'static str]) -> Option<(f64, &'static str)> {
    first_f64_where(header, keys, |_| true)
}

fn first_positive_f64(header: &HduHeader, keys: &[&'static str]) -> Option<(f64, &'static str)> {
    first_f64_where(header, keys, |v| v > 0.0)
}

fn first_f64_where(
    header: &HduHeader,
    keys: &[&'static str],
    accept: impl Fn(f64) -> bool,
) -> Option<(f64, &'static str)> {
    keys.iter().find_map(|&key| {
        header
            .get_f64(key)
            .filter(|v| v.is_finite() && accept(*v))
            .map(|v| (v, key))
    })
}

pub(crate) fn first_i64(header: &HduHeader, keys: &[&'static str]) -> Option<(i64, &'static str)> {
    keys.iter()
        .find_map(|&key| integer_card(header, key).map(|v| (v, key)))
}

fn integer_card(header: &HduHeader, key: &str) -> Option<i64> {
    header.get_i64(key).or_else(|| {
        header
            .get_f64(key)
            .filter(|v| v.is_finite())
            .map(|v| v.round() as i64)
    })
}

pub(crate) fn first_text(
    header: &HduHeader,
    keys: &[&'static str],
) -> Option<(String, &'static str)> {
    keys.iter()
        .find_map(|&key| card_text(header, key).map(|text| (text, key)))
}

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    let text = header.get(key)?.trim().trim_matches('\'').trim();
    (!text.is_empty()).then(|| text.to_string())
}

pub(crate) fn keyword_alternatives(keys: &[&str]) -> String {
    match keys.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {}", rest.join(", "), last),
        Some((last, _)) => last.to_string(),
        None => String::new(),
    }
}

pub fn pixel_size_um(header: &HduHeader) -> Option<(f64, &'static str)> {
    first_positive_f64(header, &PIXEL_SIZE_UM_KEYS)
}

pub fn focal_length_mm(header: &HduHeader) -> Option<(f64, &'static str)> {
    FOCAL_MM_KEYS.iter().find_map(|&key| {
        header
            .get_f64(key)
            .map(|value| {
                if key == FOCAL_LENGTH_IN_METRES_KEY {
                    value * MILLIMETRES_PER_METRE
                } else {
                    value
                }
            })
            .filter(|millimetres| millimetres.is_finite() && *millimetres > 0.0)
            .map(|millimetres| (millimetres, key))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (key, value) in cards {
            h.set(key, (*value).to_string());
        }
        h
    }

    #[test]
    fn first_f64_returns_the_first_finite_value_with_its_key() {
        let h = header(&[("A", "NaN"), ("B", "'text'"), ("C", "-2.5"), ("D", "7")]);
        assert_eq!(first_f64(&h, &["A", "B", "C", "D"]), Some((-2.5, "C")));
        assert_eq!(first_positive_f64(&h, &["C", "D"]), Some((7.0, "D")));
        assert_eq!(first_f64(&h, &["MISSING"]), None);
    }

    #[test]
    fn first_i64_rounds_float_cards_and_first_text_skips_blank_cards() {
        let h = header(&[
            ("XBINNING", "'x'"),
            ("BINX", "2.0"),
            ("IMAGETYP", "'  '"),
            ("FRAMETYP", "' Light '"),
        ]);
        assert_eq!(first_i64(&h, &BINNING_KEYS), Some((2, "BINX")));
        assert_eq!(
            first_text(&h, &IMAGETYP_KEYS),
            Some(("Light".to_string(), "FRAMETYP"))
        );
    }

    #[test]
    fn focal_length_stays_finite_after_the_metres_conversion() {
        assert_eq!(focal_length_mm(&header(&[("FLENGTH", "1E306")])), None);
        assert_eq!(
            focal_length_mm(&header(&[("FOCALLEN", "0"), ("FLENGTH", "1E306")])),
            None
        );
    }

    #[test]
    fn keyword_alternatives_join_with_commas_and_a_final_or() {
        assert_eq!(
            keyword_alternatives(&TEMPERATURE_KEYS),
            "CCD-TEMP, CCD_TEMP, CCDTEMP, TEMPERAT, CAMTCCD or SET-TEMP"
        );
        assert_eq!(
            keyword_alternatives(&["XBINNING", "BINX"]),
            "XBINNING or BINX"
        );
        assert_eq!(keyword_alternatives(&["FOCALLEN"]), "FOCALLEN");
        assert_eq!(keyword_alternatives(&[]), "");
    }

    #[test]
    fn temperature_table_keeps_the_measured_card_first_and_the_set_point_last() {
        assert_eq!(TEMPERATURE_KEYS.first(), Some(&"CCD-TEMP"));
        assert_eq!(TEMPERATURE_KEYS.last(), Some(&"SET-TEMP"));
    }
}
