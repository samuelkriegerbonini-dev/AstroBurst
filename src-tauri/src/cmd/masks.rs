use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use anyhow::bail;
use ndarray::{s, Array2};
use rayon::prelude::*;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tauri::ipc::Response;

use crate::cmd::common::{
    blocking_cmd, cached_header, image_ref, invalidate_written, load_from_cache_or_disk, output_stem,
    resolve_output_dir, try_extract_rgb_resolved, write_derived_fits, ResolvedRgbImage, HEADER_ABPROC,
};
use crate::cmd::compose::composite_chain::key_luminance;
use crate::service::cutout::refuse_source_as_target;
use crate::cmd::helpers;
use crate::cmd::ops::{history_root_of, resolve_parent, RunGuard, Scope};
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::history::{self, HistoryDoc, MaskRef};
use crate::core::imaging::geometry::GeometryOp;
use crate::core::imaging::region::{
    physical_to_image_shape, shape_to_pixel, PhysicalMap, RegionError, RegionShape, RegionSystem,
};
use crate::core::imaging::sampling::{cell_range, preview_dims};
use crate::core::imaging::HISTORY_TEXT_BYTES;
use crate::core::mask::cache::{entry_key, file_source, MASK_CACHE};
use crate::core::mask::{
    check_dims, check_file_grid, check_regions_grid, coverage, label_image, rasterize_with_stars, recipe_hash, Grid,
    MaskError, MaskMods, MaskRecipe, MaskSpec, Rasterized, WcsSummary,
};
use crate::infra::cache::ImageEntry;
use crate::infra::fits::writer::{filter_header, write_fits_mono_int};
use crate::infra::ipc::encode_mask_with_header;
use crate::infra::wcs_source::{header_wcs, load_wcs};
use crate::types::constants::{DEFAULT_MASK_PREVIEW_DIM, RES_FITS_PATH};
use crate::types::header::HduHeader;

pub(crate) const COMPOSITE_RUN_KEY: &str = "__composite__";
const COMPOSITE_NAME: &str = "the colour composite";
const COMPOSITE_STEM: &str = "composite";
const MASK_ID_PREFIX: &str = "m";
const HEADER_ABMASK: &str = "ABMASK";
const HEADER_ABMKIND: &str = "ABMKIND";
const HEADER_HISTORY: &str = "HISTORY";
const ABPROC_MASK: &str = "mask";
const MAX_BITPIX_16_LABELS: usize = i16::MAX as usize;
const FITS_EXTENSIONS: [&str; 3] = ["fits", "fit", "fts"];

static COMPOSITE_MASKS: LazyLock<Mutex<BTreeMap<String, Value>>> = LazyLock::new(Default::default);

fn composite_masks() -> MutexGuard<'static, BTreeMap<String, Value>> {
    COMPOSITE_MASKS.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportKind {
    Float,
    Label,
}

impl ExportKind {
    fn name(self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Label => "label",
        }
    }

    fn file_suffix(self) -> &'static str {
        match self {
            Self::Float => "mask",
            Self::Label => "labels",
        }
    }
}

pub(crate) struct MaskAt<'a> {
    pub image_key: &'a str,
    pub base_path: Option<&'a str>,
    pub input_path: Option<&'a str>,
}

impl MaskAt<'_> {
    fn is_composite(&self) -> bool {
        self.image_key == COMPOSITE_RUN_KEY
    }
}

enum Planes {
    Mono(ImageEntry),
    Rgb(Box<ResolvedRgbImage>),
    Luminance(Array2<f32>),
}

impl Planes {
    fn load(path: &str) -> anyhow::Result<Self> {
        let rgb = if image_ref(path).is_synthetic() { None } else { try_extract_rgb_resolved(path)? };
        Ok(match rgb {
            Some(rgb) => Self::Rgb(Box::new(rgb)),
            None => Self::Mono(load_from_cache_or_disk(path)?),
        })
    }

    fn composite() -> anyhow::Result<(Self, u64)> {
        let (snapshot, generation) = helpers::snapshot_composite()?;
        Ok((Self::Luminance(key_luminance(&snapshot.key)?), generation))
    }

    fn refs(&self) -> Vec<&Array2<f32>> {
        match self {
            Self::Mono(entry) => vec![entry.arr()],
            Self::Rgb(rgb) => vec![&rgb.r, &rgb.g, &rgb.b],
            Self::Luminance(luminance) => vec![luminance],
        }
    }

    fn dims(&self) -> (usize, usize) {
        self.refs()[0].dim()
    }
}

fn geometry_of(header: &HduHeader) -> Option<GeometryOp> {
    header.get(HEADER_ABPROC).and_then(|value| GeometryOp::from_name(value.trim().trim_matches('\'')))
}

fn grid_of(path: &str, dims: (usize, usize)) -> anyhow::Result<Grid> {
    let header = cached_header(path)?;
    Ok(Grid {
        stem: output_stem(path),
        geometry: geometry_of(&header),
        wcs: header_wcs(&header).ok().map(|wcs| WcsSummary::of(&wcs, dims)),
    })
}

fn composite_grid() -> Grid {
    Grid { stem: COMPOSITE_NAME.to_string(), geometry: None, wcs: None }
}

fn check_grid(recipe: &MaskRecipe, root: &str, dims: (usize, usize)) -> anyhow::Result<()> {
    match recipe {
        MaskRecipe::File { path, .. } => check_file_grid(&grid_of(path, dims)?, &grid_of(root, dims)?)?,
        MaskRecipe::Regions { source_wcs, .. } => check_regions_grid(source_wcs.as_ref(), &grid_of(root, dims)?)?,
        MaskRecipe::Luminance { .. } | MaskRecipe::Range { .. } | MaskRecipe::Stars { .. } => {}
    }
    Ok(())
}

fn image_dims(planes: &[&Array2<f32>]) -> (usize, usize) {
    planes.first().map_or((0, 0), |plane| plane.dim())
}

fn rasterize_checked(mask_id: &str, spec: &MaskSpec, planes: &[&Array2<f32>]) -> Result<Rasterized, MaskError> {
    let rasterized = rasterize_with_stars(&spec.recipe, &spec.mods, planes)?;
    check_dims(mask_id, rasterized.mask.dim(), image_dims(planes))?;
    Ok(rasterized)
}

fn composite_source(generation: u64) -> String {
    format!("{COMPOSITE_RUN_KEY}@{generation}")
}

fn cached_raster(
    mask_id: &str,
    source: Option<&str>,
    spec: &MaskSpec,
    planes: &[&Array2<f32>],
) -> Result<Arc<Array2<f32>>, MaskError> {
    let raster = match source {
        Some(source) => MASK_CACHE.raster(source, &spec.recipe, &spec.mods, planes)?,
        None => Arc::new(spec.rasterize(planes)?),
    };
    check_dims(mask_id, raster.dim(), image_dims(planes))?;
    Ok(raster)
}

pub(crate) fn guarded_raster(
    doc: &HistoryDoc,
    mask: &MaskRef,
    planes: &[&Array2<f32>],
    root: &str,
    source: Option<&str>,
) -> anyhow::Result<Arc<Array2<f32>>> {
    let spec = doc.mask_spec(mask)?;
    let raster = cached_raster(mask.label(), source, &spec, planes)?;
    check_grid(&spec.recipe, root, raster.dim())?;
    Ok(raster)
}

pub(crate) fn composite_spec(mask: Option<Value>) -> anyhow::Result<Option<MaskSpec>> {
    let Some(mask) = mask else {
        return Ok(None);
    };
    let (label, recipe) = match mask {
        Value::String(id) => {
            let recipe = composite_masks().get(&id).cloned().ok_or_else(|| MaskError::Unknown(id.clone()))?;
            (format!("mask {id}"), recipe)
        }
        recipe => ("inline mask".to_string(), recipe),
    };
    let spec = MaskSpec::deserialize(&recipe).map_err(|e| MaskError::Invalid(format!("{label}: {e}")))?;
    Ok(Some(spec))
}

fn shown_input(scope: &Scope, doc: &HistoryDoc, input_path: Option<&str>) -> anyhow::Result<String> {
    match input_path.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => {
            resolve_parent(scope, doc, path)?;
            Ok(path.to_string())
        }
        None => Ok(scope.root_input().to_string()),
    }
}

fn parsed(mask_id: &str, recipe: &Value) -> Result<MaskSpec, MaskError> {
    MaskSpec::deserialize(recipe).map_err(|e| MaskError::Invalid(format!("mask {mask_id}: {e}")))
}

