pub mod runtime;
pub mod install;
pub mod saved;

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use anyhow::{bail, ensure, Context, Result};
use ndarray::Array2;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use crate::core::cubeprep::cube::{cube_base_header, dq_table_of, Cube, CubePlane};
use crate::core::cubeprep::align::{same_grid, CARD_NCORRK, CARD_NCORRSRC, NOISCORR_EXTNAME, NOISCORR_SOURCE_ALIGN, NOISCORR_SOURCE_MATCH};
use crate::core::imaging::dq_flags::{bits_present, DqTable};
use crate::core::metadata::photcal::card_text;
use crate::core::metadata::units::{classify_unit, UnitClass, UnitInfo};
use crate::core::stacking::memory::{effective_memory, format_bytes, MemoryInfo};
use crate::infra::fits::writer::{is_wcs_card, HduData, ImageHdu};
use crate::infra::fits::reader::{extract_header_by_index, extract_image_planes_by_index, list_extensions, HduInfo};
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::wcs_source::header_wcs;
use crate::infra::paths::plain;
use crate::infra::progress::ProgressHandle;
use crate::service::cubeprep::{filter_registry, read_cube_file};
use crate::service::image_files::write_derived_mef;
use crate::types::image_ref::{ImageRef, PlaneSelector};
use crate::types::header::HduHeader;

pub const SAGUI_SEED: u32 = 1;
const SED_SCALINGS: [&str; 2] = ["median_centre", "median_mad"];
const RUN_STAGE: &str = "Running official SAGUI in isolated R";
const CANCEL_CHECK_PIXELS: usize = 1024;
const CSV_HEADER: &str = "region,n_pixels,filter,pivot_um,flux,error,error_correlated_jy,unit";
const REGION_PHOTOMETRY_NOTE: &str = "sum of original calibrated SCI converted to Jy; error = sqrt(sum(ERR^2)) converted to Jy, ERR = 0 is used as zero variance; unknown if any used ERR is NaN, infinite, negative or absent; error_correlated = sqrt(sum over pixel pairs p,q of ERR_p ERR_q NOISCORR_z(p-q)) converted to Jy when the input carries NOISCORR, empty otherwise and on PSF-matched planes whose NOISCORR covers alignment only (NCORRSRC='align')";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaguiConfig {
    pub input_path: String,
    pub output_dir: String,
    pub rscript_path: String,
    pub library_path: String,
    pub n_regions: usize,
    pub starlet: bool,
    pub starlet_scales: Vec<usize>,
    pub starlet_threshold: f64,
    pub dq_exclude_bits: u32,
    pub psf_target: String,
    pub mode: String,
    pub knn_k: usize,
    #[serde(default = "default_sed_scaling")]
    pub sed_scaling: String,
}

fn default_sed_scaling() -> String { SED_SCALINGS[0].into() }

