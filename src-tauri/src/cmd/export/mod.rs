use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, extract_image_resolved, load_cached, load_from_cache_or_disk, try_extract_rgb_resolved};
use crate::cmd::helpers;
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{apply_stf_f32, AutoStfConfig, StfParams};
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::infra::fits::writer::{
    filter_header, write_fits_mono_bitpix, write_fits_mono_rice, write_fits_rgb_bitpix,
    write_fits_rgb_rice,
};
use crate::infra::render::grayscale::{render_grayscale_hq, render_grayscale_16bit, render_stretched_8bit, render_stretched_16bit};
use crate::infra::render::rgb::{render_rgb, render_rgb_16bit};
use crate::types::constants::{COPY_WCS, COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B, RES_APPLY_STF, RES_BIT_DEPTH, RES_BITPIX, RES_COMPRESS, RES_COPY_METADATA, RES_DIMENSIONS, RES_ELAPSED_MS, RES_FILE_SIZE_BYTES, RES_OUTPUT_PATH, RES_QUANTIZE_LEVEL};

const DEFAULT_QUANTIZE_LEVEL: f64 = 16.0;

fn wants_rice_compression(compress: &Option<String>) -> bool {
    compress.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("rice"))
}

#[tauri::command]
pub async fn export_fits(
    path: String,
    output_path: String,
    apply_stf_stretch: Option<bool>,
    shadow: Option<f64>,
    midtone: Option<f64>,
    highlight: Option<f64>,
    copy_wcs: Option<bool>,
    copy_metadata: Option<bool>,
    bitpix: Option<i32>,
    compress: Option<String>,
    quantize_level: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let do_stf = apply_stf_stretch.unwrap_or(false);
        let do_wcs = copy_wcs.unwrap_or(true);
        let do_meta = copy_metadata.unwrap_or(true);
        let target_bitpix = bitpix.unwrap_or(-32);
        let use_rice = wants_rice_compression(&compress);
        let qlevel = quantize_level.unwrap_or(DEFAULT_QUANTIZE_LEVEL);

        let resolved = extract_image_resolved(&path)?;
        let filtered = filter_header(&resolved.header, do_wcs, do_meta);

        let cached = load_from_cache_or_disk(&path).ok();
        let source_ref = cached.as_ref().map(|e| e.arr()).unwrap_or(&resolved.arr);

        let stretched;
        let write_ref = if do_stf {
            let stf = StfParams {
                shadow: shadow.unwrap_or(0.0),
                midtone: midtone.unwrap_or(0.5),
                highlight: highlight.unwrap_or(1.0),
            };
            let stats = compute_image_stats(source_ref);
            stretched = apply_stf_f32(source_ref, &stf, &stats);
            &stretched
        } else {
            source_ref
        };

        crate::core::cube::cache::GLOBAL_CUBE_CACHE.invalidate(&output_path);
        if use_rice {
            write_fits_mono_rice(&output_path, write_ref, filtered.as_ref(), target_bitpix, qlevel)?;
        } else {
            write_fits_mono_bitpix(&output_path, write_ref, filtered.as_ref(), target_bitpix)?;
        }

        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            RES_BITPIX: target_bitpix,
            RES_APPLY_STF: do_stf,
            COPY_WCS: do_wcs,
            RES_COPY_METADATA: do_meta,
            RES_FILE_SIZE_BYTES: file_size,
            RES_COMPRESS: if use_rice { "rice" } else { "none" },
            RES_QUANTIZE_LEVEL: qlevel,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn export_fits_rgb(
    r_path: Option<String>,
    g_path: Option<String>,
    b_path: Option<String>,
    output_path: String,
    copy_wcs: Option<bool>,
    copy_metadata: Option<bool>,
    bitpix: Option<i32>,
    history: Option<Vec<String>>,
    compress: Option<String>,
    quantize_level: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let do_wcs = copy_wcs.unwrap_or(true);
        let do_meta = copy_metadata.unwrap_or(true);
        let target_bitpix = bitpix.unwrap_or(-32);
        let use_rice = wants_rice_compression(&compress);
        let qlevel = quantize_level.unwrap_or(DEFAULT_QUANTIZE_LEVEL);

        let cache_r = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_R);
        let cache_g = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_G);
        let cache_b = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_B);

        let (r_arr, g_arr, b_arr, header_source) = if let (Some(cr), Some(cg), Some(cb)) = (cache_r, cache_g, cache_b) {
            let r_hdr = r_path.as_deref()
                .filter(|p| !p.starts_with("__"))
                .and_then(|p| extract_image_resolved(p).ok())
                .map(|r| r.header)
                .or_else(|| cr.header().cloned());
            (cr.data_arc(), cg.data_arc(), cb.data_arc(), r_hdr)
        } else {
            let r_resolved = extract_image_resolved(
                r_path.as_deref().ok_or_else(|| anyhow::anyhow!("R channel path required"))?
            )?;
            let g_resolved = extract_image_resolved(
                g_path.as_deref().ok_or_else(|| anyhow::anyhow!("G channel path required"))?
            )?;
            let b_resolved = extract_image_resolved(
                b_path.as_deref().ok_or_else(|| anyhow::anyhow!("B channel path required"))?
            )?;
            let g_raw = g_resolved.arr;
            let b_raw = b_resolved.arr;

            let (r_rows, r_cols) = r_resolved.arr.dim();
            let g_ok = g_raw.dim() == (r_rows, r_cols);
            let b_ok = b_raw.dim() == (r_rows, r_cols);

            let (g_final, b_final) = if g_ok && b_ok {
                (g_raw, b_raw)
            } else {
                let max_rows = r_rows.max(g_raw.dim().0).max(b_raw.dim().0);
                let max_cols = r_cols.max(g_raw.dim().1).max(b_raw.dim().1);
                let resample_if = |arr: ndarray::Array2<f32>| -> anyhow::Result<ndarray::Array2<f32>> {
                    if arr.dim() == (max_rows, max_cols) { return Ok(arr); }
                    crate::core::imaging::resample::resample_image(&arr, max_rows, max_cols)
                };
                (resample_if(g_raw)?, resample_if(b_raw)?)
            };

            let r_final = if r_resolved.arr.dim() != g_final.dim() {
                let (tr, tc) = g_final.dim();
                crate::core::imaging::resample::resample_image(&r_resolved.arr, tr, tc)?
            } else {
                r_resolved.arr
            };

            (
                std::sync::Arc::new(r_final),
                std::sync::Arc::new(g_final),
                std::sync::Arc::new(b_final),
                Some(r_resolved.header),
            )
        };

        let mut filtered = header_source
            .as_ref()
            .and_then(|h| filter_header(h, do_wcs, do_meta));

        if let Some(steps) = &history {
            let h = filtered.get_or_insert_with(crate::types::header::HduHeader::empty);
            for step in steps {
                let line: String = step.chars().take(70).collect();
                h.cards.push(("HISTORY".to_string(), line));
            }
        }

        crate::core::cube::cache::GLOBAL_CUBE_CACHE.invalidate(&output_path);
        if use_rice {
            write_fits_rgb_rice(&output_path, &r_arr, &g_arr, &b_arr, filtered.as_ref(), target_bitpix, qlevel)?;
        } else {
            write_fits_rgb_bitpix(&output_path, &r_arr, &g_arr, &b_arr, filtered.as_ref(), target_bitpix)?;
        }

        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        let (rows, cols) = r_arr.dim();

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            RES_BITPIX: target_bitpix,
            COPY_WCS: do_wcs,
            RES_COPY_METADATA: do_meta,
            RES_FILE_SIZE_BYTES: file_size,
            RES_DIMENSIONS: [cols, rows],
            RES_COMPRESS: if use_rice { "rice" } else { "none" },
            RES_QUANTIZE_LEVEL: qlevel,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn export_png(
    path: String,
    output_path: String,
    bit_depth: Option<u8>,
    apply_stf_stretch: Option<bool>,
    shadow: Option<f64>,
    midtone: Option<f64>,
    highlight: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let depth = bit_depth.unwrap_or(16);
        let do_stf = apply_stf_stretch.unwrap_or(false);

        if let Some(rgb) = try_extract_rgb_resolved(&path)? {
            let sr = compute_image_stats(&rgb.r);
            let sg = compute_image_stats(&rgb.g);
            let sb = compute_image_stats(&rgb.b);

            let combined = crate::core::imaging::stats::combine_channel_stats(&sr, &sg, &sb);
            let (r_out, g_out, b_out) = if do_stf {
                let stf = StfParams {
                    shadow: shadow.unwrap_or(0.0),
                    midtone: midtone.unwrap_or(0.5),
                    highlight: highlight.unwrap_or(1.0),
                };
                (
                    apply_stf_f32(&rgb.r, &stf, &combined),
                    apply_stf_f32(&rgb.g, &stf, &combined),
                    apply_stf_f32(&rgb.b, &stf, &combined),
                )
            } else {
                let stf_config = AutoStfConfig::default();
                let (linked, _) = helpers::compute_linked_stf_with_stats(&sr, &sg, &sb, &stf_config);
                (
                    apply_stf_f32(&rgb.r, &linked, &combined),
                    apply_stf_f32(&rgb.g, &linked, &combined),
                    apply_stf_f32(&rgb.b, &linked, &combined),
                )
            };

            if depth == 16 {
                render_rgb_16bit(&r_out, &g_out, &b_out, &output_path)?;
            } else {
                render_rgb(&r_out, &g_out, &b_out, &output_path)?;
            }

            let file_size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
            let (rows, cols) = rgb.r.dim();

            return Ok(json!({
                RES_OUTPUT_PATH: output_path,
                RES_BIT_DEPTH: depth,
                RES_APPLY_STF: true,
                RES_FILE_SIZE_BYTES: file_size,
                RES_DIMENSIONS: [cols, rows],
                RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            }));
        }

        let resolved = extract_image_resolved(&path)?;

        if do_stf {
            let stf = StfParams {
                shadow: shadow.unwrap_or(0.0),
                midtone: midtone.unwrap_or(0.5),
                highlight: highlight.unwrap_or(1.0),
            };
            let stats = compute_image_stats(&resolved.arr);
            let stretched = apply_stf_f32(&resolved.arr, &stf, &stats);
            if depth == 16 {
                render_stretched_16bit(&stretched, &output_path)?;
            } else {
                render_stretched_8bit(&stretched, &output_path)?;
            }
        } else if depth == 16 {
            render_grayscale_16bit(&resolved.arr, &output_path)?;
        } else {
            render_grayscale_hq(&resolved.arr, &output_path)?;
        }

        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        let (rows, cols) = resolved.arr.dim();

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            RES_BIT_DEPTH: depth,
            RES_APPLY_STF: do_stf,
            RES_FILE_SIZE_BYTES: file_size,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

fn explicit_stf_requested(do_stf: bool, mr: Option<f64>, mg: Option<f64>, mb: Option<f64>) -> bool {
    do_stf
        && [mr, mg, mb]
            .iter()
            .any(|m| m.map_or(false, |v| (v - 0.5).abs() > 1e-4))
}

#[tauri::command]
pub async fn export_rgb_png(
    r_path: Option<String>,
    g_path: Option<String>,
    b_path: Option<String>,
    output_path: String,
    bit_depth: Option<u8>,
    apply_stf_stretch: Option<bool>,
    shadow_r: Option<f64>,
    midtone_r: Option<f64>,
    highlight_r: Option<f64>,
    shadow_g: Option<f64>,
    midtone_g: Option<f64>,
    highlight_g: Option<f64>,
    shadow_b: Option<f64>,
    midtone_b: Option<f64>,
    highlight_b: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let depth = bit_depth.unwrap_or(16);
        let do_stf = apply_stf_stretch.unwrap_or(false);

        let real_path_count = [r_path.as_deref(), g_path.as_deref(), b_path.as_deref()]
            .iter()
            .flatten()
            .filter(|p| !p.starts_with("__"))
            .count();
        let single_channel_export = real_path_count == 1;

        if !single_channel_export {
                if let Some((tr, tg, tb)) = helpers::load_composite_toned().or_else(helpers::load_composite_stretched) {
                if depth == 16 {
                    render_rgb_16bit(tr.arr(), tg.arr(), tb.arr(), &output_path)?;
                } else {
                    render_rgb(tr.arr(), tg.arr(), tb.arr(), &output_path)?;
                }
                let (rows, cols) = tr.arr().dim();
                let file_size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
                return Ok(json!({
                    RES_OUTPUT_PATH: output_path,
                    RES_BIT_DEPTH: depth,
                    RES_APPLY_STF: false,
                    RES_FILE_SIZE_BYTES: file_size,
                    RES_DIMENSIONS: [cols, rows],
                    RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
                }));
            }

            let cache_r = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_R);
            let cache_g = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_G);
            let cache_b = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_B);

            if let (Some(cr), Some(cg), Some(cb)) = (&cache_r, &cache_g, &cache_b) {
                let has_explicit_stf = explicit_stf_requested(do_stf, midtone_r, midtone_g, midtone_b);

                let (stf_r, stf_g, stf_b) = if has_explicit_stf {
                    (
                        StfParams {
                            shadow: shadow_r.unwrap_or(0.0),
                            midtone: midtone_r.unwrap_or(0.5),
                            highlight: highlight_r.unwrap_or(1.0),
                        },
                        StfParams {
                            shadow: shadow_g.unwrap_or(0.0),
                            midtone: midtone_g.unwrap_or(0.5),
                            highlight: highlight_g.unwrap_or(1.0),
                        },
                        StfParams {
                            shadow: shadow_b.unwrap_or(0.0),
                            midtone: midtone_b.unwrap_or(0.5),
                            highlight: highlight_b.unwrap_or(1.0),
                        },
                    )
                } else {
                    let stf_config = AutoStfConfig::default();
                    let (linked, _) = helpers::compute_linked_stf_with_stats(cr.stats(), cg.stats(), cb.stats(), &stf_config);
                    (linked, linked, linked)
                };

                let identical_params = stf_r.shadow == stf_g.shadow
                    && stf_g.shadow == stf_b.shadow
                    && stf_r.midtone == stf_g.midtone
                    && stf_g.midtone == stf_b.midtone
                    && stf_r.highlight == stf_g.highlight
                    && stf_g.highlight == stf_b.highlight;

                let linked_stats = if identical_params {
                    Some(crate::core::imaging::stats::combine_channel_stats(
                        cr.stats(),
                        cg.stats(),
                        cb.stats(),
                    ))
                } else {
                    None
                };

                let (r_out, g_out, b_out) = match &linked_stats {
                    Some(combined) => (
                        apply_stf_f32(cr.arr(), &stf_r, combined),
                        apply_stf_f32(cg.arr(), &stf_g, combined),
                        apply_stf_f32(cb.arr(), &stf_b, combined),
                    ),
                    None => (
                        apply_stf_f32(cr.arr(), &stf_r, cr.stats()),
                        apply_stf_f32(cg.arr(), &stf_g, cg.stats()),
                        apply_stf_f32(cb.arr(), &stf_b, cb.stats()),
                    ),
                };
                if depth == 16 {
                    render_rgb_16bit(&r_out, &g_out, &b_out, &output_path)?;
                } else {
                    render_rgb(&r_out, &g_out, &b_out, &output_path)?;
                }
                let (rows, cols) = cr.arr().dim();
                let file_size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
                return Ok(json!({
                    RES_OUTPUT_PATH: output_path,
                    RES_BIT_DEPTH: depth,
                    RES_APPLY_STF: do_stf,
                    RES_FILE_SIZE_BYTES: file_size,
                    RES_DIMENSIONS: [cols, rows],
                    RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
                }));
            }
        }

        let r_entry = r_path.as_deref().map(load_cached).transpose()?;
        let g_entry = g_path.as_deref().map(load_cached).transpose()?;
        let b_entry = b_path.as_deref().map(load_cached).transpose()?;

        let any_entry = r_entry.as_ref().or(g_entry.as_ref()).or(b_entry.as_ref())
            .ok_or_else(|| anyhow::anyhow!("At least one channel path required"))?;
        let zeros = ndarray::Array2::<f32>::zeros(any_entry.arr().dim());

        let ra = r_entry.as_ref().map(|e| e.arr()).unwrap_or(&zeros);
        let ga = g_entry.as_ref().map(|e| e.arr()).unwrap_or(&zeros);
        let ba = b_entry.as_ref().map(|e| e.arr()).unwrap_or(&zeros);

        let has_explicit_stf = shadow_r.is_some() || shadow_g.is_some() || shadow_b.is_some();

        let stretch_and_render = |r: &ndarray::Array2<f32>, g: &ndarray::Array2<f32>, b: &ndarray::Array2<f32>| -> anyhow::Result<serde_json::Value> {
            let sr = compute_image_stats(r);
            let sg = compute_image_stats(g);
            let sb = compute_image_stats(b);

            let (r_out, g_out, b_out) = if do_stf && has_explicit_stf {
                let stf_r = StfParams { shadow: shadow_r.unwrap_or(0.0), midtone: midtone_r.unwrap_or(0.5), highlight: highlight_r.unwrap_or(1.0) };
                let stf_g = StfParams { shadow: shadow_g.unwrap_or(0.0), midtone: midtone_g.unwrap_or(0.5), highlight: highlight_g.unwrap_or(1.0) };
                let stf_b = StfParams { shadow: shadow_b.unwrap_or(0.0), midtone: midtone_b.unwrap_or(0.5), highlight: highlight_b.unwrap_or(1.0) };

                let identical_params = stf_r.shadow == stf_g.shadow
                    && stf_g.shadow == stf_b.shadow
                    && stf_r.midtone == stf_g.midtone
                    && stf_g.midtone == stf_b.midtone
                    && stf_r.highlight == stf_g.highlight
                    && stf_g.highlight == stf_b.highlight;

                if identical_params {
                    let combined = crate::core::imaging::stats::combine_channel_stats(&sr, &sg, &sb);
                    (
                        apply_stf_f32(r, &stf_r, &combined),
                        apply_stf_f32(g, &stf_g, &combined),
                        apply_stf_f32(b, &stf_b, &combined),
                    )
                } else {
                    (
                        apply_stf_f32(r, &stf_r, &sr),
                        apply_stf_f32(g, &stf_g, &sg),
                        apply_stf_f32(b, &stf_b, &sb),
                    )
                }
            } else {
                let stf_config = AutoStfConfig::default();
                let (linked, combined) = helpers::compute_linked_stf_with_stats(&sr, &sg, &sb, &stf_config);
                (
                    apply_stf_f32(r, &linked, &combined),
                    apply_stf_f32(g, &linked, &combined),
                    apply_stf_f32(b, &linked, &combined),
                )
            };

            if depth == 16 { render_rgb_16bit(&r_out, &g_out, &b_out, &output_path)?; }
            else { render_rgb(&r_out, &g_out, &b_out, &output_path)?; }

            let (rows, cols) = r.dim();
            let file_size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
            Ok(json!({
                RES_OUTPUT_PATH: output_path,
                RES_BIT_DEPTH: depth,
                RES_APPLY_STF: true,
                RES_FILE_SIZE_BYTES: file_size,
                RES_DIMENSIONS: [cols, rows],
                RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            }))
        };

        if ra.dim() != ga.dim() || ra.dim() != ba.dim() {
            let max_rows = ra.dim().0.max(ga.dim().0).max(ba.dim().0);
            let max_cols = ra.dim().1.max(ga.dim().1).max(ba.dim().1);
            let resample_if = |arr: &ndarray::Array2<f32>| -> anyhow::Result<ndarray::Array2<f32>> {
                if arr.dim() == (max_rows, max_cols) { return Ok(arr.to_owned()); }
                crate::core::imaging::resample::resample_image(arr, max_rows, max_cols)
            };
            let ro = resample_if(ra)?;
            let go = resample_if(ga)?;
            let bo = resample_if(ba)?;
            return stretch_and_render(&ro, &go, &bo);
        }

        stretch_and_render(ra, ga, ba)
    })
}

#[cfg(test)]
mod tests {
    use super::explicit_stf_requested;

    #[test]
    fn explicit_stf_honored_when_any_channel_deviates() {
        assert!(explicit_stf_requested(true, Some(0.5), Some(0.3), Some(0.5)));
        assert!(explicit_stf_requested(true, Some(0.2), Some(0.5), Some(0.5)));
        assert!(explicit_stf_requested(true, Some(0.5), Some(0.5), Some(0.7)));
        assert!(!explicit_stf_requested(true, Some(0.5), Some(0.5), Some(0.5)));
        assert!(!explicit_stf_requested(false, Some(0.2), Some(0.3), Some(0.7)));
        assert!(!explicit_stf_requested(true, None, None, None));
    }
}
