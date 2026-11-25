use std::time::Instant;
use serde_json::json;
use rayon::prelude::*;
use crate::cmd::common::{blocking_cmd, load_cached};
use crate::types::constants::{RES_ELAPSED_MS, RES_SUBFRAMES, RES_TOTAL, RES_ACCEPTED, RES_REJECTED};

#[tauri::command]
pub async fn analyze_subframes_cmd(
    paths: Vec<String>,
    max_fwhm: Option<f64>,
    max_eccentricity: Option<f64>,
    min_snr: Option<f64>,
    min_stars: Option<usize>,
    fwhm_weight: Option<f64>,
    eccentricity_weight: Option<f64>,
    snr_weight: Option<f64>,
    noise_weight: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();

        let config = crate::core::analysis::subframe::SubframeWeightConfig {
            max_fwhm: max_fwhm.unwrap_or(8.0),
            max_eccentricity: max_eccentricity.unwrap_or(0.7),
            min_snr: min_snr.unwrap_or(5.0),
            min_stars: min_stars.unwrap_or(5),
            fwhm_weight: fwhm_weight.unwrap_or(1.0),
            eccentricity_weight: eccentricity_weight.unwrap_or(0.5),
            snr_weight: snr_weight.unwrap_or(1.0),
            noise_weight: noise_weight.unwrap_or(0.3),
        };

        let mut metrics: Vec<crate::core::analysis::subframe::SubframeMetrics> = paths
            .par_iter()
            .map(|p| {
                let entry = load_cached(p)?;
                Ok(crate::core::analysis::subframe::analyze_subframe(entry.arr(), p, &config))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        crate::core::analysis::subframe::normalize_weights(&mut metrics);

        let accepted = metrics.iter().filter(|m| m.accepted).count();
        let rejected = metrics.len() - accepted;

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_SUBFRAMES: metrics,
            RES_TOTAL: metrics.len(),
            RES_ACCEPTED: accepted,
            RES_REJECTED: rejected,
            RES_ELAPSED_MS: elapsed,
        }))
    })
}
