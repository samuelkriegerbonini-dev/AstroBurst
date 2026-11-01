use anyhow::Result;
use crate::cmd::common::blocking_cmd;
use crate::core::imaging::region::RegionShape;
use crate::service::cutout::export_cutout;

#[tauri::command]
pub async fn export_cutout_cmd(
    path: String,
    output_path: Option<String>,
    output_dir: Option<String>,
    region: RegionShape,
    include_err: Option<bool>,
    include_dq: Option<bool>,
    sky: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let report = export_cutout(
            &path,
            output_path.as_deref().unwrap_or(""),
            output_dir.as_deref(),
            &region,
            include_err.unwrap_or(true),
            include_dq.unwrap_or(true),
            sky.unwrap_or(false),
        )?;
        Ok(serde_json::to_value(&report)?)
    })
}
