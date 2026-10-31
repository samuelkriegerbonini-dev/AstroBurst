use std::collections::VecDeque;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use ndarray::{s, Array2, ArrayView2, Zip};
use serde_json::{json, Value};

use super::{canvas_spec, estimate_mosaic_request, run_mosaic_request, CanvasRequest, MosaicRequest, RejectRequest, TileSolveOptions};
use crate::cmd::astrometry::{
    add_online_tier, offline_tiers, GaiaFetch, NovaSettings, NovaSolve, SharedCancelCheck, SolveEnv, TierRequest, TierSet,
};
use crate::core::alignment::wcs_reproject::{prefilter_coarse_target, reproject_to_grid};
use crate::core::analysis::star_detection::{detect_stars, DetectedStar};
use crate::core::astrometry::catalog::{query_gaia_cached, ConeQuery};
use crate::core::astrometry::gwcs::test_support::{env_dir, mast_dir, real_data_dir, skip_if_absent, MAST_DIR_VAR};
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::stacking::memory::effective_memory;
use crate::core::stacking::mosaic::{
    canvas_wcs, load_tile_pixels, load_tiles, mosaic_estimate, run_mosaic_with, warp_to_window, window_frame, MosaicEstimate,
    MosaicStage, TileGeometry, TilePixels,
};
use crate::core::stacking::never_cancelled;
use crate::infra::catalog_store::{CatalogStore, StoreRoots};
use crate::infra::fits::reader::extract_header_by_index;
use crate::infra::fits::writer::{filter_header, write_fits_mono};
use crate::infra::image_source::load_plane;
use crate::infra::wcs_source::{header_wcs, load_wcs};
use crate::types::config::AppConfig;
use crate::types::header::HduHeader;
use crate::types::image_ref::ImageRef;
use crate::types::stacking::{LevelModel, MosaicConfig};

const I2D_DIR_VAR: &str = "ASTROBURST_MOSAIC_I2D_DIR";
const V2_FILE_VAR: &str = "ASTROBURST_MOSAIC_V2_FILE";
const V3_DIR_VAR: &str = "ASTROBURST_MOSAIC_V3_DIR";
const V4_DIR_VAR: &str = "ASTROBURST_MOSAIC_V4_DIR";
const ONLINE_VAR: &str = "ASTROBURST_MOSAIC_ONLINE";
const KEEP_DIR_VAR: &str = "ASTROBURST_MOSAIC_KEEP_DIR";
const PROTECTED_FOLDERS: [&str; 3] = ["fits-n3", "heavytest", "sample-data"];

const NRCA1_I2D: &str = "jw01475001001_02101_00001_nrca1_i2d.fits";
const NRCB1_I2D: &str = "jw01475001001_02101_00001_nrcb1_i2d.fits";
const NIRCAM_CAL: &str = "jw02739001001_02105_00001_nrca1_cal.fits";
const MIRI_CAL: &str = "jw02739002001_02101_00001_mirimage_cal.fits";
const V1_PREFIX: &str = "jw01475001001_02101_";
const V1_EXPOSURES: [&str; 2] = ["00001", "00002"];
const V1_DETECTORS: [&str; 8] = ["nrca1", "nrca2", "nrca3", "nrca4", "nrcb1", "nrcb2", "nrcb3", "nrcb4"];
const CAL_SUFFIX: &str = "_cal";
const CRF_SUFFIX: &str = "_o001_crf";
const V1_REFERENCES: [&str; 2] = ["_00001_nrca1_", "_00001_nrcb1_"];
const V1_INJECTED: [(&str, f64); 3] = [("_00002_nrca3_", 5.0), ("_00001_nrcb4_", -3.0), ("_00002_nrcb2_", 2.0)];
const V1_RUNTIME_S: f64 = 90.0;
const V1_REJECT_RUNTIME_S: f64 = 180.0;
const V1_PEAK_RSS_BYTES: u64 = 4_000_000_000;
const V1_SUMMARY: &str = "v1_summary.json";
const V3_WHOLE: &str = "whole.fits";
const V3_TILE_PREFIX: &str = "tile_";
const V3_HINT_SHIFT_DEG: f64 = 0.05;
const V3_PIXEL_UM: f64 = 10.0;
const ARCSEC_PER_RADIAN_MM_PER_UM: f64 = 206.264_806;
const FITS_EXTENSIONS: [&str; 3] = ["fits", "fit", "fts"];

const SCI_HDU: usize = 1;
const COVERAGE_HDU: usize = 2;
const SPLIT_PARTS: usize = 3;
const SPLIT_OVERLAP: f64 = 0.15;
const EXACT_TOLERANCE: f64 = 1e-6;
const V2_INJECTED: [(usize, f32, f64); 4] = [(4, 1.0, 5.0), (2, 1.0, -3.0), (7, 1.0, 2.0), (5, 0.98, 0.0)];
const SECOND_FILTER_GAIN: f32 = 0.5;
const LEVEL_TOLERANCE_SIGMA: f64 = 0.2;
const SCALE_TOLERANCE: f64 = 0.005;
const SEAM_TOLERANCE_SIGMA: f64 = 0.5;
const REGISTRATION_TOLERANCE_PX: f64 = 0.05;
const MIN_REGISTRATION_MATCHES: usize = 60;
const DETECTION_SIGMA: f64 = 5.0;
const STAR_SNR: f64 = 20.0;
const MIN_STARS: usize = 20;
const TARGET_STARS: usize = 50;
const ISOLATION_PX: f64 = 12.0;
const FLUX_TOLERANCE: f64 = 0.005;
const CENTROID_TOLERANCE_PX: f64 = 0.02;
const FLUX_APERTURE_PX: f64 = 6.0;
const CENTROID_APERTURE_PX: f64 = 3.0;
const SKY_ANNULUS_PX: (f64, f64) = (8.0, 11.0);
const CENTROID_ITERATIONS: usize = 5;
const MATCH_RADIUS_PX: f64 = 1.0;
const SEAM_PROFILES: usize = 100;
const SEAM_BAND_PX: usize = 16;
const SEAM_PATCH_PX: usize = 16;
const SEAM_LANE_STEP_PX: usize = 7;
const FORCED_AREA_RATIO: f64 = 1e6;
const V3_MEDIAN_PX: f64 = 0.25;
const V3_WORST_PX: f64 = 1.0;
const V3_MIN_STARS: usize = 50;
const RESIDUAL_WARNING: &str = "disagree by";

struct HarnessSky;

impl SolveEnv for HarnessSky {
    fn tiers(&self, request: &TierRequest<'_>) -> TierSet {
        let mut set = match StoreRoots::from_app(None) {
            Ok(roots) => offline_tiers(&CatalogStore::new(roots), &AppConfig::default(), request.use_twomass),
            Err(error) => TierSet { notes: vec![format!("the catalogue folders are not reachable: {error:#}")], ..TierSet::default() },
        };
        let online: Option<GaiaFetch> = cfg!(feature = "vizier")
            .then(|| Arc::new(|query: &ConeQuery| query_gaia_cached(query).map(|hit| hit.rows.as_ref().clone())) as GaiaFetch);
        let never: SharedCancelCheck = Arc::new(|| false);
        add_online_tier(&mut set, request, online, never);
        set
    }

    fn stored_api_key(&self) -> Option<String> {
        None
    }

    fn nova_settings(&self) -> anyhow::Result<NovaSettings> {
        anyhow::bail!("the mosaic harness never sends a tile to astrometry.net")
    }

    fn nova(&self) -> Option<NovaSolve> {
        None
    }

    fn cancelled(&self) -> bool {
        false
    }

    fn stage(&self, _: &str) {}
}

fn no_report(_: u64, _: u64, _: &str) {}

fn no_progress(_: MosaicStage, _: usize, _: usize) {}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn hdu(path: &str, index: usize) -> HduHeader {
    extract_header_by_index(&File::open(path).unwrap(), index).unwrap()
}

fn plane_of(path: &str, index: usize) -> Array2<f32> {
    load_plane(&ImageRef::parse(&format!("{path}#hdu={index}"))).unwrap().arr
}

fn primary_plane(path: &str) -> Array2<f32> {
    load_plane(&ImageRef::parse(path)).unwrap().arr
}

fn real_file(dir: &Path, name: &str) -> Option<String> {
    let path = dir.join(name);
    (!skip_if_absent(&path)).then(|| text(&path))
}

fn v2_i2d() -> Option<String> {
    let path = std::env::var_os(V2_FILE_VAR).filter(|value| !value.is_empty()).map_or_else(|| env_dir(I2D_DIR_VAR).join(NRCA1_I2D), PathBuf::from);
    (!skip_if_absent(&path)).then(|| text(&path))
}

fn describe_memory(label: &str) {
    match process_memory() {
        Some(memory) => println!(
            "{label} memory: working set {:.3} GB, peak working set {:.3} GB",
            memory.current as f64 / 1e9,
            memory.peak as f64 / 1e9
        ),
        None => println!("{label} memory: this platform reports no working set"),
    }
}

