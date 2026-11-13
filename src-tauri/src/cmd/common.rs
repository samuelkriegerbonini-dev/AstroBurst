use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use ndarray::Array2;
use rayon::prelude::*;
use serde_json::json;

use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::imaging::asinh::linearised_warning;
use crate::core::imaging::dq_flags::DqTable;
use crate::core::imaging::layer_roles::{LayerGuess, LayerRole};
use crate::core::imaging::sampling::{cell_range, preview_dims};
use crate::core::imaging::stats::{compute_image_stats, is_valid_pixel};
use crate::core::imaging::stf::{auto_stf, apply_stf, AutoStfConfig, ImageStats, StfParams};
use crate::infra::cache::{GLOBAL_IMAGE_CACHE, ImageEntry};
use crate::infra::fits::band_planes::{band_plane_of_path, loaded_band_plane};
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::gzip::SharedTempDir;
use crate::infra::image_source::{
    dq_source_text, err_source_text, is_stored_sigma, load_plane, mask_unreadable_by_header, resolve_plane_info,
    sigma_key, CompanionChoice, CompanionOrigin, ErrOrigin, ErrRecipe, CARD_NO_DATA,
};
use crate::infra::render::grayscale::save_stf_png;
use crate::service::common::derived_output_header;
use crate::types::constants::{
    PLANE_KIND_ARRAY, PLANE_KIND_HDU, RES_DQ_REF, RES_DQ_TABLE, RES_ERR_REF, RES_EXTNAME, RES_EXTVER, RES_INDEX,
    RES_IS_DQ, RES_IS_ERR, RES_KEY, RES_KIND, RES_SOURCE_PATH,
};
use crate::types::header::HduHeader;
use crate::types::image_ref::{ImageRef, PlaneSelector};

pub(crate) use crate::infra::image_source::LoadedCompanions;
pub(crate) use crate::service::image_files::{
    invalidate_written, load_cached, load_cached_full, load_companions, output_stem, source_path, source_primary_header,
    write_derived_mef,
};
pub(crate) use crate::service::common::{HEADER_ABPROC, HEADER_DISPLAY_REFERRED};

pub(crate) const MAX_PREVIEW_DIM: usize = 4096;

const PREVIEW_PEAK_SIGNIFICANCE: f32 = 3.0;

pub(crate) use crate::core::ops::OutputValues;

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

