use serde_json::json;

use tauri::ipc::Response;

use crate::cmd::common::{
    blocking_cmd, load_cached, load_cached_full, load_companions, output_stem,
    resolve_output_dir, save_preview_png, LoadedCompanions,
};
use crate::cmd::helpers;
use crate::core::imaging::colormap::Colormap;
use crate::core::imaging::dq_flags::{mask_preview_or, DqTable};
use crate::core::imaging::pixel_probe::{
    data_unit, probe_companions, probe_json_with_companions, probe_pixel,
};
use crate::core::imaging::scale::{resolve_limits, LimitMode};
use crate::core::imaging::stf::{apply_stf, apply_stf_f32, auto_stf, AutoStfConfig, StfParams};
use crate::infra::ipc::encode_mask_with_header;
use crate::infra::render::tiles;
use crate::types::constants::{
    DEFAULT_MASK_PREVIEW_DIM, DEFAULT_PROBE_BOX, RES_ALGORITHM, RES_BIT, RES_COLORMAPS,
    RES_DEFAULT_MASK, RES_DQ_REF, RES_ERR_REF, RES_EXCLUSION_MASK, RES_FLAGS, RES_HIGHLIGHT,
    RES_LABEL, RES_MIDTONE, RES_NAME, RES_PNG_PATH, RES_RGBA, RES_SHADOW, RES_TABLE, RES_VMAX,
    RES_VMIN,
};
use crate::types::image_ref::ImageRef;

pub(crate) const NO_DQ_PLANE: &str = "No DQ plane available for this image";

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
        let (lo, hi) = resolve_limits(cached.arr(), mode);
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
        resolve_output_dir(&output_dir)?;

        let cached = load_cached(&path)?;
        let (rows, cols) = cached.arr().dim();

        let stf_params = StfParams {
            shadow,
            midtone,
            highlight,
        };

        let rendered = apply_stf(cached.arr(), &stf_params, cached.stats());

        let png_path = format!("{}/{}_stf.png", output_dir, output_stem(&path));
        save_preview_png(rendered, cols, rows, &png_path)?;

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
        resolve_output_dir(&output_dir)?;

        let cached = load_cached(&path)?;

        let stf_params = auto_stf(cached.stats(), &AutoStfConfig::default());
        let normalized = apply_stf_f32(cached.arr(), &stf_params, cached.stats());

        let params = tiles::TileParams {
            tile_size: tile_size as usize,
        };

        let result = tiles::generate_tile_pyramid(&normalized, &output_dir, &params)?;
        Ok(serde_json::to_value(&result).unwrap_or(json!({})))
    })
}

#[tauri::command]
pub async fn generate_tiles_rgb(
    output_dir: String,
    tile_size: u32,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let (entry_r, entry_g, entry_b) = helpers::load_composite_rgb()
            .map_err(|_| anyhow::anyhow!("RGB composite not available. Run Compose RGB first."))?;

        let cfg = AutoStfConfig::default();
        let (stf, combined) = helpers::compute_linked_stf_with_stats(
            entry_r.stats(), entry_g.stats(), entry_b.stats(), &cfg,
        );

        use crate::core::imaging::stf::make_stf_u8_fn;
        let fn_r = make_stf_u8_fn(&stf, &combined);
        let fn_g = make_stf_u8_fn(&stf, &combined);
        let fn_b = make_stf_u8_fn(&stf, &combined);

        let params = tiles::TileParams {
            tile_size: tile_size as usize,
        };

        let result = tiles::generate_tile_pyramid_rgb_stf(
            entry_r.arr(),
            entry_g.arr(),
            entry_b.arr(),
            &output_dir,
            &params,
            fn_r, fn_g, fn_b,
        )?;
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
