use serde_yaml::Value;

use crate::core::astrometry::frames::{
    cartesian_to_spherical, mat_apply, mat_mul, mat_transpose, rot_x, rot_z, spherical_to_cartesian, Mat3,
};
use crate::core::astrometry::time::{jd_from_mjd, julian_centuries_j2000, parse_fits_datetime, JD_J2000};
use crate::infra::asdf::tree::untag;

pub const FK5_REFERENCE: &str = "fk5";

const ARCSEC_PER_DEG: f64 = 3600.0;
const J2000_YEAR: f64 = 2000.0;
const DAYS_PER_JULIAN_YEAR: f64 = 365.25;
const B1900_JD: f64 = 2415020.31352;
const B1900_YEAR: f64 = 1900.0;
const DAYS_PER_TROPICAL_YEAR_B1900: f64 = 365.242198781;
const J2000_TOLERANCE_DAYS: f64 = 1e-3;
const FRAME_TIE_NOTE: &str = "22 mas FK5-ICRS tie not applied";

const IAU2006_GAMMA_BAR_ARCSEC: [f64; 6] = [-0.052928, 10.556378, 0.4932044, -0.00031238, -0.000002788, 0.0000000260];
const IAU2006_PHI_BAR_ARCSEC: [f64; 6] = [84381.412819, -46.811016, 0.0511268, 0.00053289, -0.000000440, -0.0000000176];
const IAU2006_PSI_BAR_ARCSEC: [f64; 6] = [-0.041775, 5038.481484, 1.5584175, -0.00018522, -0.000026452, -0.0000000148];
const IAU2006_OBLIQUITY_ARCSEC: [f64; 6] = [84381.406, -46.836769, -0.0001831, 0.00200340, -0.000000576, -0.0000000434];

#[derive(Debug, Clone, PartialEq)]
pub struct Fk5Frame {
    pub equinox_label: String,
    pub jd_tt: f64,
}

fn polynomial_deg(coefficients_arcsec: &[f64; 6], t: f64) -> f64 {
    coefficients_arcsec.iter().rev().fold(0.0, |acc, c| acc * t + c) / ARCSEC_PER_DEG
}

fn bias_precession_matrix(jd_tt: f64) -> Mat3 {
    let t = julian_centuries_j2000(jd_tt);
    let gamma_bar = polynomial_deg(&IAU2006_GAMMA_BAR_ARCSEC, t);
    let phi_bar = polynomial_deg(&IAU2006_PHI_BAR_ARCSEC, t);
    let psi_bar = polynomial_deg(&IAU2006_PSI_BAR_ARCSEC, t);
    let obliquity = polynomial_deg(&IAU2006_OBLIQUITY_ARCSEC, t);
    mat_mul(
        &mat_mul(&rot_x(-obliquity), &rot_z(-psi_bar)),
        &mat_mul(&rot_x(phi_bar), &rot_z(gamma_bar)),
    )
}

pub fn precession_to_j2000(jd_tt: f64) -> Mat3 {
    mat_mul(&bias_precession_matrix(JD_J2000), &mat_transpose(&bias_precession_matrix(jd_tt)))
}

pub fn rotate_sky(matrix: &Mat3, lon_deg: f64, lat_deg: f64) -> [f64; 2] {
    if !(lon_deg.is_finite() && lat_deg.is_finite()) {
        return [lon_deg, lat_deg];
    }
    let (lon, lat) = cartesian_to_spherical(mat_apply(matrix, spherical_to_cartesian(lon_deg, lat_deg)));
    [lon, lat]
}

fn julian_year_jd(year: f64) -> f64 {
    JD_J2000 + (year - J2000_YEAR) * DAYS_PER_JULIAN_YEAR
}

fn besselian_year_jd(year: f64) -> f64 {
    B1900_JD + (year - B1900_YEAR) * DAYS_PER_TROPICAL_YEAR_B1900
}

fn year_text(year: f64) -> String {
    let fixed = format!("{year:.3}");
    let trimmed = fixed.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{trimmed}0")
    } else {
        trimmed.to_string()
    }
}

