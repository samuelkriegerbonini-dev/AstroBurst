use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use ndarray::Array2;
use serde::Serialize;
use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, load_cached, load_cached_full, load_companions, output_stem, reduce_for_preview,
    render_and_save, resolve_output_dir, write_derived_fits, MAX_PREVIEW_DIM,
};
use crate::core::imaging::cosmetic::{
    cosmetic_correct_with_cfa, cosmetic_defect_map, count_flags, effective_cfa, parse_defect_list, CosmeticConfig,
    CosmeticResult, FLAG_COLD, FLAG_HOT,
};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{apply_stf, auto_stf, AutoStfConfig};
use crate::infra::cache::ImageEntry;
use crate::types::constants::{
    RES_CFA_APPLIED, RES_CFA_SOURCE, RES_DIMENSIONS, RES_ELAPSED_MS, RES_ERROR, RES_FAILED, RES_FITS_PATH,
    RES_PATH, RES_PNG_PATH, RES_RESULTS, RES_SUCCEEDED,
};
use crate::types::header::HduHeader;

pub const KEY_COUNTS: &str = "counts";
pub const KEY_DQ_PRESENT: &str = "dq_present";
pub const KEY_WARNING: &str = "warning";
pub const SUFFIX_COSMETIC: &str = "cosmetic";
pub const KEY_DRY_RUN: &str = "dry_run";
pub const KEY_DEFECTS: &str = "defects";
pub const KEY_DEFECTS_TOTAL: &str = "defects_total";
pub const SUFFIX_DEFECTS: &str = "defects";
const MAX_LISTED_DEFECTS: usize = 5000;
const DEFECT_MARK_RADIUS: usize = 1;
const DEFECT_MARK_RGB: [u8; 3] = [255, 0, 0];
const DEFECT_HOT: &str = "hot";
const DEFECT_COLD: &str = "cold";
const DEFECT_LISTED: &str = "listed";
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
    let (cfa, cfa_source) = effective_cfa(cfg, entry.header());
    let result = cosmetic_correct_with_cfa(entry.arr(), master_dark, cfg, cfa)?;
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
        RES_CFA_APPLIED: cfa,
        RES_CFA_SOURCE: cfa_source,
    }))
}

fn defect_kind(flag: u8) -> &'static str {
    if flag & FLAG_HOT != 0 {
        DEFECT_HOT
    } else if flag & FLAG_COLD != 0 {
        DEFECT_COLD
    } else {
        DEFECT_LISTED
    }
}

fn defect_entries(map: &Array2<u8>, limit: usize) -> (Vec<serde_json::Value>, usize) {
    let mut entries = Vec::new();
    let mut total = 0usize;
    for ((y, x), &flag) in map.indexed_iter() {
        if flag == 0 {
            continue;
        }
        total += 1;
        if entries.len() < limit {
            entries.push(json!([x, y, defect_kind(flag)]));
        }
    }
    (entries, total)
}

fn defect_overlay_rgb(grey: &[u8], shown_rows: usize, shown_cols: usize, map: &Array2<u8>) -> Vec<u8> {
    let (rows, cols) = map.dim();
    let mut rgb: Vec<u8> = grey.iter().flat_map(|&g| [g, g, g]).collect();
    if shown_rows == 0 || shown_cols == 0 {
        return rgb;
    }
    for ((y, x), &flag) in map.indexed_iter() {
        if flag == 0 {
            continue;
        }
        let sy = y * shown_rows / rows;
        let sx = x * shown_cols / cols;
        for my in sy.saturating_sub(DEFECT_MARK_RADIUS)..=(sy + DEFECT_MARK_RADIUS).min(shown_rows - 1) {
            for mx in sx.saturating_sub(DEFECT_MARK_RADIUS)..=(sx + DEFECT_MARK_RADIUS).min(shown_cols - 1) {
                let o = (my * shown_cols + mx) * 3;
                rgb[o..o + 3].copy_from_slice(&DEFECT_MARK_RGB);
            }
        }
    }
    rgb
}

