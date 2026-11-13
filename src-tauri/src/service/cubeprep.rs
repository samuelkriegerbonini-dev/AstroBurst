use std::fs::File;
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::service::area::{area_companion, AREA_EXTNAME};
use crate::service::image_files::{
    invalidate_written, load_cached_full, load_companions, output_stem, source_primary_header,
    write_derived_mef, write_derived_mef_with_tables,
};
use crate::core::ops::OutputValues;
use crate::service::cutout::{export_cutout_rect, padded_dq_bits, refuse_source_as_target, resolve_output_target, CutoutExport};
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::cubeprep::align::{
    align_image_with_companions, choose_reference, plan_alignment_from_dims, same_grid, AlignCompanions, AlignGrid, AlignInput, AlignOptions, AlignReport,
    ReferenceCandidate, ReferencePolicy, ResampleMethod, CARD_NCORRK, CARD_NCORRSRC, CARD_NOISCORR_ALIGN, NOISCORR_EXTNAME, NOISCORR_SOURCE_ALIGN,
};
use crate::core::cubeprep::cube::{
    apply_nan_mask, assemble_cube, assemble_cube_with_noise, companion_header, cube_base_header, dq_table_of, drop_invalid_planes, is_cube_axis_card,
    match_statuses, planes_from_header, planes_from_table, remove_filter, subcube, update_metadata, without_hdu_identity, Cube, CubeCentre,
    CubeEdit, CubeError, CubeInput, CubeLayout, CubeNoise, CubeOptions, CubePlane, DqPlanes, FiltersTable, InputNoise, MatchStatus, PlaneOrder,
    DQ_DO_NOT_USE, FILTERS_EXTNAME,
};
use crate::core::cubeprep::kernel::{
    build_kernel, kernel_file_name, kernel_header, AutoChoice, KernelDiagnostics, KernelError, KernelMethod, KernelOptions, KernelShape, ShapeName,
};
use crate::core::cubeprep::match_cube::{
    match_cube_with, KernelCards, MatchError, MatchMode, MatchOptions, MatchProducts, MatchedCube, MatchedPlane, PlaneKernel, INSUFFICIENT_MEMORY,
};
use crate::core::cubeprep::psf_io::{
    decode_planes_f64, image_hdus, load_psf, pixel_scale, prepare_psf_set, prepared_psf_header, LoadedPsf, PsfDiagnostics, PsfError,
    PsfResampleMethod, PsfSetOptions, ResampleRecord, ScaleSource, SCALE_RTOL,
};
use crate::core::cubeprep::footprint::{
    polygon_area_arcsec2, resolve_crop_shape, CropFrame, CropRegion, CropRounding, CropShape,
};
use crate::core::imaging::dq_flags::DqTable;
use crate::core::metadata::filter_wavelengths::{FilterEntry, FilterRegistry, FilterSource};
use crate::core::metadata::photcal::card_text;
use crate::core::metadata::units::{apply_conversion, classify_unit, unit_conversion, TargetUnit, UnitClass};
use crate::core::stacking::memory::{effective_memory, format_bytes, header_size, MemoryInfo};
use crate::infra::cache::{ImageEntry, GLOBAL_IMAGE_CACHE};
use crate::infra::config::load_config;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::file_bytes::read_file_bytes;
use crate::infra::fits::reader::{
    extract_header_by_index, extract_image_planes_by_index, extract_int_planes_by_index, list_extensions, HduInfo,
};
use crate::infra::fits::table::{find_bintable, load_bintable, BintableData};
use crate::infra::fits::writer::{is_wcs_card, write_fits_cube, write_fits_f64, BinTableHdu, HduData, ImageHdu};
use crate::infra::image_source::{layer_absent_without_fault, LoadedCompanions};
use crate::infra::wcs_source::{header_wcs, load_wcs};
use crate::service::common::{cached_header, derived_output_header};
use crate::types::constants::{EXTNAME_DQ, EXTNAME_ERR, EXTNAME_SCI};
use crate::types::error::{AppError, CodedError};
use crate::types::header::HduHeader;
use crate::types::image_ref::ImageRef;

const ABPROC_UNITS: &str = "units";
const ABPROC_ALIGNED: &str = "aligned";
const ABPROC_CUBE: &str = "cube";
const ABPROC_CUBE_EDIT: &str = "cube-edit";
const ABPROC_PSF_MATCHED: &str = "psf-matched";
const DEFAULT_CUBE_NAME: &str = "datacube.fits";
const PRIMARY_HDU_NAME: &str = "PRIMARY";
const KERNEL_SHAPE: &str = "kernel_shape";
const LIGHTSTACK_LAYOUT_DROPS_COMPANIONS: &str =
    "the lightstack layout holds the science planes only: the ERR and DQ cubes are not written; the mef layout keeps them";
const LIGHTSTACK_LAYOUT_DROPS_NOISE: &str =
    "the lightstack layout holds no NOISCORR noise correlation maps: they are not written; the mef layout keeps them";
const AREA_MAP_WARNING: &str =
    "the AREA pixel-area map was carried unchanged; the conversion used the nominal pixel area, as LIGHTSTACK does";
const CROP_FIRST_ADVICE: &str =
    "add a crop step before this one, close other applications, or, if the system can page this much, set ASTROBURST_MEMORY_BUDGET_MB to the memory to allow (in MB) and restart";
const CLOSE_APPLICATIONS_ADVICE: &str =
    "crop the images one at a time, close other applications, or, if the system can page this much, set ASTROBURST_MEMORY_BUDGET_MB to the memory to allow (in MB) and restart";
const PAGING_FRACTION: f64 = 0.8;
const INPUT_BYTES_PER_PIXEL: u64 = 16;
const SCIENCE_BYTES_PER_PIXEL: u64 = 8;
const ALIGN_GRID_BYTES_PER_PIXEL: u64 = 32;
const ALIGN_INPUT_WORK_BYTES_PER_PIXEL: u64 = 12;
const NOISE_BAND_ROWS: u64 = 32;
const NOISE_TAP_BYTES: u64 = 16;
const NOISE_CENTRE_TOLERANCE: f64 = 1e-6;

fn check_cancelled(cancelled: &(dyn Fn() -> bool + Sync)) -> Result<()> {
    if cancelled() {
        return Err(AppError::Cancelled.into());
    }
    Ok(())
}

fn check_preparation_memory(required: u64, memory: Option<MemoryInfo>, subject: &str, advice: &'static str) -> Result<Option<String>> {
    let Some(memory) = memory else {
        return Ok(None);
    };
    if required > memory.commit_available {
        let message = format!(
            "{subject} needs about {} of memory but only {} is available: {advice}",
            format_bytes(required),
            format_bytes(memory.commit_available)
        );
        return Err(CodedError { code: INSUFFICIENT_MEMORY, message, hint: Some(advice.to_string()) }.into());
    }
    Ok((required as f64 > PAGING_FRACTION * memory.physical_available as f64).then(|| {
        format!(
            "{subject} needs about {} of memory and {} is free; the system may page to disk and slow down",
            format_bytes(required),
            format_bytes(memory.physical_available)
        )
    }))
}

fn pixels(rows: usize, cols: usize) -> u64 {
    (rows as u64).saturating_mul(cols as u64)
}

enum DqPixels {
    Unsigned(Array2<u32>),
    Signed(Array2<i32>),
}

fn load_preparation_companions(path: &str, sci: &ImageEntry) -> Result<(LoadedCompanions, Vec<String>)> {
    let loaded = load_companions(path)?;
    let faults = [loaded.err_unavailable.as_deref(), loaded.dq_unavailable.as_deref()];
    if let Some(reason) = faults.into_iter().flatten().find(|reason| !layer_absent_without_fault(reason)) {
        bail!("{}: {reason}", output_stem(path));
    }
    let mut warnings = Vec::new();
    let header = sci.header().context("SCI has no header for companion validation")?;
    let declared = sci.companions();
    for (name, expected, companion) in [
        ("ERR", declared.is_some_and(|planes| planes.err.is_some()), loaded.err.as_ref()),
        ("DQ", declared.is_some_and(|planes| planes.dq.is_some()), loaded.dq.as_ref().map(|(plane, _)| plane)),
    ] {
        let Some(companion) = companion else {
            if expected { bail!("{}: {name} companion could not be loaded with the SCI dimensions", output_stem(path)); }
            continue;
        };
        if companion.arr().dim() != sci.arr().dim() { bail!("{}: {name} dimensions differ from SCI", output_stem(path)); }
        let companion_header = companion.header().context("companion has no header")?;
        if name == "ERR" {
            let sci_unit = classify_unit(header);
            let err_unit = classify_unit(companion_header);
            match &err_unit.bunit {
                None => warnings.push(format!("ERR has no BUNIT: the SCI unit {} is assumed", sci_unit.bunit.as_deref().unwrap_or("(unspecified)"))),
                Some(unit) => {
                    let same_physical_scale = matches!(sci_unit.class, UnitClass::SurfaceBrightness { .. } | UnitClass::PerPixelPhysical { .. }) && sci_unit.class == err_unit.class;
                    if sci_unit.bunit.as_ref() != Some(unit) && !same_physical_scale {
                        bail!("{}: ERR BUNIT {unit} differs from SCI BUNIT {}; convert or correct the input units before preparation", output_stem(path), sci_unit.bunit.as_deref().unwrap_or("(missing)"));
                    }
                }
            }
        }
        if companion_header.cards.iter().any(|(key, _)| is_wcs_card(key)) {
            let companion_wcs = header_wcs(companion_header).with_context(|| format!("{}: {name} has an incomplete celestial WCS", output_stem(path)))?;
            let sci_wcs = load_wcs(path, header).with_context(|| format!("{}: SCI has no usable WCS to validate {name}", output_stem(path)))?;
            if !same_grid(&sci_wcs, sci.arr().dim(), &companion_wcs, companion.arr().dim()) {
                bail!("{}: {name} WCS differs from SCI", output_stem(path));
            }
        } else {
            warnings.push(format!("{name} has no celestial WCS: same-index alignment to SCI is assumed from matching dimensions"));
        }
    }
    Ok((loaded, warnings))
}

