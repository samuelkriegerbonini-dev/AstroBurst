use std::time::Instant;

use anyhow::bail;
use serde_json::json;

use tauri::ipc::Response;

use crate::cmd::common::{
    blocking_cmd, cached_header, load_cached, load_cached_full, load_companions, output_stem,
    resolve_output_dir, save_stf_preview_png, ErrOriginJson, LoadedCompanions,
};
use crate::cmd::helpers;
use crate::cmd::io::{load_rgb_file_planes, RgbPlanes};
use crate::cmd::processing::is_display_referred;
use crate::core::imaging::colormap::Colormap;
use crate::core::imaging::dq_flags::{bits_present, mask_preview_or, DqTable, FlagSet};
use crate::core::imaging::pixel_probe::{
    data_unit, grid_origin, grid_stats, grid_stats_json, int_grid, int_value_grid, pixel_grid,
    probe_companions_named, probe_json_with_companions, probe_pixel, wavelength_unit, with_layer_sources,
};
use crate::core::imaging::scale::{
    resolve_limits, symmetric_about, validate_symmetric_centre, LimitMode, DEFAULT_SYMMETRIC_CENTRE,
    SYMMETRIC_FALLBACK_NOTE,
};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, make_stf_u8_fn, AutoStfConfig, ImageStats, StfParams};
use crate::infra::ipc::encode_mask_with_header;
use crate::infra::render::tiles;
use crate::types::constants::{
    DEFAULT_MASK_PREVIEW_DIM, DEFAULT_PROBE_BOX, RES_ALGORITHM, RES_BIT, RES_BITS_PRESENT, RES_CENTRE, RES_COLORMAPS,
    RES_DEFAULT_MASK, RES_DQ, RES_DQ_NAMES, RES_DQ_REF, RES_DQ_TABLE, RES_ELAPSED_MS, RES_ERR,
    RES_ERR_REF, RES_ERR_STATS, RES_EXCLUSION_MASK, RES_FLAGS, RES_HIGHLIGHT, RES_LABEL,
    RES_MIDTONE, RES_NAME, RES_NODATA, RES_NOTES, RES_PNG_PATH, RES_RGBA, RES_SHADOW, RES_SIZE,
    RES_SOURCE, RES_STATS, RES_SYMMETRIC, RES_TABLE, RES_UNIT, RES_VALUES, RES_VMAX, RES_VMIN, RES_WAVELENGTH,
    RES_WAVELENGTH_STATS, RES_WAVELENGTH_UNIT, RES_X, RES_X0, RES_Y, RES_Y0,
};
use crate::types::header::HduHeader;
use crate::types::image_ref::ImageRef;

const RES_SATURATED_MASK: &str = "saturated_mask";
const RES_EXCLUSION_DEFAULT: &str = "exclusion_default";
const RES_EXCLUSION_EDITED: &str = "exclusion_edited";
const RES_DQ_ORIGIN: &str = "dq_origin";
const RES_DQ_UNAVAILABLE: &str = "dq_unavailable";
const RES_DQ_SOURCE: &str = "dq_source";
const RES_ERR_ORIGIN: &str = "err_origin";
const RES_ERR_UNAVAILABLE: &str = "err_unavailable";
const RES_ERR_SOURCE: &str = "err_source";

const SYMMETRIC_SKIPPED_DISPLAY_REFERRED_NOTE: &str =
    "symmetric limits skipped: the image is display-referred (ABDISP=T)";
pub(crate) const NO_DQ_PLANE: &str = "No DQ plane available for this image";
pub(crate) const MIN_PIXEL_TABLE_SIZE: usize = 3;
pub(crate) const MAX_PIXEL_TABLE_SIZE: usize = 15;
const DEFAULT_PIXEL_TABLE_SIZE: usize = 7;

fn pixel_table_size(size: Option<usize>) -> anyhow::Result<usize> {
    let size = size.unwrap_or(DEFAULT_PIXEL_TABLE_SIZE);
    if !(MIN_PIXEL_TABLE_SIZE..=MAX_PIXEL_TABLE_SIZE).contains(&size) || size.is_multiple_of(2) {
        bail!(
            "size must be an odd number between {} and {}, got {}",
            MIN_PIXEL_TABLE_SIZE,
            MAX_PIXEL_TABLE_SIZE,
            size
        );
    }
    Ok(size)
}

