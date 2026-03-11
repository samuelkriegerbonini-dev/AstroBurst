use serde_json::json;

use crate::cmd::common::blocking_cmd;
use crate::core::cube::eager::classify_spectral_cube;
use crate::domain::cube::{process_cube, build_wavelength_axis};
use crate::domain::lazy_cube::{process_cube_lazy, LazyCube};
use crate::types::constants::{
    RES_BITPIX, RES_FITS_PATH, RES_FRAME_INDEX, RES_FRAMES, RES_HEIGHT,
    RES_OUTPUT_PATH, RES_SPECTRUM, RES_WIDTH,
    RES_SPECTRAL_CLASSIFICATION, RES_IS_SPECTRAL, RES_SPECTRAL_REASON,
    RES_AXIS_TYPE, RES_AXIS_UNIT, RES_CHANNEL_COUNT, RES_WAVELENGTHS,
};

#[tauri::command]
pub async fn process_cube_cmd(
    path: String,
    output_dir: String,
    frame_step: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let result = process_cube(&path, &output_dir, frame_step)?;
        Ok(serde_json::to_value(&result)?)
    })
}

#[tauri::command]
pub async fn process_cube_lazy_cmd(
    path: String,
    output_dir: String,
    frame_step: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let result = process_cube_lazy(&path, &output_dir, frame_step)?;
        Ok(serde_json::to_value(&result)?)
    })
}

#[tauri::command]
pub async fn get_cube_info(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let cube = LazyCube::open(&path)?;
        let geo = &cube.geometry;
        let classification = classify_spectral_cube(&cube.header, geo.naxis3);
        let wavelengths = build_wavelength_axis(&cube.header);
        Ok(json!({
            RES_WIDTH: geo.naxis1,
            RES_HEIGHT: geo.naxis2,
            RES_FRAMES: geo.naxis3,
            RES_BITPIX: geo.bitpix,
            RES_SPECTRAL_CLASSIFICATION: {
                RES_IS_SPECTRAL: classification.is_spectral,
                RES_SPECTRAL_REASON: classification.reason,
                RES_AXIS_TYPE: classification.axis_type,
                RES_AXIS_UNIT: classification.axis_unit,
                RES_CHANNEL_COUNT: classification.channel_count,
            },
            RES_WAVELENGTHS: wavelengths,
        }))
    })
}

#[tauri::command]
pub async fn get_cube_frame(
    path: String,
    frame_index: usize,
    output_path: String,
    output_fits: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let cube = LazyCube::open(&path)?;
        let frame = cube.get_frame(frame_index)?;
        crate::infra::render::grayscale::render_grayscale(&frame, &output_path)?;
        let fits_path = if let Some(fp) = &output_fits {
            crate::infra::fits::writer::write_fits_mono(fp, &frame, None)?;
            Some(fp.clone())
        } else {
            None
        };
        Ok(json!({
            RES_FRAME_INDEX: frame_index,
            RES_OUTPUT_PATH: output_path,
            RES_FITS_PATH: fits_path,
        }))
    })
}

#[tauri::command]
pub async fn get_cube_spectrum(
    path: String,
    x: usize,
    y: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let cube = LazyCube::open(&path)?;
        let spectrum = cube.extract_spectrum_at(x, y)?;
        let wavelengths = build_wavelength_axis(&cube.header);
        let classification = classify_spectral_cube(&cube.header, cube.geometry.naxis3);
        Ok(json!({
            RES_SPECTRUM: spectrum,
            RES_WAVELENGTHS: wavelengths,
            RES_IS_SPECTRAL: classification.is_spectral,
        }))
    })
}
