use crate::cmd::common::blocking_cmd;
use crate::domain::pipeline::{run_pipeline, SingleResult};
use crate::types::constants::{RES_COLLAPSED_PATH, RES_PNG_PATH};

#[tauri::command]
pub async fn run_pipeline_cmd(
    input_path: String,
    output_dir: String,
    frame_step: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let result = run_pipeline(&input_path, &output_dir, frame_step)?;

        let mut png_path: Option<String> = None;
        let mut collapsed_path: Option<String> = None;
        for r in &result.results {
            match r {
                SingleResult::Image { png_path: p, .. } => {
                    if png_path.is_none() {
                        png_path = Some(p.clone());
                    }
                }
                SingleResult::Cube { cube, .. } => {
                    if collapsed_path.is_none() {
                        collapsed_path = Some(cube.collapsed_path.clone());
                    }
                }
                _ => {}
            }
        }

        let mut val = serde_json::to_value(&result)?;
        if let Some(p) = png_path {
            val[RES_PNG_PATH] = serde_json::Value::String(p);
        }
        if let Some(p) = collapsed_path {
            val[RES_COLLAPSED_PATH] = serde_json::Value::String(p);
        }
        Ok(val)
    })
}
