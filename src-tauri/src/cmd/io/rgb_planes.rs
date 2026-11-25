use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, source_path, try_extract_rgb_resolved};
use crate::cmd::helpers;
use crate::core::imaging::stats::compute_image_stats;
use crate::types::constants::RES_DIMENSIONS;

type RgbStamp = (u64, Option<std::time::SystemTime>);

pub(crate) type RgbPlanes = (Arc<Array2<f32>>, Arc<Array2<f32>>, Arc<Array2<f32>>);

struct RgbPreviewEntry {
    path: String,
    stamp: RgbStamp,
    planes: RgbPlanes,
}

static RGB_PREVIEW_CACHE: LazyLock<Mutex<Vec<RgbPreviewEntry>>> = LazyLock::new(|| Mutex::new(Vec::new()));

const RGB_PREVIEW_CACHE_MAX_BYTES: usize = 512 << 20;

fn lock_rgb_cache() -> MutexGuard<'static, Vec<RgbPreviewEntry>> {
    RGB_PREVIEW_CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

pub(super) fn rgb_stamp(p: &str) -> Option<RgbStamp> {
    std::fs::metadata(source_path(p)).ok().map(|m| (m.len(), m.modified().ok()))
}

fn planes_bytes(planes: &RgbPlanes) -> usize {
    (planes.0.len() + planes.1.len() + planes.2.len()) * std::mem::size_of::<f32>()
}

pub(super) fn remember_rgb_planes(path: &str, stamp: RgbStamp, planes: &RgbPlanes) {
    let bytes = planes_bytes(planes);
    if bytes > RGB_PREVIEW_CACHE_MAX_BYTES {
        return;
    }
    let mut cache = lock_rgb_cache();
    cache.retain(|e| e.path != path);
    let mut total = bytes + cache.iter().map(|e| planes_bytes(&e.planes)).sum::<usize>();
    while total > RGB_PREVIEW_CACHE_MAX_BYTES && !cache.is_empty() {
        total -= planes_bytes(&cache.remove(0).planes);
    }
    cache.push(RgbPreviewEntry { path: path.to_string(), stamp, planes: planes.clone() });
}

pub(super) fn cached_rgb_planes(path: &str, stamp: &RgbStamp) -> Option<RgbPlanes> {
    let mut cache = lock_rgb_cache();
    let index = cache.iter().position(|e| e.path == path && &e.stamp == stamp)?;
    let entry = cache.remove(index);
    let planes = entry.planes.clone();
    cache.push(entry);
    Some(planes)
}

pub(crate) fn load_rgb_file_planes(p: &str) -> anyhow::Result<RgbPlanes> {
    let stamp = rgb_stamp(p);
    if let Some(planes) = stamp.as_ref().and_then(|st| cached_rgb_planes(p, st)) {
        return Ok(planes);
    }
    let rgb = try_extract_rgb_resolved(p)?
        .ok_or_else(|| anyhow::anyhow!("Not an RGB image: {}", p))?;
    let planes = (Arc::new(rgb.r), Arc::new(rgb.g), Arc::new(rgb.b));
    if let Some(st) = stamp {
        remember_rgb_planes(p, st, &planes);
    }
    Ok(planes)
}

pub(crate) fn rgb_source_planes(path: Option<&str>) -> anyhow::Result<RgbPlanes> {
    match path {
        Some(p) => load_rgb_file_planes(p),
        None => {
            let (r, g, b) = helpers::load_composite_rgb()?;
            Ok((r.data_arc(), g.data_arc(), b.data_arc()))
        }
    }
}

#[tauri::command]
pub async fn use_rgb_file_as_composite_cmd(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let (r, g, b) = load_rgb_file_planes(&path)?;
        let (rows, cols) = r.dim();
        let (stats_r, (stats_g, stats_b)) = rayon::join(
            || compute_image_stats(&r),
            || rayon::join(|| compute_image_stats(&g), || compute_image_stats(&b)),
        );
        helpers::insert_composite_and_orig(
            Arc::unwrap_or_clone(r),
            Arc::unwrap_or_clone(g),
            Arc::unwrap_or_clone(b),
            stats_r,
            stats_g,
            stats_b,
        );
        Ok(json!({ RES_DIMENSIONS: [cols, rows] }))
    })
}
