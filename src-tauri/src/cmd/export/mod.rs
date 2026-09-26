use std::sync::Arc;
use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, cached_header, extract_image_resolved, image_ref, invalidate_written, try_extract_rgb_resolved};
use crate::cmd::compose::rescale_header_to_grid;
use crate::cmd::cutout::refuse_source_as_target;
use crate::cmd::helpers;
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::imaging::stats::{combine_channel_stats, compute_image_stats};
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, AutoStfConfig, ImageStats, StfParams};
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::infra::fits::writer::{
    filter_header, write_fits_mono_bitpix, write_fits_mono_rice, write_fits_rgb_bitpix,
    write_fits_rgb_rice,
};
use crate::infra::render::grayscale::{render_grayscale_hq, render_grayscale_16bit, render_stretched_8bit, render_stretched_16bit};
use crate::infra::render::rgb::{render_rgb, render_rgb_16bit};
use crate::infra::fits::mef_writer::{write_compressed_mef, CompressMode, CompressOptions};
use crate::types::constants::{COPY_WCS, COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B, RES_APPLY_STF, RES_BIT_DEPTH, RES_BITPIX, RES_COMPRESS, RES_COPY_METADATA, RES_DIMENSIONS, RES_DROPPED, RES_ELAPSED_MS, RES_FILE_SIZE_BYTES, RES_KEPT_RAW, RES_OUTPUT_PATH, RES_OUTPUT_SIZE_BYTES, RES_QUANTIZE_LEVEL, RES_SOURCE_SIZE_BYTES, RES_UNCOMPRESSED, RES_WCS_WRITTEN};
use crate::types::header::HduHeader;

const DEFAULT_QUANTIZE_LEVEL: f64 = 16.0;

struct ExportChannel {
    arr: Arc<Array2<f32>>,
    header: Option<HduHeader>,
}

struct RgbExport {
    r: Arc<Array2<f32>>,
    g: Arc<Array2<f32>>,
    b: Arc<Array2<f32>>,
    header: Option<HduHeader>,
}

fn load_export_channel(path: &str) -> anyhow::Result<ExportChannel> {
    if image_ref(path).is_synthetic() {
        let entry = GLOBAL_IMAGE_CACHE.get(path).ok_or_else(|| {
            anyhow::anyhow!("'{}' is no longer in memory: re-run the step that produced it", path)
        })?;
        return Ok(ExportChannel { arr: entry.data_arc(), header: entry.header().cloned() });
    }
    let resolved = extract_image_resolved(path)?;
    Ok(ExportChannel { arr: Arc::new(resolved.arr), header: Some(resolved.header) })
}

fn same_file_triplet(paths: [Option<&str>; 3]) -> Option<&str> {
    match paths {
        [Some(r), Some(g), Some(b)] if r == g && g == b && !image_ref(r).is_synthetic() => Some(r),
        _ => None,
    }
}

fn rgb_file_export(paths: [Option<&str>; 3]) -> anyhow::Result<Option<RgbExport>> {
    let Some(path) = same_file_triplet(paths) else {
        return Ok(None);
    };
    Ok(try_extract_rgb_resolved(path)?.map(|rgb| RgbExport {
        r: Arc::new(rgb.r),
        g: Arc::new(rgb.g),
        b: Arc::new(rgb.b),
        header: Some(rgb.header),
    }))
}

fn explicit_header(header_path: Option<&str>) -> anyhow::Result<Option<HduHeader>> {
    header_path
        .map(|p| {
            cached_header(p).map_err(|e| e.context(format!("Failed to read the header source {}", p)))
        })
        .transpose()
}

fn resample_to(arr: Arc<Array2<f32>>, dims: (usize, usize)) -> anyhow::Result<Arc<Array2<f32>>> {
    if arr.dim() == dims {
        return Ok(arr);
    }
    Ok(Arc::new(crate::core::imaging::resample::resample_image(&arr, dims.0, dims.1)?))
}

fn channels_export(paths: [Option<&str>; 3]) -> anyhow::Result<RgbExport> {
    let [r_path, g_path, b_path] = paths;
    let r = load_export_channel(r_path.ok_or_else(|| anyhow::anyhow!("R channel path required"))?)?;
    let g = load_export_channel(g_path.ok_or_else(|| anyhow::anyhow!("G channel path required"))?)?;
    let b = load_export_channel(b_path.ok_or_else(|| anyhow::anyhow!("B channel path required"))?)?;
    let rows = r.arr.dim().0.max(g.arr.dim().0).max(b.arr.dim().0);
    let cols = r.arr.dim().1.max(g.arr.dim().1).max(b.arr.dim().1);
    let header = [&r, &g, &b].into_iter().find_map(|channel| {
        channel.header.clone().map(|mut header| {
            rescale_header_to_grid(&mut header, channel.arr.dim(), (rows, cols));
            header
        })
    });
    Ok(RgbExport {
        r: resample_to(r.arr, (rows, cols))?,
        g: resample_to(g.arr, (rows, cols))?,
        b: resample_to(b.arr, (rows, cols))?,
        header,
    })
}

