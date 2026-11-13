use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::{LazyLock, PoisonError, RwLock};

use anyhow::{bail, Context, Result};
use ndarray::Array2;

use crate::core::imaging::dq_flags::{DqTable, FlagSet};
use crate::core::imaging::layer_roles::{classify_layer, sigma_with_no_data, LayerGuess, LayerRole};
use crate::core::imaging::stats::compute_image_stats;
use crate::infra::asdf::converter::{auto_data_key, is_asdf_file, AsdfImage};
use crate::infra::asdf::AsdfFile;
use crate::infra::asdf_bridge::{
    companion_key, extract_plane_from_asdf, extract_plane_from_open_asdf, list_asdf_arrays,
};
use crate::infra::cache::{ImageCache, ImageEntry, PlaneLoad};
use crate::infra::fits::band_planes::band_plane_of_path;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::gzip::SharedTempDir;
use crate::infra::fits::reader::{
    auto_hdu_index, extract_header_by_index, extract_header_by_index_merged, extract_header_mmap,
    extract_image_mmap_by_index, extract_int_plane_by_index, list_extensions, HduInfo,
};
use crate::infra::sdss_frame_file::{
    looks_like_sdss_frame, sdss_frame_file_refusal, sdss_frame_sigma_plane, sdss_sigma_ref, CARD_SDSS_DARK_VARIANCE,
    CARD_SDSS_GAIN, SDSS_NO_ERROR_REASON,
};
use crate::infra::sibling_files::{grid_report, sibling_candidates, SiblingCandidate};
use crate::types::constants::{EXTNAME_DQ, EXTNAME_ERR, EXTNAME_WAVELENGTH, HEADER_BUNIT};
use crate::types::image::IntPlane;
use crate::types::image_ref::{path_key, BandPlaneKey, ImageRef, PlaneSelector};
use crate::types::HduHeader;

pub const CALIB_PATTERNS: &[&str] = &[
    "distortion", "filteroffset", "sirskernel", "photom",
    "flat", "dark", "bias", "readnoise", "gain", "linearity",
    "saturation", "superbias", "ipc", "area", "specwcs",
    "regions", "wavelengthrange", "trappars", "mask",
    "drizpars", "throughput", "psfmask",
];

pub fn is_calib_ref_asdf(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.starts_with("jwst_")
        && name.ends_with(".asdf")
        && CALIB_PATTERNS.iter().any(|p| name.contains(p))
}

pub fn bail_if_calib(path: &Path) -> Result<()> {
    if is_calib_ref_asdf(path) {
        bail!(
            "Calibration reference file (no image data): {}",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown")
        );
    }
    Ok(())
}

pub const CARD_NO_DATA: &str = "ABNODATA";

