use crate::core::astrometry::catalog::observation_epoch_year;
use crate::core::astrometry::object_names;
use crate::core::astrometry::spectral::header_target_coordinates;
use crate::core::astrometry::wcs::{pixel_center, read_effective_cd};
use crate::core::imaging::cfa::CfaPattern;
use crate::core::imaging::region_file::format_num;
use crate::core::metadata::camera_db::{self, FORMAT_ESTIMATE_TOLERANCE};
use crate::core::metadata::header_aliases::{
    first_i64, first_text, focal_length_mm, pixel_size_um, BINNING_KEYS,
};
use crate::infra::wcs_source::header_wcs;
use crate::types::header::HduHeader;

#[derive(Debug, Clone, PartialEq)]
pub struct Pointing {
    pub ra: f64,
    pub dec: f64,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScaleRange {
    pub low_arcsec: f64,
    pub high_arcsec: f64,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NamedObject {
    pub text: String,
    pub designation: String,
    pub ra: f64,
    pub dec: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SolveHints {
    pub centre: Option<Pointing>,
    pub scale: Option<ScaleRange>,
    pub fov_deg: Option<f64>,
    pub epoch_year: Option<f64>,
    pub cfa: Option<CfaPattern>,
    pub object: Option<NamedObject>,
    pub warnings: Vec<String>,
}

pub const SCALE_HINT_MARGIN: f64 = 0.10;

const ARCSEC_PER_DEGREE: f64 = 3600.0;
const WCS_SOURCE: &str = "WCS";
const OBJECT_KEYS: [&str; 1] = ["OBJECT"];
const INSTRUMENT_KEYS: [&str; 1] = ["INSTRUME"];
const CELESTIAL_LONGITUDE_AXES: [&str; 3] = ["RA", "GLON", "ELON"];
const CD_KEYS: [&str; 4] = ["CD1_1", "CD1_2", "CD2_1", "CD2_2"];
const PC_KEYS: [&str; 4] = ["PC1_1", "PC1_2", "PC2_1", "PC2_2"];

pub fn solve_hints(header: &HduHeader, width: usize, height: usize) -> SolveHints {
    let mut warnings = Vec::new();
    let wcs = wcs_hints(header, width, height, &mut warnings);
    let object = named_object(header);
    let (centre, scale) = match wcs {
        Some((centre, scale)) => (Some(centre), Some(scale)),
        None => (
            card_pointing(header, &mut warnings).or_else(|| object.as_ref().map(object_pointing)),
            cd_scale(header).or_else(|| optics_scale(header, width, height)),
        ),
    };
    if centre.is_none() && object.is_none() {
        if let Some((text, key)) = first_text(header, &OBJECT_KEYS) {
            warnings.push(format!("{key} '{text}' is not in the offline object list"));
        }
    }
    let fov_deg = scale
        .as_ref()
        .and_then(|scale| long_side_deg(scale, width, height));
    SolveHints {
        centre,
        scale,
        fov_deg,
        epoch_year: observation_epoch_year(header),
        cfa: CfaPattern::detect(header),
        object,
        warnings,
    }
}

fn is_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn pointing(ra: f64, dec: f64, source: String) -> Pointing {
    Pointing {
        ra: ra.rem_euclid(360.0),
        dec,
        source,
    }
}

fn widened(low_arcsec: f64, high_arcsec: f64, source: String) -> ScaleRange {
    ScaleRange {
        low_arcsec: low_arcsec * (1.0 - SCALE_HINT_MARGIN),
        high_arcsec: high_arcsec * (1.0 + SCALE_HINT_MARGIN),
        source,
    }
}

fn long_side_deg(scale: &ScaleRange, width: usize, height: usize) -> Option<f64> {
    let long_side = width.max(height);
    let nominal = (scale.low_arcsec + scale.high_arcsec) / 2.0;
    (long_side > 0).then(|| long_side as f64 * nominal / ARCSEC_PER_DEGREE)
}

fn has_celestial_axes(header: &HduHeader) -> bool {
    first_text(header, &["CTYPE1"]).is_some_and(|(ctype, _)| {
        let axis = ctype.to_ascii_uppercase();
        CELESTIAL_LONGITUDE_AXES.contains(&axis.split('-').next().unwrap_or_default())
    })
}

fn wcs_hints(
    header: &HduHeader,
    width: usize,
    height: usize,
    warnings: &mut Vec<String>,
) -> Option<(Pointing, ScaleRange)> {
    if !has_celestial_axes(header) {
        return None;
    }
    let wcs = match header_wcs(header) {
        Ok(wcs) => wcs,
        Err(error) => {
            warnings.push(format!("WCS cards ignored for the hint: {error:#}"));
            return None;
        }
    };
    let (x, y) = pixel_center(width, height);
    let centre = wcs.pixel_to_world(x, y);
    let scale = wcs.pixel_scale_arcsec();
    if !(centre.ra.is_finite() && centre.dec.is_finite() && is_positive(scale)) {
        warnings.push(
            "WCS cards ignored for the hint: the image centre or the pixel scale is not finite"
                .to_string(),
        );
        return None;
    }
    Some((
        pointing(centre.ra, centre.dec, WCS_SOURCE.to_string()),
        widened(scale, scale, WCS_SOURCE.to_string()),
    ))
}

fn card_pointing(header: &HduHeader, warnings: &mut Vec<String>) -> Option<Pointing> {
    let (ra, dec, source) = header_target_coordinates(header)?;
    if !ra.is_finite() || !(-90.0..=90.0).contains(&dec) {
        warnings.push(format!(
            "{source} ignored for the hint: ({}, {}) is not a sky position",
            format_num(ra),
            format_num(dec)
        ));
        return None;
    }
    Some(pointing(ra, dec, source.to_string()))
}

fn named_object(header: &HduHeader) -> Option<NamedObject> {
    let (text, _) = first_text(header, &OBJECT_KEYS)?;
    let (designation, found) = object_names::resolve(&text)?;
    Some(NamedObject {
        text,
        designation,
        ra: found.ra_deg,
        dec: found.dec_deg,
    })
}

fn object_pointing(object: &NamedObject) -> Pointing {
    pointing(
        object.ra,
        object.dec,
        format!(
            "OBJECT '{}' ({}, offline list)",
            object.text, object.designation
        ),
    )
}

fn matrix_source(header: &HduHeader) -> &'static str {
    let present = |keys: &[&str]| keys.iter().any(|key| header.get(key).is_some());
    if present(&CD_KEYS) {
        "CD matrix"
    } else if present(&PC_KEYS) {
        "PC matrix with CDELT1/CDELT2"
    } else {
        "CDELT1/CDELT2"
    }
}

fn cd_scale(header: &HduHeader) -> Option<ScaleRange> {
    if !has_celestial_axes(header) {
        return None;
    }
    let cd = read_effective_cd(header).ok()?;
    let column_x = cd[0][0].hypot(cd[1][0]);
    let column_y = cd[0][1].hypot(cd[1][1]);
    let scale = (column_x + column_y) / 2.0 * ARCSEC_PER_DEGREE;
    is_positive(scale).then(|| widened(scale, scale, matrix_source(header).to_string()))
}

fn optics_scale(header: &HduHeader, width: usize, height: usize) -> Option<ScaleRange> {
    let (focal_mm, focal_key) = focal_length_mm(header)?;
    let focal = format!("{focal_key} {} mm", format_num(focal_mm));
    if let Some((pitch, key)) = pixel_size_um(header) {
        let label = format!("{key} {} µm", format_num(pitch));
        return binned_range(header, (pitch, pitch), focal_mm, label, &focal);
    }
    let (instrument, instrument_key) = first_text(header, &INSTRUMENT_KEYS)?;
    if let Some(entry) = camera_db::lookup(&instrument) {
        let other = entry.alternate_bin1_pitch_um.unwrap_or(entry.pitch_um);
        let pitches = (entry.pitch_um.min(other), entry.pitch_um.max(other));
        let pitch_text = if pitches.0 == pitches.1 {
            format_num(pitches.0)
        } else {
            format!("{} or {}", format_num(pitches.0), format_num(pitches.1))
        };
        let label = format!(
            "{instrument_key} '{instrument}' ({}, {pitch_text} µm)",
            entry.sensor
        );
        return binned_range(header, pitches, focal_mm, label, &focal);
    }
    let estimate = camera_db::format_class(&instrument, width, height)?;
    Some(ScaleRange {
        low_arcsec: camera_db::scale_arcsec_per_px(estimate.window.0, focal_mm)?,
        high_arcsec: camera_db::scale_arcsec_per_px(estimate.window.1, focal_mm)?,
        source: format!(
            "{instrument_key} '{instrument}' {} ({} µm ±{} %) / {focal}",
            estimate.label,
            format_num(estimate.pitch_um),
            format_num(FORMAT_ESTIMATE_TOLERANCE * 100.0)
        ),
    })
}

fn binned_range(
    header: &HduHeader,
    pitches_um: (f64, f64),
    focal_mm: f64,
    pitch_label: String,
    focal_label: &str,
) -> Option<ScaleRange> {
    let binning = first_i64(header, &BINNING_KEYS).filter(|(factor, _)| *factor > 1);
    let factor = binning.map_or(1.0, |(factor, _)| factor as f64);
    let low = camera_db::scale_arcsec_per_px(pitches_um.0, focal_mm)?;
    let high = camera_db::scale_arcsec_per_px(pitches_um.1 * factor, focal_mm)?;
    let pitch_label = match binning {
        Some((factor, key)) => format!("{pitch_label} × 1 to {factor} ({key})"),
        None => pitch_label,
    };
    Some(widened(low, high, format!("{pitch_label} / {focal_label}")))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use super::*;
    use crate::core::astrometry::object_names::{resolve, OFFLINE_OBJECTS};
    use crate::core::astrometry::wcs::{angular_separation, WcsTransform};
    use crate::core::imaging::debayer::BayerPattern;
    use crate::core::metadata::camera_db::scale_arcsec_per_px;

    const ARCMIN_DEG: f64 = 1.0 / 60.0;

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (key, value) in cards {
            h.set(key, (*value).to_string());
        }
        h
    }

    fn scale_of(hints: &SolveHints) -> &ScaleRange {
        hints.scale.as_ref().expect("scale hint")
    }

    fn centre_of(hints: &SolveHints) -> &Pointing {
        hints.centre.as_ref().expect("centre hint")
    }

    fn close(actual: f64, expected: f64, relative: f64) -> bool {
        (actual - expected).abs() <= relative * expected.abs()
    }

    #[test]
    fn sharpcap_binned_xpixsz_gives_a_range_covering_both_readings() {
        let h = header(&[
            ("INSTRUME", "'ZWO ASI585MC'"),
            ("XPIXSZ", "2.9"),
            ("XBINNING", "2"),
            ("FOCALLEN", "400"),
        ]);
        let hints = solve_hints(&h, 1920, 1080);
        let scale = scale_of(&hints);
        let unbinned = scale_arcsec_per_px(2.9, 400.0).unwrap();
        let binned = scale_arcsec_per_px(5.8, 400.0).unwrap();
        assert!(
            close(
                scale.low_arcsec,
                unbinned * (1.0 - SCALE_HINT_MARGIN),
                1e-12
            ),
            "{scale:?}"
        );
        assert!(
            close(scale.high_arcsec, binned * (1.0 + SCALE_HINT_MARGIN), 1e-12),
            "{scale:?}"
        );
        for key in ["XPIXSZ", "XBINNING", "FOCALLEN"] {
            assert!(
                scale.source.contains(key),
                "{key} missing from {:?}",
                scale.source
            );
        }

        let unbinned_only = solve_hints(
            &header(&[("XPIXSZ", "2.9"), ("XBINNING", "1"), ("FOCALLEN", "400")]),
            1920,
            1080,
        );
        let scale = scale_of(&unbinned_only);
        assert!(
            close(
                scale.high_arcsec,
                unbinned * (1.0 + SCALE_HINT_MARGIN),
                1e-12
            ),
            "{scale:?}"
        );
        assert!(!scale.source.contains("XBINNING"), "{:?}", scale.source);
    }

    #[test]
    fn asiair_ra_dec_are_degrees_and_named() {
        let h = header(&[
            ("OBJECT", "'M 1'"),
            ("RA", "83.633083"),
            ("DEC", "22.0145"),
            ("FOCALLEN", "250"),
            ("XPIXSZ", "2.9"),
        ]);
        let hints = solve_hints(&h, 1080, 1920);
        let centre = centre_of(&hints);
        assert_eq!((centre.ra, centre.dec), (83.633083, 22.0145));
        assert_eq!(centre.source, "RA/DEC");
        assert_eq!(
            hints.object.as_ref().map(|o| o.designation.as_str()),
            Some("M 1")
        );
    }

    #[test]
    fn a_free_text_object_starting_with_c_and_a_digit_gives_no_caldwell_centre() {
        let hints = solve_hints(&header(&[("OBJECT", "'C1 test'")]), 1536, 1024);
        assert_eq!(hints.centre, None);
        assert_eq!(hints.object, None);
        assert!(
            hints
                .warnings
                .iter()
                .any(|w| w.contains("'C1 test' is not in the offline object list")),
            "{:?}",
            hints.warnings
        );
        let caldwell = solve_hints(&header(&[("OBJECT", "'C1'")]), 1536, 1024);
        assert_eq!(
            caldwell.object.as_ref().map(|o| o.designation.as_str()),
            Some("C 1")
        );
        assert!(caldwell.centre.is_some(), "{caldwell:?}");
    }

    #[test]
    fn nina_sexagesimal_objctra_is_hours() {
        let h = header(&[
            ("OBJCTRA", "'05 34 31.940'"),
            ("OBJCTDEC", "'+22 00 52.20'"),
        ]);
        let hints = solve_hints(&h, 6248, 4176);
        let centre = centre_of(&hints);
        assert!((centre.ra - 83.633_083).abs() < 1e-5, "{centre:?}");
        assert!((centre.dec - 22.0145).abs() < 1e-5, "{centre:?}");
        assert_eq!(centre.source, "OBJCTRA/OBJCTDEC");
    }

    #[test]
    fn camera_db_pitch_fills_the_scale_when_xpixsz_is_missing() {
        let h = header(&[("INSTRUME", "'ZWO ASI2600MC Pro'"), ("FOCALLEN", "530")]);
        let hints = solve_hints(&h, 6248, 4176);
        let scale = scale_of(&hints);
        let native = scale_arcsec_per_px(3.76, 530.0).unwrap();
        assert!(
            close(scale.low_arcsec, native * (1.0 - SCALE_HINT_MARGIN), 1e-12),
            "{scale:?}"
        );
        assert!(
            close(scale.high_arcsec, native * (1.0 + SCALE_HINT_MARGIN), 1e-12),
            "{scale:?}"
        );
        for part in ["INSTRUME", "IMX571", "3.76", "FOCALLEN"] {
            assert!(
                scale.source.contains(part),
                "{part} missing from {:?}",
                scale.source
            );
        }

        let binned = solve_hints(
            &header(&[
                ("INSTRUME", "'ZWO ASI2600MM Pro'"),
                ("FOCALLEN", "530"),
                ("XBINNING", "2"),
            ]),
            3124,
            2088,
        );
        let scale = scale_of(&binned);
        assert!(
            close(
                scale.high_arcsec,
                2.0 * native * (1.0 + SCALE_HINT_MARGIN),
                1e-12
            ),
            "{scale:?}"
        );

        let two_modes = solve_hints(
            &header(&[("INSTRUME", "'QHY294M'"), ("FOCALLEN", "500")]),
            8288,
            5644,
        );
        let scale = scale_of(&two_modes);
        assert!(
            scale.low_arcsec <= scale_arcsec_per_px(2.315, 500.0).unwrap(),
            "{scale:?}"
        );
        assert!(
            scale.high_arcsec >= scale_arcsec_per_px(4.63, 500.0).unwrap(),
            "{scale:?}"
        );

        let xpixsz_wins = solve_hints(
            &header(&[
                ("INSTRUME", "'ZWO ASI2600MC Pro'"),
                ("FOCALLEN", "530"),
                ("XPIXSZ", "7.52"),
            ]),
            3124,
            2088,
        );
        assert!(
            scale_of(&xpixsz_wins).source.starts_with("XPIXSZ"),
            "{xpixsz_wins:?}"
        );
    }

    #[test]
    fn format_class_estimate_fills_the_scale_for_an_unlisted_dslr_with_flength_in_metres() {
        let h = header(&[("INSTRUME", "'Canon EOS 6D'"), ("FLENGTH", "0.4")]);
        let hints = solve_hints(&h, 5472, 3648);
        let scale = scale_of(&hints);
        let pitch = 36.0 * 1000.0 / 5472.0;
        let centre = scale_arcsec_per_px(pitch, 400.0).unwrap();
        assert!(close(scale.low_arcsec, centre * 0.85, 1e-9), "{scale:?}");
        assert!(close(scale.high_arcsec, centre * 1.15, 1e-9), "{scale:?}");
        for part in ["format class estimate", "FLENGTH", "400 mm"] {
            assert!(
                scale.source.contains(part),
                "{part} missing from {:?}",
                scale.source
            );
        }
    }

    #[test]
    fn object_name_m31_resolves_offline() {
        let hints = solve_hints(&header(&[("OBJECT", "'M31'")]), 4000, 3000);
        let centre = centre_of(&hints);
        assert!(
            angular_separation(centre.ra, centre.dec, 10.684_708, 41.268_75) < ARCMIN_DEG,
            "{centre:?}"
        );
        assert!(
            centre.source.contains("OBJECT") && centre.source.contains("M 31"),
            "{:?}",
            centre.source
        );
        let object = hints.object.as_ref().expect("named object");
        assert_eq!(
            (object.text.as_str(), object.designation.as_str()),
            ("M31", "M 31")
        );
        assert!(hints.warnings.is_empty(), "{:?}", hints.warnings);

        for text in ["Messier 31", "NGC 224", "m_31 panel 2"] {
            let hints = solve_hints(&header(&[("OBJECT", &format!("'{text}'"))]), 4000, 3000);
            let centre = centre_of(&hints);
            assert!(
                angular_separation(centre.ra, centre.dec, 10.684_708, 41.268_75) < ARCMIN_DEG,
                "{text}"
            );
        }
        let caldwell =
            centre_of(&solve_hints(&header(&[("OBJECT", "'C 20'")]), 4000, 3000)).clone();
        let ngc = centre_of(&solve_hints(
            &header(&[("OBJECT", "'NGC 7000'")]),
            4000,
            3000,
        ))
        .clone();
        assert_eq!((caldwell.ra, caldwell.dec), (ngc.ra, ngc.dec));

        let unknown = solve_hints(&header(&[("OBJECT", "'NGC 9999'")]), 4000, 3000);
        assert!(unknown.centre.is_none() && unknown.object.is_none());
        assert!(
            unknown.warnings.iter().any(|w| w.contains("NGC 9999")),
            "{:?}",
            unknown.warnings
        );

        let carded = solve_hints(
            &header(&[
                ("OBJECT", "'M31'"),
                ("OBJCTRA", "'00 40 00'"),
                ("OBJCTDEC", "'+41 00 00'"),
            ]),
            4000,
            3000,
        );
        assert_eq!(centre_of(&carded).source, "OBJCTRA/OBJCTDEC");
        assert!(carded.object.is_some());
    }

    #[test]
    fn every_offline_object_lies_within_1_arcmin_of_its_sesame_position() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/astrometry/sesame_positions.json");
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();
        let records = fixture["objects"].as_array().expect("objects array");
        let mut sesame: HashMap<String, (f64, f64)> = HashMap::new();
        for record in records {
            let position = (
                record["ra_deg"].as_f64().unwrap(),
                record["dec_deg"].as_f64().unwrap(),
            );
            sesame.insert(
                record["designation"].as_str().unwrap().to_string(),
                position,
            );
        }
        for record in records {
            let position = (
                record["ra_deg"].as_f64().unwrap(),
                record["dec_deg"].as_f64().unwrap(),
            );
            for alias in record["aliases"].as_array().unwrap() {
                sesame
                    .entry(alias.as_str().unwrap().to_string())
                    .or_insert(position);
            }
        }

        assert!(
            OFFLINE_OBJECTS.len() >= 450,
            "{} objects",
            OFFLINE_OBJECTS.len()
        );
        for object in OFFLINE_OBJECTS {
            assert!(!object.designations.is_empty());
            let explicit: Vec<&&str> = object
                .designations
                .iter()
                .filter(|d| sesame.contains_key(**d))
                .collect();
            assert_eq!(
                explicit.len(),
                object.designations.len(),
                "{:?} not all from Sesame",
                object.designations
            );
            for designation in object.designations {
                let (ra, dec) = sesame[*designation];
                let separation = angular_separation(object.ra_deg, object.dec_deg, ra, dec);
                assert!(
                    separation < ARCMIN_DEG,
                    "{designation}: {separation} deg from Sesame"
                );
            }
        }
        for record in records {
            let designation = record["designation"].as_str().unwrap();
            let (_, object) = resolve(designation)
                .unwrap_or_else(|| panic!("{designation} does not resolve offline"));
            let separation = angular_separation(
                object.ra_deg,
                object.dec_deg,
                record["ra_deg"].as_f64().unwrap(),
                record["dec_deg"].as_f64().unwrap(),
            );
            assert!(
                separation < ARCMIN_DEG,
                "{designation}: {separation} deg from Sesame"
            );
        }
        for n in 1..=110 {
            assert!(resolve(&format!("M {n}")).is_some(), "M {n}");
        }
        for n in 1..=109 {
            assert!(resolve(&format!("C {n}")).is_some(), "C {n}");
        }
    }