fn online_allowed() -> bool {
    std::env::var(ONLINE_VAR).is_ok_and(|value| value.trim() == "1")
}

fn is_protected(path: &Path) -> bool {
    let lowered = text(path).to_ascii_lowercase().replace('\\', "/");
    lowered.split('/').any(|part| PROTECTED_FOLDERS.contains(&part))
}

struct Workspace {
    path: PathBuf,
    _temp: Option<tempfile::TempDir>,
}

fn workspace(label: &str) -> Workspace {
    let workspace = match std::env::var_os(KEEP_DIR_VAR).filter(|value| !value.is_empty()) {
        Some(root) => {
            let path = PathBuf::from(root).join(label);
            std::fs::create_dir_all(&path).unwrap();
            Workspace { path, _temp: None }
        }
        None => {
            let temp = tempfile::tempdir().unwrap();
            Workspace { path: temp.path().to_path_buf(), _temp: Some(temp) }
        }
    };
    assert!(!is_protected(&workspace.path), "the harness never writes inside a data folder: {}", workspace.path.display());
    workspace
}

fn fits_files(dir: &Path, prefix: &str) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|path| {
                    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                    let extension = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                    path.is_file() && name.starts_with(prefix) && FITS_EXTENSIONS.contains(&extension.as_str())
                })
                .map(|path| text(&path))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn estimate_paths(paths: &[String], request: &MosaicRequest) -> Value {
    estimate_mosaic_request(paths, request, &HarnessSky, effective_memory(), &never_cancelled, &no_report).unwrap()
}

struct MosaicRun {
    result: Value,
    seconds: f64,
    fits: String,
}

impl MosaicRun {
    fn planes(&self) -> (Array2<f32>, Array2<f32>) {
        (plane_of(&self.fits, SCI_HDU), plane_of(&self.fits, COVERAGE_HDU))
    }
}

fn run_paths(dir: &Path, paths: &[String], request: &MosaicRequest, name: &str) -> MosaicRun {
    let started = Instant::now();
    let result = run_mosaic_request(
        paths,
        &text(&dir.join("out")),
        Some(name),
        request,
        &HarnessSky,
        effective_memory(),
        &never_cancelled,
        &no_report,
    )
    .unwrap_or_else(|error| panic!("{name}: {error:#}"));
    let seconds = started.elapsed().as_secs_f64();
    let fits = result["fits_path"].as_str().unwrap().to_string();
    MosaicRun { result, seconds, fits }
}

struct Plan {
    tiles: Vec<TileGeometry>,
    config: MosaicConfig,
    estimate: MosaicEstimate,
}

fn plan_of(paths: &[String], request: &MosaicRequest) -> Plan {
    let tiles = load_tiles(paths).unwrap();
    let config = request.config(canvas_spec(&request.canvas).unwrap());
    let estimate = mosaic_estimate(&tiles, &config, None).unwrap();
    Plan { tiles, config, estimate }
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mid = values.len() / 2;
    *values.select_nth_unstable_by(mid, f64::total_cmp).1
}

fn peak_of(values: &Array2<f32>) -> f64 {
    values.iter().filter(|v| v.is_finite()).fold(0.0, |m, &v| m.max(f64::from(v).abs()))
}

fn noise_sigma(plane: &Array2<f32>) -> f64 {
    detect_stars(plane, DETECTION_SIGMA).background_sigma
}

fn numbers(value: &Value) -> Vec<f64> {
    value.as_array().map_or_else(Vec::new, |items| items.iter().map(|v| v.as_f64().unwrap_or(f64::NAN)).collect())
}

fn indices(value: &Value) -> Vec<usize> {
    value.as_array().map_or_else(Vec::new, |items| items.iter().filter_map(Value::as_u64).map(|v| v as usize).collect())
}

fn names_of(value: &Value) -> Vec<String> {
    value["tiles"].as_array().into_iter().flatten().map(|tile| tile["name"].as_str().unwrap_or_default().to_string()).collect()
}

fn solved_levels(result: &Value) -> (Vec<f64>, Vec<f64>) {
    let plane = &result["levels"][0];
    (numbers(&plane["offsets"]), numbers(&plane["scales"]))
}

