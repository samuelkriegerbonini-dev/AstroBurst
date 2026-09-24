use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, cached_header, derived_output_header, output_stem, render_named_and_save, resolve_output_dir,
    write_derived_fits, OutputValues,
};
use crate::cmd::compose::rescale_per_pixel_calibration;
use crate::core::imaging::stats::compute_image_stats;
use crate::cmd::helpers;
use crate::core::stacking::calibration::calibrate_from_paths;
use crate::core::stacking::calibration::drizzle_from_paths;
use crate::core::stacking::calibration::stack_from_paths;
use crate::core::stacking::drizzle::drizzle_wcs_updates;
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    EVENT_CALIBRATE_PROGRESS, EVENT_STACK_PROGRESS, STAGE_RENDER, STAGE_SAVE,
    RES_CONFIDENCE, RES_DIMENSIONS, RES_DX, RES_DY, RES_ELAPSED_MS, RES_FITS_PATH, RES_FRAME_COUNT,
    RES_HAS_BIAS, RES_HAS_DARK, RES_HAS_FLAT, RES_MAX, RES_MEAN, RES_METHOD_USED, RES_MIN,
    RES_OFFSETS, RES_PATH, RES_PNG_PATH, RES_REJECTED_PIXELS, RES_SCALE, RES_SIGMA, RES_STATS,
    RES_WARNINGS,
};
use crate::types::header::HduHeader;
use crate::types::stacking::{DrizzleConfig, DrizzleResult, FrameAlignment, StackConfig, StackResult};

const ABPROC_CALIBRATED: &str = "calibrated";
const ABPROC_STACKED: &str = "stacked";
const ABPROC_DRIZZLED: &str = "drizzled";
const ABPROC_REJECTION: &str = "rejection";

pub const RES_REJECTION: &str = "rejection";
pub const RES_COMBINE: &str = "combine";
pub const RES_NORMALIZATION: &str = "normalization";
pub const RES_REJECTION_NORMALIZATION: &str = "rejection_normalization";
pub const RES_REJECTION_LOW_FITS: &str = "rejection_low_fits";
pub const RES_REJECTION_HIGH_FITS: &str = "rejection_high_fits";
pub const RES_NORMALIZATION_APPLIED: &str = "normalization_applied";
const RES_ALIGNMENT: &str = "alignment";
const RES_INCLUDED: &str = "included";

fn output_name(name: Option<&str>, default: &str) -> String {
    let mut out = String::new();
    let mut replaced = false;
    for c in name.unwrap_or("").chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
            out.push(c);
            replaced = false;
        } else if !replaced {
            out.push('_');
            replaced = true;
        }
    }
    let trimmed = out.trim_start_matches('.');
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

fn alignment_json(paths: &[String], alignment: &[FrameAlignment]) -> Vec<serde_json::Value> {
    paths
        .iter()
        .zip(alignment)
        .map(|(path, frame)| {
            json!({
                RES_PATH: path,
                RES_METHOD_USED: frame.method,
                RES_CONFIDENCE: frame.confidence,
                RES_INCLUDED: frame.included,
            })
        })
        .collect()
}

fn reference_header(paths: &[String]) -> Option<HduHeader> {
    paths.first().and_then(|p| cached_header(p).ok())
}

fn save_calibrated(calibrated: &Array2<f32>, science_path: &str, output_dir: &str) -> anyhow::Result<(String, Option<String>)> {
    let source = cached_header(science_path).ok();
    let header = derived_output_header(source.as_ref(), ABPROC_CALIBRATED, OutputValues::Linear);
    let name = format!("{}_calibrated", output_stem(science_path));
    render_named_and_save(calibrated, output_dir, &name, true, Some(&header))
}

struct StackOutputs {
    png_path: String,
    fits_path: Option<String>,
    rejection_low_fits: Option<String>,
    rejection_high_fits: Option<String>,
}

