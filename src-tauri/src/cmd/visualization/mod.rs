use serde_json::json;

use tauri::ipc::Response;

use crate::cmd::common::{
    blocking_cmd, cached_header, load_cached, load_cached_full, load_companions, output_stem,
    resolve_output_dir, save_stf_preview_png, LoadedCompanions,
};
use crate::cmd::helpers;
use crate::cmd::io::{load_rgb_file_planes, RgbPlanes};
use crate::cmd::processing::is_display_referred;
use crate::core::imaging::colormap::Colormap;
use crate::core::imaging::dq_flags::{mask_preview_or, DqTable};
use crate::core::imaging::pixel_probe::{
    data_unit, probe_companions, probe_json_with_companions, probe_pixel,
};
use crate::core::imaging::scale::{resolve_limits, LimitMode};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, make_stf_u8_fn, AutoStfConfig, ImageStats, StfParams};
use crate::infra::ipc::encode_mask_with_header;
use crate::infra::render::tiles;
use crate::types::constants::{
    DEFAULT_MASK_PREVIEW_DIM, DEFAULT_PROBE_BOX, RES_ALGORITHM, RES_BIT, RES_COLORMAPS,
    RES_DEFAULT_MASK, RES_DQ_REF, RES_ERR_REF, RES_EXCLUSION_MASK, RES_FLAGS, RES_HIGHLIGHT,
    RES_LABEL, RES_MIDTONE, RES_NAME, RES_PNG_PATH, RES_RGBA, RES_SHADOW, RES_TABLE, RES_VMAX,
    RES_VMIN,
};
use crate::types::header::HduHeader;
use crate::types::image_ref::ImageRef;

pub(crate) const NO_DQ_PLANE: &str = "No DQ plane available for this image";

fn display_limits(data: &ndarray::Array2<f32>, header: Option<&HduHeader>, mode: LimitMode) -> (f64, f64) {
    match mode {
        LimitMode::User { .. } => resolve_limits(data, mode),
        _ if is_display_referred(header) => (0.0, 1.0),
        _ => resolve_limits(data, mode),
    }
}

pub(crate) fn resolve_overlay_mask(table: DqTable, requested: Option<u32>) -> u32 {
    requested.unwrap_or_else(|| table.default_overlay_mask())
}

pub(crate) fn dq_table_for(path: &str) -> anyhow::Result<(DqTable, Option<String>, Option<String>)> {
    let active = load_cached_full(path)?;
    let comps = active.companions().cloned().unwrap_or_default();
    let dq_ref = comps.dq.as_ref().map(ImageRef::cache_key);
    let err_ref = comps.err.as_ref().map(ImageRef::cache_key);
    let loaded = load_companions(path)?;
    let table = match loaded.dq {
        Some((_, table)) => table,
        None => active.header().map(DqTable::select).unwrap_or(DqTable::Unknown),
    };
    Ok((table, dq_ref, err_ref))
}

#[tauri::command]
pub async fn get_dq_flag_table_cmd(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let (table, dq_ref, err_ref) = dq_table_for(&path)?;
        let flags: Vec<serde_json::Value> = table
            .flags()
            .iter()
            .map(|f| json!({ RES_BIT: f.bit, RES_NAME: f.name }))
            .collect();
        Ok(json!({
            RES_TABLE: table,
            RES_LABEL: table.label(),
            RES_FLAGS: flags,
            RES_DEFAULT_MASK: table.default_overlay_mask(),
            RES_EXCLUSION_MASK: table.exclusion_mask(),
            RES_DQ_REF: dq_ref,
            RES_ERR_REF: err_ref,
        }))
    })
}

#[tauri::command]
pub async fn get_dq_mask_preview(
    path: String,
    mask: Option<u32>,
    max_dim: Option<u32>,
) -> Result<Response, String> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<Response> {
        let dim = max_dim.unwrap_or(DEFAULT_MASK_PREVIEW_DIM) as usize;
        let loaded = load_companions(&path)?;
        let (entry, table) = loaded.dq.ok_or_else(|| anyhow::anyhow!(NO_DQ_PLANE))?;
        let plane = entry.int_plane().ok_or_else(|| anyhow::anyhow!(NO_DQ_PLANE))?;
        let applied = resolve_overlay_mask(table, mask);
        let (cells, width, height) = mask_preview_or(&plane.bits, applied, dim);
        Ok(Response::new(encode_mask_with_header(
            &cells,
            width as u32,
            height as u32,
            applied,
            table.id(),
        )))
    })
    .await
    .map_err(|e| format!("{}", e))?
    .map_err(|e| format!("{:#}", e))
}

