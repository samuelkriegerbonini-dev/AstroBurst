use std::fs::File;

use serde_json::json;

use crate::cmd::common::blocking_cmd;
use crate::core::astrometry::wcs::WcsTransform;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::reader::extract_image_mmap;
use crate::types::constants::{
    RES_CENTER_DEC, RES_CENTER_RA, RES_DEC, RES_FOV_H_ARCMIN, RES_FOV_W_ARCMIN,
    RES_NAXIS1, RES_NAXIS2, RES_PIXEL_SCALE_ARCSEC, RES_RA, RES_X, RES_Y,
};

fn load_wcs(path: &str) -> anyhow::Result<WcsTransform> {
    let (fits_path, _tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)?;
    let result = extract_image_mmap(&file)?;
    WcsTransform::from_header(&result.header)
}

fn load_header_and_wcs(path: &str) -> anyhow::Result<(crate::types::header::HduHeader, WcsTransform)> {
    let (fits_path, _tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)?;
    let result = extract_image_mmap(&file)?;
    let wcs = WcsTransform::from_header(&result.header)?;
    Ok((result.header, wcs))
}

#[tauri::command]
pub async fn plate_solve_cmd(
    _path: String,
    _api_key: Option<String>,
    _scale_lower: Option<f64>,
    _scale_upper: Option<f64>,
    _scale_units: Option<String>,
    _downsample_factor: Option<u32>,
    _center_ra: Option<f64>,
    _center_dec: Option<f64>,
    _radius: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let result = crate::domain::plate_solve::solve_offline_placeholder()?;
        Ok(serde_json::to_value(&result)?)
    })
}

#[tauri::command]
pub async fn get_wcs_info(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let (header, wcs) = load_header_and_wcs(&path)?;
        let naxis1 = header.get_i64("NAXIS1").unwrap_or(0) as usize;
        let naxis2 = header.get_i64("NAXIS2").unwrap_or(0) as usize;
        let pixel_scale = wcs.pixel_scale_arcsec();
        let (fov_w, fov_h) = wcs.field_of_view(naxis1, naxis2);
        let center = wcs.pixel_to_world(naxis1 as f64 / 2.0, naxis2 as f64 / 2.0);

        Ok(json!({
            RES_CENTER_RA: center.ra,
            RES_CENTER_DEC: center.dec,
            RES_PIXEL_SCALE_ARCSEC: pixel_scale,
            RES_FOV_W_ARCMIN: fov_w,
            RES_FOV_H_ARCMIN: fov_h,
            RES_NAXIS1: naxis1,
            RES_NAXIS2: naxis2,
        }))
    })
}

#[tauri::command]
pub async fn pixel_to_world(path: String, x: f64, y: f64) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let wcs = load_wcs(&path)?;
        let coord = wcs.pixel_to_world(x, y);
        Ok(json!({ RES_RA: coord.ra, RES_DEC: coord.dec }))
    })
}

#[tauri::command]
pub async fn world_to_pixel(path: String, ra: f64, dec: f64) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let wcs = load_wcs(&path)?;
        let (x, y) = wcs.world_to_pixel(ra, dec);
        Ok(json!({ RES_X: x, RES_Y: y }))
    })
}
