use std::sync::Arc;

use anyhow::Context;
use ndarray::Array2;
use serde_json::json;

use crate::core::imaging::stf::{auto_stf, AutoStfConfig};
use crate::infra::cache::{ImageEntry, GLOBAL_IMAGE_CACHE};
use crate::types::compose::{AlignMethod, WhiteBalance};
use crate::types::constants::{
    DEFAULT_SCNR_AMOUNT, DEFAULT_WB_VALUE,
    KERNEL_GAUSSIAN, KERNEL_LANCZOS, KERNEL_LANCZOS3,
    SCNR_METHOD_MAXIMUM, WB_MODE_MANUAL, WB_MODE_NONE,
    COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B,
    COMPOSITE_ORIG_R, COMPOSITE_ORIG_G, COMPOSITE_ORIG_B,
    RES_MIN, RES_MAX, RES_MEAN, RES_SIGMA, RES_MEDIAN, RES_MAD,
    RES_SHADOW, RES_MIDTONE, RES_HIGHLIGHT,
};
use crate::types::image::{ImageStats, ScnrConfig, ScnrMethod, StfParams};
use crate::types::stacking::DrizzleKernel;

pub(crate) fn parse_scnr_config(
    enabled: Option<bool>,
    method: Option<&str>,
    amount: Option<f64>,
    preserve_luminance: Option<bool>,
) -> Option<ScnrConfig> {
    if !enabled.unwrap_or(false) {
        return None;
    }
    let m = match method {
        Some(SCNR_METHOD_MAXIMUM) => ScnrMethod::MaximumNeutral,
        _ => ScnrMethod::AverageNeutral,
    };
    Some(ScnrConfig {
        method: m,
        amount: amount.unwrap_or(DEFAULT_SCNR_AMOUNT as f64) as f32,
        preserve_luminance: preserve_luminance.unwrap_or(false),
    })
}

pub(crate) fn parse_wb(
    mode: Option<&str>,
    r: Option<f64>,
    g: Option<f64>,
    b: Option<f64>,
) -> WhiteBalance {
    match mode {
        Some(WB_MODE_MANUAL) => WhiteBalance::Manual(
            r.unwrap_or(DEFAULT_WB_VALUE),
            g.unwrap_or(DEFAULT_WB_VALUE),
            b.unwrap_or(DEFAULT_WB_VALUE),
        ),
        Some(WB_MODE_NONE) => WhiteBalance::None,
        _ => WhiteBalance::Auto,
    }
}

pub(crate) fn parse_align_method(method: Option<&str>) -> AlignMethod {
    match method {
        Some("affine") => AlignMethod::Affine,
        _ => AlignMethod::PhaseCorrelation,
    }
}

pub(crate) fn align_method_str(method: AlignMethod) -> &'static str {
    match method {
        AlignMethod::Affine => "affine",
        AlignMethod::PhaseCorrelation => "phase_correlation",
    }
}

pub(crate) fn parse_drizzle_kernel(kernel: Option<&str>) -> DrizzleKernel {
    match kernel {
        Some(KERNEL_GAUSSIAN) => DrizzleKernel::Gaussian,
        Some(KERNEL_LANCZOS3) | Some(KERNEL_LANCZOS) => DrizzleKernel::Lanczos3,
        _ => DrizzleKernel::Square,
    }
}

pub(crate) fn load_composite_channel(key: &str) -> anyhow::Result<ImageEntry> {
    GLOBAL_IMAGE_CACHE
        .get(key)
        .ok_or_else(|| anyhow::anyhow!("Composite channel '{}' not found in cache", key))
}

pub(crate) fn load_composite_rgb() -> anyhow::Result<(ImageEntry, ImageEntry, ImageEntry)> {
    let r = load_composite_channel(COMPOSITE_KEY_R)?;
    let g = load_composite_channel(COMPOSITE_KEY_G)?;
    let b = load_composite_channel(COMPOSITE_KEY_B)?;
    Ok((r, g, b))
}

pub(crate) fn load_composite_orig_rgb() -> anyhow::Result<(ImageEntry, ImageEntry, ImageEntry)> {
    let r = load_composite_channel(COMPOSITE_ORIG_R)?;
    let g = load_composite_channel(COMPOSITE_ORIG_G)?;
    let b = load_composite_channel(COMPOSITE_ORIG_B)?;
    Ok((r, g, b))
}