    #[test]
    fn existing_wcs_wins_over_pointing_cards() {
        let h = header(&[
            ("NAXIS", "2"),
            ("NAXIS1", "1000"),
            ("NAXIS2", "800"),
            ("CTYPE1", "'RA---TAN'"),
            ("CTYPE2", "'DEC--TAN'"),
            ("CRVAL1", "150.0"),
            ("CRVAL2", "2.0"),
            ("CRPIX1", "1.0"),
            ("CRPIX2", "1.0"),
            ("CD1_1", "-0.0004"),
            ("CD1_2", "0.0"),
            ("CD2_1", "0.0"),
            ("CD2_2", "0.0004"),
            ("OBJECT", "'M42'"),
            ("OBJCTRA", "'05 35 17'"),
            ("OBJCTDEC", "'-05 23 28'"),
            ("FOCALLEN", "1000"),
            ("XPIXSZ", "9"),
        ]);
        let hints = solve_hints(&h, 1000, 800);
        let wcs = WcsTransform::from_header(&h).unwrap();
        let (x, y) = pixel_center(1000, 800);
        let expected = wcs.pixel_to_world(x, y);
        let centre = centre_of(&hints);
        assert_eq!(centre.source, "WCS");
        assert!(
            angular_separation(centre.ra, centre.dec, expected.ra, expected.dec) < 1e-9,
            "{centre:?}"
        );
        assert!(
            angular_separation(centre.ra, centre.dec, 150.0, 2.0) > 0.1,
            "the image centre, not CRVAL"
        );
        let scale = scale_of(&hints);
        assert_eq!(scale.source, "WCS");
        assert!(
            scale.low_arcsec < 1.44 && scale.high_arcsec > 1.44,
            "{scale:?}"
        );
        assert!(
            close(scale.low_arcsec, 1.44 * (1.0 - SCALE_HINT_MARGIN), 1e-9),
            "{scale:?}"
        );
        assert!(hints.warnings.is_empty(), "{:?}", hints.warnings);
    }