pub(crate) fn stored_recipe(spec: &MaskSpec) -> anyhow::Result<Value> {
    let mut flat: Map<String, Value> = serde_json::from_str(&serde_json::to_string(&spec.recipe)?)?;
    flat.extend(serde_json::from_str::<Map<String, Value>>(&serde_json::to_string(&spec.mods)?)?);
    Ok(Value::Object(flat))
}

fn mask_id_for(masks: &BTreeMap<String, Value>, spec: &MaskSpec) -> String {
    let hash = recipe_hash(&spec.recipe, &spec.mods);
    let same = masks.iter().find(|(_, stored)| {
        MaskSpec::deserialize(*stored).is_ok_and(|kept| recipe_hash(&kept.recipe, &kept.mods) == hash)
    });
    match same {
        Some((id, _)) => id.clone(),
        None => {
            let last = masks.keys().filter_map(|id| id.strip_prefix(MASK_ID_PREFIX)?.parse::<u32>().ok()).max().unwrap_or(0);
            format!("{MASK_ID_PREFIX}{:02}", last + 1)
        }
    }
}

fn users_of<'a>(doc: &'a HistoryDoc, mask_id: &str) -> Vec<&'a str> {
    doc.nodes
        .iter()
        .filter(|node| matches!(&node.mask, Some(MaskRef::Id(id)) if id == mask_id))
        .map(|node| node.id.as_str())
        .collect()
}

fn forget_unused_deleted(doc: &mut HistoryDoc) {
    let unused: Vec<String> = doc.deleted_masks.iter().filter(|id| users_of(doc, id).is_empty()).cloned().collect();
    for id in unused {
        doc.masks.remove(&id);
        doc.deleted_masks.remove(&id);
    }
}

fn in_pixels(
    role: &'static str,
    shapes: &[RegionShape],
    to_pixels: impl Fn(&RegionShape) -> Result<RegionShape, RegionError>,
) -> Result<Vec<RegionShape>, MaskError> {
    shapes
        .iter()
        .enumerate()
        .map(|(index, shape)| {
            to_pixels(shape).map_err(|source| MaskError::Region { role, index: index + 1, kind: shape.kind(), source })
        })
        .collect()
}

fn with_pixel_shapes(spec: MaskSpec, system: RegionSystem, root: &str, image: &Grid) -> anyhow::Result<MaskSpec> {
    let MaskSpec { mods, recipe } = spec;
    let MaskRecipe::Regions { shapes, exclude_shapes, source_wcs } = recipe else {
        return Ok(MaskSpec { mods, recipe });
    };
    let (shapes, exclude_shapes) = match system {
        RegionSystem::Image => (shapes, exclude_shapes),
        _ if source_wcs.is_some() => {
            bail!("this regions recipe already holds pixel shapes with a source_wcs; send it with shapes_system image")
        }
        _ => {
            let header = cached_header(root)?;
            let wcs = load_wcs(root, &header).ok();
            let physical = PhysicalMap::from_header(&header);
            let to_pixels = |shape: &RegionShape| match system {
                RegionSystem::Physical => physical_to_image_shape(shape, &physical).map(|converted| converted.from_ds9_image()),
                _ => shape_to_pixel(shape, system, wcs.as_ref()),
            };
            (in_pixels("shape", &shapes, to_pixels)?, in_pixels("exclude shape", &exclude_shapes, to_pixels)?)
        }
    };
    let source_wcs = source_wcs.or_else(|| image.wcs.clone());
    Ok(MaskSpec { mods, recipe: MaskRecipe::Regions { shapes, exclude_shapes, source_wcs } })
}

fn created(mask_id: &str, spec: &MaskSpec, rasterized: &Rasterized, recipe: Value) -> Value {
    let (rows, cols) = rasterized.mask.dim();
    json!({
        "mask_id": mask_id,
        "kind": spec.recipe.kind(),
        "coverage": coverage(&rasterized.mask),
        "dims": [cols, rows],
        "stars": rasterized.stars,
        "recipe": recipe,
    })
}

fn create_on_file(root: &Path, at: &MaskAt, spec: MaskSpec, system: RegionSystem) -> anyhow::Result<Value> {
    let scope = Scope::new(root, at.image_key, at.base_path);
    let _guard = RunGuard::acquire(&scope.dir)?;
    let mut doc = scope.load_or_new()?;
    let shown = shown_input(&scope, &doc, at.input_path)?;
    let source = file_source(&shown);
    let planes = Planes::load(&shown)?;
    let dims = planes.dims();
    let root_input = scope.root_input();
    let spec = with_pixel_shapes(spec, system, root_input, &grid_of(root_input, dims)?)?;
    forget_unused_deleted(&mut doc);
    let mask_id = mask_id_for(&doc.masks, &spec);
    let rasterized = rasterize_checked(&mask_id, &spec, &planes.refs())?;
    check_grid(&spec.recipe, root_input, dims)?;
    let stored = stored_recipe(&spec)?;
    doc.masks.insert(mask_id.clone(), stored.clone());
    doc.deleted_masks.remove(&mask_id);
    history::save(&scope.dir, &doc)?;
    let response = created(&mask_id, &spec, &rasterized, stored);
    if let Some(source) = source {
        MASK_CACHE.keep(&entry_key(&source, &spec.recipe, &spec.mods), Arc::new(rasterized.mask));
    }
    Ok(response)
}

fn create_on_composite(spec: MaskSpec, system: RegionSystem) -> anyhow::Result<Value> {
    if let MaskRecipe::Regions { source_wcs, .. } = &spec.recipe {
        if system != RegionSystem::Image {
            bail!("regions on {COMPOSITE_NAME} are drawn in image pixels; it has no WCS to place {} shapes", system.name());
        }
        check_regions_grid(source_wcs.as_ref(), &composite_grid())?;
    }
    let (planes, generation) = Planes::composite()?;
    let mut masks = composite_masks();
    let mask_id = mask_id_for(&masks, &spec);
    let rasterized = rasterize_checked(&mask_id, &spec, &planes.refs())?;
    let stored = stored_recipe(&spec)?;
    masks.insert(mask_id.clone(), stored.clone());
    let response = created(&mask_id, &spec, &rasterized, stored);
    MASK_CACHE.keep(&entry_key(&composite_source(generation), &spec.recipe, &spec.mods), Arc::new(rasterized.mask));
    Ok(response)
}

pub(crate) fn create(
    root: &Path,
    at: &MaskAt,
    recipe: &Value,
    shapes_system: Option<&str>,
    mods: Option<MaskMods>,
) -> anyhow::Result<Value> {
    let mut spec = MaskSpec::deserialize(recipe).map_err(|e| MaskError::Invalid(format!("mask recipe: {e}")))?;
    if let Some(mods) = mods {
        spec.mods = mods;
    }
    let system = shapes_system.map(RegionSystem::parse).transpose()?.unwrap_or_default();
    if at.is_composite() {
        create_on_composite(spec, system)
    } else {
        create_on_file(root, at, spec, system)
    }
}

struct Resolved {
    mask_id: String,
    spec: MaskSpec,
    recipe: Value,
    planes: Planes,
    root: Option<String>,
    source: Option<String>,
}

impl Resolved {
    fn of(root: &Path, at: &MaskAt, mask_id: &str) -> anyhow::Result<Self> {
        let unknown = || MaskError::Unknown(mask_id.to_string());
        if at.is_composite() {
            let recipe = composite_masks().get(mask_id).cloned().ok_or_else(unknown)?;
            let spec = parsed(mask_id, &recipe)?;
            let (planes, generation) = Planes::composite()?;
            let source = Some(composite_source(generation));
            return Ok(Self { mask_id: mask_id.to_string(), spec, recipe, planes, root: None, source });
        }
        let scope = Scope::new(root, at.image_key, at.base_path);
        let doc = history::load(&scope.dir)?.ok_or_else(unknown)?;
        let recipe = doc.masks.get(mask_id).cloned().ok_or_else(unknown)?;
        let spec = parsed(mask_id, &recipe)?;
        let shown = shown_input(&scope, &doc, at.input_path)?;
        let source = file_source(&shown);
        let planes = Planes::load(&shown)?;
        let root = Some(scope.root_input().to_string());
        Ok(Self { mask_id: mask_id.to_string(), spec, recipe, planes, root, source })
    }

    fn guard(&self, dims: (usize, usize)) -> anyhow::Result<()> {
        match &self.root {
            Some(root) => check_grid(&self.spec.recipe, root, dims),
            None => Ok(()),
        }
    }

    fn raster(&self) -> anyhow::Result<Arc<Array2<f32>>> {
        let raster = cached_raster(&self.mask_id, self.source.as_deref(), &self.spec, &self.planes.refs())?;
        self.guard(raster.dim())?;
        Ok(raster)
    }

