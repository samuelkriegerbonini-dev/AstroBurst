use std::time::Instant;

use anyhow::bail;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_cached_full, load_companions, LoadedCompanions};
use crate::core::imaging::dq_flags::FlagSet;
use crate::core::imaging::pixel_probe::{
    data_unit, grid_origin, grid_stats, grid_stats_json, int_grid, int_value_grid, pixel_grid,
    probe_companions_named, probe_json_with_companions, probe_pixel, wavelength_unit, with_layer_sources,
};
use crate::types::constants::{
    DEFAULT_PROBE_BOX, RES_DQ, RES_DQ_NAMES, RES_DQ_TABLE, RES_ELAPSED_MS, RES_ERR, RES_ERR_STATS, RES_SIZE,
    RES_STATS, RES_UNIT, RES_VALUES, RES_WAVELENGTH, RES_WAVELENGTH_STATS, RES_WAVELENGTH_UNIT, RES_X, RES_X0,
    RES_Y, RES_Y0,
};

use super::dq_flags::{RES_DQ_SOURCE, RES_DQ_UNAVAILABLE, RES_ERR_SOURCE, RES_ERR_UNAVAILABLE};

pub(crate) const MIN_PIXEL_TABLE_SIZE: usize = 3;
pub(crate) const MAX_PIXEL_TABLE_SIZE: usize = 15;
const DEFAULT_PIXEL_TABLE_SIZE: usize = 7;

pub(super) fn pixel_table_size(size: Option<usize>) -> anyhow::Result<usize> {
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

pub(super) fn dq_name_grid(bits: &[Vec<Option<u32>>], flags: &FlagSet) -> Vec<Vec<Option<String>>> {
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