fn save_stack(result: &StackResult, paths: &[String], output_dir: &str, stem: &str) -> anyhow::Result<StackOutputs> {
    let reference = reference_header(paths);
    let header = derived_output_header(reference.as_ref(), ABPROC_STACKED, OutputValues::Linear);
    let (png_path, fits_path) = render_named_and_save(&result.image, output_dir, stem, true, Some(&header))?;
    let map_header = derived_output_header(reference.as_ref(), ABPROC_REJECTION, OutputValues::Rescaled);
    let write_map = |map: &Array2<u16>, side: &str| write_rejection_map(map, output_dir, stem, side, &map_header);
    Ok(StackOutputs {
        png_path,
        fits_path,
        rejection_low_fits: result.rejection_low.as_ref().map(|map| write_map(map, "low")).transpose()?,
        rejection_high_fits: result.rejection_high.as_ref().map(|map| write_map(map, "high")).transpose()?,
    })
}

fn drizzled_header(reference: Option<&HduHeader>, scale: f64) -> HduHeader {
    let mut header = derived_output_header(reference, ABPROC_DRIZZLED, OutputValues::Linear);
    for (key, value) in drizzle_wcs_updates(&header, scale) {
        header.set_f64(&key, value);
    }
    rescale_per_pixel_calibration(&mut header, 1.0 / (scale * scale));
    header
}

fn save_drizzle(result: &DrizzleResult, paths: &[String], output_dir: &str, stem: &str) -> anyhow::Result<(String, Option<String>)> {
    let header = drizzled_header(reference_header(paths).as_ref(), result.output_scale);
    render_named_and_save(&result.image, output_dir, stem, true, Some(&header))
}

#[tauri::command]
pub async fn calibrate(
    app: tauri::AppHandle,
    science_path: String,
    output_dir: String,
    bias_paths: Option<Vec<String>>,
    dark_paths: Option<Vec<String>>,
    flat_paths: Option<Vec<String>>,
    dark_exposure_ratio: Option<f32>,
) -> Result<serde_json::Value, String> {
    let progress = ProgressHandle::new(&app, EVENT_CALIBRATE_PROGRESS, 4);
    let progress_clone = progress.clone();

    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let calibrated = calibrate_from_paths(
            &science_path,
            bias_paths.as_deref(),
            dark_paths.as_deref(),
            flat_paths.as_deref(),
            dark_exposure_ratio.unwrap_or(1.0),
        )?;

        progress_clone.tick_with_stage(STAGE_RENDER);

        let (png_path, fits_path) = save_calibrated(&calibrated, &science_path, &output_dir)?;

        let (rows, cols) = calibrated.dim();
        let stats = compute_image_stats(&calibrated);

        progress_clone.emit_complete();

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_FITS_PATH: fits_path,
            RES_DIMENSIONS: [cols, rows],
            RES_HAS_BIAS: bias_paths.is_some(),
            RES_HAS_DARK: dark_paths.is_some(),
            RES_HAS_FLAT: flat_paths.is_some(),
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_STATS: {
                RES_MIN: stats.min,
                RES_MAX: stats.max,
                RES_MEAN: stats.mean,
                RES_SIGMA: stats.sigma,
            },
        }))
    })
}

fn write_rejection_map(
    map: &Array2<u16>,
    output_dir: &str,
    stem: &str,
    side: &str,
    header: &HduHeader,
) -> anyhow::Result<String> {
    let path = format!("{}/{}_rejection_{}.fits", output_dir, stem, side);
    let counts = map.mapv(|v| v as f32);
    write_derived_fits(&path, &counts, Some(header))?;
    Ok(path)
}

