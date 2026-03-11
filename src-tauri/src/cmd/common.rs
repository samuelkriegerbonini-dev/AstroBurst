use std::fs::File;

use anyhow::{Context, Result};
use ndarray::Array2;

use crate::core::imaging::normalize::robust_asinh_preview;
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{auto_stf, apply_stf, AutoStfConfig, StfParams};
use crate::infra::cache::{GLOBAL_IMAGE_CACHE, ImageEntry};
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::reader::extract_image_mmap;
use crate::infra::render::grayscale::{render_grayscale, save_stf_png};
use crate::types::header::HduHeader;
use crate::types::image::ImageStats;

pub struct ResolvedImage {
    pub arr: Array2<f32>,
    pub header: HduHeader,
    pub _tmp: Option<tempfile::TempDir>,
}

fn is_calib_ref_asdf(path: &std::path::Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if !name.starts_with("jwst_") || !name.ends_with(".asdf") {
        return false;
    }
    const PATTERNS: &[&str] = &[
        "distortion", "filteroffset", "sirskernel", "photom",
        "flat", "dark", "bias", "readnoise", "gain", "linearity",
        "saturation", "superbias", "ipc", "area", "specwcs",
        "regions", "wavelengthrange", "trappars", "mask",
        "drizpars", "throughput", "psfmask",
    ];
    PATTERNS.iter().any(|p| name.contains(p))
}

pub fn extract_image_resolved(path: &str) -> Result<ResolvedImage> {
    let p = std::path::Path::new(path);
    if crate::infra::asdf::converter::is_asdf_file(p) {
        if is_calib_ref_asdf(p) {
            anyhow::bail!("Calibration reference file (no image data): ...");
        }
        match crate::infra::asdf_bridge::extract_image_from_asdf(p) {
            Ok(result) => return Ok(ResolvedImage { arr: result.image, header: result.header, _tmp: None }),
            Err(e) if e.to_string().contains("Missing field: data array") => {
                let fits_path = p.with_extension("fits");
                if fits_path.exists() {
                    let file = File::open(&fits_path)?;
                    let result = extract_image_mmap(&file)?;
                    return Ok(ResolvedImage { arr: result.image, header: result.header, _tmp: None });
                }
                anyhow::bail!("ASDF has no image data and no companion .fits found");
            }
            Err(e) => return Err(e.into()),
        }
    }

    let (fits_path, tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)
        .with_context(|| format!("Failed to open {}", fits_path.display()))?;
    let result = extract_image_mmap(&file)?;
    Ok(ResolvedImage {
        arr: result.image,
        header: result.header,
        _tmp: tmp,
    })
}

pub fn load_fits_array(path: &str) -> Result<Array2<f32>> {
    let (fits_path, _tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)
        .with_context(|| format!("Failed to open {}", fits_path.display()))?;
    let result = extract_image_mmap(&file)?;
    Ok(result.image)
}

pub fn load_cached(path: &str) -> Result<ImageEntry> {
    GLOBAL_IMAGE_CACHE.get_or_load(path, || {
        let p = std::path::Path::new(path);
        if crate::infra::asdf::converter::is_asdf_file(p) {
            if is_calib_ref_asdf(p) {
                anyhow::bail!(
                    "Calibration reference file (no image data): {}",
                    p.file_name().and_then(|n| n.to_str()).unwrap_or(path)
                );
            }
            let result = crate::infra::asdf_bridge::extract_image_from_asdf(p)?;
            let stats = compute_image_stats(&result.image);
            return Ok((result.image, stats));
        }

        let (fits_path, _tmp) = resolve_single_image(path)?;
        let file = File::open(&fits_path)?;
        let result = extract_image_mmap(&file)?;
        let stats = compute_image_stats(&result.image);
        Ok((result.image, stats))
    })
}