fn julian_label(jd_tt: f64) -> String {
    format!("J{}", year_text(J2000_YEAR + (jd_tt - JD_J2000) / DAYS_PER_JULIAN_YEAR))
}

fn finite_year(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|y| y.is_finite())
}

fn from_text(text: &str) -> Option<Fk5Frame> {
    let text = text.trim();
    let julian = |year: f64| {
        let jd_tt = julian_year_jd(year);
        Fk5Frame { equinox_label: julian_label(jd_tt), jd_tt }
    };
    if let Some(year) = text.strip_prefix(['J', 'j']).and_then(finite_year) {
        return Some(julian(year));
    }
    if let Some(year) = text.strip_prefix(['B', 'b']).and_then(finite_year) {
        return Some(Fk5Frame { equinox_label: format!("B{}", year_text(year)), jd_tt: besselian_year_jd(year) });
    }
    if let Some(year) = finite_year(text) {
        return Some(julian(year));
    }
    let jd_tt = parse_fits_datetime(text).ok().filter(|jd| jd.is_finite())?;
    Some(Fk5Frame { equinox_label: julian_label(jd_tt), jd_tt })
}

fn from_number(value: f64, format: &str) -> Option<Fk5Frame> {
    if !value.is_finite() {
        return None;
    }
    let jd_tt = match format {
        "" | "jyear" => julian_year_jd(value),
        "byear" => return Some(Fk5Frame { equinox_label: format!("B{}", year_text(value)), jd_tt: besselian_year_jd(value) }),
        "jd" => value,
        "mjd" => jd_from_mjd(value),
        _ => return None,
    };
    Some(Fk5Frame { equinox_label: julian_label(jd_tt), jd_tt })
}

fn scalar_text(node: &Value) -> String {
    match untag(node) {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        _ => "a nested value".to_string(),
    }
}

fn from_equinox_node(node: &Value) -> Result<Fk5Frame, String> {
    let read = match untag(node) {
        Value::String(text) => from_text(text),
        Value::Number(n) => n.as_f64().and_then(|v| from_number(v, "")),
        Value::Mapping(_) => {
            let format = node.get("format").map(scalar_text).unwrap_or_default().to_ascii_lowercase();
            match node.get("value").map(untag) {
                Some(Value::String(text)) => from_text(text),
                Some(Value::Number(n)) => n.as_f64().and_then(|v| from_number(v, &format)),
                _ => None,
            }
        }
        _ => None,
    };
    read.ok_or_else(|| match untag(node) {
        Value::Mapping(_) => node.get("value").map(scalar_text).unwrap_or_else(|| "no value".to_string()),
        _ => scalar_text(node),
    })
}

impl Fk5Frame {
    pub fn j2000() -> Self {
        Self { equinox_label: "J2000".to_string(), jd_tt: JD_J2000 }
    }

    pub fn from_reference_frame(reference_frame: &Value) -> Result<Self, String> {
        let equinox = reference_frame
            .get("frame_attributes")
            .and_then(|attributes| attributes.get("equinox"))
            .filter(|node| !untag(node).is_null());
        match equinox {
            None => Ok(Self::j2000()),
            Some(node) => from_equinox_node(node).map(|frame| if frame.is_j2000() { Self::j2000() } else { frame }),
        }
    }

    pub fn is_j2000(&self) -> bool {
        (self.jd_tt - JD_J2000).abs() <= J2000_TOLERANCE_DAYS
    }

    pub fn to_j2000(&self) -> Option<Mat3> {
        (!self.is_j2000()).then(|| precession_to_j2000(self.jd_tt))
    }