fn warnings_of(result: &Value) -> Vec<String> {
    result["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

fn describe_estimate(label: &str, estimate: &Value) {
    println!(
        "{label}: canvas {}x{} ({:.1} Mpx, about {:.2} GB), {} pairs, components {}, isolated {}, refusal {}",
        estimate["cols"],
        estimate["rows"],
        estimate["megapixels"].as_f64().unwrap_or(f64::NAN),
        estimate["ram_bytes"].as_f64().unwrap_or(f64::NAN) / 1e9,
        estimate["pairs"].as_array().map_or(0, Vec::len),
        estimate["components"],
        estimate["isolated"],
        estimate["refusal"]
    );
    for (k, tile) in estimate["tiles"].as_array().into_iter().flatten().enumerate() {
        println!(
            "{label}: tile {k} {} {} scale x{:.4} rotation {:.3} deg coverage {:.4} window {}",
            tile["name"],
            tile["wcs_kind"],
            tile["scale_ratio"].as_f64().unwrap_or(f64::NAN),
            tile["rotation_deg"].as_f64().unwrap_or(f64::NAN),
            tile["coverage_frac"].as_f64().unwrap_or(f64::NAN),
            tile["window"]
        );
    }
    for pair in estimate["pairs"].as_array().into_iter().flatten() {
        println!("{label}: pair {} {} overlap {:.4}", pair["i"], pair["j"], pair["frac"].as_f64().unwrap_or(f64::NAN));
    }
    for warning in warnings_of(estimate) {
        println!("{label}: warning {warning}");
    }
}

fn describe_run(label: &str, run: &MosaicRun) {
    let result = &run.result;
    println!(
        "{label}: {:.2} s wall (elapsed_ms {}), canvas {}x{}, reject {}, rejected_px {}",
        run.seconds, result["elapsed_ms"], result["canvas"]["cols"], result["canvas"]["rows"], result["reject"], result["rejected_px"]
    );
    for residual in result["pair_residuals"].as_array().into_iter().flatten() {
        println!(
            "{label}: residual {} {} dx {:+.4} dy {:+.4} px, matched {}, rms {:.4} px",
            residual["i"],
            residual["j"],
            residual["dx"].as_f64().unwrap_or(f64::NAN),
            residual["dy"].as_f64().unwrap_or(f64::NAN),
            residual["matched"],
            residual["rms_px"].as_f64().unwrap_or(f64::NAN)
        );
    }
    for warning in warnings_of(result) {
        println!("{label}: warning {warning}");
    }
}

struct Source {
    sci: Array2<f32>,
    header: HduHeader,
    wcs: WcsTransform,
    sigma: f64,
    stars: Vec<DetectedStar>,
}

fn source(path: &str) -> Source {
    let header = hdu(path, SCI_HDU);
    let sci = plane_of(path, SCI_HDU);
    let wcs = load_wcs(path, &header).unwrap();
    let detection = detect_stars(&sci, DETECTION_SIGMA);
    Source { sci, header, wcs, sigma: detection.background_sigma, stars: detection.stars }
}

#[derive(Debug, Clone, Copy)]
struct Part {
    x0: usize,
    y0: usize,
    cols: usize,
    rows: usize,
}

fn spans(len: usize) -> Vec<(usize, usize)> {
    let parts = SPLIT_PARTS as f64;
    let side = (len as f64 / (parts - (parts - 1.0) * SPLIT_OVERLAP)).ceil() as usize;
    (0..SPLIT_PARTS).map(|k| (((len - side) as f64 * k as f64 / (parts - 1.0)).round() as usize, side)).collect()
}

fn split_parts((rows, cols): (usize, usize)) -> Vec<Part> {
    let (ys, xs) = (spans(rows), spans(cols));
    ys.iter().flat_map(|&(y0, h)| xs.iter().map(move |&(x0, w)| Part { x0, y0, cols: w, rows: h })).collect()
}

fn part_header(source: &HduHeader, part: &Part) -> HduHeader {
    let mut header = source.clone();
    for key in ["S_REGION", "EXTNAME", "EXTVER"] {
        header.remove(key);
    }
    for (key, shift) in [("CRPIX1", part.x0), ("CRPIX2", part.y0)] {
        let crpix = source.get_f64(key).unwrap();
        header.set_f64(key, crpix - shift as f64);
    }
    header
}

fn write_parts(dir: &Path, source: &Source, parts: &[Part], stem: &str, level: impl Fn(usize) -> (f32, f32)) -> Vec<String> {
    std::fs::create_dir_all(dir).unwrap();
    parts
        .iter()
        .enumerate()
        .map(|(k, part)| {
            let (gain, offset) = level(k);
            let pixels = source.sci.slice(s![part.y0..part.y0 + part.rows, part.x0..part.x0 + part.cols]).mapv(|v| gain * v + offset);
            let path = text(&dir.join(format!("{stem}_r{}c{}.fits", k / SPLIT_PARTS, k % SPLIT_PARTS)));
            write_fits_mono(&path, &pixels, Some(&part_header(&source.header, part))).unwrap();
            path
        })
        .collect()
}

fn grid_shift(source: &HduHeader, canvas: &HduHeader) -> (i64, i64) {
    let shift = |key: &str| {
        let delta = source.get_f64(key).unwrap() - canvas.get_f64(key).unwrap();
        assert!((delta - delta.round()).abs() < 1e-6, "{key} moved by {delta} px, not by whole pixels");
        delta.round() as i64
    };
    (shift("CRPIX1"), shift("CRPIX2"))
}

fn border_connected(invalid: &Array2<bool>) -> Array2<bool> {
    let (rows, cols) = invalid.dim();
    let mut reached = Array2::from_elem((rows, cols), false);
    let mut queue: VecDeque<(usize, usize)> = VecDeque::new();
    let edge = (0..cols).flat_map(|x| [(0, x), (rows - 1, x)]).chain((0..rows).flat_map(|y| [(y, 0), (y, cols - 1)]));
    for pixel in edge {
        if invalid[pixel] && !reached[pixel] {
            reached[pixel] = true;
            queue.push_back(pixel);
        }
    }
    while let Some((y, x)) = queue.pop_front() {
        for (ny, nx) in [(y.wrapping_sub(1), x), (y + 1, x), (y, x.wrapping_sub(1)), (y, x + 1)] {
            if ny < rows && nx < cols && invalid[[ny, nx]] && !reached[[ny, nx]] {
                reached[[ny, nx]] = true;
                queue.push_back((ny, nx));
            }
        }
    }
    reached
}

#[derive(Debug, Default)]
struct Reassembly {
    compared: usize,
    worst: f64,
    peak: f64,
    padding: usize,
    leaked: usize,
    holes: usize,
    rim: usize,
    filled: usize,
    filled_at_rim: usize,
    lost: usize,
}

fn beside_finite(plane: &Array2<f32>, y: usize, x: usize) -> bool {
    let (rows, cols) = plane.dim();
    (y.saturating_sub(1)..(y + 2).min(rows)).any(|ny| (x.saturating_sub(1)..(x + 2).min(cols)).any(|nx| plane[[ny, nx]].is_finite()))
}

fn reassembly(source: &Array2<f32>, values: &Array2<f32>, coverage: &Array2<f32>, (dx, dy): (i64, i64)) -> Reassembly {
    let padding = border_connected(&source.mapv(|v| !v.is_finite()));
    let mut found = Reassembly { peak: peak_of(source), ..Reassembly::default() };
    for ((y, x), &original) in source.indexed_iter() {
        let (cy, cx) = (y as i64 - dy, x as i64 - dx);
        let inside = cy >= 0 && cx >= 0 && (cy as usize) < values.nrows() && (cx as usize) < values.ncols();
        let (value, count) = if inside { (values[[cy as usize, cx as usize]], coverage[[cy as usize, cx as usize]]) } else { (f32::NAN, 0.0) };
        match (original.is_finite(), value.is_finite()) {
            (true, true) => {
                found.compared += 1;
                found.worst = found.worst.max((f64::from(value) - f64::from(original)).abs());
            }
            (true, false) => found.lost += 1,
            (false, finite) if padding[[y, x]] => {
                found.padding += 1;
                if finite || count > 0.0 {
                    found.leaked += 1;
                }
            }
            (false, finite) => {
                let at_rim = beside_finite(source, y, x);
                found.holes += 1;
                found.rim += usize::from(at_rim);
                if finite {
                    found.filled += 1;
                    found.filled_at_rim += usize::from(at_rim);
                }
            }
        }
    }
    found
}

#[derive(Debug, Default)]
struct Identity {
    compared: usize,
    worst: f64,
    peak: f64,
    only_one: usize,
}

fn identity(expected: &Array2<f32>, found: &Array2<f32>) -> Identity {
    let mut identity = Identity { peak: peak_of(expected), ..Identity::default() };
    Zip::from(expected).and(found).for_each(|&a, &b| match (a.is_finite(), b.is_finite()) {
        (true, true) => {
            identity.compared += 1;
            identity.worst = identity.worst.max((f64::from(a) - f64::from(b)).abs());
        }
        (false, false) => {}
        _ => identity.only_one += 1,
    });
    identity
}

fn masked_plane(pixels: TilePixels) -> Array2<f32> {
    let TilePixels { mut planes, excluded } = pixels;
    let mut plane = planes.swap_remove(0);
    if let Some(excluded) = excluded {
        Zip::from(&mut plane).and(&excluded).for_each(|value, &flag| {
            if flag != 0 {
                *value = f32::NAN;
            }
        });
    }
    plane
}

fn tile_plane(plan: &Plan, k: usize) -> Array2<f32> {
    masked_plane(load_tile_pixels(&plan.tiles[k], plan.config.dq_mask).unwrap())
}

fn corrected_windows(plan: &Plan, offsets: &[f64], scales: &[f64]) -> Vec<Array2<f32>> {
    plan.tiles
        .iter()
        .zip(&plan.estimate.tiles)
        .enumerate()
        .map(|(k, (tile, row))| {
            let frame = window_frame(&plan.estimate.canvas_cards, &row.window).unwrap();
            let (scale, offset) = (scales[k], offsets[k]);
            warp_to_window(&tile_plane(plan, k), tile, &frame, &row.window).mapv(|v| (f64::from(v) / scale - offset) as f32)
        })
        .collect()
}

#[derive(Debug)]
struct PairLevel {
    i: usize,
    j: usize,
    pixels: usize,
    step_sigma: f64,
}

fn overlap_levels(estimate: &MosaicEstimate, windows: &[Array2<f32>], sigma: f64) -> Vec<PairLevel> {
    estimate
        .pairs
        .iter()
        .map(|pair| {
            let (a, b) = (&estimate.tiles[pair.i].window, &estimate.tiles[pair.j].window);
            let (x0, y0) = (a.x0.max(b.x0), a.y0.max(b.y0));
            let (x1, y1) = ((a.x0 + a.cols).min(b.x0 + b.cols), (a.y0 + a.rows).min(b.y0 + b.rows));
            let mut differences = Vec::new();
            for y in y0..y1 {
                for x in x0..x1 {
                    let (va, vb) = (windows[pair.i][[y - a.y0, x - a.x0]], windows[pair.j][[y - b.y0, x - b.x0]]);
                    if va.is_finite() && vb.is_finite() {
                        differences.push(f64::from(va) - f64::from(vb));
                    }
                }
            }
            PairLevel { i: pair.i, j: pair.j, pixels: differences.len(), step_sigma: median(&mut differences) / sigma }
        })
        .collect()
}

#[derive(Debug)]
struct SeamProfiles {
    found: usize,
    used: usize,
    mean_sigma: f64,
    median_abs_sigma: f64,
    worst_abs_sigma: f64,
}

impl SeamProfiles {
    fn describe(&self) -> String {
        format!(
            "{} cross-border profiles used of {} found: mean step {:+.4} sigma, median |step| {:.4} sigma, worst |step| {:.4} sigma",
            self.used, self.found, self.mean_sigma, self.median_abs_sigma, self.worst_abs_sigma
        )
    }
}

fn lane_profiles(values: ArrayView2<f32>, coverage: ArrayView2<f32>, feather_px: usize, steps: &mut Vec<f64>) {
    let (rows, cols) = values.dim();
    let half = SEAM_BAND_PX / 2;
    let reach = feather_px + SEAM_PATCH_PX;
    for y in (half..rows.saturating_sub(half)).step_by(SEAM_LANE_STEP_PX) {
        for x in 1..cols {
            let (left, right) = (coverage[[y, x - 1]], coverage[[y, x]]);
            if left == right || left <= 0.0 || right <= 0.0 {
                continue;
            }
            let sides = if right > left {
                (x >= SEAM_PATCH_PX && x + reach <= cols).then(|| ((x - SEAM_PATCH_PX, x, left), (x + feather_px, x + reach, right)))
            } else {
                (x >= reach && x + SEAM_PATCH_PX <= cols).then(|| ((x, x + SEAM_PATCH_PX, right), (x - reach, x - feather_px, left)))
            };
            let Some((near, far)) = sides else {
                continue;
            };
            let patch = |(from, to, count): (usize, usize, f32)| -> Option<f64> {
                let mut samples = Vec::with_capacity(SEAM_BAND_PX * SEAM_PATCH_PX);
                for row in y - half..y + half {
                    for col in from..to {
                        if coverage[[row, col]] != count {
                            return None;
                        }
                        let value = f64::from(values[[row, col]]);
                        if value.is_finite() {
                            samples.push(value);
                        }
                    }
                }
                (!samples.is_empty()).then(|| median(&mut samples))
            };
            if let (Some(before), Some(after)) = (patch(near), patch(far)) {
                steps.push(after - before);
            }
        }
    }
}

fn seam_profiles(values: &Array2<f32>, coverage: &Array2<f32>, sigma: f64, feather_px: usize) -> SeamProfiles {
    let mut steps = Vec::new();
    lane_profiles(values.view(), coverage.view(), feather_px, &mut steps);
    lane_profiles(values.t(), coverage.t(), feather_px, &mut steps);
    let found = steps.len();
    let stride = found.div_ceil(SEAM_PROFILES).max(1);
    let used: Vec<f64> = steps.iter().step_by(stride).map(|step| step / sigma).collect();
    let mut magnitudes: Vec<f64> = used.iter().map(|step| step.abs()).collect();
    let worst = magnitudes.iter().copied().fold(0.0, f64::max);
    SeamProfiles {
        found,
        used: used.len(),
        mean_sigma: used.iter().sum::<f64>() / used.len() as f64,
        median_abs_sigma: median(&mut magnitudes),
        worst_abs_sigma: worst,
    }
}

#[derive(Debug, Clone, Copy)]
struct Aperture {
    flux: f64,
    x: f64,
    y: f64,
}

fn disc(image: &Array2<f32>, (x0, y0): (f64, f64), (inner, outer): (f64, f64)) -> Option<Vec<(f64, f64, f64)>> {
    let (rows, cols) = image.dim();
    let reach = outer.ceil() + 1.0;
    let (first_x, last_x, first_y, last_y) = ((x0 - reach).floor(), (x0 + reach).ceil(), (y0 - reach).floor(), (y0 + reach).ceil());
    if !(first_x >= 0.0 && first_y >= 0.0 && last_x < cols as f64 && last_y < rows as f64) {
        return None;
    }
    let mut samples = Vec::new();
    for y in first_y as usize..=last_y as usize {
        for x in first_x as usize..=last_x as usize {
            let r = (x as f64 - x0).hypot(y as f64 - y0);
            if r >= inner && r <= outer {
                let value = f64::from(image[[y, x]]);
                if !value.is_finite() {
                    return None;
                }
                samples.push((x as f64, y as f64, value));
            }
        }
    }
    Some(samples)
}

fn aperture(image: &Array2<f32>, guess: (f64, f64)) -> Option<Aperture> {
    let mut sky: Vec<f64> = disc(image, guess, SKY_ANNULUS_PX)?.into_iter().map(|(_, _, v)| v).collect();
    let background = median(&mut sky);
    let mut centre = guess;
    for _ in 0..CENTROID_ITERATIONS {
        let (mut sum, mut sx, mut sy) = (0.0, 0.0, 0.0);
        for (x, y, value) in disc(image, centre, (0.0, CENTROID_APERTURE_PX))? {
            let weight = value - background;
            sum += weight;
            sx += weight * x;
            sy += weight * y;
        }
        if !(sum > 0.0) {
            return None;
        }
        centre = (sx / sum, sy / sum);
    }
    let flux: f64 = disc(image, centre, (0.0, FLUX_APERTURE_PX))?.into_iter().map(|(_, _, v)| v - background).sum();
    (flux > 0.0).then_some(Aperture { flux, x: centre.0, y: centre.1 })
}

fn isolated_bright(stars: &[DetectedStar]) -> Vec<&DetectedStar> {
    stars
        .iter()
        .enumerate()
        .filter(|(k, star)| {
            star.snr >= STAR_SNR
                && stars.iter().enumerate().all(|(j, other)| j == *k || (other.x - star.x).hypot(other.y - star.y) > ISOLATION_PX)
        })
        .map(|(_, star)| star)
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct StarPair {
    ratio: f64,
    dx: f64,
    dy: f64,
}

fn star_pairs(
    image: &Array2<f32>,
    wcs: &WcsTransform,
    stars: &[DetectedStar],
    mosaic: &Array2<f32>,
    canvas: &WcsTransform,
    limit: usize,
) -> Vec<StarPair> {
    let mut pairs = Vec::new();
    for star in isolated_bright(stars) {
        if pairs.len() == limit {
            break;
        }
        let Some(original) = aperture(image, (star.x, star.y)) else {
            continue;
        };
        let sky = wcs.pixel_to_world(original.x, original.y);
        let predicted = canvas.world_to_pixel(sky.ra, sky.dec);
        let Some(found) = aperture(mosaic, predicted) else {
            continue;
        };
        pairs.push(StarPair { ratio: found.flux / original.flux, dx: found.x - predicted.0, dy: found.y - predicted.1 });
    }
    pairs
}

#[derive(Debug, Clone, Copy)]
struct Shift {
    matched: usize,
    dx: f64,
    dy: f64,
}

impl Shift {
    fn magnitude(&self) -> f64 {
        self.dx.hypot(self.dy)
    }
}

fn bright_positions(plane: &Array2<f32>, to_canvas: impl Fn(f64, f64) -> (f64, f64)) -> Vec<(f64, f64)> {
    detect_stars(plane, DETECTION_SIGMA).stars.iter().filter(|star| star.snr >= STAR_SNR).map(|star| to_canvas(star.x, star.y)).collect()
}

fn nearest_offsets(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    a.iter()
        .filter_map(|&(ax, ay)| {
            b.iter()
                .map(|&(bx, by)| ((bx - ax).hypot(by - ay), (bx - ax, by - ay)))
                .min_by(|p, q| p.0.total_cmp(&q.0))
                .filter(|(distance, _)| *distance <= MATCH_RADIUS_PX)
                .map(|(_, delta)| delta)
        })
        .collect()
}

fn shift_of(offsets: &[(f64, f64)]) -> Shift {
    let (mut xs, mut ys): (Vec<f64>, Vec<f64>) = offsets.iter().copied().unzip();
    Shift { matched: offsets.len(), dx: median(&mut xs), dy: median(&mut ys) }
}

#[derive(Debug)]
struct PairShift {
    i: usize,
    j: usize,
    input: Shift,
    canvas: Shift,
}

#[derive(Debug)]
struct ComponentShift {
    reference: usize,
    input: Shift,
    canvas: Shift,
    pairs: Vec<PairShift>,
}

fn component_shift(estimate: &MosaicEstimate, members: &[usize], reference: usize, input: &[Vec<(f64, f64)>], canvas: &[Vec<(f64, f64)>]) -> ComponentShift {
    let (mut pooled_input, mut pooled_canvas) = (Vec::new(), Vec::new());
    let mut pairs = Vec::new();
    for pair in estimate.pairs.iter().filter(|p| members.contains(&p.i) && members.contains(&p.j)) {
        let (from_input, from_canvas) = (nearest_offsets(&input[pair.i], &input[pair.j]), nearest_offsets(&canvas[pair.i], &canvas[pair.j]));
        pairs.push(PairShift { i: pair.i, j: pair.j, input: shift_of(&from_input), canvas: shift_of(&from_canvas) });
        pooled_input.extend(from_input);
        pooled_canvas.extend(from_canvas);
    }
    ComponentShift { reference, input: shift_of(&pooled_input), canvas: shift_of(&pooled_canvas), pairs }
}

fn registration(plan: &Plan, windows: &[Array2<f32>]) -> Vec<ComponentShift> {
    let frame = canvas_wcs(&plan.estimate.canvas_cards).unwrap();
    let input: Vec<Vec<(f64, f64)>> = plan
        .tiles
        .iter()
        .enumerate()
        .map(|(k, tile)| {
            bright_positions(&tile_plane(plan, k), |x, y| {
                let sky = tile.wcs.pixel_to_world(x, y);
                frame.world_to_pixel(sky.ra, sky.dec)
            })
        })
        .collect();
    let canvas: Vec<Vec<(f64, f64)>> = windows
        .iter()
        .zip(&plan.estimate.tiles)
        .map(|(window, row)| {
            let (x0, y0) = (row.window.x0 as f64, row.window.y0 as f64);
            bright_positions(window, |x, y| (x + x0, y + y0))
        })
        .collect();
    plan.estimate
        .components
        .iter()
        .map(|component| component_shift(&plan.estimate, &component.tiles, component.reference, &input, &canvas))
        .collect()
}

fn aligned_by_wcs(target: &str, reference: &str) -> Array2<f32> {
    let (target_header, reference_header) = (hdu(target, SCI_HDU), hdu(reference, SCI_HDU));
    let target_wcs = load_wcs(target, &target_header).unwrap();
    let reference_wcs = load_wcs(reference, &reference_header).unwrap();
    let (rows, cols) = plane_of(reference, SCI_HDU).dim();
    let target_plane = plane_of(target, SCI_HDU);
    let ratio = reference_wcs.pixel_scale_arcsec() / target_wcs.pixel_scale_arcsec();
    match prefilter_coarse_target(&target_plane, &target_header, ratio) {
        Some((reduced, header, _)) => reproject_to_grid(&reduced, &header_wcs(&header).unwrap(), &reference_wcs, rows, cols).0,
        None => reproject_to_grid(&target_plane, &target_wcs, &reference_wcs, rows, cols).0,
    }
}

fn coverage_agreement(values: &Array2<f32>, coverage: &Array2<f32>) -> (usize, usize, usize) {
    let mut found = (0, 0, 0);
    Zip::from(values).and(coverage).for_each(|&value, &count| {
        if count > 0.0 {
            found.0 += 1;
            if !value.is_finite() {
                found.1 += 1;
            }
        } else if value.is_finite() {
            found.2 += 1;
        }
    });
    found
}

#[derive(Debug, Clone, Copy)]
struct ProcessMemory {
    current: u64,
    peak: u64,
}

#[cfg(windows)]
fn process_memory() -> Option<ProcessMemory> {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn K32GetProcessMemoryInfo(process: *mut std::ffi::c_void, counters: *mut Counters, size: u32) -> i32;
    }
    let mut counters = Counters { cb: std::mem::size_of::<Counters>() as u32, ..Counters::default() };
    let size = counters.cb;
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
    (ok != 0).then_some(ProcessMemory { current: counters.working_set_size as u64, peak: counters.peak_working_set_size as u64 })
}

#[cfg(target_os = "linux")]
fn process_memory() -> Option<ProcessMemory> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let bytes = |key: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
            .map(|kib| kib * 1024)
    };
    Some(ProcessMemory { current: bytes("VmRSS:")?, peak: bytes("VmHWM:")? })
}

