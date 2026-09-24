// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::Json;
use rayon::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::core::imaging::stats::percentile;
use astroburst_lib::infra::cache::ImageEntry;
use astroburst_lib::types::constants::HISTOGRAM_BINS;
use astroburst_lib::types::image::Histogram;

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::Session;

use super::region::{region_values, RegionSpec};

const AUTO_LO_PCT: f64 = 0.001;
const AUTO_HI_PCT: f64 = 0.999;
const HIST_CHUNK: usize = 65536;

fn check_bins(bins: usize) -> Result<()> {
    if bins == 0 || bins > HISTOGRAM_BINS {
        return Err(AppError::BadRequestWithHint {
            code: "bad_request",
            message: format!("bins must be between 1 and {HISTOGRAM_BINS}, got {bins}"),
            hint: Some("256 is the default".into()),
        });
    }
    Ok(())
}

fn finite_histogram(slice: &[f32], bins: usize, dmin: f64, dmax: f64) -> Histogram {
    let range = dmax - dmin;
    if range < 1e-10 {
        return Histogram {
            bins: vec![0u32; bins],
            bin_edges: vec![dmin; bins + 1],
            min: dmin,
            max: dmax,
        };
    }

    let inv_bin_width = bins as f64 / range;
    let last = bins - 1;
    let counts = slice
        .par_chunks(HIST_CHUNK)
        .fold(
            || vec![0u32; bins],
            |mut local, chunk| {
                for &v in chunk {
                    let vd = v as f64;
                    if vd >= dmin && vd <= dmax {
                        let idx = ((vd - dmin) * inv_bin_width) as usize;
                        local[idx.min(last)] += 1;
                    }
                }
                local
            },
        )
        .reduce_with(|mut a, b| {
            for (ai, bi) in a.iter_mut().zip(b.iter()) {
                *ai += bi;
            }
            a
        })
        .unwrap_or_else(|| vec![0u32; bins]);

    let step = range / bins as f64;
    let bin_edges: Vec<f64> = (0..=bins).map(|i| dmin + i as f64 * step).collect();

    Histogram {
        bins: counts,
        bin_edges,
        min: dmin,
        max: dmax,
    }
}

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

fn check_range(range: Option<[f64; 2]>) -> Result<()> {
    match range {
        Some([lo, hi]) if !(lo.is_finite() && hi.is_finite() && lo < hi) => Err(AppError::BadRequestWithHint {
            code: "bad_request",
            message: format!("range must be two finite numbers with range[0] < range[1], got [{lo}, {hi}]"),
            hint: Some("omit range to use the 0.1..99.9 percentile range of the data".into()),
        }),
        _ => Ok(()),
    }
}

pub async fn histogram(
    SessionExtractor(session): SessionExtractor,
    Json(mut params): Json<HistogramParams>,
) -> Result<Json<Value>> {
    if params.render_png == Some(true) {
        return Err(AppError::BadRequestWithHint {
            code: "not_implemented",
            message: "render_png is not implemented for the histogram endpoint yet".into(),
            hint: Some("omit render_png and plot the returned bins/bin_edges yourself".into()),
        });
    }
    check_bins(params.bins)?;
    check_range(params.range)?;

    let target = target_ref(&session, params.image_ref.take()).await?;
    let entry = session
        .cache
        .get(&target)
        .ok_or_else(|| AppError::NotFound(format!("image ref {target} not found in session")))?;
    tokio::task::spawn_blocking(move || histogram_body(&entry, target, &params))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
        .map(Json)
}