pub(crate) fn reduce_for_preview(arr: &Array2<f32>, max_dim: usize) -> Option<Array2<f32>> {
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

pub(crate) fn save_output_preview(arr: &Array2<f32>, values: OutputValues, png_path: &str) -> Result<()> {
    match values {
        OutputValues::DisplayReferred => save_reduced_preview(arr, png_path, display_referred_preview),
        OutputValues::Linear | OutputValues::Rescaled => save_auto_stf_preview_png(arr, png_path),
    }
}

pub(crate) struct ResolvedImage {
    pub arr: Array2<f32>,
    pub header: HduHeader,
    pub _tmp: Option<SharedTempDir>,
}

pub(crate) fn image_ref(path: &str) -> ImageRef {
    ImageRef::parse(path)
}

pub(crate) fn extract_image_resolved(path: &str) -> Result<ResolvedImage> {
    let loaded = load_plane(&image_ref(path))?;
    Ok(ResolvedImage {
        arr: loaded.arr,
        header: loaded.header,
        _tmp: loaded._tmp,
    })
}

pub(crate) fn load_from_cache_or_disk(path: &str) -> Result<ImageEntry> {
    load_cached(path)
}

pub(crate) fn load_preview_validated(path: &str) -> Result<ImageEntry> {
    load_cached(path)
}

#[cfg(test)]
pub(crate) fn dq_exclusion(path: &str) -> Result<Option<Array2<u8>>> {
    let comps = load_companions(path)?;
    Ok(comps.dq.and_then(|(entry, table)| {
        entry
            .int_plane()
            .map(|plane| crate::core::imaging::dq_flags::exclusion_map(&plane.bits, table.exclusion_mask()))
    }))
}

#[derive(serde::Serialize)]
pub(crate) struct ErrOriginJson {
    role: LayerRole,
    source: String,
    assumed: bool,
    n_no_data: Option<u64>,
    origin: CompanionOrigin,
}

impl ErrOriginJson {
    pub(crate) fn loaded(origin: &ErrOrigin, choice: &CompanionChoice) -> ErrOriginJson {
        ErrOriginJson {
            role: origin.role,
            source: origin.source.clone(),
            assumed: origin.assumed,
            n_no_data: Some(origin.n_no_data),
            origin: choice.origin,
        }
    }

    fn described(companion: &ImageRef, choice: &CompanionChoice) -> ErrOriginJson {
        ErrOriginJson {
            role: choice.role,
            source: err_source_text(choice, companion),
            assumed: choice.assumed,
            n_no_data: known_no_data(companion, choice),
            origin: choice.origin,
        }
    }
}

#[derive(serde::Serialize)]
struct PlaneLayers {
    layer: Option<LayerGuess>,
    err_origin: Option<ErrOriginJson>,
    dq_origin: Option<String>,
    dq_origin_kind: Option<CompanionOrigin>,
    err_unavailable: Option<String>,
    dq_unavailable: Option<String>,
    pairing_notes: Vec<String>,
    err_recipe: Option<ErrRecipe>,
    warnings: Vec<&'static str>,
    band_plane: Option<BandPlaneJson>,
}

#[derive(serde::Serialize)]
struct BandPlaneJson {
    hdu: usize,
    plane: usize,
    label: Option<String>,
}

fn known_no_data(companion: &ImageRef, choice: &CompanionChoice) -> Option<u64> {
    if is_stored_sigma(choice) {
        return Some(0);
    }
    let derived = GLOBAL_IMAGE_CACHE.get(&sigma_key(companion, choice.role))?;
    derived.header()?.get_i64(CARD_NO_DATA).and_then(|n| u64::try_from(n).ok())
}

pub(crate) fn plane_info_json(path: &str, entry: &ImageEntry) -> Result<serde_json::Value> {
    let r = image_ref(path);
    let info = match entry.plane_info() {
        Some(info) => info.clone(),
        None => resolve_plane_info(&r)?,
    };
    let comps = entry.companions().cloned().unwrap_or_default();
    let (kind, plane_index, key) = match &info.kind {
        PlaneSelector::Hdu(n) => (PLANE_KIND_HDU, Some(*n), None),
        PlaneSelector::Array(k) => (PLANE_KIND_ARRAY, None, Some(k.clone())),
        PlaneSelector::Auto => (PLANE_KIND_HDU, None, None),
    };
    let band = band_plane_of_path(&r.path);
    let index = band.as_ref().map_or(plane_index, |band| Some(band.hdu));
    let pairing = &info.pairing;
    let dq_refusal = comps.dq.as_ref().and_then(|dq| mask_unreadable_by_header(dq, pairing.dq.as_ref()));
    let usable_dq = comps.dq.as_ref().filter(|_| dq_refusal.is_none());
    let dq_choice = usable_dq.zip(pairing.dq.as_ref());
    let dq_ref = usable_dq.map(ImageRef::cache_key);
    let err_ref = comps.err.as_ref().map(ImageRef::cache_key);
    let dq_table = match (&dq_ref, entry.header()) {
        (Some(dq_key), Some(header)) => Some(
            DqTable::select(header)
                .or_else(|| cached_header(dq_key).map_or(DqTable::Unknown, |h| DqTable::select(&h))),
        ),
        _ => None,
    };
    let layers = PlaneLayers {
        layer: pairing.layer.clone(),
        err_origin: comps.err.as_ref().zip(pairing.err.as_ref()).map(|(err, choice)| ErrOriginJson::described(err, choice)),
        dq_origin: dq_choice.map(|(dq, choice)| dq_source_text(choice, dq)),
        dq_origin_kind: dq_choice.map(|(_, choice)| choice.origin),
        err_unavailable: pairing.err_unavailable.clone(),
        dq_unavailable: dq_refusal.or_else(|| pairing.dq_unavailable.clone()),
        pairing_notes: pairing.notes.clone(),
        err_recipe: pairing.err_recipe.clone(),
        warnings: entry.header().and_then(linearised_warning).into_iter().collect(),
        band_plane: band.as_ref().map(|band| BandPlaneJson {
            hdu: band.hdu,
            plane: band.plane,
            label: entry.header().and_then(loaded_band_plane).and_then(|loaded| loaded.label),
        }),
    };
    let source_path = band.map_or(r.path, |band| band.file);
    let mut body = json!({
        RES_KIND: kind,
        RES_INDEX: index,
        RES_KEY: key,
        RES_EXTNAME: info.extname,
        RES_EXTVER: info.extver,
        RES_IS_DQ: info.is_dq,
        RES_IS_ERR: info.is_err,
        RES_DQ_REF: dq_ref,
        RES_ERR_REF: err_ref,
        RES_SOURCE_PATH: source_path,
        RES_DQ_TABLE: dq_table,
    });
    if let (Some(fields), serde_json::Value::Object(added)) = (body.as_object_mut(), serde_json::to_value(&layers)?) {
        fields.extend(added);
    }
    Ok(body)
}

pub(crate) fn write_derived_fits(path: &str, arr: &Array2<f32>, header: Option<&HduHeader>) -> Result<()> {
    GLOBAL_CUBE_CACHE.invalidate(path);
    let written = crate::infra::fits::writer::write_fits_mono(path, arr, header);
    invalidate_written(path);
    written
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
    crate::service::common::cached_header(&GLOBAL_IMAGE_CACHE, path)
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
    pub _tmp: Option<SharedTempDir>,
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

pub(crate) async fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> std::result::Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("Task join failed: {error}"))?
        .map_err(|error| format!("{error:#}"))
}