    #[test]
    fn a_tan_header_with_pv_cards_hints_from_its_linear_tan() {
        use crate::infra::wcs_source::test_fixtures::PV_DISTORTION_CARDS;

        let linear = [
            ("NAXIS1", "1000"),
            ("NAXIS2", "800"),
            ("CTYPE1", "'RA---TAN'"),
            ("CTYPE2", "'DEC--TAN'"),
            ("CRVAL1", "180.0"),
            ("CRVAL2", "45.0"),
            ("CRPIX1", "1.0"),
            ("CRPIX2", "1.0"),
            ("CD1_1", "-0.0004"),
            ("CD2_2", "0.0004"),
        ];
        let with_pv: Vec<(&str, &str)> = linear.into_iter().chain(PV_DISTORTION_CARDS).collect();
        let hints = solve_hints(&header(&with_pv), 1000, 800);
        let reference = solve_hints(&header(&linear), 1000, 800);
        assert!(hints.warnings.is_empty(), "{:?}", hints.warnings);
        let (centre, expected) = (centre_of(&hints), centre_of(&reference));
        assert_eq!(centre.source, "WCS");
        assert!(
            angular_separation(centre.ra, centre.dec, expected.ra, expected.dec) < 1e-12,
            "{centre:?} vs {expected:?}"
        );
        assert_eq!(scale_of(&hints), scale_of(&reference));
    }