fn save_defect_png(image: &Array2<f32>, map: &Array2<u8>, png_path: &str) -> Result<()> {
    let stats = compute_image_stats(image);
    let stf = auto_stf(&stats, &AutoStfConfig::default());
    let reduced = reduce_for_preview(image, MAX_PREVIEW_DIM);
    let shown = reduced.as_ref().unwrap_or(image);
    let (shown_rows, shown_cols) = shown.dim();
    let rgb = defect_overlay_rgb(&apply_stf(shown, &stf, &stats), shown_rows, shown_cols, map);
    image::save_buffer(png_path, &rgb, shown_cols as u32, shown_rows as u32, image::ColorType::Rgb8)
        .with_context(|| format!("Failed to write the defect preview '{}'", png_path))
}

fn dry_run_one(
    path: &str,
    out_dir: &str,
    master_dark: Option<&Array2<f32>>,
    cfg: &CosmeticConfig,
) -> Result<serde_json::Value> {
    let entry = load_cached_full(path).or_else(|_| load_cached(path))?;
    let (cfa, cfa_source) = effective_cfa(cfg, entry.header());
    let map = cosmetic_defect_map(entry.arr(), master_dark, cfg, cfa)?;
    let (flagged, hot, cold, listed) = count_flags(&map);
    let png_path = format!("{}/{}_{}.png", out_dir, output_stem(path), SUFFIX_DEFECTS);
    save_defect_png(entry.arr(), &map, &png_path)?;
    let (defects, defects_total) = defect_entries(&map, MAX_LISTED_DEFECTS);
    let (rows, cols) = map.dim();
    let dq = dq_present(path);
    let counts = CosmeticCounts { flagged, replaced: 0, hot, cold, listed };
    Ok(json!({
        KEY_DRY_RUN: true,
        RES_PATH: path,
        RES_PNG_PATH: png_path,
        RES_DIMENSIONS: [cols, rows],
        KEY_COUNTS: serde_json::to_value(counts)?,
        KEY_DEFECTS: defects,
        KEY_DEFECTS_TOTAL: defects_total,
        KEY_DQ_PRESENT: dq,
        KEY_WARNING: dq.then_some(DQ_WARNING),
        RES_CFA_APPLIED: cfa,
        RES_CFA_SOURCE: cfa_source,
    }))
}