fn header_dims(header: &HduHeader) -> Option<(usize, usize)> {
    let rows = usize::try_from(header.get_i64("NAXIS2")?).ok()?;
    let cols = usize::try_from(header.get_i64("NAXIS1")?).ok()?;
    Some((rows, cols))
}

fn channel_header_for_grid(path: &str, grid: (usize, usize)) -> Option<HduHeader> {
    let (mut header, dims) = if image_ref(path).is_synthetic() {
        let entry = GLOBAL_IMAGE_CACHE.get(path)?;
        (entry.header().cloned()?, Some(entry.arr().dim()))
    } else {
        let header = cached_header(path).ok()?;
        let dims = header_dims(&header);
        (header, dims)
    };
    if let Some(dims) = dims {
        rescale_header_to_grid(&mut header, dims, grid);
    }
    Some(header)
}

fn composite_export(r_path: Option<&str>) -> Option<RgbExport> {
    let (cr, cg, cb) = match (
        GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_R),
        GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_G),
        GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_B),
    ) {
        (Some(cr), Some(cg), Some(cb)) => (cr, cg, cb),
        _ => return None,
    };
    let header = r_path
        .and_then(|p| channel_header_for_grid(p, cr.arr().dim()))
        .or_else(|| cr.header().cloned());
    Some(RgbExport { r: cr.data_arc(), g: cg.data_arc(), b: cb.data_arc(), header })
}

fn writes_celestial_wcs(header: Option<&HduHeader>, dims: (usize, usize)) -> bool {
    header.is_some_and(|written| {
        let mut probe = written.clone();
        probe.set("NAXIS1", dims.1.to_string());
        probe.set("NAXIS2", dims.0.to_string());
        WcsTransform::from_header(&probe).is_ok()
    })
}

fn validated_stf_value(name: &str, value: Option<f64>) -> anyhow::Result<Option<f64>> {
    match value {
        Some(v) if !(0.0..=1.0).contains(&v) => {
            anyhow::bail!("{} must be a number between 0 and 1, got {}", name, v)
        }
        other => Ok(other),
    }
}

#[derive(Debug, Clone, Copy)]
struct PngStf {
    apply: bool,
    channels: [[Option<f64>; 3]; 3],
    linked: Option<bool>,
}

impl PngStf {
    fn explicit(&self) -> Option<[StfParams; 3]> {
        if !self.apply || self.channels.iter().flatten().all(Option::is_none) {
            return None;
        }
        Some(self.channels.map(|[shadow, midtone, highlight]| StfParams {
            shadow: shadow.unwrap_or(0.0),
            midtone: midtone.unwrap_or(0.5),
            highlight: highlight.unwrap_or(1.0),
        }))
    }
}

fn identical_stf(p: &[StfParams; 3]) -> bool {
    p.iter().all(|s| s.shadow == p[0].shadow && s.midtone == p[0].midtone && s.highlight == p[0].highlight)
}

fn png_rgb_planes(
    planes: [&Array2<f32>; 3],
    stats: [&ImageStats; 3],
    stf: &PngStf,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    let explicit = stf.explicit();
    let linked = stf.linked.unwrap_or_else(|| explicit.as_ref().map_or(true, identical_stf));
    let combined = combine_channel_stats(stats[0], stats[1], stats[2]);
    let config = AutoStfConfig::default();
    let params = match explicit {
        Some(p) => p,
        None if linked => [auto_stf(&combined, &config); 3],
        None => stats.map(|s| auto_stf(s, &config)),
    };
    let norm = if linked { [&combined; 3] } else { stats };
    (
        apply_stf_f32(planes[0], &params[0], norm[0]),
        apply_stf_f32(planes[1], &params[1], norm[1]),
        apply_stf_f32(planes[2], &params[2], norm[2]),
    )
}

fn render_rgb_png(
    planes: (&Array2<f32>, &Array2<f32>, &Array2<f32>),
    depth: u8,
    output_path: &str,
) -> anyhow::Result<()> {
    if depth == 16 {
        render_rgb_16bit(planes.0, planes.1, planes.2, output_path)
    } else {
        render_rgb(planes.0, planes.1, planes.2, output_path)
    }
}

fn wants_rice_compression(compress: &Option<String>) -> bool {
    compress.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("rice"))
}

fn count_real_paths(paths: [Option<&str>; 3]) -> usize {
    paths
        .iter()
        .flatten()
        .filter(|p| !p.starts_with("__") || p.starts_with(crate::types::constants::WIZARD_CACHE_PREFIX))
        .count()
}

fn fits_rgb_uses_composite(paths: [Option<&str>; 3]) -> bool {
    count_real_paths(paths) <= 1
}