#[tauri::command]
pub async fn stack(
    app: tauri::AppHandle,
    paths: Vec<String>,
    output_dir: String,
    sigma_low: Option<f32>,
    sigma_high: Option<f32>,
    max_iterations: Option<usize>,
    align: Option<bool>,
    align_method: Option<String>,
    name: Option<String>,
    weights: Option<Vec<f64>>,
    rejection: Option<String>,
    combine: Option<String>,
    normalization: Option<String>,
    rejection_normalization: Option<String>,
    winsor_cutoff: Option<f32>,
    percentile_low: Option<f32>,
    percentile_high: Option<f32>,
    minmax_low: Option<usize>,
    minmax_high: Option<usize>,
    rejection_maps: Option<bool>,
) -> Result<serde_json::Value, String> {
    let frame_count = paths.len() as u64;
    let progress = ProgressHandle::new(&app, EVENT_STACK_PROGRESS, frame_count + 2);
    let progress_clone = progress.clone();

    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let defaults = StackConfig::default();
        let config = StackConfig {
            sigma_low: sigma_low.unwrap_or(defaults.sigma_low),
            sigma_high: sigma_high.unwrap_or(defaults.sigma_high),
            max_iterations: max_iterations.unwrap_or(defaults.max_iterations),
            align: align.unwrap_or(true),
            align_method: helpers::parse_align_method(align_method.as_deref()),
            weights,
            rejection: helpers::parse_rejection_method(rejection.as_deref())?,
            combine: helpers::parse_combine_method(combine.as_deref())?,
            normalization: helpers::parse_normalization_method(normalization.as_deref())?,
            rejection_normalization: helpers::parse_rejection_normalization(rejection_normalization.as_deref())?,
            winsor_cutoff: winsor_cutoff.unwrap_or(defaults.winsor_cutoff),
            percentile_low: percentile_low.unwrap_or(defaults.percentile_low),
            percentile_high: percentile_high.unwrap_or(defaults.percentile_high),
            minmax_low: minmax_low.unwrap_or(defaults.minmax_low),
            minmax_high: minmax_high.unwrap_or(defaults.minmax_high),
            rejection_maps: rejection_maps.unwrap_or(false),
        };

        let result = stack_from_paths(&paths, &config, Some(&progress_clone))?;

        progress_clone.tick_with_stage(STAGE_RENDER);

        let stem = output_name(name.as_deref(), "stacked");

        let saved = save_stack(&result, &paths, &output_dir, &stem)?;

        let (rows, cols) = result.image.dim();
        let stats = compute_image_stats(&result.image);

        progress_clone.tick_with_stage(STAGE_SAVE);
        progress_clone.emit_complete();

        Ok(json!({
            RES_PNG_PATH: saved.png_path,
            RES_FITS_PATH: saved.fits_path,
            RES_DIMENSIONS: [cols, rows],
            RES_FRAME_COUNT: result.frame_count,
            RES_REJECTED_PIXELS: result.rejected_pixels,
            RES_OFFSETS: result.offsets.iter().map(|(dy, dx)| json!({RES_DY: dy, RES_DX: dx})).collect::<Vec<_>>(),
            RES_REJECTION: config.rejection.name(),
            RES_COMBINE: config.combine.name(),
            RES_NORMALIZATION: config.normalization.name(),
            RES_REJECTION_NORMALIZATION: config.rejection_normalization.name(),
            RES_NORMALIZATION_APPLIED: result.normalization_applied.iter().map(|(offset, scale)| json!({"offset": offset, "scale": scale})).collect::<Vec<_>>(),
            RES_ALIGNMENT: alignment_json(&paths, &result.alignment),
            RES_WARNINGS: result.warnings,
            RES_REJECTION_LOW_FITS: saved.rejection_low_fits,
            RES_REJECTION_HIGH_FITS: saved.rejection_high_fits,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_STATS: {
                RES_MIN: stats.min,
                RES_MAX: stats.max,
                RES_MEAN: stats.mean,
                RES_SIGMA: stats.sigma,
            },
        }))
    })
}

