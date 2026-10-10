use std::sync::Arc;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, derived_output_header, load_from_cache_or_disk, output_stem, resolve_output_dir, save_auto_stf_preview_png, write_derived_fits, OutputValues};
use crate::cmd::compose::rescale_header_to_grid;
use crate::cmd::processing::local_contrast::source_header;
use crate::core::imaging::background::{extract_background, extract_background_linked, neutralize_background, deband, detect_band_axis, dispersed_exposure, DebandAxis, DebandConfig, BackgroundConfig, BackgroundMode};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::stacking::cfa_guard::cfa_pattern;
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    HEADER_DEBAND, MODE_DIVIDE, PROGRESS_EVENT, PROGRESS_STEPS,
    MIN_GRID_SIZE, MAX_GRID_SIZE, MIN_POLY_DEGREE, MAX_POLY_DEGREE, MIN_ITERATIONS, MAX_ITERATIONS,
    RES_AXIS, RES_CORRECTED_PNG, RES_MODEL_PNG, RES_CORRECTED_FITS, RES_SAMPLE_COUNT,
    RES_RMS_RESIDUAL, RES_ELAPSED_MS, RES_DIMENSIONS, RES_CACHE_KEY,
};
use crate::types::header::HduHeader;
use crate::types::image_ref::sanitize_fragment;

const ABPROC_BG_CORRECTED: &str = "bg_corrected";

enum DebandRequest {
    Fixed(DebandAxis),
    Auto,
}

fn deband_request(mode: &str) -> Option<DebandRequest> {
    match mode {
        "deband_rows" => Some(DebandRequest::Fixed(DebandAxis::Rows)),
        "deband_cols" => Some(DebandRequest::Fixed(DebandAxis::Columns)),
        "deband_both" => Some(DebandRequest::Fixed(DebandAxis::Both)),
        "deband_auto" => Some(DebandRequest::Auto),
        _ => None,
    }
}

fn deband_axis_token(axis: DebandAxis) -> &'static str {
    match axis {
        DebandAxis::Rows => "rows",
        DebandAxis::Columns => "cols",
        DebandAxis::Both => "both",
    }
}

fn dispersed_error(exp_type: &str) -> String {
    format!(
        "De-banding is for imaging frames; this is a dispersed exposure (EXP_TYPE={exp_type}) whose rows and columns carry spectra, so a band model would subtract signal."
    )
}

fn cfa_error(pattern: &str) -> String {
    format!(
        "De-banding is for debayered frames; this is a Bayer mosaic (BAYERPAT={pattern}) whose row and column medians alternate by colour, so a band model would tint the image. Debayer it first (Processing > Debayer)."
    )
}

fn deband_guard<'a>(mut headers: impl Iterator<Item = &'a HduHeader> + Clone) -> anyhow::Result<()> {
    if let Some(exp_type) = headers.clone().find_map(dispersed_exposure) {
        anyhow::bail!("{}", dispersed_error(&exp_type));
    }
    if let Some(pattern) = headers.find_map(cfa_pattern) {
        anyhow::bail!("{}", cfa_error(pattern.name()));
    }
    Ok(())
}

struct SingleOutput {
    corrected: Array2<f32>,
    model: Array2<f32>,
    deband: Option<(DebandAxis, bool)>,
    sample_count: Option<usize>,
    rms_residual: Option<f64>,
    elapsed_ms: u64,
}

fn deband_single(image: &Array2<f32>, request: DebandRequest, sigma_clip: f32, iterations: usize) -> SingleOutput {
    let t0 = std::time::Instant::now();
    let base = DebandConfig { axis: DebandAxis::Both, sigma_clip, iterations: iterations.clamp(1, 5) };
    let (axis, detected) = match request {
        DebandRequest::Fixed(axis) => (axis, false),
        DebandRequest::Auto => (detect_band_axis(image, &base), true),
    };
    let corrected = deband(image, &DebandConfig { axis, ..base });
    let model = Array2::from_shape_fn(image.dim(), |ix| {
        let v = image[ix];
        if v.is_finite() { v - corrected[ix] } else { f32::NAN }
    });
    SingleOutput {
        corrected,
        model,
        deband: Some((axis, detected)),
        sample_count: None,
        rms_residual: None,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    }
}

pub(crate) fn background_config(
    grid_size: usize,
    poly_degree: usize,
    sigma_clip: f64,
    iterations: usize,
    mode: &str,
) -> anyhow::Result<BackgroundConfig> {
    let clip = sigma_clip as f32;
    if !(clip.is_finite() && clip > 0.0) {
        anyhow::bail!("sigma_clip must be a finite number greater than 0, got {}", sigma_clip);
    }
    Ok(BackgroundConfig {
        grid_size: grid_size.clamp(MIN_GRID_SIZE, MAX_GRID_SIZE),
        poly_degree: poly_degree.clamp(MIN_POLY_DEGREE, MAX_POLY_DEGREE),
        sigma_clip: clip,
        iterations: iterations.clamp(MIN_ITERATIONS, MAX_ITERATIONS),
        mode: match mode {
            MODE_DIVIDE => BackgroundMode::Divide,
            _ => BackgroundMode::Subtract,
        },
    })
}

