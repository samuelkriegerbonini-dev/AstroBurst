use std::fs::File;

use anyhow::{Context, Result};
use ndarray::Array2;

use crate::core::imaging::normalize::robust_asinh_preview;
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{auto_stf, apply_stf, AutoStfConfig};
use crate::infra::cache::{GLOBAL_IMAGE_CACHE, ImageEntry};
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::reader::extract_image_mmap;
use crate::infra::render::grayscale::{render_grayscale, save_stf_png};
use crate::types::header::HduHeader;
use crate::types::image::ImageStats;

pub(crate) const MAX_PREVIEW_DIM: usize = 4096;

pub(crate) fn auto_stretch_preview(arr: &Array2<f32>) -> Vec<u8> {
    let stats = compute_image_stats(arr);
    let stf = auto_stf(&stats, &AutoStfConfig::default());
    apply_stf(arr, &stf, &stats)
}

pub(crate) struct ResolvedImage {
    pub arr: Array2<f32>,
    pub header: HduHeader,
    pub _tmp: Option<tempfile::TempDir>,
}

const CALIB_PATTERNS: &[&str] = &[
    "distortion", "filteroffset", "sirskernel", "photom",
    "flat", "dark", "bias", "readnoise", "gain", "linearity",
    "saturation", "superbias", "ipc", "area", "specwcs",
    "regions", "wavelengthrange", "trappars", "mask",
    "drizpars", "throughput", "psfmask",
];

fn is_calib_ref_asdf(path: &std::path::Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    name.starts_with("jwst_")
        && name.ends_with(".asdf")
        && CALIB_PATTERNS.iter().any(|p| name.contains(p))
}

fn bail_if_calib(path: &std::path::Path) -> Result<()> {
    if is_calib_ref_asdf(path) {
        anyhow::bail!(
            "Calibration reference file (no image data): {}",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown")
        );
    }
    Ok(())
}