const DEFAULT_EXTVER: i64 = 1;
const EXTNAME_BASIS_PREFIX: &str = "EXTNAME ";
const ERR_HDU_BASIS: &str = "EXTNAME ERR";
const DQ_HDU_BASIS: &str = "EXTNAME DQ";
const ASDF_BASIS_PREFIX: &str = "ASDF array";
const ASDF_ERR_BASIS: &str = "ASDF array err";
const ASDF_DQ_BASIS: &str = "ASDF array dq";
const ERROR_LAYER: &str = "error";
const MASK_LAYER: &str = "mask";
const WAVELENGTH_LAYER: &str = "wavelength";
const FLOAT_PLANE: &str = "floating-point plane";
const WIDE_INTEGER_PLANE: &str = "64-bit integer plane: not supported";
const UNNAMED_FLAGS_NOTE: &str = "flag layer without bit names: nothing is excluded until bits are chosen";
const ERROR_SWITCHED_OFF: &str = "error layer switched off by the user";
const MASK_SWITCHED_OFF: &str = "mask layer switched off by the user";
pub const USER_ASSIGNMENT_BASIS: &str = "user assignment";
const RECIPE_CONVERSION: &str = "rebuilt from the calibration vector and the sky table";
const NOT_HDU_0: &str = "the shown plane is not HDU 0";
const NOT_A_SCIENCE_PLANE: &str = "the shown plane is not a science image";
const SIGMA_ROLES: [LayerRole; 3] = [LayerRole::Sigma, LayerRole::Variance, LayerRole::InverseVariance];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionOrigin {
    SameFile,
    Sibling,
    User,
    #[serde(rename = "user")]
    UserSameFile,
    Recipe,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CompanionChoice {
    pub role: LayerRole,
    pub basis: String,
    pub assumed: bool,
    pub origin: CompanionOrigin,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ErrRecipe {
    SdssFrame { gain: Option<f64>, dark_variance: Option<f64> },
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Pairing {
    pub layer: Option<LayerGuess>,
    pub err: Option<CompanionChoice>,
    pub dq: Option<CompanionChoice>,
    pub err_unavailable: Option<String>,
    pub dq_unavailable: Option<String>,
    pub int_unavailable: Option<String>,
    pub exclusion_override: Option<u32>,
    pub notes: Vec<String>,
    pub err_recipe: Option<ErrRecipe>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PlaneInfo {
    pub kind: PlaneSelector,
    pub extname: Option<String>,
    pub extver: Option<i64>,
    pub is_dq: bool,
    pub is_err: bool,
    pub pairing: Pairing,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ErrOrigin {
    pub role: LayerRole,
    pub source: String,
    pub assumed: bool,
    pub n_no_data: u64,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct LayerRefusal(String);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Companions {
    pub dq: Option<ImageRef>,
    pub err: Option<ImageRef>,
    pub wavelength: Option<ImageRef>,
}

pub struct LoadedPlane {
    pub arr: Array2<f32>,
    pub header: HduHeader,
    pub info: PlaneInfo,
    pub int_plane: Option<IntPlane>,
    pub companions: Companions,
    pub _tmp: Option<SharedTempDir>,
}

impl LoadedPlane {
    pub fn into_plane_load(self) -> PlaneLoad {
        let stats = compute_image_stats(&self.arr);
        PlaneLoad {
            arr: self.arr,
            stats,
            header: self.header,
            int_plane: self.int_plane,
            info: Some(self.info),
            companions: Some(self.companions),
        }
    }
}

#[derive(Default)]
pub struct LoadedCompanions {
    pub dq: Option<(ImageEntry, DqTable)>,
    pub err: Option<ImageEntry>,
    pub wavelength: Option<ImageEntry>,
    pub err_origin: Option<ErrOrigin>,
    pub dq_origin: Option<String>,
    pub err_unavailable: Option<String>,
    pub dq_unavailable: Option<String>,
    pub flags: Option<FlagSet>,
}

impl LoadedCompanions {
    pub fn exclusion_mask(&self) -> u32 {
        self.flags.as_ref().map_or(0, |flags| flags.exclusion_mask)
    }

    pub fn saturated_mask(&self) -> u32 {
        self.flags.as_ref().map_or(0, |flags| flags.saturated_mask)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ErrAssignment {
    Layer { source: ImageRef, role: LayerRole, basis: String, assumed: bool },
    Off,
    SdssFrame { gain: f64, dark_variance: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DqAssignment {
    Layer { source: ImageRef },
    Off,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CompanionAssignment {
    pub err: Option<ErrAssignment>,
    pub dq: Option<DqAssignment>,
    pub exclusion: Option<u32>,
}

static ASSIGNMENTS: LazyLock<RwLock<HashMap<String, CompanionAssignment>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn assignment_key(path: &str, kind: &PlaneSelector) -> String {
    let plane = ImageRef { path: path.to_string(), plane: kind.clone() }.fragment().unwrap_or_default();
    format!("{}#{plane}", path_key(path))
}

pub fn assignment(key: &str) -> Option<CompanionAssignment> {
    ASSIGNMENTS.read().unwrap_or_else(PoisonError::into_inner).get(key).cloned()
}

pub fn edit_assignment(key: &str, edit: impl FnOnce(&mut CompanionAssignment)) {
    let mut assignments = ASSIGNMENTS.write().unwrap_or_else(PoisonError::into_inner);
    let mut entry = assignments.remove(key).unwrap_or_default();
    edit(&mut entry);
    if entry != CompanionAssignment::default() {
        assignments.insert(key.to_string(), entry);
    }
}

fn same_plane(a: &ImageRef, b: &ImageRef) -> bool {
    a.plane == b.plane && path_key(&a.path) == path_key(&b.path)
}

pub fn is_assigned_mask(source: &ImageRef) -> bool {
    ASSIGNMENTS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .any(|assigned| matches!(&assigned.dq, Some(DqAssignment::Layer { source: mask }) if same_plane(mask, source)))
}

pub fn user_layer_name(source: &ImageRef) -> String {
    layer_place(source, CompanionOrigin::User)
}

fn file_of(path: &str) -> String {
    band_plane_of_path(path).map_or_else(|| path.to_string(), |band| band.file)
}

fn user_origin(image_path: &str, source: &ImageRef) -> CompanionOrigin {
    if path_key(&file_of(&source.path)) == path_key(&file_of(image_path)) {
        CompanionOrigin::UserSameFile
    } else {
        CompanionOrigin::User
    }
}

fn checked_user_layer(kind: &str, source: &ImageRef, image: &HduHeader, notes: &mut Vec<String>) -> Result<HduHeader, String> {
    let name = user_layer_name(source);
    let layer = load_plane_header(source).map_err(|error| format!("{kind} layer {name} could not be read: {error:#}"))?;
    let report = grid_report(image, &layer);
    if let Some(refusal) = report.refusal(&format!("{kind} layer"), &name) {
        return Err(refusal);
    }
    notes.extend(report.warnings.iter().map(|warning| format!("{warning} (user assignment {name})")));
    Ok(layer)
}

fn recipe_refusal(file: &File, exts: &[HduInfo], idx: usize) -> String {
    sdss_frame_file_refusal(file, exts)
        .unwrap_or_else(|| if idx == 0 { NOT_A_SCIENCE_PLANE } else { NOT_HDU_0 }.to_string())
}

pub fn sdss_recipe_refusal(r: &ImageRef) -> Result<String> {
    let (source, _decompressed) = resolve_single_image(&r.path)?;
    let file = File::open(&source).with_context(|| format!("Failed to open {}", source.display()))?;
    let idx = match &r.plane {
        PlaneSelector::Hdu(n) => *n,
        _ => auto_hdu_index(&file)?,
    };
    Ok(recipe_refusal(&file, &list_extensions(&file)?, idx))
}

fn recipe_choice(gain: f64, dark_variance: f64) -> CompanionChoice {
    CompanionChoice {
        role: LayerRole::Sigma,
        basis: format!("SDSS frame recipe (gain {gain} e-/DN, dark variance {dark_variance} DN^2, typed by the user)"),
        assumed: false,
        origin: CompanionOrigin::Recipe,
    }
}

fn apply_assignment(
    file: &File,
    ref_path: &str,
    exts: &[HduInfo],
    idx: usize,
    image: &HduHeader,
    companions: &mut Companions,
    pairing: &mut Pairing,
) {
    let Some(assigned) = assignment(&assignment_key(ref_path, &PlaneSelector::Hdu(idx))) else {
        return;
    };
    if let Some(err) = assigned.err {
        companions.err = None;
        pairing.err = None;
        pairing.err_unavailable = Some(ERROR_SWITCHED_OFF.to_string());
        match err {
            ErrAssignment::Layer { source, role, basis, assumed } => {
                match checked_user_layer(ERROR_LAYER, &source, image, &mut pairing.notes) {
                    Ok(_) => {
                        pairing.err_unavailable = None;
                        pairing.err = Some(CompanionChoice { role, basis, assumed, origin: user_origin(ref_path, &source) });
                        companions.err = Some(source);
                    }
                    Err(reason) => pairing.err_unavailable = Some(reason),
                }
            }
            ErrAssignment::SdssFrame { gain, dark_variance } if pairing.err_recipe.is_some() => {
                pairing.err_unavailable = None;
                pairing.err_recipe = Some(ErrRecipe::SdssFrame { gain: Some(gain), dark_variance: Some(dark_variance) });
                pairing.err = Some(recipe_choice(gain, dark_variance));
                companions.err = Some(sdss_sigma_ref(ref_path, gain, dark_variance));
            }
            ErrAssignment::SdssFrame { .. } => {
                pairing.err_unavailable =
                    Some(format!("the SDSS frame recipe was assigned but {}", recipe_refusal(file, exts, idx)));
            }
            ErrAssignment::Off => {}
        }
    }
    if let Some(dq) = assigned.dq {
        companions.dq = None;
        pairing.dq = None;
        pairing.dq_unavailable = Some(MASK_SWITCHED_OFF.to_string());
        pairing.notes.retain(|note| note != UNNAMED_FLAGS_NOTE);
        if let DqAssignment::Layer { source } = dq {
            match checked_user_layer(MASK_LAYER, &source, image, &mut pairing.notes) {
                Ok(layer) => {
                    if lacks_flag_names(&layer, image) {
                        pairing.notes.push(UNNAMED_FLAGS_NOTE.to_string());
                    }
                    pairing.dq_unavailable = None;
                    pairing.dq = Some(CompanionChoice {
                        role: LayerRole::Mask,
                        basis: USER_ASSIGNMENT_BASIS.to_string(),
                        assumed: false,
                        origin: user_origin(ref_path, &source),
                    });
                    companions.dq = Some(source);
                }
                Err(reason) => pairing.dq_unavailable = Some(reason),
            }
        }
    }
    if let Some(mask) = assigned.exclusion {
        pairing.exclusion_override = Some(mask);
        if mask != 0 {
            pairing.notes.retain(|note| note != UNNAMED_FLAGS_NOTE);
        }
    }
}

fn last_segment(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

pub fn is_dq_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.eq_ignore_ascii_case(EXTNAME_DQ)
        || trimmed.eq_ignore_ascii_case("MASK")
        || last_segment(trimmed).eq_ignore_ascii_case("dq")
}

pub fn is_err_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.eq_ignore_ascii_case(EXTNAME_ERR) || last_segment(trimmed).eq_ignore_ascii_case("err")
}

pub fn is_wavelength_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.eq_ignore_ascii_case(EXTNAME_WAVELENGTH)
        || last_segment(trimmed).eq_ignore_ascii_case("wavelength")
}

fn plane_info_for(kind: PlaneSelector, ext: Option<&HduInfo>) -> PlaneInfo {
    let extname = ext.and_then(|e| e.extname.clone());
    let name = extname.as_deref().unwrap_or("");
    PlaneInfo {
        kind,
        is_dq: is_dq_name(name),
        is_err: is_err_name(name),
        extver: ext.and_then(|e| e.extver),
        extname,
        pairing: Pairing::default(),
    }
}

fn missing_asdf_data(e: &anyhow::Error) -> bool {
    e.to_string().contains("Missing field: data array")
}

fn companion_fits_path(source: &Path) -> Option<std::path::PathBuf> {
    let fits = source.with_extension("fits");
    fits.exists().then_some(fits)
}

fn holds_single_plane(info: &HduInfo) -> bool {
    info.has_data && (info.naxis == 2 || (info.naxis == 3 && info.naxis3 == 1))
}

fn file_name_of(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

fn effective_extver(info: &HduInfo) -> i64 {
    info.extver.unwrap_or(DEFAULT_EXTVER)
}

fn same_file_choice(role: LayerRole, basis: &str, assumed: bool) -> CompanionChoice {
    CompanionChoice { role, basis: basis.to_string(), assumed, origin: CompanionOrigin::SameFile }
}

fn guessed_choice(guess: &LayerGuess) -> CompanionChoice {
    same_file_choice(guess.role, &guess.basis, guess.assumed)
}

fn labelled(place: &str, basis: Option<&str>) -> String {
    match basis {
        Some(basis) => format!("{place} ({})", basis.strip_prefix(EXTNAME_BASIS_PREFIX).unwrap_or(basis)),
        None => place.to_string(),
    }
}

fn shape_refusal<T: std::fmt::Display>(kind: &str, place: &str, layer: (T, T), image: (T, T)) -> String {
    format!(
        "{kind} layer {place} is {}x{} and does not match the image {}x{}: not used",
        layer.0, layer.1, image.0, image.1
    )
}

fn assumed_quantity(role: LayerRole, assumed: bool) -> Option<(&'static str, &'static str)> {
    match role {
        LayerRole::InverseVariance if assumed => Some(("an", "inverse variance")),
        LayerRole::Variance if assumed => Some(("a", "variance")),
        _ => None,
    }
}

fn named_candidate<'a>(candidates: &[&'a HduInfo], name: &str) -> Option<&'a HduInfo> {
    candidates
        .iter()
        .copied()
        .find(|e| e.extname.as_deref().is_some_and(|n| n.trim().eq_ignore_ascii_case(name)))
}

fn candidate_roles<'a>(
    file: &File,
    layer_file: &str,
    candidates: &[&'a HduInfo],
) -> Result<Vec<(&'a HduInfo, LayerGuess)>> {
    candidates
        .iter()
        .map(|hdu| Ok((*hdu, classify_layer(&extract_header_by_index(file, hdu.index)?, layer_file))))
        .collect()
}

fn first_of_roles<'a>(roles: &[(&'a HduInfo, LayerGuess)], wanted: &[LayerRole]) -> Option<(&'a HduInfo, CompanionChoice)> {
    wanted.iter().find_map(|role| {
        roles.iter().find(|(_, guess)| guess.role == *role).map(|(hdu, guess)| (*hdu, guessed_choice(guess)))
    })
}

fn assumed_layer_note(hdu: &HduInfo, choice: &CompanionChoice) -> Option<String> {
    assumed_quantity(choice.role, choice.assumed).map(|(article, quantity)| {
        format!("error layer HDU {}: {} is assumed to be {article} {quantity}", hdu.index, choice.basis)
    })
}

fn names_no_flags(file: &File, image: &HduHeader, image_index: usize, dq_index: usize) -> Result<bool> {
    if DqTable::select(image) != DqTable::Unknown {
        return Ok(false);
    }
    let read;
    let mask = if dq_index == image_index {
        image
    } else {
        read = extract_header_by_index_merged(file, dq_index)?;
        &read
    };
    Ok(lacks_flag_names(mask, image))
}

fn lacks_flag_names(mask: &HduHeader, image: &HduHeader) -> bool {
    if DqTable::select(image) != DqTable::Unknown {
        return false;
    }
    let flags = FlagSet::from_headers(Some(mask), Some(image), DqTable::select(mask));
    !flags.is_named() && flags.exclusion_default == 0
}

pub fn inexact_bitpix(bitpix: i64) -> Option<String> {
    match bitpix {
        -32 | -64 => Some(format!("{FLOAT_PLANE} (BITPIX {bitpix}): flag bits cannot be read exactly")),
        64 => Some(WIDE_INTEGER_PLANE.to_string()),
        _ => None,
    }
}

fn inexact_mask(place: &str, reason: Option<&str>) -> String {
    match reason {
        Some(kind) if kind.starts_with(FLOAT_PLANE) || kind == WIDE_INTEGER_PLANE => {
            format!("mask layer {place} is a {kind}")
        }
        Some(reason) => format!("mask layer {place} has no exact integer form: {reason}"),
        None => format!("mask layer {place} has no exact integer form"),
    }
}

pub fn layer_absent_without_fault(reason: &str) -> bool {
    matches!(reason, ERROR_SWITCHED_OFF | MASK_SWITCHED_OFF | SDSS_NO_ERROR_REASON)
}

fn is_mask_layer(pairing: &Pairing) -> bool {
    pairing.layer.as_ref().is_some_and(|layer| layer.role == LayerRole::Mask)
}

fn other_shape(kind: &str, active: &HduInfo, hdu: &HduInfo, basis: &str) -> Option<String> {
    let (layer, image) = ((hdu.naxis1, hdu.naxis2), (active.naxis1, active.naxis2));
    (layer != image).then(|| shape_refusal(kind, &labelled(&format!("HDU {}", hdu.index), Some(basis)), layer, image))
}

fn band_plane_pairing(band: &BandPlaneKey, header: &HduHeader) -> Result<(Companions, Pairing)> {
    let (source, _decompressed) = resolve_single_image(&band.file)?;
    let file = File::open(&source).with_context(|| format!("Failed to open {}", source.display()))?;
    let exts = list_extensions(&file)?;
    let cube = exts
        .get(band.hdu)
        .with_context(|| format!("HDU index {} out of range (file has {} HDUs)", band.hdu, exts.len()))?;
    let layer_file = file_name_of(Path::new(&band.file));
    let layer = classify_layer(header, layer_file);
    let mut companions = Companions::default();
    let mut pairing = Pairing::default();
    if layer.role == LayerRole::Science {
        let others: Vec<&HduInfo> =
            exts.iter().filter(|e| e.index != band.hdu && effective_extver(e) == effective_extver(cube)).collect();
        let band_cubes: Vec<&HduInfo> =
            others.iter().copied().filter(|e| e.has_data && e.naxis == 3 && e.naxis3 == cube.naxis3).collect();
        let error_layer = match named_candidate(&band_cubes, EXTNAME_ERR) {
            Some(hdu) => Some((hdu, same_file_choice(LayerRole::Sigma, ERR_HDU_BASIS, false))),
            None => first_of_roles(&candidate_roles(&file, layer_file, &band_cubes)?, &SIGMA_ROLES),
        };
        if let Some((hdu, choice)) = error_layer {
            match other_shape(ERROR_LAYER, cube, hdu, &choice.basis) {
                Some(reason) => pairing.err_unavailable = Some(reason),
                None => {
                    pairing.notes.extend(assumed_layer_note(hdu, &choice));
                    companions.err = Some(ImageRef::auto(&BandPlaneKey::new(&band.file, hdu.index, band.plane).key()));
                    pairing.err = Some(choice);
                }
            }
        }
        let planes: Vec<&HduInfo> = others.iter().copied().filter(|e| holds_single_plane(e)).collect();
        let mask_layer = match named_candidate(&planes, EXTNAME_DQ) {
            Some(hdu) => Some((hdu, same_file_choice(LayerRole::Mask, DQ_HDU_BASIS, false))),
            None => first_of_roles(&candidate_roles(&file, layer_file, &planes)?, &[LayerRole::Mask]),
        };
        if let Some((hdu, choice)) = mask_layer {
            match other_shape(MASK_LAYER, cube, hdu, &choice.basis) {
                Some(reason) => pairing.dq_unavailable = Some(reason),
                None => {
                    if lacks_flag_names(&extract_header_by_index_merged(&file, hdu.index)?, header) {
                        pairing.notes.push(UNNAMED_FLAGS_NOTE.to_string());
                    }
                    companions.dq = Some(ImageRef::hdu(&band.file, hdu.index));
                    pairing.dq = Some(choice);
                }
            }
        }
    }
    pairing.layer = Some(layer);
    Ok((companions, pairing))
}

fn fits_pairing(
    file: &File,
    ref_path: &str,
    exts: &[HduInfo],
    idx: usize,
    header: &HduHeader,
) -> Result<(Companions, Pairing)> {
    if let Some(band) = band_plane_of_path(ref_path) {
        let (mut companions, mut pairing) = band_plane_pairing(&band, header)?;
        apply_assignment(file, ref_path, exts, idx, header, &mut companions, &mut pairing);
        return Ok((companions, pairing));
    }
    let active = exts
        .get(idx)
        .with_context(|| format!("HDU index {} out of range (file has {} HDUs)", idx, exts.len()))?;
    let layer_file = file_name_of(Path::new(ref_path));
    let layer = classify_layer(header, layer_file);
    let name = active.extname.as_deref().unwrap_or("");
    let active_ref = ImageRef::hdu(ref_path, idx);
    let candidates: Vec<&HduInfo> = exts
        .iter()
        .filter(|e| e.index != idx && holds_single_plane(e) && effective_extver(e) == effective_extver(active))
        .collect();
    let own_error = is_err_name(name);
    let own_mask = is_dq_name(name) || layer.role == LayerRole::Mask;
    let wants_error = !own_error && !layer.role.is_error() && layer.role != LayerRole::Mask;
    let stored_error = named_candidate(&candidates, EXTNAME_ERR);
    let pipeline_mask = named_candidate(&candidates, EXTNAME_DQ);
    let roles = if (wants_error && stored_error.is_none()) || (!own_mask && pipeline_mask.is_none()) {
        candidate_roles(file, layer_file, &candidates)?
    } else {
        Vec::new()
    };
    let mut companions = Companions::default();
    let mut pairing = Pairing::default();
    if own_error {
        companions.err = Some(active_ref.clone());
        pairing.err = Some(guessed_choice(&layer));
    } else if wants_error {
        let error_layer = stored_error
            .map(|hdu| (hdu, same_file_choice(LayerRole::Sigma, ERR_HDU_BASIS, false)))
            .or_else(|| first_of_roles(&roles, &SIGMA_ROLES));
        if let Some((hdu, choice)) = error_layer {
            match other_shape(ERROR_LAYER, active, hdu, &choice.basis) {
                Some(reason) => pairing.err_unavailable = Some(reason),
                None => {
                    pairing.notes.extend(assumed_layer_note(hdu, &choice));
                    companions.err = Some(ImageRef::hdu(ref_path, hdu.index));
                    pairing.err = Some(choice);
                }
            }
        }
    }
    let mask_layer = pipeline_mask
        .map(|hdu| (hdu, same_file_choice(LayerRole::Mask, DQ_HDU_BASIS, false)))
        .or_else(|| first_of_roles(&roles, &[LayerRole::Mask]));
    let paired_mask = if own_mask {
        companions.dq = Some(active_ref);
        pairing.dq = Some(guessed_choice(&layer));
        Some(idx)
    } else if let Some((hdu, choice)) = mask_layer {
        match other_shape(MASK_LAYER, active, hdu, &choice.basis) {
            Some(reason) => {
                pairing.dq_unavailable = Some(reason);
                None
            }
            None => {
                companions.dq = Some(ImageRef::hdu(ref_path, hdu.index));
                pairing.dq = Some(choice);
                Some(hdu.index)
            }
        }
    } else {
        None
    };
    if let Some(dq_index) = paired_mask {
        if names_no_flags(file, header, idx, dq_index)? {
            pairing.notes.push(UNNAMED_FLAGS_NOTE.to_string());
        }
    }
    if !is_wavelength_name(name) {
        companions.wavelength =
            named_candidate(&candidates, EXTNAME_WAVELENGTH).map(|hdu| ImageRef::hdu(ref_path, hdu.index));
    }
    if layer.role == LayerRole::Science {
        pair_sibling_files(ref_path, header, &mut companions, &mut pairing);
        if idx == 0 && looks_like_sdss_frame(file, exts) {
            pairing.err_recipe = Some(ErrRecipe::SdssFrame { gain: None, dark_variance: None });
            if companions.err.is_none() && pairing.err_unavailable.is_none() {
                pairing.err_unavailable = Some(SDSS_NO_ERROR_REASON.to_string());
            }
        }
    }
    apply_assignment(file, ref_path, exts, idx, header, &mut companions, &mut pairing);
    pairing.layer = Some(layer);
    Ok((companions, pairing))
}

pub fn declared_bitpix(header: &HduHeader) -> Option<i64> {
    header.get_i64("ZBITPIX").or_else(|| header.get_i64("BITPIX"))
}

fn mask_plane_kind(bitpix: i64) -> Option<&'static str> {
    match bitpix {
        8 | 16 | 32 => None,
        -32 | -64 => Some("a floating-point plane"),
        64 => Some("a 64-bit integer plane"),
        _ => Some("not an 8, 16 or 32 bit integer plane"),
    }
}

fn sibling_reasons(candidate: &SiblingCandidate, image: &HduHeader, layer: &HduHeader) -> (Vec<String>, (i64, i64)) {
    let file = file_name_of(Path::new(&candidate.path));
    let rule = candidate.rule;
    let report = grid_report(image, layer);
    let kind = if candidate.role == LayerRole::Mask { MASK_LAYER } else { ERROR_LAYER };
    if let Some(refusal) = report.refusal(&format!("{kind} layer"), file) {
        return (vec![refusal], report.layer);
    }
    let mut reasons: Vec<String> = report
        .warnings
        .iter()
        .map(|warning| format!("{file} matches the {rule} name rule but {warning}: not applied automatically"))
        .collect();
    if candidate.role == LayerRole::Mask {
        let bitpix = declared_bitpix(layer).unwrap_or(0);
        if let Some(plane) = mask_plane_kind(bitpix) {
            reasons.push(format!("{file} matches the {rule} name rule but is {plane} (BITPIX {bitpix}): not used as a mask"));
        }
    }
    (reasons, report.layer)
}

fn eligible_sibling(
    candidates: &[SiblingCandidate],
    image: &HduHeader,
    notes: &mut Vec<String>,
) -> Option<(SiblingCandidate, HduHeader, (i64, i64))> {
    let mut eligible = Vec::new();
    for candidate in candidates {
        let layer = match load_plane_header(&ImageRef::auto(&candidate.path)) {
            Ok(layer) => layer,
            Err(error) => {
                notes.push(format!(
                    "{} matches the {} name rule but could not be read: {error:#}",
                    file_name_of(Path::new(&candidate.path)),
                    candidate.rule
                ));
                continue;
            }
        };
        let (reasons, shape) = sibling_reasons(candidate, image, &layer);
        if reasons.is_empty() {
            eligible.push((candidate.clone(), layer, shape));
        } else {
            notes.extend(reasons);
        }
    }
    if eligible.len() > 1 {
        let kind = if candidates[0].role == LayerRole::Mask { MASK_LAYER } else { ERROR_LAYER };
        notes.push(format!(
            "{} files match the {} name rule for the {kind} layer: none applied, assign one by hand",
            eligible.len(),
            candidates[0].rule
        ));
        return None;
    }
    eligible.pop()
}

fn sibling_companion(candidate: &SiblingCandidate, layer: &HduHeader) -> CompanionChoice {
    let stated = classify_layer(layer, "");
    let basis = if candidate.role == LayerRole::Variance && stated.role == LayerRole::Variance {
        format!("{} and {}", candidate.basis, stated.basis)
    } else {
        candidate.basis.clone()
    };
    CompanionChoice { role: candidate.role, basis, assumed: false, origin: CompanionOrigin::Sibling }
}

fn taken_note(kind: &str, candidate: &SiblingCandidate, shape: (i64, i64)) -> String {
    format!(
        "{kind} layer taken from the sibling file {} (name rule {}, same {}x{} grid)",
        file_name_of(Path::new(&candidate.path)),
        candidate.rule,
        shape.0,
        shape.1
    )
}

fn pair_sibling_files(ref_path: &str, image: &HduHeader, companions: &mut Companions, pairing: &mut Pairing) {
    let wants_error = companions.err.is_none() && pairing.err_unavailable.is_none();
    let wants_mask = companions.dq.is_none() && pairing.dq_unavailable.is_none();
    if !wants_error && !wants_mask {
        return;
    }
    let siblings = sibling_candidates(Path::new(ref_path));
    if wants_error {
        if let Some((candidate, layer, shape)) = eligible_sibling(&siblings.err, image, &mut pairing.notes) {
            pairing.notes.push(taken_note(ERROR_LAYER, &candidate, shape));
            companions.err = Some(ImageRef::auto(&candidate.path));
            pairing.err = Some(sibling_companion(&candidate, &layer));
        }
    }
    if wants_mask {
        if let Some((candidate, layer, shape)) = eligible_sibling(&siblings.dq, image, &mut pairing.notes) {
            pairing.notes.push(taken_note(MASK_LAYER, &candidate, shape));
            if lacks_flag_names(&layer, image) {
                pairing.notes.push(UNNAMED_FLAGS_NOTE.to_string());
            }
            companions.dq = Some(ImageRef::auto(&candidate.path));
            pairing.dq = Some(sibling_companion(&candidate, &layer));
        }
    }
}

fn asdf_companions(ref_path: &str, asdf: &AsdfFile, data_key: &str) -> Companions {
    let find = |suffix: &str| {
        let key = companion_key(data_key, suffix);
        AsdfImage::array_exists(&asdf.tree, &key).then(|| ImageRef::array(ref_path, &key))
    };
    let active = ImageRef::array(ref_path, data_key);
    let dq = if is_dq_name(data_key) { Some(active.clone()) } else { find("dq") };
    let err = if is_err_name(data_key) { Some(active) } else { find("err") };
    Companions { dq, err, wavelength: None }
}

fn asdf_pairing(ref_path: &str, data_key: &str, companions: &Companions) -> Pairing {
    let mut named = HduHeader::empty();
    named.set("EXTNAME", data_key.to_string());
    Pairing {
        layer: Some(classify_layer(&named, file_name_of(Path::new(ref_path)))),
        err: companions.err.as_ref().map(|_| same_file_choice(LayerRole::Sigma, ASDF_ERR_BASIS, false)),
        dq: companions.dq.as_ref().map(|_| same_file_choice(LayerRole::Mask, ASDF_DQ_BASIS, false)),
        exclusion_override: assignment(&assignment_key(ref_path, &PlaneSelector::Array(data_key.to_string())))
            .and_then(|assigned| assigned.exclusion),
        ..Pairing::default()
    }
}

fn exact_integers(file: &File, index: usize, extension: Option<&HduInfo>) -> Result<IntPlane, String> {
    extract_int_plane_by_index(file, index).map_err(|error| {
        log::warn!("DQ plane HDU {} has no lossless integer form: {:#}", index, error);
        extension.and_then(|hdu| inexact_bitpix(hdu.bitpix)).unwrap_or_else(|| format!("{error:#}"))
    })
}

fn fits_loaded(
    source: &Path,
    index: Option<usize>,
    tmp: Option<SharedTempDir>,
    ref_path: Option<&str>,
) -> Result<LoadedPlane> {
    let file = File::open(source).with_context(|| format!("Failed to open {}", source.display()))?;
    let idx = match index {
        Some(i) => i,
        None => auto_hdu_index(&file)?,
    };
    let result = extract_image_mmap_by_index(&file, idx)?;
    let extension = result.extensions.get(idx);
    let mut info = plane_info_for(PlaneSelector::Hdu(idx), extension);
    let companions = match ref_path {
        Some(p) => {
            let (companions, pairing) = fits_pairing(&file, p, &result.extensions, idx, &result.header)?;
            info.pairing = pairing;
            companions
        }
        None => {
            info.pairing.layer = Some(classify_layer(&result.header, file_name_of(source)));
            Companions::default()
        }
    };
    info.is_dq = info.is_dq || is_mask_layer(&info.pairing);
    let assigned_mask =
        ref_path.is_some_and(|p| is_assigned_mask(&ImageRef::hdu(p, idx)) || is_assigned_mask(&ImageRef::auto(p)));
    let int_plane = if info.is_dq || assigned_mask {
        exact_integers(&file, idx, extension).map_err(|reason| info.pairing.int_unavailable = Some(reason)).ok()
    } else {
        None
    };
    Ok(LoadedPlane {
        arr: result.image,
        header: result.header,
        info,
        int_plane,
        companions,
        _tmp: tmp,
    })
}

fn asdf_loaded(source: &Path, key: Option<&str>, tmp: Option<SharedTempDir>, ref_path: &str) -> Result<LoadedPlane> {
    let asdf = AsdfFile::open(source).map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;
    let result = extract_plane_from_open_asdf(&asdf, key)?;
    let data_key = result.selected_extension.clone().unwrap_or_default();
    let ext = result.extensions.iter().find(|e| e.extname.as_deref() == Some(data_key.as_str()));
    let mut info = plane_info_for(PlaneSelector::Array(data_key.clone()), ext);
    let int_plane = if info.is_dq {
        AsdfImage::load_array_int(&asdf, &data_key).ok().flatten()
    } else {
        None
    };
    let companions = asdf_companions(ref_path, &asdf, &data_key);
    info.pairing = asdf_pairing(ref_path, &data_key, &companions);
    Ok(LoadedPlane {
        arr: result.image,
        header: result.header,
        info,
        int_plane,
        companions,
        _tmp: tmp,
    })
}

pub fn load_plane(r: &ImageRef) -> Result<LoadedPlane> {
    let (source, tmp) = resolve_single_image(&r.path)?;
    if is_asdf_file(&source) {
        bail_if_calib(&source)?;
        return match &r.plane {
            PlaneSelector::Hdu(_) => bail!("ASDF files have no HDU index; use #array=<key>"),
            PlaneSelector::Array(k) => asdf_loaded(&source, Some(k), tmp, &r.path),
            PlaneSelector::Auto => match asdf_loaded(&source, None, None, &r.path) {
                Ok(mut loaded) => {
                    loaded._tmp = tmp;
                    Ok(loaded)
                }
                Err(e) if missing_asdf_data(&e) => match companion_fits_path(&source) {
                    Some(fits) => fits_loaded(&fits, None, tmp, None),
                    None => bail!("ASDF has no image data and no companion .fits found"),
                },
                Err(e) => Err(e),
            },
        };
    }
    match &r.plane {
        PlaneSelector::Hdu(n) => fits_loaded(&source, Some(*n), tmp, Some(&r.path)),
        PlaneSelector::Auto => fits_loaded(&source, None, tmp, Some(&r.path)),
        PlaneSelector::Array(_) => bail!("FITS files have no ASDF arrays; use #hdu=<n>"),
    }
}

pub fn load_plane_header(r: &ImageRef) -> Result<HduHeader> {
    let (source, _tmp) = resolve_single_image(&r.path)?;
    if is_asdf_file(&source) {
        bail_if_calib(&source)?;
        return match &r.plane {
            PlaneSelector::Hdu(_) => bail!("ASDF files have no HDU index; use #array=<key>"),
            PlaneSelector::Array(k) => Ok(extract_plane_from_asdf(&source, Some(k))?.header),
            PlaneSelector::Auto => match extract_plane_from_asdf(&source, None) {
                Ok(res) => Ok(res.header),
                Err(e) if missing_asdf_data(&e) => match companion_fits_path(&source) {
                    Some(fits) => {
                        let file = File::open(&fits)?;
                        extract_header_mmap(&file)
                    }
                    None => bail!("ASDF has no image data and no companion .fits found"),
                },
                Err(e) => Err(e),
            },
        };
    }
    let file = File::open(&source).with_context(|| format!("Failed to open {}", source.display()))?;
    match &r.plane {
        PlaneSelector::Hdu(n) => extract_header_by_index_merged(&file, *n),
        PlaneSelector::Auto => extract_header_mmap(&file),
        PlaneSelector::Array(_) => bail!("FITS files have no ASDF arrays; use #hdu=<n>"),
    }
}

pub fn sigma_key(source: &ImageRef, role: LayerRole) -> String {
    let plane = source.fragment().unwrap_or_else(|| "auto".to_string());
    ImageRef::array(&source.path, &format!("__sigma.{}.{}", role.name(), plane)).cache_key()
}

pub fn is_stored_sigma(choice: &CompanionChoice) -> bool {
    choice.role == LayerRole::Sigma
        && (choice.basis.starts_with(ERR_HDU_BASIS) || choice.basis.starts_with(ASDF_BASIS_PREFIX))
}

fn layer_place(source: &ImageRef, origin: CompanionOrigin) -> String {
    let band = band_plane_of_path(&source.path);
    let plane = match (&band, &source.plane) {
        (Some(band), _) => format!("HDU {} plane {}", band.hdu, band.plane),
        (None, PlaneSelector::Hdu(n)) => format!("HDU {n}"),
        (None, PlaneSelector::Array(key)) => key.clone(),
        (None, PlaneSelector::Auto) => String::new(),
    };
    if matches!(origin, CompanionOrigin::SameFile | CompanionOrigin::UserSameFile) {
        return plane;
    }
    let file = band.map_or_else(|| source.path.clone(), |band| band.file);
    format!("{} {plane}", file_name_of(Path::new(&file))).trim_end().to_string()
}

pub fn err_source_text(choice: &CompanionChoice, companion: &ImageRef) -> String {
    if choice.origin == CompanionOrigin::Recipe {
        return format!("{}: {RECIPE_CONVERSION}", choice.basis);
    }
    let assumption = assumed_quantity(choice.role, choice.assumed)
        .map(|(_, quantity)| format!(", assumed {quantity}"))
        .unwrap_or_default();
    format!(
        "{}, {}: {}{assumption}",
        choice.basis,
        layer_place(companion, choice.origin),
        choice.role.conversion()
    )
}

pub fn dq_source_text(choice: &CompanionChoice, companion: &ImageRef) -> String {
    format!("{}, {}", choice.basis, layer_place(companion, choice.origin))
}

pub fn mask_unreadable_by_header(companion: &ImageRef, choice: Option<&CompanionChoice>) -> Option<String> {
    if matches!(companion.plane, PlaneSelector::Array(_)) {
        return None;
    }
    let kind = inexact_bitpix(declared_bitpix(&load_plane_header(companion).ok()?)?)?;
    let place = choice.map_or_else(|| companion.cache_key(), |c| layer_place(companion, c.origin));
    Some(inexact_mask(&place, Some(&kind)))
}

fn err_origin(choice: &CompanionChoice, source: &ImageRef, n_no_data: u64) -> ErrOrigin {
    ErrOrigin { role: choice.role, source: err_source_text(choice, source), assumed: choice.assumed, n_no_data }
}

fn unreadable_layer(kind: &str, key: &str, error: anyhow::Error) -> String {
    match error.downcast::<LayerRefusal>() {
        Ok(refusal) => refusal.0,
        Err(error) => {
            log::warn!("companion plane {} failed to load: {:#}", key, error);
            format!("{kind} layer {key} could not be read: {error:#}")
        }
    }
}

fn recipe_sigma_load(frame: &ImageRef, active: &ImageEntry, recipe: Option<&ErrRecipe>) -> Result<PlaneLoad> {
    let Some(ErrRecipe::SdssFrame { gain: Some(gain), dark_variance: Some(dark_variance) }) = recipe else {
        bail!("the gain and the dark variance were not typed");
    };
    let sigma = sdss_frame_sigma_plane(&frame.path, active.arr(), *gain, *dark_variance)?;
    let no_data = sigma.iter().filter(|value| value.is_nan()).count();
    let mut header = HduHeader::empty();
    if let Some(unit) = active.header().and_then(|h| h.get(HEADER_BUNIT)) {
        header.set(HEADER_BUNIT, unit.to_string());
    }
    header.set(CARD_NO_DATA, no_data.to_string());
    header.set_f64(CARD_SDSS_GAIN, *gain);
    header.set_f64(CARD_SDSS_DARK_VARIANCE, *dark_variance);
    let stats = compute_image_stats(&sigma);
    Ok(PlaneLoad::synthetic(sigma, stats, header))
}

fn one_sigma_load(layer: PlaneLoad, role: LayerRole, image_unit: Option<&str>) -> Result<PlaneLoad> {
    let (sigma, no_data) = sigma_with_no_data(role, &layer.arr)
        .with_context(|| format!("a {} layer has no one-sigma form", role.name()))?;
    let mut header = layer.header;
    match image_unit {
        Some(unit) => {
            header.set(HEADER_BUNIT, unit.to_string());
            if let Some(keys) = header.string_keys.as_mut() {
                keys.insert(HEADER_BUNIT.to_string());
            }
        }
        None => header.remove(HEADER_BUNIT),
    }
    header.set(CARD_NO_DATA, no_data.to_string());
    let stats = compute_image_stats(&sigma);
    Ok(PlaneLoad::synthetic(sigma, stats, header))
}

pub fn load_companions_into(
    cache: &ImageCache,
    active: &ImageEntry,
    load: impl Fn(&ImageRef) -> Result<PlaneLoad>,
) -> LoadedCompanions {
    let Some(comps) = active.companions() else {
        return LoadedCompanions::default();
    };
    let unpaired = Pairing::default();
    let pairing = active.plane_info().map_or(&unpaired, |info| &info.pairing);
    for (layer, choice) in [(comps.err.as_ref(), pairing.err.as_ref()), (comps.dq.as_ref(), pairing.dq.as_ref())] {
        if let (Some(layer), Some(choice)) = (layer, choice) {
            if matches!(choice.origin, CompanionOrigin::Sibling | CompanionOrigin::User) {
                cache.revalidate_source(&layer.cache_key());
            }
        }
    }
    let dims = active.arr().dim();
    let refusal = |kind: &str, key: &str, basis: Option<&str>, layer: (usize, usize)| {
        shape_refusal(kind, &labelled(key, basis), (layer.1, layer.0), (dims.1, dims.0))
    };
    let stored = |r: &ImageRef, kind: &str, basis: Option<&str>| -> Result<ImageEntry, String> {
        let key = r.cache_key();
        let entry = cache.get_or_load_plane(&key, || load(r)).map_err(|e| unreadable_layer(kind, &key, e))?;
        if entry.arr().dim() == dims {
            Ok(entry)
        } else {
            Err(refusal(kind, &key, basis, entry.arr().dim()))
        }
    };
    let converted = |r: &ImageRef, choice: &CompanionChoice| -> Result<ImageEntry, String> {
        let key = r.cache_key();
        cache
            .get_or_load_plane(&sigma_key(r, choice.role), || {
                let layer = load(r)?;
                if layer.arr.dim() != dims {
                    return Err(LayerRefusal(refusal(ERROR_LAYER, &key, Some(&choice.basis), layer.arr.dim())).into());
                }
                one_sigma_load(layer, choice.role, active.header().and_then(|h| h.get(HEADER_BUNIT)))
            })
            .map_err(|e| unreadable_layer(ERROR_LAYER, &key, e))
    };
    let rebuilt = |r: &ImageRef| {
        cache.get_or_load_plane(&sigma_key(r, LayerRole::Sigma), || recipe_sigma_load(r, active, pairing.err_recipe.as_ref()))
    };
    let no_data_of = |entry: &ImageEntry| {
        u64::try_from(entry.header().and_then(|h| h.get_i64(CARD_NO_DATA)).unwrap_or(0)).unwrap_or(0)
    };
    let mut loaded = LoadedCompanions::default();
    match (comps.err.as_ref(), pairing.err.as_ref()) {
        (None, _) => loaded.err_unavailable = pairing.err_unavailable.clone(),
        (Some(r), Some(choice)) if choice.origin == CompanionOrigin::Recipe => match rebuilt(r) {
            Ok(entry) => {
                loaded.err_origin = Some(err_origin(choice, r, no_data_of(&entry)));
                loaded.err = Some(entry);
            }
            Err(error) => {
                loaded.err_unavailable = Some(format!("the SDSS frame error could not be rebuilt: {error:#}"));
            }
        },
        (Some(r), Some(choice)) if !is_stored_sigma(choice) => match converted(r, choice) {
            Ok(entry) => {
                loaded.err_origin = Some(err_origin(choice, r, no_data_of(&entry)));
                loaded.err = Some(entry);
            }
            Err(reason) => loaded.err_unavailable = Some(reason),
        },
        (Some(r), choice) => match stored(r, ERROR_LAYER, choice.map(|c| c.basis.as_str())) {
            Ok(entry) => {
                loaded.err_origin = choice.map(|c| err_origin(c, r, 0));
                loaded.err = Some(entry);
            }
            Err(reason) => loaded.err_unavailable = Some(reason),
        },
    }
    match comps.dq.as_ref() {
        None => loaded.dq_unavailable = pairing.dq_unavailable.clone(),
        Some(r) => match stored(r, MASK_LAYER, pairing.dq.as_ref().map(|c| c.basis.as_str())) {
            Ok(entry) if entry.int_plane().is_some() => {
                let table = active
                    .header()
                    .map_or(DqTable::Unknown, DqTable::select)
                    .or_else(|| entry.header().map_or(DqTable::Unknown, DqTable::select));
                let flags = FlagSet::from_headers(entry.header(), active.header(), table);
                loaded.flags = Some(match pairing.exclusion_override {
                    Some(chosen) => flags.with_exclusion(Some(chosen)),
                    None => flags,
                });
                loaded.dq_origin = pairing.dq.as_ref().map(|c| dq_source_text(c, r));
                loaded.dq = Some((entry, table));
            }
            Ok(entry) => {
                let place = pairing.dq.as_ref().map_or_else(|| r.cache_key(), |c| layer_place(r, c.origin));
                let reason = entry.plane_info().and_then(|info| info.pairing.int_unavailable.as_deref());
                loaded.dq_unavailable = Some(inexact_mask(&place, reason));
            }
            Err(reason) => loaded.dq_unavailable = Some(reason),
        },
    }
    loaded.wavelength = comps.wavelength.as_ref().and_then(|r| stored(r, WAVELENGTH_LAYER, None).ok());
    loaded
}

pub fn resolve_plane_info(r: &ImageRef) -> Result<PlaneInfo> {
    resolve_plane(r).map(|(_, info)| info)
}

pub fn resolve_plane(r: &ImageRef) -> Result<(Companions, PlaneInfo)> {
    let (source, _tmp) = resolve_single_image(&r.path)?;
    if is_asdf_file(&source) {
        let open = || AsdfFile::open(&source).map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e));
        let (asdf, key) = match &r.plane {
            PlaneSelector::Hdu(_) => bail!("ASDF files have no HDU index; use #array=<key>"),
            PlaneSelector::Array(k) => (open()?, k.clone()),
            PlaneSelector::Auto => {
                let asdf = open()?;
                let key = auto_data_key(&asdf).context("ASDF load failed: Missing field: data array")?;
                (asdf, key)
            }
        };
        let arrays = list_asdf_arrays(&source)?;
        let ext = arrays.iter().find(|e| e.extname.as_deref() == Some(key.as_str()));
        let mut info = plane_info_for(PlaneSelector::Array(key.clone()), ext);
        if info.extname.is_none() {
            info.extname = Some(key.clone());
            info.is_dq = is_dq_name(&key);
            info.is_err = is_err_name(&key);
        }
        let companions = asdf_companions(&r.path, &asdf, &key);
        info.pairing = asdf_pairing(&r.path, &key, &companions);
        return Ok((companions, info));
    }
    let file = File::open(&source).with_context(|| format!("Failed to open {}", source.display()))?;
    let idx = match &r.plane {
        PlaneSelector::Hdu(n) => *n,
        PlaneSelector::Auto => auto_hdu_index(&file)?,
        PlaneSelector::Array(_) => bail!("FITS files have no ASDF arrays; use #hdu=<n>"),
    };
    let exts = list_extensions(&file)?;
    let ext = exts
        .get(idx)
        .with_context(|| format!("HDU index {} out of range (file has {} HDUs)", idx, exts.len()))?;
    let mut info = plane_info_for(PlaneSelector::Hdu(idx), Some(ext));
    let header = extract_header_by_index_merged(&file, idx)?;
    let (companions, pairing) = fits_pairing(&file, &r.path, &exts, idx, &header)?;
    info.pairing = pairing;
    info.is_dq = info.is_dq || is_mask_layer(&info.pairing);
    if info.is_dq {
        info.pairing.int_unavailable = inexact_bitpix(ext.bitpix);
    }
    Ok((companions, info))
}

pub fn list_planes(path: &str) -> Result<(Vec<HduInfo>, bool)> {
    let r = ImageRef::parse(path);
    let (source, _tmp) = resolve_single_image(&r.path)?;
    if is_asdf_file(&source) {
        return Ok((list_asdf_arrays(&source)?, true));
    }
    let file = File::open(&source).with_context(|| format!("Failed to open {}", source.display()))?;
    Ok((list_extensions(&file)?, false))
}

pub fn plane_ref(path: &str, info: &HduInfo, is_asdf: bool) -> ImageRef {
    if is_asdf {
        ImageRef::array(path, info.extname.as_deref().unwrap_or(""))
    } else {
        ImageRef::hdu(path, info.index)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) const ROMAN_TREE: &str = "roman:\n  meta:\n    instrument: {name: WFI}\n  data: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n  dq: !core/ndarray-1.0.0\n    data: [[0, 1], [0, 0]]\n    datatype: uint32\n";

    pub(crate) fn write_asdf(dir: &tempfile::TempDir, name: &str, tree_yaml: &str) -> String {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"#ASDF 1.0.0\n#ASDF_STANDARD 1.5.0\n%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n");
        bytes.extend_from_slice(tree_yaml.as_bytes());
        bytes.extend_from_slice(b"...\n");
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_string()
    }

    pub(crate) const CUTOUT_NAME: &str = "ls-dr10_A_gr_invvar_maskbits_4px.fits";
    pub(crate) const CUTOUT_INVVAR: (usize, usize, usize) = (4, 3, 2);

    fn cutout_band_cards() -> Vec<(&'static str, String)> {
        vec![("BANDS", "'gr      '".into()), ("BAND0", "'g       '".into()), ("BAND1", "'r       '".into())]
    }

    pub(crate) fn write_band_cutout(dir: &std::path::Path, name: &str, invvar: (usize, usize, usize)) -> String {
        let (cols, rows, planes) = invvar;
        let mut image_cards: Vec<(&'static str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "3".into()),
            ("NAXIS1", "4".into()),
            ("NAXIS2", "3".into()),
            ("NAXIS3", "2".into()),
            ("EXTEND", "T".into()),
            ("IMAGETYP", "'IMAGE   '".into()),
        ];
        image_cards.extend(cutout_band_cards());
        let image: Vec<u8> = (0..12).chain(100..112).flat_map(|v| (v as f32).to_be_bytes()).collect();
        let mut invvar_cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "3".into()),
            ("NAXIS1", cols.to_string()),
            ("NAXIS2", rows.to_string()),
            ("NAXIS3", planes.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("IMAGETYP", "'INVVAR  '".into()),
        ];
        invvar_cards.extend(cutout_band_cards());
        let weights: Vec<u8> = (0..planes)
            .flat_map(|plane| {
                (0..cols * rows).map(move |i| match (plane, i) {
                    (0, _) => 4.0_f32,
                    (_, 5) => 0.0,
                    _ => 16.0,
                })
            })
            .flat_map(f32::to_be_bytes)
            .collect();
        let mask_cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "4".into()),
            ("NAXIS2", "3".into()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("IMAGETYP", "'MASKBITS'".into()),
        ];
        let mask: Vec<u8> = (0..12)
            .map(|i| match i {
                0 => 1024_i32,
                7 => 8,
                _ => 0,
            })
            .flat_map(i32::to_be_bytes)
            .collect();
        let path = dir.join(name);
        crate::infra::fits::reader::test_fixtures::write_raw_hdus(
            &path,
            &[(image_cards, image), (invvar_cards, weights), (mask_cards, mask)],
        );
        path.to_str().unwrap().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test_support::{write_asdf, write_band_cutout, CUTOUT_INVVAR, CUTOUT_NAME, ROMAN_TREE};
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::core::imaging::region::test_support::{header_with_cd, north_up_cd};
    use crate::infra::fits::gzip::test_support::gzip_bytes;
    use crate::infra::fits::writer::write_fits_mono;
    use crate::infra::sdss_frame_file::test_support::{
        assert_sigma_plane, frame_image, write_frame, FRAME_XINTERP, SIGMA_GAIN_2_DARK_0, SIGMA_GAIN_4_DARK_1,
    };
    use crate::infra::sibling_files::GRID_CARDS;
    use crate::infra::fits::reader::test_fixtures::{
        cube_hdu, empty_primary_cards, plane_hdu, ramp_f32, sci_err_dq_mef, sci_err_dq_wave_mef,
        write_raw_hdus, write_test_mef, HduData, TestHdu,
    };

    fn mef_path(dir: &tempfile::TempDir) -> String {
        let path = dir.path().join("jw_cal.fits");
        let mut dq = vec![0i32; 16];
        dq[0] = 1 - 2147483647 - 1;
        dq[3] = 3 - 2147483647 - 1;
        sci_err_dq_mef(&path, 4, 4, dq);
        path.to_str().unwrap().to_string()
    }

    fn companions_of(r: &ImageRef) -> Companions {
        load_plane(r).unwrap().companions
    }

    fn plane_load(r: &ImageRef) -> Result<PlaneLoad> {
        load_plane(r).map(LoadedPlane::into_plane_load)
    }

    const ASDF_TREE: &str = "data: !core/ndarray-1.0.0\n  data: [[1, 2], [3, 4]]\n  datatype: float32\ndq: !core/ndarray-1.0.0\n  data: [[0, 1], [0, 0]]\n  datatype: uint32\nerr: !core/ndarray-1.0.0\n  data: [[0.1, 0.2], [0.3, 0.4]]\n  datatype: float32\n";

    #[test]
    fn load_plane_resolves_fits_companions_by_extver() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let c = companions_of(&ImageRef::hdu(&p, 1));
        assert_eq!(c.dq, Some(ImageRef::hdu(&p, 3)));
        assert_eq!(c.err, Some(ImageRef::hdu(&p, 2)));

        let c = companions_of(&ImageRef::hdu(&p, 4));
        assert_eq!(c, Companions::default());

        let c = companions_of(&ImageRef::hdu(&p, 3));
        assert_eq!(c.dq, Some(ImageRef::hdu(&p, 3)));
        assert_eq!(c.err, None);

        let c = companions_of(&ImageRef::auto(&p));
        assert_eq!(c.dq, Some(ImageRef::hdu(&p, 3)));
    }

    fn write_mef(dir: &tempfile::TempDir, name: &str, hdus: &[TestHdu]) -> String {
        let path = dir.path().join(name);
        write_test_mef(&path, &[], hdus);
        path.to_str().unwrap().to_string()
    }

    fn f32_hdu(extname: &'static str, extver: Option<i64>, side: usize, data: Vec<f32>) -> TestHdu {
        TestHdu { extname: Some(extname), extver, cols: side, rows: side, data: HduData::F32(data), extra_cards: vec![] }
    }

    #[test]
    fn err_and_dq_without_extver_pair_with_a_sci_of_extver_1() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_mef(
            &dir,
            "astropy_default.fits",
            &[
                f32_hdu("SCI", Some(1), 4, ramp_f32(4, 4)),
                f32_hdu("ERR", None, 4, vec![0.5; 16]),
                TestHdu { extname: Some("DQ"), extver: None, cols: 4, rows: 4, data: HduData::I32(vec![0; 16]), extra_cards: vec![] },
            ],
        );
        let c = companions_of(&ImageRef::hdu(&p, 1));
        assert_eq!(c.err, Some(ImageRef::hdu(&p, 2)), "an ERR without EXTVER has the FITS default version 1");
        assert_eq!(c.dq, Some(ImageRef::hdu(&p, 3)), "a DQ without EXTVER has the FITS default version 1");
    }

    #[test]
    fn a_sci_without_extver_never_takes_the_err_of_extver_2() {
        let dir = tempfile::tempdir().unwrap();
        let both = write_mef(
            &dir,
            "two_err.fits",
            &[
                f32_hdu("SCI", None, 4, ramp_f32(4, 4)),
                f32_hdu("ERR", Some(2), 4, vec![2.0; 16]),
                f32_hdu("ERR", None, 4, vec![0.5; 16]),
            ],
        );
        assert_eq!(
            companions_of(&ImageRef::hdu(&both, 1)).err,
            Some(ImageRef::hdu(&both, 3)),
            "a SCI without EXTVER is version 1 and pairs with the ERR of version 1, not with the first ERR listed"
        );

        let other_version = write_mef(
            &dir,
            "err_of_version_2.fits",
            &[f32_hdu("SCI", None, 4, ramp_f32(4, 4)), f32_hdu("ERR", Some(2), 4, vec![2.0; 16])],
        );
        assert_eq!(
            companions_of(&ImageRef::hdu(&other_version, 1)).err,
            None,
            "the only ERR belongs to version 2 and the SCI is version 1"
        );
    }

    fn sci_2d_hdu(cols: usize, rows: usize) -> (Vec<(&'static str, String)>, Vec<u8>) {
        let cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", cols.to_string()),
            ("NAXIS2", rows.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", "'SCI     '".into()),
        ];
        let data: Vec<u8> = (0..cols * rows).flat_map(|i| (i as f32).to_be_bytes()).collect();
        (cards, data)
    }

    #[test]
    fn a_cube_companion_is_skipped_instead_of_being_read_as_its_first_plane() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cube_companions.fits");
        write_raw_hdus(
            &path,
            &[
                (empty_primary_cards(), Vec::new()),
                sci_2d_hdu(4, 3),
                cube_hdu("ERR", 4, 3, 5, &[]),
                cube_hdu("DQ", 4, 3, 5, &[]),
            ],
        );
        let p = path.to_str().unwrap().to_string();

        let c = companions_of(&ImageRef::hdu(&p, 1));
        assert_eq!(
            c,
            Companions::default(),
            "a 3D ERR or DQ extension holds one plane per group read, none of which is the companion of plane 1"
        );

        let single_plane = dir.path().join("single_plane_companions.fits");
        write_raw_hdus(
            &single_plane,
            &[
                (empty_primary_cards(), Vec::new()),
                sci_2d_hdu(4, 3),
                cube_hdu("ERR", 4, 3, 1, &[]),
            ],
        );
        let sp = single_plane.to_str().unwrap().to_string();
        assert_eq!(
            companions_of(&ImageRef::hdu(&sp, 1)).err,
            Some(ImageRef::hdu(&sp, 2)),
            "a rank-3 extension of a single plane is still a usable companion"
        );
    }

    #[test]
    fn load_plane_resolves_the_wavelength_companion_and_skips_it_when_absent_or_cubic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jw_wave.fits");
        sci_err_dq_wave_mef(&path, 4, 4, vec![-2147483648i32; 16], vec![1.5f32; 16]);
        let p = path.to_str().unwrap().to_string();

        let c = companions_of(&ImageRef::hdu(&p, 1));
        assert_eq!(c.wavelength, Some(ImageRef::hdu(&p, 5)));
        assert_eq!(c.dq, Some(ImageRef::hdu(&p, 3)));
        assert_eq!(c.err, Some(ImageRef::hdu(&p, 2)));
        assert_eq!(companions_of(&ImageRef::hdu(&p, 5)).wavelength, None);
        assert_eq!(companions_of(&ImageRef::hdu(&p, 4)).wavelength, None);

        let cache = ImageCache::new(8, usize::MAX);
        let key = format!("{}#hdu=1", p);
        let active = cache.get_or_load_plane(&key, || plane_load(&ImageRef::hdu(&p, 1))).unwrap();
        let loaded = load_companions_into(&cache, &active, plane_load);
        assert_eq!(loaded.wavelength.expect("wavelength companion").arr()[[0, 0]], 1.5);
        assert!(cache.contains(&format!("{}#hdu=5", p)));

        let plain = mef_path(&dir);
        assert_eq!(companions_of(&ImageRef::hdu(&plain, 1)).wavelength, None);

        let cubic = dir.path().join("wave_cube.fits");
        write_raw_hdus(
            &cubic,
            &[
                (empty_primary_cards(), Vec::new()),
                sci_2d_hdu(4, 3),
                cube_hdu("WAVELENGTH", 4, 3, 5, &[]),
            ],
        );
        let cp = cubic.to_str().unwrap().to_string();
        assert_eq!(companions_of(&ImageRef::hdu(&cp, 1)).wavelength, None);

        assert!(is_wavelength_name("WAVELENGTH"));
        assert!(is_wavelength_name(" wavelength "));
        assert!(is_wavelength_name("roman.wavelength"));
        assert!(!is_wavelength_name("WAVELENGTHS"));
        assert!(!is_wavelength_name("SCI"));
    }

    #[test]
    fn a_signed_byte_dq_companion_is_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("int8_companion.fits");
        let dq_cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "4".into()),
            ("NAXIS2", "3".into()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("BZERO", "-128".into()),
            ("BSCALE", "1".into()),
            ("EXTNAME", "'DQ      '".into()),
        ];
        let dq_bytes: Vec<u8> = (0..12u8).map(|v| v + 128).collect();
        write_raw_hdus(
            &path,
            &[(empty_primary_cards(), Vec::new()), sci_2d_hdu(4, 3), (dq_cards, dq_bytes)],
        );
        let p = path.to_str().unwrap().to_string();
        let cache = ImageCache::new(8, usize::MAX);
        let active = cache.get_or_load_plane(&p, || plane_load(&ImageRef::auto(&p))).unwrap();
        let comps = load_companions_into(&cache, &active, plane_load);
        let (dq, _) = comps.dq.expect("a signed-byte DQ plane is a usable companion");
        assert_eq!(dq.int_plane().unwrap().value_at(0, 1), 1);
        assert_eq!(dq.int_plane().unwrap().value_at(2, 3), 11);
    }

    #[test]
    fn a_mono_load_of_an_rgb_file_does_not_fall_through_to_another_plane() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rgb_and_mask.fits");
        write_raw_hdus(
            &path,
            &[
                (empty_primary_cards(), Vec::new()),
                cube_hdu("SCI", 4, 3, 3, &[("CTYPE3", "'RGB     '".into())]),
                plane_hdu("MASK", 4, 3),
            ],
        );
        let p = path.to_str().unwrap().to_string();

        let err = load_plane(&ImageRef::auto(&p)).err().expect("Auto must not read the mask");
        assert!(format!("{err:#}").contains("RGB colour cube"), "{err:#}");
        let header = load_plane_header(&ImageRef::auto(&p)).expect("the header of a colour file is its cube's");
        assert_eq!(header.get("EXTNAME"), Some("SCI"));
        assert_eq!(header.get_i64("NAXIS3"), Some(3));
        assert!(resolve_plane_info(&ImageRef::auto(&p)).is_err());

        let mask = load_plane(&ImageRef::hdu(&p, 2)).unwrap();
        assert_eq!(mask.info.extname.as_deref(), Some("MASK"));
        assert_eq!(mask.arr.dim(), (3, 4));
    }

    #[test]
    fn load_plane_decodes_int_plane_only_for_dq() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let dq = load_plane(&ImageRef::hdu(&p, 3)).unwrap();
        assert!(dq.info.is_dq);
        assert!(!dq.info.is_err);
        assert_eq!(dq.info.kind, PlaneSelector::Hdu(3));
        assert_eq!(dq.info.extname.as_deref(), Some("DQ"));
        assert_eq!(dq.info.extver, Some(1));
        let plane = dq.int_plane.as_ref().unwrap();
        assert_eq!(plane.bits[[0, 0]], 1);
        assert_eq!(plane.bits[[0, 3]], 3);
        assert_eq!(dq.arr[[0, 3]], 3.0);

        let sci = load_plane(&ImageRef::hdu(&p, 1)).unwrap();
        assert!(sci.int_plane.is_none());
        assert!(!sci.info.is_dq);
        assert_eq!(sci.header.get("BUNIT"), Some("MJy/sr"));

        let auto = load_plane(&ImageRef::auto(&p)).unwrap();
        assert_eq!(auto.info.kind, PlaneSelector::Hdu(1));
        assert_eq!(auto.info.extname.as_deref(), Some("SCI"));

        let err = load_plane(&ImageRef::hdu(&p, 2)).unwrap();
        assert!(err.info.is_err);
        assert!(load_plane(&ImageRef::array(&p, "dq")).is_err());
        assert!(load_plane(&ImageRef::hdu(&p, 7)).is_err());
    }

    #[test]
    fn into_plane_load_carries_info_and_companions() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let load = load_plane(&ImageRef::hdu(&p, 3)).unwrap().into_plane_load();
        assert_eq!(load.stats.min, 1.0);
        assert_eq!(load.stats.valid_count, 16);
        assert!(load.int_plane.is_some());
        assert_eq!(load.info.as_ref().unwrap().kind, PlaneSelector::Hdu(3));
        assert_eq!(load.companions.as_ref().unwrap().dq, Some(ImageRef::hdu(&p, 3)));
        assert_eq!(load.header.get("EXTNAME"), Some("DQ"));
    }

    #[test]
    fn load_companions_into_uses_the_entry_companions() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let cache = ImageCache::new(8, usize::MAX);
        let key = format!("{}#hdu=1", p);
        let active = cache.get_or_load_plane(&key, || plane_load(&ImageRef::hdu(&p, 1))).unwrap();

        let comps = load_companions_into(&cache, &active, plane_load);
        assert_eq!(comps.flags.as_ref().map(|f| f.source.as_str()), Some("unknown (raw bits)"));
        let (dq, table) = comps.dq.expect("dq companion");
        assert_eq!(table, DqTable::Unknown);
        assert_eq!(dq.int_plane().unwrap().bits[[0, 3]], 3);
        assert_eq!(comps.err.expect("err companion").arr()[[0, 1]], 0.5);
        assert!(cache.contains(&format!("{}#hdu=3", p)));
        assert!(cache.contains(&format!("{}#hdu=2", p)));

        let again = load_companions_into(&cache, &active, |_| panic!("companions must come from the cache"));
        assert!(again.dq.is_some());
        assert!(again.err.is_some());

        let lonely = cache
            .get_or_load_plane(&format!("{}#hdu=4", p), || plane_load(&ImageRef::hdu(&p, 4)))
            .unwrap();
        let none = load_companions_into(&cache, &lonely, plane_load);
        assert!(none.dq.is_none());
        assert!(none.flags.is_none());
        assert!(none.err.is_none());

        let synthetic = cache
            .get_or_load("__composite_r", || Ok((Array2::<f32>::zeros((4, 4)), compute_image_stats(&Array2::zeros((4, 4))))))
            .unwrap();
        let skipped = load_companions_into(&cache, &synthetic, |_| panic!("synthetic entries have no companions"));
        assert!(skipped.dq.is_none());
    }

    #[test]
    fn a_dq_of_another_size_is_refused_with_both_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mismatch.fits");
        write_test_mef(
            &path,
            &[],
            &[
                TestHdu { extname: Some("SCI"), extver: Some(1), cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![] },
                TestHdu { extname: Some("DQ"), extver: Some(1), cols: 2, rows: 2, data: HduData::I32(vec![1; 4]), extra_cards: vec![] },
            ],
        );
        let p = path.to_str().unwrap().to_string();
        let cache = ImageCache::new(8, usize::MAX);
        let active = cache.get_or_load_plane(&p, || plane_load(&ImageRef::auto(&p))).unwrap();
        assert_eq!(active.companions().unwrap().dq, None);
        let comps = load_companions_into(&cache, &active, plane_load);
        assert!(comps.dq.is_none());
        assert!(comps.err.is_none());
        assert_eq!(
            comps.dq_unavailable.as_deref(),
            Some("mask layer HDU 2 (DQ) is 2x2 and does not match the image 4x4: not used")
        );
        assert_eq!(comps.dq_origin, None);
        assert_eq!(comps.err_unavailable, None);
    }

    fn calexp_mef(dir: &tempfile::TempDir) -> String {
        let mut variance: Vec<f32> = (0..16).map(|i| (i as f32).powi(2)).collect();
        variance[5] = -1.0;
        write_mef(
            dir,
            "calexp.fits",
            &[
                TestHdu { extname: Some("IMAGE"), extver: None, cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![("BUNIT", "'nJy     '".into())] },
                TestHdu { extname: Some("MASK"), extver: None, cols: 4, rows: 4, data: HduData::I32(vec![0; 16]), extra_cards: vec![] },
                TestHdu { extname: Some("VARIANCE"), extver: None, cols: 4, rows: 4, data: HduData::F32(variance), extra_cards: vec![("BUNIT", "'nJy^2   '".into())] },
            ],
        )
    }

    fn small_variance_mef(dir: &tempfile::TempDir) -> String {
        write_mef(
            dir,
            "small_variance.fits",
            &[f32_hdu("SCI", None, 4, ramp_f32(4, 4)), f32_hdu("VARIANCE", None, 2, vec![1.0; 4])],
        )
    }

    fn open_with_companions(cache: &ImageCache, r: &ImageRef) -> (ImageEntry, LoadedCompanions) {
        let active = cache.get_or_load_plane(&r.cache_key(), || plane_load(r)).unwrap();
        let companions = load_companions_into(cache, &active, plane_load);
        (active, companions)
    }

    fn no_data_pixels(entry: &ImageEntry) -> usize {
        entry.arr().iter().filter(|v| v.is_nan()).count()
    }

    #[test]
    fn a_variance_hdu_is_the_error_layer_as_its_square_root_in_the_unit_of_the_image() {
        let dir = tempfile::tempdir().unwrap();
        let p = calexp_mef(&dir);
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));

        let err = comps.err.expect("the VARIANCE HDU is the error layer of the IMAGE HDU");
        assert_eq!(err.arr()[[0, 3]], 3.0);
        assert!(err.arr()[[1, 1]].is_nan(), "a negative variance is no data");
        assert_eq!(no_data_pixels(&err), 1);
        assert_eq!(err.header().and_then(|h| h.get("BUNIT")), Some("nJy"));
        assert_eq!(err.header().and_then(|h| h.get_i64("ABNODATA")), Some(1));
        assert_eq!(
            comps.err_origin,
            Some(ErrOrigin {
                role: LayerRole::Variance,
                source: "EXTNAME VARIANCE, HDU 3: square root".to_string(),
                assumed: false,
                n_no_data: 1,
            })
        );
        assert_eq!(comps.err_unavailable, None);
        assert_eq!(comps.dq_origin.as_deref(), Some("EXTNAME MASK, HDU 2"));
        assert!(cache.contains(&sigma_key(&ImageRef::hdu(&p, 3), LayerRole::Variance)));
        assert!(!cache.contains(&format!("{p}#hdu=3")), "the raw variance stays out of the cache");

        let again = load_companions_into(&cache, &active, |_| panic!("the derived sigma must come from the cache"));
        assert!(std::ptr::eq(again.err.expect("the cached error layer").arr(), err.arr()));
        assert_eq!(again.err_origin, comps.err_origin);
    }

    #[test]
    fn an_ivar_hdu_and_a_wgt_hdu_give_one_over_the_square_root_and_the_weight_is_marked_assumed() {
        let dir = tempfile::tempdir().unwrap();
        let mut weights = vec![4.0_f32; 16];
        weights[6] = 0.0;
        for (file, extname, source, notes) in [
            ("ivar.fits", "IVAR", "EXTNAME IVAR, HDU 2: 1 / square root", vec![]),
            (
                "wgt.fits",
                "WGT",
                "EXTNAME WGT, HDU 2: 1 / square root, assumed inverse variance",
                vec!["error layer HDU 2: EXTNAME WGT is assumed to be an inverse variance".to_string()],
            ),
        ] {
            let p = write_mef(
                &dir,
                file,
                &[f32_hdu("SCI", None, 4, ramp_f32(4, 4)), f32_hdu(extname, None, 4, weights.clone())],
            );
            let cache = ImageCache::new(8, usize::MAX);
            let (active, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));
            let err = comps.err.expect(extname);
            assert_eq!(err.arr()[[0, 0]], 0.5, "{extname}");
            assert!(err.arr()[[1, 2]].is_nan(), "a zero weight is no data ({extname})");
            assert_eq!(no_data_pixels(&err), 1, "{extname}");
            let origin = comps.err_origin.expect(extname);
            assert_eq!(origin.role, LayerRole::InverseVariance);
            assert_eq!(origin.source, source);
            assert_eq!(origin.assumed, extname == "WGT");
            assert_eq!(origin.n_no_data, 1, "{extname}");
            assert_eq!(active.plane_info().unwrap().pairing.notes, notes, "{extname}");
        }

        let mut variance = vec![4.0_f32; 16];
        variance[6] = -1.0;
        let p = write_mef(
            &dir,
            "ps1_like.fits",
            &[
                f32_hdu("SCI", None, 4, ramp_f32(4, 4)),
                TestHdu { extname: None, extver: None, cols: 4, rows: 4, data: HduData::F32(variance), extra_cards: vec![("PS_COVAR", "T".into())] },
            ],
        );
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));
        let err = comps.err.expect("the PS_COVAR layer is the error layer");
        assert_eq!(err.arr()[[0, 0]], 2.0);
        assert!(err.arr()[[1, 2]].is_nan());
        assert_eq!(no_data_pixels(&err), 1);
        assert_eq!(
            comps.err_origin,
            Some(ErrOrigin {
                role: LayerRole::Variance,
                source: "PS_COVAR card (PS1 variance layer), HDU 2: square root, assumed variance".to_string(),
                assumed: true,
                n_no_data: 1,
            })
        );
        assert_eq!(
            active.plane_info().unwrap().pairing.notes,
            vec!["error layer HDU 2: PS_COVAR card (PS1 variance layer) is assumed to be a variance".to_string()]
        );
    }

    #[test]
    fn an_rms_hdu_is_used_as_it_is_with_its_sentinel_as_no_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut rms = vec![0.25_f32; 16];
        rms[9] = 1.0e16;
        let p = write_mef(&dir, "rms.fits", &[f32_hdu("SCI", None, 4, ramp_f32(4, 4)), f32_hdu("RMS", None, 4, rms)]);
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));
        let err = comps.err.expect("the RMS HDU is the error layer");
        assert_eq!(err.arr()[[0, 0]], 0.25);
        assert!(err.arr()[[2, 1]].is_nan(), "the 1e16 sentinel is no data");
        assert_eq!(no_data_pixels(&err), 1);
        assert_eq!(
            comps.err_origin,
            Some(ErrOrigin {
                role: LayerRole::Sigma,
                source: "EXTNAME RMS, HDU 2: as is".to_string(),
                assumed: false,
                n_no_data: 1,
            })
        );
        assert!(active.plane_info().unwrap().pairing.notes.is_empty());
    }

    #[test]
    fn err_wins_over_a_variance_hdu_of_the_same_version() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_mef(
            &dir,
            "err_and_variance.fits",
            &[
                f32_hdu("SCI", Some(1), 4, ramp_f32(4, 4)),
                f32_hdu("VARIANCE", Some(1), 4, vec![4.0; 16]),
                f32_hdu("ERR", Some(1), 4, vec![0.5; 16]),
            ],
        );
        assert_eq!(companions_of(&ImageRef::hdu(&p, 1)).err, Some(ImageRef::hdu(&p, 3)));
        let cache = ImageCache::new(8, usize::MAX);
        let (_, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));
        assert_eq!(comps.err.expect("the ERR HDU").arr()[[0, 0]], 0.5);
        assert_eq!(
            comps.err_origin,
            Some(ErrOrigin {
                role: LayerRole::Sigma,
                source: "EXTNAME ERR, HDU 3: as is".to_string(),
                assumed: false,
                n_no_data: 0,
            })
        );
        assert!(cache.contains(&format!("{p}#hdu=3")), "the ERR HDU is served as the raw entry");
        assert!(!cache.contains(&format!("{p}#hdu=2")));
    }

    fn float_image_cards(first: (&'static str, String), imagetyp: &str) -> Vec<(&'static str, String)> {
        let mut cards = vec![
            first,
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "4".into()),
            ("NAXIS2", "3".into()),
        ];
        if cards[0].0 == "XTENSION" {
            cards.push(("PCOUNT", "0".into()));
            cards.push(("GCOUNT", "1".into()));
        } else {
            cards.push(("EXTEND", "T".into()));
        }
        cards.push(("IMAGETYP", format!("'{imagetyp:<8}'")));
        cards
    }

    #[test]
    fn a_layer_named_only_by_imagetyp_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cutout_by_imagetyp.fits");
        let ramp: Vec<u8> = (0..12).flat_map(|i| (i as f32).to_be_bytes()).collect();
        let weights: Vec<u8> = (0..12).flat_map(|_| 4.0_f32.to_be_bytes()).collect();
        write_raw_hdus(
            &path,
            &[
                (float_image_cards(("SIMPLE", "T".into()), "IMAGE"), ramp),
                (float_image_cards(("XTENSION", "'IMAGE   '".into()), "INVVAR"), weights),
            ],
        );
        let p = path.to_str().unwrap().to_string();
        assert_eq!(resolve_plane_info(&ImageRef::auto(&p)).unwrap().kind, PlaneSelector::Hdu(1));

        let cache = ImageCache::new(8, usize::MAX);
        let (image, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 0));
        assert_eq!(image.plane_info().unwrap().pairing.layer.as_ref().map(|l| l.role), Some(LayerRole::Science));
        let err = comps.err.expect("the IMAGETYP INVVAR extension is the error layer of the primary image");
        assert!(err.arr().iter().all(|v| *v == 0.5));
        assert_eq!(
            comps.err_origin,
            Some(ErrOrigin {
                role: LayerRole::InverseVariance,
                source: "IMAGETYP INVVAR, HDU 1: 1 / square root".to_string(),
                assumed: false,
                n_no_data: 0,
            })
        );

        let (weight_plane, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));
        assert!(comps.err.is_none());
        assert_eq!(comps.err_unavailable, None);
        assert_eq!(
            weight_plane.plane_info().unwrap().pairing.layer,
            Some(LayerGuess { role: LayerRole::InverseVariance, basis: "IMAGETYP INVVAR".to_string(), assumed: false })
        );
    }

    #[test]
    fn a_layer_of_another_size_is_refused_with_both_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let p = small_variance_mef(&dir);
        let reason = "error layer HDU 2 (VARIANCE) is 2x2 and does not match the image 4x4: not used";
        let loaded = load_plane(&ImageRef::hdu(&p, 1)).unwrap();
        assert_eq!(loaded.companions.err, None);
        assert_eq!(loaded.info.pairing.err, None);
        assert_eq!(loaded.info.pairing.err_unavailable.as_deref(), Some(reason));
        assert!(loaded.info.pairing.notes.is_empty());

        let cache = ImageCache::new(8, usize::MAX);
        let (_, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));
        assert!(comps.err.is_none());
        assert_eq!(comps.err_origin, None);
        assert_eq!(comps.err_unavailable.as_deref(), Some(reason));
    }

    #[test]
    fn an_error_or_mask_plane_shown_on_its_own_gets_no_error_layer() {
        let dir = tempfile::tempdir().unwrap();
        let p = calexp_mef(&dir);
        for (hdu, role) in [(3, LayerRole::Variance), (2, LayerRole::Mask)] {
            let loaded = load_plane(&ImageRef::hdu(&p, hdu)).unwrap();
            assert_eq!(loaded.companions.err, None, "HDU {hdu}");
            assert_eq!(loaded.info.pairing.err, None, "HDU {hdu}");
            assert_eq!(loaded.info.pairing.err_unavailable, None, "HDU {hdu}");
            assert_eq!(loaded.info.pairing.layer.map(|l| l.role), Some(role), "HDU {hdu}");
        }
    }

    #[test]
    fn the_derived_sigma_goes_away_with_its_source_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = calexp_mef(&dir);
        let cache = ImageCache::new(8, usize::MAX);
        let (_, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));
        assert!(comps.err.is_some());
        let derived = sigma_key(&ImageRef::hdu(&p, 3), LayerRole::Variance);
        assert_eq!(ImageRef::parse(&derived).path, p);
        assert!(cache.contains(&derived));

        cache.invalidate_written(&p);
        assert!(!cache.any_key(|key| ImageRef::parse(key).path == p));
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn resolve_plane_info_and_load_plane_agree_on_the_pairing() {
        let dir = tempfile::tempdir().unwrap();
        let calexp = calexp_mef(&dir);
        let small = small_variance_mef(&dir);
        let pipeline = mef_path(&dir);
        let asdf = write_asdf(&dir, "pairing.asdf", ASDF_TREE);
        let refs = [
            ImageRef::auto(&calexp),
            ImageRef::hdu(&calexp, 1),
            ImageRef::hdu(&calexp, 2),
            ImageRef::hdu(&calexp, 3),
            ImageRef::auto(&small),
            ImageRef::hdu(&small, 1),
            ImageRef::hdu(&small, 2),
            ImageRef::hdu(&pipeline, 1),
            ImageRef::hdu(&pipeline, 2),
            ImageRef::hdu(&pipeline, 3),
            ImageRef::hdu(&pipeline, 4),
            ImageRef::auto(&asdf),
            ImageRef::array(&asdf, "dq"),
            ImageRef::array(&asdf, "err"),
        ];
        for r in &refs {
            let resolved = resolve_plane_info(r).unwrap().pairing;
            assert!(resolved.layer.is_some(), "{r:?}");
            assert_eq!(resolved, load_plane(r).unwrap().info.pairing, "{r:?}");
        }
        assert_eq!(
            resolve_plane_info(&ImageRef::hdu(&calexp, 1)).unwrap().pairing.err,
            Some(CompanionChoice {
                role: LayerRole::Variance,
                basis: "EXTNAME VARIANCE".to_string(),
                assumed: false,
                origin: CompanionOrigin::SameFile,
            })
        );
        assert!(resolve_plane_info(&ImageRef::hdu(&small, 1)).unwrap().pairing.err_unavailable.is_some());
    }

    #[test]
    fn pipeline_products_keep_their_error_layer_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let cache = ImageCache::new(8, usize::MAX);
        let (_, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));
        assert_eq!(
            comps.err_origin,
            Some(ErrOrigin {
                role: LayerRole::Sigma,
                source: "EXTNAME ERR, HDU 2: as is".to_string(),
                assumed: false,
                n_no_data: 0,
            })
        );
        let err = comps.err.expect("the ERR HDU");
        assert_eq!(err.arr()[[0, 1]], 0.5);
        let raw = cache.get(&format!("{p}#hdu=2")).expect("the ERR HDU is cached under its own key");
        assert!(std::ptr::eq(err.arr(), raw.arr()));
        assert_eq!(err.header().and_then(|h| h.get(CARD_NO_DATA)), None);
        assert_eq!(err.header().and_then(|h| h.get("BUNIT")), Some("MJy/sr"));
        assert!(!cache.contains(&sigma_key(&ImageRef::hdu(&p, 2), LayerRole::Sigma)));
        assert_eq!(comps.dq_origin.as_deref(), Some("EXTNAME DQ, HDU 3"));
        assert_eq!(comps.err_unavailable, None);
        assert_eq!(comps.dq_unavailable, None);
    }

    #[test]
    fn asdf_arrays_carry_their_origin() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_asdf(&dir, "origin.asdf", ASDF_TREE);
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::auto(&p));
        assert_eq!(
            active.plane_info().unwrap().pairing.layer,
            Some(LayerGuess { role: LayerRole::Science, basis: "EXTNAME DATA".to_string(), assumed: false })
        );
        assert_eq!(
            comps.err_origin,
            Some(ErrOrigin {
                role: LayerRole::Sigma,
                source: "ASDF array err, err: as is".to_string(),
                assumed: false,
                n_no_data: 0,
            })
        );
        assert!(cache.contains(&format!("{p}#array=err")), "the err array is served as the raw entry");
        assert_eq!(comps.dq_origin.as_deref(), Some("ASDF array dq, dq"));

        let r = write_asdf(&dir, "roman_origin.asdf", ROMAN_TREE);
        let (_, comps) = open_with_companions(&cache, &ImageRef::auto(&r));
        assert_eq!(comps.dq_origin.as_deref(), Some("ASDF array dq, roman.dq"));
        assert!(comps.err.is_none());
        assert_eq!(comps.err_unavailable, None);
    }

    const FLOAT_PLANE_REASON: &str = "floating-point plane (BITPIX -32): flag bits cannot be read exactly";
    const UNNAMED_FLAGS_NOTE: &str = "flag layer without bit names: nothing is excluded until bits are chosen";

    #[test]
    fn a_float_dq_is_not_used_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_mef(
            &dir,
            "float_dq.fits",
            &[f32_hdu("SCI", Some(1), 4, ramp_f32(4, 4)), f32_hdu("DQ", Some(1), 4, vec![0.0; 16])],
        );
        let cache = ImageCache::new(8, usize::MAX);
        let (_, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));
        assert!(comps.dq.is_none());
        assert!(comps.flags.is_none());
        assert_eq!(comps.dq_origin, None);
        assert_eq!(comps.dq_unavailable, Some(format!("mask layer HDU 2 is a {FLOAT_PLANE_REASON}")));
        assert_eq!((comps.exclusion_mask(), comps.saturated_mask()), (0, 0));

        let mask = load_plane(&ImageRef::hdu(&p, 2)).unwrap();
        assert!(mask.info.is_dq);
        assert!(mask.int_plane.is_none());
        assert_eq!(mask.info.pairing.int_unavailable.as_deref(), Some(FLOAT_PLANE_REASON));
        assert_eq!(
            resolve_plane_info(&ImageRef::hdu(&p, 2)).unwrap().pairing.int_unavailable.as_deref(),
            Some(FLOAT_PLANE_REASON)
        );
        assert_eq!(load_plane(&ImageRef::hdu(&p, 1)).unwrap().info.pairing.int_unavailable, None);
    }

    fn int_extension(bitpix: i64, extra: &[(&'static str, &str)], values: &[i64]) -> (Vec<(&'static str, String)>, Vec<u8>) {
        let mut cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", bitpix.to_string()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "4".into()),
            ("NAXIS2", (values.len() / 4).to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
        ];
        cards.extend(extra.iter().map(|(key, value)| (*key, value.to_string())));
        let bytes = values
            .iter()
            .flat_map(|value| match bitpix {
                64 => value.to_be_bytes().to_vec(),
                _ => (*value as i32).to_be_bytes().to_vec(),
            })
            .collect();
        (cards, bytes)
    }

    #[test]
    fn a_mask_without_exact_integers_gives_the_reason_of_the_reader() {
        let dir = tempfile::tempdir().unwrap();
        for (file, bitpix, extra, expected) in [
            (
                "wide_dq.fits",
                64,
                &[("EXTNAME", "'DQ      '")][..],
                "mask layer HDU 2 is a 64-bit integer plane: not supported",
            ),
            (
                "scaled_dq.fits",
                32,
                &[("EXTNAME", "'DQ      '"), ("BSCALE", "2"), ("BZERO", "0")],
                "mask layer HDU 2 has no exact integer form: scaled integer data (BSCALE=2, BZERO=0)",
            ),
        ] {
            let path = dir.path().join(file);
            write_raw_hdus(
                &path,
                &[(empty_primary_cards(), Vec::new()), sci_2d_hdu(4, 3), int_extension(bitpix, extra, &[0; 12])],
            );
            let p = path.to_str().unwrap().to_string();
            let cache = ImageCache::new(8, usize::MAX);
            let (_, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));
            assert!(comps.dq.is_none(), "{file}");
            assert!(comps.flags.is_none(), "{file}");
            assert_eq!(comps.dq_unavailable.as_deref(), Some(expected), "{file}");
        }
    }

    fn subaru_calexp(dir: &tempfile::TempDir) -> String {
        let path = dir.path().join("hsc_calexp.fits");
        let mut mask = vec![0i32; 16];
        mask[5] = 2 | 32;
        write_test_mef(
            &path,
            &[("TELESCOP", "'Subaru'".into())],
            &[
                f32_hdu("IMAGE", None, 4, ramp_f32(4, 4)),
                TestHdu {
                    extname: Some("MASK"),
                    extver: None,
                    cols: 4,
                    rows: 4,
                    data: HduData::I32(mask),
                    extra_cards: vec![("MP_BAD", "0".into()), ("MP_SAT", "1".into()), ("HIERARCH MP_DETECTED", "5".into())],
                },
                f32_hdu("VARIANCE", None, 4, vec![1.0; 16]),
            ],
        );
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn a_mask_hdu_is_the_dq_layer_of_the_image_with_the_names_of_its_mp_cards() {
        let dir = tempfile::tempdir().unwrap();
        let p = subaru_calexp(&dir);
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));

        assert_eq!(active.companions().unwrap().dq, Some(ImageRef::hdu(&p, 2)));
        let pairing = &active.plane_info().unwrap().pairing;
        assert_eq!(
            pairing.dq,
            Some(CompanionChoice {
                role: LayerRole::Mask,
                basis: "EXTNAME MASK".to_string(),
                assumed: false,
                origin: CompanionOrigin::SameFile,
            })
        );
        assert!(pairing.notes.is_empty(), "{:?}", pairing.notes);
        let (dq, table) = comps.dq.as_ref().expect("the MASK HDU is the DQ layer of the IMAGE HDU");
        assert_eq!(*table, DqTable::Unknown);
        assert_eq!(dq.int_plane().unwrap().bits[[1, 1]], 34);
        let flags = comps.flags.as_ref().expect("a loaded DQ layer has its flag set");
        assert_eq!(flags.source, "MP_ cards (afw mask planes)");
        assert_eq!(flags.table, DqTable::Unknown);
        assert_eq!(flags.decode(34), vec!["SAT".to_string(), "DETECTED".to_string()]);
        assert_eq!(comps.exclusion_mask(), 3);
        assert_eq!(comps.saturated_mask(), 2);
        assert_eq!(comps.dq_origin.as_deref(), Some("EXTNAME MASK, HDU 2"));
        assert_eq!(comps.dq_unavailable, None);
        assert_eq!(comps.err.as_ref().expect("the VARIANCE HDU is still the error layer").arr()[[0, 0]], 1.0);
        assert_eq!(comps.err_origin.as_ref().map(|o| o.source.as_str()), Some("EXTNAME VARIANCE, HDU 3: square root"));

        let own = load_plane(&ImageRef::hdu(&p, 2)).unwrap();
        assert_eq!(own.companions.dq, Some(ImageRef::hdu(&p, 2)));
        assert!(own.info.pairing.notes.is_empty());

        let chosen = ImageCache::new(8, usize::MAX);
        let image = ImageRef::hdu(&p, 1);
        let active = chosen
            .get_or_load_plane(&image.cache_key(), || {
                let mut load = plane_load(&image)?;
                load.info.as_mut().unwrap().pairing.exclusion_override = Some(1 | 32);
                Ok(load)
            })
            .unwrap();
        let comps = load_companions_into(&chosen, &active, plane_load);
        assert_eq!(comps.exclusion_mask(), 1, "the user's bits replace the default and DETECTED is never excluded");
        assert_eq!(comps.flags.as_ref().map(|f| f.exclusion_default), Some(3));
    }

    #[test]
    fn an_exposure_without_extname_pairs_its_layers_by_exttype() {
        let dir = tempfile::tempdir().unwrap();
        let mut mask = vec![0i32; 16];
        mask[6] = 2;
        let unnamed = |exttype: &str, data: HduData, mut cards: Vec<(&'static str, String)>| {
            cards.insert(0, ("EXTTYPE", format!("'{exttype:<8}'")));
            TestHdu { extname: None, extver: None, cols: 4, rows: 4, data, extra_cards: cards }
        };
        let path = write_mef(
            &dir,
            "medexp.fits",
            &[
                unnamed("IMAGE", HduData::F32(ramp_f32(4, 4)), vec![]),
                unnamed("MASK", HduData::I32(mask), vec![("MP_BAD", "0".into()), ("MP_SAT", "1".into())]),
                unnamed("VARIANCE", HduData::F32(vec![4.0; 16]), vec![]),
            ],
        );
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::auto(&path));
        let info = active.plane_info().unwrap();
        assert_eq!(info.kind, PlaneSelector::Hdu(1));
        assert_eq!(info.pairing.layer.as_ref().map(|l| l.basis.as_str()), Some("EXTTYPE IMAGE"));
        assert_eq!(active.companions().unwrap().err, Some(ImageRef::hdu(&path, 3)));
        assert_eq!(active.companions().unwrap().dq, Some(ImageRef::hdu(&path, 2)));
        assert_eq!(comps.err.as_ref().expect("the VARIANCE HDU is the error layer").arr()[[0, 0]], 2.0);
        assert_eq!(comps.err_origin.as_ref().map(|o| o.source.as_str()), Some("EXTTYPE VARIANCE, HDU 3: square root"));
        assert_eq!(comps.dq_origin.as_deref(), Some("EXTTYPE MASK, HDU 2"));
        assert_eq!(comps.saturated_mask(), 2);
        assert_eq!(comps.dq.as_ref().unwrap().0.int_plane().unwrap().value_at(1, 2), 2);
        assert_eq!(resolve_plane_info(&ImageRef::auto(&path)).unwrap().pairing, info.pairing);

        let own = load_plane(&ImageRef::hdu(&path, 2)).unwrap();
        assert!(own.info.is_dq);
        assert_eq!(own.companions.dq, Some(ImageRef::hdu(&path, 2)));
        assert_eq!(own.companions.err, None);
    }

    #[test]
    fn mask_layers_named_msk_flag_maskbits_or_by_card_pair_and_load_as_integers() {
        const ABOVE_FLOAT_PRECISION: i32 = 25_296_896;
        let dir = tempfile::tempdir().unwrap();
        let mut mask = vec![0i32; 16];
        mask[6] = ABOVE_FLOAT_PRECISION;
        for name in ["MSK", "FLAG", "MASKBITS"] {
            let p = write_mef(
                &dir,
                &format!("layer_{name}.fits"),
                &[
                    f32_hdu("SCI", None, 4, ramp_f32(4, 4)),
                    TestHdu { extname: Some(name), extver: None, cols: 4, rows: 4, data: HduData::I32(mask.clone()), extra_cards: vec![] },
                ],
            );
            let cache = ImageCache::new(8, usize::MAX);
            let (_, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));
            let (dq, _) = comps.dq.as_ref().unwrap_or_else(|| panic!("the {name} HDU is the DQ layer of the SCI HDU"));
            assert_eq!(dq.int_plane().unwrap().value_at(1, 2), i64::from(ABOVE_FLOAT_PRECISION), "{name}");
            assert_eq!(comps.dq_origin, Some(format!("EXTNAME {name}, HDU 2")));

            let own = load_plane(&ImageRef::hdu(&p, 2)).unwrap();
            assert!(own.info.is_dq, "{name}");
            assert!(!own.info.is_err, "{name}");
            assert_eq!(own.companions.dq, Some(ImageRef::hdu(&p, 2)), "{name}");
            assert_eq!(own.int_plane.as_ref().unwrap().value_at(1, 2), i64::from(ABOVE_FLOAT_PRECISION), "{name}");
            assert_eq!(own.info.pairing.int_unavailable, None, "{name}");
            assert!(resolve_plane_info(&ImageRef::hdu(&p, 2)).unwrap().is_dq, "{name}");
        }

        let path = dir.path().join("cutout_mask_by_imagetyp.fits");
        let ramp: Vec<u8> = (0..12).flat_map(|i| (i as f32).to_be_bytes()).collect();
        let mut bits = [0i64; 12];
        bits[6] = i64::from(ABOVE_FLOAT_PRECISION);
        write_raw_hdus(
            &path,
            &[
                (float_image_cards(("SIMPLE", "T".into()), "IMAGE"), ramp),
                int_extension(32, &[("IMAGETYP", "'MASKBITS'")], &bits),
            ],
        );
        let p = path.to_str().unwrap().to_string();
        let cache = ImageCache::new(8, usize::MAX);
        let (image, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 0));
        assert_eq!(image.companions().unwrap().dq, Some(ImageRef::hdu(&p, 1)));
        assert_eq!(comps.dq_origin.as_deref(), Some("IMAGETYP MASKBITS, HDU 1"));
        let (dq, _) = comps.dq.as_ref().expect("the IMAGETYP MASKBITS extension is the DQ layer of the primary image");
        assert_eq!(dq.int_plane().unwrap().value_at(1, 2), i64::from(ABOVE_FLOAT_PRECISION));
        assert_eq!(comps.flags.as_ref().map(|f| f.source.as_str()), Some("Legacy DR10 maskbits (built-in table)"));
        assert!(image.plane_info().unwrap().pairing.notes.is_empty());

        let own = load_plane(&ImageRef::hdu(&p, 1)).unwrap();
        assert!(own.info.is_dq);
        assert_eq!(own.info.extname, None);
        assert_eq!(own.int_plane.as_ref().unwrap().value_at(1, 2), i64::from(ABOVE_FLOAT_PRECISION));
        assert!(resolve_plane_info(&ImageRef::hdu(&p, 1)).unwrap().is_dq);
        assert!(!resolve_plane_info(&ImageRef::hdu(&p, 0)).unwrap().is_dq);
    }

    #[test]
    fn a_mask_without_names_excludes_nothing_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_mef(
            &dir,
            "unnamed_mask.fits",
            &[
                f32_hdu("SCI", None, 4, ramp_f32(4, 4)),
                TestHdu { extname: Some("MASK"), extver: None, cols: 4, rows: 4, data: HduData::I32(vec![5; 16]), extra_cards: vec![] },
            ],
        );
        let cache = ImageCache::new(8, usize::MAX);
        let (_, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));
        assert!(comps.dq.is_some());
        assert_eq!(comps.exclusion_mask(), 0);
        assert_eq!(comps.saturated_mask(), 0);
        let flags = comps.flags.expect("an unnamed mask still has a flag set");
        assert_eq!(flags.source, "unknown (raw bits)");
        assert_eq!(flags.decode(5), vec!["BIT0".to_string(), "BIT2".to_string()]);

        for plane in [1, 2] {
            let r = ImageRef::hdu(&p, plane);
            for notes in [load_plane(&r).unwrap().info.pairing.notes, resolve_plane_info(&r).unwrap().pairing.notes] {
                assert_eq!(notes, vec![UNNAMED_FLAGS_NOTE.to_string()], "HDU {plane}");
            }
        }
    }

    #[test]
    fn pipeline_products_keep_the_masks_of_their_flag_table() {
        let dir = tempfile::tempdir().unwrap();
        for (file, cards, table, saturated) in [
            ("jw_cal.fits", vec![("TELESCOP", "'JWST    '".to_string())], DqTable::Jwst, 2),
            (
                "acs_flt.fits",
                vec![("TELESCOP", "'HST     '".to_string()), ("INSTRUME", "'ACS     '".to_string())],
                DqTable::Hst,
                256,
            ),
        ] {
            let path = dir.path().join(file);
            write_test_mef(
                &path,
                &cards,
                &[
                    f32_hdu("SCI", Some(1), 4, ramp_f32(4, 4)),
                    TestHdu { extname: Some("DQ"), extver: Some(1), cols: 4, rows: 4, data: HduData::I32(vec![0; 16]), extra_cards: vec![("MP_BAD", "0".into())] },
                ],
            );
            let p = path.to_str().unwrap().to_string();
            let cache = ImageCache::new(8, usize::MAX);
            let (active, comps) = open_with_companions(&cache, &ImageRef::hdu(&p, 1));
            assert_eq!(comps.dq.as_ref().map(|(_, table)| *table), Some(table));
            assert_eq!(comps.flags, Some(FlagSet::from_table(table)), "{file}");
            assert_eq!(comps.exclusion_mask(), table.exclusion_mask(), "{file}");
            assert_eq!(comps.saturated_mask(), table.mask_from_names(&["SATURATED"]).unwrap(), "{file}");
            assert_eq!(comps.saturated_mask(), saturated, "{file}");
            assert!(active.plane_info().unwrap().pairing.notes.is_empty(), "{file}");
        }
    }

    #[test]
    fn a_des_mask_layer_excludes_every_non_zero_pixel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("des_coadd.fits");
        let image_cards: Vec<(&'static str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "4".into()),
            ("NAXIS2", "4".into()),
            ("EXTEND", "T".into()),
            ("EXTNAME", "'SCI     '".into()),
            ("DES_EXT", "'IMAGE   '".into()),
        ];
        let ramp: Vec<u8> = (0..16).flat_map(|i| (i as f32).to_be_bytes()).collect();
        let mut bits = [0i64; 16];
        bits[7] = 1;
        write_raw_hdus(
            &path,
            &[
                (image_cards, ramp),
                int_extension(32, &[("EXTNAME", "'MSK     '"), ("DES_EXT", "'MASK    '")], &bits),
            ],
        );
        let p = path.to_str().unwrap().to_string();
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::auto(&p));
        assert_eq!(active.plane_info().unwrap().kind, PlaneSelector::Hdu(0));
        let (dq, _) = comps.dq.as_ref().expect("the MSK extension is the DQ layer of the primary image");
        assert_eq!(dq.int_plane().unwrap().bits[[1, 3]], 1);
        assert_eq!(comps.exclusion_mask(), u32::MAX);
        assert_eq!(comps.saturated_mask(), 0);
        assert_eq!(comps.flags.as_ref().map(|f| f.source.as_str()), Some("DES coadd mask (non-zero = no good data)"));
        assert_eq!(comps.dq_origin.as_deref(), Some("EXTNAME MSK, HDU 1"));
        assert!(active.plane_info().unwrap().pairing.notes.is_empty());
    }

    fn with_sibling_error(science: &str, sibling: ImageRef) -> PlaneLoad {
        let mut load = plane_load(&ImageRef::hdu(science, 1)).unwrap();
        load.companions = Some(Companions { err: Some(sibling), ..Companions::default() });
        load.info.as_mut().unwrap().pairing.err = Some(CompanionChoice {
            role: LayerRole::Variance,
            basis: "file name .wt".to_string(),
            assumed: false,
            origin: CompanionOrigin::Sibling,
        });
        load
    }

    #[test]
    fn a_layer_in_another_file_is_named_by_its_file_and_a_failure_gives_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let science = write_mef(&dir, "stack.fits", &[f32_hdu("SCI", None, 4, ramp_f32(4, 4))]);
        let weights = write_mef(&dir, "stack.wt.fits", &[f32_hdu("SCI", None, 4, vec![9.0; 16])]);
        let small = write_mef(&dir, "small.wt.fits", &[f32_hdu("SCI", None, 2, vec![9.0; 4])]);
        let missing = dir.path().join("missing.wt.fits").to_str().unwrap().to_string();

        for (sibling, source) in [
            (ImageRef::hdu(&weights, 1), "file name .wt, stack.wt.fits HDU 1: square root"),
            (ImageRef::auto(&weights), "file name .wt, stack.wt.fits: square root"),
        ] {
            let cache = ImageCache::new(8, usize::MAX);
            let active = cache.get_or_load_plane(&science, || Ok(with_sibling_error(&science, sibling.clone()))).unwrap();
            let comps = load_companions_into(&cache, &active, plane_load);
            assert_eq!(comps.err.expect("the sibling variance").arr()[[3, 3]], 3.0);
            assert_eq!(comps.err_origin.expect("origin").source, source);
            assert!(cache.contains(&sigma_key(&sibling, LayerRole::Variance)));
            assert!(!cache.contains(&sibling.cache_key()));
        }
        assert_eq!(
            sigma_key(&ImageRef::auto(&weights), LayerRole::Variance),
            format!("{weights}#array=__sigma.variance.auto")
        );

        let cache = ImageCache::new(8, usize::MAX);
        let other_size = ImageRef::hdu(&small, 1);
        let active = cache.get_or_load_plane(&science, || Ok(with_sibling_error(&science, other_size.clone()))).unwrap();
        let comps = load_companions_into(&cache, &active, plane_load);
        assert!(comps.err.is_none());
        assert_eq!(comps.err_origin, None);
        assert_eq!(
            comps.err_unavailable,
            Some(format!("error layer {small}#hdu=1 (file name .wt) is 2x2 and does not match the image 4x4: not used"))
        );
        assert_eq!(cache.len(), 1, "neither the raw layer nor a sigma of another size is cached");

        let cache = ImageCache::new(8, usize::MAX);
        let active = cache.get_or_load_plane(&science, || Ok(with_sibling_error(&science, ImageRef::auto(&missing)))).unwrap();
        let comps = load_companions_into(&cache, &active, plane_load);
        assert!(comps.err.is_none());
        let reason = comps.err_unavailable.expect("a layer that cannot be read says why");
        assert!(reason.starts_with(&format!("error layer {missing} could not be read: ")), "{reason}");
    }

    fn survey_file(set: &str, name: &str) -> String {
        env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("survey").join(set).join(name).to_str().unwrap().to_string()
    }

    fn within_1e6(measured: f32, expected: f64) -> bool {
        ((f64::from(measured) - expected) / expected).abs() <= 1e-6
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
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-invvar-{g,r,z}.fits.fz"]
    fn real_legacy_invvar_layers_become_sigma_with_zero_weight_as_no_data() {
        for (band, zero_weights) in [("g", 4724_u64), ("r", 14346), ("z", 14128)] {
            let path = survey_file("L2", &format!("legacysurvey-0532m280-invvar-{band}.fits.fz"));
            let loaded = load_plane(&ImageRef::auto(&path)).unwrap();
            println!("legacy invvar {band}: plane {:?}, layer {:?}", loaded.info.kind, loaded.info.pairing.layer);
            assert_eq!(
                loaded.info.pairing.layer,
                Some(LayerGuess { role: LayerRole::InverseVariance, basis: "IMTYPE INVVAR".to_string(), assumed: false })
            );
            let (sigma, no_data) = sigma_with_no_data(LayerRole::InverseVariance, &loaded.arr).unwrap();
            println!("legacy invvar {band}: {} x {} px, no-data pixels {no_data} (expected {zero_weights})", sigma.ncols(), sigma.nrows());
            assert_eq!(no_data, zero_weights, "{band}");
            if band != "r" {
                continue;
            }
            for (at, expected) in [
                ([0, 0], 7.175998180759543e-4),
                ([1799, 1799], 5.70805405704646e-4),
                ([3599, 3599], 1.0788353980608423e-3),
                ([100, 2500], 7.164843951123039e-4),
            ] {
                println!("legacy invvar r: sigma{at:?} = {:e} (expected {expected:e})", sigma[at]);
                assert!(within_1e6(sigma[at], expected), "sigma{at:?} = {:e}, expected {expected:e}", sigma[at]);
            }
            println!("legacy invvar r: weight[128, 2978] = {}, sigma = {}", loaded.arr[[128, 2978]], sigma[[128, 2978]]);
            assert_eq!(loaded.arr[[128, 2978]], 0.0);
            assert!(sigma[[128, 2978]].is_nan());
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1/des-dr2_A_r_extn3.fits"]
    fn real_des_weight_is_an_assumed_inverse_variance() {
        let loaded = load_plane(&ImageRef::auto(&survey_file("D1", "des-dr2_A_r_extn3.fits"))).unwrap();
        println!("des weight: plane {:?}, layer {:?}", loaded.info.kind, loaded.info.pairing.layer);
        assert_eq!(
            loaded.info.pairing.layer,
            Some(LayerGuess { role: LayerRole::InverseVariance, basis: "EXTNAME WGT".to_string(), assumed: true })
        );
        let (sigma, no_data) = sigma_with_no_data(LayerRole::InverseVariance, &loaded.arr).unwrap();
        let median = median_of(&sigma);
        println!(
            "des weight: sigma[0, 0] = {}, sigma[229, 229] = {}, median sigma = {median}, no-data pixels {no_data}",
            sigma[[0, 0]],
            sigma[[229, 229]]
        );
        assert!(within_1e6(sigma[[0, 0]], 2.452611726935556));
        assert!(within_1e6(sigma[[229, 229]], 2.1765951301075823));
        assert!(((median - 2.339353610712241) / 2.339353610712241).abs() <= 1e-6, "median {median}");
        assert_eq!(no_data, 0);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1/EUC_MER_MOSAIC-VIS-RMS_TILE102044185-8BF2FC_20241021T010718.207983Z_00.00_cut60_A.fits"]
    fn real_euclid_rms_is_a_sigma_with_22_sentinel_pixels_as_no_data() {
        let path = survey_file("E1", "EUC_MER_MOSAIC-VIS-RMS_TILE102044185-8BF2FC_20241021T010718.207983Z_00.00_cut60_A.fits");
        let loaded = load_plane(&ImageRef::auto(&path)).unwrap();
        println!("euclid rms: plane {:?}, layer {:?}", loaded.info.kind, loaded.info.pairing.layer);
        assert_eq!(
            loaded.info.pairing.layer,
            Some(LayerGuess { role: LayerRole::Sigma, basis: "file name -RMS".to_string(), assumed: false })
        );
        let (sigma, no_data) = sigma_with_no_data(LayerRole::Sigma, &loaded.arr).unwrap();
        println!(
            "euclid rms: no-data pixels {no_data}, sigma[0, 0] = {:e}, sigma[300, 300] = {:e}, raw[70, 541] = {:e}",
            sigma[[0, 0]],
            sigma[[300, 300]],
            loaded.arr[[70, 541]]
        );
        assert_eq!(no_data, 22);
        for at in [[70, 541], [172, 304], [361, 117]] {
            assert!(sigma[at].is_nan(), "sigma{at:?} = {}", sigma[at]);
        }
        assert_eq!(sigma[[0, 0]], 0.004187881480902433_f32);
        assert_eq!(sigma[[300, 300]], 0.004112852271646261_f32);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_r_wt_480px.fits, survey/P1/ps1_A_r_stack_480px.fits"]
    fn real_ps1_weight_cutout_is_a_variance_by_its_ps_covar_card() {
        let loaded = load_plane(&ImageRef::auto(&survey_file("P1", "ps1_A_r_wt_480px.fits"))).unwrap();
        println!("ps1 wt: plane {:?}, layer {:?}", loaded.info.kind, loaded.info.pairing.layer);
        assert_eq!(
            loaded.info.pairing.layer,
            Some(LayerGuess {
                role: LayerRole::Variance,
                basis: "PS_COVAR card (PS1 variance layer)".to_string(),
                assumed: true,
            })
        );
        let (sigma, no_data) = sigma_with_no_data(LayerRole::Variance, &loaded.arr).unwrap();
        let median = median_of(&sigma);
        println!(
            "ps1 wt: sigma[0, 0] = {}, sigma[240, 240] = {}, median sigma = {median}, no-data pixels {no_data}",
            sigma[[0, 0]],
            sigma[[240, 240]]
        );
        assert!(within_1e6(sigma[[0, 0]], 65.84874803052446));
        assert!(within_1e6(sigma[[240, 240]], 81.00975900894595));
        assert!(((median - 72.20157127466999) / 72.20157127466999).abs() <= 1e-6, "median {median}");
        assert_eq!(no_data, 0);

        let stack = load_plane(&ImageRef::auto(&survey_file("P1", "ps1_A_r_stack_480px.fits"))).unwrap();
        println!("ps1 stack: layer {:?}", stack.info.pairing.layer);
        assert_eq!(stack.info.pairing.layer.map(|l| l.role), Some(LayerRole::Science));
    }

    fn pixels_with(plane: &IntPlane, mask: u32) -> usize {
        plane.bits.iter().filter(|bits| *bits & mask != 0).count()
    }

    fn bits_seen(plane: &IntPlane) -> u32 {
        crate::core::imaging::dq_flags::bits_present(&plane.bits)
    }

    fn mask_guess(basis: &str, assumed: bool) -> Option<LayerGuess> {
        Some(LayerGuess { role: LayerRole::Mask, basis: basis.to_string(), assumed })
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1/EUC_MER_MOSAIC-VIS-FLAG_TILE102044185-A61C42_20241021T010718.208008Z_00.00_cut60_A.fits"]
    fn real_euclid_flag_layer_keeps_values_above_2_pow_24_and_excludes_nothing() {
        let path = survey_file("E1", "EUC_MER_MOSAIC-VIS-FLAG_TILE102044185-A61C42_20241021T010718.208008Z_00.00_cut60_A.fits");
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::auto(&path));
        let info = active.plane_info().unwrap();
        println!("euclid flag: plane {:?}, layer {:?}, notes {:?}", info.kind, info.pairing.layer, info.pairing.notes);
        assert_eq!(info.pairing.layer, mask_guess("file name -FLAG", false));
        assert!(info.is_dq);
        let plane = active.int_plane().expect("the FLAG layer is read as integers");
        let flagged = pixels_with(plane, u32::MAX);
        println!(
            "euclid flag: {} x {} px, value[540, 591] = {}, bits present {}, pixels with bit 24: {}, non-zero pixels {flagged}",
            plane.bits.ncols(),
            plane.bits.nrows(),
            plane.value_at(540, 591),
            bits_seen(plane),
            pixels_with(plane, 1 << 24)
        );
        assert_eq!(plane.bits.dim(), (601, 601));
        assert_eq!(plane.value_at(540, 591), 25296896);
        assert_eq!(bits_seen(plane), 25301342);
        assert_eq!(pixels_with(plane, 1 << 24), 27331);
        assert_eq!(flagged, 81015);

        let flags = comps.flags.as_ref().expect("the FLAG layer is its own DQ layer");
        println!("euclid flag: flag set {:?}, exclusion {}", flags.source, comps.exclusion_mask());
        assert_eq!(flags.source, "unknown (raw bits)");
        assert!(!flags.is_named());
        assert_eq!(comps.exclusion_mask(), 0);
        assert_eq!(info.pairing.notes, vec![UNNAMED_FLAGS_NOTE.to_string()]);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L1/ls-dr10_A_griz_invvar_maskbits_458px.fits"]
    fn real_legacy_cutout_mask_is_named_by_imagetyp() {
        let path = survey_file("L1", "ls-dr10_A_griz_invvar_maskbits_458px.fits");
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::hdu(&path, 2));
        let info = active.plane_info().unwrap();
        println!("legacy cutout mask: plane {:?}, layer {:?}, notes {:?}", info.kind, info.pairing.layer, info.pairing.notes);
        assert_eq!(info.pairing.layer, mask_guess("IMAGETYP MASKBITS", false));
        assert!(info.is_dq);
        let plane = active.int_plane().expect("the maskbits layer is read as integers");
        for (at, expected) in [
            ([0, 9], 0),
            ([0, 301], 1),
            ([30, 7], 256),
            ([0, 0], 1024),
            ([7, 380], 1025),
            ([30, 0], 1280),
            ([429, 453], 2048),
        ] {
            println!("legacy cutout mask: value{at:?} = {} (expected {expected})", plane.bits[at]);
            assert_eq!(plane.bits[at], expected, "{at:?}");
        }
        println!("legacy cutout mask: bits present {}", bits_seen(plane));
        assert_eq!(bits_seen(plane), 3329);

        let flags = comps.flags.as_ref().expect("the maskbits layer is its own DQ layer");
        println!(
            "legacy cutout mask: flag set {:?}, exclusion {}, saturated {}, 1025 = {:?}",
            flags.source,
            comps.exclusion_mask(),
            comps.saturated_mask(),
            flags.decode(1025)
        );
        assert_eq!(flags.source, "Legacy DR10 maskbits (built-in table)");
        assert_eq!(flags.flags.len(), 17);
        assert_eq!(flags.decode(1025), vec!["NPRIMARY".to_string(), "BAILOUT".to_string()]);
        assert!(info.pairing.notes.is_empty());
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-maskbits.fits.fz"]
    fn real_legacy_brick_mask_names_come_from_its_header() {
        let path = survey_file("L2", "legacysurvey-0532m280-maskbits.fits.fz");
        let header = load_plane_header(&ImageRef::hdu(&path, 1)).unwrap();
        let mut r_band = HduHeader::empty();
        r_band.set("FILTERX", "r".to_string());
        let flags = FlagSet::from_headers(Some(&header), Some(&r_band), DqTable::Unknown);
        println!(
            "legacy brick mask: flag set {:?}, {} flags, exclusion {} in r, saturated {}",
            flags.source,
            flags.flags.len(),
            flags.exclusion_default,
            flags.saturated_mask
        );
        assert_eq!(flags.flags.len(), 17);
        assert_eq!(flags.source, "MBIT_ cards (Legacy maskbits)");
        assert_eq!(flags.exclusion_default, 72);
        assert_eq!(flags.flags.last(), Some(&(65536, "SUB_BLOB".to_string())));

        let loaded = load_plane(&ImageRef::auto(&path)).unwrap();
        println!(
            "legacy brick mask: plane {:?}, layer {:?}, integer plane unavailable: {:?}",
            loaded.info.kind, loaded.info.pairing.layer, loaded.info.pairing.int_unavailable
        );
        assert!(loaded.info.is_dq);
        let plane = loaded.int_plane.as_ref().expect("the tile-compressed maskbits plane is read as integers");
        for (at, expected) in [([0, 0], 1), ([1799, 1799], 0), ([100, 2500], 1024), ([86, 86], 1025)] {
            println!("legacy brick mask: value{at:?} = {} (expected {expected})", plane.bits[at]);
            assert_eq!(plane.bits[at], expected, "{at:?}");
        }
        println!("legacy brick mask: bits present {}", bits_seen(plane));
        assert_eq!(bits_seen(plane), 57343);
        for (bit, expected) in [(0, 1183084), (3, 74604), (10, 6334143)] {
            let counted = pixels_with(plane, 1 << bit);
            println!("legacy brick mask: pixels with bit {bit}: {counted} (expected {expected})");
            assert_eq!(counted, expected, "bit {bit}");
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1/des-dr2_A_r_extn2.fits"]
    fn real_des_mask_cutout_is_all_good() {
        let path = survey_file("D1", "des-dr2_A_r_extn2.fits");
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::auto(&path));
        let info = active.plane_info().unwrap();
        println!("des mask: plane {:?}, layer {:?}, notes {:?}", info.kind, info.pairing.layer, info.pairing.notes);
        assert_eq!(info.pairing.layer, mask_guess("EXTNAME MSK", false));
        assert!(info.is_dq);
        let plane = active.int_plane().expect("the MSK layer is read as integers");
        let flags = comps.flags.as_ref().expect("the MSK layer is its own DQ layer");
        println!(
            "des mask: {} x {} px, bits present {}, flag set {:?}, exclusion {:#x}",
            plane.bits.ncols(),
            plane.bits.nrows(),
            bits_seen(plane),
            flags.source,
            comps.exclusion_mask()
        );
        assert_eq!(bits_seen(plane), 0);
        assert_eq!(flags.source, "DES coadd mask (non-zero = no good data)");
        assert_eq!(comps.exclusion_mask(), u32::MAX);
        assert!(info.pairing.notes.is_empty());
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_r_mask_480px.fits"]
    fn real_ps1_mask_cutout_is_reported_unusable() {
        let path = survey_file("P1", "ps1_A_r_mask_480px.fits");
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::auto(&path));
        let info = active.plane_info().unwrap();
        println!(
            "ps1 mask: plane {:?}, layer {:?}, integer plane unavailable: {:?}",
            info.kind, info.pairing.layer, info.pairing.int_unavailable
        );
        assert_eq!(info.pairing.layer, mask_guess("MSKNAM cards (PS1 mask layer)", true));
        assert!(info.is_dq);
        assert!(active.int_plane().is_none());
        assert_eq!(info.pairing.int_unavailable.as_deref(), Some(FLOAT_PLANE_REASON));
        let no_data = no_data_pixels(&active);
        println!("ps1 mask: {no_data} of {} pixels are NaN; {:?}", active.arr().len(), comps.dq_unavailable);
        assert_eq!((no_data, active.arr().len()), (230400, 230400));
        assert!(comps.dq.is_none());
        assert!(comps.flags.is_none());
        assert_eq!(comps.dq_unavailable, Some(format!("mask layer HDU 0 is a {FLOAT_PLANE_REASON}")));

        let named = FlagSet::from_headers(active.header(), None, DqTable::Unknown);
        println!(
            "ps1 mask: flag set {:?}, flags {:?}, default exclusion {}, saturated {}",
            named.source, named.flags, named.exclusion_default, named.saturated_mask
        );
        assert_eq!(named.source, "MSKNAM cards (PS1 mask)");
        assert_eq!(named.flags.len(), 16);
        assert!(named.flags.contains(&(128, "SUSPECT/BURNTOOL".to_string())));
        assert!(named.flags.contains(&(32768, "MARK".to_string())));
        assert_eq!(named.exclusion_default, 8575);
        assert_eq!(named.saturated_mask, 32);
    }

    const LEGACY_IMAGE_NAME: &str = "legacysurvey-0001p000-image-r.fits";
    const LEGACY_INVVAR_NAME: &str = "legacysurvey-0001p000-invvar-r.fits";
    const LEGACY_MASKBITS_NAME: &str = "legacysurvey-0001p000-maskbits.fits";
    const EUCLID_SCIENCE_NAME: &str = "EUC_MER_BGSUB-MOSAIC-VIS_TILE1-AAA_t1_00.00.fits";
    const EUCLID_RMS_NAME: &str = "EUC_MER_MOSAIC-VIS-RMS_TILE1-BBB_t2_00.00.fits";
    const EUCLID_FLAG_NAME: &str = "EUC_MER_MOSAIC-VIS-FLAG_TILE1-CCC_t3_00.00.fits";
    const PS1_STACK_NAME: &str = "rings.v3.skycell.0647.098.stk.r.unconv.fits";
    const PS1_WT_NAME: &str = "rings.v3.skycell.0647.098.stk.r.unconv.wt.fits";
    const PS1_MASK_NAME: &str = "rings.v3.skycell.0647.098.stk.r.unconv.mask.fits";
    const SIBLING_COLS: usize = 8;
    const SIBLING_ROWS: usize = 6;

    fn in_folder(dir: &Path, name: &str) -> String {
        dir.join(name).to_str().unwrap().to_string()
    }

    fn grid_header(cards: &[(&str, &str)]) -> HduHeader {
        let mut header = header_with_cd(north_up_cd());
        header.set("NAXIS1", SIBLING_COLS.to_string());
        header.set("NAXIS2", SIBLING_ROWS.to_string());
        for (key, value) in cards {
            header.set(key, value.to_string());
        }
        header
    }

    fn grid_cards() -> Vec<(&'static str, String)> {
        let header = grid_header(&[]);
        GRID_CARDS.iter().filter_map(|card| header.get(card).map(|value| (*card, value.to_string()))).collect()
    }

    fn write_grid_plane(path: &str, values: &Array2<f32>, cards: &[(&str, &str)]) {
        write_fits_mono(path, values, Some(&grid_header(cards))).unwrap();
    }

    fn sibling_ramp() -> Array2<f32> {
        Array2::from_shape_fn((SIBLING_ROWS, SIBLING_COLS), |(y, x)| (y * SIBLING_COLS + x) as f32)
    }

    fn sibling_choice(role: LayerRole, basis: &str) -> Option<CompanionChoice> {
        Some(CompanionChoice { role, basis: basis.to_string(), assumed: false, origin: CompanionOrigin::Sibling })
    }

    fn taken(kind: &str, file: &str, rule: &str) -> String {
        format!("{kind} layer taken from the sibling file {file} (name rule {rule}, same 8x6 grid)")
    }

    struct SiblingSet {
        image: String,
        err: String,
        dq: String,
    }

    fn legacy_brick(dir: &Path) -> SiblingSet {
        let set = SiblingSet {
            image: in_folder(dir, LEGACY_IMAGE_NAME),
            err: in_folder(dir, LEGACY_INVVAR_NAME),
            dq: in_folder(dir, LEGACY_MASKBITS_NAME),
        };
        write_grid_plane(&set.image, &sibling_ramp(), &[("FILTERX", "r"), ("BUNIT", "nanomaggy")]);
        let mut weights = Array2::from_elem((SIBLING_ROWS, SIBLING_COLS), 4.0_f32);
        weights[[2, 3]] = 0.0;
        write_grid_plane(&set.err, &weights, &[("FILTERX", "r"), ("BUNIT", "1/nanomaggy^2")]);
        let mut bits = vec![0i32; SIBLING_ROWS * SIBLING_COLS];
        bits[10] = 8;
        let mut cards = grid_cards();
        cards.push(("MBIT_3", "'SATUR_R'".into()));
        cards.push(("MBIT_6", "'ALLMASK_R'".into()));
        write_test_mef(
            Path::new(&set.dq),
            &[],
            &[TestHdu { extname: Some("MASKBITS"), extver: None, cols: SIBLING_COLS, rows: SIBLING_ROWS, data: HduData::I32(bits), extra_cards: cards }],
        );
        set
    }

    fn write_int_primary(path: &str, cols: usize, rows: usize, values: &[i32]) {
        let mut cards: Vec<(&'static str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", cols.to_string()),
            ("NAXIS2", rows.to_string()),
        ];
        cards.extend(grid_cards());
        write_raw_hdus(Path::new(path), &[(cards, values.iter().flat_map(|v| v.to_be_bytes()).collect())]);
    }

    fn euclid_rms() -> Array2<f32> {
        let mut rms = Array2::from_elem((SIBLING_ROWS, SIBLING_COLS), 0.25_f32);
        rms[[3, 1]] = 1.0e16;
        rms
    }

    fn euclid_tile(dir: &Path) -> SiblingSet {
        let set = SiblingSet {
            image: in_folder(dir, EUCLID_SCIENCE_NAME),
            err: in_folder(dir, EUCLID_RMS_NAME),
            dq: in_folder(dir, EUCLID_FLAG_NAME),
        };
        write_grid_plane(&set.image, &sibling_ramp(), &[]);
        write_grid_plane(&set.err, &euclid_rms(), &[]);
        let mut flags = vec![0i32; SIBLING_ROWS * SIBLING_COLS];
        flags[21] = 1 << 24;
        write_int_primary(&set.dq, SIBLING_COLS, SIBLING_ROWS, &flags);
        write_grid_plane(&in_folder(dir, "EUC_MER_MOSAIC-VIS-RMS_TILE2-DDD_t4_00.00.fits"), &euclid_rms(), &[]);
        set
    }

    #[test]
    fn a_legacy_brick_image_takes_its_invvar_and_maskbits_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let brick = legacy_brick(dir.path());
        let loaded = load_plane(&ImageRef::auto(&brick.image)).unwrap();
        assert_eq!(loaded.companions.err, Some(ImageRef::auto(&brick.err)));
        assert_eq!(loaded.companions.dq, Some(ImageRef::auto(&brick.dq)));
        let pairing = &loaded.info.pairing;
        assert_eq!(pairing.err, sibling_choice(LayerRole::InverseVariance, "file name -invvar-"));
        assert_eq!(pairing.dq, sibling_choice(LayerRole::Mask, "file name -maskbits"));
        assert_eq!(
            pairing.notes,
            vec![taken("error", LEGACY_INVVAR_NAME, "Legacy brick"), taken("mask", LEGACY_MASKBITS_NAME, "Legacy brick")]
        );
        assert_eq!((pairing.err_unavailable.as_deref(), pairing.dq_unavailable.as_deref()), (None, None));

        let cache = ImageCache::new(8, usize::MAX);
        let (_, comps) = open_with_companions(&cache, &ImageRef::auto(&brick.image));
        let err = comps.err.as_ref().expect("the sibling invvar is the error layer");
        assert_eq!(err.arr()[[0, 0]], 0.5);
        assert!(err.arr()[[2, 3]].is_nan(), "a zero weight is no data");
        assert_eq!(no_data_pixels(err), 1);
        assert_eq!(err.header().and_then(|h| h.get("BUNIT")), Some("nanomaggy"));
        let origin = comps.err_origin.as_ref().expect("origin of the sibling error layer");
        assert_eq!(origin.source, "file name -invvar-, legacysurvey-0001p000-invvar-r.fits: 1 / square root");
        assert_eq!((origin.role, origin.assumed, origin.n_no_data), (LayerRole::InverseVariance, false, 1));
        let (dq, _) = comps.dq.as_ref().expect("the sibling maskbits is the mask layer");
        assert_eq!(dq.int_plane().unwrap().value_at(1, 2), 8);
        assert_eq!(comps.exclusion_mask(), 72);
        assert_eq!(comps.saturated_mask(), 8);
        assert_eq!(comps.flags.as_ref().map(|f| f.source.as_str()), Some("MBIT_ cards (Legacy maskbits)"));
        assert_eq!(comps.dq_origin.as_deref(), Some("file name -maskbits, legacysurvey-0001p000-maskbits.fits"));
    }

    #[test]
    fn a_euclid_mosaic_takes_the_rms_and_flag_files_of_its_tile_whatever_their_hash() {
        let dir = tempfile::tempdir().unwrap();
        let tile = euclid_tile(dir.path());
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::auto(&tile.image));
        assert_eq!(active.companions().unwrap().err, Some(ImageRef::auto(&tile.err)));
        assert_eq!(active.companions().unwrap().dq, Some(ImageRef::auto(&tile.dq)));
        let pairing = &active.plane_info().unwrap().pairing;
        assert_eq!(pairing.err, sibling_choice(LayerRole::Sigma, "file name -RMS"));
        assert_eq!(pairing.dq, sibling_choice(LayerRole::Mask, "file name -FLAG"));
        assert_eq!(
            pairing.notes,
            vec![
                taken("error", EUCLID_RMS_NAME, "Euclid MER mosaic"),
                taken("mask", EUCLID_FLAG_NAME, "Euclid MER mosaic"),
                UNNAMED_FLAGS_NOTE.to_string(),
            ]
        );
        assert!(pairing.notes.iter().all(|note| !note.contains("TILE2")), "{:?}", pairing.notes);

        let err = comps.err.as_ref().expect("the RMS file is the error layer");
        for ((at, sigma), raw) in err.arr().indexed_iter().zip(euclid_rms().iter()) {
            if at == (3, 1) {
                assert!(sigma.is_nan(), "the 1e16 sentinel is no data");
            } else {
                assert_eq!(sigma.to_bits(), raw.to_bits(), "{at:?}");
            }
        }
        let origin = comps.err_origin.as_ref().expect("origin");
        assert_eq!(origin.source, format!("file name -RMS, {EUCLID_RMS_NAME}: as is"));
        assert_eq!(origin.n_no_data, 1);
        assert_eq!(err.header().and_then(|h| h.get("BUNIT")), None);
        let (dq, _) = comps.dq.as_ref().expect("the FLAG file is the mask layer");
        assert_eq!(dq.int_plane().unwrap().value_at(2, 5), 1 << 24);
        assert_eq!(comps.flags.as_ref().map(|f| f.source.as_str()), Some("unknown (raw bits)"));
        assert_eq!(comps.exclusion_mask(), 0);
        assert_eq!(comps.dq_origin, Some(format!("file name -FLAG, {EUCLID_FLAG_NAME}")));
    }

    #[test]
    fn two_rms_files_of_one_tile_are_told_apart_by_the_grid_or_left_to_the_user() {
        let other_shape = tempfile::tempdir().unwrap();
        let tile = euclid_tile(other_shape.path());
        let small = "EUC_MER_MOSAIC-VIS-RMS_TILE1-ZZZ_t9_00.00.fits";
        write_fits_mono(&in_folder(other_shape.path(), small), &Array2::from_elem((4, 4), 0.5), Some(&header_with_cd(north_up_cd())))
            .unwrap();
        let loaded = load_plane(&ImageRef::auto(&tile.image)).unwrap();
        assert_eq!(loaded.companions.err, Some(ImageRef::auto(&tile.err)));
        assert_eq!(
            loaded.info.pairing.notes,
            vec![
                format!("error layer {small} is 4x4 and does not match the image 8x6: not used"),
                taken("error", EUCLID_RMS_NAME, "Euclid MER mosaic"),
                taken("mask", EUCLID_FLAG_NAME, "Euclid MER mosaic"),
                UNNAMED_FLAGS_NOTE.to_string(),
            ]
        );

        let twins = tempfile::tempdir().unwrap();
        let tile = euclid_tile(twins.path());
        write_grid_plane(&in_folder(twins.path(), "EUC_MER_MOSAIC-VIS-RMS_TILE1-YYY_t8_00.00.fits"), &euclid_rms(), &[]);
        let loaded = load_plane(&ImageRef::auto(&tile.image)).unwrap();
        assert_eq!(loaded.companions.err, None);
        assert_eq!(loaded.info.pairing.err, None);
        assert_eq!(loaded.info.pairing.err_unavailable, None);
        assert_eq!(loaded.companions.dq, Some(ImageRef::auto(&tile.dq)));
        assert_eq!(
            loaded.info.pairing.notes,
            vec![
                "2 files match the Euclid MER mosaic name rule for the error layer: none applied, assign one by hand".to_string(),
                taken("mask", EUCLID_FLAG_NAME, "Euclid MER mosaic"),
                UNNAMED_FLAGS_NOTE.to_string(),
            ]
        );

        let shifted = tempfile::tempdir().unwrap();
        let tile = euclid_tile(shifted.path());
        write_grid_plane(&tile.err, &euclid_rms(), &[("CRPIX1", "51.5")]);
        let loaded = load_plane(&ImageRef::auto(&tile.image)).unwrap();
        assert_eq!(loaded.companions.err, None);
        assert_eq!(
            loaded.info.pairing.notes[0],
            format!(
                "{EUCLID_RMS_NAME} matches the Euclid MER mosaic name rule but CRPIX1 differs: image 50.5 against layer 51.5: not applied automatically"
            )
        );
        assert_eq!(loaded.info.pairing.notes.len(), 3, "{:?}", loaded.info.pairing.notes);
    }

    #[test]
    fn a_ps1_stack_takes_its_wt_and_mask_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let image = in_folder(dir.path(), PS1_STACK_NAME);
        let wt = in_folder(dir.path(), PS1_WT_NAME);
        let mask = in_folder(dir.path(), PS1_MASK_NAME);
        write_grid_plane(&image, &sibling_ramp(), &[]);
        write_grid_plane(&wt, &Array2::from_elem((SIBLING_ROWS, SIBLING_COLS), 4.0), &[]);
        let mut bits = vec![0i16; SIBLING_ROWS * SIBLING_COLS];
        bits[4] = 32;
        let mut cards = grid_cards();
        cards.extend([
            ("MSKNAM00", "'DETECTOR'".to_string()),
            ("MSKVAL00", "1".to_string()),
            ("MSKNAM01", "'SAT'".to_string()),
            ("MSKVAL01", "32".to_string()),
            ("MSKNAM02", "'MASK.VALUE'".to_string()),
            ("MSKVAL02", "33".to_string()),
        ]);
        write_test_mef(
            Path::new(&mask),
            &[],
            &[TestHdu { extname: None, extver: None, cols: SIBLING_COLS, rows: SIBLING_ROWS, data: HduData::I16(bits), extra_cards: cards }],
        );

        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::auto(&image));
        assert_eq!(active.companions().unwrap().err, Some(ImageRef::auto(&wt)));
        assert_eq!(active.companions().unwrap().dq, Some(ImageRef::auto(&mask)));
        let pairing = &active.plane_info().unwrap().pairing;
        assert_eq!(pairing.err, sibling_choice(LayerRole::Variance, "file name .wt"));
        assert_eq!(pairing.dq, sibling_choice(LayerRole::Mask, "file name .mask"));
        assert_eq!(pairing.notes, vec![taken("error", PS1_WT_NAME, "PS1 stack"), taken("mask", PS1_MASK_NAME, "PS1 stack")]);
        let err = comps.err.as_ref().expect("the .wt file is the error layer");
        assert!(err.arr().iter().all(|v| *v == 2.0));
        assert_eq!(comps.err_origin.as_ref().map(|o| o.source.as_str()), Some(format!("file name .wt, {PS1_WT_NAME}: square root").as_str()));
        assert_eq!(comps.exclusion_mask(), 33);
        assert_eq!(comps.saturated_mask(), 32);
        assert_eq!(comps.dq.as_ref().unwrap().0.int_plane().unwrap().value_at(0, 4), 32);
    }

    #[test]
    fn a_ps1_weight_sibling_with_its_ps_covar_card_names_the_card_in_its_origin() {
        let dir = tempfile::tempdir().unwrap();
        let image = in_folder(dir.path(), PS1_STACK_NAME);
        let wt = in_folder(dir.path(), PS1_WT_NAME);
        write_grid_plane(&image, &sibling_ramp(), &[]);
        write_grid_plane(&wt, &Array2::from_elem((SIBLING_ROWS, SIBLING_COLS), 4.0), &[("PS_COVAR", "T")]);
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::auto(&image));
        let basis = "file name .wt and PS_COVAR card (PS1 variance layer)";
        assert_eq!(active.plane_info().unwrap().pairing.err, sibling_choice(LayerRole::Variance, basis));
        assert_eq!(comps.err_origin.map(|o| o.source), Some(format!("{basis}, {PS1_WT_NAME}: square root")));
        assert!(comps.err.expect("the .wt file is the error layer").arr().iter().all(|v| *v == 2.0));
        assert_eq!(resolve_plane_info(&ImageRef::auto(&image)).unwrap().pairing.err, sibling_choice(LayerRole::Variance, basis));
    }

    #[test]
    fn a_float_mask_sibling_is_not_used_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let tile = euclid_tile(dir.path());
        write_grid_plane(&tile.dq, &Array2::zeros((SIBLING_ROWS, SIBLING_COLS)), &[]);
        let loaded = load_plane(&ImageRef::auto(&tile.image)).unwrap();
        assert_eq!(loaded.companions.dq, None);
        assert_eq!(loaded.info.pairing.dq, None);
        assert_eq!(loaded.companions.err, Some(ImageRef::auto(&tile.err)));
        assert_eq!(
            loaded.info.pairing.notes,
            vec![
                taken("error", EUCLID_RMS_NAME, "Euclid MER mosaic"),
                format!(
                    "{EUCLID_FLAG_NAME} matches the Euclid MER mosaic name rule but is a floating-point plane (BITPIX -32): not used as a mask"
                ),
            ]
        );
    }

    #[test]
    fn a_layer_inside_the_file_wins_over_a_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let brick = legacy_brick(dir.path());
        let mut cards = grid_cards();
        cards.push(("BUNIT", "'nanomaggy'".into()));
        write_test_mef(
            Path::new(&brick.image),
            &[],
            &[
                TestHdu { extname: Some("SCI"), extver: None, cols: SIBLING_COLS, rows: SIBLING_ROWS, data: HduData::F32(ramp_f32(SIBLING_COLS, SIBLING_ROWS)), extra_cards: cards },
                TestHdu { extname: Some("ERR"), extver: None, cols: SIBLING_COLS, rows: SIBLING_ROWS, data: HduData::F32(vec![0.1; SIBLING_ROWS * SIBLING_COLS]), extra_cards: vec![] },
            ],
        );
        let loaded = load_plane(&ImageRef::auto(&brick.image)).unwrap();
        assert_eq!(loaded.companions.err, Some(ImageRef::hdu(&brick.image, 2)));
        assert_eq!(loaded.info.pairing.err.as_ref().map(|c| c.origin), Some(CompanionOrigin::SameFile));
        assert!(loaded.info.pairing.notes.iter().all(|note| !note.contains(LEGACY_INVVAR_NAME)), "{:?}", loaded.info.pairing.notes);
        assert_eq!(loaded.companions.dq, Some(ImageRef::auto(&brick.dq)));
    }

    #[test]
    fn a_file_without_a_name_rule_or_without_siblings_is_untouched() {
        let dir = tempfile::tempdir().unwrap();
        legacy_brick(dir.path());
        let plain = in_folder(dir.path(), "image.fits");
        write_grid_plane(&plain, &sibling_ramp(), &[("FILTERX", "r")]);
        let alone = tempfile::tempdir().unwrap();
        let lonely = in_folder(alone.path(), LEGACY_IMAGE_NAME);
        write_grid_plane(&lonely, &sibling_ramp(), &[("FILTERX", "r")]);
        for path in [&plain, &lonely] {
            let loaded = load_plane(&ImageRef::auto(path)).unwrap();
            assert_eq!(loaded.companions, Companions::default(), "{path}");
            assert!(loaded.info.pairing.notes.is_empty(), "{path}: {:?}", loaded.info.pairing.notes);
            assert_eq!(resolve_plane_info(&ImageRef::auto(path)).unwrap().pairing, loaded.info.pairing, "{path}");
        }
    }

    #[test]
    fn a_rewritten_sibling_refreshes_the_derived_sigma() {
        let dir = tempfile::tempdir().unwrap();
        let brick = legacy_brick(dir.path());
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::auto(&brick.image));
        assert_eq!(comps.err.expect("the sibling invvar").arr()[[0, 0]], 0.5);

        write_grid_plane(&brick.err, &Array2::from_elem((SIBLING_ROWS, SIBLING_COLS), 16.0), &[("BUNIT", "1/nanomaggy^2")]);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&brick.err)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        let again = load_companions_into(&cache, &active, plane_load);
        assert_eq!(again.err.expect("the rewritten invvar").arr()[[0, 0]], 0.25, "the sigma of the old file was served");
        assert_eq!(again.err_origin.map(|o| o.n_no_data), Some(0));
    }

    #[test]
    fn resolve_plane_info_and_load_plane_agree_with_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let brick = legacy_brick(dir.path());
        for path in [&brick.image, &brick.err, &brick.dq] {
            let r = ImageRef::auto(path);
            assert_eq!(resolve_plane_info(&r).unwrap().pairing, load_plane(&r).unwrap().info.pairing, "{path}");
        }
        assert_eq!(resolve_plane_info(&ImageRef::auto(&brick.image)).unwrap().pairing.notes.len(), 2);
    }

    #[test]
    fn an_error_or_mask_file_opened_on_its_own_gets_no_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let brick = legacy_brick(dir.path());
        let loaded = load_plane(&ImageRef::auto(&brick.err)).unwrap();
        assert_eq!(loaded.companions, Companions::default());
        assert_eq!(loaded.info.pairing.layer.as_ref().map(|l| l.role), Some(LayerRole::InverseVariance));
        assert!(loaded.info.pairing.notes.is_empty());
        let mask = load_plane(&ImageRef::auto(&brick.dq)).unwrap();
        assert_eq!(mask.companions.err, None);
        assert_eq!(mask.info.pairing.dq.as_ref().map(|c| c.origin), Some(CompanionOrigin::SameFile));
    }

    #[test]
    fn a_gzipped_legacy_image_finds_its_siblings_in_its_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        let brick = legacy_brick(dir.path());
        let gzipped = in_folder(dir.path(), "legacysurvey-0001p000-image-r.fits.gz");
        std::fs::write(&gzipped, gzip_bytes(&std::fs::read(&brick.image).unwrap())).unwrap();
        std::fs::remove_file(&brick.image).unwrap();

        let loaded = load_plane(&ImageRef::auto(&gzipped)).unwrap();
        assert_eq!(loaded.companions.err, Some(ImageRef::auto(&brick.err)));
        assert_eq!(loaded.companions.dq, Some(ImageRef::auto(&brick.dq)));
        assert_eq!(loaded.info.pairing.err, sibling_choice(LayerRole::InverseVariance, "file name -invvar-"));
        assert_eq!(loaded.info.pairing.dq, sibling_choice(LayerRole::Mask, "file name -maskbits"));
        assert_eq!(
            loaded.info.pairing.notes,
            vec![taken("error", LEGACY_INVVAR_NAME, "Legacy brick"), taken("mask", LEGACY_MASKBITS_NAME, "Legacy brick")]
        );
        assert_eq!(resolve_plane_info(&ImageRef::auto(&gzipped)).unwrap().pairing, loaded.info.pairing);
    }

    const EUCLID_E1_SCIENCE: &str = "EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits";
    const EUCLID_E1_RMS: &str = "EUC_MER_MOSAIC-VIS-RMS_TILE102044185-8BF2FC_20241021T010718.207983Z_00.00_cut60_A.fits";
    const EUCLID_E1_FLAG: &str = "EUC_MER_MOSAIC-VIS-FLAG_TILE102044185-A61C42_20241021T010718.208008Z_00.00_cut60_A.fits";

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1 science, RMS and FLAG files of tile 102044185"]
    fn real_euclid_science_cutout_pairs_its_rms_and_flag_files() {
        let science = survey_file("E1", EUCLID_E1_SCIENCE);
        let rms = survey_file("E1", EUCLID_E1_RMS);
        let flag = survey_file("E1", EUCLID_E1_FLAG);
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::auto(&science));
        let pairing = &active.plane_info().unwrap().pairing;
        println!("euclid science: companions {:?}", active.companions());
        println!("euclid science: notes {:?}", pairing.notes);
        assert_eq!(active.companions().unwrap().err, Some(ImageRef::auto(&rms)));
        assert_eq!(active.companions().unwrap().dq, Some(ImageRef::auto(&flag)));
        for (kind, file) in [("error", EUCLID_E1_RMS), ("mask", EUCLID_E1_FLAG)] {
            let note = format!("{kind} layer taken from the sibling file {file} (name rule Euclid MER mosaic, same 601x601 grid)");
            assert!(pairing.notes.contains(&note), "{note}");
        }

        let raw = load_plane(&ImageRef::auto(&rms)).unwrap().arr;
        let err = comps.err.as_ref().expect("the RMS file is the error layer");
        let mut differing = 0;
        let mut sentinels = Vec::new();
        for ((at, sigma), value) in err.arr().indexed_iter().zip(raw.iter()) {
            if *value < 1.0e15 {
                differing += usize::from(sigma.to_bits() != value.to_bits());
            } else {
                assert!(sigma.is_nan(), "{at:?}");
                sentinels.push(at);
            }
        }
        let science_on_sentinels: Vec<f32> = sentinels.iter().map(|&(y, x)| active.arr()[[y, x]]).collect();
        println!(
            "euclid science: {differing} differing pixels of {}, {} sentinel pixels, science there {:?}",
            raw.len() - sentinels.len(),
            sentinels.len(),
            science_on_sentinels
        );
        assert_eq!(differing, 0);
        assert_eq!(raw.len() - sentinels.len(), 361179);
        assert_eq!(sentinels.len(), 22);
        for at in [(70, 541), (172, 304), (361, 117)] {
            assert!(sentinels.contains(&at), "{at:?}");
        }
        assert!(science_on_sentinels.iter().all(|v| v.to_bits() == 0.0_f32.to_bits()));
        let origin = comps.err_origin.as_ref().expect("origin");
        println!("euclid science: err origin {origin:?}, BUNIT {:?}", err.header().and_then(|h| h.get("BUNIT")));
        assert_eq!((origin.n_no_data, origin.role, origin.assumed), (22, LayerRole::Sigma, false));
        assert_eq!(err.header().and_then(|h| h.get("BUNIT")), None);

        let (dq, _) = comps.dq.as_ref().expect("the FLAG file is the mask layer");
        let flags = comps.flags.as_ref().expect("flag set");
        println!(
            "euclid science: flag set {:?}, exclusion {}, value[540, 591] = {}",
            flags.source,
            comps.exclusion_mask(),
            dq.int_plane().unwrap().value_at(540, 591)
        );
        assert_eq!(flags.source, "unknown (raw bits)");
        assert_eq!(comps.exclusion_mask(), 0);
        assert_eq!(dq.int_plane().unwrap().value_at(540, 591), 25296896);
    }

    fn legacy_brick_file(name: &str) -> String {
        survey_file("L2", &format!("legacysurvey-0532m280-{name}.fits.fz"))
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-{image,invvar}-{g,r,z}.fits.fz and -maskbits.fits.fz"]
    fn real_legacy_brick_image_pairs_its_invvar_and_maskbits_files() {
        for (band, no_data, exclusion) in [("r", 14346_u64, 72_u32), ("g", 4724, 36), ("z", 14128, 144)] {
            let image = legacy_brick_file(&format!("image-{band}"));
            let cache = ImageCache::new(8, usize::MAX);
            let (active, comps) = open_with_companions(&cache, &ImageRef::auto(&image));
            let pairing = &active.plane_info().unwrap().pairing;
            println!("legacy brick {band}: notes {:?}", pairing.notes);
            assert_eq!(active.companions().unwrap().err, Some(ImageRef::auto(&legacy_brick_file(&format!("invvar-{band}")))));
            assert_eq!(active.companions().unwrap().dq, Some(ImageRef::auto(&legacy_brick_file("maskbits"))));
            let origin = comps.err_origin.as_ref().expect("origin");
            let flags = comps.flags.as_ref().expect("flag set");
            println!(
                "legacy brick {band}: n_no_data {} (expected {no_data}), flag set {:?}, exclusion {} (expected {exclusion}), saturated {}",
                origin.n_no_data,
                flags.source,
                comps.exclusion_mask(),
                comps.saturated_mask()
            );
            assert_eq!(origin.n_no_data, no_data, "{band}");
            assert_eq!(comps.exclusion_mask(), exclusion, "{band}");
            if band != "r" {
                continue;
            }
            let err = comps.err.as_ref().expect("the invvar file is the error layer");
            for (at, expected) in [([0, 0], 7.175998180759543e-4), ([1799, 1799], 5.70805405704646e-4), ([2500, 100], 5.626682720415609e-4)] {
                println!("legacy brick r: sigma{at:?} = {:e} (expected {expected:e})", err.arr()[at]);
                assert!(within_1e6(err.arr()[at], expected), "sigma{at:?}");
            }
            println!("legacy brick r: err BUNIT {:?}, err origin {:?}", err.header().and_then(|h| h.get("BUNIT")), origin.source);
            assert_eq!(err.header().and_then(|h| h.get("BUNIT")), Some("nanomaggy"));
            assert_eq!(flags.source, "MBIT_ cards (Legacy maskbits)");
            assert_eq!(comps.saturated_mask(), 8);
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-image-r.fits.fz with its invvar and maskbits files"]
    fn real_legacy_mask_equals_the_catalogue_maskbits_at_the_69_sources() {
        const FLAGGED: [(usize, usize, i64); 20] = [
            (140, 3445, 1024), (295, 3461, 1024), (155, 3409, 1024), (893, 2972, 51352), (391, 2527, 18584),
            (1103, 3021, 51420), (1456, 3169, 16), (1149, 2927, 16), (2312, 2948, 51228), (2569, 3224, 1040),
            (2514, 3173, 1024), (2817, 3246, 4096), (2825, 3226, 4096), (2096, 1740, 16), (2864, 2638, 16400),
            (2966, 2508, 16408), (3202, 2090, 16408), (1754, 1277, 16400), (2041, 1403, 16400), (2725, 1564, 256),
        ];
        const CLEAR: [(usize, usize); 49] = [
            (714, 3442), (1984, 3474), (3514, 3505), (389, 3240), (368, 2898), (625, 3334), (1073, 3399), (987, 3329),
            (859, 3081), (884, 3082), (1238, 3378), (1284, 3288), (1342, 3367), (1507, 3224), (995, 2939), (1376, 2958),
            (1497, 3041), (1778, 3230), (1860, 3277), (1748, 3127), (1376, 1505), (2050, 3147), (2301, 3137), (2616, 2849),
            (2637, 2644), (3027, 3361), (3085, 3366), (2916, 3028), (2818, 2736), (3022, 2743), (2868, 2387), (3093, 2467),
            (3098, 2420), (3012, 2289), (2944, 2055), (2994, 2085), (3056, 1829), (3179, 1758), (1527, 1376), (1488, 1358),
            (2072, 873), (1724, 254), (2696, 1359), (2747, 1347), (2677, 1054), (2192, 784), (1992, 133), (2669, 464),
            (2712, 195),
        ];
        let cache = ImageCache::new(8, usize::MAX);
        let (_, comps) = open_with_companions(&cache, &ImageRef::auto(&legacy_brick_file("image-r")));
        let (dq, _) = comps.dq.as_ref().expect("the maskbits sibling is the mask layer");
        let plane = dq.int_plane().expect("the maskbits sibling is read as integers");
        let rows = FLAGGED.iter().copied().chain(CLEAR.iter().map(|&(y, x)| (y, x, 0)));
        let mut equal = 0;
        for (y, x, expected) in rows {
            let value = plane.value_at(y, x);
            if value == expected {
                equal += 1;
            } else {
                println!("legacy brick mask: value[{y}, {x}] = {value}, catalogue {expected}");
            }
        }
        println!("legacy brick mask: {equal} of 69 rows equal the catalogue maskbits");
        assert_eq!(equal, 69);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_r_stack_480px.fits, survey/D1/des-dr2_A_r_extn1.fits"]
    fn real_ps1_and_des_cutouts_have_no_name_rule() {
        for (set, name) in [("P1", "ps1_A_r_stack_480px.fits"), ("D1", "des-dr2_A_r_extn1.fits")] {
            let loaded = load_plane(&ImageRef::auto(&survey_file(set, name))).unwrap();
            println!("{name}: companions {:?}, notes {:?}", loaded.companions, loaded.info.pairing.notes);
            assert_eq!(loaded.companions, Companions::default(), "{name}");
            assert!(loaded.info.pairing.notes.is_empty(), "{name}");
        }
    }

    #[test]
    fn load_plane_header_uses_requested_hdu() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let h = load_plane_header(&ImageRef::hdu(&p, 3)).unwrap();
        assert_eq!(h.get("EXTNAME"), Some("DQ"));
        assert_eq!(h.get("EXTEND"), Some("T"));
        let h = load_plane_header(&ImageRef::auto(&p)).unwrap();
        assert_eq!(h.get("EXTNAME"), Some("SCI"));
        assert!(load_plane_header(&ImageRef::array(&p, "x")).is_err());
    }

    #[test]
    fn asdf_companions_and_planes() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_asdf(&dir, "a.asdf", ASDF_TREE);
        let c = companions_of(&ImageRef::auto(&p));
        assert_eq!(c.dq, Some(ImageRef::array(&p, "dq")));
        assert_eq!(c.err, Some(ImageRef::array(&p, "err")));
        let c = companions_of(&ImageRef::array(&p, "dq"));
        assert_eq!(c.dq, Some(ImageRef::array(&p, "dq")));
        assert_eq!(c.err, Some(ImageRef::array(&p, "err")));

        let loaded = load_plane(&ImageRef::array(&p, "dq")).unwrap();
        assert!(loaded.info.is_dq);
        assert_eq!(loaded.info.kind, PlaneSelector::Array("dq".into()));
        assert_eq!(loaded.int_plane.as_ref().unwrap().bits[[0, 1]], 1);

        let auto = load_plane(&ImageRef::auto(&p)).unwrap();
        assert_eq!(auto.info.kind, PlaneSelector::Array("data".into()));
        assert!(auto.int_plane.is_none());
        assert_eq!(load_plane_header(&ImageRef::array(&p, "err")).unwrap().get("EXTNAME"), Some("err"));
        assert!(load_plane(&ImageRef::hdu(&p, 1)).is_err());

        let r = write_asdf(&dir, "r.asdf", ROMAN_TREE);
        let c = companions_of(&ImageRef::auto(&r));
        assert_eq!(c.dq, Some(ImageRef::array(&r, "roman.dq")));
        assert_eq!(c.err, None);
        let c = companions_of(&ImageRef::array(&r, "roman.data"));
        assert_eq!(c.dq, Some(ImageRef::array(&r, "roman.dq")));
    }

    #[test]
    fn list_planes_and_plane_ref_for_both_formats() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let (exts, is_asdf) = list_planes(&format!("{}#hdu=2", p)).unwrap();
        assert!(!is_asdf);
        assert_eq!(exts.len(), 5);
        assert_eq!(plane_ref(&p, &exts[3], false), ImageRef::hdu(&p, 3));

        let a = write_asdf(&dir, "b.asdf", ASDF_TREE);
        let (exts, is_asdf) = list_planes(&a).unwrap();
        assert!(is_asdf);
        assert_eq!(exts.len(), 3);
        assert_eq!(plane_ref(&a, &exts[1], true), ImageRef::array(&a, "dq"));
    }

    #[test]
    fn resolve_plane_info_is_header_only_and_matches_load_plane() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let auto = resolve_plane_info(&ImageRef::auto(&p)).unwrap();
        assert_eq!(auto.kind, PlaneSelector::Hdu(1));
        assert_eq!(auto.extname.as_deref(), Some("SCI"));
        assert_eq!(auto.extver, Some(1));
        let dq = resolve_plane_info(&ImageRef::hdu(&p, 3)).unwrap();
        assert!(dq.is_dq);
        assert!(resolve_plane_info(&ImageRef::hdu(&p, 8)).is_err());

        let a = write_asdf(&dir, "c.asdf", ROMAN_TREE);
        let info = resolve_plane_info(&ImageRef::auto(&a)).unwrap();
        assert_eq!(info.kind, PlaneSelector::Array("roman.data".into()));
        assert_eq!(info.extname.as_deref(), Some("roman.data"));
        let info = resolve_plane_info(&ImageRef::array(&a, "roman.dq")).unwrap();
        assert!(info.is_dq);
        assert!(resolve_plane_info(&ImageRef::hdu(&a, 0)).is_err());
    }

    #[test]
    fn dq_and_err_name_rules() {
        assert!(is_dq_name("DQ"));
        assert!(is_dq_name("dq"));
        assert!(is_dq_name("MASK"));
        assert!(is_dq_name("roman.dq"));
        assert!(is_dq_name(" DQ "));
        assert!(!is_dq_name("SCI"));
        assert!(!is_dq_name("dqx"));
        assert!(is_err_name("ERR"));
        assert!(is_err_name("err"));
        assert!(is_err_name("roman.err"));
        assert!(!is_err_name("ERROR"));
        assert!(!is_err_name(""));
    }

    #[test]
    fn calibration_reference_asdf_bails() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_asdf(&dir, "jwst_nircam_flat_0001.asdf", ASDF_TREE);
        let err = load_plane(&ImageRef::auto(&p)).err().expect("calibration reference must be rejected");
        assert!(err.to_string().contains("Calibration reference file"), "{err}");
        assert!(load_plane_header(&ImageRef::auto(&p)).is_err());
        assert!(is_calib_ref_asdf(Path::new("jwst_miri_photom_0042.asdf")));
        assert!(!is_calib_ref_asdf(Path::new("jw01234_cal.asdf")));
    }

    const RAMP_TREE: &str = "roman:\n  data: !core/ndarray-1.0.0\n    data: [[[1, 2], [3, 4]], [[5, 6], [7, 8]], [[9, 10], [11, 12]]]\n    datatype: float32\n";

    #[test]
    fn a_multi_plane_asdf_refusal_is_never_read_as_missing_data() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_asdf(&dir, "jw_cal.asdf", RAMP_TREE);
        mef_path(&dir);

        let refusal = extract_plane_from_asdf(Path::new(&p), None)
            .err()
            .expect("a ramp must not load as a 2D image");
        assert!(
            !missing_asdf_data(&refusal),
            "the multi-plane refusal must stay clear of the missing-data phrase that triggers the companion fallback: {refusal:#}"
        );

        let err = load_plane(&ImageRef::auto(&p))
            .err()
            .expect("the refusal must surface instead of opening the companion .fits");
        assert!(format!("{err:#}").contains("not a single 2D image"), "{err:#}");
        let header_err = load_plane_header(&ImageRef::auto(&p))
            .err()
            .expect("the header path must refuse the ramp too");
        assert!(format!("{header_err:#}").contains("not a single 2D image"), "{header_err:#}");
    }

    #[test]
    fn an_explicit_array_ref_recovers_the_first_plane_of_a_multi_plane_ramp() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_asdf(&dir, "jw_ramp.asdf", RAMP_TREE);

        let loaded = load_plane(&ImageRef::array(&p, "roman.data"))
            .expect("an explicit array ref is the recovery the queue retries with");
        assert_eq!(loaded.arr.dim(), (2, 2));
        assert_eq!(loaded.arr[[0, 0]], 1.0);
        assert_eq!(loaded.arr[[1, 1]], 4.0);
        assert_eq!(loaded.info.kind, PlaneSelector::Array("roman.data".into()));

        load_plane_header(&ImageRef::array(&p, "roman.data"))
            .expect("the header path must accept the same explicit ref");
    }

    #[test]
    fn asdf_without_arrays_falls_back_to_companion_fits() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_asdf(&dir, "jw_cal.asdf", "meta:\n  telescope: JWST\n");
        mef_path(&dir);
        let loaded = load_plane(&ImageRef::auto(&p)).unwrap();
        assert_eq!(loaded.info.kind, PlaneSelector::Hdu(1));
        assert_eq!(loaded.companions, Companions::default());
        assert_eq!(load_plane_header(&ImageRef::auto(&p)).unwrap().get("EXTNAME"), Some("SCI"));
        let lonely = write_asdf(&dir, "lonely.asdf", "meta:\n  telescope: JWST\n");
        assert!(load_plane(&ImageRef::auto(&lonely)).is_err());
    }

    #[test]
    fn the_assignment_key_is_the_same_for_the_automatic_plane_and_its_hdu() {
        assert_eq!(
            assignment_key("C:/d//a.fits", &PlaneSelector::Hdu(1)),
            assignment_key("C:\\d\\a.fits", &PlaneSelector::Hdu(1))
        );
        assert_ne!(
            assignment_key("C:/d/a.fits", &PlaneSelector::Hdu(1)),
            assignment_key("C:/d/a.fits", &PlaneSelector::Hdu(2))
        );
        let dir = tempfile::tempdir().unwrap();
        let key = assignment_key(&in_folder(dir.path(), "a.fits"), &PlaneSelector::Hdu(1));
        assert_eq!(assignment(&key), None);
        edit_assignment(&key, |entry| entry.exclusion = Some(5));
        assert_eq!(assignment(&key), Some(CompanionAssignment { exclusion: Some(5), ..CompanionAssignment::default() }));
        edit_assignment(&key, |entry| entry.dq = Some(DqAssignment::Off));
        assert_eq!(
            assignment(&key),
            Some(CompanionAssignment { dq: Some(DqAssignment::Off), exclusion: Some(5), ..CompanionAssignment::default() })
        );
        edit_assignment(&key, |entry| {
            entry.dq = None;
            entry.exclusion = None;
        });
        assert_eq!(assignment(&key), None, "an entry left empty is removed");
    }

    #[test]
    fn a_user_layer_wins_over_the_one_found_in_the_file_and_off_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let var = in_folder(dir.path(), "var.fits");
        write_fits_mono(&var, &Array2::from_elem((4, 4), 16.0), None).unwrap();
        let image = ImageRef::hdu(&p, 1);
        let key = assignment_key(&p, &PlaneSelector::Hdu(1));
        edit_assignment(&key, |entry| {
            entry.err = Some(ErrAssignment::Layer {
                source: ImageRef::auto(&var),
                role: LayerRole::Variance,
                basis: "user assignment".to_string(),
                assumed: false,
            })
        });
        let loaded = load_plane(&image).unwrap();
        assert_eq!(loaded.companions.err, Some(ImageRef::auto(&var)));
        assert_eq!(loaded.info.pairing.err.as_ref().map(|choice| choice.origin), Some(CompanionOrigin::User));
        assert_eq!(loaded.info.pairing.err_unavailable, None);
        assert_eq!(resolve_plane_info(&image).unwrap().pairing, loaded.info.pairing);
        let (_, comps) = open_with_companions(&ImageCache::new(8, usize::MAX), &image);
        assert_eq!(comps.err.as_ref().expect("the assigned variance is the error layer").arr()[[0, 0]], 4.0);
        assert_eq!(comps.err_origin.map(|origin| origin.source).as_deref(), Some("user assignment, var.fits: square root"));

        edit_assignment(&key, |entry| entry.err = Some(ErrAssignment::Off));
        let off = load_plane(&image).unwrap();
        assert_eq!(off.companions.err, None);
        assert_eq!(off.info.pairing.err, None);
        assert_eq!(off.info.pairing.err_unavailable.as_deref(), Some("error layer switched off by the user"));
        assert_eq!(resolve_plane_info(&image).unwrap().pairing, off.info.pairing);
        let (_, comps) = open_with_companions(&ImageCache::new(8, usize::MAX), &image);
        assert!(comps.err.is_none());
        assert_eq!(comps.err_unavailable.as_deref(), Some("error layer switched off by the user"));

        edit_assignment(&key, |entry| entry.err = None);
        assert_eq!(assignment(&key), None);
        assert_eq!(load_plane(&image).unwrap().companions.err, Some(ImageRef::hdu(&p, 2)));
    }

    #[test]
    fn an_assigned_mask_is_read_as_integers_whatever_its_name() {
        const ABOVE_FLOAT_PRECISION: i32 = 25_296_896;
        let dir = tempfile::tempdir().unwrap();
        let image = in_folder(dir.path(), "image.fits");
        write_fits_mono(&image, &Array2::from_elem((4, 4), 1.0), None).unwrap();
        let bits = in_folder(dir.path(), "bits.fits");
        let mut values = vec![0i32; 16];
        values[6] = ABOVE_FLOAT_PRECISION;
        write_int_primary(&bits, 4, 4, &values);
        assert!(load_plane(&ImageRef::auto(&bits)).unwrap().int_plane.is_none(), "an unnamed plane is read as floats");

        edit_assignment(&assignment_key(&image, &PlaneSelector::Hdu(0)), |entry| {
            entry.dq = Some(DqAssignment::Layer { source: ImageRef::auto(&bits) })
        });
        let cache = ImageCache::new(8, usize::MAX);
        let (active, comps) = open_with_companions(&cache, &ImageRef::auto(&image));
        assert_eq!(active.companions().unwrap().dq, Some(ImageRef::auto(&bits)));
        let pairing = &active.plane_info().unwrap().pairing;
        assert_eq!(pairing.dq.as_ref().map(|choice| (choice.basis.as_str(), choice.origin)), Some(("user assignment", CompanionOrigin::User)));
        assert_eq!(pairing.notes, vec![UNNAMED_FLAGS_NOTE.to_string()]);
        let (dq, _) = comps.dq.as_ref().expect("the assigned file is the mask layer");
        assert_eq!(dq.int_plane().expect("an assigned mask is read as integers").value_at(1, 2), i64::from(ABOVE_FLOAT_PRECISION));
        assert_eq!(comps.flags.as_ref().map(|flags| flags.source.as_str()), Some("unknown (raw bits)"));
        assert_eq!(comps.dq_origin.as_deref(), Some("user assignment, bits.fits"));
        assert!(load_plane(&ImageRef::hdu(&bits, 0)).unwrap().int_plane.is_some());
    }

    #[test]
    fn the_chosen_exclusion_bits_replace_the_default_of_the_flag_set() {
        let dir = tempfile::tempdir().unwrap();
        let p = subaru_calexp(&dir);
        let image = ImageRef::hdu(&p, 1);
        let key = assignment_key(&p, &PlaneSelector::Hdu(1));
        let excluded = || open_with_companions(&ImageCache::new(8, usize::MAX), &image).1.exclusion_mask();
        edit_assignment(&key, |entry| entry.exclusion = Some(2 | 32));
        assert_eq!(excluded(), 2, "DETECTED (bit 5) is never excluded");
        edit_assignment(&key, |entry| entry.exclusion = Some(0));
        assert_eq!(excluded(), 0);
        edit_assignment(&key, |entry| entry.exclusion = None);
        assert_eq!(excluded(), 3);
    }

    fn recipe_before_the_noise_values() -> Option<ErrRecipe> {
        Some(ErrRecipe::SdssFrame { gain: None, dark_variance: None })
    }

    fn frame_key(frame: &str) -> String {
        assignment_key(frame, &PlaneSelector::Hdu(0))
    }

    fn type_noise_values(frame: &str, gain: f64, dark_variance: f64) {
        edit_assignment(&frame_key(frame), |entry| entry.err = Some(ErrAssignment::SdssFrame { gain, dark_variance }));
    }

    #[test]
    fn an_sdss_frame_says_that_its_error_can_be_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        let frame = write_frame(dir.path(), "frame.fits", &FRAME_XINTERP);
        let loaded = load_plane(&ImageRef::auto(&frame)).unwrap();
        assert_eq!(loaded.info.kind, PlaneSelector::Hdu(0));
        assert_eq!(loaded.companions.err, None);
        assert_eq!(loaded.info.pairing.err, None);
        assert_eq!(loaded.info.pairing.err_recipe, recipe_before_the_noise_values());
        assert_eq!(loaded.info.pairing.err_unavailable.as_deref(), Some(SDSS_NO_ERROR_REASON));
        assert_eq!(resolve_plane_info(&ImageRef::auto(&frame)).unwrap().pairing, loaded.info.pairing);

        let mono = in_folder(dir.path(), "mono.fits");
        write_fits_mono(&mono, &frame_image(), None).unwrap();
        let pairing = load_plane(&ImageRef::auto(&mono)).unwrap().info.pairing;
        assert_eq!(pairing.err_recipe, None);
        assert_eq!(pairing.err_unavailable, None);
    }

    #[test]
    fn typed_noise_values_rebuild_the_error_layer() {
        const SOURCE: &str = "SDSS frame recipe (gain 4 e-/DN, dark variance 1 DN^2, typed by the user): rebuilt from the calibration vector and the sky table";
        let dir = tempfile::tempdir().unwrap();
        let frame = write_frame(dir.path(), "frame.fits", &FRAME_XINTERP);
        type_noise_values(&frame, 4.0, 1.0);
        let cache = ImageCache::new(8, usize::MAX);
        let active = cache.get_or_load_plane(&frame, || plane_load(&ImageRef::auto(&frame))).unwrap();
        let rebuilt = sdss_sigma_ref(&frame, 4.0, 1.0);
        let never_the_recipe = |r: &ImageRef| {
            assert_ne!(r, &rebuilt, "the rebuilt error never goes through the plane loader");
            plane_load(r)
        };
        let pairing = &active.plane_info().unwrap().pairing;
        assert_eq!(active.companions().unwrap().err, Some(rebuilt.clone()));
        assert_eq!(
            pairing.err.as_ref().map(|choice| (choice.role, choice.origin)),
            Some((LayerRole::Sigma, CompanionOrigin::Recipe))
        );
        assert_eq!(pairing.err_unavailable, None);
        assert_eq!(pairing.err_recipe, Some(ErrRecipe::SdssFrame { gain: Some(4.0), dark_variance: Some(1.0) }));
        assert_eq!(resolve_plane_info(&ImageRef::auto(&frame)).unwrap().pairing, *pairing);
        for _ in 0..2 {
            let comps = load_companions_into(&cache, &active, never_the_recipe);
            let err = comps.err.as_ref().expect("the rebuilt sigma is the error layer");
            assert_sigma_plane(err.arr(), &SIGMA_GAIN_4_DARK_1);
            let header = err.header().expect("the rebuilt sigma has a header");
            assert_eq!(header.get("BUNIT"), Some("nanomaggy"));
            assert_eq!(header.get_i64(CARD_NO_DATA), Some(1));
            assert_eq!(header.get_f64("ABGAIN"), Some(4.0));
            assert_eq!(header.get_f64("ABDARKV"), Some(1.0));
            assert_eq!(header.cards.len(), 4);
            assert_eq!(
                comps.err_origin,
                Some(ErrOrigin { role: LayerRole::Sigma, source: SOURCE.to_string(), assumed: false, n_no_data: 1 })
            );
            assert_eq!(comps.err_unavailable, None);
        }
    }

    #[test]
    fn new_noise_values_never_reuse_the_old_sigma() {
        let dir = tempfile::tempdir().unwrap();
        let frame = write_frame(dir.path(), "frame.fits", &FRAME_XINTERP);
        type_noise_values(&frame, 4.0, 1.0);
        let cache = ImageCache::new(8, usize::MAX);
        let (_, first) = open_with_companions(&cache, &ImageRef::auto(&frame));
        assert_sigma_plane(first.err.as_ref().expect("the first pair of noise values").arr(), &SIGMA_GAIN_4_DARK_1);

        type_noise_values(&frame, 2.0, 0.0);
        let (_, second) = open_with_companions(&cache, &ImageRef::hdu(&frame, 0));
        let err = second.err.as_ref().expect("the second pair of noise values");
        assert_sigma_plane(err.arr(), &SIGMA_GAIN_2_DARK_0);
        assert_eq!(err.arr()[[1, 1]], 0.008);
        assert_eq!(err.arr()[[0, 0]], 0.0);
        let source = second.err_origin.expect("origin of the rebuilt sigma").source;
        assert!(source.contains("gain 2 e-/DN, dark variance 0 DN^2"), "{source}");
        for (gain, dark_variance) in [(4.0, 1.0), (2.0, 0.0)] {
            assert!(cache.contains(&sigma_key(&sdss_sigma_ref(&frame, gain, dark_variance), LayerRole::Sigma)));
        }
    }

    #[test]
    fn the_recipe_assigned_to_another_file_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mono = in_folder(dir.path(), "mono.fits");
        write_fits_mono(&mono, &frame_image(), None).unwrap();
        type_noise_values(&mono, 4.0, 1.0);
        let loaded = load_plane(&ImageRef::auto(&mono)).unwrap();
        assert_eq!(loaded.companions.err, None);
        assert_eq!(loaded.info.pairing.err, None);
        assert_eq!(loaded.info.pairing.err_recipe, None);
        assert_eq!(
            loaded.info.pairing.err_unavailable.as_deref(),
            Some("the SDSS frame recipe was assigned but the file has no calibration vector and no sky table (an SDSS frame has three HDUs or more, this one has 1)")
        );
        let (_, comps) = open_with_companions(&ImageCache::new(8, usize::MAX), &ImageRef::auto(&mono));
        assert!(comps.err.is_none());
        assert_eq!(comps.err_unavailable, loaded.info.pairing.err_unavailable);
    }

    #[test]
    fn a_file_layer_and_off_still_work_on_a_frame() {
        let dir = tempfile::tempdir().unwrap();
        let frame = write_frame(dir.path(), "frame.fits", &FRAME_XINTERP);
        edit_assignment(&frame_key(&frame), |entry| entry.err = Some(ErrAssignment::Off));
        let off = load_plane(&ImageRef::auto(&frame)).unwrap();
        assert_eq!(off.companions.err, None);
        assert_eq!(off.info.pairing.err_unavailable.as_deref(), Some("error layer switched off by the user"));
        assert_eq!(off.info.pairing.err_recipe, recipe_before_the_noise_values());

        let noise = in_folder(dir.path(), "noise.fits");
        write_fits_mono(&noise, &Array2::from_elem((3, 4), 0.25), None).unwrap();
        edit_assignment(&frame_key(&frame), |entry| {
            entry.err = Some(ErrAssignment::Layer {
                source: ImageRef::auto(&noise),
                role: LayerRole::Sigma,
                basis: USER_ASSIGNMENT_BASIS.to_string(),
                assumed: false,
            })
        });
        let (active, comps) = open_with_companions(&ImageCache::new(8, usize::MAX), &ImageRef::auto(&frame));
        let pairing = &active.plane_info().unwrap().pairing;
        assert_eq!(pairing.err.as_ref().map(|choice| choice.origin), Some(CompanionOrigin::User));
        assert_eq!(pairing.err_unavailable, None);
        assert_eq!(pairing.err_recipe, recipe_before_the_noise_values());
        let err = comps.err.as_ref().expect("the assigned file is the error layer");
        assert!(err.arr().iter().all(|sigma| *sigma == 0.25));
    }

    #[test]
    fn a_layer_left_out_by_choice_or_for_lack_of_inputs_is_no_fault_of_the_file() {
        for reason in [ERROR_SWITCHED_OFF, MASK_SWITCHED_OFF, SDSS_NO_ERROR_REASON] {
            assert!(layer_absent_without_fault(reason), "{reason}");
        }
        for reason in [
            "error layer HDU 2 (ERR) is 8x7 and does not match the image 8x8: not used",
            "mask layer C:/x/a.fits#hdu=3 could not be read: truncated",
            "the SDSS frame error could not be rebuilt: no sky table",
            "",
        ] {
            assert!(!layer_absent_without_fault(reason), "{reason}");
        }
    }

    fn band_ref(file: &str, hdu: usize, plane: usize) -> ImageRef {
        ImageRef::auto(&BandPlaneKey::new(file, hdu, plane).key())
    }

    #[test]
    fn a_band_of_a_cutout_cube_takes_the_same_band_of_the_invvar_cube_and_the_mask_hdu() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_band_cutout(dir.path(), CUTOUT_NAME, CUTOUT_INVVAR);
        let r_band = band_ref(&p, 0, 1);
        let loaded = load_plane(&r_band).unwrap();
        assert_eq!(loaded.companions.err, Some(ImageRef::auto(&format!("{p}#hdu=1&plane=1"))));
        assert_eq!(loaded.companions.dq, Some(ImageRef::hdu(&p, 2)));
        assert_eq!(
            loaded.info.pairing.layer,
            Some(LayerGuess { role: LayerRole::Science, basis: "IMAGETYP IMAGE".into(), assumed: false })
        );
        assert_eq!(
            loaded.info.pairing.err,
            Some(CompanionChoice {
                role: LayerRole::InverseVariance,
                basis: "IMAGETYP INVVAR".into(),
                assumed: false,
                origin: CompanionOrigin::SameFile,
            })
        );

        let cache = ImageCache::new(8, usize::MAX);
        let (_, comps) = open_with_companions(&cache, &r_band);
        let err = comps.err.as_ref().expect("the r plane of the INVVAR cube is the error layer");
        assert_eq!(err.arr().dim(), (3, 4));
        for ((y, x), sigma) in err.arr().indexed_iter() {
            if (y, x) == (1, 1) {
                assert!(sigma.is_nan(), "a zero inverse variance is no data");
            } else {
                assert_eq!(*sigma, 0.25, "[{y}, {x}]");
            }
        }
        assert_eq!(
            comps.err_origin,
            Some(ErrOrigin {
                role: LayerRole::InverseVariance,
                source: "IMAGETYP INVVAR, HDU 1 plane 1: 1 / square root".into(),
                assumed: false,
                n_no_data: 1,
            })
        );
        assert_eq!(comps.dq_origin.as_deref(), Some("IMAGETYP MASKBITS, HDU 2"));
        let (mask, _) = comps.dq.as_ref().expect("the MASKBITS HDU is the mask");
        let bits = mask.int_plane().expect("the mask is read as integers");
        assert_eq!((bits.value_at(0, 0), bits.value_at(1, 3)), (1024, 8));
        assert_eq!(comps.flags.as_ref().map(|flags| flags.source.as_str()), Some("Legacy DR10 maskbits (built-in table)"));
        assert_eq!(comps.exclusion_mask(), 72);
        assert_eq!(comps.saturated_mask(), 8);
    }

    #[test]
    fn each_band_gets_its_own_plane_of_the_error_cube() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_band_cutout(dir.path(), CUTOUT_NAME, CUTOUT_INVVAR);
        let cache = ImageCache::new(8, usize::MAX);
        let (_, g) = open_with_companions(&cache, &band_ref(&p, 0, 0));
        let err = g.err.as_ref().expect("the g plane of the INVVAR cube is the error layer");
        assert!(err.arr().iter().all(|sigma| *sigma == 0.5), "{:?}", err.arr());
        assert_eq!(
            g.err_origin,
            Some(ErrOrigin {
                role: LayerRole::InverseVariance,
                source: "IMAGETYP INVVAR, HDU 1 plane 0: 1 / square root".into(),
                assumed: false,
                n_no_data: 0,
            })
        );
        assert_eq!(g.exclusion_mask(), 36);

        let (_, r) = open_with_companions(&cache, &band_ref(&p, 0, 1));
        assert_eq!(r.err_origin.map(|origin| origin.n_no_data), Some(1));
        for plane in [0, 1] {
            let derived = sigma_key(&band_ref(&p, 1, plane), LayerRole::InverseVariance);
            assert!(cache.contains(&derived), "{derived}");
        }
        cache.invalidate_written(&p);
        assert_eq!(cache.len(), 0, "a rewrite of the cutout drops the derived sigma of its bands");
    }

    #[test]
    fn a_band_of_an_error_cube_shown_on_its_own_gets_no_companion() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_band_cutout(dir.path(), CUTOUT_NAME, CUTOUT_INVVAR);
        let loaded = load_plane(&band_ref(&p, 1, 0)).unwrap();
        assert_eq!(loaded.companions, Companions::default());
        assert_eq!(loaded.info.pairing.layer.map(|layer| layer.role), Some(LayerRole::InverseVariance));
        assert_eq!(loaded.info.pairing.err_unavailable, None);
    }

    #[test]
    fn an_error_cube_with_another_number_of_planes_or_another_size_is_not_paired() {
        let dir = tempfile::tempdir().unwrap();
        let three_planes = write_band_cutout(dir.path(), "three_planes.fits", (4, 3, 3));
        let loaded = load_plane(&band_ref(&three_planes, 0, 1)).unwrap();
        assert_eq!(loaded.companions.err, None);
        assert_eq!(loaded.info.pairing.err_unavailable, None);
        assert_eq!(loaded.companions.dq, Some(ImageRef::hdu(&three_planes, 2)));

        let small = write_band_cutout(dir.path(), "small_planes.fits", (2, 2, 2));
        let loaded = load_plane(&band_ref(&small, 0, 1)).unwrap();
        assert_eq!(loaded.companions.err, None);
        assert_eq!(
            loaded.info.pairing.err_unavailable.as_deref(),
            Some("error layer HDU 1 (IMAGETYP INVVAR) is 2x2 and does not match the image 4x3: not used")
        );
    }

    #[test]
    fn resolve_plane_info_and_load_plane_agree_for_a_band_plane() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_band_cutout(dir.path(), CUTOUT_NAME, CUTOUT_INVVAR);
        for band in [band_ref(&p, 0, 1), band_ref(&p, 1, 0)] {
            let resolved = resolve_plane_info(&band).unwrap();
            assert_eq!(resolved.pairing, load_plane(&band).unwrap().info.pairing, "{}", band.path);
        }
        let pairing = resolve_plane_info(&band_ref(&p, 0, 1)).unwrap().pairing;
        assert!(pairing.err.is_some() && pairing.dq.is_some(), "{pairing:?}");
    }

    #[test]
    fn a_band_paired_with_a_mask_without_bit_names_says_that_nothing_is_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cube_and_mask.fits");
        let cube_cards: Vec<(&'static str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "3".into()),
            ("NAXIS1", "4".into()),
            ("NAXIS2", "3".into()),
            ("NAXIS3", "2".into()),
            ("EXTEND", "T".into()),
        ];
        let cube: Vec<u8> = (0..24).flat_map(|v| (v as f32).to_be_bytes()).collect();
        let (mask_cards, mask) = int_extension(32, &[("EXTNAME", "'MASK    '")], &[0; 12]);
        write_raw_hdus(&path, &[(cube_cards, cube), (mask_cards, mask)]);
        let p = path.to_str().unwrap().to_string();
        let loaded = load_plane(&band_ref(&p, 0, 1)).unwrap();
        assert_eq!(loaded.companions.dq, Some(ImageRef::hdu(&p, 1)));
        assert_eq!(loaded.info.pairing.notes, vec![UNNAMED_FLAGS_NOTE.to_string()]);
    }

    #[test]
    fn the_cube_itself_and_ordinary_files_are_paired_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_band_cutout(dir.path(), CUTOUT_NAME, CUTOUT_INVVAR);
        let loaded = load_plane(&ImageRef::hdu(&p, 0)).unwrap();
        assert_eq!(loaded.companions.err, None, "a cube companion is still skipped");
        assert_eq!(loaded.info.pairing.err_unavailable, None);
        assert_eq!(loaded.companions.dq, Some(ImageRef::hdu(&p, 2)));
        assert_eq!(
            loaded.info.pairing.dq,
            Some(CompanionChoice {
                role: LayerRole::Mask,
                basis: "IMAGETYP MASKBITS".into(),
                assumed: false,
                origin: CompanionOrigin::SameFile,
            })
        );
    }
}