fn png_rgb_uses_composite(paths: [Option<&str>; 3]) -> bool {
    count_real_paths(paths) == 0
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
        refuse_source_as_target(&output_path, &path)?;

        let resolved = extract_image_resolved(&path)?;
        let filtered = filter_header(&resolved.header, do_wcs, do_meta);
        let source_ref = &resolved.arr;

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
        invalidate_written(&output_path);

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
    header_path: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let do_wcs = copy_wcs.unwrap_or(true);
        let do_meta = copy_metadata.unwrap_or(true);
        let target_bitpix = bitpix.unwrap_or(-32);
        let use_rice = wants_rice_compression(&compress);
        let qlevel = quantize_level.unwrap_or(DEFAULT_QUANTIZE_LEVEL);
        let paths = [r_path.as_deref(), g_path.as_deref(), b_path.as_deref()];
        let sources = paths.into_iter().chain([header_path.as_deref()]).flatten();
        for source in sources.filter(|p| !image_ref(p).is_synthetic()) {
            refuse_source_as_target(&output_path, source)?;
        }
        let requested_header = explicit_header(header_path.as_deref())?;

        let source = match rgb_file_export(paths)? {
            Some(file) => file,
            None => {
                let composite = if fits_rgb_uses_composite(paths) { composite_export(r_path.as_deref()) } else { None };
                match composite {
                    Some(composite) => composite,
                    None => channels_export(paths)?,
                }
            }
        };
        let (r_arr, g_arr, b_arr) = (source.r, source.g, source.b);
        let header_source = requested_header.or(source.header);

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
        invalidate_written(&output_path);

        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        let (rows, cols) = r_arr.dim();
        let wcs_written = writes_celestial_wcs(filtered.as_ref(), (rows, cols));

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            RES_BITPIX: target_bitpix,
            COPY_WCS: do_wcs,
            RES_COPY_METADATA: do_meta,
            RES_WCS_WRITTEN: wcs_written,
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

#[tauri::command]
pub async fn compress_mef_cmd(
    source_path: String,
    output_path: String,
    lossless: bool,
    quantize_level: Option<f64>,
    drop_extnames: Vec<String>,
    raw_extnames: Vec<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();

        let mode = if lossless {
            CompressMode::Lossless
        } else {
            CompressMode::Lossy { quantize_level: quantize_level.unwrap_or(DEFAULT_QUANTIZE_LEVEL) }
        };

        let opts = CompressOptions { mode, drop_extnames, raw_extnames };

        crate::core::cube::cache::GLOBAL_CUBE_CACHE.invalidate(&output_path);
        let report = write_compressed_mef(&source_path, &output_path, &opts)?;
        invalidate_written(&output_path);

        let source_size = std::fs::metadata(&source_path).map(|m| m.len()).unwrap_or(0);
        let output_size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            RES_DROPPED: report.dropped,
            RES_KEPT_RAW: report.kept_raw,
            RES_UNCOMPRESSED: report.uncompressed,
            RES_SOURCE_SIZE_BYTES: source_size,
            RES_OUTPUT_SIZE_BYTES: output_size,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

fn rgb_png_response(
    output_path: &str,
    depth: u8,
    applied_stf: bool,
    dims: (usize, usize),
    t0: Instant,
) -> serde_json::Value {
    let file_size = std::fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
    json!({
        RES_OUTPUT_PATH: output_path,
        RES_BIT_DEPTH: depth,
        RES_APPLY_STF: applied_stf,
        RES_FILE_SIZE_BYTES: file_size,
        RES_DIMENSIONS: [dims.1, dims.0],
        RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
    })
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
    linked: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let depth = bit_depth.unwrap_or(16);
        let do_stf = apply_stf_stretch.unwrap_or(false);
        let stf = PngStf {
            apply: do_stf,
            channels: [
                [
                    validated_stf_value("shadow_r", shadow_r)?,
                    validated_stf_value("midtone_r", midtone_r)?,
                    validated_stf_value("highlight_r", highlight_r)?,
                ],
                [
                    validated_stf_value("shadow_g", shadow_g)?,
                    validated_stf_value("midtone_g", midtone_g)?,
                    validated_stf_value("highlight_g", highlight_g)?,
                ],
                [
                    validated_stf_value("shadow_b", shadow_b)?,
                    validated_stf_value("midtone_b", midtone_b)?,
                    validated_stf_value("highlight_b", highlight_b)?,
                ],
            ],
            linked,
        };
        let paths = [r_path.as_deref(), g_path.as_deref(), b_path.as_deref()];

        if let Some(file) = rgb_file_export(paths)? {
            let stats = [compute_image_stats(&file.r), compute_image_stats(&file.g), compute_image_stats(&file.b)];
            let (r_out, g_out, b_out) =
                png_rgb_planes([&file.r, &file.g, &file.b], [&stats[0], &stats[1], &stats[2]], &stf);
            render_rgb_png((&r_out, &g_out, &b_out), depth, &output_path)?;
            return Ok(rgb_png_response(&output_path, depth, true, file.r.dim(), t0));
        }

        if png_rgb_uses_composite(paths) {
            if let Some((tr, tg, tb)) = helpers::load_composite_toned().or_else(helpers::load_composite_stretched) {
                render_rgb_png((tr.arr(), tg.arr(), tb.arr()), depth, &output_path)?;
                return Ok(rgb_png_response(&output_path, depth, false, tr.arr().dim(), t0));
            }

            let cache_r = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_R);
            let cache_g = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_G);
            let cache_b = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_B);

            if let (Some(cr), Some(cg), Some(cb)) = (&cache_r, &cache_g, &cache_b) {
                let (r_out, g_out, b_out) =
                    png_rgb_planes([cr.arr(), cg.arr(), cb.arr()], [cr.stats(), cg.stats(), cb.stats()], &stf);
                render_rgb_png((&r_out, &g_out, &b_out), depth, &output_path)?;
                return Ok(rgb_png_response(&output_path, depth, do_stf, cr.arr().dim(), t0));
            }
        }

        let [r_entry, g_entry, b_entry] = paths.map(|p| p.map(load_export_channel).transpose());
        let (r_entry, g_entry, b_entry) = (r_entry?, g_entry?, b_entry?);

        let any_entry = r_entry.as_ref().or(g_entry.as_ref()).or(b_entry.as_ref())
            .ok_or_else(|| anyhow::anyhow!("At least one channel path required"))?;
        let rows = [&r_entry, &g_entry, &b_entry].iter().flat_map(|e| e.as_ref()).map(|e| e.arr.dim().0).max().unwrap_or(0);
        let cols = [&r_entry, &g_entry, &b_entry].iter().flat_map(|e| e.as_ref()).map(|e| e.arr.dim().1).max().unwrap_or(0);
        let zeros = Arc::new(Array2::<f32>::zeros(any_entry.arr.dim()));

        let [ra, ga, ba] = [r_entry, g_entry, b_entry]
            .map(|e| resample_to(e.map_or_else(|| Arc::clone(&zeros), |e| e.arr), (rows, cols)));
        let (ra, ga, ba) = (ra?, ga?, ba?);

        let stats = [compute_image_stats(&ra), compute_image_stats(&ga), compute_image_stats(&ba)];
        let (r_out, g_out, b_out) = png_rgb_planes([&ra, &ga, &ba], [&stats[0], &stats[1], &stats[2]], &stf);
        render_rgb_png((&r_out, &g_out, &b_out), depth, &output_path)?;
        Ok(rgb_png_response(&output_path, depth, true, (rows, cols), t0))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::load_cached;
    use crate::infra::fits::reader::parse_header_at;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef;
    use crate::types::constants::{
        BLOCK_SIZE, RES_DROPPED, RES_KEPT_RAW, RES_OUTPUT_PATH, RES_OUTPUT_SIZE_BYTES,
        RES_SOURCE_SIZE_BYTES,
    };

    #[tokio::test]
    async fn compress_mef_cmd_compresses_sci_drops_dq_and_keeps_err_raw() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("mef_source.fits");
        sci_err_dq_mef(&source, 8, 8, vec![0i32; 64]);
        let output = dir.path().join("mef_output.fits");

        let value = compress_mef_cmd(
            source.to_str().unwrap().to_string(),
            output.to_str().unwrap().to_string(),
            false,
            Some(16.0),
            vec!["DQ".to_string()],
            vec!["ERR".to_string()],
        )
        .await
        .unwrap();

        assert_eq!(value[RES_OUTPUT_PATH], output.to_str().unwrap());
        assert_eq!(value[RES_DROPPED], serde_json::json!(["DQ"]));
        assert_eq!(value[RES_KEPT_RAW], serde_json::json!(["ERR"]));
        assert!(value[RES_SOURCE_SIZE_BYTES].as_u64().unwrap() > 0);
        assert!(value[RES_OUTPUT_SIZE_BYTES].as_u64().unwrap() > 0);

        let bytes = std::fs::read(&output).unwrap();
        let primary = parse_header_at(&bytes, 0).unwrap();
        let mut offset = primary.next_hdu_offset;
        let mut extnames = Vec::new();
        let mut compressed = Vec::new();
        while offset + BLOCK_SIZE <= bytes.len() {
            let hdu = parse_header_at(&bytes, offset).unwrap();
            if let Some(name) = hdu.header.get("EXTNAME") {
                extnames.push(name.trim().trim_matches('\'').trim().to_string());
                compressed.push(hdu.header.get("ZCMPTYPE").is_some());
            }
            offset = hdu.next_hdu_offset;
        }

        assert!(!extnames.iter().any(|n| n == "DQ"), "{extnames:?}");
        let sci = extnames.iter().position(|n| n == "SCI").unwrap();
        assert!(compressed[sci], "SCI must be compressed");
        let err = extnames.iter().position(|n| n == "ERR").unwrap();
        assert!(!compressed[err], "ERR must be copied verbatim");
    }

    #[tokio::test]
    async fn compress_mef_cmd_reports_the_image_hdus_it_left_uncompressed() {
        use crate::infra::fits::reader::test_fixtures::{empty_primary_cards, plane_hdu, write_raw_hdus};
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("with_spectrum.fits");
        let spectrum_cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "1".into()),
            ("NAXIS1", "10".into()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", "'SPEC    '".into()),
        ];
        let spectrum: Vec<u8> = (0..10).flat_map(|i| (i as f32).to_be_bytes()).collect();
        write_raw_hdus(&source, &[(empty_primary_cards(), Vec::new()), plane_hdu("SCI", 8, 8), (spectrum_cards, spectrum)]);
        let output = dir.path().join("with_spectrum_compressed.fits");

        let value = compress_mef_cmd(
            source.to_str().unwrap().to_string(),
            output.to_str().unwrap().to_string(),
            true,
            None,
            Vec::new(),
            Vec::new(),
        )
        .await
        .unwrap();
        assert_eq!(value[RES_UNCOMPRESSED], serde_json::json!(["SPEC"]));
    }

    #[tokio::test]
    async fn fits_exports_refuse_to_overwrite_a_file_they_read() {
        let dir = tempfile::tempdir().unwrap();
        let [r, g, b, header_src] = ["guard_r.fits", "guard_g.fits", "guard_b.fits", "guard_wcs.fits"].map(|n| tmp_path(&dir, n));
        for (i, path) in [&r, &g, &b, &header_src].into_iter().enumerate() {
            crate::infra::fits::writer::write_fits_mono(path, &ramp(i as f32 * 10.0), None).unwrap();
        }
        let before = std::fs::read(&r).unwrap();

        for target in [r.clone(), r.replace('\\', "/")] {
            let err = export_fits(r.clone(), target, None, None, None, None, None, None, None, None, None).await.unwrap_err();
            assert!(err.contains("source"), "{err}");
        }
        let same = || [Some(r.clone()), Some(r.clone()), Some(r.clone())];
        let [sr, sg, sb] = same();
        let err = export_fits_rgb(sr, sg, sb, r.clone(), None, None, None, None, None, None, None).await.unwrap_err();
        assert!(err.contains("source"), "{err}");
        let err = export_fits_rgb(Some(r.clone()), Some(g.clone()), Some(b.clone()), g.clone(), None, None, None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("source"), "{err}");
        let err = export_fits_rgb(
            Some(r.clone()), Some(g.clone()), Some(b.clone()), header_src.clone(),
            None, None, None, None, None, None, Some(header_src.clone()),
        )
        .await
        .unwrap_err();
        assert!(err.contains("source"), "{err}");
        assert!(std::fs::read(&r).unwrap() == before, "the source file was modified");

        let fresh = tmp_path(&dir, "guard_out.fits");
        export_fits_rgb(Some(r), Some(g), Some(b), fresh, None, None, None, None, None, None, None).await.unwrap();
    }

    #[tokio::test]
    async fn a_channel_resampled_onto_a_larger_grid_exports_a_header_for_that_grid() {
        let dir = tempfile::tempdir().unwrap();
        let [r, g, b] = ["grid_r.fits", "grid_g.fits", "grid_b.fits"].map(|n| tmp_path(&dir, n));
        let native = header_with(&[
            ("CTYPE1", "'RA---TAN'"),
            ("CTYPE2", "'DEC--TAN'"),
            ("CRPIX1", "4.5"),
            ("CRPIX2", "3.5"),
            ("CDELT1", "-1.0E-4"),
            ("CDELT2", "1.0E-4"),
            ("PIXAR_SR", "4.0E-14"),
        ]);
        crate::infra::fits::writer::write_fits_mono(&r, &ramp(1.0), Some(&native)).unwrap();
        let large = Array2::from_shape_fn((12, 16), |(y, x)| (y * 16 + x) as f32);
        crate::infra::fits::writer::write_fits_mono(&g, &large, None).unwrap();
        crate::infra::fits::writer::write_fits_mono(&b, &large, None).unwrap();

        let out = tmp_path(&dir, "grid_rgb.fits");
        export_fits_rgb(Some(r), Some(g), Some(b), out.clone(), None, None, None, None, None, None, None).await.unwrap();
        let written = try_extract_rgb_resolved(&out).unwrap().expect("a 3-plane RGB FITS");
        assert_eq!(written.r.dim(), (12, 16));
        let close = |key: &str, expected: f64| {
            let actual = written.header.get_f64(key);
            assert!(actual.is_some_and(|v| (v - expected).abs() <= expected.abs() * 1e-9), "{key}: {actual:?}, expected {expected}");
        };
        close("CDELT1", -5.0e-5);
        close("CDELT2", 5.0e-5);
        close("CRPIX1", 8.5);
        close("CRPIX2", 6.5);
        close("PIXAR_SR", 1.0e-14);
    }

    #[test]
    fn fits_rgb_composite_only_when_at_most_one_real_path() {
        let a = Some("C:/data/r.fits");
        let b = Some("C:/data/g.fits");
        let c = Some("C:/data/b.fits");
        assert!(fits_rgb_uses_composite([None, None, None]));
        assert!(fits_rgb_uses_composite([a, None, None]));
        assert!(fits_rgb_uses_composite([Some("__wizard_ch_r_aligned"), None, None]));
        assert!(fits_rgb_uses_composite([Some("__composite_r"), Some("__composite_g"), Some("__composite_b")]));
        assert!(!fits_rgb_uses_composite([Some("__wizard_ch_r_aligned"), Some("__wizard_ch_g_aligned"), Some("__wizard_ch_b_aligned")]));
        assert!(!fits_rgb_uses_composite([a, Some("__wizard_ch_g_aligned"), None]));
        assert!(!fits_rgb_uses_composite([a, b, None]));
        assert!(!fits_rgb_uses_composite([a, b, c]));
    }

    #[test]
    fn png_single_wizard_channel_is_not_composite() {
        assert!(!png_rgb_uses_composite([Some("__wizard_ch_r_aligned"), None, None]));
        assert!(!png_rgb_uses_composite([None, Some("__wizard_ch_g_cropped"), None]));
        assert!(png_rgb_uses_composite([Some("__composite_r"), Some("__composite_g"), None]));
    }

    #[test]
    fn png_rgb_composite_only_when_no_real_path() {
        let a = Some("C:/data/r.fits");
        let b = Some("C:/data/g.fits");
        let c = Some("C:/data/b.fits");
        assert!(png_rgb_uses_composite([None, None, None]));
        assert!(png_rgb_uses_composite([Some("__composite_r"), None, None]));
        assert!(!png_rgb_uses_composite([a, None, None]));
        assert!(!png_rgb_uses_composite([a, b, None]));
        assert!(!png_rgb_uses_composite([a, b, c]));
    }

    fn tmp_path(dir: &tempfile::TempDir, name: &str) -> String {
        dir.path().join(name).to_str().unwrap().to_string()
    }

    fn ramp(base: f32) -> Array2<f32> {
        Array2::from_shape_fn((6, 8), |(y, x)| base + (y * 8 + x) as f32)
    }

    fn header_with(cards: &[(&str, &str)]) -> HduHeader {
        let mut header = HduHeader::empty();
        for (k, v) in cards {
            header.set(k, v.to_string());
        }
        header
    }

    fn uniform_stf(shadow: f64, midtone: f64, highlight: f64) -> [[Option<f64>; 3]; 3] {
        [[Some(shadow), Some(midtone), Some(highlight)]; 3]
    }

    fn apply_each(
        planes: [&Array2<f32>; 3],
        params: [StfParams; 3],
        stats: [&ImageStats; 3],
    ) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
        (
            apply_stf_f32(planes[0], &params[0], stats[0]),
            apply_stf_f32(planes[1], &params[1], stats[1]),
            apply_stf_f32(planes[2], &params[2], stats[2]),
        )
    }

    struct Channels {
        planes: [Array2<f32>; 3],
        stats: [ImageStats; 3],
        combined: ImageStats,
    }

    fn unbalanced_channels() -> Channels {
        let planes = [ramp(10.0), ramp(10.0).mapv(|v| v * 3.0), ramp(10.0).mapv(|v| v * 9.0 + 400.0)];
        let stats = [
            compute_image_stats(&planes[0]),
            compute_image_stats(&planes[1]),
            compute_image_stats(&planes[2]),
        ];
        let combined = combine_channel_stats(&stats[0], &stats[1], &stats[2]);
        Channels { planes, stats, combined }
    }

    impl Channels {
        fn planes(&self) -> [&Array2<f32>; 3] {
            [&self.planes[0], &self.planes[1], &self.planes[2]]
        }

        fn stats(&self) -> [&ImageStats; 3] {
            [&self.stats[0], &self.stats[1], &self.stats[2]]
        }

        fn linked(&self) -> [&ImageStats; 3] {
            [&self.combined; 3]
        }
    }

    fn stf(shadow: f64, midtone: f64, highlight: f64) -> StfParams {
        StfParams { shadow, midtone, highlight }
    }

    #[test]
    fn an_explicit_shadow_and_highlight_are_honoured_with_a_neutral_midtone() {
        let ch = unbalanced_channels();
        let request = PngStf { apply: true, channels: uniform_stf(0.2, 0.5, 0.9), linked: None };
        let expected = apply_each(ch.planes(), [stf(0.2, 0.5, 0.9); 3], ch.linked());
        assert_eq!(png_rgb_planes(ch.planes(), ch.stats(), &request), expected, "the user's clip was replaced by auto STF");

        let ignored = PngStf { apply: false, ..request };
        let auto = auto_stf(&ch.combined, &AutoStfConfig::default());
        assert_eq!(png_rgb_planes(ch.planes(), ch.stats(), &ignored), apply_each(ch.planes(), [auto; 3], ch.linked()));
    }

    #[test]
    fn the_linked_flag_decides_the_normalisation_instead_of_parameter_equality() {
        let ch = unbalanced_channels();
        let equal = uniform_stf(0.1, 0.3, 0.95);
        let unlinked = PngStf { apply: true, channels: equal, linked: Some(false) };
        assert_eq!(
            png_rgb_planes(ch.planes(), ch.stats(), &unlinked),
            apply_each(ch.planes(), [stf(0.1, 0.3, 0.95); 3], ch.stats()),
            "unlinked STF with equal values was normalised with the combined stats"
        );

        let mut different = equal;
        different[2] = [Some(0.0), Some(0.6), Some(1.0)];
        let linked = PngStf { apply: true, channels: different, linked: Some(true) };
        let params = [stf(0.1, 0.3, 0.95), stf(0.1, 0.3, 0.95), stf(0.0, 0.6, 1.0)];
        assert_eq!(png_rgb_planes(ch.planes(), ch.stats(), &linked), apply_each(ch.planes(), params, ch.linked()));

        let inferred = PngStf { linked: None, ..linked };
        assert_eq!(png_rgb_planes(ch.planes(), ch.stats(), &inferred), apply_each(ch.planes(), params, ch.stats()));
        let inferred_equal = PngStf { linked: None, ..unlinked };
        assert_eq!(
            png_rgb_planes(ch.planes(), ch.stats(), &inferred_equal),
            apply_each(ch.planes(), [stf(0.1, 0.3, 0.95); 3], ch.linked())
        );

        let auto_unlinked = PngStf { apply: false, channels: [[None; 3]; 3], linked: Some(false) };
        let config = AutoStfConfig::default();
        let per_channel = [auto_stf(&ch.stats[0], &config), auto_stf(&ch.stats[1], &config), auto_stf(&ch.stats[2], &config)];
        assert_eq!(
            png_rgb_planes(ch.planes(), ch.stats(), &auto_unlinked),
            apply_each(ch.planes(), per_channel, ch.stats())
        );
    }

    #[tokio::test]
    async fn export_rgb_png_rejects_stf_values_outside_the_unit_range() {
        let dir = tempfile::tempdir().unwrap();
        let out = tmp_path(&dir, "bad_stf.png");
        for bad in [2.0, -0.1, f64::NAN] {
            let err = export_rgb_png(
                None, None, None, out.clone(), Some(8), Some(true),
                None, Some(bad), None, None, None, None, None, None, None, None,
            )
            .await
            .unwrap_err();
            assert!(err.contains("midtone_r"), "{err}");
        }
    }

    #[tokio::test]
    async fn the_linear_composite_png_honours_a_neutral_midtone_stf_and_the_linked_flag() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = tmp_path(&dir, "composite_linear.png");
        let expected_path = tmp_path(&dir, "expected.png");
        let ch = unbalanced_channels();
        helpers::clear_composite();
        helpers::insert_composite_and_orig(
            ch.planes[0].clone(), ch.planes[1].clone(), ch.planes[2].clone(),
            ch.stats[0].clone(), ch.stats[1].clone(), ch.stats[2].clone(),
        );

        let (s, m, h) = (Some(0.1), Some(0.5), Some(0.9));
        let value = export_rgb_png(None, None, None, out.clone(), Some(8), Some(true), s, m, h, s, m, h, s, m, h, Some(false)).await;
        helpers::clear_composite();
        assert_eq!(value.unwrap()[RES_APPLY_STF], json!(true));

        let (er, eg, eb) = apply_each(ch.planes(), [stf(0.1, 0.5, 0.9); 3], ch.stats());
        render_rgb_png((&er, &eg, &eb), 8, &expected_path).unwrap();
        assert!(
            std::fs::read(&out).unwrap() == std::fs::read(&expected_path).unwrap(),
            "the composite export replaced the explicit STF or ignored linked=false"
        );
    }

    #[tokio::test]
    async fn export_fits_writes_the_pixels_it_read_from_disk_not_a_stale_cache_entry() {
        let dir = tempfile::tempdir().unwrap();
        let src = tmp_path(&dir, "rerun_hdr.fits");
        let out = tmp_path(&dir, "exported.fits");
        crate::infra::fits::writer::write_fits_mono(&src, &Array2::from_elem((4, 4), 1.0), None).unwrap();
        let first_stamp = std::fs::metadata(&src).unwrap().modified().unwrap();
        assert_eq!(load_cached(&src).unwrap().arr()[[0, 0]], 1.0);

        crate::infra::fits::writer::write_fits_mono(&src, &Array2::from_elem((4, 4), 2.0), None).unwrap();
        std::fs::OpenOptions::new().write(true).open(&src).unwrap().set_modified(first_stamp).unwrap();

        export_fits(src.clone(), out.clone(), None, None, None, None, None, None, None, None, None).await.unwrap();
        assert_eq!(extract_image_resolved(&out).unwrap().arr[[0, 0]], 2.0, "the previous run's pixels were exported");
    }

    #[tokio::test]
    async fn an_rgb_fits_given_for_all_three_channels_exports_its_own_planes() {
        let dir = tempfile::tempdir().unwrap();
        let src = tmp_path(&dir, "osc_rgb.fits");
        let (r, g, b) = (ramp(1.0), ramp(100.0), ramp(1000.0));
        let header = header_with(&[("OBJECT", "'NGC7000'")]);
        crate::infra::fits::writer::write_fits_rgb(&src, &r, &g, &b, Some(&header)).unwrap();

        let fits_out = tmp_path(&dir, "osc_cube.fits");
        export_fits_rgb(
            Some(src.clone()), Some(src.clone()), Some(src.clone()), fits_out.clone(),
            None, None, None, None, None, None, None,
        )
        .await
        .unwrap();
        let written = try_extract_rgb_resolved(&fits_out).unwrap().expect("a 3-plane RGB FITS");
        assert_eq!((&written.r, &written.g, &written.b), (&r, &g, &b));
        assert_eq!(written.header.get("OBJECT").map(|v| v.trim().trim_matches('\'').trim()), Some("NGC7000"));

        let png_out = tmp_path(&dir, "osc_rgb.png");
        export_rgb_png(
            Some(src.clone()), Some(src.clone()), Some(src), png_out.clone(), Some(8), None,
            None, None, None, None, None, None, None, None, None, None,
        )
        .await
        .unwrap();
        let png = image::open(&png_out).unwrap().to_rgb8();
        assert_eq!((png.width(), png.height()), (8, 6));
        assert!(png.pixels().any(|p| p[0] != p[2]), "the RGB file was exported as a grey plane");
    }

    #[tokio::test]
    async fn wizard_channels_held_only_in_memory_export_as_fits_with_the_requested_header() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let keys = ["r", "g", "b"].map(|c| crate::types::constants::wizard_bg_key(&format!("export_test_{c}")));
        let planes = [ramp(3.0), ramp(30.0), ramp(300.0)];
        for (key, plane) in keys.iter().zip(&planes) {
            GLOBAL_IMAGE_CACHE.insert_synthetic(key, Arc::new(plane.clone()), compute_image_stats(plane));
        }
        let source = tmp_path(&dir, "bin_first_file.fits");
        let solved = header_with(&[("CTYPE1", "'RA---TAN'"), ("CRVAL1", "83.8"), ("CRPIX1", "4.0")]);
        crate::infra::fits::writer::write_fits_mono(&source, &ramp(0.0), Some(&solved)).unwrap();

        let out = tmp_path(&dir, "wizard_rgb.fits");
        let [kr, kg, kb] = keys.clone().map(Some);
        let result = export_fits_rgb(kr, kg, kb, out.clone(), None, None, None, None, None, None, Some(source)).await;
        let bare_out = tmp_path(&dir, "wizard_rgb_bare.fits");
        let [kr, kg, kb] = keys.clone().map(Some);
        let bare = export_fits_rgb(kr, kg, kb, bare_out, None, None, None, None, None, None, None).await;
        for key in &keys {
            GLOBAL_IMAGE_CACHE.remove(key);
        }
        result.unwrap();
        assert_eq!(bare.unwrap()[RES_WCS_WRITTEN], json!(false), "an export without any header claimed a WCS");

        let written = try_extract_rgb_resolved(&out).unwrap().expect("a 3-plane RGB FITS");
        assert_eq!([&written.r, &written.g, &written.b], [&planes[0], &planes[1], &planes[2]]);
        assert_eq!(written.header.get("CRVAL1").map(str::trim), Some("83.8"), "the WCS of the bin's source was dropped");

        let [kr, kg, kb] = keys.map(Some);
        let gone = export_fits_rgb(kr, kg, kb, tmp_path(&dir, "gone.fits"), None, None, None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(gone.contains("no longer in memory"), "{gone}");
    }

    #[tokio::test]
    async fn a_background_corrected_wizard_channel_keeps_its_wcs_in_the_composite_export() {
        let _composite = helpers::composite_test_lock().await;
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let key = crate::types::constants::wizard_bg_key("export_wcs_r");
        let solved = header_with(&[
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
            ("CRVAL1", "83.8"),
            ("CRVAL2", "-5.4"),
            ("CRPIX1", "4.5"),
            ("CRPIX2", "3.5"),
            ("CDELT1", "-1.0E-4"),
            ("CDELT2", "1.0E-4"),
        ]);
        let channel = ramp(1.0);
        GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(&key, Arc::new(channel.clone()), compute_image_stats(&channel), Some(solved));
        let composite = Array2::from_shape_fn((12, 16), |(y, x)| (y * 16 + x) as f32);
        let stats = compute_image_stats(&composite);
        helpers::clear_composite();
        helpers::insert_composite_and_orig(
            composite.clone(), composite.clone(), composite.clone(),
            stats.clone(), stats.clone(), stats,
        );

        let out = tmp_path(&dir, "composite_bg.fits");
        let result = export_fits_rgb(Some(key.clone()), None, None, out.clone(), None, None, None, None, None, None, None).await;
        helpers::clear_composite();
        GLOBAL_IMAGE_CACHE.remove(&key);
        assert_eq!(result.unwrap()[RES_WCS_WRITTEN], json!(true), "the export did not report the WCS it wrote");

        let written = try_extract_rgb_resolved(&out).unwrap().expect("a 3-plane RGB FITS");
        assert_eq!(written.r.dim(), (12, 16));
        assert_eq!(written.header.get("CRVAL1").map(str::trim), Some("83.8"), "the WCS of the background-corrected channel was dropped");
        let close = |key: &str, expected: f64| {
            let actual = written.header.get_f64(key);
            assert!(actual.is_some_and(|v| (v - expected).abs() <= expected.abs() * 1e-9), "{key}: {actual:?}, expected {expected}");
        };
        close("CRPIX1", 8.5);
        close("CRPIX2", 6.5);
        close("CDELT1", -5.0e-5);
        close("CDELT2", 5.0e-5);
    }
}
