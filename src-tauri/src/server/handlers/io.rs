// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::fs::File;

use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::imaging::stats::compute_image_stats;
use astroburst_lib::core::imaging::stf::auto_stf;
use astroburst_lib::types::image::AutoStfConfig;
use astroburst_lib::infra::asdf::converter::is_asdf_file;
use astroburst_lib::infra::asdf_bridge::extract_image_from_asdf;
use astroburst_lib::infra::cache::PlaneLoad;
use astroburst_lib::infra::fits::dispatcher::resolve_single_image;
use astroburst_lib::infra::fits::reader::extract_image_mmap;

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::state::AppState;
use crate::v2::images::{finite_stats_json, load_replacing, plane_load_error};

#[derive(Deserialize)]
pub struct OpenParams {
    pub path: String,
    pub slot: Option<String>,
}

#[derive(Deserialize)]
pub struct SlotParams {
    pub slot: String,
}

fn load_image_stats_header(path: &str) -> anyhow::Result<PlaneLoad> {
    let p = std::path::Path::new(path);
    if is_asdf_file(p) {
        let r = extract_image_from_asdf(p)?;
        let stats = compute_image_stats(&r.image);
        return Ok(PlaneLoad::synthetic(r.image, stats, r.header));
    }
    let (fits_path, _tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)?;
    let r = extract_image_mmap(&file)?;
    let stats = compute_image_stats(&r.image);
    Ok(PlaneLoad::synthetic(r.image, stats, r.header))
}

pub async fn fits_open(
    SessionExtractor(session): SessionExtractor,
    State(_state): State<AppState>,
    Json(params): Json<OpenParams>,
) -> Result<Json<Value>> {
    let path = params.path.clone();
    let slot = params.slot.unwrap_or_else(|| path.clone());
    let slot2 = slot.clone();
    let cache = session.cache.clone();

    let (entry, response_stats) = tokio::task::spawn_blocking(move || {
        load_replacing(&cache, &slot2, || load_image_stats_header(&path)).map(|entry| {
            let stats = finite_stats_json(entry.arr());
            (entry, stats)
        })
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    .map_err(|e| plane_load_error(e, &params.path))?;

    let (rows, cols) = entry.arr().dim();
    let stf = auto_stf(entry.stats(), &AutoStfConfig::default());

    let header_map: Value = entry
        .header()
        .map(|h| serde_json::to_value(&h.index).unwrap_or(json!(null)))
        .unwrap_or(json!(null));

    Ok(Json(json!({
        "slot": slot,
        "dims": [cols, rows],
        "stats": response_stats,
        "stf": { "shadow": stf.shadow, "midtone": stf.midtone, "highlight": stf.highlight },
        "header": header_map,
    })))
}

pub async fn fits_header(
    SessionExtractor(session): SessionExtractor,
    State(_state): State<AppState>,
    Json(params): Json<SlotParams>,
) -> Result<Json<Value>> {
    let slot = params.slot.clone();

    let entry = session
        .cache
        .get(&slot)
        .ok_or_else(|| AppError::NotFound(format!("slot {slot} not in cache")))?;

    let header = entry
        .header()
        .ok_or_else(|| AppError::NotFound(format!("slot {slot} has no header; re-open with fits/open")))?;

    let cards: Vec<Value> = header
        .cards
        .iter()
        .map(|(k, v)| json!({"key": k, "value": v}))
        .collect();

    let index_map = serde_json::to_value(&header.index).unwrap_or(json!(null));

    Ok(Json(json!({
        "slot": slot,
        "total_cards": header.cards.len(),
        "cards": cards,
        "index": index_map,
    })))
}