#[tauri::command]
pub async fn compute_scale_limits_cmd(
    path: String,
    algorithm: String,
    vmin: Option<f64>,
    vmax: Option<f64>,
    percentile: Option<[f64; 2]>,
    zscale_contrast: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let mode = LimitMode::from_parts(&algorithm, vmin, vmax, percentile, zscale_contrast)
            .map_err(|e| anyhow::anyhow!(e))?;
        let cached = load_cached(&path)?;
        let header = cached_header(&path).ok();
        let (lo, hi) = display_limits(cached.arr(), header.as_ref(), mode);
        Ok(json!({
            RES_VMIN: lo,
            RES_VMAX: hi,
            RES_ALGORITHM: mode.name(),
        }))
    })
}

#[tauri::command]
pub async fn get_colormap_lut_cmd(name: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let cmap = Colormap::from_name(&name).map_err(|e| anyhow::anyhow!(e))?;
        let names: Vec<&str> = Colormap::ALL.iter().map(|c| c.name()).collect();
        Ok(json!({
            RES_NAME: cmap.name(),
            RES_RGBA: cmap.lut_rgba().to_vec(),
            RES_COLORMAPS: names,
        }))
    })
}

#[tauri::command]
pub async fn probe_pixel_cmd(
    path: String,
    x: f64,
    y: f64,
    box_size: Option<u32>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let entry = load_cached_full(&path)?;
        let probe = probe_pixel(entry.arr(), x, y, box_size.unwrap_or(DEFAULT_PROBE_BOX))
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let unit = entry.header().and_then(data_unit);
        let comps = load_companions(&path).unwrap_or_else(|e| {
            log::warn!("companion lookup failed for {}: {:#}", path, e);
            LoadedCompanions::default()
        });
        let dq = comps
            .dq
            .as_ref()
            .and_then(|(e, table)| e.int_plane().map(|p| (p, *table)));
        let err_unit = comps.err.as_ref().and_then(|e| e.header().and_then(data_unit));
        let companions = probe_companions(dq, comps.err.as_ref().map(|e| e.arr()), probe.x, probe.y);
        Ok(probe_json_with_companions(&probe, unit.as_deref(), err_unit.as_deref(), &companions))
    })
}

#[tauri::command]
pub async fn apply_stf_render(
    path: String,
    output_dir: String,
    shadow: f64,
    midtone: f64,
    highlight: f64,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;

        let cached = load_cached(&path)?;

        let stf_params = StfParams {
            shadow,
            midtone,
            highlight,
        };

        let png_path = format!("{}/{}_stf.png", output_dir, output_stem(&path));
        save_stf_preview_png(cached.arr(), &stf_params, cached.stats(), &png_path)?;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_SHADOW: shadow,
            RES_MIDTONE: midtone,
            RES_HIGHLIGHT: highlight,
        }))
    })
}

#[tauri::command]
pub async fn generate_tiles(
    path: String,
    output_dir: String,
    tile_size: u32,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let params = tile_params(tile_size)?;
        let output_dir = resolve_output_dir(&output_dir)?;

        let cached = load_cached(&path)?;

        let stf_params = auto_stf(cached.stats(), &AutoStfConfig::default());
        let normalized = apply_stf_f32(cached.arr(), &stf_params, cached.stats());

        let result = tiles::generate_tile_pyramid(&normalized, &output_dir, &params)?;
        Ok(serde_json::to_value(&result).unwrap_or(json!({})))
    })
}

fn tile_params(tile_size: u32) -> anyhow::Result<tiles::TileParams> {
    let size = tile_size as usize;
    if !(tiles::MIN_TILE_SIZE..=tiles::MAX_TILE_SIZE).contains(&size) {
        anyhow::bail!(
            "tile_size must be between {} and {} pixels, got {}",
            tiles::MIN_TILE_SIZE,
            tiles::MAX_TILE_SIZE,
            tile_size
        );
    }
    Ok(tiles::TileParams { tile_size: size })
}