fn background_png_paths(output_dir: &str, stem: &str, bin_id: Option<&str>) -> (String, String) {
    let base = match bin_id {
        Some(bid) => format!("{}_wizard_{}", stem, sanitize_fragment(bid)),
        None => stem.to_string(),
    };
    (
        format!("{}/{}_bg_corrected.png", output_dir, base),
        format!("{}/{}_bg_model.png", output_dir, base),
    )
}

fn run_extract_background(
    path: &str,
    output_dir: &str,
    config: &BackgroundConfig,
    mode: &str,
    iterations: usize,
    bin_id: Option<&str>,
    progress: Option<&ProgressHandle>,
) -> anyhow::Result<serde_json::Value> {
    let entry = load_from_cache_or_disk(path)?;
    let source = source_header(path, &entry);
    let request = deband_request(mode);
    if request.is_some() {
        deband_guard(source.iter())?;
    }
    let output = match request {
        Some(request) => {
            let output = deband_single(entry.arr(), request, config.sigma_clip, iterations);
            if let Some(p) = progress {
                p.emit_complete();
            }
            output
        }
        None => {
            let bg = extract_background(entry.arr(), config, progress)?;
            SingleOutput {
                corrected: bg.corrected,
                model: bg.model,
                deband: None,
                sample_count: Some(bg.sample_count),
                rms_residual: Some(bg.rms_residual),
                elapsed_ms: bg.elapsed_ms,
            }
        }
    };

    let stem = output_stem(path);
    let (corrected_png, model_png) = background_png_paths(output_dir, &stem, bin_id);
    let (rows, cols) = output.corrected.dim();
    save_auto_stf_preview_png(&output.corrected, &corrected_png)?;
    save_auto_stf_preview_png(&output.model, &model_png)?;

    let corrected_fits = format!("{}/{}_bg_corrected.fits", output_dir, stem);
    let mut header = derived_output_header(source.as_ref(), ABPROC_BG_CORRECTED, OutputValues::Linear);
    let axis = output.deband.map(|(axis, _)| deband_axis_token(axis));
    if let Some((axis, detected)) = output.deband {
        let token = deband_axis_token(axis);
        header.set(HEADER_DEBAND, token.to_string());
        let note = if detected { format!("deband {token} (auto)") } else { format!("deband {token}") };
        header.cards.push(("HISTORY".to_string(), note));
    }
    let (cache_key, entry_header) = match bin_id {
        Some(bid) => (crate::types::constants::wizard_bg_key(bid), Some(header)),
        None => {
            write_derived_fits(&corrected_fits, &output.corrected, Some(&header))?;
            (corrected_fits, None)
        }
    };

    let stats = compute_image_stats(&output.corrected);
    GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(&cache_key, Arc::new(output.corrected), stats, entry_header);

    Ok(json!({
        RES_CORRECTED_PNG: corrected_png,
        RES_MODEL_PNG: model_png,
        RES_CORRECTED_FITS: cache_key,
        RES_CACHE_KEY: cache_key,
        RES_SAMPLE_COUNT: output.sample_count,
        RES_RMS_RESIDUAL: output.rms_residual,
        RES_AXIS: axis,
        RES_ELAPSED_MS: output.elapsed_ms,
        RES_DIMENSIONS: [cols, rows],
    }))
}

#[tauri::command]
pub async fn extract_background_cmd(
    app: tauri::AppHandle,
    path: String,
    output_dir: String,
    grid_size: usize,
    poly_degree: usize,
    sigma_clip: f64,
    iterations: usize,
    mode: String,
    bin_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let progress = ProgressHandle::new(&app, PROGRESS_EVENT, PROGRESS_STEPS as u64);

    blocking_cmd!({
        let config = background_config(grid_size, poly_degree, sigma_clip, iterations, &mode)?;
        let output_dir = resolve_output_dir(&output_dir)?;
        run_extract_background(&path, &output_dir, &config, &mode, iterations, bin_id.as_deref(), Some(&progress))
    })
}

fn mean_reference(channels: &[Array2<f32>]) -> Array2<f32> {
    let (rows, cols) = channels[0].dim();
    Array2::from_shape_fn((rows, cols), |(y, x)| {
        let mut sum = 0.0f32;
        let mut cnt = 0u32;
        for ch in channels {
            let v = ch[[y, x]];
            if v.is_finite() {
                sum += v;
                cnt += 1;
            }
        }
        if cnt > 0 {
            sum / cnt as f32
        } else {
            f32::NAN
        }
    })
}

