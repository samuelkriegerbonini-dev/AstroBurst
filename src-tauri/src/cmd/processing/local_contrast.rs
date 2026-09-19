use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, load_cached, load_cached_full, output_stem, render_and_save, resolve_output_dir,
    MAX_PREVIEW_DIM,
};
use crate::cmd::helpers;
use crate::core::imaging::local_contrast::{lhe_rgb, lhe_with_progress, LheConfig};
use crate::infra::cache::ImageEntry;
use crate::infra::fits::writer::{filter_header, write_fits_mono};
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_PNG_PATH};
use crate::types::header::HduHeader;

pub const EVENT_LHE_PROGRESS: &str = "lhe-progress";
pub const SUFFIX_LHE: &str = "lhe";
pub const ABPROC_LHE: &str = "lhe";
pub const COMPOSITE_STRETCH_REQUIRED: &str =
    "No stretched composite is loaded: run a stretch on the composite first";

const ABPROC_KEY: &str = "ABPROC";

pub(crate) fn processed_header(source: Option<&HduHeader>, abproc: &str) -> HduHeader {
    let mut header = source
        .and_then(|h| filter_header(h, true, true))
        .unwrap_or_else(HduHeader::empty);
    header.set(ABPROC_KEY, abproc.to_string());
    header
}

pub(crate) fn source_header(path: &str, entry: &ImageEntry) -> Option<HduHeader> {
    if let Some(h) = entry.header() {
        return Some(h.clone());
    }
    load_cached_full(path)
        .ok()
        .and_then(|e| e.header().cloned())
}

pub(crate) fn processed_fits_path(path: &str, output_dir: &str, suffix: &str) -> String {
    format!("{}/{}_{}.fits", output_dir, output_stem(path), suffix)
}

pub(crate) fn composite_png_path(output_dir: &str, suffix: &str) -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{}/composite_{}_{}.png", output_dir, suffix, ts)
}

pub(crate) fn load_stretched_composite() -> anyhow::Result<(ImageEntry, ImageEntry, ImageEntry)> {
    helpers::load_composite_stretched().ok_or_else(|| anyhow::anyhow!(COMPOSITE_STRETCH_REQUIRED))
}

#[tauri::command]
pub async fn lhe_cmd(
    app: tauri::AppHandle,
    path: String,
    output_dir: String,
    config: LheConfig,
) -> Result<serde_json::Value, String> {
    let progress = ProgressHandle::new(&app, EVENT_LHE_PROGRESS, 1);
    let progress_clone = progress.clone();

    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;
        let entry = load_cached(&path)?;
        let header = source_header(&path, &entry);

        let t0 = std::time::Instant::now();
        let result = lhe_with_progress(entry.arr(), &config, Some(&progress_clone))?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let ro = render_and_save(&result.image, &path, &output_dir, SUFFIX_LHE, false)?;
        let fits_path = processed_fits_path(&path, &output_dir, SUFFIX_LHE);
        let out_header = processed_header(header.as_ref(), ABPROC_LHE);
        write_fits_mono(&fits_path, &result.image, Some(&out_header))?;
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
pub async fn lhe_composite_cmd(
    output_dir: String,
    config: LheConfig,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;
        let (er, eg, eb) = load_stretched_composite()?;

        let t0 = std::time::Instant::now();
        let (r, g, b) = lhe_rgb(er.arr(), eg.arr(), eb.arr(), &config)?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;
        let (rows, cols) = r.dim();

        let png_path = composite_png_path(&output_dir, SUFFIX_LHE);
        helpers::render_rgb_preview(&r, &g, &b, &png_path, MAX_PREVIEW_DIM)?;
        helpers::insert_composite_stretched(r, g, b);

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processed_header_adds_abproc_and_keeps_source_cards() {
        let mut source = HduHeader::empty();
        source.set("CRPIX1", "128.0".to_string());
        source.set("OBJECT", "M42".to_string());
        let header = processed_header(Some(&source), ABPROC_LHE);
        assert_eq!(header.get(ABPROC_KEY), Some("lhe"));
        assert_eq!(header.get("CRPIX1"), Some("128.0"));
        assert_eq!(header.get("OBJECT"), Some("M42"));
        assert!(header.cards.iter().any(|(k, _)| k == ABPROC_KEY));

        let bare = processed_header(None, "hdrmt");
        assert_eq!(bare.get(ABPROC_KEY), Some("hdrmt"));
        assert_eq!(bare.cards.len(), 1);
    }

    #[test]
    fn output_paths_follow_the_stem_suffix_convention() {
        assert_eq!(
            processed_fits_path("C:/d/jw_cal.fits#hdu=3", "C:/out", SUFFIX_LHE),
            "C:/out/jw_cal_hdu3_lhe.fits"
        );
        let png = composite_png_path("C:/out", SUFFIX_LHE);
        assert!(png.starts_with("C:/out/composite_lhe_"));
        assert!(png.ends_with(".png"));
    }

    #[test]
    fn lhe_config_deserialises_camel_case_with_defaults() {
        let cfg: LheConfig = serde_json::from_str(r#"{"kernelRadius":32,"histBits":10}"#).unwrap();
        assert_eq!(cfg.kernel_radius, 32);
        assert_eq!(cfg.hist_bits, 10);
        assert_eq!(cfg.contrast_limit, 2.0);
        assert_eq!(cfg.amount, 1.0);
        assert!(cfg.circular);
    }
}
