use std::time::Instant;
use serde_json::json;
use crate::cmd::common::{blocking_cmd, load_cached};
use crate::types::constants::{RES_ELAPSED_MS, RES_N_DETECTED};
use crate::core::analysis::star_detection::{detect_stars as detect_stars_core, DetectionResult};
use crate::core::imaging::luminance::rgb_to_luminance;

pub(in crate::cmd::analysis) fn capped_detection_json(mut result: DetectionResult, max_stars: usize, t0: Instant) -> anyhow::Result<serde_json::Value> {
    let n_detected = result.stars.len();
    result.stars.truncate(max_stars);
    let mut val = serde_json::to_value(&result)?;
    if let Some(obj) = val.as_object_mut() {
        obj.insert(RES_N_DETECTED.to_string(), json!(n_detected));
        obj.insert(RES_ELAPSED_MS.to_string(), json!(t0.elapsed().as_millis() as u64));
    }
    Ok(val)
}

#[tauri::command]
pub async fn detect_stars(
    path: String,
    sigma: f64,
    max_stars: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        capped_detection_json(detect_stars_core(load_cached(&path)?.arr(), sigma), max_stars, t0)
    })
}

#[tauri::command]
pub async fn detect_stars_composite(
    sigma: f64,
    max_stars: usize,
    path: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let (r, g, b) = crate::cmd::io::rgb_source_planes(path.as_deref())?;
        let lum = composite_detection_luminance(&r, &g, &b)?;
        capped_detection_json(detect_stars_core(&lum, sigma), max_stars, t0)
    })
}

fn composite_detection_luminance(
    r: &ndarray::Array2<f32>,
    g: &ndarray::Array2<f32>,
    b: &ndarray::Array2<f32>,
) -> anyhow::Result<ndarray::Array2<f32>> {
    if r.dim() != g.dim() || r.dim() != b.dim() {
        anyhow::bail!(
            "Composite planes differ in size: R {:?}, G {:?}, B {:?}",
            r.dim(),
            g.dim(),
            b.dim()
        );
    }
    Ok(rgb_to_luminance(r, g, b))
}