    fn labels(&self) -> anyhow::Result<Array2<i32>> {
        let dims = self.planes.dims();
        let labels = label_image(&self.spec.recipe, dims)?;
        self.guard(dims)?;
        Ok(labels)
    }

    fn shape_count(&self) -> usize {
        match &self.spec.recipe {
            MaskRecipe::Regions { shapes, .. } => shapes.len(),
            _ => 0,
        }
    }

    fn read_paths(&self, at: &MaskAt) -> Vec<String> {
        let file = match &self.spec.recipe {
            MaskRecipe::File { path, .. } => Some(path.clone()),
            _ => None,
        };
        let image = (!at.is_composite()).then(|| at.image_key.to_string());
        self.root.iter().cloned().chain(image).chain(at.input_path.map(str::to_string)).chain(file).collect()
    }
}

fn preview_cells(mask: &Array2<f32>, max_dim: usize) -> (Vec<u8>, usize, usize) {
    let (rows, cols) = mask.dim();
    if rows == 0 || cols == 0 {
        return (Vec::new(), cols, rows);
    }
    let (dst_rows, dst_cols) = preview_dims(rows, cols, max_dim);
    let mut cells = vec![0u8; dst_rows * dst_cols];
    cells.par_chunks_mut(dst_cols).enumerate().for_each(|(dy, out_row)| {
        let (y0, y1) = cell_range(dy, rows, dst_rows);
        for (dx, cell) in out_row.iter_mut().enumerate() {
            let (x0, x1) = cell_range(dx, cols, dst_cols);
            let peak = mask.slice(s![y0..y1, x0..x1]).fold(0.0f32, |peak, &m| peak.max(m));
            *cell = (peak.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    });
    (cells, dst_cols, dst_rows)
}

pub(crate) fn preview(root: &Path, at: &MaskAt, mask_id: &str, max_dim: Option<u32>) -> anyhow::Result<Vec<u8>> {
    let mask = Resolved::of(root, at, mask_id)?.raster()?;
    let max_dim = max_dim.map_or(DEFAULT_MASK_PREVIEW_DIM, |dim| dim.clamp(1, DEFAULT_MASK_PREVIEW_DIM)) as usize;
    let (cells, width, height) = preview_cells(&mask, max_dim);
    Ok(encode_mask_with_header(&cells, width as u32, height as u32, 0, 0))
}

fn deleted(mask_id: &str, recipe: Value, used_by: &[String]) -> Value {
    json!({ "mask_id": mask_id, "recipe": recipe, "used_by": used_by, "kept_for_replay": !used_by.is_empty() })
}

pub(crate) fn delete(root: &Path, at: &MaskAt, mask_id: &str) -> anyhow::Result<Value> {
    let unknown = || MaskError::Unknown(mask_id.to_string());
    if at.is_composite() {
        let recipe = composite_masks().remove(mask_id).ok_or_else(unknown)?;
        return Ok(deleted(mask_id, recipe, &[]));
    }
    let scope = Scope::new(root, at.image_key, at.base_path);
    let _guard = RunGuard::acquire(&scope.dir)?;
    let mut doc = history::load(&scope.dir)?.ok_or_else(unknown)?;
    let recipe = doc.masks.get(mask_id).cloned().ok_or_else(unknown)?;
    let used_by: Vec<String> = users_of(&doc, mask_id).into_iter().map(str::to_string).collect();
    if used_by.is_empty() {
        doc.masks.remove(mask_id);
        doc.deleted_masks.remove(mask_id);
    } else {
        doc.deleted_masks.insert(mask_id.to_string());
    }
    forget_unused_deleted(&mut doc);
    history::save(&scope.dir, &doc)?;
    Ok(deleted(mask_id, recipe, &used_by))
}

fn names_a_fits_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| FITS_EXTENSIONS.iter().any(|fits| extension.eq_ignore_ascii_case(fits)))
}

fn export_target(path: &str, stem: &str, mask_id: &str, kind: ExportKind) -> anyhow::Result<String> {
    let requested = path.trim();
    if requested.is_empty() {
        bail!("no output folder or FITS path given for the mask export");
    }
    if !names_a_fits_file(Path::new(requested)) {
        let folder = resolve_output_dir(requested)?;
        return Ok(format!("{}/{stem}_{mask_id}_{}.fits", folder.trim_end_matches(['/', '\\']), kind.file_suffix()));
    }
    if let Some(folder) = Path::new(requested).parent().filter(|folder| !folder.as_os_str().is_empty()) {
        resolve_output_dir(&folder.to_string_lossy())?;
    }
    Ok(requested.to_string())
}

