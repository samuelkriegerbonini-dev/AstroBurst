// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::core::imaging::stats::{finite_slice_stats, percentile};
use astroburst_lib::math::sigma_clipped_stats;

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::Session;

use super::region::{region_values, RegionSpec};

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
    let wcs = entry.header().and_then(|h| WcsTransform::from_header(h).ok());
    let values = region_values(arr, params.region.as_ref(), wcs.as_ref())?;
    let mut finite = values.finite;
    let n_nan = values.n_nan;
    let base = finite_slice_stats(&mut finite);

    let mut body = json!({
        "ref": target,
        "region": values.region,
        "min": base.min,
        "max": base.max,
        "median": base.median,
        "mad": base.mad,
        "sigma": base.sigma,
        "mean": base.mean,
        "valid_count": base.valid_count,
        "n_nan": n_nan,
    });

    if let Some(shape) = &values.shape {
        body["sum"] = json!(finite.iter().map(|&v| v as f64).sum::<f64>());
        body["area"] = json!(shape.area());
    }

    if let Some(sc) = &params.sigma_clip {
        let mut vals = finite.clone();
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

#[cfg(test)]
mod tests {
    use astroburst_lib::core::imaging::stats::compute_image_stats;
    use astroburst_lib::types::constants::MAD_TO_SIGMA;
    use ndarray::Array2;

    use super::*;

    #[test]
    fn finite_stats_keeps_zero_and_negative_pixels() {
        let region = Array2::from_shape_vec((2, 3), vec![-5.0f32, -1.0, 0.0, 2.0, 6.0, f32::NAN]).unwrap();
        let mut finite: Vec<f32> = region.iter().copied().filter(|v| v.is_finite()).collect();
        let s = finite_slice_stats(&mut finite);

        assert_eq!(s.valid_count, 5);
        assert_eq!(s.min, -5.0);
        assert_eq!(s.max, 6.0);
        assert!((s.mean - 0.4).abs() < 1e-12);
        assert_eq!(s.median, 0.0);
        assert_eq!(s.mad, 2.0);
        assert!((s.sigma - 2.0 * MAD_TO_SIGMA).abs() < 1e-12);

        let legacy = compute_image_stats(&region);
        assert_eq!(legacy.valid_count, 2);
        assert_eq!(legacy.min, 2.0);
    }

    #[test]
    fn finite_stats_matches_core_for_positive_data_and_handles_empty() {
        let region = Array2::from_shape_vec(
            (4, 4),
            vec![
                100.0f32, 101.0, 102.0, 103.0,
                104.0, 105.0, 106.0, 107.0,
                108.0, 109.0, 110.0, 111.0,
                112.0, 8000.0, 9000.0, f32::NAN,
            ],
        )
        .unwrap();
        let mut finite: Vec<f32> = region.iter().copied().filter(|v| v.is_finite()).collect();
        let ours = finite_slice_stats(&mut finite);
        let core = compute_image_stats(&region);
        assert_eq!(ours.valid_count, core.valid_count);
        assert_eq!(ours.min, core.min);
        assert_eq!(ours.max, core.max);
        assert_eq!(ours.median, core.median);
        assert_eq!(ours.mad, core.mad);
        assert!((ours.mean - core.mean).abs() < 1e-9);

        let empty = finite_slice_stats(&mut []);
        assert_eq!(empty.valid_count, 0);
    }
}
