use serde_json::json;

use crate::cmd::common::blocking_cmd;
use crate::domain::cube::process_cube;
use crate::domain::lazy_cube::{process_cube_lazy, LazyCube};
use crate::types::constants::{
    RES_BITPIX, RES_FITS_PATH, RES_FRAME_INDEX, RES_FRAMES, RES_HEIGHT,
    RES_OUTPUT_PATH, RES_SPECTRUM, RES_WIDTH,
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
        Ok(json!({
            RES_WIDTH: geo.naxis1,
            RES_HEIGHT: geo.naxis2,
            RES_FRAMES: geo.naxis3,
            RES_BITPIX: geo.bitpix,
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
        Ok(json!({ RES_SPECTRUM: spectrum }))
    })
}
