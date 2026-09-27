use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk};
use crate::core::imaging::psf_estimation::{
    estimate_psf, PsfEstimationConfig, PsfResult,
};
use crate::types::constants::{
    RES_X, RES_Y, RES_PEAK, RES_FLUX, RES_FWHM, RES_ELLIPTICITY, RES_SNR,
    RES_KERNEL_SIZE, RES_AVERAGE_FWHM, RES_AVERAGE_ELLIPTICITY,
    RES_SPREAD_PIXELS, RES_STARS_USED, RES_STARS_REJECTED, RES_KERNEL,
};

pub(crate) fn psf_config(
    num_stars: Option<usize>,
    cutout_radius: Option<usize>,
    saturation_threshold: Option<f64>,
    max_ellipticity: Option<f64>,
) -> PsfEstimationConfig {
    PsfEstimationConfig {
        num_stars: num_stars.unwrap_or(30),
        cutout_radius: cutout_radius.unwrap_or(15),
        saturation_threshold: saturation_threshold.unwrap_or(0.95),
        min_peak_fraction: 0.10,
        max_ellipticity: max_ellipticity.unwrap_or(0.3),
        edge_margin: 30,
        max_center_distance_fraction: 0.7,
    }
}

pub(crate) fn psf_result_json(result: &PsfResult) -> serde_json::Value {
    let stars_json: Vec<serde_json::Value> = result.stars_used.iter().map(|s| {
        json!({
            RES_X: s.x,
            RES_Y: s.y,
            RES_PEAK: s.peak,
            RES_FLUX: s.flux,
            RES_FWHM: s.fwhm,
            RES_ELLIPTICITY: s.ellipticity,
            RES_SNR: s.snr,
        })
    }).collect();

    json!({
        RES_KERNEL_SIZE: result.kernel_size,
        RES_AVERAGE_FWHM: result.average_fwhm,
        RES_AVERAGE_ELLIPTICITY: result.average_ellipticity,
        RES_SPREAD_PIXELS: result.spread_pixels,
        RES_STARS_USED: stars_json,
        RES_STARS_REJECTED: result.stars_rejected,
        RES_KERNEL: result.kernel,
    })
}

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

        let config = psf_config(num_stars, cutout_radius, saturation_threshold, max_ellipticity);

        let result = estimate_psf(entry.arr(), &config)
            .map_err(|e| anyhow::anyhow!(e))?;

        Ok(psf_result_json(&result))
    })
}
