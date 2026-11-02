use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use crate::cmd::common::blocking_cmd;
use crate::core::cubeprep::{align::{AlignOptions, ReferencePolicy, ResampleMethod}, cube::{CubeEdit, CubeLayout, CubeOptions, PlaneOrder}, footprint::{CropRegion, CropRounding}, kernel::KernelOptions, match_cube::{MatchMode, MatchOptions}, psf_io::{PsfResampleMethod, PsfSetOptions}};
use crate::core::metadata::units::{AreaRule, TargetUnit};
use crate::service::cubeprep::{
    align_set, build_cube, build_kernels, convert_units_file, crop_set, edit_cube, load_psfs, psf_match_cube, CubeInputSpec,
    KernelFileSpec, PsfInputSpec,
};
use crate::infra::progress::ProgressHandle;
use crate::service::cubeprep_recipe::{run_recipe, Recipe, RecipeReport};
use crate::types::error::{AppError, CodedError};

#[derive(Debug, Serialize, Deserialize)]
pub struct CubePrepFailure {
    pub code: String,
    pub message: String,
    pub hint: Option<String>,
}

fn recipe_failure(error: anyhow::Error) -> CubePrepFailure {
    if let Some(coded) = error.downcast_ref::<CodedError>() {
        return CubePrepFailure { code: coded.code.to_string(), message: format!("{error:#}"), hint: coded.hint.clone() };
    }
    let code = if error.downcast_ref::<AppError>().is_some() { "cancelled" } else { "cubeprep_failed" };
    CubePrepFailure { code: code.to_string(), message: format!("{error:#}"), hint: None }
}

fn desktop_recipe(value: &Value, base_dir: &str) -> Result<Recipe> {
    let base = Path::new(base_dir).canonicalize()?;
    anyhow::ensure!(base.is_dir(), "The input folder is not a directory");
    let recipe = Recipe::from_value(value)?.resolved(&base);
    recipe.validate()?;
    Ok(recipe)
}

fn recipe_progress_event(run_id: &str) -> Result<String> {
    anyhow::ensure!(!run_id.is_empty() && run_id.len() <= 80 && run_id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'), "Invalid cube preparation run identifier");
    Ok(format!("cubeprep-progress-{run_id}"))
}

#[tauri::command]
pub async fn cubeprep_filters_cmd() -> Result<Value, CubePrepFailure> {
    tokio::task::spawn_blocking(|| {
        let registry = crate::service::cubeprep::filter_registry();
        json!({ "datasets": registry.datasets(), "filters": registry.entries() })
    }).await.map_err(|error| recipe_failure(error.into()))
}

#[tauri::command]
pub async fn validate_cubeprep_recipe_cmd(recipe: Value, base_dir: String) -> Result<Value, CubePrepFailure> {
    tokio::task::spawn_blocking(move || {
        let recipe = desktop_recipe(&recipe, &base_dir)?;
        let mut dependencies: Vec<String> = recipe.paths().into_iter().filter(|path| Path::new(path).is_file()).map(str::to_string).collect();
        dependencies.extend(recipe.empirical_files()?);
        dependencies.sort();
        dependencies.dedup();
        let mut warnings = vec!["Input hashes are checked before and after execution; keep input files unchanged until completion".to_string()];
        for step in &recipe.steps {
            match step {
                crate::service::cubeprep_recipe::RecipeStep::Cube(cube) if cube.layout == CubeLayout::Lightstack => warnings.push("The LIGHTSTACK layout writes SCI only; choose MEF to preserve ERR and DQ".to_string()),
                crate::service::cubeprep_recipe::RecipeStep::Align(align) if align.area_rule == AreaRule::Lightstack => warnings.push("The LIGHTSTACK area rule requires per-pixel units to conserve flux; convert to Jy/pixel first or choose scale for surface brightness".to_string()),
                _ => {}
            }
        }
        Ok::<_, anyhow::Error>(json!({ "recipe": recipe, "dependencies": dependencies, "warnings": warnings }))
    }).await.map_err(|error| recipe_failure(error.into()))?.map_err(recipe_failure)
}

#[tauri::command]
pub async fn run_cubeprep_recipe_cmd(app: tauri::AppHandle, recipe: Value, base_dir: String, output_dir: String, run_id: String) -> Result<RecipeReport, CubePrepFailure> {
    let event = recipe_progress_event(&run_id).map_err(recipe_failure)?;
    let progress = ProgressHandle::new(&app, &event, 1);
    tokio::task::spawn_blocking(move || {
        let recipe = desktop_recipe(&recipe, &base_dir)?;
        anyhow::ensure!(Path::new(&output_dir).is_absolute(), "The output folder must be an absolute path");
        run_recipe(&recipe, Path::new(&output_dir), &progress)
    }).await.map_err(|error| recipe_failure(error.into()))?.map_err(recipe_failure)
}

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

#[cfg(test)]
mod recipe_tests {
    use super::*;

    #[test]
    fn desktop_validation_resolves_the_complete_recipe_without_writing_outputs() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lightstack/d1");
        let value: Value = serde_json::from_slice(&std::fs::read(base.join("recipe_d1.json")).unwrap()).unwrap();
        let recipe = desktop_recipe(&value, base.to_str().unwrap()).unwrap();
        assert_eq!(recipe.steps.len(), 8);
        assert!(recipe.inputs.iter().all(|input| Path::new(&input.path).is_absolute()));
        assert_eq!(serde_json::to_value(&recipe.steps[2]).unwrap()["area_rule"], "lightstack");
    }

    #[test]
    fn desktop_failures_preserve_codes_hints_and_cancellation() {
        let error = recipe_failure(CodedError { code: "unknown_step", message: "No such step".into(), hint: Some("Use align".into()) }.into());
        assert_eq!(serde_json::to_value(error).unwrap(), json!({ "code": "unknown_step", "message": "No such step", "hint": "Use align" }));
        assert_eq!(recipe_failure(AppError::Cancelled.into()).code, "cancelled");
        assert_eq!(recipe_progress_event("run_123-abc").unwrap(), "cubeprep-progress-run_123-abc");
        for invalid in ["", "bad event", "a/b", "a:b"] {
            assert!(recipe_progress_event(invalid).is_err());
        }
        assert!(recipe_progress_event(&"a".repeat(81)).is_err());
    }
}