#[tauri::command]
pub async fn cosmetic_correct_cmd(
    path: String,
    output_dir: String,
    master_dark_path: Option<String>,
    config: CosmeticConfig,
    defect_list_text: Option<String>,
    dry_run: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let out_dir = resolve_output_dir(&output_dir)?;
        let cfg = resolve_config(config, defect_list_text.as_deref())?;
        let dark = load_master_dark(master_dark_path.as_deref())?;
        let dark_arr = dark.as_ref().map(ImageEntry::arr);
        let mut body = if dry_run.unwrap_or(false) {
            dry_run_one(&path, &out_dir, dark_arr, &cfg)?
        } else {
            correct_one(&path, &out_dir, dark_arr, &cfg)?
        };
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
    use crate::core::imaging::cosmetic::{Defect, FLAG_LISTED};
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
    fn correct_one_reports_the_cfa_source() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        let out = out.to_str().unwrap().to_string();

        let mut header = HduHeader::empty();
        header.set("BAYERPAT", "'RGGB'".to_string());
        let bayer = dir.path().join("bayer.fits").to_str().unwrap().to_string();
        let frame = Array2::from_shape_fn((8, 8), |(y, x)| if y % 2 == 0 && x % 2 == 0 { 1000.0 } else { 100.0 });
        crate::infra::fits::writer::write_fits_mono(&bayer, &frame, Some(&header)).unwrap();

        let auto = list_only("Point 2 2");
        let body = correct_one(&bayer, &out, None, &auto).unwrap();
        assert_eq!(body[RES_CFA_APPLIED], true);
        assert_eq!(body[RES_CFA_SOURCE], "header");
        let written = crate::infra::fits::reader::load_fits_image(body[RES_FITS_PATH].as_str().unwrap()).unwrap();
        assert!((written[[2, 2]] - 1000.0).abs() < 1e-3, "{}", written[[2, 2]]);

        let explicit = CosmeticConfig { cfa: Some(false), ..auto.clone() };
        let body = correct_one(&bayer, &out, None, &explicit).unwrap();
        assert_eq!(body[RES_CFA_APPLIED], false);
        assert_eq!(body[RES_CFA_SOURCE], "explicit");

        let plain = mef(&dir, "plain.fits", 8, 8);
        let body = correct_one(&format!("{plain}#hdu=1"), &out, None, &auto).unwrap();
        assert_eq!(body[RES_CFA_APPLIED], false);
        assert_eq!(body[RES_CFA_SOURCE], "none");
    }

    #[test]
    fn dry_run_returns_the_defect_list_and_writes_no_fits() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        let out = out.to_str().unwrap().to_string();
        let light = dir.path().join("light.fits").to_str().unwrap().to_string();
        let mut frame = Array2::from_shape_fn((32, 32), |(y, x)| 100.0 + ((y * 7 + x * 13) % 5) as f32);
        frame[[5, 9]] = 5000.0;
        frame[[20, 3]] = -4000.0;
        crate::infra::fits::writer::write_fits_mono(&light, &frame, None).unwrap();

        let cfg = CosmeticConfig {
            use_master_dark: false,
            auto_hot_sigma: Some(5.0),
            auto_cold_sigma: Some(5.0),
            defects: vec![Defect::Point { x: 30, y: 1 }],
            ..Default::default()
        };
        let body = dry_run_one(&light, &out, None, &cfg).unwrap();
        assert_eq!(body[KEY_DRY_RUN], true);
        assert_eq!(body[KEY_COUNTS]["hot"], 1);
        assert_eq!(body[KEY_COUNTS]["cold"], 1);
        assert_eq!(body[KEY_COUNTS]["listed"], 1);
        assert_eq!(body[KEY_COUNTS]["flagged"], 3);
        assert_eq!(body[KEY_COUNTS]["replaced"], 0);
        assert_eq!(body[KEY_DEFECTS_TOTAL], 3);
        assert_eq!(body[KEY_DEFECTS], json!([[30, 1, "listed"], [9, 5, "hot"], [3, 20, "cold"]]));
        assert_eq!(body[RES_DIMENSIONS], json!([32, 32]));
        assert!(body.get(RES_FITS_PATH).is_none());

        let png = body[RES_PNG_PATH].as_str().unwrap();
        assert!(png.ends_with("light_defects.png"), "{}", png);
        let rgb = image::open(png).unwrap().to_rgb8();
        assert_eq!(rgb.dimensions(), (32, 32));
        assert_eq!(rgb.get_pixel(9, 5).0, [255, 0, 0]);
        assert_eq!(rgb.get_pixel(10, 6).0, [255, 0, 0]);
        let grey = rgb.get_pixel(20, 12).0;
        assert_eq!((grey[0], grey[1]), (grey[1], grey[2]), "unflagged pixels stay grey");

        let fits: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".fits"))
            .collect();
        assert!(fits.is_empty(), "dry run wrote {:?}", fits);
    }

    #[test]
    fn defect_entries_cap_the_list_and_keep_the_total() {
        let mut map = Array2::<u8>::zeros((4, 4));
        map[[0, 1]] = FLAG_HOT | FLAG_LISTED;
        map[[2, 2]] = FLAG_COLD;
        map[[3, 0]] = FLAG_HOT;
        let (entries, total) = defect_entries(&map, 2);
        assert_eq!(total, 3);
        assert_eq!(entries, vec![json!([1, 0, "hot"]), json!([2, 2, "cold"])]);
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
