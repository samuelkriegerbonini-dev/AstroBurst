use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Instant;

use ndarray::Array2;
use serde_json::json;
use tauri::ipc::Response;

use crate::cmd::common::{blocking_cmd, image_ref, load_cached, load_cached_full, load_preview_validated, output_stem, plane_info_json, resolve_output_dir, save_stf_preview_png, source_path, try_extract_rgb_resolved, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::imaging::stats::{compute_histogram_with_stats, compute_image_stats, downsample_histogram};
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, AutoStfConfig};
use crate::infra::cache::ImageEntry;
use crate::infra::ipc::{encode_rgb_with_header_downsampled, encode_with_header_downsampled};
use crate::types::constants::{HISTOGRAM_BINS_DISPLAY, RES_AUTO_STF, RES_BINS, RES_BIN_COUNT, RES_DATA_MAX, RES_DATA_MIN, RES_DIMENSIONS, RES_ELAPSED_MS, RES_HEADER, RES_HISTOGRAM, RES_IMAGE_REF, RES_MAD, RES_MEAN, RES_MEDIAN, RES_PLANE, RES_PNG_PATH, RES_SIGMA, RES_STATS, RES_STF, RES_TOTAL_PIXELS, RES_IS_RGB, STF_R, STF_G, STF_B};
use crate::types::image::StfParams;

fn png_path_for(path: &str, output_dir: &str) -> String {
    format!("{}/{}.png", output_dir, output_stem(path))
}

fn render_to_png(cached: &ImageEntry, png_path: &str) -> anyhow::Result<(StfParams, usize, usize)> {
    let stats = cached.stats();
    let stf_params = auto_stf(stats, &AutoStfConfig::default());
    save_stf_preview_png(cached.arr(), &stf_params, stats, png_path)?;
    let (rows, cols) = cached.arr().dim();
    Ok((stf_params, rows, cols))
}

fn process_rgb_fits(
    path: &str,
    output_dir: &str,
    t0: Instant,
    full: bool,
) -> anyhow::Result<Option<serde_json::Value>> {
    let stamp = rgb_stamp(path);
    let rgb = match try_extract_rgb_resolved(path)? {
        Some(r) => r,
        None => return Ok(None),
    };

    let stats_r = compute_image_stats(&rgb.r);
    let stats_g = compute_image_stats(&rgb.g);
    let stats_b = compute_image_stats(&rgb.b);

    let stf_r = auto_stf(&stats_r, &AutoStfConfig::default());
    let stf_g = auto_stf(&stats_g, &AutoStfConfig::default());
    let stf_b = auto_stf(&stats_b, &AutoStfConfig::default());

    let r_stretched = apply_stf_f32(&rgb.r, &stf_r, &stats_r);
    let g_stretched = apply_stf_f32(&rgb.g, &stf_g, &stats_g);
    let b_stretched = apply_stf_f32(&rgb.b, &stf_b, &stats_b);

    let (rows, cols) = rgb.r.dim();
    let png_path = png_path_for(path, output_dir);
    helpers::render_rgb_preview(&r_stretched, &g_stretched, &b_stretched, &png_path, MAX_PREVIEW_DIM)?;

    let full_data = if full {
        let hist = compute_histogram_with_stats(&rgb.r, &stats_r);
        let display_bins = downsample_histogram(&hist, HISTOGRAM_BINS_DISPLAY);
        let header_json = serde_json::to_value(&rgb.header.index)?;
        Some((display_bins, header_json))
    } else {
        None
    };

    if let Some(stamp) = stamp {
        remember_rgb_planes(path, stamp, &(Arc::new(rgb.r), Arc::new(rgb.g), Arc::new(rgb.b)));
    }

    let mut result = json!({
        RES_PNG_PATH: png_path,
        RES_DIMENSIONS: [cols, rows],
        RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        RES_STATS: helpers::stats_json_full(&stats_r),
        RES_STF: helpers::stf_json(&stf_r),
        RES_IS_RGB: true,
        STF_R: helpers::stf_json(&stf_r),
        STF_G: helpers::stf_json(&stf_g),
        STF_B: helpers::stf_json(&stf_b),
    });

    if let Some((display_bins, header_json)) = full_data {
        if let Some(obj) = result.as_object_mut() {
            obj.insert(RES_HEADER.to_string(), header_json);
            obj.insert(RES_HISTOGRAM.to_string(), json!({
                RES_BINS: display_bins,
                RES_BIN_COUNT: display_bins.len(),
                RES_DATA_MIN: stats_r.min,
                RES_DATA_MAX: stats_r.max,
                RES_MEDIAN: stats_r.median,
                RES_MEAN: stats_r.mean,
                RES_SIGMA: stats_r.sigma,
                RES_MAD: stats_r.mad,
                RES_TOTAL_PIXELS: stats_r.valid_count,
                RES_AUTO_STF: helpers::stf_json(&stf_r),
            }));
        }
    }

    Ok(Some(result))
}

