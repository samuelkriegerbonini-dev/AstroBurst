use std::fs::File;

use serde_json::json;

use crate::cmd::common::blocking_cmd;
use crate::core::astrometry::wcs::WcsTransform;
use crate::infra::config;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::reader::extract_image_mmap;
use crate::types::constants::{
    DEFAULT_API_KEY_SERVICE, DEFAULT_ASTROMETRY_API_URL, HEADER_NAXIS1,
    HEADER_NAXIS2, RES_CENTER_DEC, RES_CENTER_RA, RES_FOV_ARCMIN,
    RES_FOV_H_ARCMIN, RES_FOV_W_ARCMIN, RES_NAXIS1, RES_NAXIS2,
    RES_PIXEL_SCALE_ARCSEC, RES_WCS_CD, RES_WCS_CRPIX1, RES_WCS_CRPIX2,
    RES_WCS_CRVAL1, RES_WCS_CRVAL2, RES_WCS_PARAMS, RES_WCS_PROJECTION,
};

const MAX_UPLOAD_DIM: usize = 2048;

fn load_header_and_wcs(path: &str) -> anyhow::Result<(crate::types::header::HduHeader, WcsTransform)> {
    let (fits_path, _tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)?;
    let result = extract_image_mmap(&file)?;
    let wcs = WcsTransform::from_header(&result.header)?;
    Ok((result.header, wcs))
}

fn resolve_api_key(provided: Option<String>) -> Option<String> {
    if let Some(ref k) = provided {
        if !k.is_empty() {
            return provided;
        }
    }
    config::load_api_key(DEFAULT_API_KEY_SERVICE).ok().flatten()
}

#[tauri::command]
pub async fn plate_solve_cmd(
    path: String,
    api_key: Option<String>,
    scale_lower: Option<f64>,
    scale_upper: Option<f64>,
    _scale_units: Option<String>,
    _downsample_factor: Option<u32>,
    center_ra: Option<f64>,
    center_dec: Option<f64>,
    radius: Option<f64>,
) -> Result<serde_json::Value, String> {
    let (upload_path, _tmp, _tmp_ds, stars, width, height, cfg) = tokio::task::spawn_blocking(
        move || -> anyhow::Result<_> {
            let resolved_key = resolve_api_key(api_key);

            let (resolved_path, tmp) = resolve_single_image(&path)?;
            let file = File::open(&resolved_path)?;
            let result = extract_image_mmap(&file)?;

            let naxis1 = result.header.get_i64(HEADER_NAXIS1).unwrap_or(0) as usize;
            let naxis2 = result.header.get_i64(HEADER_NAXIS2).unwrap_or(0) as usize;

            let detection = crate::core::analysis::star_detection::detect_stars(
                &result.image,
                5.0,
            );

            let (upload_fits, tmp_ds) = if naxis1 > MAX_UPLOAD_DIM || naxis2 > MAX_UPLOAD_DIM {
                let scale = MAX_UPLOAD_DIM as f64 / naxis1.max(naxis2) as f64;
                let ds_rows = (naxis2 as f64 * scale).round() as usize;
                let ds_cols = (naxis1 as f64 * scale).round() as usize;

                log::info!(
                    "Plate solve: downsampling {}x{} to {}x{} for upload",
                    naxis1, naxis2, ds_cols, ds_rows
                );

                let downsampled = crate::core::alignment::downsample::area_downsample(
                    &result.image, ds_rows, ds_cols,
                );

                let tmp_file = tempfile::Builder::new()
                    .suffix(".fits")
                    .tempfile()?;
                let tmp_path = tmp_file.path().to_string_lossy().to_string();

                crate::infra::fits::writer::write_fits_mono(
                    &tmp_path,
                    &downsampled,
                    Some(&result.header),
                )?;

                (tmp_path, Some(tmp_file))
            } else {
                (resolved_path.to_string_lossy().to_string(), None)
            };

            let cfg = crate::infra::astrometry::plate_solve::SolveConfig {
                api_url: config::load_config()
                    .map(|c| c.astrometry_api_url)
                    .unwrap_or_else(|_| DEFAULT_ASTROMETRY_API_URL.into()),
                api_key: resolved_key.unwrap_or_default(),
                ra_hint: center_ra,
                dec_hint: center_dec,
                radius_hint: radius,
                scale_low: scale_lower,
                scale_high: scale_upper,
                max_stars: config::load_config()
                    .map(|c| Some(c.plate_solve_max_stars))
                    .unwrap_or(Some(100)),
            };

            Ok((upload_fits, tmp, tmp_ds, detection.stars, naxis1, naxis2, cfg))
        },
    )
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e: anyhow::Error| e.to_string())?;

    #[cfg(feature = "astrometry-net")]
    {
        let solve_result = crate::infra::astrometry::plate_solve::solve_astrometry_net(
            &upload_path, &stars, width, height, &cfg,
        )
            .await
            .map_err(|e| e.to_string())?;

        drop((_tmp, _tmp_ds));
        return serde_json::to_value(&solve_result).map_err(|e| e.to_string());
    }

    #[cfg(not(feature = "astrometry-net"))]
    {
        drop((_tmp, _tmp_ds, upload_path, stars, width, height, cfg));
        let result = crate::infra::astrometry::plate_solve::solve_offline_placeholder()
            .map_err(|e| e.to_string())?;
        serde_json::to_value(&result).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn get_wcs_info(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let (header, wcs) = load_header_and_wcs(&path)?;
        let naxis1 = header.get_i64(HEADER_NAXIS1).unwrap_or(0) as usize;
        let naxis2 = header.get_i64(HEADER_NAXIS2).unwrap_or(0) as usize;
        let pixel_scale = wcs.pixel_scale_arcsec();
        let (fov_w, fov_h) = wcs.field_of_view(naxis1, naxis2);
        let center = wcs.pixel_to_world(naxis1 as f64 / 2.0, naxis2 as f64 / 2.0);
        let params = wcs.raw_params();

        Ok(json!({
            RES_CENTER_RA: center.ra,
            RES_CENTER_DEC: center.dec,
            RES_PIXEL_SCALE_ARCSEC: pixel_scale,
            RES_FOV_W_ARCMIN: fov_w,
            RES_FOV_H_ARCMIN: fov_h,
            RES_FOV_ARCMIN: [fov_w, fov_h],
            RES_NAXIS1: naxis1,
            RES_NAXIS2: naxis2,
            RES_WCS_PARAMS: {
                RES_WCS_CRPIX1: params.0,
                RES_WCS_CRPIX2: params.1,
                RES_WCS_CRVAL1: params.2,
                RES_WCS_CRVAL2: params.3,
                RES_WCS_CD: params.4,
                RES_WCS_PROJECTION: params.5,
            },
        }))
    })
}
