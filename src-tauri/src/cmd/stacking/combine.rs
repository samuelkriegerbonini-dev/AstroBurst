use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, cached_header, derived_output_header, output_stem, render_named_and_save, resolve_output_dir,
    write_derived_fits, OutputValues,
};
use super::cards::{stack_output_cards, DrizzleCards, StackCards};
use crate::cmd::compose::rescale_per_pixel_calibration;
use crate::core::imaging::stats::compute_image_stats;
use crate::cmd::helpers;
use crate::core::stacking::calibration::calibrate_from_paths;
use crate::core::stacking::calibration::drizzle_from_paths;
use crate::core::stacking::calibration::stack_from_paths;
use crate::core::stacking::calibration::CalibratedFrame;
use crate::core::stacking::drizzle::drizzle_wcs_updates;
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    EVENT_CALIBRATE_PROGRESS, EVENT_STACK_PROGRESS, KERNEL_GAUSSIAN, KERNEL_LANCZOS3, STAGE_RENDER, STAGE_SAVE,
    RES_CONFIDENCE, RES_DARK_SCALE, RES_DIMENSIONS, RES_DX, RES_DY, RES_ELAPSED_MS, RES_FITS_PATH, RES_FRAME_COUNT,
    RES_HAS_BIAS, RES_HAS_DARK, RES_HAS_FLAT, RES_HAS_FLAT_DARK, RES_MAX, RES_MEAN, RES_METHOD_USED, RES_MIN,
    RES_OFFSETS, RES_PATH, RES_PNG_PATH, RES_REJECTED_PIXELS, RES_SCALE, RES_SIGMA, RES_STATS,
    RES_WARNINGS, RES_WEIGHTS_APPLIED,
};
use crate::types::header::HduHeader;
use crate::types::image::ImageStats;
use crate::types::stacking::{
    CombineMethod, DrizzleConfig, DrizzleKernel, DrizzleResult, FrameAlignment, StackConfig, StackResult,
};

const KERNEL_SQUARE: &str = "square";

const ABPROC_CALIBRATED: &str = "calibrated";
pub(super) const ABPROC_STACKED: &str = "stacked";
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

pub(super) fn output_name(name: Option<&str>, default: &str) -> String {
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

pub(super) fn reference_header(paths: &[String]) -> Option<HduHeader> {
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

fn included_paths<'a>(paths: &'a [String], alignment: &[FrameAlignment]) -> Vec<&'a str> {
    paths
        .iter()
        .zip(alignment)
        .filter(|(_, frame)| frame.included)
        .map(|(path, _)| path.as_str())
        .collect()
}

fn kernel_name(kernel: DrizzleKernel) -> &'static str {
    match kernel {
        DrizzleKernel::Square => KERNEL_SQUARE,
        DrizzleKernel::Gaussian => KERNEL_GAUSSIAN,
        DrizzleKernel::Lanczos3 => KERNEL_LANCZOS3,
    }
}

fn save_stack(
    result: &StackResult,
    paths: &[String],
    output_dir: &str,
    stem: &str,
    config: &StackConfig,
) -> anyhow::Result<StackOutputs> {
    let reference = reference_header(paths);
    let mut header = derived_output_header(reference.as_ref(), ABPROC_STACKED, OutputValues::Linear);
    let included = included_paths(paths, &result.alignment);
    stack_output_cards(
        &mut header,
        &StackCards {
            included_paths: &included,
            combine: config.combine,
            rejection: config.rejection.name(),
            normalization: Some(config.normalization.name()),
            drizzle: None,
        },
    );
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

fn save_drizzle(
    result: &DrizzleResult,
    paths: &[String],
    output_dir: &str,
    stem: &str,
    config: &DrizzleConfig,
) -> anyhow::Result<(String, Option<String>)> {
    let mut header = drizzled_header(reference_header(paths).as_ref(), result.output_scale);
    let included = included_paths(paths, &result.alignment);
    stack_output_cards(
        &mut header,
        &StackCards {
            included_paths: &included,
            combine: CombineMethod::Mean,
            rejection: config.rejection.name(),
            normalization: None,
            drizzle: Some(DrizzleCards {
                scale: result.output_scale,
                pixfrac: config.pixfrac.clamp(0.1, 1.0),
                kernel: kernel_name(config.kernel),
            }),
        },
    );
    render_named_and_save(&result.image, output_dir, stem, true, Some(&header))
}

struct CalibrateInputs<'a> {
    bias: Option<&'a [String]>,
    dark: Option<&'a [String]>,
    flat: Option<&'a [String]>,
    flat_dark: Option<&'a [String]>,
}

