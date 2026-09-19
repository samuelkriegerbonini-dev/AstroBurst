use std::sync::Arc;

use serde_json::json;

use crate::cmd::common::{
    auto_stretch_preview, blocking_cmd, load_cached, load_cached_full, output_stem,
    resolve_output_dir, save_preview_png,
};
use crate::core::imaging::background::BackgroundMode;
use crate::core::imaging::dbe::{extract_background_dbe, DbeConfig};
use crate::core::imaging::stats::compute_image_stats;
use crate::infra::cache::{ImageEntry, GLOBAL_IMAGE_CACHE};
use crate::infra::fits::writer::{filter_header, write_fits_mono};
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    PROGRESS_EVENT, PROGRESS_STEPS, RES_CACHE_KEY, RES_CORRECTED_FITS, RES_CORRECTED_PNG,
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_MODEL_PNG, RES_RMS_RESIDUAL, RES_SAMPLE_COUNT,
};
use crate::types::header::HduHeader;

pub const RES_DBE_MODEL_FITS: &str = "model_fits";
pub const RES_DBE_REJECTED_COUNT: &str = "rejected_count";
pub const RES_DBE_SAMPLES: &str = "samples";
pub const ABPROC_KEY: &str = "ABPROC";
pub const ABPROC_DBE_SUBTRACT: &str = "dbe-subtract";
pub const ABPROC_DBE_DIVIDE: &str = "dbe-divide";
pub const ABPROC_DBE_MODEL: &str = "dbe-model";

pub(crate) fn abproc_for_mode(mode: &BackgroundMode) -> &'static str {
    match mode {
        BackgroundMode::Subtract => ABPROC_DBE_SUBTRACT,
        BackgroundMode::Divide => ABPROC_DBE_DIVIDE,
    }
}

pub(crate) fn output_header(source: Option<&HduHeader>, abproc: &str) -> HduHeader {
    let mut header = source
        .and_then(|h| filter_header(h, true, true))
        .unwrap_or_else(HduHeader::empty);
    header.set(ABPROC_KEY, abproc.to_string());
    header
}

fn source_header(path: &str, entry: &ImageEntry) -> Option<HduHeader> {
    if let Some(h) = entry.header() {
        return Some(h.clone());
    }
    load_cached_full(path).ok().and_then(|e| e.header().cloned())
}

#[tauri::command]
pub async fn extract_background_dbe_cmd(
    app: tauri::AppHandle,
    path: String,
    output_dir: String,
    config: DbeConfig,
) -> Result<serde_json::Value, String> {
    let progress = ProgressHandle::new(&app, PROGRESS_EVENT, PROGRESS_STEPS as u64);
    let progress_clone = progress.clone();

    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;
        let entry = load_cached(&path)?;
        let header = source_header(&path, &entry);

        let result = extract_background_dbe(entry.arr(), &config, Some(&progress_clone))?;

        let stem = output_stem(&path);
        let (rows, cols) = result.corrected.dim();
        let corrected_png = format!("{}/{}_dbe_corrected.png", output_dir, stem);
        let model_png = format!("{}/{}_dbe_model.png", output_dir, stem);
        save_preview_png(auto_stretch_preview(&result.corrected), cols, rows, &corrected_png)?;
        save_preview_png(auto_stretch_preview(&result.model), cols, rows, &model_png)?;

        let corrected_fits = format!("{}/{}_dbe_corrected.fits", output_dir, stem);
        let model_fits = format!("{}/{}_dbe_model.fits", output_dir, stem);
        let corrected_header = output_header(header.as_ref(), abproc_for_mode(&config.mode));
        let model_header = output_header(header.as_ref(), ABPROC_DBE_MODEL);
        write_fits_mono(&corrected_fits, &result.corrected, Some(&corrected_header))?;
        write_fits_mono(&model_fits, &result.model, Some(&model_header))?;

        let stats = compute_image_stats(&result.corrected);
        GLOBAL_IMAGE_CACHE.insert_synthetic(&corrected_fits, Arc::new(result.corrected), stats);

        Ok(json!({
            RES_CORRECTED_PNG: corrected_png,
            RES_MODEL_PNG: model_png,
            RES_CORRECTED_FITS: corrected_fits,
            RES_CACHE_KEY: corrected_fits,
            RES_DBE_MODEL_FITS: model_fits,
            RES_SAMPLE_COUNT: result.sample_count,
            RES_DBE_REJECTED_COUNT: result.rejected_count,
            RES_RMS_RESIDUAL: result.rms_residual,
            RES_ELAPSED_MS: result.elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
            RES_DBE_SAMPLES: serde_json::to_value(&result.samples)?,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::image_source::load_plane;
    use crate::types::image_ref::ImageRef;
    use ndarray::Array2;

    #[test]
    fn output_header_adds_the_abproc_card_and_keeps_source_cards() {
        let mut source = HduHeader::empty();
        source.set("CRPIX1", "128.0".to_string());
        source.set("OBJECT", "M31".to_string());
        let header = output_header(Some(&source), abproc_for_mode(&BackgroundMode::Subtract));
        assert_eq!(header.get(ABPROC_KEY), Some(ABPROC_DBE_SUBTRACT));
        assert_eq!(header.get("CRPIX1"), Some("128.0"));
        assert_eq!(header.get("OBJECT"), Some("M31"));
        assert!(header.cards.iter().any(|(k, _)| k == ABPROC_KEY));

        let bare = output_header(None, abproc_for_mode(&BackgroundMode::Divide));
        assert_eq!(bare.get(ABPROC_KEY), Some(ABPROC_DBE_DIVIDE));
        assert_eq!(bare.cards.len(), 1);
    }

    #[test]
    fn abproc_card_survives_a_fits_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dbe_corrected.fits");
        let path_str = path.to_str().unwrap().to_string();
        let data = Array2::from_elem((8, 8), 1.5f32);
        let header = output_header(None, ABPROC_DBE_SUBTRACT);
        write_fits_mono(&path_str, &data, Some(&header)).unwrap();
        let loaded = load_plane(&ImageRef::parse(&path_str)).unwrap();
        assert_eq!(loaded.header.get(ABPROC_KEY), Some("dbe-subtract"));
        assert_eq!(loaded.arr.dim(), (8, 8));
    }
}
