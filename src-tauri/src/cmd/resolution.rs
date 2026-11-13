use std::time::Instant;

use serde::Serialize;

use crate::cmd::analysis::photometry_context;
use crate::cmd::common::blocking_cmd;
use crate::core::analysis::photometry::saturation_level;
use crate::core::analysis::resolution::{
    anisotropy_warning, area_pixel_scale_arcsec, classify, image_psf, seeing_cards, ImagePsfStar, ResolutionInput,
    SeeingCard, DEFAULT_MIN_PIXELS_PER_FWHM, IMAGE_PSF_DEFAULT_MAX_STARS, IMAGE_PSF_MAX_STARS_LIMIT, IMAGE_PSF_MODEL,
};

#[derive(Debug, Clone, Serialize)]
pub struct ImagePsfReport {
    pub fwhm_px: f64,
    pub fwhm_px_scatter: Option<f64>,
    pub fwhm_arcsec: Option<f64>,
    pub fwhm_arcsec_scatter: Option<f64>,
    pub half_light_radius_px: Option<f64>,
    pub half_light_radius_arcsec: Option<f64>,
    pub n_stars: usize,
    pub n_measured: usize,
    pub n_candidates: usize,
    pub n_detected: usize,
    pub max_stars: usize,
    pub model: String,
    pub pixel_scale_arcsec: Option<f64>,
    pub pixels_per_fwhm: f64,
    pub undersampled: bool,
    pub header_values: Vec<SeeingCard>,
    pub stars: Vec<ImagePsfStar>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

pub(crate) fn image_psf_for_path(path: &str, max_stars: Option<usize>) -> anyhow::Result<ImagePsfReport> {
    let started = Instant::now();
    let max_stars = max_stars.unwrap_or(IMAGE_PSF_DEFAULT_MAX_STARS);
    if !(1..=IMAGE_PSF_MAX_STARS_LIMIT).contains(&max_stars) {
        anyhow::bail!("maxStars must be between 1 and {IMAGE_PSF_MAX_STARS_LIMIT}");
    }
    let ctx = photometry_context(path, true)?;
    let header = ctx.entry.header();
    let psf = image_psf(
        ctx.entry.arr(),
        ctx.mask.as_ref().map(|m| &m.map),
        ctx.planes.saturated.as_ref(),
        Some(saturation_level(header, ctx.entry.stats().max)),
        max_stars,
    )
    .map_err(anyhow::Error::msg)?;

    let cd = ctx.wcs.as_ref().map(|w| w.raw_params().4);
    let pixel_scale_arcsec = cd.as_ref().and_then(area_pixel_scale_arcsec);
    let in_arcsec = |pixels: Option<f64>| pixels.zip(pixel_scale_arcsec).map(|(value, scale)| value * scale);
    let header_values = header.map(seeing_cards).unwrap_or_default();

    let mut notes = vec![
        format!("PSF FWHM = median over {} stars of the {IMAGE_PSF_MODEL}; scatter = 1.4826 x MAD", psf.n_stars),
        format!("stars: detections at 5 sigma with a peak of at least 50 sigma, eccentricity up to 0.6, at least 16 px from the edges, no neighbour above 5 percent of their flux within 12 px, unsaturated and on unmasked pixels; of the {max_stars} brightest, those between 0.80 and 1.10 of the 25th percentile of the widths are kept"),
    ];
    if pixel_scale_arcsec.is_none() {
        notes.push("no celestial WCS: the FWHM is given in pixels only".to_string());
    }
    if !header_values.is_empty() {
        notes.push("header cards read as arcsec: SEEING, PSF_FWHM, PSFFWHM".to_string());
    }
    let mut warnings = psf.warnings;
    warnings.extend(cd.as_ref().and_then(anisotropy_warning));

    Ok(ImagePsfReport {
        fwhm_px: psf.fwhm_px,
        fwhm_px_scatter: psf.fwhm_px_scatter,
        fwhm_arcsec: in_arcsec(Some(psf.fwhm_px)),
        fwhm_arcsec_scatter: in_arcsec(psf.fwhm_px_scatter),
        half_light_radius_px: psf.half_light_radius_px,
        half_light_radius_arcsec: in_arcsec(psf.half_light_radius_px),
        n_stars: psf.n_stars,
        n_measured: psf.n_measured,
        n_candidates: psf.n_candidates,
        n_detected: psf.n_detected,
        max_stars,
        model: IMAGE_PSF_MODEL.to_string(),
        pixel_scale_arcsec,
        pixels_per_fwhm: psf.fwhm_px,
        undersampled: psf.fwhm_px < DEFAULT_MIN_PIXELS_PER_FWHM,
        header_values,
        stars: psf.stars,
        notes,
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

#[tauri::command]
pub async fn image_psf_fwhm_cmd(path: String, max_stars: Option<usize>) -> Result<serde_json::Value, String> {
    blocking_cmd!(Ok(serde_json::to_value(&image_psf_for_path(&path, max_stars)?)?))
}

#[tauri::command]
pub async fn resolution_check_cmd(request: ResolutionInput) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let check = classify(&request).map_err(anyhow::Error::msg)?;
        Ok(serde_json::to_value(&check)?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::core::analysis::photometry::{measure_star_full, PhotometryConfig};
    use crate::core::analysis::resolution::test_support::star_field;
    use crate::core::analysis::resolution::Verdict;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::infra::image_source::load_plane;
    use crate::types::header::HduHeader;
    use crate::types::image_ref::ImageRef;

    const FWHM_OF_SIGMA_1_8: f64 = 4.238676081055709;
    const LEGACY_PIXEL_SCALE_ARCSEC: f64 = 0.262;
    const LEGACY_STARS_MOFFAT_FWHM_ARCSEC: f64 = 0.8907913080108724;

    fn wcs_header(cd1_1_arcsec: f64, cd2_2_arcsec: f64) -> HduHeader {
        let mut header = HduHeader::empty();
        header.set("CTYPE1", "RA---TAN".to_string());
        header.set("CTYPE2", "DEC--TAN".to_string());
        header.set_f64("CRPIX1", 200.5);
        header.set_f64("CRPIX2", 200.5);
        header.set_f64("CRVAL1", 150.0);
        header.set_f64("CRVAL2", 2.0);
        header.set_f64("CD1_1", -cd1_1_arcsec / 3600.0);
        header.set_f64("CD1_2", 0.0);
        header.set_f64("CD2_1", 0.0);
        header.set_f64("CD2_2", cd2_2_arcsec / 3600.0);
        header
    }

    fn write_field(dir: &std::path::Path, name: &str, header: Option<&HduHeader>) -> String {
        let path = dir.join(name);
        crate::infra::fits::writer::write_fits_mono(path.to_str().unwrap(), &star_field(1.8, 0, None), header).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn field_file(dir: &std::path::Path, name: &str, arcsec_per_px: Option<f64>) -> String {
        let header = arcsec_per_px.map(|scale| wcs_header(scale, scale));
        write_field(dir, name, header.as_ref())
    }

    fn assert_relative(value: f64, expected: f64, tolerance: f64) {
        assert!(
            ((value - expected) / expected).abs() <= tolerance,
            "{value} is not within {tolerance} relative of {expected}"
        );
    }

    #[test]
    fn the_image_psf_is_reported_in_arcsec_with_the_pixel_area_scale() {
        let dir = tempfile::tempdir().unwrap();
        let path = field_file(dir.path(), "scaled.fits", Some(0.2));
        let report = image_psf_for_path(&path, None).unwrap();
        let scale = report.pixel_scale_arcsec.unwrap();
        assert!((scale - 0.2).abs() <= 1e-9, "scale {scale}");
        assert_relative(report.fwhm_arcsec.unwrap(), report.fwhm_px * scale, 1e-12);
        assert_relative(report.fwhm_px, FWHM_OF_SIGMA_1_8, 0.01);
        assert_eq!(report.pixels_per_fwhm, report.fwhm_px);
        assert!(!report.undersampled);
        assert_eq!(report.model, IMAGE_PSF_MODEL);
        assert_eq!(report.max_stars, 200);
        assert!(report.header_values.is_empty(), "{:?}", report.header_values);
        assert!(report.notes[0].starts_with("PSF FWHM = median over"), "{:?}", report.notes);
    }

    #[test]
    fn without_a_wcs_the_psf_is_in_pixels_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = field_file(dir.path(), "plain.fits", None);
        let report = image_psf_for_path(&path, None).unwrap();
        assert_eq!(report.fwhm_arcsec, None);
        assert_eq!(report.fwhm_arcsec_scatter, None);
        assert_eq!(report.half_light_radius_arcsec, None);
        assert_eq!(report.pixel_scale_arcsec, None);
        assert!(
            report.notes.iter().any(|note| note == "no celestial WCS: the FWHM is given in pixels only"),
            "{:?}",
            report.notes
        );
    }

    #[test]
    fn rectangular_pixels_use_the_area_scale_and_warn() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_field(dir.path(), "rectangular.fits", Some(&wcs_header(0.29, 0.68)));
        let report = image_psf_for_path(&path, None).unwrap();
        let scale = report.pixel_scale_arcsec.unwrap();
        assert!((scale - 0.4440720662234904).abs() <= 1e-9, "scale {scale}");
        assert!(
            report.warnings.iter().any(|w| w.starts_with("pixel scales differ per axis (0.29000 x 0.68000 arcsec)")),
            "{:?}",
            report.warnings
        );
    }

    #[test]
    fn max_stars_outside_its_range_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = field_file(dir.path(), "limits.fits", Some(0.2));
        for refused in [0, 2001] {
            let error = image_psf_for_path(&path, Some(refused)).unwrap_err();
            assert_eq!(error.to_string(), "maxStars must be between 1 and 2000");
        }
        let report = image_psf_for_path(&path, Some(5)).unwrap();
        assert!(report.n_candidates <= 5, "n_candidates {}", report.n_candidates);
    }

    #[test]
    fn a_seeing_card_is_listed_beside_the_measurement() {
        let dir = tempfile::tempdir().unwrap();
        let mut header = wcs_header(0.2, 0.2);
        header.set_f64("SEEING", 0.95);
        let path = write_field(dir.path(), "seeing.fits", Some(&header));
        let report = image_psf_for_path(&path, None).unwrap();
        assert_eq!(report.header_values, vec![SeeingCard { key: "SEEING".to_string(), value_arcsec: 0.95 }]);
        assert_eq!(
            report.notes.last().map(String::as_str),
            Some("header cards read as arcsec: SEEING, PSF_FWHM, PSFFWHM")
        );
    }

    #[tokio::test]
    async fn the_check_command_returns_the_rule_and_its_refusals() {
        let request = json!({"psf_fwhm_arcsec":1.14,"pixel_scale_arcsec":0.2,"source_fwhm_arcsec":1.2,"snr":50.0,"redshift":1.0});
        let check = resolution_check_cmd(serde_json::from_value(request).unwrap()).await.unwrap();
        assert_eq!(check["verdict"], "marginal");
        assert_relative(check["kpc_per_arcsec"].as_f64().unwrap(), 8.23125144264795, 1e-9);
        assert_relative(check["psf_fwhm_kpc"].as_f64().unwrap(), 9.383626644618662, 1e-9);

        let refused = resolution_check_cmd(serde_json::from_value(json!({"psf_fwhm_arcsec":0.0})).unwrap()).await;
        assert_eq!(refused, Err("PSF FWHM must be a positive number of arcsec".to_string()));

        let empty = resolution_check_cmd(serde_json::from_value(json!({})).unwrap()).await.unwrap();
        assert!(empty["verdict"].is_null(), "{}", empty["verdict"]);
    }

    fn legacy_r_image_key() -> String {
        let path = env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("survey").join("L2").join("legacysurvey-0532m280-image-r.fits.fz");
        assert!(path.is_file(), "{} is missing", path.display());
        format!("{}#hdu=1", path.to_str().unwrap())
    }

    fn legacy_fitted_rows() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("survey_qa")
            .join("legacy_l2_r_fitted_rows.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-image-r.fits.fz"]
    fn real_legacy_l2_r_image_psf_is_within_5_percent_of_the_moffat_fwhm_of_its_stars() {
        let report = image_psf_for_path(&legacy_r_image_key(), None).unwrap();
        println!("fwhm_px {}", report.fwhm_px);
        println!("fwhm_arcsec {:?}", report.fwhm_arcsec);
        println!("fwhm_px_scatter {:?}", report.fwhm_px_scatter);
        println!("fwhm_arcsec_scatter {:?}", report.fwhm_arcsec_scatter);
        println!("half_light_radius_arcsec {:?}", report.half_light_radius_arcsec);
        println!("pixel_scale_arcsec {:?}", report.pixel_scale_arcsec);
        println!(
            "n_stars {} n_measured {} n_candidates {} n_detected {}",
            report.n_stars, report.n_measured, report.n_candidates, report.n_detected
        );
        println!("elapsed_ms {}", report.elapsed_ms);
        println!("warnings {:?}", report.warnings);
        assert_relative(report.fwhm_arcsec.unwrap(), LEGACY_STARS_MOFFAT_FWHM_ARCSEC, 0.05);
        assert!((report.pixel_scale_arcsec.unwrap() - LEGACY_PIXEL_SCALE_ARCSEC).abs() <= 1e-6);
        assert!(!report.undersampled);
        assert!(report.n_stars >= 30, "n_stars {}", report.n_stars);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-image-r.fits.fz"]
    fn real_legacy_l2_r_psf_sources_are_not_classed_resolved_with_the_image_psf() {
        let key = legacy_r_image_key();
        let psf_fwhm_arcsec = image_psf_for_path(&key, None).unwrap().fwhm_arcsec.unwrap();
        let image = load_plane(&ImageRef::parse(&key)).unwrap().arr;
        let fixture = legacy_fitted_rows();
        let psf_rows: Vec<&serde_json::Value> =
            fixture["rows"].as_array().unwrap().iter().filter(|row| row["type"] == "PSF").collect();
        let (mut unresolved, mut marginal, mut resolved) = (0, 0, 0);
        for row in &psf_rows {
            let (x, y) = (row["x"].as_f64().unwrap(), row["y"].as_f64().unwrap());
            let Some(fwhm_px) = measure_star_full(&image, None, None, None, x, y, &PhotometryConfig::default())
                .ok()
                .and_then(|measured| measured.fwhm)
            else {
                continue;
            };
            let check = classify(&ResolutionInput {
                psf_fwhm_arcsec: Some(psf_fwhm_arcsec),
                pixel_scale_arcsec: Some(LEGACY_PIXEL_SCALE_ARCSEC),
                source_fwhm_arcsec: Some(fwhm_px * LEGACY_PIXEL_SCALE_ARCSEC),
                snr: row["snr"].as_f64(),
                ..Default::default()
            })
            .unwrap();
            match check.verdict.unwrap() {
                Verdict::Unresolved => unresolved += 1,
                Verdict::Marginal => marginal += 1,
                Verdict::Resolved => resolved += 1,
            }
        }
        println!("psf_fwhm_arcsec {psf_fwhm_arcsec}");
        println!("psf rows {} unresolved {unresolved} marginal {marginal} resolved {resolved}", psf_rows.len());
        assert_eq!(resolved, 0);
        assert!(unresolved + marginal >= 20, "{} of {} measured", unresolved + marginal, psf_rows.len());
    }

    const EUCLID_E1_VIS_IMAGE: &str = "EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits";

    fn survey_image_psf(set: &str, name: &str) -> ImagePsfReport {
        let path = env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("survey").join(set).join(name);
        assert!(path.is_file(), "{} is missing", path.display());
        let report = image_psf_for_path(path.to_str().unwrap(), None).unwrap();
        println!("fwhm_px {} fwhm_px_scatter {:?}", report.fwhm_px, report.fwhm_px_scatter);
        println!("fwhm_arcsec {:?} fwhm_arcsec_scatter {:?}", report.fwhm_arcsec, report.fwhm_arcsec_scatter);
        println!(
            "half_light_radius_px {:?} half_light_radius_arcsec {:?}",
            report.half_light_radius_px, report.half_light_radius_arcsec
        );
        println!("pixel_scale_arcsec {:?} undersampled {}", report.pixel_scale_arcsec, report.undersampled);
        println!(
            "n_stars {} n_measured {} n_candidates {} n_detected {}",
            report.n_stars, report.n_measured, report.n_candidates, report.n_detected
        );
        println!("header_values {:?}", report.header_values);
        println!("elapsed_ms {}", report.elapsed_ms);
        println!("warnings {:?}", report.warnings);
        report
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/S1/frame-r-001462-4-0165.fits.bz2"]
    fn real_sdss_s1_r_image_psf_is_within_5_percent_of_the_moffat_fwhm_of_its_stars() {
        let report = survey_image_psf("S1", "frame-r-001462-4-0165.fits.bz2");
        assert_relative(report.fwhm_arcsec.unwrap(), 0.9918283786226811, 0.05);
        assert!((report.pixel_scale_arcsec.unwrap() - 0.39603565421543135).abs() <= 1e-6);
        assert!(report.n_stars >= 20, "n_stars {}", report.n_stars);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1/des-dr2_A_r_extn1.fits"]
    fn real_des_d1_r_image_psf_is_reported_against_two_catalogue_stars() {
        let report = survey_image_psf("D1", "des-dr2_A_r_extn1.fits");
        assert_relative(report.fwhm_arcsec.unwrap(), 0.9972773536167278, 0.05);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_r_stack_480px.fits"]
    fn real_ps1_p1_r_image_psf_is_reported_against_three_catalogue_stars() {
        let report = survey_image_psf("P1", "ps1_A_r_stack_480px.fits");
        assert_relative(report.fwhm_arcsec.unwrap(), 1.095679709279208, 0.05);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1/EUC_MER_BGSUB-MOSAIC-VIS of tile 102044185, cut60_A"]
    fn real_euclid_e1_vis_image_psf_is_undersampled() {
        let report = survey_image_psf("E1", EUCLID_E1_VIS_IMAGE);
        assert!(report.undersampled, "fwhm_px {}", report.fwhm_px);
        assert_relative(report.fwhm_arcsec.unwrap(), 0.17663025155773465, 0.05);
    }
}
