// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
    Json,
};
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use ndarray::Array2;
use serde::Deserialize;
use serde_json::json;

use astroburst_lib::core::imaging::stf::{apply_stf, auto_stf};
use astroburst_lib::types::image::{AutoStfConfig, ImageStats, StfParams};

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::state::AppState;

fn encode_png_l8(pixels: &[u8], width: usize, height: usize) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(width * height);
    let encoder = PngEncoder::new_with_quality(
        std::io::Cursor::new(&mut buf),
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    );
    encoder.write_image(pixels, width as u32, height as u32, ColorType::L8.into())?;
    Ok(buf)
}

fn png_response(bytes: Vec<u8>) -> Response {
    (
        [(header::CONTENT_TYPE, "image/png")],
        bytes,
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct RenderParams {
    pub slot: String,
    pub shadow: f64,
    pub midtone: f64,
    pub highlight: f64,
}

#[derive(Deserialize)]
pub struct StfParams_ {
    pub slot: String,
}

#[derive(Deserialize)]
pub struct ViewportParams {
    pub slot: String,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub shadow: Option<f64>,
    pub midtone: Option<f64>,
    pub highlight: Option<f64>,
}

pub async fn render(
    SessionExtractor(session): SessionExtractor,
    State(_state): State<AppState>,
    Json(params): Json<RenderParams>,
) -> Result<Response> {
    let entry = session
        .cache
        .get(&params.slot)
        .ok_or_else(|| AppError::NotFound(format!("slot {} not in cache", params.slot)))?;

    let stf = StfParams {
        shadow: params.shadow,
        midtone: params.midtone,
        highlight: params.highlight,
    };

    let arr = entry.data_arc();
    let stats = entry.stats().clone();

    let bytes = tokio::task::spawn_blocking(move || {
        let pixels = apply_stf(&arr, &stf, &stats);
        let (rows, cols) = arr.dim();
        encode_png_l8(&pixels, cols, rows)
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    .map_err(AppError::Internal)?;

    Ok(png_response(bytes))
}

pub async fn auto_render(
    SessionExtractor(session): SessionExtractor,
    State(_state): State<AppState>,
    Json(params): Json<StfParams_>,
) -> Result<Response> {
    let entry = session
        .cache
        .get(&params.slot)
        .ok_or_else(|| AppError::NotFound(format!("slot {} not in cache", params.slot)))?;

    let stf = auto_stf(entry.stats(), &AutoStfConfig::default());
    let arr = entry.data_arc();
    let stats = entry.stats().clone();
    let stf_copy = stf;

    let bytes = tokio::task::spawn_blocking(move || {
        let pixels = apply_stf(&arr, &stf_copy, &stats);
        let (rows, cols) = arr.dim();
        encode_png_l8(&pixels, cols, rows)
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    .map_err(AppError::Internal)?;

    let stf_json = serde_json::to_string(&json!({
        "shadow": stf.shadow, "midtone": stf.midtone, "highlight": stf.highlight,
    }))
    .unwrap_or_default();

    Ok((
        [
            (header::CONTENT_TYPE, "image/png".to_string()),
            ("x-stf-params".parse().unwrap(), stf_json),
        ],
        bytes,
    )
        .into_response())
}

fn viewport_pixels(crop: &Array2<f32>, explicit: Option<StfParams>, global_stats: &ImageStats) -> Vec<u8> {
    match explicit {
        Some(stf) => apply_stf(crop, &stf, global_stats),
        None => {
            let crop_stats = astroburst_lib::core::imaging::stats::compute_image_stats(crop);
            let stf = auto_stf(&crop_stats, &AutoStfConfig::default());
            apply_stf(crop, &stf, &crop_stats)
        }
    }
}

fn viewport_bounds(params: &ViewportParams, cols: usize, rows: usize) -> Result<(usize, usize, usize, usize)> {
    let (x, y, w, h) = (params.x, params.y, params.w, params.h);
    if w == 0 || h == 0 {
        return Err(AppError::BadRequest(format!("viewport size must be at least 1x1, got {w}x{h}")));
    }
    if x >= cols || y >= rows {
        return Err(AppError::BadRequest(format!(
            "viewport origin ({x},{y}) is outside image ({cols}×{rows})"
        )));
    }
    let x_end = x.checked_add(w).ok_or_else(|| AppError::BadRequest(format!("viewport width {w} is too large")))?;
    let y_end = y.checked_add(h).ok_or_else(|| AppError::BadRequest(format!("viewport height {h} is too large")))?;
    Ok((x, y, x_end.min(cols), y_end.min(rows)))
}

pub async fn viewport(
    SessionExtractor(session): SessionExtractor,
    State(_state): State<AppState>,
    Json(params): Json<ViewportParams>,
) -> Result<Response> {
    let entry = session
        .cache
        .get(&params.slot)
        .ok_or_else(|| AppError::NotFound(format!("slot {} not in cache", params.slot)))?;

    let arr = entry.data_arc();
    let global_stats = entry.stats().clone();
    let (rows, cols) = arr.dim();
    let (x, y, x_end, y_end) = viewport_bounds(&params, cols, rows)?;
    let shadow = params.shadow;
    let midtone = params.midtone;
    let highlight = params.highlight;

    let bytes = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let crop: Array2<f32> = arr.slice(ndarray::s![y..y_end, x..x_end]).to_owned();

        let explicit = match (shadow, midtone, highlight) {
            (Some(s), Some(m), Some(hi)) => Some(StfParams { shadow: s, midtone: m, highlight: hi }),
            _ => None,
        };

        let pixels = viewport_pixels(&crop, explicit, &global_stats);
        let (ch, cw) = crop.dim();
        encode_png_l8(&pixels, cw, ch)
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    .map_err(AppError::Internal)?;

    Ok(png_response(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use astroburst_lib::core::imaging::stats::compute_image_stats;

    fn full_image() -> Array2<f32> {
        let mut arr = Array2::from_shape_fn((64, 64), |(i, j)| 1000.0 + ((i * 7 + j * 13) % 50) as f32);
        arr[[0, 0]] = 65535.0;
        arr
    }

    fn crop_of(full: &Array2<f32>) -> Array2<f32> {
        full.slice(ndarray::s![8..40, 8..40]).to_owned()
    }

    fn median_byte(pixels: &[u8]) -> u8 {
        let mut v = pixels.to_vec();
        v.sort_unstable();
        v[v.len() / 2]
    }

    #[test]
    fn auto_viewport_stretch_uses_crop_normalisation() {
        let full = full_image();
        let global_stats = compute_image_stats(&full);
        let crop = crop_of(&full);

        let pixels = viewport_pixels(&crop, None, &global_stats);

        let target = (AutoStfConfig::default().target_bg * 255.0).round() as i32;
        let med = median_byte(&pixels) as i32;
        assert!((med - target).abs() <= 3, "median byte {med}, expected about {target}");
    }

    #[test]
    fn viewport_bounds_reject_empty_and_overflowing_requests_and_clamp_the_rest() {
        let params = |x, y, w, h| ViewportParams {
            slot: "s".into(),
            x,
            y,
            w,
            h,
            shadow: None,
            midtone: None,
            highlight: None,
        };
        for bad in [params(0, 0, 0, 4), params(0, 0, 4, 0), params(1, 0, usize::MAX, 4), params(0, 1, 4, usize::MAX), params(8, 0, 1, 1)] {
            assert!(matches!(viewport_bounds(&bad, 8, 6), Err(AppError::BadRequest(_))));
        }
        assert_eq!(viewport_bounds(&params(6, 4, 10, 10), 8, 6).unwrap(), (6, 4, 8, 6));
    }

    #[test]
    fn explicit_viewport_params_use_global_normalisation() {
        let full = full_image();
        let global_stats = compute_image_stats(&full);
        let crop = crop_of(&full);
        let stf = auto_stf(&global_stats, &AutoStfConfig::default());

        let pixels = viewport_pixels(&crop, Some(stf), &global_stats);
        let full_pixels = apply_stf(&full, &stf, &global_stats);
        let fp = &full_pixels;
        let expected: Vec<u8> = (8..40)
            .flat_map(|i| (8..40).map(move |j| fp[i * 64 + j]))
            .collect();

        assert_eq!(pixels, expected);
    }
}