fn try_asdf_image(p: &std::path::Path) -> Result<ResolvedImage> {
    bail_if_calib(p)?;
    match crate::infra::asdf_bridge::extract_image_from_asdf(p) {
        Ok(result) => Ok(ResolvedImage { arr: result.image, header: result.header, _tmp: None }),
        Err(e) if e.to_string().contains("Missing field: data array") => {
            let fits_path = p.with_extension("fits");
            if fits_path.exists() {
                let file = File::open(&fits_path)?;
                let result = extract_image_mmap(&file)?;
                return Ok(ResolvedImage { arr: result.image, header: result.header, _tmp: None });
            }
            anyhow::bail!("ASDF has no image data and no companion .fits found");
        }
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn extract_image_resolved(path: &str) -> Result<ResolvedImage> {
    let p = std::path::Path::new(path);
    if crate::infra::asdf::converter::is_asdf_file(p) {
        return try_asdf_image(p);
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

fn load_image_and_stats(path: &str) -> Result<(Array2<f32>, ImageStats)> {
    let p = std::path::Path::new(path);
    if crate::infra::asdf::converter::is_asdf_file(p) {
        bail_if_calib(p)?;
        let result = crate::infra::asdf_bridge::extract_image_from_asdf(p)?;
        let stats = compute_image_stats(&result.image);
        return Ok((result.image, stats));
    }

    let (fits_path, _tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)?;
    let result = extract_image_mmap(&file)?;
    let stats = compute_image_stats(&result.image);
    Ok((result.image, stats))
}

fn load_image_stats_header(path: &str) -> Result<(Array2<f32>, ImageStats, HduHeader)> {
    let p = std::path::Path::new(path);
    if crate::infra::asdf::converter::is_asdf_file(p) {
        bail_if_calib(p)?;
        let result = crate::infra::asdf_bridge::extract_image_from_asdf(p)?;
        let stats = compute_image_stats(&result.image);
        return Ok((result.image, stats, result.header));
    }

    let (fits_path, _tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)?;
    let result = extract_image_mmap(&file)?;
    let stats = compute_image_stats(&result.image);
    Ok((result.image, stats, result.header))
}

pub(crate) fn load_cached(path: &str) -> Result<ImageEntry> {
    GLOBAL_IMAGE_CACHE.get_or_load(path, || load_image_and_stats(path))
}

pub(crate) fn load_cached_full(path: &str) -> Result<ImageEntry> {
    if let Some(entry) = GLOBAL_IMAGE_CACHE.get(path) {
        if entry.header().is_some() {
            return Ok(entry);
        }
        if let Ok(upgraded) = GLOBAL_IMAGE_CACHE.upgrade_header(path, || {
            let resolved = extract_image_resolved(path)?;
            Ok(resolved.header)
        }) {
            return Ok(upgraded);
        }
    }
    GLOBAL_IMAGE_CACHE.get_or_load_full(path, || load_image_stats_header(path))
}

pub(crate) fn load_from_cache_or_disk(path: &str) -> Result<ImageEntry> {
    if let Some(entry) = GLOBAL_IMAGE_CACHE.get(path) {
        return Ok(entry);
    }
    let resolved = extract_image_resolved(path)?;
    let stats = compute_image_stats(&resolved.arr);
    GLOBAL_IMAGE_CACHE.get_or_load(path, || Ok((resolved.arr, stats)))
}

fn downsample_nn<const BPP: usize>(
    pixels: &[u8],
    width: usize,
    height: usize,
    max_dim: usize,
) -> (Vec<u8>, usize, usize) {
    if width <= max_dim && height <= max_dim {
        return (pixels.to_vec(), width, height);
    }

    let scale = max_dim as f64 / (width.max(height) as f64);
    let dst_w = ((width as f64) * scale).round().max(1.0) as usize;
    let dst_h = ((height as f64) * scale).round().max(1.0) as usize;

    let y_ratio = height as f64 / dst_h as f64;
    let x_ratio = width as f64 / dst_w as f64;

    let mut out = vec![0u8; dst_w * dst_h * BPP];

    for dy in 0..dst_h {
        let sy = ((dy as f64) * y_ratio).min((height - 1) as f64) as usize;
        let src_row = sy * width;
        let dst_row = dy * dst_w;
        for dx in 0..dst_w {
            let sx = ((dx as f64) * x_ratio).min((width - 1) as f64) as usize;
            let si = (src_row + sx) * BPP;
            let di = (dst_row + dx) * BPP;
            out[di..di + BPP].copy_from_slice(&pixels[si..si + BPP]);
        }
    }

    (out, dst_w, dst_h)
}

pub(crate) fn downsample_u8(pixels: &[u8], width: usize, height: usize, max_dim: usize) -> (Vec<u8>, usize, usize) {
    downsample_nn::<1>(pixels, width, height, max_dim)
}

pub(crate) fn save_preview_png(pixels: Vec<u8>, width: usize, height: usize, path: &str) -> Result<()> {
    let (preview, pw, ph) = downsample_u8(&pixels, width, height, MAX_PREVIEW_DIM);
    save_stf_png(preview, pw, ph, path)
}

fn make_filename(stem: &str, suffix: &str, ext: &str) -> String {
    if suffix.is_empty() {
        format!("{}.{}", stem, ext)
    } else {
        format!("{}_{}.{}", stem, suffix, ext)
    }
}

pub(crate) struct RenderOutput {
    pub png_path: String,
    pub fits_path: Option<String>,
    pub dims: (usize, usize),
}

pub(crate) fn render_and_save(
    arr: &Array2<f32>,
    path: &str,
    output_dir: &str,
    suffix: &str,
    write_fits: bool,
) -> Result<RenderOutput> {
    let rendered = auto_stretch_preview(arr);

    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");

    let png_path = format!("{}/{}", output_dir, make_filename(stem, suffix, "png"));
    let (rows, cols) = arr.dim();
    save_preview_png(rendered, cols, rows, &png_path)?;

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
        dims: (rows, cols),
    })
}

pub(crate) fn render_asinh_and_save(
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

fn platform_fallback_dir() -> std::path::PathBuf {
    if let Some(data) = dirs::data_dir() {
        return data.join("AstroBurst").join("output");
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".astroburst").join("output");
    }
    std::path::PathBuf::from("/tmp/astroburst/output")
}

pub(crate) fn resolve_output_dir(output_dir: &str) -> Result<String> {
    let path = std::path::Path::new(output_dir);
    if path.exists() {
        maybe_enforce_lru(output_dir);
        return Ok(output_dir.to_string());
    }
    match std::fs::create_dir_all(path) {
        Ok(_) => Ok(output_dir.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied
            || e.raw_os_error() == Some(5)
            || e.raw_os_error() == Some(30) => {
            let fallback = platform_fallback_dir();
            std::fs::create_dir_all(&fallback)
                .context("Failed to create fallback output directory")?;
            eprintln!(
                "[AstroBurst] Permission denied on '{}', falling back to '{}'",
                output_dir,
                fallback.display()
            );
            let resolved = fallback.to_string_lossy().to_string();
            maybe_enforce_lru(&resolved);
            Ok(resolved)
        }
        Err(e) => Err(e).context(format!("Failed to create output directory: {}", output_dir)),
    }
}

fn maybe_enforce_lru(dir: &str) {
    use crate::types::constants::DEFAULT_OUTPUT_MAX_BYTES;
    static MAX_BYTES: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

    let threshold = *MAX_BYTES.get_or_init(|| {
        crate::infra::config::load_config()
            .ok()
            .and_then(|cfg| cfg.output_max_size_mb)
            .map(|mb| mb * 1_048_576)
            .unwrap_or(DEFAULT_OUTPUT_MAX_BYTES)
    });

    let _ = crate::cmd::output::enforce_output_lru(std::path::Path::new(dir), threshold);
}

pub(crate) struct ResolvedRgbImage {
    pub r: Array2<f32>,
    pub g: Array2<f32>,
    pub b: Array2<f32>,
    pub header: HduHeader,
    pub _tmp: Option<tempfile::TempDir>,
}

pub(crate) fn try_extract_rgb_resolved(path: &str) -> Result<Option<ResolvedRgbImage>> {
    let p = std::path::Path::new(path);
    if crate::infra::asdf::converter::is_asdf_file(p) {
        return Ok(None);
    }

    let (fits_path, tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)
        .with_context(|| format!("Failed to open {}", fits_path.display()))?;

    match crate::infra::fits::reader::try_extract_rgb_mmap(&file)? {
        Some(result) => Ok(Some(ResolvedRgbImage {
            r: result.r,
            g: result.g,
            b: result.b,
            header: result.header,
            _tmp: tmp,
        })),
        None => Ok(None),
    }
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
