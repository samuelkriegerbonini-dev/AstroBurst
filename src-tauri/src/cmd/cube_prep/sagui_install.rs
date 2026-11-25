use crate::infra::progress::ProgressHandle;
use crate::service::sagui::{install, valid_run_id};

#[tauri::command]
pub async fn install_sagui_runtime_cmd(app: tauri::AppHandle, rscript_path: String, output_dir: String, run_id: String) -> Result<install::InstallResult, String> {
    if !valid_run_id(&run_id) { return Err("Invalid SAGUI installation identifier".into()); }
    let progress = ProgressHandle::new(&app, &format!("sagui-install-progress-{run_id}"), 4);
    tokio::task::spawn_blocking(move || install::install(&rscript_path, &output_dir, &run_id, &progress).map_err(|error| format!("{error:#}")))
        .await.map_err(|error| error.to_string())?
}
