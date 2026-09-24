use std::time::Instant;

use anyhow::{Context, Result};
use ndarray::Array2;
use rayon::prelude::*;
use serde_json::{json, Value};

use crate::cmd::analysis::resolve_dq_mask;
use crate::cmd::common::{blocking_cmd, load_cached, load_cached_full};
use crate::cmd::io::rgb_source_planes;
use crate::core::imaging::pixel_probe::data_unit;
use crate::core::imaging::region::RegionShape;
use crate::core::imaging::statistics::{
    evaluate_noise, evaluate_noise_in_region, evaluate_noise_masked, exact_statistics, data_range,
    statistics_for_region, ChannelStatistics, NoiseEvaluation, RegionNoise,
};
use crate::types::constants::{RES_DATA_MAX, RES_DATA_MIN, RES_DQ_EXCLUDED, RES_ELAPSED_MS, RES_MASKED, RES_PATH};

pub const KEY_STATISTICS: &str = "statistics";
pub const KEY_NOISE: &str = "noise";
pub const KEY_NOISE_NOTE: &str = "noise_note";
pub const KEY_UNIT: &str = "unit";
pub const KEY_RESULTS: &str = "results";
pub const KEY_SIGMA: &str = "sigma";
pub const KEY_FRACTION: &str = "fraction";
pub const KEY_ERROR: &str = "error";
pub const KEY_CHANNEL_R: &str = "r";
pub const KEY_CHANNEL_G: &str = "g";
pub const KEY_CHANNEL_B: &str = "b";

const MAX_BATCH_PATHS: usize = 4096;

struct ChannelBody {
    statistics: ChannelStatistics,
    noise: Option<NoiseEvaluation>,
    noise_note: Option<String>,
    data_min: f64,
    data_max: f64,
}

fn channel_body(
    arr: &Array2<f32>,
    region: Option<&RegionShape>,
    mask: Option<&Array2<u8>>,
    want_noise: bool,
) -> Result<ChannelBody> {
    let statistics = match region {
        Some(shape) => statistics_for_region(arr, shape, mask)?,
        None => exact_statistics(arr, mask),
    };
    let (noise, noise_note) = match (want_noise, region) {
        (false, _) => (None, None),
        (true, Some(shape)) => match evaluate_noise_in_region(arr, shape, mask)? {
            RegionNoise::Evaluated(noise) => (Some(noise), None),
            too_small => (None, too_small.note()),
        },
        (true, None) => (Some(evaluate_noise_masked(arr, mask)), None),
    };
    let (data_min, data_max) = data_range(arr);
    Ok(ChannelBody { statistics, noise, noise_note, data_min, data_max })
}

fn nullable_f64(v: f64) -> Value {
    if v.is_finite() {
        json!(v)
    } else {
        Value::Null
    }
}

fn channel_json(body: &ChannelBody) -> Result<Value> {
    Ok(json!({
        KEY_STATISTICS: serde_json::to_value(&body.statistics)?,
        KEY_NOISE: body.noise.as_ref().map(serde_json::to_value).transpose()?,
        KEY_NOISE_NOTE: body.noise_note.as_deref(),
        RES_DATA_MIN: nullable_f64(body.data_min),
        RES_DATA_MAX: nullable_f64(body.data_max),
    }))
}

#[tauri::command]
pub async fn compute_statistics_cmd(
    path: String,
    exclude_dq: Option<bool>,
    region: Option<RegionShape>,
    noise: Option<bool>,
) -> Result<Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let entry = load_cached_full(&path)?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), entry.arr().dim());
        let body = channel_body(entry.arr(), region.as_ref(), mask.as_ref().map(|m| &m.map), noise.unwrap_or(false))?;
        let unit = entry.header().and_then(data_unit);
        let mut out = channel_json(&body)?;
        let obj = out.as_object_mut().context("statistics body is an object")?;
        obj.insert(KEY_UNIT.into(), json!(unit));
        obj.insert(RES_MASKED.into(), json!(mask.is_some()));
        obj.insert(RES_DQ_EXCLUDED.into(), json!(mask.as_ref().map(|m| m.excluded)));
        obj.insert(RES_ELAPSED_MS.into(), json!(t0.elapsed().as_millis() as u64));
        Ok(out)
    })
}

