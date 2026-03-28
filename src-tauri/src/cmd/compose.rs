use std::sync::Arc;
use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_cached, resolve_output_dir, extract_image_resolved, MAX_PREVIEW_DIM};
use crate::core::compose::rgb::{process_rgb, harmonize_dimensions, align_channels};
use crate::core::compose::lrgb::apply_lrgb;
use crate::core::imaging::resample::resample_image;
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{StfParams, apply_stf_f32};
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::infra::render::rgb::render_rgb;
use crate::types::compose::{AlignMethod, RgbComposeConfig, RgbComposeResult, WhiteBalance};
use crate::types::constants::{DEFAULT_DIMENSION_TOLERANCE, DEFAULT_RGB_COMPOSITE_FILENAME, DEFAULT_SCNR_AMOUNT, DEFAULT_WB_VALUE, SCNR_METHOD_MAXIMUM, WB_MODE_MANUAL, WB_MODE_NONE, RES_DIMENSIONS, RES_DIMENSION_CROP, RES_ELAPSED_MS, RES_MAX, RES_MEAN, RES_MEDIAN, RES_MIN, RES_OFFSET_B, RES_OFFSET_G, RES_PNG_PATH, RES_SCNR_APPLIED, RES_STATS_B, RES_STATS_G, RES_STATS_R, RES_SHADOW, RES_MIDTONE, RES_HIGHLIGHT, LRGB_APPLIED, RESAMPLED, STF_G, STF_R, STF_B, ALIGN_METHOD, DIMENSIONS, CHANNELS, RES_CHANNEL, RES_PATH, RES_FILE_SIZE_BYTES, RES_OFFSET, COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B};
use crate::types::image::{ScnrConfig, ScnrMethod};

use crate::infra::cache::ImageEntry;
use crate::infra::fits::writer::{write_fits_mono, filter_header};

fn load_entry(path: &Option<String>) -> anyhow::Result<Option<ImageEntry>> {
    match path {
        Some(p) => Ok(Some(load_cached(p)?)),
        None => Ok(None),
    }
}

fn needs_resample(entries: &[Option<&ImageEntry>]) -> bool {
    let dims: Vec<(usize, usize)> = entries
        .iter()
        .filter_map(|e| e.map(|entry| entry.arr().dim()))
        .collect();
    if dims.len() < 2 {
        return false;
    }
    let max_rows = dims.iter().map(|d| d.0).max().unwrap();
    let max_cols = dims.iter().map(|d| d.1).max().unwrap();
    let min_rows = dims.iter().map(|d| d.0).min().unwrap();
    let min_cols = dims.iter().map(|d| d.1).min().unwrap();
    let ratio_rows = max_rows as f64 / min_rows as f64;
    let ratio_cols = max_cols as f64 / min_cols as f64;
    ratio_rows >= 1.1 || ratio_cols >= 1.1
}

fn resample_to_largest(
    l: Option<Array2<f32>>,
    r: Option<Array2<f32>>,
    g: Option<Array2<f32>>,
    b: Option<Array2<f32>>,
) -> anyhow::Result<(Option<Array2<f32>>, Option<Array2<f32>>, Option<Array2<f32>>, Option<Array2<f32>>, bool)> {
    let dims: Vec<(usize, usize)> = [l.as_ref(), r.as_ref(), g.as_ref(), b.as_ref()]
        .iter()
        .filter_map(|ch| ch.map(|a| a.dim()))
        .collect();

    if dims.len() < 2 {
        return Ok((l, r, g, b, false));
    }

    let max_rows = dims.iter().map(|d| d.0).max().unwrap();
    let max_cols = dims.iter().map(|d| d.1).max().unwrap();
    let min_rows = dims.iter().map(|d| d.0).min().unwrap();
    let min_cols = dims.iter().map(|d| d.1).min().unwrap();

    let ratio_rows = max_rows as f64 / min_rows as f64;
    let ratio_cols = max_cols as f64 / min_cols as f64;

    if ratio_rows < 1.1 && ratio_cols < 1.1 {
        return Ok((l, r, g, b, false));
    }

    log::info!(
        "Auto-resample: harmonizing channels to {}x{} (ratio {:.1}x{:.1})",
        max_cols, max_rows, ratio_cols, ratio_rows
    );

    let resample_if_needed = |ch: Option<Array2<f32>>| -> anyhow::Result<Option<Array2<f32>>> {
        match ch {
            Some(arr) => {
                let (rows, cols) = arr.dim();
                if rows == max_rows && cols == max_cols {
                    Ok(Some(arr))
                } else {
                    Ok(Some(resample_image(&arr, max_rows, max_cols)?))
                }
            }
            None => Ok(None),
        }
    };

    Ok((
        resample_if_needed(l)?,
        resample_if_needed(r)?,
        resample_if_needed(g)?,
        resample_if_needed(b)?,
        true,
    ))
}