fn dq_name_grid(bits: &[Vec<Option<u32>>], flags: &FlagSet) -> Vec<Vec<Option<String>>> {
    bits.iter()
        .map(|row| {
            row.iter()
                .map(|cell| match cell {
                    Some(b) if *b != 0 => Some(flags.decode(*b).join(" | ")),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

fn display_limits(data: &ndarray::Array2<f32>, header: Option<&HduHeader>, mode: LimitMode) -> (f64, f64) {
    match mode {
        LimitMode::User { .. } => resolve_limits(data, mode),
        _ if is_display_referred(header) => (0.0, 1.0),
        _ => resolve_limits(data, mode),
    }
}

pub(crate) fn scale_limits_json(
    path: &str,
    mode: LimitMode,
    symmetric: bool,
    centre: Option<f64>,
) -> anyhow::Result<serde_json::Value> {
    let centre = centre.unwrap_or(DEFAULT_SYMMETRIC_CENTRE);
    if symmetric {
        validate_symmetric_centre(centre).map_err(|e| anyhow::anyhow!(e))?;
    }
    let cached = load_cached(path)?;
    let header = cached_header(path).ok();
    let (mut lo, mut hi) = display_limits(cached.arr(), header.as_ref(), mode);
    let mut notes: Vec<&str> = Vec::new();
    let mut applied = false;
    if symmetric && is_display_referred(header.as_ref()) {
        notes.push(SYMMETRIC_SKIPPED_DISPLAY_REFERRED_NOTE);
    } else if symmetric {
        let s = symmetric_about(centre, lo, hi);
        (lo, hi) = (s.vmin, s.vmax);
        applied = true;
        if s.fallback {
            notes.push(SYMMETRIC_FALLBACK_NOTE);
        }
    }
    Ok(json!({
        RES_VMIN: lo,
        RES_VMAX: hi,
        RES_ALGORITHM: mode.name(),
        RES_SYMMETRIC: applied,
        RES_CENTRE: applied.then_some(centre),
        RES_NOTES: notes,
    }))
}

pub(crate) fn resolve_overlay_mask(flags: &FlagSet, requested: Option<u32>) -> u32 {
    requested.unwrap_or(flags.overlay_mask)
}

pub(crate) struct DqFlagTable {
    pub(crate) set: FlagSet,
    pub(crate) bits_present: Option<u32>,
    pub(crate) dq_ref: Option<String>,
    pub(crate) err_ref: Option<String>,
    pub(crate) dq_origin: Option<String>,
    pub(crate) dq_unavailable: Option<String>,
    pub(crate) err_origin: Option<ErrOriginJson>,
    pub(crate) err_unavailable: Option<String>,
}

pub(crate) fn dq_table_for(path: &str) -> anyhow::Result<DqFlagTable> {
    let active = load_cached_full(path)?;
    let comps = active.companions().cloned().unwrap_or_default();
    let loaded = load_companions(path)?;
    let set = loaded.flags.clone().unwrap_or_else(|| match active.header() {
        Some(header) => FlagSet::from_headers(Some(header), Some(header), DqTable::select(header)),
        None => FlagSet::from_table(DqTable::Unknown),
    });
    let plane = loaded.dq.as_ref().and_then(|(entry, _)| entry.int_plane());
    let err_choice = active.plane_info().and_then(|info| info.pairing.err.as_ref());
    Ok(DqFlagTable {
        bits_present: plane.filter(|_| !set.is_named()).map(|plane| bits_present(&plane.bits)),
        dq_ref: loaded.dq.as_ref().and(comps.dq.as_ref()).map(ImageRef::cache_key),
        err_ref: comps.err.as_ref().map(ImageRef::cache_key),
        err_origin: loaded.err_origin.as_ref().zip(err_choice).map(|(origin, choice)| ErrOriginJson::loaded(origin, choice)),
        dq_origin: loaded.dq_origin,
        dq_unavailable: loaded.dq_unavailable,
        err_unavailable: loaded.err_unavailable,
        set,
    })
}

#[tauri::command]
pub async fn get_dq_flag_table_cmd(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let found = dq_table_for(&path)?;
        let set = &found.set;
        let flags: Vec<serde_json::Value> =
            set.flags.iter().map(|(bit, name)| json!({ RES_BIT: bit, RES_NAME: name })).collect();
        Ok(json!({
            RES_TABLE: set.table,
            RES_LABEL: set.source,
            RES_FLAGS: flags,
            RES_DEFAULT_MASK: set.overlay_mask,
            RES_EXCLUSION_MASK: set.exclusion_mask,
            RES_BITS_PRESENT: found.bits_present,
            RES_DQ_REF: found.dq_ref,
            RES_ERR_REF: found.err_ref,
            RES_SOURCE: set.source,
            RES_SATURATED_MASK: set.saturated_mask,
            RES_EXCLUSION_DEFAULT: set.exclusion_default,
            RES_EXCLUSION_EDITED: set.exclusion_mask != set.exclusion_default,
            RES_DQ_ORIGIN: found.dq_origin,
            RES_DQ_UNAVAILABLE: found.dq_unavailable,
            RES_ERR_ORIGIN: found.err_origin,
            RES_ERR_UNAVAILABLE: found.err_unavailable,
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
        let (Some((entry, table)), Some(flags)) = (loaded.dq.as_ref(), loaded.flags.as_ref()) else {
            bail!(NO_DQ_PLANE);
        };
        let plane = entry.int_plane().ok_or_else(|| anyhow::anyhow!(NO_DQ_PLANE))?;
        let applied = resolve_overlay_mask(flags, mask);
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
    symmetric: Option<bool>,
    centre: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let mode = LimitMode::from_parts(&algorithm, vmin, vmax, percentile, zscale_contrast)
            .map_err(|e| anyhow::anyhow!(e))?;
        scale_limits_json(&path, mode, symmetric.unwrap_or(false), centre)
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
            RES_NODATA: cmap.nodata_rgb().to_vec(),
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
            .zip(comps.flags.as_ref())
            .and_then(|((e, _), flags)| e.int_plane().map(|p| (p, flags)));
        let err_unit = comps.err.as_ref().and_then(|e| e.header().and_then(data_unit));
        let wave_unit = wavelength_unit(comps.wavelength.as_ref().and_then(|e| e.header()));
        let companions = probe_companions_named(
            dq,
            comps.err.as_ref().map(|e| e.arr()),
            comps.wavelength.as_ref().map(|e| e.arr()),
            probe.x,
            probe.y,
        );
        let body = probe_json_with_companions(
            &probe,
            unit.as_deref(),
            err_unit.as_deref(),
            Some(&wave_unit),
            &companions,
        );
        Ok(with_layer_sources(
            body,
            comps.err_origin.as_ref().map(|origin| origin.source.as_str()),
            comps.flags.as_ref().map(|flags| flags.source.as_str()),
        ))
    })
}

#[tauri::command]
pub async fn pixel_table_cmd(
    path: String,
    x: i64,
    y: i64,
    size: Option<usize>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let size = pixel_table_size(size)?;
        let entry = load_cached_full(&path)?;
        let values = pixel_grid(entry.arr(), x, y, size);
        let stats = grid_stats(entry.arr(), x, y, size);
        let unit = entry.header().and_then(data_unit);
        let (x0, y0) = grid_origin(x, y, size);
        let comps = load_companions(&path).unwrap_or_else(|e| {
            log::warn!("companion lookup failed for {}: {:#}", path, e);
            LoadedCompanions::default()
        });
        let err = comps.err.as_ref().map(|e| pixel_grid(e.arr(), x, y, size));
        let err_stats = comps
            .err
            .as_ref()
            .map(|e| grid_stats_json(&grid_stats(e.arr(), x, y, size)));
        let wavelength = comps.wavelength.as_ref().map(|e| pixel_grid(e.arr(), x, y, size));
        let wavelength_stats = comps
            .wavelength
            .as_ref()
            .map(|e| grid_stats_json(&grid_stats(e.arr(), x, y, size)));
        let wave_unit = comps.wavelength.as_ref().map(|e| wavelength_unit(e.header()));
        let dq = comps
            .dq
            .as_ref()
            .zip(comps.flags.as_ref())
            .and_then(|((e, _), flags)| e.int_plane().map(|plane| (plane, flags)));
        let (dq_values, dq_names, dq_table) = match dq {
            Some((plane, flags)) => (
                Some(int_value_grid(plane, x, y, size)),
                Some(dq_name_grid(&int_grid(&plane.bits, x, y, size), flags)),
                Some(flags.source.as_str()),
            ),
            None => (None, None, None),
        };
        Ok(json!({
            RES_X: x,
            RES_Y: y,
            RES_SIZE: size,
            RES_X0: x0,
            RES_Y0: y0,
            RES_VALUES: values,
            RES_ERR: err,
            RES_DQ: dq_values,
            RES_DQ_NAMES: dq_names,
            RES_DQ_TABLE: dq_table,
            RES_UNIT: unit,
            RES_STATS: grid_stats_json(&stats),
            RES_ERR_STATS: err_stats,
            RES_WAVELENGTH: wavelength,
            RES_WAVELENGTH_STATS: wavelength_stats,
            RES_WAVELENGTH_UNIT: wave_unit,
            RES_ERR_SOURCE: comps.err_origin.as_ref().map(|origin| origin.source.as_str()),
            RES_DQ_SOURCE: comps.flags.as_ref().map(|flags| flags.source.as_str()),
            RES_ERR_UNAVAILABLE: comps.err_unavailable,
            RES_DQ_UNAVAILABLE: comps.dq_unavailable,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
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
            let (r, g, b) = helpers::load_composite_rgb()?;
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
    use crate::cmd::common::test_support::{afw_mef, float_dq_mef};
    use crate::infra::fits::reader::test_fixtures::{
        sci_err_dq_mef, sci_err_dq_mef_with_dq_cards, sci_err_dq_wave_mef, sci_err_dq_wave_mef_with_wave_cards,
    };
    use crate::types::constants::RES_VALUE;

    #[test]
    fn the_default_overlay_is_the_one_of_the_flag_set() {
        let (jwst, hst, raw) =
            (FlagSet::from_table(DqTable::Jwst), FlagSet::from_table(DqTable::Hst), FlagSet::from_table(DqTable::Unknown));
        assert_eq!(resolve_overlay_mask(&jwst, None), 7);
        assert_eq!(resolve_overlay_mask(&hst, None), 4 | 256 | 4096 | 8192);
        assert_eq!(resolve_overlay_mask(&raw, None), 0);
        assert_eq!(resolve_overlay_mask(&raw, Some(5)), 5);
        assert_eq!(resolve_overlay_mask(&jwst, Some(0)), 0);
        assert_eq!(resolve_overlay_mask(&hst, Some(1 << 31)), 1 << 31);

        let dir = tempfile::tempdir().unwrap();
        let k = format!("{}#hdu=1", afw_mef(dir.path(), "calexp.fits", vec![]));
        let afw = load_companions(&k).unwrap().flags.expect("the MASK HDU is the flag layer of the IMAGE HDU");
        assert_eq!(resolve_overlay_mask(&afw, None), 3);
        assert_eq!(resolve_overlay_mask(&afw, Some(5)), 5);
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

        let found = dq_table_for(&format!("{}#hdu=1", p)).unwrap();
        assert_eq!(found.set.table, DqTable::Jwst);
        assert_eq!(found.dq_ref.as_deref(), Some(format!("{}#hdu=3", p).as_str()));
        assert_eq!(found.err_ref.as_deref(), Some(format!("{}#hdu=2", p).as_str()));
        assert_eq!(found.bits_present, None);

        let lonely = dq_table_for(&format!("{}#hdu=4", p)).unwrap();
        assert_eq!(lonely.set.table, DqTable::Unknown);
        assert!(lonely.dq_ref.is_none());
        assert_eq!(lonely.bits_present, None);

        let own = dq_table_for(&format!("{}#hdu=3", p)).unwrap();
        assert_eq!(own.set.table, DqTable::Jwst);
        assert_eq!(own.dq_ref.as_deref(), Some(format!("{}#hdu=3", p).as_str()));
    }

    #[tokio::test]
    async fn dq_flag_table_lists_raw_bits_for_an_unknown_mask_and_names_for_jwst() {
        use crate::infra::fits::reader::test_fixtures::{ramp_f32, write_test_mef, HduData, TestHdu};
        let dir = tempfile::tempdir().unwrap();
        let hsc = dir.path().join("hsc_mask.fits");
        let mut mask = vec![0i32; 16];
        mask[5] = 1;
        write_test_mef(
            &hsc,
            &[("TELESCOP", "'Subaru'".into())],
            &[
                TestHdu { extname: Some("SCI"), extver: None, cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![] },
                TestHdu { extname: Some("MASK"), extver: None, cols: 4, rows: 4, data: HduData::I32(mask), extra_cards: vec![] },
            ],
        );
        let k = format!("{}#hdu=2", hsc.to_str().unwrap());
        let unknown = get_dq_flag_table_cmd(k.clone()).await.unwrap();
        assert_eq!(unknown[RES_TABLE], "unknown");
        assert_eq!(unknown[RES_LABEL], "unknown (raw bits)");
        assert_eq!(unknown[RES_FLAGS], json!([]));
        assert_eq!(unknown[RES_BITS_PRESENT], 1);
        assert_eq!(unknown[RES_EXCLUSION_MASK], 0);
        assert_eq!(unknown[RES_DEFAULT_MASK], 0);
        assert_eq!(unknown[RES_DQ_REF], k);

        let jwst = dir.path().join("jwst_bits.fits");
        let raw = |bits: u32| (bits as i64 - 2147483648) as i32;
        let mut dq = vec![raw(0); 16];
        dq[0] = raw(1);
        dq[7] = raw(4);
        dq[15] = raw(1 << 31);
        sci_err_dq_mef_with_dq_cards(&jwst, 4, 4, dq, vec![("TELESCOP", "'JWST'".into())]);
        let p = jwst.to_str().unwrap();
        let named = get_dq_flag_table_cmd(format!("{p}#hdu=1")).await.unwrap();
        assert_eq!(named[RES_TABLE], "jwst");
        assert_eq!(named[RES_FLAGS].as_array().unwrap().len(), 32);
        assert!(named[RES_BITS_PRESENT].is_null(), "{named}");
        assert_eq!(named[RES_EXCLUSION_MASK], 1);
        let lonely = get_dq_flag_table_cmd(format!("{p}#hdu=4")).await.unwrap();
        assert!(lonely[RES_BITS_PRESENT].is_null(), "{lonely}");
        assert!(lonely[RES_DQ_REF].is_null());
    }

    #[tokio::test]
    async fn the_flag_table_of_an_unreadable_dq_has_no_bits_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let p = float_dq_mef(dir.path(), "float_dq.fits");

        let t = get_dq_flag_table_cmd(format!("{p}#hdu=1")).await.unwrap();
        assert!(t[RES_BITS_PRESENT].is_null(), "{t}");
        assert!(t[RES_DQ_REF].is_null(), "{t}");
        assert_eq!(
            t["dq_unavailable"],
            "mask layer HDU 2 is a floating-point plane (BITPIX -32): flag bits cannot be read exactly"
        );
    }

    #[tokio::test]
    async fn the_flag_table_of_an_afw_mask_lists_the_names_of_its_cards() {
        let dir = tempfile::tempdir().unwrap();
        let p = afw_mef(dir.path(), "calexp.fits", vec![]);

        let t = get_dq_flag_table_cmd(format!("{p}#hdu=1")).await.unwrap();
        assert_eq!(t[RES_TABLE], "unknown");
        assert_eq!(t[RES_LABEL], "MP_ cards (afw mask planes)");
        assert_eq!(t[RES_SOURCE], t[RES_LABEL]);
        assert_eq!(t[RES_FLAGS], json!([{"bit": 1, "name": "BAD"}, {"bit": 2, "name": "SAT"}, {"bit": 32, "name": "DETECTED"}]));
        assert_eq!(t[RES_EXCLUSION_MASK], 3);
        assert_eq!(t["exclusion_default"], 3);
        assert_eq!(t["exclusion_edited"], false);
        assert_eq!(t["saturated_mask"], 2);
        assert_eq!(t[RES_DEFAULT_MASK], 3);
        assert!(t[RES_BITS_PRESENT].is_null(), "{t}");
        assert_eq!(t[RES_DQ_REF], format!("{p}#hdu=2"));
        assert_eq!(t["dq_origin"], "EXTNAME MASK, HDU 2");
        assert!(t["dq_unavailable"].is_null(), "{t}");
        assert_eq!(
            t["err_origin"],
            json!({"role": "variance", "source": "EXTNAME VARIANCE, HDU 3: square root", "assumed": false, "n_no_data": 1, "origin": "same_file"})
        );
        assert!(t["err_unavailable"].is_null(), "{t}");
    }

    #[tokio::test]
    async fn the_hover_readout_names_the_flags_and_the_origin_of_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let k = format!("{}#hdu=1", afw_mef(dir.path(), "calexp.fits", vec![]));

        let j = probe_pixel_cmd(k.clone(), 1.0, 1.0, None).await.unwrap();
        assert_eq!(
            j[RES_DQ],
            json!({"bits": 34, "value": 34, "names": ["SAT", "DETECTED"], "table": "unknown", "text": "34: SAT | DETECTED", "source": "MP_ cards (afw mask planes)"})
        );
        assert_eq!(j[RES_ERR], json!({"value": 5.0, "unit": "nJy", "source": "EXTNAME VARIANCE, HDU 3: square root"}));

        let no_data = probe_pixel_cmd(k, 2.0, 1.0, None).await.unwrap();
        assert!(no_data[RES_ERR].is_null(), "{no_data}");
    }

    #[tokio::test]
    async fn the_pixel_table_names_the_layers() {
        let dir = tempfile::tempdir().unwrap();
        let k = format!("{}#hdu=1", afw_mef(dir.path(), "calexp.fits", vec![]));

        let t = pixel_table_cmd(k, 1, 1, Some(3)).await.unwrap();
        assert_eq!(t["err_source"], "EXTNAME VARIANCE, HDU 3: square root");
        assert_eq!(t["dq_source"], "MP_ cards (afw mask planes)");
        assert_eq!(t[RES_DQ_TABLE], "MP_ cards (afw mask planes)");
        assert_eq!(t[RES_DQ_NAMES][1][1], "SAT | DETECTED");
        assert_eq!(t[RES_ERR][1][1], 5.0);
        for key in ["err_unavailable", "dq_unavailable"] {
            assert!(t.as_object().unwrap().contains_key(key), "{key}: {t}");
            assert!(t[key].is_null(), "{key}: {t}");
        }
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
    async fn deep_zoom_tiles_of_a_cleared_composite_say_to_run_blend_again() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        helpers::clear_composite();
        let err = generate_tiles_rgb(out, 256, None).await.expect_err("the composite was cleared");
        assert_eq!(err, "The colour composite is no longer in memory; run Blend again.");
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
            let limits = compute_scale_limits_cmd(stretched.clone(), algorithm.into(), None, None, None, None, None, None).await.unwrap();
            assert_eq!((limits[RES_VMIN].as_f64(), limits[RES_VMAX].as_f64()), (Some(0.0), Some(1.0)), "{algorithm}");
            let own = compute_scale_limits_cmd(linear.clone(), algorithm.into(), None, None, None, None, None, None).await.unwrap();
            assert!(own[RES_VMIN].as_f64().unwrap() > 0.1, "{algorithm} on linear data: {own}");
        }
        let user = compute_scale_limits_cmd(stretched, "user".into(), Some(0.3), Some(0.4), None, None, None, None).await.unwrap();
        assert_eq!((user[RES_VMIN].as_f64(), user[RES_VMAX].as_f64()), (Some(0.3), Some(0.4)));
    }

    fn write_ramp_16x16(dir: &std::path::Path, name: &str, header: Option<&HduHeader>) -> String {
        let data = ndarray::Array2::from_shape_fn((16, 16), |(y, x)| 0.2 + (y * 16 + x) as f32 / 640.0);
        let path = dir.join(name).to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&path, &data, header).unwrap();
        path
    }

    fn display_referred_header() -> HduHeader {
        let mut flagged = HduHeader::empty();
        flagged.set(crate::cmd::common::HEADER_DISPLAY_REFERRED, "T".to_string());
        flagged
    }

    #[test]
    fn symmetric_limits_mirror_the_base_pair_about_the_centre() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_ramp_16x16(dir.path(), "sym_ramp.fits", None);

        let plain = scale_limits_json(&path, LimitMode::MinMax, false, None).unwrap();
        let hi = plain[RES_VMAX].as_f64().unwrap();
        assert_eq!(plain[RES_VMIN].as_f64().unwrap(), 0.2f32 as f64);
        assert_eq!(hi, (0.2f32 + 0.3984375f32) as f64);
        assert_eq!(plain[RES_SYMMETRIC], false);
        assert!(plain[RES_CENTRE].is_null());
        assert_eq!(plain[RES_NOTES], json!([]));

        let sym = scale_limits_json(&path, LimitMode::MinMax, true, None).unwrap();
        assert_eq!(sym[RES_VMIN].as_f64().unwrap(), -hi);
        assert_eq!(sym[RES_VMAX].as_f64().unwrap(), hi);
        assert_eq!(sym[RES_SYMMETRIC], true);
        assert_eq!(sym[RES_CENTRE], 0.0);
        assert_eq!(sym[RES_NOTES], json!([]));
        assert_eq!(sym[RES_ALGORITHM], "minmax");

        let shifted = scale_limits_json(&path, LimitMode::MinMax, true, Some(0.4)).unwrap();
        let lo = 0.2f32 as f64;
        assert!((shifted[RES_VMIN].as_f64().unwrap() - lo).abs() < 1e-12, "{shifted}");
        assert!((shifted[RES_VMAX].as_f64().unwrap() - (0.8 - lo)).abs() < 1e-12, "{shifted}");
        assert_eq!(shifted[RES_CENTRE], 0.4);
        assert_eq!(shifted[RES_SYMMETRIC], true);
    }

    #[test]
    fn user_mode_with_one_bound_symmetrises_the_filled_pair() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_ramp_16x16(dir.path(), "sym_user.fits", None);
        let hi = scale_limits_json(&path, LimitMode::MinMax, false, None).unwrap()[RES_VMAX].as_f64().unwrap();
        let mode = LimitMode::User { vmin: Some(-0.1), vmax: None };
        let sym = scale_limits_json(&path, mode, true, Some(0.0)).unwrap();
        assert_eq!(sym[RES_VMIN].as_f64().unwrap(), -hi);
        assert_eq!(sym[RES_VMAX].as_f64().unwrap(), hi);
        assert_eq!(sym[RES_ALGORITHM], "user");
        assert_eq!(sym[RES_SYMMETRIC], true);
    }

    #[test]
    fn symmetric_limits_are_skipped_on_a_display_referred_image_with_a_note() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_ramp_16x16(dir.path(), "sym_abdisp.fits", Some(&display_referred_header()));
        let auto = scale_limits_json(&path, LimitMode::MinMax, true, None).unwrap();
        assert_eq!((auto[RES_VMIN].as_f64(), auto[RES_VMAX].as_f64()), (Some(0.0), Some(1.0)));
        let user = LimitMode::User { vmin: Some(0.3), vmax: Some(0.4) };
        let own = scale_limits_json(&path, user, true, Some(0.0)).unwrap();
        assert_eq!((own[RES_VMIN].as_f64(), own[RES_VMAX].as_f64()), (Some(0.3), Some(0.4)));
        for reply in [auto, own] {
            assert_eq!(reply[RES_SYMMETRIC], false, "{reply}");
            assert!(reply[RES_CENTRE].is_null(), "{reply}");
            let notes = reply[RES_NOTES].as_array().unwrap();
            assert_eq!(notes.len(), 1, "{reply}");
            let note = notes[0].as_str().unwrap();
            assert!(note.contains("display-referred"), "{note}");
            assert!(!note.contains("0..1"), "{note}");
        }
    }

    #[test]
    fn symmetric_limits_on_a_constant_image_fall_back_with_a_note() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("flat3.fits").to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&small, &ndarray::Array2::from_elem((8, 8), 3.0f32), None).unwrap();
        let reply = scale_limits_json(&small, LimitMode::MinMax, true, Some(3.0)).unwrap();
        assert_eq!((reply[RES_VMIN].as_f64(), reply[RES_VMAX].as_f64()), (Some(2.0), Some(4.0)));
        assert_eq!(reply[RES_SYMMETRIC], true);
        assert_eq!(reply[RES_CENTRE], 3.0);
        assert!(reply[RES_NOTES][0].as_str().unwrap().contains("half-width"), "{reply}");

        let large = dir.path().join("flat3e7.fits").to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&large, &ndarray::Array2::from_elem((8, 8), 3e7f32), None).unwrap();
        let reply = scale_limits_json(&large, LimitMode::MinMax, true, Some(3e7)).unwrap();
        let (vmin, vmax) = (reply[RES_VMIN].as_f64().unwrap(), reply[RES_VMAX].as_f64().unwrap());
        assert!((vmin as f32) < (vmax as f32), "{reply}");
        assert!(((vmin + vmax) / 2.0 - 3e7).abs() < 1e-6, "{reply}");
        assert!(reply[RES_NOTES][0].as_str().unwrap().contains("half-width"), "{reply}");
    }

    #[test]
    fn a_non_finite_or_out_of_range_centre_is_refused_before_loading() {
        let err = scale_limits_json("missing.fits", LimitMode::MinMax, true, Some(f64::NAN)).unwrap_err().to_string();
        assert!(err.contains("centre must be finite"), "{err}");
        let err = scale_limits_json("missing.fits", LimitMode::MinMax, true, Some(1e308)).unwrap_err().to_string();
        assert!(err.contains("centre must be within"), "{err}");

        let dir = tempfile::tempdir().unwrap();
        let path = write_ramp_16x16(dir.path(), "sym_unused_centre.fits", None);
        let reply = scale_limits_json(&path, LimitMode::MinMax, false, Some(1e308)).unwrap();
        assert_eq!(reply[RES_SYMMETRIC], false);
        assert!(reply[RES_CENTRE].is_null());
        assert_eq!(reply[RES_NOTES], json!([]));
    }

    #[tokio::test]
    async fn get_colormap_lut_reports_the_no_data_colour() {
        let bwr = get_colormap_lut_cmd("bwr".into()).await.unwrap();
        assert_eq!(bwr[RES_NODATA], json!([64, 64, 64]));
        assert_eq!(bwr[RES_RGBA].as_array().unwrap().len(), 1024);
        assert_eq!(bwr[RES_COLORMAPS].as_array().unwrap().len(), 13);
        assert_eq!(bwr[RES_NAME], "bwr");
        let gray = get_colormap_lut_cmd("gray".into()).await.unwrap();
        assert_eq!(gray[RES_NODATA], json!([0, 0, 0]));
    }


    #[tokio::test]
    async fn pixel_table_reports_the_grid_the_companions_and_the_stats_with_null_padding_at_the_edge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("table.fits");
        let mut dq = vec![-2147483648i32; 16];
        dq[5] = -2147483645;
        sci_err_dq_mef_with_dq_cards(&path, 4, 4, dq, vec![("TELESCOP", "'JWST'".into())]);
        let key = format!("{}#hdu=1", path.to_str().unwrap());

        let t = pixel_table_cmd(key.clone(), 0, 1, Some(3)).await.unwrap();
        assert_eq!(t[RES_X], 0);
        assert_eq!(t[RES_Y], 1);
        assert_eq!(t[RES_SIZE], 3);
        assert_eq!(t[RES_X0], -1);
        assert_eq!(t[RES_Y0], 0);
        assert_eq!(t[RES_VALUES], json!([[null, 0.0, 1.0], [null, 4.0, 5.0], [null, 8.0, 9.0]]));
        assert_eq!(t[RES_ERR], json!([[null, 0.0, 0.5], [null, 2.0, 2.5], [null, 4.0, 4.5]]));
        assert_eq!(t[RES_DQ], json!([[null, 0, 0], [null, 0, 3], [null, 0, 0]]));
        assert_eq!(t[RES_DQ_NAMES], json!([[null, null, null], [null, null, "DO_NOT_USE | SATURATED"], [null, null, null]]));
        assert_eq!(t[RES_DQ_TABLE], "jwst");
        assert_eq!(t[RES_UNIT], "MJy/sr");
        assert_eq!(t[RES_STATS]["min"], 0.0);
        assert_eq!(t[RES_STATS]["max"], 9.0);
        assert_eq!(t[RES_STATS]["mean"], 4.5);
        assert_eq!(t[RES_STATS]["median"], 4.5);
        assert_eq!(t[RES_STATS]["n_finite"], 6);
        assert_eq!(t[RES_STATS]["n_nan"], 0);
        assert_eq!(
            t[RES_ERR_STATS],
            json!({ "min": 0.0, "max": 4.5, "mean": 2.25, "median": 2.25, "n_finite": 6, "n_nan": 0 })
        );
        assert!(t[RES_ELAPSED_MS].is_number());

        let far = pixel_table_cmd(key.clone(), 100, -100, None).await.unwrap();
        assert_eq!(far[RES_SIZE], 7);
        assert_eq!(far[RES_VALUES].as_array().unwrap().len(), 7);
        assert!(far[RES_VALUES].as_array().unwrap().iter().flat_map(|r| r.as_array().unwrap()).all(|c| c.is_null()));
        assert_eq!(far[RES_STATS]["n_finite"], 0);
        assert!(far[RES_STATS]["median"].is_null());

        let plain = dir.path().join("plain.fits").to_str().unwrap().to_string();
        let data = ndarray::Array2::from_shape_fn((5, 5), |(y, x)| (y * 5 + x) as f32);
        crate::infra::fits::writer::write_fits_mono(&plain, &data, None).unwrap();
        let p = pixel_table_cmd(plain, 2, 2, Some(3)).await.unwrap();
        assert_eq!(p[RES_VALUES][1][1], 12.0);
        assert!(p[RES_ERR].is_null());
        assert!(p[RES_ERR_STATS].is_null());
        assert!(p[RES_DQ].is_null());
        assert!(p[RES_DQ_NAMES].is_null());
        assert!(p[RES_DQ_TABLE].is_null());
        assert!(p[RES_UNIT].is_null());
    }

    #[tokio::test]
    async fn probe_pixel_reports_the_wavelength_companion_with_its_unit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wave_probe.fits");
        let mut wave = vec![2.0f32; 16];
        wave[5] = 1.654;
        wave[15] = f32::NAN;
        sci_err_dq_wave_mef(&path, 4, 4, vec![-2147483648i32; 16], wave);
        let key = format!("{}#hdu=1", path.to_str().unwrap());

        let j = probe_pixel_cmd(key.clone(), 1.0, 1.0, Some(1)).await.unwrap();
        assert!((j[RES_WAVELENGTH][RES_VALUE].as_f64().unwrap() - 1.654).abs() < 1e-6, "{j}");
        assert_eq!(j[RES_WAVELENGTH][RES_UNIT], "um");
        assert_eq!(j[RES_ERR][RES_VALUE], 2.5);
        assert_eq!(j[RES_VALUE], 5.0);

        let nan = probe_pixel_cmd(key, 3.0, 3.0, Some(1)).await.unwrap();
        assert!(nan.as_object().unwrap().contains_key(RES_WAVELENGTH), "{nan}");
        assert!(nan[RES_WAVELENGTH].is_null(), "{nan}");
        assert_eq!(nan[RES_VALUE], 15.0);

        let plain = dir.path().join("plain_probe.fits").to_str().unwrap().to_string();
        let data = ndarray::Array2::from_shape_fn((5, 5), |(y, x)| (y * 5 + x) as f32);
        crate::infra::fits::writer::write_fits_mono(&plain, &data, None).unwrap();
        let p = probe_pixel_cmd(plain, 2.0, 2.0, Some(1)).await.unwrap();
        assert_eq!(p[RES_VALUE], 12.0);
        assert!(p.as_object().unwrap().contains_key(RES_WAVELENGTH), "{p}");
        assert!(p[RES_WAVELENGTH].is_null(), "{p}");
    }

    #[tokio::test]
    async fn pixel_table_includes_the_wavelength_grid_stats_and_unit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wave_table.fits");
        let wave: Vec<f32> = (0..16).map(|i| 1.0 + 0.25 * i as f32).collect();
        sci_err_dq_wave_mef(&path, 4, 4, vec![-2147483648i32; 16], wave);
        let key = format!("{}#hdu=1", path.to_str().unwrap());

        let t = pixel_table_cmd(key, 0, 1, Some(3)).await.unwrap();
        assert_eq!(t[RES_WAVELENGTH], json!([[null, 1.0, 1.25], [null, 2.0, 2.25], [null, 3.0, 3.25]]));
        assert_eq!(
            t[RES_WAVELENGTH_STATS],
            json!({ "min": 1.0, "max": 3.25, "mean": 2.125, "median": 2.125, "n_finite": 6, "n_nan": 0 })
        );
        assert_eq!(t[RES_WAVELENGTH_UNIT], "um");
        assert_eq!(t[RES_VALUES][1][1], 4.0);
        assert_eq!(t[RES_ERR][1][2], 2.5);

        let plain = dir.path().join("plain_table.fits").to_str().unwrap().to_string();
        let data = ndarray::Array2::from_shape_fn((5, 5), |(y, x)| (y * 5 + x) as f32);
        crate::infra::fits::writer::write_fits_mono(&plain, &data, None).unwrap();
        let p = pixel_table_cmd(plain, 2, 2, Some(3)).await.unwrap();
        for key in [RES_WAVELENGTH, RES_WAVELENGTH_STATS, RES_WAVELENGTH_UNIT] {
            assert!(p.as_object().unwrap().contains_key(key), "{key}: {p}");
            assert!(p[key].is_null(), "{key}: {p}");
        }
    }

    #[tokio::test]
    async fn probe_pixel_reads_the_wavelength_unit_from_bunit_and_falls_back_to_um_without_it() {
        let dir = tempfile::tempdir().unwrap();
        let wave: Vec<f32> = (0..16).map(|i| 1.6 + 0.01 * i as f32).collect();
        let bare = dir.path().join("wave_nobunit.fits");
        sci_err_dq_wave_mef_with_wave_cards(&bare, 4, 4, vec![-2147483648i32; 16], wave.clone(), vec![]);
        let j = probe_pixel_cmd(format!("{}#hdu=1", bare.to_str().unwrap()), 2.0, 1.0, Some(1)).await.unwrap();
        assert!((j[RES_WAVELENGTH][RES_VALUE].as_f64().unwrap() - wave[6] as f64).abs() < 1e-6, "{j}");
        assert_eq!(j[RES_WAVELENGTH][RES_UNIT], "um", "{j}");

        let nm = dir.path().join("wave_nm.fits");
        sci_err_dq_wave_mef_with_wave_cards(&nm, 4, 4, vec![-2147483648i32; 16], wave, vec![("BUNIT", "'nm'".into())]);
        let j = probe_pixel_cmd(format!("{}#hdu=1", nm.to_str().unwrap()), 2.0, 1.0, Some(1)).await.unwrap();
        assert_eq!(j[RES_WAVELENGTH][RES_UNIT], "nm", "{j}");
    }

    #[tokio::test]
    async fn pixel_table_reads_the_wavelength_unit_from_bunit_and_falls_back_to_um_without_it() {
        let dir = tempfile::tempdir().unwrap();
        let wave: Vec<f32> = (0..16).map(|i| 1.6 + 0.01 * i as f32).collect();
        let bare = dir.path().join("table_nobunit.fits");
        sci_err_dq_wave_mef_with_wave_cards(&bare, 4, 4, vec![-2147483648i32; 16], wave.clone(), vec![]);
        let t = pixel_table_cmd(format!("{}#hdu=1", bare.to_str().unwrap()), 1, 1, Some(3)).await.unwrap();
        assert_eq!(t[RES_WAVELENGTH_UNIT], "um", "{t}");
        assert!((t[RES_WAVELENGTH][1][1].as_f64().unwrap() - wave[5] as f64).abs() < 1e-6, "{t}");
        assert_eq!(t[RES_WAVELENGTH_STATS]["n_finite"], 9, "{t}");

        let nm = dir.path().join("table_nm.fits");
        sci_err_dq_wave_mef_with_wave_cards(&nm, 4, 4, vec![-2147483648i32; 16], wave, vec![("BUNIT", "'nm'".into())]);
        let t = pixel_table_cmd(format!("{}#hdu=1", nm.to_str().unwrap()), 1, 1, Some(3)).await.unwrap();
        assert_eq!(t[RES_WAVELENGTH_UNIT], "nm", "{t}");
    }

    #[tokio::test]
    async fn pixel_table_err_stats_come_from_the_err_plane_including_its_own_nan_count() {
        use crate::infra::fits::reader::test_fixtures::{
            ramp_f32, write_test_mef, HduData, TestHdu,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("err_nan.fits");
        let mut err = ramp_f32(4, 4).iter().map(|v| v * 0.5).collect::<Vec<f32>>();
        err[5] = f32::NAN;
        write_test_mef(
            &path,
            &[],
            &[
                TestHdu {
                    extname: Some("SCI"),
                    extver: Some(1),
                    cols: 4,
                    rows: 4,
                    data: HduData::F32(ramp_f32(4, 4)),
                    extra_cards: vec![],
                },
                TestHdu {
                    extname: Some("ERR"),
                    extver: Some(1),
                    cols: 4,
                    rows: 4,
                    data: HduData::F32(err),
                    extra_cards: vec![],
                },
            ],
        );
        let key = format!("{}#hdu=1", path.to_str().unwrap());

        let t = pixel_table_cmd(key, 0, 1, Some(3)).await.unwrap();
        assert_eq!(
            t[RES_ERR],
            json!([[null, 0.0, 0.5], [null, 2.0, null], [null, 4.0, 4.5]])
        );
        assert_eq!(
            t[RES_ERR_STATS],
            json!({ "min": 0.0, "max": 4.5, "mean": 2.2, "median": 2.0, "n_finite": 5, "n_nan": 1 })
        );
        assert_eq!(t[RES_STATS]["max"], 9.0);
        assert_eq!(t[RES_STATS]["n_finite"], 6);
        assert_eq!(t[RES_STATS]["n_nan"], 0);
    }

    #[tokio::test]
    async fn pixel_table_refuses_even_and_out_of_range_sizes_before_loading_the_image() {
        let missing = "/nonexistent/pixel_table.fits".to_string();
        for bad in [4usize, 17, 1, 0, 2, 16] {
            let err = pixel_table_cmd(missing.clone(), 0, 0, Some(bad)).await.unwrap_err();
            assert!(err.contains("size must be an odd number between 3 and 15"), "{bad}: {err}");
            assert!(err.contains(&format!("got {bad}")), "{bad}: {err}");
        }
        assert_eq!(pixel_table_size(None).unwrap(), 7);
        assert_eq!(pixel_table_size(Some(15)).unwrap(), 15);
        assert_eq!(pixel_table_size(Some(3)).unwrap(), 3);
    }

    #[test]
    fn dq_name_grid_decodes_set_bits_and_leaves_good_and_off_image_cells_null() {
        let bits = vec![vec![None, Some(0), Some(1)], vec![Some(6), Some(1 << 20), None]];
        let names = dq_name_grid(&bits, &FlagSet::from_table(DqTable::Jwst));
        assert_eq!(names[0], vec![None, None, Some("DO_NOT_USE".to_string())]);
        assert_eq!(names[1][0].as_deref(), Some("SATURATED | JUMP_DET"));
        assert!(names[1][1].is_some());
        assert_eq!(names[1][2], None);
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
        let flags = loaded.flags.clone().unwrap();
        let (entry, table) = loaded.dq.unwrap();
        assert_eq!(table, DqTable::Unknown);
        let plane = entry.int_plane().unwrap();
        let applied = resolve_overlay_mask(&flags, None);
        assert_eq!(applied, 0);
        assert!(mask_preview_or(&plane.bits, applied, 2).0.iter().all(|&c| c == 0));
        let applied = resolve_overlay_mask(&flags, Some(1));
        let (cells, w, h) = mask_preview_or(&plane.bits, applied, 2);
        let bytes = encode_mask_with_header(&cells, w as u32, h as u32, applied, table.id());
        assert_eq!(bytes.len(), 16 + 4);
        assert_eq!(u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]), 1);
        assert_eq!(u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]), 0xFFFF_FFFF);
        assert_eq!(&bytes[16..], &[0, 1, 0, 0]);
    }

    fn survey_file(set: &str, name: &str) -> String {
        let root = crate::core::astrometry::gwcs::test_support::env_dir("ASTROBURST_SCIENCE_DATA_DIR");
        root.join("survey").join(set).join(name).to_str().unwrap().to_string()
    }

    #[tokio::test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1 science, RMS and FLAG files of tile 102044185"]
    async fn real_euclid_open_response_and_flag_table_name_the_paired_files() {
        let rms = "EUC_MER_MOSAIC-VIS-RMS_TILE102044185-8BF2FC_20241021T010718.207983Z_00.00_cut60_A.fits";
        let flag = "EUC_MER_MOSAIC-VIS-FLAG_TILE102044185-A61C42_20241021T010718.208008Z_00.00_cut60_A.fits";
        let p = survey_file("E1", "EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits");

        let plane = crate::cmd::common::plane_info_json(&p, &load_cached_full(&p).unwrap()).unwrap();
        println!("euclid open response: err_origin {}, dq_origin {}, notes {}", plane["err_origin"], plane["dq_origin"], plane["pairing_notes"]);
        assert_eq!(plane["err_origin"]["source"], format!("file name -RMS, {rms}: as is"));
        assert_eq!(plane["err_origin"]["origin"], "sibling");
        assert_eq!(
            plane["pairing_notes"],
            json!([
                format!("error layer taken from the sibling file {rms} (name rule Euclid MER mosaic, same 601x601 grid)"),
                format!("mask layer taken from the sibling file {flag} (name rule Euclid MER mosaic, same 601x601 grid)"),
                "flag layer without bit names: nothing is excluded until bits are chosen",
            ])
        );

        let t = get_dq_flag_table_cmd(p.clone()).await.unwrap();
        println!("euclid flag table: label {}, bits_present {}, exclusion {}, err n_no_data {}", t[RES_LABEL], t[RES_BITS_PRESENT], t[RES_EXCLUSION_MASK], t["err_origin"]["n_no_data"]);
        assert_eq!(t[RES_LABEL], "unknown (raw bits)");
        assert_eq!(t[RES_FLAGS], json!([]));
        assert_eq!(t[RES_BITS_PRESENT], 25301342);
        assert_eq!(t[RES_EXCLUSION_MASK], 0);
        assert_eq!(t["err_origin"]["n_no_data"], 22);

        let sentinel = probe_pixel_cmd(p.clone(), 541.0, 70.0, None).await.unwrap();
        let corner = probe_pixel_cmd(p, 0.0, 0.0, None).await.unwrap();
        println!("euclid probe: (541, 70) value {} err {}; (0, 0) err {}", sentinel[RES_VALUE], sentinel[RES_ERR], corner[RES_ERR]);
        assert!(sentinel[RES_ERR].is_null(), "{sentinel}");
        assert_eq!(sentinel[RES_VALUE], 0.0);
        assert_eq!(corner[RES_ERR][RES_VALUE], 0.004187881480902433_f32 as f64);
        assert!(corner[RES_ERR][RES_UNIT].is_null(), "{corner}");
    }

    #[tokio::test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-image-r.fits.fz with its invvar and maskbits files"]
    async fn real_legacy_brick_flag_table_has_the_names_of_the_file() {
        let p = survey_file("L2", "legacysurvey-0532m280-image-r.fits.fz");

        let t = get_dq_flag_table_cmd(p.clone()).await.unwrap();
        println!("legacy flag table: label {}, {} flags, exclusion {}, saturated {}, bits_present {}", t[RES_LABEL], t[RES_FLAGS].as_array().map_or(0, Vec::len), t[RES_EXCLUSION_MASK], t["saturated_mask"], t[RES_BITS_PRESENT]);
        assert_eq!(t[RES_LABEL], "MBIT_ cards (Legacy maskbits)");
        assert_eq!(t[RES_FLAGS].as_array().unwrap().len(), 17);
        assert_eq!(t[RES_EXCLUSION_MASK], 72);
        assert_eq!(t["saturated_mask"], 8);
        assert!(t[RES_BITS_PRESENT].is_null(), "{t}");

        let star = probe_pixel_cmd(p.clone(), 2972.0, 893.0, None).await.unwrap();
        let corner = probe_pixel_cmd(p, 0.0, 0.0, None).await.unwrap();
        println!("legacy probe: (2972, 893) dq {}; (0, 0) err {}", star[RES_DQ], corner[RES_ERR]);
        assert_eq!(star[RES_DQ]["bits"], 51352);
        assert_eq!(star[RES_DQ]["names"], json!(["SATUR_R", "SATUR_Z", "ALLMASK_Z", "MEDIUM", "SATUR_I", "ALLMASK_I"]));
        let expected = 7.175998180759543e-4;
        let sigma = corner[RES_ERR][RES_VALUE].as_f64().unwrap();
        assert!((sigma - expected).abs() <= 1e-6 * expected, "{sigma} against {expected}");
        assert_eq!(corner[RES_ERR][RES_UNIT], "nanomaggy");
    }

    #[tokio::test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_r_mask_480px.fits"]
    async fn real_ps1_mask_cutout_shown_on_its_own_says_it_holds_no_mask() {
        let p = survey_file("P1", "ps1_A_r_mask_480px.fits");

        let t = get_dq_flag_table_cmd(p).await.unwrap();
        println!("ps1 mask flag table: label {}, {} flags, dq_ref {}, bits_present {}, dq_unavailable {}", t[RES_LABEL], t[RES_FLAGS].as_array().map_or(0, Vec::len), t[RES_DQ_REF], t[RES_BITS_PRESENT], t["dq_unavailable"]);
        assert!(t[RES_DQ_REF].is_null(), "{t}");
        assert!(t[RES_BITS_PRESENT].is_null(), "{t}");
        assert!(t["dq_unavailable"].as_str().unwrap().contains("floating-point plane (BITPIX -32)"), "{t}");
        assert_eq!(t[RES_LABEL], "MSKNAM cards (PS1 mask)");
        assert_eq!(t[RES_FLAGS].as_array().unwrap().len(), 16);
    }
}