    #[test]
    fn celestial_cdelt_without_a_usable_wcs_still_gives_the_scale() {
        let h = header(&[
            ("CTYPE1", "'RA---TAN'"),
            ("CTYPE2", "'DEC--TAN'"),
            ("CDELT1", "-0.0005"),
            ("CDELT2", "0.0005"),
            ("RA", "10.0"),
            ("DEC", "20.0"),
        ]);
        let hints = solve_hints(&h, 2000, 1500);
        let scale = scale_of(&hints);
        assert!(
            close(scale.low_arcsec, 1.8 * (1.0 - SCALE_HINT_MARGIN), 1e-9),
            "{scale:?}"
        );
        assert_eq!(scale.source, "CDELT1/CDELT2");
        assert_eq!(centre_of(&hints).source, "RA/DEC");
        assert!(
            hints.warnings.iter().any(|w| w.contains("CRPIX1")),
            "{:?}",
            hints.warnings
        );

        let spectral = header(&[("CTYPE1", "'WAVE'"), ("CDELT1", "2.5"), ("CDELT2", "1.0")]);
        let hints = solve_hints(&spectral, 2000, 1);
        assert!(
            hints.scale.is_none() && hints.warnings.is_empty(),
            "{hints:?}"
        );
    }

    #[test]
    fn a_wcs_less_frame_returns_centre_scale_fov_and_epoch_with_their_sources() {
        let h = header(&[
            ("OBJECT", "'M 42'"),
            ("OBJCTRA", "'05 35 17.3'"),
            ("OBJCTDEC", "'-05 23 28'"),
            ("FOCALLEN", "250"),
            ("XPIXSZ", "3.76"),
            ("DATE-OBS", "'2026-03-21T22:00:00'"),
            ("EXPTIME", "120"),
            ("BAYERPAT", "'RGGB'"),
        ]);
        let hints = solve_hints(&h, 6248, 4176);
        let centre = centre_of(&hints);
        assert_eq!(centre.source, "OBJCTRA/OBJCTDEC");
        assert!(
            (centre.ra - 83.822_083).abs() < 1e-5 && (centre.dec + 5.391_111).abs() < 1e-5,
            "{centre:?}"
        );
        let scale = scale_of(&hints);
        assert_eq!(scale.source, "XPIXSZ 3.76 µm / FOCALLEN 250 mm");
        let nominal = scale_arcsec_per_px(3.76, 250.0).unwrap();
        let fov = hints.fov_deg.expect("fov");
        let expected_fov = 6248.0 * nominal / 3600.0;
        assert!(
            close(fov, expected_fov, 0.01),
            "fov {fov} vs {expected_fov}"
        );
        let epoch = hints.epoch_year.expect("epoch");
        assert!((epoch - 2026.22).abs() < 0.01, "{epoch}");
        assert_eq!(hints.cfa, Some(CfaPattern::Bayer(BayerPattern::Rggb)));
        assert!(hints.warnings.is_empty(), "{:?}", hints.warnings);

        let bare = solve_hints(&HduHeader::empty(), 100, 100);
        assert_eq!(bare, SolveHints::default());
    }
}
