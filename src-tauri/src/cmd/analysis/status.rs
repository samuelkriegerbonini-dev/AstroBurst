use serde::Serialize;

use super::{
    axis_scale_warning, axis_scales_arcsec, check_user_zero_point, photometry_context_with, pixel_area_scale_arcsec, pixel_units,
    UserZeroPoint,
};
use crate::cmd::common::blocking_cmd;
use crate::cmd::filters::{identify_filter_in, IdentifiedFilter};
use crate::core::metadata::photcal::PhotCal;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatusErr {
    pub origin: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatusDq {
    pub source: String,
    pub exclusion_available: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatusFilter {
    pub dataset: String,
    pub filter: String,
    pub pivot_um: f64,
    pub source: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub photcal: serde_json::Value,
    pub calibration_origin: Option<String>,
    pub calibration_source: Option<String>,
    pub missing_reason: Option<String>,
    pub bunit: Option<String>,
    pub summed_unit: Option<String>,
    pub err: Option<StatusErr>,
    pub dq: Option<StatusDq>,
    pub pixel_scale_arcsec: Option<f64>,
    pub pixel_scale_axes_arcsec: Option<[f64; 2]>,
    pub filter: Option<StatusFilter>,
    pub warnings: Vec<String>,
}

fn status_filter_of(found: IdentifiedFilter) -> StatusFilter {
    StatusFilter {
        dataset: found.dataset,
        filter: found.filter,
        pivot_um: found.pivot_um,
        source: found.source,
        warning: found.warning,
    }
}

fn calibration_origin(cal: &PhotCal) -> String {
    if cal.origin.is_empty() {
        cal.label()
    } else {
        cal.origin.clone()
    }
}

pub(crate) fn photometry_status_for_path(
    path: &str,
    exclude_dq: bool,
    zero_point: Option<&UserZeroPoint>,
) -> anyhow::Result<StatusReport> {
    let ctx = photometry_context_with(path, exclude_dq, zero_point)?;
    let header = ctx.entry.header();
    let layers = &ctx.layers;
    let missing_reason = ctx.calibration_gap.clone();
    let (bunit, summed_unit) = pixel_units(header);
    let dq = layers.dq_source.as_ref().map(|source| StatusDq {
        source: match &layers.dq_flags {
            Some(flags) => format!("{source}, {flags} flags"),
            None => source.clone(),
        },
        exclusion_available: layers.dq_exclusion_available,
        unavailable_reason: layers.dq_gap.clone(),
    });
    let filter = header.and_then(|header| identify_filter_in(header, path, None).ok()).map(status_filter_of);
    let mut warnings: Vec<String> =
        ctx.warnings.iter().filter(|warning| missing_reason.as_ref() != Some(*warning)).cloned().collect();
    warnings.extend(ctx.wcs.as_ref().and_then(axis_scale_warning));
    Ok(StatusReport {
        photcal: ctx.photcal_json()?,
        calibration_origin: ctx.photcal.as_ref().map(calibration_origin),
        calibration_source: ctx.calibration_source.map(str::to_string),
        missing_reason,
        bunit,
        summed_unit,
        err: layers.err_source.clone().map(|origin| StatusErr { origin }),
        dq,
        pixel_scale_arcsec: ctx.wcs.as_ref().and_then(pixel_area_scale_arcsec),
        pixel_scale_axes_arcsec: ctx.wcs.as_ref().map(axis_scales_arcsec).map(|(sx, sy)| [sx, sy]),
        filter,
        warnings,
    })
}

#[tauri::command]
pub async fn photometry_status_cmd(
    path: String,
    exclude_dq: Option<bool>,
    zero_point: Option<UserZeroPoint>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        check_user_zero_point(zero_point.as_ref())?;
        let report = photometry_status_for_path(&path, exclude_dq.unwrap_or(false), zero_point.as_ref())?;
        Ok(serde_json::to_value(&report)?)
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::cmd::analysis::has_err_extension;
    use crate::cmd::analysis::layer_fixtures::{
        afw_star_mef, flat, inverse_variance_star_mef, legacy_band_cutout, legacy_brick, unnamed_flag_star_mef,
        LEGACY_BRICK_INVVAR, SIZE,
    };
    use crate::types::image_ref::BandPlaneKey;
    use crate::cmd::common::{cached_header, load_companions};
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::core::imaging::stats::compute_image_stats;
    use crate::core::metadata::filter_wavelengths::FilterRegistry;
    use crate::core::metadata::photcal::missing_calibration_reason;
    use crate::infra::cache::GLOBAL_IMAGE_CACHE;
    use crate::cmd::analysis::{UserMagSystem, USER_ZERO_POINT_WARNING};
    use crate::service::area::test_fixtures::{cal_sci_cards, gaussian_pixels, jwst_cal_mef, CalPlanes};
    use crate::types::header::HduHeader;

    const AXIS_TOLERANCE: f64 = 1e-9;
    const AREA_TOLERANCE: f64 = 1e-12;
    const REAL_SCALE_TOLERANCE: f64 = 1e-9;
    const REAL_FACTOR_TOLERANCE: f64 = 1e-6;
    const NOT_APPLIED_PREFIX: &str = "user zero point not applied: the header calibrates this image (";

    fn ab_25() -> UserZeroPoint {
        UserZeroPoint { zp: 25.0, system: UserMagSystem::Ab, per_second: false }
    }

    fn star_file(dir: &Path, name: &str, cards: &[(&str, &str)], cd_arcsec: Option<(f64, f64)>) -> String {
        let size = 64;
        let arr = ndarray::Array2::from_shape_vec((size, size), gaussian_pixels(size, 1000.0, 2.0, 100.0)).unwrap();
        let mut header = HduHeader::empty();
        for (key, value) in cards {
            header.set(key, value.to_string());
        }
        if let Some((sx, sy)) = cd_arcsec {
            header.set("CTYPE1", "RA---TAN".to_string());
            header.set("CTYPE2", "DEC--TAN".to_string());
            header.set_f64("CRPIX1", 32.5);
            header.set_f64("CRPIX2", 32.5);
            header.set_f64("CRVAL1", 150.0);
            header.set_f64("CRVAL2", 2.0);
            header.set_f64("CD1_1", -sx / 3600.0);
            header.set_f64("CD1_2", 0.0);
            header.set_f64("CD2_1", 0.0);
            header.set_f64("CD2_2", sy / 3600.0);
        }
        let path = dir.join(name);
        crate::infra::fits::writer::write_fits_mono(path.to_str().unwrap(), &arr, Some(&header)).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn tan_cards(arcsec: f64) -> Vec<(&'static str, String)> {
        let deg = arcsec / 3600.0;
        vec![
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
            ("CRPIX1", "32.5".into()),
            ("CRPIX2", "32.5".into()),
            ("CRVAL1", "150.0".into()),
            ("CRVAL2", "2.0".into()),
            ("CD1_1", format!("{:.12E}", -deg)),
            ("CD1_2", "0.0".into()),
            ("CD2_1", "0.0".into()),
            ("CD2_2", format!("{:.12E}", deg)),
        ]
    }

    fn f200w_cal_cards() -> Vec<(&'static str, String)> {
        let mut cards = cal_sci_cards();
        cards.extend(tan_cards(0.031));
        cards.push(("FILTER", "'F200W'".into()));
        cards
    }

    fn assert_close(measured: f64, expected: f64, tolerance: f64) {
        assert!((measured - expected).abs() <= tolerance, "measured {measured} expected {expected} within {tolerance}");
    }

    #[test]
    fn status_of_a_calibrated_frame_names_calibration_layers_scale_and_filter() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = jwst_cal_mef(&dir.path().join("cal.fits"), &[], f200w_cal_cards(), CalPlanes::ALL, vec![]);
        let status = photometry_status_for_path(&fixture.key, false, None).unwrap();
        assert_eq!(status.photcal["convention"]["kind"], "jwst_mjy_sr", "{}", status.photcal);
        assert_eq!(status.calibration_origin.as_deref(), status.photcal["origin"].as_str());
        assert_eq!(status.missing_reason, None);
        assert_eq!(status.bunit.as_deref(), Some("MJy/sr"));
        assert_eq!(status.summed_unit.as_deref(), Some("MJy/sr x pix"));
        let loaded = load_companions(&fixture.key).unwrap();
        let err_origin = loaded.err_origin.expect("the ERR plane of a cal file").source;
        let dq_origin = loaded.dq_origin.expect("the DQ plane of a cal file");
        assert_eq!(status.err, Some(StatusErr { origin: err_origin }));
        assert_eq!(
            status.dq,
            Some(StatusDq { source: format!("{dq_origin}, jwst flags"), exclusion_available: true, unavailable_reason: None })
        );
        let pixel: f64 = 0.031 / 3600.0;
        assert_close(status.pixel_scale_arcsec.unwrap(), ((-pixel) * pixel).abs().sqrt() * 3600.0, AXIS_TOLERANCE);
        let [sx, sy] = status.pixel_scale_axes_arcsec.unwrap();
        assert_close(sx, 0.031, AXIS_TOLERANCE);
        assert_close(sy, 0.031, AXIS_TOLERANCE);
        let pivot_um = FilterRegistry::builtin().entry("JWST", "F200W").unwrap().pivot_angstrom / 1.0e4;
        let filter = status.filter.clone().unwrap();
        assert_close(filter.pivot_um, pivot_um, AXIS_TOLERANCE);
        assert_close(filter.pivot_um, 1.988648, AXIS_TOLERANCE);
        assert_eq!(
            filter,
            StatusFilter {
                dataset: "JWST".to_string(),
                filter: "F200W".to_string(),
                pivot_um: filter.pivot_um,
                source: "header FILTER".to_string(),
                warning: None,
            }
        );
        assert!(status.warnings.is_empty(), "{:?}", status.warnings);
    }

    #[test]
    fn status_of_an_uncalibrated_image_gives_the_reason_and_the_pixel_unit() {
        let dir = tempfile::tempdir().unwrap();
        let path = star_file(dir.path(), "adu.fits", &[("BUNIT", "adu")], None);
        let header = cached_header(&path).unwrap();
        let reason = missing_calibration_reason(Some(&header));
        let status = photometry_status_for_path(&path, false, None).unwrap();
        assert!(status.photcal.is_null(), "{}", status.photcal);
        assert_eq!(status.calibration_origin, None);
        assert_eq!(status.missing_reason.as_deref(), Some(reason.as_str()));
        assert!(!status.warnings.contains(&reason), "{:?}", status.warnings);
        assert_eq!(status.bunit.as_deref(), Some("adu"));
        assert_eq!(status.summed_unit.as_deref(), Some("adu"));
        assert_eq!(status.err, None);
        assert_eq!(status.dq, None);
        assert_eq!(status.pixel_scale_arcsec, None);
        assert_eq!(status.pixel_scale_axes_arcsec, None);
        assert_eq!(status.filter, None);
    }

    #[test]
    fn status_reports_dq_exclusion_that_cannot_be_applied_only_when_it_is_requested() {
        let dir = tempfile::tempdir().unwrap();
        let path = star_file(dir.path(), "adu.fits", &[("BUNIT", "adu")], None);
        let requested = photometry_status_for_path(&path, true, None).unwrap();
        assert!(
            requested.warnings.iter().any(|w| w == "DQ exclusion requested but not applied: this image has no DQ plane"),
            "{:?}",
            requested.warnings
        );
        let not_requested = photometry_status_for_path(&path, false, None).unwrap();
        assert!(!not_requested.warnings.iter().any(|w| w.starts_with("DQ exclusion")), "{:?}", not_requested.warnings);
    }

    #[test]
    fn status_scale_is_the_square_root_of_the_pixel_area_and_names_unequal_axes() {
        let dir = tempfile::tempdir().unwrap();
        let unequal = photometry_status_for_path(&star_file(dir.path(), "unequal.fits", &[], Some((0.29, 0.68))), false, None).unwrap();
        assert_close(unequal.pixel_scale_arcsec.unwrap(), 0.4440720662234904, AREA_TOLERANCE);
        let [sx, sy] = unequal.pixel_scale_axes_arcsec.unwrap();
        assert_close(sx, 0.29, AXIS_TOLERANCE);
        assert_close(sy, 0.68, AXIS_TOLERANCE);
        let sentence = "pixel scales differ per axis (0.29000 x 0.68000 arcsec); radii in arcsec use the square root of the pixel area (0.44407 arcsec)";
        assert!(unequal.warnings.iter().any(|w| w == sentence), "{:?}", unequal.warnings);

        let square = photometry_status_for_path(&star_file(dir.path(), "square.fits", &[], Some((0.262, 0.262))), false, None).unwrap();
        assert_close(square.pixel_scale_arcsec.unwrap(), 0.262, AREA_TOLERANCE);
        assert!(!square.warnings.iter().any(|w| w.starts_with("pixel scales differ per axis")), "{:?}", square.warnings);
    }

    #[test]
    fn status_of_a_surface_brightness_image_without_calibration_still_names_the_summed_unit() {
        let dir = tempfile::tempdir().unwrap();
        let path = star_file(dir.path(), "sb.fits", &[("BUNIT", "MJy/sr")], None);
        let status = photometry_status_for_path(&path, false, None).unwrap();
        assert_eq!(status.bunit.as_deref(), Some("MJy/sr"));
        assert_eq!(status.summed_unit.as_deref(), Some("MJy/sr x pix"));
        assert!(status.photcal.is_null(), "{}", status.photcal);
    }

    #[test]
    fn status_of_an_in_memory_key_without_a_header_answers_with_nulls() {
        let key = "__phot_panels_r2_headerless_status_probe";
        let arr = ndarray::Array2::<f32>::from_elem((8, 8), 10.0);
        GLOBAL_IMAGE_CACHE.insert_synthetic(key, std::sync::Arc::new(arr.clone()), compute_image_stats(&arr));
        let status = photometry_status_for_path(key, false, None);
        GLOBAL_IMAGE_CACHE.remove(key);
        let status = status.unwrap();
        assert_eq!(status.missing_reason, Some(missing_calibration_reason(None)));
        assert_eq!(
            status.missing_reason.as_deref(),
            Some("no header available: photometric calibration requires BUNIT and zero-point cards")
        );
        assert!(status.photcal.is_null(), "{}", status.photcal);
        assert_eq!(status.calibration_origin, None);
        assert_eq!(status.bunit, None);
        assert_eq!(status.summed_unit, None);
        assert_eq!(status.err, None);
        assert_eq!(status.dq, None);
        assert_eq!(status.pixel_scale_arcsec, None);
        assert_eq!(status.pixel_scale_axes_arcsec, None);
        assert_eq!(status.filter, None);
        assert!(status.warnings.is_empty(), "{:?}", status.warnings);
    }

    #[test]
    fn status_of_a_missing_file_is_an_error_that_names_the_path() {
        let error = photometry_status_for_path("C:/nope/missing_status.fits", false, None).unwrap_err();
        assert!(format!("{error:#}").contains("missing_status.fits"), "{error:#}");
    }

    #[test]
    fn status_keeps_the_context_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let path = star_file(dir.path(), "processed.fits", &[("ABPROC", "ghs_stretch")], None);
        let status = photometry_status_for_path(&path, false, None).unwrap();
        assert!(status.warnings.iter().any(|w| w == "photometry on processed data (ghs_stretch)"), "{:?}", status.warnings);
    }

    #[test]
    fn status_filter_keeps_the_registry_warning_and_its_source() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = jwst_cal_mef(&dir.path().join("jwst_f444w_cal.fits"), &[], f200w_cal_cards(), CalPlanes::ALL, vec![]);
        let named = FilterRegistry::builtin().identify_file_name(&fixture.path, "JWST").unwrap();
        assert_eq!(named.entry.filter, "F444W");
        let filter = photometry_status_for_path(&fixture.key, false, None).unwrap().filter.unwrap();
        assert_eq!(filter.filter, "F200W");
        assert_eq!(filter.source, "header FILTER");
        assert_eq!(filter.warning.as_deref(), Some("the file name names F444W but the header names F200W; the header wins"));
    }

    #[tokio::test]
    async fn the_command_defaults_exclude_dq_to_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = star_file(dir.path(), "adu.fits", &[("BUNIT", "adu")], Some((0.29, 0.68)));
        let defaulted = photometry_status_cmd(path.clone(), None, None).await.unwrap();
        let explicit = photometry_status_cmd(path, Some(false), None).await.unwrap();
        assert_eq!(defaulted, explicit);
        let mut keys: Vec<&str> = defaulted.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "bunit",
                "calibration_origin",
                "calibration_source",
                "dq",
                "err",
                "filter",
                "missing_reason",
                "photcal",
                "pixel_scale_arcsec",
                "pixel_scale_axes_arcsec",
                "summed_unit",
                "warnings",
            ]
        );
    }

    fn survey_file(relative: &str) -> String {
        let path = env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("survey").join(relative);
        assert!(path.is_file(), "{} is missing", path.display());
        path.to_str().unwrap().to_string()
    }

    fn assert_real_scale(relative: &str, expected_scale: f64, expected_bunit: Option<&str>) {
        let status = photometry_status_for_path(&survey_file(relative), false, None).unwrap();
        let scale = status.pixel_scale_arcsec.unwrap();
        println!("{relative}: pixel_scale_arcsec {scale:?}, bunit {:?}", status.bunit);
        assert_close(scale, expected_scale, REAL_SCALE_TOLERANCE);
        assert_eq!(status.bunit.as_deref(), expected_bunit);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1/des-dr2_A_r_extn1.fits"]
    fn real_des_d1_r_status_scale() {
        assert_real_scale("D1/des-dr2_A_r_extn1.fits", 0.263000000000016, Some("COUNTS/S"));
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits"]
    fn real_euclid_e1_vis_status_scale() {
        assert_real_scale(
            "E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits",
            0.100000000000008,
            None,
        );
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_r_stack_480px.fits"]
    fn real_ps1_p1_r_status_scale() {
        assert_real_scale("P1/ps1_A_r_stack_480px.fits", 0.2500000060535957, None);
    }

    #[test]
    fn status_with_a_typed_zero_point_reports_the_user_calibration() {
        let dir = tempfile::tempdir().unwrap();
        let path = star_file(dir.path(), "adu.fits", &[("BUNIT", "adu")], None);
        let status = photometry_status_for_path(&path, false, Some(&ab_25())).unwrap();
        assert_eq!(status.calibration_source.as_deref(), Some("user"));
        assert_eq!(status.missing_reason, None);
        let origin = status.calibration_origin.clone().unwrap();
        assert!(!origin.is_empty());
        assert_eq!(status.photcal["origin"].as_str(), Some(origin.as_str()), "{}", status.photcal);
        assert_eq!(status.warnings.first().map(String::as_str), Some(USER_ZERO_POINT_WARNING), "{:?}", status.warnings);

        let without = photometry_status_for_path(&path, false, None).unwrap();
        assert_eq!(without.calibration_source, None);
        assert!(without.missing_reason.is_some());
    }

    #[test]
    fn status_of_a_header_calibration_reports_header_and_the_origin_sentence() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = jwst_cal_mef(&dir.path().join("cal.fits"), &[], f200w_cal_cards(), CalPlanes::ALL, vec![]);
        let status = photometry_status_for_path(&fixture.key, false, None).unwrap();
        assert_eq!(status.calibration_source.as_deref(), Some("header"));
        let origin = status.photcal["origin"].as_str().unwrap();
        assert!(!origin.is_empty());
        assert_eq!(status.calibration_origin.as_deref(), Some(origin));
    }

    fn assert_real_calibration(relative: &str, expected_jy_per_unit: f64) {
        let path = survey_file(relative);
        let status = photometry_status_for_path(&path, false, None).unwrap();
        let factor = status.photcal["jy_per_native_unit"].as_f64().unwrap();
        let origin = status.calibration_origin.clone().unwrap_or_default();
        println!("{relative}: jy_per_native_unit {factor:e} Jy (expected {expected_jy_per_unit:e}), origin '{origin}'");
        assert!(
            ((factor - expected_jy_per_unit) / expected_jy_per_unit).abs() <= REAL_FACTOR_TOLERANCE,
            "{factor:e} vs {expected_jy_per_unit:e}"
        );
        assert_eq!(status.calibration_source.as_deref(), Some("header"));
        assert!(!origin.is_empty());
        let typed = photometry_status_for_path(&path, false, Some(&ab_25())).unwrap();
        assert_eq!(typed.calibration_source.as_deref(), Some("header"));
        assert_eq!(typed.photcal, status.photcal);
        assert!(typed.warnings.iter().any(|w| w.starts_with(NOT_APPLIED_PREFIX)), "{:?}", typed.warnings);
    }

    #[test]
    fn status_reads_the_survey_layers_in_the_words_of_the_layer_reader() {
        let dir = tempfile::tempdir().unwrap();
        let inverse_variance = inverse_variance_star_mef(&dir.path().join("ivar.fits"));
        let origin = load_companions(&inverse_variance).unwrap().err_origin.expect("the IVAR layer is the error layer");
        let status = photometry_status_for_path(&inverse_variance, false, None).unwrap();
        assert_eq!(status.err, Some(StatusErr { origin: format!("{}, 3 pixels without data", origin.source) }));

        let unnamed = unnamed_flag_star_mef(&dir.path().join("flag.fits"));
        let flags = load_companions(&unnamed).unwrap().flags.expect("the FLAG layer is the mask").source;
        let dq = photometry_status_for_path(&unnamed, false, None).unwrap().dq.expect("the FLAG layer is reported");
        assert!(!dq.exclusion_available);
        assert_eq!(
            dq.unavailable_reason,
            Some(format!("the flag layer has no usable bit names ({flags}), so nothing is excluded until bits are chosen"))
        );

        let afw = afw_star_mef(&dir.path().join("calexp.fits"), flat("VARIANCE", 4.0));
        let dq = photometry_status_for_path(&afw, false, None).unwrap().dq.expect("the MASK layer is reported");
        assert!(dq.exclusion_available);
        assert_eq!(dq.unavailable_reason, None);
        assert!(dq.source.ends_with(", MP_ cards (afw mask planes) flags"), "{}", dq.source);
    }

    #[test]
    fn the_status_filter_is_the_answer_of_the_filter_command() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = jwst_cal_mef(&dir.path().join("cal.fits"), &[], f200w_cal_cards(), CalPlanes::ALL, vec![]);
        let header = cached_header(&fixture.key).unwrap();
        let answer = identify_filter_in(&header, &fixture.key, None).unwrap();
        assert_eq!(
            photometry_status_for_path(&fixture.key, false, None).unwrap().filter,
            Some(StatusFilter {
                dataset: answer.dataset,
                filter: answer.filter,
                pivot_um: answer.pivot_um,
                source: answer.source,
                warning: answer.warning,
            })
        );

        let cutout = legacy_band_cutout(&dir.path().join("cut.fits"), SIZE);
        let band = photometry_status_for_path(&cutout, false, None).unwrap().filter.expect("the band of the plane");
        assert_eq!(band.dataset, "DECAM");
        assert_eq!(band.filter, "r");
        assert_eq!(band.source, "header BAND1");
        let warning = band.warning.unwrap_or_default();
        assert!(warning.contains("dataset DECAM is assumed from SURVEY = LegacySurvey"), "{warning}");

        let plain = star_file(dir.path(), "plain.fits", &[], None);
        assert_eq!(photometry_status_for_path(&plain, false, None).unwrap().filter, None);
    }

    #[test]
    fn the_status_filter_keeps_the_warning_of_the_filter_answer() {
        let skipped = "custom filter dataset EUCLID is not used: EUCLID is a built-in dataset; rename it in filter_datasets to keep its filters";
        let answer = |warning: Option<&str>| IdentifiedFilter {
            dataset: "EUCLID".into(),
            filter: "VIS".into(),
            pivot_um: 0.71034,
            instrument: "VIS".into(),
            source: "header FILTER".into(),
            warning: warning.map(str::to_string),
        };
        let status = |warning: Option<&str>| StatusFilter {
            dataset: "EUCLID".into(),
            filter: "VIS".into(),
            pivot_um: 0.71034,
            source: "header FILTER".into(),
            warning: warning.map(str::to_string),
        };
        assert_eq!(status_filter_of(answer(Some(skipped))), status(Some(skipped)));
        assert_eq!(status_filter_of(answer(None)), status(None));
    }

    #[test]
    fn status_of_a_sibling_error_layer_names_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let image = legacy_brick(dir.path(), Some(SIZE));
        let source = load_companions(&image).unwrap().err_origin.expect("the invvar sibling is the error layer").source;
        let err = photometry_status_for_path(&image, false, None).unwrap().err.expect("the status names the error layer");
        assert_eq!(err.origin, source);
        assert!(err.origin.contains(LEGACY_BRICK_INVVAR), "{}", err.origin);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L1/ls-dr10_A_griz_invvar_maskbits_458px.fits"]
    fn real_legacy_l1_cutout_r_plane_status() {
        let key = BandPlaneKey::new(&survey_file("L1/ls-dr10_A_griz_invvar_maskbits_458px.fits"), 0, 1).key();
        let status = photometry_status_for_path(&key, false, None).unwrap();
        let has_err = has_err_extension(&key, (458, 458));
        println!("legacy l1 r plane: filter {:?}, err {:?}, has_err_extension {has_err}", status.filter, status.err);
        let filter = status.filter.expect("plan/goldens/survey/phot_legacy_l1.json band_addressing: plane 1 is r");
        assert_eq!(filter.dataset, "DECAM");
        assert_eq!(filter.filter, "r");
        assert_eq!(filter.source, "header BAND1");
        let err = status.err.expect("the r plane of the INVVAR cube is the error layer");
        assert!(err.origin.contains("HDU 1 plane 1"), "{}", err.origin);
        assert!(has_err);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1/des-dr2_A_r_extn1.fits"]
    fn real_des_d1_r_status_calibration() {
        assert_real_calibration("D1/des-dr2_A_r_extn1.fits", 3.6307805477010066e-09);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits"]
    fn real_euclid_e1_vis_status_calibration() {
        assert_real_calibration(
            "E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits",
            5.248074602497712e-07,
        );
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_r_stack_480px.fits"]
    fn real_ps1_p1_r_status_calibration() {
        assert_real_calibration("P1/ps1_A_r_stack_480px.fits", 6.051300912835008e-10);
    }
}
