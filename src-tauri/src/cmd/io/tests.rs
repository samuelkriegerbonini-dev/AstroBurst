use std::sync::Arc;
use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::load_cached;
use crate::cmd::helpers;
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::types::constants::{RES_DIMENSIONS, RES_MEAN, RES_STATS};

use super::fits_loading::{png_path_for, process_rgb_fits, render_to_png};
use super::rgb_planes::cached_rgb_planes;

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
