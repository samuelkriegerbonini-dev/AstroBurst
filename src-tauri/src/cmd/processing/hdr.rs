use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_cached, resolve_output_dir};
use crate::cmd::processing::local_contrast::{run_composite_contrast, save_contrast_output, source_header};
use crate::core::imaging::hdr::{hdrmt_rgb, hdrmt_with_progress, HdrConfig};
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_PNG_PATH};

pub const EVENT_HDR_PROGRESS: &str = "hdr-progress";
pub const SUFFIX_HDR: &str = "hdr";
pub const ABPROC_HDRMT: &str = "hdrmt";

#[tauri::command]
pub async fn hdrmt_cmd(
    app: tauri::AppHandle,
    path: String,
    output_dir: String,
    config: HdrConfig,
) -> Result<serde_json::Value, String> {
    let progress = ProgressHandle::new(&app, EVENT_HDR_PROGRESS, 1);
    let progress_clone = progress.clone();

    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;
        let entry = load_cached(&path)?;
        let header = source_header(&path, &entry);

        let t0 = std::time::Instant::now();
        let compressed = hdrmt_with_progress(entry.arr(), &config, Some(&progress_clone))?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let (ro, fits_path) =
            save_contrast_output(&compressed, &path, &output_dir, SUFFIX_HDR, ABPROC_HDRMT, header.as_ref())?;
        let (rows, cols) = ro.dims;

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: fits_path,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

#[tauri::command]
pub async fn hdrmt_composite_cmd(
    output_dir: String,
    config: HdrConfig,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;
        run_composite_contrast(SUFFIX_HDR, &output_dir, move |r, g, b| hdrmt_rgb(r, g, b, &config))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::processing::local_contrast::{composite_png_path, processed_fits_path};

    #[test]
    fn hdr_config_deserialises_camel_case_with_defaults() {
        let cfg: HdrConfig =
            serde_json::from_str(r#"{"layers":4,"toLightness":false,"deringingAmount":0.25}"#)
                .unwrap();
        assert_eq!(cfg.layers, 4);
        assert!(!cfg.to_lightness);
        assert_eq!(cfg.deringing_amount, 0.25);
        assert_eq!(cfg.iterations, 1);
        assert_eq!(cfg.overdrive, 0.0);
        assert!(!cfg.inverted);
        assert!(!cfg.deringing);
    }

    #[test]
    fn hdr_output_paths_use_the_hdr_suffix() {
        assert_eq!(
            processed_fits_path("C:/d/m42.fits", "C:/out", SUFFIX_HDR),
            "C:/out/m42_hdr.fits"
        );
        assert!(composite_png_path("C:/out", SUFFIX_HDR).starts_with("C:/out/composite_hdr_"));
    }
}
