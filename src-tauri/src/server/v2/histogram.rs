// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::Json;
use ndarray::{s, Array2};
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::core::imaging::stats::{build_histogram, is_valid_pixel, percentile};

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::Session;

use super::region::{resolve_region, RegionSpec, ResolvedRegion};

const AUTO_LO_PCT: f64 = 0.001;
const AUTO_HI_PCT: f64 = 0.999;

#[derive(Deserialize)]
pub struct HistogramParams {
    #[serde(default, alias = "ref")]
    pub image_ref: Option<String>,
    #[serde(default)]
    pub region: Option<RegionSpec>,
    #[serde(default = "default_bins")]
    pub bins: usize,
    #[serde(default)]
    pub range: Option<[f64; 2]>,
    #[serde(default)]
    pub log_counts: bool,
    #[serde(default)]
    pub render_png: Option<bool>,
}

fn default_bins() -> usize {
    256
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

pub async fn histogram(
    SessionExtractor(session): SessionExtractor,
    Json(params): Json<HistogramParams>,
) -> Result<Json<Value>> {
    if params.render_png == Some(true) {
        return Err(AppError::BadRequestWithHint {
            code: "not_implemented",
            message: "render_png is not implemented for the histogram endpoint yet".into(),
            hint: Some("omit render_png and plot the returned bins/bin_edges yourself".into()),
        });
    }
    if params.bins == 0 {
        return Err(AppError::BadRequestWithHint {
            code: "bad_request",
            message: "bins must be > 0".into(),
            hint: None,
        });
    }

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

    let (dmin, dmax, range_source) = match params.range {
        Some([lo, hi]) => (lo, hi, "explicit"),
        None => {
            let mut valid: Vec<f32> = slice.iter().copied().filter(|&v| is_valid_pixel(v)).collect();
            if valid.is_empty() {
                (0.0, 0.0, "auto")
            } else {
                let lo = percentile(&mut valid, AUTO_LO_PCT) as f64;
                let hi = percentile(&mut valid, AUTO_HI_PCT) as f64;
                (lo, hi, "auto")
            }
        }
    };

    let hist = build_histogram(slice, params.bins, dmin, dmax);

    let mode = hist
        .bins
        .iter()
        .enumerate()
        .max_by_key(|(_, &c)| c)
        .filter(|(_, &c)| c > 0)
        .map(|(i, _)| (hist.bin_edges[i] + hist.bin_edges[i + 1]) / 2.0);

    let counts: Value = if params.log_counts {
        json!(hist
            .bins
            .iter()
            .map(|&c| (c as f64 + 1.0).ln())
            .collect::<Vec<f64>>())
    } else {
        json!(hist.bins)
    };

    Ok(Json(json!({
        "ref": target,
        "region": resolved,
        "bins": counts,
        "bin_edges": hist.bin_edges,
        "min": hist.min,
        "max": hist.max,
        "log_counts": params.log_counts,
        "range_source": range_source,
        "mode": mode,
    })))
}
