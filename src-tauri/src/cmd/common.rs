use std::fs::File;

use anyhow::{Context, Result};
use ndarray::Array2;
use serde_json::json;

use crate::core::imaging::dq_flags::{exclusion_map, DqTable};
use crate::core::imaging::normalize::robust_asinh_preview;
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{auto_stf, apply_stf, AutoStfConfig};
use crate::infra::cache::{GLOBAL_IMAGE_CACHE, ImageEntry, PlaneLoad};
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::image_source::{
    load_companions_into, load_plane, load_plane_header, resolve_plane_info, LoadedPlane,
};
use crate::infra::render::grayscale::{render_grayscale, save_stf_png};
use crate::types::constants::{
    PLANE_KIND_ARRAY, PLANE_KIND_HDU, RES_DQ_REF, RES_DQ_TABLE, RES_ERR_REF, RES_EXTNAME,
    RES_EXTVER, RES_INDEX, RES_IS_DQ, RES_IS_ERR, RES_KEY, RES_KIND, RES_SOURCE_PATH,
};
use crate::types::header::HduHeader;
use crate::types::image_ref::{ImageRef, PlaneSelector};

pub(crate) use crate::infra::image_source::LoadedCompanions;

pub(crate) const MAX_PREVIEW_DIM: usize = 4096;

pub(crate) fn auto_stretch_preview(arr: &Array2<f32>) -> Vec<u8> {
    let stats = compute_image_stats(arr);
    let stf = auto_stf(&stats, &AutoStfConfig::default());
    apply_stf(arr, &stf, &stats)
}

pub(crate) struct ResolvedImage {
    pub arr: Array2<f32>,
    pub header: HduHeader,
    pub _tmp: Option<tempfile::TempDir>,
}

pub(crate) fn image_ref(path: &str) -> ImageRef {
    ImageRef::parse(path)
}

pub(crate) fn source_path(path: &str) -> String {
    ImageRef::parse(path).path
}

pub(crate) fn output_stem(path: &str) -> String {
    ImageRef::parse(path).output_stem()
}

pub(crate) fn extract_image_resolved(path: &str) -> Result<ResolvedImage> {
    let loaded = load_plane(&image_ref(path))?;
    Ok(ResolvedImage {
        arr: loaded.arr,
        header: loaded.header,
        _tmp: loaded._tmp,
    })
}

fn plane_load(r: &ImageRef) -> Result<PlaneLoad> {
    load_plane(r).map(LoadedPlane::into_plane_load)
}

fn load_plane_entry(key: &str) -> Result<ImageEntry> {
    GLOBAL_IMAGE_CACHE.get_or_load_plane(key, || plane_load(&image_ref(key)))
}

pub(crate) fn load_cached(path: &str) -> Result<ImageEntry> {
    if let Some(entry) = GLOBAL_IMAGE_CACHE.get(path) {
        return Ok(entry);
    }
    let entry = load_plane_entry(path)?;
    record_preview_stamp(path);
    Ok(entry)
}

pub(crate) fn load_cached_full(path: &str) -> Result<ImageEntry> {
    if let Some(entry) = GLOBAL_IMAGE_CACHE.get(path) {
        if entry.header().is_some() {
            return Ok(entry);
        }
        if let Ok(upgraded) =
            GLOBAL_IMAGE_CACHE.upgrade_header(path, || load_plane_header(&image_ref(path)))
        {
            return Ok(upgraded);
        }
    }
    load_plane_entry(path)
}

pub(crate) fn load_from_cache_or_disk(path: &str) -> Result<ImageEntry> {
    load_cached(path)
}

pub(crate) fn load_companions(path: &str) -> Result<LoadedCompanions> {
    let active = load_cached(path)?;
    Ok(load_companions_into(&GLOBAL_IMAGE_CACHE, &active, plane_load))
}

pub(crate) fn dq_exclusion(path: &str) -> Result<Option<Array2<u8>>> {
    let comps = load_companions(path)?;
    Ok(comps.dq.and_then(|(entry, table)| {
        entry
            .int_plane()
            .map(|plane| exclusion_map(&plane.bits, table.exclusion_mask()))
    }))
}