macro_rules! blocking_cmd {
    ($body:expr) => {
        $crate::cmd::common::off_thread(move || -> anyhow::Result<serde_json::Value> { $body }).await
    };
}

pub(crate) use blocking_cmd;

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;

    use ndarray::Array2;

    use super::MAX_PREVIEW_DIM;
    use crate::core::imaging::stats::compute_image_stats;
    use crate::core::imaging::stf::{apply_stf, auto_stf, AutoStfConfig};
    use crate::infra::fits::reader::test_fixtures::{ramp_f32, write_test_mef, HduData, TestHdu};
    use crate::infra::ipc::encode_with_header_downsampled;

    pub(crate) fn afw_mef(dir: &Path, name: &str, more: Vec<TestHdu>) -> String {
        let mut mask = vec![0i32; 16];
        mask[5] = 34;
        let mut variance: Vec<f32> = (0..16).map(|i| (i * i) as f32).collect();
        variance[6] = -1.0;
        let afw_cards = vec![("MP_BAD", "0".to_string()), ("MP_SAT", "1".to_string()), ("HIERARCH MP_DETECTED", "5".to_string())];
        let mut hdus = vec![
            TestHdu { extname: Some("IMAGE"), extver: None, cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![("BUNIT", "'nJy     '".into())] },
            TestHdu { extname: Some("MASK"), extver: None, cols: 4, rows: 4, data: HduData::I32(mask), extra_cards: afw_cards },
            TestHdu { extname: Some("VARIANCE"), extver: None, cols: 4, rows: 4, data: HduData::F32(variance), extra_cards: vec![("BUNIT", "'nJy^2   '".into())] },
        ];
        hdus.extend(more);
        let path = dir.join(name);
        write_test_mef(&path, &[("TELESCOP", "'Subaru'".into())], &hdus);
        path.to_str().unwrap().to_string()
    }

    pub(crate) fn float_dq_mef(dir: &Path, name: &str) -> String {
        let path = dir.join(name);
        write_test_mef(
            &path,
            &[],
            &[
                TestHdu { extname: Some("SCI"), extver: Some(1), cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![] },
                TestHdu { extname: Some("DQ"), extver: Some(1), cols: 4, rows: 4, data: HduData::F32(vec![0.0; 16]), extra_cards: vec![] },
            ],
        );
        path.to_str().unwrap().to_string()
    }

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

    use super::test_support::{
        afw_mef, assert_png_matches, float_dq_mef, gpu_reduced, gpu_view_with_auto_stf, wide_textured_sky,
    };
    use super::*;
    use crate::core::cube::lazy::test_support::write_line_cube;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef_with_dq_cards;
    use crate::infra::fits::writer::write_fits_mono;
    use crate::infra::image_source::load_companions_into;
    use crate::service::common::{load_cached_with, plane_load};

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
    fn wizard_cache_keys_get_readable_output_stems() {
        use crate::types::constants::{wizard_aligned_key, wizard_bg_key, wizard_bg_key_for, wizard_cropped_key_for};
        let first_run = wizard_bg_key_for(Some("mgk3x9p2a1"), "ha");
        assert_eq!(output_stem(&first_run), "wizard_ha_bg");
        assert_eq!(output_stem(&wizard_bg_key_for(Some("mgk3x9q7zz"), "ha")), "wizard_ha_bg_2");
        assert_eq!(output_stem(&first_run), "wizard_ha_bg", "the same cache key keeps its stem");
        assert_eq!(output_stem(&wizard_aligned_key("oiii")), "wizard_oiii_aligned");
        assert_eq!(output_stem(&wizard_bg_key("test")), "wizard_test_bg", "a tokenless bin starting with t is not a run token");
        assert_eq!(output_stem(&wizard_cropped_key_for(Some("k1"), "my_filter")), "wizard_my_filter_cropped");
        assert_eq!(output_stem("C:/d/wizard_sii_bg.fits"), "wizard_sii_bg");
        assert_eq!(output_stem(&wizard_bg_key("sii")), "wizard_sii_bg_2", "a file already named like the readable stem keeps its outputs");
        assert_eq!(output_stem("__composite_r"), "__composite_r");
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

        let (served, _) = load_cached_with(&GLOBAL_IMAGE_CACHE, &p, |key| {
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
        assert_eq!(j[RES_DQ_TABLE], "jwst");
        assert_eq!(
            j["err_origin"],
            json!({"role": "sigma", "source": "EXTNAME ERR, HDU 2: as is", "assumed": false, "n_no_data": 0, "origin": "same_file"})
        );

        let lonely = format!("{}#hdu=4", p);
        let entry = load_cached_full(&lonely).unwrap();
        let j = plane_info_json(&lonely, &entry).unwrap();
        assert!(j[RES_DQ_REF].is_null());
        assert!(j[RES_DQ_TABLE].is_null());
        assert_eq!(j[RES_INDEX], 4);
        assert_eq!(j["layer"], json!({"role": "science", "basis": "EXTNAME SCI", "assumed": false}));
        for key in ["err_origin", "dq_origin", "dq_origin_kind", "err_unavailable", "dq_unavailable"] {
            assert!(j.as_object().unwrap().contains_key(key), "{key}: {j}");
            assert!(j[key].is_null(), "{key}: {j}");
        }
        assert_eq!(j["pairing_notes"], json!([]));
        assert_eq!(j["warnings"], json!([]));
        assert!(j.as_object().unwrap().contains_key("err_recipe") && j["err_recipe"].is_null(), "{j}");

        let synthetic_key = "__composite_info";
        GLOBAL_IMAGE_CACHE.insert_synthetic(
            synthetic_key,
            std::sync::Arc::new(Array2::<f32>::zeros((2, 2))),
            compute_image_stats(&Array2::<f32>::zeros((2, 2))),
        );
        let synthetic = GLOBAL_IMAGE_CACHE.get(synthetic_key).unwrap();
        assert!(plane_info_json(synthetic_key, &synthetic).is_err());
    }

    #[test]
    fn an_active_mask_hdu_of_unknown_origin_keeps_raw_bits_and_excludes_nothing() {
        use crate::infra::fits::reader::test_fixtures::{ramp_f32, write_test_mef, HduData, TestHdu};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hsc_mask.fits");
        let mut mask = vec![0i32; 16];
        mask[5] = 1;
        write_test_mef(
            &path,
            &[("TELESCOP", "'Subaru'".into())],
            &[
                TestHdu { extname: Some("SCI"), extver: None, cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![] },
                TestHdu { extname: Some("MASK"), extver: None, cols: 4, rows: 4, data: HduData::I32(mask), extra_cards: vec![] },
            ],
        );
        let p = path.to_str().unwrap().to_string();
        let k = format!("{p}#hdu=2");

        let (dq, table) = load_companions(&k).unwrap().dq.expect("the active MASK HDU is its own DQ plane");
        assert_eq!(table, DqTable::Unknown);
        assert_eq!(dq.int_plane().unwrap().bits[[1, 1]], 1);
        let excluded = dq_exclusion(&k).unwrap().expect("exclusion map of the MASK plane");
        assert_eq!(excluded.dim(), (4, 4));
        assert!(excluded.iter().all(|&v| v == 0), "{excluded:?}");

        let entry = load_cached_full(&k).unwrap();
        let info = plane_info_json(&k, &entry).unwrap();
        assert_eq!(info[RES_IS_DQ], true);
        assert_eq!(info[RES_DQ_REF], k);
        assert_eq!(info[RES_DQ_TABLE], "unknown");

        let of_the_image = dq_exclusion(&format!("{p}#hdu=1")).unwrap().expect("the MASK HDU is the DQ plane of the SCI HDU");
        assert_eq!(of_the_image.dim(), (4, 4));
        assert!(of_the_image.iter().all(|&v| v == 0), "{of_the_image:?}");
    }

    #[test]
    fn a_dq_plane_without_telescope_cards_takes_the_table_of_its_image_plane() {
        use crate::infra::fits::reader::test_fixtures::{ramp_f32, write_test_mef, HduData, TestHdu};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jwst_sci_cards.fits");
        let mut dq = vec![0i32; 16];
        dq[5] = 1;
        dq[6] = 2;
        write_test_mef(
            &path,
            &[],
            &[
                TestHdu { extname: Some("SCI"), extver: Some(1), cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![("TELESCOP", "'JWST'".into())] },
                TestHdu { extname: Some("DQ"), extver: Some(1), cols: 4, rows: 4, data: HduData::I32(dq), extra_cards: vec![] },
            ],
        );
        let k = format!("{}#hdu=1", path.to_str().unwrap());

        let (_, table) = load_companions(&k).unwrap().dq.expect("dq companion");
        assert_eq!(table, DqTable::Jwst);
        let excluded = dq_exclusion(&k).unwrap().expect("exclusion map");
        assert_eq!(excluded[[1, 1]], 1);
        assert_eq!(excluded[[1, 2]], 0);
        assert_eq!(excluded.iter().filter(|&&v| v != 0).count(), 1);
        let info = plane_info_json(&k, &load_cached_full(&k).unwrap()).unwrap();
        assert_eq!(info[RES_DQ_TABLE], "jwst");
    }

    #[test]
    fn the_open_response_names_the_layers_without_loading_them() {
        let dir = tempfile::tempdir().unwrap();
        let p = afw_mef(dir.path(), "calexp.fits", vec![]);
        let k = format!("{p}#hdu=1");

        let j = plane_info_json(&k, &load_cached_full(&k).unwrap()).unwrap();
        assert_eq!(j["layer"], json!({"role": "science", "basis": "EXTNAME IMAGE", "assumed": false}));
        assert_eq!(
            j["err_origin"],
            json!({"role": "variance", "source": "EXTNAME VARIANCE, HDU 3: square root", "assumed": false, "n_no_data": null, "origin": "same_file"})
        );
        assert_eq!(j["dq_origin"], "EXTNAME MASK, HDU 2");
        assert_eq!(j["dq_origin_kind"], "same_file");
        assert_eq!(j["pairing_notes"], json!([]));
        assert!(j["err_unavailable"].is_null() && j["dq_unavailable"].is_null(), "{j}");
        assert!(!GLOBAL_IMAGE_CACHE.contains(&sigma_key(&ImageRef::hdu(&p, 3), LayerRole::Variance)));
        assert!(!GLOBAL_IMAGE_CACHE.contains(&format!("{p}#hdu=3")));

        let loaded = load_companions(&k).unwrap();
        let again = plane_info_json(&k, &load_cached_full(&k).unwrap()).unwrap();
        assert_eq!(again["err_origin"]["n_no_data"], 1);
        assert_eq!(again["err_origin"]["source"], loaded.err_origin.expect("the variance was converted").source.as_str());
    }

    #[test]
    fn a_dq_that_cannot_be_read_as_integers_is_not_advertised_as_usable() {
        let dir = tempfile::tempdir().unwrap();
        let p = float_dq_mef(dir.path(), "float_dq.fits");
        let k = format!("{p}#hdu=1");

        let j = plane_info_json(&k, &load_cached_full(&k).unwrap()).unwrap();
        assert!(j[RES_DQ_REF].is_null(), "{j}");
        assert_eq!(
            j["dq_unavailable"],
            "mask layer HDU 2 is a floating-point plane (BITPIX -32): flag bits cannot be read exactly"
        );
        assert!(j[RES_DQ_TABLE].is_null(), "{j}");
        assert!(j["dq_origin"].is_null() && j["dq_origin_kind"].is_null(), "{j}");
    }

    #[test]
    fn a_layer_of_another_size_is_reported_in_the_open_response() {
        use crate::infra::fits::reader::test_fixtures::{ramp_f32, write_test_mef, HduData, TestHdu};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small_variance.fits");
        write_test_mef(
            &path,
            &[],
            &[
                TestHdu { extname: Some("SCI"), extver: None, cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![] },
                TestHdu { extname: Some("VARIANCE"), extver: None, cols: 2, rows: 2, data: HduData::F32(vec![1.0; 4]), extra_cards: vec![] },
            ],
        );
        let k = format!("{}#hdu=1", path.to_str().unwrap());

        let j = plane_info_json(&k, &load_cached_full(&k).unwrap()).unwrap();
        assert!(j[RES_ERR_REF].is_null(), "{j}");
        assert!(j["err_origin"].is_null(), "{j}");
        assert_eq!(j["err_unavailable"], "error layer HDU 2 (VARIANCE) is 2x2 and does not match the image 4x4: not used");
    }

    #[test]
    fn a_sibling_pairing_is_said_in_the_open_response() {
        use crate::infra::fits::reader::test_fixtures::{write_test_mef, HduData, TestHdu};
        let dir = tempfile::tempdir().unwrap();
        let in_folder = |name: &str| dir.path().join(name).to_str().unwrap().to_string();
        let image = in_folder("legacysurvey-0001p000-image-r.fits");
        let invvar = in_folder("legacysurvey-0001p000-invvar-r.fits");
        let maskbits = in_folder("legacysurvey-0001p000-maskbits.fits");
        write_fits_mono(&image, &Array2::from_shape_fn((4, 4), |(y, x)| (y * 4 + x) as f32), None).unwrap();
        write_fits_mono(&invvar, &Array2::from_elem((4, 4), 4.0), None).unwrap();
        let names = vec![("MBIT_3", "'SATUR_R'".to_string()), ("MBIT_6", "'ALLMASK_R'".to_string())];
        write_test_mef(
            Path::new(&maskbits),
            &[],
            &[TestHdu { extname: Some("MASKBITS"), extver: None, cols: 4, rows: 4, data: HduData::I32(vec![0; 16]), extra_cards: names }],
        );

        let j = plane_info_json(&image, &load_cached_full(&image).unwrap()).unwrap();
        assert_eq!(
            j["pairing_notes"],
            json!([
                "error layer taken from the sibling file legacysurvey-0001p000-invvar-r.fits (name rule Legacy brick, same 4x4 grid)",
                "mask layer taken from the sibling file legacysurvey-0001p000-maskbits.fits (name rule Legacy brick, same 4x4 grid)",
            ])
        );
        assert_eq!(j[RES_ERR_REF], invvar);
        assert_eq!(j["err_origin"]["origin"], "sibling");
        assert_eq!(j["err_origin"]["source"], "file name -invvar-, legacysurvey-0001p000-invvar-r.fits: 1 / square root");
        assert_eq!(j[RES_DQ_REF], maskbits);
        assert_eq!(j["dq_origin"], "file name -maskbits, legacysurvey-0001p000-maskbits.fits");
        assert_eq!(j["dq_origin_kind"], "sibling");
    }

    #[test]
    fn the_plane_description_carries_the_asinh_warning_and_the_recipe() {
        use crate::infra::sdss_frame_file::test_support::{write_frame, FRAME_XINTERP};
        use crate::infra::sdss_frame_file::SDSS_NO_ERROR_REASON;
        let dir = tempfile::tempdir().unwrap();
        let in_folder = |name: &str| dir.path().join(name).to_str().unwrap().to_string();
        let mut linearised_cards = HduHeader::empty();
        linearised_cards.set_f64("ABSOFTEN", 0.25);
        linearised_cards.set_f64("ABOFFSET", 3.5);
        let linearised = in_folder("ps1_linearised.fits");
        write_fits_mono(&linearised, &sky(4, 4), Some(&linearised_cards)).unwrap();
        let j = plane_info_json(&linearised, &load_cached_full(&linearised).unwrap()).unwrap();
        assert_eq!(j["warnings"], json!(["PS1 asinh-scaled pixels were linearised (BOFFSET, BSOFTEN)"]), "{j}");
        assert!(j["err_recipe"].is_null(), "{j}");

        let plain = in_folder("plain.fits");
        write_fits_mono(&plain, &sky(4, 4), None).unwrap();
        let j = plane_info_json(&plain, &load_cached_full(&plain).unwrap()).unwrap();
        assert_eq!(j["warnings"], json!([]), "{j}");

        let frame = write_frame(dir.path(), "frame.fits", &FRAME_XINTERP);
        let j = plane_info_json(&frame, &load_cached_full(&frame).unwrap()).unwrap();
        assert_eq!(j["err_recipe"], json!({"kind": "sdss_frame", "gain": null, "dark_variance": null}), "{j}");
        assert_eq!(j["err_unavailable"], SDSS_NO_ERROR_REASON);
        assert_eq!(j["warnings"], json!([]));
    }

    #[test]
    fn the_plane_description_of_a_band_names_the_cube_the_band_and_the_real_file() {
        use crate::infra::image_source::test_support::{write_band_cutout, CUTOUT_INVVAR, CUTOUT_NAME};
        let dir = tempfile::tempdir().unwrap();
        let p = write_band_cutout(dir.path(), CUTOUT_NAME, CUTOUT_INVVAR);
        let r_band = format!("{p}#hdu=0&plane=1");
        let j = plane_info_json(&r_band, &load_cached_full(&r_band).unwrap()).unwrap();
        assert_eq!(j[RES_KIND], "hdu", "{j}");
        assert_eq!(j[RES_INDEX], 0);
        assert_eq!(j[RES_SOURCE_PATH], p);
        assert_eq!(j["band_plane"], json!({"hdu": 0, "plane": 1, "label": "r"}));
        assert_eq!(j[RES_ERR_REF], format!("{p}#hdu=1&plane=1"));
        assert_eq!(j[RES_DQ_REF], format!("{p}#hdu=2"));
        assert_eq!(j["err_origin"]["source"], "IMAGETYP INVVAR, HDU 1 plane 1: 1 / square root");
        assert_eq!(j["layer"]["basis"], "IMAGETYP IMAGE");

        let mask = format!("{p}#hdu=2");
        let j = plane_info_json(&mask, &load_cached_full(&mask).unwrap()).unwrap();
        assert!(j["band_plane"].is_null() && j.as_object().unwrap().contains_key("band_plane"), "{j}");
        assert_eq!(j[RES_SOURCE_PATH], p);

        let plain = dir.path().join("plain.fits").to_str().unwrap().to_string();
        write_fits_mono(&plain, &sky(3, 4), None).unwrap();
        let j = plane_info_json(&plain, &load_cached_full(&plain).unwrap()).unwrap();
        assert!(j["band_plane"].is_null(), "{j}");
        assert_eq!(j[RES_SOURCE_PATH], plain);

        let weight_band = format!("{p}#hdu=1&plane=0");
        let j = plane_info_json(&weight_band, &load_cached_full(&weight_band).unwrap()).unwrap();
        assert_eq!(j[RES_INDEX], 1, "{j}");
        assert_eq!(j["band_plane"], json!({"hdu": 1, "plane": 0, "label": "g"}));
        assert_eq!(j[RES_SOURCE_PATH], p);
        assert!(j[RES_ERR_REF].is_null() && j[RES_DQ_REF].is_null(), "{j}");

        let odd = dir.path().join("odd.fits#hdu=0&plane=1").to_str().unwrap().to_string();
        write_fits_mono(&odd, &sky(3, 4), None).unwrap();
        let j = plane_info_json(&odd, &load_cached_full(&odd).unwrap()).unwrap();
        assert_eq!(j[RES_INDEX], 0, "{j}");
        assert!(j["band_plane"].is_null(), "{j}");
        assert_eq!(j[RES_SOURCE_PATH], odd);
    }

    fn legacy_cutout() -> String {
        crate::core::astrometry::gwcs::test_support::env_dir("ASTROBURST_SCIENCE_DATA_DIR")
            .join("survey")
            .join("L1")
            .join("ls-dr10_A_griz_invvar_maskbits_458px.fits")
            .to_str()
            .unwrap()
            .to_string()
    }

    fn relative_gap(measured: f32, expected: f64) -> f64 {
        ((f64::from(measured) - expected) / expected).abs()
    }

    fn median_of(values: &Array2<f32>) -> f64 {
        let mut sorted: Vec<f64> = values.iter().filter(|v| v.is_finite()).map(|v| f64::from(*v)).collect();
        sorted.sort_by(f64::total_cmp);
        let mid = sorted.len() / 2;
        if sorted.len() % 2 == 0 {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        } else {
            sorted[mid]
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L1/ls-dr10_A_griz_invvar_maskbits_458px.fits"]
    fn real_legacy_cutout_bands_take_their_invvar_plane_and_the_mask() {
        use crate::cmd::metadata::extensions_json;
        let file = legacy_cutout();
        let r_band = format!("{file}#hdu=0&plane=1");
        let j = plane_info_json(&r_band, &load_cached_full(&r_band).unwrap()).unwrap();
        println!("legacy cutout r band: band_plane {}, err_ref {}, dq_ref {}", j["band_plane"], j[RES_ERR_REF], j[RES_DQ_REF]);
        assert_eq!(j["band_plane"], json!({"hdu": 0, "plane": 1, "label": "r"}));
        assert!(j[RES_ERR_REF].as_str().unwrap().ends_with("#hdu=1&plane=1"), "{j}");
        assert!(j[RES_DQ_REF].as_str().unwrap().ends_with("#hdu=2"), "{j}");

        let loaded = load_companions(&r_band).unwrap();
        let sigma = loaded.err.as_ref().expect("the r plane of the INVVAR cube is the error layer").arr().clone();
        for (at, expected) in [
            ([0, 0], 0.0010828025949942315),
            ([229, 229], 0.0021034510956936597),
            ([457, 457], 0.0014306037729749956),
            ([30, 7], 0.0011229846740154635),
            ([429, 453], 0.0009375348263118371),
        ] {
            let gap = relative_gap(sigma[at], expected);
            println!("legacy cutout r sigma{at:?} = {} (oracle {expected}, relative {gap:.1e})", sigma[at]);
            assert!(gap <= 1e-6, "sigma{at:?} = {}, oracle {expected}", sigma[at]);
        }
        let median = median_of(&sigma);
        let origin = loaded.err_origin.as_ref().expect("the error layer says where it came from");
        println!(
            "legacy cutout r sigma: median {median} (oracle 0.001301182817121074), n_no_data {}, source {}",
            origin.n_no_data, origin.source
        );
        assert!(((median - 0.001301182817121074) / 0.001301182817121074).abs() <= 1e-6);
        assert_eq!(origin.n_no_data, 0);
        assert_eq!(origin.source, "IMAGETYP INVVAR, HDU 1 plane 1: 1 / square root");

        let (mask, _) = loaded.dq.as_ref().expect("the MASKBITS HDU is the mask");
        let bits = mask.int_plane().expect("the mask is read as integers");
        for ((y, x), expected) in [((0, 0), 1024), ((30, 7), 256), ((429, 453), 2048), ((7, 380), 1025), ((0, 9), 0)] {
            println!("legacy cutout mask [{y}, {x}] = {} (expected {expected})", bits.value_at(y, x));
            assert_eq!(bits.value_at(y, x), expected, "[{y}, {x}]");
        }
        let source = loaded.flags.as_ref().map(|flags| flags.source.clone());
        println!("legacy cutout r flags: {source:?}, exclusion {}, dq_origin {:?}", loaded.exclusion_mask(), loaded.dq_origin);
        assert_eq!(source.as_deref(), Some("Legacy DR10 maskbits (built-in table)"));
        assert_eq!(loaded.exclusion_mask(), 72);

        for (plane, band, expected, exclusion) in
            [(0, "g", 0.0009721814728914486, 36), (2, "i", 0.0017546505292082596, 49152), (3, "z", 0.0033517020869796863, 144)]
        {
            let key = format!("{file}#hdu=0&plane={plane}");
            let loaded = load_companions(&key).unwrap();
            let sigma = loaded.err.as_ref().expect("each band has its error plane").arr()[[0, 0]];
            let gap = relative_gap(sigma, expected);
            println!("legacy cutout {band} sigma[0, 0] = {sigma} (oracle {expected}, relative {gap:.1e}), exclusion {}", loaded.exclusion_mask());
            assert!(gap <= 1e-6, "{band}");
            assert_eq!(loaded.exclusion_mask(), exclusion, "{band}");
        }

        let listed = extensions_json(&file).unwrap();
        let rows = listed["extensions"].as_array().unwrap();
        for row in rows {
            println!("legacy cutout HDU {}: role {}, bands {}", row[RES_INDEX], row["role"], row["bands"]);
        }
        let labels = |row: &serde_json::Value| -> Vec<String> {
            row["bands"].as_array().unwrap().iter().map(|band| band["label"].as_str().unwrap().to_string()).collect()
        };
        assert_eq!(rows.len(), 3);
        assert_eq!(labels(&rows[0]), ["g", "r", "i", "z"]);
        assert_eq!(labels(&rows[1]), ["g", "r", "i", "z"]);
        assert_eq!(rows[2]["bands"], json!([]));
        println!("legacy cutout r plane description: {j}");
    }
}
