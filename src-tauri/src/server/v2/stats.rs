// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::core::imaging::region::RegionShape;
use astroburst_lib::core::imaging::statistics::{
    evaluate_noise, evaluate_noise_in_region, evaluate_noise_window, statistics_from_finite,
    RegionNoise,
};
use astroburst_lib::core::imaging::stats::{finite_slice_stats, percentile};
use astroburst_lib::infra::cache::ImageEntry;
use astroburst_lib::math::sigma_clipped_stats;
use ndarray::{s, Array2};

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::Session;

use super::region::{region_values, RegionSpec};

const MAX_SIGMA_CLIP_ITERS: usize = 100;
const MAX_PERCENTILES: usize = 100;

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
    #[serde(default)]
    pub noise: bool,
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

fn pixel_window(arr: &Array2<f32>, region: &Value) -> Option<Array2<f32>> {
    let x = region.get("x")?.as_u64()? as usize;
    let y = region.get("y")?.as_u64()? as usize;
    let width = region.get("width")?.as_u64()? as usize;
    let height = region.get("height")?.as_u64()? as usize;
    let (rows, cols) = arr.dim();
    if width == 0 || height == 0 || x + width > cols || y + height > rows {
        return None;
    }
    Some(arr.slice(s![y..y + height, x..x + width]).to_owned())
}

fn noise_for_request(
    arr: &Array2<f32>,
    shape: Option<&RegionShape>,
    region: &Value,
    explicit_region: bool,
) -> Result<RegionNoise> {
    if let Some(shape) = shape {
        return evaluate_noise_in_region(arr, shape, None)
            .map_err(|e| AppError::BadRequest(format!("noise evaluation: {e:#}")));
    }
    if !explicit_region {
        return Ok(RegionNoise::Evaluated(evaluate_noise(arr)));
    }
    Ok(match pixel_window(arr, region) {
        Some(window) => evaluate_noise_window(&window),
        None => RegionNoise::Evaluated(evaluate_noise(arr)),
    })
}

fn rect_pixel_count(region: &Value) -> Option<u64> {
    let width = region.get("width")?.as_u64()?;
    let height = region.get("height")?.as_u64()?;
    Some(width * height)
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

fn bad_param(message: String, hint: &str) -> AppError {
    AppError::BadRequestWithHint { code: "bad_request", message, hint: Some(hint.into()) }
}

fn validate_stats_params(params: &StatsParams) -> Result<()> {
    if let Some(sc) = &params.sigma_clip {
        if !(1..=MAX_SIGMA_CLIP_ITERS).contains(&sc.maxiters) {
            return Err(bad_param(
                format!("sigma_clip.maxiters must be between 1 and {MAX_SIGMA_CLIP_ITERS}, got {}", sc.maxiters),
                "5 is the default",
            ));
        }
        if !(sc.sigma.is_finite() && sc.sigma > 0.0) {
            return Err(bad_param(
                format!("sigma_clip.sigma must be a finite number greater than 0, got {}", sc.sigma),
                "3.0 is the default",
            ));
        }
    }
    if params.percentiles.len() > MAX_PERCENTILES {
        return Err(bad_param(
            format!("at most {MAX_PERCENTILES} percentiles per request, got {}", params.percentiles.len()),
            "split the request",
        ));
    }
    if let Some(p) = params.percentiles.iter().find(|p| !(p.is_finite() && (0.0..=100.0).contains(*p))) {
        return Err(bad_param(format!("percentile {p} is outside 0..100"), "percentiles are given in percent"));
    }
    Ok(())
}

pub async fn stats(
    SessionExtractor(session): SessionExtractor,
    Json(mut params): Json<StatsParams>,
) -> Result<Json<Value>> {
    validate_stats_params(&params)?;
    let target = target_ref(&session, params.image_ref.take()).await?;
    let entry = session
        .cache
        .get(&target)
        .ok_or_else(|| AppError::NotFound(format!("image ref {target} not found in session")))?;
    tokio::task::spawn_blocking(move || stats_body(&entry, target, &params))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
        .map(Json)
}

fn stats_body(entry: &ImageEntry, target: String, params: &StatsParams) -> Result<Value> {
    let arr = entry.arr();
    let wcs = entry.header().and_then(|h| WcsTransform::from_header(h).ok());
    let values = region_values(arr, params.region.as_ref(), wcs.as_ref())?;
    let mut finite = values.finite;
    let n_nan = match values.shape {
        Some(_) => values.n_nan,
        None => rect_pixel_count(&values.region)
            .map(|total| total.saturating_sub(finite.len() as u64))
            .unwrap_or(values.n_nan),
    };
    let exact = statistics_from_finite(finite.clone(), finite.len() as u64 + n_nan, n_nan, 0);
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
        "avg_dev": exact.avg_dev,
        "bwmv_sqrt": exact.bwmv_sqrt,
        "std_dev": exact.std_dev,
        "count_fraction": exact.fraction,
        "nan_count": exact.nan_count,
    });

    if params.noise {
        let explicit_region = params.region.is_some();
        let noise = noise_for_request(arr, values.shape.as_ref(), &body["region"], explicit_region)?;
        body["noise"] = serde_json::to_value(noise.evaluation())
            .map_err(|e| AppError::Internal(anyhow::anyhow!("serialising noise evaluation: {e}")))?;
        body["noise_note"] = json!(noise.note());
    }

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
        let mean = (n_survivors > 0).then(|| vals.iter().map(|&v| v as f64).sum::<f64>() / n_survivors as f64);
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

    Ok(body)
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

        let core = compute_image_stats(&region);
        assert_eq!(core.valid_count, 4);
        assert_eq!(core.min, -5.0);
    }

    #[test]
    fn sigma_clip_on_an_all_nan_image_reports_no_measurement() {
        let cache = astroburst_lib::infra::cache::ImageCache::new(4, 1 << 20);
        let arr = Array2::from_elem((3, 3), f32::NAN);
        let stats = compute_image_stats(&arr);
        cache.insert_synthetic("all-nan", std::sync::Arc::new(arr), stats);
        let entry = cache.get("all-nan").unwrap();
        let params = StatsParams {
            image_ref: None,
            region: None,
            sigma_clip: Some(SigmaClipParams { sigma: 3.0, maxiters: 5 }),
            percentiles: Vec::new(),
            noise: false,
        };

        let body = stats_body(&entry, "all-nan".into(), &params).unwrap();

        assert_eq!(body["clipped"], json!({ "mean": null, "median": null, "std": null, "n_rejected": 0 }));
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
