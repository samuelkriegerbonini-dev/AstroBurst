// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::Json;
use ndarray::{s, Array2};
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::core::imaging::stats::{compute_image_stats, percentile};
use astroburst_lib::math::sigma_clipped_stats;

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::Session;

use super::region::{resolve_region, RegionSpec, ResolvedRegion};

#[derive(Deserialize)]
pub struct StatsParams {
    #[serde(default, alias = "ref")]
    pub image_ref: Option<String>,
    #[serde(default)]
    pub region: Option<RegionSpec>,
    #[serde(default)]
    pub sigma_clip: Option<SigmaClipParams>,
    #[serde(default)]
    pub percentiles: Vec<f64>,
}

#[derive(Deserialize)]
pub struct SigmaClipParams {
    #[serde(default = "default_sigma")]
    pub sigma: f32,
    #[serde(default = "default_maxiters")]
    pub maxiters: usize,
}

fn default_sigma() -> f32 {
    3.0
}
fn default_maxiters() -> usize {
    5
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

pub async fn stats(
    SessionExtractor(session): SessionExtractor,
    Json(params): Json<StatsParams>,
) -> Result<Json<Value>> {
    let target = target_ref(&session, params.image_ref).await?;
    let entry = session
        .cache
        .get(&target)
        .ok_or_else(|| AppError::NotFound(format!("image ref {target} not found in session")))?;
    let arr = entry.arr();
    let (rows, cols) = arr.dim();

    let (region_arr, resolved): (Array2<f32>, ResolvedRegion) = match &params.region {
        Some(spec) => {
            let wcs = entry.header().and_then(|h| WcsTransform::from_header(h).ok());
            let r = resolve_region(spec, cols, rows, wcs.as_ref())?;
            let sub = arr
                .slice(s![r.y..r.y + r.height, r.x..r.x + r.width])
                .to_owned();
            (sub, r)
        }
        None => (
            arr.to_owned(),
            ResolvedRegion { x: 0, y: 0, width: cols, height: rows, clipped: false },
        ),
    };

    let slice = region_arr
        .as_slice()
        .expect("region_arr is standard-layout after to_owned()");
    let n_nan = slice.iter().filter(|v| v.is_nan()).count() as u64;

    let base = compute_image_stats(&region_arr);

    let mut body = json!({
        "ref": target,
        "region": resolved,
        "min": base.min,
        "max": base.max,
        "median": base.median,
        "mad": base.mad,
        "sigma": base.sigma,
        "mean": base.mean,
        "valid_count": base.valid_count,
        "n_nan": n_nan,
    });

    if let Some(sc) = &params.sigma_clip {
        let mut vals: Vec<f32> = slice.iter().copied().filter(|v| v.is_finite()).collect();
        let n_input = vals.len();
        let (median, std) = sigma_clipped_stats(&mut vals, sc.sigma, sc.maxiters);
        let n_survivors = vals.len();
        let n_rejected = n_input - n_survivors;
        let mean = if n_survivors > 0 {
            vals.iter().map(|&v| v as f64).sum::<f64>() / n_survivors as f64
        } else {
            0.0
        };
        body["clipped"] = json!({
            "mean": mean,
            "median": median,
            "std": std,
            "n_rejected": n_rejected,
        });
    }

    if !params.percentiles.is_empty() {
        let mut finite: Vec<f32> = slice.iter().copied().filter(|v| v.is_finite()).collect();
        let results: Vec<Value> = params
            .percentiles
            .iter()
            .map(|&p| {
                let v = percentile(&mut finite, p / 100.0);
                json!({ "percentile": p, "value": v })
            })
            .collect();
        body["percentiles"] = json!(results);
    }

    Ok(Json(body))
}
