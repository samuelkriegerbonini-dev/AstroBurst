use tauri_plugin_fs::FsExt;
use crate::infra::progress::ProgressHandle;
use crate::service::sagui::{self, InputInfo, SaguiConfig, SaguiResult};
use crate::service::sagui::runtime::RuntimeInfo;

fn allow_verified_products(app: &tauri::AppHandle, result: &SaguiResult) {
    for (product, path) in [("photometry table", &result.table_path), ("label map", &result.labels_bin_path)] {
        if let Err(error) = app.fs_scope().allow_file(path) {
            log::warn!("the verified SAGUI {product} {path} could not be opened to the interface: {error}");
        }
    }
}

#[tauri::command]
pub async fn load_sagui_result_cmd(app: tauri::AppHandle, manifest_path: String) -> Result<sagui::saved::SavedResult, String> {
    let saved = tokio::task::spawn_blocking(move || sagui::saved::load(&manifest_path).map_err(|error| format!("{error:#}"))).await.map_err(|error| error.to_string())??;
    allow_verified_products(&app, &saved.result);
    Ok(saved)
}

#[tauri::command]
pub async fn inspect_sagui_input_cmd(path: String) -> Result<InputInfo, String> {
    tokio::task::spawn_blocking(move || sagui::inspect_input(&path).map_err(|e| format!("{e:#}"))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn inspect_sagui_runtime_cmd(rscript_path: String, library_path: String) -> Result<RuntimeInfo, String> {
    tokio::task::spawn_blocking(move || sagui::runtime::inspect(&rscript_path, &library_path)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_sagui_cmd(app: tauri::AppHandle, config: SaguiConfig, run_id: String) -> Result<SaguiResult, String> {
    if !sagui::valid_run_id(&run_id) { return Err("Invalid SAGUI run identifier".into()); }
    let progress = ProgressHandle::new(&app, &format!("sagui-progress-{run_id}"), 5);
    let result = tokio::task::spawn_blocking(move || sagui::run(config, run_id, &progress).map_err(|e| format!("{e:#}"))).await.map_err(|e| e.to_string())??;
    allow_verified_products(&app, &result);
    Ok(result)
}