pub(crate) fn load_orig_or_composite() -> anyhow::Result<(ImageEntry, ImageEntry, ImageEntry)> {
    let r = GLOBAL_IMAGE_CACHE.get(COMPOSITE_ORIG_R)
        .or_else(|| GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_R))
        .ok_or_else(|| anyhow::anyhow!("Composite R not in cache"))?;
    let g = GLOBAL_IMAGE_CACHE.get(COMPOSITE_ORIG_G)
        .or_else(|| GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_G))
        .ok_or_else(|| anyhow::anyhow!("Composite G not in cache"))?;
    let b = GLOBAL_IMAGE_CACHE.get(COMPOSITE_ORIG_B)
        .or_else(|| GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_B))
        .ok_or_else(|| anyhow::anyhow!("Composite B not in cache"))?;
    Ok((r, g, b))
}

pub(crate) fn insert_composite_rgb(
    r: Array2<f32>,
    g: Array2<f32>,
    b: Array2<f32>,
    stats_r: ImageStats,
    stats_g: ImageStats,
    stats_b: ImageStats,
) {
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_R, Arc::new(r), stats_r);
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_G, Arc::new(g), stats_g);
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_B, Arc::new(b), stats_b);
}

pub(crate) fn insert_composite_and_orig(
    r: Array2<f32>,
    g: Array2<f32>,
    b: Array2<f32>,
    stats_r: ImageStats,
    stats_g: ImageStats,
    stats_b: ImageStats,
) {
    let arc_r = Arc::new(r);
    let arc_g = Arc::new(g);
    let arc_b = Arc::new(b);
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_ORIG_R, Arc::clone(&arc_r), stats_r.clone());
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_ORIG_G, Arc::clone(&arc_g), stats_g.clone());
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_ORIG_B, Arc::clone(&arc_b), stats_b.clone());
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_R, arc_r, stats_r);
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_G, arc_g, stats_g);
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_B, arc_b, stats_b);
}

pub(crate) fn stats_json(stats: &ImageStats) -> serde_json::Value {
    json!({
        RES_MIN: stats.min,
        RES_MAX: stats.max,
        RES_MEAN: stats.mean,
        RES_SIGMA: stats.sigma,
        RES_MEDIAN: stats.median,
    })
}

pub(crate) fn stats_json_full(stats: &ImageStats) -> serde_json::Value {
    json!({
        RES_MIN: stats.min,
        RES_MAX: stats.max,
        RES_MEAN: stats.mean,
        RES_SIGMA: stats.sigma,
        RES_MEDIAN: stats.median,
        RES_MAD: stats.mad,
    })
}

pub(crate) fn stf_json(stf: &StfParams) -> serde_json::Value {
    json!({
        RES_SHADOW: stf.shadow,
        RES_MIDTONE: stf.midtone,
        RES_HIGHLIGHT: stf.highlight,
    })
}

pub(crate) fn compute_linked_stf(
    stats_r: &ImageStats,
    stats_g: &ImageStats,
    stats_b: &ImageStats,
    config: &AutoStfConfig,
) -> StfParams {
    let combined = ImageStats {
        min: stats_r.min.min(stats_g.min).min(stats_b.min),
        max: stats_r.max.max(stats_g.max).max(stats_b.max),
        mean: (stats_r.mean + stats_g.mean + stats_b.mean) / 3.0,
        median: (stats_r.median + stats_g.median + stats_b.median) / 3.0,
        sigma: ((stats_r.sigma.powi(2) + stats_g.sigma.powi(2) + stats_b.sigma.powi(2)) / 3.0).sqrt(),
        mad: (stats_r.mad + stats_g.mad + stats_b.mad) / 3.0,
        valid_count: stats_r.valid_count,
    };
    auto_stf(&combined, config)
}