fn render_rgb_preview(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    path: &str,
    max_dim: usize,
) -> anyhow::Result<()> {
    use rayon::prelude::*;
    use anyhow::Context;

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

fn apply_scnr_to_channels(
    r: &Array2<f32>,
    g: &mut Array2<f32>,
    b: &Array2<f32>,
    scnr: &ScnrConfig,
) {
    use rayon::prelude::*;

    let r_slice = r.as_slice().unwrap();
    let b_slice = b.as_slice().unwrap();
    let g_slice = g.as_slice_mut().unwrap();
    let amount = scnr.amount;

    g_slice
        .par_iter_mut()
        .enumerate()
        .for_each(|(i, gv)| {
            let rv = r_slice[i];
            let bv = b_slice[i];
            let neutral = match scnr.method {
                ScnrMethod::AverageNeutral => (rv + bv) * 0.5,
                ScnrMethod::MaximumNeutral => rv.max(bv),
            };
            if *gv > neutral {
                *gv = *gv * (1.0 - amount) + neutral * amount;
            }
        });
}

#[tauri::command]
pub async fn compose_rgb_cmd(
    l_path: Option<String>,
    r_path: Option<String>,
    g_path: Option<String>,
    b_path: Option<String>,
    output_dir: String,
    auto_stretch: Option<bool>,
    linked_stf: Option<bool>,
    align: Option<bool>,
    align_method: Option<String>,
    wb_mode: Option<String>,
    wb_r: Option<f64>,
    wb_g: Option<f64>,
    wb_b: Option<f64>,
    scnr_enabled: Option<bool>,
    scnr_method: Option<String>,
    scnr_amount: Option<f64>,
    dimension_tolerance: Option<usize>,
    lrgb_lightness: Option<f64>,
    lrgb_chrominance: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let l_entry = load_entry(&l_path)?;
        let r_entry = load_entry(&r_path)?;
        let g_entry = load_entry(&g_path)?;
        let b_entry = load_entry(&b_path)?;

        let refs = [l_entry.as_ref(), r_entry.as_ref(), g_entry.as_ref(), b_entry.as_ref()];
        let do_resample = needs_resample(&refs);

        let (l_owned, r_owned, g_owned, b_owned, resampled);
        if do_resample {
            let to_owned = |e: &Option<ImageEntry>| -> Option<Array2<f32>> {
                e.as_ref().map(|entry| entry.arr().to_owned())
            };
            let (l_rs, r_rs, g_rs, b_rs, rs) =
                resample_to_largest(to_owned(&l_entry), to_owned(&r_entry), to_owned(&g_entry), to_owned(&b_entry))?;
            l_owned = l_rs;
            r_owned = r_rs;
            g_owned = g_rs;
            b_owned = b_rs;
            resampled = rs;
        } else {
            l_owned = None;
            r_owned = None;
            g_owned = None;
            b_owned = None;
            resampled = false;
        }

        let l_ref = l_owned.as_ref().or_else(|| l_entry.as_ref().map(|e| e.arr()));
        let r_ref = r_owned.as_ref().or_else(|| r_entry.as_ref().map(|e| e.arr()));
        let g_ref = g_owned.as_ref().or_else(|| g_entry.as_ref().map(|e| e.arr()));
        let b_ref = b_owned.as_ref().or_else(|| b_entry.as_ref().map(|e| e.arr()));

        let wb = match wb_mode.as_deref() {
            Some(WB_MODE_MANUAL) => WhiteBalance::Manual(
                wb_r.unwrap_or(DEFAULT_WB_VALUE),
                wb_g.unwrap_or(DEFAULT_WB_VALUE),
                wb_b.unwrap_or(DEFAULT_WB_VALUE),
            ),
            Some(WB_MODE_NONE) => WhiteBalance::None,
            _ => WhiteBalance::Auto,
        };

        let scnr_cfg = if scnr_enabled.unwrap_or(false) {
            let method = match scnr_method.as_deref() {
                Some(SCNR_METHOD_MAXIMUM) => ScnrMethod::MaximumNeutral,
                _ => ScnrMethod::AverageNeutral,
            };
            Some(ScnrConfig {
                method,
                amount: scnr_amount.unwrap_or(DEFAULT_SCNR_AMOUNT as f64) as f32,
                preserve_luminance: false,
            })
        } else {
            None
        };

        let align_m = match align_method.as_deref() {
            Some("affine") => AlignMethod::Affine,
            _ => AlignMethod::PhaseCorrelation,
        };

        let config = RgbComposeConfig {
            white_balance: wb,
            auto_stretch: auto_stretch.unwrap_or(true),
            linked_stf: linked_stf.unwrap_or(false),
            align: align.unwrap_or(true),
            align_method: align_m,
            scnr: scnr_cfg,
            dimension_tolerance: dimension_tolerance.unwrap_or(DEFAULT_DIMENSION_TOLERANCE),
            ..RgbComposeConfig::default()
        };

        let mut processed = process_rgb(
            r_ref,
            g_ref,
            b_ref,
            &config,
        )?;

        if let (Some(pre_r), Some(pre_g), Some(pre_b)) = (
            processed.pre_stretch_r.take(),
            processed.pre_stretch_g.take(),
            processed.pre_stretch_b.take(),
        ) {
            let stats_r = processed.stats_wb_r.clone().unwrap_or_else(|| compute_image_stats(&pre_r));
            let stats_g = processed.stats_wb_g.clone().unwrap_or_else(|| compute_image_stats(&pre_g));
            let stats_b = processed.stats_wb_b.clone().unwrap_or_else(|| compute_image_stats(&pre_b));

            GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_R, Arc::new(pre_r), stats_r);
            GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_G, Arc::new(pre_g), stats_g);
            GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_B, Arc::new(pre_b), stats_b);
        }

        let lrgb_applied = if let Some(l_data) = l_ref {
            let lightness = lrgb_lightness.unwrap_or(1.0) as f32;
            let chrominance = lrgb_chrominance.unwrap_or(1.0) as f32;

            let l_stretched = if config.auto_stretch {
                use crate::core::imaging::stf::{auto_stf, analyze};
                use crate::types::image::AutoStfConfig;
                let (stats, _) = analyze(l_data);
                let stf = auto_stf(&stats, &AutoStfConfig::default());
                apply_stf_f32(l_data, &stf, &stats)
            } else {
                l_data.clone()
            };

            apply_lrgb(
                &l_stretched,
                &mut processed.r,
                &mut processed.g,
                &mut processed.b,
                lightness,
                chrominance,
            )?;
            true
        } else {
            false
        };

        let png_path = format!("{}/{}", output_dir, DEFAULT_RGB_COMPOSITE_FILENAME);

        render_rgb_preview(
            &processed.r,
            &processed.g,
            &processed.b,
            &png_path,
            MAX_PREVIEW_DIM,
        )?;

        let result = RgbComposeResult {
            png_path: png_path.clone(),
            stf_r: processed.stf_r,
            stf_g: processed.stf_g,
            stf_b: processed.stf_b,
            stats_r: processed.stats_r.clone(),
            stats_g: processed.stats_g.clone(),
            stats_b: processed.stats_b.clone(),
            offset_g: processed.offset_g,
            offset_b: processed.offset_b,
            width: processed.cols,
            height: processed.rows,
            scnr_applied: processed.scnr_applied,
            dimension_crop: processed.dimension_crop,
        };

        let elapsed = t0.elapsed().as_millis() as u64;

        let stf_r_json = json!({RES_SHADOW: result.stf_r.shadow, RES_MIDTONE: result.stf_r.midtone, RES_HIGHLIGHT: result.stf_r.highlight});
        let stf_g_json = json!({RES_SHADOW: result.stf_g.shadow, RES_MIDTONE: result.stf_g.midtone, RES_HIGHLIGHT: result.stf_g.highlight});
        let stf_b_json = json!({RES_SHADOW: result.stf_b.shadow, RES_MIDTONE: result.stf_b.midtone, RES_HIGHLIGHT: result.stf_b.highlight});

        Ok(json!({
            RES_PNG_PATH: result.png_path,
            RES_DIMENSIONS: [result.width, result.height],
            RES_SCNR_APPLIED: result.scnr_applied,
            RES_OFFSET_G: [result.offset_g.0, result.offset_g.1],
            RES_OFFSET_B: [result.offset_b.0, result.offset_b.1],
            RES_DIMENSION_CROP: result.dimension_crop,
            RESAMPLED: resampled,
            LRGB_APPLIED: lrgb_applied,
            STF_R: stf_r_json,
            STF_G: stf_g_json,
            STF_B: stf_b_json,
            RES_STATS_R: { RES_MEDIAN: result.stats_r.median, RES_MEAN: result.stats_r.mean, RES_MIN: result.stats_r.min, RES_MAX: result.stats_r.max },
            RES_STATS_G: { RES_MEDIAN: result.stats_g.median, RES_MEAN: result.stats_g.mean, RES_MIN: result.stats_g.min, RES_MAX: result.stats_g.max },
            RES_STATS_B: { RES_MEDIAN: result.stats_b.median, RES_MEAN: result.stats_b.mean, RES_MIN: result.stats_b.min, RES_MAX: result.stats_b.max },
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

#[tauri::command]
pub async fn restretch_composite_cmd(
    output_dir: String,
    shadow_r: f64,
    midtone_r: f64,
    highlight_r: f64,
    shadow_g: f64,
    midtone_g: f64,
    highlight_g: f64,
    shadow_b: f64,
    midtone_b: f64,
    highlight_b: f64,
    scnr_enabled: Option<bool>,
    scnr_method: Option<String>,
    scnr_amount: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let entry_r = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_R)
            .ok_or_else(|| anyhow::anyhow!("Composite R channel not in cache. Please recompose first."))?;
        let entry_g = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_G)
            .ok_or_else(|| anyhow::anyhow!("Composite G channel not in cache. Please recompose first."))?;
        let entry_b = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_B)
            .ok_or_else(|| anyhow::anyhow!("Composite B channel not in cache. Please recompose first."))?;

        let stf_r = StfParams { shadow: shadow_r, midtone: midtone_r, highlight: highlight_r };
        let stf_g = StfParams { shadow: shadow_g, midtone: midtone_g, highlight: highlight_g };
        let stf_b = StfParams { shadow: shadow_b, midtone: midtone_b, highlight: highlight_b };

        let r_stretched = apply_stf_f32(entry_r.arr(), &stf_r, entry_r.stats());
        let mut g_stretched = apply_stf_f32(entry_g.arr(), &stf_g, entry_g.stats());
        let b_stretched = apply_stf_f32(entry_b.arr(), &stf_b, entry_b.stats());

        if scnr_enabled.unwrap_or(false) {
            let method = match scnr_method.as_deref() {
                Some(SCNR_METHOD_MAXIMUM) => ScnrMethod::MaximumNeutral,
                _ => ScnrMethod::AverageNeutral,
            };
            let cfg = ScnrConfig {
                method,
                amount: scnr_amount.unwrap_or(DEFAULT_SCNR_AMOUNT as f64) as f32,
                preserve_luminance: false,
            };
            apply_scnr_to_channels(&r_stretched, &mut g_stretched, &b_stretched, &cfg);
        }

        let png_path = format!("{}/{}", output_dir, DEFAULT_RGB_COMPOSITE_FILENAME);

        render_rgb_preview(
            &r_stretched,
            &g_stretched,
            &b_stretched,
            &png_path,
            MAX_PREVIEW_DIM,
        )?;

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

#[tauri::command]
pub async fn clear_composite_cache_cmd() -> Result<(), String> {
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_KEY_R);
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_KEY_G);
    GLOBAL_IMAGE_CACHE.remove(COMPOSITE_KEY_B);
    Ok(())
}