#[tauri::command]
pub async fn drizzle_stack(
    app: tauri::AppHandle,
    paths: Vec<String>,
    output_dir: String,
    scale: Option<f64>,
    pixfrac: Option<f64>,
    kernel: Option<String>,
    align: Option<bool>,
    name: Option<String>,
    rejection: Option<String>,
    sigma_low: Option<f32>,
    sigma_high: Option<f32>,
) -> Result<serde_json::Value, String> {
    let frame_count = paths.len() as u64;
    let progress = ProgressHandle::new(&app, EVENT_STACK_PROGRESS, frame_count + 2);
    let progress_clone = progress.clone();

    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let defaults = DrizzleConfig::default();
        let config = DrizzleConfig {
            scale: scale.unwrap_or(2.0),
            pixfrac: pixfrac.unwrap_or(0.7),
            kernel: helpers::parse_drizzle_kernel(kernel.as_deref()),
            align: align.unwrap_or(true),
            rejection: helpers::parse_rejection_method(rejection.as_deref())?,
            sigma_low: sigma_low.unwrap_or(defaults.sigma_low),
            sigma_high: sigma_high.unwrap_or(defaults.sigma_high),
            ..defaults
        };

        let result = drizzle_from_paths(&paths, &config, Some(&progress_clone))?;

        progress_clone.tick_with_stage(STAGE_RENDER);

        let stem = output_name(name.as_deref(), "drizzled");

        let (png_path, fits_path) = save_drizzle(&result, &paths, &output_dir, &stem)?;

        let (rows, cols) = result.image.dim();
        let stats = compute_image_stats(&result.image);

        progress_clone.tick_with_stage(STAGE_SAVE);
        progress_clone.emit_complete();

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_FITS_PATH: fits_path,
            RES_DIMENSIONS: [cols, rows],
            RES_FRAME_COUNT: result.frame_count,
            RES_REJECTED_PIXELS: result.rejected_pixels,
            RES_SCALE: result.output_scale,
            RES_ALIGNMENT: alignment_json(&paths, &result.alignment),
            RES_WARNINGS: result.warnings,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_STATS: {
                RES_MIN: stats.min,
                RES_MAX: stats.max,
                RES_MEAN: stats.mean,
                RES_SIGMA: stats.sigma,
            },
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_names_stay_inside_the_output_directory() {
        assert_eq!(output_name(Some("../evil"), "stacked"), "_evil");
        assert_eq!(output_name(Some("a b/c\\d"), "stacked"), "a_b_c_d");
        assert_eq!(output_name(Some(".."), "stacked"), "stacked");
        assert_eq!(output_name(Some(""), "drizzled"), "drizzled");
        assert_eq!(output_name(None, "drizzled"), "drizzled");
        assert_eq!(output_name(Some("M31_stack3_20260924-101010-123"), "stacked"), "M31_stack3_20260924-101010-123");
    }

    #[test]
    fn alignment_report_names_each_input_path() {
        let paths = vec!["a.fits".to_string(), "b.fits".to_string()];
        let alignment = vec![
            FrameAlignment::reference(),
            FrameAlignment { method: "phase_correlation_identity".into(), confidence: Some(2.5), included: false },
        ];
        let report = alignment_json(&paths, &alignment);
        assert_eq!(report.len(), 2);
        assert_eq!(report[0][RES_PATH], "a.fits");
        assert_eq!(report[0][RES_CONFIDENCE], serde_json::Value::Null);
        assert_eq!(report[1][RES_METHOD_USED], "phase_correlation_identity");
        assert_eq!(report[1][RES_INCLUDED], false);
        assert_eq!(report[1][RES_CONFIDENCE], 2.5);
    }

    #[test]
    fn rejection_map_is_written_as_float_counts_next_to_the_stack() {
        let dir = tempfile::tempdir().unwrap();
        let map = Array2::from_shape_vec((2, 3), vec![0u16, 1, 2, 65535, 4, 5]).unwrap();
        let out = dir.path().to_str().unwrap().replace('\\', "/");
        let header = derived_output_header(None, ABPROC_REJECTION, OutputValues::Rescaled);
        let path = write_rejection_map(&map, &out, "stacked", "high", &header).unwrap();
        assert!(path.ends_with("/stacked_rejection_high.fits"));
        let back = crate::infra::fits::reader::load_fits_image(&path).unwrap();
        assert_eq!(back.dim(), (2, 3));
        let mut values: Vec<f32> = back.iter().copied().collect();
        values.sort_by(crate::math::median::f32_cmp);
        assert_eq!(values, vec![0.0, 1.0, 2.0, 4.0, 5.0, 65535.0]);
    }

    fn solved_header(crpix1: &str) -> HduHeader {
        let mut header = HduHeader::empty();
        for (key, value) in [
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
            ("CRVAL1", "83.8"),
            ("CRVAL2", "-5.4"),
            ("CRPIX1", crpix1),
            ("CRPIX2", "4.5"),
            ("CD1_1", "-0.0001"),
            ("CD1_2", "0.0"),
            ("CD2_1", "0.0"),
            ("CD2_2", "0.0001"),
            ("EXPTIME", "300"),
            ("BUNIT", "MJy/sr"),
            ("PIXAR_SR", "2.0E-13"),
            ("SATURATE", "60000"),
        ] {
            header.set(key, value.to_string());
        }
        header
    }

    fn write_frame(dir: &tempfile::TempDir, name: &str, level: f32, header: &HduHeader) -> String {
        let path = dir.path().join(name).to_str().unwrap().replace('\\', "/");
        let frame = Array2::from_shape_fn((8, 8), |(y, x)| level + ((y * 3 + x * 5) % 7) as f32);
        crate::infra::fits::writer::write_fits_mono(&path, &frame, Some(header)).unwrap();
        path
    }

    fn written_header(path: Option<&str>) -> HduHeader {
        crate::infra::fits::reader::read_primary_header(path.expect("a FITS path")).unwrap()
    }

    fn text<'a>(header: &'a HduHeader, key: &str) -> Option<&'a str> {
        header.get(key).map(str::trim)
    }

    #[test]
    fn calibrated_fits_keeps_the_science_wcs_and_units() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().replace('\\', "/");
        let science = write_frame(&dir, "science.fits", 100.0, &solved_header("3.5"));
        let calibrated = calibrate_from_paths(&science, None, None, None, 1.0).unwrap();

        let (_, fits) = save_calibrated(&calibrated, &science, &out).unwrap();
        let header = written_header(fits.as_deref());
        assert_eq!(text(&header, "CTYPE1"), Some("RA---TAN"), "WCS dropped from the calibrated frame");
        assert_eq!(header.get_f64("CRPIX1"), Some(3.5));
        assert_eq!(header.get_f64("EXPTIME"), Some(300.0));
        assert_eq!(text(&header, "BUNIT"), Some("MJy/sr"));
        assert_eq!(text(&header, "ABPROC"), Some(ABPROC_CALIBRATED));
    }

    #[test]
    fn stacked_fits_and_rejection_maps_carry_the_reference_frame_wcs() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().replace('\\', "/");
        let paths = vec![
            write_frame(&dir, "a.fits", 100.0, &solved_header("3.5")),
            write_frame(&dir, "b.fits", 102.0, &solved_header("9.5")),
            write_frame(&dir, "c.fits", 101.0, &solved_header("9.5")),
        ];
        let config = StackConfig { align: false, rejection_maps: true, ..StackConfig::default() };
        let result = stack_from_paths(&paths, &config, None).unwrap();

        let saved = save_stack(&result, &paths, &out, "stacked").unwrap();
        let stacked = written_header(saved.fits_path.as_deref());
        assert_eq!(stacked.get_f64("CRPIX1"), Some(3.5), "the stack must carry the reference frame's WCS");
        assert_eq!(text(&stacked, "CTYPE2"), Some("DEC--TAN"));
        assert_eq!(text(&stacked, "BUNIT"), Some("MJy/sr"));
        assert_eq!(text(&stacked, "ABPROC"), Some(ABPROC_STACKED));

        for map in [saved.rejection_low_fits.as_deref(), saved.rejection_high_fits.as_deref()] {
            let header = written_header(map);
            assert_eq!(header.get_f64("CRPIX1"), Some(3.5));
            assert!(header.get("BUNIT").is_none(), "a count map is not in the frame's flux unit");
            assert_eq!(text(&header, "ABPROC"), Some(ABPROC_REJECTION));
        }
    }

    #[test]
    fn drizzled_header_rescales_the_wcs_and_the_per_pixel_calibration() {
        let mut source = solved_header("3.5");
        source.set("ZP", "25.0".to_string());
        source.set("ZPTMAG", "24.0".to_string());
        let header = drizzled_header(Some(&source), 2.0);
        let area_shift = 2.5 * 4f64.log10();
        assert!(header.get_f64("ZP").is_some_and(|zp| (zp - (25.0 + area_shift)).abs() < 1e-9), "ZP {:?}", header.get("ZP"));
        assert!(header.get_f64("ZPTMAG").is_some_and(|zp| (zp - (24.0 + area_shift)).abs() < 1e-9), "ZPTMAG {:?}", header.get("ZPTMAG"));
        assert_eq!(header.get_f64("CRPIX1"), Some(6.5));
        assert_eq!(header.get_f64("CRPIX2"), Some(8.5));
        assert_eq!(header.get_f64("CD1_1"), Some(-0.00005));
        assert_eq!(header.get_f64("CD2_2"), Some(0.00005));
        assert_eq!(header.get_f64("CRVAL1"), Some(83.8));
        assert_eq!(header.get_f64("PIXAR_SR"), Some(5.0e-14));
        assert!(header.get("SATURATE").is_none());
        assert_eq!(text(&header, "BUNIT"), Some("MJy/sr"));
        assert_eq!(text(&header, "ABPROC"), Some(ABPROC_DRIZZLED));
    }

    #[test]
    fn drizzled_fits_uses_the_applied_scale_for_its_wcs() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().replace('\\', "/");
        let paths = vec![
            write_frame(&dir, "d1.fits", 100.0, &solved_header("3.5")),
            write_frame(&dir, "d2.fits", 100.0, &solved_header("3.5")),
        ];
        let config = DrizzleConfig { scale: 1.5, align: false, ..DrizzleConfig::default() };
        let result = drizzle_from_paths(&paths, &config, None).unwrap();

        let (_, fits) = save_drizzle(&result, &paths, &out, "drizzled").unwrap();
        let header = written_header(fits.as_deref());
        assert_eq!(result.output_dims, (12, 12));
        assert_eq!(header.get_f64("CRPIX1"), Some(5.0));
        assert_eq!(text(&header, "CTYPE1"), Some("RA---TAN"));
        assert_eq!(text(&header, "ABPROC"), Some(ABPROC_DRIZZLED));
    }

    #[test]
    fn min_max_counts_are_refused_before_any_frame_is_loaded() {
        let paths = vec!["missing_1.fits".to_string(), "missing_2.fits".to_string(), "missing_3.fits".to_string()];
        let minmax = |low, high| StackConfig {
            rejection: crate::types::stacking::RejectionMethod::MinMax,
            minmax_low: low,
            minmax_high: high,
            ..StackConfig::default()
        };
        let err = stack_from_paths(&paths, &minmax(usize::MAX, 1), None).unwrap_err().to_string();
        assert!(err.contains("too large"), "{err}");
        let err = stack_from_paths(&paths, &minmax(2, 1), None).unwrap_err().to_string();
        assert!(err.contains("needs more than 3 frames"), "{err}");
        let err = stack_from_paths(&paths, &minmax(1, 1), None).unwrap_err().to_string();
        assert!(!err.contains("Min/max"), "{err}");
    }
}
