use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk};
use crate::core::imaging::psf_estimation::{
    estimate_psf, PsfEstimationConfig,
};

#[tauri::command]
pub async fn estimate_psf_cmd(
    path: String,
    num_stars: Option<usize>,
    cutout_radius: Option<usize>,
    saturation_threshold: Option<f64>,
    max_ellipticity: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let entry = load_from_cache_or_disk(&path)?;

        let config = PsfEstimationConfig {
            num_stars: num_stars.unwrap_or(3),
            cutout_radius: cutout_radius.unwrap_or(15),
            saturation_threshold: saturation_threshold.unwrap_or(0.95),
            min_peak_fraction: 0.10,
            max_ellipticity: max_ellipticity.unwrap_or(0.3),
            edge_margin: 30,
            max_center_distance_fraction: 0.7,
        };

        let result = estimate_psf(entry.arr(), &config)
            .map_err(|e| anyhow::anyhow!(e))?;

        let stars_json: Vec<serde_json::Value> = result.stars_used.iter().map(|s| {
            json!({
                "x": s.x,
                "y": s.y,
                "peak": s.peak,
                "flux": s.flux,
                "fwhm": s.fwhm,
                "ellipticity": s.ellipticity,
                "snr": s.snr,
            })
        }).collect();

        Ok(json!({
            "kernel_size": result.kernel_size,
            "average_fwhm": result.average_fwhm,
            "average_ellipticity": result.average_ellipticity,
            "spread_pixels": result.spread_pixels,
            "stars_used": stars_json,
            "stars_rejected": result.stars_rejected,
            "kernel": result.kernel,
        }))
    })
}