#[cfg(not(any(windows, target_os = "linux")))]
fn process_memory() -> Option<ProcessMemory> {
    None
}

fn v1_paths(suffix: &str) -> Option<Vec<String>> {
    let dir = mast_dir();
    let paths: Vec<PathBuf> = V1_EXPOSURES
        .iter()
        .flat_map(|exposure| V1_DETECTORS.iter().map(move |detector| (exposure, detector)))
        .map(|(exposure, detector)| dir.join(format!("{V1_PREFIX}{exposure}_{detector}{suffix}.fits")))
        .collect();
    let missing: Vec<&PathBuf> = paths.iter().filter(|path| !path.is_file()).collect();
    if let Some(first) = missing.first() {
        eprintln!("skipped: {} of {} V1 tiles absent under {MAST_DIR_VAR}, first {}", missing.len(), paths.len(), first.display());
        return None;
    }
    Some(paths.iter().map(|path| text(path)).collect())
}

fn hinted_copy(path: &str, dir: &Path, shift_deg: f64) -> String {
    let header = hdu(path, 0);
    let plane = primary_plane(path);
    let wcs = load_wcs(path, &header).unwrap();
    let (rows, cols) = plane.dim();
    let centre = wcs.pixel_to_world((cols as f64 - 1.0) / 2.0, (rows as f64 - 1.0) / 2.0);
    let mut hinted = filter_header(&header, false, true).unwrap_or_else(HduHeader::empty);
    hinted.remove("RADECSYS");
    hinted.set_f64("RA", centre.ra + shift_deg / centre.dec.to_radians().cos());
    hinted.set_f64("DEC", centre.dec + shift_deg);
    hinted.set_f64("XPIXSZ", V3_PIXEL_UM);
    hinted.set_f64("YPIXSZ", V3_PIXEL_UM);
    hinted.set_f64("FOCALLEN", ARCSEC_PER_RADIAN_MM_PER_UM * V3_PIXEL_UM / wcs.pixel_scale_arcsec());
    std::fs::create_dir_all(dir).unwrap();
    let name = Path::new(path).file_name().unwrap().to_string_lossy().into_owned();
    let copy = text(&dir.join(name));
    write_fits_mono(&copy, &plane, Some(&hinted)).unwrap();
    copy
}