#[tauri::command]
pub async fn process_fits(path: String, output_dir: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let output_dir = resolve_output_dir(&output_dir)?;

        if let Some(result) = process_rgb_fits(&path, &output_dir, t0, false)? {
            return Ok(result);
        }

        let cached = load_cached(&path)?;
        let png_path = png_path_for(&path, &output_dir);
        let (stf_params, rows, cols) = render_to_png(&cached, &png_path)?;
        let plane = plane_json_for(&path, &cached);

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_STATS: helpers::stats_json(cached.stats()),
            RES_STF: helpers::stf_json(&stf_params),
            RES_IMAGE_REF: image_ref(&path).cache_key(),
            RES_PLANE: plane,
        }))
    })
}

fn plane_json_for(path: &str, cached: &ImageEntry) -> serde_json::Value {
    match plane_info_json(path, cached) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("plane info unavailable for {}: {:#}", path, e);
            serde_json::Value::Null
        }
    }
}

#[tauri::command]
pub async fn process_fits_full(path: String, output_dir: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let output_dir = resolve_output_dir(&output_dir)?;

        if let Some(result) = process_rgb_fits(&path, &output_dir, t0, true)? {
            return Ok(result);
        }

        let cached = load_cached_full(&path)?;
        let png_path = png_path_for(&path, &output_dir);
        let (stf_params, rows, cols) = render_to_png(&cached, &png_path)?;

        let stats = cached.stats();
        let hist = compute_histogram_with_stats(cached.arr(), stats);
        let display_bins = downsample_histogram(&hist, HISTOGRAM_BINS_DISPLAY);

        let header_json = match cached.header() {
            Some(h) => serde_json::to_value(&h.index)?,
            None => json!(null),
        };
        let plane = plane_json_for(&path, &cached);

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_STATS: helpers::stats_json_full(stats),
            RES_STF: helpers::stf_json(&stf_params),
            RES_HEADER: header_json,
            RES_IMAGE_REF: image_ref(&path).cache_key(),
            RES_PLANE: plane,
            RES_HISTOGRAM: {
                RES_BINS: display_bins,
                RES_BIN_COUNT: display_bins.len(),
                RES_DATA_MIN: stats.min,
                RES_DATA_MAX: stats.max,
                RES_MEDIAN: stats.median,
                RES_MEAN: stats.mean,
                RES_SIGMA: stats.sigma,
                RES_MAD: stats.mad,
                RES_TOTAL_PIXELS: stats.valid_count,
                RES_AUTO_STF: helpers::stf_json(&stf_params),
            },
        }))
    })
}

#[tauri::command]
pub async fn get_raw_pixels_preview(path: String, max_dim: Option<u32>) -> Result<Response, String> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<Response> {
        let dim = max_dim.unwrap_or(2048) as usize;
        let entry = load_preview_validated(&path)?;
        let data = encode_with_header_downsampled(entry.arr(), dim)?;
        Ok(Response::new(data))
    })
        .await
        .map_err(|e| format!("{}", e))?
        .map_err(|e| format!("{:#}", e))
}

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

