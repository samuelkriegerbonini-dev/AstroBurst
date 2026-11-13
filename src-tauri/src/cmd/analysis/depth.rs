use std::time::Instant;

use anyhow::bail;
use ndarray::{Array2, Zip};
use serde::Serialize;
use serde_json::Value;

use super::{photometry_context, DQ_NOT_APPLIED_PREFIX};
use crate::cmd::common::{blocking_cmd, load_companions};
use crate::core::analysis::depth::{
    depth_statistics, draw_aperture_positions, empty_aperture_sums, excluded_fraction, formal_point_source_limit,
    source_exclusion_mask, unmasked_pixel_noise, EmptyAperture, SOURCE_GROW_RADIUS_PX, SOURCE_MIN_PIXELS,
    SOURCE_SMOOTH_FWHM_PX,
};
use crate::core::analysis::photometry::PhotometryConfig;
use crate::core::analysis::resolution::{anisotropy_warning, area_pixel_scale_arcsec};
use crate::core::imaging::dq_flags::exclusion_map;
use crate::core::metadata::photcal::card_text;
use crate::types::constants::{HEADER_BUNIT, RES_LABEL};

pub const DEPTH_DEFAULT_RADIUS_ARCSEC: f64 = 1.0;
pub const DEPTH_DEFAULT_APERTURES: usize = 500;
pub const DEPTH_DEFAULT_NSIGMA: f64 = 5.0;
pub const DEPTH_DEFAULT_SEED: u64 = 1;
pub const DEPTH_DEFAULT_DETECT_NSIGMA: f64 = 3.0;
pub const DEPTH_MIN_USABLE_APERTURES: usize = 200;
pub const DEPTH_LOW_DETECT_NSIGMA: f64 = 2.5;
pub const DEPTH_NOISE_SPREAD_WARNING: f64 = 1.5;
pub const DEPTH_SOURCE_PLANES: [&str; 2] = ["DETECTED", "DETECTED_NEGATIVE"];