#[test]
#[ignore = "needs ASTROBURST_MOSAIC_V2_FILE with an i2d, or ASTROBURST_MOSAIC_I2D_DIR with jw01475001001_02101_00001_nrca1_i2d.fits"]
fn v2_split_on_the_i2d_grid_reassembles_it_and_keeps_its_nan_padding() {
    let Some(i2d) = v2_i2d() else {
        return;
    };
    let source = source(&i2d);
    let parts = split_parts(source.sci.dim());
    let work = workspace("v2-exact");
    let paths = write_parts(&work.path.join("tiles"), &source, &parts, "v2", |_| (1.0, 0.0));
    let request = MosaicRequest { canvas: CanvasRequest::Like { path: i2d.clone() }, reject: RejectRequest::Off, ..MosaicRequest::default() };
    let run = run_paths(&work.path, &paths, &request, "v2_exact");
    let (values, coverage) = run.planes();
    let shift = grid_shift(&source.header, &hdu(&run.fits, SCI_HDU));
    let found = reassembly(&source.sci, &values, &coverage, shift);
    let seams = seam_profiles(&values, &coverage, source.sigma, request.feather_px as usize);
    let (rows, cols) = source.sci.dim();
    println!("V2a source {cols}x{rows}, sigma {:.4e}, parts {parts:?}", source.sigma);
    describe_run("V2a", &run);
    println!("V2a canvas {}x{}, whole-pixel shift {shift:?}", values.ncols(), values.nrows());
    println!(
        "V2a compared {} px: worst |diff| {:.3e} against peak {:.4e}, ratio {:.3e} (tolerance {EXACT_TOLERANCE:e})",
        found.compared,
        found.worst,
        found.peak,
        found.worst / found.peak
    );
    println!(
        "V2a NaN padding {} px, {} of them finite or covered (tolerance 0); interior NaN {} px, {} filled; finite pixels lost {}",
        found.padding, found.leaked, found.holes, found.filled, found.lost
    );
    println!(
        "V2a interior NaN beside a finite pixel {} px, {} of them filled; deeper interior NaN {} px, {} of them filled",
        found.rim,
        found.filled_at_rim,
        found.holes - found.rim,
        found.filled - found.filled_at_rim
    );
    println!("V2a seams: {}", seams.describe());
    describe_memory("V2a");
    assert!(found.compared > 0 && found.padding > 0, "{found:?}");
    assert!(found.worst <= EXACT_TOLERANCE * found.peak, "{found:?}");
    assert_eq!(found.leaked, 0, "NaN padding pixels became data: {found:?}");
    assert_eq!(found.lost, 0, "finite pixels of the i2d are missing from the mosaic: {found:?}");
}

#[test]
#[ignore = "needs ASTROBURST_MOSAIC_V2_FILE with an i2d, or ASTROBURST_MOSAIC_I2D_DIR with jw01475001001_02101_00001_nrca1_i2d.fits"]
fn v2_split_with_injected_offsets_and_a_scale_recovers_them() {
    let Some(i2d) = v2_i2d() else {
        return;
    };
    let source = source(&i2d);
    let sigma = source.sigma;
    let parts = split_parts(source.sci.dim());
    let injected = |k: usize| V2_INJECTED.iter().find(|(tile, ..)| *tile == k).map_or((1.0f32, 0.0f64), |&(_, gain, sigmas)| (gain, sigmas * sigma));
    let work = workspace("v2-levels");
    let paths = write_parts(&work.path.join("tiles"), &source, &parts, "v2", |k| {
        let (gain, offset) = injected(k);
        (gain, offset as f32)
    });
    let request = MosaicRequest {
        canvas: CanvasRequest::Like { path: i2d.clone() },
        levels: LevelModel::OffsetScale,
        reject: RejectRequest::Off,
        ..MosaicRequest::default()
    };
    let run = run_paths(&work.path, &paths, &request, "v2_levels");
    describe_run("V2b", &run);
    let (offsets, scales) = solved_levels(&run.result);
    let reference = run.result["components"][0]["reference"].as_u64().unwrap() as usize;
    let (reference_gain, reference_offset) = injected(reference);
    let solved_plane = &run.result["levels"][0];
    let (unscaled, unmatched) = (indices(&solved_plane["unscaled"]), indices(&solved_plane["unmatched"]));
    let mut worst_offset = 0.0f64;
    let mut worst_scaled = (0.0f64, 0.0f64);
    for k in 0..parts.len() {
        let (gain, offset) = injected(k);
        let expected_scale = f64::from(gain) / f64::from(reference_gain);
        let expected_offset = offset * f64::from(reference_gain) / f64::from(gain) - reference_offset;
        let offset_error = (offsets[k] - expected_offset) / sigma;
        let scale_error = scales[k] / expected_scale - 1.0;
        if gain == reference_gain {
            worst_offset = worst_offset.max(offset_error.abs());
        } else {
            worst_scaled = (worst_scaled.0.max(scale_error.abs()), worst_scaled.1.max(offset_error.abs()));
        }
        println!(
            "V2b tile {k}: injected gain {gain} offset {:+.2} sigma; solved offset {:+.4e} (expected {:+.4e}, error {offset_error:+.4} sigma), scale {:.6} (expected {expected_scale:.6}, error {:+.3e})",
            offset / sigma,
            offsets[k],
            expected_offset,
            scales[k],
            scale_error
        );
    }
    let (values, coverage) = run.planes();
    let seams = seam_profiles(&values, &coverage, sigma, request.feather_px as usize);
    let plan = plan_of(&paths, &request);
    let windows = corrected_windows(&plan, &offsets, &scales);
    let steps = overlap_levels(&plan.estimate, &windows, sigma);
    for step in &steps {
        println!("V2b overlap {} {}: {} px, median difference after the levels {:+.4} sigma", step.i, step.j, step.pixels, step.step_sigma);
    }
    println!("V2b levels: unscaled tiles {unscaled:?}, unmatched tiles {unmatched:?}");
    println!(
        "V2b reference tile {reference}, sigma {sigma:.4e}: worst offset error of the gain-1 tiles {worst_offset:.4} sigma (tolerance {LEVEL_TOLERANCE_SIGMA}); scaled tiles: worst scale error {:.3e} (tolerance {SCALE_TOLERANCE}), worst offset error {:.4} sigma (tolerance {LEVEL_TOLERANCE_SIGMA})",
        worst_scaled.0,
        worst_scaled.1
    );
    println!("V2b seams: {}", seams.describe());
    describe_memory("V2b");
    assert!(worst_offset <= LEVEL_TOLERANCE_SIGMA, "an offset is {worst_offset} sigma off");
    assert!(
        worst_scaled.0 <= SCALE_TOLERANCE,
        "a scale is {} off; unscaled tiles {unscaled:?}; warnings {:?}",
        worst_scaled.0,
        warnings_of(&run.result)
    );
    assert!(worst_scaled.1 <= LEVEL_TOLERANCE_SIGMA, "the offset of a scaled tile is {} sigma off", worst_scaled.1);
}

