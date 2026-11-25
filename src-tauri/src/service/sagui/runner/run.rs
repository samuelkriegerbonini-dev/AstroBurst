use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use anyhow::{ensure, Context, Result};
use ndarray::Array2;
use serde_json::json;
use crate::core::cubeprep::align::NOISCORR_EXTNAME;
use crate::core::cubeprep::cube::{cube_base_header, Cube};
use crate::core::stacking::memory::{effective_memory, format_bytes, MemoryInfo};
use crate::infra::fits::writer::{HduData, ImageHdu};
use crate::infra::paths::plain;
use crate::infra::progress::ProgressHandle;
use crate::service::image_files::write_derived_mef;
use crate::types::image_ref::ImageRef;
use crate::service::sagui::bridge::{parse_backend, parse_bridge_markers, BridgeReport};
use crate::service::sagui::dq::{dq_exclusion_report, eligible};
use crate::service::sagui::input::load_input;
use super::integrity::{hash_input, installed_identity, library_fingerprint, sha256, LibraryIdentity};
use crate::service::sagui::maps::{check_connected, fragment_warning, map_counts, map_warnings, read_integers, support_per_region_warning, validate_maps};
use crate::service::sagui::memory::{footprint_memory, memory_estimate};
use crate::service::sagui::photometry::{photometry, regions_csv, REGION_PHOTOMETRY_NOTE};
use super::types::{InputInfo, NoiseCorrelation, SaguiConfig, SaguiResult, SED_SCALINGS};
use crate::service::sagui::{install, runtime};

pub const SAGUI_SEED: u32 = 1;

const RUN_STAGE: &str = "Running official SAGUI in isolated R";

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
    installed: Option<LibraryIdentity>,
}

pub fn valid_run_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub(in crate::service::sagui) fn validate_config(config: &SaguiConfig, pixels: usize) -> Result<()> {
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

pub(in crate::service::sagui) fn write_science(path: &Path, cube: &Cube, eligible: &[bool]) -> Result<()> {
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

pub(in crate::service::sagui) fn clustering_provenance(config: &SaguiConfig, bridge: &BridgeReport) -> serde_json::Value {
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

pub(in crate::service::sagui) fn create_run_dir(output_dir: &str, run_id: &str) -> Result<PathBuf> {
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
    let installed = installed_identity(Path::new(&config.rscript_path), Path::new(&config.library_path), progress)?;
    let runtime = runtime::inspect_with_progress(&config.rscript_path, &config.library_path, progress);
    ensure!(runtime.ready, "R environment is not ready: {}{}", runtime.issues.join("; "), install::r_version_hint(&runtime.issues));
    let work = create_run_dir(&config.output_dir, &run_id)?;
    let checked = CheckedRun { input, input_hash, cube, info, correlation, eligible, estimate, memory, runtime, installed };
    let result = run_in_dir(&config, &run_id, &work, checked, progress);
    if let Err(error) = &result { let _ = fs::write(work.join("failure.json"), serde_json::to_vec_pretty(&json!({ "run_id": run_id, "error": format!("{error:#}"), "published": false }))?); }
    result
}

fn run_in_dir(config: &SaguiConfig, run_id: &str, work: &Path, checked: CheckedRun, progress: &ProgressHandle) -> Result<SaguiResult> {
    let CheckedRun { input, input_hash, cube, info, correlation, eligible, estimate, memory, runtime, installed } = checked;
    let InputInfo { width, height, unit, mut warnings, .. } = info;
    fs::write(work.join("config.json"), serde_json::to_vec_pretty(config)?).with_context(|| format!("Cannot write {}", work.join("config.json").display()))?;
    let (exe_hash, library) = match installed {
        Some(identity) => (identity.rscript_sha256, identity.files),
        None => (sha256(Path::new(&config.rscript_path), progress)?, library_fingerprint(Path::new(&config.library_path), progress)?),
    };
    write_science(&work.join("science.bin"), &cube, &eligible)?;
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
    let result = SaguiResult { run_id: run_id.into(), output_dir: work.to_string_lossy().into_owned(), labels_path: path("labels.fits"), mask_path: path("support.fits"), table_path: path("regions.csv"), manifest_path: path("manifest.json"), width, height, unit: "Jy".into(), labels, labels_bin_path: path("labels.bin"), regions, warnings };
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
