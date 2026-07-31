// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::fs::File;
use std::sync::Arc;

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use astroburst_lib::core::imaging::stats::compute_image_stats;
use astroburst_lib::core::imaging::stf::auto_stf;
use astroburst_lib::types::image::AutoStfConfig;
use astroburst_lib::infra::asdf::converter::is_asdf_file;
use astroburst_lib::infra::asdf_bridge::extract_image_from_asdf;
use astroburst_lib::infra::fits::dispatcher::resolve_single_image;
use astroburst_lib::infra::fits::reader::extract_image_mmap;
use astroburst_lib::types::header::HduHeader;
use astroburst_lib::types::ImageStats;
use ndarray::Array2;

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct OpenParams {
    pub path: String,
    pub slot: Option<String>,
}

#[derive(Deserialize)]
pub struct SlotParams {
    pub slot: String,
}

fn load_image_and_stats(path: &str) -> anyhow::Result<(Array2<f32>, ImageStats)> {
    let p = std::path::Path::new(path);
    if is_asdf_file(p) {
        let r = extract_image_from_asdf(p)?;
        let stats = compute_image_stats(&r.image);
        return Ok((r.image, stats));
    }
    let (fits_path, _tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)?;
    let r = extract_image_mmap(&file)?;
    let stats = compute_image_stats(&r.image);
    Ok((r.image, stats))
}

fn load_image_stats_header(path: &str) -> anyhow::Result<(Array2<f32>, ImageStats, HduHeader)> {
    let p = std::path::Path::new(path);
    if is_asdf_file(p) {
        let r = extract_image_from_asdf(p)?;
        let stats = compute_image_stats(&r.image);
        return Ok((r.image, stats, r.header));
    }
    let (fits_path, _tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)?;
    let r = extract_image_mmap(&file)?;
    let stats = compute_image_stats(&r.image);
    Ok((r.image, stats, r.header))
}

fn stats_json(s: &ImageStats) -> Value {
    json!({
        "min": s.min, "max": s.max, "median": s.median,
        "mad": s.mad, "sigma": s.sigma, "mean": s.mean,
        "valid_count": s.valid_count,
    })
}

pub async fn fits_open(
    SessionExtractor(session): SessionExtractor,
    State(_state): State<AppState>,
    Json(params): Json<OpenParams>,
) -> Result<Json<Value>> {
    let path = params.path.clone();
    let slot = params.slot.unwrap_or_else(|| path.clone());
    let slot2 = slot.clone();

    let entry = tokio::task::spawn_blocking(move || {
        session.cache.get_or_load_full(&slot2, || load_image_stats_header(&path))
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    .map_err(AppError::Internal)?;

    let (rows, cols) = entry.arr().dim();
    let stats = entry.stats();
    let stf = auto_stf(stats, &AutoStfConfig::default());

    let header_map: Value = entry
        .header()
        .map(|h| serde_json::to_value(&h.index).unwrap_or(json!(null)))
        .unwrap_or(json!(null));

    Ok(Json(json!({
        "slot": slot,
        "dims": [cols, rows],
        "stats": stats_json(stats),
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