#[test]
#[ignore = "needs ASTROBURST_MOSAIC_V2_FILE with an i2d, or ASTROBURST_MOSAIC_I2D_DIR with jw01475001001_02101_00001_nrca1_i2d.fits"]
fn v2_split_on_the_default_canvas_keeps_star_flux_and_positions_and_shares_its_canvas() {
    let Some(i2d) = v2_i2d() else {
        return;
    };
    let source = source(&i2d);
    let parts = split_parts(source.sci.dim());
    let work = workspace("v2-default");
    let paths = write_parts(&work.path.join("tiles"), &source, &parts, "v2", |_| (1.0, 0.0));
    let request = MosaicRequest { reject: RejectRequest::Off, ..MosaicRequest::default() };
    let first = run_paths(&work.path, &paths, &request, "v2_default");
    describe_run("V2c", &first);
    let (first_values, first_coverage) = first.planes();
    let first_header = hdu(&first.fits, SCI_HDU);
    let canvas = load_wcs(&first.fits, &first_header).unwrap();
    let probe = canvas.world_to_pixel(source.wcs.pixel_to_world(0.0, 0.0).ra, source.wcs.pixel_to_world(0.0, 0.0).dec);
    println!("V2c canvas {}x{}: source pixel (0, 0) lands on ({:.4}, {:.4})", first_values.ncols(), first_values.nrows(), probe.0, probe.1);
    let pairs = star_pairs(&source.sci, &source.wcs, &source.stars, &first_values, &canvas, TARGET_STARS);
    let mut ratios: Vec<f64> = pairs.iter().map(|p| p.ratio).collect();
    let (mut dxs, mut dys): (Vec<f64>, Vec<f64>) = pairs.iter().map(|p| (p.dx, p.dy)).unzip();
    let worst_ratio = ratios.iter().map(|r| (r - 1.0).abs()).fold(0.0, f64::max);
    let flux_ratio = median(&mut ratios);
    let centroid = (median(&mut dxs), median(&mut dys));
    println!(
        "V2c {} isolated stars at SNR >= {STAR_SNR}: median flux ratio {flux_ratio:.5} (tolerance 1 +- {FLUX_TOLERANCE}), worst {worst_ratio:.4}; median centroid vector ({:+.4}, {:+.4}) px, |v| {:.4} px (tolerance {CENTROID_TOLERANCE_PX})",
        pairs.len(),
        centroid.0,
        centroid.1,
        centroid.0.hypot(centroid.1)
    );

    let halves = write_parts(&work.path.join("halves"), &source, &parts, "v2_half", |_| (SECOND_FILTER_GAIN, 0.0));
    let shared = MosaicRequest { canvas: CanvasRequest::Like { path: first.fits.clone() }, ..request.clone() };
    let second = run_paths(&work.path, &halves, &shared, "v2_half");
    describe_run("V2d", &second);
    let (second_values, second_coverage) = second.planes();
    let second_header = hdu(&second.fits, SCI_HDU);
    let same_grid = second_values.dim() == first_values.dim()
        && ["CRPIX1", "CRPIX2", "CRVAL1", "CRVAL2", "CD1_1", "CD1_2", "CD2_1", "CD2_2"]
            .iter()
            .all(|key| first_header.get_f64(key) == second_header.get_f64(key));
    let halved = first_values.mapv(|v| SECOND_FILTER_GAIN * v);
    let scaled = identity(&halved, &second_values);
    let coverage_equal = first_coverage == second_coverage;
    let aligned_first = identity(&first_values, &aligned_by_wcs(&first.fits, &first.fits));
    let aligned_second = identity(&second_values, &aligned_by_wcs(&second.fits, &first.fits));
    println!("V2d same grid and WCS cards {same_grid}, same coverage {coverage_equal}");
    println!(
        "V2d second mosaic against 0.5 x first: {} px, worst {:.3e}, ratio {:.3e}, finite in one only {}",
        scaled.compared,
        scaled.worst,
        scaled.worst / scaled.peak,
        scaled.only_one
    );
    for (label, found) in [("first", &aligned_first), ("second", &aligned_second)] {
        println!(
            "V2d Align by WCS of the {label} mosaic onto the first: {} px, worst {:.3e}, ratio {:.3e} (tolerance {EXACT_TOLERANCE:e}), finite in one only {}",
            found.compared,
            found.worst,
            found.worst / found.peak,
            found.only_one
        );
    }
    describe_memory("V2c and V2d");
    assert!(pairs.len() >= MIN_STARS, "only {} stars measured", pairs.len());
    assert!((flux_ratio - 1.0).abs() <= FLUX_TOLERANCE, "median flux ratio {flux_ratio}");
    assert!(centroid.0.hypot(centroid.1) <= CENTROID_TOLERANCE_PX, "median centroid vector {centroid:?}");
    assert!(same_grid && coverage_equal, "the second mosaic does not share the canvas of the first");
    assert!(scaled.worst <= EXACT_TOLERANCE * scaled.peak, "{scaled:?}");
    for found in [&aligned_first, &aligned_second] {
        assert!(found.worst <= EXACT_TOLERANCE * found.peak, "{found:?}");
    }
}

#[test]
#[ignore = "needs ASTROBURST_MOSAIC_I2D_DIR with the jw01475 nrca1 and nrcb1 i2d"]
fn v1_lite_i2d_pair_names_its_components_and_isolated_tiles() {
    let dir = env_dir(I2D_DIR_VAR);
    let (Some(a1), Some(b1)) = (real_file(&dir, NRCA1_I2D), real_file(&dir, NRCB1_I2D)) else {
        return;
    };
    let paths = vec![a1, b1];
    let estimate = estimate_paths(&paths, &MosaicRequest::default());
    describe_estimate("V1-lite i2d", &estimate);
    let names = names_of(&estimate);
    let isolated = indices(&estimate["isolated"]);
    let pairs = estimate["pairs"].as_array().map_or(0, Vec::len);
    if pairs == 0 {
        assert_eq!(isolated, vec![0, 1], "two tiles without an overlap are both isolated");
        assert_eq!(estimate["components"].as_array().map_or(0, Vec::len), 2);
    }
    if let Some(refusal) = estimate["refusal"].as_str() {
        assert!(names.iter().any(|name| refusal.contains(&format!("'{name}'"))), "the refusal names no tile: {refusal}");
    }

    let tiles = load_tiles(&paths).unwrap();
    let config = MosaicConfig { max_area_ratio: FORCED_AREA_RATIO, ..MosaicConfig::default() };
    let started = Instant::now();
    let load = |k: usize| load_tile_pixels(&tiles[k], config.dq_mask);
    let output = run_mosaic_with(&tiles, &config, &load, effective_memory(), &never_cancelled, &no_progress).unwrap();
    let seconds = started.elapsed().as_secs_f64();
    let coverage = output.coverage.mapv(|c| c as f32);
    let (covered, nan_inside, finite_outside) = coverage_agreement(&output.planes[0], &coverage);
    println!(
        "V1-lite i2d forced run ({FORCED_AREA_RATIO:e} area ratio): {:.2} s, canvas {}x{}, covered {covered} px, NaN inside the coverage {nan_inside}, finite outside {finite_outside}",
        seconds,
        coverage.ncols(),
        coverage.nrows()
    );
    for warning in &output.warnings {
        println!("V1-lite i2d run warning: {warning}");
    }
    assert_eq!((nan_inside, finite_outside), (0, 0));
    for &k in &isolated {
        let quoted = format!("'{}'", tiles[k].name);
        assert!(
            output.warnings.iter().any(|w| w.contains(&quoted) && w.contains("overlaps no other tile")),
            "isolated tile {quoted} is not named in {:?}",
            output.warnings
        );
    }
}