#[derive(Debug, Clone, Serialize)]
pub struct UnitsExport {
    pub output_path: String,
    pub bunit_in: Option<String>,
    pub bunit_out: String,
    pub factor: f64,
    pub method: String,
    pub hdus: Vec<String>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

pub(crate) fn convert_units_file(path: &str, target: TargetUnit, output_path: &str, output_dir: Option<&str>) -> Result<UnitsExport> {
    let started = Instant::now();
    let entry = load_cached_full(path)?;
    let source_header = entry.header().context("the image has no header, so its unit is unknown")?;
    let wcs = load_wcs(path, source_header).ok();
    let conversion = unit_conversion(source_header, wcs.as_ref(), target)?;
    let (companions, companion_warnings) = load_preparation_companions(path, &entry)?;
    let destination =
        resolve_output_target(path, output_path, output_dir, &format!("{}_{}.fits", output_stem(path), target.file_suffix()))?;
    let mut warnings = conversion.warnings.clone();
    warnings.extend(companion_warnings);

    let mut sci = entry.arr().to_owned();
    let mut sci_header = derived_output_header(Some(source_header), ABPROC_UNITS, OutputValues::Linear);
    let mut err = companions
        .err
        .as_ref()
        .map(|plane| (plane.arr().to_owned(), derived_output_header(plane.header(), ABPROC_UNITS, OutputValues::Linear)));
    let (err_plane, err_header) = match err.as_mut() {
        Some((plane, header)) => (Some(plane), Some(header)),
        None => (None, None),
    };
    apply_conversion(&conversion, &mut sci, err_plane, &mut sci_header, err_header);

    let dq = match companions.dq.as_ref() {
        Some((plane, _)) => {
            let bits = plane.int_plane().context("DQ companion has no integer plane")?;
            let pixels = if bits.signed { DqPixels::Signed(bits.bits.mapv(|b| b as i32)) } else { DqPixels::Unsigned(bits.bits.clone()) };
            Some((pixels, plane.header().cloned()))
        }
        None => None,
    };
    let area = match area_companion(path, &entry) {
        Ok(area) => area,
        Err(reason) => {
            warnings.push(format!("AREA extension not carried: {reason}"));
            None
        }
    };
    if area.is_some() {
        warnings.push(AREA_MAP_WARNING.to_string());
    }

    let primary = source_primary_header(path, Some(source_header));
    let mut hdus = vec![ImageHdu { data: HduData::F32(&sci), header: Some(&sci_header), extname: EXTNAME_SCI, extver: 1 }];
    if let Some((plane, header)) = err.as_ref() {
        hdus.push(ImageHdu { data: HduData::F32(plane), header: Some(header), extname: EXTNAME_ERR, extver: 1 });
    }
    if let Some((pixels, header)) = dq.as_ref() {
        let data = match pixels {
            DqPixels::Unsigned(bits) => HduData::U32(bits),
            DqPixels::Signed(bits) => HduData::I32(bits),
        };
        hdus.push(ImageHdu { data, header: header.as_ref(), extname: EXTNAME_DQ, extver: 1 });
    }
    if let Some(plane) = area.as_ref() {
        hdus.push(ImageHdu { data: HduData::F32(plane.arr()), header: plane.header(), extname: AREA_EXTNAME, extver: 1 });
    }
    write_derived_mef(&destination, Some(&primary), &hdus)?;

    Ok(UnitsExport {
        output_path: destination,
        bunit_in: conversion.bunit_in,
        bunit_out: conversion.bunit_out.to_string(),
        factor: conversion.factor,
        method: conversion.method,
        hdus: hdus.iter().map(|hdu| hdu.extname.to_string()).collect(),
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}


#[derive(Debug, Clone, Serialize)]
pub struct CroppedFile {
    pub path: String,
    #[serde(flatten)]
    pub cutout: CutoutExport,
}

#[derive(Debug, Clone, Serialize)]
pub struct CropSetReport {
    pub rounding: CropRounding,
    pub shape: CropShape,
    pub area_arcsec2: Option<f64>,
    pub files: Vec<CroppedFile>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

struct LoadedImage {
    entry: ImageEntry,
    header: HduHeader,
    wcs: WcsTransform,
}

impl LoadedImage {
    fn load(path: &str) -> Result<Self> {
        let entry = load_cached_full(path)?;
        let header = entry
            .header()
            .cloned()
            .ok_or_else(|| anyhow!("{}: the image has no header, so it has no WCS", output_stem(path)))?;
        let wcs = load_wcs(path, &header).map_err(|error| anyhow!("{}: no usable WCS: {error:#}", output_stem(path)))?;
        Ok(Self { entry, header, wcs })
    }
}

struct ImageFrame {
    header: HduHeader,
    wcs: WcsTransform,
    rows: usize,
    cols: usize,
}

impl ImageFrame {
    fn read(path: &str) -> Result<Self> {
        let header = cached_header(&GLOBAL_IMAGE_CACHE, path)?;
        let wcs = load_wcs(path, &header).map_err(|error| anyhow!("{}: no usable WCS: {error:#}", output_stem(path)))?;
        let (rows, cols) = declared_dims(path, &header)?;
        Ok(Self { header, wcs, rows, cols })
    }

    fn crop_frame(&self) -> CropFrame<'_> {
        CropFrame { header: &self.header, wcs: &self.wcs, dims: (self.cols, self.rows) }
    }

    fn pixels(&self) -> u64 {
        pixels(self.rows, self.cols)
    }
}

fn declared_dims(path: &str, header: &HduHeader) -> Result<(usize, usize)> {
    let size = header_size(header);
    if size.width > 0 && size.height > 0 {
        return Ok((size.height as usize, size.width as usize));
    }
    Ok(load_cached_full(path)?.arr().dim())
}

fn read_frames(paths: &[String], what: &str) -> Result<Vec<ImageFrame>> {
    if paths.is_empty() {
        bail!("no image to {what}");
    }
    paths.iter().map(|path| ImageFrame::read(path)).collect()
}

fn largest<'a>(paths: &'a [String], frames: &'a [ImageFrame], bytes: impl Fn(&ImageFrame) -> u64) -> Result<(&'a String, &'a ImageFrame, u64)> {
    paths
        .iter()
        .zip(frames)
        .map(|(path, frame)| (path, frame, bytes(frame)))
        .max_by_key(|(_, _, bytes)| *bytes)
        .context("no image in the set")
}

pub(crate) fn crop_set(
    paths: &[String],
    region: &CropRegion,
    rounding: CropRounding,
    output_dir: &str,
    include_err: bool,
    include_dq: bool,
) -> Result<CropSetReport> {
    crop_set_with(paths, region, rounding, output_dir, include_err, include_dq, &|| false)
}

pub(crate) fn crop_set_with(
    paths: &[String],
    region: &CropRegion,
    rounding: CropRounding,
    output_dir: &str,
    include_err: bool,
    include_dq: bool,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<CropSetReport> {
    let started = Instant::now();
    let images = read_frames(paths, "crop")?;
    let frames: Vec<CropFrame> = images.iter().map(ImageFrame::crop_frame).collect();
    let reference = match region {
        CropRegion::Reference { reference } => Some(ImageFrame::read(reference)?),
        _ => None,
    };
    let region_text = match region {
        CropRegion::Region { region_file, .. } | CropRegion::RegionCommon { region_file } => Some(
            std::fs::read_to_string(region_file).with_context(|| format!("Failed to read the region file {region_file}"))?,
        ),
        _ => None,
    };
    let reference_frame = reference.as_ref().map(ImageFrame::crop_frame);
    let resolved = resolve_crop_shape(region, rounding, &frames, reference_frame.as_ref(), region_text.as_deref())?;
    let rects = frames
        .iter()
        .zip(paths)
        .map(|(frame, path)| resolved.shape.rect(frame).map_err(|error| anyhow!("{}: {error}", output_stem(path))))
        .collect::<Result<Vec<_>>>()?;
    let (path, frame, required) = largest(paths, &images, |frame| frame.pixels().saturating_mul(INPUT_BYTES_PER_PIXEL))?;
    let subject = format!("Cropping {} ({}x{})", output_stem(path), frame.cols, frame.rows);
    let memory = check_preparation_memory(required, effective_memory(), &subject, CLOSE_APPLICATIONS_ADVICE)?;
    let mut warnings = resolved.warnings.clone();
    warnings.extend(memory);
    let mut files = Vec::with_capacity(paths.len());
    for (path, rect) in paths.iter().zip(rects) {
        check_cancelled(cancelled)?;
        let cutout = export_cutout_rect(path, "", Some(output_dir), rect, include_err, include_dq)?;
        if cutout.fraction_on_image < 1.0 {
            warnings.push(format!(
                "{}: the crop extends beyond the image; the outside is NaN and flagged in DQ",
                output_stem(path)
            ));
        }
        files.push(CroppedFile { path: path.clone(), cutout });
    }
    let area_arcsec2 = match &resolved.shape {
        CropShape::Polygon { vertices } => Some(polygon_area_arcsec2(vertices)),
        _ => None,
    };
    Ok(CropSetReport {
        rounding,
        shape: resolved.shape,
        area_arcsec2,
        files,
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}


#[derive(Debug, Clone, Serialize)]
pub struct AlignedFile {
    pub path: String,
    pub output_path: String,
    #[serde(flatten)]
    pub report: AlignReport,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlignSetReport {
    pub reference_index: usize,
    pub reference_rule: &'static str,
    pub reference_path: String,
    pub method: ResampleMethod,
    pub border_crop: usize,
    pub rows: usize,
    pub cols: usize,
    pub files: Vec<AlignedFile>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

static LOGGED_FILTER_REGISTRY_WARNINGS: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

pub fn filter_registry_report() -> (FilterRegistry, Vec<String>) {
    load_config().map_or_else(|_| (FilterRegistry::builtin(), Vec::new()), |config| FilterRegistry::with_custom_datasets(&config.filter_datasets))
}

pub fn filter_registry() -> FilterRegistry {
    let (registry, warnings) = filter_registry_report();
    let mut logged = LOGGED_FILTER_REGISTRY_WARNINGS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for warning in warnings {
        if !logged.contains(&warning) {
            log::warn!("{warning}");
            logged.insert(warning);
        }
    }
    registry
}

fn identified_filters(paths: &[String], frames: &[ImageFrame], policy: &ReferencePolicy) -> Vec<Option<String>> {
    if !matches!(policy, ReferencePolicy::Filter { .. }) {
        return vec![None; paths.len()];
    }
    let registry = filter_registry();
    paths
        .iter()
        .zip(frames)
        .map(|(path, frame)| registry.identify(Some(&frame.header), path, None).ok().map(|found| found.entry.filter))
        .collect()
}

fn noise_pass_bytes(method: ResampleMethod, scale_ratio: f64, cols: usize) -> u64 {
    let ratio = if scale_ratio.is_finite() && scale_ratio > 0.0 { scale_ratio } else { 1.0 };
    let taps: u64 = match method {
        ResampleMethod::Bilinear => 4,
        ResampleMethod::Bicubic => 16,
        ResampleMethod::Exact => (ratio.ceil() as u64).saturating_add(1).saturating_pow(2),
    };
    let rows = NOISE_BAND_ROWS.saturating_add((1.0 / ratio).ceil() as u64);
    (rayon::current_num_threads() as u64)
        .saturating_mul(rows)
        .saturating_mul(cols as u64)
        .saturating_mul(taps)
        .saturating_mul(NOISE_TAP_BYTES)
}

fn alignment_bytes(frame: &ImageFrame, grid: &AlignGrid, method: ResampleMethod) -> u64 {
    let ratio = grid.wcs.pixel_scale_arcsec() / frame.wcs.pixel_scale_arcsec();
    frame
        .pixels()
        .saturating_mul(INPUT_BYTES_PER_PIXEL + ALIGN_INPUT_WORK_BYTES_PER_PIXEL)
        .saturating_add(pixels(grid.rows, grid.cols).saturating_mul(ALIGN_GRID_BYTES_PER_PIXEL))
        .saturating_add(noise_pass_bytes(method, ratio, grid.cols))
}

fn aligned_destination(output_dir: &str, path: &str, method: ResampleMethod) -> String {
    std::path::Path::new(output_dir).join(format!("{}_aligned_{}.fits", output_stem(path), method.label())).to_string_lossy().into_owned()
}

pub(crate) fn align_set(paths: &[String], output_dir: &str, options: &AlignOptions) -> Result<AlignSetReport> {
    align_set_with(paths, output_dir, options, &|| false)
}

pub(crate) fn align_set_with(paths: &[String], output_dir: &str, options: &AlignOptions, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<AlignSetReport> {
    let started = Instant::now();
    let frames = read_frames(paths, "align")?;
    let filters = identified_filters(paths, &frames, &options.reference);
    let candidates: Vec<ReferenceCandidate> = paths
        .iter()
        .zip(&frames)
        .zip(&filters)
        .map(|((path, frame), filter)| ReferenceCandidate {
            path,
            scale_arcsec: frame.wcs.pixel_scale_arcsec(),
            filter: filter.as_deref(),
        })
        .collect();
    let choice = choose_reference(&candidates, &options.reference)?;
    let reference = &frames[choice.index];
    let (rows, cols) = (reference.rows, reference.cols);
    let label = filters[choice.index].clone().unwrap_or_else(|| output_stem(&paths[choice.index]));
    let grid = AlignGrid { label: &label, header: &reference.header, wcs: &reference.wcs, rows, cols };
    let plans = frames
        .iter()
        .zip(paths)
        .map(|(frame, path)| {
            plan_alignment_from_dims((frame.rows, frame.cols), &frame.header, &frame.wcs, &grid, options).map_err(|error| anyhow!("{}: {error}", output_stem(path)))
        })
        .collect::<Result<Vec<_>>>()?;
    let destinations: Vec<String> = paths.iter().map(|path| aligned_destination(output_dir, path, options.method)).collect();
    for destination in &destinations {
        for path in paths {
            refuse_source_as_target(destination, path)?;
        }
    }
    let (largest_path, largest_frame, required) = largest(paths, &frames, |frame| alignment_bytes(frame, &grid, options.method))?;
    let subject = format!(
        "Aligning {} ({}x{}) onto the {}x{} grid of {label}",
        output_stem(largest_path),
        largest_frame.cols,
        largest_frame.rows,
        cols,
        rows
    );
    let memory = check_preparation_memory(required, effective_memory(), &subject, CROP_FIRST_ADVICE)?;
    for path in paths {
        check_cancelled(cancelled)?;
        let entry = load_cached_full(path)?;
        load_preparation_companions(path, &entry)?;
    }
    std::fs::create_dir_all(output_dir).with_context(|| format!("Failed to create output directory: {output_dir}"))?;
    let mut files = Vec::with_capacity(paths.len());
    let mut warnings: Vec<String> = memory.into_iter().collect();
    for (((path, frame), plan), destination) in paths.iter().zip(&frames).zip(&plans).zip(destinations) {
        check_cancelled(cancelled)?;
        let stem = output_stem(path);
        let entry = load_cached_full(path)?;
        if entry.arr().dim() != (frame.rows, frame.cols) {
            bail!(
                "{stem}: the image is {}x{} but its header declared {}x{} when the set was planned; the file changed during the alignment",
                entry.arr().ncols(),
                entry.arr().nrows(),
                frame.cols,
                frame.rows
            );
        }
        let (companions, companion_warnings) = load_preparation_companions(path, &entry)?;
        let input = AlignInput { image: entry.arr(), header: &frame.header, wcs: &frame.wcs };
        let table = companions.dq.as_ref().map_or_else(|| DqTable::select(input.header), |(_, table)| *table);
        let data = AlignCompanions {
            err: companions.err.as_ref().map(ImageEntry::arr),
            dq: companions.dq.as_ref().and_then(|(plane, _)| plane.int_plane()),
            no_data_dq: padded_dq_bits(table)?,
        };
        let result = align_image_with_companions(&input, &grid, options, plan, data)?;
        let noise_header = result.noise_header();
        let mut aligned = result.aligned;
        aligned.report.warnings.extend(companion_warnings);
        let header = derived_output_header(Some(&aligned.header), ABPROC_ALIGNED, OutputValues::Linear);
        let primary = source_primary_header(path, Some(input.header));
        let err_header = companion_header(&header, true);
        let dq_header = companion_header(&header, false);
        let signed_dq = result.dq.signed.then(|| result.dq.bits.mapv(|bits| bits as i32));
        let mut hdus = vec![ImageHdu { data: HduData::F32(&aligned.image), header: Some(&header), extname: EXTNAME_SCI, extver: 1 }];
        if let Some(err) = result.err.as_ref() {
            hdus.push(ImageHdu { data: HduData::F32(err), header: Some(&err_header), extname: EXTNAME_ERR, extver: 1 });
        }
        let dq_data = signed_dq.as_ref().map_or_else(|| HduData::U32(&result.dq.bits), HduData::I32);
        hdus.push(ImageHdu { data: dq_data, header: Some(&dq_header), extname: EXTNAME_DQ, extver: 1 });
        hdus.push(ImageHdu { data: HduData::F32(&result.noise_correlation), header: Some(&noise_header), extname: NOISCORR_EXTNAME, extver: 1 });
        write_derived_mef(&destination, Some(&primary), &hdus)?;
        warnings.extend(aligned.report.warnings.iter().map(|warning| format!("{stem}: {warning}")));
        files.push(AlignedFile { path: path.clone(), output_path: destination, report: aligned.report });
    }
    let (rows, cols) = files.first().map_or((rows, cols), |file| (file.report.rows, file.report.cols));
    Ok(AlignSetReport {
        reference_index: choice.index,
        reference_rule: choice.rule,
        reference_path: paths[choice.index].clone(),
        method: options.method,
        border_crop: options.border_crop,
        rows,
        cols,
        files,
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CubeInputSpec {
    pub path: String,
    #[serde(default)]
    pub dataset: Option<String>,
    #[serde(default)]
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CubePlaneReport {
    pub path: String,
    #[serde(flatten)]
    pub plane: CubePlane,
    pub identified_by: String,
    pub has_err: bool,
    pub has_dq: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CubeReport {
    pub output_path: String,
    pub layout: CubeLayout,
    pub order: PlaneOrder,
    pub reference_path: String,
    pub rows: usize,
    pub cols: usize,
    pub bunit: String,
    pub hdus: Vec<String>,
    pub planes: Vec<CubePlaneReport>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaneValidity {
    pub filter: String,
    pub pixels: usize,
    pub usable: usize,
    pub above: usize,
    pub dq_mask: Option<u32>,
    pub kept: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CubeEditReport {
    pub output_path: String,
    pub layout: CubeLayout,
    pub edit: CubeEdit,
    pub filters: Vec<String>,
    pub removed: Vec<String>,
    pub rows: usize,
    pub cols: usize,
    pub masked_pixels: Option<usize>,
    pub centre: Option<CubeCentre>,
    pub validity: Option<Vec<PlaneValidity>>,
    pub hdus: Vec<String>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

pub(crate) struct CubeFile {
    pub cube: Cube,
    pub layout: CubeLayout,
    pub noise: Option<CubeNoise>,
    pub statuses: Option<Vec<MatchStatus>>,
    pub warnings: Vec<String>,
}

fn named_cube<'a>(extensions: &'a [HduInfo], name: &str) -> Option<&'a HduInfo> {
    extensions
        .iter()
        .find(|hdu| hdu.naxis == 3 && hdu.extname.as_deref().is_some_and(|extname| extname.trim().trim_matches('\'').trim().eq_ignore_ascii_case(name)))
}

fn check_depth(name: &'static str, planes: usize, depth: usize) -> Result<()> {
    if planes != depth {
        bail!(CubeError::CompanionMismatch(name, planes, depth));
    }
    Ok(())
}

fn noise_extension(fits_path: &std::path::Path) -> Result<Option<(HduHeader, Vec<Array2<f32>>)>> {
    let file = File::open(fits_path).with_context(|| format!("Failed to open {}", fits_path.display()))?;
    let bytes = read_file_bytes(&file)?;
    let label = fits_path.to_string_lossy();
    let hdus = image_hdus(&bytes, &label)?;
    let Some((_, parsed)) = hdus.iter().find(|(_, parsed)| card_text(&parsed.header, "EXTNAME").is_some_and(|name| name.eq_ignore_ascii_case(NOISCORR_EXTNAME))) else {
        return Ok(None);
    };
    let maps = decode_planes_f64(&bytes, parsed, &label)?.into_iter().map(|plane| plane.mapv(|value| value as f32)).collect();
    Ok(Some((parsed.header.clone(), maps)))
}

fn read_cube_noise(fits_path: &std::path::Path, table: &BintableData, depth: usize) -> Result<Option<CubeNoise>> {
    let Some((header, maps)) = noise_extension(fits_path)? else {
        return Ok(None);
    };
    if header.get_i64("NAXIS") != Some(3) {
        bail!("the {NOISCORR_EXTNAME} extension of a cube is a 3-D image with one map per plane");
    }
    check_depth(NOISCORR_EXTNAME, maps.len(), depth)?;
    CubeNoise::from_hdu(table, &header, maps).map(Some)
}

pub(crate) fn read_cube_file(path: &str, registry: &FilterRegistry) -> Result<CubeFile> {
    let reference = ImageRef::parse(path);
    let (fits_path, _tmp) = resolve_single_image(&reference.path)?;
    let file = File::open(&fits_path).with_context(|| format!("Failed to open {path}"))?;
    let extensions = list_extensions(&file)?;
    let (index, layout) = match named_cube(&extensions, EXTNAME_SCI) {
        Some(sci) => (sci.index, CubeLayout::Mef),
        None if extensions.first().is_some_and(|primary| primary.naxis == 3) => (0, CubeLayout::Lightstack),
        None => bail!("{path} is not a filter cube: it has no 3D primary HDU and no 3D SCI extension"),
    };
    let header = without_hdu_identity(&extract_header_by_index(&file, index)?);
    let planes = extract_image_planes_by_index(&file, index)?;
    let depth = planes.len();
    let (err, dq) = match layout {
        CubeLayout::Lightstack => (None, None),
        CubeLayout::Mef => {
            let err = named_cube(&extensions, EXTNAME_ERR).map(|hdu| extract_image_planes_by_index(&file, hdu.index)).transpose()?;
            let dq = named_cube(&extensions, EXTNAME_DQ).map(|hdu| extract_int_planes_by_index(&file, hdu.index)).transpose()?;
            (err, dq.map(|planes| DqPlanes { signed: planes.iter().all(|plane| plane.signed), planes: planes.into_iter().map(|plane| plane.bits).collect() }))
        }
    };
    if let Some(err) = &err {
        check_depth(EXTNAME_ERR, err.len(), depth)?;
    }
    if let Some(dq) = &dq {
        check_depth(EXTNAME_DQ, dq.planes.len(), depth)?;
    }
    let fits_text = fits_path.to_string_lossy();
    let table = find_bintable(&fits_text, FILTERS_EXTNAME, 1).ok().map(|table_index| load_bintable(&fits_text, table_index)).transpose()?;
    let filters = match &table {
        Some(table) => planes_from_table(table)?,
        None => planes_from_header(&header, depth, registry)?,
    };
    check_depth(FILTERS_EXTNAME, filters.len(), depth)?;
    let mut warnings = Vec::new();
    let statuses = table.as_ref().and_then(|table| {
        match_statuses(table).unwrap_or_else(|error| {
            warnings.push(format!("the {FILTERS_EXTNAME} match statuses were not carried: {error:#}"));
            None
        })
    });
    let noise = match (layout, &table) {
        (CubeLayout::Mef, Some(table)) => match read_cube_noise(&fits_path, table, depth) {
            Ok(noise) => noise,
            Err(error) => {
                warnings.push(format!("the {NOISCORR_EXTNAME} maps were not carried: {error:#}"));
                None
            }
        },
        _ => None,
    };
    Ok(CubeFile { cube: Cube { header, planes, err, dq, filters }, layout, noise, statuses, warnings })
}

#[cfg(test)]
pub(crate) fn write_cube_file(path: &str, cube: &Cube, layout: CubeLayout, abproc: &str) -> Result<(Vec<String>, Vec<String>)> {
    write_prepared_cube(path, cube, layout, abproc, FiltersTable::new(&cube.filters), None)
}

fn filters_table(planes: &[CubePlane], statuses: Option<&[MatchStatus]>, noise: Option<&CubeNoise>) -> Result<FiltersTable> {
    let mut table = FiltersTable::new(planes);
    if let Some(statuses) = statuses {
        table = table.with_match(statuses)?;
    }
    if let Some(noise) = noise {
        table = table.with_noise(noise)?;
    }
    Ok(table)
}

fn write_prepared_cube(
    path: &str,
    cube: &Cube,
    layout: CubeLayout,
    abproc: &str,
    table: FiltersTable,
    noise: Option<&CubeNoise>,
) -> Result<(Vec<String>, Vec<String>)> {
    match layout {
        CubeLayout::Lightstack => {
            let mut warnings: Vec<String> =
                (cube.err.is_some() || cube.dq.is_some()).then(|| LIGHTSTACK_LAYOUT_DROPS_COMPANIONS.to_string()).into_iter().collect();
            warnings.extend(noise.map(|_| LIGHTSTACK_LAYOUT_DROPS_NOISE.to_string()));
            GLOBAL_CUBE_CACHE.invalidate(path);
            let written = write_fits_cube(path, &cube.planes, Some(&cube.header));
            invalidate_written(path);
            written?;
            Ok((vec![PRIMARY_HDU_NAME.to_string()], warnings))
        }
        CubeLayout::Mef => {
            let primary = derived_output_header(None, abproc, OutputValues::Linear);
            let err_header = companion_header(&cube.header, true);
            let dq_header = companion_header(&cube.header, false);
            let signed_dq: Option<Vec<Array2<i32>>> =
                cube.dq.as_ref().filter(|dq| dq.signed).map(|dq| dq.planes.iter().map(|plane| plane.mapv(|bits| bits as i32)).collect());
            let noise_maps = noise.map(|noise| (noise.padded_maps(), noise.header()));
            let mut hdus = vec![ImageHdu { data: HduData::F32Cube(&cube.planes), header: Some(&cube.header), extname: EXTNAME_SCI, extver: 1 }];
            if let Some(err) = &cube.err {
                hdus.push(ImageHdu { data: HduData::F32Cube(err), header: Some(&err_header), extname: EXTNAME_ERR, extver: 1 });
            }
            if let Some(dq) = &cube.dq {
                let data = match &signed_dq {
                    Some(planes) => HduData::I32Cube(planes),
                    None => HduData::U32Cube(&dq.planes),
                };
                hdus.push(ImageHdu { data, header: Some(&dq_header), extname: EXTNAME_DQ, extver: 1 });
            }
            if let Some((maps, header)) = &noise_maps {
                hdus.push(ImageHdu { data: HduData::F32Cube(maps), header: Some(header), extname: NOISCORR_EXTNAME, extver: 1 });
            }
            let columns = table.columns();
            let tables = [BinTableHdu { columns: &columns, extname: FILTERS_EXTNAME, extver: 1 }];
            write_derived_mef_with_tables(path, Some(&primary), &hdus, &tables)?;
            let names = hdus.iter().map(|hdu| hdu.extname.to_string()).chain([FILTERS_EXTNAME.to_string()]).collect();
            Ok((names, Vec::new()))
        }
    }
}

fn cube_output_header(cube_header: &HduHeader, abproc: &str) -> HduHeader {
    let mut header = derived_output_header(Some(cube_header), abproc, OutputValues::Linear);
    for (key, value) in cube_header.cards.iter().filter(|(key, _)| is_cube_axis_card(key.trim())) {
        header.set(key.trim(), value.clone());
    }
    header
}

fn identify_input(spec: &CubeInputSpec, header: &HduHeader, registry: &FilterRegistry) -> Result<(FilterEntry, String, Vec<String>)> {
    let stem = output_stem(&spec.path);
    match &spec.filter {
        Some(filter) => {
            let dataset = match &spec.dataset {
                Some(dataset) => dataset.clone(),
                None => registry
                    .identify(Some(header), &spec.path, None)
                    .map(|found| found.entry.dataset)
                    .map_err(|error| anyhow!("{stem}: give the dataset of filter {filter}: {error}"))?,
            };
            let entry = registry.entry(&dataset, filter).with_context(|| format!("{stem}: the registry has no {dataset} filter {filter}"))?;
            Ok((entry.clone(), "given".to_string(), Vec::new()))
        }
        None => {
            let found = registry.identify(Some(header), &spec.path, spec.dataset.as_deref()).map_err(|error| anyhow!("{stem}: {error}"))?;
            let identified_by = match &found.source {
                FilterSource::Header { keyword } => format!("header {keyword}"),
                FilterSource::FileName => "file name".to_string(),
            };
            Ok((found.entry, identified_by, found.warnings))
        }
    }
}

struct LoadedCubeInput {
    input: CubeInput,
    path: String,
    identified_by: String,
    has_err: bool,
    has_dq: bool,
    warnings: Vec<String>,
}

fn load_cube_input(spec: &CubeInputSpec, registry: &FilterRegistry, layout: CubeLayout) -> Result<LoadedCubeInput> {
    let image = LoadedImage::load(&spec.path)?;
    let (entry, identified_by, mut warnings) = identify_input(spec, &image.header, registry)?;
    let (companions, companion_warnings) = load_preparation_companions(&spec.path, &image.entry)?;
    warnings.extend(companion_warnings);
    let (has_err, has_dq) = (companions.err.is_some(), companions.dq.is_some());
    let (err, dq) = match layout {
        CubeLayout::Lightstack => (None, None),
        CubeLayout::Mef => {
            let err = companions.err.as_ref().map(|plane| plane.arr().to_owned());
            let dq = match companions.dq.as_ref() {
                Some((plane, _)) => Some(plane.int_plane().context("DQ companion has no integer plane")?.clone()),
                None => None,
            };
            (err, dq)
        }
    };
    let plane = CubePlane::from_entry(&entry, &image.header, &image.wcs);
    let input = CubeInput {
        label: output_stem(&spec.path),
        sci: image.entry.arr().to_owned(),
        err,
        dq,
        header: image.header,
        wcs: image.wcs,
        plane,
    };
    Ok(LoadedCubeInput { input, path: spec.path.clone(), identified_by, has_err, has_dq, warnings })
}

fn input_noise(path: &str) -> Result<Option<InputNoise>> {
    let reference = ImageRef::parse(path);
    let (fits_path, _tmp) = resolve_single_image(&reference.path)?;
    let Some((header, mut maps)) = noise_extension(&fits_path)? else {
        return Ok(None);
    };
    if header.get_i64("NAXIS") != Some(2) || maps.len() != 1 {
        bail!("the {NOISCORR_EXTNAME} extension holds {} maps; an aligned image holds one 2-D map", maps.len());
    }
    let correlation = maps.remove(0);
    let size = correlation.nrows();
    if correlation.ncols() != size || size % 2 == 0 || header.get_i64(CARD_NCORRK) != Some(size as i64) {
        bail!(
            "the {NOISCORR_EXTNAME} map is {}x{size} with {CARD_NCORRK}={}; it must be square with an odd size equal to {CARD_NCORRK}",
            correlation.ncols(),
            header.get(CARD_NCORRK).unwrap_or("absent")
        );
    }
    let declared = card_text(&header, CARD_NCORRSRC).unwrap_or_default();
    if declared != NOISCORR_SOURCE_ALIGN {
        bail!("the {NOISCORR_EXTNAME} map has {CARD_NCORRSRC}='{declared}'; a cube is built from '{NOISCORR_SOURCE_ALIGN}' maps");
    }
    let centre = correlation[[size / 2, size / 2]];
    if (f64::from(centre) - 1.0).abs() > NOISE_CENTRE_TOLERANCE || correlation.iter().any(|value| !value.is_finite()) {
        bail!("the {NOISCORR_EXTNAME} map is not a normalised finite autocorrelation: its centre is {centre}");
    }
    Ok(Some(InputNoise { correlation, noiscorr_align: header.get_f64(CARD_NOISCORR_ALIGN).unwrap_or(f64::NAN) }))
}

fn reference_index(loaded: &[LoadedCubeInput], reference: Option<&str>) -> Result<usize> {
    let Some(wanted) = reference.map(str::trim).filter(|wanted| !wanted.is_empty()) else {
        return Ok(0);
    };
    let normalized = |path: &str| path.replace('\\', "/");
    loaded
        .iter()
        .position(|item| item.input.plane.filter.eq_ignore_ascii_case(wanted))
        .or_else(|| loaded.iter().position(|item| normalized(&item.path) == normalized(wanted)))
        .with_context(|| format!("no input matches the reference {wanted}"))
}

fn lightstack_drop_warning(loaded: &[LoadedCubeInput]) -> Option<String> {
    let carried: Vec<&str> = [(loaded.iter().any(|item| item.has_err), "ERR"), (loaded.iter().any(|item| item.has_dq), "DQ")]
        .into_iter()
        .filter(|(any, _)| *any)
        .map(|(_, name)| name)
        .collect();
    (!carried.is_empty()).then(|| {
        format!(
            "the lightstack layout holds the science planes only: the {} planes of the inputs are not written; the mef layout keeps them",
            carried.join(" and ")
        )
    })
}

pub(crate) fn build_cube(
    specs: &[CubeInputSpec],
    output_path: &str,
    output_dir: Option<&str>,
    options: CubeOptions,
    reference: Option<&str>,
) -> Result<CubeReport> {
    build_cube_with(specs, output_path, output_dir, options, reference, &|| false)
}

pub(crate) fn build_cube_with(
    specs: &[CubeInputSpec],
    output_path: &str,
    output_dir: Option<&str>,
    options: CubeOptions,
    reference: Option<&str>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<CubeReport> {
    let started = Instant::now();
    let first = specs.first().context("no image to build a cube from")?;
    let sizes = specs
        .iter()
        .map(|spec| cached_header(&GLOBAL_IMAGE_CACHE, &spec.path).and_then(|header| declared_dims(&spec.path, &header)))
        .collect::<Result<Vec<_>>>()?;
    let per_pixel = match options.layout {
        CubeLayout::Lightstack => SCIENCE_BYTES_PER_PIXEL,
        CubeLayout::Mef => INPUT_BYTES_PER_PIXEL,
    };
    let required = sizes.iter().fold(0u64, |total, &(rows, cols)| total.saturating_add(pixels(rows, cols).saturating_mul(per_pixel)));
    let (rows, cols) = sizes.iter().copied().max_by_key(|&(rows, cols)| pixels(rows, cols)).unwrap_or_default();
    let subject = format!("Building a {cols}x{rows} cube of {} {}", specs.len(), if specs.len() == 1 { "plane" } else { "planes" });
    let memory = check_preparation_memory(required, effective_memory(), &subject, CROP_FIRST_ADVICE)?;
    let registry = filter_registry();
    let loaded = specs
        .iter()
        .map(|spec| {
            check_cancelled(cancelled)?;
            load_cube_input(spec, &registry, options.layout)
        })
        .collect::<Result<Vec<_>>>()?;
    let reference = reference_index(&loaded, reference)?;
    let base = derived_output_header(Some(&cube_base_header(&loaded[reference].input.header)), ABPROC_CUBE, OutputValues::Linear);
    let mut warnings: Vec<String> = memory.into_iter().collect();
    warnings.extend(loaded.iter().flat_map(|item| item.warnings.iter().map(move |warning| format!("{}: {warning}", item.input.label))));
    let described: Vec<(String, String, bool, bool)> =
        loaded.iter().map(|item| (item.path.clone(), item.identified_by.clone(), item.has_err, item.has_dq)).collect();
    let dropped = match options.layout {
        CubeLayout::Lightstack => lightstack_drop_warning(&loaded),
        CubeLayout::Mef => None,
    };
    let inputs: Vec<CubeInput> = loaded.into_iter().map(|item| item.input).collect();
    let assembled = match options.layout {
        CubeLayout::Lightstack => assemble_cube(inputs, reference, base, options)?,
        CubeLayout::Mef => {
            let noise: Vec<Option<InputNoise>> = specs
                .iter()
                .zip(&inputs)
                .map(|(spec, input)| {
                    input_noise(&spec.path).unwrap_or_else(|error| {
                        warnings.push(format!("{}: its {NOISCORR_EXTNAME} map was not used: {error:#}", input.label));
                        None
                    })
                })
                .collect();
            let without: Vec<&str> = inputs.iter().zip(&noise).filter(|(_, noise)| noise.is_none()).map(|(input, _)| input.label.as_str()).collect();
            if without.is_empty() {
                assemble_cube_with_noise(inputs, noise.into_iter().flatten().collect(), reference, base, options)?
            } else {
                if without.len() < inputs.len() {
                    warnings.push(format!(
                        "the cube carries no {NOISCORR_EXTNAME} noise correlation maps: {} {} none, and the maps are kept only when every input has one",
                        without.join(", "),
                        if without.len() == 1 { "has" } else { "have" }
                    ));
                }
                assemble_cube(inputs, reference, base, options)?
            }
        }
    };
    let destination = resolve_output_target(&first.path, output_path, output_dir, DEFAULT_CUBE_NAME)?;
    for spec in specs {
        refuse_source_as_target(&destination, &spec.path)?;
    }
    let table = assembled.filters_table()?;
    check_cancelled(cancelled)?;
    let (hdus, write_warnings) = write_prepared_cube(&destination, &assembled.cube, options.layout, ABPROC_CUBE, table, assembled.noise.as_ref())?;
    warnings.extend(dropped);
    warnings.extend(assembled.warnings);
    warnings.extend(write_warnings);
    let planes = assembled
        .order
        .iter()
        .zip(&assembled.cube.filters)
        .map(|(&i, plane)| {
            let (path, identified_by, has_err, has_dq) = described[i].clone();
            CubePlaneReport { path, plane: plane.clone(), identified_by, has_err, has_dq }
        })
        .collect();
    let (rows, cols) = assembled.cube.dims();
    Ok(CubeReport {
        output_path: destination,
        layout: options.layout,
        order: options.order,
        reference_path: described[reference].0.clone(),
        rows,
        cols,
        bunit: assembled.bunit,
        hdus,
        planes,
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}


fn masked_dq_bits(plane: &CubePlane) -> u32 {
    padded_dq_bits(dq_table_of(plane)).unwrap_or(DQ_DO_NOT_USE)
}

fn plane_validity(cube: &Cube, threshold: f64, use_dq: bool, kept: &[CubePlane]) -> Vec<PlaneValidity> {
    let dq = cube.dq.as_ref().filter(|_| use_dq);
    cube.planes
        .iter()
        .zip(&cube.filters)
        .enumerate()
        .map(|(z, (plane, filter))| {
            let mask = dq.map(|_| match dq_table_of(filter).exclusion_mask() {
                0 => DQ_DO_NOT_USE,
                mask => mask,
            });
            let flagged = |index: (usize, usize)| dq.zip(mask).is_some_and(|(dq, mask)| dq.planes[z][index] & mask != 0);
            let (usable, above) = plane
                .indexed_iter()
                .filter(|&(index, value)| value.is_finite() && !flagged(index))
                .fold((0, 0), |(usable, above), (_, &value)| (usable + 1, above + usize::from(f64::from(value) > threshold)));
            PlaneValidity {
                filter: filter.filter.clone(),
                pixels: plane.len(),
                usable,
                above,
                dq_mask: mask,
                kept: kept.iter().any(|plane| plane.filter == filter.filter),
            }
        })
        .collect()
}

pub(crate) fn edit_cube(
    path: &str,
    edit: &CubeEdit,
    output_path: &str,
    output_dir: Option<&str>,
    layout: Option<CubeLayout>,
) -> Result<CubeEditReport> {
    edit_cube_with(path, edit, output_path, output_dir, layout, &|| false)
}

pub(crate) fn edit_cube_with(
    path: &str,
    edit: &CubeEdit,
    output_path: &str,
    output_dir: Option<&str>,
    layout: Option<CubeLayout>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<CubeEditReport> {
    let started = Instant::now();
    let CubeFile { cube, layout: source_layout, noise, statuses, warnings: read_warnings } = read_cube_file(path, &filter_registry())?;
    check_cancelled(cancelled)?;
    let mut warnings = read_warnings;
    let mut removed = Vec::new();
    let mut masked_pixels = None;
    let mut centre = None;
    let mut validity = None;
    let named_statuses: Option<Vec<(String, MatchStatus)>> =
        statuses.map(|statuses| cube.filters.iter().map(|plane| plane.filter.clone()).zip(statuses).collect());
    let mut edited = match edit {
        CubeEdit::DropInvalid { threshold, frac_valid, use_dq } => {
            if *use_dq && cube.dq.is_none() {
                warnings.push("use_dq was asked but the cube has no DQ planes: every finite pixel counts".to_string());
            }
            let (valid, dropped) = drop_invalid_planes(&cube, *threshold, *frac_valid, *use_dq)?;
            validity = Some(plane_validity(&cube, *threshold, *use_dq, &valid.filters));
            removed = dropped;
            valid
        }
        CubeEdit::RemoveFilter { filter } => {
            let kept = remove_filter(&cube, filter)?;
            removed = cube.filters.iter().filter(|plane| !kept.filters.contains(plane)).map(|plane| plane.filter.clone()).collect();
            kept
        }
        CubeEdit::Cut { x_start, x_end, y_start, y_end } => subcube(&cube, *x_start, *x_end, *y_start, *y_end)?,
        CubeEdit::CutSky { ra, dec, size_arcsec, rounding } => {
            let (x_start, x_end, y_start, y_end) = sky_box(&cube, *ra, *dec, *size_arcsec, *rounding, &mut warnings)?;
            subcube(&cube, x_start, x_end, y_start, y_end)?
        }
        CubeEdit::Header { redshift, ra_center, dec_center, use_brightest_pixel } => {
            let mut updated = cube;
            centre = Some(update_metadata(&mut updated, *redshift, ra_center.zip(*dec_center), *use_brightest_pixel)?);
            updated
        }
        CubeEdit::ReferenceNanMask { reference_slice, mode } => {
            let mut updated = cube;
            let flags: Vec<u32> = updated.filters.iter().map(masked_dq_bits).collect();
            masked_pixels = Some(apply_nan_mask(&mut updated, *mode, *reference_slice, &flags)?);
            updated
        }
    };
    let noise = noise.map(|noise| noise.for_planes(&edited.filters)).transpose()?;
    let statuses: Option<Vec<MatchStatus>> = named_statuses.map(|named| {
        named.into_iter().filter(|(filter, _)| edited.filters.iter().any(|plane| plane.filter == *filter)).map(|(_, status)| status).collect()
    });
    let layout = layout.unwrap_or(source_layout);
    let file_name = format!("{}_{}.fits", output_stem(path), edit.suffix());
    let destination = resolve_output_target(path, output_path, output_dir, &file_name)?;
    edited.header = cube_output_header(&edited.header, ABPROC_CUBE_EDIT);
    let table = filters_table(&edited.filters, statuses.as_deref(), noise.as_ref())?;
    check_cancelled(cancelled)?;
    let (hdus, write_warnings) = write_prepared_cube(&destination, &edited, layout, ABPROC_CUBE_EDIT, table, noise.as_ref())?;
    warnings.extend(write_warnings);
    let (rows, cols) = edited.dims();
    Ok(CubeEditReport {
        output_path: destination,
        layout,
        edit: edit.clone(),
        filters: edited.filters.iter().map(|plane| plane.filter.clone()).collect(),
        removed,
        rows,
        cols,
        masked_pixels,
        centre,
        validity,
        hdus,
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

fn sky_box(cube: &Cube, ra: f64, dec: f64, size_arcsec: f64, rounding: CropRounding, warnings: &mut Vec<String>) -> Result<(i64, i64, i64, i64)> {
    let wcs = cube.wcs()?;
    let (rows, cols) = cube.dims();
    let frame = CropFrame { header: &cube.header, wcs: &wcs, dims: (cols, rows) };
    let resolved = resolve_crop_shape(&CropRegion::Radec { ra, dec, size_arcsec }, rounding, &[frame], None, None)?;
    warnings.extend(resolved.warnings);
    let rect = resolved.shape.rect(&frame)?;
    let (x_end, y_end) = (rect.x0 + rect.width as i64, rect.y0 + rect.height as i64);
    let clipped = (rect.x0.max(0), x_end.min(cols as i64), rect.y0.max(0), y_end.min(rows as i64));
    if clipped != (rect.x0, x_end, rect.y0, y_end) {
        warnings.push(format!(
            "the sky box covers x {}:{}, y {}:{} and was clipped to the {cols}x{rows} cube",
            rect.x0, x_end, rect.y0, y_end
        ));
    }
    Ok(clipped)
}


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PsfInputSpec {
    pub path: String,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub pixel_scale_arcsec: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreparedPsfFile {
    pub path: String,
    pub filter: Option<String>,
    pub hdu: usize,
    pub extname: Option<String>,
    pub source_pixel_scale_arcsec: f64,
    pub scale_source: ScaleSource,
    pub oversampling: Option<f64>,
    pub pixel_scale_arcsec: f64,
    pub rows: usize,
    pub cols: usize,
    pub before: PsfDiagnostics,
    pub after: PsfDiagnostics,
    pub padded_to_odd: bool,
    pub recentred_by_px: Option<[f64; 2]>,
    pub resampled: Option<ResampleRecord>,
    pub clipped_negative_fraction: f64,
    pub clipped_negative_flux_fraction: f64,
    pub output_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PsfSetReport {
    pub reference_index: Option<usize>,
    pub target_pixel_scale_arcsec: Option<f64>,
    pub target_rows: Option<usize>,
    pub target_cols: Option<usize>,
    pub method: PsfResampleMethod,
    pub psfs: Vec<PreparedPsfFile>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

pub(crate) fn load_psfs(inputs: &[PsfInputSpec], options: &PsfSetOptions, output_dir: Option<&str>) -> Result<PsfSetReport> {
    load_psfs_with(inputs, options, output_dir, &|| false)
}

pub(crate) fn load_psfs_with(
    inputs: &[PsfInputSpec],
    options: &PsfSetOptions,
    output_dir: Option<&str>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<PsfSetReport> {
    let started = Instant::now();
    if inputs.is_empty() {
        bail!("no PSF to load");
    }
    let psfs = inputs
        .iter()
        .map(|spec| {
            check_cancelled(cancelled)?;
            let mut psf = load_psf(std::path::Path::new(&spec.path), spec.pixel_scale_arcsec)?;
            if let Some(filter) = spec.filter.as_deref().map(str::trim).filter(|filter| !filter.is_empty()) {
                psf.filter = Some(filter.to_string());
            }
            Ok(psf)
        })
        .collect::<Result<Vec<_>>>()?;
    let set = prepare_psf_set(&psfs, options)?;
    let targets = match output_dir {
        Some(dir) => {
            let targets = inputs
                .iter()
                .map(|spec| resolve_output_target(&spec.path, "", Some(dir), &format!("{}_prepared.fits", output_stem(&spec.path))).map(Some))
                .collect::<Result<Vec<_>>>()?;
            if let Some(duplicate) = targets.iter().enumerate().find(|(i, target)| targets[..*i].contains(target)).map(|(_, target)| target) {
                bail!("two PSFs would be written to {}; give the inputs distinct file names", duplicate.as_deref().unwrap_or_default());
            }
            targets
        }
        None => vec![None; inputs.len()],
    };
    let mut warnings = Vec::new();
    let mut files = Vec::with_capacity(inputs.len());
    for (((spec, psf), prepared), target) in inputs.iter().zip(&psfs).zip(&set.prepared).zip(targets) {
        let stem = output_stem(&spec.path);
        if let Some(target) = &target {
            check_cancelled(cancelled)?;
            let source_name = std::path::Path::new(&spec.path).file_name().map_or_else(|| spec.path.clone(), |name| name.to_string_lossy().into_owned());
            write_fits_f64(target, &prepared.data, Some(&prepared_psf_header(psf, prepared, &source_name)))?;
        }
        warnings.extend(prepared.warnings.iter().map(|warning| format!("{stem}: {warning}")));
        let (rows, cols) = prepared.data.dim();
        files.push(PreparedPsfFile {
            path: spec.path.clone(),
            filter: psf.filter.clone(),
            hdu: psf.hdu,
            extname: psf.extname.clone(),
            source_pixel_scale_arcsec: psf.pixel_scale_arcsec,
            scale_source: psf.scale_source,
            oversampling: psf.oversampling,
            pixel_scale_arcsec: prepared.pixel_scale_arcsec,
            rows,
            cols,
            before: prepared.before.clone(),
            after: prepared.after.clone(),
            padded_to_odd: prepared.padded_to_odd,
            recentred_by_px: prepared.recentred_by_px,
            resampled: prepared.resampled.clone(),
            clipped_negative_fraction: prepared.clipped_negative_fraction,
            clipped_negative_flux_fraction: prepared.clipped_negative_flux_fraction,
            output_path: target,
        });
    }
    Ok(PsfSetReport {
        reference_index: set.reference_index,
        target_pixel_scale_arcsec: set.target_pixel_scale_arcsec,
        target_rows: set.target_shape.map(|shape| shape.0),
        target_cols: set.target_shape.map(|shape| shape.1),
        method: options.method,
        psfs: files,
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}


#[derive(Debug, Clone, Serialize)]
pub struct KernelFile {
    pub path: String,
    pub filter: String,
    pub source_label: String,
    pub method: KernelMethod,
    pub eps_absolute: Option<f64>,
    pub auto: Option<AutoChoice>,
    pub diagnostics: KernelDiagnostics,
    pub output_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KernelSetReport {
    pub reference: String,
    pub reference_path: String,
    pub target_label: String,
    pub pixel_scale_arcsec: f64,
    pub kernels: Vec<KernelFile>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

fn psf_label(psf: &LoadedPsf, filter: &str) -> String {
    match card_text(&psf.header, "DATASET").or_else(|| card_text(&psf.header, "TELESCOP")) {
        Some(dataset) => format!("{}_{filter}", dataset.to_uppercase()),
        None => filter.to_string(),
    }
}

fn kernel_failure(error: KernelError, filter: &str, reference: &str, source: (usize, usize), method: KernelMethod) -> anyhow::Error {
    let context = format!("kernel from {filter} to {reference}");
    let (message, remedy) = match error {
        KernelError::KernelShape { rows, cols, needed_rows, needed_cols } => match method {
            KernelMethod::Wiener { shape: KernelShape::Named(ShapeName::Reference), .. } => (
                format!(
                    "{context}: the {filter} PSF is {}x{} and the reference PSF {cols}x{rows}; shape \"reference\" makes the kernel {cols}x{rows}, the size of the reference PSF, so it cannot hold the larger PSF",
                    source.1, source.0
                ),
                format!(
                    "Give the kernels step \"shape\": \"max\", which takes the larger odd size ({needed_cols}x{needed_rows}), or an odd fixed shape of at least {needed_cols}x{needed_rows}, e.g. \"shape\": [{needed_rows}, {needed_cols}]; or give the psfs step pixel_scale_arcsec with reference, which puts every PSF on the reference grid and footprint (a larger PSF is cropped to it and renormalised, so its outer flux is dropped)"
                ),
            ),
            _ => (
                format!("{context}: {error}"),
                format!("Give the kernels step an odd shape of at least {needed_cols}x{needed_rows}, e.g. \"shape\": [{needed_rows}, {needed_cols}]"),
            ),
        },
        KernelError::ShapeMismatch { .. } => (
            format!("{context}: {error}"),
            "Give the psfs step pixel_scale_arcsec with reference, which puts every PSF on the reference grid and footprint, or use the wiener method with \"shape\": \"max\"".to_string(),
        ),
        other => return anyhow::Error::new(other).context(context),
    };
    CodedError { code: KERNEL_SHAPE, message: format!("{message}. {remedy}"), hint: Some(remedy) }.into()
}

pub(crate) fn build_kernels(inputs: &[PsfInputSpec], reference: &str, options: &KernelOptions, output_dir: Option<&str>) -> Result<KernelSetReport> {
    build_kernels_with(inputs, reference, options, output_dir, &|| false)
}

pub(crate) fn build_kernels_with(
    inputs: &[PsfInputSpec],
    reference: &str,
    options: &KernelOptions,
    output_dir: Option<&str>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<KernelSetReport> {
    let started = Instant::now();
    if inputs.is_empty() {
        bail!("no PSF to build kernels from");
    }
    let loaded = inputs
        .iter()
        .map(|spec| {
            check_cancelled(cancelled)?;
            let psf = load_psf(std::path::Path::new(&spec.path), spec.pixel_scale_arcsec)?;
            let filter = spec
                .filter
                .as_deref()
                .map(str::trim)
                .filter(|filter| !filter.is_empty())
                .map(str::to_string)
                .or_else(|| psf.filter.clone())
                .ok_or_else(|| anyhow!("{} has no FILTER card; give its filter", spec.path))?;
            Ok((spec, filter, psf))
        })
        .collect::<Result<Vec<_>>>()?;
    let wanted = reference.trim();
    let matches: Vec<usize> = (0..loaded.len()).filter(|&i| loaded[i].1.eq_ignore_ascii_case(wanted)).collect();
    let target_index = match matches.as_slice() {
        [index] => *index,
        [] => bail!(PsfError::NoReference(wanted.to_string())),
        several => bail!(PsfError::AmbiguousReference { filter: wanted.to_string(), count: several.len() }),
    };
    let (target_spec, reference, target) = &loaded[target_index];
    let target_label = psf_label(target, reference);
    let mut warnings: Vec<String> = target.warnings.iter().map(|warning| format!("{}: {warning}", output_stem(&target_spec.path))).collect();
    let mut built = Vec::new();
    for (i, (spec, filter, psf)) in loaded.iter().enumerate().filter(|(i, _)| *i != target_index) {
        if let Some((first, _, _)) = loaded[..i].iter().find(|(_, other, _)| other.eq_ignore_ascii_case(filter)) {
            bail!("{} and {} both hold filter {filter}; a cube plane takes one kernel per filter", first.path, spec.path);
        }
        if (psf.pixel_scale_arcsec / target.pixel_scale_arcsec - 1.0).abs() > SCALE_RTOL {
            bail!(
                "{} is sampled at {} arcsec per pixel and the reference PSF {} at {}; put the PSFs on one grid first with load_psfs",
                spec.path,
                psf.pixel_scale_arcsec,
                target_spec.path,
                target.pixel_scale_arcsec
            );
        }
        check_cancelled(cancelled)?;
        let kernel = build_kernel(&psf.data, &target.data, options).map_err(|error| kernel_failure(error, filter, reference, psf.data.dim(), options.method))?;
        warnings.extend(psf.warnings.iter().map(|warning| format!("{}: {warning}", output_stem(&spec.path))));
        warnings.extend(kernel.diagnostics.warnings.iter().map(|warning| format!("{filter}: {warning}")));
        built.push((spec, filter, psf_label(psf, filter), kernel));
    }
    let mut kernels = Vec::with_capacity(built.len());
    for (spec, filter, source_label, kernel) in built {
        let output_path = match output_dir {
            Some(dir) => {
                check_cancelled(cancelled)?;
                let destination = resolve_output_target(&spec.path, "", Some(dir), &kernel_file_name(filter, reference))?;
                let header = kernel_header(&kernel, &source_label, &target_label, Some(target.pixel_scale_arcsec));
                write_fits_f64(&destination, &kernel.data, Some(&header))?;
                Some(destination)
            }
            None => None,
        };
        kernels.push(KernelFile {
            path: spec.path.clone(),
            filter: filter.clone(),
            source_label,
            method: kernel.method,
            eps_absolute: kernel.eps_absolute,
            auto: kernel.auto,
            diagnostics: kernel.diagnostics,
            output_path,
        });
    }
    Ok(KernelSetReport {
        reference: reference.clone(),
        reference_path: target_spec.path.clone(),
        target_label,
        pixel_scale_arcsec: target.pixel_scale_arcsec,
        kernels,
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelFileSpec {
    pub filter: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PsfMatchReport {
    pub output_path: String,
    pub layout: CubeLayout,
    pub mode: MatchMode,
    pub reference: String,
    pub planes: Vec<MatchedPlane>,
    pub rows: usize,
    pub cols: usize,
    pub hdus: Vec<String>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

pub(crate) fn psf_match_cube(
    path: &str,
    kernel_dir: Option<&str>,
    kernels: &[KernelFileSpec],
    options: &MatchOptions,
    output_path: &str,
    output_dir: Option<&str>,
    layout: Option<CubeLayout>,
) -> Result<PsfMatchReport> {
    psf_match_cube_with(path, kernel_dir, kernels, options, output_path, output_dir, layout, &|| false)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn psf_match_cube_with(
    path: &str,
    kernel_dir: Option<&str>,
    kernels: &[KernelFileSpec],
    options: &MatchOptions,
    output_path: &str,
    output_dir: Option<&str>,
    layout: Option<CubeLayout>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<PsfMatchReport> {
    let started = Instant::now();
    let CubeFile { cube, layout: source_layout, noise, mut warnings, .. } = read_cube_file(path, &filter_registry())?;
    check_cancelled(cancelled)?;
    let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    warnings.extend(
        kernels
            .iter()
            .filter(|spec| !cube.filters.iter().any(|plane| same(&plane.filter, &spec.filter)))
            .map(|spec| format!("the kernel {} is for {}, which is not a plane of the cube", spec.path, spec.filter)),
    );
    let mut plane_kernels = Vec::new();
    let mut cards = Vec::new();
    for plane in cube.filters.iter().filter(|plane| !same(&plane.filter, &options.reference)) {
        let given = kernels.iter().find(|spec| same(&spec.filter, &plane.filter)).map(|spec| spec.path.clone());
        let named = kernel_dir
            .map(|dir| std::path::Path::new(dir).join(kernel_file_name(&plane.filter, &options.reference)))
            .filter(|candidate| candidate.is_file())
            .map(|candidate| candidate.to_string_lossy().into_owned());
        if let Some(kernel_path) = given.or(named) {
            check_cancelled(cancelled)?;
            let (kernel, kernel_cards) = read_kernel_file(&kernel_path, &plane.filter, &mut warnings)?;
            plane_kernels.push(kernel);
            cards.push(kernel_cards);
        }
    }
    let flags: Vec<u32> = cube.filters.iter().map(masked_dq_bits).collect();
    let MatchProducts { matched: MatchedCube { cube: mut matched, planes, warnings: match_warnings }, noise, statuses } =
        match_cube_with(&cube, &plane_kernels, &cards, options, &flags, noise.as_ref(), effective_memory(), cancelled).map_err(|error| match error {
            MatchError::Cancelled => anyhow::Error::from(AppError::Cancelled),
            other => other.into(),
        })?;
    warnings.extend(match_warnings);
    let layout = layout.unwrap_or(source_layout);
    let destination = resolve_output_target(path, output_path, output_dir, &format!("{}_psfmatched.fits", output_stem(path)))?;
    matched.header = cube_output_header(&matched.header, ABPROC_PSF_MATCHED);
    let table = filters_table(&matched.filters, Some(statuses.as_slice()), noise.as_ref())?;
    check_cancelled(cancelled)?;
    let (hdus, write_warnings) = write_prepared_cube(&destination, &matched, layout, ABPROC_PSF_MATCHED, table, noise.as_ref())?;
    warnings.extend(write_warnings);
    let (rows, cols) = matched.dims();
    Ok(PsfMatchReport {
        output_path: destination,
        layout,
        mode: options.mode,
        reference: options.reference.trim().to_string(),
        planes,
        rows,
        cols,
        hdus,
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

fn read_kernel_file(path: &str, filter: &str, warnings: &mut Vec<String>) -> Result<(PlaneKernel, KernelCards)> {
    let bytes = std::fs::read(path).with_context(|| format!("Failed to read the kernel {path}"))?;
    let hdus = image_hdus(&bytes, path)?;
    let (hdu, parsed) = hdus.first().ok_or_else(|| PsfError::NoImage(path.to_string()))?;
    let mut planes = decode_planes_f64(&bytes, parsed, path)?;
    if planes.len() != 1 {
        bail!(PsfError::Cube { label: path.to_string(), hdu: *hdu, planes: planes.len() });
    }
    let label = std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |name| name.to_string_lossy().into_owned());
    let mut scale_warnings = Vec::new();
    let pixel_scale_arcsec = pixel_scale(&parsed.header, None, &mut scale_warnings)?.map(|(scale, _)| scale);
    warnings.extend(scale_warnings.into_iter().map(|warning| format!("{label}: {warning}")));
    let kernel = PlaneKernel { filter: filter.to_string(), label, data: planes.remove(0), pixel_scale_arcsec };
    Ok((kernel, KernelCards::read(&parsed.header)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cubeprep::match_cube::match_cube;
    use crate::core::metadata::photcal::card_text;
    use crate::core::metadata::units::{AreaRule, CARD_BUNIT_IN, CARD_FLUXCONV};
    use crate::infra::fits::reader::test_fixtures::{write_test_mef, HduData as TestData, TestHdu};
    use crate::infra::fits::reader::{extract_image_mmap_by_index, extract_int_plane_by_index};

    const COLS: usize = 4;
    const ROWS: usize = 3;
    const PIXAR_SR: f64 = 2.1e-13;

    fn i2d_like(dir: &tempfile::TempDir, bunit: &str) -> String {
        let path = dir.path().join("jw_f444w_i2d.fits");
        let sci: Vec<f32> = (0..COLS * ROWS).map(|i| i as f32 + 0.5).collect();
        let err: Vec<f32> = sci.iter().map(|value| value * 0.1).collect();
        let dq: Vec<i32> = (0..COLS * ROWS).map(|i| i32::from(i == 5)).collect();
        let unit = format!("'{bunit}'");
        write_test_mef(
            &path,
            &[("TELESCOP", "'JWST'".into())],
            &[
                TestHdu {
                    extname: Some("SCI"),
                    extver: Some(1),
                    cols: COLS,
                    rows: ROWS,
                    data: TestData::F32(sci),
                    extra_cards: vec![("BUNIT", unit.clone()), ("PIXAR_SR", format!("{PIXAR_SR:E}"))],
                },
                TestHdu { extname: Some("ERR"), extver: Some(1), cols: COLS, rows: ROWS, data: TestData::F32(err), extra_cards: vec![("BUNIT", unit)] },
                TestHdu { extname: Some("DQ"), extver: Some(1), cols: COLS, rows: ROWS, data: TestData::I32(dq), extra_cards: vec![] },
            ],
        );
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn convert_units_writes_sci_err_and_dq_in_jy_per_pixel() {
        let dir = tempfile::tempdir().unwrap();
        let source = i2d_like(&dir, "MJy/sr");
        let out = dir.path().join("out");
        let report = convert_units_file(&source, TargetUnit::JyPerPixel, "", Some(out.to_str().unwrap())).unwrap();
        let factor = 1e6 * PIXAR_SR;
        assert!(report.output_path.ends_with("jw_f444w_i2d_Jy.fits"), "{}", report.output_path);
        assert_eq!(report.hdus, ["SCI", "ERR", "DQ"]);
        assert_eq!((report.bunit_in.as_deref(), report.bunit_out.as_str(), report.factor), (Some("MJy/sr"), "Jy", factor));
        assert_eq!(report.warnings, [
            "ERR has no celestial WCS: same-index alignment to SCI is assumed from matching dimensions",
            "DQ has no celestial WCS: same-index alignment to SCI is assumed from matching dimensions",
        ]);

        let file = std::fs::File::open(&report.output_path).unwrap();
        let sci = extract_image_mmap_by_index(&file, 1).unwrap();
        assert_eq!(sci.image[[1, 2]], (6.5 * factor) as f32);
        assert_eq!(card_text(&sci.header, "BUNIT").as_deref(), Some("Jy"));
        assert_eq!(card_text(&sci.header, CARD_BUNIT_IN).as_deref(), Some("MJy/sr"));
        assert!((sci.header.get_f64(CARD_FLUXCONV).unwrap() / factor - 1.0).abs() < 1e-13);
        assert!(sci.header.cards.iter().any(|(key, value)| key == "HISTORY" && value.contains("MJy/sr")), "{:?}", sci.header.cards);
        let err = extract_image_mmap_by_index(&file, 2).unwrap();
        assert_eq!(err.image[[1, 2]], (f64::from(6.5f32 * 0.1) * factor) as f32);
        assert_eq!(card_text(&err.header, "BUNIT").as_deref(), Some("Jy"));
        let dq = extract_int_plane_by_index(&file, 3).unwrap();
        assert_eq!(dq.bits[[1, 1]], 1);
        assert_eq!(dq.bits.iter().filter(|bits| **bits != 0).count(), 1);
    }

    #[test]
    fn convert_units_refuses_an_unknown_unit_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let source = i2d_like(&dir, "ADU");
        let out = dir.path().join("out");
        let err = convert_units_file(&source, TargetUnit::JyPerPixel, "", Some(out.to_str().unwrap())).unwrap_err().to_string();
        assert!(err.contains("'ADU'"), "{err}");
        assert!(!out.join("jw_f444w_i2d_Jy.fits").exists());
    }

    fn wcs_cards(crpix: (f64, f64), scale_arcsec: f64, rotation_deg: f64) -> Vec<(&'static str, String)> {
        let (sin, cos) = rotation_deg.to_radians().sin_cos();
        let s = scale_arcsec / 3600.0;
        vec![
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
            ("CRPIX1", format!("{:E}", crpix.0)),
            ("CRPIX2", format!("{:E}", crpix.1)),
            ("CRVAL1", "150.0".into()),
            ("CRVAL2", "2.0".into()),
            ("CD1_1", format!("{:E}", -s * cos)),
            ("CD1_2", format!("{:E}", -s * sin)),
            ("CD2_1", format!("{:E}", -s * sin)),
            ("CD2_2", format!("{:E}", s * cos)),
        ]
    }

    fn sky_of(path: &str, hdu: usize, x: f64, y: f64) -> (f64, f64) {
        let file = std::fs::File::open(path).unwrap();
        let header = crate::infra::fits::reader::extract_header_by_index(&file, hdu).unwrap();
        let sky = crate::infra::wcs_source::header_wcs(&header).unwrap().pixel_to_world(x, y);
        (sky.ra, sky.dec)
    }

    fn files_in(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir).map(|entries| entries.count()).unwrap_or(0)
    }

    #[test]
    fn batch_crop_carries_err_dq_area_for_each_file() {
        use crate::service::area::test_fixtures::{area_hdu, blocked_area, cal_sci_cards, jwst_cal_mef, square, CalPlanes, CAL_SIZE};
        use crate::core::cubeprep::footprint::{lightstack_square_rect, lightstack_wcs, SkyPoint};
        use crate::core::imaging::cutout::{cut_int_plane, cut_plane};
        use crate::infra::fits::reader::{extract_header_by_index, list_extensions};

        let dir = tempfile::tempdir().unwrap();
        let mut sources = Vec::new();
        for (name, crpix, rotation) in [("north.fits", (32.5, 32.5), 0.0), ("turned.fits", (30.5, 33.5), 20.0)] {
            let mut cards = cal_sci_cards();
            cards.extend(wcs_cards(crpix, 0.1, rotation));
            sources.push(jwst_cal_mef(&dir.path().join(name), &[], cards, CalPlanes::ALL, vec![area_hdu(blocked_area(), CAL_SIZE, Some(1))]));
        }
        let paths: Vec<String> = sources.iter().map(|source| source.key.clone()).collect();
        let out = dir.path().join("crops");
        let report = crop_set(&paths, &CropRegion::Common, CropRounding::Astroburst, out.to_str().unwrap(), true, true).unwrap();
        assert_eq!(report.files.len(), 2);
        assert!(matches!(report.shape, CropShape::Polygon { .. }));
        assert!(report.area_arcsec2.is_some_and(|area| area > 20.0 && area < 6.4 * 6.4), "{:?}", report.area_arcsec2);
        for (source, cropped) in sources.iter().zip(&report.files) {
            let cut = &cropped.cutout;
            assert_eq!(cropped.path, source.key);
            assert_eq!(cut.hdus, ["SCI", "ERR", "DQ", "AREA"], "{:?}", cut.warnings);
            assert_eq!(cut.fraction_on_image, 1.0);
            let file = std::fs::File::open(&cut.output_path).unwrap();
            let exts = list_extensions(&file).unwrap();
            let plane = |name: &str| exts.iter().find(|e| e.extname.as_deref() == Some(name)).unwrap().index;
            let image = |name: &str| extract_image_mmap_by_index(&file, plane(name)).unwrap().image;
            assert_eq!(image("SCI"), cut_plane(&source.sci, &cut.rect));
            assert_eq!(image("ERR"), cut_plane(&source.err, &cut.rect));
            assert_eq!(image("AREA"), cut_plane(&square(&blocked_area()), &cut.rect));
            let source_dq = extract_int_plane_by_index(&std::fs::File::open(&source.path).unwrap(), 3).unwrap().bits;
            assert_eq!(extract_int_plane_by_index(&file, plane("DQ")).unwrap().bits, cut_int_plane(&source_dq, &cut.rect, 0));
            let (ra, dec) = sky_of(&cut.output_path, plane("SCI"), 0.0, 0.0);
            let (ra0, dec0) = sky_of(&source.path, 1, cut.rect.x0 as f64, cut.rect.y0 as f64);
            assert!((ra - ra0).abs() < 1e-10 && (dec - dec0).abs() < 1e-10);
            assert_eq!(extract_header_by_index(&file, plane("DQ")).unwrap().get_f64("LTV1"), Some(-(cut.rect.x0 as f64)));
        }

        let radec = CropRegion::Radec { ra: 150.0, dec: 2.0, size_arcsec: 2.0 };
        let lightstack = crop_set(&paths, &radec, CropRounding::Lightstack, out.join("ls").to_str().unwrap(), true, true).unwrap();
        for (source, cropped) in sources.iter().zip(&lightstack.files) {
            let header = extract_header_by_index(&std::fs::File::open(&source.path).unwrap(), 1).unwrap();
            let expected = lightstack_square_rect(&lightstack_wcs(&header).unwrap(), (CAL_SIZE, CAL_SIZE), SkyPoint { ra: 150.0, dec: 2.0 }, 2.0).unwrap();
            assert_eq!(cropped.cutout.rect, expected);
            assert_eq!(cropped.cutout.hdus, ["SCI", "ERR", "DQ", "AREA"]);
        }

        let elsewhere = dir.path().join("refused");
        let far = CropRegion::Radec { ra: 150.0, dec: 2.0 + 5.0 / 3600.0, size_arcsec: 2.0 };
        let refused = crop_set(&paths, &far, CropRounding::Astroburst, elsewhere.to_str().unwrap(), true, true).unwrap_err().to_string();
        assert!(refused.contains("outside"), "{refused}");
        assert_eq!(files_in(&elsewhere), 0, "a refused batch writes nothing");
    }

    fn aligned_source(dir: &tempfile::TempDir, name: &str, size: usize, scale: f64, rotation: f64, cards: &[(&'static str, &str)]) -> String {
        let path = dir.path().join(name);
        let centre = size as f64 / 2.0 + 0.5;
        let mut extra = wcs_cards((centre, centre), scale, rotation);
        extra.extend(cards.iter().map(|(key, value)| (*key, value.to_string())));
        let pixels: Vec<f32> = (0..size * size)
            .map(|i| {
                let (x, y) = ((i % size) as f64 - size as f64 / 2.0, (i / size) as f64 - size as f64 / 2.0);
                (5.0 + 100.0 * (-(x * x + y * y) / (2.0 * (0.1 / scale).powi(2))).exp()) as f32
            })
            .collect();
        write_test_mef(
            &path,
            &[("TELESCOP", "'JWST'".into()), ("INSTRUME", "'NIRCAM'".into())],
            &[TestHdu { extname: Some("SCI"), extver: Some(1), cols: size, rows: size, data: TestData::F32(pixels), extra_cards: extra }],
        );
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn align_set_writes_each_input_on_the_reference_grid() {
        let dir = tempfile::tempdir().unwrap();
        let pixar = |scale: f64| format!("{:E}", (scale * std::f64::consts::PI / 648_000.0).powi(2));
        let (fine_pixar, coarse_pixar) = (pixar(0.02), pixar(0.04));
        let fine = aligned_source(&dir, "fine.fits", 64, 0.02, 10.0, &[("BUNIT", "'MJy/sr'"), ("FILTER", "'F150W'"), ("PIXAR_SR", &fine_pixar)]);
        let coarse = aligned_source(&dir, "coarse.fits", 32, 0.04, 0.0, &[("BUNIT", "'MJy/sr'"), ("FILTER", "'F444W'"), ("PIXAR_SR", &coarse_pixar)]);
        let out = dir.path().join("aligned");
        let report = align_set(&[fine.clone(), coarse.clone()], out.to_str().unwrap(), &AlignOptions::default()).unwrap();
        assert_eq!((report.reference_index, report.reference_rule, report.reference_path.as_str()), (1, "coarsest", coarse.as_str()));
        assert_eq!((report.method, report.border_crop, report.rows, report.cols), (ResampleMethod::Exact, 1, 30, 30));
        let [resampled, copied] = [&report.files[0], &report.files[1]];
        assert!(resampled.output_path.ends_with("fine_aligned_exact.fits"), "{}", resampled.output_path);
        assert!(resampled.report.reprojected && !copied.report.reprojected);
        assert_eq!(resampled.report.area.as_ref().map(|area| area.rule), Some(AreaRule::Keep));
        assert!(resampled.report.coverage > 0.2 && resampled.report.flux_ratio.is_some_and(|ratio| (ratio - 1.0).abs() < 0.01), "{:?}", resampled.report);
        for written in &report.files {
            let file = std::fs::File::open(&written.output_path).unwrap();
            let sci = extract_image_mmap_by_index(&file, 1).unwrap();
            assert_eq!(sci.image.dim(), (30, 30));
            assert_eq!((sci.header.get_f64("CRPIX1"), sci.header.get_f64("CRPIX2")), (Some(15.5), Some(15.5)));
            assert_eq!(card_text(&sci.header, "BUNIT").as_deref(), Some("MJy/sr"));
            assert!((sci.header.get_f64("PIXAR_SR").unwrap() / coarse_pixar.parse::<f64>().unwrap() - 1.0).abs() < 1e-9);
        }
        let fine_header = extract_image_mmap_by_index(&std::fs::File::open(&resampled.output_path).unwrap(), 1).unwrap().header;
        assert_eq!(card_text(&fine_header, "FILTER").as_deref(), Some("F150W"));

        let by_filter = AlignOptions { reference: ReferencePolicy::Filter { filter: "F150W".into() }, method: ResampleMethod::Bilinear, ..AlignOptions::default() };
        let fine_grid = align_set(&[fine.clone(), coarse.clone()], out.join("f150w").to_str().unwrap(), &by_filter).unwrap();
        assert_eq!((fine_grid.reference_index, fine_grid.reference_rule, fine_grid.rows), (0, "filter", 62));
        assert!(fine_grid.files[1].output_path.ends_with("coarse_aligned_bilinear.fits"), "{}", fine_grid.files[1].output_path);

        let adu = aligned_source(&dir, "adu.fits", 32, 0.03, 0.0, &[("BUNIT", "'ADU'")]);
        let refused_dir = out.join("refused");
        let refused = align_set(&[fine, coarse, adu], refused_dir.to_str().unwrap(), &AlignOptions::default()).unwrap_err().to_string();
        assert!(refused.contains("adu") && refused.contains("'ADU'"), "{refused}");
        assert_eq!(files_in(&refused_dir), 0, "a refused set writes nothing");
    }

    const CUBE_COLS: usize = 6;

    fn alignment_companion_fixture(dir: &tempfile::TempDir, name: &str, scale: f64) -> String {
        let path = dir.path().join(name);
        let mut cards = wcs_cards((4.5, 4.5), scale, 0.0);
        cards.push(("BUNIT", "'Jy'".to_string()));
        write_test_mef(
            &path,
            &[("TELESCOP", "'JWST'".to_string())],
            &[
                TestHdu { extname: Some("SCI"), extver: Some(1), cols: 8, rows: 8, data: TestData::F32((0..64).map(|i| i as f32 - 20.0).collect()), extra_cards: cards },
                TestHdu { extname: Some("ERR"), extver: Some(1), cols: 8, rows: 8, data: TestData::F32(vec![2.0; 64]), extra_cards: vec![("BUNIT", "'Jy'".to_string())] },
                TestHdu { extname: Some("DQ"), extver: Some(1), cols: 8, rows: 8, data: TestData::I32(vec![0x8000_0001u32 as i32; 64]), extra_cards: vec![] },
            ],
        );
        path.to_string_lossy().into_owned()
    }

    fn companion_contract_fixture(dir: &tempfile::TempDir, case: &str) -> String {
        let path = dir.path().join(format!("{case}.fits"));
        let mut sci = wcs_cards((4.5, 4.5), 0.04, 0.0);
        sci.push(("BUNIT", "'Jy'".into()));
        let mut err = match case {
            "missing" => Vec::new(),
            "mjy" => vec![("BUNIT", "'mJy'".into())],
            "brightness" => vec![("BUNIT", "'MJy/sr'".into())],
            _ => vec![("BUNIT", "'Jy/pixel'".into())],
        };
        let dq = match case {
            "dq_shift" => wcs_cards((5.5, 4.5), 0.04, 0.0),
            "dq_partial" => vec![("CRPIX1", "4.5".into())],
            "same" => wcs_cards((4.5, 4.5), 0.04, 0.0),
            _ => Vec::new(),
        };
        if case == "err_shift" { err.extend(wcs_cards((5.5, 4.5), 0.04, 0.0)); }
        if case == "same" { err.extend(wcs_cards((4.5, 4.5), 0.04, 0.0)); }
        let err_rows = if case == "err_shape" { 7 } else { 8 };
        let dq_rows = if case == "dq_shape" { 7 } else { 8 };
        write_test_mef(&path, &[("TELESCOP", "'JWST'".into()), ("INSTRUME", "'NIRCAM'".into()), ("FILTER", "'F200W'".into())], &[
            TestHdu { extname: Some("SCI"), extver: Some(1), cols: 8, rows: 8, data: TestData::F32(vec![10.0; 64]), extra_cards: sci },
            TestHdu { extname: Some("ERR"), extver: Some(1), cols: 8, rows: err_rows, data: TestData::F32(vec![2.0; 8 * err_rows]), extra_cards: err },
            TestHdu { extname: Some("DQ"), extver: Some(1), cols: 8, rows: dq_rows, data: TestData::I32(vec![0; 8 * dq_rows]), extra_cards: dq },
        ]);
        path.to_string_lossy().into_owned()
    }

    fn run_companion_contract_stage(path: &str, output: &std::path::Path, stage: &str) -> Result<Vec<String>> {
        match stage {
            "units" => convert_units_file(path, TargetUnit::JyPerPixel, "", output.to_str()).map(|report| report.warnings),
            "align" => align_set(&[path.to_string()], output.to_str().unwrap(), &AlignOptions::default()).map(|report| report.warnings),
            "cube" => build_cube(&specs_for(&[path.to_string()]), "", output.to_str(), CubeOptions { layout: CubeLayout::Mef, order: PlaneOrder::Wavelength }, None).map(|report| report.warnings),
            _ => unreachable!(),
        }
    }

    #[test]
    fn companion_contract_refuses_conflicting_units_geometry_and_unreadable_planes_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let mut failures = Vec::new();
        for case in ["mjy", "brightness", "err_shift", "dq_shift", "dq_partial", "err_shape", "dq_shape"] {
            let path = companion_contract_fixture(&dir, case);
            for stage in ["units", "align", "cube"] {
                let output = dir.path().join(format!("{case}-{stage}"));
                let result = run_companion_contract_stage(&path, &output, stage);
                if result.is_ok() || output.exists() { failures.push(format!("{case}/{stage}: {result:?}, output exists={}", output.exists())); }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn companion_contract_reports_assumptions_and_accepts_equivalent_physical_units_on_the_same_grid() {
        let dir = tempfile::tempdir().unwrap();
        for case in ["missing", "same"] {
            let path = companion_contract_fixture(&dir, case);
            for stage in ["units", "align", "cube"] {
                let warnings = run_companion_contract_stage(&path, &dir.path().join(format!("{case}-{stage}")), stage).unwrap();
                if case == "missing" {
                    for text in ["ERR has no BUNIT", "ERR has no celestial WCS", "DQ has no celestial WCS"] {
                        assert!(warnings.iter().any(|warning| warning.contains(text)), "{stage}: missing assumption {text}: {warnings:?}");
                    }
                } else {
                    assert!(!warnings.iter().any(|warning| warning.contains("is assumed")), "{stage}: {warnings:?}");
                }
            }
        }
    }

    #[test]
    fn align_companions_survive_copy_crop_and_fits_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let source = alignment_companion_fixture(&dir, "same.fits", 0.04);
        let options = AlignOptions { border_crop: 1, ..AlignOptions::default() };
        let report = align_set(&[source], dir.path().join("out").to_str().unwrap(), &options).unwrap();
        let file = File::open(&report.files[0].output_path).unwrap();
        let extensions = list_extensions(&file).unwrap();
        for name in [EXTNAME_SCI, EXTNAME_ERR, EXTNAME_DQ] {
            assert!(extensions.iter().any(|hdu| hdu.extname.as_deref() == Some(name)), "alignment lost {name}");
        }
        let science = extract_image_mmap_by_index(&file, 1).unwrap();
        let error = extract_image_mmap_by_index(&file, 2).unwrap();
        let dq = extract_int_plane_by_index(&file, 3).unwrap();
        assert_eq!(science.image.dim(), (6, 6));
        assert_eq!(science.image[[0, 0]], -11.0);
        assert!(error.image.iter().all(|&value| value == 2.0));
        assert!(dq.bits.iter().all(|&bits| bits == 0x8000_0001));
        assert_eq!(card_text(&error.header, "BUNIT").as_deref(), Some("Jy"));
        assert_eq!(error.header.get_f64("CRPIX1"), science.header.get_f64("CRPIX1"));
    }

    #[test]
    fn align_companions_propagate_variance_and_dq_for_all_resamplers() {
        let dir = tempfile::tempdir().unwrap();
        let source = alignment_companion_fixture(&dir, "fine.fits", 0.04);
        let reference = alignment_companion_fixture(&dir, "reference.fits", 0.08);
        for method in [ResampleMethod::Bilinear, ResampleMethod::Bicubic, ResampleMethod::Exact] {
            let options = AlignOptions { method, border_crop: 0, ..AlignOptions::default() };
            let report = align_set(&[source.clone(), reference.clone()], dir.path().join(method.label()).to_str().unwrap(), &options).unwrap();
            let file = File::open(&report.files[0].output_path).unwrap();
            let extensions = list_extensions(&file).unwrap();
            assert!(extensions.iter().any(|hdu| hdu.extname.as_deref() == Some(EXTNAME_ERR)), "{} alignment lost ERR", method.label());
            assert!(extensions.iter().any(|hdu| hdu.extname.as_deref() == Some(EXTNAME_DQ)), "{} alignment lost DQ", method.label());
            let science = extract_image_mmap_by_index(&file, 1).unwrap();
            let error = extract_image_mmap_by_index(&file, 2).unwrap();
            let dq = extract_int_plane_by_index(&file, 3).unwrap();
            assert!((science.image[[3, 3]] - 10.0).abs() < 1e-3, "{method:?}: {}", science.image[[3, 3]]);
            assert!((error.image[[3, 3]] - 4.0).abs() < 1e-4, "{method:?}: {}", error.image[[3, 3]]);
            assert_eq!(dq.bits[[3, 3]], 0x8000_0001, "{method:?}");
            assert!(error.image[[0, 0]].is_nan(), "{method:?}");
            assert_eq!(dq.bits[[0, 0]] & 513, 513, "{method:?}");
            assert!(report.files[0].report.warnings.iter().any(|note| note.contains("covariance")), "{method:?}");
        }
    }
    const CUBE_ROWS: usize = 5;

    fn plane_value(filter: &str, x: usize, y: usize) -> f32 {
        let seed = filter.bytes().map(f32::from).sum::<f32>();
        seed + (y * CUBE_COLS + x) as f32 * 0.25
    }

    fn dq_value(filter: &str, x: usize, y: usize) -> i32 {
        i32::from((x + y + filter.len()) % 3 == 0) * 4 + i32::from(x == 0) * 1024
    }

    fn filter_image(dir: &std::path::Path, filter: &str, companions: bool) -> String {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("jw_{}_i2d.fits", filter.to_lowercase()));
        let pixels = |f: &dyn Fn(usize, usize) -> f32| (0..CUBE_ROWS * CUBE_COLS).map(|i| f(i % CUBE_COLS, i / CUBE_COLS)).collect::<Vec<f32>>();
        let mut sci_cards = wcs_cards((3.5, 3.0), 0.04, 15.0);
        sci_cards.push(("BUNIT", "'Jy'".into()));
        let mut hdus = vec![TestHdu {
            extname: Some("SCI"),
            extver: Some(1),
            cols: CUBE_COLS,
            rows: CUBE_ROWS,
            data: TestData::F32(pixels(&|x, y| plane_value(filter, x, y))),
            extra_cards: sci_cards,
        }];
        if companions {
            hdus.push(TestHdu {
                extname: Some("ERR"),
                extver: Some(1),
                cols: CUBE_COLS,
                rows: CUBE_ROWS,
                data: TestData::F32(pixels(&|x, y| plane_value(filter, x, y) * 0.01)),
                extra_cards: vec![("BUNIT", "'Jy'".into())],
            });
            let dq: Vec<i32> = (0..CUBE_ROWS * CUBE_COLS).map(|i| dq_value(filter, i % CUBE_COLS, i / CUBE_COLS)).collect();
            hdus.push(TestHdu { extname: Some("DQ"), extver: Some(1), cols: CUBE_COLS, rows: CUBE_ROWS, data: TestData::I32(dq), extra_cards: vec![] });
        }
        let primary = [("TELESCOP", "'JWST'".to_string()), ("INSTRUME", "'NIRCAM'".to_string()), ("FILTER", format!("'{filter}'"))];
        write_test_mef(&path, &primary, &hdus);
        path.to_str().unwrap().to_string()
    }

    fn specs_for(paths: &[String]) -> Vec<CubeInputSpec> {
        paths.iter().map(|path| CubeInputSpec { path: path.clone(), dataset: None, filter: None }).collect()
    }

    fn header_cards(bytes: &[u8]) -> Vec<String> {
        bytes
            .chunks(80)
            .map(|card| String::from_utf8_lossy(card).into_owned())
            .take_while(|card| !card.starts_with("END     "))
            .collect()
    }

    fn release(path: &str) {
        GLOBAL_CUBE_CACHE.invalidate(path);
        invalidate_written(path);
    }

    #[test]
    fn mef_layout_round_trips_through_the_reader() {
        use crate::core::cube::companions::open_cube_set;
        use crate::infra::fits::table::ColumnValues;
        use crate::infra::wcs_source::header_wcs;

        let dir = tempfile::tempdir().unwrap();
        let inputs = dir.path().join("inputs");
        let paths = [filter_image(&inputs, "F444W", true), filter_image(&inputs, "F150W", true), filter_image(&inputs, "F277W", false)];
        let out = dir.path().join("cubes");
        let options = CubeOptions { layout: CubeLayout::Mef, order: PlaneOrder::Wavelength };
        let report = build_cube(&specs_for(&paths), "", Some(out.to_str().unwrap()), options, Some("F444W")).unwrap();
        assert!(report.output_path.ends_with("datacube.fits"), "{}", report.output_path);
        assert_eq!(report.hdus, ["SCI", "ERR", "DQ", "FILTERS"]);
        assert_eq!(report.reference_path, paths[0]);
        assert_eq!((report.rows, report.cols, report.bunit.as_str()), (CUBE_ROWS, CUBE_COLS, "Jy"));
        let order: Vec<&str> = report.planes.iter().map(|plane| plane.plane.filter.as_str()).collect();
        assert_eq!(order, ["F150W", "F277W", "F444W"]);
        assert_eq!(report.planes[0].identified_by, "header FILTER");
        assert_eq!((report.planes[1].has_err, report.planes[1].has_dq), (false, false));
        assert_eq!(report.warnings.len(), 6, "{:?}", report.warnings);
        assert_eq!(report.warnings.iter().filter(|warning| warning.contains("has no celestial WCS")).count(), 4);

        let file = File::open(&report.output_path).unwrap();
        let exts = list_extensions(&file).unwrap();
        let names: Vec<Option<&str>> = exts.iter().map(|hdu| hdu.extname.as_deref()).collect();
        assert_eq!(names, [None, Some("SCI"), Some("ERR"), Some("DQ"), Some("FILTERS")]);
        assert_eq!((exts[1].naxis, exts[1].naxis1, exts[1].naxis2, exts[1].naxis3), (3, CUBE_COLS as i64, CUBE_ROWS as i64, 3));
        let sci_header = extract_header_by_index(&file, 1).unwrap();
        assert_eq!(card_text(&sci_header, "CTYPE3").as_deref(), Some("FILTER"));
        assert_eq!(card_text(&sci_header, "FILTERS").as_deref(), Some("F150W,F277W,F444W"));
        assert_eq!(card_text(&sci_header, "ABPROC").as_deref(), Some("cube"));
        assert!(card_text(&sci_header, "PROGRAM").is_some_and(|program| program.starts_with("AstroBurst")));
        let history: Vec<&str> = sci_header.cards.iter().filter(|(key, _)| key == "HISTORY").map(|(_, value)| value.as_str()).collect();
        assert!(history.join(" ").contains("Prepared following LIGHTSTACK (Wille & Pessi 2026, doi:10.5281/zenodo.19393438)"), "{history:?}");
        let err_header = extract_header_by_index(&file, 2).unwrap();
        assert_eq!((card_text(&err_header, "BUNIT").as_deref(), card_text(&err_header, "CTYPE3").as_deref()), (Some("Jy"), Some("FILTER")));
        let dq_header = extract_header_by_index(&file, 3).unwrap();
        assert!(dq_header.get("BUNIT").is_none() && dq_header.get("CRPIX1").is_some());

        let source_header = extract_header_by_index(&File::open(&paths[1]).unwrap(), 1).unwrap();
        let (cube_wcs, plane_wcs) = (header_wcs(&sci_header).unwrap(), header_wcs(&source_header).unwrap());
        for (x, y) in [(0.0, 0.0), (5.0, 4.0), (2.5, 1.0)] {
            let (a, b) = (cube_wcs.pixel_to_world(x, y), plane_wcs.pixel_to_world(x, y));
            assert!((a.ra - b.ra).abs() < 1e-12 && (a.dec - b.dec).abs() < 1e-12);
        }

        let set = open_cube_set(&report.output_path).unwrap();
        assert_eq!((set.sci.geometry.depth, set.err.as_ref().map(|err| err.hdu_index), set.dq.as_ref().map(|dq| dq.hdu_index)), (3, Some(2), Some(3)));
        for (z, filter) in ["F150W", "F277W", "F444W"].iter().enumerate() {
            let frame = set.sci.frame_uncached(z).unwrap();
            assert!(frame.indexed_iter().all(|((y, x), &v)| v.to_bits() == plane_value(filter, x, y).to_bits()), "{filter}");
        }
        let err_frame = set.err.as_ref().unwrap().frame_uncached(1).unwrap();
        assert!(err_frame.iter().all(|v| v.is_nan()), "F277W has no ERR");
        let table_index = find_bintable(&report.output_path, "FILTERS", 1).unwrap();
        let table = load_bintable(&report.output_path, table_index).unwrap();
        assert_eq!(table.text_column("FILTER").unwrap(), ["F150W", "F277W", "F444W"]);
        assert_eq!(table.text_column("INSTRUME").unwrap(), ["NIRCam", "NIRCam", "NIRCam"]);
        let ColumnValues::F64(pivots) = table.flattened_column("PIVOT_UM").unwrap().values else { panic!("PIVOT_UM is not numeric") };
        assert!((pivots[2] - 4.404315).abs() < 1e-12, "{pivots:?}");

        let back = read_cube_file(&report.output_path, &FilterRegistry::builtin()).unwrap();
        assert_eq!(back.layout, CubeLayout::Mef);
        assert_eq!(back.cube.filters.iter().map(|p| p.filter.as_str()).collect::<Vec<_>>(), ["F150W", "F277W", "F444W"]);
        assert_eq!(back.cube.filters[0].fluxconv, Some(1.0));
        assert_eq!(back.cube.filters[0].psf_fwhm_arcsec, None);
        let dq = back.cube.dq.as_ref().unwrap();
        assert!(dq.signed);
        assert!(dq.planes[2].indexed_iter().all(|((y, x), &bits)| bits as i32 == dq_value("F444W", x, y)));
        assert!(dq.planes[1].iter().all(|&bits| bits == 0));
        assert!(back.cube.header.get("EXTNAME").is_none());
        release(&report.output_path);
    }

    #[test]
    fn ten_or_more_filters_write_hierarch_dataset_keys_like_astropy() {
        let dir = tempfile::tempdir().unwrap();
        let registry = FilterRegistry::builtin();
        let filters = ["F070W", "F090W", "F115W", "F140M", "F150W", "F200W", "F277W", "F335M", "F356W", "F410M", "F444W", "F480M"];
        let paths: Vec<String> = filters.iter().map(|filter| filter_image(&dir.path().join("in"), filter, false)).collect();
        let target = dir.path().join("lightstack_cube.fits");
        let report = build_cube(&specs_for(&paths), target.to_str().unwrap(), None, CubeOptions::default(), None).unwrap();
        assert_eq!((report.layout, report.hdus.as_slice()), (CubeLayout::Lightstack, ["PRIMARY".to_string()].as_slice()));
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        let bytes = std::fs::read(&target).unwrap();
        let cards = header_cards(&bytes);
        let card = |prefix: &str| cards.iter().find(|card| card.starts_with(prefix)).cloned().unwrap_or_default();
        for n in 10..=12 {
            assert_eq!(card(&format!("HIERARCH DATASET{n} ")), format!("{:<80}", format!("HIERARCH DATASET{n} = 'JWST    '")));
        }
        assert_eq!(card("DATASET9"), format!("{:<80}", "DATASET9= 'JWST    '"));
        assert_eq!(card("FILTER10"), format!("{:<80}", "FILTER10= 'F410M   '"));
        let joined = filters.join(",");
        let first = card("FILTERS =");
        assert_eq!(first, format!("FILTERS = '{}&'", &joined[..67]));
        let next = cards.iter().position(|c| c.starts_with("FILTERS =")).map(|i| cards[i + 1].clone()).unwrap();
        assert_eq!(next, format!("{:<80}", format!("CONTINUE  '{}'", &joined[67..])));
        assert_eq!(&cards[..7].iter().map(|c| c[..8].trim().to_string()).collect::<Vec<_>>(), &["SIMPLE", "BITPIX", "NAXIS", "NAXIS1", "NAXIS2", "NAXIS3", "EXTEND"]);

        let header = extract_header_by_index(&File::open(&target).unwrap(), 0).unwrap();
        assert_eq!(header.get("HIERARCH DATASET12"), Some("JWST"));
        assert_eq!(header.get("FILTERS"), Some(joined.as_str()));
        assert_eq!((header.get_i64("BITPIX"), header.get_i64("NAXIS3"), header.get_i64("NFILTERS")), (Some(-32), Some(12), Some(12)));
        let planes = planes_from_header(&header, 12, &registry).unwrap();
        assert_eq!(planes.iter().map(|p| p.filter.as_str()).collect::<Vec<_>>(), filters);
        assert!(planes.iter().all(|p| p.pivot_um.is_some()));
        let back = read_cube_file(target.to_str().unwrap(), &registry).unwrap();
        assert_eq!(back.layout, CubeLayout::Lightstack);
        assert_eq!(back.cube.planes.len(), 12);
        release(target.to_str().unwrap());
    }

    #[test]
    fn build_cube_refuses_a_plane_off_the_grid_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = dir.path().join("inputs");
        let on_grid = filter_image(&inputs, "F444W", false);
        let off_grid = inputs.join("jw_f150w_shifted.fits");
        let mut cards = wcs_cards((4.5, 3.0), 0.04, 15.0);
        cards.push(("BUNIT", "'Jy'".into()));
        write_test_mef(
            &off_grid,
            &[("TELESCOP", "'JWST'".into()), ("INSTRUME", "'NIRCAM'".into()), ("FILTER", "'F150W'".into())],
            &[TestHdu { extname: Some("SCI"), extver: Some(1), cols: CUBE_COLS, rows: CUBE_ROWS, data: TestData::F32(vec![1.0; CUBE_COLS * CUBE_ROWS]), extra_cards: cards }],
        );
        let out = dir.path().join("refused");
        let specs = specs_for(&[on_grid, off_grid.to_str().unwrap().to_string()]);
        let refused = build_cube(&specs, "", Some(out.to_str().unwrap()), CubeOptions::default(), None).unwrap_err().to_string();
        assert!(refused.contains("jw_f150w_shifted") && refused.contains("align"), "{refused}");
        assert_eq!(files_in(&out), 0);
    }

    fn mef_cube(dir: &std::path::Path) -> String {
        let inputs = dir.join("inputs");
        let paths = [filter_image(&inputs, "F150W", true), filter_image(&inputs, "F277W", true), filter_image(&inputs, "F444W", true)];
        let options = CubeOptions { layout: CubeLayout::Mef, order: PlaneOrder::Wavelength };
        build_cube(&specs_for(&paths), dir.join("cube.fits").to_str().unwrap(), None, options, None).unwrap().output_path
    }

    #[test]
    fn subcube_keeps_err_dq_and_filters_table() {
        let dir = tempfile::tempdir().unwrap();
        let source = mef_cube(dir.path());
        let registry = FilterRegistry::builtin();
        let before = read_cube_file(&source, &registry).unwrap().cube;
        let out = dir.path().join("edited");
        let edit = CubeEdit::Cut { x_start: 1, x_end: 5, y_start: 2, y_end: 5 };
        let report = edit_cube(&source, &edit, "", Some(out.to_str().unwrap()), None).unwrap();
        assert!(report.output_path.ends_with("cube_cut.fits"), "{}", report.output_path);
        assert_eq!((report.layout, report.rows, report.cols), (CubeLayout::Mef, 3, 4));
        assert_eq!(report.hdus, ["SCI", "ERR", "DQ", "FILTERS"]);

        let after = read_cube_file(&report.output_path, &registry).unwrap().cube;
        let window = |plane: &Array2<f32>| plane.slice(ndarray::s![2..5, 1..5]).to_owned();
        for z in 0..3 {
            assert!(after.planes[z].iter().zip(window(&before.planes[z]).iter()).all(|(a, b)| a.to_bits() == b.to_bits()));
            let err = &after.err.as_ref().unwrap()[z];
            assert!(err.iter().zip(window(&before.err.as_ref().unwrap()[z]).iter()).all(|(a, b)| a.to_bits() == b.to_bits()));
            let dq = &after.dq.as_ref().unwrap().planes[z];
            assert_eq!(dq, &before.dq.as_ref().unwrap().planes[z].slice(ndarray::s![2..5, 1..5]).to_owned());
        }
        assert_eq!(after.filters, before.filters);
        let file = File::open(&report.output_path).unwrap();
        for hdu in 1..=3 {
            let header = extract_header_by_index(&file, hdu).unwrap();
            assert_eq!(header.get_f64("CRPIX1"), before.header.get_f64("CRPIX1").map(|v| v - 1.0), "HDU {hdu}");
            assert_eq!(header.get_f64("CRPIX2"), before.header.get_f64("CRPIX2").map(|v| v - 2.0), "HDU {hdu}");
            assert_eq!((header.get_i64("NAXIS1"), header.get_i64("NAXIS2"), header.get_i64("NAXIS3")), (Some(4), Some(3), Some(3)));
        }
        let sci = extract_header_by_index(&file, 1).unwrap();
        let bounds: Vec<Option<i64>> = ["XMINPIX", "XMAXPIX", "YMINPIX", "YMAXPIX"].iter().map(|key| sci.get_i64(key)).collect();
        assert_eq!(bounds, [Some(1), Some(5), Some(2), Some(5)]);
        assert_eq!(card_text(&sci, "ABPROC").as_deref(), Some("cube-edit"));
        assert_eq!((sci.get_i64("WCSAXES"), card_text(&sci, "CTYPE3").as_deref()), (Some(3), Some("FILTER")));

        let centre = before.wcs().unwrap().pixel_to_world(3.0, 2.0);
        let sky = CubeEdit::CutSky { ra: centre.ra, dec: centre.dec, size_arcsec: 0.1, rounding: CropRounding::Lightstack };
        let sky_cut = edit_cube(&source, &sky, out.join("sky.fits").to_str().unwrap(), None, None).unwrap();
        let sky_header = extract_header_by_index(&File::open(&sky_cut.output_path).unwrap(), 1).unwrap();
        let (x0, y0) = (sky_header.get_i64("XMINPIX").unwrap(), sky_header.get_i64("YMINPIX").unwrap());
        let moved = header_wcs_for(&sky_header).pixel_to_world(3.0 - x0 as f64, 2.0 - y0 as f64);
        assert!((moved.ra - centre.ra).abs() < 1e-12 && (moved.dec - centre.dec).abs() < 1e-12);
        assert!(sky_cut.rows < CUBE_ROWS && sky_cut.cols < CUBE_COLS, "{} x {}", sky_cut.cols, sky_cut.rows);

        let refused = edit_cube(&source, &CubeEdit::Cut { x_start: 0, x_end: 9, y_start: 0, y_end: 2 }, out.join("bad.fits").to_str().unwrap(), None, None);
        assert!(refused.unwrap_err().to_string().contains("does not lie inside"));
        for path in [source.as_str(), report.output_path.as_str(), sky_cut.output_path.as_str()] {
            release(path);
        }
    }

    fn header_wcs_for(header: &HduHeader) -> WcsTransform {
        crate::infra::wcs_source::header_wcs(header).unwrap()
    }

    #[test]
    fn edits_keep_the_layout_unless_another_is_asked_and_parse_the_recipe_names() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = dir.path().join("inputs");
        let paths: Vec<String> = ["F150W", "F200W", "F277W", "F444W"].iter().map(|filter| filter_image(&inputs, filter, true)).collect();
        let lightstack = dir.path().join("ls_cube.fits");
        let built = build_cube(&specs_for(&paths), lightstack.to_str().unwrap(), None, CubeOptions::default(), None).unwrap();
        assert!(built.warnings.iter().any(|w| w.contains("ERR and DQ")), "{:?}", built.warnings);
        let source = lightstack.to_str().unwrap();
        let out = dir.path().join("edits");

        let parse = |json: serde_json::Value| serde_json::from_value::<CubeEdit>(json).unwrap();
        let remove = parse(serde_json::json!({ "step": "edit", "action": "remove_filter", "filter": "F200W" }));
        let removed = edit_cube(source, &remove, "", Some(out.to_str().unwrap()), None).unwrap();
        assert!(removed.output_path.ends_with("ls_cube_removed.fits"));
        assert_eq!((removed.layout, removed.filters.as_slice()), (CubeLayout::Lightstack, ["F150W", "F277W", "F444W"].map(String::from).as_slice()));
        let header = extract_header_by_index(&File::open(&removed.output_path).unwrap(), 0).unwrap();
        assert_eq!(card_text(&header, "DATASETS").as_deref(), Some("JWST,JWST,JWST"));
        assert!(header.get("FILTER4").is_none() && header.get("DATASET4").is_none());

        let drop = parse(serde_json::json!({ "action": "drop_invalid" }));
        assert_eq!(drop, CubeEdit::DropInvalid { threshold: 0.0, frac_valid: 0.01, use_dq: false });
        let kept = edit_cube(source, &drop, "", Some(out.to_str().unwrap()), Some(CubeLayout::Mef)).unwrap();
        assert_eq!((kept.layout, kept.removed.len(), kept.hdus.len()), (CubeLayout::Mef, 0, 2));
        let converted = read_cube_file(&kept.output_path, &FilterRegistry::builtin()).unwrap();
        assert_eq!(converted.cube.filters[0].fluxconv, None, "a LIGHTSTACK layout records no conversion factor");
        assert!((converted.cube.filters[3].pivot_um.unwrap() - 4.404315).abs() < 1e-12);

        let header_edit = parse(serde_json::json!({ "action": "header", "redshift": 0.303, "use_brightest_pixel": true }));
        let more = edit_cube(source, &header_edit, "", Some(out.to_str().unwrap()), None).unwrap();
        assert!(more.output_path.ends_with("ls_cube_more.fits"));
        let centre = more.centre.unwrap();
        assert_eq!((centre.kind, centre.x, centre.y), ("BRIGHTEST", Some(CUBE_COLS - 1), Some(CUBE_ROWS - 1)));
        let header = extract_header_by_index(&File::open(&more.output_path).unwrap(), 0).unwrap();
        assert_eq!((header.get_f64("REDSHIFT"), header.get_i64("GAL_XCEN")), (Some(0.303), Some(CUBE_COLS as i64 - 1)));

        let mask = parse(serde_json::json!({ "action": "reference_nan_mask", "reference_slice": 0 }));
        assert_eq!(mask, CubeEdit::ReferenceNanMask { reference_slice: 0, mode: crate::core::cubeprep::cube::MaskMode::Reference });
        let masked = edit_cube(source, &mask, "", Some(out.to_str().unwrap()), None).unwrap();
        assert_eq!(masked.masked_pixels, Some(0));
        let cut = parse(serde_json::json!({ "action": "cut", "x_start": 0, "x_end": 2, "y_start": 1, "y_end": 3 }));
        assert_eq!(cut, CubeEdit::Cut { x_start: 0, x_end: 2, y_start: 1, y_end: 3 });
        for path in [source, removed.output_path.as_str(), kept.output_path.as_str(), more.output_path.as_str(), masked.output_path.as_str()] {
            release(path);
        }
    }

    #[test]
    fn a_nan_mask_flags_the_dq_cube_with_the_padding_bits_of_each_mission() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = dir.path().join("inputs");
        let nan_corner = inputs.join("jw_f150w_i2d.fits");
        let mut sci_cards = wcs_cards((3.5, 3.0), 0.04, 15.0);
        sci_cards.push(("BUNIT", "'Jy'".into()));
        let mut pixels = vec![1.0f32; CUBE_COLS * CUBE_ROWS];
        pixels[0] = f32::NAN;
        std::fs::create_dir_all(&inputs).unwrap();
        write_test_mef(
            &nan_corner,
            &[("TELESCOP", "'JWST'".into()), ("INSTRUME", "'NIRCAM'".into()), ("FILTER", "'F150W'".into())],
            &[
                TestHdu { extname: Some("SCI"), extver: Some(1), cols: CUBE_COLS, rows: CUBE_ROWS, data: TestData::F32(pixels), extra_cards: sci_cards },
                TestHdu { extname: Some("DQ"), extver: Some(1), cols: CUBE_COLS, rows: CUBE_ROWS, data: TestData::I32(vec![0; CUBE_COLS * CUBE_ROWS]), extra_cards: vec![] },
            ],
        );
        let paths = [nan_corner.to_str().unwrap().to_string(), filter_image(&inputs, "F444W", true)];
        let options = CubeOptions { layout: CubeLayout::Mef, order: PlaneOrder::Wavelength };
        let cube = build_cube(&specs_for(&paths), dir.path().join("masked_in.fits").to_str().unwrap(), None, options, None).unwrap();
        let edit = CubeEdit::ReferenceNanMask { reference_slice: 0, mode: crate::core::cubeprep::cube::MaskMode::Reference };
        let masked = edit_cube(&cube.output_path, &edit, dir.path().join("masked.fits").to_str().unwrap(), None, None).unwrap();
        assert_eq!(masked.masked_pixels, Some(1));
        let back = read_cube_file(&masked.output_path, &FilterRegistry::builtin()).unwrap().cube;
        assert!(back.planes[1][[0, 0]].is_nan() && back.err.as_ref().unwrap()[1][[0, 0]].is_nan());
        let jwst_padding = padded_dq_bits(DqTable::Jwst).unwrap();
        assert_eq!(back.dq.as_ref().unwrap().planes[1][[0, 0]], dq_value("F444W", 0, 0) as u32 | jwst_padding);
        assert_eq!(back.dq.as_ref().unwrap().planes[1][[0, 1]], dq_value("F444W", 1, 0) as u32);
        for path in [cube.output_path.as_str(), masked.output_path.as_str()] {
            release(path);
        }
    }

    #[test]
    fn load_psfs_writes_odd_unit_sum_psfs_on_the_reference_grid() {
        let d1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lightstack/d1/psf");
        let spec = |name: &str| PsfInputSpec { path: d1.join(name).to_str().unwrap().to_string(), filter: None, pixel_scale_arcsec: None };
        let inputs = vec![spec("d1_psf_JWST_F150W_20mas.fits"), spec("d1_psf_JWST_F444W_40mas.fits")];
        let dir = tempfile::tempdir().unwrap();
        let options = PsfSetOptions { reference: Some("F444W".into()), ..PsfSetOptions::default() };
        let report = load_psfs(&inputs, &options, Some(dir.path().to_str().unwrap())).unwrap();
        assert_eq!(
            (report.reference_index, report.target_pixel_scale_arcsec, report.target_rows, report.target_cols),
            (Some(1), Some(0.04), Some(21), Some(21))
        );
        for file in &report.psfs {
            let written = load_psf(std::path::Path::new(file.output_path.as_ref().unwrap()), None).unwrap();
            assert_eq!((written.data.dim(), written.pixel_scale_arcsec, written.scale_source), ((21, 21), 0.04, ScaleSource::Pixscale));
            assert!((written.data.sum() - 1.0).abs() < 1e-12, "{}: sum {}", file.path, written.data.sum());
            assert_eq!(written.filter, file.filter);
        }
        assert_eq!(report.psfs[0].filter.as_deref(), Some("F150W"));
        assert_eq!(report.psfs[0].resampled.as_ref().map(|record| record.method), Some(PsfResampleMethod::Area));
        assert!(report.psfs[1].resampled.is_none());
        assert!(report.psfs[0].output_path.as_ref().unwrap().ends_with("d1_psf_JWST_F150W_20mas_prepared.fits"));
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["psfs"][1]["scale_source"], "pixscale");
        assert_eq!(value["method"], "area");

        let empty = tempfile::tempdir().unwrap();
        let refused = PsfSetOptions { reference: Some("F999W".into()), ..PsfSetOptions::default() };
        let error = load_psfs(&inputs, &refused, Some(empty.path().to_str().unwrap())).unwrap_err();
        assert!(format!("{error:#}").contains("F999W"), "{error:#}");
        assert_eq!(std::fs::read_dir(empty.path()).unwrap().count(), 0, "nothing is written when the set is refused");

        let renamed = vec![PsfInputSpec { filter: Some("REF".into()), ..inputs[1].clone() }, inputs[0].clone()];
        let report = load_psfs(&renamed, &PsfSetOptions { reference: Some("ref".into()), ..PsfSetOptions::default() }, None).unwrap();
        assert_eq!(report.reference_index, Some(0));
        assert!(report.psfs.iter().all(|file| file.output_path.is_none()));
    }

    fn d1_psf(name: &str) -> PsfInputSpec {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lightstack/d1/psf").join(name);
        PsfInputSpec { path: path.to_str().unwrap().to_string(), filter: None, pixel_scale_arcsec: None }
    }

    #[test]
    fn build_kernels_writes_one_kernel_per_filter_with_its_recipe_and_diagnostics() {
        let inputs = vec![d1_psf("d1_psf_JWST_F150W_40mas.fits"), d1_psf("d1_psf_JWST_F444W_40mas.fits"), d1_psf("d1_psf_HST_F814W_40mas.fits")];
        let dir = tempfile::tempdir().unwrap();
        let options = KernelOptions { auto: true, ..KernelOptions::default() };
        let report = build_kernels(&inputs, "f444w", &options, Some(dir.path().to_str().unwrap())).unwrap();
        assert_eq!((report.reference.as_str(), report.target_label.as_str(), report.pixel_scale_arcsec), ("F444W", "JWST_F444W", 0.04));
        assert!(report.reference_path.ends_with("d1_psf_JWST_F444W_40mas.fits"));
        let filters: Vec<&str> = report.kernels.iter().map(|kernel| kernel.filter.as_str()).collect();
        assert_eq!(filters, ["F150W", "F814W"]);
        for (file, name, label) in [(&report.kernels[0], "kernel_f150w_to_f444w.fits", "JWST_F150W"), (&report.kernels[1], "kernel_f814w_to_f444w.fits", "HST_F814W")] {
            let output = file.output_path.as_deref().unwrap();
            assert!(output.ends_with(name), "{output}");
            assert_eq!(file.source_label, label);
            assert!(file.auto.is_some() && file.diagnostics.residual_max < 0.01, "{:?}", file.diagnostics);
            let source = load_psf(std::path::Path::new(&file.path), None).unwrap();
            let target = load_psf(std::path::Path::new(&report.reference_path), None).unwrap();
            let expected = build_kernel(&source.data, &target.data, &options).unwrap();
            let written = load_psf(std::path::Path::new(output), None).unwrap();
            assert!(written.data.iter().zip(expected.data.iter()).all(|(a, b)| a.to_bits() == b.to_bits()), "{name}: the FITS holds the kernel bit for bit");
            assert_eq!((written.pixel_scale_arcsec, written.scale_source), (0.04, ScaleSource::Pixscale));
            assert_eq!(card_text(&written.header, "SRCPSF").as_deref(), Some(label));
            assert_eq!(card_text(&written.header, "TGTPSF").as_deref(), Some("JWST_F444W"));
            assert_eq!(card_text(&written.header, "KEPSMODE").as_deref(), Some("relative"));
            assert_eq!(written.header.get("KAUTO").map(str::trim), Some("T"));
        }
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["kernels"][0]["method"]["method"], "wiener");
        assert_eq!(value["kernels"][0]["auto"]["parameter"], "eps");

        let preview = build_kernels(&inputs[..2], "F444W", &KernelOptions::default(), None).unwrap();
        assert!(preview.kernels[0].output_path.is_none(), "without an output directory nothing is written");

        let empty = tempfile::tempdir().unwrap();
        let fine = vec![d1_psf("d1_psf_JWST_F150W_20mas.fits"), d1_psf("d1_psf_JWST_F444W_40mas.fits")];
        let error = build_kernels(&fine, "F444W", &options, Some(empty.path().to_str().unwrap())).unwrap_err();
        assert!(format!("{error:#}").contains("0.02") && format!("{error:#}").contains("load_psfs"), "{error:#}");
        let narrower = build_kernels(&inputs, "F150W", &options, Some(empty.path().to_str().unwrap())).unwrap_err();
        assert!(format!("{narrower:#}").contains("narrower"), "{narrower:#}");
        let missing = build_kernels(&inputs, "F999W", &options, Some(empty.path().to_str().unwrap())).unwrap_err();
        assert!(format!("{missing:#}").contains("F999W"), "{missing:#}");
        let twice = vec![inputs[0].clone(), inputs[1].clone(), PsfInputSpec { filter: Some("F150W".into()), ..inputs[2].clone() }];
        let duplicate = build_kernels(&twice, "F444W", &options, Some(empty.path().to_str().unwrap())).unwrap_err();
        assert!(format!("{duplicate:#}").contains("F150W"), "{duplicate:#}");
        assert_eq!(std::fs::read_dir(empty.path()).unwrap().count(), 0, "nothing is written when the set is refused");
    }

    fn write_kernel(dir: &std::path::Path, name: &str, data: &Array2<f64>, cards: &[(&str, f64)]) -> String {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name).to_str().unwrap().to_string();
        let mut header = HduHeader::empty();
        for (key, value) in cards {
            header.set_f64(key, *value);
        }
        write_fits_f64(&path, data, Some(&header)).unwrap();
        path
    }

    #[test]
    fn psf_match_cube_reads_kernels_by_name_and_writes_the_matched_cube() {
        let dir = tempfile::tempdir().unwrap();
        let source = mef_cube(dir.path());
        let registry = FilterRegistry::builtin();
        let before = read_cube_file(&source, &registry).unwrap().cube;
        let blur = Array2::from_shape_vec((3, 3), vec![0.05, 0.1, 0.05, 0.1, 0.4, 0.1, 0.05, 0.1, 0.05]).unwrap();
        let wide = Array2::from_shape_fn((5, 5), |(r, c)| (-((r as f64 - 2.0).powi(2) + (c as f64 - 2.0).powi(2)) / 2.0).exp());
        let wide = &wide / wide.sum();
        let kernels_dir = dir.path().join("kernels");
        let f150w = write_kernel(&kernels_dir, "kernel_f150w_to_f444w.fits", &blur, &[("PIXSCALE", 0.04)]);
        let out = dir.path().join("out");
        let options = MatchOptions { reference: "F444W".into(), mode: MatchMode::Astroburst, allow_unmatched: false };

        let refused = psf_match_cube(&source, Some(kernels_dir.to_str().unwrap()), &[], &options, "", Some(out.to_str().unwrap()), None).unwrap_err();
        assert!(format!("{refused:#}").starts_with("kernel_missing: ") && format!("{refused:#}").contains("F277W"), "{refused:#}");
        assert!(!out.exists() || std::fs::read_dir(&out).unwrap().count() == 0, "nothing is written when a kernel is missing");

        write_kernel(&kernels_dir, "kernel_f277w_to_f444w.fits", &wide, &[("PIXSCALE", 0.04)]);
        let report = psf_match_cube(&source, Some(kernels_dir.to_str().unwrap()), &[], &options, "", Some(out.to_str().unwrap()), None).unwrap();
        assert!(report.output_path.ends_with("cube_psfmatched.fits"), "{}", report.output_path);
        assert_eq!((report.layout, report.mode, report.hdus.clone()), (CubeLayout::Mef, MatchMode::Astroburst, vec!["SCI".to_string(), "ERR".into(), "DQ".into(), "FILTERS".into()]));
        let statuses: Vec<&str> = report.planes.iter().map(|plane| plane.kernel.as_deref().unwrap_or("-")).collect();
        assert_eq!(statuses, ["kernel_f150w_to_f444w.fits", "kernel_f277w_to_f444w.fits", "-"]);

        let after = read_cube_file(&report.output_path, &registry).unwrap().cube;
        let plane_kernels = [
            PlaneKernel { filter: "F150W".into(), label: "kernel_f150w_to_f444w.fits".into(), data: blur.clone(), pixel_scale_arcsec: Some(0.04) },
            PlaneKernel { filter: "F277W".into(), label: "kernel_f277w_to_f444w.fits".into(), data: wide.clone(), pixel_scale_arcsec: Some(0.04) },
        ];
        let flags: Vec<u32> = before.filters.iter().map(masked_dq_bits).collect();
        let expected = match_cube(&before, &plane_kernels, &options, &flags).unwrap().cube;
        for z in 0..3 {
            assert!(after.planes[z].iter().zip(expected.planes[z].iter()).all(|(a, b)| a.to_bits() == b.to_bits()), "plane {z}");
            assert!(after.err.as_ref().unwrap()[z].iter().zip(expected.err.as_ref().unwrap()[z].iter()).all(|(a, b)| a.to_bits() == b.to_bits()), "ERR {z}");
        }
        assert!(after.planes[2].iter().zip(before.planes[2].iter()).all(|(a, b)| a.to_bits() == b.to_bits()), "the reference plane is unchanged");
        assert_eq!(after.filters.iter().map(|plane| plane.kernel.as_str()).collect::<Vec<_>>(), ["kernel_f150w_to_f444w.fits", "kernel_f277w_to_f444w.fits", ""]);
        let sci = extract_header_by_index(&File::open(&report.output_path).unwrap(), 1).unwrap();
        assert_eq!(card_text(&sci, "ABPROC").as_deref(), Some("psf-matched"));
        let history: String = sci.cards.iter().filter(|(key, _)| key == "HISTORY").map(|(_, value)| value.as_str()).collect::<Vec<_>>().join(" ");
        assert!(history.contains("PSF matched to F444W"), "{history}");

        let renamed_dir = dir.path().join("tutorial");
        let tutorial = [
            KernelFileSpec { filter: "F150W".into(), path: f150w.clone() },
            KernelFileSpec { filter: "F277W".into(), path: write_kernel(&renamed_dir, "kernel_JWST_F277W_to_JWST_F444W.fits", &wide, &[]) },
            KernelFileSpec { filter: "F999W".into(), path: f150w.clone() },
        ];
        let compat = MatchOptions { mode: MatchMode::Lightstack, ..options.clone() };
        let lightstack = psf_match_cube(&source, None, &tutorial, &compat, out.join("compat.fits").to_str().unwrap(), None, Some(CubeLayout::Lightstack)).unwrap();
        assert_eq!((lightstack.layout, lightstack.hdus.clone()), (CubeLayout::Lightstack, vec!["PRIMARY".to_string()]));
        assert!(lightstack.warnings.iter().any(|warning| warning.contains("F999W")), "{:?}", lightstack.warnings);
        assert!(
            lightstack.warnings.iter().any(|warning| warning.contains("kernel_JWST_F277W_to_JWST_F444W.fits") && warning.contains("no pixel scale")),
            "{:?}",
            lightstack.warnings
        );
        let primary = extract_header_by_index(&File::open(&lightstack.output_path).unwrap(), 0).unwrap();
        let history: Vec<String> = primary.cards.iter().filter(|(key, _)| key == "HISTORY").map(|(_, value)| value.trim().to_string()).collect();
        assert!(history.iter().any(|line| line == "PSF matched to F444W using FFT convolution kernels"), "{history:?}");

        let coarse = write_kernel(&renamed_dir, "coarse.fits", &wide, &[("PIXSCALE", 0.03)]);
        let scale = psf_match_cube(&source, Some(kernels_dir.to_str().unwrap()), &[KernelFileSpec { filter: "F277W".into(), path: coarse }], &options, out.join("bad.fits").to_str().unwrap(), None, None).unwrap_err();
        assert!(format!("{scale:#}").starts_with("kernel_scale: "), "{scale:#}");
        let stpsf_style = write_kernel(&renamed_dir, "pixelscl.fits", &wide, &[("PIXELSCL", 0.03)]);
        let pixelscl = psf_match_cube(&source, Some(kernels_dir.to_str().unwrap()), &[KernelFileSpec { filter: "F277W".into(), path: stpsf_style }], &options, out.join("bad.fits").to_str().unwrap(), None, None);
        assert!(pixelscl.as_ref().is_err_and(|error| format!("{error:#}").starts_with("kernel_scale: ")), "a PIXELSCL card is checked too: {:?}", pixelscl.map(|report| report.output_path));
        for path in [source.as_str(), report.output_path.as_str(), lightstack.output_path.as_str()] {
            release(path);
        }
    }

    fn noise_source(dir: &std::path::Path, filter: &str, size: usize, scale: f64, shift: f64) -> String {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("{}_src.fits", filter.to_lowercase()));
        let centre = size as f64 / 2.0 + 0.5;
        let mut sci_cards = wcs_cards((centre - shift, centre - shift), scale, 0.0);
        sci_cards.push(("BUNIT", "'Jy'".into()));
        let sci: Vec<f32> = (0..size * size).map(|i| 10.0 + (i % size) as f32 + 0.5 * (i / size) as f32).collect();
        write_test_mef(
            &path,
            &[("TELESCOP", "'JWST'".into()), ("INSTRUME", "'NIRCAM'".into()), ("FILTER", format!("'{filter}'"))],
            &[
                TestHdu { extname: Some("SCI"), extver: Some(1), cols: size, rows: size, data: TestData::F32(sci), extra_cards: sci_cards },
                TestHdu { extname: Some("ERR"), extver: Some(1), cols: size, rows: size, data: TestData::F32(vec![1.0; size * size]), extra_cards: vec![("BUNIT", "'Jy'".into())] },
                TestHdu { extname: Some("DQ"), extver: Some(1), cols: size, rows: size, data: TestData::I32(vec![0; size * size]), extra_cards: vec![] },
            ],
        );
        path.to_string_lossy().into_owned()
    }

    fn noise_sources(dir: &std::path::Path) -> Vec<String> {
        let inputs = dir.join("inputs");
        vec![noise_source(&inputs, "F150W", 30, 0.04, 0.0), noise_source(&inputs, "F277W", 24, 0.05, 0.3), noise_source(&inputs, "F444W", 24, 0.05, 0.0)]
    }

    fn bilinear_without_crop() -> AlignOptions {
        AlignOptions { method: ResampleMethod::Bilinear, border_crop: 0, ..AlignOptions::default() }
    }

    fn aligned_noise_set(dir: &std::path::Path) -> AlignSetReport {
        align_set(&noise_sources(dir), dir.join("aligned").to_str().unwrap(), &bilinear_without_crop()).unwrap()
    }

    fn aligned_paths(report: &AlignSetReport) -> Vec<String> {
        report.files.iter().map(|file| file.output_path.clone()).collect()
    }

    fn noise_cube(dir: &std::path::Path, aligned: &AlignSetReport) -> CubeReport {
        let options = CubeOptions { layout: CubeLayout::Mef, order: PlaneOrder::Wavelength };
        build_cube(&specs_for(&aligned_paths(aligned)), dir.join("noise_cube.fits").to_str().unwrap(), None, options, None).unwrap()
    }

    fn noise_hdu(path: &str) -> Option<(HduInfo, HduHeader, Vec<Array2<f32>>)> {
        let file = File::open(path).unwrap();
        let hdu = list_extensions(&file).unwrap().into_iter().find(|hdu| hdu.extname.as_deref() == Some("NOISCORR"))?;
        let (header, maps) = noise_extension(std::path::Path::new(path)).unwrap().unwrap();
        Some((hdu, header, maps))
    }

    fn table_numbers(path: &str, column: &str) -> Vec<f64> {
        let table = load_bintable(path, find_bintable(path, "FILTERS", 1).unwrap()).unwrap();
        let crate::infra::fits::table::ColumnValues::F64(values) = table.flattened_column(column).unwrap().values else { panic!("{column} is not numeric") };
        values
    }

    fn table_texts(path: &str, column: &str) -> Vec<String> {
        load_bintable(path, find_bintable(path, "FILTERS", 1).unwrap()).unwrap().text_column(column).unwrap()
    }

    fn support(map: &Array2<f32>) -> Array2<f32> {
        crate::core::alignment::wcs_reproject::smallest_odd_support(map)
    }

    fn close_maps(a: &Array2<f32>, b: &Array2<f32>, tolerance: f32) -> bool {
        a.dim() == b.dim() && a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= tolerance)
    }

    #[test]
    fn aligned_products_carry_their_noise_correlation_map_beside_sci_err_and_dq() {
        let dir = tempfile::tempdir().unwrap();
        let report = aligned_noise_set(dir.path());
        assert_eq!(report.reference_index, 2);
        for file in &report.files {
            let opened = File::open(&file.output_path).unwrap();
            let names: Vec<Option<String>> = list_extensions(&opened).unwrap().into_iter().map(|hdu| hdu.extname).collect();
            assert_eq!(names, [None, Some("SCI".into()), Some("ERR".into()), Some("DQ".into()), Some("NOISCORR".into())], "{}", file.output_path);
            let (hdu, header, maps) = noise_hdu(&file.output_path).unwrap();
            let k = maps[0].nrows();
            assert_eq!((hdu.naxis, maps.len(), maps[0].ncols(), k % 2), (2, 1, k, 1), "{}", file.output_path);
            assert_eq!((header.get_i64("NCORRK"), card_text(&header, "NCORRSRC").as_deref()), (Some(k as i64), Some("align")));
            assert_eq!(maps[0][[k / 2, k / 2]], 1.0);
            let factor = file.report.noiscorr_align.unwrap();
            assert!((header.get_f64("NCORRALN").unwrap() / factor - 1.0).abs() < 1e-12, "{factor}");
            assert!(header.cards.iter().any(|(key, value)| key == "HISTORY" && value.contains("NOISCORR")));
            let sci = extract_header_by_index(&opened, 1).unwrap();
            assert!(["NCORRK", "NCORRSRC", "NCORRALN"].iter().all(|key| sci.get(key).is_none()), "{:?}", sci.cards);
            assert!(!sci.cards.iter().any(|(key, value)| key == "HISTORY" && value.contains("NOISCORR")));
        }
        let shifted = noise_hdu(&report.files[1].output_path).unwrap().2.remove(0);
        assert_eq!(shifted.dim(), (3, 3));
        let interior_squares = 0.3 * 0.3 + 0.7 * 0.7;
        let axis_squares = (1.0 + 23.0 * interior_squares) / 24.0;
        let axis_cross = (0.3 + 22.0 * 0.3 * 0.7) / 23.0;
        let neighbour = axis_cross / axis_squares;
        for (index, expected) in [((1, 2), neighbour), ((2, 1), neighbour), ((1, 0), neighbour), ((2, 2), neighbour * neighbour), ((0, 2), neighbour * neighbour)] {
            assert!((f64::from(shifted[index]) - expected).abs() < 1e-5, "{index:?}: {} against {expected}", shifted[index]);
        }
        assert_eq!(noise_hdu(&report.files[2].output_path).unwrap().2[0], Array2::<f32>::ones((1, 1)));
        assert_eq!(report.files[2].report.noiscorr_align, Some(1.0));
    }

    #[test]
    fn a_mef_cube_of_aligned_images_stacks_their_noise_maps_and_writes_the_noise_columns() {
        let dir = tempfile::tempdir().unwrap();
        let aligned = aligned_noise_set(dir.path());
        let paths = aligned_paths(&aligned);
        let inputs: Vec<(Array2<f32>, f64)> = paths
            .iter()
            .map(|path| {
                let (_, header, mut maps) = noise_hdu(path).unwrap();
                (maps.remove(0), header.get_f64("NCORRALN").unwrap())
            })
            .collect();
        let cube = noise_cube(dir.path(), &aligned);
        assert_eq!(cube.hdus, ["SCI", "ERR", "DQ", "NOISCORR", "FILTERS"]);
        assert!(!cube.warnings.iter().any(|warning| warning.contains("NOISCORR")), "{:?}", cube.warnings);
        let (hdu, header, maps) = noise_hdu(&cube.output_path).unwrap();
        let k = inputs.iter().map(|(map, _)| map.nrows()).max().unwrap();
        assert_eq!((hdu.naxis, hdu.naxis1, hdu.naxis2, hdu.naxis3), (3, k as i64, k as i64, 3));
        assert_eq!((header.get_i64("NCORRK"), card_text(&header, "NCORRSRC").as_deref()), (Some(k as i64), Some("align")));
        for (z, (map, _)) in inputs.iter().enumerate() {
            let offset = (k - map.nrows()) / 2;
            for ((y, x), &value) in maps[z].indexed_iter() {
                let inside = (offset..offset + map.nrows()).contains(&y) && (offset..offset + map.ncols()).contains(&x);
                let expected = if inside { map[[y - offset, x - offset]] } else { 0.0 };
                assert!((value - expected).abs() <= f32::EPSILON, "plane {z} ({y}, {x}): {value} against {expected}");
            }
        }
        assert_eq!(table_texts(&cube.output_path, "FILTER"), ["F150W", "F277W", "F444W"]);
        let align = table_numbers(&cube.output_path, "NOISCORR_ALIGN");
        assert!(align.iter().zip(&inputs).all(|(column, (_, card))| (column / card - 1.0).abs() < 1e-12), "{align:?}");
        assert_eq!(table_numbers(&cube.output_path, "NOISCORR_MATCH"), [1.0; 3]);
        assert_eq!(table_texts(&cube.output_path, "MATCH"), ["unmatched"; 3]);
        let back = read_cube_file(&cube.output_path, &FilterRegistry::builtin()).unwrap();
        assert_eq!(back.statuses, Some(vec![MatchStatus::Unmatched; 3]));
        assert_eq!(back.noise.as_ref().map(|noise| noise.planes.len()), Some(3));

        let lightstack = build_cube(&specs_for(&paths), dir.path().join("ls.fits").to_str().unwrap(), None, CubeOptions::default(), None).unwrap();
        assert_eq!(lightstack.hdus, ["PRIMARY"]);
        assert!(!lightstack.warnings.iter().any(|warning| warning.contains("NOISCORR")), "{:?}", lightstack.warnings);
        assert!(noise_hdu(&lightstack.output_path).is_none());

        let raw_reference = noise_sources(dir.path()).remove(2);
        let mixed_specs = specs_for(&[paths[0].clone(), paths[1].clone(), raw_reference]);
        let options = CubeOptions { layout: CubeLayout::Mef, order: PlaneOrder::Wavelength };
        let mixed = build_cube(&mixed_specs, dir.path().join("mixed.fits").to_str().unwrap(), None, options, None).unwrap();
        assert_eq!(mixed.hdus, ["SCI", "ERR", "DQ", "FILTERS"]);
        assert!(mixed.warnings.iter().any(|warning| warning.contains("NOISCORR") && warning.contains("f444w_src") && !warning.contains("f150w")), "{:?}", mixed.warnings);
        assert_eq!(table_texts(&mixed.output_path, "MATCH"), ["unmatched"; 3]);
        for path in [cube.output_path.as_str(), lightstack.output_path.as_str(), mixed.output_path.as_str()] {
            release(path);
        }
    }

    #[test]
    fn edits_carry_the_noise_maps_and_columns_of_the_planes_they_keep() {
        let dir = tempfile::tempdir().unwrap();
        let aligned = aligned_noise_set(dir.path());
        let cube = noise_cube(dir.path(), &aligned);
        let (_, _, before) = noise_hdu(&cube.output_path).unwrap();
        let align = table_numbers(&cube.output_path, "NOISCORR_ALIGN");
        let out = dir.path().join("edits");

        let removed = edit_cube(&cube.output_path, &CubeEdit::RemoveFilter { filter: "F277W".into() }, "", Some(out.to_str().unwrap()), None).unwrap();
        assert_eq!(removed.hdus, ["SCI", "ERR", "DQ", "NOISCORR", "FILTERS"]);
        let (_, header, kept) = noise_hdu(&removed.output_path).unwrap();
        assert_eq!((kept.len(), card_text(&header, "NCORRSRC").as_deref()), (2, Some("align")));
        assert!(close_maps(&support(&kept[0]), &support(&before[0]), 0.0) && close_maps(&support(&kept[1]), &support(&before[2]), 0.0));
        assert_eq!(table_texts(&removed.output_path, "FILTER"), ["F150W", "F444W"]);
        assert_eq!(table_numbers(&removed.output_path, "NOISCORR_ALIGN"), [align[0], align[2]]);
        assert_eq!(table_numbers(&removed.output_path, "NOISCORR_MATCH"), [1.0; 2]);
        assert_eq!(table_texts(&removed.output_path, "MATCH"), ["unmatched"; 2]);

        let cut = edit_cube(&cube.output_path, &CubeEdit::Cut { x_start: 2, x_end: 20, y_start: 3, y_end: 21 }, "", Some(out.to_str().unwrap()), None).unwrap();
        assert_eq!(cut.hdus, ["SCI", "ERR", "DQ", "NOISCORR", "FILTERS"]);
        assert_eq!(noise_hdu(&cut.output_path).unwrap().2, before);
        assert_eq!(table_numbers(&cut.output_path, "NOISCORR_ALIGN"), align);
        assert_eq!(table_texts(&cut.output_path, "MATCH"), ["unmatched"; 3]);

        let lightstack = edit_cube(&cube.output_path, &CubeEdit::Cut { x_start: 2, x_end: 20, y_start: 3, y_end: 21 }, out.join("ls.fits").to_str().unwrap(), None, Some(CubeLayout::Lightstack)).unwrap();
        assert_eq!(lightstack.hdus, ["PRIMARY"]);
        assert!(noise_hdu(&lightstack.output_path).is_none());
        assert!(lightstack.warnings.iter().any(|warning| warning.contains("NOISCORR")), "{:?}", lightstack.warnings);
        for path in [cube.output_path.as_str(), removed.output_path.as_str(), cut.output_path.as_str(), lightstack.output_path.as_str()] {
            release(path);
        }
    }

    #[test]
    fn psf_matching_writes_match_noise_maps_and_the_kernel_cards_into_the_filters_table() {
        let dir = tempfile::tempdir().unwrap();
        let aligned = aligned_noise_set(dir.path());
        let cube = noise_cube(dir.path(), &aligned);
        let align = table_numbers(&cube.output_path, "NOISCORR_ALIGN");
        let blur = Array2::from_shape_vec((3, 3), vec![0.05, 0.1, 0.05, 0.1, 0.4, 0.1, 0.05, 0.1, 0.05]).unwrap();
        let sum_squares = blur.iter().map(|weight| weight * weight).sum::<f64>();
        let kernels = dir.path().join("kernels");
        let carded = write_kernel(&kernels, "kernel_f150w_to_f444w.fits", &blur, &[("PIXSCALE", 0.05), ("SRCFWHM", 2.0), ("TGTFWHM", 3.0), ("MATFWHM", 3.05), ("KSUMSQ", sum_squares)]);
        write_kernel(&kernels, "kernel_f277w_to_f444w.fits", &blur, &[("PIXSCALE", 0.05)]);
        let options = MatchOptions { reference: "F444W".into(), mode: MatchMode::Astroburst, allow_unmatched: false };
        let out = dir.path().join("matched");
        let report = psf_match_cube(&cube.output_path, Some(kernels.to_str().unwrap()), &[], &options, "", Some(out.to_str().unwrap()), None).unwrap();
        assert_eq!(report.hdus, ["SCI", "ERR", "DQ", "NOISCORR", "FILTERS"]);

        let source = read_cube_file(&cube.output_path, &FilterRegistry::builtin()).unwrap();
        let kernel_header = extract_header_by_index(&File::open(&carded).unwrap(), 0).unwrap();
        let plane_kernels = [
            PlaneKernel { filter: "F150W".into(), label: "kernel_f150w_to_f444w.fits".into(), data: blur.clone(), pixel_scale_arcsec: Some(0.05) },
            PlaneKernel { filter: "F277W".into(), label: "kernel_f277w_to_f444w.fits".into(), data: blur.clone(), pixel_scale_arcsec: Some(0.05) },
        ];
        let cards = [KernelCards::read(&kernel_header), KernelCards::ABSENT];
        let flags: Vec<u32> = source.cube.filters.iter().map(masked_dq_bits).collect();
        let expected = match_cube_with(&source.cube, &plane_kernels, &cards, &options, &flags, source.noise.as_ref(), None, &|| false).unwrap();
        let expected_maps = expected.noise.as_ref().unwrap().padded_maps();
        let (_, header, maps) = noise_hdu(&report.output_path).unwrap();
        assert_eq!((card_text(&header, "NCORRSRC").as_deref(), header.get_i64("NCORRK")), (Some("match"), Some(maps[0].nrows() as i64)));
        assert_eq!(maps.len(), 3);
        for (z, (written, wanted)) in maps.iter().zip(&expected_maps).enumerate() {
            assert!(close_maps(written, wanted, 1e-6), "plane {z}: {written:?} against {wanted:?}");
            assert_eq!(written[[written.nrows() / 2, written.ncols() / 2]], 1.0, "plane {z}");
        }
        assert!(maps[0].nrows() > 3, "the kernel autocorrelation widens the matched maps");
        assert!(close_maps(&support(&maps[2]), &Array2::ones((1, 1)), 0.0), "the reference keeps its alignment map");
        assert_eq!(table_texts(&report.output_path, "MATCH"), ["matched", "external", "reference"]);
        let matched = table_numbers(&report.output_path, "NOISCORR_MATCH");
        assert!((matched[0] * sum_squares.sqrt() - 1.0).abs() < 1e-12 && matched[1].is_nan() && matched[2] == 1.0, "{matched:?}");
        assert_eq!(table_numbers(&report.output_path, "NOISCORR_ALIGN"), align);
        let fwhm = table_numbers(&report.output_path, "PSF_FWHM");
        assert!((fwhm[0] / (3.05 * 0.05) - 1.0).abs() < 1e-9 && fwhm[1].is_nan() && (fwhm[2] / (3.0 * 0.05) - 1.0).abs() < 1e-9, "{fwhm:?}");
        let after = read_cube_file(&report.output_path, &FilterRegistry::builtin()).unwrap().cube;
        assert!(after.planes.iter().zip(&expected.matched.cube.planes).all(|(a, b)| a.iter().zip(b.iter()).all(|(x, y)| x.to_bits() == y.to_bits())));
        for path in [cube.output_path.as_str(), report.output_path.as_str()] {
            release(path);
        }
    }

    fn psf_spec(dir: &std::path::Path, name: &str, filter: &str, size: usize, sigma: f64) -> PsfInputSpec {
        let centre = (size / 2) as f64;
        let data = Array2::from_shape_fn((size, size), |(r, c)| (-((r as f64 - centre).powi(2) + (c as f64 - centre).powi(2)) / (2.0 * sigma * sigma)).exp());
        let data = &data / data.sum();
        PsfInputSpec { path: write_kernel(dir, name, &data, &[("PIXSCALE", 0.04)]), filter: Some(filter.into()), pixel_scale_arcsec: None }
    }

    #[test]
    fn kernel_shape_failure_carries_code_and_hint() {
        use crate::core::cubeprep::kernel::{EpsMode, KernelShape};
        use crate::math::window::TaperWindow;

        let dir = tempfile::tempdir().unwrap();
        let psfs = dir.path().join("psfs");
        let inputs = vec![psf_spec(&psfs, "stpsf_f150w.fits", "F150W", 101, 1.5), psf_spec(&psfs, "f444w.fits", "F444W", 21, 3.0)];
        let out = dir.path().join("kernels");
        let error = build_kernels(&inputs, "F444W", &KernelOptions::default(), Some(out.to_str().unwrap())).unwrap_err();
        let coded = error.downcast_ref::<CodedError>().unwrap_or_else(|| panic!("{error:#}"));
        assert_eq!(coded.code, "kernel_shape");
        for needed in ["F150W", "101x101", "21x21", "\"shape\": \"max\"", "\"shape\": [101, 101]", "pixel_scale_arcsec", "reference"] {
            assert!(coded.message.contains(needed), "{needed}: {}", coded.message);
        }
        let hint = coded.hint.as_deref().unwrap();
        assert!(coded.message.contains(hint) && hint.contains("\"shape\": \"max\"") && hint.contains("\"shape\": [101, 101]"), "{hint}");
        assert_eq!(files_in(&out), 0);

        let fixed = KernelOptions { method: KernelMethod::Wiener { eps: 0.05, eps_mode: EpsMode::Absolute, shape: KernelShape::Fixed([51, 51]) }, auto: false };
        let small = build_kernels(&inputs, "F444W", &fixed, Some(out.to_str().unwrap())).unwrap_err();
        let coded = small.downcast_ref::<CodedError>().unwrap_or_else(|| panic!("{small:#}"));
        assert_eq!(coded.code, "kernel_shape");
        assert!(coded.message.contains("\"shape\": [101, 101]") && !coded.message.contains("\"max\""), "{}", coded.message);
        assert!(coded.hint.as_deref().is_some_and(|hint| coded.message.contains(hint)), "{coded:?}");

        let window = KernelOptions { method: KernelMethod::Window { window: TaperWindow::Hanning, regularization: 1e-4 }, auto: false };
        let mismatch = build_kernels(&inputs, "F444W", &window, Some(out.to_str().unwrap())).unwrap_err();
        let coded = mismatch.downcast_ref::<CodedError>().unwrap_or_else(|| panic!("{mismatch:#}"));
        assert_eq!(coded.code, "kernel_shape");
        assert!(coded.message.contains("pixel_scale_arcsec") && coded.message.contains("\"shape\": \"max\""), "{}", coded.message);
        assert_eq!(files_in(&out), 0);

        let other = build_kernels(&inputs, "F150W", &KernelOptions::default(), None).unwrap_err();
        assert!(other.downcast_ref::<CodedError>().is_none() && format!("{other:#}").contains("narrower"), "{other:#}");
    }

    #[test]
    fn prepared_psf_reports_carry_the_clipped_negative_fractions() {
        let inputs = vec![d1_psf("d1_psf_JWST_F150W_20mas.fits"), d1_psf("d1_psf_JWST_F444W_40mas.fits")];
        let options = PsfSetOptions { reference: Some("F444W".into()), method: PsfResampleMethod::Lightstack, ..PsfSetOptions::default() };
        let report = load_psfs(&inputs, &options, None).unwrap();
        let psfs: Vec<LoadedPsf> = inputs.iter().map(|spec| load_psf(std::path::Path::new(&spec.path), None).unwrap()).collect();
        let set = prepare_psf_set(&psfs, &options).unwrap();
        let value = serde_json::to_value(&report).unwrap();
        for (z, (file, prepared)) in report.psfs.iter().zip(&set.prepared).enumerate() {
            assert_eq!((file.clipped_negative_fraction, file.clipped_negative_flux_fraction), (prepared.clipped_negative_fraction, prepared.clipped_negative_flux_fraction));
            assert_eq!(value["psfs"][z]["clipped_negative_fraction"].as_f64(), Some(prepared.clipped_negative_fraction));
            assert_eq!(value["psfs"][z]["clipped_negative_flux_fraction"].as_f64(), Some(prepared.clipped_negative_flux_fraction));
        }
    }

    #[test]
    fn a_cancel_request_during_alignment_keeps_only_the_files_already_written() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("aligned");
        let first = out.join("f150w_src_aligned_bilinear.fits");
        let error = align_set_with(&noise_sources(dir.path()), out.to_str().unwrap(), &bilinear_without_crop(), &|| first.exists()).unwrap_err();
        assert!(error.downcast_ref::<AppError>().is_some(), "{error:#}");
        assert!(first.exists());
        assert_eq!(files_in(&out), 1);
    }

    #[test]
    fn every_step_stops_at_a_cancel_request_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = dir.path().join("inputs");
        let paths = vec![filter_image(&inputs, "F150W", true), filter_image(&inputs, "F444W", true)];
        let source = mef_cube(dir.path());
        let kernels = dir.path().join("match_kernels");
        let blur = Array2::from_shape_vec((3, 3), vec![0.05, 0.1, 0.05, 0.1, 0.4, 0.1, 0.05, 0.1, 0.05]).unwrap();
        for filter in ["f150w", "f277w"] {
            write_kernel(&kernels, &format!("kernel_{filter}_to_f444w.fits"), &blur, &[("PIXSCALE", 0.04)]);
        }
        let psfs = vec![d1_psf("d1_psf_JWST_F150W_40mas.fits"), d1_psf("d1_psf_JWST_F444W_40mas.fits")];
        let target = |step: &str| dir.path().join(step).to_string_lossy().into_owned();
        let stop = || true;
        let options = MatchOptions { reference: "F444W".into(), mode: MatchMode::Astroburst, allow_unmatched: false };
        let outcomes: Vec<(&str, Option<anyhow::Error>)> = vec![
            ("crop", crop_set_with(&paths, &CropRegion::Common, CropRounding::Astroburst, &target("crop"), true, true, &stop).err()),
            ("align", align_set_with(&paths, &target("align"), &AlignOptions::default(), &stop).err()),
            ("cube", build_cube_with(&specs_for(&paths), "", Some(&target("cube")), CubeOptions::default(), None, &stop).err()),
            ("edit", edit_cube_with(&source, &CubeEdit::Cut { x_start: 0, x_end: 3, y_start: 0, y_end: 3 }, "", Some(&target("edit")), None, &stop).err()),
            ("psfs", load_psfs_with(&psfs, &PsfSetOptions::default(), Some(&target("psfs")), &stop).err()),
            ("kernels", build_kernels_with(&psfs, "F444W", &KernelOptions::default(), Some(&target("kernels")), &stop).err()),
            ("match", psf_match_cube_with(&source, Some(kernels.to_str().unwrap()), &[], &options, "", Some(&target("match")), None, &stop).err()),
        ];
        for (step, error) in outcomes {
            let error = error.unwrap_or_else(|| panic!("{step} ran to the end"));
            assert!(error.downcast_ref::<AppError>().is_some(), "{step}: {error:#}");
            assert_eq!(files_in(std::path::Path::new(&target(step))), 0, "{step} wrote a product");
        }
        release(&source);
    }

    #[test]
    fn align_set_refuses_a_bad_companion_on_the_last_input_before_writing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let paths = [
            alignment_companion_fixture(&dir, "fine.fits", 0.04),
            alignment_companion_fixture(&dir, "reference.fits", 0.08),
            companion_contract_fixture(&dir, "err_shape"),
        ];
        let out = dir.path().join("refused");
        let error = align_set(&paths, out.to_str().unwrap(), &AlignOptions::default()).unwrap_err();
        assert!(format!("{error:#}").contains("err_shape"), "{error:#}");
        assert_eq!(files_in(&out), 0);
    }

    #[test]
    fn drop_invalid_reports_the_usable_and_above_counts_of_every_plane_with_its_dataset_mask() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = dir.path().join("inputs");
        std::fs::create_dir_all(&inputs).unwrap();
        let hst = inputs.join("hst_f814w.fits");
        let mut sci_cards = wcs_cards((3.5, 3.0), 0.04, 15.0);
        sci_cards.push(("BUNIT", "'Jy'".into()));
        let mut pixels = vec![2.0f32; CUBE_COLS * CUBE_ROWS];
        pixels[7] = f32::NAN;
        let hst_dq: Vec<i32> = (0..CUBE_COLS * CUBE_ROWS).map(|i| dq_value("F814W", i % CUBE_COLS, i / CUBE_COLS)).collect();
        write_test_mef(
            &hst,
            &[("TELESCOP", "'HST'".into())],
            &[
                TestHdu { extname: Some("SCI"), extver: Some(1), cols: CUBE_COLS, rows: CUBE_ROWS, data: TestData::F32(pixels.clone()), extra_cards: sci_cards },
                TestHdu { extname: Some("DQ"), extver: Some(1), cols: CUBE_COLS, rows: CUBE_ROWS, data: TestData::I32(hst_dq.clone()), extra_cards: vec![] },
            ],
        );
        let jwst = filter_image(&inputs, "F444W", true);
        let specs = vec![
            CubeInputSpec { path: hst.to_string_lossy().into_owned(), dataset: Some("HST".into()), filter: Some("F814W".into()) },
            CubeInputSpec { path: jwst, dataset: None, filter: None },
        ];
        let options = CubeOptions { layout: CubeLayout::Mef, order: PlaneOrder::Wavelength };
        let cube = build_cube(&specs, dir.path().join("cube.fits").to_str().unwrap(), None, options, None).unwrap();
        let usable_hst = pixels.iter().zip(&hst_dq).filter(|(value, bits)| value.is_finite() && **bits as u32 & 0x3FFF == 0).count();
        let flagged_hst = pixels.iter().zip(&hst_dq).filter(|(value, bits)| value.is_finite() && **bits != 0).count();
        assert!(flagged_hst > 0 && usable_hst > 0);

        let with_dq = CubeEdit::DropInvalid { threshold: 1.0, frac_valid: 0.5, use_dq: true };
        let report = edit_cube(&cube.output_path, &with_dq, dir.path().join("with_dq.fits").to_str().unwrap(), None, None).unwrap();
        let validity = report.validity.clone().unwrap();
        let total = CUBE_COLS * CUBE_ROWS;
        assert_eq!(
            validity.iter().map(|plane| (plane.filter.as_str(), plane.pixels, plane.usable, plane.above, plane.dq_mask, plane.kept)).collect::<Vec<_>>(),
            [("F814W", total, usable_hst, usable_hst, Some(0x3FFF), true), ("F444W", total, total, total, Some(1), true)]
        );
        assert!(!report.warnings.iter().any(|warning| warning.contains("REED_SOLOMON") || warning.contains("other HST flags")), "{:?}", report.warnings);

        let strict = CubeEdit::DropInvalid { threshold: 100.0, frac_valid: 0.5, use_dq: false };
        let dropped = edit_cube(&cube.output_path, &strict, dir.path().join("strict.fits").to_str().unwrap(), None, None).unwrap();
        let validity = dropped.validity.clone().unwrap();
        assert_eq!(
            validity.iter().map(|plane| (plane.filter.as_str(), plane.usable, plane.above, plane.dq_mask, plane.kept)).collect::<Vec<_>>(),
            [("F814W", total - 1, 0, None, false), ("F444W", total, total, None, true)]
        );
        assert_eq!(dropped.removed, ["F814W"]);
        let other = edit_cube(&cube.output_path, &CubeEdit::Cut { x_start: 0, x_end: 3, y_start: 0, y_end: 3 }, dir.path().join("cut.fits").to_str().unwrap(), None, None).unwrap();
        assert!(other.validity.is_none());
        for path in [cube.output_path.as_str(), report.output_path.as_str(), dropped.output_path.as_str(), other.output_path.as_str()] {
            release(path);
        }
    }

    #[test]
    fn oversized_sets_are_refused_from_their_headers_before_any_pixel_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let huge = dir.path().join("huge.fits");
        let side = 400_000;
        let mut cards = wcs_cards((side as f64 / 2.0, side as f64 / 2.0), 0.04, 0.0);
        cards.push(("BUNIT", "'Jy'".into()));
        write_test_mef(
            &huge,
            &[("TELESCOP", "'JWST'".into()), ("INSTRUME", "'NIRCAM'".into()), ("FILTER", "'F444W'".into())],
            &[TestHdu { extname: Some("SCI"), extver: Some(1), cols: side, rows: side, data: TestData::F32(vec![0.0; 4]), extra_cards: cards }],
        );
        let paths = vec![huge.to_string_lossy().into_owned()];
        let out = dir.path().join("out");
        let refusals = [
            ("crop", crop_set(&paths, &CropRegion::Radec { ra: 150.0, dec: 2.0, size_arcsec: 1.0 }, CropRounding::Astroburst, out.to_str().unwrap(), true, true).err()),
            ("align", align_set(&paths, out.to_str().unwrap(), &AlignOptions::default()).err()),
            ("cube", build_cube(&specs_for(&paths), "", Some(out.to_str().unwrap()), CubeOptions::default(), None).err()),
        ];
        for (step, error) in refusals {
            let error = error.unwrap_or_else(|| panic!("{step} accepted a {side}x{side} image"));
            let coded = error.downcast_ref::<CodedError>().unwrap_or_else(|| panic!("{step}: {error:#}"));
            assert_eq!(coded.code, "insufficient_memory", "{step}");
            assert!(coded.message.contains(&format!("{side}x{side}")) && coded.message.contains("ASTROBURST_MEMORY_BUDGET_MB"), "{step}: {}", coded.message);
            assert!(coded.hint.as_deref().is_some_and(|hint| coded.message.contains(hint)), "{step}: {coded:?}");
        }
        assert_eq!(files_in(&out), 0);
    }

    #[test]
    fn preparation_memory_refuses_beyond_the_commit_limit_and_warns_near_the_physical_one() {
        let info = Some(MemoryInfo { physical_available: 1000, commit_available: 2000 });
        assert_eq!(check_preparation_memory(500, info, "Aligning this set", CROP_FIRST_ADVICE).unwrap(), None);
        let near = check_preparation_memory(900, info, "Aligning this set", CROP_FIRST_ADVICE).unwrap().unwrap();
        assert!(near.starts_with("Aligning this set needs about") && near.contains("page"), "{near}");
        let refused = check_preparation_memory(2001, info, "Aligning this set", CROP_FIRST_ADVICE).unwrap_err();
        let coded = refused.downcast_ref::<CodedError>().unwrap();
        assert_eq!((coded.code, coded.hint.as_deref()), ("insufficient_memory", Some(CROP_FIRST_ADVICE)));
        assert_eq!(check_preparation_memory(u64::MAX, None, "Aligning this set", CROP_FIRST_ADVICE).unwrap(), None);
    }

    #[test]
    fn an_sdss_frame_without_a_typed_gain_is_prepared_without_an_error_plane() {
        use crate::infra::sdss_frame_file::test_support::{write_frame, FRAME_XINTERP};
        use crate::infra::sdss_frame_file::SDSS_NO_ERROR_REASON;
        let dir = tempfile::tempdir().unwrap();
        let path = write_frame(dir.path(), "frame-r-000094-1-0131.fits", &FRAME_XINTERP);
        let entry = load_cached_full(&path).unwrap();
        let (loaded, _) = load_preparation_companions(&path, &entry).unwrap();
        assert!(loaded.err.is_none());
        assert_eq!(loaded.err_unavailable.as_deref(), Some(SDSS_NO_ERROR_REASON));
    }
}
