use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use ndarray::Array2;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::Manager;

use super::combine::output_name;
use crate::cmd::astrometry::{
    add_online_tier, offline_tiers, solve_credit_history, solve_from_header_hints, GaiaFetch, NovaSettings, NovaSolve,
    SolveEnv, TierRequest, TierSet,
};
use crate::cmd::common::{
    blocking_cmd, cached_header, invalidate_written, resolve_output_dir, save_auto_stf_preview_png, write_derived_mef,
    OutputValues, MAX_PREVIEW_DIM,
};
use crate::service::common::derived_output_header;
use crate::cmd::helpers;
use crate::core::astrometry::catalog::{query_gaia_cached, ConeQuery};
use crate::core::astrometry::plate_solve::SolveResult;
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::imaging::push_history;
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{make_stf_u8_fn, AutoStfConfig};
use crate::core::stacking::memory::{effective_memory, MemoryInfo};
use crate::core::stacking::mosaic::{
    default_rejection, load_tile_pixels, load_tiles_with, mosaic_estimate, run_mosaic_with, MosaicEstimate, MosaicOutput,
    MosaicStage, TileGeometry, UnsolvedTile,
};
use crate::core::stacking::{stop_if_cancelled, CancelCheck};
use crate::infra::catalog_store::{CatalogStore, StoreRoots};
use crate::infra::config::load_config;
use crate::infra::fits::writer::{is_wcs_card, write_fits_mono_int, write_fits_rgb, HduData, ImageHdu};
use crate::infra::progress::ProgressHandle;
use crate::types::config::AppConfig;
use crate::types::constants::{
    EXTNAME_SCI, HEADER_BUNIT, RES_CARDS, RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_HEIGHT, RES_NAME, RES_PATH,
    RES_PLANES, RES_PNG_PATH, RES_ROWS, RES_SCALE, RES_WARNINGS, RES_WIDTH,
};
use crate::types::error::AppError;
use crate::types::header::{is_commentary_key, HduHeader};
use crate::types::stacking::{CanvasSpec, LevelModel, MosaicConfig, RejectSpec};

pub const EVENT_MOSAIC_PROGRESS: &str = "mosaic-progress";

const DEFAULT_STEM: &str = "mosaic";
const ABPROC_MOSAIC: &str = "mosaic";
const EXTNAME_COVERAGE: &str = "COVERAGE";
const COVERAGE_SUFFIX: &str = "_coverage";
const COVERAGE_BITPIX: i32 = 32;
const TILE_ONLY_CARDS: [&str; 5] = [HEADER_BUNIT, "PIXAR_SR", "PIXAR_A2", "FILENAME", "PROGRAM"];
const NEVER_UPLOADED: &str = "A mosaic tile is solved offline only and is never sent to astrometry.net";

const STAGE_SOLVE: &str = "solve";
const STAGE_OVERLAPS: &str = "overlaps";
const STAGE_LEVELS: &str = "levels";
const STAGE_REJECT: &str = "reject";
const STAGE_WARP: &str = "warp";
const STAGE_RESIDUALS: &str = "residuals";
const STAGE_SAVING: &str = "saving";

const RES_COLS: &str = "cols";
const RES_MEGAPIXELS: &str = "megapixels";
const RES_CANVAS: &str = "canvas";
const RES_TILES: &str = "tiles";
const RES_PAIRS: &str = "pairs";
const RES_COMPONENTS: &str = "components";
const RES_ISOLATED: &str = "isolated";
const RES_LEVELS: &str = "levels";
const RES_PAIR_RESIDUALS: &str = "pair_residuals";
const RES_REJECT: &str = "reject";
const RES_DEFAULT_REJECT: &str = "default_reject";
const RES_REJECTED_PX: &str = "rejected_px";
const RES_REJECTIONS: &str = "rejections";
const RES_COVERAGE_PATH: &str = "coverage_path";
const RES_WRITTEN: &str = "written";
const RES_BUNIT: &str = "bunit";
const RES_SINGLE_EXPOSURE: &str = "single_exposure";
const RES_SOLVE: &str = "solve";
const RES_SNR: &str = "snr";

type ProgressReport<'a> = &'a (dyn Fn(u64, u64, &str) + Sync);

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanvasRequest {
    Auto {
        #[serde(default)]
        north_up: bool,
        #[serde(default)]
        reference: Option<usize>,
    },
    Like {
        path: String,
    },
}

