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
use serde_json::{json, Value};

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

    let x = params.x;
    let y = params.y;
    let w = params.w;
    let h = params.h;
    let shadow = params.shadow;
    let midtone = params.midtone;
    let highlight = params.highlight;

    let bytes = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let (rows, cols) = arr.dim();
        let x_end = (x + w).min(cols);
        let y_end = (y + h).min(rows);
        if x >= cols || y >= rows {
            anyhow::bail!("viewport origin ({x},{y}) is outside image ({cols}×{rows})");
        }

        let crop: Array2<f32> = arr.slice(ndarray::s![y..y_end, x..x_end]).to_owned();

        let stf = match (shadow, midtone, highlight) {
            (Some(s), Some(m), Some(hi)) => StfParams { shadow: s, midtone: m, highlight: hi },
            _ => {
                let crop_stats = astroburst_lib::core::imaging::stats::compute_image_stats(&crop);
                auto_stf(&crop_stats, &AutoStfConfig::default())
            }
        };

        let pixels = apply_stf(&crop, &stf, &global_stats);
        let (ch, cw) = crop.dim();
        encode_png_l8(&pixels, cw, ch)
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    .map_err(AppError::Internal)?;

    Ok(png_response(bytes))
}
