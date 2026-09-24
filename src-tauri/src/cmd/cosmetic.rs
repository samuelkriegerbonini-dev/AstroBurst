use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use ndarray::Array2;
use serde::Serialize;
use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, load_cached, load_cached_full, load_companions, output_stem, render_and_save,
    resolve_output_dir, write_derived_fits,
};
use crate::core::imaging::cosmetic::{cosmetic_correct, parse_defect_list, CosmeticConfig, CosmeticResult};
use crate::infra::cache::ImageEntry;
use crate::types::constants::{
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_ERROR, RES_FAILED, RES_FITS_PATH, RES_PATH, RES_PNG_PATH,
    RES_RESULTS, RES_SUCCEEDED,
};
use crate::types::header::HduHeader;

pub const KEY_COUNTS: &str = "counts";
pub const KEY_DQ_PRESENT: &str = "dq_present";
pub const KEY_WARNING: &str = "warning";
pub const SUFFIX_COSMETIC: &str = "cosmetic";
const DQ_WARNING: &str = "This image carries a DQ plane: its pipeline already flags bad pixels, so cosmetic correction may alter calibrated science data.";

#[derive(Debug, Serialize)]
struct CosmeticCounts {
    flagged: usize,
    replaced: usize,
    hot: usize,
    cold: usize,
    listed: usize,
}

impl From<&CosmeticResult> for CosmeticCounts {
    fn from(r: &CosmeticResult) -> Self {
        Self {
            flagged: r.flagged,
            replaced: r.replaced,
            hot: r.hot,
            cold: r.cold,
            listed: r.listed,
        }
    }
}

fn resolve_config(mut config: CosmeticConfig, defect_list_text: Option<&str>) -> Result<CosmeticConfig> {
    if let Some(text) = defect_list_text {
        let parsed = parse_defect_list(text).map_err(|msg| anyhow!("Defect list: {}", msg))?;
        config.defects.extend(parsed);
    }
    Ok(config)
}

fn load_master_dark(path: Option<&str>) -> Result<Option<ImageEntry>> {
    match path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => {
            let entry = load_cached(p).with_context(|| format!("Failed to load master dark '{}'", p))?;
            Ok(Some(entry))
        }
        None => Ok(None),
    }
}

fn dq_present(path: &str) -> bool {
    load_companions(path).map(|c| c.dq.is_some()).unwrap_or(false)
}

fn output_header(source: Option<&HduHeader>) -> Option<HduHeader> {
    let mut header = source?.clone();
    for key in ["XTENSION", "PCOUNT", "GCOUNT"] {
        header.remove(key);
    }
    Some(header)
}

fn correct_one(
    path: &str,
    out_dir: &str,
    master_dark: Option<&Array2<f32>>,
    cfg: &CosmeticConfig,
) -> Result<serde_json::Value> {
    let entry = load_cached_full(path).or_else(|_| load_cached(path))?;
    let result = cosmetic_correct(entry.arr(), master_dark, cfg)?;
    let ro = render_and_save(&result.corrected, path, out_dir, SUFFIX_COSMETIC, false)?;
    let fits_path = format!("{}/{}_{}.fits", out_dir, output_stem(path), SUFFIX_COSMETIC);
    write_derived_fits(&fits_path, &result.corrected, output_header(entry.header()).as_ref())?;
    let (rows, cols) = ro.dims;
    let dq = dq_present(path);
    Ok(json!({
        RES_PATH: path,
        RES_PNG_PATH: ro.png_path,
        RES_FITS_PATH: fits_path,
        RES_DIMENSIONS: [cols, rows],
        KEY_COUNTS: serde_json::to_value(CosmeticCounts::from(&result))?,
        KEY_DQ_PRESENT: dq,
        KEY_WARNING: dq.then_some(DQ_WARNING),
    }))
}

#[tauri::command]
pub async fn cosmetic_correct_cmd(
    path: String,
    output_dir: String,
    master_dark_path: Option<String>,
    config: CosmeticConfig,
    defect_list_text: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let out_dir = resolve_output_dir(&output_dir)?;
        let cfg = resolve_config(config, defect_list_text.as_deref())?;
        let dark = load_master_dark(master_dark_path.as_deref())?;
        let mut body = correct_one(&path, &out_dir, dark.as_ref().map(ImageEntry::arr), &cfg)?;
        if let Some(obj) = body.as_object_mut() {
            obj.insert(RES_ELAPSED_MS.to_string(), json!(t0.elapsed().as_millis() as u64));
        }
        Ok(body)
    })
}