#[test]
#[ignore = "needs ASTROBURST_GWCS_DATA_DIR with the jw02739 nrca1 and mirimage _cal files"]
fn v1_lite_gwcs_cal_pair_estimates_and_mosaics_when_they_overlap() {
    let dir = real_data_dir();
    let (Some(nircam), Some(miri)) = (real_file(&dir, NIRCAM_CAL), real_file(&dir, MIRI_CAL)) else {
        return;
    };
    let paths = vec![nircam, miri];
    let estimate = estimate_paths(&paths, &MosaicRequest::default());
    describe_estimate("V1-lite gWCS", &estimate);
    let kinds: Vec<String> =
        estimate["tiles"].as_array().into_iter().flatten().map(|t| t["wcs_kind"].as_str().unwrap_or_default().to_string()).collect();
    assert_eq!(kinds, vec!["gwcs", "gwcs"], "both _cal tiles use their gWCS");
    let pairs = estimate["pairs"].as_array().map_or(0, Vec::len);
    if pairs == 0 {
        assert_eq!(indices(&estimate["isolated"]), vec![0, 1]);
        println!("V1-lite gWCS: the tiles do not overlap, so no mosaic is run");
        return;
    }
    let tiles = load_tiles(&paths).unwrap();
    let config = MosaicConfig { max_area_ratio: FORCED_AREA_RATIO, ..MosaicConfig::default() };
    let started = Instant::now();
    let load = |k: usize| load_tile_pixels(&tiles[k], config.dq_mask);
    let output = run_mosaic_with(&tiles, &config, &load, effective_memory(), &never_cancelled, &no_progress).unwrap();
    let coverage = output.coverage.mapv(|c| c as f32);
    let (covered, nan_inside, finite_outside) = coverage_agreement(&output.planes[0], &coverage);
    println!(
        "V1-lite gWCS run: {:.2} s, canvas {}x{}, covered {covered} px, NaN inside {nan_inside}, finite outside {finite_outside}, residuals {:?}, offsets {:?}",
        started.elapsed().as_secs_f64(),
        coverage.ncols(),
        coverage.nrows(),
        output.pair_residuals,
        output.levels[0].offsets
    );
    for warning in &output.warnings {
        println!("V1-lite gWCS run warning: {warning}");
    }
    assert_eq!((nan_inside, finite_outside), (0, 0));
}

#[test]
#[ignore = "needs ASTROBURST_MAST_DIR with the 16 jw01475 _cal tiles; run it alone in release for the runtime and memory gates"]
fn v1_cal_tiles_meet_the_component_level_seam_registration_runtime_and_memory_gates() {
    let Some(paths) = v1_paths(CAL_SUFFIX) else {
        return;
    };
    let work = workspace("v1");
    let request = MosaicRequest { reject: RejectRequest::Off, ..MosaicRequest::default() };
    let estimate = estimate_paths(&paths, &request);
    describe_estimate("V1", &estimate);
    let names = names_of(&estimate);
    let references: Vec<String> = estimate["components"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| names[c["reference"].as_u64().unwrap() as usize].clone())
        .collect();
    println!("V1 component references {references:?}");
    assert!(estimate["refusal"].is_null(), "{}", estimate["refusal"]);
    assert_eq!(references.len(), V1_REFERENCES.len(), "two components, NIRCam modules A and B");
    for (name, fragment) in references.iter().zip(V1_REFERENCES) {
        assert!(name.contains(fragment), "reference {name} is not the {fragment} tile");
    }
    assert!(indices(&estimate["isolated"]).is_empty());

    let clean = run_paths(&work.path, &paths, &request, "v1_clean");
    describe_run("V1 clean", &clean);
    let rejecting = MosaicRequest { reject: RejectRequest::Auto, ..request.clone() };
    let rejected = run_paths(&work.path, &paths, &rejecting, "v1_reject");
    describe_run("V1 reject", &rejected);
    let rejection_ran = !rejected.result["reject"].is_null();
    let single_exposures = estimate["tiles"].as_array().into_iter().flatten().filter(|t| t["single_exposure"] == json!(true)).count();
    println!(
        "V1 reject: automatic rejection {} (estimate default_reject {}, {single_exposures} of {} tiles are single exposures)",
        if rejection_ran { "ran" } else { "did not run" },
        estimate["default_reject"],
        paths.len()
    );
    for (k, rejection) in rejected.result["rejections"].as_array().into_iter().flatten().enumerate() {
        println!("V1 reject tile {k}: tested {} px, rejected {} px", rejection["tested_px"], rejection["rejected_px"]);
    }

    let plan = plan_of(&paths, &request);
    let sigma = noise_sigma(&tile_plane(&plan, plan.estimate.components[0].reference));
    let (clean_offsets, clean_scales) = solved_levels(&clean.result);
    let injected: Vec<f64> = names
        .iter()
        .map(|name| V1_INJECTED.iter().find(|(fragment, _)| name.contains(fragment)).map_or(0.0, |(_, sigmas)| sigmas * sigma))
        .collect();
    let load = |k: usize| -> anyhow::Result<TilePixels> {
        let mut pixels = load_tile_pixels(&plan.tiles[k], plan.config.dq_mask)?;
        let offset = injected[k] as f32;
        pixels.planes.iter_mut().for_each(|plane| plane.mapv_inplace(|v| v + offset));
        Ok(pixels)
    };
    let started = Instant::now();
    let shifted = run_mosaic_with(&plan.tiles, &plan.config, &load, effective_memory(), &never_cancelled, &no_progress).unwrap();
    println!("V1 injected run (core path): {:.2} s", started.elapsed().as_secs_f64());
    let recovered: Vec<f64> = shifted.levels[0].offsets.iter().zip(&clean_offsets).map(|(a, b)| a - b).collect();
    drop(shifted);
    let memory = process_memory();
    let mut worst_level = 0.0f64;
    for (k, name) in names.iter().enumerate() {
        let error = (recovered[k] - injected[k]) / sigma;
        worst_level = worst_level.max(error.abs());
        println!(
            "V1 tile {k} {name}: clean offset {:+.4e}, injected {:+.3} sigma, recovered {:+.4} sigma, error {error:+.4} sigma",
            clean_offsets[k],
            injected[k] / sigma,
            recovered[k] / sigma
        );
    }

    let (values, coverage) = clean.planes();
    let seams = seam_profiles(&values, &coverage, sigma, request.feather_px as usize);
    drop((values, coverage));
    let windows = corrected_windows(&plan, &clean_offsets, &clean_scales);
    for step in overlap_levels(&plan.estimate, &windows, sigma) {
        println!("V1 overlap {} {}: {} px, median difference after the levels {:+.4} sigma", step.i, step.j, step.pixels, step.step_sigma);
    }
    let shifts = registration(&plan, &windows);
    drop(windows);
    let matched: usize = shifts.iter().map(|s| s.canvas.matched).sum();
    let mut worst_added = f64::NEG_INFINITY;
    for shift in &shifts {
        let added = shift.canvas.magnitude() - shift.input.magnitude();
        worst_added = worst_added.max(added);
        println!(
            "V1 component of {}: input {} matches, median ({:+.4}, {:+.4}) |v| {:.4} px; canvas {} matches, median ({:+.4}, {:+.4}) |v| {:.4} px; added {added:+.4} px",
            names[shift.reference],
            shift.input.matched,
            shift.input.dx,
            shift.input.dy,
            shift.input.magnitude(),
            shift.canvas.matched,
            shift.canvas.dx,
            shift.canvas.dy,
            shift.canvas.magnitude()
        );
        for pair in &shift.pairs {
            println!(
                "V1 registration pair {} {}: input {} matches ({:+.4}, {:+.4}) px; canvas {} matches ({:+.4}, {:+.4}) px",
                pair.i, pair.j, pair.input.matched, pair.input.dx, pair.input.dy, pair.canvas.matched, pair.canvas.dx, pair.canvas.dy
            );
        }
    }
    println!("V1 sigma {sigma:.4e}, worst level error {worst_level:.4} sigma (tolerance {LEVEL_TOLERANCE_SIGMA})");
    println!("V1 seams: {} (tolerance {SEAM_TOLERANCE_SIGMA} sigma on the median |step|)", seams.describe());
    println!("V1 registration: {matched} canvas matches (at least {MIN_REGISTRATION_MATCHES}), worst added {worst_added:+.4} px (tolerance {REGISTRATION_TOLERANCE_PX})");
    println!(
        "V1 runtime: clean {:.2} s (tolerance {V1_RUNTIME_S}), automatic rejection run {:.2} s (tolerance {V1_REJECT_RUNTIME_S}, {})",
        clean.seconds,
        rejected.seconds,
        if rejection_ran { "rejection active" } else { "not exercised: the estimate gives no default rejection" }
    );
    match memory {
        Some(memory) => println!(
            "V1 memory after the three runs: working set {:.3} GB, peak working set {:.3} GB (tolerance {:.1} GB)",
            memory.current as f64 / 1e9,
            memory.peak as f64 / 1e9,
            V1_PEAK_RSS_BYTES as f64 / 1e9
        ),
        None => println!("V1 memory: this platform reports no working set"),
    }
    let summary = json!({
        "tiles": paths,
        "names": names,
        "components": clean.result["components"],
        "offsets": clean_offsets,
        "scales": clean_scales,
        "sigma": sigma,
        "mosaic": clean.fits,
        "feather_px": request.feather_px,
    });
    let summary_path = work.path.join(V1_SUMMARY);
    std::fs::write(&summary_path, serde_json::to_string_pretty(&summary).unwrap()).unwrap();
    println!("V1 oracle summary {}", summary_path.display());
    assert!(worst_level <= LEVEL_TOLERANCE_SIGMA, "an injected offset came back {worst_level} sigma off");
    assert!(seams.used > 0 && seams.median_abs_sigma <= SEAM_TOLERANCE_SIGMA, "{seams:?}");
    assert!(matched >= MIN_REGISTRATION_MATCHES, "only {matched} matches on the canvas");
    assert!(worst_added <= REGISTRATION_TOLERANCE_PX, "the mosaic adds {worst_added} px of registration error");
    assert!(clean.seconds <= V1_RUNTIME_S, "the run took {} s", clean.seconds);
    assert!(rejected.seconds <= V1_REJECT_RUNTIME_S, "the run with rejection took {} s", rejected.seconds);
    if let Some(memory) = memory {
        assert!(memory.peak <= V1_PEAK_RSS_BYTES, "peak working set {} B", memory.peak);
    }
}

