use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, invalidate_written};
use crate::infra::fits::gzip::resolve_gzip;
use crate::infra::fits::mef_writer::{write_compressed_mef, CompressMode, CompressOptions};
use crate::service::cutout::refuse_source_as_target;
use crate::types::constants::{RES_DROPPED, RES_ELAPSED_MS, RES_KEPT_RAW, RES_OUTPUT_PATH, RES_OUTPUT_SIZE_BYTES, RES_SOURCE_DECOMPRESSED_BYTES, RES_SOURCE_SIZE_BYTES, RES_UNCOMPRESSED};

use super::fits_export::DEFAULT_QUANTIZE_LEVEL;

#[tauri::command]
pub async fn compress_mef_cmd(
    source_path: String,
    output_path: String,
    lossless: bool,
    quantize_level: Option<f64>,
    drop_extnames: Vec<String>,
    raw_extnames: Vec<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();

        let mode = if lossless {
            CompressMode::Lossless
        } else {
            CompressMode::Lossy { quantize_level: quantize_level.unwrap_or(DEFAULT_QUANTIZE_LEVEL) }
        };

        let opts = CompressOptions { mode, drop_extnames, raw_extnames };

        let (readable, decompressed) = resolve_gzip(std::path::Path::new(&source_path))?;
        if decompressed.is_some() {
            refuse_source_as_target(&output_path, &source_path)?;
        }
        crate::core::cube::cache::GLOBAL_CUBE_CACHE.invalidate(&output_path);
        let report = write_compressed_mef(&readable.to_string_lossy(), &output_path, &opts)?;
        invalidate_written(&output_path);

        let source_size = std::fs::metadata(&source_path).map(|m| m.len()).unwrap_or(0);
        let source_decompressed = std::fs::metadata(&readable).map_or(source_size, |m| m.len());
        let output_size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            RES_DROPPED: report.dropped,
            RES_KEPT_RAW: report.kept_raw,
            RES_UNCOMPRESSED: report.uncompressed,
            RES_SOURCE_SIZE_BYTES: source_size,
            RES_SOURCE_DECOMPRESSED_BYTES: source_decompressed,
            RES_OUTPUT_SIZE_BYTES: output_size,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}
