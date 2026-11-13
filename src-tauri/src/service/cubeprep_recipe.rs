use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::core::cubeprep::align::{AlignOptions, ReferencePolicy, ResampleMethod};
use crate::core::cubeprep::cube::{CubeEdit, CubeLayout, CubeOptions, PlaneOrder};
use crate::core::cubeprep::footprint::{CropRegion, CropRounding};
use crate::core::cubeprep::kernel::{KernelMethod, KernelOptions, KernelShape};
use crate::core::cubeprep::match_cube::{MatchError, MatchMode, MatchOptions};
use crate::core::cubeprep::psf_io::{PsfResampleMethod, PsfSetOptions};
use crate::core::imaging::cutout::CutoutRect;
use crate::core::imaging::psf_estimation::EmpiricalPsfOptions;
use crate::core::history::stamp::{file_stamp, utc_timestamp};
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::metadata::filter_wavelengths::FilterRegistry;
use crate::core::metadata::units::{AreaRule, TargetUnit};
use crate::infra::paths::plain;
use crate::infra::progress::ProgressHandle;
use crate::service::cubeprep::{
    align_set_with, build_cube_with, build_kernels_with, convert_units_file, crop_set_with, edit_cube_with, filter_registry, filter_registry_report, load_psfs_with,
    psf_match_cube_with, CubeInputSpec, PsfInputSpec,
};
use crate::service::cutout::export_cutout_rect;
use crate::service::empirical_psf::{build_empirical_psf_file, PsfStarSource};
use crate::service::image_files::invalidate_written;
use crate::types::error::{AppError, CodedError};

pub const RECIPE_VERSION: u64 = 1;
pub const RECIPE_FILE: &str = "recipe.json";
pub const RESULT_FILE: &str = "result.json";
pub const FAILURE_FILE: &str = "failure.json";
const RUN_LOCK: &str = ".cubeprep.lock";
pub const STEPS: [&str; 9] = ["units", "crop", "align", "cube", "edit", "psfs", "kernels", "match", "export"];

pub const UNKNOWN_STEP: &str = "unknown_step";
pub const UNKNOWN_PARAMETER: &str = "unknown_parameter";
pub const RECIPE_INVALID: &str = "recipe_invalid";
pub const RECIPE_VERSION_UNSUPPORTED: &str = "recipe_version_unsupported";
pub const FILTER_DUPLICATE: &str = "filter_duplicate";
pub const FILE_NOT_FOUND: &str = "file_not_found";

const DEFAULT_CUBE_OUTPUT: &str = "cube.fits";
const DEFAULT_BORDER_CROP: usize = 1;
const HASH_BLOCK: usize = 8 * 1024 * 1024;
const HASH_WORKERS: usize = 4;
const HASHING_INPUTS: &str = "hashing_inputs";
const VERIFYING_INPUTS: &str = "verifying_inputs";
const HASHING_PRODUCTS: &str = "hashing_products";
const PUBLISHING: &str = "publishing";
const PANIC: &str = "panic";
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recentre: Option<bool>,
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
    pub provenance: RecipeProvenance,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecipeProvenance {
    pub schema_version: u8,
    pub software_version: String,
    pub operating_system: String,
    pub architecture: String,
    pub started_at_utc: String,
    pub recipe_sha256: String,
    pub dependencies: Vec<RecipeDependency>,
    pub filter_registry: Value,
    pub integrity: &'static str,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecipeDependency {
    pub path: String,
    pub roles: Vec<String>,
    pub bytes: u64,
    pub sha256: String,
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
    pub sha256: String,
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
            Err(coded(
                UNKNOWN_PARAMETER,
                format!("{place}: unknown {}", first.path),
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
        *path = plain(&base.join(path.as_str())).to_string_lossy().into_owned();
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
        if version.as_f64() != Some(RECIPE_VERSION as f64) {
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

    pub fn validate(&self) -> Result<Vec<String>, CodedError> {
        let (registry, skipped) = filter_registry_report();
        self.validate_with(&registry, &skipped)
    }

    fn validate_with(&self, registry: &FilterRegistry, skipped: &[String]) -> Result<Vec<String>, CodedError> {
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
        let suffix: String = skipped.iter().map(|warning| format!("; {warning}")).collect();
        let datasets = registry.datasets();
        for (index, input) in self.inputs.iter().enumerate() {
            let place = format!("inputs[{index}]");
            if !datasets.iter().any(|dataset| same(dataset, &input.dataset)) {
                return Err(coded(RECIPE_INVALID, format!("{place}: dataset {} is not in the filter registry", input.dataset), Some(format!("known datasets: {}{suffix}", datasets.join(", ")))));
            }
            let entry = registry.entry(&input.dataset, &input.filter).ok_or_else(|| coded(RECIPE_INVALID,
                format!("{place}: the filter registry has no {} filter {}", input.dataset, input.filter),
                Some(format!("choose a filter from the dataset's filter registry{suffix}"))))?;
            if !same(&entry.filter, &input.filter) {
                return Err(coded(RECIPE_INVALID, format!("{place}: {} is an alias of {} {}; write {}", input.filter, entry.dataset, entry.filter, entry.filter),
                    Some(format!("use {} consistently in inputs and step references", entry.filter))));
            }
        }
        let mut planned = Planned::default();
        for (index, step) in self.steps.iter().enumerate() {
            validate_step(&format!("steps[{index}] ({})", step.kind()), step, &filters, &mut planned)?;
        }
        let mut warnings = cut_before_match(&self.steps);
        warnings.extend(skipped.iter().cloned());
        Ok(warnings)
    }
}

fn step_folder(index: usize, step: &RecipeStep) -> String {
    format!("{:02}_{}", index + 1, step.kind())
}

fn cut_before_match(steps: &[RecipeStep]) -> Vec<String> {
    let mut cut = None;
    let mut kernel_half_width = None;
    let mut warnings = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        match step {
            RecipeStep::Edit(CubeEdit::Cut { .. } | CubeEdit::CutSky { .. }) => cut = Some(step_folder(index, step)),
            RecipeStep::Kernels(kernels) => {
                kernel_half_width = match kernels.options.method {
                    KernelMethod::Wiener { shape: KernelShape::Fixed([rows, cols]), .. } => Some(rows.max(cols) / 2),
                    _ => None,
                };
            }
            RecipeStep::Match(_) => {
                if let Some(cut) = &cut {
                    let band = kernel_half_width.map_or_else(|| "half a kernel width".to_string(), |px| format!("{px} px"));
                    warnings.push(format!(
                        "{}: {cut} cuts the cube before the match; pixels within {band} of the cut edges are matched against zeros; place the cut after match to use the cut-away data",
                        step_folder(index, step)
                    ));
                }
            }
            _ => {}
        }
    }
    warnings
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
                    crate::core::imaging::psf_estimation::validate_recipe(&empirical.options)
                        .map_err(|error| invalid(format!("{place}: {error}")))?;
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
            for name in names {
                if !planned.psfs.iter().any(|known| same(known, &name)) {
                    planned.psfs.push(name);
                }
            }
        }
        RecipeStep::Kernels(kernels) => {
            if let KernelMethod::Window { window, regularization } = kernels.options.method {
                crate::core::cubeprep::kernel::validate_window_options(window, regularization)
                    .map_err(|error| invalid(format!("{place}: {error}")))?;
            }
            if let KernelMethod::Wiener { eps, shape, .. } = kernels.options.method {
                if !positive(eps) {
                    return Err(invalid(format!("{place}: eps must be positive")));
                }
                if let KernelShape::Fixed(sides) = shape {
                    if sides.iter().any(|side| side % 2 == 0) {
                        return Err(invalid(format!("{place}: shape must be reference, max or two odd integers")));
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
            RecipeStep::Align(align) => self.align(align, dir, progress),
            RecipeStep::Cube(cube) => self.build(cube, dir, progress),
            RecipeStep::Edit(edit) => self.edit(edit, dir, progress),
            RecipeStep::Psfs(psfs) => self.psfs(psfs, dir, progress),
            RecipeStep::Kernels(kernels) => self.kernels(kernels, dir, progress),
            RecipeStep::Match(matching) => self.matching(matching, dir, progress),
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
        let report = crop_set_with(&self.paths(), &region, crop.rounding, dir, true, true, &|| progress.is_cancelled())?;
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

    fn align(&mut self, align: &AlignStep, dir: &str, progress: &ProgressHandle) -> Result<StepDone> {
        let reference = self.filter(&align.reference)?;
        let (reference_path, reference_filter) = (reference.path.clone(), reference.filter.clone());
        let options = AlignOptions {
            method: align.method,
            reference: ReferencePolicy::Path { path: reference_path },
            border_crop: align.border_crop,
            area_rule: Some(align.area_rule),
            per_actual_pixel: false,
        };
        let report = align_set_with(&self.paths(), dir, &options, &|| progress.is_cancelled())?;
        for (image, file) in self.images.iter_mut().zip(&report.files) {
            image.path = file.output_path.clone();
        }
        self.aligned = Some(reference_filter);
        Ok(StepDone { warnings: report.warnings.clone(), report: serde_json::to_value(&report)? })
    }

    fn build(&mut self, cube: &CubeStep, dir: &str, progress: &ProgressHandle) -> Result<StepDone> {
        let wanted = cube.reference.clone().or_else(|| self.aligned.clone()).context("name a reference filter or align first")?;
        let reference = self.filter(&wanted)?.filter.clone();
        let specs: Vec<CubeInputSpec> = self
            .images
            .iter()
            .map(|image| CubeInputSpec { path: image.path.clone(), dataset: Some(image.dataset.clone()), filter: Some(image.filter.clone()) })
            .collect();
        let target = Path::new(dir).join(&cube.output).to_string_lossy().into_owned();
        let report =
            build_cube_with(&specs, &target, None, CubeOptions { layout: cube.layout, order: cube.order }, Some(&reference), &|| progress.is_cancelled())?;
        self.cube = Some(report.output_path.clone());
        Ok(StepDone { warnings: report.warnings.clone(), report: serde_json::to_value(&report)? })
    }

    fn edit(&mut self, edit: &CubeEdit, dir: &str, progress: &ProgressHandle) -> Result<StepDone> {
        let report = edit_cube_with(&self.cube()?, edit, "", Some(dir), None, &|| progress.is_cancelled())?;
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
            recentre: psfs.recentre.unwrap_or(false),
        };
        let set = load_psfs_with(&specs, &options, Some(dir), &|| progress.is_cancelled())?;
        warnings.extend(set.warnings.iter().cloned());
        let prepared = sources
            .iter()
            .zip(&set.psfs)
            .map(|((filter, _), prepared)| Ok((filter.clone(), prepared.output_path.clone().context("the prepared PSF was not written")?)))
            .collect::<Result<Vec<_>>>()?;
        for (filter, path) in prepared {
            self.psfs.retain(|(known, _)| !same(known, &filter));
            self.psfs.push((filter, path));
        }
        report.insert("prepared".into(), serde_json::to_value(&set)?);
        Ok(StepDone { report: Value::Object(report), warnings })
    }

    fn kernels(&mut self, kernels: &KernelsStep, dir: &str, progress: &ProgressHandle) -> Result<StepDone> {
        let reference = self.filter(&kernels.reference)?.filter.clone();
        let specs: Vec<PsfInputSpec> = self
            .psfs
            .iter()
            .map(|(filter, path)| PsfInputSpec { path: path.clone(), filter: Some(filter.clone()), pixel_scale_arcsec: None })
            .collect();
        let report = build_kernels_with(&specs, &reference, &kernels.options, Some(dir), &|| progress.is_cancelled())?;
        self.kernel_dir = Some(dir.to_string());
        Ok(StepDone { warnings: report.warnings.clone(), report: serde_json::to_value(&report)? })
    }

    fn matching(&mut self, matching: &MatchStep, dir: &str, progress: &ProgressHandle) -> Result<StepDone> {
        let kernel_dir = self.kernel_dir.clone().context("no kernels were built before this step")?;
        let options = MatchOptions { reference: self.filter(&matching.reference)?.filter.clone(), mode: matching.mode, allow_unmatched: matching.allow_unmatched };
        let report = psf_match_cube_with(&self.cube()?, Some(&kernel_dir), &[], &options, "", Some(dir), None, &|| progress.is_cancelled())?;
        self.cube = Some(report.output_path.clone());
        Ok(StepDone { warnings: report.warnings.clone(), report: serde_json::to_value(&report)? })
    }

    fn export(&mut self, output: &str, dir: &str) -> Result<StepDone> {
        let source = self.cube()?;
        let target = Path::new(dir).join(output).to_string_lossy().into_owned();
        let mut staged = tempfile::NamedTempFile::new_in(dir).with_context(|| format!("Failed to stage {target}"))?;
        let mut cube = File::open(&source).with_context(|| format!("Failed to open {source}"))?;
        std::io::copy(&mut cube, staged.as_file_mut()).with_context(|| format!("Failed to copy {source} to {target}"))?;
        let permissions = cube.metadata().with_context(|| format!("Failed to read {source}"))?.permissions();
        staged.as_file().set_permissions(permissions).with_context(|| format!("Failed to give {target} the permissions of {source}"))?;
        staged.as_file().sync_all().with_context(|| format!("Failed to flush {target}"))?;
        staged.persist_noclobber(&target).map_err(|error| error.error).with_context(|| format!("Failed to publish {target}"))?;
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

fn file_digest(path: &Path, progress: &ProgressHandle) -> Result<(u64, String)> {
    check_cancelled(progress)?;
    let mut file = File::open(path).with_context(|| format!("Cannot hash {}", path.display()))?;
    let before = file_stamp(&file.metadata()?);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BLOCK];
    let mut bytes = 0u64;
    loop {
        check_cancelled(progress)?;
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        bytes += read as u64;
        hasher.update(&buffer[..read]);
    }
    if before != file_stamp(&file.metadata()?) || before.0 != bytes {
        return Err(coded("input_changed", format!("File changed while hashing {}", path.display()), Some("Keep inputs unchanged and run again in a new output folder".into())).into());
    }
    let digest = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok((bytes, digest))
}

fn dependency_paths(recipe: &Recipe) -> Result<BTreeMap<String, Vec<String>>> {
    let mut paths: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut add = |path: &str, role: String| -> Result<()> {
        let canonical = Path::new(path).canonicalize().with_context(|| format!("Cannot resolve dependency {path}"))?;
        let key = plain(&canonical).to_string_lossy().into_owned();
        let roles = paths.entry(key).or_default();
        if !roles.contains(&role) { roles.push(role); }
        Ok(())
    };
    for input in &recipe.inputs {
        add(&input.path, format!("science:{}:{}", input.dataset, input.filter))?;
    }
    for (index, step) in recipe.steps.iter().enumerate() {
        match step {
            RecipeStep::Crop(CropStep { region: CropMode::Region { region_file, .. } | CropMode::RegionCommon { region_file }, .. }) => add(region_file, format!("region:step{}", index + 1))?,
            RecipeStep::Psfs(PsfsStep { source: PsfSource::Files { files }, .. }) => {
                for file in files { add(&file.path, format!("psf:{}:step{}", file.filter, index + 1))?; }
            }
            RecipeStep::Psfs(PsfsStep { source: PsfSource::Empirical(empirical), .. }) => {
                for filter in &empirical.filters {
                    let (image, region) = empirical_inputs(empirical, filter)?;
                    add(&image, format!("empirical_image:{filter}:step{}", index + 1))?;
                    add(&region, format!("empirical_region:{filter}:step{}", index + 1))?;
                }
            }
            _ => {}
        }
    }
    Ok(paths)
}

fn digests(paths: &[&Path], progress: &ProgressHandle, stage: &str) -> Result<Vec<(u64, String)>> {
    let workers = std::thread::available_parallelism().map_or(1, |cores| cores.get()).min(HASH_WORKERS).min(paths.len());
    let files = paths.len() as u64;
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let done = Mutex::new(Vec::with_capacity(paths.len()));
    progress.report(0, files, stage);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                while !failed.load(Ordering::SeqCst) {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(path) = paths.get(index) else { break };
                    let digest = file_digest(path, progress);
                    failed.fetch_or(digest.is_err(), Ordering::SeqCst);
                    let mut done = done.lock().unwrap();
                    done.push((index, digest));
                    progress.report(done.len() as u64, files, stage);
                }
            });
        }
    });
    let mut done = done.into_inner().unwrap();
    done.sort_by_key(|(index, _)| *index);
    done.into_iter().map(|(_, digest)| digest).collect()
}

