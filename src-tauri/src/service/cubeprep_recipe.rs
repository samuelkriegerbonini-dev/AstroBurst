use std::path::Path;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::core::cubeprep::align::{AlignOptions, ReferencePolicy, ResampleMethod};
use crate::core::cubeprep::cube::{CubeEdit, CubeLayout, CubeOptions, PlaneOrder};
use crate::core::cubeprep::footprint::{CropRegion, CropRounding};
use crate::core::cubeprep::kernel::{KernelMethod, KernelOptions, KernelShape};
use crate::core::cubeprep::match_cube::{MatchError, MatchMode, MatchOptions};
use crate::core::cubeprep::psf_io::{PsfResampleMethod, PsfSetOptions};
use crate::core::imaging::cutout::CutoutRect;
use crate::core::imaging::psf_estimation::EmpiricalPsfOptions;
use crate::core::metadata::units::{AreaRule, TargetUnit};
use crate::infra::progress::ProgressHandle;
use crate::service::cubeprep::{
    align_set, build_cube, build_kernels, convert_units_file, crop_set, edit_cube, filter_registry, load_psfs, psf_match_cube,
    CubeInputSpec, PsfInputSpec,
};
use crate::service::cutout::export_cutout_rect;
use crate::service::empirical_psf::{build_empirical_psf_file, PsfStarSource};
use crate::service::image_files::invalidate_written;
use crate::types::error::{AppError, CodedError};

pub const RECIPE_VERSION: u64 = 1;
pub const RECIPE_FILE: &str = "recipe.json";
pub const RESULT_FILE: &str = "result.json";
pub const STEPS: [&str; 9] = ["units", "crop", "align", "cube", "edit", "psfs", "kernels", "match", "export"];

pub const UNKNOWN_STEP: &str = "unknown_step";
pub const UNKNOWN_PARAMETER: &str = "unknown_parameter";
pub const RECIPE_INVALID: &str = "recipe_invalid";
pub const RECIPE_VERSION_UNSUPPORTED: &str = "recipe_version_unsupported";
pub const FILTER_DUPLICATE: &str = "filter_duplicate";
pub const FILE_NOT_FOUND: &str = "file_not_found";