impl Default for CanvasRequest {
    fn default() -> Self {
        CanvasRequest::Auto { north_up: false, reference: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum RejectRequest {
    #[default]
    Auto,
    Off,
    On {
        snr: f32,
        scale: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TileSolveOptions {
    pub online: bool,
    pub use_twomass: bool,
    pub max_sip_order: Option<u8>,
}

impl Default for TileSolveOptions {
    fn default() -> Self {
        Self { online: false, use_twomass: true, max_sip_order: None }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MosaicRequest {
    pub canvas: CanvasRequest,
    pub levels: LevelModel,
    pub references: Vec<Option<usize>>,
    pub feather_px: f32,
    pub min_overlap_frac: f64,
    pub dq_mask: bool,
    pub reject: RejectRequest,
    pub solve: TileSolveOptions,
}

impl Default for MosaicRequest {
    fn default() -> Self {
        let config = MosaicConfig::default();
        Self {
            canvas: CanvasRequest::default(),
            levels: config.levels,
            references: config.references,
            feather_px: config.feather_px,
            min_overlap_frac: config.min_overlap_frac,
            dq_mask: config.dq_mask,
            reject: RejectRequest::default(),
            solve: TileSolveOptions::default(),
        }
    }
}

impl RejectRequest {
    fn chosen(self) -> Option<RejectSpec> {
        match self {
            RejectRequest::On { snr, scale } => Some(RejectSpec { snr, scale }),
            RejectRequest::Auto | RejectRequest::Off => None,
        }
    }
}

impl MosaicRequest {
    fn config(&self, canvas: CanvasSpec) -> MosaicConfig {
        MosaicConfig {
            canvas,
            levels: self.levels,
            references: self.references.clone(),
            feather_px: self.feather_px,
            min_overlap_frac: self.min_overlap_frac,
            dq_mask: self.dq_mask,
            reject: self.reject.chosen(),
            ..MosaicConfig::default()
        }
    }
}

struct RunTicks {
    tiles: u64,
}

impl RunTicks {
    fn of(tiles: usize) -> Self {
        RunTicks { tiles: tiles as u64 }
    }

    fn total(&self) -> u64 {
        4 * self.tiles + 2
    }

    fn overlaps(&self, done: usize) -> u64 {
        self.tiles + done as u64
    }

    fn levels(&self) -> u64 {
        2 * self.tiles + 1
    }

    fn reject(&self, done: usize) -> u64 {
        2 * self.tiles + 1 + done as u64
    }

    fn warp(&self, done: usize) -> u64 {
        3 * self.tiles + 1 + done as u64
    }

    fn saving(&self) -> u64 {
        4 * self.tiles + 1
    }
}

fn stage_text(stage: &str, tile: usize, done: usize, of: usize, detail: &str) -> String {
    format!("{stage}:{tile}:{done}/{of}:{detail}")
}

fn solve_stage(tile: &UnsolvedTile<'_>, tiles: usize) -> String {
    stage_text(STAGE_SOLVE, tile.index + 1, tile.index + 1, tiles, tile.name)
}

fn quoted_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn fits_text(value: &str) -> Option<String> {
    let inner = value.trim().strip_prefix('\'')?.strip_suffix('\'')?;
    Some(inner.replace("''", "'").trim_end().to_string())
}

fn with_cards(mut header: HduHeader, cards: &[(String, String)]) -> HduHeader {
    let mut strings = header.string_keys.take().unwrap_or_default();
    for (key, value) in cards {
        let key = key.trim();
        match fits_text(value) {
            Some(text) => {
                header.set(key, text);
                strings.insert(key.to_string());
            }
            None => {
                header.set(key, value.clone());
                strings.remove(key);
            }
        }
    }
    header.string_keys = Some(strings);
    header
}

fn typed_header(cards: &[(String, String)]) -> HduHeader {
    with_cards(HduHeader::empty(), cards)
}

fn literal_cards(header: &HduHeader) -> Vec<(String, String)> {
    header
        .cards
        .iter()
        .filter(|(key, _)| !is_commentary_key(key))
        .map(|(key, value)| {
            let key = key.trim();
            let text = header.is_string_value(key) == Some(true);
            (key.to_string(), if text { quoted_literal(value) } else { value.clone() })
        })
        .collect()
}

fn canvas_spec(canvas: &CanvasRequest) -> Result<CanvasSpec> {
    match canvas {
        CanvasRequest::Auto { north_up, reference } => Ok(CanvasSpec::Auto { north_up: *north_up, reference: *reference }),
        CanvasRequest::Like { path } => {
            let header = cached_header(path).with_context(|| format!("The canvas of {path} cannot be read"))?;
            Ok(CanvasSpec::Like { cards: literal_cards(&header) })
        }
    }
}

fn solve_message(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .map_or_else(|| error.to_string(), str::to_string)
}

fn solve_tile(env: &dyn SolveEnv, tile: &UnsolvedTile<'_>, options: &TileSolveOptions, cancel: CancelCheck) -> Result<SolveResult> {
    stop_if_cancelled(cancel)?;
    let solved = match solve_from_header_hints(env, tile.path, options.online, options.use_twomass, options.max_sip_order) {
        Ok(solved) => solved,
        Err(_) if cancel() => return Err(AppError::Cancelled.into()),
        Err(error) => bail!(
            "Tile '{}' ({}) has no celestial WCS and its solve from the header hints failed: {}",
            tile.name,
            tile.path,
            solve_message(&error)
        ),
    };
    serde_json::from_value(solved)
        .with_context(|| format!("Tile '{}' ({}): the solve answered in an unexpected form", tile.name, tile.path))
}

struct MosaicPlan {
    tiles: Vec<TileGeometry>,
    config: MosaicConfig,
    estimate: MosaicEstimate,
}

fn plan_mosaic(
    paths: &[String],
    request: &MosaicRequest,
    env: &dyn SolveEnv,
    mem: Option<MemoryInfo>,
    cancel: CancelCheck,
    solving: &dyn Fn(&UnsolvedTile<'_>),
) -> Result<MosaicPlan> {
    stop_if_cancelled(cancel)?;
    let canvas = canvas_spec(&request.canvas)?;
    let solve = |tile: &UnsolvedTile<'_>| -> Result<SolveResult> {
        solving(tile);
        solve_tile(env, tile, &request.solve, cancel)
    };
    let tiles = load_tiles_with(paths, &solve)?;
    let mut config = request.config(canvas);
    let automatic = request.reject == RejectRequest::Auto;
    if automatic {
        config.reject = default_rejection(&tiles);
    }
    let estimate = mosaic_estimate(&tiles, &config, mem)?;
    if automatic {
        config.reject = estimate.default_reject;
    }
    Ok(MosaicPlan { tiles, config, estimate })
}

fn reject_json(spec: Option<RejectSpec>) -> Value {
    let decimal = |value: f32| value.to_string().parse::<f64>().unwrap_or(f64::from(value));
    spec.map_or(Value::Null, |spec| json!({ RES_SNR: decimal(spec.snr), RES_SCALE: decimal(spec.scale) }))
}

fn planes_of(tiles: &[TileGeometry]) -> usize {
    tiles.iter().map(|tile| tile.planes).max().unwrap_or(1)
}

fn tile_rows(tiles: &[TileGeometry], estimate: &MosaicEstimate) -> Result<Vec<Value>> {
    tiles
        .iter()
        .enumerate()
        .map(|(k, tile)| {
            let mut row = match estimate.tiles.get(k) {
                Some(geometry) => serde_json::to_value(geometry)?,
                None => json!({ RES_NAME: tile.name }),
            };
            let fields = row.as_object_mut().context("a tile row is not a JSON object")?;
            fields.insert(RES_PATH.to_string(), json!(tile.path));
            fields.insert(RES_WIDTH.to_string(), json!(tile.width));
            fields.insert(RES_HEIGHT.to_string(), json!(tile.height));
            fields.insert(RES_PLANES.to_string(), json!(tile.planes));
            fields.insert(RES_BUNIT.to_string(), json!(tile.bunit));
            fields.insert(RES_SINGLE_EXPOSURE.to_string(), json!(tile.single_exposure));
            fields.insert(RES_SOLVE.to_string(), serde_json::to_value(&tile.solve)?);
            Ok(row)
        })
        .collect()
}

fn estimate_json(plan: &MosaicPlan) -> Result<Value> {
    let mut value = serde_json::to_value(&plan.estimate)?;
    let rows = tile_rows(&plan.tiles, &plan.estimate)?;
    let fields = value.as_object_mut().context("the estimate is not a JSON object")?;
    fields.insert(RES_TILES.to_string(), Value::Array(rows));
    fields.insert(RES_DEFAULT_REJECT.to_string(), reject_json(plan.estimate.default_reject));
    fields.insert(RES_REJECT.to_string(), reject_json(plan.config.reject));
    fields.insert(RES_PLANES.to_string(), json!(planes_of(&plan.tiles)));
    Ok(value)
}

fn estimate_mosaic_request(
    paths: &[String],
    request: &MosaicRequest,
    env: &dyn SolveEnv,
    mem: Option<MemoryInfo>,
    cancel: CancelCheck,
    report: ProgressReport,
) -> Result<Value> {
    let tiles = paths.len();
    let solving = |tile: &UnsolvedTile<'_>| report(tile.index as u64, tiles as u64, &solve_stage(tile, tiles));
    estimate_json(&plan_mosaic(paths, request, env, mem, cancel, &solving)?)
}

fn shared_metadata(tiles: &[TileGeometry]) -> HduHeader {
    let mut shared = HduHeader::empty();
    let headers = match tiles.iter().map(|tile| cached_header(&tile.path)).collect::<Result<Vec<_>>>() {
        Ok(headers) => headers,
        Err(error) => {
            log::warn!("mosaic: a tile header cannot be read again, so no tile metadata is carried over: {error:#}");
            return shared;
        }
    };
    let Some((first, rest)) = headers.split_first() else {
        return shared;
    };
    let mut strings = HashSet::new();
    for (key, _) in &first.cards {
        let key = key.trim();
        let carried = !(is_commentary_key(key) || is_wcs_card(key) || TILE_ONLY_CARDS.contains(&key)) && shared.get(key).is_none();
        let Some(value) = first.get(key).filter(|_| carried) else {
            continue;
        };
        if rest.iter().all(|header| header.get(key) == Some(value)) {
            shared.set(key, value.to_string());
            if first.is_string_value(key) == Some(true) {
                strings.insert(key.to_string());
            }
        }
    }
    shared.string_keys = Some(strings);
    shared
}

fn history_lines(plan: &MosaicPlan, output: &MosaicOutput) -> Vec<String> {
    let mut lines = output.history.clone();
    for tile in &plan.tiles {
        for line in solve_credit_history(tile.solve.as_ref().and_then(|solve| solve.catalog.as_deref())) {
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
    }
    lines
}

struct SavedMosaic {
    fits_path: String,
    coverage_path: Option<String>,
    png_path: String,
}

fn write_fresh(path: &str, write: impl FnOnce() -> Result<()>) -> Result<()> {
    GLOBAL_CUBE_CACHE.invalidate(path);
    let written = write();
    invalidate_written(path);
    written
}

fn save_rgb_preview(planes: [&Array2<f32>; 3], png_path: &str) -> Result<()> {
    let [r, g, b] = planes;
    let stats = planes.map(compute_image_stats);
    let (stf, combined) =
        helpers::compute_linked_stf_with_stats(&stats[0], &stats[1], &stats[2], &AutoStfConfig::default());
    let [fr, fg, fb] = [(); 3].map(|_| make_stf_u8_fn(&stf, &combined));
    helpers::render_rgb_preview_with_stf(r, g, b, fr, fg, fb, png_path, MAX_PREVIEW_DIM)
}

fn save_mosaic(plan: &MosaicPlan, output: &MosaicOutput, output_dir: &str, stem: &str) -> Result<SavedMosaic> {
    let metadata = derived_output_header(Some(&shared_metadata(&plan.tiles)), ABPROC_MOSAIC, OutputValues::Linear);
    let history = history_lines(plan, output);
    let coverage_header = typed_header(&plan.estimate.canvas_cards);
    let fits_path = format!("{output_dir}/{stem}.fits");
    let png_path = format!("{output_dir}/{stem}.png");
    match &output.planes[..] {
        [plane] => {
            let mut sci = typed_header(&output.cards);
            for line in &history {
                push_history(&mut sci, line);
            }
            let hdus = [
                ImageHdu { data: HduData::F32(plane), header: Some(&sci), extname: EXTNAME_SCI, extver: 1 },
                ImageHdu {
                    data: HduData::I32(&output.coverage),
                    header: Some(&coverage_header),
                    extname: EXTNAME_COVERAGE,
                    extver: 1,
                },
            ];
            write_derived_mef(&fits_path, Some(&metadata), &hdus)?;
            save_auto_stf_preview_png(plane, &png_path)?;
            Ok(SavedMosaic { fits_path, coverage_path: None, png_path })
        }
        [r, g, b] => {
            let mut cube = with_cards(metadata, &output.cards);
            for line in &history {
                push_history(&mut cube, line);
            }
            let coverage_path = format!("{output_dir}/{stem}{COVERAGE_SUFFIX}.fits");
            write_fresh(&fits_path, || write_fits_rgb(&fits_path, r, g, b, Some(&cube)))?;
            write_fresh(&coverage_path, || {
                write_fits_mono_int(&coverage_path, &output.coverage, Some(&coverage_header), COVERAGE_BITPIX)
            })?;
            save_rgb_preview([r, g, b], &png_path)?;
            Ok(SavedMosaic { fits_path, coverage_path: Some(coverage_path), png_path })
        }
        planes => bail!("A mosaic holds 1 or 3 planes, not {}", planes.len()),
    }
}

fn run_json(plan: &MosaicPlan, output: &MosaicOutput, saved: &SavedMosaic, started: Instant) -> Result<Value> {
    let estimate = &plan.estimate;
    let tiles = tile_rows(&plan.tiles, estimate)?;
    let mut written = vec![saved.fits_path.clone()];
    written.extend(saved.coverage_path.clone());
    written.push(saved.png_path.clone());
    Ok(json!({
        RES_FITS_PATH: saved.fits_path,
        RES_PNG_PATH: saved.png_path,
        RES_COVERAGE_PATH: saved.coverage_path,
        RES_WRITTEN: written,
        RES_DIMENSIONS: [estimate.cols, estimate.rows],
        RES_PLANES: output.planes.len(),
        RES_CANVAS: {
            RES_COLS: estimate.cols,
            RES_ROWS: estimate.rows,
            RES_MEGAPIXELS: estimate.megapixels,
            RES_CARDS: estimate.canvas_cards,
        },
        RES_TILES: tiles,
        RES_PAIRS: estimate.pairs,
        RES_COMPONENTS: estimate.components,
        RES_ISOLATED: estimate.isolated,
        RES_LEVELS: output.levels,
        RES_PAIR_RESIDUALS: output.pair_residuals,
        RES_REJECT: reject_json(plan.config.reject),
        RES_REJECTED_PX: output.rejected_px,
        RES_REJECTIONS: output.rejections,
        RES_WARNINGS: output.warnings,
        RES_ELAPSED_MS: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }))
}

#[allow(clippy::too_many_arguments)]
fn run_mosaic_request(
    paths: &[String],
    output_dir: &str,
    name: Option<&str>,
    request: &MosaicRequest,
    env: &dyn SolveEnv,
    mem: Option<MemoryInfo>,
    cancel: CancelCheck,
    report: ProgressReport,
) -> Result<Value> {
    let started = Instant::now();
    let ticks = RunTicks::of(paths.len());
    let total = ticks.total();
    let solving = |tile: &UnsolvedTile<'_>| report(tile.index as u64, total, &solve_stage(tile, paths.len()));
    let plan = plan_mosaic(paths, request, env, mem, cancel, &solving)?;
    let warped = AtomicUsize::new(0);
    let tile_name = |tile: usize| tile.checked_sub(1).and_then(|k| plan.tiles.get(k)).map_or("", |t| t.name.as_str());
    let progress = |stage: MosaicStage, done: usize, of: usize| {
        let (label, current, tile) = match stage {
            MosaicStage::Overlaps => (STAGE_OVERLAPS, ticks.overlaps(done), done),
            MosaicStage::Levels => (STAGE_LEVELS, ticks.levels(), 0),
            MosaicStage::Reject => (STAGE_REJECT, ticks.reject(done), done),
            MosaicStage::Warp => {
                warped.store(done, Ordering::SeqCst);
                (STAGE_WARP, ticks.warp(done), done)
            }
            MosaicStage::Residuals => {
                let finished = warped.load(Ordering::SeqCst);
                (STAGE_RESIDUALS, ticks.warp(finished), finished + 1)
            }
        };
        report(current, total, &stage_text(label, tile, done, of, tile_name(tile)));
    };
    let load = |k: usize| load_tile_pixels(&plan.tiles[k], plan.config.dq_mask);
    let output = run_mosaic_with(&plan.tiles, &plan.config, &load, mem, cancel, &progress)?;
    stop_if_cancelled(cancel)?;
    let stem = output_name(name, DEFAULT_STEM);
    report(ticks.saving(), total, &stage_text(STAGE_SAVING, 0, 1, 1, &format!("{stem}.fits")));
    let output_dir = resolve_output_dir(output_dir)?;
    let saved = save_mosaic(&plan, &output, &output_dir, &stem)?;
    run_json(&plan, &output, &saved, started)
}

struct MosaicSolveEnv {
    progress: ProgressHandle,
    resource_dir: Option<PathBuf>,
}

impl MosaicSolveEnv {
    fn new(app: &tauri::AppHandle, total: u64) -> Self {
        MosaicSolveEnv {
            progress: ProgressHandle::new(app, EVENT_MOSAIC_PROGRESS, total),
            resource_dir: app.path().resource_dir().ok(),
        }
    }
}

impl SolveEnv for MosaicSolveEnv {
    fn tiers(&self, request: &TierRequest<'_>) -> TierSet {
        let config = load_config().unwrap_or_else(|error| {
            log::warn!("config unavailable ({error:#}); external catalogues are not used for this mosaic solve");
            AppConfig::default()
        });
        let mut set = match StoreRoots::from_app(self.resource_dir.as_deref()) {
            Ok(roots) => offline_tiers(&CatalogStore::new(roots), &config, request.use_twomass),
            Err(error) => TierSet {
                notes: vec![format!("the catalogue folders are not reachable: {error:#}")],
                ..TierSet::default()
            },
        };
        let online: Option<GaiaFetch> = cfg!(feature = "vizier").then(|| {
            Arc::new(|query: &ConeQuery| query_gaia_cached(query).map(|hit| hit.rows.as_ref().clone())) as GaiaFetch
        });
        let progress = self.progress.clone();
        add_online_tier(&mut set, request, online, Arc::new(move || progress.is_cancelled()));
        set
    }

    fn stored_api_key(&self) -> Option<String> {
        None
    }

    fn nova_settings(&self) -> anyhow::Result<NovaSettings> {
        bail!(NEVER_UPLOADED)
    }

    fn nova(&self) -> Option<NovaSolve> {
        None
    }

    fn cancelled(&self) -> bool {
        self.progress.is_cancelled()
    }

    fn stage(&self, _: &str) {}
}

#[tauri::command]
pub async fn mosaic_estimate_cmd(
    app: tauri::AppHandle,
    paths: Vec<String>,
    config: MosaicRequest,
) -> Result<Value, String> {
    let env = MosaicSolveEnv::new(&app, paths.len() as u64);
    blocking_cmd!({
        let cancel = || env.progress.is_cancelled();
        let report = |current: u64, total: u64, stage: &str| env.progress.report(current, total, stage);
        estimate_mosaic_request(&paths, &config, &env, effective_memory(), &cancel, &report)
    })
}

#[tauri::command]
pub async fn mosaic_cmd(
    app: tauri::AppHandle,
    paths: Vec<String>,
    output_dir: String,
    name: Option<String>,
    config: MosaicRequest,
) -> Result<Value, String> {
    let env = MosaicSolveEnv::new(&app, RunTicks::of(paths.len()).total());
    blocking_cmd!({
        let cancel = || env.progress.is_cancelled();
        let report = |current: u64, total: u64, stage: &str| env.progress.report(current, total, stage);
        let result =
            run_mosaic_request(&paths, &output_dir, name.as_deref(), &config, &env, effective_memory(), &cancel, &report)?;
        env.progress.emit_complete();
        Ok(result)
    })
}

#[cfg(test)]
mod validation;

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs::File;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use ndarray::Array2;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};
    use serde_json::json;

    use crate::cmd::astrometry::{
        add_online_tier, GaiaFetch, NovaFuture, NovaSettings, NovaSolve, NovaUpload, SharedCancelCheck, TierRequest, TierSet,
        TIER0_LABEL,
    };
    use crate::core::astrometry::catalog::{CatalogRow, ConeQuery};
    use crate::core::astrometry::local_solve::synthetic::{
        catalog_row, random_in_cap, render_field, tan_cards, FrameStyle, SyntheticProvider, TruthField,
    };
    use crate::core::astrometry::local_solve::CatalogTier;
    use crate::core::astrometry::wcs::{angular_separation, WcsTransform};
    use crate::core::stacking::never_cancelled;
    use crate::infra::fits::reader::extract_header_by_index;
    use crate::infra::fits::writer::{write_fits_mono, write_fits_rgb};
    use crate::infra::image_source::{load_plane, load_plane_header};
    use crate::types::header::HduHeader;
    use crate::types::image_ref::ImageRef;

    const TILE: (usize, usize) = (100, 80);
    const CENTRE: (f64, f64) = (150.0, 30.0);
    const SCALE_ARCSEC: f64 = 1.0;
    const WCS_NUMBERS: [&str; 8] = ["CRPIX1", "CRPIX2", "CRVAL1", "CRVAL2", "CD1_1", "CD1_2", "CD2_1", "CD2_2"];

    const SKY_CENTRE: (f64, f64) = (83.82, -5.39);
    const SKY_FRAME: (usize, usize) = (1024, 768);
    const SKY_SCALE_ARCSEC: f64 = 3.5;
    const SKY_ROTATION_DEG: f64 = 20.0;
    const SKY_PIXEL_UM: f64 = 3.76;
    const SKY_DENSITY_PER_DEG2: f64 = 400.0;
    const SKY_CAP_DEG: f64 = 1.6;
    const SKY_BRIGHT_MAG: f64 = 8.0;
    const SKY_DEPTH_MAG: f64 = 13.0;
    const ARCSEC_PER_RADIAN: f64 = 206_264.806;

    struct MemorySky {
        rows: Vec<CatalogRow>,
        online_requested: Mutex<Vec<bool>>,
        online_fetches: Arc<AtomicUsize>,
        uploads: Arc<AtomicUsize>,
    }

    impl MemorySky {
        fn new(rows: Vec<CatalogRow>) -> Self {
            Self {
                rows,
                online_requested: Mutex::new(Vec::new()),
                online_fetches: Arc::new(AtomicUsize::new(0)),
                uploads: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn online_requested(&self) -> Vec<bool> {
            self.online_requested.lock().unwrap().clone()
        }

        fn online_fetches(&self) -> usize {
            self.online_fetches.load(Ordering::SeqCst)
        }

        fn uploads(&self) -> usize {
            self.uploads.load(Ordering::SeqCst)
        }
    }

    impl SolveEnv for MemorySky {
        fn tiers(&self, request: &TierRequest<'_>) -> TierSet {
            self.online_requested.lock().unwrap().push(request.online);
            let mut set = TierSet::default();
            set.push(CatalogTier::Tier0, Box::new(SyntheticProvider::new(TIER0_LABEL, self.rows.clone())));
            let fetches = Arc::clone(&self.online_fetches);
            let fetch: GaiaFetch = Arc::new(move |_: &ConeQuery| {
                fetches.fetch_add(1, Ordering::SeqCst);
                Err("this test has no network".to_string())
            });
            let never: SharedCancelCheck = Arc::new(|| false);
            add_online_tier(&mut set, request, Some(fetch), never);
            set
        }

        fn stored_api_key(&self) -> Option<String> {
            Some("stored-key".to_string())
        }

        fn nova_settings(&self) -> anyhow::Result<NovaSettings> {
            Ok(NovaSettings { api_url: "http://127.0.0.1:9".to_string(), timeout_secs: 1 })
        }

        fn nova(&self) -> Option<NovaSolve> {
            let uploads = Arc::clone(&self.uploads);
            Some(Box::new(move |_: NovaUpload| -> NovaFuture {
                uploads.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { anyhow::bail!("a mosaic tile went to astrometry.net") })
            }))
        }

        fn cancelled(&self) -> bool {
            false
        }

        fn stage(&self, _: &str) {}
    }

    fn no_report(_: u64, _: u64, _: &str) {}

    fn text(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    fn tile_header(crval: (f64, f64), extra: &[(&str, &str)]) -> HduHeader {
        let (w, h) = TILE;
        let s = SCALE_ARCSEC / 3600.0;
        let mut header = HduHeader::empty();
        for (key, value) in [
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
            ("CTYPE1", "RA---TAN".to_string()),
            ("CTYPE2", "DEC--TAN".to_string()),
            ("CRPIX1", ((w as f64 + 1.0) / 2.0).to_string()),
            ("CRPIX2", ((h as f64 + 1.0) / 2.0).to_string()),
            ("CRVAL1", crval.0.to_string()),
            ("CRVAL2", crval.1.to_string()),
            ("CD1_1", (-s).to_string()),
            ("CD1_2", "0".to_string()),
            ("CD2_1", "0".to_string()),
            ("CD2_2", s.to_string()),
        ] {
            header.set(key, value);
        }
        for (key, value) in extra {
            header.set(key, value.to_string());
        }
        header
    }

    fn shifted(pixels: (f64, f64)) -> (f64, f64) {
        let s = SCALE_ARCSEC / 3600.0;
        (CENTRE.0 - pixels.0 * s / CENTRE.1.to_radians().cos(), CENTRE.1 + pixels.1 * s)
    }

    fn flat_sky(level: f32, seed: u64) -> Array2<f32> {
        let (w, h) = TILE;
        let mut rng = StdRng::seed_from_u64(seed);
        Array2::from_shape_fn((h, w), |_| level + rng.gen_range(-1.0f32..1.0))
    }

    struct Panel {
        name: &'static str,
        offset_px: (f64, f64),
        level: f32,
        date: &'static str,
    }

    const PAIR: [Panel; 2] = [
        Panel { name: "west", offset_px: (0.0, 0.0), level: 100.0, date: "2026-01-01T00:00:00" },
        Panel { name: "east", offset_px: (60.0, 40.0), level: 120.0, date: "2026-01-02T00:00:00" },
    ];

    fn panel_header(panel: &Panel) -> HduHeader {
        tile_header(
            shifted(panel.offset_px),
            &[("BUNIT", "DN/s"), ("FILTER", "F200W"), ("TELESCOP", "JWST"), ("DATE-OBS", panel.date)],
        )
    }

    fn write_panels(dir: &Path, panels: &[Panel], rgb: bool) -> Vec<String> {
        panels
            .iter()
            .enumerate()
            .map(|(k, panel)| {
                let path = text(&dir.join(format!("{}.fits", panel.name)));
                let header = panel_header(panel);
                let plane = flat_sky(panel.level, k as u64);
                if rgb {
                    let (g, b) = (plane.mapv(|v| 0.5 * v), plane.mapv(|v| 0.25 * v));
                    write_fits_rgb(&path, &plane, &g, &b, Some(&header)).unwrap();
                } else {
                    write_fits_mono(&path, &plane, Some(&header)).unwrap();
                }
                path
            })
            .collect()
    }

    fn cards_of(value: &Value) -> Vec<(String, String)> {
        serde_json::from_value(value.clone()).unwrap()
    }

    fn number(cards: &[(String, String)], key: &str) -> Option<f64> {
        cards.iter().find(|(k, _)| k == key).and_then(|(_, v)| v.trim().parse().ok())
    }

    fn card<'a>(cards: &'a [(String, String)], key: &str) -> Option<&'a str> {
        cards.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    fn history(header: &HduHeader) -> Vec<String> {
        header.cards.iter().filter(|(k, _)| k == "HISTORY").map(|(_, v)| v.trim().to_string()).collect()
    }

    fn hdu(path: &str, index: usize) -> HduHeader {
        extract_header_by_index(&File::open(path).unwrap(), index).unwrap()
    }

    fn plane_of(path: &str, index: usize) -> Array2<f32> {
        load_plane(&ImageRef::parse(&format!("{path}#hdu={index}"))).unwrap().arr
    }

    fn coverage_at(canvas: &WcsTransform, coverage: &Array2<f32>, (ra, dec): (f64, f64)) -> f32 {
        let (x, y) = canvas.world_to_pixel(ra, dec);
        coverage[[y.round() as usize, x.round() as usize]]
    }

    fn run_pair(dir: &Path, name: &str) -> (Vec<String>, Value) {
        let paths = write_panels(dir, &PAIR, false);
        let out_dir = text(&dir.join("out"));
        let env = MemorySky::new(Vec::new());
        let result =
            run_mosaic_request(&paths, &out_dir, Some(name), &MosaicRequest::default(), &env, None, &never_cancelled, &no_report)
                .unwrap();
        (paths, result)
    }

    #[test]
    fn mosaic_cmd_writes_sci_and_coverage_with_canvas_wcs() {
        let dir = tempfile::tempdir().unwrap();
        let (_, result) = run_pair(dir.path(), "pair");

        let fits = result["fits_path"].as_str().unwrap().to_string();
        let png = result["png_path"].as_str().unwrap().to_string();
        assert!(fits.ends_with("pair.fits") && png.ends_with("pair.png"), "{result}");
        assert_eq!(result["written"], json!([fits, png]));
        assert!(result["coverage_path"].is_null(), "{result}");
        assert!(Path::new(&fits).is_file() && Path::new(&png).is_file());

        let (cols, rows) = (result["canvas"]["cols"].as_i64().unwrap(), result["canvas"]["rows"].as_i64().unwrap());
        assert_eq!(result["dimensions"], json!([cols, rows]));
        let canvas_cards = cards_of(&result["canvas"]["cards"]);
        let (primary, sci, coverage_header) = (hdu(&fits, 0), hdu(&fits, 1), hdu(&fits, 2));
        assert_eq!((sci.get("EXTNAME"), coverage_header.get("EXTNAME")), (Some("SCI"), Some("COVERAGE")));
        assert_eq!((sci.get_i64("BITPIX"), coverage_header.get_i64("BITPIX")), (Some(-32), Some(32)));
        for header in [&sci, &coverage_header] {
            assert_eq!((header.get_i64("NAXIS1"), header.get_i64("NAXIS2")), (Some(cols), Some(rows)));
            assert_eq!((header.get("CTYPE1"), header.get("CTYPE2")), (Some("RA---TAN"), Some("DEC--TAN")));
            assert_eq!(header.get("RADESYS"), Some("ICRS"));
            for key in WCS_NUMBERS {
                assert_eq!(header.get_f64(key), number(&canvas_cards, key), "{key}");
            }
        }
        assert!(primary.get("CTYPE1").is_none() && primary.get("CRVAL1").is_none(), "{:?}", primary.cards);
        assert_eq!((primary.get("FILTER"), primary.get("TELESCOP")), (Some("F200W"), Some("JWST")));
        assert!(primary.get("DATE-OBS").is_none(), "a card the tiles disagree on was kept");
        assert_eq!(sci.get("BUNIT"), Some("DN/s"));
        let pixar_a2 = sci.get_f64("PIXAR_A2").unwrap();
        assert!((pixar_a2 - 1.0).abs() < 1e-9, "PIXAR_A2 {pixar_a2}");
        let pixar_sr = sci.get_f64("PIXAR_SR").unwrap();
        assert!((pixar_sr / (1f64 / 3600.0).to_radians().powi(2) - 1.0).abs() < 1e-9, "PIXAR_SR {pixar_sr}");

        let lines = history(&sci);
        for expected in [
            "Mosaic of 2 tiles",
            "Tile 1 'west' from west.fits: header WCS",
            "Tile 2 'east' from east.fits: header WCS",
            "Group 1 of tiles 1, 2: reference tile 1 'west'",
            "Feather 64 px",
            "DQ flags applied",
            "Tile 2 'east': 0 of 8000 px left out",
            "Tile 2 'east': offset",
            "Outlier rejection off",
        ] {
            assert!(lines.iter().any(|h| h.contains(expected)), "{expected} is missing from {lines:#?}");
        }

        let values = plane_of(&fits, 1);
        let coverage = plane_of(&fits, 2);
        assert_eq!(values.dim(), (rows as usize, cols as usize));
        assert_eq!(coverage.dim(), values.dim());
        let canvas = WcsTransform::from_header(&sci).unwrap();
        let west = WcsTransform::from_header(&panel_header(&PAIR[0])).unwrap();
        let shared = west.pixel_to_world(79.5, 59.5);
        assert_eq!(coverage_at(&canvas, &coverage, CENTRE), 1.0);
        assert_eq!(coverage_at(&canvas, &coverage, (shared.ra, shared.dec)), 2.0);
        assert!(coverage.iter().any(|&c| c == 0.0), "the canvas corners outside both tiles");
        for ((y, x), &count) in coverage.indexed_iter() {
            assert_eq!(values[[y, x]].is_finite(), count > 0.0, "pixel ({x}, {y}) with coverage {count}");
        }
        let east_centre = shifted(PAIR[1].offset_px);
        let (wx, wy) = canvas.world_to_pixel(CENTRE.0, CENTRE.1);
        let (ex, ey) = canvas.world_to_pixel(east_centre.0, east_centre.1);
        let step = values[[ey.round() as usize, ex.round() as usize]] - values[[wy.round() as usize, wx.round() as usize]];
        assert!(step.abs() < 3.0, "the levels left a step of {step} between the tiles");

        assert_eq!(result["components"], json!([{ "tiles": [0, 1], "reference": 0 }]));
        assert_eq!(result["levels"].as_array().unwrap().len(), 1);
        assert_eq!(result["pair_residuals"].as_array().unwrap().len(), 1);
        assert_eq!(result["pair_residuals"][0]["i"], 0);
        assert!(result["warnings"].is_array());
        assert_eq!(result["planes"], 1);
    }

    #[test]
    fn mosaic_estimate_cmd_equals_the_run_canvas() {
        let dir = tempfile::tempdir().unwrap();
        let paths = write_panels(dir.path(), &PAIR, false);
        let env = MemorySky::new(Vec::new());
        let request = MosaicRequest::default();
        let estimate = estimate_mosaic_request(&paths, &request, &env, None, &never_cancelled, &no_report).unwrap();
        assert!(estimate["refusal"].is_null(), "{estimate}");

        let out_dir = text(&dir.path().join("out"));
        let run = run_mosaic_request(&paths, &out_dir, None, &request, &env, None, &never_cancelled, &no_report).unwrap();
        for (estimated, ran) in [("cols", "cols"), ("rows", "rows"), ("megapixels", "megapixels"), ("canvas_cards", "cards")] {
            assert_eq!(estimate[estimated], run["canvas"][ran], "{estimated}");
        }
        assert_eq!(estimate["components"], run["components"]);
        assert_eq!(estimate["pairs"], run["pairs"]);
        let windows = |v: &Value| v["tiles"].as_array().unwrap().iter().map(|t| t["window"].clone()).collect::<Vec<_>>();
        assert_eq!(windows(&estimate), windows(&run));

        let fits = run["fits_path"].as_str().unwrap();
        assert!(fits.ends_with("mosaic.fits"), "{fits}");
        let sci = hdu(fits, 1);
        assert_eq!((sci.get_i64("NAXIS1"), sci.get_i64("NAXIS2")), (estimate["cols"].as_i64(), estimate["rows"].as_i64()));
        let estimated_cards = cards_of(&estimate["canvas_cards"]);
        for key in WCS_NUMBERS {
            assert_eq!(sci.get_f64(key), number(&estimated_cards, key), "{key}");
        }
        assert_eq!(card(&estimated_cards, "CTYPE1"), Some("'RA---TAN'"));

        let tiles = estimate["tiles"].as_array().unwrap();
        assert_eq!((tiles[0]["name"].as_str(), tiles[1]["name"].as_str()), (Some("west"), Some("east")));
        assert_eq!(tiles[1]["path"], json!(paths[1]));
        assert_eq!((tiles[0]["width"].as_u64(), tiles[0]["height"].as_u64(), tiles[0]["planes"].as_u64()), (Some(100), Some(80), Some(1)));
        assert_eq!(tiles[0]["wcs_kind"], "header");
        assert!(tiles[0]["solve"].is_null() && tiles[1]["solve"].is_null());
        assert_eq!(tiles[1]["single_exposure"], false);
        assert!(estimate["reject"].is_null() && estimate["default_reject"].is_null());
        assert!(estimate["ram_bytes"].as_u64().unwrap() > 0);
    }

    fn sky_rows(seed: u64) -> Vec<CatalogRow> {
        let mut rng = StdRng::seed_from_u64(seed);
        let cap_area =
            2.0 * std::f64::consts::PI * (1.0 - SKY_CAP_DEG.to_radians().cos()) * (180.0 / std::f64::consts::PI).powi(2);
        let count = (SKY_DENSITY_PER_DEG2 * cap_area).round() as usize;
        let (bright, faint) = (10f64.powf(0.4 * SKY_BRIGHT_MAG), 10f64.powf(0.4 * SKY_DEPTH_MAG));
        (0..count)
            .map(|index| {
                let (ra, dec) = random_in_cap(&mut rng, SKY_CENTRE, SKY_CAP_DEG);
                let mag = (bright + rng.gen::<f64>() * (faint - bright)).log10() / 0.4;
                catalog_row(index, ra, dec, Some(mag))
            })
            .collect()
    }

    fn east_of(centre: (f64, f64), deg: f64) -> (f64, f64) {
        (centre.0 + deg / centre.1.to_radians().cos(), centre.1)
    }

    fn hint_cards(pointing: Option<(f64, f64)>) -> Vec<(&'static str, String)> {
        let focal_mm = ARCSEC_PER_RADIAN * SKY_PIXEL_UM / 1000.0 / SKY_SCALE_ARCSEC;
        let mut cards = vec![
            ("XPIXSZ", SKY_PIXEL_UM.to_string()),
            ("FOCALLEN", format!("{focal_mm:.4}")),
            ("DATE-OBS", "2026-01-15T22:00:00".to_string()),
        ];
        if let Some((ra, dec)) = pointing {
            cards.push(("RA", ra.to_string()));
            cards.push(("DEC", dec.to_string()));
        }
        cards
    }

    fn sky_tile(dir: &Path, name: &str, truth: &TruthField, rows: &[CatalogRow], with_wcs: bool, seed: u64) -> String {
        let style = FrameStyle { uncatalogued: 0, seed, ..FrameStyle::default() };
        let image = render_field(truth, rows, SKY_DEPTH_MAG, &style);
        let mut header = HduHeader::empty();
        if with_wcs {
            for (key, value) in tan_cards(&truth.tan, truth.width, truth.height) {
                header.set(&key, value);
            }
        }
        let pointing = (!with_wcs).then(|| truth.centre());
        for (key, value) in hint_cards(pointing) {
            header.set(key, value);
        }
        let path = text(&dir.join(format!("{name}.fits")));
        write_fits_mono(&path, &image, Some(&header)).unwrap();
        path
    }

    #[test]
    fn a_tile_without_wcs_is_solved_from_its_header_hints_and_never_uploaded() {
        let dir = tempfile::tempdir().unwrap();
        let rows = sky_rows(3);
        let (w, h) = SKY_FRAME;
        let west = TruthField::new(SKY_CENTRE, SKY_SCALE_ARCSEC, SKY_ROTATION_DEG, false, w, h);
        let east = TruthField::new(east_of(SKY_CENTRE, 0.6), SKY_SCALE_ARCSEC, SKY_ROTATION_DEG, false, w, h);
        let paths = vec![sky_tile(dir.path(), "west", &west, &rows, true, 1), sky_tile(dir.path(), "east", &east, &rows, false, 2)];
        let untouched = std::fs::read(&paths[1]).unwrap();
        let env = MemorySky::new(rows.clone());

        let estimate = estimate_mosaic_request(&paths, &MosaicRequest::default(), &env, None, &never_cancelled, &no_report).unwrap();
        assert!(estimate["refusal"].is_null(), "{estimate}");
        let tiles = estimate["tiles"].as_array().unwrap();
        assert!(tiles[0]["solve"].is_null(), "{}", tiles[0]);
        let solve = tiles[1]["solve"].clone();
        assert_eq!((solve["solver"].as_str(), solve["tier"].as_u64()), (Some("local"), Some(0)), "{solve}");
        assert_eq!(solve["catalog"], TIER0_LABEL);
        assert!(solve["matched"].as_u64().unwrap() >= 8, "{solve}");
        assert!(solve["rms_px"].as_f64().unwrap() <= 1.0, "{solve}");
        let hint = solve["hint_source"].as_str().unwrap();
        assert!(hint.contains("RA/DEC") && hint.contains("XPIXSZ"), "{hint}");
        let truth = east.wcs().pixel_to_world((w as f64 - 1.0) / 2.0, (h as f64 - 1.0) / 2.0);
        let (ra, dec) = (solve["center_ra"].as_f64().unwrap(), solve["center_dec"].as_f64().unwrap());
        let off_px = angular_separation(ra, dec, truth.ra, truth.dec) * 3600.0 / SKY_SCALE_ARCSEC;
        assert!(off_px < 0.5, "the solved centre is {off_px} px from the truth");
        assert!(!solve["wcs_cards"].as_array().unwrap().is_empty(), "{solve}");
        assert_eq!(env.online_requested(), vec![false]);
        assert_eq!((env.online_fetches(), env.uploads()), (0, 0));

        let out_dir = text(&dir.path().join("out"));
        let run =
            run_mosaic_request(&paths, &out_dir, Some("solved"), &MosaicRequest::default(), &env, None, &never_cancelled, &no_report)
                .unwrap();
        assert_eq!(run["canvas"]["cards"], estimate["canvas_cards"]);
        assert_eq!(run["tiles"][1]["solve"]["wcs_cards"], solve["wcs_cards"]);
        let residual = &run["pair_residuals"][0];
        assert!(residual["matched"].as_u64().unwrap() > 0, "{residual}");
        let shift = residual["dx"].as_f64().unwrap().hypot(residual["dy"].as_f64().unwrap());
        assert!(shift < 0.5, "the solved tile sits {shift} px off the other on shared stars");
        let lines = history(&hdu(run["fits_path"].as_str().unwrap(), 1));
        let solved_line = "Tile 2 'east' from east.fits: WCS solved offline from its header hints";
        assert!(lines.iter().any(|l| l.starts_with(solved_line)), "{lines:#?}");
        assert_eq!(std::fs::read(&paths[1]).unwrap(), untouched, "the tile file was rewritten");
        assert!(load_plane_header(&ImageRef::parse(&paths[1])).unwrap().get("CTYPE1").is_none());
        assert_eq!(env.online_requested(), vec![false, false]);
        assert_eq!((env.online_fetches(), env.uploads()), (0, 0));

        let online = MosaicRequest { solve: TileSolveOptions { online: true, ..TileSolveOptions::default() }, ..MosaicRequest::default() };
        let allowed = estimate_mosaic_request(&paths, &online, &env, None, &never_cancelled, &no_report).unwrap();
        assert_eq!(allowed["tiles"][1]["solve"]["tier"], 0, "{}", allowed["tiles"][1]);
        assert_eq!(allowed["canvas_cards"], estimate["canvas_cards"]);
        assert_eq!(env.online_requested(), vec![false, false, true]);
        assert!(env.online_fetches() >= 1, "the online tier was not offered when the settings allow it");
        assert_eq!(env.uploads(), 0);
    }

    #[test]
    fn a_tile_without_pointing_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = write_panels(dir.path(), &PAIR, false);
        let mut header = HduHeader::empty();
        for (key, value) in hint_cards(None) {
            header.set(key, value);
        }
        let bare = text(&dir.path().join("nopoint.fits"));
        write_fits_mono(&bare, &flat_sky(100.0, 9), Some(&header)).unwrap();
        paths.push(bare);
        let env = MemorySky::new(Vec::new());

        let refused = format!(
            "{:#}",
            estimate_mosaic_request(&paths, &MosaicRequest::default(), &env, None, &never_cancelled, &no_report).unwrap_err()
        );
        assert!(refused.contains("Tile 'nopoint'") && refused.contains("no pointing in the header"), "{refused}");
        assert!(refused.contains("Plate Solve") && refused.contains("Write WCS"), "{refused}");
        let out_dir = dir.path().join("out");
        let ran = format!(
            "{:#}",
            run_mosaic_request(&paths, &text(&out_dir), None, &MosaicRequest::default(), &env, None, &never_cancelled, &no_report)
                .unwrap_err()
        );
        assert_eq!(ran, refused);
        assert!(!out_dir.exists());
        assert!(env.online_requested().is_empty(), "the solver ran for a tile it cannot solve");
    }

    #[test]
    fn a_mosaic_cancelled_during_the_warp_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = write_panels(dir.path(), &PAIR, false);
        let out_dir = dir.path().join("out");
        let env = MemorySky::new(Vec::new());
        let stop = AtomicBool::new(false);
        let cancelled = || stop.load(Ordering::SeqCst);
        let report = |_: u64, _: u64, stage: &str| {
            if stage.starts_with("warp:") {
                stop.store(true, Ordering::SeqCst);
            }
        };
        let error = run_mosaic_request(&paths, &text(&out_dir), None, &MosaicRequest::default(), &env, None, &cancelled, &report)
            .unwrap_err();
        assert!(format!("{error:#}").to_lowercase().contains("cancel"), "{error:#}");
        assert!(!out_dir.exists(), "a cancelled mosaic created its output folder");
    }

    #[test]
    fn mosaic_progress_counts_overlaps_levels_warp_and_saving_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let paths = write_panels(dir.path(), &PAIR, false);
        let env = MemorySky::new(Vec::new());
        let reports = Mutex::new(Vec::new());
        let report = |current: u64, total: u64, stage: &str| reports.lock().unwrap().push((current, total, stage.to_string()));
        run_mosaic_request(&paths, &text(&dir.path().join("out")), None, &MosaicRequest::default(), &env, None, &never_cancelled, &report)
            .unwrap();
        let reports = reports.into_inner().unwrap();
        assert!(reports.iter().all(|(_, total, _)| *total == 10), "{reports:?}");
        let fields: Vec<(u64, Vec<&str>)> = reports.iter().map(|(current, _, stage)| (*current, stage.splitn(4, ':').collect())).collect();
        let shape: Vec<(u64, &str, &str, &str)> = fields.iter().map(|(c, f)| (*c, f[0], f[2], f[3])).collect();
        assert_eq!(
            shape,
            vec![
                (3, "overlaps", "1/2", "west"),
                (4, "overlaps", "2/2", "east"),
                (5, "levels", "1/1", ""),
                (8, "warp", "1/2", "west"),
                (8, "residuals", "1/1", "east"),
                (9, "warp", "2/2", "east"),
                (9, "saving", "1/1", "mosaic.fits"),
            ]
        );
        assert!(fields.iter().all(|(_, f)| f[1].parse::<u64>().is_ok()), "{reports:?}");
    }

    #[test]
    fn rgb_tiles_write_an_rgb_cube_and_a_coverage_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = write_panels(dir.path(), &PAIR, true);
        let env = MemorySky::new(Vec::new());
        let out_dir = text(&dir.path().join("out"));
        let result =
            run_mosaic_request(&paths, &out_dir, Some("colour"), &MosaicRequest::default(), &env, None, &never_cancelled, &no_report)
                .unwrap();
        let fits = result["fits_path"].as_str().unwrap().to_string();
        let coverage_path = result["coverage_path"].as_str().unwrap().to_string();
        let png = result["png_path"].as_str().unwrap().to_string();
        assert!(fits.ends_with("colour.fits") && coverage_path.ends_with("colour_coverage.fits"), "{result}");
        assert_eq!(result["written"], json!([fits, coverage_path, png]));
        assert_eq!(result["planes"], 3);
        assert_eq!(result["levels"].as_array().unwrap().len(), 3);

        let (cols, rows) = (result["canvas"]["cols"].as_i64().unwrap(), result["canvas"]["rows"].as_i64().unwrap());
        let canvas_cards = cards_of(&result["canvas"]["cards"]);
        let cube = hdu(&fits, 0);
        assert_eq!((cube.get_i64("NAXIS"), cube.get_i64("NAXIS3")), (Some(3), Some(3)));
        let coverage_header = hdu(&coverage_path, 0);
        assert_eq!((coverage_header.get_i64("BITPIX"), coverage_header.get_i64("NAXIS")), (Some(32), Some(2)));
        for header in [&cube, &coverage_header] {
            assert_eq!((header.get_i64("NAXIS1"), header.get_i64("NAXIS2")), (Some(cols), Some(rows)));
            assert_eq!(header.get("CTYPE1"), Some("RA---TAN"));
            for key in WCS_NUMBERS {
                assert_eq!(header.get_f64(key), number(&canvas_cards, key), "{key}");
            }
        }
        assert!(cube.get_f64("PIXAR_A2").is_some() && cube.get("FILTER") == Some("F200W"), "{:?}", cube.cards);
        assert!(history(&cube).iter().any(|l| l.contains("Mosaic of 2 tiles")));
        let coverage = load_plane(&ImageRef::parse(&coverage_path)).unwrap().arr;
        assert_eq!(coverage.dim(), (rows as usize, cols as usize));
        assert_eq!(coverage.iter().cloned().fold(0.0f32, f32::max), 2.0);
        assert!(coverage.iter().any(|&c| c == 0.0));
        assert!(Path::new(&png).is_file());
    }

    #[test]
    fn the_canvas_of_a_written_mosaic_keeps_crval_and_cd() {
        let dir = tempfile::tempdir().unwrap();
        let (paths, first) = run_pair(dir.path(), "first");
        let request = MosaicRequest {
            canvas: CanvasRequest::Like { path: first["fits_path"].as_str().unwrap().to_string() },
            ..MosaicRequest::default()
        };
        let env = MemorySky::new(Vec::new());
        let second = estimate_mosaic_request(&paths[1..], &request, &env, None, &never_cancelled, &no_report).unwrap();
        assert!(second["refusal"].is_null(), "{second}");
        let (old, new) = (cards_of(&first["canvas"]["cards"]), cards_of(&second["canvas_cards"]));
        for key in ["CRVAL1", "CRVAL2", "CD1_1", "CD1_2", "CD2_1", "CD2_2"] {
            assert_eq!(number(&new, key), number(&old, key), "{key}");
        }
        for key in ["CRPIX1", "CRPIX2"] {
            let shift = number(&new, key).unwrap() - number(&old, key).unwrap();
            assert!((shift - shift.round()).abs() < 1e-9, "{key} moved by {shift}");
        }
        assert!(second["cols"].as_u64() < first["canvas"]["cols"].as_u64());
        assert_eq!(card(&new, "CTYPE1"), Some("'RA---TAN'"));
    }

    #[test]
    fn auto_rejection_follows_the_tiles_and_an_explicit_choice_wins() {
        let dir = tempfile::tempdir().unwrap();
        let trio = [
            Panel { name: "a_cal", offset_px: (0.0, 0.0), level: 100.0, date: "2026-01-01T00:00:00" },
            Panel { name: "b_cal", offset_px: (30.0, 0.0), level: 100.0, date: "2026-01-01T00:00:00" },
            Panel { name: "c_cal", offset_px: (15.0, 30.0), level: 100.0, date: "2026-01-01T00:00:00" },
        ];
        let singles = write_panels(dir.path(), &trio, false);
        let env = MemorySky::new(Vec::new());
        let estimate = |paths: &[String], reject: RejectRequest| {
            let request = MosaicRequest { reject, ..MosaicRequest::default() };
            estimate_mosaic_request(paths, &request, &env, None, &never_cancelled, &no_report).unwrap()
        };
        let auto = estimate(&singles, RejectRequest::Auto);
        assert_eq!(auto["default_reject"], json!({ "snr": 3.5, "scale": 1.2 }));
        assert_eq!(auto["reject"], json!({ "snr": 3.5, "scale": 1.2 }));
        assert!(estimate(&singles, RejectRequest::Off)["reject"].is_null());
        assert_eq!(estimate(&singles, RejectRequest::On { snr: 5.0, scale: 0.5 })["reject"], json!({ "snr": 5.0, "scale": 0.5 }));

        let stacked = tempfile::tempdir().unwrap();
        let panels = [
            Panel { name: "a", ..trio[0] },
            Panel { name: "b", ..trio[1] },
            Panel { name: "c", ..trio[2] },
        ];
        let stacked_paths = write_panels(stacked.path(), &panels, false);
        assert!(estimate(&stacked_paths, RejectRequest::Auto)["reject"].is_null());
    }

    #[test]
    fn mosaic_request_reads_snake_case_fields_and_defaults_the_rest() {
        let empty: MosaicRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(empty, MosaicRequest::default());
        assert_eq!(empty.canvas, CanvasRequest::Auto { north_up: false, reference: None });
        assert_eq!((empty.feather_px, empty.min_overlap_frac, empty.dq_mask), (64.0, 0.02, true));
        assert_eq!((empty.levels, empty.reject), (LevelModel::Offset, RejectRequest::Auto));
        assert_eq!(empty.solve, TileSolveOptions { online: false, use_twomass: true, max_sip_order: None });

        let full: MosaicRequest = serde_json::from_value(json!({
            "canvas": { "mode": "auto", "north_up": true, "reference": 1 },
            "levels": "offset_scale",
            "references": [null, 1],
            "feather_px": 32.0,
            "min_overlap_frac": 0.05,
            "dq_mask": false,
            "reject": { "mode": "on", "snr": 4.0, "scale": 1.0 },
            "solve": { "online": true, "use_twomass": false, "max_sip_order": 3 }
        }))
        .unwrap();
        assert_eq!(full.canvas, CanvasRequest::Auto { north_up: true, reference: Some(1) });
        assert_eq!((full.levels, full.references.clone()), (LevelModel::OffsetScale, vec![None, Some(1)]));
        assert_eq!((full.feather_px, full.min_overlap_frac, full.dq_mask), (32.0, 0.05, false));
        assert_eq!(full.reject, RejectRequest::On { snr: 4.0, scale: 1.0 });
        assert_eq!(full.solve, TileSolveOptions { online: true, use_twomass: false, max_sip_order: Some(3) });

        let like: MosaicRequest =
            serde_json::from_value(json!({ "canvas": { "mode": "like", "path": "m.fits" }, "reject": { "mode": "off" } })).unwrap();
        assert_eq!(like.canvas, CanvasRequest::Like { path: "m.fits".to_string() });
        assert_eq!(like.reject, RejectRequest::Off);
        for wrong in [json!({ "featherPx": 3 }), json!({ "reject": { "mode": "on" } }), json!({ "canvas": { "mode": "like" } })] {
            assert!(serde_json::from_value::<MosaicRequest>(wrong.clone()).is_err(), "{wrong} was accepted");
        }
    }
}
