pub mod runtime;
pub mod install;
pub mod saved;

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use anyhow::{ensure, Context, Result};
use ndarray::Array2;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use crate::core::cubeprep::cube::{cube_base_header, Cube};
use crate::core::cubeprep::align::same_grid;
use crate::core::metadata::photcal::card_text;
use crate::core::metadata::units::{classify_unit, UnitClass};
use crate::core::stacking::memory::effective_memory;
use crate::infra::fits::writer::{is_wcs_card, HduData, ImageHdu};
use crate::infra::fits::reader::{extract_header_by_index, list_extensions};
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::wcs_source::header_wcs;
use crate::infra::progress::ProgressHandle;
use crate::service::cubeprep::{filter_registry, read_cube_file};
use crate::service::image_files::write_derived_mef;
use crate::types::image_ref::{ImageRef, PlaneSelector};
use crate::types::header::HduHeader;

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
}

#[derive(Debug, Clone, Serialize)]
pub struct Band {
    pub filter: String,
    pub pivot_um: f64,
}

#[derive(Debug, Serialize)]
pub struct InputInfo {
    pub width: usize,
    pub height: usize,
    pub bands: Vec<Band>,
    pub unit: String,
    pub has_err: bool,
    pub has_dq: bool,
    pub estimated_memory_bytes: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionalBand {
    pub filter: String,
    pub pivot_um: f64,
    pub flux: f64,
    pub error: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Region {
    pub region: u32,
    pub n_pixels: usize,
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

pub fn valid_run_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn memory_estimate(pixels: u64, bands: u64, mode: &str, knn: u64) -> u64 {
    let cube = pixels.saturating_mul(bands).saturating_mul(64);
    let clustering = if mode == "ward" { pixels.saturating_mul(pixels.saturating_sub(1)).saturating_mul(12) }
        else { pixels.saturating_mul(knn).saturating_mul(128) };
    cube.saturating_add(clustering).saturating_add(128 * 1024 * 1024)
}

fn validate_cube(cube: &Cube) -> Result<InputInfo> {
    ensure!(cube.planes.len() >= 2, "SAGUI requires at least two photometric bands");
    ensure!(card_text(&cube.header, "CTYPE3").is_some_and(|value| value.eq_ignore_ascii_case("FILTER")), "SAGUI requires CTYPE3=FILTER; a spectral cube is not a multiband preparation");
    ensure!(matches!(classify_unit(&cube.header).class, UnitClass::PerPixelPhysical { .. }), "SAGUI requires calibrated per-pixel flux units; convert surface brightness to Jy/pixel in Cube Prep");
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
    let mut warnings = vec!["Regional errors assume independent pixels; resampling and PSF matching create covariance that is not stored".into(), "External cubes require aligned bands and a verified common PSF; a target name alone does not validate matching".into()];
    if cube.err.is_none() { warnings.push("ERR is absent: regional uncertainties remain unknown".into()); }
    if cube.dq.is_none() { warnings.push("DQ is absent: only finite-value support can be checked".into()); }
    Ok(InputInfo { width, height, bands, unit: card_text(&cube.header, "BUNIT").unwrap_or_default(), has_err: cube.err.is_some(), has_dq: cube.dq.is_some(), estimated_memory_bytes: memory_estimate((width * height) as u64, cube.planes.len() as u64, "ward", 40), warnings })
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

fn load_input(path: &str) -> Result<(Cube, InputInfo)> {
    let reference = ImageRef::parse(path);
    ensure!(matches!(reference.plane, PlaneSelector::Auto), "Select the whole multiband FITS file; SAGUI identifies SCI/ERR/DQ by extension name");
    let cube = read_cube_file(path, &filter_registry())?.cube;
    let mut info = validate_cube(&cube)?;
    let (fits_path, _temporary) = resolve_single_image(&reference.path)?;
    let file = File::open(fits_path)?;
    for hdu in list_extensions(&file)? {
        let name = hdu.extname.as_deref().unwrap_or_default().trim().trim_matches('\'').to_uppercase();
        if name == "ERR" || name == "DQ" {
            ensure!((name == "ERR" && cube.err.is_some()) || (name == "DQ" && cube.dq.is_some()), "{name} was declared but could not be loaded; use a MEF with 3D SCI/ERR/DQ extensions");
            ensure!(hdu.naxis == 3 && hdu.naxis1 == info.width as i64 && hdu.naxis2 == info.height as i64 && hdu.naxis3 == info.bands.len() as i64, "{name} dimensions differ from SCI");
            info.warnings.extend(validate_companion_header(&cube, &extract_header_by_index(&file, hdu.index)?, &name)?);
        }
    }
    Ok((cube, info))
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
    ensure!(Path::new(&config.output_dir).is_absolute(), "Choose an absolute persistent output directory");
    Ok(())
}

fn eligible(cube: &Cube, bits: u32) -> Vec<bool> {
    let (height, width) = cube.dims();
    (0..height * width).map(|i| cube.planes.iter().all(|p| p[(i / width, i % width)].is_finite()) && cube.dq.as_ref().is_none_or(|dq| dq.planes.iter().all(|p| p[(i / width, i % width)] & bits == 0))).collect()
}

fn write_science(path: &Path, cube: &Cube, support: &[bool]) -> Result<()> {
    let mut file = BufWriter::new(File::create(path)?);
    for plane in &cube.planes {
        for (i, &value) in plane.iter().enumerate() {
            let value = if support[i] { f64::from(value) } else { f64::NAN };
            file.write_all(&value.to_le_bytes())?;
        }
    }
    file.flush()?;
    Ok(())
}

fn read_integers(path: &Path, pixels: usize) -> Result<Vec<u32>> {
    ensure!(fs::metadata(path)?.len() == (pixels as u64).saturating_mul(4), "SAGUI returned an incorrectly sized map");
    Ok(fs::read(path)?.chunks_exact(4).map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap())).collect())
}

fn validate_maps(labels: &[u32], mask: &[u32], support: &[bool], n_regions: usize) -> Result<()> {
    ensure!(labels.len() == support.len() && mask.len() == support.len(), "Map dimensions disagree");
    let mut seen = HashSet::new();
    for ((&label, &mask), &valid) in labels.iter().zip(mask).zip(support) {
        ensure!(label <= n_regions as u32 && mask <= 1, "Invalid SAGUI labels or support values");
        ensure!(label == 0 || (valid && mask == 1), "SAGUI assigned an excluded pixel to a region");
        if label > 0 { seen.insert(label); }
    }
    ensure!(seen.len() == n_regions, "SAGUI returned a different number of regions than requested");
    Ok(())
}

fn photometry(cube: &Cube, labels: &[u32]) -> Result<Vec<Region>> {
    let (_, width) = cube.dims();
    let UnitClass::PerPixelPhysical { jy_per_unit } = classify_unit(&cube.header).class else {
        anyhow::bail!("Regional photometry requires calibrated per-pixel flux units");
    };
    let mut regions: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (i, &label) in labels.iter().enumerate().filter(|(_, label)| **label > 0) { regions.entry(label).or_default().push(i); }
    regions.into_iter().map(|(region, indices)| {
        let bands = cube.planes.iter().zip(&cube.filters).enumerate().map(|(b, (plane, filter))| {
            let flux = indices.iter().map(|&i| f64::from(plane[(i / width, i % width)])).sum::<f64>() * jy_per_unit;
            ensure!(flux.is_finite(), "Nonfinite regional flux");
            let error = cube.err.as_ref().and_then(|err| {
                indices.iter().try_fold(0.0_f64, |sum, &i| {
                    let sigma = f64::from(err[b][(i / width, i % width)]);
                    (sigma.is_finite() && sigma >= 0.0).then_some(sum + sigma * sigma)
                }).filter(|v| v.is_finite()).map(|v| v.sqrt() * jy_per_unit).filter(|v| v.is_finite())
            });
            Ok(RegionalBand { filter: filter.filter.clone(), pivot_um: filter.pivot_um.context("Missing wavelength")?, flux, error })
        }).collect::<Result<Vec<_>>>()?;
        Ok(Region { region, n_pixels: indices.len(), bands })
    }).collect()
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

fn csv_text(text: &str) -> String { format!("\"{}\"", text.replace('"', "\"\"")) }

pub fn run(config: SaguiConfig, run_id: String, progress: &ProgressHandle) -> Result<SaguiResult> {
    ensure!(valid_run_id(&run_id), "Invalid SAGUI run identifier");
    ensure!(!progress.is_cancelled(), "SAGUI cancelled");
    let input = Path::new(&ImageRef::parse(&config.input_path).path).canonicalize()?;
    progress.report(0, 5, "Checking calibrated input and runtime");
    let input_hash = sha256(&input, progress)?;
    let (cube, info) = load_input(&config.input_path)?;
    let pixels = info.width * info.height;
    validate_config(&config, pixels)?;
    let support = eligible(&cube, config.dq_exclude_bits);
    let available = support.iter().filter(|&&valid| valid).count();
    ensure!(available >= config.n_regions, "Too few pixels remain after finite-value and DQ exclusion");
    let estimate = memory_estimate(pixels as u64, info.bands.len() as u64, &config.mode, config.knn_k as u64);
    if let Some(memory) = effective_memory() { ensure!(estimate <= memory.commit_available, "Segmentation needs an estimated {estimate} bytes; choose a smaller field or explicitly select sparse Ward"); }
    let runtime = runtime::inspect_with_progress(&config.rscript_path, &config.library_path, progress);
    ensure!(runtime.ready, "R environment is not ready: {}", runtime.issues.join("; "));
    fs::create_dir_all(&config.output_dir)?;
    let parent = Path::new(&config.output_dir).canonicalize()?;
    let work = parent.join(format!("sagui-{run_id}"));
    fs::create_dir(&work).context("The run directory already exists or cannot be created; choose a new run identifier")?;
    let result = run_in_dir(&config, &run_id, &work, &input, &input_hash, &cube, info, support, estimate, runtime, progress);
    if let Err(error) = &result { let _ = fs::write(work.join("failure.json"), serde_json::to_vec_pretty(&json!({ "run_id": run_id, "error": format!("{error:#}"), "published": false }))?); }
    result
}

fn run_in_dir(config: &SaguiConfig, run_id: &str, work: &Path, input: &Path, input_hash: &str, cube: &Cube, info: InputInfo, support: Vec<bool>, estimate: u64, runtime: runtime::RuntimeInfo, progress: &ProgressHandle) -> Result<SaguiResult> {
    fs::write(work.join("config.json"), serde_json::to_vec_pretty(config)?)?;
    let library = library_fingerprint(Path::new(&config.library_path), progress)?;
    write_science(&work.join("science.bin"), cube, &support)?;
    let exe_hash = sha256(Path::new(&config.rscript_path), progress)?;
    let mut command = runtime::command(&config.rscript_path, &config.library_path, work)?;
    command.arg("run").arg(Path::new(&config.library_path).canonicalize()?).arg(work)
        .arg(info.width.to_string()).arg(info.height.to_string()).arg(info.bands.len().to_string())
        .arg(config.n_regions.to_string()).arg(config.starlet.to_string())
        .arg(config.starlet_scales.iter().map(usize::to_string).collect::<Vec<_>>().join(","))
        .arg(config.starlet_threshold.to_string()).arg(&config.mode).arg(config.knn_k.to_string()).arg("1");
    progress.report(1, 5, "Running official SAGUI in isolated R");
    runtime::execute(command, work, progress, None)?;
    let actual_runtime = runtime::parse_runtime(&fs::read_to_string(work.join("stdout.log"))?)?;
    ensure!(actual_runtime.ready, "The R environment changed during execution");
    let labels = read_integers(&work.join("labels.bin"), support.len())?;
    let mask = read_integers(&work.join("support.bin"), support.len())?;
    validate_maps(&labels, &mask, &support, config.n_regions)?;
    progress.report(3, 5, "Measuring regional flux on the original calibrated cube");
    let regions = photometry(cube, &labels)?;
    let path = |name: &str| work.join(name).to_string_lossy().into_owned();
    let mut header = cube_base_header(&cube.header);
    header.set("BUNIT", "1".into());
    header.set("ABPROC", "sagui".into());
    let map = Array2::from_shape_vec((info.height, info.width), labels.clone())?;
    let mask = Array2::from_shape_vec((info.height, info.width), mask)?;
    write_derived_mef(&path("labels.fits"), None, &[ImageHdu { data: HduData::U32(&map), header: Some(&header), extname: "LABELS", extver: 1 }])?;
    write_derived_mef(&path("support.fits"), None, &[ImageHdu { data: HduData::U32(&mask), header: Some(&header), extname: "SUPPORT", extver: 1 }])?;
    let mut csv = BufWriter::new(File::create(work.join("regions.csv"))?);
    writeln!(csv, "region,n_pixels,filter,pivot_um,flux,error,unit")?;
    for region in &regions { for band in &region.bands {
        writeln!(csv, "{},{},{},{},{},{},Jy", region.region, region.n_pixels, csv_text(&band.filter), band.pivot_um, band.flux, band.error.map(|v| v.to_string()).unwrap_or_default())?;
    }}
    csv.flush()?;
    progress.report(4, 5, "Verifying input identity and output products");
    ensure!(!progress.is_cancelled(), "SAGUI cancelled; results were not published");
    ensure!(sha256(input, progress)? == input_hash, "The input cube changed during segmentation; results were not published");
    ensure!(sha256(Path::new(&config.rscript_path), progress)? == exe_hash, "Rscript changed during segmentation");
    ensure!(library_fingerprint(Path::new(&config.library_path), progress)? == library, "The private R library changed during segmentation");
    let result = SaguiResult { run_id: run_id.into(), output_dir: work.to_string_lossy().into_owned(), labels_path: path("labels.fits"), mask_path: path("support.fits"), table_path: path("regions.csv"), manifest_path: path("manifest.json"), width: info.width, height: info.height, unit: "Jy".into(), labels, regions, warnings: info.warnings };
    let files = ["labels.fits", "support.fits", "regions.csv", "config.json", "bridge.R", "science.bin", "labels.bin", "support.bin", "packages.tsv", "session.txt", "stdout.log", "stderr.log"].iter().map(|name| Ok(json!({ "path": name, "sha256": sha256(&work.join(name), progress)? }))).collect::<Result<Vec<_>>>()?;
    let manifest = json!({ "schema_version": 1, "status": "complete", "astroburst": env!("CARGO_PKG_VERSION"), "sagui_commit": runtime::SAGUI_COMMIT, "runtime": runtime, "rscript_sha256": exe_hash, "private_library": library, "input": { "path": input, "sha256": input_hash, "unit": info.unit }, "config": config, "array_order": "x,y,band; x contiguous", "region_photometry": "sum of original calibrated SCI converted to Jy; sqrt(sum(ERR^2)) converted to Jy; unknown if any used ERR invalid or absent", "covariance": "not represented", "estimated_memory_bytes": estimate, "platform": std::env::consts::OS, "files": files, "result": result });
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
