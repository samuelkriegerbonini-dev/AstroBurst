use tauri::ipc::Response;

use crate::cmd::common::load_preview_validated;
use crate::cmd::helpers;
use crate::infra::ipc::{encode_rgb_with_header_downsampled, encode_with_header_downsampled};

use super::rgb_planes::load_rgb_file_planes;

#[tauri::command]
pub async fn get_raw_pixels_preview(path: String, max_dim: Option<u32>) -> Result<Response, String> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<Response> {
        let dim = max_dim.unwrap_or(2048) as usize;
        let entry = load_preview_validated(&path)?;
        let data = encode_with_header_downsampled(entry.arr(), dim)?;
        Ok(Response::new(data))
    })
        .await
        .map_err(|e| format!("{}", e))?
        .map_err(|e| format!("{:#}", e))
}

#[tauri::command]
pub async fn get_raw_rgb_pixels_preview(
    path: Option<String>,
    max_dim: Option<u32>,
) -> Result<Response, String> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<Response> {
        let dim = max_dim.unwrap_or(2048) as usize;
        let data = match path {
            Some(p) => {
                let (r, g, b) = load_rgb_file_planes(&p)?;
                encode_rgb_with_header_downsampled(&r, &g, &b, dim)?
            }
            None => {
                let derived = helpers::load_composite_toned()
                    .or_else(helpers::load_composite_stretched);
                match derived {
                    Some((r, g, b)) => {
                        crate::infra::ipc::encode_rgb_with_header_downsampled_flagged(
                            r.arr(), g.arr(), b.arr(), dim, 1,
                        )?
                    }
                    None => {
                        let (r, g, b) = helpers::load_composite_rgb()?;
                        encode_rgb_with_header_downsampled(r.arr(), g.arr(), b.arr(), dim)?
                    }
                }
            }
        };
        Ok(Response::new(data))
    })
        .await
        .map_err(|e| format!("{}", e))?
        .map_err(|e| format!("{:#}", e))
}
