// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::fs::File;

use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::core::imaging::stats::compute_image_stats;
use astroburst_lib::infra::asdf::converter::is_asdf_file;
use astroburst_lib::infra::asdf_bridge::extract_image_from_asdf;
use astroburst_lib::infra::cache::ImageEntry;
use astroburst_lib::infra::fits::dispatcher::resolve_single_image;
use astroburst_lib::infra::fits::reader::{extract_image_mmap, extract_image_mmap_by_index};
use astroburst_lib::types::header::HduHeader;
use astroburst_lib::types::ImageStats;
use ndarray::Array2;

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::{ImageMeta, Session};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct OpenParams {
    pub path: String,
    pub hdu: Option<usize>,
    pub name: Option<String>,
}

#[derive(Deserialize)]
pub struct HduParams {
    pub hdu: usize,
    pub name: Option<String>,
}

fn load_auto(path: &str) -> anyhow::Result<(Array2<f32>, ImageStats, HduHeader)> {
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

fn load_by_index(path: &str, hdu: usize) -> anyhow::Result<(Array2<f32>, ImageStats, HduHeader)> {
    let p = std::path::Path::new(path);
    if is_asdf_file(p) {
        anyhow::bail!("ASDF files have no addressable HDU index; open without `hdu`");
    }
    let (fits_path, _tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)?;
    let r = extract_image_mmap_by_index(&file, hdu)?;
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

pub(crate) fn register_and_respond(
    session: &Session,
    image_ref: String,
    source: Option<String>,
    hdu: Option<usize>,
    entry: &ImageEntry,
) -> Value {
    let (rows, cols) = entry.arr().dim();
    let stats = entry.stats();
    let header = entry.header();

    let wcs_present = header
        .map(|h| WcsTransform::from_header(h).is_ok())
        .unwrap_or(false);
    let extname = header.and_then(|h| h.get("EXTNAME").map(|s| s.to_string()));
    let header_map: Value = header
        .map(|h| serde_json::to_value(&h.index).unwrap_or(json!(null)))
        .unwrap_or(json!(null));

    let meta = ImageMeta {
        image_ref: image_ref.clone(),
        source,
        hdu,
        width: cols,
        height: rows,
        wcs_present,
        extname: extname.clone(),
    };
    session.v2.meta.insert(image_ref.clone(), meta);

    json!({
        "ref": image_ref,
        "active_ref": image_ref,
        "dims": [cols, rows],
        "hdu": hdu,
        "extname": extname,
        "wcs_present": wcs_present,
        "stats": stats_json(stats),
        "header": header_map,
    })
}

pub async fn open(
    SessionExtractor(session): SessionExtractor,
    State(_state): State<AppState>,
    Json(params): Json<OpenParams>,
) -> Result<Json<Value>> {
    let image_ref = params
        .name
        .clone()
        .unwrap_or_else(|| session.v2.next_ref("img"));
    let path = params.path.clone();
    let hdu = params.hdu;

    let sess = session.clone();
    let ref_for_load = image_ref.clone();
    let entry = tokio::task::spawn_blocking(move || {
        sess.cache.get_or_load_full(&ref_for_load, || match hdu {
            Some(i) => load_by_index(&path, i),
            None => load_auto(&path),
        })
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    .map_err(AppError::Internal)?;

    let body = register_and_respond(&session, image_ref.clone(), Some(params.path), hdu, &entry);
    *session.v2.active_ref.write().await = Some(image_ref);
    Ok(Json(body))
}

pub async fn switch_hdu(
    SessionExtractor(session): SessionExtractor,
    State(_state): State<AppState>,
    Json(params): Json<HduParams>,
) -> Result<Json<Value>> {
    let active = session.v2.active_ref.read().await.clone();
    let active = active.ok_or_else(|| {
        AppError::BadRequest("no active image in this session; open a file first".into())
    })?;
    let source = session
        .v2
        .meta
        .get(&active)
        .and_then(|m| m.source.clone())
        .ok_or_else(|| {
            AppError::BadRequest(format!("active ref {active} has no source file to re-open"))
        })?;

    let image_ref = params
        .name
        .clone()
        .unwrap_or_else(|| session.v2.next_ref("img"));
    let hdu = params.hdu;
    let path = source.clone();

    let sess = session.clone();
    let ref_for_load = image_ref.clone();
    let entry = tokio::task::spawn_blocking(move || {
        sess.cache
            .get_or_load_full(&ref_for_load, || load_by_index(&path, hdu))
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    .map_err(AppError::Internal)?;

    let body = register_and_respond(&session, image_ref.clone(), Some(source), Some(hdu), &entry);
    *session.v2.active_ref.write().await = Some(image_ref);
    Ok(Json(body))
}

pub async fn list_images(
    SessionExtractor(session): SessionExtractor,
) -> Result<Json<Value>> {
    let active = session.v2.active_ref.read().await.clone();
    let mut images: Vec<ImageMeta> = session
        .v2
        .meta
        .iter()
        .map(|e| e.value().clone())
        .collect();
    images.sort_by(|a, b| a.image_ref.cmp(&b.image_ref));

    Ok(Json(json!({
        "active_ref": active,
        "count": images.len(),
        "images": images,
    })))
}