pub(crate) fn render_rgb_preview(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    path: &str,
    max_dim: usize,
) -> anyhow::Result<()> {
    use rayon::prelude::*;
    use crate::infra::render::rgb::render_rgb;

    let (rows, cols) = r.dim();

    if rows <= max_dim && cols <= max_dim {
        return render_rgb(r, g, b, path);
    }

    let r_slice = r.as_slice().context("R not contiguous")?;
    let g_slice = g.as_slice().context("G not contiguous")?;
    let b_slice = b.as_slice().context("B not contiguous")?;

    let scale = max_dim as f64 / (rows.max(cols) as f64);
    let pw = ((cols as f64) * scale).round().max(1.0) as usize;
    let ph = ((rows as f64) * scale).round().max(1.0) as usize;

    let y_ratio = rows as f64 / ph as f64;
    let x_ratio = cols as f64 / pw as f64;

    let mut preview = vec![0u8; pw * ph * 3];

    preview
        .par_chunks_mut(pw * 3)
        .enumerate()
        .for_each(|(dy, row_buf)| {
            let sy = ((dy as f64) * y_ratio).min((rows - 1) as f64) as usize;
            let src_base = sy * cols;
            for dx in 0..pw {
                let sx = ((dx as f64) * x_ratio).min((cols - 1) as f64) as usize;
                let si = src_base + sx;
                let o = dx * 3;
                row_buf[o] = (r_slice[si].clamp(0.0, 1.0) * 255.0) as u8;
                row_buf[o + 1] = (g_slice[si].clamp(0.0, 1.0) * 255.0) as u8;
                row_buf[o + 2] = (b_slice[si].clamp(0.0, 1.0) * 255.0) as u8;
            }
        });

    let file = std::fs::File::create(path).context("Failed to create output file")?;
    let buf_writer = std::io::BufWriter::with_capacity(2 * 1024 * 1024, file);
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        buf_writer,
        image::codecs::png::CompressionType::Default,
        image::codecs::png::FilterType::Sub,
    );
    use image::ImageEncoder;
    encoder
        .write_image(&preview, pw as u32, ph as u32, image::ColorType::Rgb8.into())
        .context("Failed to write RGB preview PNG")?;

    Ok(())
}

pub(crate) fn render_rgb_preview_with_stf(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    stf_r: impl Fn(f32) -> u8 + Send + Sync,
    stf_g: impl Fn(f32) -> u8 + Send + Sync,
    stf_b: impl Fn(f32) -> u8 + Send + Sync,
    path: &str,
    max_dim: usize,
) -> anyhow::Result<()> {
    use rayon::prelude::*;

    let (rows, cols) = r.dim();

    let r_slice = r.as_slice().context("R not contiguous")?;
    let g_slice = g.as_slice().context("G not contiguous")?;
    let b_slice = b.as_slice().context("B not contiguous")?;

    let (pw, ph, y_ratio, x_ratio) = if rows <= max_dim && cols <= max_dim {
        (cols, rows, 1.0, 1.0)
    } else {
        let scale = max_dim as f64 / (rows.max(cols) as f64);
        let pw = ((cols as f64) * scale).round().max(1.0) as usize;
        let ph = ((rows as f64) * scale).round().max(1.0) as usize;
        (pw, ph, rows as f64 / ph as f64, cols as f64 / pw as f64)
    };

    let mut preview = vec![0u8; pw * ph * 3];

    preview
        .par_chunks_mut(pw * 3)
        .enumerate()
        .for_each(|(dy, row_buf)| {
            let sy = ((dy as f64) * y_ratio).min((rows - 1) as f64) as usize;
            let src_base = sy * cols;
            for dx in 0..pw {
                let sx = ((dx as f64) * x_ratio).min((cols - 1) as f64) as usize;
                let si = src_base + sx;
                let o = dx * 3;
                row_buf[o] = stf_r(r_slice[si]);
                row_buf[o + 1] = stf_g(g_slice[si]);
                row_buf[o + 2] = stf_b(b_slice[si]);
            }
        });

    let file = std::fs::File::create(path).context("Failed to create output file")?;
    let buf_writer = std::io::BufWriter::with_capacity(2 * 1024 * 1024, file);
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        buf_writer,
        image::codecs::png::CompressionType::Default,
        image::codecs::png::FilterType::Sub,
    );
    use image::ImageEncoder;
    encoder
        .write_image(&preview, pw as u32, ph as u32, image::ColorType::Rgb8.into())
        .context("Failed to write RGB preview PNG")?;

    Ok(())
}