fn rgb_stamp(p: &str) -> Option<RgbStamp> {
    std::fs::metadata(source_path(p)).ok().map(|m| (m.len(), m.modified().ok()))
}

fn planes_bytes(planes: &RgbPlanes) -> usize {
    (planes.0.len() + planes.1.len() + planes.2.len()) * std::mem::size_of::<f32>()
}

fn remember_rgb_planes(path: &str, stamp: RgbStamp, planes: &RgbPlanes) {
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

fn cached_rgb_planes(path: &str, stamp: &RgbStamp) -> Option<RgbPlanes> {
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

#[tauri::command]
pub async fn release_cube_cmd(path: String) -> Result<(), String> {
    GLOBAL_CUBE_CACHE.invalidate(&path);
    Ok(())
}

#[tauri::command]
pub async fn get_raw_rgb_pixels_preview(
    path: Option<String>,
    max_dim: Option<u32>,
) -> Result<Response, String> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<Response> {
        let dim = max_dim.unwrap_or(2048) as usize;
        let data = match path {
            Some(p) => {
                let (r, g, b) = load_rgb_file_planes(&p)?;
                encode_rgb_with_header_downsampled(&r, &g, &b, dim)?
            }
            None => {
                let derived = helpers::load_composite_toned()
                    .or_else(helpers::load_composite_stretched);
                match derived {
                    Some((r, g, b)) => {
                        crate::infra::ipc::encode_rgb_with_header_downsampled_flagged(
                            r.arr(), g.arr(), b.arr(), dim, 1,
                        )?
                    }
                    None => {
                        let (r, g, b) = helpers::load_composite_rgb()?;
                        encode_rgb_with_header_downsampled(r.arr(), g.arr(), b.arr(), dim)?
                    }
                }
            }
        };
        Ok(Response::new(data))
    })
        .await
        .map_err(|e| format!("{}", e))?
        .map_err(|e| format!("{:#}", e))
}

#[cfg(test)]
pub(crate) mod test_support {
    use ndarray::Array2;

    use crate::cmd::common::MAX_PREVIEW_DIM;
    use crate::core::imaging::stf::{apply_stf, ImageStats, StfParams};