pub(crate) fn plane_info_json(path: &str, entry: &ImageEntry) -> Result<serde_json::Value> {
    let r = image_ref(path);
    let info = match entry.plane_info() {
        Some(info) => info.clone(),
        None => resolve_plane_info(&r)?,
    };
    let comps = entry.companions().cloned().unwrap_or_default();
    let (kind, index, key) = match &info.kind {
        PlaneSelector::Hdu(n) => (PLANE_KIND_HDU, Some(*n), None),
        PlaneSelector::Array(k) => (PLANE_KIND_ARRAY, None, Some(k.clone())),
        PlaneSelector::Auto => (PLANE_KIND_HDU, None, None),
    };
    let dq_ref = comps.dq.as_ref().map(ImageRef::cache_key);
    let err_ref = comps.err.as_ref().map(ImageRef::cache_key);
    let dq_table = match (&dq_ref, entry.header()) {
        (Some(_), Some(header)) => Some(DqTable::select(header)),
        _ => None,
    };
    Ok(json!({
        RES_KIND: kind,
        RES_INDEX: index,
        RES_KEY: key,
        RES_EXTNAME: info.extname,
        RES_EXTVER: info.extver,
        RES_IS_DQ: info.is_dq,
        RES_IS_ERR: info.is_err,
        RES_DQ_REF: dq_ref,
        RES_ERR_REF: err_ref,
        RES_SOURCE_PATH: r.path,
        RES_DQ_TABLE: dq_table,
    }))
}

type FileStamp = (u64, Option<std::time::SystemTime>);

static PREVIEW_STAMPS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, FileStamp>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

pub(crate) fn record_preview_stamp(path: &str) {
    if let Ok(m) = std::fs::metadata(source_path(path)) {
        let stamp: FileStamp = (m.len(), m.modified().ok());
        let mut stamps = PREVIEW_STAMPS.lock().unwrap();
        if stamps.len() > 1024 {
            stamps.clear();
        }
        stamps.insert(path.to_string(), stamp);
    }
}

pub(crate) fn load_preview_validated(path: &str) -> Result<ImageEntry> {
    if let Ok(m) = std::fs::metadata(source_path(path)) {
        let stamp: FileStamp = (m.len(), m.modified().ok());
        let mut stamps = PREVIEW_STAMPS.lock().unwrap();
        match stamps.get(path) {
            Some(s) if *s == stamp => {}
            Some(_) => {
                GLOBAL_IMAGE_CACHE.invalidate(path);
                stamps.insert(path.to_string(), stamp);
            }
            None => {
                stamps.insert(path.to_string(), stamp);
            }
        }
    }
    load_from_cache_or_disk(path)
}

fn downsample_nn<const BPP: usize>(
    pixels: &[u8],
    width: usize,
    height: usize,
    max_dim: usize,
) -> (Vec<u8>, usize, usize) {
    if width <= max_dim && height <= max_dim {
        return (pixels.to_vec(), width, height);
    }

    let scale = max_dim as f64 / (width.max(height) as f64);
    let dst_w = ((width as f64) * scale).round().max(1.0) as usize;
    let dst_h = ((height as f64) * scale).round().max(1.0) as usize;

    let y_ratio = height as f64 / dst_h as f64;
    let x_ratio = width as f64 / dst_w as f64;

    let mut out = vec![0u8; dst_w * dst_h * BPP];

    for dy in 0..dst_h {
        let sy = ((dy as f64) * y_ratio).min((height - 1) as f64) as usize;
        let src_row = sy * width;
        let dst_row = dy * dst_w;
        for dx in 0..dst_w {
            let sx = ((dx as f64) * x_ratio).min((width - 1) as f64) as usize;
            let si = (src_row + sx) * BPP;
            let di = (dst_row + dx) * BPP;
            out[di..di + BPP].copy_from_slice(&pixels[si..si + BPP]);
        }
    }

    (out, dst_w, dst_h)
}

pub(crate) fn downsample_u8(pixels: &[u8], width: usize, height: usize, max_dim: usize) -> (Vec<u8>, usize, usize) {
    downsample_nn::<1>(pixels, width, height, max_dim)
}