fn calibrate_json(
    frame: &CalibratedFrame,
    inputs: &CalibrateInputs,
    png_path: String,
    fits_path: Option<String>,
    elapsed_ms: u64,
) -> serde_json::Value {
    let (rows, cols) = frame.image.dim();
    let stats = compute_image_stats(&frame.image);
    json!({
        RES_PNG_PATH: png_path,
        RES_FITS_PATH: fits_path,
        RES_DIMENSIONS: [cols, rows],
        RES_HAS_BIAS: inputs.bias.is_some(),
        RES_HAS_DARK: inputs.dark.is_some(),
        RES_HAS_FLAT: inputs.flat.is_some(),
        RES_HAS_FLAT_DARK: inputs.flat_dark.is_some_and(|p| !p.is_empty()),
        RES_DARK_SCALE: frame.dark_scale,
        RES_WARNINGS: frame.warnings,
        RES_ELAPSED_MS: elapsed_ms,
        RES_STATS: {
            RES_MIN: stats.min,
            RES_MAX: stats.max,
            RES_MEAN: stats.mean,
            RES_SIGMA: stats.sigma,
        },
    })
}

#[tauri::command]
pub async fn calibrate(
    app: tauri::AppHandle,
    science_path: String,
    output_dir: String,
    bias_paths: Option<Vec<String>>,
    dark_paths: Option<Vec<String>>,
    flat_paths: Option<Vec<String>>,
    flat_dark_paths: Option<Vec<String>>,
    dark_exposure_ratio: Option<f32>,
) -> Result<serde_json::Value, String> {
    let progress = ProgressHandle::new(&app, EVENT_CALIBRATE_PROGRESS, 4);
    let progress_clone = progress.clone();

    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let inputs = CalibrateInputs {
            bias: bias_paths.as_deref(),
            dark: dark_paths.as_deref(),
            flat: flat_paths.as_deref(),
            flat_dark: flat_dark_paths.as_deref(),
        };
        let calibrated = calibrate_from_paths(
            &science_path,
            inputs.bias,
            inputs.dark,
            inputs.flat,
            inputs.flat_dark,
            dark_exposure_ratio,
        )?;

        progress_clone.tick_with_stage(STAGE_RENDER);

        let (png_path, fits_path) = save_calibrated(&calibrated.image, &science_path, &output_dir)?;

        progress_clone.emit_complete();

        Ok(calibrate_json(&calibrated, &inputs, png_path, fits_path, t0.elapsed().as_millis() as u64))
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
            align_method: helpers::parse_align_method_checked(align_method.as_deref())?,
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

        let saved = save_stack(&result, &paths, &output_dir, &stem, &config)?;

        let stats = compute_image_stats(&result.image);

        progress_clone.tick_with_stage(STAGE_SAVE);
        progress_clone.emit_complete();

        Ok(stack_json(&result, &config, &paths, saved, &stats, t0.elapsed().as_millis() as u64))
    })
}

