use crate::infra::progress::ProgressHandle;
use crate::service::sagui::{self, InputInfo, SaguiConfig, SaguiResult};
use crate::service::sagui::runtime::RuntimeInfo;

#[tauri::command]
pub async fn load_sagui_result_cmd(manifest_path: String) -> Result<sagui::saved::SavedResult, String> {
    tokio::task::spawn_blocking(move || sagui::saved::load(&manifest_path).map_err(|error| format!("{error:#}"))).await.map_err(|error| error.to_string())?
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
    tokio::task::spawn_blocking(move || sagui::run(config, run_id, &progress).map_err(|e| format!("{e:#}"))).await.map_err(|e| e.to_string())?
}
