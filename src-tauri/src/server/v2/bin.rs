// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::sync::Arc;

use axum::Json;
use ndarray::{s, Array2};
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::alignment::downsample::area_downsample;
use astroburst_lib::core::imaging::stats::compute_image_stats;

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::{ImageMeta, Session};

fn block_mean(src: &Array2<f32>, factor: usize, out_rows: usize, out_cols: usize) -> Array2<f32> {
    let (in_rows, in_cols) = src.dim();
    if in_rows == out_rows * factor && in_cols == out_cols * factor {
        return area_downsample(src, out_rows, out_cols);
    }
    let cropped = src
        .slice(s![..out_rows * factor, ..out_cols * factor])
        .to_owned();
    area_downsample(&cropped, out_rows, out_cols)
}

#[derive(Deserialize)]
pub struct BinParams {
    pub factor: usize,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default, alias = "ref")]
    pub image_ref: Option<String>,
    pub name: Option<String>,
}

fn default_method() -> String {
    "mean".to_string()
}

async fn target_ref(session: &Session, explicit: Option<String>) -> Result<String> {
    match explicit {
        Some(r) => Ok(r),
        None => session
            .v2
            .active_ref
            .read()
            .await
            .clone()
            .ok_or_else(|| {
                AppError::BadRequest("no active image in this session; open a file first".into())
            }),
    }
}

pub async fn bin(
    SessionExtractor(session): SessionExtractor,
    Json(params): Json<BinParams>,
) -> Result<Json<Value>> {
    if params.method != "mean" {
        return Err(AppError::BadRequestWithHint {
            code: "bad_request",
            message: format!(
                "unsupported bin method {:?}; only \"mean\" is supported",
                params.method
            ),
            hint: Some(
                "sum-binning is inexact on edge blocks when dims aren't divisible by factor; \
                 use method \"mean\""
                    .into(),
            ),
        });
    }

    if params.factor == 0 {
        return Err(AppError::BadRequest(
            "factor must be a positive integer".into(),
        ));
    }

    let target = target_ref(&session, params.image_ref).await?;

    let entry = session.cache.get(&target).ok_or_else(|| {
        AppError::NotFound(format!("image ref {target} not found in session"))
    })?;
    let (in_rows, in_cols) = entry.arr().dim();

    let out_rows = in_rows / params.factor;
    let out_cols = in_cols / params.factor;
    if out_rows == 0 || out_cols == 0 {
        return Err(AppError::BadRequestWithHint {
            code: "bad_request",
            message: format!(
                "factor {} is larger than image dims {}x{}, producing an empty image",
                params.factor, in_cols, in_rows
            ),
            hint: Some("choose a factor no larger than the smaller image dimension".into()),
        });
    }

    let out_ref = params
        .name
        .clone()
        .unwrap_or_else(|| session.v2.next_ref("bin"));

    let src = entry.data_arc();
    let factor = params.factor;
    let binned = tokio::task::spawn_blocking(move || block_mean(&src, factor, out_rows, out_cols))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?;

    let stats = compute_image_stats(&binned);
    let arc = Arc::new(binned);
    session
        .cache
        .insert_synthetic(&out_ref, Arc::clone(&arc), stats.clone());

    let source = session.v2.meta.get(&target).and_then(|m| m.source.clone());
    session.v2.meta.insert(
        out_ref.clone(),
        ImageMeta {
            image_ref: out_ref.clone(),
            source,
            hdu: None,
            width: out_cols,
            height: out_rows,
            wcs_present: false,
            extname: None,
        },
    );
    session.prune_evicted_meta();
    *session.v2.active_ref.write().await = Some(out_ref.clone());

    Ok(Json(json!({
        "ref": out_ref,
        "active_ref": out_ref,
        "from_ref": target,
        "factor": params.factor,
        "method": "mean",
        "dims": [out_cols, out_rows],
        "wcs_present": false,
        "stats": {
            "min": stats.min, "max": stats.max, "median": stats.median,
            "mad": stats.mad, "sigma": stats.sigma, "mean": stats.mean,
            "valid_count": stats.valid_count,
        },
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_mean_uses_disjoint_factor_blocks_when_dims_are_not_divisible() {
        let row: Vec<f32> = (0..10).map(|i| i as f32).collect();
        let src = Array2::from_shape_fn((3, 10), |(_, x)| row[x]);
        let out = block_mean(&src, 3, 1, 3);
        assert_eq!(out.dim(), (1, 3));
        assert!((out[[0, 0]] - 1.0).abs() < 1e-6);
        assert!((out[[0, 1]] - 4.0).abs() < 1e-6);
        assert!((out[[0, 2]] - 7.0).abs() < 1e-6);

        let src = Array2::from_shape_fn((5, 7), |(y, x)| (y * 7 + x) as f32);
        let out = block_mean(&src, 2, 2, 3);
        assert_eq!(out.dim(), (2, 3));
        assert!((out[[0, 0]] - 4.0).abs() < 1e-6);
        assert!((out[[1, 2]] - 22.0).abs() < 1e-6);
    }

    #[test]
    fn block_mean_matches_area_downsample_when_dims_divide_exactly() {
        let src = Array2::from_shape_fn((4, 4), |(y, x)| (y * 4 + x) as f32);
        let out = block_mean(&src, 2, 2, 2);
        let expected = area_downsample(&src, 2, 2);
        assert_eq!(out, expected);
        assert!((out[[0, 0]] - 2.5).abs() < 1e-6);
    }
}