    pub(crate) fn wide_sky(rows: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, 2 * MAX_PREVIEW_DIM + 8), |(r, c)| {
            let noise = ((r * 7919 + c * 104_729) % 41) as f32 - 20.0;
            let texture = if (r + c) % 2 == 0 { 0.0 } else { 90.0 };
            let star = if r == 2 && c % 997 == 11 { 4000.0 } else { 0.0 };
            1000.0 + 0.01 * c as f32 + noise + texture + star
        })
    }

    pub(crate) fn assert_matches_gpu_view(png_path: &str, arr: &Array2<f32>, stf: &StfParams, stats: &ImageStats) {
        let encoded = crate::infra::ipc::encode_with_header_downsampled(arr, MAX_PREVIEW_DIM).unwrap();
        let width = u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);
        let height = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
        let values: Vec<f32> = encoded[16..].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let reduced = Array2::from_shape_vec((height as usize, width as usize), values).unwrap();
        let expected = apply_stf(&reduced, stf, stats);
        let png = image::open(png_path).unwrap().to_luma8();
        assert_eq!((png.width(), png.height()), (width, height));
        let off = png.as_raw().iter().zip(&expected).filter(|(a, b)| a.abs_diff(**b) > 1).count();
        assert_eq!(off, 0, "{off} of {} preview pixels of {png_path} differ from the GPU view", expected.len());
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{assert_matches_gpu_view, wide_sky};
    use super::*;
    use crate::infra::cache::GLOBAL_IMAGE_CACHE;
    use crate::infra::fits::writer::{write_fits_mono, write_fits_rgb};
    use crate::types::constants::{
        COMPOSITE_KEY_B, COMPOSITE_KEY_G, COMPOSITE_KEY_R, COMPOSITE_ORIG_B, COMPOSITE_ORIG_G, COMPOSITE_ORIG_R,
    };

    #[test]
    fn the_ingest_preview_of_a_large_image_reduces_the_values_before_the_stretch_like_the_gpu_view() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide_ingest.fits").to_str().unwrap().to_string();
        write_fits_mono(&path, &wide_sky(6), None).unwrap();
        let entry = load_cached(&path).unwrap();
        let png_path = dir.path().join("wide_ingest.png").to_str().unwrap().to_string();

        let (stf, _, _) = render_to_png(&entry, &png_path).unwrap();
        assert_matches_gpu_view(&png_path, entry.arr(), &stf, entry.stats());
    }

    #[tokio::test]
    async fn releasing_a_cube_lets_its_file_be_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("released_cube.fits");
        crate::core::cube::lazy::test_support::write_line_cube(&path, 0.0);
        let key = path.to_str().unwrap().to_string();
        let first = GLOBAL_CUBE_CACHE.get_or_open(&key).unwrap();
        let again = GLOBAL_CUBE_CACHE.get_or_open(&key).unwrap();
        assert!(Arc::ptr_eq(&first, &again), "precondition: the cube stays open in the cache");
        drop((first, again));

        release_cube_cmd(key.replace('\\', "/")).await.unwrap();
        std::fs::OpenOptions::new().write(true).truncate(true).open(&path).expect("the released cube is still mapped");
        crate::core::cube::lazy::test_support::write_line_cube(&path, 1.0);
        assert!(GLOBAL_CUBE_CACHE.get_or_open(&key).is_ok());
        GLOBAL_CUBE_CACHE.invalidate(&key);
    }

    #[test]
    fn png_path_for_uses_plane_aware_stem() {
        assert!(png_path_for("C:/x/a.fits#hdu=3", "out").ends_with("a_hdu3.png"));
        assert_eq!(png_path_for("C:/x/a.fits", "out"), "out/a.png");
        assert_eq!(png_path_for("C:/x/r.asdf#array=roman.dq", "out"), "out/r_roman_dq.png");
    }

    struct RgbFile {
        path: String,
        r: Array2<f32>,
        g: Array2<f32>,
        b: Array2<f32>,
    }

    fn rgb_file(dir: &tempfile::TempDir, name: &str, base: f32) -> RgbFile {
        let path = dir.path().join(name).to_str().unwrap().to_string();
        let r = Array2::from_shape_fn((4, 5), |(y, x)| base + (y * 5 + x) as f32);
        let g = r.mapv(|v| v + 100.0);
        let b = r.mapv(|v| v + 200.0);
        write_fits_rgb(&path, &r, &g, &b, None).unwrap();
        RgbFile { path, r, g, b }
    }

    fn bump_mtime(path: &str) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
    }

    fn slot_holds(key: &str, plane: &Array2<f32>) -> bool {
        GLOBAL_IMAGE_CACHE.get(key).is_some_and(|e| e.arr() == plane)
    }

    fn composite_slots(file: &RgbFile) -> [(&'static str, &Array2<f32>); 6] {
        [
            (COMPOSITE_KEY_R, &file.r),
            (COMPOSITE_KEY_G, &file.g),
            (COMPOSITE_KEY_B, &file.b),
            (COMPOSITE_ORIG_R, &file.r),
            (COMPOSITE_ORIG_G, &file.g),
            (COMPOSITE_ORIG_B, &file.b),
        ]
    }

    #[test]
    fn each_ingested_rgb_file_keeps_its_own_planes_under_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let out_dir = dir.path().to_str().unwrap();
        let a = rgb_file(&dir, "rgb_a.fits", 1000.0);
        let b = rgb_file(&dir, "rgb_b.fits", 5000.0);
        assert!(process_rgb_fits(&a.path, out_dir, Instant::now(), false).unwrap().is_some());
        assert!(process_rgb_fits(&b.path, out_dir, Instant::now(), false).unwrap().is_some());

        let stamp_a = rgb_stamp(&a.path).unwrap();
        let primed_a = cached_rgb_planes(&a.path, &stamp_a).expect("A was evicted by B's ingest");
        let (ar, ag, ab) = load_rgb_file_planes(&a.path).unwrap();
        assert!(Arc::ptr_eq(&primed_a.0, &ar));
        assert!(Arc::ptr_eq(&primed_a.1, &ag));
        assert!(Arc::ptr_eq(&primed_a.2, &ab));
        assert_eq!((ar.as_ref(), ag.as_ref(), ab.as_ref()), (&a.r, &a.g, &a.b));

        let (br, bg, bb) = load_rgb_file_planes(&b.path).unwrap();
        assert_eq!((br.as_ref(), bg.as_ref(), bb.as_ref()), (&b.r, &b.g, &b.b));
    }

    #[tokio::test]
    async fn ingesting_an_rgb_fits_leaves_the_composite_slots_alone() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let file = rgb_file(&dir, "osc_ingest.fits", 7_123_000.0);
        let out_dir = dir.path().to_str().unwrap();
        assert!(process_rgb_fits(&file.path, out_dir, Instant::now(), true).unwrap().is_some());
        for (key, plane) in composite_slots(&file) {
            assert!(!slot_holds(key, plane), "ingest wrote the file into the global composite slot {key}");
        }
    }

    #[tokio::test]
    async fn an_rgb_file_becomes_the_composite_only_through_the_explicit_command() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let file = rgb_file(&dir, "osc_explicit.fits", 9_456_000.0);
        let value = use_rgb_file_as_composite_cmd(file.path.clone()).await;
        let held: Vec<bool> = composite_slots(&file).iter().map(|(key, plane)| slot_holds(key, plane)).collect();
        helpers::clear_composite();
        assert_eq!(held, vec![true; 6], "the composite slots do not hold the file's planes");
        assert_eq!(value.unwrap()[RES_DIMENSIONS], json!([5, 4]));

        let mono = dir.path().join("mono.fits").to_str().unwrap().to_string();
        write_fits_mono(&mono, &Array2::from_elem((4, 5), 1.0), None).unwrap();
        assert!(use_rgb_file_as_composite_cmd(mono).await.is_err());
    }

    #[tokio::test]
    async fn the_rgb_source_of_a_cleared_composite_says_to_run_blend_again() {
        let _guard = helpers::composite_test_lock().await;
        helpers::clear_composite();
        let err = rgb_source_planes(None).expect_err("the composite was cleared");
        assert_eq!(err.to_string(), "The colour composite is no longer in memory; run Blend again.");
    }

    #[test]
    fn an_rgb_file_rewritten_on_disk_is_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let first = rgb_file(&dir, "rewritten_rgb.fits", 10.0);
        assert_eq!(load_rgb_file_planes(&first.path).unwrap().0.as_ref(), &first.r);

        let second = rgb_file(&dir, "rewritten_rgb.fits", 20.0);
        bump_mtime(&second.path);
        assert_eq!(load_rgb_file_planes(&second.path).unwrap().0.as_ref(), &second.r);
    }

    #[tokio::test]
    async fn reopening_a_file_rewritten_on_disk_serves_the_new_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let out_dir = dir.path().to_str().unwrap().to_string();
        let path = dir.path().join("recalibrated.fits").to_str().unwrap().to_string();
        write_fits_mono(&path, &Array2::from_elem((6, 6), 1.0), None).unwrap();
        let first = process_fits_full(path.clone(), out_dir.clone()).await.unwrap();
        assert_eq!(first[RES_STATS][RES_MEAN], json!(1.0));

        write_fits_mono(&path, &Array2::from_elem((6, 6), 3.0), None).unwrap();
        bump_mtime(&path);
        let again = process_fits_full(path.clone(), out_dir.clone()).await.unwrap();
        assert_eq!(again[RES_STATS][RES_MEAN], json!(3.0), "the previous calibration was served from memory");
        let quick = process_fits(path, out_dir).await.unwrap();
        assert_eq!(quick[RES_STATS][RES_MEAN], json!(3.0));
    }
}