#[tauri::command]
pub async fn extract_background_batch_cmd(
    paths: Vec<String>,
    bin_ids: Vec<String>,
    output_dir: String,
    grid_size: usize,
    poly_degree: usize,
    sigma_clip: f64,
    iterations: usize,
    mode: String,
    reference_bin: Option<String>,
    run_token: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = std::time::Instant::now();
        let config = background_config(grid_size, poly_degree, sigma_clip, iterations, &mode)?;
        resolve_output_dir(&output_dir)?;

        if paths.is_empty() || paths.len() != bin_ids.len() {
            anyhow::bail!("paths and bin_ids must be non-empty and of equal length");
        }

        let deband_mode = deband_request(&mode).is_some();
        let neutralize = mode.as_str() == "neutralize";

        let mut loaded: Vec<Array2<f32>> = Vec::with_capacity(paths.len());
        let mut headers = Vec::with_capacity(paths.len());
        for p in &paths {
            let entry = load_from_cache_or_disk(p)?;
            headers.push(source_header(p, &entry));
            loaded.push(entry.arr().to_owned());
        }
        if deband_mode {
            deband_guard(headers.iter().flatten())?;
        }

        let (rows, cols) = loaded[0].dim();
        for (ch, header) in loaded.iter_mut().zip(headers.iter_mut()).skip(1) {
            if ch.dim() != (rows, cols) {
                if let Some(header) = header {
                    rescale_header_to_grid(header, ch.dim(), (rows, cols));
                }
                *ch = crate::core::imaging::resample::resample_image(ch, rows, cols)?;
            }
        }

        let mut axes: Vec<Option<&'static str>> = vec![None; loaded.len()];

        let (corrected, sample_counts, rms_residual): (Vec<Array2<f32>>, Vec<Option<usize>>, Option<f64>) = if deband_mode {
            let base_cfg = DebandConfig {
                axis: DebandAxis::Both,
                sigma_clip: config.sigma_clip,
                iterations: iterations.clamp(1, 5),
            };
            let mut out = Vec::with_capacity(loaded.len());
            for (i, ch) in loaded.iter().enumerate() {
                let axis = match mode.as_str() {
                    "deband_rows" => DebandAxis::Rows,
                    "deband_cols" => DebandAxis::Columns,
                    "deband_both" => DebandAxis::Both,
                    _ => detect_band_axis(ch, &base_cfg),
                };
                axes[i] = Some(deband_axis_token(axis));
                let dcfg = DebandConfig { axis, ..base_cfg.clone() };
                out.push(deband(ch, &dcfg));
            }
            let counts = vec![None; loaded.len()];
            (out, counts, None)
        } else if neutralize {
            let mut out = Vec::with_capacity(loaded.len());
            let mut counts = Vec::with_capacity(loaded.len());
            for ch in &loaded {
                let r = neutralize_background(ch, &config)?;
                counts.push(Some(r.sample_count));
                out.push(r.corrected);
            }
            (out, counts, None)
        } else {
            let reference = match reference_bin
                .as_ref()
                .and_then(|rb| bin_ids.iter().position(|b| b == rb))
            {
                Some(idx) => loaded[idx].clone(),
                None => mean_reference(&loaded),
            };
            let refs: Vec<&Array2<f32>> = loaded.iter().collect();
            let linked = extract_background_linked(&refs, &reference, &config)?;
            let count = linked.sample_count;
            (linked.corrected, vec![Some(count); loaded.len()], Some(linked.rms_residual))
        };

        let mut results = Vec::with_capacity(paths.len());
        for (i, bin_id) in bin_ids.iter().enumerate() {
            let img = &corrected[i];
            let cache_key = crate::types::constants::wizard_bg_key_for(run_token.as_deref(), bin_id);
            let stats = compute_image_stats(img);
            let header = derived_output_header(headers[i].as_ref(), ABPROC_BG_CORRECTED, OutputValues::Linear);
            GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(&cache_key, Arc::new(img.clone()), stats, Some(header));
            results.push(json!({
                "bin_id": bin_id,
                RES_CACHE_KEY: cache_key,
                RES_SAMPLE_COUNT: sample_counts[i],
                "axis": axes[i],
            }));
        }
        crate::cmd::compose::settle_wizard_run(run_token.as_deref().map(str::trim).filter(|t| !t.is_empty()));

        Ok(json!({
            "results": results,
            "mode": mode,
            RES_RMS_RESIDUAL: rms_residual,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::constants::wizard_bg_key;

    fn sky(seed: usize) -> Array2<f32> {
        Array2::from_shape_fn((64, 64), |(y, x)| {
            let noise = ((y * 31 + x * 17 + seed * 7) % 23) as f32 - 11.0;
            100.0 + 0.5 * x as f32 + 0.25 * y as f32 + noise
        })
    }

    async fn run_batch(mode: &str, tag: &str) -> serde_json::Value {
        let dir = tempfile::tempdir().unwrap();
        let bins: Vec<String> = ["r", "g"].iter().map(|c| format!("bg_batch_{tag}_{c}")).collect();
        let inputs: Vec<String> = bins.iter().map(|b| format!("__bg_batch_input_{b}")).collect();
        for (i, key) in inputs.iter().enumerate() {
            let arr = sky(i);
            GLOBAL_IMAGE_CACHE.insert_synthetic(key, Arc::new(arr.clone()), compute_image_stats(&arr));
        }
        let value = extract_background_batch_cmd(
            inputs.clone(),
            bins.clone(),
            dir.path().to_str().unwrap().to_string(),
            8,
            2,
            3.0,
            3,
            mode.to_string(),
            None,
            None,
        )
        .await;
        for key in inputs.iter().cloned().chain(bins.iter().map(|b| wizard_bg_key(b))) {
            GLOBAL_IMAGE_CACHE.remove(&key);
        }
        value.unwrap()
    }

    fn sky_file(dir: &tempfile::TempDir, name: &str) -> String {
        let path = dir.path().join(name).to_str().unwrap().to_string();
        let mut header = crate::types::header::HduHeader::empty();
        for (k, v) in [("BUNIT", "'MJy/sr'"), ("PHOTMJSR", "1.5"), ("CTYPE1", "'RA---TAN'"), ("CRVAL1", "83.8")] {
            header.set(k, v.to_string());
        }
        crate::infra::fits::writer::write_fits_mono(&path, &sky(0), Some(&header)).unwrap();
        path
    }

    fn config(grid_size: usize) -> BackgroundConfig {
        background_config(grid_size, 2, 3.0, 3, "subtract").unwrap()
    }

    #[tokio::test]
    async fn sigma_clip_values_that_would_poison_the_sky_level_are_refused() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e300, 1e-60] {
            let err = background_config(8, 2, bad, 3, "subtract").unwrap_err().to_string();
            assert!(err.contains("sigma_clip"), "{bad}: {err}");
        }
        assert_eq!(background_config(8, 2, 2.5, 3, "divide").unwrap().sigma_clip, 2.5);

        let dir = tempfile::tempdir().unwrap();
        let key = "__bg_bad_sigma_input".to_string();
        let arr = sky(3);
        GLOBAL_IMAGE_CACHE.insert_synthetic(&key, Arc::new(arr.clone()), compute_image_stats(&arr));
        let refused = extract_background_batch_cmd(
            vec![key.clone()],
            vec!["bg_bad_sigma".to_string()],
            dir.path().to_str().unwrap().to_string(),
            8,
            2,
            0.0,
            3,
            "subtract".to_string(),
            None,
            None,
        )
        .await;
        GLOBAL_IMAGE_CACHE.remove(&key);
        GLOBAL_IMAGE_CACHE.remove(&wizard_bg_key("bg_bad_sigma"));
        assert!(refused.unwrap_err().contains("sigma_clip"));
    }

    #[test]
    fn the_background_corrected_fits_keeps_the_source_calibration_and_wcs() {
        let dir = tempfile::tempdir().unwrap();
        let src = sky_file(&dir, "bg_calibrated.fits");
        let value = run_extract_background(&src, dir.path().to_str().unwrap(), &config(8), "subtract", 3, None, None).unwrap();
        let fits = value[RES_CORRECTED_FITS].as_str().unwrap().to_string();
        GLOBAL_IMAGE_CACHE.remove(&fits);
        let header = crate::cmd::common::cached_header(&fits).unwrap();
        assert_eq!(header.get("BUNIT").map(|v| v.trim().trim_matches('\'').trim()), Some("MJy/sr"));
        assert_eq!(header.get_f64("PHOTMJSR"), Some(1.5));
        assert_eq!(header.get_f64("CRVAL1"), Some(83.8));
        assert_eq!(
            header.get(crate::cmd::common::HEADER_ABPROC).map(|v| v.trim().trim_matches('\'').trim()),
            Some(ABPROC_BG_CORRECTED)
        );
    }

    #[test]
    fn a_wizard_extraction_leaves_the_processing_previews_of_the_same_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap();
        let src = sky_file(&dir, "M42.fits");
        let processing = run_extract_background(&src, out, &config(16), "subtract", 3, None, None).unwrap();
        let corrected_png = processing[RES_CORRECTED_PNG].as_str().unwrap().to_string();
        let model_png = processing[RES_MODEL_PNG].as_str().unwrap().to_string();
        let before = (std::fs::read(&corrected_png).unwrap(), std::fs::read(&model_png).unwrap());

        let wizard = run_extract_background(&src, out, &config(8), "subtract", 3, Some("r"), None).unwrap();
        GLOBAL_IMAGE_CACHE.remove(processing[RES_CORRECTED_FITS].as_str().unwrap());
        GLOBAL_IMAGE_CACHE.remove(&wizard_bg_key("r"));

        for key in [RES_CORRECTED_PNG, RES_MODEL_PNG] {
            let path = wizard[key].as_str().unwrap();
            assert_ne!(path, processing[key].as_str().unwrap(), "{key}");
            assert!(path.contains("_wizard_r_"), "{path}");
            assert!(std::path::Path::new(path).exists(), "{path}");
        }
        let after = (std::fs::read(&corrected_png).unwrap(), std::fs::read(&model_png).unwrap());
        assert!(before == after, "the wizard run overwrote the Processing Background previews");
    }

    #[test]
    fn background_previews_of_a_large_image_match_the_gpu_view_of_the_written_result() {
        use crate::cmd::io::test_support::{assert_matches_gpu_view, wide_sky};
        use crate::core::imaging::stf::{auto_stf, AutoStfConfig};
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("wide_bg.fits").to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&src, &wide_sky(32), None).unwrap();

        let value = run_extract_background(&src, dir.path().to_str().unwrap(), &config(8), "subtract", 3, None, None).unwrap();
        let fits = value[RES_CORRECTED_FITS].as_str().unwrap().to_string();
        GLOBAL_IMAGE_CACHE.remove(&fits);
        let corrected = crate::cmd::common::extract_image_resolved(&fits).unwrap().arr;
        let stats = compute_image_stats(&corrected);
        let stf = auto_stf(&stats, &AutoStfConfig::default());
        assert_matches_gpu_view(value[RES_CORRECTED_PNG].as_str().unwrap(), &corrected, &stf, &stats);
    }

    fn solved_header(crpix: f64, cd: f64) -> crate::types::header::HduHeader {
        let mut header = crate::types::header::HduHeader::empty();
        header.set("CTYPE1", "RA---TAN".to_string());
        header.set("CTYPE2", "DEC--TAN".to_string());
        header.set_f64("CRVAL1", 83.8);
        header.set_f64("CRVAL2", -5.4);
        header.set_f64("CRPIX1", crpix);
        header.set_f64("CRPIX2", crpix);
        header.set_f64("CD1_1", -cd);
        header.set_f64("CD2_2", cd);
        header.set("BUNIT", "MJy/sr".to_string());
        header
    }

    #[tokio::test]
    async fn wizard_background_entries_carry_the_header_of_their_grid() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let inputs = ["__bg_wcs_input_r".to_string(), "__bg_wcs_input_g".to_string()];
        let bins = ["bgw_r".to_string(), "bgw_g".to_string()];
        let full = sky(0);
        let half = Array2::from_shape_fn((32, 32), |(y, x)| full[[2 * y, 2 * x]]);
        GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(&inputs[0], Arc::new(full.clone()), compute_image_stats(&full), Some(solved_header(10.0, 1e-5)));
        GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(&inputs[1], Arc::new(half.clone()), compute_image_stats(&half), Some(solved_header(10.0, 2e-5)));

        let value = extract_background_batch_cmd(
            inputs.to_vec(),
            bins.to_vec(),
            dir.path().to_str().unwrap().to_string(),
            8,
            2,
            3.0,
            3,
            "subtract".to_string(),
            None,
            None,
        )
        .await;
        let headers: Vec<Option<crate::types::header::HduHeader>> = bins
            .iter()
            .map(|b| GLOBAL_IMAGE_CACHE.get(&wizard_bg_key(b)).and_then(|e| e.header().cloned()))
            .collect();
        for key in inputs.iter().cloned().chain(bins.iter().map(|b| wizard_bg_key(b))) {
            GLOBAL_IMAGE_CACHE.remove(&key);
        }
        value.unwrap();

        let r = headers[0].clone().expect("a wizard background channel carries the header of its grid");
        assert_eq!(r.get_f64("CRVAL1"), Some(83.8));
        assert!((r.get_f64("CRPIX1").unwrap() - 10.0).abs() < 1e-9, "CRPIX1 {:?}", r.get("CRPIX1"));
        assert!((r.get_f64("CD2_2").unwrap() - 1e-5).abs() < 1e-18);
        assert_eq!(r.get_i64("NAXIS1"), Some(64));
        assert_eq!(r.get_i64("NAXIS2"), Some(64));
        assert_eq!(r.get("BUNIT"), Some("MJy/sr"));
        assert_eq!(r.get(crate::cmd::common::HEADER_ABPROC), Some(ABPROC_BG_CORRECTED));

        let g = headers[1].clone().expect("a resampled wizard background channel carries the header of its grid");
        assert!((g.get_f64("CRPIX1").unwrap() - 19.5).abs() < 1e-9, "CRPIX1 of the 32 px input was not moved to the 64 px grid: {:?}", g.get("CRPIX1"));
        assert!((g.get_f64("CD2_2").unwrap() - 1e-5).abs() < 1e-18, "CD of the 32 px input was not halved on the 64 px grid: {:?}", g.get("CD2_2"));
        assert_eq!(g.get_i64("NAXIS1"), Some(64));
        assert_eq!(g.get_i64("NAXIS2"), Some(64));
    }

    #[test]
    fn a_wizard_background_of_a_file_carries_its_header() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let src = sky_file(&dir, "bg_wizard_header.fits");
        let value = run_extract_background(&src, dir.path().to_str().unwrap(), &config(8), "subtract", 3, Some("bgw_single"), None).unwrap();
        let key = wizard_bg_key("bgw_single");
        let header = GLOBAL_IMAGE_CACHE.get(&key).and_then(|e| e.header().cloned());
        GLOBAL_IMAGE_CACHE.remove(&key);
        assert_eq!(value[RES_CACHE_KEY], key);
        let header = header.expect("a wizard background channel carries the header of its grid");
        assert_eq!(header.get_f64("CRVAL1"), Some(83.8));
        assert_eq!(header.get("BUNIT").map(|v| v.trim().trim_matches('\'').trim()), Some("MJy/sr"));
        assert_eq!(header.get_f64("PHOTMJSR"), Some(1.5));
        assert_eq!(header.get_i64("NAXIS1"), Some(64));
        assert_eq!(header.get_i64("NAXIS2"), Some(64));
        assert_eq!(header.get(crate::cmd::common::HEADER_ABPROC), Some(ABPROC_BG_CORRECTED));
    }

    #[tokio::test]
    async fn modes_that_fit_no_model_report_no_residual_instead_of_a_perfect_one() {
        let deband = run_batch("deband_rows", "deband").await;
        assert!(deband[RES_RMS_RESIDUAL].is_null(), "{deband}");
        for r in deband["results"].as_array().unwrap() {
            assert!(r[RES_SAMPLE_COUNT].is_null(), "de-band measured no samples: {r}");
        }

        let neutralize = run_batch("neutralize", "neutralize").await;
        assert!(neutralize[RES_RMS_RESIDUAL].is_null(), "{neutralize}");
        for r in neutralize["results"].as_array().unwrap() {
            assert!(r[RES_SAMPLE_COUNT].as_u64().is_some(), "{r}");
        }

        let fitted = run_batch("subtract", "fitted").await;
        assert!(fitted[RES_RMS_RESIDUAL].as_f64().is_some_and(|v| v > 0.0), "{fitted}");
    }

    struct Lcg(u64);

    impl Lcg {
        fn uniform(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }

        fn gaussian(&mut self) -> f32 {
            let s: f32 = (0..4).map(|_| self.uniform()).sum();
            (s - 2.0) * 3.0f32.sqrt()
        }
    }

    fn banded_sky(seed: u64) -> Array2<f32> {
        let mut rng = Lcg(seed);
        let offsets: Vec<f32> = (0..64).map(|_| (rng.uniform() * 2.0 - 1.0) * 8.0).collect();
        Array2::from_shape_fn((64, 64), |(y, _)| 100.0 + rng.gaussian() + offsets[y])
    }

    fn row_median_std(arr: &Array2<f32>) -> f32 {
        let medians: Vec<f32> = arr
            .rows()
            .into_iter()
            .map(|row| {
                let mut v: Vec<f32> = row.iter().copied().collect();
                v.sort_by(|a, b| a.total_cmp(b));
                v[v.len() / 2]
            })
            .collect();
        let mean = medians.iter().sum::<f32>() / medians.len() as f32;
        (medians.iter().map(|m| (m - mean).powi(2)).sum::<f32>() / medians.len() as f32).sqrt()
    }

    fn header_text<'a>(header: &'a crate::types::header::HduHeader, key: &str) -> Option<&'a str> {
        header.get(key).map(|v| v.trim().trim_matches('\'').trim())
    }

    #[test]
    fn single_file_deband_rows_removes_the_row_pattern_and_reports_the_axis() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("banded.fits").to_str().unwrap().to_string();
        let banded = banded_sky(7);
        let mut header = crate::types::header::HduHeader::empty();
        header.set("EXP_TYPE", "'NRC_IMAGE'".to_string());
        crate::infra::fits::writer::write_fits_mono(&src, &banded, Some(&header)).unwrap();
        let std_before = row_median_std(&banded);
        assert!(std_before > 3.0, "the fixture carries no row pattern: {std_before}");

        let value = run_extract_background(&src, dir.path().to_str().unwrap(), &config(8), "deband_auto", 3, None, None).unwrap();
        let fits = value[RES_CORRECTED_FITS].as_str().unwrap().to_string();
        GLOBAL_IMAGE_CACHE.remove(&fits);
        let corrected = crate::cmd::common::extract_image_resolved(&fits).unwrap().arr;
        let written = crate::cmd::common::cached_header(&fits).unwrap();

        assert_eq!(value[RES_AXIS], "rows", "{value}");
        assert!(value[RES_SAMPLE_COUNT].is_null() && value[RES_RMS_RESIDUAL].is_null(), "{value}");
        let std_after = row_median_std(&corrected);
        assert!(std_after < 0.3, "row-median std after deband: {std_after} (before {std_before})");
        assert_eq!(header_text(&written, HEADER_DEBAND), Some("rows"));
        assert_eq!(header_text(&written, crate::cmd::common::HEADER_ABPROC), Some(ABPROC_BG_CORRECTED));
        assert!(
            written.cards.iter().any(|(k, v)| k.trim() == "HISTORY" && v.contains("deband rows (auto)")),
            "{:?}",
            written.cards
        );
        assert!(std::path::Path::new(value[RES_MODEL_PNG].as_str().unwrap()).exists());
    }

    async fn deband_both_commands(key: &str, out: &str) -> (anyhow::Result<serde_json::Value>, Result<serde_json::Value, String>) {
        let single = run_extract_background(key, out, &config(8), "deband_auto", 3, None, None);
        let batch = extract_background_batch_cmd(
            vec![key.to_string()],
            vec![format!("guard_{}", sanitize_fragment(key))],
            out.to_string(),
            8,
            2,
            3.0,
            3,
            "deband_rows".to_string(),
            None,
            None,
        )
        .await;
        (single, batch)
    }

    fn insert_with_cards(key: &str, cards: &[(&str, String)]) {
        let arr = sky(1);
        let mut header = crate::types::header::HduHeader::empty();
        for (k, v) in cards {
            header.set(k, v.clone());
        }
        GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(key, Arc::new(arr.clone()), compute_image_stats(&arr), Some(header));
    }

    fn insert_with_exp_type(key: &str, exp_type: Option<&str>) {
        let cards: Vec<(&str, String)> = exp_type.map(|v| ("EXP_TYPE", format!("'{v}'"))).into_iter().collect();
        insert_with_cards(key, &cards);
    }

    #[tokio::test]
    async fn deband_refuses_bayer_mosaics() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        for (card, pattern) in [("BAYERPAT", "RGGB"), ("BAYERPAT", "GRBG"), ("COLORTYP", "BGGR")] {
            let key = format!("__deband_cfa_{card}_{pattern}");
            insert_with_cards(&key, &[(card, format!("'{pattern}'"))]);
            let (single, batch) = deband_both_commands(&key, &out).await;
            GLOBAL_IMAGE_CACHE.remove(&key);
            let needle = format!("Bayer mosaic (BAYERPAT={pattern})");
            let single_err = format!("{:#}", single.expect_err(pattern));
            assert!(single_err.contains(&needle), "{card}={pattern}: {single_err}");
            assert!(single_err.contains("Debayer it first"), "{card}={pattern}: {single_err}");
            let batch_err = batch.expect_err(pattern);
            assert!(batch_err.contains(&needle), "{card}={pattern}: {batch_err}");
        }
        let key = "__deband_cfa_subtract";
        insert_with_cards(key, &[("BAYERPAT", "'RGGB'".to_string())]);
        let subtract = run_extract_background(key, &out, &config(8), "subtract", 3, None, None);
        GLOBAL_IMAGE_CACHE.remove(key);
        let subtract = subtract.unwrap_or_else(|e| panic!("subtract on a Bayer frame: {e:#}"));
        GLOBAL_IMAGE_CACHE.remove(subtract[RES_CORRECTED_FITS].as_str().unwrap());
    }

    #[tokio::test]
    async fn deband_refuses_dispersed_exposures() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        for exp_type in ["NRS_IFU", "NRS_FIXEDSLIT", "MIR_MRS", "MIR_LRS-FIXEDSLIT", "NIS_SOSS", "NIS_WFSS", "NRC_GRISM", "NRC_WFSS", "NRC_TSGRISM"] {
            let key = format!("__deband_guard_{}", sanitize_fragment(exp_type));
            insert_with_exp_type(&key, Some(exp_type));
            let (single, batch) = deband_both_commands(&key, &out).await;
            GLOBAL_IMAGE_CACHE.remove(&key);
            let single_err = format!("{:#}", single.expect_err(exp_type));
            assert!(single_err.contains("dispersed exposure (EXP_TYPE="), "{exp_type}: {single_err}");
            assert!(single_err.contains(exp_type), "{exp_type}: {single_err}");
            let batch_err = batch.expect_err(exp_type);
            assert!(batch_err.contains("dispersed exposure (EXP_TYPE="), "{exp_type}: {batch_err}");
        }
        for exp_type in [Some("NRC_IMAGE"), Some("MIR_IMAGE"), Some("NIS_IMAGE"), None] {
            let key = format!("__deband_guard_ok_{}", exp_type.unwrap_or("none"));
            insert_with_exp_type(&key, exp_type);
            let (single, batch) = deband_both_commands(&key, &out).await;
            GLOBAL_IMAGE_CACHE.remove(&key);
            let single = single.unwrap_or_else(|e| panic!("{exp_type:?}: {e:#}"));
            GLOBAL_IMAGE_CACHE.remove(single[RES_CORRECTED_FITS].as_str().unwrap());
            let batch = batch.unwrap_or_else(|e| panic!("{exp_type:?}: {e}"));
            for r in batch["results"].as_array().unwrap() {
                GLOBAL_IMAGE_CACHE.remove(r[RES_CACHE_KEY].as_str().unwrap());
            }
        }
    }

    #[tokio::test]
    async fn batch_deband_reports_the_single_file_axis_token() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let key = "__deband_axis_token_input";
        insert_with_exp_type(key, Some("NRC_IMAGE"));
        let batch = extract_background_batch_cmd(
            vec![key.to_string()],
            vec!["axis_token".to_string()],
            out,
            8,
            2,
            3.0,
            3,
            "deband_cols".to_string(),
            None,
            None,
        )
        .await;
        GLOBAL_IMAGE_CACHE.remove(key);
        let batch = batch.unwrap();
        let results = batch["results"].as_array().unwrap();
        for r in results {
            GLOBAL_IMAGE_CACHE.remove(r[RES_CACHE_KEY].as_str().unwrap());
        }
        assert_eq!(results[0]["axis"], deband_axis_token(DebandAxis::Columns), "{batch}");
        assert_eq!(results[0]["axis"], "cols", "{batch}");
    }

    #[test]
    fn the_deband_model_is_the_band_pattern_and_keeps_nan() {
        let mut image = banded_sky(7);
        image[[5, 6]] = f32::NAN;
        let out = deband_single(&image, DebandRequest::Fixed(DebandAxis::Rows), 3.0, 3);
        assert_eq!(out.model.dim(), image.dim());
        assert!(out.model[[5, 6]].is_nan() && out.corrected[[5, 6]].is_nan());
        assert_eq!(out.model.iter().filter(|v| v.is_nan()).count(), 1);

        let row_levels: Vec<f32> = image
            .rows()
            .into_iter()
            .map(|row| {
                let mut v: Vec<f32> = row.iter().copied().filter(|v| v.is_finite()).collect();
                v.sort_by(|a, b| a.total_cmp(b));
                v[v.len() / 2]
            })
            .collect();
        let mut sorted = row_levels.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let global = sorted[sorted.len() / 2];
        for (y, row) in out.model.rows().into_iter().enumerate() {
            let finite: Vec<f32> = row.iter().copied().filter(|v| v.is_finite()).collect();
            let spread = finite.iter().cloned().fold(f32::MIN, f32::max) - finite.iter().cloned().fold(f32::MAX, f32::min);
            assert!(spread < 1e-3, "row {y}: the model is not a per-row constant (spread {spread})");
            let expected = row_levels[y] - global;
            assert!((finite[0] - expected).abs() < 0.5, "row {y}: model {} vs band {expected}", finite[0]);
        }
        assert!(out.model.iter().any(|v| v.abs() > 2.0), "the model carries no band pattern");
    }

    #[tokio::test]
    async fn batch_background_keys_carry_the_run_token() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let input = "__bg_token_input_r".to_string();
        let arr = sky(2);
        GLOBAL_IMAGE_CACHE.insert_synthetic(&input, Arc::new(arr.clone()), compute_image_stats(&arr));
        let run = |token: Option<&str>| {
            extract_background_batch_cmd(
                vec![input.clone()],
                vec!["r".to_string()],
                out.clone(),
                8,
                2,
                3.0,
                3,
                "subtract".to_string(),
                None,
                token.map(str::to_string),
            )
        };
        *crate::cmd::compose::latest_wizard_run() = Some("t1".to_string());
        let tokened = run(Some("t1")).await.unwrap();
        let legacy = run(None).await.unwrap();
        GLOBAL_IMAGE_CACHE.remove(&input);
        let tokened_key = tokened["results"][0][RES_CACHE_KEY].as_str().unwrap().to_string();
        let legacy_key = legacy["results"][0][RES_CACHE_KEY].as_str().unwrap().to_string();
        let stored = GLOBAL_IMAGE_CACHE.get(&tokened_key).is_some();
        GLOBAL_IMAGE_CACHE.remove(&tokened_key);
        GLOBAL_IMAGE_CACHE.remove(&legacy_key);
        assert_eq!(tokened_key, "__wizard_ch_tt1_r_bg");
        assert_eq!(legacy_key, "__wizard_ch_r_bg");
        assert!(stored, "the tokened key was not inserted");
    }

    #[tokio::test]
    async fn a_late_batch_background_removes_its_keys_when_a_newer_run_started() {
        use crate::cmd::compose::latest_wizard_run;
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let input = "__bg_late_input_r".to_string();
        let arr = sky(4);
        GLOBAL_IMAGE_CACHE.insert_synthetic(&input, Arc::new(arr.clone()), compute_image_stats(&arr));
        let run = |token: Option<&str>| {
            extract_background_batch_cmd(
                vec![input.clone()],
                vec!["r".to_string()],
                out.clone(),
                8,
                2,
                3.0,
                3,
                "subtract".to_string(),
                None,
                token.map(str::to_string),
            )
        };

        *latest_wizard_run() = Some("newer".to_string());
        let late = run(Some("t1")).await.unwrap();
        let late_key = late["results"][0][RES_CACHE_KEY].as_str().unwrap().to_string();
        let late_left = GLOBAL_IMAGE_CACHE.contains(&late_key);
        let latest_after_late = latest_wizard_run().clone();

        let legacy = run(None).await.unwrap();
        let legacy_key = legacy["results"][0][RES_CACHE_KEY].as_str().unwrap().to_string();
        let legacy_left = GLOBAL_IMAGE_CACHE.contains(&legacy_key);

        *latest_wizard_run() = Some("t1".to_string());
        let current = run(Some("t1")).await.unwrap();
        let current_key = current["results"][0][RES_CACHE_KEY].as_str().unwrap().to_string();
        let current_left = GLOBAL_IMAGE_CACHE.contains(&current_key);

        GLOBAL_IMAGE_CACHE.remove(&input);
        for key in [&late_key, &legacy_key, &current_key] {
            GLOBAL_IMAGE_CACHE.remove(key);
        }

        assert_eq!(late_key, "__wizard_ch_tt1_r_bg");
        assert!(!late_left, "a batch that finishes after a newer run started must remove its own keys");
        assert_eq!(latest_after_late.as_deref(), Some("newer"), "a late batch never claims LATEST_WIZARD_RUN");
        assert_eq!(legacy_key, "__wizard_ch_r_bg");
        assert!(legacy_left, "a legacy run never applies the late-finisher removal");
        assert_eq!(current_key, late_key);
        assert!(current_left, "the latest run keeps its keys");
    }
}