#[tauri::command]
pub async fn compute_statistics_composite_cmd(noise: Option<bool>, path: Option<String>) -> Result<Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let (r, g, b) = rgb_source_planes(path.as_deref())?;
        let want_noise = noise.unwrap_or(false);
        let bodies: Vec<ChannelBody> = [r.as_ref(), g.as_ref(), b.as_ref()]
            .into_par_iter()
            .map(|arr| channel_body(arr, None, None, want_noise))
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({
            KEY_CHANNEL_R: channel_json(&bodies[0])?,
            KEY_CHANNEL_G: channel_json(&bodies[1])?,
            KEY_CHANNEL_B: channel_json(&bodies[2])?,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

fn noise_entry(path: &str) -> Value {
    match load_cached(path) {
        Ok(entry) => {
            let n = evaluate_noise(entry.arr());
            json!({ RES_PATH: path, KEY_SIGMA: n.sigma, KEY_FRACTION: n.fraction, KEY_ERROR: Value::Null })
        }
        Err(e) => json!({ RES_PATH: path, KEY_SIGMA: Value::Null, KEY_FRACTION: Value::Null, KEY_ERROR: format!("{:#}", e) }),
    }
}

#[tauri::command]
pub async fn evaluate_noise_batch_cmd(paths: Vec<String>) -> Result<Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        if paths.len() > MAX_BATCH_PATHS {
            anyhow::bail!("too many paths in one call ({}); the limit is {}", paths.len(), MAX_BATCH_PATHS);
        }
        let results: Vec<Value> = paths.par_iter().map(|p| noise_entry(p)).collect();
        Ok(json!({
            KEY_RESULTS: results,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef_with_dq_cards;

    fn mef_with_bunit(dir: &tempfile::TempDir, name: &str) -> String {
        let path = dir.path().join(name);
        let mut dq = vec![-2147483648i32; 16];
        dq[3] = -2147483647;
        sci_err_dq_mef_with_dq_cards(&path, 4, 4, dq, vec![("BUNIT", "'MJy/sr'".into())]);
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn channel_body_reports_exact_statistics_noise_and_full_frame_range_for_a_region() {
        let data = Array2::from_shape_fn((32, 32), |(y, x)| ((y * 32 + x) as f32) - 100.0);
        let shape = RegionShape::Box { x: 16.0, y: 16.0, width: 9.0, height: 9.0, angle: 0.0 };
        let body = channel_body(&data, Some(&shape), None, true).unwrap();
        assert_eq!(body.statistics.count, 81);
        assert_eq!(body.statistics.median, (16.0 * 32.0 + 16.0) - 100.0);
        assert_eq!(body.data_min, -100.0);
        assert_eq!(body.data_max, 923.0);
        let noise = body.noise.expect("noise requested");
        assert_eq!(noise.method, "k-sigma-mrs");
        let no_noise = channel_body(&data, None, None, false).unwrap();
        assert!(no_noise.noise.is_none());
        assert_eq!(no_noise.statistics.count, 1023);
        assert_eq!(no_noise.statistics.padding, 1);
    }

    #[test]
    fn the_data_range_of_a_zero_padded_frame_skips_the_padding_like_the_statistics() {
        let data = Array2::from_shape_fn((16, 16), |(y, x)| if x < 4 { 0.0 } else { 10.0 + (y * 16 + x) as f32 });
        let body = channel_body(&data, None, None, false).unwrap();
        assert_eq!(body.statistics.padding, 64);
        assert_eq!(body.data_min, 14.0);
        assert_eq!(body.data_max, 265.0);
        assert_eq!((body.data_min, body.data_max), (body.statistics.min, body.statistics.max));
    }

    #[test]
    fn channel_json_turns_a_non_finite_range_into_null() {
        let data = Array2::from_elem((2, 2), f32::NAN);
        let body = channel_body(&data, None, None, false).unwrap();
        let j = channel_json(&body).unwrap();
        assert!(j[RES_DATA_MIN].is_null());
        assert!(j[RES_DATA_MAX].is_null());
        assert_eq!(j[KEY_STATISTICS]["count"], 0);
        assert_eq!(j[KEY_STATISTICS]["nan_count"], 4);
        assert!(j[KEY_NOISE].is_null());
        assert!(j[KEY_NOISE_NOTE].is_null());
    }

    #[test]
    fn channel_json_reports_a_note_instead_of_zero_sigma_for_a_tiny_region() {
        let data = Array2::from_shape_fn((32, 32), |(y, x)| ((y * 32 + x) as f32) - 100.0);
        let point = RegionShape::Point { x: 10.0, y: 10.0 };
        let body = channel_body(&data, Some(&point), None, true).unwrap();
        assert_eq!(body.statistics.count, 1);
        assert!(body.noise.is_none());
        let j = channel_json(&body).unwrap();
        assert!(j[KEY_NOISE].is_null());
        assert_eq!(j[KEY_NOISE_NOTE], "region too small for noise evaluation (0 finite pixels, 64 needed)");

        let circle = RegionShape::Circle { x: 10.0, y: 10.0, r: 2.0 };
        let j = channel_json(&channel_body(&data, Some(&circle), None, true).unwrap()).unwrap();
        assert_eq!(j[KEY_STATISTICS]["count"], 13);
        assert!(j[KEY_NOISE].is_null());
        assert_eq!(j[KEY_NOISE_NOTE], "region too small for noise evaluation (13 finite pixels, 64 needed)");

        let big = RegionShape::Box { x: 16.0, y: 16.0, width: 9.0, height: 9.0, angle: 0.0 };
        let j = channel_json(&channel_body(&data, Some(&big), None, true).unwrap()).unwrap();
        assert!(j[KEY_NOISE].is_object());
        assert!(j[KEY_NOISE_NOTE].is_null());

        let j = channel_json(&channel_body(&data, Some(&circle), None, false).unwrap()).unwrap();
        assert!(j[KEY_NOISE].is_null());
        assert!(j[KEY_NOISE_NOTE].is_null());
    }

    #[test]
    fn statistics_of_a_mef_plane_honour_the_dq_mask_and_report_the_bunit() {
        let dir = tempfile::tempdir().unwrap();
        let path = mef_with_bunit(&dir, "stats.fits");
        let key = format!("{}#hdu=1", path);
        let entry = load_cached_full(&key).unwrap();
        let unit = entry.header().and_then(data_unit);
        assert_eq!(unit.as_deref(), Some("MJy/sr"));
        let mask = resolve_dq_mask(&key, true, entry.arr().dim()).expect("mask");
        let masked = channel_body(entry.arr(), None, Some(&mask.map), false).unwrap();
        let plain = channel_body(entry.arr(), None, None, false).unwrap();
        assert_eq!(masked.statistics.excluded, mask.excluded);
        assert_eq!(masked.statistics.count + masked.statistics.excluded, plain.statistics.count);
        assert_eq!(plain.statistics.excluded, 0);
    }

    #[tokio::test]
    async fn composite_statistics_of_an_rgb_file_measure_that_file_and_not_the_blend_slots() {
        let _guard = crate::cmd::helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("osc_stats.fits").to_str().unwrap().to_string();
        let r = Array2::from_shape_fn((5, 5), |(y, x)| 1000.0 + (y * 5 + x) as f32);
        let g = r.mapv(|v| v + 100.0);
        let b = r.mapv(|v| v + 200.0);
        crate::infra::fits::writer::write_fits_rgb(&path, &r, &g, &b, None).unwrap();
        let blend = Array2::from_elem((5, 5), 7.0f32);
        let blend_stats = crate::core::imaging::stats::compute_image_stats(&blend);
        crate::cmd::helpers::insert_composite_and_orig(
            blend.clone(),
            blend.clone(),
            blend,
            blend_stats.clone(),
            blend_stats.clone(),
            blend_stats,
        );

        let of_file = compute_statistics_composite_cmd(None, Some(path)).await;
        let of_slots = compute_statistics_composite_cmd(None, None).await;
        crate::cmd::helpers::clear_composite();
        let of_file = of_file.unwrap();
        let medians: Vec<f64> = [KEY_CHANNEL_R, KEY_CHANNEL_G, KEY_CHANNEL_B]
            .iter()
            .map(|k| of_file[*k][KEY_STATISTICS]["median"].as_f64().unwrap())
            .collect();
        assert_eq!(medians, vec![1012.0, 1112.0, 1212.0]);
        assert_eq!(of_slots.unwrap()[KEY_CHANNEL_R][KEY_STATISTICS]["median"], 7.0);
    }

    #[test]
    fn noise_entry_reports_an_error_for_a_missing_file_instead_of_failing_the_batch() {
        let entry = noise_entry("C:/definitely/missing/frame.fits");
        assert!(entry[KEY_SIGMA].is_null());
        assert!(entry[KEY_ERROR].is_string());
        assert_eq!(entry[RES_PATH], "C:/definitely/missing/frame.fits");
    }
}