fn histogram_body(entry: &ImageEntry, target: String, params: &HistogramParams) -> Result<Value> {
    let arr = entry.arr();
    let wcs = entry.header().and_then(|h| WcsTransform::from_header(h).ok());
    let values = region_values(arr, params.region.as_ref(), wcs.as_ref())?;
    let slice: &[f32] = &values.finite;

    let (dmin, dmax, range_source) = match params.range {
        Some([lo, hi]) => (lo, hi, "explicit"),
        None => {
            let mut valid: Vec<f32> = slice.iter().copied().filter(|v| v.is_finite()).collect();
            if valid.is_empty() {
                (0.0, 0.0, "auto")
            } else {
                let lo = percentile(&mut valid, AUTO_LO_PCT) as f64;
                let hi = percentile(&mut valid, AUTO_HI_PCT) as f64;
                (lo, hi, "auto")
            }
        }
    };

    let hist = finite_histogram(slice, params.bins, dmin, dmax);

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

    Ok(json!({
        "ref": target,
        "region": values.region,
        "bins": counts,
        "bin_edges": hist.bin_edges,
        "min": hist.min,
        "max": hist.max,
        "log_counts": params.log_counts,
        "range_source": range_source,
        "mode": mode,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use astroburst_lib::core::imaging::stats::build_histogram;

    fn code_of(e: &AppError) -> Option<&'static str> {
        match e {
            AppError::BadRequestWithHint { code, .. } => Some(code),
            _ => None,
        }
    }

    #[test]
    fn bins_outside_1_to_max_are_rejected_before_allocation() {
        for bins in [0usize, HISTOGRAM_BINS + 1, 10_000_000_000, 1 << 62, usize::MAX] {
            let err = check_bins(bins).unwrap_err();
            assert_eq!(code_of(&err), Some("bad_request"), "bins {bins}");
        }
        assert!(check_bins(1).is_ok());
        assert!(check_bins(256).is_ok());
        assert!(check_bins(HISTOGRAM_BINS).is_ok());
    }

    #[test]
    fn finite_histogram_counts_zero_and_negative_pixels() {
        let data = [-5.0f32, -1.0, 0.0, 2.0, 6.0, f32::NAN, f32::INFINITY];
        let hist = finite_histogram(&data, 4, -10.0, 10.0);
        assert_eq!(hist.bins, vec![0, 2, 2, 1]);
        assert_eq!(hist.bins.iter().sum::<u32>(), 5);
        assert_eq!(hist.bin_edges, vec![-10.0, -5.0, 0.0, 5.0, 10.0]);

        let core = build_histogram(&data, 4, -10.0, 10.0);
        assert_eq!(core.bins, vec![0, 2, 1, 1]);
        assert_eq!(core.bins.iter().sum::<u32>(), 4);
    }

    #[test]
    fn finite_histogram_skips_values_outside_the_range_and_keeps_the_upper_edge() {
        let data = [-1.0f32, 0.0, 1.0, 5.0, 10.0, 10.5, 1e6, f32::NEG_INFINITY];
        let hist = finite_histogram(&data, 2, 0.0, 10.0);
        assert_eq!(hist.bins, vec![2, 2]);
    }

    #[test]
    fn explicit_range_must_be_finite_and_increasing() {
        for range in [[10.0, 0.0], [5.0, 5.0], [f64::NAN, 1.0], [0.0, f64::INFINITY]] {
            assert_eq!(code_of(&check_range(Some(range)).unwrap_err()), Some("bad_request"), "{range:?}");
        }
        assert!(check_range(Some([0.0, 1e-3])).is_ok());
        assert!(check_range(None).is_ok());
    }

    #[test]
    fn finite_histogram_matches_core_binning_for_positive_data() {
        let data: Vec<f32> = (1..=16).map(|i| i as f32).collect();
        let ours = finite_histogram(&data, 8, 1.0, 16.0);
        let core = build_histogram(&data, 8, 1.0, 16.0);
        assert_eq!(ours.bins, core.bins);
        assert_eq!(ours.bin_edges, core.bin_edges);
        assert_eq!(ours.min, core.min);
        assert_eq!(ours.max, core.max);

        let flat = finite_histogram(&data, 3, 2.0, 2.0);
        assert_eq!(flat.bins, vec![0, 0, 0]);
        assert_eq!(flat.bin_edges.len(), 4);
    }
}