pub fn load_cached_full(path: &str) -> Result<ImageEntry> {
    GLOBAL_IMAGE_CACHE.get_or_load_full(path, || {
        let p = std::path::Path::new(path);
        if crate::infra::asdf::converter::is_asdf_file(p) {
            if is_calib_ref_asdf(p) {
                anyhow::bail!(
                    "Calibration reference file (no image data): {}",
                    p.file_name().and_then(|n| n.to_str()).unwrap_or(path)
                );
            }
            let result = crate::infra::asdf_bridge::extract_image_from_asdf(p)?;
            let stats = compute_image_stats(&result.image);
            return Ok((result.image, stats, result.header));
        }

        let (fits_path, _tmp) = resolve_single_image(path)?;
        let file = File::open(&fits_path)?;
        let result = extract_image_mmap(&file)?;
        let stats = compute_image_stats(&result.image);
        Ok((result.image, stats, result.header))
    })
}

pub fn load_from_cache_or_disk(path: &str) -> Result<ImageEntry> {
    let cache_result = GLOBAL_IMAGE_CACHE.get(path);
    if let Some(entry) = cache_result {
        Ok(entry)
    } else {
        let resolved = extract_image_resolved(path)?;
        let stats = compute_image_stats(&resolved.arr);
        GLOBAL_IMAGE_CACHE.get_or_load(path, || Ok((resolved.arr, stats)))
    }
}

fn make_filename(stem: &str, suffix: &str, ext: &str) -> String {
    if suffix.is_empty() {
        format!("{}.{}", stem, ext)
    } else {
        format!("{}_{}.{}", stem, suffix, ext)
    }
}

pub struct RenderOutput {
    pub png_path: String,
    pub fits_path: Option<String>,
    pub stats: ImageStats,
    pub stf: StfParams,
    pub dims: (usize, usize),
}

pub fn render_and_save(
    arr: &Array2<f32>,
    path: &str,
    output_dir: &str,
    suffix: &str,
    write_fits: bool,
) -> Result<RenderOutput> {
    let stats = compute_image_stats(arr);
    let stf_params = auto_stf(&stats, &AutoStfConfig::default());
    let rendered = apply_stf(arr, &stf_params, &stats);

    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");

    let png_path = format!("{}/{}", output_dir, make_filename(stem, suffix, "png"));
    let (rows, cols) = arr.dim();
    save_stf_png(&rendered, cols, rows, &png_path)?;

    let fits_path = if write_fits {
        let fp = format!("{}/{}", output_dir, make_filename(stem, suffix, "fits"));
        crate::infra::fits::writer::write_fits_mono(&fp, arr, None)?;
        Some(fp)
    } else {
        None
    };

    Ok(RenderOutput {
        png_path,
        fits_path,
        stats,
        stf: stf_params,
        dims: (rows, cols),
    })
}

pub fn render_asinh_and_save(
    arr: &Array2<f32>,
    output_dir: &str,
    name: &str,
    write_fits: bool,
) -> Result<(String, Option<String>)> {
    let normalized = robust_asinh_preview(arr);
    let png_path = format!("{}/{}.png", output_dir, name);
    render_grayscale(&normalized, &png_path)?;

    let fits_path = if write_fits {
        let fp = format!("{}/{}.fits", output_dir, name);
        crate::infra::fits::writer::write_fits_mono(&fp, arr, None)?;
        Some(fp)
    } else {
        None
    };

    Ok((png_path, fits_path))
}

pub fn resolve_output_dir(output_dir: &str) -> Result<String> {
    let path = std::path::Path::new(output_dir);
    if !path.exists() {
        std::fs::create_dir_all(path).context("Failed to create output directory")?;
    }
    Ok(output_dir.to_string())
}

macro_rules! blocking_cmd {
    ($body:expr) => {
        tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> { $body })
            .await
            .map_err(|e| format!("Task join failed: {}", e))?
            .map_err(|e| format!("{:#}", e))
    };
}

pub(crate) use blocking_cmd;
