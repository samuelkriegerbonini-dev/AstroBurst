use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

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


const COMPOSITE_GONE: &str = "The colour composite is no longer in memory; run Blend again.";

pub(crate) fn load_composite_channel(key: &str) -> anyhow::Result<ImageEntry> {
    GLOBAL_IMAGE_CACHE
        .get(key)
        .ok_or_else(|| anyhow::anyhow!(COMPOSITE_GONE))
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
        .ok_or_else(|| anyhow::anyhow!(COMPOSITE_GONE))?;
    let g = GLOBAL_IMAGE_CACHE.get(COMPOSITE_ORIG_G)
        .or_else(|| GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_G))
        .ok_or_else(|| anyhow::anyhow!(COMPOSITE_GONE))?;
    let b = GLOBAL_IMAGE_CACHE.get(COMPOSITE_ORIG_B)
        .or_else(|| GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_B))
        .ok_or_else(|| anyhow::anyhow!(COMPOSITE_GONE))?;
    Ok((r, g, b))
}

const COMPOSITE_KEYS: [&str; 3] = [COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B];
const COMPOSITE_ORIG_KEYS: [&str; 3] = [COMPOSITE_ORIG_R, COMPOSITE_ORIG_G, COMPOSITE_ORIG_B];
const COMPOSITE_STRETCHED_KEYS: [&str; 3] = [COMPOSITE_STRETCHED_R, COMPOSITE_STRETCHED_G, COMPOSITE_STRETCHED_B];
const COMPOSITE_TONED_KEYS: [&str; 3] = [COMPOSITE_TONED_R, COMPOSITE_TONED_G, COMPOSITE_TONED_B];
const NEUTRAL_WB: [f32; 3] = [1.0; 3];

pub(crate) const COMPOSITE_CHANGED: &str = "The composite changed while this step ran; run it again.";

static COMPOSITE_WB: Mutex<[f32; 3]> = Mutex::new(NEUTRAL_WB);
static COMPOSITE_GENERATION: AtomicU64 = AtomicU64::new(0);

fn lock_composite_wb() -> MutexGuard<'static, [f32; 3]> {
    COMPOSITE_WB.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn composite_generation() -> u64 {
    COMPOSITE_GENERATION.load(Ordering::SeqCst)
}

fn bump_composite_generation() -> u64 {
    COMPOSITE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

#[cfg(test)]
pub(crate) fn composite_wb_factors() -> [f32; 3] {
    *lock_composite_wb()
}

pub(crate) type CompositePlane = (Arc<Array2<f32>>, ImageStats);
pub(crate) type CompositeTriplet = [CompositePlane; 3];

#[derive(Clone)]
pub(crate) struct CompositeSnapshot {
    pub orig: CompositeTriplet,
    pub key: CompositeTriplet,
    pub wb: [f32; 3],
    pub stretched: Option<CompositeTriplet>,
    pub toned: Option<CompositeTriplet>,
}

fn scaled_stats(s: &ImageStats, k: f64) -> ImageStats {
    ImageStats {
        min: s.min * k,
        max: s.max * k,
        mean: s.mean * k,
        median: s.median * k,
        sigma: s.sigma * k,
        mad: s.mad * k,
        valid_count: s.valid_count,
    }
}

fn rescaled(arr: &Arc<Array2<f32>>, stats: &ImageStats, factor: f32) -> (Arc<Array2<f32>>, ImageStats) {
    if factor == 1.0 {
        return (Arc::clone(arr), stats.clone());
    }
    (Arc::new(arr.mapv(|v| v * factor)), scaled_stats(stats, factor as f64))
}

pub(crate) fn insert_composite_and_orig(
    r: Array2<f32>,
    g: Array2<f32>,
    b: Array2<f32>,
    stats_r: ImageStats,
    stats_g: ImageStats,
    stats_b: ImageStats,
) {
    let mut wb = lock_composite_wb();
    clear_derived_unlocked();
    let arc_r = Arc::new(r);
    let arc_g = Arc::new(g);
    let arc_b = Arc::new(b);
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_ORIG_R, Arc::clone(&arc_r), stats_r.clone());
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_ORIG_G, Arc::clone(&arc_g), stats_g.clone());
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_ORIG_B, Arc::clone(&arc_b), stats_b.clone());
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_R, arc_r, stats_r);
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_G, arc_g, stats_g);
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_B, arc_b, stats_b);
    *wb = NEUTRAL_WB;
    bump_composite_generation();
}

pub(crate) fn insert_composite_white_balanced(
    channels: [(Arc<Array2<f32>>, ImageStats); 3],
    factors: [f32; 3],
) {
    let mut wb = lock_composite_wb();
    clear_derived_unlocked();
    for (key, (arr, stats)) in COMPOSITE_KEYS.into_iter().zip(channels) {
        GLOBAL_IMAGE_CACHE.insert_synthetic(key, arr, stats);
    }
    *wb = factors;
    bump_composite_generation();
}