fn stack_json(
    result: &StackResult,
    config: &StackConfig,
    paths: &[String],
    saved: StackOutputs,
    stats: &ImageStats,
    elapsed_ms: u64,
) -> serde_json::Value {
    let (rows, cols) = result.image.dim();
    json!({
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
        RES_ALIGNMENT: alignment_json(paths, &result.alignment),
        RES_WEIGHTS_APPLIED: result.weights_applied,
        RES_WARNINGS: result.warnings,
        RES_REJECTION_LOW_FITS: saved.rejection_low_fits,
        RES_REJECTION_HIGH_FITS: saved.rejection_high_fits,
        RES_ELAPSED_MS: elapsed_ms,
        RES_STATS: {
            RES_MIN: stats.min,
            RES_MAX: stats.max,
            RES_MEAN: stats.mean,
            RES_SIGMA: stats.sigma,
        },
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

        let (png_path, fits_path) = save_drizzle(&result, &paths, &output_dir, &stem, &config)?;

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
    use crate::types::constants::{
        HEADER_COMBINE_METHOD, HEADER_DRIZZLE_SCALE, HEADER_NCOMBINE, HEADER_NORMALIZATION_METHOD,
        HEADER_REJECTION_METHOD, HEADER_TOTEXP,
    };

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

    fn write_constant(dir: &tempfile::TempDir, name: &str, value: f32, cards: &[(&str, &str)]) -> String {
        let path = dir.path().join(name).to_str().unwrap().replace('\\', "/");
        let mut header = HduHeader::empty();
        for (k, v) in cards {
            header.set(k, (*v).to_string());
        }
        let frame = Array2::from_elem((8, 8), value);
        crate::infra::fits::writer::write_fits_mono(&path, &frame, Some(&header)).unwrap();
        path
    }

    fn exposure_header(exptime: &str) -> HduHeader {
        let mut header = solved_header("3.5");
        header.set("EXPTIME", exptime.to_string());
        header
    }

    fn written_header(path: Option<&str>) -> HduHeader {
        crate::infra::fits::reader::read_primary_header(path.expect("a FITS path")).unwrap()
    }

    fn text<'a>(header: &'a HduHeader, key: &str) -> Option<&'a str> {
        header.get(key).map(str::trim)
    }

    fn history(header: &HduHeader) -> Vec<String> {
        header.cards.iter().filter(|(k, _)| k.trim() == "HISTORY").map(|(_, v)| v.clone()).collect()
    }

    fn calibrate(
        science: &str,
        bias: &[String],
        darks: &[String],
        ratio: Option<f32>,
    ) -> anyhow::Result<CalibratedFrame> {
        let some = |paths: &[String]| (!paths.is_empty()).then(|| paths.to_vec());
        calibrate_from_paths(science, some(bias).as_deref(), some(darks).as_deref(), None, None, ratio)
    }

    #[test]
    fn calibrated_fits_keeps_the_science_wcs_and_units() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().replace('\\', "/");
        let science = write_frame(&dir, "science.fits", 100.0, &solved_header("3.5"));
        let calibrated = calibrate_from_paths(&science, None, None, None, None, None).unwrap().image;

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

        let saved = save_stack(&result, &paths, &out, "stacked", &config).unwrap();
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

        let (_, fits) = save_drizzle(&result, &paths, &out, "drizzled", &config).unwrap();
        let header = written_header(fits.as_deref());
        assert_eq!(result.output_dims, (12, 12));
        assert_eq!(header.get_f64("CRPIX1"), Some(5.0));
        assert_eq!(text(&header, "CTYPE1"), Some("RA---TAN"));
        assert_eq!(text(&header, "ABPROC"), Some(ABPROC_DRIZZLED));
    }

    #[test]
    fn stacked_fits_carries_ncombine_totexp_and_combine_cards() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().replace('\\', "/");
        let paths = vec![
            write_frame(&dir, "a.fits", 100.0, &exposure_header("300")),
            write_frame(&dir, "b.fits", 102.0, &exposure_header("300")),
            write_frame(&dir, "c.fits", 101.0, &exposure_header("300")),
        ];
        let config = StackConfig { align: false, ..StackConfig::default() };
        let result = stack_from_paths(&paths, &config, None).unwrap();

        let saved = save_stack(&result, &paths, &out, "stacked", &config).unwrap();
        let header = written_header(saved.fits_path.as_deref());
        assert_eq!(header.get_i64(HEADER_NCOMBINE), Some(3));
        assert_eq!(header.get_f64(HEADER_TOTEXP), Some(900.0));
        assert_eq!(header.get_f64("EXPTIME"), Some(300.0));
        assert_eq!(text(&header, HEADER_COMBINE_METHOD), Some("mean"));
        assert_eq!(text(&header, HEADER_REJECTION_METHOD), Some("sigma_clip"));
        assert_eq!(text(&header, HEADER_NORMALIZATION_METHOD), Some("additive_scaling"));
        assert_eq!(header.get(HEADER_DRIZZLE_SCALE), None);
        let notes = history(&header);
        assert!(notes.iter().any(|h| h.starts_with("AstroBurst stack: 3 frames")), "{notes:?}");
    }

    #[test]
    fn totexp_sums_only_included_frames() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().replace('\\', "/");
        let paths = vec![
            write_frame(&dir, "a.fits", 100.0, &exposure_header("300")),
            write_frame(&dir, "b.fits", 102.0, &exposure_header("300")),
            write_frame(&dir, "c.fits", 101.0, &exposure_header("300")),
        ];
        let result = StackResult {
            image: Array2::from_elem((8, 8), 100.0),
            frame_count: 2,
            rejected_pixels: 0,
            offsets: vec![(0, 0); 3],
            rejection_low: None,
            rejection_high: None,
            normalization_applied: Vec::new(),
            weights_applied: vec![None; 3],
            alignment: vec![
                FrameAlignment::reference(),
                FrameAlignment { method: "phase_correlation".into(), confidence: Some(0.1), included: false },
                FrameAlignment { method: "phase_correlation".into(), confidence: Some(0.9), included: true },
            ],
            warnings: Vec::new(),
        };
        let config = StackConfig::default();

        let saved = save_stack(&result, &paths, &out, "stacked", &config).unwrap();
        let header = written_header(saved.fits_path.as_deref());
        assert_eq!(header.get_i64(HEADER_NCOMBINE), Some(2));
        assert_eq!(header.get_f64(HEADER_TOTEXP), Some(600.0));
        assert!(history(&header).iter().any(|h| h.starts_with("AstroBurst stack: 2 frames")), "{:?}", history(&header));
    }

    #[test]
    fn drizzled_fits_carries_ncombine_abcomb_mean_and_abdrzscl() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().replace('\\', "/");
        let paths: Vec<String> = (0..4)
            .map(|i| write_frame(&dir, &format!("d{i}.fits"), 100.0, &exposure_header("300")))
            .collect();
        let config = DrizzleConfig { scale: 2.0, align: false, ..DrizzleConfig::default() };
        let result = drizzle_from_paths(&paths, &config, None).unwrap();

        let (_, fits) = save_drizzle(&result, &paths, &out, "drizzled", &config).unwrap();
        let header = written_header(fits.as_deref());
        assert_eq!(header.get_i64(HEADER_NCOMBINE), Some(4));
        assert_eq!(text(&header, HEADER_COMBINE_METHOD), Some("mean"));
        assert_eq!(header.get_f64(HEADER_DRIZZLE_SCALE), Some(2.0));
        assert_eq!(header.get(HEADER_NORMALIZATION_METHOD), None);
        assert_eq!(header.get_f64(HEADER_TOTEXP), Some(1200.0));
        let notes = history(&header);
        assert!(
            notes.iter().any(|h| *h == "AstroBurst drizzle: 4 frames, scale 2, pixfrac 0.7, square"),
            "{notes:?}"
        );
    }

    #[test]
    fn calibration_tab_derives_the_dark_scale_from_exposures_with_bias() {
        let dir = tempfile::tempdir().unwrap();
        let science = write_frame(&dir, "light.fits", 1000.0, &exposure_header("120"));
        let darks: Vec<String> = (0..2)
            .map(|i| write_constant(&dir, &format!("dark{i}.fits"), 500.0, &[("EXPTIME", "300"), ("IMAGETYP", "Dark")]))
            .collect();
        let bias: Vec<String> = (0..2)
            .map(|i| write_constant(&dir, &format!("bias{i}.fits"), 100.0, &[("EXPTIME", "0"), ("IMAGETYP", "Bias")]))
            .collect();

        let frame = calibrate(&science, &bias, &darks, None).unwrap();
        assert_eq!(frame.dark_scale, 0.4);
        assert!(frame.warnings.is_empty(), "{:?}", frame.warnings);
        let raw = crate::infra::fits::reader::load_fits_image(&science).unwrap();
        for (calibrated, raw) in frame.image.iter().zip(raw.iter()) {
            let expected = raw - 100.0 - 0.4 * (500.0 - 100.0);
            assert!((calibrated - expected).abs() < 1e-3, "{calibrated} vs {expected}");
        }

        let inputs = CalibrateInputs { bias: Some(&bias), dark: Some(&darks), flat: None, flat_dark: Some(&[]) };
        let json = calibrate_json(&frame, &inputs, "out.png".into(), None, 3);
        let reported = json[RES_DARK_SCALE].as_f64().expect("dark_scale is a number");
        assert!((reported - 0.4).abs() < 1e-6, "{reported}");
        assert_eq!(json[RES_WARNINGS], json!([]));
        assert_eq!(json[RES_HAS_FLAT_DARK], false);
        assert_eq!(json[RES_HAS_DARK], true);
    }

    #[test]
    fn calibration_tab_refuses_a_manual_ratio_without_bias() {
        let dir = tempfile::tempdir().unwrap();
        let science = write_frame(&dir, "light.fits", 1000.0, &exposure_header("120"));
        let darks = vec![write_constant(&dir, "dark0.fits", 500.0, &[("EXPTIME", "300")])];

        let err = calibrate(&science, &[], &darks, Some(0.4)).unwrap_err().to_string();
        assert_eq!(
            err,
            "A dark exposure ratio of 0.4 needs a master bias: without one the master dark still contains the bias level and cannot be scaled. Add bias frames or leave the ratio at 1."
        );
        assert_eq!(calibrate(&science, &[], &darks, Some(1.0)).unwrap().dark_scale, 1.0);
    }

    #[test]
    fn calibration_tab_warns_when_light_and_dark_exposures_differ_without_bias() {
        let dir = tempfile::tempdir().unwrap();
        let science = write_frame(&dir, "light.fits", 1000.0, &exposure_header("120"));
        let darks: Vec<String> = (0..2)
            .map(|i| write_constant(&dir, &format!("dark{i}.fits"), 500.0, &[("EXPTIME", "300")]))
            .collect();

        let frame = calibrate(&science, &[], &darks, None).unwrap();
        assert_eq!(frame.dark_scale, 1.0);
        assert_eq!(
            frame.warnings,
            vec!["lights are 120 s but the darks are 300 s and there is no master bias, so the dark cannot be scaled; the lights get the unscaled 300 s dark. Add bias frames or darks of 120 s.".to_string()]
        );
        for (calibrated, raw) in frame.image.iter().zip(crate::infra::fits::reader::load_fits_image(&science).unwrap().iter()) {
            assert!((calibrated - (raw - 500.0)).abs() < 1e-3, "{calibrated} vs {}", raw - 500.0);
        }

        let same = write_frame(&dir, "light300.fits", 1000.0, &exposure_header("300"));
        assert!(calibrate(&same, &[], &darks, None).unwrap().warnings.is_empty());
    }

    #[test]
    fn calibration_tab_subtracts_the_dark_group_nearest_the_science_temperature() {
        let dir = tempfile::tempdir().unwrap();
        let science = write_constant(&dir, "light.fits", 500.0, &[("EXPTIME", "120"), ("CCD-TEMP", "-9.5")]);
        let darks: Vec<String> = (0..10)
            .map(|i| {
                let (level, temp) = if i < 5 { (20.0, "-20.0") } else { (60.0, "-10.0") };
                write_constant(&dir, &format!("dark{i}.fits"), level, &[("EXPTIME", "120"), ("CCD-TEMP", temp)])
            })
            .collect();

        let frame = calibrate(&science, &[], &darks, None).unwrap();
        assert_eq!(frame.dark_scale, 1.0);
        assert!(frame.warnings.is_empty(), "{:?}", frame.warnings);
        for &v in frame.image.iter() {
            assert!((v - 440.0).abs() < 5.0, "pixel {v}: the -9.5 C science frame must get the -10 C dark group (level 60)");
        }
    }

    #[test]
    fn stack_response_reports_the_effective_weights() {
        let result = StackResult {
            image: Array2::from_elem((2, 2), 1.0),
            frame_count: 2,
            rejected_pixels: 0,
            offsets: vec![(0, 0); 3],
            rejection_low: None,
            rejection_high: None,
            normalization_applied: vec![(0.0, 1.0), (0.0, 2.0)],
            weights_applied: vec![Some(0.4), None, Some(1.6)],
            alignment: vec![
                FrameAlignment::reference(),
                FrameAlignment { method: "phase_correlation".into(), confidence: Some(0.1), included: false },
                FrameAlignment::unaligned(),
            ],
            warnings: Vec::new(),
        };
        let paths = vec!["a.fits".to_string(), "b.fits".to_string(), "c.fits".to_string()];
        let saved = StackOutputs {
            png_path: "stacked.png".into(),
            fits_path: Some("stacked.fits".into()),
            rejection_low_fits: None,
            rejection_high_fits: None,
        };
        let stats = compute_image_stats(&result.image);
        let json = stack_json(&result, &StackConfig::default(), &paths, saved, &stats, 5);
        assert_eq!(json[RES_WEIGHTS_APPLIED], json!([0.4, null, 1.6]));
        assert_eq!(json[RES_FRAME_COUNT], 2);
        assert_eq!(json[RES_ALIGNMENT][1][RES_INCLUDED], false);
    }

    const WFPC2_MOSAICS: [&str; 3] = [
        "C:/astrokit/exampleFits/sample-data/502nmos.fits",
        "C:/astrokit/exampleFits/sample-data/656nmos.fits",
        "C:/astrokit/exampleFits/sample-data/673nmos.fits",
    ];

    #[test]
    #[ignore]
    fn real_data_wfpc2_stack_writes_ncombine_and_totexp() {
        if WFPC2_MOSAICS.iter().any(|p| !std::path::Path::new(p).exists()) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().replace('\\', "/");
        let paths: Vec<String> = WFPC2_MOSAICS.iter().map(|p| p.to_string()).collect();
        let config = StackConfig { align: false, ..StackConfig::default() };
        let result = stack_from_paths(&paths, &config, None).unwrap();
        assert_eq!(result.frame_count, 3);

        let saved = save_stack(&result, &paths, &out, "wfpc2_stack", &config).unwrap();
        let header = written_header(saved.fits_path.as_deref());
        assert_eq!(header.get_i64(HEADER_NCOMBINE), Some(3), "the input NCOMBINE=2 has no ABCOMB and must be overwritten");
        assert_eq!(header.get_f64(HEADER_TOTEXP), Some(3300.0));
        assert_eq!(header.get_f64("EXPTIME"), Some(1100.0));
        assert_eq!(text(&header, HEADER_COMBINE_METHOD), Some("mean"));
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
