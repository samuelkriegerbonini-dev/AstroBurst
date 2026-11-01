use anyhow::Result;
use crate::cmd::common::blocking_cmd;
use crate::core::cubeprep::{align::{AlignOptions, ReferencePolicy, ResampleMethod}, cube::{CubeEdit, CubeLayout, CubeOptions, PlaneOrder}, footprint::{CropRegion, CropRounding}, kernel::KernelOptions, match_cube::{MatchMode, MatchOptions}, psf_io::{PsfResampleMethod, PsfSetOptions}};
use crate::core::metadata::units::{AreaRule, TargetUnit};
use crate::service::cubeprep::{
    align_set, build_cube, build_kernels, convert_units_file, crop_set, edit_cube, load_psfs, psf_match_cube, CubeInputSpec,
    KernelFileSpec, PsfInputSpec,
};

#[tauri::command]
pub async fn convert_units_cmd(
    path: String,
    target: TargetUnit,
    output_path: Option<String>,
    output_dir: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let report = convert_units_file(&path, target, output_path.as_deref().unwrap_or(""), output_dir.as_deref())?;
        Ok(serde_json::to_value(&report)?)
    })
}

#[tauri::command]
pub async fn crop_set_cmd(
    paths: Vec<String>,
    region: CropRegion,
    rounding: Option<CropRounding>,
    output_dir: String,
    include_err: Option<bool>,
    include_dq: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let report = crop_set(
            &paths,
            &region,
            rounding.unwrap_or_default(),
            &output_dir,
            include_err.unwrap_or(true),
            include_dq.unwrap_or(true),
        )?;
        Ok(serde_json::to_value(&report)?)
    })
}

#[tauri::command]
pub async fn align_set_cmd(
    paths: Vec<String>,
    output_dir: String,
    method: Option<ResampleMethod>,
    reference: Option<ReferencePolicy>,
    border_crop: Option<usize>,
    area_rule: Option<AreaRule>,
    per_actual_pixel: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let defaults = AlignOptions::default();
        let options = AlignOptions {
            method: method.unwrap_or(defaults.method),
            reference: reference.unwrap_or(defaults.reference),
            border_crop: border_crop.unwrap_or(defaults.border_crop),
            area_rule,
            per_actual_pixel: per_actual_pixel.unwrap_or(defaults.per_actual_pixel),
        };
        let report = align_set(&paths, &output_dir, &options)?;
        Ok(serde_json::to_value(&report)?)
    })
}

#[tauri::command]
pub async fn build_cube_cmd(
    inputs: Vec<CubeInputSpec>,
    output_path: Option<String>,
    output_dir: Option<String>,
    layout: Option<CubeLayout>,
    order: Option<PlaneOrder>,
    reference: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let options = CubeOptions { layout: layout.unwrap_or_default(), order: order.unwrap_or_default() };
        let report = build_cube(&inputs, output_path.as_deref().unwrap_or(""), output_dir.as_deref(), options, reference.as_deref())?;
        Ok(serde_json::to_value(&report)?)
    })
}

#[tauri::command]
pub async fn edit_cube_cmd(
    path: String,
    edit: CubeEdit,
    output_path: Option<String>,
    output_dir: Option<String>,
    layout: Option<CubeLayout>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let report = edit_cube(&path, &edit, output_path.as_deref().unwrap_or(""), output_dir.as_deref(), layout)?;
        Ok(serde_json::to_value(&report)?)
    })
}

#[tauri::command]
pub async fn load_psfs_cmd(
    psfs: Vec<PsfInputSpec>,
    target_pixel_scale_arcsec: Option<f64>,
    reference: Option<String>,
    method: Option<PsfResampleMethod>,
    recentre: Option<bool>,
    output_dir: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let defaults = PsfSetOptions::default();
        let options = PsfSetOptions {
            target_pixel_scale_arcsec,
            reference,
            method: method.unwrap_or(defaults.method),
            recentre: recentre.unwrap_or(defaults.recentre),
        };
        let report = load_psfs(&psfs, &options, output_dir.as_deref())?;
        Ok(serde_json::to_value(&report)?)
    })
}

#[tauri::command]
pub async fn build_kernels_cmd(
    psfs: Vec<PsfInputSpec>,
    reference: String,
    options: Option<KernelOptions>,
    output_dir: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let report = build_kernels(&psfs, &reference, &options.unwrap_or_default(), output_dir.as_deref())?;
        Ok(serde_json::to_value(&report)?)
    })
}

#[tauri::command]
pub async fn psf_match_cube_cmd(
    path: String,
    reference: String,
    kernel_dir: Option<String>,
    kernels: Option<Vec<KernelFileSpec>>,
    mode: Option<MatchMode>,
    allow_unmatched: Option<bool>,
    output_path: Option<String>,
    output_dir: Option<String>,
    layout: Option<CubeLayout>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let options = MatchOptions { reference, mode: mode.unwrap_or_default(), allow_unmatched: allow_unmatched.unwrap_or(false) };
        let report = psf_match_cube(
            &path,
            kernel_dir.as_deref(),
            &kernels.unwrap_or_default(),
            &options,
            output_path.as_deref().unwrap_or(""),
            output_dir.as_deref(),
            layout,
        )?;
        Ok(serde_json::to_value(&report)?)
    })
}