#[derive(Debug, Clone, Default)]
pub struct DepthArgs {
    pub radius_arcsec: Option<f64>,
    pub radius_px: Option<f64>,
    pub n_apertures: Option<usize>,
    pub n_sigma: Option<f64>,
    pub seed: Option<u64>,
    pub detect_nsigma: Option<f64>,
    pub psf_fwhm_arcsec: Option<f64>,
    pub exclude_dq: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DepthReport {
    pub radius_arcsec: Option<f64>,
    pub radius_px: f64,
    pub pixel_scale_arcsec: Option<f64>,
    pub n_requested: usize,
    pub n_placed: usize,
    pub n_used: usize,
    pub seed: u64,
    pub n_sigma: f64,
    pub detect_nsigma: f64,
    pub masked_fraction: f64,
    pub median_sum_native: f64,
    pub sigma_empty_native: f64,
    pub sigma_clipped_native: f64,
    pub n_after_clip: usize,
    pub sigma_empty_rel_err: f64,
    pub sigma_formal_native: Option<f64>,
    pub f_corr: Option<f64>,
    pub limit_native: f64,
    pub limit_jy: Option<f64>,
    pub limit_ab: Option<f64>,
    pub psf_fwhm_arcsec: Option<f64>,
    pub psf_limit_formal_native: Option<f64>,
    pub psf_limit_formal_ab: Option<f64>,
    pub sigma_pix_median_native: Option<f64>,
    pub sigma_pix_spread: Option<f64>,
    pub bunit: Option<String>,
    pub calibration_origin: Option<String>,
    pub err_origin: Option<String>,
    pub dq_source: Option<String>,
    pub definition: String,
    pub apertures: Vec<EmptyAperture>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
    pub detected_plane_pixels: Option<u64>,
}

fn is_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn check_depth_args(args: &DepthArgs) -> anyhow::Result<()> {
    if args.radius_arcsec.is_some() && args.radius_px.is_some() {
        bail!("Give the aperture radius in arcsec or in pixels, not both");
    }
    if args.radius_arcsec.or(args.radius_px).is_some_and(|radius| !is_positive(radius)) {
        bail!("Aperture radius must be a positive number");
    }
    if args.n_apertures.is_some_and(|count| !(10..=5000).contains(&count)) {
        bail!("The number of apertures must be between 10 and 5000");
    }
    if args.n_sigma.is_some_and(|n_sigma| !(is_positive(n_sigma) && n_sigma <= 100.0)) {
        bail!("The significance must be above 0 and at most 100 sigma");
    }
    if args.detect_nsigma.is_some_and(|detect_nsigma| !(1.0..=10.0).contains(&detect_nsigma)) {
        bail!("The source detection threshold must be between 1 and 10 sigma");
    }
    if args.psf_fwhm_arcsec.is_some_and(|fwhm| !is_positive(fwhm)) {
        bail!("PSF FWHM must be a positive number of arcsec");
    }
    Ok(())
}

fn detected_source_map(path: &str, dims: (usize, usize)) -> Option<Array2<u8>> {
    let companions = load_companions(path).ok()?;
    let flags = companions.flags.as_ref()?;
    let (entry, _) = companions.dq.as_ref()?;
    let plane = entry.int_plane().filter(|plane| plane.bits.dim() == dims)?;
    let bits = DEPTH_SOURCE_PLANES
        .iter()
        .filter_map(|name| flags.mask_from_names(&[name]).ok())
        .fold(0, |known, bit| known | bit);
    (bits != 0).then(|| exclusion_map(&plane.bits, bits))
}

pub(crate) fn depth_for_path(path: &str, args: &DepthArgs) -> anyhow::Result<DepthReport> {
    let started = Instant::now();
    check_depth_args(args)?;
    let n_requested = args.n_apertures.unwrap_or(DEPTH_DEFAULT_APERTURES);
    let n_sigma = args.n_sigma.unwrap_or(DEPTH_DEFAULT_NSIGMA);
    let seed = args.seed.unwrap_or(DEPTH_DEFAULT_SEED);
    let detect_nsigma = args.detect_nsigma.unwrap_or(DEPTH_DEFAULT_DETECT_NSIGMA);

    let ctx = photometry_context(path, args.exclude_dq.unwrap_or(true))?;
    let image = ctx.entry.arr();
    let sigma = ctx.planes.err.as_ref().map(|e| e.arr());
    let context_bad = ctx.mask.as_ref().map(|m| &m.map);
    let detected = detected_source_map(path, image.dim());
    let detected_plane_pixels = detected.as_ref().map(|map| map.iter().filter(|&&flag| flag != 0).count() as u64);
    let context_or_detected = context_bad.zip(detected.as_ref()).map(|(context, detected)| {
        Zip::from(context).and(detected).map_collect(|&context, &detected| u8::from(context != 0 || detected != 0))
    });
    let bad = context_or_detected.as_ref().or(detected.as_ref()).or(context_bad);
    let cd = ctx.wcs.as_ref().map(|w| w.raw_params().4);
    let scale = cd.as_ref().and_then(area_pixel_scale_arcsec);

    let (radius_px, radius_arcsec, radius_text) = match args.radius_px {
        Some(radius_px) => (radius_px, scale.map(|scale| radius_px * scale), format!("{radius_px:.2} px")),
        None => {
            let radius_arcsec = args.radius_arcsec.unwrap_or(DEPTH_DEFAULT_RADIUS_ARCSEC);
            let Some(scale) = scale else {
                bail!("The image has no celestial WCS: give the aperture radius in pixels");
            };
            (radius_arcsec / scale, Some(radius_arcsec), format!("{radius_arcsec:.2} arcsec"))
        }
    };
    if !(1.0..=100.0).contains(&radius_px) {
        bail!("Aperture radius of {radius_px:.2} px is outside 1 to 100 px");
    }

    let subsamples = PhotometryConfig::default().subsamples;
    let mask = source_exclusion_mask(image, sigma, bad, radius_px, detect_nsigma);
    let masked_fraction = excluded_fraction(&mask);
    let positions = draw_aperture_positions(&mask, radius_px, n_requested, seed);
    let n_placed = positions.len();
    let apertures = empty_aperture_sums(image, sigma, Some(&mask), &positions, radius_px, subsamples);
    let Some(stats) = depth_statistics(&apertures, n_sigma) else {
        let n = apertures.iter().filter(|aperture| aperture.usable).count();
        bail!(
            "Only {n} empty aperture{} could be placed: the image has too little source-free sky for a radius of {radius_px:.2} px",
            if n == 1 { "" } else { "s" }
        );
    };

    let calibrated = ctx.photcal.as_ref().and_then(|cal| cal.calibrate(stats.limit, None));
    let limit_jy = calibrated.as_ref().map(|flux| flux.flux_jy);
    let limit_ab = calibrated.and_then(|flux| flux.mag_ab);
    let photcal = ctx.photcal_json()?;
    let calibration_origin = photcal
        .get("origin")
        .and_then(Value::as_str)
        .or_else(|| photcal.get(RES_LABEL).and_then(Value::as_str))
        .map(str::to_string);
    let bunit = ctx.entry.header().and_then(|header| card_text(header, HEADER_BUNIT));

    let noise = sigma.and_then(|plane| unmasked_pixel_noise(plane, Some(&mask)));
    let sigma_pix_spread = noise.map(|noise| noise.p84 / noise.p16);
    let psf_limit_formal_native = match (args.psf_fwhm_arcsec, scale, noise) {
        (Some(fwhm_arcsec), Some(scale), Some(noise)) => formal_point_source_limit(noise.median, fwhm_arcsec / scale, n_sigma),
        _ => None,
    };
    let psf_limit_formal_ab =
        psf_limit_formal_native.and_then(|limit| ctx.photcal.as_ref()?.calibrate(limit, None)?.mag_ab);

    let mut notes = vec![
        format!(
            "limit = n_sigma x 1.4826 x MAD of the sums of non-overlapping apertures on source-free sky; the sums use the {subsamples} x {subsamples} sub-pixel weights of the photometry"
        ),
        format!(
            "source mask: image over its error map (or over the robust pixel scatter) smoothed with a Gaussian of FWHM {SOURCE_SMOOTH_FWHM_PX} px; groups of at least {SOURCE_MIN_PIXELS} pixels above the median plus {detect_nsigma} clipped standard deviations, grown by {SOURCE_GROW_RADIUS_PX} px"
        ),
    ];
    if let Some(n) = detected_plane_pixels.filter(|&n| n > 0) {
        notes.push(format!("{n} pixels of the mask planes DETECTED and DETECTED_NEGATIVE are treated as sources"));
    }
    notes.push(match sigma {
        Some(_) => "formal error of an aperture = sqrt(sum of (weight x sigma)^2); sigma_formal is its median over the apertures; f_corr = sigma_empty / sigma_formal".to_string(),
        None => "no error layer: the formal error, f_corr and the formal point-source limit are not available".to_string(),
    });
    if psf_limit_formal_native.is_some() {
        notes.push("formal point-source limit = n_sigma x median pixel sigma x sqrt(pi FWHM^2 / (2 ln 2)); it carries no correlated-noise correction".to_string());
    }
    if args.psf_fwhm_arcsec.is_some() && scale.is_none() {
        notes.push("no celestial WCS: the formal point-source limit needs the PSF FWHM in pixels and was not computed".to_string());
    }
    if n_placed < n_requested {
        notes.push(format!("{n_placed} of the {n_requested} requested apertures fit on the source-free sky"));
    }

    let mut warnings: Vec<String> = ctx
        .warnings
        .iter()
        .filter(|warning| args.exclude_dq.is_some() || !warning.starts_with(DQ_NOT_APPLIED_PREFIX))
        .cloned()
        .collect();
    if stats.n_used < DEPTH_MIN_USABLE_APERTURES {
        warnings.push(format!(
            "only {} empty apertures were usable (fewer than {DEPTH_MIN_USABLE_APERTURES}): the scatter is uncertain by {:.0} percent",
            stats.n_used,
            100.0 * stats.sigma_empty_rel_err
        ));
    }
    if detect_nsigma < DEPTH_LOW_DETECT_NSIGMA {
        warnings.push(format!(
            "a source mask below {DEPTH_LOW_DETECT_NSIGMA} sigma also removes positive noise peaks, so the depth reads too deep"
        ));
    }
    if let Some(spread) = sigma_pix_spread.filter(|&spread| spread > DEPTH_NOISE_SPREAD_WARNING) {
        warnings.push(format!(
            "the error layer varies across the image: its 84th percentile is {spread:.2} times its 16th; one depth does not describe every part of the image"
        ));
    }
    if args.radius_px.is_none() {
        warnings.extend(cd.as_ref().and_then(anisotropy_warning));
    }

    Ok(DepthReport {
        radius_arcsec,
        radius_px,
        pixel_scale_arcsec: scale,
        n_requested,
        n_placed,
        n_used: stats.n_used,
        seed,
        n_sigma,
        detect_nsigma,
        masked_fraction,
        median_sum_native: stats.median_sum,
        sigma_empty_native: stats.sigma_empty,
        sigma_clipped_native: stats.sigma_clipped,
        n_after_clip: stats.n_after_clip,
        sigma_empty_rel_err: stats.sigma_empty_rel_err,
        sigma_formal_native: stats.sigma_formal,
        f_corr: stats.f_corr,
        limit_native: stats.limit,
        limit_jy,
        limit_ab,
        psf_fwhm_arcsec: args.psf_fwhm_arcsec,
        psf_limit_formal_native,
        psf_limit_formal_ab,
        sigma_pix_median_native: noise.map(|noise| noise.median),
        sigma_pix_spread,
        bunit,
        calibration_origin,
        err_origin: sigma.and(ctx.layers.err_source.clone()),
        dq_source: context_bad.and(ctx.layers.dq_source.clone()),
        definition: format!(
            "{n_sigma} sigma, empty apertures of radius {radius_text}, 1.4826 x MAD of {} sums, sources masked at {detect_nsigma} sigma, seed {seed}",
            stats.n_used
        ),
        apertures,
        notes,
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
        detected_plane_pixels,
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn image_depth_cmd(
    path: String,
    radius_arcsec: Option<f64>,
    radius_px: Option<f64>,
    n_apertures: Option<usize>,
    n_sigma: Option<f64>,
    seed: Option<u64>,
    detect_nsigma: Option<f64>,
    psf_fwhm_arcsec: Option<f64>,
    exclude_dq: Option<bool>,
) -> Result<serde_json::Value, String> {
    let args = DepthArgs { radius_arcsec, radius_px, n_apertures, n_sigma, seed, detect_nsigma, psf_fwhm_arcsec, exclude_dq };
    blocking_cmd!({
        let report = depth_for_path(&path, &args)?;
        Ok(serde_json::to_value(&report)?)
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ndarray::Array2;

    use super::*;
    use crate::cmd::common::cached_header;
    use crate::cmd::companions::set_companion_json;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::core::imaging::layer_roles::LayerRole;
    use crate::core::metadata::photcal::missing_calibration_reason;
    use crate::infra::fits::reader::test_fixtures::{write_test_mef, HduData, TestHdu};

    const NO_ERROR_LAYER_NOTE: &str = "no error layer: the formal error, f_corr and the formal point-source limit are not available";
    const LOW_DETECTION_WARNING: &str = "a source mask below 2.5 sigma also removes positive noise peaks, so the depth reads too deep";
    const DEPTH_REPORT_KEYS: [&str; 35] = [
        "radius_arcsec",
        "radius_px",
        "pixel_scale_arcsec",
        "n_requested",
        "n_placed",
        "n_used",
        "seed",
        "n_sigma",
        "detect_nsigma",
        "masked_fraction",
        "median_sum_native",
        "sigma_empty_native",
        "sigma_clipped_native",
        "n_after_clip",
        "sigma_empty_rel_err",
        "sigma_formal_native",
        "f_corr",
        "limit_native",
        "limit_jy",
        "limit_ab",
        "psf_fwhm_arcsec",
        "psf_limit_formal_native",
        "psf_limit_formal_ab",
        "sigma_pix_median_native",
        "sigma_pix_spread",
        "bunit",
        "calibration_origin",
        "err_origin",
        "dq_source",
        "definition",
        "apertures",
        "notes",
        "warnings",
        "elapsed_ms",
        "detected_plane_pixels",
    ];

    fn gaussian_noise(rows: usize, cols: usize, sigma: f64, seed: u64) -> Array2<f32> {
        let mut state = seed;
        let mut uniform = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        Array2::from_shape_fn((rows, cols), |_| {
            let g = (-2.0 * uniform().ln()).sqrt() * (2.0 * std::f64::consts::PI * uniform()).cos();
            (sigma * g) as f32
        })
    }

    fn wcs_cards(arcsec_per_px: f64, size: usize) -> Vec<(&'static str, String)> {
        let deg = arcsec_per_px / 3600.0;
        let reference_pixel = (size as f64 / 2.0 + 0.5).to_string();
        vec![
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
            ("CRPIX1", reference_pixel.clone()),
            ("CRPIX2", reference_pixel),
            ("CRVAL1", "150.0".into()),
            ("CRVAL2", "2.0".into()),
            ("CD1_1", format!("{:.12E}", -deg)),
            ("CD1_2", "0.0".into()),
            ("CD2_1", "0.0".into()),
            ("CD2_2", format!("{:.12E}", deg)),
        ]
    }

    fn pixels(plane: &Array2<f32>) -> HduData {
        HduData::F32(plane.iter().copied().collect())
    }

    fn noise_mef(
        dir: &Path,
        name: &str,
        image: &Array2<f32>,
        err: Option<&Array2<f32>>,
        extra: Vec<(&'static str, String)>,
    ) -> String {
        let (rows, cols) = image.dim();
        let mut cards = wcs_cards(0.25, cols);
        cards.extend(extra);
        let mut hdus = vec![TestHdu { extname: Some("SCI"), extver: Some(1), cols, rows, data: pixels(image), extra_cards: cards }];
        if let Some(err) = err {
            hdus.push(TestHdu { extname: Some("ERR"), extver: Some(1), cols, rows, data: pixels(err), extra_cards: vec![] });
        }
        let path = dir.join(name);
        write_test_mef(&path, &[], &hdus);
        format!("{}#hdu=1", path.to_str().unwrap())
    }

    fn flat(size: usize, value: f32) -> Array2<f32> {
        Array2::from_elem((size, size), value)
    }

    fn noise_file(dir: &Path, size: usize, sigma: f64, seed: u64, extra: Vec<(&'static str, String)>) -> String {
        noise_mef(dir, "noise.fits", &gaussian_noise(size, size, sigma, seed), Some(&flat(size, sigma as f32)), extra)
    }

    fn file_of_test_3(dir: &Path) -> String {
        noise_file(dir, 512, 2.0, 11, vec![])
    }

    fn file_of_test_6(dir: &Path) -> String {
        noise_file(dir, 512, 2.0, 11, vec![("MAGZERO", "22.5".into())])
    }

    fn args() -> DepthArgs {
        DepthArgs::default()
    }

    fn depth(key: &str, args: DepthArgs) -> DepthReport {
        depth_for_path(key, &args).unwrap()
    }

    fn refusal(key: &str, args: DepthArgs) -> String {
        format!("{:#}", depth_for_path(key, &args).unwrap_err())
    }

    fn relative_difference(measured: f64, expected: f64) -> f64 {
        ((measured - expected) / expected).abs()
    }

    fn has_warning_starting_with(report: &DepthReport, prefix: &str) -> bool {
        report.warnings.iter().any(|warning| warning.starts_with(prefix))
    }

    #[test]
    fn the_command_runs_the_core_on_the_planes_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let key = file_of_test_3(dir.path());
        let report = depth(&key, DepthArgs { radius_px: Some(4.0), ..args() });

        let image = gaussian_noise(512, 512, 2.0, 11);
        let sigma = flat(512, 2.0);
        let mask = source_exclusion_mask(&image, Some(&sigma), None, 4.0, 3.0);
        let pos = draw_aperture_positions(&mask, 4.0, 500, 1);
        let aps = empty_aperture_sums(&image, Some(&sigma), Some(&mask), &pos, 4.0, 5);
        let stats = depth_statistics(&aps, 5.0).unwrap();

        let bits = |values: [Option<f64>; 6]| values.map(|value| value.map(f64::to_bits));
        assert_eq!(
            bits([
                Some(report.sigma_empty_native),
                Some(report.sigma_clipped_native),
                report.sigma_formal_native,
                report.f_corr,
                Some(report.limit_native),
                Some(report.median_sum_native),
            ]),
            bits([Some(stats.sigma_empty), Some(stats.sigma_clipped), stats.sigma_formal, stats.f_corr, Some(stats.limit), Some(stats.median_sum)])
        );
        assert_eq!(report.n_placed, pos.len());
        assert_eq!(pos.len(), 500);
        assert_eq!(report.n_used, 500);
        assert_eq!(report.apertures, aps);
        assert_eq!(report.masked_fraction, excluded_fraction(&mask));
        assert_eq!(report.limit_native, 5.0 * report.sigma_empty_native);
        assert!((report.sigma_empty_rel_err - 0.05214510523529509).abs() <= 1e-12, "{}", report.sigma_empty_rel_err);
        assert!((report.radius_arcsec.unwrap() - 1.0).abs() <= 1e-9, "{:?}", report.radius_arcsec);
        assert!(report.err_origin.as_deref().is_some_and(|origin| origin.contains("ERR")), "{:?}", report.err_origin);
        assert_eq!(report.dq_source, None);
        assert!(!has_warning_starting_with(&report, "DQ exclusion requested"), "{:?}", report.warnings);
    }

    #[test]
    fn the_default_aperture_is_one_arcsec_and_the_definition_says_what_was_done() {
        let dir = tempfile::tempdir().unwrap();
        let key = file_of_test_3(dir.path());
        let report = depth(&key, args());
        assert_eq!(report.radius_arcsec, Some(1.0));
        assert!((report.radius_px - 4.0).abs() <= 1e-9, "{}", report.radius_px);
        assert!((report.pixel_scale_arcsec.unwrap() - 0.25).abs() <= 1e-9, "{:?}", report.pixel_scale_arcsec);
        assert_eq!(report.n_requested, 500);
        assert_eq!(report.seed, 1);
        assert_eq!(report.n_sigma, 5.0);
        assert_eq!(report.detect_nsigma, 3.0);
        assert_eq!(
            report.definition,
            format!("5 sigma, empty apertures of radius 1.00 arcsec, 1.4826 x MAD of {} sums, sources masked at 3 sigma, seed 1", report.n_used)
        );

        let in_pixels = depth(&key, DepthArgs { radius_px: Some(4.0), ..args() });
        assert_eq!(
            in_pixels.definition,
            format!("5 sigma, empty apertures of radius 4.00 px, 1.4826 x MAD of {} sums, sources masked at 3 sigma, seed 1", in_pixels.n_used)
        );
    }

    #[test]
    fn white_noise_with_a_true_error_map_gives_f_corr_one() {
        let dir = tempfile::tempdir().unwrap();
        let key = noise_file(dir.path(), 1024, 2.0, 11, vec![]);
        let report = depth(&key, DepthArgs { n_apertures: Some(2000), ..args() });
        assert_eq!(report.n_used, 2000);
        assert!((report.f_corr.unwrap() - 1.0).abs() <= 0.1, "{:?}", report.f_corr);
        assert!((13.0..=14.5).contains(&report.sigma_formal_native.unwrap()), "{:?}", report.sigma_formal_native);
        assert_eq!(report.sigma_pix_median_native, Some(2.0));
        assert_eq!(report.sigma_pix_spread, Some(1.0));
        for prefix in ["only ", "a source mask below", "the error layer varies"] {
            assert!(!has_warning_starting_with(&report, prefix), "{prefix}: {:?}", report.warnings);
        }
    }

    #[test]
    fn a_zero_point_gives_the_limit_in_jy_and_ab() {
        let dir = tempfile::tempdir().unwrap();
        let key = file_of_test_6(dir.path());
        let report = depth(&key, DepthArgs { radius_px: Some(4.0), ..args() });
        let limit = report.limit_native;
        assert!(relative_difference(report.limit_jy.unwrap(), limit * 3.6307805477010175e-6) <= 1e-9, "{:?}", report.limit_jy);
        assert!((report.limit_ab.unwrap() - (22.5 - 2.5 * limit.log10())).abs() <= 1e-9, "{:?}", report.limit_ab);
        assert!(report.calibration_origin.as_deref().is_some_and(|origin| origin.contains("MAGZERO")), "{:?}", report.calibration_origin);
    }

    #[test]
    fn without_calibration_or_error_layer_the_limit_stays_native() {
        let dir = tempfile::tempdir().unwrap();
        let key = noise_mef(dir.path(), "adu.fits", &gaussian_noise(512, 512, 2.0, 11), None, vec![("BUNIT", "'adu'".into())]);
        let report = depth(&key, args());
        assert_eq!(report.limit_jy, None);
        assert_eq!(report.limit_ab, None);
        assert_eq!(report.calibration_origin, None);
        assert_eq!(report.sigma_formal_native, None);
        assert_eq!(report.f_corr, None);
        assert_eq!(report.sigma_pix_median_native, None);
        assert_eq!(report.err_origin, None);
        assert_eq!(report.bunit.as_deref(), Some("adu"));
        assert!(report.notes.iter().any(|note| note == NO_ERROR_LAYER_NOTE), "{:?}", report.notes);
        let reason = missing_calibration_reason(Some(&cached_header(&key).unwrap()));
        assert!(report.warnings.contains(&reason), "{reason}: {:?}", report.warnings);
    }

    #[test]
    fn a_psf_fwhm_adds_the_formal_point_source_limit() {
        let dir = tempfile::tempdir().unwrap();
        let key = file_of_test_6(dir.path());
        let report = depth(&key, DepthArgs { psf_fwhm_arcsec: Some(1.0), ..args() });
        let expected = 60.2153478231402f64;
        assert!(relative_difference(report.psf_limit_formal_native.unwrap(), expected) <= 1e-9, "{:?}", report.psf_limit_formal_native);
        assert!((report.psf_limit_formal_ab.unwrap() - (22.5 - 2.5 * expected.log10())).abs() <= 1e-9, "{:?}", report.psf_limit_formal_ab);
        assert_eq!(report.psf_fwhm_arcsec, Some(1.0));
        let point_source_note = "formal point-source limit = n_sigma x median pixel sigma x sqrt(pi FWHM^2 / (2 ln 2)); it carries no correlated-noise correction";
        assert!(report.notes.iter().any(|note| note == point_source_note), "{:?}", report.notes);
    }

    #[test]
    fn no_aperture_is_placed_on_a_source() {
        let dir = tempfile::tempdir().unwrap();
        let centre = (200.3, 250.6);
        let noise = gaussian_noise(512, 512, 1.0, 5);
        let image = Array2::from_shape_fn((512, 512), |(y, x)| {
            let d2 = (x as f64 - centre.0).powi(2) + (y as f64 - centre.1).powi(2);
            noise[[y, x]] + (30.0 * (-d2 / (2.0 * 2.0 * 2.0)).exp()) as f32
        });
        let path = dir.path().join("source.fits");
        let key = path.to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&key, &image, None).unwrap();

        let report = depth(&key, DepthArgs { radius_px: Some(3.8), ..args() });
        let nearest = report.apertures.iter().map(|aperture| (aperture.x - centre.0).hypot(aperture.y - centre.1)).fold(f64::INFINITY, f64::min);
        assert!(nearest >= 11.8, "{nearest}");
        assert_eq!(report.radius_arcsec, None);
        assert_eq!(report.pixel_scale_arcsec, None);
        assert_eq!(refusal(&key, args()), "The image has no celestial WCS: give the aperture radius in pixels");
    }

    #[test]
    fn a_small_image_returns_its_depth_with_a_warning_about_the_count() {
        let dir = tempfile::tempdir().unwrap();
        let key = noise_file(dir.path(), 128, 2.0, 3, vec![]);
        let report = depth(&key, args());
        assert!(report.n_placed < 200, "{}", report.n_placed);
        assert!((40..=199).contains(&report.n_used), "{}", report.n_used);
        assert!(
            report.warnings.iter().any(|warning| warning.starts_with("only ") && warning.contains("empty apertures were usable (fewer than 200)")),
            "{:?}",
            report.warnings
        );
        assert_eq!(report.notes.last().unwrap(), &format!("{} of the 500 requested apertures fit on the source-free sky", report.n_placed));
    }

    fn jwst_noise_mef(dir: &Path) -> String {
        let size = 256;
        let dq: Vec<i32> = (0..size * size).map(|index| i32::from((100..140).contains(&(index / size))) + i32::MIN).collect();
        let mut cards = vec![
            ("BUNIT", "'MJy/sr'".to_string()),
            ("PIXAR_SR", "2.1E-13".to_string()),
            ("PHOTMJSR", "0.5".to_string()),
            ("TELESCOP", "'JWST'".to_string()),
        ];
        cards.extend(wcs_cards(0.25, size));
        let path = dir.join("jwst.fits");
        write_test_mef(
            &path,
            &[],
            &[
                TestHdu { extname: Some("SCI"), extver: Some(1), cols: size, rows: size, data: pixels(&gaussian_noise(size, size, 2.0, 11)), extra_cards: cards },
                TestHdu {
                    extname: Some("ERR"),
                    extver: Some(1),
                    cols: size,
                    rows: size,
                    data: pixels(&flat(size, 2.0)),
                    extra_cards: vec![("BUNIT", "'MJy/sr'".into())],
                },
                TestHdu {
                    extname: Some("DQ"),
                    extver: Some(1),
                    cols: size,
                    rows: size,
                    data: HduData::I32(dq),
                    extra_cards: vec![("BZERO", "2147483648".into()), ("BSCALE", "1".into())],
                },
            ],
        );
        format!("{}#hdu=1", path.to_str().unwrap())
    }

    #[test]
    fn flagged_pixels_are_kept_out_of_the_apertures_unless_dq_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let key = jwst_noise_mef(dir.path());
        let masked = depth(&key, args());
        assert!(!masked.apertures.iter().any(|aperture| (96.0..=143.0).contains(&aperture.y)), "{:?}", masked.apertures);
        assert!(masked.masked_fraction >= 0.15, "{}", masked.masked_fraction);
        assert!(masked.dq_source.is_some());

        let unmasked = depth(&key, DepthArgs { exclude_dq: Some(false), ..args() });
        assert!(unmasked.apertures.iter().any(|aperture| (100.0..=139.0).contains(&aperture.y)));
        assert_eq!(unmasked.dq_source, None);

        let plain = file_of_test_3(dir.path());
        let asked = depth(&plain, DepthArgs { exclude_dq: Some(true), ..args() });
        assert_eq!(asked.warnings.iter().filter(|warning| warning.starts_with("DQ exclusion requested but not applied")).count(), 1, "{:?}", asked.warnings);
        assert!(!has_warning_starting_with(&depth(&plain, args()), "DQ exclusion requested"));
    }

    const DETECTED_PLANE_NOTE: &str = "10240 pixels of the mask planes DETECTED and DETECTED_NEGATIVE are treated as sources";

    fn afw_file(dir: &Path, name: &str, detected_rows: std::ops::Range<usize>, plane_value: i32) -> String {
        let size = 256;
        let mask: Vec<i32> = (0..size * size).map(|index| if detected_rows.contains(&(index / size)) { plane_value } else { 0 }).collect();
        let path = dir.join(name);
        write_test_mef(
            &path,
            &[("TELESCOP", "'Subaru'".into())],
            &[
                TestHdu {
                    extname: Some("IMAGE"),
                    extver: None,
                    cols: size,
                    rows: size,
                    data: pixels(&gaussian_noise(size, size, 2.0, 11)),
                    extra_cards: wcs_cards(0.2, size),
                },
                TestHdu {
                    extname: Some("MASK"),
                    extver: None,
                    cols: size,
                    rows: size,
                    data: HduData::I32(mask),
                    extra_cards: vec![
                        ("MP_BAD", "0".into()),
                        ("MP_SAT", "1".into()),
                        ("HIERARCH MP_DETECTED", "5".into()),
                        ("HIERARCH MP_DETECTED_NEGATIVE", "6".into()),
                    ],
                },
                TestHdu { extname: Some("VARIANCE"), extver: None, cols: size, rows: size, data: pixels(&flat(size, 4.0)), extra_cards: vec![] },
            ],
        );
        format!("{}#hdu=1", path.to_str().unwrap())
    }

    fn mentions_the_detected_planes(report: &DepthReport) -> bool {
        report.notes.iter().any(|note| note.contains("DETECTED"))
    }

    fn assert_the_marked_rows_are_sources(key: &str) {
        let report = depth(key, DepthArgs { radius_px: Some(4.0), exclude_dq: Some(false), ..args() });
        assert!(!report.apertures.iter().any(|aperture| (96.0..=143.0).contains(&aperture.y)), "{:?}", report.apertures);
        assert_eq!(report.detected_plane_pixels, Some(40 * 256));
        assert!(report.masked_fraction >= 0.15, "{}", report.masked_fraction);
        assert_eq!(report.notes[2], DETECTED_PLANE_NOTE, "{:?}", report.notes);
        assert_eq!(report.sigma_pix_median_native, Some(2.0));

        let with_dq_exclusion = depth(key, DepthArgs { radius_px: Some(4.0), ..args() });
        assert_eq!(with_dq_exclusion.apertures, report.apertures);
        assert_eq!(with_dq_exclusion.detected_plane_pixels, Some(40 * 256));
    }

    #[test]
    fn pixels_of_the_detected_plane_are_sources_even_without_dq_exclusion() {
        let dir = tempfile::tempdir().unwrap();
        assert_the_marked_rows_are_sources(&afw_file(dir.path(), "detected.fits", 100..140, 32));
    }

    #[test]
    fn a_file_without_such_a_plane_reports_none() {
        let dir = tempfile::tempdir().unwrap();
        let plain = depth(&file_of_test_3(dir.path()), DepthArgs { radius_px: Some(4.0), ..args() });
        assert_eq!(plain.detected_plane_pixels, None);
        assert!(!mentions_the_detected_planes(&plain), "{:?}", plain.notes);

        let unmarked = depth(&afw_file(dir.path(), "unmarked.fits", 0..0, 32), DepthArgs { radius_px: Some(4.0), ..args() });
        assert_eq!(unmarked.detected_plane_pixels, Some(0));
        assert!(!mentions_the_detected_planes(&unmarked), "{:?}", unmarked.notes);
    }

    #[test]
    fn the_negative_detections_count_too() {
        let dir = tempfile::tempdir().unwrap();
        assert_the_marked_rows_are_sources(&afw_file(dir.path(), "negative.fits", 100..140, 64));
    }

    #[test]
    fn an_error_layer_that_varies_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut image = gaussian_noise(512, 512, 1.0, 11);
        image.slice_mut(ndarray::s![342.., ..]).mapv_inplace(|value| value * 2.0);
        let err = Array2::from_shape_fn((512, 512), |(y, _)| if y < 342 { 1.0f32 } else { 2.0 });
        let key = noise_mef(dir.path(), "two_levels.fits", &image, Some(&err), vec![]);
        let report = depth(&key, args());
        assert_eq!(report.sigma_pix_spread, Some(2.0));
        let expected = "the error layer varies across the image: its 84th percentile is 2.00 times its 16th; one depth does not describe every part of the image";
        assert!(report.warnings.iter().any(|warning| warning == expected), "{:?}", report.warnings);
    }

    #[test]
    fn a_low_detection_threshold_is_warned_about() {
        let dir = tempfile::tempdir().unwrap();
        let key = file_of_test_3(dir.path());
        let usual = depth(&key, DepthArgs { radius_px: Some(4.0), ..args() });
        let low = depth(&key, DepthArgs { radius_px: Some(4.0), detect_nsigma: Some(1.5), ..args() });
        assert!(low.warnings.iter().any(|warning| warning == LOW_DETECTION_WARNING), "{:?}", low.warnings);
        assert!(low.masked_fraction > usual.masked_fraction, "{} {}", low.masked_fraction, usual.masked_fraction);
    }

    #[test]
    fn arguments_outside_their_range_are_refused_before_the_file_is_opened() {
        let missing = "does-not-exist.fits";
        let cases = [
            (DepthArgs { radius_arcsec: Some(1.0), radius_px: Some(4.0), ..args() }, "Give the aperture radius in arcsec or in pixels, not both"),
            (DepthArgs { radius_px: Some(0.0), ..args() }, "Aperture radius must be a positive number"),
            (DepthArgs { radius_arcsec: Some(f64::NAN), ..args() }, "Aperture radius must be a positive number"),
            (DepthArgs { n_apertures: Some(9), ..args() }, "The number of apertures must be between 10 and 5000"),
            (DepthArgs { n_apertures: Some(5001), ..args() }, "The number of apertures must be between 10 and 5000"),
            (DepthArgs { n_sigma: Some(0.0), ..args() }, "The significance must be above 0 and at most 100 sigma"),
            (DepthArgs { n_sigma: Some(f64::NAN), ..args() }, "The significance must be above 0 and at most 100 sigma"),
            (DepthArgs { n_sigma: Some(101.0), ..args() }, "The significance must be above 0 and at most 100 sigma"),
            (DepthArgs { detect_nsigma: Some(0.5), ..args() }, "The source detection threshold must be between 1 and 10 sigma"),
            (DepthArgs { detect_nsigma: Some(11.0), ..args() }, "The source detection threshold must be between 1 and 10 sigma"),
            (DepthArgs { psf_fwhm_arcsec: Some(0.0), ..args() }, "PSF FWHM must be a positive number of arcsec"),
        ];
        for (case, message) in cases {
            assert_eq!(refusal(missing, case.clone()), message, "{case:?}");
        }

        let dir = tempfile::tempdir().unwrap();
        let key = file_of_test_3(dir.path());
        assert_eq!(refusal(&key, DepthArgs { radius_px: Some(0.5), ..args() }), "Aperture radius of 0.50 px is outside 1 to 100 px");
        assert_eq!(refusal(&key, DepthArgs { radius_arcsec: Some(40.0), ..args() }), "Aperture radius of 160.00 px is outside 1 to 100 px");
    }

    #[tokio::test]
    async fn the_command_has_the_defaults_of_the_function() {
        let dir = tempfile::tempdir().unwrap();
        let key = file_of_test_3(dir.path());
        let mut from_command = image_depth_cmd(key.clone(), None, None, None, None, None, None, None, None).await.unwrap();
        let mut keys: Vec<&str> = from_command.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut expected_keys = DEPTH_REPORT_KEYS.to_vec();
        expected_keys.sort_unstable();
        assert_eq!(keys, expected_keys);

        let mut from_function = serde_json::to_value(depth_for_path(&key, &args()).unwrap()).unwrap();
        from_command.as_object_mut().unwrap().remove("elapsed_ms");
        from_function.as_object_mut().unwrap().remove("elapsed_ms");
        assert_eq!(from_command.as_object().unwrap().len(), 34);
        assert_eq!(from_command, from_function);
    }

    const EUCLID_E1_VIS_IMAGE: &str = "EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits";

    fn survey_file(set: &str, name: &str) -> String {
        let path = env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("survey").join(set).join(name);
        assert!(path.is_file(), "{} is missing", path.display());
        path.to_str().unwrap().to_string()
    }

    fn survey_copy(dir: &Path, set: &str, name: &str) -> String {
        let target = dir.join(name);
        std::fs::copy(survey_file(set, name), &target).unwrap();
        target.to_str().unwrap().to_string()
    }

    fn print_depth(report: &DepthReport) {
        println!("radius_px {} radius_arcsec {:?} pixel_scale_arcsec {:?}", report.radius_px, report.radius_arcsec, report.pixel_scale_arcsec);
        println!(
            "n_requested {} n_placed {} n_used {} n_after_clip {}",
            report.n_requested, report.n_placed, report.n_used, report.n_after_clip
        );
        println!("masked_fraction {} detected_plane_pixels {:?}", report.masked_fraction, report.detected_plane_pixels);
        println!(
            "median_sum_native {} sigma_empty_native {} sigma_clipped_native {} sigma_empty_rel_err {}",
            report.median_sum_native, report.sigma_empty_native, report.sigma_clipped_native, report.sigma_empty_rel_err
        );
        println!("sigma_formal_native {:?} f_corr {:?}", report.sigma_formal_native, report.f_corr);
        println!("limit_native {} limit_jy {:?} limit_ab {:?}", report.limit_native, report.limit_jy, report.limit_ab);
        println!(
            "psf_fwhm_arcsec {:?} psf_limit_formal_native {:?} psf_limit_formal_ab {:?}",
            report.psf_fwhm_arcsec, report.psf_limit_formal_native, report.psf_limit_formal_ab
        );
        println!("sigma_pix_median_native {:?} sigma_pix_spread {:?}", report.sigma_pix_median_native, report.sigma_pix_spread);
        println!("bunit {:?} calibration_origin {:?}", report.bunit, report.calibration_origin);
        println!("err_origin {:?} dq_source {:?}", report.err_origin, report.dq_source);
        println!("definition {}", report.definition);
        println!("notes {:?}", report.notes);
        println!("warnings {:?}", report.warnings);
        println!("elapsed_ms {}", report.elapsed_ms);
    }

    fn assert_within(name: &str, measured: f64, target: f64, tolerance: f64) {
        assert!((measured - target).abs() <= tolerance, "{name} {measured} is not within {tolerance} of {target}");
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-image-r.fits.fz, legacysurvey-0532m280-invvar-r.fits.fz, legacysurvey-0532m280-maskbits.fits.fz"]
    fn real_legacy_l2_r_depth_through_the_command_is_within_0_05_mag_of_the_oracle() {
        let key = format!("{}#hdu=1", survey_file("L2", "legacysurvey-0532m280-image-r.fits.fz"));
        let report = depth(&key, DepthArgs { n_apertures: Some(5000), psf_fwhm_arcsec: Some(1.084), ..args() });
        print_depth(&report);
        println!("sigma_clipped limit_ab {}", 22.5 - 2.5 * (5.0 * report.sigma_clipped_native).log10());
        assert_within("radius_px", report.radius_px, 3.8167938931297707, 1e-6);
        assert_eq!(report.n_used, 5000);
        assert!(report.err_origin.is_some());
        assert!(report.dq_source.is_some());
        assert!(report.calibration_origin.is_some());
        assert_within("limit_ab", report.limit_ab.unwrap(), 25.581924675163673, 0.05);
        let oracle_f_corr = 1.6465651242079744;
        assert_within("f_corr", report.f_corr.unwrap(), oracle_f_corr, 0.1 * oracle_f_corr);
        assert_within("psf_limit_formal_ab", report.psf_limit_formal_ab.unwrap(), 26.173, 0.15);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1/des-dr2_A_r_extn1.fits, des-dr2_A_r_extn2.fits, des-dr2_A_r_extn3.fits"]
    fn real_des_d1_r_depth_through_the_command_is_within_0_1_mag_of_the_oracle() {
        let dir = tempfile::tempdir().unwrap();
        let image = survey_copy(dir.path(), "D1", "des-dr2_A_r_extn1.fits");
        let mask = survey_copy(dir.path(), "D1", "des-dr2_A_r_extn2.fits");
        let weight = survey_copy(dir.path(), "D1", "des-dr2_A_r_extn3.fits");
        set_companion_json(&image, "err", &weight, None).unwrap();
        set_companion_json(&image, "dq", &mask, None).unwrap();
        let report = depth(&image, DepthArgs { n_apertures: Some(2000), ..args() });
        print_depth(&report);
        println!("oracle f_corr 1.0541617446782123");
        assert_within("radius_px", report.radius_px, 3.8022813688212924, 1e-6);
        assert_within("limit_ab", report.limit_ab.unwrap(), 25.250591282240478, 0.1);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1/EUC_MER_BGSUB-MOSAIC-VIS, EUC_MER_MOSAIC-VIS-RMS and EUC_MER_MOSAIC-VIS-FLAG of tile 102044185, cut60_A"]
    fn real_euclid_e1_vis_depth_through_the_command_is_within_0_1_mag_of_the_oracle() {
        let image = survey_file("E1", EUCLID_E1_VIS_IMAGE);
        let report = depth(&image, DepthArgs { n_apertures: Some(2000), ..args() });
        print_depth(&report);
        println!("oracle f_corr 1.0738031457273518");
        assert_within("radius_px", report.radius_px, 10.0, 1e-6);
        assert_within("limit_ab", report.limit_ab.unwrap(), 25.606656166108493, 0.1);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_r_stack_480px.fits, ps1_A_r_wt_480px.fits"]
    fn real_ps1_p1_r_depth_through_the_command_is_within_0_1_mag_of_the_oracle() {
        let dir = tempfile::tempdir().unwrap();
        let image = survey_copy(dir.path(), "P1", "ps1_A_r_stack_480px.fits");
        let weight = survey_copy(dir.path(), "P1", "ps1_A_r_wt_480px.fits");
        set_companion_json(&image, "err", &weight, Some(LayerRole::Variance)).unwrap();
        let report = depth(&image, DepthArgs { n_apertures: Some(2000), ..args() });
        print_depth(&report);
        println!("oracle f_corr 1.5059979215639165");
        assert_within("radius_px", report.radius_px, 4.0, 1e-6);
        assert_within("limit_ab", report.limit_ab.unwrap(), 23.025188873915404, 0.1);
    }
}