fn dependencies(recipe: &Recipe, progress: &ProgressHandle, stage: &str) -> Result<Vec<RecipeDependency>> {
    let listed: Vec<(String, Vec<String>)> = dependency_paths(recipe)?.into_iter().collect();
    let paths: Vec<&Path> = listed.iter().map(|(path, _)| Path::new(path)).collect();
    let digests = digests(&paths, progress, stage)?;
    Ok(listed.into_iter().zip(digests).map(|((path, roles), (bytes, sha256))| RecipeDependency { path, roles, bytes, sha256 }).collect())
}

struct RecipeLock(PathBuf);

impl RecipeLock {
    fn acquire(out_dir: &Path, started_at_utc: &str) -> Result<Self> {
        let path = out_dir.join(RUN_LOCK);
        let mut file = OpenOptions::new().write(true).create_new(true).open(&path).context("Another preparation is using this output folder")?;
        let lock = Self(path);
        let holder = json!({ "pid": std::process::id(), "started_at_utc": started_at_utc, "software_version": env!("CARGO_PKG_VERSION") });
        serde_json::to_writer_pretty(&mut file, &holder)?;
        Ok(lock)
    }
}

impl Drop for RecipeLock {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

struct Stage {
    name: String,
    step_index: Option<usize>,
}

impl Stage {
    fn named(name: &str) -> Self {
        Self { name: name.to_string(), step_index: None }
    }
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let folder = path.parent().context("The file has no parent folder")?;
    #[cfg(unix)]
    let mut file = tempfile::Builder::new().permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666)).tempfile_in(folder)?;
    #[cfg(not(unix))]
    let mut file = tempfile::NamedTempFile::new_in(folder)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path).map_err(|error| error.error)?;
    Ok(())
}

fn write_failure(out_dir: &Path, stage: &Stage, error: &anyhow::Error, warnings: &[String], progress: &ProgressHandle, started_at_utc: &str) {
    let coded = error.downcast_ref::<CodedError>();
    let cancelled = error.downcast_ref::<AppError>().is_some();
    let marker = json!({
        "schema_version": 1,
        "status": if cancelled || progress.is_cancelled() { "cancelled" } else { "failed" },
        "stage": stage.name,
        "step_index": stage.step_index,
        "code": coded.map_or(if cancelled { "cancelled" } else { "cubeprep_failed" }, |coded| coded.code),
        "message": format!("{error:#}"),
        "hint": coded.and_then(|coded| coded.hint.clone()),
        "warnings": warnings,
        "published": false,
        "software_version": env!("CARGO_PKG_VERSION"),
        "started_at_utc": started_at_utc,
        "ended_at_utc": utc_timestamp(SystemTime::now()),
    });
    let _ = atomic_json(&out_dir.join(FAILURE_FILE), &marker);
}