pub(crate) fn save_preview_png(pixels: Vec<u8>, width: usize, height: usize, path: &str) -> Result<()> {
    let (preview, pw, ph) = downsample_u8(&pixels, width, height, MAX_PREVIEW_DIM);
    save_stf_png(preview, pw, ph, path)
}

fn make_filename(stem: &str, suffix: &str, ext: &str) -> String {
    if suffix.is_empty() {
        format!("{}.{}", stem, ext)
    } else {
        format!("{}_{}.{}", stem, suffix, ext)
    }
}

pub(crate) struct RenderOutput {
    pub png_path: String,
    pub fits_path: Option<String>,
    pub dims: (usize, usize),
}

pub(crate) fn render_and_save(
    arr: &Array2<f32>,
    path: &str,
    output_dir: &str,
    suffix: &str,
    write_fits: bool,
) -> Result<RenderOutput> {
    let rendered = auto_stretch_preview(arr);

    let stem = output_stem(path);

    let png_path = format!("{}/{}", output_dir, make_filename(&stem, suffix, "png"));
    let (rows, cols) = arr.dim();
    save_preview_png(rendered, cols, rows, &png_path)?;

    let fits_path = if write_fits {
        let fp = format!("{}/{}", output_dir, make_filename(&stem, suffix, "fits"));
        crate::infra::fits::writer::write_fits_mono(&fp, arr, None)?;
        Some(fp)
    } else {
        None
    };

    Ok(RenderOutput {
        png_path,
        fits_path,
        dims: (rows, cols),
    })
}

pub(crate) fn render_asinh_and_save(
    arr: &Array2<f32>,
    output_dir: &str,
    name: &str,
    write_fits: bool,
) -> Result<(String, Option<String>)> {
    let normalized = robust_asinh_preview(arr);
    let png_path = format!("{}/{}.png", output_dir, name);
    render_grayscale(&normalized, &png_path)?;

    let fits_path = if write_fits {
        let fp = format!("{}/{}.fits", output_dir, name);
        crate::infra::fits::writer::write_fits_mono(&fp, arr, None)?;
        Some(fp)
    } else {
        None
    };

    Ok((png_path, fits_path))
}

fn platform_fallback_dir() -> std::path::PathBuf {
    if let Some(data) = dirs::data_dir() {
        return data.join("AstroBurst").join("output");
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".astroburst").join("output");
    }
    std::path::PathBuf::from("/tmp/astroburst/output")
}

pub(crate) fn resolve_output_dir(output_dir: &str) -> Result<String> {
    let path = std::path::Path::new(output_dir);
    if path.exists() {
        maybe_enforce_lru(output_dir);
        return Ok(output_dir.to_string());
    }
    match std::fs::create_dir_all(path) {
        Ok(_) => Ok(output_dir.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied
            || e.raw_os_error() == Some(5)
            || e.raw_os_error() == Some(30) => {
            let fallback = platform_fallback_dir();
            std::fs::create_dir_all(&fallback)
                .context("Failed to create fallback output directory")?;
            eprintln!(
                "[AstroBurst] Permission denied on '{}', falling back to '{}'",
                output_dir,
                fallback.display()
            );
            let resolved = fallback.to_string_lossy().to_string();
            maybe_enforce_lru(&resolved);
            Ok(resolved)
        }
        Err(e) => Err(e).context(format!("Failed to create output directory: {}", output_dir)),
    }
}

fn maybe_enforce_lru(dir: &str) {
    use crate::types::constants::DEFAULT_OUTPUT_MAX_BYTES;
    static MAX_BYTES: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

    let threshold = *MAX_BYTES.get_or_init(|| {
        crate::infra::config::load_config()
            .ok()
            .and_then(|cfg| cfg.output_max_size_mb)
            .map(|mb| mb * 1_048_576)
            .unwrap_or(DEFAULT_OUTPUT_MAX_BYTES)
    });

    let _ = crate::cmd::output::enforce_output_lru(std::path::Path::new(dir), threshold);
}

pub(crate) struct ResolvedRgbImage {
    pub r: Array2<f32>,
    pub g: Array2<f32>,
    pub b: Array2<f32>,
    pub header: HduHeader,
    pub _tmp: Option<tempfile::TempDir>,
}

pub(crate) fn try_extract_rgb_resolved(path: &str) -> Result<Option<ResolvedRgbImage>> {
    let r = image_ref(path);
    if !r.is_auto() {
        return Ok(None);
    }
    let p = std::path::Path::new(&r.path);
    if crate::infra::asdf::converter::is_asdf_file(p) {
        return Ok(None);
    }

    let (fits_path, tmp) = resolve_single_image(&r.path)?;
    let file = File::open(&fits_path)
        .with_context(|| format!("Failed to open {}", fits_path.display()))?;

    match crate::infra::fits::reader::try_extract_rgb_mmap(&file)? {
        Some(result) => Ok(Some(ResolvedRgbImage {
            r: result.r,
            g: result.g,
            b: result.b,
            header: result.header,
            _tmp: tmp,
        })),
        None => Ok(None),
    }
}

macro_rules! blocking_cmd {
    ($body:expr) => {
        tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> { $body })
            .await
            .map_err(|e| format!("Task join failed: {}", e))?
            .map_err(|e| format!("{:#}", e))
    };
}