fn tile_rgb_source(path: Option<&str>) -> anyhow::Result<(RgbPlanes, [ImageStats; 3])> {
    match path {
        Some(p) => {
            let (r, g, b) = load_rgb_file_planes(p)?;
            let (stats_r, (stats_g, stats_b)) = rayon::join(
                || compute_image_stats(&r),
                || rayon::join(|| compute_image_stats(&g), || compute_image_stats(&b)),
            );
            Ok(((r, g, b), [stats_r, stats_g, stats_b]))
        }
        None => {
            let (r, g, b) = helpers::load_composite_rgb()
                .map_err(|_| anyhow::anyhow!("RGB composite not available. Run Compose RGB first."))?;
            let stats = [r.stats().clone(), g.stats().clone(), b.stats().clone()];
            Ok(((r.data_arc(), g.data_arc(), b.data_arc()), stats))
        }
    }
}

#[tauri::command]
pub async fn generate_tiles_rgb(
    output_dir: String,
    tile_size: u32,
    path: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let params = tile_params(tile_size)?;
        let output_dir = resolve_output_dir(&output_dir)?;

        let ((r, g, b), [stats_r, stats_g, stats_b]) = tile_rgb_source(path.as_deref())?;

        let cfg = AutoStfConfig::default();
        let (stf, combined) = helpers::compute_linked_stf_with_stats(&stats_r, &stats_g, &stats_b, &cfg);

        let fn_r = make_stf_u8_fn(&stf, &combined);
        let fn_g = make_stf_u8_fn(&stf, &combined);
        let fn_b = make_stf_u8_fn(&stf, &combined);

        let result = tiles::generate_tile_pyramid_rgb_stf(&r, &g, &b, &output_dir, &params, fn_r, fn_g, fn_b)?;
        Ok(serde_json::to_value(&result).unwrap_or(json!({})))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::fits::reader::test_fixtures::{sci_err_dq_mef, sci_err_dq_mef_with_dq_cards};

    #[test]
    fn overlay_mask_defaults_per_table() {
        assert_eq!(resolve_overlay_mask(DqTable::Jwst, None), 7);
        assert_eq!(resolve_overlay_mask(DqTable::Hst, None), 4 | 256 | 4096 | 8192);
        assert_eq!(resolve_overlay_mask(DqTable::Unknown, None), 7);
        assert_eq!(resolve_overlay_mask(DqTable::Jwst, Some(0)), 0);
        assert_eq!(resolve_overlay_mask(DqTable::Hst, Some(1 << 31)), 1 << 31);
    }

    #[test]
    fn dq_table_for_uses_companion_then_active_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vis.fits");
        sci_err_dq_mef_with_dq_cards(&path, 4, 4, vec![-2147483648i32; 16], vec![("TELESCOP", "'JWST'".into())]);
        let p = path.to_str().unwrap().to_string();
        let sci_header = load_cached_full(&format!("{}#hdu=1", p)).unwrap().header().cloned().unwrap();
        assert_eq!(sci_header.get("TELESCOP"), None);
        assert_eq!(DqTable::select(&sci_header), DqTable::Unknown);

        let (table, dq_ref, err_ref) = dq_table_for(&format!("{}#hdu=1", p)).unwrap();
        assert_eq!(table, DqTable::Jwst);
        assert_eq!(dq_ref.as_deref(), Some(format!("{}#hdu=3", p).as_str()));
        assert_eq!(err_ref.as_deref(), Some(format!("{}#hdu=2", p).as_str()));

        let (table, dq_ref, _) = dq_table_for(&format!("{}#hdu=4", p)).unwrap();
        assert_eq!(table, DqTable::Unknown);
        assert!(dq_ref.is_none());

        let (table, dq_ref, _) = dq_table_for(&format!("{}#hdu=3", p)).unwrap();
        assert_eq!(table, DqTable::Jwst);
        assert_eq!(dq_ref.as_deref(), Some(format!("{}#hdu=3", p).as_str()));
    }

    #[tokio::test]
    async fn deep_zoom_tiles_of_an_rgb_file_come_from_that_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("tiles_rgb.fits").to_str().unwrap().to_string();
        let r = ndarray::Array2::from_shape_fn((9, 7), |(y, x)| (y * 7 + x) as f32);
        crate::infra::fits::writer::write_fits_rgb(&src, &r, &r.mapv(|v| v * 2.0), &r.mapv(|v| v * 3.0), None).unwrap();
        let out = dir.path().join("tiles").to_str().unwrap().to_string();

        let pyramid = generate_tiles_rgb(out.clone(), 256, Some(src)).await.unwrap();
        assert_eq!(pyramid["original_width"], 7);
        assert_eq!(pyramid["original_height"], 9);
        assert_eq!(pyramid["base_dir"], out.as_str());
    }

    #[tokio::test]
    async fn tile_sizes_that_would_divide_by_zero_or_explode_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        for bad in [0u32, 8, 100_000] {
            let err = generate_tiles(dir.path().join("none.fits").to_str().unwrap().to_string(), out.clone(), bad)
                .await
                .unwrap_err();
            assert!(err.contains("tile_size"), "{err}");
            let err = generate_tiles_rgb(out.clone(), bad, None).await.unwrap_err();
            assert!(err.contains("tile_size"), "{err}");
        }
        assert_eq!(tile_params(256).unwrap().tile_size, 256);
    }

    #[tokio::test]
    async fn an_stf_render_of_a_large_image_matches_the_gpu_view_with_the_same_stf() {
        use crate::cmd::io::test_support::{assert_matches_gpu_view, wide_sky};
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("wide_stf.fits").to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&src, &wide_sky(6), None).unwrap();
        let out = dir.path().to_str().unwrap().to_string();

        let value = apply_stf_render(src.clone(), out, 0.1, 0.3, 0.95).await.unwrap();
        let entry = load_cached(&src).unwrap();
        let stf = StfParams { shadow: 0.1, midtone: 0.3, highlight: 0.95 };
        assert_matches_gpu_view(value[RES_PNG_PATH].as_str().unwrap(), entry.arr(), &stf, entry.stats());
    }

    #[test]
    fn tile_sizes_the_pyramid_builder_refuses_are_rejected_before_any_work() {
        for size in [tiles::MIN_TILE_SIZE, tiles::MAX_TILE_SIZE] {
            assert_eq!(tile_params(size as u32).unwrap().tile_size, size);
        }
        for bad in [16u32, 32, 63, 1025, 2048, 4096] {
            let err = tile_params(bad).unwrap_err().to_string();
            assert!(err.contains("tile_size") && err.contains("1024"), "{bad}: {err}");
        }
    }

    #[tokio::test]
    async fn a_display_referred_image_keeps_its_computed_stretch_under_automatic_limits() {
        let dir = tempfile::tempdir().unwrap();
        let data = ndarray::Array2::from_shape_fn((16, 16), |(y, x)| 0.2 + (y * 16 + x) as f32 / 640.0);
        let mut flagged = HduHeader::empty();
        flagged.set(crate::cmd::common::HEADER_DISPLAY_REFERRED, "T".to_string());
        let stretched = dir.path().join("limits_arcsinh.fits").to_str().unwrap().to_string();
        let linear = dir.path().join("limits_linear.fits").to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&stretched, &data, Some(&flagged)).unwrap();
        crate::infra::fits::writer::write_fits_mono(&linear, &data, None).unwrap();

        for algorithm in ["minmax", "zscale", "percentile"] {
            let limits = compute_scale_limits_cmd(stretched.clone(), algorithm.into(), None, None, None, None).await.unwrap();
            assert_eq!((limits[RES_VMIN].as_f64(), limits[RES_VMAX].as_f64()), (Some(0.0), Some(1.0)), "{algorithm}");
            let own = compute_scale_limits_cmd(linear.clone(), algorithm.into(), None, None, None, None).await.unwrap();
            assert!(own[RES_VMIN].as_f64().unwrap() > 0.1, "{algorithm} on linear data: {own}");
        }
        let user = compute_scale_limits_cmd(stretched, "user".into(), Some(0.3), Some(0.4), None, None).await.unwrap();
        assert_eq!((user[RES_VMIN].as_f64(), user[RES_VMAX].as_f64()), (Some(0.3), Some(0.4)));
    }

    #[test]
    fn mask_preview_payload_from_mef() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mask.fits");
        let mut dq = vec![-2147483648i32; 16];
        dq[3] = -2147483647;
        sci_err_dq_mef(&path, 4, 4, dq);
        let key = format!("{}#hdu=1", path.to_str().unwrap());
        let loaded = load_companions(&key).unwrap();
        let (entry, table) = loaded.dq.unwrap();
        let plane = entry.int_plane().unwrap();
        let applied = resolve_overlay_mask(table, None);
        let (cells, w, h) = mask_preview_or(&plane.bits, applied, 2);
        let bytes = encode_mask_with_header(&cells, w as u32, h as u32, applied, table.id());
        assert_eq!(bytes.len(), 16 + 4);
        assert_eq!(u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]), 7);
        assert_eq!(u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]), 0xFFFF_FFFF);
        assert_eq!(&bytes[16..], &[0, 1, 0, 0]);
    }
}