pub(crate) fn insert_composite_content(
    r: Array2<f32>,
    g: Array2<f32>,
    b: Array2<f32>,
    stats_r: ImageStats,
    stats_g: ImageStats,
    stats_b: ImageStats,
) {
    let wb = lock_composite_wb();
    clear_derived_unlocked();
    let channels = [(Arc::new(r), stats_r), (Arc::new(g), stats_g), (Arc::new(b), stats_b)];
    for (i, (arr, stats)) in channels.into_iter().enumerate() {
        let (orig, orig_stats) = rescaled(&arr, &stats, 1.0 / wb[i]);
        GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_ORIG_KEYS[i], orig, orig_stats);
        GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEYS[i], arr, stats);
    }
    bump_composite_generation();
}

pub(crate) fn clear_composite() {
    let mut wb = lock_composite_wb();
    for key in COMPOSITE_KEYS.into_iter().chain(COMPOSITE_ORIG_KEYS) {
        GLOBAL_IMAGE_CACHE.remove(key);
    }
    clear_derived_unlocked();
    *wb = NEUTRAL_WB;
    bump_composite_generation();
}

fn load_planes(keys: [&str; 3]) -> Option<CompositeTriplet> {
    let (r, g, b) = load_triplet(keys)?;
    Some([r, g, b].map(|e| (e.data_arc(), e.stats().clone())))
}

fn store_planes(keys: [&str; 3], planes: &CompositeTriplet) {
    for (key, (arr, stats)) in keys.into_iter().zip(planes) {
        GLOBAL_IMAGE_CACHE.insert_synthetic(key, Arc::clone(arr), stats.clone());
    }
}

fn store_optional_planes(keys: [&str; 3], planes: Option<&CompositeTriplet>) {
    match planes {
        Some(planes) => store_planes(keys, planes),
        None => remove_triplet(keys),
    }
}

pub(crate) fn unbalanced_planes(key: &CompositeTriplet, wb: [f32; 3]) -> CompositeTriplet {
    [0, 1, 2].map(|i| rescaled(&key[i].0, &key[i].1, 1.0 / wb[i]))
}

pub(crate) fn composite_state_from_key(key: CompositeTriplet, wb: [f32; 3]) -> CompositeSnapshot {
    let orig = unbalanced_planes(&key, wb);
    CompositeSnapshot { orig, key, wb, stretched: None, toned: None }
}

pub(crate) fn snapshot_composite() -> anyhow::Result<(CompositeSnapshot, u64)> {
    let wb = lock_composite_wb();
    let key = load_planes(COMPOSITE_KEYS).ok_or_else(|| anyhow::anyhow!(COMPOSITE_GONE))?;
    let orig = load_planes(COMPOSITE_ORIG_KEYS).unwrap_or_else(|| unbalanced_planes(&key, *wb));
    let snapshot = CompositeSnapshot {
        orig,
        key,
        wb: *wb,
        stretched: load_planes(COMPOSITE_STRETCHED_KEYS),
        toned: load_planes(COMPOSITE_TONED_KEYS),
    };
    Ok((snapshot, composite_generation()))
}

pub(crate) fn commit_composite(snapshot: &CompositeSnapshot, expected: Option<u64>) -> anyhow::Result<u64> {
    let mut wb = lock_composite_wb();
    if expected.is_some_and(|e| e != composite_generation()) {
        anyhow::bail!(COMPOSITE_CHANGED);
    }
    store_planes(COMPOSITE_ORIG_KEYS, &snapshot.orig);
    store_planes(COMPOSITE_KEYS, &snapshot.key);
    *wb = snapshot.wb;
    store_optional_planes(COMPOSITE_STRETCHED_KEYS, snapshot.stretched.as_ref());
    store_optional_planes(COMPOSITE_TONED_KEYS, snapshot.toned.as_ref());
    crate::cmd::processing::forget_composite_contrast();
    Ok(bump_composite_generation())
}

#[cfg(test)]
pub(crate) async fn composite_test_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
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

fn remove_triplet(keys: [&str; 3]) {
    for key in keys {
        GLOBAL_IMAGE_CACHE.remove(key);
    }
}

fn clear_stretched_unlocked() {
    remove_triplet(COMPOSITE_STRETCHED_KEYS);
}

fn clear_toned_unlocked() {
    remove_triplet(COMPOSITE_TONED_KEYS);
    crate::cmd::processing::forget_composite_contrast();
}

fn clear_derived_unlocked() {
    clear_stretched_unlocked();
    clear_toned_unlocked();
}