fn update_wcs_for_offset(header: &mut crate::types::header::HduHeader, dy: f64, dx: f64) {
    if dy.abs() < 1e-12 && dx.abs() < 1e-12 {
        return;
    }
    if let Some(crpix1) = header.get_f64("CRPIX1") {
        header.set_f64("CRPIX1", crpix1 - dx);
    }
    if let Some(crpix2) = header.get_f64("CRPIX2") {
        header.set_f64("CRPIX2", crpix2 - dy);
    }
}

fn build_channel_header(
    path: &Option<String>,
    offset: (f64, f64),
    copy_wcs: bool,
    copy_metadata: bool,
) -> Option<crate::types::header::HduHeader> {
    let p = path.as_ref()?;
    let resolved = extract_image_resolved(p).ok()?;
    let mut hdr = filter_header(&resolved.header, copy_wcs, copy_metadata)?;
    if copy_wcs {
        update_wcs_for_offset(&mut hdr, offset.0, offset.1);
    }
    Some(hdr)
}

#[tauri::command]
pub async fn export_aligned_channels_cmd(
    r_path: Option<String>,
    g_path: Option<String>,
    b_path: Option<String>,
    output_dir: String,
    align_method: Option<String>,
    copy_wcs: Option<bool>,
    copy_metadata: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let out_dir = resolve_output_dir(&output_dir)?;

        let r_entry = load_entry(&r_path)?;
        let g_entry = load_entry(&g_path)?;
        let b_entry = load_entry(&b_path)?;

        let r_ref = r_entry.as_ref().map(|e| e.arr());
        let g_ref = g_entry.as_ref().map(|e| e.arr());
        let b_ref = b_entry.as_ref().map(|e| e.arr());

        let (r_harm, g_harm, b_harm, rows, cols, _crop) =
            harmonize_dimensions(r_ref, g_ref, b_ref, DEFAULT_DIMENSION_TOLERANCE)?;

        let rh = r_harm.as_ref().or(r_ref);
        let gh = g_harm.as_ref().or(g_ref);
        let bh = b_harm.as_ref().or(b_ref);

        let method = match align_method.as_deref() {
            Some("affine") => AlignMethod::Affine,
            _ => AlignMethod::PhaseCorrelation,
        };

        let (r_aligned, g_aligned, b_aligned, off_g, off_b) =
            align_channels(rh, gh, bh, rows, cols, method)?;

        let do_wcs = copy_wcs.unwrap_or(true);
        let do_meta = copy_metadata.unwrap_or(true);

        let mut exported = Vec::new();

        let channels: Vec<(&str, &Array2<f32>, &Option<String>, (f64, f64))> = vec![
            ("R", &r_aligned, &r_path, (0.0, 0.0)),
            ("G", &g_aligned, &g_path, off_g),
            ("B", &b_aligned, &b_path, off_b),
        ];

        for (label, data, src_path, offset) in &channels {
            if src_path.is_none() {
                continue;
            }
            let stem = src_path.as_ref().unwrap()
                .split(&['/', '\\'][..])
                .last()
                .unwrap_or("channel")
                .replace(".fits", "")
                .replace(".fit", "")
                .replace(".fts", "");
            let out_path = format!("{}/{}_aligned.fits", out_dir, stem);

            let hdr = build_channel_header(src_path, *offset, do_wcs, do_meta);
            write_fits_mono(&out_path, data, hdr.as_ref())?;

            let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
            exported.push(json!({
                RES_CHANNEL: label,
                RES_PATH: out_path,
                RES_FILE_SIZE_BYTES: size,
                RES_OFFSET: [offset.0, offset.1],
            }));
        }

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            CHANNELS: exported,
            ALIGN_METHOD: match method {
                AlignMethod::Affine => "affine",
                AlignMethod::PhaseCorrelation => "phase_correlation",
            },
            DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: elapsed,
        }))
    })
}