fn files_under(root: &Path, progress: &ProgressHandle, stage: &str) -> Result<Vec<RecipeFile>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).with_context(|| format!("Failed to list {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            if path.file_name().is_some_and(|name| name == RUN_LOCK) { continue; }
            anyhow::ensure!(!entry.file_type()?.is_symlink(), "Unexpected symbolic link in output: {}", path.display());
            if entry.metadata()?.is_dir() {
                pending.push(path);
                continue;
            }
            let relative = path.strip_prefix(root).unwrap_or(&path);
            let text = relative.components().map(|part| part.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            found.push((text, path));
        }
    }
    let paths: Vec<&Path> = found.iter().map(|(_, path)| path.as_path()).collect();
    let digests = digests(&paths, progress, stage)?;
    let mut files: Vec<RecipeFile> = found.into_iter().zip(digests).map(|((path, _), (bytes, sha256))| RecipeFile { path, bytes, sha256 }).collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

pub fn run_recipe(recipe: &Recipe, out_dir: &Path, progress: &ProgressHandle) -> Result<RecipeReport> {
    let started = Instant::now();
    let started_at_utc = utc_timestamp(SystemTime::now());
    let mut warnings = recipe.validate()?;
    check_cancelled(progress)?;
    prepare_output_dir(out_dir)?;
    let _lock = RecipeLock::acquire(out_dir, &started_at_utc)?;
    let total = recipe.steps.len() as u64;
    let half_steps = 2 * total;
    let mut stage = Stage::named(HASHING_INPUTS);
    let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<RecipeReport> {
        let initial_dependencies = dependencies(recipe, &progress.part(0, half_steps), HASHING_INPUTS)?;
        let initial_registry = serde_json::to_value(filter_registry().entries())?;
        let mut input_paths: Vec<String> = recipe.paths().into_iter().filter(|p| Path::new(p).is_file()).map(str::to_string).collect();
        input_paths.extend(recipe.empirical_files()?);
        input_paths.extend(initial_dependencies.iter().map(|d| d.path.clone()));
        for input in input_paths {
            invalidate_written(&input);
            GLOBAL_CUBE_CACHE.invalidate(&input);
        }
        write_json(&out_dir.join(RECIPE_FILE), recipe)?;
        let mut run = Run::new(recipe);
        let mut steps = Vec::with_capacity(recipe.steps.len());
        for (index, step) in recipe.steps.iter().enumerate() {
            let directory = step_folder(index, step);
            stage = Stage { name: directory.clone(), step_index: Some(index + 1) };
            check_cancelled(progress)?;
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
        stage = Stage::named(VERIFYING_INPUTS);
        let verifying = progress.part(half_steps.saturating_sub(2), half_steps);
        if dependencies(recipe, &verifying, VERIFYING_INPUTS)? != initial_dependencies || serde_json::to_value(filter_registry().entries())? != initial_registry {
            return Err(coded("input_changed", "A recipe dependency or filter registry changed during execution", Some("Keep inputs unchanged and run again in a new output folder".into())).into());
        }
        stage = Stage::named(HASHING_PRODUCTS);
        let files = files_under(out_dir, &progress.part(half_steps.saturating_sub(1), half_steps), HASHING_PRODUCTS)?;
        let recipe_sha256 = files.iter().find(|file| file.path == RECIPE_FILE).context("The saved recipe is missing")?.sha256.clone();
        let report = RecipeReport {
            recipe_version: recipe.recipe_version,
            name: recipe.name.clone(),
            output_dir: out_dir.to_string_lossy().into_owned(),
            cube: run.cube.clone(),
            steps,
            files,
            warnings: warnings.clone(),
            elapsed_ms: started.elapsed().as_millis() as u64,
            provenance: RecipeProvenance {
                schema_version: 1,
                software_version: env!("CARGO_PKG_VERSION").to_string(),
                operating_system: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                started_at_utc: started_at_utc.to_string(),
                recipe_sha256,
                dependencies: initial_dependencies,
                filter_registry: initial_registry,
                integrity: "sha256_before_after",
                limitations: vec![
                    "Input hashes are verified before and after execution, not an atomic snapshot; do not modify inputs while a run is active".into(),
                    "The saved recipe contains resolved paths; its SHA-256 identifies the exact recipe.json bytes, not a location-independent recipe".into(),
                    "SCI/ERR/DQ availability and propagation follow each step report and the selected output layout; hashes do not certify scientific accuracy".into(),
                ],
            },
        };
        stage = Stage::named(PUBLISHING);
        check_cancelled(progress)?;
        atomic_json(&out_dir.join(RESULT_FILE), &report)?;
        progress.report(total, total, "done");
        Ok(report)
    }));
    match outcome {
        Ok(Ok(report)) => Ok(report),
        Ok(Err(error)) => {
            write_failure(out_dir, &stage, &error, &warnings, progress, &started_at_utc);
            Err(error)
        }
        Err(panic) => {
            let error: anyhow::Error = coded(PANIC, format!("the run panicked: {}", panic_text(panic.as_ref())), None).into();
            write_failure(out_dir, &stage, &error, &warnings, progress, &started_at_utc);
            resume_unwind(panic)
        }
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    match (payload.downcast_ref::<&str>(), payload.downcast_ref::<String>()) {
        (Some(text), _) => text.to_string(),
        (None, Some(text)) => text.clone(),
        (None, None) => "no panic message".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::core::cubeprep::kernel::{EpsMode, KernelMethod, KernelShape, ShapeName};
    use crate::core::cubeprep::psf_io::{decode_planes_f64, image_hdus};
    use crate::core::imaging::psf_estimation::RadiusUnit;
    use crate::core::metadata::photcal::card_text;
    use crate::infra::fits::reader::{extract_header_by_index, list_extensions};
    use crate::types::config::CustomFilter;
    use crate::types::header::HduHeader;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("lightstack")
    }

    fn fits_products(folder: &Path) -> usize {
        std::fs::read_dir(folder).map_or(0, |entries| {
            entries.filter_map(Result::ok).filter(|entry| entry.path().extension().is_some_and(|ext| ext.eq_ignore_ascii_case("fits"))).count()
        })
    }

    fn extension_names(path: &Path) -> Vec<String> {
        list_extensions(&File::open(path).unwrap()).unwrap().into_iter().filter_map(|hdu| hdu.extname).collect()
    }

    fn noise_extension(path: &Path) -> (HduHeader, Vec<ndarray::Array2<f64>>) {
        let bytes = std::fs::read(path).unwrap();
        let label = path.to_string_lossy();
        let (_, parsed) = image_hdus(&bytes, &label)
            .unwrap()
            .into_iter()
            .find(|(_, parsed)| card_text(&parsed.header, "EXTNAME").as_deref() == Some("NOISCORR"))
            .unwrap_or_else(|| panic!("{} holds no NOISCORR extension", path.display()));
        let maps = decode_planes_f64(&bytes, &parsed, &label).unwrap();
        (parsed.header, maps)
    }

    fn assert_noiscorr_history_only_beside_the_extension(root: &Path, files: &[RecipeFile]) {
        for product in files.iter().filter(|file| file.path.ends_with(".fits")) {
            let file = File::open(root.join(&product.path)).unwrap();
            let extensions = list_extensions(&file).unwrap();
            let holds = extensions.iter().any(|hdu| hdu.extname.as_deref() == Some("NOISCORR"));
            for hdu in &extensions {
                let header = extract_header_by_index(&file, hdu.index).unwrap();
                let named = header.cards.iter().any(|(key, text)| key == "HISTORY" && text.contains("NOISCORR"));
                assert!(holds || !named, "{} HDU {} names NOISCORR in its HISTORY but the file holds no NOISCORR extension", product.path, hdu.index);
            }
        }
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
    fn provenance_manifest_hashes_recipe_dependencies_and_every_product() {
        use sha2::{Digest, Sha256};
        let out = tempfile::tempdir().unwrap();
        let target = out.path().join("run");
        let recipe = Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1"));
        let report = run_recipe(&recipe, &target, &quiet()).unwrap();
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["provenance"]["schema_version"], 1);
        assert_eq!(value["provenance"]["software_version"], env!("CARGO_PKG_VERSION"));
        let digest = |path: &Path| Sha256::digest(std::fs::read(path).unwrap()).iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(value["provenance"]["recipe_sha256"], digest(&target.join(RECIPE_FILE)));
        let sources = value["provenance"]["dependencies"].as_array().unwrap();
        assert_eq!(sources.len(), 12);
        for source in sources {
            assert_eq!(source["sha256"], digest(Path::new(source["path"].as_str().unwrap())));
            assert!(!source["roles"].as_array().unwrap().is_empty());
        }
        for file in value["files"].as_array().unwrap() {
            assert_eq!(file["sha256"], digest(&target.join(file["path"].as_str().unwrap())));
        }
        assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(target.join(RESULT_FILE)).unwrap()).unwrap(), value);
        assert!(!target.join("result.json.tmp").exists());
    }

    #[test]
    fn provenance_manifest_resolves_empirical_images_and_regions_instead_of_hashing_a_folder() {
        let out = tempfile::tempdir().unwrap();
        let recipe = Recipe::from_value(&fixture_recipe("d1_stars", "recipe_d1_stars.json")).unwrap().resolved(&fixtures().join("d1_stars"));
        let report = run_recipe(&recipe, &out.path().join("run"), &quiet()).unwrap();
        let value = serde_json::to_value(&report).unwrap();
        let deps = value["provenance"]["dependencies"].as_array().unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|d| d["path"].as_str().unwrap().ends_with("stars_f444w.reg")));
        assert!(deps.iter().all(|d| Path::new(d["path"].as_str().unwrap()).is_file()));
    }

    #[test]
    fn provenance_refuses_input_changes_even_when_size_and_mtime_are_preserved() {
        let out = tempfile::tempdir().unwrap();
        let source = out.path().join("source.fits");
        let mut value = d1_recipe();
        let original = fixtures().join("d1").join(value["inputs"][0]["path"].as_str().unwrap());
        std::fs::copy(original, &source).unwrap();
        value["inputs"] = json!([value["inputs"][0].clone()]);
        value["inputs"][0]["path"] = json!(source);
        value["steps"] = json!([{ "step": "units", "to": "Jy/pixel" }, { "step": "units", "to": "Jy/pixel" }]);
        let recipe = Recipe::from_value(&value).unwrap();
        let progress = ProgressHandle::with_sink(Arc::new(move |event| {
            if event.stage == "02_units" {
                let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
                let mut bytes = std::fs::read(&source).unwrap();
                *bytes.last_mut().unwrap() ^= 1;
                std::fs::write(&source, bytes).unwrap();
                std::fs::File::options().write(true).open(&source).unwrap().set_modified(modified).unwrap();
            }
        }), 2);
        let target = out.path().join("run");
        let failure = run_recipe(&recipe, &target, &progress).unwrap_err();
        assert_eq!(failure.downcast_ref::<CodedError>().unwrap().code, "input_changed");
        assert!(!target.join(RESULT_FILE).exists());
    }

    #[test]
    fn provenance_hashes_file_bytes_with_sha256_and_obeys_cancellation() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(file_digest(&path, &quiet()).unwrap(), (3, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into()));
        let flag = Arc::new(AtomicBool::new(true));
        let progress = quiet().with_cancel_source(Arc::new(move || flag.load(Ordering::SeqCst)));
        assert!(file_digest(&path, &progress).unwrap_err().downcast_ref::<AppError>().is_some());
    }

    #[test]
    fn provenance_run_invalidates_pixels_cached_before_a_same_stamp_rewrite() {
        use ndarray::Array2;
        use crate::infra::fits::writer::write_fits_mono;
        use crate::infra::fits::reader::extract_image_mmap_by_index;
        use crate::service::image_files::load_cached_full;
        use crate::types::header::HduHeader;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.fits");
        let source = path.to_str().unwrap();
        let mut header = HduHeader::empty();
        header.set("BUNIT", "Jy".into());
        write_fits_mono(source, &Array2::from_elem((3, 4), 1.0), Some(&header)).unwrap();
        let cached = load_cached_full(source).unwrap();
        assert_eq!(cached.arr()[[0, 0]], 1.0);
        let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
        write_fits_mono(source, &Array2::from_elem((3, 4), 7.0), Some(&header)).unwrap();
        File::options().write(true).open(&path).unwrap().set_modified(stamp).unwrap();
        assert_eq!(load_cached_full(source).unwrap().arr()[[0, 0]], 1.0);
        let recipe = Recipe::from_value(&json!({ "recipe_version": 1, "inputs": [{ "path": source, "dataset": "JWST", "filter": "F444W" }], "steps": [{ "step": "units", "to": "Jy/pixel" }] })).unwrap();
        let target = dir.path().join("run");
        let report = run_recipe(&recipe, &target, &quiet()).unwrap();
        let result = report.steps[0].report["files"][0]["output_path"].as_str().unwrap();
        let image = extract_image_mmap_by_index(&File::open(result).unwrap(), 1).unwrap();
        assert!(image.image.iter().all(|value| *value == 7.0));
        let written = std::fs::read(target.join(RESULT_FILE)).unwrap();
        assert!(atomic_json(&target.join(RESULT_FILE), &report).is_err());
        assert_eq!(std::fs::read(target.join(RESULT_FILE)).unwrap(), written);
    }

    #[test]
    fn provenance_cancelled_product_hashing_never_publishes_success_and_releases_the_lock() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let progress = ProgressHandle::with_sink(Arc::new(move |event| {
            if event.stage == "hashing_products" { signal.store(true, Ordering::SeqCst); }
        }), 1).with_cancel_source(Arc::new(move || cancelled.load(Ordering::SeqCst)));
        let recipe = Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1"));
        let target = dir.path().join("run");
        assert!(run_recipe(&recipe, &target, &progress).unwrap_err().downcast_ref::<AppError>().is_some());
        assert!(!target.join(RESULT_FILE).exists());
        assert!(!target.join(RUN_LOCK).exists());
        assert!(target.join(RECIPE_FILE).is_file());
    }

    #[test]
    fn the_complete_mef_recipe_keeps_science_error_quality_and_filter_products() {
        use crate::infra::fits::reader::{extract_image_planes_by_index, extract_int_planes_by_index, list_extensions};
        use crate::infra::fits::table::{load_bintable, ColumnValues};
        let dir = tempfile::tempdir().unwrap();
        let mut value = d1_recipe();
        value["steps"][3]["layout"] = json!("mef");
        value["steps"].as_array_mut().unwrap().push(json!({ "step": "export", "output": "complete.fits" }));
        let recipe = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1"));
        let target = dir.path().join("run");
        let report = run_recipe(&recipe, &target, &quiet()).unwrap();
        let mut checked = Vec::new();
        for product in report.files.iter().filter(|file| file.path.ends_with(".fits") && ["03_align/", "04_cube/", "05_edit/", "08_match/", "09_export/"].iter().any(|prefix| file.path.starts_with(prefix))) {
            let file = File::open(target.join(&product.path)).unwrap();
            let extensions = list_extensions(&file).unwrap();
            for name in ["SCI", "ERR", "DQ", "NOISCORR"] {
                assert!(extensions.iter().any(|hdu| hdu.extname.as_deref() == Some(name)), "{} lost {name}", product.path);
            }
            let folder = product.path.split('/').next().unwrap();
            let (noise, maps) = noise_extension(&target.join(&product.path));
            let k = noise.get_i64("NCORRK").unwrap();
            let source = if ["08_match", "09_export"].contains(&folder) { "match" } else { "align" };
            assert_eq!(card_text(&noise, "NCORRSRC").as_deref(), Some(source), "{}", product.path);
            assert_eq!((k % 2, noise.get_i64("NAXIS1"), noise.get_i64("NAXIS2")), (1, Some(k), Some(k)), "{}", product.path);
            let centre = (k / 2) as usize;
            assert!(maps.iter().all(|map| (map[[centre, centre]] - 1.0).abs() < 1e-6 && map.iter().all(|value| value.is_finite())), "{}", product.path);
            if folder == "03_align" {
                assert_eq!((noise.get_i64("NAXIS"), maps.len()), (Some(2), 1), "{}", product.path);
                assert!(noise.get_f64("NCORRALN").is_some_and(|factor| factor.is_finite() && factor >= 1.0 - 1e-9), "{}", product.path);
            } else {
                assert_eq!((noise.get_i64("NAXIS"), maps.len()), (Some(3), 6), "{}", product.path);
                let filters = load_bintable(target.join(&product.path).to_str().unwrap(), extensions.iter().find(|hdu| hdu.extname.as_deref() == Some("FILTERS")).unwrap().index).unwrap();
                let matched = folder == "08_match" || folder == "09_export";
                let statuses: Vec<&str> = if matched { vec!["matched", "matched", "matched", "matched", "matched", "reference"] } else { vec!["unmatched"; 6] };
                assert_eq!(filters.text_column("MATCH").unwrap(), statuses, "{}", product.path);
                let column = |name: &str| match filters.flattened_column(name).unwrap().values {
                    ColumnValues::F64(values) => values,
                    other => panic!("{name} holds {other:?}"),
                };
                assert!(column("NOISCORR_ALIGN").iter().all(|factor| factor.is_finite() && *factor >= 1.0 - 1e-9), "{}", product.path);
                let factors = column("NOISCORR_MATCH");
                assert!(factors.iter().zip(&statuses).all(|(factor, status)| if *status == "matched" { *factor > 1.0 } else { (factor - 1.0).abs() < 1e-12 }), "{}: {factors:?}", product.path);
            }
            checked.push(folder.to_string());
        }
        checked.dedup();
        assert_eq!(checked, ["03_align", "04_cube", "05_edit", "08_match", "09_export"]);
        assert_noiscorr_history_only_beside_the_extension(&target, &report.files);
        let file = File::open(report.cube.as_ref().unwrap()).unwrap();
        let extensions = list_extensions(&file).unwrap();
        let index = |name: &str| extensions.iter().find(|hdu| hdu.extname.as_deref() == Some(name)).unwrap().index;
        assert!(extensions.iter().any(|hdu| hdu.extname.as_deref() == Some("FILTERS")));
        let sci = extract_image_planes_by_index(&file, index("SCI")).unwrap();
        let err = extract_image_planes_by_index(&file, index("ERR")).unwrap();
        let dq = extract_int_planes_by_index(&file, index("DQ")).unwrap();
        assert_eq!((sci.len(), err.len(), dq.len()), (6, 6, 6));
        assert_eq!(sci[0].dim(), (44, 43));
        for ((sci, err), dq) in sci.iter().zip(&err).zip(&dq) {
            assert_eq!(sci.dim(), err.dim());
            assert_eq!(sci.dim(), dq.bits.dim());
            assert!(err.iter().any(|value| value.is_finite() && *value > 0.0));
            assert!(err.iter().all(|value| value.is_nan() || *value >= 0.0));
        }
        assert_eq!(extract_header_by_index(&file, index("SCI")).unwrap().get("BUNIT"), extract_header_by_index(&file, index("ERR")).unwrap().get("BUNIT"));
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
        let RecipeStep::Psfs(PsfsStep { source: PsfSource::Empirical(empirical), reference: None, pixel_scale_arcsec: None, recentre: None }) =
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
            (6, json!({ "step": "kernels", "reference": "F444W", "method": "wiener", "shape": [20, 21] }), "steps[6] (kernels): shape must be reference, max or two odd integers"),
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
        assert_eq!(std::fs::read_dir(target.join("09_export")).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode(Path::new(cube)), mode(Path::new(matched)), "the exported cube keeps the mode of every other product");
            assert_eq!(mode(&target.join(RESULT_FILE)), mode(&target.join(RECIPE_FILE)), "result.json keeps the mode of recipe.json");
        }
        assert!(report.files.iter().all(|file| !file.path.ends_with(".partial")), "{:?}", report.files);
        let header = extract_header_by_index(&std::fs::File::open(cube).unwrap(), 0).unwrap();
        assert_eq!(header.get_i64("NAXIS3"), Some(6));
        let aligned: Vec<&RecipeFile> = report.files.iter().filter(|file| file.path.starts_with("03_align/")).collect();
        assert_eq!(aligned.len(), 6);
        for file in aligned {
            let (noise, maps) = noise_extension(&target.join(&file.path));
            assert_eq!((card_text(&noise, "NCORRSRC").as_deref(), noise.get_i64("NAXIS"), maps.len()), (Some("align"), Some(2), 1), "{}", file.path);
        }
        for folder in ["04_cube", "05_edit", "08_match", "09_export"] {
            for entry in std::fs::read_dir(target.join(folder)).unwrap() {
                let path = entry.unwrap().path();
                assert_eq!(extension_names(&path), Vec::<String>::new(), "{}: the lightstack layout holds the primary cube only", path.display());
            }
        }
        assert_eq!(report.steps[3].report["hdus"], json!(["PRIMARY"]));
        assert_noiscorr_history_only_beside_the_extension(&target, &report.files);
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
            json!({ "step": "psfs", "source": "files", "files": [{ "filter": "F444W", "path": "p.fits" }], "pixel_scale_arcsec": 0.04, "reference": "F444W", "recentre": false }),
            json!({
                "step": "psfs", "source": "empirical", "dataset": "JWST", "filters": ["F444W"], "data_dir": "d", "cutout_size": 31,
                "background_border": 5, "background_sigma": 4.0, "clip_negative": true, "noise_threshold": 0.5, "radial_cleaning": true,
                "cleaning_sigma": 3.0, "cleaning_consecutive_bins": 2, "normalization_radius": 0.5, "normalization_radius_unit": "arcsec",
                "psf_size": 21, "oversampling": 2, "stack": "median", "oversampling_order": 3
            }),
            json!({ "step": "kernels", "reference": "F444W", "method": "wiener", "eps": 0.01, "eps_mode": "relative", "shape": [21, 21], "auto": false }),
            json!({ "step": "kernels", "reference": "F444W", "method": "wiener", "shape": "max" }),
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

    fn read_value(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))).unwrap()
    }

    fn scientific_profile_recipe() -> Value {
        let mut value = d1_recipe();
        value["steps"][0]["rounding"] = json!("astroburst");
        value["steps"][2]["area_rule"] = json!("scale");
        value["steps"][3]["layout"] = json!("mef");
        value["steps"][4]["use_dq"] = json!(true);
        value["steps"][7]["mode"] = json!("astroburst");
        value["steps"].as_array_mut().unwrap().push(json!({ "step": "export", "output": "prepared_cube.fits" }));
        value
    }

    #[test]
    fn a_failed_step_leaves_a_failure_marker_naming_the_step_code_and_hint() {
        let mut value = d1_recipe();
        value["steps"][5]["files"].as_array_mut().unwrap().remove(1);
        let recipe = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1"));
        let out = tempfile::tempdir().unwrap();
        let target = out.path().join("run");
        let error = run_recipe(&recipe, &target, &quiet()).unwrap_err();
        let marker = read_value(&target.join(FAILURE_FILE));
        assert_eq!(marker["schema_version"], 1);
        assert_eq!(marker["status"], "failed");
        assert_eq!(marker["stage"], "08_match");
        assert_eq!(marker["step_index"], 8);
        assert_eq!(marker["code"], "kernel_missing");
        assert_eq!(marker["message"], json!(format!("{error:#}")));
        assert!(marker["hint"].as_str().unwrap().contains("allow_unmatched"), "{marker}");
        assert_eq!(marker["published"], false);
        assert_eq!(marker["software_version"], env!("CARGO_PKG_VERSION"));
        assert!(marker["started_at_utc"].as_str().unwrap().ends_with('Z') && marker["ended_at_utc"].as_str().unwrap().ends_with('Z'), "{marker}");
        assert!(!target.join(RESULT_FILE).exists());
        assert!(!target.join(RUN_LOCK).exists());
        assert!(target.join("07_kernels").is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode(&target.join(FAILURE_FILE)), mode(&target.join(RECIPE_FILE)), "failure.json keeps the mode of recipe.json");
        }
    }

    #[test]
    fn a_cancelled_product_hashing_leaves_a_cancelled_failure_marker() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let progress = ProgressHandle::with_sink(Arc::new(move |event| {
            if event.stage == "hashing_products" { signal.store(true, Ordering::SeqCst); }
        }), 1).with_cancel_source(Arc::new(move || cancelled.load(Ordering::SeqCst)));
        let recipe = Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1"));
        let target = dir.path().join("run");
        let error = run_recipe(&recipe, &target, &progress).unwrap_err();
        assert!(error.downcast_ref::<AppError>().is_some(), "{error:#}");
        let marker = read_value(&target.join(FAILURE_FILE));
        assert_eq!(marker["status"], "cancelled");
        assert_eq!(marker["stage"], "hashing_products");
        assert_eq!(marker["step_index"], Value::Null);
        assert_eq!(marker["code"], "cancelled");
        assert!(marker["message"].as_str().unwrap().to_lowercase().contains("cancel"), "{marker}");
        assert_eq!(marker["hint"], Value::Null);
        assert_eq!(marker["published"], false);
        assert!(!target.join(RESULT_FILE).exists());
        assert!(!target.join(RUN_LOCK).exists());
    }

    #[test]
    fn a_successful_run_writes_no_failure_marker_and_a_refused_folder_stays_untouched() {
        let recipe = Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1"));
        let out = tempfile::tempdir().unwrap();
        std::fs::write(out.path().join("keep.txt"), "x").unwrap();
        run_recipe(&recipe, out.path(), &quiet()).unwrap_err();
        let names: Vec<String> = std::fs::read_dir(out.path()).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["keep.txt"]);
        let target = out.path().join("run");
        let report = run_recipe(&recipe, &target, &quiet()).unwrap();
        assert!(!target.join(FAILURE_FILE).exists());
        assert!(target.join(RESULT_FILE).is_file());
        assert!(report.files.iter().all(|file| file.path != FAILURE_FILE && file.path != RUN_LOCK));
    }

    #[test]
    fn the_lock_names_the_process_and_the_start_of_the_run_while_it_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("run");
        let seen = Arc::new(std::sync::Mutex::new(None));
        let lock_path = target.join(RUN_LOCK);
        let sink_seen = seen.clone();
        let progress = ProgressHandle::with_sink(Arc::new(move |event| {
            if event.stage == "01_crop" {
                *sink_seen.lock().unwrap() = Some(std::fs::read_to_string(&lock_path).unwrap_or_default());
            }
        }), 1);
        let recipe = Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1"));
        let report = run_recipe(&recipe, &target, &progress).unwrap();
        let held = seen.lock().unwrap().clone().expect("the sink saw 01_crop");
        let lock: Value = serde_json::from_str(&held).unwrap_or_else(|error| panic!("{held:?}: {error}"));
        assert_eq!(lock["pid"], std::process::id());
        assert_eq!(lock["started_at_utc"], json!(report.provenance.started_at_utc));
        assert_eq!(lock["software_version"], env!("CARGO_PKG_VERSION"));
        assert!(!target.join(RUN_LOCK).exists());
    }

    #[test]
    fn resolved_inputs_and_dependency_paths_carry_no_verbatim_prefix() {
        let base = std::fs::canonicalize(fixtures().join("d1")).unwrap();
        let recipe = Recipe::from_value(&d1_recipe()).unwrap().resolved(&base);
        for path in recipe.paths() {
            assert!(!path.starts_with(r"\\?\"), "{path}");
            assert!(Path::new(path).is_file(), "{path}");
        }
        let dependencies = dependency_paths(&recipe).unwrap();
        assert_eq!(dependencies.len(), 12);
        for path in dependencies.keys() {
            assert!(!path.starts_with(r"\\?\"), "{path}");
            assert!(Path::new(path).is_file(), "{path}");
        }
        let mut value = d1_recipe();
        value["inputs"][0]["path"] = json!("../d1/d1_HST_F814W.fits");
        let recipe = Recipe::from_value(&value).unwrap().resolved(&base);
        let resolved = &recipe.inputs[0].path;
        assert!(!resolved.starts_with(r"\\?\"), "{resolved}");
        if cfg!(windows) {
            assert!(!resolved.contains(".."), "joining onto the verbatim folder removes the parent step: {resolved}");
        }
        assert_eq!(std::fs::canonicalize(resolved).unwrap(), base.join("d1_HST_F814W.fits"));
    }

    #[test]
    fn parallel_and_serial_dependency_manifests_are_identical() {
        let recipe = Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1"));
        let pooled = dependencies(&recipe, &quiet(), HASHING_INPUTS).unwrap();
        let serial: Vec<RecipeDependency> = dependency_paths(&recipe)
            .unwrap()
            .into_iter()
            .map(|(path, roles)| {
                let (bytes, sha256) = file_digest(Path::new(&path), &quiet()).unwrap();
                RecipeDependency { path, roles, bytes, sha256 }
            })
            .collect();
        assert_eq!(pooled.len(), 12);
        assert_eq!(pooled, serial);
        let out = tempfile::tempdir().unwrap();
        let target = out.path().join("run");
        let report = run_recipe(&recipe, &target, &quiet()).unwrap();
        let products = files_under(&target, &quiet(), HASHING_PRODUCTS).unwrap();
        let serial: Vec<RecipeFile> = products
            .iter()
            .map(|file| {
                let (bytes, sha256) = file_digest(&target.join(&file.path), &quiet()).unwrap();
                RecipeFile { path: file.path.clone(), bytes, sha256 }
            })
            .collect();
        assert_eq!(products, serial);
        assert!(products.windows(2).all(|pair| pair[0].path < pair[1].path));
        assert_eq!(products.iter().filter(|file| file.path != RESULT_FILE).cloned().collect::<Vec<_>>(), report.files);
    }

    const SCIENTIFIC_PLANES: usize = 6;
    const SCIENTIFIC_PLANE_SHAPE: (usize, usize) = (43, 43);
    const SCIENTIFIC_MATCH_MASKED_PIXELS: [u64; SCIENTIFIC_PLANES] = [1, 10, 10, 8, 2, 8];
    const SCIENTIFIC_FINITE_PIXELS_PER_PLANE: [usize; SCIENTIFIC_PLANES] = [1835; SCIENTIFIC_PLANES];
    const SCIENTIFIC_FINITE_IN_EVERY_PLANE: usize = 1835;
    const SCIENTIFIC_REFERENCE_UNION_WARNING: &str = "F444W: 8 pixels finite in the reference are NaN in another plane";
    const SCIENTIFIC_HDUS: [&str; 5] = ["SCI", "ERR", "DQ", "NOISCORR", "FILTERS"];
    const SCIENTIFIC_FILTERS_COLUMNS: [&str; 12] = [
        "FILTER", "DATASET", "INSTRUME", "PIVOT_UM", "PIXSCALE", "BUNIT_IN", "FLUXCONV", "PSF_FWHM", "KERNEL", "NOISCORR_ALIGN", "NOISCORR_MATCH", "MATCH",
    ];
    const SCIENTIFIC_FILTERS: [&str; SCIENTIFIC_PLANES] = ["F814W", "F150W", "F200W", "F277W", "F356W", "F444W"];
    const SCIENTIFIC_DATASETS: [&str; SCIENTIFIC_PLANES] = ["HST", "JWST", "JWST", "JWST", "JWST", "JWST"];
    const SCIENTIFIC_PIVOT_UM: [f64; SCIENTIFIC_PLANES] = [0.804553, 1.500744, 1.988648, 2.76174, 3.568362, 4.404315];
    const SCIENTIFIC_PSF_FWHM_ARCSEC: [f64; SCIENTIFIC_PLANES] = [0.15369, 0.14046, 0.14152, 0.14502, 0.16839, 0.13884];
    const SCIENTIFIC_PSF_FWHM_TOLERANCE_ARCSEC: f64 = 5e-6;
    const SCIENTIFIC_MATCH: [&str; SCIENTIFIC_PLANES] = ["matched", "matched", "matched", "matched", "matched", "reference"];
    const SCIENTIFIC_KERNEL_HALF_WIDTH: usize = 10;
    const SCIENTIFIC_KERNELS: [&str; SCIENTIFIC_PLANES] = [
        "kernel_f814w_to_f444w.fits",
        "kernel_f150w_to_f444w.fits",
        "kernel_f200w_to_f444w.fits",
        "kernel_f277w_to_f444w.fits",
        "kernel_f356w_to_f444w.fits",
        "",
    ];

    #[test]
    fn the_scientific_profile_recipe_wires_its_options_into_the_43x43_mef_cube() {
        use crate::infra::fits::reader::{extract_image_planes_by_index, extract_int_planes_by_index, list_extensions};
        use crate::infra::fits::table::{list_tables, load_bintable, ColumnValues};
        let dir = tempfile::tempdir().unwrap();
        let recipe = Recipe::from_value(&scientific_profile_recipe()).unwrap().resolved(&fixtures().join("d1"));
        let target = dir.path().join("run");
        let report = run_recipe(&recipe, &target, &quiet()).unwrap();
        let rects: Vec<(i64, i64, u64, u64)> = report.steps[0].report["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| {
                let rect = &file["rect"];
                (rect["x0"].as_i64().unwrap(), rect["y0"].as_i64().unwrap(), rect["width"].as_u64().unwrap(), rect["height"].as_u64().unwrap())
            })
            .collect();
        assert_eq!(rects, [(5, 5, 53, 53), (7, 6, 115, 115), (7, 6, 115, 115), (9, 9, 45, 45), (9, 8, 47, 47), (9, 9, 45, 45)]);
        assert_eq!(report.steps[0].report["rounding"], json!("astroburst"));
        let rules: Vec<Option<&str>> = report.steps[2].report["files"].as_array().unwrap().iter().map(|file| file["area"]["rule"].as_str()).collect();
        assert_eq!(rules, [Some("scale"), Some("scale"), Some("scale"), None, Some("scale"), None]);
        assert_eq!(report.steps[4].parameters["use_dq"], json!(true));
        assert_eq!(report.steps[4].report["removed"], json!([]));
        assert_eq!(report.steps[7].report["mode"], json!("astroburst"));
        let planes = report.steps[7].report["planes"].as_array().unwrap();
        let masked: Vec<u64> = planes.iter().map(|plane| plane["masked_pixels"].as_u64().unwrap()).collect();
        assert_eq!(masked, SCIENTIFIC_MATCH_MASKED_PIXELS);
        assert_eq!((planes[5]["filter"].as_str(), planes[5]["status"].as_str()), (Some("F444W"), Some("reference")));
        assert!(report.steps[7].warnings.iter().any(|warning| warning.starts_with(SCIENTIFIC_REFERENCE_UNION_WARNING)), "{:?}", report.steps[7].warnings);
        assert_eq!(report.steps[7].report["hdus"], json!(SCIENTIFIC_HDUS));
        let history = |path: &str| {
            let header = extract_header_by_index(&File::open(path).unwrap(), 1).unwrap();
            header.cards.iter().filter(|(key, _)| key == "HISTORY").map(|(_, text)| text.as_str()).collect::<Vec<_>>().join(" ")
        };
        assert!(history(report.steps[4].report["output_path"].as_str().unwrap()).contains("DO_NOT_USE"));
        let matched_history = history(report.steps[7].report["output_path"].as_str().unwrap());
        assert!(matched_history.contains("union NaN mask") && matched_history.contains("the reference included"), "{matched_history}");
        let cube_path = report.cube.as_deref().unwrap();
        let cube = File::open(cube_path).unwrap();
        let extensions = list_extensions(&cube).unwrap();
        let index = |name: &str| extensions.iter().find(|hdu| hdu.extname.as_deref() == Some(name)).unwrap_or_else(|| panic!("no {name}")).index;
        let (filters_hdu, _) = list_tables(&cube).unwrap().into_iter().find(|(table, _)| table.index == index("FILTERS")).unwrap();
        let columns: Vec<String> = filters_hdu.columns.into_iter().map(|column| column.name).collect();
        assert_eq!(columns, SCIENTIFIC_FILTERS_COLUMNS);
        let filters = load_bintable(cube_path, index("FILTERS")).unwrap();
        assert_eq!(filters.text_column("FILTER").unwrap(), SCIENTIFIC_FILTERS);
        assert_eq!(filters.text_column("DATASET").unwrap(), SCIENTIFIC_DATASETS);
        assert_eq!(filters.text_column("KERNEL").unwrap(), SCIENTIFIC_KERNELS);
        assert_eq!(filters.text_column("MATCH").unwrap(), SCIENTIFIC_MATCH);
        let numbers = |name: &str| match filters.flattened_column(name).unwrap().values {
            ColumnValues::F64(values) => values,
            other => panic!("{name} holds {other:?}"),
        };
        let pivots = numbers("PIVOT_UM");
        assert_eq!(pivots.len(), SCIENTIFIC_PLANES);
        assert!(pivots.iter().zip(SCIENTIFIC_PIVOT_UM).all(|(found, pinned)| (found - pinned).abs() <= 1e-12 * pinned), "{pivots:?}");
        let fwhm = numbers("PSF_FWHM");
        assert!(fwhm.iter().zip(SCIENTIFIC_PSF_FWHM_ARCSEC).all(|(found, pinned)| (found - pinned).abs() <= SCIENTIFIC_PSF_FWHM_TOLERANCE_ARCSEC), "{fwhm:?}");
        let kernel_squares: Vec<Option<f64>> = SCIENTIFIC_KERNELS
            .iter()
            .map(|name| (!name.is_empty()).then(|| extract_header_by_index(&File::open(target.join("07_kernels").join(name)).unwrap(), 0).unwrap().get_f64("KSUMSQ").unwrap()))
            .collect();
        let noiscorr_match = numbers("NOISCORR_MATCH");
        let expected_match: Vec<f64> = kernel_squares.iter().map(|squares| squares.map_or(1.0, |squares| 1.0 / squares.sqrt())).collect();
        assert!(noiscorr_match.iter().zip(&expected_match).all(|(found, expected)| (found / expected - 1.0).abs() < 1e-12), "{noiscorr_match:?} against {expected_match:?}");
        let aligned: Vec<(HduHeader, Vec<ndarray::Array2<f64>>)> = report.steps[2].report["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| noise_extension(Path::new(file["output_path"].as_str().unwrap())))
            .collect();
        let factors: Vec<f64> = aligned.iter().map(|(header, _)| header.get_f64("NCORRALN").unwrap()).collect();
        let noiscorr_align = numbers("NOISCORR_ALIGN");
        assert!(noiscorr_align.iter().zip(&factors).all(|(found, expected)| (found / expected - 1.0).abs() < 1e-12), "{noiscorr_align:?} against {factors:?}");
        assert!((noiscorr_align[5] - 1.0).abs() < 1e-12, "the reference is copied onto its own grid: {noiscorr_align:?}");
        let (noise, maps) = noise_extension(Path::new(cube_path));
        let k = noise.get_i64("NCORRK").unwrap() as usize;
        assert_eq!((card_text(&noise, "NCORRSRC").as_deref(), maps.len(), k % 2), (Some("match"), SCIENTIFIC_PLANES, 1));
        let centre = k / 2;
        let support = |map: &ndarray::Array2<f64>| 2 * map.indexed_iter().filter(|(_, value)| **value != 0.0).map(|((y, x), _)| y.abs_diff(centre).max(x.abs_diff(centre))).max().unwrap() + 1;
        let aligned_sizes: Vec<usize> = aligned.iter().map(|(header, _)| header.get_i64("NCORRK").unwrap() as usize).collect();
        let supports: Vec<usize> = maps.iter().map(support).collect();
        let expected_supports: Vec<usize> =
            aligned_sizes.iter().zip(SCIENTIFIC_MATCH).map(|(size, status)| if status == "matched" { size + 4 * SCIENTIFIC_KERNEL_HALF_WIDTH } else { *size }).collect();
        assert_eq!(supports, expected_supports, "aligned map sizes {aligned_sizes:?}");
        assert_eq!(k, *expected_supports.iter().max().unwrap());
        assert!(maps.iter().all(|map| map.dim() == (k, k) && (map[[centre, centre]] - 1.0).abs() < 1e-6 && map.iter().all(|value| value.is_finite())));
        let reference = &maps[5];
        assert!(reference.indexed_iter().all(|((y, x), value)| (y, x) == (centre, centre) || value.abs() < 1e-12), "the reference keeps its own aligned map");
        let sci = extract_image_planes_by_index(&cube, index("SCI")).unwrap();
        let err = extract_image_planes_by_index(&cube, index("ERR")).unwrap();
        let dq = extract_int_planes_by_index(&cube, index("DQ")).unwrap();
        assert_eq!((sci.len(), err.len(), dq.len()), (SCIENTIFIC_PLANES, SCIENTIFIC_PLANES, SCIENTIFIC_PLANES));
        let finite: Vec<usize> = sci.iter().map(|plane| plane.iter().filter(|value| value.is_finite()).count()).collect();
        assert_eq!(finite, SCIENTIFIC_FINITE_PIXELS_PER_PLANE);
        let (rows, cols) = SCIENTIFIC_PLANE_SHAPE;
        let finite_everywhere = (0..rows * cols).filter(|pixel| sci.iter().all(|plane| plane[(pixel / cols, pixel % cols)].is_finite())).count();
        assert_eq!(finite_everywhere, SCIENTIFIC_FINITE_IN_EVERY_PLANE);
        for ((sci, err), dq) in sci.iter().zip(&err).zip(&dq) {
            assert_eq!(sci.dim(), SCIENTIFIC_PLANE_SHAPE);
            assert_eq!(err.dim(), SCIENTIFIC_PLANE_SHAPE);
            assert_eq!(dq.bits.dim(), SCIENTIFIC_PLANE_SHAPE);
            assert!(sci.iter().zip(err.iter()).all(|(sci, err)| sci.is_nan() == err.is_nan()));
            assert!(sci.iter().zip(dq.bits.iter()).all(|(sci, bits)| sci.is_finite() || *bits != 0));
        }
    }

    #[test]
    fn the_d1_recipe_writes_the_same_products_on_one_thread_and_on_eight() {
        let out = tempfile::tempdir().unwrap();
        let mut value = d1_recipe();
        value["steps"].as_array_mut().unwrap().push(json!({ "step": "export", "output": "d1_final.fits" }));
        let recipe = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1"));
        let run = |threads: usize| {
            let target = out.path().join(format!("threads_{threads}"));
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
            pool.install(|| run_recipe(&recipe, &target, &quiet())).unwrap()
        };
        let one = run(1);
        let eight = run(8);
        assert_eq!(one.files.len(), 34);
        assert_eq!(one.files, eight.files);
        assert_eq!(one.warnings, eight.warnings);
    }

    #[test]
    fn the_saved_form_of_the_d1_recipe_reads_back_as_the_same_recipe() {
        let saved = read_value(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("cubeprep").join("recipe_d1_saved.json"));
        let base = fixtures().join("d1");
        let from_saved = Recipe::from_value(&saved).unwrap().resolved(&base);
        let from_source = Recipe::from_value(&d1_recipe()).unwrap().resolved(&base);
        assert_eq!(from_saved, from_source);
        from_saved.validate().unwrap();
        for (index, key) in [(0, "rounding"), (2, "area_rule"), (3, "layout"), (4, "use_dq"), (6, "eps_mode"), (6, "auto"), (7, "mode"), (7, "allow_unmatched")] {
            assert!(saved["steps"][index].get(key).is_some(), "steps[{index}].{key}");
            assert!(d1_recipe()["steps"][index].get(key).is_none(), "steps[{index}].{key}");
        }
    }

    #[test]
    fn a_cut_before_the_match_is_accepted_with_a_warning_about_zero_filled_edges() {
        let cut = json!({ "step": "edit", "action": "cut", "x_start": 2, "x_end": 40, "y_start": 2, "y_end": 40 });
        let mut before = d1_recipe();
        before["steps"].as_array_mut().unwrap().insert(5, cut.clone());
        let recipe = Recipe::from_value(&before).unwrap().resolved(&fixtures().join("d1"));
        let expected = "09_match: 06_edit cuts the cube before the match; pixels within half a kernel width of the cut edges are matched against zeros; place the cut after match to use the cut-away data";
        assert_eq!(recipe.validate().unwrap(), [expected]);
        let out = tempfile::tempdir().unwrap();
        let report = run_recipe(&recipe, &out.path().join("run"), &quiet()).unwrap();
        assert_eq!(report.warnings.first().map(String::as_str), Some(expected));
        assert_eq!(report.steps[8].directory, "09_match");
        assert_eq!(report.steps[5].directory, "06_edit");
        let mut fixed = before.clone();
        fixed["steps"][7]["shape"] = json!([21, 21]);
        let warnings = Recipe::from_value(&fixed).unwrap().resolved(&fixtures().join("d1")).validate().unwrap();
        assert!(warnings[0].contains("pixels within 10 px of the cut edges"), "{warnings:?}");
        let mut sky = d1_recipe();
        sky["steps"].as_array_mut().unwrap().insert(4, json!({ "step": "edit", "action": "cut_sky", "ra": 53.1625, "dec": -27.7914, "size_arcsec": 1.0 }));
        let warnings = Recipe::from_value(&sky).unwrap().resolved(&fixtures().join("d1")).validate().unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].starts_with("09_match: 05_edit cuts the cube before the match"), "{warnings:?}");
        let mut after = d1_recipe();
        after["steps"].as_array_mut().unwrap().push(cut);
        assert!(Recipe::from_value(&after).unwrap().resolved(&fixtures().join("d1")).validate().unwrap().is_empty());
        assert!(Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1")).validate().unwrap().is_empty());
    }

    const SKIPPED_JWST_WARNING: &str = "custom filter dataset JWST is not used: JWST is a built-in dataset; rename it in filter_datasets to keep its filters";

    fn skipped_jwst() -> (FilterRegistry, Vec<String>) {
        let filter = CustomFilter { pivot_angstrom: 9990.0, instrument: "X".into(), group: "X".into(), aliases: vec![] };
        let datasets = BTreeMap::from([("JWST".to_string(), BTreeMap::from([("F999X".to_string(), filter)]))]);
        FilterRegistry::with_custom_datasets(&datasets)
    }

    fn d1_input_as(dataset: &str, filter: &str) -> Recipe {
        let mut value = d1_recipe();
        value["inputs"][0]["dataset"] = json!(dataset);
        value["inputs"][0]["filter"] = json!(filter);
        Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1"))
    }

    #[test]
    fn the_fixture_of_the_skipped_dataset_is_what_the_registry_reports() {
        let (registry, skipped) = skipped_jwst();
        assert_eq!(skipped, [SKIPPED_JWST_WARNING]);
        assert!(registry.entry("JWST", "F444W").is_some());
    }

    #[test]
    fn a_skipped_custom_dataset_is_told_with_a_valid_recipe() {
        let (registry, skipped) = skipped_jwst();
        let d1 = Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1"));
        assert_eq!(d1.validate_with(&registry, &skipped).unwrap(), [SKIPPED_JWST_WARNING]);
        assert!(d1.validate_with(&registry, &[]).unwrap().is_empty());
        let mut before = d1_recipe();
        before["steps"].as_array_mut().unwrap().insert(5, json!({ "step": "edit", "action": "cut", "x_start": 2, "x_end": 40, "y_start": 2, "y_end": 40 }));
        let cut = Recipe::from_value(&before).unwrap().resolved(&fixtures().join("d1"));
        let advisory = "09_match: 06_edit cuts the cube before the match; pixels within half a kernel width of the cut edges are matched against zeros; place the cut after match to use the cut-away data";
        assert_eq!(cut.validate_with(&registry, &skipped).unwrap(), [advisory, SKIPPED_JWST_WARNING]);
        assert_eq!(cut.validate_with(&registry, &[]).unwrap(), [advisory]);
    }

    #[test]
    fn a_registry_refusal_carries_the_skipped_dataset_in_its_hint() {
        let (registry, skipped) = skipped_jwst();
        let tail = format!("; {SKIPPED_JWST_WARNING}");
        let known = format!("known datasets: {}", registry.datasets().join(", "));

        let unknown = d1_input_as("HTS", "F814W");
        let error = unknown.validate_with(&registry, &skipped).unwrap_err();
        assert_eq!((error.code, error.message.as_str()), (RECIPE_INVALID, "inputs[0]: dataset HTS is not in the filter registry"));
        assert_eq!(error.hint, Some(format!("{known}{tail}")));
        assert_eq!(unknown.validate_with(&registry, &[]).unwrap_err().hint, Some(known));

        let missing = d1_input_as("HST", "F20W");
        let error = missing.validate_with(&registry, &skipped).unwrap_err();
        assert_eq!((error.code, error.message.as_str()), (RECIPE_INVALID, "inputs[0]: the filter registry has no HST filter F20W"));
        assert_eq!(error.hint, Some(format!("choose a filter from the dataset's filter registry{tail}")));
        assert_eq!(missing.validate_with(&registry, &[]).unwrap_err().hint.as_deref(), Some("choose a filter from the dataset's filter registry"));

        let alias = d1_input_as("HST", "F275WU");
        let error = alias.validate_with(&registry, &skipped).unwrap_err();
        assert_eq!(error.hint.as_deref(), Some("use F275W consistently in inputs and step references"));
        assert_eq!(error, alias.validate_with(&registry, &[]).unwrap_err());
    }

    #[test]
    fn validate_reads_the_registry_report() {
        let recipe = Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1"));
        let (registry, skipped) = filter_registry_report();
        assert_eq!(recipe.validate(), recipe.validate_with(&registry, &skipped));
    }

    #[test]
    fn cut_sky_defaults_to_astroburst_rounding_while_crop_defaults_to_lightstack() {
        let cut = parse_step(0, &json!({ "step": "edit", "action": "cut_sky", "ra": 53.1625, "dec": -27.7914, "size_arcsec": 1.0 })).unwrap();
        assert_eq!(cut, RecipeStep::Edit(CubeEdit::CutSky { ra: 53.1625, dec: -27.7914, size_arcsec: 1.0, rounding: CropRounding::Astroburst }));
        let crop = parse_step(0, &json!({ "step": "crop", "mode": "radec", "ra": 53.1625, "dec": -27.7914, "size_arcsec": 1.0 })).unwrap();
        assert_eq!(crop, RecipeStep::Crop(CropStep { region: CropMode::Radec { ra: 53.1625, dec: -27.7914, size_arcsec: 1.0 }, rounding: CropRounding::Lightstack }));
    }

    #[test]
    fn a_panic_inside_a_step_leaves_a_failure_marker_and_keeps_unwinding() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("run");
        let progress = ProgressHandle::with_sink(Arc::new(|event| {
            if event.stage == "03_align" {
                panic!("probe panic in 03_align");
            }
        }), 1);
        let recipe = Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1"));
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_recipe(&recipe, &target, &progress)));
        let payload = unwound.expect_err("the panic reaches the caller");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"probe panic in 03_align"));
        let marker = read_value(&target.join(FAILURE_FILE));
        assert_eq!(marker["status"], "failed");
        assert_eq!(marker["stage"], "03_align");
        assert_eq!(marker["step_index"], 3);
        assert_eq!(marker["code"], "panic");
        assert!(marker["message"].as_str().unwrap().contains("probe panic in 03_align"), "{marker}");
        assert_eq!(marker["hint"], Value::Null);
        assert_eq!(marker["published"], false);
        assert!(!target.join(RESULT_FILE).exists());
        assert!(!target.join(RUN_LOCK).exists());
        assert!(target.join("02_units").is_dir());
    }

    #[test]
    fn the_failure_code_follows_the_error_and_the_status_follows_the_cancel_request() {
        let stage = Stage { name: "03_align".to_string(), step_index: Some(3) };
        let marker = |error: anyhow::Error, progress: &ProgressHandle| {
            let dir = tempfile::tempdir().unwrap();
            write_failure(dir.path(), &stage, &error, &[], progress, "2026-11-06T00:00:00Z");
            let marker = read_value(&dir.path().join(FAILURE_FILE));
            (marker["status"].as_str().unwrap().to_string(), marker["code"].as_str().unwrap().to_string())
        };
        let cancel_requested = quiet().with_cancel_source(Arc::new(|| true));
        let pair = |status: &str, code: &str| (status.to_string(), code.to_string());
        assert_eq!(marker(anyhow::anyhow!("disk full"), &cancel_requested), pair("cancelled", "cubeprep_failed"));
        assert_eq!(marker(anyhow::anyhow!("disk full"), &quiet()), pair("failed", "cubeprep_failed"));
        assert_eq!(marker(AppError::Cancelled.into(), &quiet()), pair("cancelled", "cancelled"));
        assert_eq!(marker(coded("kernel_missing", "no kernel", None).into(), &cancel_requested), pair("cancelled", "kernel_missing"));
    }

    #[test]
    fn hashing_stops_taking_new_files_once_one_fails() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let mut paths = vec![dir.path().join("missing.fits")];
        for index in 0..40 {
            let path = dir.path().join(format!("{index:02}.bin"));
            std::fs::write(&path, b"x").unwrap();
            paths.push(path);
        }
        let polls = Arc::new(AtomicUsize::new(0));
        let counted = polls.clone();
        let progress = quiet().with_cancel_source(Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(20));
            false
        }));
        let listed: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
        let error = digests(&listed, &progress, HASHING_INPUTS).unwrap_err();
        assert!(format!("{error:#}").contains("missing.fits"), "{error:#}");
        let polled = polls.load(Ordering::SeqCst);
        assert!(polled < paths.len(), "{polled} cancel polls: the workers kept hashing after the first error");
    }

    #[test]
    fn hashing_reports_the_files_done_within_the_first_and_the_last_step() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink_seen = seen.clone();
        let progress = ProgressHandle::with_sink(Arc::new(move |event| {
            sink_seen.lock().unwrap().push((event.stage.clone(), event.current, event.total, event.percent));
        }), 1);
        let recipe = Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1"));
        let out = tempfile::tempdir().unwrap();
        let report = run_recipe(&recipe, &out.path().join("run"), &progress).unwrap();
        let steps = recipe.steps.len() as u32;
        let products = report.files.len() as u64;
        let seen = seen.lock().unwrap();
        let last = |stage: &str| seen.iter().filter(|event| event.0 == stage).last().cloned().unwrap_or_else(|| panic!("no {stage} event in {seen:?}"));
        assert_eq!(seen.first().cloned(), Some(("hashing_inputs".to_string(), 0, 12, 0)));
        assert_eq!(last("hashing_inputs"), ("hashing_inputs".to_string(), 12, 12, 100 / (2 * steps)));
        assert!(seen.iter().filter(|event| event.0 == "hashing_inputs").all(|event| event.3 <= 100 / (2 * steps)), "{seen:?}");
        assert_eq!(last("verifying_inputs"), ("verifying_inputs".to_string(), 12, 12, (2 * steps - 1) * 100 / (2 * steps)));
        assert_eq!(last("hashing_products"), ("hashing_products".to_string(), products, products, 100));
        let closing: Vec<u32> = seen.iter().skip_while(|event| event.0 != "08_match").map(|event| event.3).collect();
        assert!(closing.windows(2).all(|pair| pair[0] <= pair[1]), "the percent goes back after 08_match: {closing:?}");
        assert_eq!(seen.last().cloned(), Some(("done".to_string(), u64::from(steps), u64::from(steps), 100)));
        let folders: Vec<&str> = seen.iter().map(|event| event.0.as_str()).filter(|stage| stage.as_bytes()[0].is_ascii_digit()).collect();
        assert_eq!(folders, ["01_crop", "02_units", "03_align", "04_cube", "05_edit", "06_psfs", "07_kernels", "08_match"]);
    }

    #[test]
    fn the_psfs_step_recentres_only_when_asked_and_saves_the_key_only_when_given() {
        use ndarray::Array2;
        use crate::infra::fits::writer::write_fits_mono;
        use crate::types::header::HduHeader;
        let unset = Recipe::from_value(&d1_recipe()).unwrap();
        assert!(serde_json::to_value(&unset).unwrap()["steps"][5].get("recentre").is_none());
        for given in [true, false] {
            let mut value = d1_recipe();
            value["steps"][5]["recentre"] = json!(given);
            let recipe = Recipe::from_value(&value).unwrap_or_else(|error| panic!("recentre {given}: {}", error.message));
            assert_eq!(serde_json::to_value(&recipe).unwrap()["steps"][5]["recentre"], json!(given));
        }
        let dir = tempfile::tempdir().unwrap();
        let psf_path = dir.path().join("offset_psf.fits");
        let psf = Array2::from_shape_fn((21, 21), |(y, x)| (-((x as f64 - 10.4).powi(2) + (y as f64 - 9.7).powi(2)) / 4.5).exp() as f32);
        let mut header = HduHeader::empty();
        header.set_f64("PIXSCALE", 0.04);
        write_fits_mono(psf_path.to_str().unwrap(), &psf, Some(&header)).unwrap();
        let recentred = |recentre: Option<bool>| {
            let mut step = json!({ "step": "psfs", "source": "files", "files": [{ "filter": "F444W", "path": psf_path }] });
            if let Some(recentre) = recentre {
                step["recentre"] = json!(recentre);
            }
            let value = json!({ "recipe_version": 1, "inputs": d1_recipe()["inputs"].clone(), "steps": [step] });
            let recipe = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1"));
            let target = dir.path().join(format!("run_{recentre:?}"));
            let report = run_recipe(&recipe, &target, &quiet()).unwrap();
            report.steps[0].report["prepared"]["psfs"][0]["recentred_by_px"].clone()
        };
        assert!(recentred(Some(true)).is_array(), "{}", recentred(Some(true)));
        assert_eq!(recentred(Some(false)), Value::Null);
        assert_eq!(recentred(None), Value::Null);
    }

    #[test]
    fn an_explicit_recentre_false_writes_the_products_of_the_default_d1_run() {
        let mut explicit = d1_recipe();
        explicit["steps"][5]["recentre"] = json!(false);
        let out = tempfile::tempdir().unwrap();
        let run = |value: &Value, folder: &str| {
            let recipe = Recipe::from_value(value).unwrap().resolved(&fixtures().join("d1"));
            let target = out.path().join(folder);
            rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap().install(|| run_recipe(&recipe, &target, &quiet())).unwrap()
        };
        let default = run(&d1_recipe(), "default");
        let given = run(&explicit, "explicit");
        let products = |report: &RecipeReport| report.files.iter().filter(|file| file.path != RECIPE_FILE).cloned().collect::<Vec<_>>();
        assert_eq!(products(&default), products(&given));
        assert_eq!(default.warnings, given.warnings);
        assert_eq!(given.steps[5].parameters["recentre"], json!(false));
        assert!(default.steps[5].parameters.get("recentre").is_none());
        assert!(default.steps[5].report["prepared"]["psfs"].as_array().unwrap().iter().all(|psf| psf["recentred_by_px"].is_null()));
    }

    #[test]
    fn a_cancel_requested_as_a_step_starts_stops_that_step_before_its_first_product() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let recipe = Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1"));
        let dir = tempfile::tempdir().unwrap();
        for (index, folder) in ["01_crop", "02_units", "03_align", "04_cube", "05_edit", "06_psfs", "07_kernels", "08_match"].into_iter().enumerate() {
            let cancelled = Arc::new(AtomicBool::new(false));
            let signal = cancelled.clone();
            let progress = ProgressHandle::with_sink(Arc::new(move |event| {
                if event.stage == folder {
                    signal.store(true, Ordering::SeqCst);
                }
            }), 1)
            .with_cancel_source(Arc::new(move || cancelled.load(Ordering::SeqCst)));
            let target = dir.path().join(folder);
            let error = run_recipe(&recipe, &target, &progress).unwrap_err();
            assert!(error.downcast_ref::<AppError>().is_some(), "{folder}: {error:#}");
            let written: Vec<String> = std::fs::read_dir(target.join(folder)).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
            assert!(written.is_empty(), "{folder} wrote {written:?} after the cancel request");
            let marker = read_value(&target.join(FAILURE_FILE));
            assert_eq!((marker["status"].as_str(), marker["stage"].as_str(), marker["step_index"].as_u64()), (Some("cancelled"), Some(folder), Some(index as u64 + 1)), "{marker}");
            assert!(!target.join(RESULT_FILE).exists() && !target.join(RUN_LOCK).exists(), "{folder}");
        }
    }

    #[test]
    fn a_cancel_inside_the_align_step_keeps_only_its_first_product() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("run");
        let aligned = target.join("03_align");
        let watched = aligned.clone();
        let progress = quiet().with_cancel_source(Arc::new(move || fits_products(&watched) > 0));
        let recipe = Recipe::from_value(&d1_recipe()).unwrap().resolved(&fixtures().join("d1"));
        let error = run_recipe(&recipe, &target, &progress).unwrap_err();
        assert!(error.downcast_ref::<AppError>().is_some(), "{error:#}");
        assert_eq!(fits_products(&aligned), 1, "the align step went on after the cancel request");
        let marker = read_value(&target.join(FAILURE_FILE));
        assert_eq!(
            (marker["status"].as_str(), marker["stage"].as_str(), marker["step_index"].as_u64(), marker["code"].as_str()),
            (Some("cancelled"), Some("03_align"), Some(3), Some("cancelled")),
            "{marker}"
        );
        assert!(!target.join("04_cube").exists());
        assert!(!target.join(RESULT_FILE).exists());
        assert!(!target.join(RUN_LOCK).exists());
    }

    #[test]
    fn a_failure_marker_keeps_the_advisories_and_the_step_warnings_gathered_before_the_failure() {
        let mut value = d1_recipe();
        value["steps"][5]["files"].as_array_mut().unwrap().remove(1);
        value["steps"].as_array_mut().unwrap().insert(5, json!({ "step": "edit", "action": "cut", "x_start": 2, "x_end": 40, "y_start": 2, "y_end": 40 }));
        let recipe = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1"));
        let advisories = recipe.validate().unwrap();
        assert_eq!(advisories.len(), 1, "{advisories:?}");
        let out = tempfile::tempdir().unwrap();
        let target = out.path().join("run");
        let error = run_recipe(&recipe, &target, &quiet()).unwrap_err();
        assert_eq!(error.downcast_ref::<CodedError>().map(|coded| coded.code), Some("kernel_missing"), "{error:#}");
        let marker = read_value(&target.join(FAILURE_FILE));
        let warnings: Vec<&str> = marker["warnings"].as_array().unwrap_or_else(|| panic!("{marker}")).iter().map(|warning| warning.as_str().unwrap()).collect();
        assert_eq!(warnings.first().copied(), Some(advisories[0].as_str()));
        assert!(warnings.iter().any(|warning| warning.starts_with("08_kernels: ")), "{warnings:?}");
        let folders = ["01_crop", "02_units", "03_align", "04_cube", "05_edit", "06_edit", "07_psfs", "08_kernels", "09_match"];
        assert!(warnings.iter().all(|warning| folders.iter().any(|folder| warning.starts_with(&format!("{folder}: ")))), "{warnings:?}");
        use std::sync::atomic::{AtomicBool, Ordering};
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let progress = ProgressHandle::with_sink(Arc::new(move |event| {
            if event.stage == HASHING_INPUTS {
                signal.store(true, Ordering::SeqCst);
            }
        }), 1)
        .with_cancel_source(Arc::new(move || cancelled.load(Ordering::SeqCst)));
        let stopped = out.path().join("cancelled");
        run_recipe(&recipe, &stopped, &progress).unwrap_err();
        let marker = read_value(&stopped.join(FAILURE_FILE));
        assert_eq!((marker["stage"].as_str(), &marker["warnings"]), (Some(HASHING_INPUTS), &json!(advisories)), "{marker}");
    }

    #[test]
    fn the_kernels_step_takes_shape_max_where_shape_reference_refuses_a_larger_source_psf() {
        use ndarray::Array2;
        use crate::infra::fits::writer::write_fits_mono;
        let max = json!({ "step": "kernels", "reference": "F444W", "method": "wiener", "eps": 0.05, "shape": "max" });
        let parsed = parse_step(6, &max).unwrap();
        let options = KernelOptions { method: KernelMethod::Wiener { eps: 0.05, eps_mode: EpsMode::Absolute, shape: KernelShape::Named(ShapeName::Max) }, auto: false };
        assert_eq!(parsed, RecipeStep::Kernels(KernelsStep { reference: "F444W".into(), options }));
        assert_eq!(serde_json::to_value(&parsed).unwrap()["shape"], json!("max"));
        let d1 = Recipe::from_value(&with_step(6, max.clone())).unwrap().resolved(&fixtures().join("d1"));
        assert!(d1.validate().unwrap().is_empty());
        let mut cut = with_step(6, max);
        cut["steps"].as_array_mut().unwrap().insert(5, json!({ "step": "edit", "action": "cut", "x_start": 2, "x_end": 40, "y_start": 2, "y_end": 40 }));
        let advisories = Recipe::from_value(&cut).unwrap().resolved(&fixtures().join("d1")).validate().unwrap();
        assert!(advisories.len() == 1 && advisories[0].contains("pixels within half a kernel width of the cut edges"), "{advisories:?}");

        let dir = tempfile::tempdir().unwrap();
        let wide = dir.path().join("wide_f150w_psf.fits");
        let psf = Array2::from_shape_fn((41, 41), |(y, x)| (-((x as f64 - 20.0).powi(2) + (y as f64 - 20.0).powi(2)) / 2.0).exp() as f32);
        let mut header = HduHeader::empty();
        header.set_f64("PIXSCALE", 0.04);
        write_fits_mono(wide.to_str().unwrap(), &psf, Some(&header)).unwrap();
        let run = |shape: Value| {
            let value = json!({
                "recipe_version": 1,
                "inputs": d1_recipe()["inputs"].clone(),
                "steps": [
                    { "step": "psfs", "source": "files", "files": [{ "filter": "F150W", "path": wide }, { "filter": "F444W", "path": "psf/d1_psf_JWST_F444W_40mas.fits" }] },
                    { "step": "kernels", "reference": "F444W", "method": "wiener", "eps": 0.05, "shape": shape }
                ]
            });
            let recipe = Recipe::from_value(&value).unwrap().resolved(&fixtures().join("d1"));
            recipe.validate().unwrap();
            let target = dir.path().join(format!("run_{}", shape.as_str().unwrap()));
            (run_recipe(&recipe, &target, &quiet()), target)
        };
        let (refused, target) = run(json!("reference"));
        let error = refused.unwrap_err();
        assert_eq!(error.downcast_ref::<CodedError>().map(|coded| coded.code), Some("kernel_shape"), "{error:#}");
        let marker = read_value(&target.join(FAILURE_FILE));
        assert_eq!((marker["stage"].as_str(), marker["code"].as_str()), (Some("02_kernels"), Some("kernel_shape")), "{marker}");
        assert!(marker["hint"].as_str().is_some_and(|hint| hint.contains("\"shape\": \"max\"")), "{marker}");
        let (built, target) = run(json!("max"));
        let report = built.unwrap_or_else(|error| panic!("{error:#}"));
        assert_eq!(report.steps[1].parameters["shape"], json!("max"));
        let kernel = extract_header_by_index(&File::open(target.join("02_kernels").join("kernel_f150w_to_f444w.fits")).unwrap(), 0).unwrap();
        assert_eq!((kernel.get_i64("NAXIS1"), kernel.get_i64("NAXIS2")), (Some(41), Some(41)));
    }
}
