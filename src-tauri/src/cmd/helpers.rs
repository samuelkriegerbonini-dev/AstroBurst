use std::sync::Arc;

use anyhow::Context;
use ndarray::Array2;
use serde_json::json;

use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{auto_stf, AutoStfConfig};
use crate::infra::cache::{ImageEntry, GLOBAL_IMAGE_CACHE};
use crate::types::compose::{AlignMethod, WhiteBalance};
use crate::types::constants::{
    ALIGN_METHOD_AFFINE, ALIGN_METHOD_PHASE,
    DEFAULT_SCNR_AMOUNT, DEFAULT_WB_VALUE,
    KERNEL_GAUSSIAN, KERNEL_LANCZOS, KERNEL_LANCZOS3,
    SCNR_METHOD_MAXIMUM, WB_MODE_MANUAL, WB_MODE_NONE,
    COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B,
    COMPOSITE_ORIG_R, COMPOSITE_ORIG_G, COMPOSITE_ORIG_B,
    COMPOSITE_STRETCHED_R, COMPOSITE_STRETCHED_G, COMPOSITE_STRETCHED_B,
    COMPOSITE_TONED_R, COMPOSITE_TONED_G, COMPOSITE_TONED_B,
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
        Some(ALIGN_METHOD_AFFINE) => AlignMethod::Affine,
        _ => AlignMethod::PhaseCorrelation,
    }
}

pub(crate) fn align_method_str(method: AlignMethod) -> &'static str {
    match method {
        AlignMethod::Affine => ALIGN_METHOD_AFFINE,
        AlignMethod::PhaseCorrelation => ALIGN_METHOD_PHASE,
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
    clear_composite_derived();
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
    clear_composite_derived();
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

fn insert_triplet(keys: [&str; 3], r: Array2<f32>, g: Array2<f32>, b: Array2<f32>) {
    let (stats_r, (stats_g, stats_b)) = rayon::join(
        || compute_image_stats(&r),
        || rayon::join(
            || compute_image_stats(&g),
            || compute_image_stats(&b),
        ),
    );
    GLOBAL_IMAGE_CACHE.insert_synthetic(keys[0], Arc::new(r), stats_r);
    GLOBAL_IMAGE_CACHE.insert_synthetic(keys[1], Arc::new(g), stats_g);
    GLOBAL_IMAGE_CACHE.insert_synthetic(keys[2], Arc::new(b), stats_b);
}

fn load_triplet(keys: [&str; 3]) -> Option<(ImageEntry, ImageEntry, ImageEntry)> {
    let r = GLOBAL_IMAGE_CACHE.get(keys[0])?;
    let g = GLOBAL_IMAGE_CACHE.get(keys[1])?;
    let b = GLOBAL_IMAGE_CACHE.get(keys[2])?;
    Some((r, g, b))
}

pub(crate) fn insert_composite_stretched(r: Array2<f32>, g: Array2<f32>, b: Array2<f32>) {
    clear_composite_toned();
    insert_triplet([COMPOSITE_STRETCHED_R, COMPOSITE_STRETCHED_G, COMPOSITE_STRETCHED_B], r, g, b);
}

pub(crate) fn load_composite_stretched() -> Option<(ImageEntry, ImageEntry, ImageEntry)> {
    load_triplet([COMPOSITE_STRETCHED_R, COMPOSITE_STRETCHED_G, COMPOSITE_STRETCHED_B])
}

pub(crate) fn clear_composite_stretched() {
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_STRETCHED_R);
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_STRETCHED_G);
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_STRETCHED_B);
}

pub(crate) fn insert_composite_toned(r: Array2<f32>, g: Array2<f32>, b: Array2<f32>) {
    insert_triplet([COMPOSITE_TONED_R, COMPOSITE_TONED_G, COMPOSITE_TONED_B], r, g, b);
}

pub(crate) fn load_composite_toned() -> Option<(ImageEntry, ImageEntry, ImageEntry)> {
    load_triplet([COMPOSITE_TONED_R, COMPOSITE_TONED_G, COMPOSITE_TONED_B])
}

pub(crate) fn clear_composite_toned() {
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_TONED_R);
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_TONED_G);
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_TONED_B);
}