pub(crate) use blocking_cmd;

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef_with_dq_cards;

    fn mef(dir: &tempfile::TempDir, name: &str) -> String {
        let path = dir.path().join(name);
        let mut dq = vec![0i32; 16];
        dq[0] = 1 - 2147483647 - 1;
        dq[5] = 3 - 2147483647 - 1;
        dq[10] = 2 - 2147483647 - 1;
        sci_err_dq_mef_with_dq_cards(&path, 4, 4, dq, vec![("TELESCOP", "'JWST'".into())]);
        path.to_str().unwrap().to_string()
    }

    fn zipped_mef(dir: &tempfile::TempDir, name: &str) -> String {
        let inner = mef(dir, "inner.fits");
        let zip_path = dir.path().join(name);
        let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        writer.start_file("inner.fits", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(&std::fs::read(&inner).unwrap()).unwrap();
        writer.finish().unwrap();
        std::fs::remove_file(&inner).unwrap();
        zip_path.to_str().unwrap().to_string()
    }

    #[test]
    fn output_stem_and_source_path_on_refs() {
        assert_eq!(output_stem("C:/d/jw01234_cal.fits#hdu=3"), "jw01234_cal_hdu3");
        assert_eq!(output_stem("C:/d/r0000.asdf#array=roman.dq"), "r0000_roman_dq");
        assert_eq!(output_stem("C:/d/plain.fits"), "plain");
        assert_eq!(source_path("C:/d/plain.fits#hdu=3"), "C:/d/plain.fits");
        assert_eq!(source_path("C:/d/plain.fits"), "C:/d/plain.fits");
        assert_eq!(source_path("__composite_r"), "__composite_r");
        assert!(image_ref("a.fits#hdu=1") == ImageRef::hdu("a.fits", 1));
    }

    #[test]
    fn load_cached_uses_the_ref_as_cache_key() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef(&dir, "cached.fits");
        let key = format!("{}#hdu=1", p);
        let entry = load_cached(&key).unwrap();
        assert_eq!(entry.arr().dim(), (4, 4));
        assert_eq!(entry.header().and_then(|h| h.get("EXTNAME")), Some("SCI"));
        assert!(GLOBAL_IMAGE_CACHE.contains(&key));
        assert!(!GLOBAL_IMAGE_CACHE.contains(&p));
        let dq_key = format!("{}#hdu=3", p);
        let dq = load_cached_full(&dq_key).unwrap();
        assert!(dq.int_plane().is_some());
        assert_eq!(dq.int_plane().unwrap().bits[[0, 0]], 1);
        assert!(load_cached(&format!("{}#hdu=9", p)).is_err());
        let resolved = extract_image_resolved(&format!("{}#hdu=2", p)).unwrap();
        assert_eq!(resolved.header.get("EXTNAME"), Some("ERR"));
    }

    #[test]
    fn load_companions_and_dq_exclusion_on_mef() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef(&dir, "comp.fits");
        let key = format!("{}#hdu=1", p);
        let active_table = load_cached(&key).unwrap().header().map(DqTable::select).unwrap();
        assert_eq!(active_table, DqTable::Unknown);
        let comps = load_companions(&key).unwrap();
        let (dq_entry, table) = comps.dq.expect("dq companion");
        assert_eq!(table, DqTable::Jwst);
        assert_eq!(table.decode(3), vec!["DO_NOT_USE", "SATURATED"]);
        assert_eq!(dq_entry.int_plane().unwrap().bits[[1, 1]], 3);
        let err_entry = comps.err.expect("err companion");
        assert_eq!(err_entry.arr()[[0, 1]], 0.5);
        assert!(GLOBAL_IMAGE_CACHE.contains(&format!("{}#hdu=3", p)));

        let excl = dq_exclusion(&key).unwrap().expect("exclusion map");
        assert_eq!(excl.iter().filter(|&&v| v == 1).count(), 2);
        assert_eq!(excl[[0, 0]], 1);
        assert_eq!(excl[[1, 1]], 1);
        assert_eq!(excl[[2, 2]], 0);

        let lonely = format!("{}#hdu=4", p);
        assert!(dq_exclusion(&lonely).unwrap().is_none());
        assert!(load_companions(&lonely).unwrap().err.is_none());
    }

    #[test]
    fn companions_of_a_zipped_mef_come_from_the_cache_after_the_first_load() {
        let dir = tempfile::tempdir().unwrap();
        let p = zipped_mef(&dir, "jw_cal.zip");
        let first = load_companions(&p).unwrap();
        assert!(first.dq.is_some());
        assert!(first.err.is_some());
        assert!(GLOBAL_IMAGE_CACHE.contains(&format!("{}#hdu=3", p)));

        std::fs::remove_file(&p).unwrap();
        assert!(!std::path::Path::new(&p).exists());

        let second = load_companions(&p).unwrap();
        let (dq, table) = second.dq.expect("dq companion served from the cache");
        assert_eq!(table, DqTable::Jwst);
        assert_eq!(dq.int_plane().unwrap().bits[[1, 1]], 3);
        assert_eq!(second.err.expect("err companion served from the cache").arr()[[0, 1]], 0.5);
        assert!(dq_exclusion(&p).unwrap().is_some());

        let entry = load_cached(&p).unwrap();
        let j = plane_info_json(&p, &entry).unwrap();
        assert_eq!(j[RES_DQ_REF], format!("{}#hdu=3", p));
        assert_eq!(j[RES_INDEX], 1);
        let err_plane = load_companions(&format!("{}#hdu=2", p)).unwrap();
        assert!(err_plane.dq.is_some());
        assert!(load_companions(&format!("{}#hdu=4", p)).is_err());
    }

    #[test]
    fn plane_info_json_reports_kind_and_companions() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef(&dir, "info.fits");
        let entry = load_cached_full(&p).unwrap();
        let j = plane_info_json(&p, &entry).unwrap();
        assert_eq!(j[RES_KIND], "hdu");
        assert_eq!(j[RES_INDEX], 1);
        assert!(j[RES_KEY].is_null());
        assert_eq!(j[RES_EXTNAME], "SCI");
        assert_eq!(j[RES_EXTVER], 1);
        assert_eq!(j[RES_IS_DQ], false);
        assert_eq!(j[RES_DQ_REF], format!("{}#hdu=3", p));
        assert_eq!(j[RES_ERR_REF], format!("{}#hdu=2", p));
        assert_eq!(j[RES_SOURCE_PATH], p);
        assert_eq!(j[RES_DQ_TABLE], "unknown");

        let lonely = format!("{}#hdu=4", p);
        let entry = load_cached_full(&lonely).unwrap();
        let j = plane_info_json(&lonely, &entry).unwrap();
        assert!(j[RES_DQ_REF].is_null());
        assert!(j[RES_DQ_TABLE].is_null());
        assert_eq!(j[RES_INDEX], 4);

        let synthetic_key = "__composite_info";
        GLOBAL_IMAGE_CACHE.insert_synthetic(
            synthetic_key,
            std::sync::Arc::new(Array2::<f32>::zeros((2, 2))),
            compute_image_stats(&Array2::<f32>::zeros((2, 2))),
        );
        let synthetic = GLOBAL_IMAGE_CACHE.get(synthetic_key).unwrap();
        assert!(plane_info_json(synthetic_key, &synthetic).is_err());
    }
}
