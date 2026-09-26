use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::{LazyLock, Mutex, MutexGuard};

use anyhow::{Context, Result};
use ndarray::Array2;
use rayon::prelude::*;
use serde_json::json;

use crate::core::analysis::photometry::SATURATION_KEYWORDS;
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::imaging::dq_flags::{exclusion_map, DqTable};
use crate::core::imaging::sampling::{cell_range, preview_dims};
use crate::core::imaging::stats::{compute_image_stats, is_valid_pixel};
use crate::core::imaging::stf::{auto_stf, apply_stf, AutoStfConfig, ImageStats, StfParams};
use crate::core::metadata::photcal::{
    GENERIC_ZERO_POINT_KEYS, ROMAN_CONVERSION_MJY_KEYS, ROMAN_PIXEL_AREA_SR_KEYS,
};
use crate::infra::cache::{GLOBAL_IMAGE_CACHE, ImageEntry, PlaneLoad};
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::writer::is_layout_card;
use crate::infra::image_source::{
    load_companions_into, load_plane, load_plane_header, resolve_plane_info, LoadedPlane,
};
use crate::infra::render::grayscale::save_stf_png;
use crate::types::constants::{
    PLANE_KIND_ARRAY, PLANE_KIND_HDU, RES_DQ_REF, RES_DQ_TABLE, RES_ERR_REF, RES_EXTNAME,
    RES_EXTVER, RES_INDEX, RES_IS_DQ, RES_IS_ERR, RES_KEY, RES_KIND, RES_SOURCE_PATH,
};
use crate::types::header::HduHeader;
use crate::types::image_ref::{ImageRef, OutputStems, PlaneSelector};

pub(crate) use crate::infra::image_source::LoadedCompanions;

pub(crate) const MAX_PREVIEW_DIM: usize = 4096;
pub(crate) const HEADER_ABPROC: &str = "ABPROC";
pub(crate) const HEADER_DISPLAY_REFERRED: &str = "ABDISP";

const DEFAULT_ABPROC: &str = "processed";
const MAX_TRACKED_STAMPS: usize = 1024;
const PREVIEW_PEAK_SIGNIFICANCE: f32 = 3.0;