fn ascii_json(value: &Value) -> String {
    let mut out = String::new();
    for ch in value.to_string().chars() {
        if ch.is_ascii_graphic() {
            out.push(ch);
        } else {
            for unit in ch.encode_utf16(&mut [0; 2]).iter() {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

fn mask_header(source: Option<&HduHeader>, mask_id: &str, kind: ExportKind, recipe: &Value) -> HduHeader {
    let mut header = source.and_then(|h| filter_header(h, true, false)).unwrap_or_else(HduHeader::empty);
    if header.get("WCSAXES").is_some() {
        header.set("WCSAXES", "2".to_string());
    }
    header.set(HEADER_ABMASK, "T".to_string());
    header.set(HEADER_ABMKIND, kind.name().to_string());
    header.set(HEADER_ABPROC, source.and_then(geometry_of).map_or(ABPROC_MASK, GeometryOp::name).to_string());
    let chars: Vec<char> = ascii_json(recipe).chars().collect();
    let lines: Vec<String> = chars.chunks(HISTORY_TEXT_BYTES).map(|chunk| chunk.iter().collect()).collect();
    let marker = format!("mask {mask_id} recipe JSON in the next {} HISTORY lines", lines.len());
    header.cards.push((HEADER_HISTORY.to_string(), marker));
    header.cards.extend(lines.into_iter().map(|line| (HEADER_HISTORY.to_string(), line)));
    header
}

pub(crate) fn export(root: &Path, at: &MaskAt, mask_id: &str, path: &str, kind: ExportKind) -> anyhow::Result<Value> {
    let resolved = Resolved::of(root, at, mask_id)?;
    let source = resolved.root.as_deref().map(cached_header).transpose()?;
    let stem = resolved.root.as_deref().map_or_else(|| COMPOSITE_STEM.to_string(), output_stem);
    let target = export_target(path, &stem, mask_id, kind)?;
    for read in resolved.read_paths(at) {
        refuse_source_as_target(&target, &read)?;
    }
    let header = mask_header(source.as_ref(), mask_id, kind, &resolved.recipe);
    let (bitpix, (rows, cols)) = match kind {
        ExportKind::Float => {
            let mask = resolved.raster()?;
            write_derived_fits(&target, &mask, Some(&header))?;
            (-32, mask.dim())
        }
        ExportKind::Label => {
            let labels = resolved.labels()?;
            let bitpix = if resolved.shape_count() <= MAX_BITPIX_16_LABELS { 16 } else { 32 };
            GLOBAL_CUBE_CACHE.invalidate(&target);
            let written = write_fits_mono_int(&target, &labels, Some(&header), bitpix);
            invalidate_written(&target);
            written?;
            (bitpix, labels.dim())
        }
    };
    Ok(json!({ "mask_id": mask_id, "kind": kind.name(), RES_FITS_PATH: target, "bitpix": bitpix, "dims": [cols, rows] }))
}

#[tauri::command]
pub async fn mask_create_cmd(
    app: tauri::AppHandle,
    image_key: String,
    base_path: Option<String>,
    input_path: Option<String>,
    recipe: Value,
    shapes_system: Option<String>,
    mods: Option<MaskMods>,
) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    blocking_cmd!({
        let at = MaskAt { image_key: &image_key, base_path: base_path.as_deref(), input_path: input_path.as_deref() };
        create(&root, &at, &recipe, shapes_system.as_deref(), mods)
    })
}

#[tauri::command]
pub async fn mask_preview_cmd(
    app: tauri::AppHandle,
    image_key: String,
    base_path: Option<String>,
    input_path: Option<String>,
    mask_id: String,
    max_dim: Option<u32>,
) -> Result<Response, String> {
    let root = history_root_of(&app)?;
    tokio::task::spawn_blocking(move || {
        let at = MaskAt { image_key: &image_key, base_path: base_path.as_deref(), input_path: input_path.as_deref() };
        preview(&root, &at, &mask_id, max_dim).map(Response::new)
    })
    .await
    .map_err(|e| format!("{e}"))?
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn mask_delete_cmd(
    app: tauri::AppHandle,
    image_key: String,
    base_path: Option<String>,
    mask_id: String,
) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    blocking_cmd!({
        let at = MaskAt { image_key: &image_key, base_path: base_path.as_deref(), input_path: None };
        delete(&root, &at, &mask_id)
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn mask_export_cmd(
    app: tauri::AppHandle,
    image_key: String,
    base_path: Option<String>,
    input_path: Option<String>,
    mask_id: String,
    path: String,
    kind: ExportKind,
) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    blocking_cmd!({
        let at = MaskAt { image_key: &image_key, base_path: base_path.as_deref(), input_path: input_path.as_deref() };
        export(&root, &at, &mask_id, &path, kind)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use serde_json::json;

    use crate::cmd::common::{cached_header, extract_image_resolved};
    use crate::cmd::compose::composite_chain::key_luminance;
    use crate::cmd::helpers;
    use crate::cmd::ops::{move_head, rehydrate, rerun, run_recorded, HeadMove, RerunRequest, RunRequest};
    use crate::cmd::processing::transform_geometry_cmd;
    use crate::core::history::id::image_id;
    use crate::core::imaging::region::test_support::{header_with_cd, north_up_cd, rotated_cd};
    use crate::core::imaging::region::{shape_to_sky, RegionShape, RegionSystem};
    use crate::core::imaging::sampling::{cell_range, preview_dims};
    use crate::core::imaging::stats::compute_image_stats;
    use crate::core::mask::{
        coverage, label_image, rasterize, rasterized_on_this_thread, MarkMode, MaskRecipe, MaskSpec, WcsSummary,
    };
    use crate::core::ops::tone::{ToneCurveInput, ToneLevelsInput};
    use crate::core::ops::{OpSpec, OpVersion};
    use crate::infra::fits::writer::write_fits_mono;
    use crate::infra::wcs_source::header_wcs;
    use crate::types::constants::{DQ_MASK_HEADER_BYTES, RES_DIMENSIONS, RES_FITS_PATH};
    use crate::types::header::HduHeader;

    struct Fixture {
        app: tempfile::TempDir,
        data: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            Self { app: tempfile::tempdir().unwrap(), data: tempfile::tempdir().unwrap() }
        }

        fn root(&self) -> PathBuf {
            history::history_root(self.app.path())
        }

        fn path(&self, name: &str) -> String {
            self.data.path().join(name).to_string_lossy().into_owned()
        }

        fn fits(&self, name: &str, arr: &Array2<f32>, header: Option<&HduHeader>) -> String {
            let path = self.path(name);
            write_fits_mono(&path, arr, header).unwrap();
            path
        }

        fn out(&self) -> String {
            self.path("output")
        }

        fn doc(&self, key: &str) -> HistoryDoc {
            history::load(&history::image_dir(&self.root(), &image_id(key, None))).unwrap().expect("no sidecar was written")
        }

        fn create(&self, key: &str, recipe: &Value) -> anyhow::Result<Value> {
            create(&self.root(), &at(key), recipe, None, None)
        }

        fn run(&self, key: &str, mask: MaskRef) -> anyhow::Result<Value> {
            let request = RunRequest { image_key: key, base_path: None, input_path: key, op: tone(), psf_kernel: None, mask: Some(mask) };
            run_recorded(&self.root(), request, None)
        }
    }

    fn at(image_key: &str) -> MaskAt<'_> {
        MaskAt { image_key, base_path: None, input_path: None }
    }

    fn field(rows: usize, cols: usize, seed: u64) -> Array2<f32> {
        let mut state = seed;
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            100.0 + 0.5 * x as f32 + 0.25 * y as f32 + 8.0 * ((state >> 40) as f32 / (1u64 << 24) as f32)
        })
    }

    fn tone() -> OpSpec {
        OpSpec::Tone {
            v: OpVersion,
            levels: Some(ToneLevelsInput { black: 0.05, gamma: 1.0, white: 0.95 }),
            curve: Some(ToneCurveInput { points: vec![[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]] }),
        }
    }

    fn wcs_header() -> HduHeader {
        header_with_cd(north_up_cd())
    }

    fn circle(x: f64, y: f64, r: f64) -> RegionShape {
        RegionShape::Circle { x, y, r }
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> RegionShape {
        RegionShape::Box { x, y, width, height, angle: 0.0 }
    }

    fn spec(stored: &Value) -> MaskSpec {
        MaskSpec::deserialize(stored).unwrap()
    }

    fn text(header: &HduHeader, key: &str) -> Option<String> {
        header.get(key).map(|v| v.trim().trim_matches('\'').trim().to_string())
    }

    fn assert_bitwise(actual: &Array2<f32>, expected: &Array2<f32>, what: &str) {
        assert_eq!(actual.dim(), expected.dim(), "{what}");
        let differing = actual.iter().zip(expected.iter()).filter(|(a, e)| a.to_bits() != e.to_bits()).count();
        assert_eq!(differing, 0, "{what}: {differing} pixels differ");
    }

    fn close_json(actual: &Value, expected: &Value) -> bool {
        match (actual, expected) {
            (Value::Number(a), Value::Number(e)) => (a.as_f64().unwrap() - e.as_f64().unwrap()).abs() <= 1e-6,
            (Value::Array(a), Value::Array(e)) => a.len() == e.len() && a.iter().zip(e).all(|(a, e)| close_json(a, e)),
            (Value::Object(a), Value::Object(e)) => {
                a.len() == e.len() && a.iter().all(|(key, value)| e.get(key).is_some_and(|other| close_json(value, other)))
            }
            _ => actual == expected,
        }
    }

    fn assert_shapes_close(actual: &[RegionShape], expected: &[RegionShape]) {
        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(expected) {
            let (a, e) = (serde_json::to_value(a).unwrap(), serde_json::to_value(e).unwrap());
            assert!(close_json(&a, &e), "{a} is not within 1e-6 px of {e}");
        }
    }

    fn header_words(bytes: &[u8]) -> [u32; 4] {
        [0, 1, 2, 3].map(|i| u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap()))
    }

    fn quantised_blocks(mask: &Array2<f32>, rows: usize, cols: usize) -> Vec<u8> {
        let (src_rows, src_cols) = mask.dim();
        let mut cells = Vec::with_capacity(rows * cols);
        for dy in 0..rows {
            let (y0, y1) = cell_range(dy, src_rows, rows);
            for dx in 0..cols {
                let (x0, x1) = cell_range(dx, src_cols, cols);
                let mut peak = 0.0f32;
                for y in y0..y1 {
                    for x in x0..x1 {
                        peak = peak.max(mask[[y, x]]);
                    }
                }
                cells.push((peak * 255.0).round() as u8);
            }
        }
        cells
    }

    fn history_recipe(header: &HduHeader, mask_id: &str) -> Value {
        let lines: Vec<&str> = header.cards.iter().filter(|(k, _)| k.trim() == "HISTORY").map(|(_, v)| v.as_str()).collect();
        let marker = format!("mask {mask_id} recipe JSON in the next ");
        let at = lines.iter().position(|line| line.starts_with(&marker)).unwrap_or_else(|| panic!("no recipe marker in {lines:?}"));
        let count: usize = lines[at][marker.len()..].split(' ').next().unwrap().parse().unwrap();
        let chunks = &lines[at + 1..=at + count];
        assert!(chunks.iter().all(|line| line.len() <= 72 && line.bytes().all(|b| b.is_ascii_graphic())), "{chunks:?}");
        serde_json::from_str(&chunks.concat()).unwrap()
    }

    async fn geometry_copy(path: &str, out: &str, op: &str) -> String {
        let value = transform_geometry_cmd(path.to_string(), out.to_string(), op.to_string()).await.unwrap();
        value[RES_FITS_PATH].as_str().unwrap().to_string()
    }

    #[test]
    fn mask_preview_cmd_returns_header_plus_u8_cells_downsampled_to_2048() {
        let fx = Fixture::new();
        let (rows, cols) = (600, 4500);
        let ramp = Array2::from_shape_fn((rows, cols), |(y, x)| x as f32 / cols as f32 + 1e-4 * y as f32);
        let key = fx.fits("wide.fits", &ramp, None);
        let recipe = json!({"kind": "range", "lo": 0.3, "hi": 0.5, "fuzz": 0.2, "feather_px": 1.5});
        assert_eq!(fx.create(&key, &recipe).unwrap()["mask_id"], "m01");
        let mods = MaskMods { feather_px: 1.5, invert: false };
        let mask = rasterize(&MaskRecipe::Range { lo: 0.3, hi: 0.5, fuzz: 0.2 }, &mods, &[&ramp]).unwrap();
        let (preview_rows, preview_cols) = preview_dims(rows, cols, 2048);
        assert_eq!(preview_cols, 2048, "precondition: the long side is downsampled to 2048");
        let expected = quantised_blocks(&mask, preview_rows, preview_cols);
        assert!(expected.iter().any(|&c| c > 0 && c < 255), "precondition: feathered cells between 0 and 255");

        for requested in [None, Some(2048), Some(8192)] {
            let bytes = preview(&fx.root(), &at(&key), "m01", requested).unwrap();
            assert_eq!(header_words(&bytes), [preview_cols as u32, preview_rows as u32, 0, 0], "max_dim {requested:?}");
            assert_eq!(bytes.len(), DQ_MASK_HEADER_BYTES + preview_rows * preview_cols);
            assert!(bytes[DQ_MASK_HEADER_BYTES..] == expected[..], "max_dim {requested:?}: the cells are not the block maxima of the mask");
        }
        let small = preview(&fx.root(), &at(&key), "m01", Some(512)).unwrap();
        let (small_rows, small_cols) = preview_dims(rows, cols, 512);
        assert_eq!(header_words(&small), [small_cols as u32, small_rows as u32, 0, 0]);
        assert!(small[DQ_MASK_HEADER_BYTES..] == quantised_blocks(&mask, small_rows, small_cols)[..]);

        let tile = Array2::from_shape_fn((40, 30), |(y, x)| (x + y) as f32 / 68.0);
        let tile_key = fx.fits("tile.fits", &tile, None);
        fx.create(&tile_key, &json!({"kind": "range", "lo": 0.2, "hi": 0.6, "fuzz": 0.1})).unwrap();
        let full = preview(&fx.root(), &at(&tile_key), "m01", None).unwrap();
        let tile_mask = rasterize(&MaskRecipe::Range { lo: 0.2, hi: 0.6, fuzz: 0.1 }, &MaskMods::default(), &[&tile]).unwrap();
        assert_eq!(header_words(&full), [30, 40, 0, 0]);
        let quantised: Vec<u8> = tile_mask.iter().map(|&m| (m * 255.0).round() as u8).collect();
        assert!(full[DQ_MASK_HEADER_BYTES..] == quantised[..], "a preview within 2048 is not the full-resolution mask");

        let foreign = fx.fits("foreign.fits", &tile, None);
        let shown_elsewhere = MaskAt { input_path: Some(&foreign), ..at(&tile_key) };
        let refused = preview(&fx.root(), &shown_elsewhere, "m01", None).unwrap_err().to_string();
        assert!(refused.contains("is not a node of this image's history"), "{refused}");
        let missing = preview(&fx.root(), &at(&tile_key), "m07", None).unwrap_err().to_string();
        assert_eq!(missing, "mask m07 is not in this image's history");
    }

    #[test]
    fn mask_create_converts_fk5_shapes_to_pixels_and_records_source_wcs() {
        let fx = Fixture::new();
        let image = field(100, 100, 3);
        let key = fx.fits("m51.fits", &image, Some(&wcs_header()));
        let header = cached_header(&key).unwrap();
        let wcs = header_wcs(&header).unwrap();
        let pixel = vec![
            circle(40.0, 55.0, 8.0),
            RegionShape::Box { x: 70.0, y: 30.0, width: 12.0, height: 6.0, angle: 30.0 },
            RegionShape::Polygon { points: vec![[10.0, 10.0], [25.0, 12.0], [18.0, 30.0]] },
        ];
        let excluded = vec![circle(40.0, 55.0, 3.0)];
        let sky = |shapes: &[RegionShape]| -> Vec<RegionShape> {
            shapes.iter().map(|shape| shape_to_sky(shape, RegionSystem::Fk5, Some(&wcs)).unwrap()).collect()
        };
        let recipe = json!({"kind": "regions", "shapes": sky(&pixel), "exclude_shapes": sky(&excluded)});
        assert!(!close_json(&recipe["shapes"][0], &serde_json::to_value(&pixel[0]).unwrap()), "precondition: the request carries sky coordinates");

        let created = create(&fx.root(), &at(&key), &recipe, Some("fk5"), None).unwrap();
        assert_eq!(created["mask_id"], "m01");
        assert_eq!(created["kind"], "regions");
        assert_eq!(created["dims"], json!([100, 100]));
        let stored = fx.doc(&key).masks["m01"].clone();
        assert_eq!(created["recipe"], stored);
        let MaskRecipe::Regions { shapes, exclude_shapes, source_wcs } = spec(&stored).recipe else {
            panic!("m01 is not a regions recipe: {stored}");
        };
        assert_shapes_close(&shapes, &pixel);
        assert_shapes_close(&exclude_shapes, &excluded);
        let card = |name: &str| header.get_f64(name).unwrap();
        let expected = WcsSummary {
            crval: [card("CRVAL1"), card("CRVAL2")],
            crpix: [card("CRPIX1"), card("CRPIX2")],
            cd: [[card("CD1_1"), card("CD1_2")], [card("CD2_1"), card("CD2_2")]],
            naxis: [100, 100],
        };
        assert_eq!(source_wcs.as_ref(), Some(&expected));
        let raster = rasterize(&spec(&stored).recipe, &MaskMods::default(), &[&image]).unwrap();
        assert_eq!(created["coverage"].as_f64().unwrap(), coverage(&raster));
        assert!(coverage(&raster) > 0.0);

        let drawn = fx.create(&key, &json!({"kind": "regions", "shapes": [circle(20.0, 20.0, 5.0)]})).unwrap();
        assert_eq!(drawn["mask_id"], "m02");
        assert_eq!(drawn["recipe"]["source_wcs"], serde_json::to_value(&expected).unwrap(), "an image-pixel recipe lost the WCS summary");

        let plain = fx.fits("plain.fits", &image, None);
        let refused = create(&fx.root(), &at(&plain), &recipe, Some("fk5"), None).unwrap_err().to_string();
        assert_eq!(refused, "shape 1 (circle): sky region requires a WCS on the image, but none is present");
        let unsupported = create(&fx.root(), &at(&key), &recipe, Some("galactic"), None).unwrap_err().to_string();
        assert_eq!(unsupported, "unsupported coordinate system 'galactic'");
    }

    #[test]
    fn mask_export_writes_a_float_fits_in_0_1_with_abmask_and_the_recipe_history() {
        let fx = Fixture::new();
        let image = field(100, 100, 5);
        let key = fx.fits("ngc1300.fits", &image, Some(&wcs_header()));
        let source = cached_header(&key).unwrap();
        let shapes = json!([circle(30.0, 40.0, 12.0), circle(62.0, 50.0, 15.0)]);
        let excluded = json!([rect(46.0, 45.0, 10.0, 10.0)]);
        fx.create(&key, &json!({"kind": "regions", "shapes": shapes, "exclude_shapes": excluded, "feather_px": 2.0})).unwrap();
        let stored = fx.doc(&key).masks["m01"].clone();
        let m01 = spec(&stored);
        let expected = rasterize(&m01.recipe, &m01.mods, &[&image]).unwrap();

        let exported = export(&fx.root(), &at(&key), "m01", &fx.out(), ExportKind::Float).unwrap();
        let path = exported[RES_FITS_PATH].as_str().unwrap().to_string();
        assert!(path.ends_with("ngc1300_m01_mask.fits"), "{path}");
        assert_eq!(exported["kind"], "float");
        let written = extract_image_resolved(&path).unwrap().arr;
        assert_bitwise(&written, &expected, "exported float mask");
        assert!(written.iter().all(|m| (0.0..=1.0).contains(m)), "a value outside [0, 1]");
        let soft = written.iter().any(|&m| m > 0.0 && m < 1.0);
        assert!(written.iter().any(|&m| m == 0.0) && written.iter().any(|&m| m == 1.0) && soft, "precondition: m 0, m 1 and a feathered edge");

        let header = cached_header(&path).unwrap();
        assert_eq!(header.get_i64("BITPIX"), Some(-32));
        assert_eq!(text(&header, "ABMASK").as_deref(), Some("T"));
        assert_eq!(text(&header, "ABMKIND").as_deref(), Some("float"));
        assert_eq!(text(&header, "ABPROC").as_deref(), Some("mask"));
        for card in ["CRVAL1", "CRVAL2", "CRPIX1", "CRPIX2", "CD1_1", "CD1_2", "CD2_1", "CD2_2"] {
            assert_eq!(header.get_f64(card), source.get_f64(card), "{card} was not copied from the image");
        }
        assert_eq!(text(&header, "CTYPE1").as_deref(), Some("RA---TAN"));
        assert_eq!(history_recipe(&header, "m01"), stored);

        assert_eq!(fx.create(&key, &json!({"kind": "regions", "shapes": shapes, "exclude_shapes": excluded})).unwrap()["mask_id"], "m02");
        let named = fx.path("máscara m02.fits");
        let sharp = export(&fx.root(), &at(&key), "m02", &named, ExportKind::Float).unwrap();
        assert_eq!(sharp[RES_FITS_PATH], named);
        assert_eq!(fx.create(&key, &json!({"kind": "file", "path": named})).unwrap()["mask_id"], "m03");
        let m02 = spec(&fx.doc(&key).masks["m02"]);
        let sharp_mask = rasterize(&m02.recipe, &m02.mods, &[&image]).unwrap();
        let file = MaskRecipe::File { path: named.clone(), mark: MarkMode::NonZero, lo: None, hi: None };
        assert_bitwise(&rasterize(&file, &MaskMods::default(), &[&image]).unwrap(), &sharp_mask, "float export re-imported as a file mask");

        let file_export = export(&fx.root(), &at(&key), "m03", &fx.out(), ExportKind::Float).unwrap();
        let file_header = cached_header(file_export[RES_FITS_PATH].as_str().unwrap()).unwrap();
        assert_eq!(history_recipe(&file_header, "m03")["path"], named, "the recipe path did not survive the HISTORY lines");
    }

    #[test]
    fn mask_export_label_writes_region_ids_with_zero_outside_and_excluded() {
        let fx = Fixture::new();
        let image = field(100, 100, 7);
        let key = fx.fits("ic434.fits", &image, Some(&wcs_header()));
        let (first, second, cut) = (circle(40.0, 50.0, 15.0), circle(55.0, 50.0, 15.0), rect(47.5, 50.0, 5.0, 20.0));
        let recipe = json!({"kind": "regions", "shapes": [first, second], "exclude_shapes": [cut], "feather_px": 3.0, "invert": true});
        fx.create(&key, &recipe).unwrap();
        let stored = fx.doc(&key).masks["m01"].clone();

        let exported = export(&fx.root(), &at(&key), "m01", &fx.out(), ExportKind::Label).unwrap();
        let path = exported[RES_FITS_PATH].as_str().unwrap().to_string();
        assert!(path.ends_with("ic434_m01_labels.fits"), "{path}");
        assert_eq!(exported["kind"], "label");
        assert_eq!(exported["bitpix"], 16);
        let header = cached_header(&path).unwrap();
        assert_eq!(header.get_i64("BITPIX"), Some(16));
        assert_eq!(text(&header, "ABMASK").as_deref(), Some("T"));
        assert_eq!(text(&header, "ABMKIND").as_deref(), Some("label"));
        assert_eq!(header.get_f64("CD2_2"), cached_header(&key).unwrap().get_f64("CD2_2"));
        assert_eq!(history_recipe(&header, "m01"), stored);

        let labels = extract_image_resolved(&path).unwrap().arr;
        let expected = label_image(&spec(&stored).recipe, (100, 100)).unwrap().mapv(|label| label as f32);
        assert_bitwise(&labels, &expected, "label image");
        let mut values: Vec<i64> = labels.iter().map(|&v| v as i64).collect();
        values.sort_unstable();
        values.dedup();
        assert_eq!(values, [0, 1, 2]);
        assert!(labels.iter().all(|v| v.fract() == 0.0), "a label is not an integer");
        for y in 41..=59 {
            for x in 46..=49 {
                assert_eq!(labels[[y, x]], 0.0, "excluded pixel ({x}, {y}) carries a label");
            }
        }
        assert_eq!(labels[[50, 30]], 1.0);
        assert_eq!(labels[[50, 43]], 2.0, "the later shape does not win where the circles overlap");
        assert_eq!(labels[[50, 52]], 2.0, "the later shape does not win where the circles overlap");
        assert_eq!(labels[[50, 65]], 2.0);
        assert_eq!(labels[[5, 5]], 0.0);

        let ranged = json!({"kind": "file", "path": path, "mark": "range", "lo": 2.0, "hi": 2.0});
        assert_eq!(fx.create(&key, &ranged).unwrap()["mask_id"], "m02");
        let reimported = rasterize(&spec(&fx.doc(&key).masks["m02"]).recipe, &MaskMods::default(), &[&image]).unwrap();
        let second_minus_cut = MaskRecipe::Regions { shapes: vec![second], exclude_shapes: vec![cut], source_wcs: None };
        let expected_second = rasterize(&second_minus_cut, &MaskMods::default(), &[&image]).unwrap();
        assert_bitwise(&reimported, &expected_second, "label 2 re-imported with mark range [2, 2]");

        assert_eq!(fx.create(&key, &json!({"kind": "range", "lo": 110.0, "hi": 140.0})).unwrap()["mask_id"], "m03");
        let refused = export(&fx.root(), &at(&key), "m03", &fx.out(), ExportKind::Label).unwrap_err().to_string();
        assert_eq!(refused, "only a regions mask has a label image; this mask is range");
    }

    #[test]
    fn deleting_a_mask_used_by_a_node_keeps_its_recipe_for_replay() {
        let fx = Fixture::new();
        let key = fx.fits("m42.fits", &field(48, 64, 11), None);
        let used = json!({"kind": "range", "lo": 118.0, "hi": 132.0, "fuzz": 4.0});
        let unused = json!({"kind": "luminance", "auto_stf": true});
        assert_eq!(fx.create(&key, &used).unwrap()["mask_id"], "m01");
        assert_eq!(fx.create(&key, &unused).unwrap()["mask_id"], "m02");
        let run = fx.run(&key, MaskRef::Id("m01".to_string())).unwrap();
        let fits = run[RES_FITS_PATH].as_str().unwrap().to_string();
        let recorded = extract_image_resolved(&fits).unwrap().arr;
        let stored = fx.doc(&key).masks["m01"].clone();

        let deleted = delete(&fx.root(), &at(&key), "m01").unwrap();
        assert_eq!(deleted, json!({"mask_id": "m01", "recipe": stored, "used_by": ["n0001"], "kept_for_replay": true}));
        let doc = fx.doc(&key);
        assert_eq!(doc.masks.get("m01"), Some(&stored), "the recipe a node replays with was dropped");
        assert!(doc.deleted_masks.contains("m01"), "a deleted mask is still listed");

        std::fs::remove_file(&fits).unwrap();
        std::fs::remove_file(fits.replace(".fits", ".png")).unwrap();
        assert_eq!(rehydrate(&fx.root(), &key, None, "n0001").unwrap()["rebuilt"], json!(["n0001"]));
        assert_bitwise(&extract_image_resolved(&fits).unwrap().arr, &recorded, "n0001 replayed after its mask was deleted");

        let gone = delete(&fx.root(), &at(&key), "m02").unwrap();
        assert_eq!(gone["kept_for_replay"], false);
        assert_eq!(gone["used_by"], json!([]));
        assert_eq!(gone["recipe"]["kind"], "luminance");
        let doc = fx.doc(&key);
        assert!(!doc.masks.contains_key("m02") && !doc.deleted_masks.contains("m02"), "an unused mask was kept");

        assert_eq!(fx.create(&key, &used).unwrap()["mask_id"], "m01", "the same recipe did not bring its mask back");
        assert!(!fx.doc(&key).deleted_masks.contains("m01"));
        let missing = delete(&fx.root(), &at(&key), "m09").unwrap_err().to_string();
        assert_eq!(missing, "mask m09 is not in this image's history");
    }

    #[test]
    fn a_deleted_mask_refuses_new_runs_while_its_nodes_replay_rerun_and_show_their_mask() {
        let fx = Fixture::new();
        let key = fx.fits("m8.fits", &field(48, 64, 43), None);
        assert_eq!(fx.create(&key, &json!({"kind": "range", "lo": 118.0, "hi": 132.0, "fuzz": 4.0})).unwrap()["mask_id"], "m01");
        let run = fx.run(&key, MaskRef::Id("m01".to_string())).unwrap();
        let fits = run[RES_FITS_PATH].as_str().unwrap().to_string();
        let recorded = extract_image_resolved(&fits).unwrap().arr;
        let stored = fx.doc(&key).masks["m01"].clone();
        let head = |shown: &Value| shown["node"]["mask"].clone();

        let listed = move_head(&fx.root(), &key, None, HeadMove::To(Some("n0001"))).unwrap();
        assert_eq!(head(&listed), json!({"id": "m01", "recipe": stored, "deleted": false}), "{listed}");
        assert!(delete(&fx.root(), &at(&key), "m01").unwrap()["kept_for_replay"].as_bool().unwrap());

        let refused = fx.run(&key, MaskRef::Id("m01".to_string())).unwrap_err().to_string();
        assert_eq!(refused, "mask m01 was deleted; pick another mask or create it again");
        assert_eq!(fx.doc(&key).nodes.len(), 1, "a run naming a deleted mask was recorded");
        assert!(move_head(&fx.root(), &key, None, HeadMove::Undo).unwrap()["node"].is_null());
        let redone = move_head(&fx.root(), &key, None, HeadMove::Redo).unwrap();
        assert_eq!(head(&redone), json!({"id": "m01", "recipe": stored, "deleted": true}), "{redone}");

        std::fs::remove_file(&fits).unwrap();
        std::fs::remove_file(fits.replace(".fits", ".png")).unwrap();
        let rebuilt = rehydrate(&fx.root(), &key, None, "n0001").unwrap();
        assert_eq!(head(&rebuilt)["id"], "m01", "{rebuilt}");
        assert_bitwise(&extract_image_resolved(&fits).unwrap().arr, &recorded, "n0001 replayed after m01 was deleted");

        let request = RerunRequest { image_key: &key, base_path: None, node: "n0001", op: None, replay_downstream: false, psf_kernel: None };
        let again = rerun(&fx.root(), request, None).unwrap();
        assert_eq!(head(&again), json!({"id": "m01", "recipe": stored, "deleted": true}), "a re-run lost its mask: {again}");
        let rerun_fits = again["run"][RES_FITS_PATH].as_str().unwrap();
        assert_bitwise(&extract_image_resolved(rerun_fits).unwrap().arr, &recorded, "re-run of n0001 with its deleted mask");
        let created = again["created"][0].as_str().unwrap().to_string();
        assert_eq!(fx.doc(&key).node(&created).unwrap().mask, Some(MaskRef::Id("m01".to_string())));

        let inline = fx.run(&key, MaskRef::Recipe(stored.clone())).unwrap();
        assert_bitwise(&extract_image_resolved(inline[RES_FITS_PATH].as_str().unwrap()).unwrap().arr, &recorded, "the recipe sent inline");
        let shown = move_head(&fx.root(), &key, None, HeadMove::To(inline["node"].as_str())).unwrap();
        assert_eq!(head(&shown), json!({"id": null, "recipe": stored, "deleted": false}), "{shown}");
        let unmasked = RunRequest { image_key: &key, base_path: None, input_path: &key, op: tone(), psf_kernel: None, mask: None };
        let plain = run_recorded(&fx.root(), unmasked, None).unwrap();
        let plain_view = move_head(&fx.root(), &key, None, HeadMove::To(plain["node"].as_str())).unwrap();
        assert!(plain_view["node"]["mask"].is_null(), "{plain_view}");
    }

    #[test]
    fn mask_create_rasterises_once_for_its_previews_exports_and_runs_on_the_same_input() {
        let fx = Fixture::new();
        let image = field(48, 64, 47);
        let key = fx.fits("m16.fits", &image, None);
        let recipe = json!({"kind": "range", "lo": 118.0, "hi": 132.0, "fuzz": 4.0, "feather_px": 1.5});
        let expected = spec(&recipe).rasterize(&[&image]).unwrap();
        let start = rasterized_on_this_thread();

        let id = fx.create(&key, &recipe).unwrap()["mask_id"].as_str().unwrap().to_string();
        let full = preview(&fx.root(), &at(&key), &id, None).unwrap();
        preview(&fx.root(), &at(&key), &id, Some(16)).unwrap();
        let exported = export(&fx.root(), &at(&key), &id, &fx.out(), ExportKind::Float).unwrap();
        let run = fx.run(&key, MaskRef::Id(id.clone())).unwrap();
        assert_eq!(rasterized_on_this_thread() - start, 1, "create, two previews, a float export and a run rasterised the mask again");
        assert!(full[DQ_MASK_HEADER_BYTES..] == quantised_blocks(&expected, 48, 64)[..], "the cached preview is not the mask");
        let written = extract_image_resolved(exported[RES_FITS_PATH].as_str().unwrap()).unwrap().arr;
        assert_bitwise(&written, &expected, "float export from the cached raster");

        let node = run[RES_FITS_PATH].as_str().unwrap().to_string();
        let on_node = MaskAt { input_path: Some(&node), ..at(&key) };
        preview(&fx.root(), &on_node, &id, None).unwrap();
        preview(&fx.root(), &on_node, &id, None).unwrap();
        assert_eq!(rasterized_on_this_thread() - start, 2, "another input must be rasterised once of its own");
    }

    #[tokio::test]
    async fn file_mask_on_a_rotated_or_flipped_copy_is_refused_naming_the_geometry_step() {
        let fx = Fixture::new();
        let image = field(64, 64, 13);
        let key = fx.fits("orion.fits", &image, Some(&wcs_header()));
        let out = fx.out();
        fx.create(&key, &json!({"kind": "regions", "shapes": [circle(20.0, 30.0, 9.0)]})).unwrap();
        let exported = export(&fx.root(), &at(&key), "m01", &out, ExportKind::Float).unwrap();
        let mask_file = exported[RES_FITS_PATH].as_str().unwrap().to_string();
        let file_recipe = |path: &str| json!({"kind": "file", "path": path});
        assert_eq!(fx.create(&key, &file_recipe(&mask_file)).unwrap()["mask_id"], "m02", "the mask file was refused on its own image");

        for op in ["rot90", "rot180", "rot270", "flip_h", "flip_v"] {
            let copy = geometry_copy(&key, &out, op).await;
            let expected = format!(
                "mask file orion_m01_mask was not rotated or flipped like orion_{op} (geometry step {op} on orion_{op}, none on orion_m01_mask); redraw or re-export the mask"
            );
            assert_eq!(fx.create(&copy, &file_recipe(&mask_file)).unwrap_err().to_string(), expected);
            assert_eq!(fx.run(&copy, MaskRef::Recipe(file_recipe(&mask_file))).unwrap_err().to_string(), expected);
            let turned_mask = geometry_copy(&mask_file, &out, op).await;
            assert_eq!(fx.create(&copy, &file_recipe(&turned_mask)).unwrap()["mask_id"], "m01", "{op}: a mask file turned like the image was refused");
            fx.run(&copy, MaskRef::Id("m01".to_string())).unwrap();
        }

        let marks = image.mapv(|v| if v > 120.0 { 1.0 } else { 0.0 });
        let tilted = fx.fits("tilted.fits", &marks, Some(&header_with_cd(rotated_cd(30.0))));
        let refused = fx.create(&key, &file_recipe(&tilted)).unwrap_err().to_string();
        assert!(refused.starts_with("mask file tilted was not rotated or flipped like orion (WCS summary differs in CD: "), "{refused}");
        assert!(refused.ends_with("; redraw or re-export the mask"), "{refused}");

        let wide = fx.fits("wide.fits", &field(48, 64, 17), None);
        let wide_marks = fx.fits("wide_marks.fits", &field(48, 64, 19), None);
        let turned = geometry_copy(&wide, &out, "rot90").await;
        assert_eq!(fx.create(&turned, &file_recipe(&wide_marks)).unwrap_err().to_string(), "mask m01 is 64×48, image is 48×64");
    }

    #[tokio::test]
    async fn regions_mask_on_a_rotated_copy_is_refused_by_the_wcs_summary() {
        let fx = Fixture::new();
        let image = field(64, 64, 23);
        let key = fx.fits("rosette.fits", &image, Some(&wcs_header()));
        let out = fx.out();
        fx.create(&key, &json!({"kind": "regions", "shapes": [circle(20.0, 30.0, 9.0)], "exclude_shapes": [circle(20.0, 30.0, 3.0)]})).unwrap();
        let drawn = fx.doc(&key).masks["m01"].clone();
        assert!(drawn["source_wcs"].is_object(), "precondition: the recipe records the WCS summary of rosette");
        fx.run(&key, MaskRef::Recipe(drawn.clone())).unwrap();

        for op in ["rot90", "rot180", "flip_h"] {
            let copy = geometry_copy(&key, &out, op).await;
            let refused = fx.create(&copy, &drawn).unwrap_err().to_string();
            let start = format!("region mask was drawn on another grid than rosette_{op} (WCS summary differs in ");
            assert!(refused.starts_with(&start), "{op}: {refused}");
            assert!(refused.contains("CD ("), "{op}: {refused}");
            assert!(refused.ends_with(&format!(" px apart, over the 0.5 px limit); redraw the regions on rosette_{op}")), "{op}: {refused}");
            assert!(corner_offset(&refused) > 10.0, "{op}: {refused}");
            assert_eq!(fx.run(&copy, MaskRef::Recipe(drawn.clone())).unwrap_err().to_string(), refused);
            let copy_dir = history::image_dir(&fx.root(), &image_id(&copy, None));
            assert!(!copy_dir.join("n0001.fits").exists(), "{op}: a refused masked run was recorded");
        }

        let plain = fx.fits("rosette_plain.fits", &image, None);
        let no_wcs = fx.create(&plain, &drawn).unwrap_err().to_string();
        assert_eq!(
            no_wcs,
            "region mask was drawn on another grid than rosette_plain (rosette_plain has no WCS); redraw the regions on rosette_plain"
        );
        let crop = fx.fits("rosette_crop.fits", &field(48, 64, 29), Some(&wcs_header()));
        let cropped = fx.create(&crop, &drawn).unwrap_err().to_string();
        assert!(cropped.contains("NAXIS 64×64 on the mask, NAXIS 64×48 on rosette_crop"), "{cropped}");
    }

    fn corner_offset(refusal: &str) -> f64 {
        let before = refusal.split(" px apart").next().unwrap();
        before.rsplit(' ').next().unwrap().parse().unwrap_or_else(|e| panic!("no corner offset in {refusal}: {e}"))
    }

    fn re_solved(header: &HduHeader, crpix_shift: (f64, f64), cd_scale: f64, dec_offset_arcsec: f64) -> HduHeader {
        let wcs = header_wcs(header).unwrap();
        let card = |name: &str| header.get_f64(name).unwrap();
        let crpix = (card("CRPIX1") + crpix_shift.0, card("CRPIX2") + crpix_shift.1);
        let centre = wcs.pixel_to_world(crpix.0 - 1.0, crpix.1 - 1.0);
        let mut copy = header.clone();
        copy.set("CRPIX1", crpix.0.to_string());
        copy.set("CRPIX2", crpix.1.to_string());
        copy.set("CRVAL1", centre.ra.to_string());
        copy.set("CRVAL2", (centre.dec + dec_offset_arcsec / 3600.0).to_string());
        for name in ["CD1_1", "CD1_2", "CD2_1", "CD2_2"] {
            copy.set(name, (card(name) * cd_scale).to_string());
        }
        copy
    }

    #[test]
    fn regions_mask_on_a_re_solved_copy_is_accepted_within_half_a_pixel_at_the_corners() {
        let fx = Fixture::new();
        let image = field(64, 64, 59);
        let header = wcs_header();
        let key = fx.fits("lagoon.fits", &image, Some(&header));
        let drawn = fx.create(&key, &json!({"kind": "regions", "shapes": [circle(20.0, 30.0, 9.0)]})).unwrap()["recipe"].clone();
        assert!(drawn["source_wcs"].is_object(), "precondition: the recipe records the WCS summary of lagoon");

        let solved_again = fx.fits("lagoon_solved.fits", &image, Some(&re_solved(&header, (3.0, -2.0), 1.0 + 2e-6, 0.1)));
        let summary = |path: &str| WcsSummary::of(&header_wcs(&cached_header(path).unwrap()).unwrap(), (64, 64));
        assert!(summary(&solved_again).mismatch(&summary(&key), "copy", "lagoon").is_some(), "precondition: the strict summaries differ");
        let accepted = fx.create(&solved_again, &drawn).unwrap();
        assert_eq!(accepted["mask_id"], "m01");
        assert_eq!(accepted["recipe"], drawn, "the recipe must keep its shapes and the WCS it was drawn on");
        let run = fx.run(&solved_again, MaskRef::Recipe(drawn.clone())).unwrap();
        assert_eq!(run[RES_DIMENSIONS], json!([64, 64]));

        let shifted = fx.fits("lagoon_shifted.fits", &image, Some(&re_solved(&header, (3.0, -2.0), 1.0, 0.8)));
        let refused = fx.create(&shifted, &drawn).unwrap_err().to_string();
        assert!(refused.starts_with("region mask was drawn on another grid than lagoon_shifted (WCS summary differs in "), "{refused}");
        assert!(refused.ends_with(" px apart, over the 0.5 px limit); redraw the regions on lagoon_shifted"), "{refused}");
        let offset = corner_offset(&refused);
        assert!((0.75..=0.85).contains(&offset), "a 0.8 arcsec shift at 1 arcsec/px is {offset} px at the corners: {refused}");
        assert_eq!(fx.run(&shifted, MaskRef::Recipe(drawn)).unwrap_err().to_string(), refused);
    }

    #[tokio::test]
    async fn composite_masks_are_rasterised_from_the_key_luminance_for_the_session() {
        let _guard = helpers::composite_test_lock().await;
        composite_masks().clear();
        let planes = [field(40, 56, 31), field(40, 56, 37), field(40, 56, 41)];
        let stats = [&planes[0], &planes[1], &planes[2]].map(compute_image_stats);
        let [r, g, b] = planes.clone();
        let [stats_r, stats_g, stats_b] = stats.clone();
        helpers::insert_composite_and_orig(r, g, b, stats_r, stats_g, stats_b);
        let fx = Fixture::new();
        let composite = at(COMPOSITE_RUN_KEY);

        let start = rasterized_on_this_thread();
        let created = create(&fx.root(), &composite, &json!({"kind": "range", "lo": 118.0, "hi": 128.0, "fuzz": 3.0}), None, None).unwrap();
        assert_eq!(created["mask_id"], "m01");
        assert_eq!(created["dims"], json!([56, 40]));
        let bytes = preview(&fx.root(), &composite, "m01", None).unwrap();
        let exported = export(&fx.root(), &composite, "m01", &fx.out(), ExportKind::Float).unwrap();
        assert_eq!(rasterized_on_this_thread() - start, 1, "create, preview and export on the composite rasterised the mask again");
        let luminance = key_luminance(&helpers::snapshot_composite().unwrap().0.key).unwrap();
        let expected = rasterize(&MaskRecipe::Range { lo: 118.0, hi: 128.0, fuzz: 3.0 }, &MaskMods::default(), &[&luminance]).unwrap();
        assert_eq!(created["coverage"].as_f64().unwrap(), coverage(&expected));
        assert_eq!(header_words(&bytes), [56, 40, 0, 0]);
        assert!(bytes[DQ_MASK_HEADER_BYTES..] == quantised_blocks(&expected, 40, 56)[..]);
        assert_bitwise(&extract_image_resolved(exported[RES_FITS_PATH].as_str().unwrap()).unwrap().arr, &expected, "composite float export");
        assert!(!fx.root().exists(), "a composite mask wrote a history sidecar");
        let [r, g, b] = planes.clone();
        let [stats_r, stats_g, stats_b] = stats.clone();
        helpers::insert_composite_and_orig(r, g, b, stats_r, stats_g, stats_b);
        let counted = rasterized_on_this_thread();
        assert!(preview(&fx.root(), &composite, "m01", None).unwrap()[DQ_MASK_HEADER_BYTES..] == quantised_blocks(&expected, 40, 56)[..]);
        assert_eq!(rasterized_on_this_thread() - counted, 1, "a new composite generation must not reuse the raster of the old one");

        let sky = json!({"kind": "regions", "shapes": [circle(150.0, 2.0, 30.0)]});
        let refused = create(&fx.root(), &composite, &sky, Some("fk5"), None).unwrap_err().to_string();
        assert!(refused.contains("colour composite"), "{refused}");
        assert_eq!(delete(&fx.root(), &composite, "m01").unwrap()["kept_for_replay"], false);
        let gone = preview(&fx.root(), &composite, "m01", None).unwrap_err().to_string();
        assert_eq!(gone, "mask m01 is not in this image's history");
        composite_masks().clear();
    }

    #[test]
    fn label_images_beyond_bitpix_16_are_written_exactly_as_bitpix_32() {
        let fx = Fixture::new();
        let labels = Array2::from_shape_fn((6, 9), |(y, x)| match (y + x) % 4 {
            0 => 0,
            1 => 1,
            2 => 32_768,
            _ => 70_001,
        });
        let wide = fx.path("wide_labels.fits");
        write_fits_mono_int(&wide, &labels, None, 32).unwrap();
        assert_eq!(cached_header(&wide).unwrap().get_i64("BITPIX"), Some(32));
        assert_bitwise(&extract_image_resolved(&wide).unwrap().arr, &labels.mapv(|label| label as f32), "BITPIX 32 labels");

        let narrow = fx.path("narrow_labels.fits");
        let refused = write_fits_mono_int(&narrow, &labels, None, 16).unwrap_err().to_string();
        assert_eq!(refused, "a value does not fit BITPIX 16; write this image with BITPIX 32");
        let small = labels.mapv(|label| label.min(32_767));
        write_fits_mono_int(&narrow, &small, None, 16).unwrap();
        assert_bitwise(&extract_image_resolved(&narrow).unwrap().arr, &small.mapv(|label| label as f32), "BITPIX 16 labels");
        let bitpix_8 = write_fits_mono_int(&fx.path("byte_labels.fits"), &small, None, 8).unwrap_err().to_string();
        assert_eq!(bitpix_8, "an integer image is written with BITPIX 16 or 32, not 8");
    }
}