#[derive(Debug, Clone, Serialize)]
pub struct Band {
    pub filter: String,
    pub pivot_um: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaneDqFlags {
    pub filter: String,
    pub table: &'static str,
    pub flagged_finite: BTreeMap<String, usize>,
}

#[derive(Debug, Serialize)]
pub struct InputInfo {
    pub width: usize,
    pub height: usize,
    pub bands: Vec<Band>,
    pub unit: String,
    pub has_err: bool,
    pub has_dq: bool,
    pub has_noise_correlation: bool,
    pub dq_flags: Vec<PlaneDqFlags>,
    pub estimated_memory_bytes: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionalBand {
    pub filter: String,
    pub pivot_um: f64,
    pub flux: f64,
    pub error: Option<f64>,
    #[serde(default)]
    pub error_correlated: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Region {
    pub region: u32,
    pub n_pixels: usize,
    #[serde(default)]
    pub n_components: usize,
    #[serde(default)]
    pub min_snr: Option<f64>,
    #[serde(default)]
    pub min_snr_lower_bound: Option<f64>,
    pub bands: Vec<RegionalBand>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SaguiResult {
    pub run_id: String,
    pub output_dir: String,
    pub labels_path: String,
    pub mask_path: String,
    pub table_path: String,
    pub manifest_path: String,
    pub width: usize,
    pub height: usize,
    pub unit: String,
    pub labels: Vec<u32>,
    pub regions: Vec<Region>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NoiseCorrelation {
    pub half: usize,
    pub source: String,
    pub planes: Vec<Array2<f32>>,
}

impl NoiseCorrelation {
    fn covers(&self, plane: &CubePlane) -> bool {
        self.source == NOISCORR_SOURCE_MATCH || plane.kernel.trim().is_empty()
    }

    fn withheld(&self, cube: &Cube) -> Vec<String> {
        cube.filters.iter().filter(|plane| !self.covers(plane)).map(|plane| plane.filter.clone()).collect()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct MapCounts {
    pub pixels: usize,
    pub eligible_pixels: usize,
    pub excluded_by_policy: usize,
    pub support_pixels: usize,
    pub labelled_pixels: usize,
    pub support_unlabelled: usize,
    pub support_on_excluded: usize,
}

#[derive(Debug, Default)]
pub struct BridgeReport {
    pub clustered_pixels: Option<u64>,
    pub clustering_memory_bytes: Option<u64>,
    pub hclust_limit: Option<u64>,
    pub warnings: Vec<String>,
}

struct Footprint<'a> {
    labels: &'a [u32],
    width: usize,
    region: u32,
    pixels: &'a [usize],
}

struct CheckedRun {
    input: PathBuf,
    input_hash: String,
    cube: Cube,
    info: InputInfo,
    correlation: Option<NoiseCorrelation>,
    eligible: Vec<bool>,
    estimate: u64,
    memory: Option<MemoryInfo>,
    runtime: runtime::RuntimeInfo,
}

pub fn valid_run_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn memory_estimate(pixels: u64, bands: u64, mode: &str, knn: u64) -> u64 {
    let cube = pixels.saturating_mul(bands).saturating_mul(64);
    let clustering = if mode == "ward" { pixels.saturating_mul(pixels.saturating_sub(1)).saturating_mul(12) }
        else { pixels.saturating_mul(knn).saturating_mul(128) };
    cube.saturating_add(clustering).saturating_add(128 * 1024 * 1024)
}

fn footprint_memory(pixels: u64, bands: u64, mode: &str, knn: u64) -> u64 {
    let cube = pixels.saturating_mul(bands).saturating_mul(64);
    let graph = if mode == "sparse" { pixels.saturating_mul(knn).saturating_mul(128) } else { 0 };
    cube.saturating_add(graph).saturating_add(128 * 1024 * 1024)
}

fn unit_refusal(unit: &UnitInfo) -> Option<String> {
    let bunit = unit.bunit.clone().unwrap_or_default();
    match &unit.class {
        UnitClass::PerPixelPhysical { .. } => None,
        UnitClass::SurfaceBrightness { .. } => Some(format!("BUNIT '{bunit}' is a surface brightness; SAGUI requires a calibrated per-pixel flux density, so convert it to Jy/pixel in Cube Prep")),
        UnitClass::PerPixelCalibrated { cards } => Some(format!("BUNIT '{bunit}' is calibrated counts ({}); SAGUI requires a calibrated per-pixel flux density, so convert it to Jy/pixel in Cube Prep", cards.join(", "))),
        UnitClass::Unknown => Some(format!("BUNIT '{bunit}' is not a recognised per-pixel flux density (accepted: Jy, mJy, uJy, nJy, MJy (exact case), the words milli, micro, nano and mega, N*unit, optional /pixel; kilo prefixes, mixed-case Mjy and all-caps MJY are refused as ambiguous); Cube Prep cannot convert it either")),
    }
}

fn all_finite(cube: &Cube) -> Vec<bool> {
    let (height, width) = cube.dims();
    (0..height * width).map(|i| cube.planes.iter().all(|p| p[(i / width, i % width)].is_finite())).collect()
}

fn plane_dq_table(cube: &Cube, plane: &CubePlane) -> DqTable {
    dq_table_of(plane).or_else(|| DqTable::select(&cube.header))
}

fn flag_name(table: DqTable, bit: u32) -> String {
    table.decode(bit).into_iter().next().unwrap_or_default()
}

fn flagged_finite_counts(plane: &Array2<u32>, finite: &[bool], bits: u32, table: DqTable) -> BTreeMap<String, usize> {
    let present = bits_present(plane) & bits;
    (0..32).map(|n| 1u32 << n).filter(|bit| present & bit != 0).filter_map(|bit| {
        let count = plane.iter().enumerate().filter(|(i, &value)| finite[*i] && value & bit != 0).count();
        (count > 0).then(|| (flag_name(table, bit), count))
    }).collect()
}

fn dq_flag_counts(cube: &Cube) -> Vec<PlaneDqFlags> {
    let Some(dq) = &cube.dq else { return Vec::new() };
    let finite = all_finite(cube);
    dq.planes.iter().zip(&cube.filters).map(|(plane, filter)| {
        let table = plane_dq_table(cube, filter);
        PlaneDqFlags { filter: filter.filter.clone(), table: table.label(), flagged_finite: flagged_finite_counts(plane, &finite, u32::MAX, table) }
    }).collect()
}

fn dq_exclusion_report(cube: &Cube, bits: u32, eligible: &[bool]) -> (serde_json::Value, Vec<String>) {
    let finite = all_finite(cube);
    let finite_pixels = finite.iter().filter(|&&f| f).count();
    let available = eligible.iter().filter(|&&valid| valid).count();
    let mut warnings = Vec::new();
    let planes: Vec<serde_json::Value> = cube.dq.as_ref().map(|dq| dq.planes.iter().zip(&cube.filters).map(|(plane, filter)| {
        let table = plane_dq_table(cube, filter);
        let removed = plane.iter().enumerate().filter(|(i, &value)| finite[*i] && value & bits != 0).count();
        let flags = flagged_finite_counts(plane, &finite, bits, table);
        if removed > 0 {
            let detail = flags.iter().map(|(name, count)| format!("{name} {count}")).collect::<Vec<_>>().join(", ");
            warnings.push(format!("{}: DQ policy 0x{bits:08x} removes {removed} finite pixel{} ({detail})", filter.filter, if removed == 1 { "" } else { "s" }));
        }
        json!({ "filter": filter.filter, "table": table.label(), "removed_pixels": removed, "flags": flags })
    }).collect()).unwrap_or_default();
    (json!({ "bits": bits, "finite_pixels": finite_pixels, "excluded_pixels": finite_pixels - available, "planes": planes }), warnings)
}

fn zero_error_warnings(cube: &Cube) -> Vec<String> {
    let Some(err) = &cube.err else { return Vec::new() };
    cube.planes.iter().zip(err).zip(&cube.filters).filter_map(|((plane, sigma), filter)| {
        let zeros = plane.iter().zip(sigma).filter(|(value, sigma)| value.is_finite() && **sigma == 0.0).count();
        (zeros > 0).then(|| format!("{}: {zeros} pixels with finite SCI have ERR = 0; regional uncertainties use them as zero variance, as sagui::extract_region_sed does, so regions containing them understate the error", filter.filter))
    }).collect()
}

fn validate_cube(cube: &Cube) -> Result<InputInfo> {
    ensure!(cube.planes.len() >= 2, "SAGUI requires at least two photometric bands");
    ensure!(card_text(&cube.header, "CTYPE3").is_some_and(|value| value.eq_ignore_ascii_case("FILTER")), "SAGUI requires CTYPE3=FILTER; a spectral cube is not a multiband preparation");
    if let Some(refusal) = unit_refusal(&classify_unit(&cube.header)) { bail!(refusal); }
    cube.wcs().context("The multiband cube needs a valid celestial WCS")?;
    let (height, width) = cube.dims();
    ensure!(width > 0 && height > 0 && cube.planes.iter().all(|p| p.dim() == (height, width)), "Inconsistent science dimensions");
    if let Some(err) = &cube.err { ensure!(err.len() == cube.planes.len() && err.iter().all(|p| p.dim() == (height, width)), "ERR dimensions differ from SCI"); }
    if let Some(dq) = &cube.dq { ensure!(dq.planes.len() == cube.planes.len() && dq.planes.iter().all(|p| p.dim() == (height, width)), "DQ dimensions differ from SCI"); }
    ensure!(cube.filters.len() == cube.planes.len(), "FILTERS order does not cover every science plane");
    let mut seen = HashSet::new();
    let bands = cube.filters.iter().map(|plane| {
        ensure!(!plane.filter.trim().is_empty() && seen.insert(plane.filter.to_lowercase()), "Filters must be nonempty and unique");
        let pivot_um = plane.pivot_um.filter(|v| v.is_finite() && *v > 0.0).context("Every filter requires a positive pivot wavelength")?;
        Ok(Band { filter: plane.filter.clone(), pivot_um })
    }).collect::<Result<Vec<_>>>()?;
    let mut warnings = vec!["External cubes require aligned bands and a verified common PSF; a target name alone does not validate matching".to_string()];
    if cube.err.is_none() { warnings.push("ERR is absent: regional uncertainties remain unknown".into()); }
    if cube.dq.is_none() { warnings.push("DQ is absent: only finite-value support can be checked".into()); }
    warnings.extend(zero_error_warnings(cube));
    Ok(InputInfo { width, height, bands, unit: card_text(&cube.header, "BUNIT").unwrap_or_default(), has_err: cube.err.is_some(), has_dq: cube.dq.is_some(), has_noise_correlation: false, dq_flags: dq_flag_counts(cube), estimated_memory_bytes: memory_estimate((width * height) as u64, cube.planes.len() as u64, "ward", 40), warnings })
}

fn validate_companion_header(cube: &Cube, header: &HduHeader, name: &str) -> Result<Vec<String>> {
    if name == "ERR" {
        let sci_unit = classify_unit(&cube.header);
        let err_unit = classify_unit(header);
        ensure!(err_unit.bunit.is_some() && err_unit.class == sci_unit.class, "ERR BUNIT must explicitly match the calibrated SCI unit");
    }
    if let Some(axis) = card_text(header, "CTYPE3") { ensure!(axis.eq_ignore_ascii_case("FILTER"), "{name} band axis differs from SCI"); }
    let mut warnings = Vec::new();
    if header.cards.iter().any(|(key, _)| is_wcs_card(key) && !matches!(key.as_str(), "CTYPE3" | "CRVAL3" | "CRPIX3" | "CDELT3" | "CUNIT3" | "WCSAXES")) {
        let wcs = header_wcs(header).with_context(|| format!("{name} has an incomplete celestial WCS"))?;
        ensure!(same_grid(&cube.wcs()?, cube.dims(), &wcs, cube.dims()), "{name} WCS differs from SCI");
    } else {
        warnings.push(format!("{name} has no celestial WCS: same-index alignment to SCI is assumed from matching FITS dimensions"));
    }
    Ok(warnings)
}

fn read_noise_correlation(file: &File, hdu: &HduInfo, bands: usize) -> Result<NoiseCorrelation> {
    ensure!(hdu.naxis == 3 && hdu.naxis1 == hdu.naxis2 && hdu.naxis1 % 2 == 1 && hdu.naxis3 == bands as i64, "{NOISCORR_EXTNAME} must be a 3-D [plane, K, K] image with odd K and one map per band; found NAXIS={} {}x{}x{}", hdu.naxis, hdu.naxis1, hdu.naxis2, hdu.naxis3);
    let size = hdu.naxis1 as usize;
    let header = extract_header_by_index(file, hdu.index)?;
    let declared = header.get_i64(CARD_NCORRK);
    ensure!(declared == Some(size as i64), "{NOISCORR_EXTNAME} {CARD_NCORRK}={} disagrees with its {size}x{size} maps", declared.map_or("absent".to_string(), |v| v.to_string()));
    let source = card_text(&header, CARD_NCORRSRC).unwrap_or_default();
    ensure!(source == NOISCORR_SOURCE_ALIGN || source == NOISCORR_SOURCE_MATCH, "{NOISCORR_EXTNAME} {CARD_NCORRSRC}='{source}' is neither '{NOISCORR_SOURCE_ALIGN}' nor '{NOISCORR_SOURCE_MATCH}'");
    let planes = extract_image_planes_by_index(file, hdu.index)?;
    ensure!(planes.len() == bands && planes.iter().all(|plane| plane.dim() == (size, size)), "{NOISCORR_EXTNAME} holds {} maps for {bands} bands", planes.len());
    for (z, plane) in planes.iter().enumerate() {
        let centre = plane[(size / 2, size / 2)];
        ensure!((f64::from(centre) - 1.0).abs() <= f64::from(f32::EPSILON) && plane.iter().all(|v| v.is_finite()), "{NOISCORR_EXTNAME} plane {z} is not a normalised finite autocorrelation: Q(0,0) is {centre}, not 1 to float32 precision, or a value is not finite");
    }
    Ok(NoiseCorrelation { half: size / 2, source, planes })
}

fn load_input(path: &str) -> Result<(Cube, InputInfo, Option<NoiseCorrelation>)> {
    let reference = ImageRef::parse(path);
    ensure!(matches!(reference.plane, PlaneSelector::Auto), "Select the whole multiband FITS file; SAGUI identifies SCI/ERR/DQ by extension name");
    let cube = read_cube_file(path, &filter_registry())?.cube;
    let mut info = validate_cube(&cube)?;
    let (fits_path, _temporary) = resolve_single_image(&reference.path)?;
    let file = File::open(&fits_path).with_context(|| format!("The input cube {} is not readable", fits_path.display()))?;
    let mut correlation = None;
    let mut correlation_seen = false;
    for hdu in list_extensions(&file)? {
        let name = hdu.extname.as_deref().unwrap_or_default().trim().trim_matches('\'').trim().to_uppercase();
        if name == "ERR" || name == "DQ" {
            ensure!((name == "ERR" && cube.err.is_some()) || (name == "DQ" && cube.dq.is_some()), "{name} was declared but could not be loaded; use a MEF with 3D SCI/ERR/DQ extensions");
            ensure!(hdu.naxis == 3 && hdu.naxis1 == info.width as i64 && hdu.naxis2 == info.height as i64 && hdu.naxis3 == info.bands.len() as i64, "{name} dimensions differ from SCI");
            info.warnings.extend(validate_companion_header(&cube, &extract_header_by_index(&file, hdu.index)?, &name)?);
        } else if name == NOISCORR_EXTNAME && !correlation_seen {
            correlation_seen = true;
            match read_noise_correlation(&file, &hdu, info.bands.len()) {
                Ok(maps) => correlation = Some(maps),
                Err(error) => info.warnings.push(format!("{NOISCORR_EXTNAME} extension is malformed and was ignored ({error:#}); regional errors assume independent pixels and error_correlated stays empty")),
            }
        }
    }
    info.has_noise_correlation = correlation.is_some();
    match &correlation {
        Some(maps) => {
            info.warnings.insert(0, format!("Regional error assumes independent pixels; error_correlated applies the {NOISCORR_EXTNAME} noise autocorrelation ({CARD_NCORRSRC}='{}', {1}x{1} per plane)", maps.source, 2 * maps.half + 1));
            let withheld = maps.withheld(&cube);
            if !withheld.is_empty() {
                info.warnings.push(format!("error_correlated is left empty for {}: {CARD_NCORRSRC}='{NOISCORR_SOURCE_ALIGN}' maps hold only the resampling correlation, and these planes were PSF-matched (FILTERS KERNEL set), so their matching covariance is not represented", withheld.join(", ")));
            }
        }
        None if !correlation_seen => info.warnings.insert(0, format!("{NOISCORR_EXTNAME} extension is absent: regional errors assume independent pixels and error_correlated stays empty; resampling and PSF matching create covariance that is not stored")),
        None => {}
    }
    Ok((cube, info, correlation))
}

pub fn inspect_input(path: &str) -> Result<InputInfo> {
    Ok(load_input(path)?.1)
}

fn validate_config(config: &SaguiConfig, pixels: usize) -> Result<()> {
    ensure!(config.n_regions >= 2 && config.n_regions <= pixels, "Choose between two regions and the available pixel count");
    ensure!(config.mode == "ward" || config.mode == "sparse", "Choose exact Ward or explicit sparse Ward");
    ensure!(config.knn_k >= 2 && config.knn_k <= 4096, "Sparse neighbours must be between 2 and 4096");
    ensure!(!config.psf_target.trim().is_empty(), "Identify the verified common target PSF before segmentation");
    ensure!(!config.starlet_scales.is_empty() && config.starlet_scales.iter().all(|&s| (1..=10).contains(&s)), "Starlet scales must be between 1 and 10");
    ensure!(config.starlet_threshold.is_finite() && config.starlet_threshold > 0.0, "Starlet threshold must be positive and finite");
    ensure!(SED_SCALINGS.contains(&config.sed_scaling.as_str()), "sed_scaling must be median_centre or median_mad");
    ensure!(Path::new(&config.output_dir).is_absolute(), "Choose an absolute persistent output directory");
    Ok(())
}

fn eligible(cube: &Cube, bits: u32) -> Vec<bool> {
    let (height, width) = cube.dims();
    (0..height * width).map(|i| cube.planes.iter().all(|p| p[(i / width, i % width)].is_finite()) && cube.dq.as_ref().is_none_or(|dq| dq.planes.iter().all(|p| p[(i / width, i % width)] & bits == 0))).collect()
}

fn write_science(path: &Path, cube: &Cube, eligible: &[bool]) -> Result<()> {
    let mut file = BufWriter::new(File::create(path).with_context(|| format!("Cannot write {}", path.display()))?);
    for plane in &cube.planes {
        for (i, &value) in plane.iter().enumerate() {
            let value = if eligible[i] { f64::from(value) } else { f64::NAN };
            file.write_all(&value.to_le_bytes())?;
        }
    }
    file.flush()?;
    Ok(())
}

fn read_integers(path: &Path, pixels: usize) -> Result<Vec<u32>> {
    let length = fs::metadata(path).with_context(|| format!("Cannot read the SAGUI map {}", path.display()))?.len();
    ensure!(length == (pixels as u64).saturating_mul(4), "SAGUI returned an incorrectly sized map: {}", path.display());
    Ok(fs::read(path).with_context(|| format!("Cannot read the SAGUI map {}", path.display()))?.chunks_exact(4).map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap())).collect())
}

fn validate_maps(labels: &[u32], mask: &[u32], eligible: &[bool], n_regions: usize) -> Result<()> {
    ensure!(labels.len() == eligible.len() && mask.len() == eligible.len(), "Map dimensions disagree");
    let mut seen = HashSet::new();
    for ((&label, &mask), &valid) in labels.iter().zip(mask).zip(eligible) {
        ensure!(label <= n_regions as u32 && mask <= 1, "Invalid SAGUI labels or support values");
        ensure!(mask == 0 || valid, "SAGUI marked an excluded pixel as support");
        ensure!(label == 0 || (valid && mask == 1), "SAGUI assigned an excluded pixel to a region");
        if label > 0 { seen.insert(label); }
    }
    ensure!(seen.len() == n_regions, "SAGUI returned a different number of regions than requested");
    Ok(())
}

fn map_counts(labels: &[u32], mask: &[u32], eligible: &[bool]) -> MapCounts {
    let mut counts = MapCounts { pixels: labels.len(), ..MapCounts::default() };
    for ((&label, &mask), &valid) in labels.iter().zip(mask).zip(eligible) {
        counts.eligible_pixels += usize::from(valid);
        counts.support_pixels += usize::from(mask == 1);
        counts.labelled_pixels += usize::from(label > 0);
        counts.support_unlabelled += usize::from(mask == 1 && label == 0);
        counts.support_on_excluded += usize::from(mask == 1 && !valid);
    }
    counts.excluded_by_policy = counts.pixels - counts.eligible_pixels;
    counts
}

fn map_warnings(counts: &MapCounts, upstream_support_on_excluded: usize) -> Vec<String> {
    let mut warnings = Vec::new();
    if counts.support_unlabelled > 0 {
        warnings.push(format!("{} support pixels were left unlabelled by SAGUI's validity filter (non-positive sum, zero MAD or a non-finite spectrum)", counts.support_unlabelled));
    }
    if upstream_support_on_excluded > 0 {
        warnings.push(format!("{upstream_support_on_excluded} excluded pixels were marked as support by SAGUI's upstream mask; the published support removes them and support-upstream.bin keeps the raw mask"));
    }
    warnings
}

fn check_connected(labels: &[u32], config: &SaguiConfig) -> Result<()> {
    let clusters = labels.iter().copied().max().unwrap_or(0) as usize;
    ensure!(config.mode != "sparse" || clusters <= config.n_regions, "Sparse Ward neighbour graph is disconnected: {clusters} clusters for {} requested with knn_k {}; increase the nearest-neighbour count", config.n_regions, config.knn_k);
    Ok(())
}

fn support_per_region_warning(cube: &Cube, psf_target: &str, support_pixels: usize, n_regions: usize) -> Option<String> {
    let per_region = support_pixels as f64 / n_regions as f64;
    let positive = |value: Option<f64>| value.filter(|v| v.is_finite() && *v > 0.0);
    let target = psf_target.trim();
    let resolved = cube.filters.iter().find(|plane| plane.filter.eq_ignore_ascii_case(target)).and_then(|plane| Some((&plane.filter, positive(plane.psf_fwhm_arcsec)?, positive(plane.pixscale_arcsec)?)));
    let Some((filter, fwhm, scale)) = resolved else {
        return Some(format!("The PSF-size check could not run: FILTERS gives no finite PSF_FWHM and PIXSCALE for the PSF target {target}; support of {support_pixels} pixels over {n_regions} regions gives {per_region:.1} pixels per region, and regions smaller than the PSF noise-equivalent area (pi FWHM^2 / (2 ln 2) in pixels) are not resolved"));
    };
    let fwhm_pixels = fwhm / scale;
    let noise_equivalent_area = std::f64::consts::PI * fwhm_pixels * fwhm_pixels / (2.0 * std::f64::consts::LN_2);
    (per_region < noise_equivalent_area).then(|| format!("Support of {support_pixels} pixels over {n_regions} regions gives {per_region:.1} pixels per region, below the PSF noise-equivalent area of {noise_equivalent_area:.1} pixels ({filter} FWHM {fwhm:.4} arcsec at {scale:.4} arcsec per pixel); regions smaller than the PSF are not resolved, so request fewer regions"))
}

fn component_sizes(labels: &[u32], width: usize, region: u32) -> Vec<usize> {
    let height = labels.len() / width;
    let mut visited = vec![false; labels.len()];
    let mut sizes = Vec::new();
    for start in (0..labels.len()).filter(|&i| labels[i] == region) {
        if visited[start] { continue; }
        visited[start] = true;
        let mut stack = vec![start];
        let mut size = 0;
        while let Some(index) = stack.pop() {
            size += 1;
            let (y, x) = ((index / width) as isize, (index % width) as isize);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (ny, nx) = (y + dy, x + dx);
                    if ny < 0 || nx < 0 || ny >= height as isize || nx >= width as isize { continue; }
                    let neighbour = ny as usize * width + nx as usize;
                    if labels[neighbour] == region && !visited[neighbour] {
                        visited[neighbour] = true;
                        stack.push(neighbour);
                    }
                }
            }
        }
        sizes.push(size);
    }
    sizes
}

fn fragment_warning(labels: &[u32], width: usize) -> Option<String> {
    let regions: Vec<u32> = labels.iter().copied().filter(|&label| label > 0).collect::<std::collections::BTreeSet<_>>().into_iter()
        .filter(|&region| component_sizes(labels, width, region).contains(&1)).collect();
    let ids = regions.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
    match regions.len() {
        0 => None,
        1 => Some(format!("Region {ids} contains 1-pixel fragments (8-connected); SAGUI groups pixels by spectrum, not by position")),
        _ => Some(format!("Regions {ids} contain 1-pixel fragments (8-connected); SAGUI groups pixels by spectrum, not by position")),
    }
}

fn correlated_variance(footprint: &Footprint, err: &Array2<f32>, map: &Array2<f32>, progress: &ProgressHandle) -> Result<f64> {
    let Footprint { labels, width, region, pixels } = *footprint;
    let height = labels.len() / width;
    let centre = (map.dim().0 / 2) as isize;
    let (reach_y, reach_x) = (centre.min(height as isize - 1), centre.min(width as isize - 1));
    let mut total = 0.0;
    for (n, &p) in pixels.iter().enumerate() {
        if n % CANCEL_CHECK_PIXELS == 0 { ensure!(!progress.is_cancelled(), "SAGUI cancelled; results were not published"); }
        let (y, x) = ((p / width) as isize, (p % width) as isize);
        let sigma_p = f64::from(err[(p / width, p % width)]);
        for dy in -reach_y..=reach_y {
            let qy = y + dy;
            if qy < 0 || qy >= height as isize { continue; }
            for dx in -reach_x..=reach_x {
                let qx = x + dx;
                if qx < 0 || qx >= width as isize || labels[qy as usize * width + qx as usize] != region { continue; }
                let weight = f64::from(map[((centre + dy) as usize, (centre + dx) as usize)]);
                if weight != 0.0 { total += sigma_p * f64::from(err[(qy as usize, qx as usize)]) * weight; }
            }
        }
    }
    Ok(total)
}

fn min_snr(bands: &[RegionalBand]) -> Option<f64> {
    let lowest = bands.iter().map(|band| Some(band.flux / band.error?)).collect::<Option<Vec<f64>>>()?.into_iter().fold(f64::INFINITY, f64::min);
    lowest.is_finite().then_some(lowest)
}

fn diagnostics(labels: &[u32], width: usize, region: &Region) -> (usize, Option<f64>, Option<f64>) {
    let snr = min_snr(&region.bands);
    (component_sizes(labels, width, region.region).len(), snr, snr.map(|value| value / (region.n_pixels as f64).sqrt()))
}

fn photometry(cube: &Cube, labels: &[u32], correlation: Option<&NoiseCorrelation>, progress: &ProgressHandle) -> Result<Vec<Region>> {
    let (_, width) = cube.dims();
    let UnitClass::PerPixelPhysical { jy_per_unit } = classify_unit(&cube.header).class else {
        bail!("Regional photometry requires calibrated per-pixel flux units");
    };
    let mut regions: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (i, &label) in labels.iter().enumerate().filter(|(_, label)| **label > 0) { regions.entry(label).or_default().push(i); }
    let count = regions.len();
    regions.into_iter().map(|(region, indices)| {
        let footprint = Footprint { labels, width, region, pixels: &indices };
        let bands = cube.planes.iter().zip(&cube.filters).enumerate().map(|(b, (plane, filter))| {
            let flux = indices.iter().map(|&i| f64::from(plane[(i / width, i % width)])).sum::<f64>() * jy_per_unit;
            ensure!(flux.is_finite(), "Nonfinite regional flux");
            let err = cube.err.as_ref().map(|err| &err[b]);
            let sigma = err.and_then(|err| indices.iter().map(|&i| {
                let sigma = f64::from(err[(i / width, i % width)]);
                (sigma.is_finite() && sigma >= 0.0).then_some(sigma)
            }).collect::<Option<Vec<f64>>>());
            let error = sigma.as_ref().map(|sigma| sigma.iter().map(|s| s * s).sum::<f64>().sqrt() * jy_per_unit).filter(|v| v.is_finite());
            let map = correlation.filter(|maps| maps.covers(filter)).map(|maps| &maps.planes[b]);
            let error_correlated = match (&sigma, err, map) {
                (Some(_), Some(err), Some(map)) => {
                    progress.emit_stage(&format!("Measuring correlated regional error: region {region} of {count}, {}", filter.filter));
                    Some(correlated_variance(&footprint, err, map, progress)?.sqrt() * jy_per_unit).filter(|v| v.is_finite())
                }
                _ => None,
            };
            Ok(RegionalBand { filter: filter.filter.clone(), pivot_um: filter.pivot_um.context("Missing wavelength")?, flux, error, error_correlated })
        }).collect::<Result<Vec<_>>>()?;
        let mut entry = Region { region, n_pixels: indices.len(), n_components: 0, min_snr: None, min_snr_lower_bound: None, bands };
        (entry.n_components, entry.min_snr, entry.min_snr_lower_bound) = diagnostics(labels, width, &entry);
        Ok(entry)
    }).collect()
}

fn hash_input(input: &Path, progress: &ProgressHandle, failure: &str) -> Result<String> {
    sha256(input, progress).map_err(|error| if progress.is_cancelled() { error } else { error.context(format!("The input cube {} {failure}", input.display())) })
}

fn sha256(path: &Path, progress: &ProgressHandle) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        ensure!(!progress.is_cancelled(), "SAGUI cancelled during integrity verification");
        let n = file.read(&mut buffer)?;
        if n == 0 { break; }
        digest.update(&buffer[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(crate) fn library_fingerprint(path: &Path, progress: &ProgressHandle) -> Result<BTreeMap<String, String>> {
    let root = path.canonicalize()?;
    let mut pending = vec![root.clone()];
    let mut visited = HashSet::new();
    let mut files = BTreeMap::new();
    while let Some(directory) = pending.pop() {
        ensure!(!progress.is_cancelled(), "SAGUI cancelled during library verification");
        ensure!(visited.insert(directory.clone()), "The private R library contains a directory cycle");
        for entry in fs::read_dir(directory)? {
            let path = entry?.path().canonicalize()?;
            ensure!(path.starts_with(&root), "The private R library contains an external link");
            if path.is_dir() { pending.push(path); }
            else { files.insert(path.strip_prefix(&root)?.to_string_lossy().into_owned(), sha256(&path, progress)?); }
        }
    }
    Ok(files)
}

pub(crate) fn csv_text(text: &str) -> String { format!("\"{}\"", text.replace('"', "\"\"")) }

pub(crate) fn csv_number(value: Option<f64>) -> String { value.map(|v| v.to_string()).unwrap_or_default() }

pub(crate) fn regions_csv(regions: &[Region]) -> String {
    let mut table = format!("{CSV_HEADER}\n");
    for region in regions {
        for band in &region.bands {
            table.push_str(&format!("{},{},{},{},{},{},{},Jy\n", region.region, region.n_pixels, csv_text(&band.filter), band.pivot_um, band.flux, csv_number(band.error), csv_number(band.error_correlated)));
        }
    }
    table
}

fn clustering_provenance(config: &SaguiConfig, bridge: &BridgeReport) -> serde_json::Value {
    let scale_fn = if config.sed_scaling == "median_mad" { "sagui:::.safe_scale_spectrum ((x - median(x)) / MAD(x), sd then 1 as fallbacks)" } else { "sagui::median_scale (x - median(x))" };
    let mut clustering = json!({ "cluster_pretransform": "none", "sed_scaling": config.sed_scaling, "scale_fn": scale_fn, "distance": "euclidean" });
    if config.mode == "ward" {
        clustering["linkage"] = json!("ward.D2");
        clustering["hclust_limit"] = json!(bridge.hclust_limit);
    } else {
        clustering["linkage"] = json!("sparse Ward (kNN graph)");
        clustering["knn_k"] = json!(config.knn_k);
    }
    clustering
}

fn parse_bridge_markers(text: &str) -> BridgeReport {
    let mut report = BridgeReport::default();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("ASTROBURST_CLUSTER\t") {
            let mut fields = rest.split('\t');
            report.clustered_pixels = fields.next().and_then(|v| v.trim().parse().ok());
            report.clustering_memory_bytes = fields.next().and_then(|v| v.trim().parse().ok());
            report.hclust_limit = fields.next().and_then(|v| v.trim().parse().ok());
        } else if let Some(rest) = line.strip_prefix("ASTROBURST_WARNING\t") {
            report.warnings.push(rest.into());
        }
    }
    report
}

fn parse_backend(text: &str) -> Result<serde_json::Value> {
    let mut lines = text.lines();
    ensure!(lines.next() == Some("knn_k\trequested_Ncomp\tactual_Ncomp\tdisconnected\tvalid_pixels"), "Malformed SAGUI backend record");
    let fields: Vec<_> = lines.next().context("Missing SAGUI backend record")?.split('\t').collect();
    ensure!(fields.len() == 5, "Malformed SAGUI backend record");
    Ok(json!({ "knn_k_used": fields[0].trim().parse::<u64>()?, "requested_regions": fields[1].trim().parse::<u64>()?, "actual_regions": fields[2].trim().parse::<u64>()?, "disconnected": fields[3].trim() == "TRUE", "valid_pixels": fields[4].trim().parse::<u64>()? }))
}

fn create_run_dir(output_dir: &str, run_id: &str) -> Result<PathBuf> {
    fs::create_dir_all(output_dir).with_context(|| format!("Cannot create the SAGUI working folder {output_dir}"))?;
    let parent = plain(&Path::new(output_dir).canonicalize().with_context(|| format!("Cannot resolve the SAGUI working folder {output_dir}"))?);
    let work = parent.join(format!("sagui-{run_id}"));
    fs::create_dir(&work).with_context(|| format!("The run folder {} already exists or cannot be created", work.display()))?;
    Ok(work)
}

pub fn run(config: SaguiConfig, run_id: String, progress: &ProgressHandle) -> Result<SaguiResult> {
    ensure!(valid_run_id(&run_id), "Invalid SAGUI run identifier");
    ensure!(!progress.is_cancelled(), "SAGUI cancelled");
    let input_text = ImageRef::parse(&config.input_path).path;
    let input = plain(&Path::new(&input_text).canonicalize().with_context(|| format!("The input cube {input_text} is not readable"))?);
    progress.report(0, 5, "Checking calibrated input and runtime");
    let input_hash = hash_input(&input, progress, "is not readable")?;
    let (cube, info, correlation) = load_input(&config.input_path)?;
    let pixels = info.width * info.height;
    validate_config(&config, pixels)?;
    let eligible = eligible(&cube, config.dq_exclude_bits);
    let available = eligible.iter().filter(|&&valid| valid).count();
    ensure!(available >= config.n_regions, "Too few pixels remain after finite-value and DQ exclusion");
    let estimate = memory_estimate(pixels as u64, info.bands.len() as u64, &config.mode, config.knn_k as u64);
    let footprint = footprint_memory(pixels as u64, info.bands.len() as u64, &config.mode, config.knn_k as u64);
    let memory = effective_memory();
    if let Some(memory) = memory { ensure!(footprint <= memory.commit_available, "Loading the cube needs an estimated {} but only {} can be committed; choose a smaller field", format_bytes(footprint), format_bytes(memory.commit_available)); }
    let runtime = runtime::inspect_with_progress(&config.rscript_path, &config.library_path, progress);
    ensure!(runtime.ready, "R environment is not ready: {}{}", runtime.issues.join("; "), install::r_version_hint(&runtime.issues));
    let work = create_run_dir(&config.output_dir, &run_id)?;
    let checked = CheckedRun { input, input_hash, cube, info, correlation, eligible, estimate, memory, runtime };
    let result = run_in_dir(&config, &run_id, &work, checked, progress);
    if let Err(error) = &result { let _ = fs::write(work.join("failure.json"), serde_json::to_vec_pretty(&json!({ "run_id": run_id, "error": format!("{error:#}"), "published": false }))?); }
    result
}

fn run_in_dir(config: &SaguiConfig, run_id: &str, work: &Path, checked: CheckedRun, progress: &ProgressHandle) -> Result<SaguiResult> {
    let CheckedRun { input, input_hash, cube, info, correlation, eligible, estimate, memory, runtime } = checked;
    let InputInfo { width, height, unit, mut warnings, .. } = info;
    fs::write(work.join("config.json"), serde_json::to_vec_pretty(config)?).with_context(|| format!("Cannot write {}", work.join("config.json").display()))?;
    let library = library_fingerprint(Path::new(&config.library_path), progress)?;
    write_science(&work.join("science.bin"), &cube, &eligible)?;
    let exe_hash = sha256(Path::new(&config.rscript_path), progress)?;
    let mut command = runtime::command(&config.rscript_path, &config.library_path, work)?;
    let memory_arg = |value: Option<u64>| value.map_or_else(|| "NA".to_string(), |bytes| bytes.to_string());
    command.arg("run").arg(Path::new(&config.library_path).canonicalize()?).arg(work)
        .arg(runtime::R_VERSION).arg(runtime::SAGUI_COMMIT)
        .arg(width.to_string()).arg(height.to_string()).arg(cube.planes.len().to_string())
        .arg(config.n_regions.to_string()).arg(config.starlet.to_string())
        .arg(config.starlet_scales.iter().map(usize::to_string).collect::<Vec<_>>().join(","))
        .arg(config.starlet_threshold.to_string()).arg(&config.mode).arg(config.knn_k.to_string()).arg(SAGUI_SEED.to_string())
        .arg(memory_arg(memory.map(|m| m.commit_available))).arg(memory_arg(memory.map(|m| m.physical_available))).arg(&config.sed_scaling);
    command.env("LANGUAGE", "en");
    progress.report(1, 5, RUN_STAGE);
    runtime::execute_labelled(RUN_STAGE, command, work, progress, None)?;
    let stdout = fs::read_to_string(work.join("stdout.log")).with_context(|| format!("Cannot read {}", work.join("stdout.log").display()))?;
    let bridge = parse_bridge_markers(&stdout);
    let actual_runtime = runtime::parse_runtime(&stdout)?;
    ensure!(actual_runtime.ready, "The R environment changed during execution");
    let pixels = eligible.len();
    let labels = read_integers(&work.join("labels.bin"), pixels)?;
    check_connected(&labels, config)?;
    let mask = read_integers(&work.join("support.bin"), pixels)?;
    validate_maps(&labels, &mask, &eligible, config.n_regions)?;
    let upstream = read_integers(&work.join("support-upstream.bin"), pixels)?;
    let counts = map_counts(&labels, &mask, &eligible);
    let upstream_support_on_excluded = upstream.iter().zip(&eligible).filter(|(&marked, &valid)| marked == 1 && !valid).count();
    let backend = match fs::read_to_string(work.join("backend.tsv")) { Ok(text) => parse_backend(&text)?, Err(_) => serde_json::Value::Null };
    progress.report(3, 5, "Measuring regional flux on the original calibrated cube");
    let regions = photometry(&cube, &labels, correlation.as_ref(), progress)?;
    let (dq_exclusion, dq_warnings) = dq_exclusion_report(&cube, config.dq_exclude_bits, &eligible);
    warnings.extend(dq_warnings);
    warnings.extend(bridge.warnings.iter().cloned());
    warnings.extend(map_warnings(&counts, upstream_support_on_excluded));
    warnings.extend(support_per_region_warning(&cube, &config.psf_target, counts.support_pixels, config.n_regions));
    warnings.extend(fragment_warning(&labels, width));
    let path = |name: &str| work.join(name).to_string_lossy().into_owned();
    let mut header = cube_base_header(&cube.header);
    header.set("BUNIT", "1".into());
    header.set("ABPROC", "sagui".into());
    let map = Array2::from_shape_vec((height, width), labels.clone())?;
    let mask = Array2::from_shape_vec((height, width), mask)?;
    write_derived_mef(&path("labels.fits"), None, &[ImageHdu { data: HduData::U32(&map), header: Some(&header), extname: "LABELS", extver: 1 }])?;
    write_derived_mef(&path("support.fits"), None, &[ImageHdu { data: HduData::U32(&mask), header: Some(&header), extname: "SUPPORT", extver: 1 }])?;
    fs::write(work.join("regions.csv"), regions_csv(&regions)).with_context(|| format!("Cannot write {}", work.join("regions.csv").display()))?;
    progress.report(4, 5, "Verifying input identity and output products");
    ensure!(!progress.is_cancelled(), "SAGUI cancelled; results were not published");
    ensure!(hash_input(&input, progress, "is no longer readable; results were not published")? == input_hash, "The input cube {} changed during segmentation; results were not published", input.display());
    ensure!(sha256(Path::new(&config.rscript_path), progress)? == exe_hash, "Rscript changed during segmentation");
    ensure!(library_fingerprint(Path::new(&config.library_path), progress)? == library, "The private R library changed during segmentation");
    let result = SaguiResult { run_id: run_id.into(), output_dir: work.to_string_lossy().into_owned(), labels_path: path("labels.fits"), mask_path: path("support.fits"), table_path: path("regions.csv"), manifest_path: path("manifest.json"), width, height, unit: "Jy".into(), labels, regions, warnings };
    let products = ["labels.fits", "support.fits", "regions.csv", "config.json", "bridge.R", "science.bin", "labels.bin", "support.bin", "support-upstream.bin", "packages.tsv", "session.txt", "stdout.log", "stderr.log"];
    let optional = ["backend.tsv"].into_iter().filter(|name| work.join(name).is_file());
    let files = products.into_iter().chain(optional).map(|name| Ok(json!({ "path": name, "sha256": sha256(&work.join(name), progress)? }))).collect::<Result<Vec<_>>>()?;
    let mut map_counts = serde_json::to_value(counts)?;
    map_counts["upstream_support_on_excluded"] = json!(upstream_support_on_excluded);
    let manifest = json!({ "schema_version": 1, "status": "complete", "astroburst": env!("CARGO_PKG_VERSION"), "sagui_commit": runtime::SAGUI_COMMIT, "runtime": runtime, "rscript_sha256": exe_hash, "private_library": library, "input": { "path": input, "sha256": input_hash, "unit": unit }, "config": config, "seed": SAGUI_SEED, "clustering": clustering_provenance(config, &bridge), "clustered_pixels": bridge.clustered_pixels, "clustering_memory_bytes": bridge.clustering_memory_bytes, "map_counts": map_counts, "backend": backend, "dq_exclusion": dq_exclusion, "noise_correlation": correlation.as_ref().map(|maps| json!({ "extension": NOISCORR_EXTNAME, "size": 2 * maps.half + 1, "source": maps.source, "withheld": maps.withheld(&cube) })), "array_order": "x,y,band; x contiguous", "region_photometry": REGION_PHOTOMETRY_NOTE, "covariance": if correlation.is_some() { "NOISCORR autocorrelation per plane" } else { "not represented" }, "estimated_memory_bytes": estimate, "platform": std::env::consts::OS, "files": files, "result": result });
    let mut pending = tempfile::NamedTempFile::new_in(work)?;
    pending.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    pending.as_file().sync_all()?;
    ensure!(!progress.is_cancelled(), "SAGUI cancelled; results were not published");
    pending.persist_noclobber(work.join("manifest.json"))?;
    progress.report(5, 5, "SAGUI products verified");
    Ok(result)
}

#[cfg(test)]
mod tests;