const DERIVED_STRUCTURAL_CARDS: &[&str] = &["EXTNAME", "EXTVER", "EXTLEVEL", "INHERIT", "DATAMIN", "DATAMAX"];
const AXIS_INDEXED_PREFIXES: &[&str] = &["NAXIS", "CTYPE", "CRVAL", "CRPIX", "CDELT", "CUNIT", "CROTA"];
const AXIS_MATRIX_PREFIXES: &[&str] = &["CD", "PC"];
const AXIS_PARAMETER_PREFIXES: &[&str] = &["PV", "PS"];
const CALIBRATION_CARDS: &[&str] = &["BUNIT", "PIXAR_SR", "PIXAR_A2", "ZP", "ZPTMAG"];
const CALIBRATION_PREFIXES: &[&str] = &["PHOT", "ROMAN_META_PHOTOMETRY"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputValues {
    Linear,
    Rescaled,
    DisplayReferred,
}

#[cfg(test)]
pub(crate) fn auto_stretch_preview(arr: &Array2<f32>) -> Vec<u8> {
    let stats = compute_image_stats(arr);
    let stf = auto_stf(&stats, &AutoStfConfig::default());
    apply_stf(arr, &stf, &stats)
}

fn display_referred_preview(arr: &Array2<f32>) -> Vec<u8> {
    arr.iter()
        .map(|&v| if v.is_finite() { (v.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 })
        .collect()
}

fn valid_sigma(slice: &[f32]) -> f64 {
    let (n, sum, sum_sq) = slice
        .par_chunks(65536)
        .map(|chunk| {
            chunk.iter().filter(|v| is_valid_pixel(**v)).fold((0u64, 0.0f64, 0.0f64), |(n, s, q), &v| {
                (n + 1, s + v as f64, q + (v as f64) * (v as f64))
            })
        })
        .reduce(|| (0, 0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2));
    if n == 0 {
        return 0.0;
    }
    let mean = sum / n as f64;
    (sum_sq / n as f64 - mean * mean).max(0.0).sqrt()
}

fn reduce_for_preview(arr: &Array2<f32>, max_dim: usize) -> Option<Array2<f32>> {
    let (rows, cols) = arr.dim();
    if rows <= max_dim && cols <= max_dim {
        return None;
    }
    let standard = arr.as_standard_layout();
    let slice = standard.as_slice()?;
    let (dst_rows, dst_cols) = preview_dims(rows, cols, max_dim);
    let peak_scale = (PREVIEW_PEAK_SIGNIFICANCE * valid_sigma(slice) as f32).max(0.0);
    let mut out = vec![f32::NAN; dst_rows * dst_cols];
    out.par_chunks_mut(dst_cols).enumerate().for_each(|(dy, row)| {
        let (y0, y1) = cell_range(dy, rows, dst_rows);
        for (dx, px) in row.iter_mut().enumerate() {
            let (x0, x1) = cell_range(dx, cols, dst_cols);
            let mut sum = 0.0f64;
            let mut count = 0u64;
            let mut peak = f32::MIN;
            for y in y0..y1 {
                let line = slice.get(y * cols + x0..y * cols + x1).unwrap_or(&[]);
                for &v in line.iter().filter(|v| is_valid_pixel(**v)) {
                    sum += v as f64;
                    count += 1;
                    peak = peak.max(v);
                }
            }
            if count == 0 {
                continue;
            }
            let mean = (sum / count as f64) as f32;
            let d = peak - mean;
            *px = if d > 0.0 { mean + d * (d / (d + peak_scale)) } else { mean };
        }
    });
    Array2::from_shape_vec((dst_rows, dst_cols), out).ok()
}

fn save_reduced_preview(arr: &Array2<f32>, png_path: &str, render: impl FnOnce(&Array2<f32>) -> Vec<u8>) -> Result<()> {
    let reduced = reduce_for_preview(arr, MAX_PREVIEW_DIM);
    let shown = reduced.as_ref().unwrap_or(arr);
    let (rows, cols) = shown.dim();
    save_stf_png(render(shown), cols, rows, png_path)
}

pub(crate) fn save_stf_preview_png(arr: &Array2<f32>, stf: &StfParams, stats: &ImageStats, png_path: &str) -> Result<()> {
    save_reduced_preview(arr, png_path, |shown| apply_stf(shown, stf, stats))
}

pub(crate) fn save_auto_stf_preview_png(arr: &Array2<f32>, png_path: &str) -> Result<()> {
    let stats = compute_image_stats(arr);
    let stf = auto_stf(&stats, &AutoStfConfig::default());
    save_stf_preview_png(arr, &stf, &stats, png_path)
}

fn save_output_preview(arr: &Array2<f32>, values: OutputValues, png_path: &str) -> Result<()> {
    match values {
        OutputValues::DisplayReferred => save_reduced_preview(arr, png_path, display_referred_preview),
        OutputValues::Linear | OutputValues::Rescaled => save_auto_stf_preview_png(arr, png_path),
    }
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

#[cfg(not(test))]
static OUTPUT_STEMS: LazyLock<Mutex<OutputStems>> = LazyLock::new(|| Mutex::new(OutputStems::default()));

#[cfg(not(test))]
fn with_output_stems<T>(f: impl FnOnce(&mut OutputStems) -> T) -> T {
    f(&mut OUTPUT_STEMS.lock().unwrap_or_else(|e| e.into_inner()))
}

#[cfg(test)]
thread_local! {
    static TEST_ISOLATED_OUTPUT_STEMS: std::cell::RefCell<OutputStems> = std::cell::RefCell::new(OutputStems::default());
}

#[cfg(test)]
fn with_output_stems<T>(f: impl FnOnce(&mut OutputStems) -> T) -> T {
    TEST_ISOLATED_OUTPUT_STEMS.with(|stems| f(&mut stems.borrow_mut()))
}

pub(crate) fn output_stem(path: &str) -> String {
    let r = ImageRef::parse(path);
    with_output_stems(|stems| r.output_stem_in(stems))
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

fn load_cached_with(
    path: &str,
    load: impl FnOnce(&str) -> Result<ImageEntry>,
) -> Result<(ImageEntry, Option<FileStamp>)> {
    let before = revalidate_source(path);
    if let Some(entry) = GLOBAL_IMAGE_CACHE.get(path) {
        return Ok((entry, before));
    }
    let entry = load(path)?;
    forget_if_rewritten(path, before);
    Ok((entry, before))
}

pub(crate) fn load_cached(path: &str) -> Result<ImageEntry> {
    load_cached_with(path, load_plane_entry).map(|(entry, _)| entry)
}

pub(crate) fn load_cached_full(path: &str) -> Result<ImageEntry> {
    let before = revalidate_source(path);
    if let Some(entry) = GLOBAL_IMAGE_CACHE.get(path) {
        if entry.header().is_some() {
            return Ok(entry);
        }
        if let Ok(upgraded) =
            GLOBAL_IMAGE_CACHE.upgrade_header(path, || load_plane_header(&image_ref(path)))
        {
            forget_if_rewritten(path, before);
            return Ok(upgraded);
        }
    }
    let entry = load_plane_entry(path)?;
    forget_if_rewritten(path, before);
    Ok(entry)
}

pub(crate) fn load_from_cache_or_disk(path: &str) -> Result<ImageEntry> {
    load_cached(path)
}

pub(crate) fn load_preview_validated(path: &str) -> Result<ImageEntry> {
    load_cached(path)
}

pub(crate) fn load_companions(path: &str) -> Result<LoadedCompanions> {
    let (active, before) = load_cached_with(path, load_plane_entry)?;
    let companions = load_companions_into(&GLOBAL_IMAGE_CACHE, &active, plane_load);
    forget_if_rewritten(path, before);
    Ok(companions)
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

static SOURCE_STAMPS: LazyLock<Mutex<HashMap<String, FileStamp>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn lock_stamps() -> MutexGuard<'static, HashMap<String, FileStamp>> {
    SOURCE_STAMPS.lock().unwrap_or_else(|e| e.into_inner())
}

fn file_stamp(meta: &std::fs::Metadata) -> FileStamp {
    (meta.len(), meta.modified().ok())
}

fn is_key_of_source(key: &str, source: &str) -> bool {
    let r = ImageRef::parse(key);
    !r.is_synthetic() && r.path == source
}

fn invalidate_source(source: &str) {
    GLOBAL_IMAGE_CACHE.remove_where(|key| is_key_of_source(key, source));
}

fn revalidate_source(key: &str) -> Option<FileStamp> {
    let r = ImageRef::parse(key);
    if r.is_synthetic() {
        return None;
    }
    let source = r.path;
    let meta = std::fs::metadata(&source);
    let mut stamps = lock_stamps();
    match meta {
        Ok(meta) => {
            let current = file_stamp(&meta);
            match stamps.get(&source) {
                Some(known) if *known == current => {}
                Some(_) => {
                    invalidate_source(&source);
                    stamps.insert(source, current);
                }
                None => {
                    if GLOBAL_IMAGE_CACHE.any_key(|k| is_key_of_source(k, &source)) {
                        invalidate_source(&source);
                    }
                    if stamps.len() >= MAX_TRACKED_STAMPS {
                        stamps.clear();
                    }
                    stamps.insert(source, current);
                }
            }
            Some(current)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            invalidate_source(&source);
            stamps.remove(&source);
            None
        }
        Err(_) => None,
    }
}

fn forget_if_rewritten(key: &str, before: Option<FileStamp>) {
    let r = ImageRef::parse(key);
    if r.is_synthetic() {
        return;
    }
    let now = std::fs::metadata(&r.path).ok().map(|meta| file_stamp(&meta));
    if now != before {
        invalidate_source(&r.path);
    }
}

pub(crate) fn invalidate_written(path: &str) {
    let source = source_path(path);
    let meta = std::fs::metadata(&source);
    let mut stamps = lock_stamps();
    invalidate_source(&source);
    match meta {
        Ok(meta) => {
            stamps.insert(source, file_stamp(&meta));
        }
        Err(_) => {
            stamps.remove(&source);
        }
    }
}

pub(crate) fn write_derived_fits(path: &str, arr: &Array2<f32>, header: Option<&HduHeader>) -> Result<()> {
    GLOBAL_CUBE_CACHE.invalidate(path);
    let written = crate::infra::fits::writer::write_fits_mono(path, arr, header);
    invalidate_written(path);
    written
}

fn indexed_number(digits: &str) -> Option<usize> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn indexed(key: &str, prefix: &str) -> Option<usize> {
    key.strip_prefix(prefix).and_then(indexed_number)
}

fn axis_pair(key: &str, prefix: &str) -> Option<(usize, usize)> {
    let (i, j) = key.strip_prefix(prefix)?.split_once('_')?;
    Some((indexed_number(i)?, indexed_number(j)?))
}

fn describes_a_higher_axis(key: &str) -> bool {
    AXIS_INDEXED_PREFIXES.iter().any(|p| indexed(key, p).is_some_and(|n| n >= 3))
        || AXIS_MATRIX_PREFIXES
            .iter()
            .any(|p| axis_pair(key, p).is_some_and(|(i, j)| i >= 3 || j >= 3))
        || AXIS_PARAMETER_PREFIXES
            .iter()
            .any(|p| axis_pair(key, p).is_some_and(|(i, _)| i >= 3))
}

fn is_structural_card(key: &str) -> bool {
    is_layout_card(key) || DERIVED_STRUCTURAL_CARDS.contains(&key) || describes_a_higher_axis(key)
}

fn is_calibration_card(key: &str) -> bool {
    CALIBRATION_CARDS.contains(&key)
        || CALIBRATION_PREFIXES.iter().any(|p| key.starts_with(p))
        || GENERIC_ZERO_POINT_KEYS.contains(&key)
        || SATURATION_KEYWORDS.contains(&key)
        || ROMAN_CONVERSION_MJY_KEYS.contains(&key)
        || ROMAN_PIXEL_AREA_SR_KEYS.contains(&key)
}

fn dropped_from_derived_output(key: &str, values: OutputValues) -> bool {
    is_structural_card(key) || (values != OutputValues::Linear && is_calibration_card(key))
}

pub(crate) fn derived_output_header(source: Option<&HduHeader>, abproc: &str, values: OutputValues) -> HduHeader {
    let mut header = source.cloned().unwrap_or_else(HduHeader::empty);
    let doomed: Vec<String> = header
        .cards
        .iter()
        .map(|(k, _)| k)
        .chain(header.index.keys())
        .filter(|k| dropped_from_derived_output(k.trim(), values))
        .cloned()
        .collect();
    for key in doomed {
        header.remove(&key);
    }
    if header.get("WCSAXES").is_some() {
        header.set("WCSAXES", "2".to_string());
    }
    let abproc = if abproc.trim().is_empty() { DEFAULT_ABPROC } else { abproc };
    header.set(HEADER_ABPROC, abproc.to_string());
    match values {
        OutputValues::DisplayReferred => header.set(HEADER_DISPLAY_REFERRED, "T".to_string()),
        OutputValues::Rescaled => header.remove(HEADER_DISPLAY_REFERRED),
        OutputValues::Linear => {}
    }
    header
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

pub(crate) fn cached_header(path: &str) -> Result<HduHeader> {
    revalidate_source(path);
    match GLOBAL_IMAGE_CACHE.get(path).and_then(|entry| entry.header().cloned()) {
        Some(header) => Ok(header),
        None => load_plane_header(&image_ref(path)),
    }
}

fn source_header_of(path: &str) -> Option<HduHeader> {
    if image_ref(path).is_synthetic() {
        return GLOBAL_IMAGE_CACHE.get(path).and_then(|entry| entry.header().cloned());
    }
    cached_header(path).ok()
}

pub(crate) fn render_and_save(
    arr: &Array2<f32>,
    path: &str,
    output_dir: &str,
    suffix: &str,
    write_fits: bool,
) -> Result<RenderOutput> {
    render_and_save_as(arr, path, output_dir, suffix, write_fits, OutputValues::Rescaled)
}

pub(crate) fn render_and_save_as(
    arr: &Array2<f32>,
    path: &str,
    output_dir: &str,
    suffix: &str,
    write_fits: bool,
    values: OutputValues,
) -> Result<RenderOutput> {
    let stem = output_stem(path);

    let png_path = format!("{}/{}", output_dir, make_filename(&stem, suffix, "png"));
    let (rows, cols) = arr.dim();
    save_output_preview(arr, values, &png_path)?;

    let fits_path = if write_fits {
        let fp = format!("{}/{}", output_dir, make_filename(&stem, suffix, "fits"));
        let header = derived_output_header(source_header_of(path).as_ref(), suffix, values);
        write_derived_fits(&fp, arr, Some(&header))?;
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

pub(crate) fn render_named_and_save(
    arr: &Array2<f32>,
    output_dir: &str,
    name: &str,
    write_fits: bool,
    header: Option<&HduHeader>,
) -> Result<(String, Option<String>)> {
    let png_path = format!("{}/{}.png", output_dir, name);
    save_auto_stf_preview_png(arr, &png_path)?;

    let fits_path = if write_fits {
        let fp = format!("{}/{}.fits", output_dir, name);
        write_derived_fits(&fp, arr, header)?;
        Some(fp)
    } else {
        None
    };

    Ok((png_path, fits_path))
}

pub(crate) fn resolve_output_dir(output_dir: &str) -> Result<String> {
    ensure_output_dir(output_dir, |path| std::fs::create_dir_all(path))
}

fn ensure_output_dir(output_dir: &str, create: impl FnOnce(&Path) -> std::io::Result<()>) -> Result<String> {
    let path = Path::new(output_dir);
    if !path.is_dir() {
        create(path).with_context(|| format!("Failed to create output directory: {}", output_dir))?;
    }
    Ok(output_dir.to_string())
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
        let (asdf_path, tmp) = resolve_single_image(&r.path)?;
        return Ok(
            crate::infra::asdf_bridge::try_extract_rgb_from_asdf(&asdf_path)?.map(|rgb| {
                ResolvedRgbImage {
                    r: rgb.r,
                    g: rgb.g,
                    b: rgb.b,
                    header: rgb.header,
                    _tmp: tmp,
                }
            }),
        );
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
pub(crate) mod test_support {
    use ndarray::Array2;

    use super::MAX_PREVIEW_DIM;
    use crate::core::imaging::stats::compute_image_stats;
    use crate::core::imaging::stf::{apply_stf, auto_stf, AutoStfConfig};
    use crate::infra::ipc::encode_with_header_downsampled;

    pub(crate) fn wide_textured_sky() -> Array2<f32> {
        Array2::from_shape_fn((6, 2 * MAX_PREVIEW_DIM), |(r, c)| {
            let noise = ((r * 7919 + c * 104_729) % 41) as f32 - 20.0;
            let texture = if (r + c) % 2 == 0 { 0.0 } else { 90.0 };
            let star = if r == 3 && c % 1021 == 17 { 4000.0 } else { 0.0 };
            1000.0 + noise + texture + star
        })
    }

    pub(crate) fn gpu_reduced(arr: &Array2<f32>) -> Array2<f32> {
        let encoded = encode_with_header_downsampled(arr, MAX_PREVIEW_DIM).unwrap();
        let width = u32::from_le_bytes(encoded[0..4].try_into().unwrap()) as usize;
        let height = u32::from_le_bytes(encoded[4..8].try_into().unwrap()) as usize;
        let values: Vec<f32> = encoded[16..]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        Array2::from_shape_vec((height, width), values).unwrap()
    }

    pub(crate) fn gpu_view_with_auto_stf(arr: &Array2<f32>) -> (Vec<u8>, u32, u32) {
        let reduced = gpu_reduced(arr);
        let stats = compute_image_stats(arr);
        let stf = auto_stf(&stats, &AutoStfConfig::default());
        let (h, w) = reduced.dim();
        (apply_stf(&reduced, &stf, &stats), w as u32, h as u32)
    }

    pub(crate) fn assert_png_matches(png_path: &str, expected: &(Vec<u8>, u32, u32)) {
        let png = image::open(png_path).unwrap().to_luma8();
        assert_eq!((png.width(), png.height()), (expected.1, expected.2));
        let off: Vec<(usize, u8, u8)> = png
            .as_raw()
            .iter()
            .zip(&expected.0)
            .enumerate()
            .filter(|(_, (a, b))| a.abs_diff(**b) > 1)
            .map(|(i, (a, b))| (i, *a, *b))
            .collect();
        assert!(
            off.is_empty(),
            "{} of {} preview pixels differ from the GPU view by more than one level, first {:?}",
            off.len(),
            png.as_raw().len(),
            off.iter().take(5).collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::{Duration, SystemTime};

    use super::test_support::{assert_png_matches, gpu_reduced, gpu_view_with_auto_stf, wide_textured_sky};
    use super::*;
    use crate::core::cube::lazy::test_support::write_line_cube;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef_with_dq_cards;
    use crate::infra::fits::writer::write_fits_mono;

    fn mef_with_dq(dir: &tempfile::TempDir, name: &str, center_bits: u32) -> String {
        let path = dir.path().join(name);
        let mut dq = vec![0i32; 16];
        dq[0] = 1 - 2147483647 - 1;
        dq[5] = center_bits as i32 - 2147483647 - 1;
        dq[10] = 2 - 2147483647 - 1;
        sci_err_dq_mef_with_dq_cards(&path, 4, 4, dq, vec![("TELESCOP", "'JWST'".into())]);
        path.to_str().unwrap().to_string()
    }

    fn mef(dir: &tempfile::TempDir, name: &str) -> String {
        mef_with_dq(dir, name, 3)
    }

    fn bump_mtime(path: &str) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(5)).unwrap();
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

    fn asdf(dir: &tempfile::TempDir, name: &str, tree_yaml: &str) -> String {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"#ASDF 1.0.0\n#ASDF_STANDARD 1.5.0\n%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n");
        bytes.extend_from_slice(tree_yaml.as_bytes());
        bytes.extend_from_slice(b"...\n");
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn header_with(cards: &[(&str, &str)]) -> HduHeader {
        let mut header = HduHeader::empty();
        for (k, v) in cards {
            header.set(k, v.to_string());
        }
        header
    }

    fn sky(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(r, c)| {
            let noise = ((r * 31 + c * 17) % 23) as f32 - 11.0;
            if r == rows / 2 && c == cols / 2 { 5000.0 } else { 100.0 + noise }
        })
    }

    #[test]
    fn an_asdf_colour_array_is_recovered_as_rgb_while_a_ramp_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let values: Vec<String> = (0..30).map(|v| v.to_string()).collect();
        let colour = asdf(
            &dir,
            "colour.asdf",
            &format!(
                "rgb: !core/ndarray-1.0.0\n  data: [{}]\n  shape: [5, 2, 3]\n  datatype: float32\n",
                values.join(", ")
            ),
        );

        let rgb = try_extract_rgb_resolved(&colour)
            .unwrap()
            .expect("an interleaved colour array must reach the RGB path");
        assert_eq!(rgb.r.dim(), (5, 2));
        assert_eq!(rgb.r[[0, 0]], 0.0);
        assert_eq!(rgb.g[[0, 0]], 1.0);
        assert_eq!(rgb.b[[0, 0]], 2.0);
        assert_eq!(rgb.header.get("EXTNAME"), Some("rgb"));

        assert!(
            try_extract_rgb_resolved(&format!("{}#array=rgb", colour)).unwrap().is_none(),
            "an explicit plane reference asks for one plane, not a composite"
        );

        let ramp = asdf(
            &dir,
            "ramp.asdf",
            "roman:\n  data: !core/ndarray-1.0.0\n    data: [[[1, 2], [3, 4]], [[5, 6], [7, 8]], [[9, 10], [11, 12]]]\n    datatype: float32\n",
        );
        assert!(
            try_extract_rgb_resolved(&ramp).unwrap().is_none(),
            "a planar ramp must fall through to the normal loader"
        );

        let plain = asdf(&dir, "plain.asdf", "data: !core/ndarray-1.0.0\n  data: [[1, 2], [3, 4]]\n  datatype: float32\n");
        assert!(try_extract_rgb_resolved(&plain).unwrap().is_none());
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
    fn two_sources_with_the_same_file_name_never_share_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        let out = out.to_str().unwrap().to_string();
        let night1 = dir.path().join("night1");
        let night2 = dir.path().join("night2");
        std::fs::create_dir_all(&night1).unwrap();
        std::fs::create_dir_all(&night2).unwrap();
        let a = night1.join("light.fits").to_str().unwrap().to_string();
        let b = night2.join("light.fits").to_str().unwrap().to_string();
        write_fits_mono(&a, &Array2::from_elem((4, 4), 1.0), None).unwrap();
        write_fits_mono(&b, &Array2::from_elem((4, 4), 2.0), None).unwrap();

        assert_eq!(output_stem(&a), "light");
        assert_eq!(output_stem(&b), "light_2");
        assert_eq!(output_stem(&format!("{}#hdu=0", b)), "light_2_hdu0");
        assert_eq!(output_stem(&a), "light", "the first source keeps its stem for the whole session");

        let ra = render_and_save(&load_cached(&a).unwrap().arr().to_owned(), &a, &out, "denoised", true).unwrap();
        let rb = render_and_save(&load_cached(&b).unwrap().arr().to_owned(), &b, &out, "denoised", true).unwrap();
        assert_ne!(ra.png_path, rb.png_path);
        assert_ne!(ra.fits_path, rb.fits_path);
        assert_eq!(load_cached(ra.fits_path.as_deref().unwrap()).unwrap().arr()[[0, 0]], 1.0);
        assert_eq!(load_cached(rb.fits_path.as_deref().unwrap()).unwrap().arr()[[0, 0]], 2.0);
    }

    #[test]
    fn a_file_rewritten_on_disk_is_reloaded_by_the_next_load() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("rewritten.fits").to_str().unwrap().to_string();
        write_fits_mono(&p, &Array2::zeros((4, 4)), None).unwrap();
        assert_eq!(load_cached(&p).unwrap().arr()[[0, 0]], 0.0);

        write_fits_mono(&p, &Array2::from_elem((4, 4), 1.0), None).unwrap();
        bump_mtime(&p);
        assert_eq!(load_cached(&p).unwrap().arr()[[0, 0]], 1.0, "stale cache entry served after a rewrite");
        assert_eq!(load_cached_full(&p).unwrap().arr()[[0, 0]], 1.0);
    }

    #[test]
    fn re_running_a_step_on_the_same_output_path_serves_the_new_result() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.fits").to_str().unwrap().to_string();
        let out = dir.path().to_str().unwrap().to_string();
        write_fits_mono(&src, &Array2::zeros((4, 4)), None).unwrap();

        let first = render_and_save(&Array2::from_elem((4, 4), 1.0), &src, &out, "denoised", true).unwrap();
        let fits = first.fits_path.expect("fits written");
        assert_eq!(load_cached(&fits).unwrap().arr()[[0, 0]], 1.0);

        let second = render_and_save(&Array2::from_elem((4, 4), 2.0), &src, &out, "denoised", true).unwrap();
        assert_eq!(second.fits_path.as_deref(), Some(fits.as_str()));
        assert_eq!(load_cached(&fits).unwrap().arr()[[0, 0]], 2.0, "downstream step read the previous run");
        assert_eq!(load_from_cache_or_disk(&fits).unwrap().arr()[[0, 0]], 2.0);
    }

    #[test]
    fn a_rewritten_mef_refreshes_its_cached_companion_planes() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_with_dq(&dir, "companions.fits", 3);
        let key = format!("{}#hdu=1", p);
        let (dq, _) = load_companions(&key).unwrap().dq.expect("dq companion");
        assert_eq!(dq.int_plane().unwrap().bits[[1, 1]], 3);

        mef_with_dq(&dir, "companions.fits", 1);
        bump_mtime(&p);
        load_preview_validated(&key).unwrap();
        let (dq, _) = load_companions(&key).unwrap().dq.expect("dq companion");
        assert_eq!(dq.int_plane().unwrap().bits[[1, 1]], 1, "the new SCI was masked with the previous DQ");
        let direct = load_cached_full(&format!("{}#hdu=3", p)).unwrap();
        assert_eq!(direct.int_plane().unwrap().bits[[1, 1]], 1);
    }

    #[test]
    fn a_deleted_file_is_not_served_from_memory() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("deleted.fits").to_str().unwrap().to_string();
        write_fits_mono(&p, &Array2::zeros((4, 4)), None).unwrap();
        assert!(load_cached(&p).is_ok());
        std::fs::remove_file(&p).unwrap();
        assert!(load_cached(&p).is_err(), "an output removed from disk was still served from the cache");
        assert!(!GLOBAL_IMAGE_CACHE.contains(&p));
    }

    #[test]
    fn a_load_overtaken_by_a_rewrite_does_not_pin_the_old_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("overtaken.fits").to_str().unwrap().to_string();
        write_fits_mono(&p, &Array2::from_elem((4, 4), 1.0), None).unwrap();

        let (served, _) = load_cached_with(&p, |key| {
            let stale = plane_load(&image_ref(key))?;
            write_derived_fits(key, &Array2::from_elem((40, 40), 2.0), None)?;
            GLOBAL_IMAGE_CACHE.get_or_load_plane(key, || Ok(stale))
        })
        .unwrap();
        assert_eq!(served.arr().dim(), (4, 4));

        let next = load_cached(&p).unwrap();
        assert_eq!(next.arr().dim(), (40, 40), "the read that lost the race stayed cached under the new stamp");
        assert_eq!(next.arr()[[0, 0]], 2.0);
    }

    #[test]
    fn the_header_of_a_file_rewritten_on_disk_is_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("reheadered.fits").to_str().unwrap().to_string();
        write_fits_mono(&p, &Array2::zeros((4, 4)), Some(&header_with(&[("EXPTIME", "300")]))).unwrap();
        assert_eq!(load_cached_full(&p).unwrap().header().and_then(|h| h.get_f64("EXPTIME")), Some(300.0));

        write_fits_mono(&p, &Array2::zeros((4, 4)), Some(&header_with(&[("EXPTIME", "600")]))).unwrap();
        bump_mtime(&p);
        assert_eq!(
            cached_header(&p).unwrap().get_f64("EXPTIME"),
            Some(600.0),
            "the cached header of the previous file was served"
        );
    }

    #[test]
    fn synthetic_keys_are_not_revalidated_against_the_disk() {
        let key = "__composite_revalidation_probe";
        GLOBAL_IMAGE_CACHE.insert_synthetic(
            key,
            std::sync::Arc::new(Array2::<f32>::from_elem((2, 2), 4.0)),
            compute_image_stats(&Array2::<f32>::from_elem((2, 2), 4.0)),
        );
        assert_eq!(load_cached(key).unwrap().arr()[[0, 0]], 4.0);
        GLOBAL_IMAGE_CACHE.remove(key);
    }

    #[test]
    fn render_and_save_keeps_the_source_wcs_and_drops_cards_the_values_no_longer_match() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("solved.fits").to_str().unwrap().to_string();
        let out = dir.path().to_str().unwrap().to_string();
        let source_header = header_with(&[
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
            ("CRVAL1", "83.8"),
            ("CRVAL2", "-5.4"),
            ("CRPIX1", "2.0"),
            ("CRPIX2", "2.0"),
            ("CD1_1", "-0.0001"),
            ("CD2_2", "0.0001"),
            ("EXPTIME", "300"),
            ("BUNIT", "MJy/sr"),
            ("MAGZERO", "30.0"),
            ("DATAMAX", "60000"),
        ]);
        write_fits_mono(&src, &sky(8, 8), Some(&source_header)).unwrap();

        let ro = render_and_save(&sky(8, 8), &src, &out, "arcsinh", true).unwrap();
        let written = load_cached_full(ro.fits_path.as_deref().unwrap()).unwrap();
        let header = written.header().expect("header");
        assert_eq!(header.get("CRVAL1").map(str::trim), Some("83.8"), "WCS dropped from the derived FITS");
        assert_eq!(header.get("CTYPE1").map(str::trim), Some("RA---TAN"));
        assert_eq!(header.get("EXPTIME").map(str::trim), Some("300"));
        assert_eq!(header.get(HEADER_ABPROC).map(str::trim), Some("arcsinh"));
        for dropped in ["BUNIT", "MAGZERO", "DATAMAX"] {
            assert!(header.get(dropped).is_none(), "{dropped} survived a rescaling step");
        }
    }

    #[test]
    fn derived_output_header_removes_structure_and_keeps_calibration_only_for_linear_values() {
        let source = header_with(&[
            ("XTENSION", "IMAGE"),
            ("EXTNAME", "SCI"),
            ("PCOUNT", "0"),
            ("ZIMAGE", "T"),
            ("ZTILE1", "64"),
            ("ZCMPTYPE", "RICE_1"),
            ("TFIELDS", "1"),
            ("TTYPE1", "COMPRESSED_DATA"),
            ("TFORM1", "1PB"),
            ("TBCOL1", "1"),
            ("THEAP", "0"),
            ("CHECKSUM", "abc"),
            ("DATASUM", "1"),
            ("DATAMIN", "0"),
            ("DATAMAX", "1"),
            ("NAXIS3", "5"),
            ("WCSAXES", "3"),
            ("CTYPE3", "WAVE"),
            ("CD3_3", "0.1"),
            ("PC1_3", "0"),
            ("PV3_1", "0"),
            ("CRVAL1", "10.5"),
            ("CD1_1", "-0.0001"),
            ("PV2_1", "45.0"),
            ("TELESCOP", "JWST"),
            ("BUNIT", "MJy/sr"),
            ("PHOTMJSR", "1.5"),
            ("PIXAR_SR", "2.1E-13"),
            ("ZPT", "25.0"),
            ("ZEROPT", "25.0"),
            ("PHOTZP", "25.0"),
            ("MAGZERO", "30.0"),
            ("SATURATE", "60000"),
        ]);

        let linear = derived_output_header(Some(&source), "deconv", OutputValues::Linear);
        for dropped in [
            "XTENSION", "EXTNAME", "PCOUNT", "ZIMAGE", "ZTILE1", "ZCMPTYPE", "TFIELDS", "TTYPE1", "TFORM1",
            "TBCOL1", "THEAP", "CHECKSUM", "DATASUM", "DATAMIN", "DATAMAX", "NAXIS3", "CTYPE3", "CD3_3", "PC1_3", "PV3_1",
        ] {
            assert!(linear.get(dropped).is_none(), "{dropped} survived");
            assert!(!linear.cards.iter().any(|(k, _)| k == dropped), "{dropped} card survived");
        }
        for kept in ["CRVAL1", "CD1_1", "PV2_1", "TELESCOP", "BUNIT", "PHOTMJSR", "PIXAR_SR", "ZPT", "SATURATE"] {
            assert!(linear.get(kept).is_some(), "{kept} dropped from a flux-preserving output");
        }
        assert_eq!(linear.get("WCSAXES"), Some("2"));
        assert_eq!(linear.get(HEADER_ABPROC), Some("deconv"));
        assert!(linear.get(HEADER_DISPLAY_REFERRED).is_none());

        let rescaled = derived_output_header(Some(&source), "pixelmath", OutputValues::Rescaled);
        for dropped in ["BUNIT", "PHOTMJSR", "PIXAR_SR", "ZPT", "ZEROPT", "PHOTZP", "MAGZERO", "SATURATE"] {
            assert!(rescaled.get(dropped).is_none(), "{dropped} survived a rescaling");
        }
        assert_eq!(rescaled.get("CRVAL1"), Some("10.5"));
        assert_eq!(rescaled.get("TELESCOP"), Some("JWST"));

        let stretched = derived_output_header(Some(&source), "arcsinh", OutputValues::DisplayReferred);
        assert_eq!(stretched.get(HEADER_DISPLAY_REFERRED), Some("T"));
        assert!(stretched.get("BUNIT").is_none());
        let denoised = derived_output_header(Some(&stretched), "denoised", OutputValues::Linear);
        assert_eq!(denoised.get(HEADER_DISPLAY_REFERRED), Some("T"), "a linear step keeps display-referred values");
        let remapped = derived_output_header(Some(&stretched), "pixelmath", OutputValues::Rescaled);
        assert!(remapped.get(HEADER_DISPLAY_REFERRED).is_none());

        let bare = derived_output_header(None, "  ", OutputValues::Rescaled);
        assert_eq!(bare.get(HEADER_ABPROC), Some("processed"));
    }

    #[test]
    fn a_display_referred_render_writes_the_values_linearly_and_flags_the_fits() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("linear_src.fits").to_str().unwrap().to_string();
        let out = dir.path().to_str().unwrap().to_string();
        write_fits_mono(&src, &Array2::from_elem((2, 2), 7.0), None).unwrap();
        let stretched = Array2::from_shape_vec((2, 2), vec![0.0, 0.5, 1.0, f32::NAN]).unwrap();

        let ro = render_and_save_as(&stretched, &src, &out, "arcsinh", true, OutputValues::DisplayReferred).unwrap();
        let png = image::open(&ro.png_path).unwrap().to_luma8();
        assert_eq!(png.as_raw(), &vec![0u8, 128, 255, 0]);
        let written = load_cached_full(ro.fits_path.as_deref().unwrap()).unwrap();
        assert_eq!(written.header().and_then(|h| h.get(HEADER_DISPLAY_REFERRED)).map(str::trim), Some("T"));
    }

    #[test]
    fn preview_reduction_averages_each_cell_and_keeps_isolated_peaks() {
        let checker = Array2::from_shape_fn((8, 8), |(r, c)| if (r + c) % 2 == 0 { 10.0f32 } else { 210.0 });
        let out = reduce_for_preview(&checker, 4).expect("larger than the cap");
        assert_eq!(out.dim(), (4, 4));
        let first = out[[0, 0]];
        assert!(out.iter().all(|&v| v == first), "cells of the same texture differ: {out:?}");
        assert!((110.0..=150.0).contains(&first), "noise texture decimated instead of averaged: {first}");

        let mut single = Array2::<f32>::from_elem((8, 8), 1.0);
        single[[1, 1]] = 255.0;
        let out = reduce_for_preview(&single, 4).expect("larger than the cap");
        assert!(out[[0, 0]] > 128.0, "a one-pixel star on odd parity vanished: {}", out[[0, 0]]);
        assert!(out.iter().skip(1).all(|&v| v == 1.0));
        assert!(reduce_for_preview(&single, 8).is_none());
    }

    #[test]
    fn named_renders_use_the_same_auto_stf_transfer_as_the_ingest_preview() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let data = sky(16, 16);
        let (png, fits) = render_named_and_save(&data, &out, "stacked", true, None).unwrap();
        let decoded = image::open(&png).unwrap().to_luma8();
        assert_eq!(decoded.as_raw(), &auto_stretch_preview(&data), "stack preview uses a different display transfer");
        let fits = fits.expect("fits written");
        assert_eq!(load_cached(&fits).unwrap().arr()[[8, 8]], 5000.0);

        let header = header_with(&[("CRVAL1", "12.0")]);
        let (_, fits) = render_named_and_save(&data, &out, "calibrated", true, Some(&header)).unwrap();
        let written = load_cached_full(fits.as_deref().unwrap()).unwrap();
        assert_eq!(written.header().and_then(|h| h.get("CRVAL1")).map(str::trim), Some("12.0"));
    }

    #[test]
    fn previews_of_large_outputs_reduce_the_values_before_the_stretch_like_the_gpu_view() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("wide_src.fits").to_str().unwrap().to_string();
        let out = dir.path().to_str().unwrap().to_string();
        let data = wide_textured_sky();
        write_fits_mono(&src, &data, None).unwrap();
        let expected = gpu_view_with_auto_stf(&data);

        let ro = render_and_save(&data, &src, &out, "denoised", false).unwrap();
        assert_png_matches(&ro.png_path, &expected);
        let (png, _) = render_named_and_save(&data, &out, "stacked_wide", false, None).unwrap();
        assert_png_matches(&png, &expected);
    }

    #[test]
    fn preview_reduction_matches_the_gpu_kernel_on_padded_data() {
        let mut data = wide_textured_sky();
        for r in 0..6 {
            data[[r, 0]] = 0.0;
            data[[r, 3]] = f32::NAN;
            for c in 8..12 {
                data[[r, c]] = 0.0;
            }
            for c in 20..24 {
                data[[r, c]] = f32::NAN;
            }
        }
        let reduced = reduce_for_preview(&data, MAX_PREVIEW_DIM).expect("wider than the preview cap");
        let gpu = gpu_reduced(&data);
        assert_eq!(reduced.dim(), gpu.dim());
        for ((i, &cpu), &gpu) in reduced.indexed_iter().zip(gpu.iter()) {
            let same = (cpu.is_nan() && gpu.is_nan()) || (cpu - gpu).abs() <= 1e-4 * gpu.abs().max(1.0);
            assert!(same, "cell {i:?}: preview {cpu} vs gpu {gpu}");
        }
        assert!(reduced[[0, 4]].is_nan(), "an all-zero cell is padding in both paths: {}", reduced[[0, 4]]);
        assert!(reduced[[0, 10]].is_nan());
        assert!(reduce_for_preview(&Array2::from_elem((4, 4), 1.0f32), MAX_PREVIEW_DIM).is_none());
    }

    #[test]
    fn preview_reduction_skips_zero_padding_like_nan_at_mosaic_edges() {
        let mosaic = |border: f32| {
            Array2::from_shape_fn((8, 8), |(y, x)| {
                if x < 3 {
                    border
                } else if (y, x) == (5, 5) {
                    5000.0
                } else {
                    1000.0 + ((y * 8 + x) % 5) as f32
                }
            })
        };
        let zero = reduce_for_preview(&mosaic(0.0), 4).expect("larger than the cap");
        let nan = reduce_for_preview(&mosaic(f32::NAN), 4).expect("larger than the cap");
        for ((i, z), n) in zero.indexed_iter().zip(nan.iter()) {
            let same = z.to_bits() == n.to_bits() || (z.is_nan() && n.is_nan());
            assert!(same, "cell {i:?}: zero padding {z} vs NaN padding {n}");
        }
        for row in 0..4 {
            assert!(zero[[row, 0]].is_nan(), "an all-padding cell is padding, got {}", zero[[row, 0]]);
            let edge = zero[[row, 1]];
            assert!(edge >= 1000.0, "the mosaic edge cell was darkened by its padding: {edge}");
        }
        assert!(zero[[2, 2]] > zero[[2, 3]], "the star cell lost its peak");
    }

    #[test]
    fn a_derived_fits_can_replace_a_cube_that_is_open_in_the_cube_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cube_target.fits");
        write_line_cube(&path, 0.0);
        let p = path.to_str().unwrap().to_string();
        assert!(GLOBAL_CUBE_CACHE.get_or_open(&p).is_ok());

        write_derived_fits(&p, &Array2::from_elem((4, 4), 3.0), None)
            .expect("the cube cache kept the file mapped and blocked the write");
        assert_eq!(load_cached(&p).unwrap().arr()[[0, 0]], 3.0);
    }

    #[test]
    fn an_output_directory_that_cannot_be_created_is_an_error_not_a_different_directory() {
        let denied = ensure_output_dir("Z:/astroburst-denied/output", |_| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        });
        let message = format!("{:#}", denied.unwrap_err());
        assert!(message.contains("Z:/astroburst-denied/output"), "{message}");

        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").to_str().unwrap().to_string();
        assert_eq!(resolve_output_dir(&nested).unwrap(), nested);
        assert!(Path::new(&nested).is_dir());
        let file = dir.path().join("plain_file").to_str().unwrap().to_string();
        std::fs::write(&file, b"x").unwrap();
        assert!(resolve_output_dir(&file).is_err(), "an existing file was accepted as the output directory");
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
        assert!(dq_exclusion(&p).unwrap().is_some());

        let entry = load_cached(&p).unwrap();
        let j = plane_info_json(&p, &entry).unwrap();
        assert_eq!(j[RES_DQ_REF], format!("{}#hdu=3", p));
        assert_eq!(j[RES_INDEX], 1);
        let err_plane = load_companions(&format!("{}#hdu=2", p)).unwrap();
        assert!(err_plane.dq.is_some());
        let lonely = load_companions(&format!("{}#hdu=4", p)).unwrap();
        assert!(lonely.err.is_none());
        assert!(lonely.dq.is_none());

        let private_cache = crate::infra::cache::ImageCache::new(8, 64 * 1024 * 1024);
        let active = private_cache.get_or_load_plane(&p, || plane_load(&image_ref(&p))).unwrap();
        let primed = load_companions_into(&private_cache, &active, plane_load);
        assert!(primed.dq.is_some());
        assert!(private_cache.contains(&format!("{}#hdu=3", p)));

        std::fs::remove_file(&p).unwrap();
        assert!(!std::path::Path::new(&p).exists());

        let second = load_companions_into(&private_cache, &active, plane_load);
        let (dq, table) = second.dq.expect("dq companion served from the cache");
        assert_eq!(table, DqTable::Jwst);
        assert_eq!(dq.int_plane().unwrap().bits[[1, 1]], 3);
        assert_eq!(second.err.expect("err companion served from the cache").arr()[[0, 1]], 0.5);
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