pub(crate) fn clear_composite_derived() {
    clear_composite_stretched();
    clear_composite_toned();
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

pub(crate) fn compute_linked_stf_with_stats(
    stats_r: &ImageStats,
    stats_g: &ImageStats,
    stats_b: &ImageStats,
    config: &AutoStfConfig,
) -> (StfParams, ImageStats) {
    let combined = crate::core::imaging::stats::combine_channel_stats(stats_r, stats_g, stats_b);
    let stf = auto_stf(&combined, config);
    (stf, combined)
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

    if r.dim() != g.dim() || g.dim() != b.dim() {
        return render_rgb(r, g, b, path);
    }

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
                row_buf[o] = (r_slice[si].clamp(0.0, 1.0) * 255.0).round() as u8;
                row_buf[o + 1] = (g_slice[si].clamp(0.0, 1.0) * 255.0).round() as u8;
                row_buf[o + 2] = (b_slice[si].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        });

    let file = std::fs::File::create(path).context("Failed to create output file")?;
    let buf_writer = std::io::BufWriter::with_capacity(2 * 1024 * 1024, file);
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        buf_writer,
        image::codecs::png::CompressionType::Fast,
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
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    );
    use image::ImageEncoder;
    encoder
        .write_image(&preview, pw as u32, ph as u32, image::ColorType::Rgb8.into())
        .context("Failed to write RGB preview PNG")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    fn test_channel(rows: usize, cols: usize, seed: u32) -> Array2<f32> {
        let mut state = seed;
        Array2::from_shape_fn((rows, cols), |_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 8) as f32 / (1u32 << 24) as f32
        })
    }

    fn encode_fast_reference(r: &Array2<f32>, g: &Array2<f32>, b: &Array2<f32>) -> Vec<u8> {
        let (rows, cols) = r.dim();
        let mut pixels = vec![0u8; rows * cols * 3];
        for y in 0..rows {
            for x in 0..cols {
                let o = (y * cols + x) * 3;
                pixels[o] = (r[[y, x]].clamp(0.0, 1.0) * 255.0).round() as u8;
                pixels[o + 1] = (g[[y, x]].clamp(0.0, 1.0) * 255.0).round() as u8;
                pixels[o + 2] = (b[[y, x]].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
        let mut out = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new_with_quality(
            std::io::Cursor::new(&mut out),
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Sub,
        );
        encoder
            .write_image(&pixels, cols as u32, rows as u32, image::ColorType::Rgb8.into())
            .unwrap();
        out
    }

    #[test]
    fn render_rgb_preview_small_image_uses_fast_encoder_without_upscale() {
        let r = test_channel(48, 64, 1);
        let g = test_channel(48, 64, 2);
        let b = test_channel(48, 64, 3);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.png");

        render_rgb_preview(&r, &g, &b, path.to_str().unwrap(), 4096).unwrap();

        let written = std::fs::read(&path).unwrap();
        let decoded = image::load_from_memory(&written).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (64, 48));
        assert_eq!(written, encode_fast_reference(&r, &g, &b));
    }

    #[test]
    fn render_rgb_preview_downscales_when_larger_than_max_dim() {
        let r = test_channel(20, 40, 4);
        let g = test_channel(20, 40, 5);
        let b = test_channel(20, 40, 6);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.png");

        render_rgb_preview(&r, &g, &b, path.to_str().unwrap(), 10).unwrap();

        let decoded = image::open(&path).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (10, 5));
    }

    #[test]
    fn render_rgb_preview_falls_back_on_channel_dim_mismatch() {
        let r = test_channel(8, 8, 7);
        let g = test_channel(4, 4, 8);
        let b = test_channel(8, 8, 9);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mismatch.png");

        render_rgb_preview(&r, &g, &b, path.to_str().unwrap(), 4096).unwrap();

        let decoded = image::open(&path).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (8, 8));
    }
}

pub(crate) fn parse_rejection_method(
    name: Option<&str>,
) -> anyhow::Result<crate::types::stacking::RejectionMethod> {
    match name {
        Some(n) => crate::types::stacking::RejectionMethod::from_name(n).map_err(anyhow::Error::msg),
        None => Ok(crate::types::stacking::RejectionMethod::default()),
    }
}

pub(crate) fn parse_combine_method(
    name: Option<&str>,
) -> anyhow::Result<crate::types::stacking::CombineMethod> {
    match name {
        Some(n) => crate::types::stacking::CombineMethod::from_name(n).map_err(anyhow::Error::msg),
        None => Ok(crate::types::stacking::CombineMethod::default()),
    }
}

pub(crate) fn parse_normalization_method(
    name: Option<&str>,
) -> anyhow::Result<crate::types::stacking::NormalizationMethod> {
    match name {
        Some(n) => crate::types::stacking::NormalizationMethod::from_name(n).map_err(anyhow::Error::msg),
        None => Ok(crate::types::stacking::NormalizationMethod::default()),
    }
}

pub(crate) fn parse_rejection_normalization(
    name: Option<&str>,
) -> anyhow::Result<crate::types::stacking::RejectionNormalization> {
    match name {
        Some(n) => crate::types::stacking::RejectionNormalization::from_name(n).map_err(anyhow::Error::msg),
        None => Ok(crate::types::stacking::RejectionNormalization::default()),
    }
}