#[test]
#[ignore = "needs ASTROBURST_MAST_DIR with the 16 jw01475 _o001_crf tiles"]
fn v1_crf_tiles_make_the_residual_check_warn_on_the_dither_pairs() {
    let Some(paths) = v1_paths(CRF_SUFFIX) else {
        return;
    };
    let work = workspace("v1-crf");
    let request = MosaicRequest { reject: RejectRequest::Off, ..MosaicRequest::default() };
    let run = run_paths(&work.path, &paths, &request, "v1_crf");
    describe_run("V1 crf", &run);
    let names = names_of(&run.result);
    let warnings = warnings_of(&run.result);
    let dither_pairs: Vec<(usize, usize, f64)> = run.result["pair_residuals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let (i, j) = (r["i"].as_u64()? as usize, r["j"].as_u64()? as usize);
            let same_detector = names[i].replace("_00001_", "_00002_") == names[j];
            let shift = r["dx"].as_f64()?.hypot(r["dy"].as_f64()?);
            same_detector.then_some((i, j, shift))
        })
        .collect();
    let warned: Vec<&(usize, usize, f64)> = dither_pairs
        .iter()
        .filter(|(i, j, _)| {
            warnings.iter().any(|w| w.contains(RESIDUAL_WARNING) && w.contains(&format!("'{}'", names[*i])) && w.contains(&format!("'{}'", names[*j])))
        })
        .collect();
    println!("V1 crf dither pairs measured {dither_pairs:?}");
    println!("V1 crf dither pairs warned {} of {}", warned.len(), dither_pairs.len());
    assert!(!warned.is_empty(), "no dither pair of the _crf set is warned: {warnings:#?}");
}

#[test]
#[ignore = "needs ASTROBURST_MOSAIC_V3_DIR with the approved DSS2 hips2fits grid (tile_*.fits) and whole.fits"]
fn v3_dss2_grid_solved_from_header_hints_matches_the_whole_field() {
    let dir = env_dir(V3_DIR_VAR);
    let whole = dir.join(V3_WHOLE);
    if skip_if_absent(&whole) {
        return;
    }
    let tiles = fits_files(&dir, V3_TILE_PREFIX);
    assert!(!tiles.is_empty(), "no {V3_TILE_PREFIX}*.fits under {}", dir.display());
    let work = workspace("v3");
    let hinted: Vec<String> = tiles.iter().map(|tile| hinted_copy(tile, &work.path.join("tiles"), V3_HINT_SHIFT_DEG)).collect();
    let request = MosaicRequest { solve: TileSolveOptions { online: online_allowed(), ..TileSolveOptions::default() }, ..MosaicRequest::default() };
    let run = run_paths(&work.path, &hinted, &request, "v3");
    describe_run("V3", &run);
    for (k, tile) in run.result["tiles"].as_array().into_iter().flatten().enumerate() {
        let solve = &tile["solve"];
        println!("V3 tile {k} {}: tier {} catalog {} matched {} rms {} px", tile["name"], solve["tier"], solve["catalog"], solve["matched"], solve["rms_px"]);
    }
    let whole_path = text(&whole);
    let whole_header = hdu(&whole_path, 0);
    let whole_plane = primary_plane(&whole_path);
    let whole_wcs = load_wcs(&whole_path, &whole_header).unwrap();
    let (values, coverage) = run.planes();
    let canvas = load_wcs(&run.fits, &hdu(&run.fits, SCI_HDU)).unwrap();
    let stars = detect_stars(&whole_plane, DETECTION_SIGMA).stars;
    let pairs = star_pairs(&whole_plane, &whole_wcs, &stars, &values, &canvas, usize::MAX);
    let mut distances: Vec<f64> = pairs.iter().map(|p| p.dx.hypot(p.dy)).collect();
    let worst = distances.iter().copied().fold(0.0, f64::max);
    let typical = median(&mut distances);
    let (covered, nan_inside, _) = coverage_agreement(&values, &coverage);
    println!("V3 {} stars: median {typical:.4} px (tolerance {V3_MEDIAN_PX}), worst {worst:.4} px (tolerance {V3_WORST_PX}); covered {covered}, NaN inside {nan_inside}", pairs.len());
    assert!(pairs.len() >= V3_MIN_STARS, "only {} stars", pairs.len());
    assert!(typical <= V3_MEDIAN_PX && worst <= V3_WORST_PX);
    assert_eq!(nan_inside, 0);
}

#[test]
#[ignore = "needs ASTROBURST_MOSAIC_V4_DIR with the owner's stacked panels"]
fn v4_amateur_panels_mosaic_or_are_refused_by_name() {
    let dir = env_dir(V4_DIR_VAR);
    if skip_if_absent(&dir) {
        return;
    }
    let panels = fits_files(&dir, "");
    assert!(panels.len() >= 2, "V4 needs at least two panels under {}", dir.display());
    let work = workspace("v4");
    let request = MosaicRequest { solve: TileSolveOptions { online: online_allowed(), ..TileSolveOptions::default() }, ..MosaicRequest::default() };
    let estimate = estimate_mosaic_request(&panels, &request, &HarnessSky, effective_memory(), &never_cancelled, &no_report);
    let estimate = match estimate {
        Ok(estimate) => estimate,
        Err(error) => {
            let message = format!("{error:#}");
            println!("V4 refused before the estimate: {message}");
            assert!(message.contains("Tile '"), "the refusal names no panel: {message}");
            return;
        }
    };
    describe_estimate("V4", &estimate);
    if let Some(refusal) = estimate["refusal"].as_str() {
        assert!(refusal.contains('\''), "the refusal names no panel: {refusal}");
        return;
    }
    let run = run_paths(&work.path, &panels, &request, "v4");
    describe_run("V4", &run);
    for (k, tile) in run.result["tiles"].as_array().into_iter().flatten().enumerate() {
        let solve = &tile["solve"];
        println!("V4 panel {k} {}: tier {} catalog {} matched {} rms {} px", tile["name"], solve["tier"], solve["catalog"], solve["matched"], solve["rms_px"]);
    }
    let (offsets, scales) = solved_levels(&run.result);
    println!("V4 offsets {offsets:?} scales {scales:?}");
    let (values, coverage) = run.planes();
    let sigma = noise_sigma(&values);
    let seams = seam_profiles(&values, &coverage, sigma, request.feather_px as usize);
    let (covered, nan_inside, finite_outside) = coverage_agreement(&values, &coverage);
    println!("V4 sigma {sigma:.4e}, seams: {}; covered {covered}, NaN inside {nan_inside}, finite outside {finite_outside}", seams.describe());
    assert_eq!((nan_inside, finite_outside), (0, 0));
}
