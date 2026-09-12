// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::infra::cache::{ImageCache, ImageEntry, PlaneLoad};
use astroburst_lib::infra::image_source::{load_plane, LoadedPlane, PlaneInfo};
use astroburst_lib::types::image_ref::{ImageRef, PlaneSelector};
use astroburst_lib::types::ImageStats;

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::{ImageMeta, Session};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct OpenParams {
    pub path: String,
    pub hdu: Option<usize>,
    pub array: Option<String>,
    pub name: Option<String>,
}

#[derive(Deserialize)]
pub struct HduParams {
    pub hdu: Option<usize>,
    pub array: Option<String>,
    pub name: Option<String>,
}

pub(crate) fn load_ref(r: &ImageRef) -> anyhow::Result<PlaneLoad> {
    load_plane(r).map(LoadedPlane::into_plane_load)
}

pub(crate) fn load_replacing<F>(
    cache: &ImageCache,
    image_ref: &str,
    loader: F,
) -> anyhow::Result<ImageEntry>
where
    F: FnOnce() -> anyhow::Result<PlaneLoad>,
{
    let loaded = loader()?;
    cache.invalidate(image_ref);
    cache.get_or_load_plane(image_ref, || Ok(loaded))
}

fn stats_json(s: &ImageStats) -> Value {
    json!({
        "min": s.min, "max": s.max, "median": s.median,
        "mad": s.mad, "sigma": s.sigma, "mean": s.mean,
        "valid_count": s.valid_count,
    })
}

fn selector_parts(kind: &PlaneSelector) -> (Option<usize>, Option<String>) {
    match kind {
        PlaneSelector::Hdu(n) => (Some(*n), None),
        PlaneSelector::Array(k) => (None, Some(k.clone())),
        PlaneSelector::Auto => (None, None),
    }
}

pub(crate) fn plane_selection(hdu: Option<usize>, array: Option<String>) -> Result<Option<PlaneSelector>> {
    match (hdu, array) {
        (Some(_), Some(_)) => Err(AppError::BadRequest("provide exactly one of hdu or array".into())),
        (Some(n), None) => Ok(Some(PlaneSelector::Hdu(n))),
        (None, Some(k)) => Ok(Some(PlaneSelector::Array(k))),
        (None, None) => Ok(None),
    }
}

pub(crate) fn plane_load_error(e: anyhow::Error) -> AppError {
    let text = format!("{:#}", e);
    if text.contains("no ASDF arrays") || text.contains("no HDU index") {
        AppError::BadRequest(text)
    } else {
        AppError::Internal(e)
    }
}

pub(crate) fn register_and_respond(
    session: &Session,
    image_ref: String,
    source: Option<String>,
    hdu: Option<usize>,
    entry: &ImageEntry,
) -> Value {
    register_plane_and_respond(session, image_ref, source, hdu, None, entry)
}

pub(crate) fn register_plane_and_respond(
    session: &Session,
    image_ref: String,
    source: Option<String>,
    hdu: Option<usize>,
    plane: Option<(&ImageRef, &PlaneInfo)>,
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

    let (hdu, array, plane_ref, is_dq) = match plane {
        Some((r, info)) => {
            let (h, a) = selector_parts(&info.kind);
            let resolved = match &info.kind {
                PlaneSelector::Hdu(n) => ImageRef::hdu(&r.path, *n),
                PlaneSelector::Array(k) => ImageRef::array(&r.path, k),
                PlaneSelector::Auto => r.clone(),
            };
            (h, a, resolved.cache_key(), info.is_dq)
        }
        None => (hdu, None, image_ref.clone(), false),
    };

    let meta = ImageMeta {
        image_ref: image_ref.clone(),
        source,
        hdu,
        array: array.clone(),
        plane_ref: plane_ref.clone(),
        is_dq,
        width: cols,
        height: rows,
        wcs_present,
        extname: extname.clone(),
    };
    session.v2.meta.insert(image_ref.clone(), meta);
    session.prune_evicted_meta();

    json!({
        "ref": image_ref,
        "active_ref": image_ref,
        "dims": [cols, rows],
        "hdu": hdu,
        "array": array,
        "plane_ref": plane_ref,
        "is_dq": is_dq,
        "extname": extname,
        "wcs_present": wcs_present,
        "stats": stats_json(stats),
        "header": header_map,
    })
}

async fn open_ref(session: &Session, image_ref: String, source: String, r: ImageRef) -> Result<Json<Value>> {
    let sess = session.cache.clone();
    let ref_for_load = image_ref.clone();
    let r_for_load = r.clone();
    let entry = tokio::task::spawn_blocking(move || {
        load_replacing(&sess, &ref_for_load, || load_ref(&r_for_load))
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    .map_err(plane_load_error)?;

    let info = entry
        .plane_info()
        .cloned()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("plane info missing after load")))?;
    let body = register_plane_and_respond(session, image_ref.clone(), Some(source), None, Some((&r, &info)), &entry);
    *session.v2.active_ref.write().await = Some(image_ref);
    Ok(Json(body))
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
    let plane = plane_selection(params.hdu, params.array)?.unwrap_or(PlaneSelector::Auto);
    let r = ImageRef { path: params.path.clone(), plane };
    open_ref(&session, image_ref, params.path, r).await
}

pub async fn switch_hdu(
    SessionExtractor(session): SessionExtractor,
    State(_state): State<AppState>,
    Json(params): Json<HduParams>,
) -> Result<Json<Value>> {
    let plane = plane_selection(params.hdu, params.array)?
        .ok_or_else(|| AppError::BadRequest("provide exactly one of hdu or array".into()))?;
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
    let r = ImageRef { path: source.clone(), plane };
    open_ref(&session, image_ref, source, r).await
}

pub async fn list_images(
    SessionExtractor(session): SessionExtractor,
) -> Result<Json<Value>> {
    let active = session.reconcile_active_ref().await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use astroburst_lib::core::imaging::stats::compute_image_stats;
    use astroburst_lib::types::header::HduHeader;
    use ndarray::Array2;

    fn image(rows: usize, cols: usize, fill: f32) -> PlaneLoad {
        let arr = Array2::from_elem((rows, cols), fill);
        let stats = compute_image_stats(&arr);
        PlaneLoad::synthetic(arr, stats, HduHeader::empty())
    }

    #[test]
    fn load_replacing_replaces_an_existing_ref_instead_of_returning_the_stale_entry() {
        let cache = ImageCache::new(4, usize::MAX);

        let first = load_replacing(&cache, "x", || Ok(image(8, 8, 1.0))).unwrap();
        assert_eq!(first.arr().dim(), (8, 8));

        let stale = cache.get_or_load_plane("x", || Ok(image(6, 6, 2.0))).unwrap();
        assert_eq!(stale.arr().dim(), (8, 8));

        let second = load_replacing(&cache, "x", || Ok(image(6, 6, 2.0))).unwrap();
        assert_eq!(second.arr().dim(), (6, 6));
        assert_eq!(second.arr()[[0, 0]], 2.0);
        assert_eq!(cache.get("x").unwrap().arr().dim(), (6, 6));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn load_replacing_keeps_the_old_entry_when_the_new_load_fails() {
        let cache = ImageCache::new(4, usize::MAX);
        load_replacing(&cache, "x", || Ok(image(8, 8, 1.0))).unwrap();

        let err = load_replacing(&cache, "x", || -> anyhow::Result<PlaneLoad> { anyhow::bail!("no such file") })
            .err()
            .expect("failed load must surface the loader error");
        assert!(err.to_string().contains("no such file"));
        assert_eq!(cache.get("x").unwrap().arr().dim(), (8, 8));
    }
}