#[tauri::command]
pub async fn cosmetic_correct_batch_cmd(
    paths: Vec<String>,
    output_dir: String,
    master_dark_path: Option<String>,
    config: CosmeticConfig,
    defect_list_text: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let out_dir = resolve_output_dir(&output_dir)?;
        let cfg = resolve_config(config, defect_list_text.as_deref())?;
        let dark = load_master_dark(master_dark_path.as_deref())?;
        let dark_arr = dark.as_ref().map(ImageEntry::arr);

        let mut results = Vec::with_capacity(paths.len());
        let mut ok_count = 0usize;
        for path in &paths {
            match correct_one(path, &out_dir, dark_arr, &cfg) {
                Ok(item) => {
                    ok_count += 1;
                    results.push(item);
                }
                Err(e) => results.push(json!({
                    RES_PATH: path,
                    RES_ERROR: format!("{:#}", e),
                })),
            }
        }

        Ok(json!({
            RES_RESULTS: results,
            RES_SUCCEEDED: ok_count,
            RES_FAILED: paths.len() - ok_count,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::cosmetic::Defect;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef;

    fn mef(dir: &tempfile::TempDir, name: &str, cols: usize, rows: usize) -> String {
        let path = dir.path().join(name);
        sci_err_dq_mef(&path, cols, rows, vec![0i32; cols * rows]);
        path.to_str().unwrap().to_string()
    }

    fn list_only(text: &str) -> CosmeticConfig {
        resolve_config(CosmeticConfig { use_master_dark: false, ..Default::default() }, Some(text)).unwrap()
    }

    #[test]
    fn resolve_config_appends_parsed_defects_and_reports_bad_lines() {
        let base = CosmeticConfig { use_master_dark: false, defects: vec![Defect::Point { x: 0, y: 0 }], ..Default::default() };
        let cfg = resolve_config(base.clone(), Some("Point 1 2\nRow 3")).unwrap();
        assert_eq!(cfg.defects.len(), 3);
        assert_eq!(cfg.defects[2], Defect::Row { y: 3, x0: None, x1: None });
        let untouched = resolve_config(base.clone(), None).unwrap();
        assert_eq!(untouched.defects.len(), 1);
        let err = resolve_config(base, Some("Point 1 2\nBogus")).unwrap_err().to_string();
        assert!(err.contains("Line 2"), "{}", err);
    }

    #[test]
    fn load_master_dark_ignores_blank_paths() {
        assert!(load_master_dark(None).unwrap().is_none());
        assert!(load_master_dark(Some("   ")).unwrap().is_none());
        let err = load_master_dark(Some("C:/definitely/missing_dark.fits")).err().expect("missing file fails");
        assert!(format!("{:#}", err).contains("master dark"), "{:#}", err);
    }

    #[test]
    fn correct_one_writes_outputs_and_flags_dq_companions() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef(&dir, "light.fits", 8, 8);
        let out_dir = dir.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let out = out_dir.to_str().unwrap().to_string();

        let sci = format!("{}#hdu=1", p);
        let body = correct_one(&sci, &out, None, &list_only("Point 1 1\nCol 7 2 4")).unwrap();
        assert_eq!(body[RES_PATH], sci);
        assert_eq!(body[RES_DIMENSIONS], json!([8, 8]));
        assert_eq!(body[KEY_COUNTS]["flagged"], 4);
        assert_eq!(body[KEY_COUNTS]["listed"], 4);
        assert_eq!(body[KEY_COUNTS]["replaced"], 4);
        assert_eq!(body[KEY_COUNTS]["hot"], 0);
        assert_eq!(body[KEY_DQ_PRESENT], true);
        assert!(body[KEY_WARNING].as_str().unwrap().contains("DQ plane"));
        let png = body[RES_PNG_PATH].as_str().unwrap();
        let fits = body[RES_FITS_PATH].as_str().unwrap();
        assert!(png.ends_with("light_hdu1_cosmetic.png"), "{}", png);
        assert!(fits.ends_with("light_hdu1_cosmetic.fits"), "{}", fits);
        assert!(std::path::Path::new(png).exists());
        assert!(std::path::Path::new(fits).exists());
        let written = crate::infra::fits::reader::read_primary_header(fits).unwrap();
        assert_eq!(written.get("EXTNAME"), Some("SCI"));
        assert_eq!(written.get("BUNIT"), Some("MJy/sr"));
        assert!(written.get("XTENSION").is_none());

        let lonely = format!("{}#hdu=4", p);
        let body = correct_one(&lonely, &out, None, &list_only("Point 0 0")).unwrap();
        assert_eq!(body[KEY_DQ_PRESENT], false);
        assert!(body[KEY_WARNING].is_null());
    }

    #[test]
    fn correct_one_uses_the_master_dark_and_checks_its_shape() {
        let dir = tempfile::tempdir().unwrap();
        let light = mef(&dir, "light.fits", 8, 8);
        let small = mef(&dir, "small.fits", 4, 4);
        let out = dir.path().to_str().unwrap().to_string();

        let dark_ref = format!("{}#hdu=2", light);
        let dark = load_master_dark(Some(dark_ref.as_str())).unwrap().unwrap();
        let cfg = CosmeticConfig::default();
        let body = correct_one(&light, &out, Some(dark.arr()), &cfg).unwrap();
        assert_eq!(body[KEY_COUNTS]["flagged"], 0);

        let mismatched = load_master_dark(Some(small.as_str())).unwrap().unwrap();
        let err = correct_one(&light, &out, Some(mismatched.arr()), &cfg).unwrap_err().to_string();
        assert!(err.contains("Master dark"), "{}", err);

        let err = correct_one(&light, &out, None, &cfg).unwrap_err().to_string();
        assert!(err.contains("master dark"), "{}", err);
    }
}