    pub fn note(&self) -> String {
        if self.is_j2000() {
            format!("FK5 J2000 read as ICRS; {FRAME_TIE_NOTE}")
        } else {
            format!("FK5 {} precessed to J2000; {FRAME_TIE_NOTE}", self.equinox_label)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::gwcs::test_support::fixtures_dir;
    use crate::core::astrometry::wcs::angular_separation;

    #[derive(serde::Deserialize)]
    struct Golden {
        astropy: String,
        cases: Vec<Case>,
    }

    #[derive(serde::Deserialize)]
    struct Case {
        equinox: String,
        julian_year: f64,
        lon_deg: f64,
        lat_deg: f64,
        fk5_j2000_ra_deg: f64,
        fk5_j2000_dec_deg: f64,
    }

    fn goldens() -> Golden {
        let path = fixtures_dir().join("fk5_precession_astropy.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn precession_to_j2000_matches_astropy_for_julian_and_besselian_equinoxes() {
        let goldens = goldens();
        assert_eq!(goldens.astropy, "8.0.2", "values of scripts/oracle/fk5_precession_goldens.py");
        assert_eq!(goldens.cases.len(), 20);
        let mut worst_mas = 0.0f64;
        for case in &goldens.cases {
            let frame = from_text(&case.equinox).unwrap_or_else(|| panic!("{} is read", case.equinox));
            let year = J2000_YEAR + (frame.jd_tt - JD_J2000) / DAYS_PER_JULIAN_YEAR;
            assert!((year - case.julian_year).abs() < 1e-9, "{}: Julian year {year} vs astropy {}", case.equinox, case.julian_year);
            let got = match frame.to_j2000() {
                Some(matrix) => rotate_sky(&matrix, case.lon_deg, case.lat_deg),
                None => [case.lon_deg, case.lat_deg],
            };
            let off_mas = angular_separation(got[0], got[1], case.fk5_j2000_ra_deg, case.fk5_j2000_dec_deg) * 3.6e6;
            worst_mas = worst_mas.max(off_mas);
            assert!(off_mas < 1e-5, "FK5 {} ({}, {}): {got:?} is {off_mas} mas from astropy", case.equinox, case.lon_deg, case.lat_deg);
        }
        println!("FK5 precession to J2000 against astropy {}: worst {worst_mas:.3e} mas over {} cases", goldens.astropy, goldens.cases.len());
    }

    #[test]
    fn the_precession_matrix_is_a_rotation_and_the_identity_at_j2000() {
        let identity = precession_to_j2000(JD_J2000);
        for (i, row) in identity.iter().enumerate() {
            for (j, cell) in row.iter().enumerate() {
                assert!((cell - if i == j { 1.0 } else { 0.0 }).abs() < 1e-15, "[{i}][{j}] = {cell}");
            }
        }
        let m = precession_to_j2000(julian_year_jd(1950.0));
        let product = mat_mul(&m, &mat_transpose(&m));
        for (i, row) in product.iter().enumerate() {
            for (j, cell) in row.iter().enumerate() {
                assert!((cell - if i == j { 1.0 } else { 0.0 }).abs() < 1e-14, "[{i}][{j}] = {cell}");
            }
        }
        assert_eq!(rotate_sky(&m, f64::NAN, 10.0)[1], 10.0, "a non-finite position is passed through");
    }

    #[test]
    fn equinox_labels_are_short_and_keep_the_epoch_kind() {
        assert_eq!(from_text("J1975.000").unwrap().equinox_label, "J1975.0");
        assert_eq!(from_text("j2015.5").unwrap().equinox_label, "J2015.5");
        assert_eq!(from_text("1975.25").unwrap().equinox_label, "J1975.25");
        assert_eq!(from_text("B1950").unwrap().equinox_label, "B1950.0");
        assert_eq!(from_text("2000-01-01T12:00:00").unwrap().equinox_label, "J2000.0");
        assert!(from_text("2000-01-01T12:00:00").unwrap().is_j2000());
        assert!(from_text("equinox").is_none());
        assert!(from_number(1975.0, "decimalyear").is_none(), "a format that is not a Julian or Besselian year, JD or MJD is not guessed");
        assert_eq!(from_number(1950.0, "byear").unwrap().equinox_label, "B1950.0");
        for frame in [Fk5Frame::j2000(), from_text("B1950.0").unwrap(), from_text("J1975.125").unwrap()] {
            assert!(frame.note().len() <= 67, "{}", frame.note());
        }
    }
}