const DEFAULT_CUBE_OUTPUT: &str = "cube.fits";
const DEFAULT_BORDER_CROP: usize = 1;
const KERNEL_MISSING_HINT: &str =
    "give a PSF for every filter of the cube in the psfs step, or set allow_unmatched to true in the match step to copy those planes unmatched";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub recipe_version: u64,
    pub name: Option<String>,
    pub inputs: Vec<RecipeInput>,
    pub steps: Vec<RecipeStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeInput {
    pub path: String,
    pub dataset: String,
    pub filter: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum RecipeStep {
    Units { to: RecipeUnit },
    Crop(CropStep),
    Align(AlignStep),
    Cube(CubeStep),
    Edit(CubeEdit),
    Psfs(PsfsStep),
    Kernels(KernelsStep),
    Match(MatchStep),
    Export { output: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecipeUnit {
    #[serde(rename = "Jy/pixel")]
    JyPerPixel,
    #[serde(rename = "MJy/sr")]
    MjyPerSr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CropStep {
    #[serde(flatten)]
    pub region: CropMode,
    #[serde(default = "lightstack_rounding")]
    pub rounding: CropRounding,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum CropMode {
    Radec { ra: f64, dec: f64, size_arcsec: f64 },
    Reference { reference: String },
    Common,
    Region {
        region_file: String,
        #[serde(default)]
        index: usize,
    },
    RegionCommon { region_file: String },
    Pixel { x_start: usize, x_end: usize, y_start: usize, y_end: usize },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignStep {
    pub reference: String,
    #[serde(default = "bilinear")]
    pub method: ResampleMethod,
    #[serde(default = "default_border_crop")]
    pub border_crop: usize,
    #[serde(default = "lightstack_area_rule")]
    pub area_rule: AreaRule,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CubeStep {
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub order: PlaneOrder,
    #[serde(default = "default_cube_output")]
    pub output: String,
    #[serde(default)]
    pub layout: CubeLayout,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PsfsStep {
    #[serde(flatten)]
    pub source: PsfSource,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub pixel_scale_arcsec: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum PsfSource {
    Files { files: Vec<PsfFile> },
    Empirical(EmpiricalPsfs),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PsfFile {
    pub filter: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmpiricalPsfs {
    pub dataset: String,
    pub filters: Vec<String>,
    #[serde(default)]
    pub data_dir: Option<String>,
    #[serde(default)]
    pub oversampling_order: Option<u32>,
    #[serde(flatten)]
    pub options: EmpiricalPsfOptions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelsStep {
    pub reference: String,
    #[serde(flatten)]
    pub options: KernelOptions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchStep {
    pub reference: String,
    #[serde(default = "lightstack_match")]
    pub mode: MatchMode,
    #[serde(default)]
    pub allow_unmatched: bool,
}

fn lightstack_rounding() -> CropRounding {
    CropRounding::Lightstack
}

fn bilinear() -> ResampleMethod {
    ResampleMethod::Bilinear
}

fn default_border_crop() -> usize {
    DEFAULT_BORDER_CROP
}

fn lightstack_area_rule() -> AreaRule {
    AreaRule::Lightstack
}

fn default_cube_output() -> String {
    DEFAULT_CUBE_OUTPUT.to_string()
}

fn lightstack_match() -> MatchMode {
    MatchMode::Lightstack
}

#[derive(Debug, Clone, Serialize)]
pub struct RecipeReport {
    pub recipe_version: u64,
    pub name: Option<String>,
    pub output_dir: String,
    pub cube: Option<String>,
    pub steps: Vec<StepReport>,
    pub files: Vec<RecipeFile>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    pub index: usize,
    pub step: &'static str,
    pub directory: String,
    pub parameters: Value,
    pub report: Value,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecipeFile {
    pub path: String,
    pub bytes: u64,
}

impl RecipeStep {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Units { .. } => "units",
            Self::Crop(_) => "crop",
            Self::Align(_) => "align",
            Self::Cube(_) => "cube",
            Self::Edit(_) => "edit",
            Self::Psfs(_) => "psfs",
            Self::Kernels(_) => "kernels",
            Self::Match(_) => "match",
            Self::Export { .. } => "export",
        }
    }
}

impl RecipeUnit {
    fn target(self) -> TargetUnit {
        match self {
            Self::JyPerPixel => TargetUnit::JyPerPixel,
            Self::MjyPerSr => TargetUnit::MjyPerSr,
        }
    }
}

fn coded(code: &'static str, message: impl Into<String>, hint: Option<String>) -> CodedError {
    CodedError { code, message: message.into(), hint }
}

fn invalid(message: impl Into<String>) -> CodedError {
    coded(RECIPE_INVALID, message, None)
}

fn placed(place: &str, error: CodedError) -> CodedError {
    coded(error.code, format!("{place}: {}", error.message), error.hint)
}

struct UnknownKey {
    path: String,
    accepted: Vec<String>,
}

fn unknown_keys(given: &Value, canonical: &Value, path: &str, found: &mut Vec<UnknownKey>) {
    match (given, canonical) {
        (Value::Object(given), Value::Object(canonical)) => {
            let accepted = || {
                let mut keys: Vec<String> = canonical.keys().filter(|key| key.as_str() != "step").cloned().collect();
                keys.sort();
                keys
            };
            for (key, value) in given {
                let here = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                match canonical.get(key) {
                    Some(known) => unknown_keys(value, known, &here, found),
                    None => found.push(UnknownKey { path: here, accepted: accepted() }),
                }
            }
        }
        (Value::Array(given), Value::Array(canonical)) => {
            for (index, (item, known)) in given.iter().zip(canonical).enumerate() {
                unknown_keys(item, known, &format!("{path}[{index}]"), found);
            }
        }
        _ => {}
    }
}

fn refuse_unknown(place: &str, given: &Value, canonical: &Value) -> Result<(), CodedError> {
    let mut found = Vec::new();
    unknown_keys(given, canonical, "", &mut found);
    match found.first() {
        None => Ok(()),
        Some(first) => {
            let names: Vec<&str> = found.iter().map(|key| key.path.as_str()).collect();
            Err(coded(
                UNKNOWN_PARAMETER,
                format!("{place}: unknown {}", names.join(", ")),
                Some(format!("accepted keys: {}", first.accepted.join(", "))),
            ))
        }
    }
}

fn typed<T: DeserializeOwned + Serialize>(place: &str, value: &Value) -> Result<T, CodedError> {
    let parsed: T = serde_json::from_value(value.clone()).map_err(|error| invalid(format!("{place}: {error}")))?;
    let canonical = serde_json::to_value(&parsed).map_err(|error| invalid(format!("{place}: {error}")))?;
    refuse_unknown(place, value, &canonical)?;
    Ok(parsed)
}

fn parse_step(index: usize, value: &Value) -> Result<RecipeStep, CodedError> {
    let place = format!("steps[{index}]");
    let kind = value.get("step").and_then(Value::as_str).ok_or_else(|| invalid(format!("{place}: no step name")))?;
    if !STEPS.contains(&kind) {
        return Err(coded(UNKNOWN_STEP, format!("{place}: unknown step {kind}"), Some(format!("known steps are {}", STEPS.join(", ")))));
    }
    typed(&format!("{place} ({kind})"), value)
}

fn listed<'a>(object: &'a Map<String, Value>, key: &str, empty: &str) -> Result<&'a Vec<Value>, CodedError> {
    match object.get(key) {
        Some(Value::Array(items)) if !items.is_empty() => Ok(items),
        Some(Value::Array(_)) => Err(invalid(empty)),
        Some(_) => Err(invalid(format!("recipe: {key} must be a list"))),
        None => Err(invalid(format!("recipe: missing {key}"))),
    }
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn resolve(base: &Path, path: &mut String) {
    if !Path::new(path.as_str()).is_absolute() {
        *path = base.join(path.as_str()).to_string_lossy().into_owned();
    }
}

fn require_file(place: &str, path: &str) -> Result<(), CodedError> {
    if Path::new(path).is_file() {
        return Ok(());
    }
    Err(coded(FILE_NOT_FOUND, format!("{place}: {path} is not a file"), Some("recipe paths are relative to the data folder unless absolute".into())))
}

fn require_dir(place: &str, path: &str) -> Result<(), CodedError> {
    if Path::new(path).is_dir() {
        return Ok(());
    }
    Err(coded(FILE_NOT_FOUND, format!("{place}: {path} is not a folder"), Some("data_dir is relative to the data folder unless absolute".into())))
}

fn bare_fits_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".fits") && Path::new(name).file_name().is_some_and(|file| file == name) && !name.starts_with('.')
}

fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

#[derive(Default)]
struct Planned<'a> {
    aligned: Option<&'a str>,
    cube: bool,
    psfs: Vec<String>,
    kernels: Option<String>,
}

impl Recipe {
    pub fn from_value(value: &Value) -> Result<Self, CodedError> {
        let object = value.as_object().ok_or_else(|| invalid("recipe: expected a JSON object"))?;
        let known = ["recipe_version", "name", "inputs", "steps"];
        let mut unknown: Vec<&str> = object.keys().map(String::as_str).filter(|key| !known.contains(key)).collect();
        unknown.sort();
        if !unknown.is_empty() {
            return Err(coded(UNKNOWN_PARAMETER, format!("recipe: unknown {}", unknown.join(", ")), Some(format!("accepted keys: {}", known.join(", ")))));
        }
        let version = object.get("recipe_version").ok_or_else(|| invalid("recipe: missing recipe_version"))?;
        if version.as_u64() != Some(RECIPE_VERSION) {
            return Err(coded(
                RECIPE_VERSION_UNSUPPORTED,
                format!("recipe_version {version} is not {RECIPE_VERSION}"),
                Some(format!("this AstroBurst reads recipe_version {RECIPE_VERSION}")),
            ));
        }
        let name = match object.get("name") {
            None | Some(Value::Null) => None,
            Some(Value::String(name)) => Some(name.clone()),
            Some(_) => return Err(invalid("recipe: name must be a string")),
        };
        let inputs = listed(object, "inputs", "the recipe has no input")?
            .iter()
            .enumerate()
            .map(|(index, item)| typed(&format!("inputs[{index}]"), item))
            .collect::<Result<Vec<RecipeInput>, _>>()?;
        let steps = listed(object, "steps", "the recipe has no step")?
            .iter()
            .enumerate()
            .map(|(index, item)| parse_step(index, item))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { recipe_version: RECIPE_VERSION, name, inputs, steps })
    }

    pub fn resolved(mut self, base: &Path) -> Self {
        for input in &mut self.inputs {
            resolve(base, &mut input.path);
        }
        for step in &mut self.steps {
            match step {
                RecipeStep::Crop(CropStep { region: CropMode::Region { region_file, .. } | CropMode::RegionCommon { region_file }, .. }) => {
                    resolve(base, region_file)
                }
                RecipeStep::Psfs(PsfsStep { source: PsfSource::Files { files }, .. }) => {
                    files.iter_mut().for_each(|file| resolve(base, &mut file.path))
                }
                RecipeStep::Psfs(PsfsStep { source: PsfSource::Empirical(empirical), .. }) => {
                    let dir = empirical.data_dir.get_or_insert_with(|| ".".to_string());
                    resolve(base, dir);
                }
                _ => {}
            }
        }
        self
    }

    pub fn paths(&self) -> Vec<&str> {
        let mut paths: Vec<&str> = self.inputs.iter().map(|input| input.path.as_str()).collect();
        for step in &self.steps {
            match step {
                RecipeStep::Crop(CropStep { region: CropMode::Region { region_file, .. } | CropMode::RegionCommon { region_file }, .. }) => {
                    paths.push(region_file)
                }
                RecipeStep::Psfs(PsfsStep { source: PsfSource::Files { files }, .. }) => paths.extend(files.iter().map(|file| file.path.as_str())),
                RecipeStep::Psfs(PsfsStep { source: PsfSource::Empirical(empirical), .. }) => paths.extend(empirical.data_dir.as_deref()),
                _ => {}
            }
        }
        paths
    }

    pub fn empirical_files(&self) -> Result<Vec<String>, CodedError> {
        let mut files = Vec::new();
        for (index, step) in self.steps.iter().enumerate() {
            if let RecipeStep::Psfs(PsfsStep { source: PsfSource::Empirical(empirical), .. }) = step {
                for filter in &empirical.filters {
                    let (image, region) = empirical_inputs(empirical, filter).map_err(|error| placed(&format!("steps[{index}] (psfs)"), error))?;
                    files.extend([image, region]);
                }
            }
        }
        Ok(files)
    }

    pub fn validate(&self) -> Result<(), CodedError> {
        let mut filters: Vec<&str> = Vec::new();
        for (index, input) in self.inputs.iter().enumerate() {
            let place = format!("inputs[{index}]");
            if input.dataset.trim().is_empty() || input.filter.trim().is_empty() {
                return Err(invalid(format!("{place}: dataset and filter must not be empty")));
            }
            if filters.iter().any(|filter| same(filter, &input.filter)) {
                return Err(coded(
                    FILTER_DUPLICATE,
                    format!("{place}: filter {} appears twice", input.filter.trim().to_uppercase()),
                    Some("each input holds one filter; a cube takes one plane per filter".into()),
                ));
            }
            require_file(&place, &input.path)?;
            filters.push(&input.filter);
        }
        let mut planned = Planned::default();
        for (index, step) in self.steps.iter().enumerate() {
            validate_step(&format!("steps[{index}] ({})", step.kind()), step, &filters, &mut planned)?;
        }
        Ok(())
    }
}

fn input_filter<'a>(place: &str, filters: &[&'a str], wanted: &str) -> Result<&'a str, CodedError> {
    filters
        .iter()
        .find(|filter| same(filter, wanted))
        .copied()
        .ok_or_else(|| invalid(format!("{place}: reference {wanted} is not an input filter")))
}

fn check_box(place: &str, x: (i64, i64), y: (i64, i64)) -> Result<(), CodedError> {
    if x.0 < 0 || y.0 < 0 {
        return Err(invalid(format!("{place}: x_start and y_start must not be negative")));
    }
    if x.0 >= x.1 || y.0 >= y.1 {
        return Err(invalid(format!("{place}: empty pixel box")));
    }
    Ok(())
}

fn validate_step<'a>(place: &str, step: &'a RecipeStep, filters: &[&'a str], planned: &mut Planned<'a>) -> Result<(), CodedError> {
    match step {
        RecipeStep::Units { .. } => {}
        RecipeStep::Crop(crop) => match &crop.region {
            CropMode::Radec { ra, dec, size_arcsec } => {
                if !(ra.is_finite() && dec.is_finite()) {
                    return Err(invalid(format!("{place}: ra and dec must be finite numbers")));
                }
                if !positive(*size_arcsec) {
                    return Err(invalid(format!("{place}: size_arcsec must be positive")));
                }
            }
            CropMode::Reference { reference } => {
                input_filter(place, filters, reference)?;
            }
            CropMode::Common => {}
            CropMode::Region { region_file, .. } | CropMode::RegionCommon { region_file } => require_file(place, region_file)?,
            CropMode::Pixel { x_start, x_end, y_start, y_end } => {
                check_box(place, (*x_start as i64, *x_end as i64), (*y_start as i64, *y_end as i64))?
            }
        },
        RecipeStep::Align(align) => planned.aligned = Some(input_filter(place, filters, &align.reference)?),
        RecipeStep::Cube(cube) => {
            match cube.reference.as_deref().or(planned.aligned) {
                Some(reference) => {
                    input_filter(place, filters, reference)?;
                }
                None => return Err(invalid(format!("{place}: name a reference filter or align first"))),
            }
            if !bare_fits_name(&cube.output) {
                return Err(invalid(format!("{place}: output must be a .fits file name")));
            }
            planned.cube = true;
        }
        RecipeStep::Edit(edit) => {
            if !planned.cube {
                return Err(invalid(format!("{place}: build a cube first")));
            }
            match edit {
                CubeEdit::RemoveFilter { filter } => {
                    if !filters.iter().any(|known| same(known, filter)) {
                        return Err(invalid(format!("{place}: {filter} is not an input filter")));
                    }
                }
                CubeEdit::Cut { x_start, x_end, y_start, y_end } => check_box(place, (*x_start, *x_end), (*y_start, *y_end))?,
                CubeEdit::CutSky { ra, dec, size_arcsec, .. } => {
                    if !(ra.is_finite() && dec.is_finite() && positive(*size_arcsec)) {
                        return Err(invalid(format!("{place}: ra and dec must be finite and size_arcsec positive")));
                    }
                }
                CubeEdit::DropInvalid { .. } | CubeEdit::Header { .. } | CubeEdit::ReferenceNanMask { .. } => {}
            }
        }
        RecipeStep::Psfs(psfs) => {
            let names: Vec<String> = match &psfs.source {
                PsfSource::Files { files } => {
                    if files.is_empty() {
                        return Err(invalid(format!("{place}: files must be a non-empty list")));
                    }
                    for (index, file) in files.iter().enumerate() {
                        require_file(&format!("{place} files[{index}]"), &file.path)?;
                    }
                    files.iter().map(|file| file.filter.clone()).collect()
                }
                PsfSource::Empirical(empirical) => {
                    if empirical.filters.is_empty() {
                        return Err(invalid(format!("{place}: filters must be a non-empty list")));
                    }
                    if empirical.oversampling_order.is_some_and(|order| order != 3) {
                        return Err(invalid(format!("{place}: only the cubic oversampling (oversampling_order 3) is ported")));
                    }
                    require_dir(place, empirical.data_dir.as_deref().unwrap_or("."))?;
                    empirical.filters.clone()
                }
            };
            if names.iter().enumerate().any(|(index, name)| names[..index].iter().any(|other| same(other, name))) {
                return Err(invalid(format!("{place}: a filter has two PSFs")));
            }
            let foreign: Vec<&str> = names.iter().map(String::as_str).filter(|name| !filters.iter().any(|known| same(known, name))).collect();
            if !foreign.is_empty() {
                return Err(invalid(format!("{place}: PSFs for filters that are not inputs: {}", foreign.join(", "))));
            }
            if let PsfSource::Empirical(empirical) = &psfs.source {
                for filter in &empirical.filters {
                    empirical_inputs(empirical, filter).map_err(|error| placed(place, error))?;
                }
            }
            if let Some(scale) = psfs.pixel_scale_arcsec {
                if !positive(scale) {
                    return Err(invalid(format!("{place}: pixel_scale_arcsec must be positive")));
                }
                if !psfs.reference.as_deref().is_some_and(|reference| names.iter().any(|name| same(name, reference))) {
                    return Err(invalid(format!("{place}: resampling needs a reference filter among the PSFs")));
                }
            }
            planned.psfs = names;
        }
        RecipeStep::Kernels(kernels) => {
            if let KernelMethod::Wiener { eps, shape, .. } = kernels.options.method {
                if !positive(eps) {
                    return Err(invalid(format!("{place}: eps must be positive")));
                }
                if let KernelShape::Fixed(sides) = shape {
                    if sides.iter().any(|side| side % 2 == 0) {
                        return Err(invalid(format!("{place}: shape must be reference or two odd integers")));
                    }
                }
            }
            if !planned.psfs.iter().any(|name| same(name, &kernels.reference)) {
                return Err(invalid(format!("{place}: no PSF for the reference {}", kernels.reference)));
            }
            planned.kernels = Some(kernels.reference.clone());
        }
        RecipeStep::Match(matching) => {
            if !planned.cube {
                return Err(invalid(format!("{place}: build a cube first")));
            }
            if !planned.kernels.as_deref().is_some_and(|reference| same(reference, &matching.reference)) {
                return Err(invalid(format!("{place}: no kernels to {}", matching.reference)));
            }
        }
        RecipeStep::Export { output } => {
            if !planned.cube {
                return Err(invalid(format!("{place}: build a cube first")));
            }
            if !bare_fits_name(output) {
                return Err(invalid(format!("{place}: output must be a .fits file name")));
            }
        }
    }
    Ok(())
}

struct Image {
    path: String,
    dataset: String,
    filter: String,
}

struct StepDone {
    report: Value,
    warnings: Vec<String>,
}

struct Run {
    images: Vec<Image>,
    aligned: Option<String>,
    cube: Option<String>,
    psfs: Vec<(String, String)>,
    kernel_dir: Option<String>,
}

fn check_cancelled(progress: &ProgressHandle) -> Result<()> {
    if progress.is_cancelled() {
        return Err(AppError::Cancelled.into());
    }
    Ok(())
}

fn labelled(label: &str, warnings: &[String]) -> Vec<String> {
    warnings.iter().map(|warning| format!("{label}: {warning}")).collect()
}

fn empirical_inputs(empirical: &EmpiricalPsfs, filter: &str) -> Result<(String, String), CodedError> {
    let dir = empirical.data_dir.as_deref().unwrap_or(".");
    let unlisted = |error: std::io::Error| coded(FILE_NOT_FOUND, format!("cannot list {dir}: {error}"), None);
    let registry = filter_registry();
    let mut images = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(unlisted)? {
        let path = entry.map_err(unlisted)?.path();
        let text = path.to_string_lossy().into_owned();
        let is_fits = path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("fits")) && path.is_file();
        if is_fits && registry.identify_file_name(&text, &empirical.dataset).is_ok_and(|found| same(&found.entry.filter, filter)) {
            images.push(text);
        }
    }
    images.sort();
    let image = match images.as_slice() {
        [image] => image.clone(),
        [] => {
            return Err(coded(
                FILE_NOT_FOUND,
                format!("no FITS file in {dir} is named for {} {filter}", empirical.dataset),
                Some(format!("data_dir holds one FITS file per filter whose name carries the filter, such as image_{}_{filter}.fits", empirical.dataset)),
            ))
        }
        several => return Err(invalid(format!("{} FITS files in {dir} are named for {filter} ({}); keep one", several.len(), several.join(", ")))),
    };
    let region = Path::new(dir).join(format!("stars_{}.reg", filter.trim().to_lowercase()));
    if !region.is_file() {
        return Err(coded(
            FILE_NOT_FOUND,
            format!("{} is missing: the empirical PSF of {filter} reads its stars from it", region.display()),
            Some("write the stars as DS9 boxes in that file, beside the image".into()),
        ));
    }
    Ok((image, region.to_string_lossy().into_owned()))
}

impl Run {
    fn new(recipe: &Recipe) -> Self {
        Self {
            images: recipe
                .inputs
                .iter()
                .map(|input| Image { path: input.path.clone(), dataset: input.dataset.clone(), filter: input.filter.trim().to_string() })
                .collect(),
            aligned: None,
            cube: None,
            psfs: Vec::new(),
            kernel_dir: None,
        }
    }

    fn filter(&self, wanted: &str) -> Result<&Image> {
        self.images.iter().find(|image| same(&image.filter, wanted)).with_context(|| format!("no input holds the filter {wanted}"))
    }

    fn paths(&self) -> Vec<String> {
        self.images.iter().map(|image| image.path.clone()).collect()
    }

    fn cube(&self) -> Result<String> {
        self.cube.clone().context("no cube was built before this step")
    }

    fn step(&mut self, step: &RecipeStep, dir: &str, progress: &ProgressHandle) -> Result<StepDone> {
        match step {
            RecipeStep::Units { to } => self.units(*to, dir, progress),
            RecipeStep::Crop(crop) => self.crop(crop, dir, progress),
            RecipeStep::Align(align) => self.align(align, dir),
            RecipeStep::Cube(cube) => self.build(cube, dir),
            RecipeStep::Edit(edit) => self.edit(edit, dir),
            RecipeStep::Psfs(psfs) => self.psfs(psfs, dir, progress),
            RecipeStep::Kernels(kernels) => self.kernels(kernels, dir),
            RecipeStep::Match(matching) => self.matching(matching, dir),
            RecipeStep::Export { output } => self.export(output, dir),
        }
    }

    fn units(&mut self, to: RecipeUnit, dir: &str, progress: &ProgressHandle) -> Result<StepDone> {
        let mut files = Vec::with_capacity(self.images.len());
        let mut warnings = Vec::new();
        for image in &mut self.images {
            check_cancelled(progress)?;
            let converted = convert_units_file(&image.path, to.target(), "", Some(dir)).with_context(|| image.filter.clone())?;
            warnings.extend(labelled(&image.filter, &converted.warnings));
            image.path = converted.output_path.clone();
            files.push(converted);
        }
        Ok(StepDone { report: json!({ "files": files }), warnings })
    }

    fn crop(&mut self, crop: &CropStep, dir: &str, progress: &ProgressHandle) -> Result<StepDone> {
        let region = match &crop.region {
            CropMode::Pixel { x_start, x_end, y_start, y_end } => {
                let rect = CutoutRect { x0: *x_start as i64, y0: *y_start as i64, width: x_end - x_start, height: y_end - y_start };
                return self.pixel_crop(rect, dir, progress);
            }
            CropMode::Radec { ra, dec, size_arcsec } => CropRegion::Radec { ra: *ra, dec: *dec, size_arcsec: *size_arcsec },
            CropMode::Reference { reference } => CropRegion::Reference { reference: self.filter(reference)?.path.clone() },
            CropMode::Common => CropRegion::Common,
            CropMode::Region { region_file, index } => CropRegion::Region { region_file: region_file.clone(), index: *index },
            CropMode::RegionCommon { region_file } => CropRegion::RegionCommon { region_file: region_file.clone() },
        };
        let report = crop_set(&self.paths(), &region, crop.rounding, dir, true, true)?;
        for (image, file) in self.images.iter_mut().zip(&report.files) {
            image.path = file.cutout.output_path.clone();
        }
        Ok(StepDone { warnings: report.warnings.clone(), report: serde_json::to_value(&report)? })
    }

    fn pixel_crop(&mut self, rect: CutoutRect, dir: &str, progress: &ProgressHandle) -> Result<StepDone> {
        let mut files = Vec::with_capacity(self.images.len());
        let mut warnings = Vec::new();
        for image in &mut self.images {
            check_cancelled(progress)?;
            let cutout = export_cutout_rect(&image.path, "", Some(dir), rect, true, true).with_context(|| image.filter.clone())?;
            if cutout.fraction_on_image < 1.0 {
                warnings.push(format!("{}: the pixel box extends beyond the image; the outside is NaN and flagged in DQ", image.filter));
            }
            warnings.extend(labelled(&image.filter, &cutout.warnings));
            image.path = cutout.output_path.clone();
            files.push(cutout);
        }
        Ok(StepDone { report: json!({ "rect": rect, "files": files }), warnings })
    }

    fn align(&mut self, align: &AlignStep, dir: &str) -> Result<StepDone> {
        let reference = self.filter(&align.reference)?;
        let (reference_path, reference_filter) = (reference.path.clone(), reference.filter.clone());
        let options = AlignOptions {
            method: align.method,
            reference: ReferencePolicy::Path { path: reference_path },
            border_crop: align.border_crop,
            area_rule: Some(align.area_rule),
            per_actual_pixel: false,
        };
        let report = align_set(&self.paths(), dir, &options)?;
        for (image, file) in self.images.iter_mut().zip(&report.files) {
            image.path = file.output_path.clone();
        }
        self.aligned = Some(reference_filter);
        Ok(StepDone { warnings: report.warnings.clone(), report: serde_json::to_value(&report)? })
    }

    fn build(&mut self, cube: &CubeStep, dir: &str) -> Result<StepDone> {
        let wanted = cube.reference.clone().or_else(|| self.aligned.clone()).context("name a reference filter or align first")?;
        let reference = self.filter(&wanted)?.filter.clone();
        let specs: Vec<CubeInputSpec> = self
            .images
            .iter()
            .map(|image| CubeInputSpec { path: image.path.clone(), dataset: Some(image.dataset.clone()), filter: Some(image.filter.clone()) })
            .collect();
        let target = Path::new(dir).join(&cube.output).to_string_lossy().into_owned();
        let report = build_cube(&specs, &target, None, CubeOptions { layout: cube.layout, order: cube.order }, Some(&reference))?;
        self.cube = Some(report.output_path.clone());
        Ok(StepDone { warnings: report.warnings.clone(), report: serde_json::to_value(&report)? })
    }

    fn edit(&mut self, edit: &CubeEdit, dir: &str) -> Result<StepDone> {
        let report = edit_cube(&self.cube()?, edit, "", Some(dir), None)?;
        self.cube = Some(report.output_path.clone());
        Ok(StepDone { warnings: report.warnings.clone(), report: serde_json::to_value(&report)? })
    }

    fn psfs(&mut self, psfs: &PsfsStep, dir: &str, progress: &ProgressHandle) -> Result<StepDone> {
        let mut report = Map::new();
        let mut warnings = Vec::new();
        let sources: Vec<(String, String)> = match &psfs.source {
            PsfSource::Files { files } => {
                files.iter().map(|file| Ok((self.filter(&file.filter)?.filter.clone(), file.path.clone()))).collect::<Result<_>>()?
            }
            PsfSource::Empirical(empirical) => {
                let mut built = Vec::with_capacity(empirical.filters.len());
                let mut reports = Vec::with_capacity(empirical.filters.len());
                for wanted in &empirical.filters {
                    check_cancelled(progress)?;
                    let filter = self.filter(wanted)?.filter.clone();
                    let (image, region) = empirical_inputs(empirical, &filter).with_context(|| filter.clone())?;
                    let source = PsfStarSource::Regions { path: region };
                    let psf = build_empirical_psf_file(&image, &source, &empirical.options, Some(&empirical.dataset), Some(&filter), None, Some(dir))
                        .with_context(|| filter.clone())?;
                    warnings.extend(labelled(&filter, &psf.warnings));
                    built.push((filter, psf.output_path.clone().context("the empirical PSF was not written")?));
                    reports.push(psf);
                }
                report.insert("empirical".into(), serde_json::to_value(&reports)?);
                built
            }
        };
        let specs: Vec<PsfInputSpec> =
            sources.iter().map(|(filter, path)| PsfInputSpec { path: path.clone(), filter: Some(filter.clone()), pixel_scale_arcsec: None }).collect();
        let options = PsfSetOptions {
            target_pixel_scale_arcsec: psfs.pixel_scale_arcsec,
            reference: psfs.pixel_scale_arcsec.and(psfs.reference.clone()),
            method: PsfResampleMethod::Lightstack,
            recentre: false,
        };
        let set = load_psfs(&specs, &options, Some(dir))?;
        warnings.extend(set.warnings.iter().cloned());
        self.psfs = sources
            .iter()
            .zip(&set.psfs)
            .map(|((filter, _), prepared)| Ok((filter.clone(), prepared.output_path.clone().context("the prepared PSF was not written")?)))
            .collect::<Result<_>>()?;
        report.insert("prepared".into(), serde_json::to_value(&set)?);
        Ok(StepDone { report: Value::Object(report), warnings })
    }

    fn kernels(&mut self, kernels: &KernelsStep, dir: &str) -> Result<StepDone> {
        let reference = self.filter(&kernels.reference)?.filter.clone();
        let specs: Vec<PsfInputSpec> = self
            .psfs
            .iter()
            .map(|(filter, path)| PsfInputSpec { path: path.clone(), filter: Some(filter.clone()), pixel_scale_arcsec: None })
            .collect();
        let report = build_kernels(&specs, &reference, &kernels.options, Some(dir))?;
        self.kernel_dir = Some(dir.to_string());
        Ok(StepDone { warnings: report.warnings.clone(), report: serde_json::to_value(&report)? })
    }

    fn matching(&mut self, matching: &MatchStep, dir: &str) -> Result<StepDone> {
        let kernel_dir = self.kernel_dir.clone().context("no kernels were built before this step")?;
        let options = MatchOptions { reference: self.filter(&matching.reference)?.filter.clone(), mode: matching.mode, allow_unmatched: matching.allow_unmatched };
        let report = psf_match_cube(&self.cube()?, Some(&kernel_dir), &[], &options, "", Some(dir), None)?;
        self.cube = Some(report.output_path.clone());
        Ok(StepDone { warnings: report.warnings.clone(), report: serde_json::to_value(&report)? })
    }

    fn export(&mut self, output: &str, dir: &str) -> Result<StepDone> {
        let source = self.cube()?;
        let target = Path::new(dir).join(output).to_string_lossy().into_owned();
        std::fs::copy(&source, &target).with_context(|| format!("Failed to copy {source} to {target}"))?;
        invalidate_written(&target);
        self.cube = Some(target.clone());
        Ok(StepDone { report: json!({ "source": source, "output_path": target }), warnings: Vec::new() })
    }
}

fn step_failure(directory: &str, error: anyhow::Error) -> anyhow::Error {
    if let Some(failure) = error.downcast_ref::<MatchError>() {
        let hint = matches!(failure, MatchError::KernelMissing { .. }).then(|| KERNEL_MISSING_HINT.to_string());
        return CodedError { code: failure.code(), message: format!("{directory}: {error:#}"), hint }.into();
    }
    if error.downcast_ref::<AppError>().is_some() {
        return error;
    }
    error.context(directory.to_string())
}

fn prepare_output_dir(dir: &Path) -> Result<()> {
    if dir.exists() && std::fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))?.next().is_some() {
        bail!("the output folder {} is not empty; give a new or empty folder", dir.display());
    }
    std::fs::create_dir_all(dir).with_context(|| format!("Failed to create the output folder {}", dir.display()))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    std::fs::write(path, text + "\n").with_context(|| format!("Failed to write {}", path.display()))
}

fn files_under(root: &Path) -> Result<Vec<RecipeFile>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).with_context(|| format!("Failed to list {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            let relative = path.strip_prefix(root).unwrap_or(&path);
            let text = relative.components().map(|part| part.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            files.push(RecipeFile { path: text, bytes: metadata.len() });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

pub fn run_recipe(recipe: &Recipe, out_dir: &Path, progress: &ProgressHandle) -> Result<RecipeReport> {
    let started = Instant::now();
    recipe.validate()?;
    check_cancelled(progress)?;
    prepare_output_dir(out_dir)?;
    write_json(&out_dir.join(RECIPE_FILE), recipe)?;
    let total = recipe.steps.len() as u64;
    let mut run = Run::new(recipe);
    let mut steps = Vec::with_capacity(recipe.steps.len());
    let mut warnings = Vec::new();
    for (index, step) in recipe.steps.iter().enumerate() {
        check_cancelled(progress)?;
        let directory = format!("{:02}_{}", index + 1, step.kind());
        progress.report(index as u64, total, &directory);
        let dir = out_dir.join(&directory);
        std::fs::create_dir(&dir).with_context(|| format!("Failed to create {}", dir.display()))?;
        let dir_text = dir.to_str().with_context(|| format!("{} is not valid UTF-8", dir.display()))?;
        let step_started = Instant::now();
        let done = run.step(step, dir_text, progress).map_err(|error| step_failure(&directory, error))?;
        warnings.extend(labelled(&directory, &done.warnings));
        steps.push(StepReport {
            index: index + 1,
            step: step.kind(),
            directory,
            parameters: serde_json::to_value(step)?,
            report: done.report,
            warnings: done.warnings,
            elapsed_ms: step_started.elapsed().as_millis() as u64,
        });
    }
    progress.report(total, total, "done");
    let report = RecipeReport {
        recipe_version: recipe.recipe_version,
        name: recipe.name.clone(),
        output_dir: out_dir.to_string_lossy().into_owned(),
        cube: run.cube.clone(),
        steps,
        files: files_under(out_dir)?,
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    write_json(&out_dir.join(RESULT_FILE), &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::core::cubeprep::kernel::{EpsMode, KernelMethod, KernelShape};
    use crate::core::imaging::psf_estimation::RadiusUnit;
    use crate::infra::fits::reader::extract_header_by_index;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("lightstack")
    }

    fn fixture_recipe(folder: &str, file: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(fixtures().join(folder).join(file)).unwrap()).unwrap()
    }

    fn d1_recipe() -> Value {
        fixture_recipe("d1", "recipe_d1.json")
    }

    fn quiet() -> ProgressHandle {
        ProgressHandle::with_sink(Arc::new(|_| {}), 1)
    }

    fn refusal(value: &Value) -> CodedError {
        Recipe::from_value(value).and_then(|recipe| recipe.resolved(&fixtures().join("d1")).validate()).unwrap_err()
    }

    fn with_step(index: usize, step: Value) -> Value {
        let mut recipe = d1_recipe();
        recipe["steps"][index] = step;
        recipe
    }

    #[test]
    fn the_d1_recipe_reads_with_lightstack_defaults_for_every_absent_key() {
        let recipe = Recipe::from_value(&d1_recipe()).unwrap();
        assert_eq!((recipe.recipe_version, recipe.name.as_deref(), recipe.inputs.len()), (1, Some("d1"), 6));
        let kinds: Vec<&str> = recipe.steps.iter().map(RecipeStep::kind).collect();
        assert_eq!(kinds, ["crop", "units", "align", "cube", "edit", "psfs", "kernels", "match"]);
        assert_eq!(
            recipe.steps[0],
            RecipeStep::Crop(CropStep {
                region: CropMode::Radec { ra: 53.1625, dec: -27.7914, size_arcsec: 1.8 },
                rounding: CropRounding::Lightstack
            })
        );
        assert_eq!(recipe.steps[1], RecipeStep::Units { to: RecipeUnit::JyPerPixel });
        assert_eq!(
            recipe.steps[2],
            RecipeStep::Align(AlignStep {
                reference: "F444W".into(),
                method: ResampleMethod::Bilinear,
                border_crop: 1,
                area_rule: AreaRule::Lightstack
            })
        );
        assert_eq!(
            recipe.steps[3],
            RecipeStep::Cube(CubeStep {
                reference: Some("F444W".into()),
                order: PlaneOrder::Wavelength,
                output: "d1_cube.fits".into(),
                layout: CubeLayout::Lightstack
            })
        );
        assert_eq!(recipe.steps[4], RecipeStep::Edit(CubeEdit::DropInvalid { threshold: 0.0, frac_valid: 0.01, use_dq: false }));
        let RecipeStep::Psfs(psfs) = &recipe.steps[5] else { panic!("{:?}", recipe.steps[5]) };
        assert_eq!((psfs.reference.as_deref(), psfs.pixel_scale_arcsec), (Some("F444W"), Some(0.04)));
        let PsfSource::Files { files } = &psfs.source else { panic!("{:?}", psfs.source) };
        assert_eq!(files[1], PsfFile { filter: "F150W".into(), path: "psf/d1_psf_JWST_F150W_20mas.fits".into() });
        assert_eq!(
            recipe.steps[6],
            RecipeStep::Kernels(KernelsStep {
                reference: "F444W".into(),
                options: KernelOptions {
                    method: KernelMethod::Wiener { eps: 0.05, eps_mode: EpsMode::Absolute, shape: KernelShape::default() },
                    auto: false
                }
            })
        );
        assert_eq!(
            recipe.steps[7],
            RecipeStep::Match(MatchStep { reference: "F444W".into(), mode: MatchMode::Lightstack, allow_unmatched: false })
        );
    }

    #[test]
    fn the_star_field_recipe_carries_its_empirical_psf_parameters() {
        let recipe = Recipe::from_value(&fixture_recipe("d1_stars", "recipe_d1_stars.json")).unwrap();
        let RecipeStep::Psfs(PsfsStep { source: PsfSource::Empirical(empirical), reference: None, pixel_scale_arcsec: None }) =
            &recipe.steps[0]
        else {
            panic!("{:?}", recipe.steps[0])
        };
        assert_eq!((empirical.dataset.as_str(), empirical.filters.as_slice()), ("JWST", ["F444W".to_string()].as_slice()));
        assert_eq!(
            empirical.options,
            EmpiricalPsfOptions {
                cutout_size: 31,
                background_border: 5,
                normalization_radius: 0.46,
                normalization_radius_unit: RadiusUnit::Arcsec,
                psf_size: 21,
                ..EmpiricalPsfOptions::default()
            }
        );
        recipe.resolved(&fixtures().join("d1_stars")).validate().unwrap();
    }

    #[test]
    fn relative_paths_resolve_against_the_data_folder_and_absolute_ones_stay() {
        let mut value = d1_recipe();
        let elsewhere = fixtures().join("d1").join("d1_JWST_F444W.fits").to_string_lossy().into_owned();
        value["inputs"][5]["path"] = json!(elsewhere);
        let base = fixtures().join("d1");
        let recipe = Recipe::from_value(&value).unwrap().resolved(&base);
        assert_eq!(Path::new(&recipe.inputs[0].path), base.join("d1_HST_F814W.fits"));
        assert_eq!(recipe.inputs[5].path, elsewhere);
        let RecipeStep::Psfs(PsfsStep { source: PsfSource::Files { files }, .. }) = &recipe.steps[5] else { panic!() };
        assert_eq!(Path::new(&files[0].path), base.join("psf/d1_psf_HST_F814W_40mas.fits"));
        let paths = recipe.paths();
        assert_eq!(paths.len(), 12, "{paths:?}");
        assert!(paths.iter().all(|path| Path::new(path).is_absolute()), "{paths:?}");
        recipe.validate().unwrap();
    }

    #[test]
    fn relative_paths_with_either_separator_or_a_parent_step_resolve_under_a_canonical_folder() {
        let base = std::fs::canonicalize(fixtures().join("d1")).unwrap();
        let mut forms = vec!["../d1/d1_HST_F814W.fits"];
        if cfg!(windows) {
            forms.push("../d1\\d1_HST_F814W.fits");
        }
        for form in forms {
            let mut value = d1_recipe();
            value["inputs"][0]["path"] = json!(form);
            let recipe = Recipe::from_value(&value).unwrap().resolved(&base);
            assert!(recipe.paths().iter().all(|path| Path::new(path).is_file()), "{form}: {:?}", recipe.paths());
            assert_eq!(std::fs::canonicalize(&recipe.inputs[0].path).unwrap(), base.join("d1_HST_F814W.fits"), "{form}");
            recipe.validate().unwrap();
        }
    }

    #[test]
    fn an_unknown_step_is_refused_with_the_known_steps_as_hint() {
        let error = refusal(&with_step(1, json!({ "step": "stretch", "to": "Jy/pixel" })));
        assert_eq!(error.code, UNKNOWN_STEP);
        assert!(error.message.contains("steps[1]") && error.message.contains("stretch"), "{}", error.message);
        assert_eq!(error.hint.as_deref(), Some("known steps are units, crop, align, cube, edit, psfs, kernels, match, export"));
    }

    #[test]
    fn an_unknown_key_is_refused_with_the_accepted_keys_as_hint() {
        let error = refusal(&with_step(2, json!({ "step": "align", "reference": "F444W", "crop": 1 })));
        assert_eq!(error.code, UNKNOWN_PARAMETER);
        assert_eq!(error.message, "steps[2] (align): unknown crop");
        assert_eq!(error.hint.as_deref(), Some("accepted keys: area_rule, border_crop, method, reference"));

        let mut nested = d1_recipe();
        nested["steps"][5]["files"][0]["scale"] = json!(0.04);
        let error = refusal(&nested);
        assert_eq!((error.code, error.message.as_str()), (UNKNOWN_PARAMETER, "steps[5] (psfs): unknown files[0].scale"));
        assert_eq!(error.hint.as_deref(), Some("accepted keys: filter, path"));

        let mut top = d1_recipe();
        top["author"] = json!("someone");
        assert_eq!(refusal(&top).message, "recipe: unknown author");
        let mut input = d1_recipe();
        input["inputs"][0]["unit"] = json!("MJy/sr");
        assert_eq!(refusal(&input).message, "inputs[0]: unknown unit");
    }

    #[test]
    fn malformed_values_name_the_step_that_holds_them() {
        let error = refusal(&with_step(1, json!({ "step": "units", "to": "Jy" })));
        assert_eq!(error.code, RECIPE_INVALID);
        assert!(error.message.starts_with("steps[1] (units): unknown variant `Jy`"), "{}", error.message);
        let error = refusal(&with_step(2, json!({ "step": "align" })));
        assert_eq!((error.code, error.message.as_str()), (RECIPE_INVALID, "steps[2] (align): missing field `reference`"));
        let mut version = d1_recipe();
        version["recipe_version"] = json!(2);
        let error = refusal(&version);
        assert_eq!(error.code, RECIPE_VERSION_UNSUPPORTED);
        assert_eq!(error.hint.as_deref(), Some("this AstroBurst reads recipe_version 1"));
        let mut empty = d1_recipe();
        empty["steps"] = json!([]);
        assert_eq!(refusal(&empty).message, "the recipe has no step");
    }

    #[test]
    fn validation_follows_the_driver_rules() {
        let mut twice = d1_recipe();
        twice["inputs"][1]["filter"] = json!("f444w");
        let error = refusal(&twice);
        assert_eq!((error.code, error.message.as_str()), (FILTER_DUPLICATE, "inputs[5]: filter F444W appears twice"));

        let mut missing = d1_recipe();
        missing["inputs"][2]["path"] = json!("nowhere/d1_JWST_F200W.fits");
        let error = refusal(&missing);
        assert_eq!(error.code, FILE_NOT_FOUND);
        assert!(error.message.starts_with("inputs[2]: ") && error.message.contains("nowhere"), "{}", error.message);

        for (index, step, message) in [
            (0, json!({ "step": "crop", "mode": "radec", "ra": 53.1625, "dec": -27.7914, "size_arcsec": 0.0 }), "steps[0] (crop): size_arcsec must be positive"),
            (0, json!({ "step": "crop", "mode": "reference", "reference": "F090W" }), "steps[0] (crop): reference F090W is not an input filter"),
            (0, json!({ "step": "crop", "mode": "pixel", "x_start": 5, "x_end": 5, "y_start": 0, "y_end": 9 }), "steps[0] (crop): empty pixel box"),
            (2, json!({ "step": "edit", "action": "drop_invalid" }), "steps[2] (edit): build a cube first"),
            (3, json!({ "step": "cube", "output": "../d1_cube.fits" }), "steps[3] (cube): output must be a .fits file name"),
            (6, json!({ "step": "kernels", "reference": "F090W", "method": "wiener" }), "steps[6] (kernels): no PSF for the reference F090W"),
            (6, json!({ "step": "kernels", "reference": "F444W", "method": "wiener", "eps": 0.0 }), "steps[6] (kernels): eps must be positive"),
            (6, json!({ "step": "kernels", "reference": "F444W", "method": "wiener", "shape": [20, 21] }), "steps[6] (kernels): shape must be reference or two odd integers"),
            (7, json!({ "step": "match", "reference": "F277W" }), "steps[7] (match): no kernels to F277W"),
        ] {
            let error = refusal(&with_step(index, step));
            assert_eq!((error.code, error.message.as_str()), (RECIPE_INVALID, message));
        }

        let mut unaligned = d1_recipe();
        unaligned["steps"] = json!([{ "step": "cube" }]);
        assert_eq!(refusal(&unaligned).message, "steps[0] (cube): name a reference filter or align first");

        let mut scale = d1_recipe();
        scale["steps"][5]["reference"] = json!(null);
        assert_eq!(refusal(&scale).message, "steps[5] (psfs): resampling needs a reference filter among the PSFs");
        let mut foreign = d1_recipe();
        foreign["steps"][5]["files"][0]["filter"] = json!("F090W");
        assert_eq!(refusal(&foreign).message, "steps[5] (psfs): PSFs for filters that are not inputs: F090W");
        let mut order = fixture_recipe("d1_stars", "recipe_d1_stars.json");
        order["steps"][0]["oversampling_order"] = json!(1);
        let error = Recipe::from_value(&order).unwrap().resolved(&fixtures().join("d1_stars")).validate().unwrap_err();
        assert_eq!(error.message, "steps[0] (psfs): only the cubic oversampling (oversampling_order 3) is ported");
    }

    #[test]
    fn the_d1_recipe_runs_every_step_into_numbered_folders() {
        let out = tempfile::tempdir().unwrap();
        let target = out.path().join("run");
        let mut value = d1_recipe();
        value["steps"].as_array_mut().unwrap().push(json!({ "step": "export", "output": "d1_final.fits" }));
        let recipe = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1"));
        let report = run_recipe(&recipe, &target, &quiet()).unwrap();
        let folders: Vec<&str> = report.steps.iter().map(|step| step.directory.as_str()).collect();
        assert_eq!(folders, ["01_crop", "02_units", "03_align", "04_cube", "05_edit", "06_psfs", "07_kernels", "08_match", "09_export"]);
        assert!(folders.iter().all(|folder| target.join(folder).is_dir()));
        let cube = report.cube.as_deref().unwrap();
        assert_eq!(Path::new(cube), target.join("09_export").join("d1_final.fits").as_path());
        let matched = report.steps[7].report["output_path"].as_str().unwrap();
        assert!(Path::new(matched).starts_with(target.join("08_match")), "{matched}");
        assert_eq!(std::fs::read(cube).unwrap(), std::fs::read(matched).unwrap());
        let header = extract_header_by_index(&std::fs::File::open(cube).unwrap(), 0).unwrap();
        assert_eq!(header.get_i64("NAXIS3"), Some(6));
        let names: Vec<&str> = report.files.iter().map(|file| file.path.as_str()).collect();
        assert!(names.contains(&RECIPE_FILE), "{names:?}");
        assert!(names.contains(&"07_kernels/kernel_f150w_to_f444w.fits"), "{names:?}");
        assert!(report.files.iter().all(|file| file.bytes > 0));
        let written: Value = serde_json::from_str(&std::fs::read_to_string(target.join(RESULT_FILE)).unwrap()).unwrap();
        assert_eq!(written["cube"], json!(cube));
        let canonical: Value = serde_json::from_str(&std::fs::read_to_string(target.join(RECIPE_FILE)).unwrap()).unwrap();
        assert_eq!(Recipe::from_value(&canonical).unwrap(), recipe);
        assert_eq!(report.steps[2].parameters["area_rule"], json!("lightstack"));
        let labelled = |warning: &String| folders.iter().any(|folder| warning.starts_with(&format!("{folder}: ")));
        assert!(!report.warnings.is_empty() && report.warnings.iter().all(labelled), "{:?}", report.warnings);
        assert_eq!(report.steps[6].warnings.len(), report.warnings.iter().filter(|warning| warning.starts_with("07_kernels: ")).count());
    }

    #[test]
    fn a_plane_without_a_kernel_fails_the_match_with_code_kernel_missing() {
        let mut value = d1_recipe();
        value["steps"][5]["files"].as_array_mut().unwrap().remove(1);
        let recipe = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1"));
        recipe.validate().unwrap();
        let out = tempfile::tempdir().unwrap();
        let error = run_recipe(&recipe, &out.path().join("run"), &quiet()).unwrap_err();
        let coded = error.downcast_ref::<CodedError>().unwrap_or_else(|| panic!("{error:#}"));
        assert_eq!(coded.code, "kernel_missing");
        assert!(coded.message.starts_with("08_match: ") && coded.message.contains("F150W"), "{}", coded.message);
        assert!(coded.hint.as_deref().is_some_and(|hint| hint.contains("allow_unmatched")));
        assert!(out.path().join("run").join("07_kernels").is_dir(), "the earlier steps stay for inspection");
    }

    #[test]
    fn a_used_output_folder_and_a_cancelled_run_stop_before_any_step() {
        let recipe = Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1"));
        let out = tempfile::tempdir().unwrap();
        std::fs::write(out.path().join("keep.txt"), "x").unwrap();
        let error = run_recipe(&recipe, out.path(), &quiet()).unwrap_err();
        assert!(format!("{error:#}").contains("is not empty"), "{error:#}");
        let cancelled = quiet().with_cancel_source(Arc::new(|| true));
        let target = out.path().join("cancelled");
        let error = run_recipe(&recipe, &target, &cancelled).unwrap_err();
        assert!(format!("{error:#}").to_lowercase().contains("cancel"), "{error:#}");
        assert!(!target.join("01_crop").exists());
    }

    #[test]
    fn run_recipe_validates_first_and_writes_nothing_for_an_invalid_recipe() {
        let mut value = d1_recipe();
        value["steps"] = json!([{ "step": "crop", "mode": "pixel", "x_start": 5, "x_end": 0, "y_start": 0, "y_end": 5 }]);
        let recipe = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1"));
        let out = tempfile::tempdir().unwrap();
        let target = out.path().join("run");
        let error = run_recipe(&recipe, &target, &quiet()).unwrap_err();
        let coded = error.downcast_ref::<CodedError>().unwrap_or_else(|| panic!("{error:#}"));
        assert_eq!((coded.code, coded.message.as_str()), (RECIPE_INVALID, "steps[0] (crop): empty pixel box"));
        assert!(!target.exists());
    }

    #[test]
    fn an_empirical_psf_needs_one_image_and_its_star_file_in_data_dir() {
        let stars = fixtures().join("d1_stars");
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("stars");
        std::fs::create_dir(&data_dir).unwrap();
        std::fs::copy(stars.join("d1_stars_JWST_F444W.fits"), root.path().join("input.fits")).unwrap();
        let mut value = fixture_recipe("d1_stars", "recipe_d1_stars.json");
        value["inputs"][0]["path"] = json!("input.fits");
        value["steps"][0]["data_dir"] = json!("stars");
        let recipe = Recipe::from_value(&value).unwrap().resolved(root.path());
        let refused = |code: &str, fragment: &str| {
            let error = recipe.validate().unwrap_err();
            assert_eq!((error.code, error.message.contains(fragment)), (code, true), "{}", error.message);
        };
        refused(FILE_NOT_FOUND, "no FITS file");
        std::fs::copy(stars.join("d1_stars_JWST_F444W.fits"), data_dir.join("x_JWST_F444W.fits")).unwrap();
        refused(FILE_NOT_FOUND, "stars_f444w.reg");
        std::fs::copy(stars.join("stars_f444w.reg"), data_dir.join("stars_f444w.reg")).unwrap();
        std::fs::create_dir(data_dir.join("y_JWST_F444W.fits")).unwrap();
        recipe.validate().unwrap();
        let found = [data_dir.join("x_JWST_F444W.fits"), data_dir.join("stars_f444w.reg")].map(|path| path.to_string_lossy().into_owned());
        assert_eq!(recipe.empirical_files().unwrap(), found);
        std::fs::copy(stars.join("d1_stars_JWST_F444W.fits"), data_dir.join("z_JWST_F444W.fits")).unwrap();
        refused(RECIPE_INVALID, "keep one");
    }

    #[test]
    fn inputs_name_a_filter_of_their_dataset_in_the_registry() {
        let cases = [
            ("HTS", "F814W", "inputs[0]: dataset HTS is not in the filter registry"),
            ("HST", "F20W", "inputs[0]: the filter registry has no HST filter F20W"),
            ("HST", "F275WU", "inputs[0]: F275WU is an alias of HST F275W; write F275W"),
        ];
        for (dataset, filter, message) in cases {
            let mut value = d1_recipe();
            value["inputs"][0]["dataset"] = json!(dataset);
            value["inputs"][0]["filter"] = json!(filter);
            let error = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1")).validate().unwrap_err();
            assert_eq!((error.code, error.message.as_str()), (RECIPE_INVALID, message));
            assert!(error.hint.is_some(), "{message}");
        }
        let mut value = d1_recipe();
        value["inputs"][0]["filter"] = json!("f814w");
        Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1")).validate().unwrap();
    }

    #[test]
    fn psfs_steps_add_up_as_in_lightstack() {
        let mut value = d1_recipe();
        let files = value["steps"][5]["files"].as_array().unwrap().clone();
        let mut jwst = value["steps"][5].clone();
        jwst["files"] = json!(files[1..].to_vec());
        let hst = json!({ "step": "psfs", "source": "files", "files": [files[0].clone()] });
        let steps = value["steps"].as_array_mut().unwrap();
        steps[5] = hst;
        steps.insert(6, jwst);
        let recipe = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1"));
        let out = tempfile::tempdir().unwrap();
        let report = run_recipe(&recipe, &out.path().join("run"), &quiet()).unwrap_or_else(|error| panic!("{error:#}"));
        let names: Vec<&str> = report.files.iter().map(|file| file.path.as_str()).collect();
        assert!(names.contains(&"08_kernels/kernel_f814w_to_f444w.fits"), "{names:?}");
        assert!(Path::new(report.cube.as_deref().unwrap()).starts_with(out.path().join("run").join("09_match")));
    }

    #[test]
    fn a_recipe_version_written_as_1_0_reads_as_1() {
        let mut value = d1_recipe();
        value["recipe_version"] = json!(1.0);
        assert_eq!(Recipe::from_value(&value).unwrap().recipe_version, RECIPE_VERSION);
        value["recipe_version"] = json!(1.5);
        assert_eq!(Recipe::from_value(&value).unwrap_err().code, RECIPE_VERSION_UNSUPPORTED);
    }

    #[test]
    fn unknown_keys_are_named_one_place_at_a_time_with_the_keys_that_place_accepts() {
        let mut value = d1_recipe();
        value["steps"][5]["files"][0]["scale"] = json!(1);
        value["steps"][5]["foo"] = json!(1);
        let error = Recipe::from_value(&value).unwrap_err();
        assert_eq!((error.code, error.message.as_str()), (UNKNOWN_PARAMETER, "steps[5] (psfs): unknown files[0].scale"));
        assert_eq!(error.hint.as_deref(), Some("accepted keys: filter, path"));
        value["steps"][5]["files"][0].as_object_mut().unwrap().remove("scale");
        let error = Recipe::from_value(&value).unwrap_err();
        assert_eq!(error.message, "steps[5] (psfs): unknown foo");
        assert_eq!(error.hint.as_deref(), Some("accepted keys: files, pixel_scale_arcsec, reference, source"));
    }

    #[test]
    fn empirical_options_and_window_kernels_are_checked_before_the_run() {
        let mut value = fixture_recipe("d1_stars", "recipe_d1_stars.json");
        value["steps"][0]["cutout_size"] = json!(30);
        let error = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1_stars")).validate().unwrap_err();
        assert_eq!((error.code, error.message.as_str()), (RECIPE_INVALID, "steps[0] (psfs): cutout_size must be a positive odd number of pixels, got 30"));
        let mut value = d1_recipe();
        value["steps"][6] = json!({ "step": "kernels", "reference": "F444W", "method": "window", "window": "hanning", "regularization": -1.0 });
        let error = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1")).validate().unwrap_err();
        assert_eq!((error.code, error.message.as_str()), (RECIPE_INVALID, "steps[6] (kernels): the regularization must lie in [0, 1), got -1"));
        value["steps"][6] = json!({ "step": "kernels", "reference": "F444W", "method": "window", "window": "cosine_bell", "alpha": 1.5 });
        let error = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1")).validate().unwrap_err();
        assert!(error.message.starts_with("steps[6] (kernels): the cosine_bell window needs alpha and beta in [0, 1]"), "{}", error.message);
    }

    #[test]
    fn every_key_the_documentation_lists_is_accepted() {
        let steps = [
            json!({ "step": "units", "to": "MJy/sr" }),
            json!({ "step": "crop", "mode": "radec", "ra": 1.0, "dec": 2.0, "size_arcsec": 3.0, "rounding": "astroburst" }),
            json!({ "step": "crop", "mode": "reference", "reference": "F444W", "rounding": "lightstack" }),
            json!({ "step": "crop", "mode": "common" }),
            json!({ "step": "crop", "mode": "region", "region_file": "a.reg", "index": 1 }),
            json!({ "step": "crop", "mode": "region_common", "region_file": "a.reg" }),
            json!({ "step": "crop", "mode": "pixel", "x_start": 0, "x_end": 4, "y_start": 0, "y_end": 4 }),
            json!({ "step": "align", "reference": "F444W", "method": "exact", "border_crop": 0, "area_rule": "scale" }),
            json!({ "step": "align", "reference": "F444W", "method": "bicubic", "area_rule": "keep" }),
            json!({ "step": "cube", "reference": "F444W", "order": "given", "output": "c.fits", "layout": "mef" }),
            json!({ "step": "edit", "action": "drop_invalid", "threshold": 0.0, "frac_valid": 0.5, "use_dq": true }),
            json!({ "step": "edit", "action": "remove_filter", "filter": "F444W" }),
            json!({ "step": "edit", "action": "cut", "x_start": 0, "x_end": 4, "y_start": 0, "y_end": 4 }),
            json!({ "step": "edit", "action": "cut_sky", "ra": 1.0, "dec": 2.0, "size_arcsec": 3.0, "rounding": "lightstack" }),
            json!({ "step": "edit", "action": "header", "redshift": 0.1, "ra_center": 1.0, "dec_center": 2.0, "use_brightest_pixel": false }),
            json!({ "step": "edit", "action": "reference_nan_mask", "reference_slice": 0, "mode": "union" }),
            json!({ "step": "psfs", "source": "files", "files": [{ "filter": "F444W", "path": "p.fits" }], "pixel_scale_arcsec": 0.04, "reference": "F444W" }),
            json!({
                "step": "psfs", "source": "empirical", "dataset": "JWST", "filters": ["F444W"], "data_dir": "d", "cutout_size": 31,
                "background_border": 5, "background_sigma": 4.0, "clip_negative": true, "noise_threshold": 0.5, "radial_cleaning": true,
                "cleaning_sigma": 3.0, "cleaning_consecutive_bins": 2, "normalization_radius": 0.5, "normalization_radius_unit": "arcsec",
                "psf_size": 21, "oversampling": 2, "stack": "median", "oversampling_order": 3
            }),
            json!({ "step": "kernels", "reference": "F444W", "method": "wiener", "eps": 0.01, "eps_mode": "relative", "shape": [21, 21], "auto": false }),
            json!({ "step": "kernels", "reference": "F444W", "method": "window", "window": "split_cosine_bell", "alpha": 0.5, "beta": 0.5, "regularization": 0.001, "auto": true }),
            json!({ "step": "kernels", "reference": "F444W", "method": "window", "window": "top_hat", "beta": 0.5 }),
            json!({ "step": "kernels", "reference": "F444W", "method": "window", "window": "tukey", "alpha": 0.5 }),
            json!({ "step": "match", "reference": "F444W", "mode": "astroburst", "allow_unmatched": true }),
            json!({ "step": "export", "output": "e.fits" }),
        ];
        for step in steps {
            let mut value = d1_recipe();
            value["steps"] = json!([step.clone()]);
            Recipe::from_value(&value).unwrap_or_else(|error| panic!("{step}: {}", error.message));
        }
    }
}
