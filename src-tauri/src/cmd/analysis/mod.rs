use std::time::Instant;

use serde_json::json;
use tauri::ipc::Response;
use rayon::prelude::*;

use crate::cmd::common::{blocking_cmd, load_cached};
use crate::types::constants::{
    HISTOGRAM_BINS_DISPLAY, RES_BINS, RES_BIN_COUNT, RES_BIN_EDGES, RES_MIN, RES_MAX,
    RES_DATA_MIN, RES_DATA_MAX, RES_MEDIAN, RES_MEAN, RES_SIGMA, RES_MAD, RES_TOTAL_PIXELS,
    RES_AUTO_STF, RES_SHADOW, RES_MIDTONE, RES_HIGHLIGHT, RES_ELAPSED_MS,
};
use crate::types::image::AutoStfConfig;
use crate::core::analysis::fft::compute_power_spectrum;
use crate::core::analysis::star_detection::detect_stars as detect_stars_core;
use crate::core::imaging::stats::{compute_histogram_with_stats, downsample_histogram};
use crate::core::imaging::stf::auto_stf;

const PAR_THRESHOLD: usize = 1_000_000;

#[tauri::command]
pub async fn compute_histogram(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let cached = load_cached(&path)?;
        let stats = cached.stats();

        let hist = compute_histogram_with_stats(cached.arr(), stats);
        let display_bins = downsample_histogram(&hist, HISTOGRAM_BINS_DISPLAY);
        let stf_params = auto_stf(stats, &AutoStfConfig::default());

        Ok(json!({
            RES_BINS: display_bins,
            RES_BIN_COUNT: display_bins.len(),
            RES_BIN_EDGES: hist.bin_edges,
            RES_MIN: hist.min,
            RES_MAX: hist.max,
            RES_DATA_MIN: stats.min,
            RES_DATA_MAX: stats.max,
            RES_MEDIAN: stats.median,
            RES_MEAN: stats.mean,
            RES_SIGMA: stats.sigma,
            RES_MAD: stats.mad,
            RES_TOTAL_PIXELS: stats.valid_count,
            RES_AUTO_STF: {
                RES_SHADOW: stf_params.shadow,
                RES_MIDTONE: stf_params.midtone,
                RES_HIGHLIGHT: stf_params.highlight,
            },
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn compute_fft_spectrum(path: String) -> Result<Response, String> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<Response> {
        let t0 = Instant::now();
        let fft_result = compute_power_spectrum(load_cached(&path)?.arr())?;
        let spectrum = &fft_result.spectrum;
        let (rows, cols) = spectrum.dim();
        let pixel_count = rows * cols;

        let slice = spectrum.as_slice().expect("FFT spectrum must be contiguous");

        let (min_val, max_val) = if pixel_count > PAR_THRESHOLD {
            (
                slice.par_iter().cloned().reduce(|| f32::INFINITY, f32::min),
                slice.par_iter().cloned().reduce(|| f32::NEG_INFINITY, f32::max),
            )
        } else {
            slice.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(mn, mx), &v| (mn.min(v), mx.max(v)))
        };

        let range = (max_val - min_val).max(1e-10);
        let inv_range = 255.0 / range;
        let dc = spectrum[[rows / 2, cols / 2]];
        let elapsed_ms = t0.elapsed().as_millis() as u32;

        let header_size = 32;
        let mut buf = Vec::with_capacity(header_size + pixel_count);

        buf.extend_from_slice(&(cols as u32).to_le_bytes());
        buf.extend_from_slice(&(rows as u32).to_le_bytes());
        buf.extend_from_slice(&dc.to_le_bytes());
        buf.extend_from_slice(&max_val.to_le_bytes());
        buf.extend_from_slice(&elapsed_ms.to_le_bytes());
        buf.extend_from_slice(&(fft_result.original_size as u32).to_le_bytes());
        buf.extend_from_slice(&if fft_result.windowed { 1u32 } else { 0u32 }.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());

        let pixels: Vec<u8> = if pixel_count > PAR_THRESHOLD {
            slice.par_iter().map(|&v| ((v - min_val) * inv_range) as u8).collect()
        } else {
            slice.iter().map(|&v| ((v - min_val) * inv_range) as u8).collect()
        };
        buf.extend(pixels);

        Ok(Response::new(buf))
    })
        .await
        .map_err(|e| format!("{}", e))?
        .map_err(|e| format!("{:#}", e))
}

#[tauri::command]
pub async fn detect_stars(
    path: String,
    sigma: f64,
    max_stars: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let mut result = detect_stars_core(load_cached(&path)?.arr(), sigma);
        result.stars.truncate(max_stars);
        let mut val = serde_json::to_value(&result)?;
        if let Some(obj) = val.as_object_mut() {
            obj.insert(RES_ELAPSED_MS.to_string(), json!(t0.elapsed().as_millis() as u64));
        }
        Ok(val)
    })
}

#[tauri::command]
pub async fn detect_stars_composite(
    sigma: f64,
    max_stars: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let (er, eg, eb) = crate::cmd::helpers::load_composite_rgb()
            .map_err(|_| anyhow::anyhow!("RGB composite not available. Run Compose RGB first."))?;

        let r = er.arr();
        let g = eg.arr();
        let b = eb.arr();
        let (rows, cols) = r.dim();

        let r_s = r.as_slice().unwrap();
        let g_s = g.as_slice().unwrap();
        let b_s = b.as_slice().unwrap();

        let n = rows * cols;
        let lum_vec: Vec<f32> = if n > PAR_THRESHOLD {
            (0..n).into_par_iter()
                .map(|i| r_s[i] * 0.2126 + g_s[i] * 0.7152 + b_s[i] * 0.0722)
                .collect()
        } else {
            (0..n)
                .map(|i| r_s[i] * 0.2126 + g_s[i] * 0.7152 + b_s[i] * 0.0722)
                .collect()
        };
        let lum = ndarray::Array2::from_shape_vec((rows, cols), lum_vec)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let mut result = detect_stars_core(&lum, sigma);
        result.stars.truncate(max_stars);
        let mut val = serde_json::to_value(&result)?;
        if let Some(obj) = val.as_object_mut() {
            obj.insert(RES_ELAPSED_MS.to_string(), json!(t0.elapsed().as_millis() as u64));
        }
        Ok(val)
    })
}