pub(crate) fn insert_composite_stretched(r: Array2<f32>, g: Array2<f32>, b: Array2<f32>) {
    let _wb = lock_composite_wb();
    clear_toned_unlocked();
    insert_triplet(COMPOSITE_STRETCHED_KEYS, r, g, b);
    bump_composite_generation();
}

pub(crate) fn load_composite_stretched() -> Option<(ImageEntry, ImageEntry, ImageEntry)> {
    load_triplet(COMPOSITE_STRETCHED_KEYS)
}

pub(crate) fn insert_composite_toned(r: Array2<f32>, g: Array2<f32>, b: Array2<f32>) {
    let _wb = lock_composite_wb();
    insert_triplet(COMPOSITE_TONED_KEYS, r, g, b);
    bump_composite_generation();
}

pub(crate) fn load_composite_toned() -> Option<(ImageEntry, ImageEntry, ImageEntry)> {
    load_triplet(COMPOSITE_TONED_KEYS)
}

#[cfg(test)]
pub(crate) fn clear_composite_derived() {
    let _wb = lock_composite_wb();
    clear_derived_unlocked();
    bump_composite_generation();
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

    if r.dim() != g.dim() || g.dim() != b.dim() {
        anyhow::bail!(
            "Composite channels differ in size (R {:?}, G {:?}, B {:?}); re-run Blend",
            r.dim(),
            g.dim(),
            b.dim()
        );
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

    #[test]
    fn stf_preview_refuses_channels_of_different_sizes() {
        let r = test_channel(8, 8, 7);
        let g = test_channel(4, 4, 8);
        let b = test_channel(8, 8, 9);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stf_mismatch.png");
        let identity = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;

        let err = render_rgb_preview_with_stf(&r, &g, &b, identity, identity, identity, path.to_str().unwrap(), 4096)
            .expect_err("a smaller G plane was indexed past its end");
        assert!(err.to_string().contains("re-run Blend"), "{err}");
        assert!(!path.exists());
    }

    #[test]
    fn content_written_under_a_white_balance_keeps_an_unbalanced_base() {
        let arr = Arc::new(Array2::from_elem((2, 2), 8.0f32));
        let stats = compute_image_stats(&arr);
        let (base, base_stats) = rescaled(&arr, &stats, 0.5);
        assert!(base.iter().all(|v| *v == 4.0));
        assert_eq!(base_stats.median, 4.0);
        assert_eq!(base_stats.valid_count, stats.valid_count);
        let (same, _) = rescaled(&arr, &stats, 1.0);
        assert!(Arc::ptr_eq(&same, &arr), "a neutral factor copied the channel");
        let (back, back_stats) = rescaled(&base, &base_stats, 2.0);
        assert!(back.iter().all(|v| *v == 8.0));
        assert_eq!(back_stats.max, stats.max);
    }

    fn contrast_plane(seed: usize) -> Array2<f32> {
        Array2::from_shape_fn((16, 16), |(y, x)| ((y * 16 + x + seed * 5) % 37) as f32 / 40.0 + 0.05)
    }

    #[tokio::test]
    async fn clearing_the_composite_releases_the_input_held_by_the_last_contrast_run() {
        let _guard = composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        insert_composite_stretched(contrast_plane(1), contrast_plane(2), contrast_plane(3));
        let input = load_composite_stretched().map(|(r, _, _)| Arc::downgrade(&r.data_arc()));
        let config = crate::core::imaging::local_contrast::LheConfig {
            kernel_radius: 3,
            ..crate::core::imaging::local_contrast::LheConfig::default()
        };
        let run = crate::cmd::processing::lhe_composite_cmd(dir.path().to_str().unwrap().to_string(), config, None, None).await;
        let held_after_run = input.as_ref().is_some_and(|w| w.upgrade().is_some());
        clear_composite();
        let held_after_clear = input.as_ref().is_some_and(|w| w.upgrade().is_some());
        run.unwrap();
        assert!(input.is_some(), "precondition: the stretched composite was stored");
        assert!(held_after_run, "precondition: the run keeps its input for a re-run");
        assert!(!held_after_clear, "the input of the last composite LHE stayed in memory after the composite was cleared");
    }

    #[tokio::test]
    async fn a_cleared_composite_says_to_run_blend_again() {
        let _guard = composite_test_lock().await;
        clear_composite();
        let expected = "The colour composite is no longer in memory; run Blend again.";
        let channel = load_composite_channel(COMPOSITE_KEY_R).err().expect("the R plane was cleared");
        assert_eq!(channel.to_string(), expected);
        let rgb = load_composite_rgb().err().expect("the planes were cleared");
        assert_eq!(rgb.to_string(), expected);
        let orig = load_orig_or_composite().err().expect("the original planes were cleared");
        assert_eq!(orig.to_string(), expected);
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
