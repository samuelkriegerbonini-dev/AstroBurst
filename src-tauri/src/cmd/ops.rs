use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::Instant;

use anyhow::{anyhow, bail, Context};
use ndarray::Array2;
use serde_json::{json, Map, Value};
use tauri::Manager;

use crate::cmd::common::{
    blocking_cmd, cached_header, derived_output_header, image_ref, invalidate_written, load_cached_full,
    load_from_cache_or_disk, output_stem, resolve_output_dir, save_auto_stf_preview_png, save_output_preview, source_path,
    try_extract_rgb_resolved, write_derived_fits, OutputValues, ResolvedRgbImage, MAX_PREVIEW_DIM,
};
use crate::cmd::helpers;
use crate::cmd::output::dir_info;
use crate::cmd::pixelmath::output_header as pixelmath_header;
use crate::cmd::processing::{
    abproc_for_mode, has_only_cube_planes, is_cube_header, source_header, ABPROC_BG_CORRECTED, ABPROC_HDRMT,
    ABPROC_LHE, EVENT_HDR_PROGRESS, EVENT_LHE_PROGRESS, SUFFIX_DENOISED, TONE_CUBE_ERROR,
};
use crate::core::analysis::deconvolution::provided_psf_kernel;
use crate::core::history::id::{canonical_ref, image_id, rebase_ref};
use crate::core::history::kernels::{read_kernel, write_kernel};
use crate::core::history::{self, HistoryDoc, HistoryError, ImageInfo, NewNode, Node, NodeOutput};
use crate::core::imaging::push_history;
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{compute_linked_stf_with_stats, make_stf_u8_fn, AutoStfConfig};
use crate::core::ops::background::BackgroundModel;
use crate::core::ops::pixelmath::SlotLoader;
use crate::core::ops::psf::PsfSpec;
use crate::core::ops::{run_op, OpCompanions, OpInput, OpOutput, OpSpec};
use crate::infra::cache::ImageEntry;
use crate::infra::fits::writer::write_fits_rgb;
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    EVENT_DECONV_PROGRESS, EVENT_WAVELET_PROGRESS, HEADER_DEBAND, PROGRESS_EVENT, PROGRESS_STEPS, RES_AXIS,
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_HISTORY_ROOT, RES_IS_RGB, RES_MODEL_PNG, RES_PNG_PATH,
    SUFFIX_DECONV, SUFFIX_MASKED_STRETCH,
};
use crate::types::header::HduHeader;
use crate::types::image_ref::path_key;

pub(crate) const IMAGE_BUSY: &str = "a step is still running on this image";
pub(crate) const HISTORY_SWEEPING: &str = "history cleanup is running; try again in a moment";
const SOURCE_CHANGED: &str =
    "the source of this history changed since it was recorded; re-select the file or start a new history";
const NOTHING_TO_UNDO: &str = "nothing to undo: the original image is already shown";
const NOTHING_TO_REDO: &str = "nothing to redo from the current step";
const NO_HISTORY: &str = "this image has no processing history yet";
const MODEL_SUFFIX: &str = "model";
const KEY_NODE: &str = "node";
const KEY_PARENT: &str = "parent";
const KEY_IMAGE_ID: &str = "image_id";
const KEY_HISTORY_DIR: &str = "history_dir";
const KEY_HEAD: &str = "head";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Holder {
    Run,
    Sweep,
}

impl Holder {
    fn refusal(self) -> &'static str {
        match self {
            Self::Run => IMAGE_BUSY,
            Self::Sweep => HISTORY_SWEEPING,
        }
    }
}

static RUN_LOCKS: LazyLock<Mutex<HashMap<String, Holder>>> = LazyLock::new(Default::default);

fn held_runs() -> MutexGuard<'static, HashMap<String, Holder>> {
    RUN_LOCKS.lock().unwrap_or_else(|e| e.into_inner())
}

fn run_key(image_dir: &Path) -> String {
    path_key(&image_dir.to_string_lossy())
}

pub(crate) struct RunGuard(String);

impl RunGuard {
    pub(crate) fn acquire(image_dir: &Path) -> anyhow::Result<Self> {
        Self::hold(image_dir, Holder::Run)
    }

    pub(crate) fn for_sweep(image_dir: &Path) -> anyhow::Result<Self> {
        Self::hold(image_dir, Holder::Sweep)
    }

    fn hold(image_dir: &Path, holder: Holder) -> anyhow::Result<Self> {
        let key = run_key(image_dir);
        let mut held = held_runs();
        if let Some(existing) = held.get(&key) {
            bail!(existing.refusal());
        }
        held.insert(key.clone(), holder);
        Ok(Self(key))
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        held_runs().remove(&self.0);
    }
}

struct Scope {
    image_key: String,
    base_path: Option<String>,
    id: String,
    dir: PathBuf,
}

impl Scope {
    fn new(root: &Path, image_key: &str, base_path: Option<&str>) -> Self {
        let base_path = base_path.map(str::trim).filter(|b| !b.is_empty()).map(str::to_string);
        let id = image_id(image_key, base_path.as_deref());
        Self { dir: history::image_dir(root, &id), image_key: image_key.to_string(), base_path, id }
    }

    fn rebased_base(&self) -> Option<&str> {
        rebase_ref(&self.image_key, self.base_path.as_deref()).and(self.base_path.as_deref())
    }

    fn root_input(&self) -> &str {
        self.rebased_base().unwrap_or(&self.image_key)
    }

    fn is_root(&self, input: &str) -> bool {
        canonical_ref(input) == canonical_ref(self.root_input())
    }

    fn is_source_file(&self, path: &str) -> bool {
        path_key(&image_ref(path).path) == path_key(&image_ref(&self.image_key).path)
    }

    fn source_unchanged(&self, doc: &HistoryDoc) -> bool {
        doc.image.fingerprint_matches(&self.image_key, self.base_path.as_deref())
    }

    fn file(&self, name: &str) -> String {
        self.dir.join(name).to_string_lossy().into_owned()
    }

    fn existing(&self) -> anyhow::Result<HistoryDoc> {
        history::load(&self.dir)?.ok_or_else(|| anyhow!(NO_HISTORY))
    }

    fn load_or_new(&self) -> anyhow::Result<HistoryDoc> {
        if let Some(doc) = history::load(&self.dir)? {
            return Ok(doc);
        }
        let image = ImageInfo::of(&self.image_key, self.base_path.as_deref())
            .with_context(|| format!("cannot start a history for {}", self.image_key))?;
        Ok(HistoryDoc::new(image, &self.image_key, self.base_path.as_deref()))
    }

    fn node_fits(&self, doc: &HistoryDoc, id: &str) -> anyhow::Result<String> {
        let node = doc.node(id).ok_or_else(|| HistoryError::UnknownNode(id.to_string()))?;
        Ok(self.file(&node.output.path))
    }
}

fn resolve_parent(scope: &Scope, doc: &HistoryDoc, input_path: &str) -> anyhow::Result<Option<String>> {
    let input = path_key(&image_ref(input_path).path);
    let dir = path_key(&scope.dir.to_string_lossy());
    match input.strip_prefix(dir.as_str()).and_then(|rest| rest.strip_prefix('/')) {
        Some(name) => {
            if let Some(node) = doc.nodes.iter().find(|n| path_key(&n.output.path) == name) {
                return Ok(Some(node.id.clone()));
            }
        }
        None if scope.is_root(input_path) => return Ok(None),
        None => {}
    }
    bail!("{input_path} is not a node of this image's history; revert to original or re-select the file")
}

enum LoadedInput {
    Mono { entry: ImageEntry, header: Option<HduHeader> },
    Rgb(Box<ResolvedRgbImage>),
}

impl LoadedInput {
    fn load(path: &str, op: &OpSpec) -> anyhow::Result<Self> {
        let is_tone = matches!(op, OpSpec::Tone { .. });
        let rgb = if image_ref(path).is_synthetic() { None } else { try_extract_rgb_resolved(path)? };
        if let Some(rgb) = rgb {
            return Ok(Self::Rgb(Box::new(rgb)));
        }
        let entry = match load_from_cache_or_disk(path) {
            Ok(entry) => entry,
            Err(_) if is_tone && has_only_cube_planes(path) => bail!(TONE_CUBE_ERROR),
            Err(e) => return Err(e),
        };
        let header = source_header(path, &entry);
        if is_tone && is_cube_header(header.as_ref()) {
            bail!(TONE_CUBE_ERROR);
        }
        Ok(Self::Mono { entry, header })
    }

    fn header(&self) -> Option<&HduHeader> {
        match self {
            Self::Mono { header, .. } => header.as_ref(),
            Self::Rgb(rgb) => Some(&rgb.header),
        }
    }

    fn input(&self) -> OpInput<'_> {
        match self {
            Self::Mono { entry, header } => OpInput::mono(entry.arr(), header.as_ref()),
            Self::Rgb(rgb) => OpInput::rgb([&rgb.r, &rgb.g, &rgb.b], Some(&rgb.header)),
        }
    }
}

fn abproc(op: &OpSpec) -> &'static str {
    match op {
        OpSpec::Background { model: BackgroundModel::Spline(config), .. } => abproc_for_mode(&config.mode),
        OpSpec::Background { .. } => ABPROC_BG_CORRECTED,
        OpSpec::WaveletDenoise { .. } => SUFFIX_DENOISED,
        OpSpec::RlDeconv { .. } => SUFFIX_DECONV,
        OpSpec::MaskedStretch { .. } => SUFFIX_MASKED_STRETCH,
        OpSpec::Lhe { .. } => ABPROC_LHE,
        OpSpec::Hdrmt { .. } => ABPROC_HDRMT,
        other => other.name(),
    }
}

fn set_text(header: &mut HduHeader, key: &str, value: &str) {
    header.set(key, value.to_string());
    if let Some(keys) = header.string_keys.as_mut() {
        keys.insert(key.to_string());
    }
}

fn node_header(op: &OpSpec, source: Option<&HduHeader>, out: &OpOutput, input_path: &str) -> HduHeader {
    let mut header = match op {
        OpSpec::Pixelmath { expression, .. } => pixelmath_header(source, expression, &source_path(input_path)),
        _ => derived_output_header(source, abproc(op), out.values),
    };
    for line in &out.history_lines {
        push_history(&mut header, line);
    }
    if let Some(axis) = out.extras.get(RES_AXIS).and_then(Value::as_str) {
        header.set(HEADER_DEBAND, axis.to_string());
    }
    header
}

fn stamp(header: &mut HduHeader, image_id: &str, node_id: &str) {
    set_text(header, history::HEADER_ABHIST, image_id);
    set_text(header, history::HEADER_ABNODE, node_id);
}

fn stored_kernel(scope: &Scope, op: &OpSpec) -> anyhow::Result<Option<Array2<f32>>> {
    match op {
        OpSpec::RlDeconv { psf: PsfSpec::Kernel { hash, rows, cols }, .. } => read_kernel(&scope.dir, hash, *rows, *cols)
            .map(Some)
            .with_context(|| format!("the PSF kernel {hash} of this history cannot be read")),
        _ => Ok(None),
    }
}

fn with_stored_kernel(scope: &Scope, op: OpSpec, supplied: Option<Vec<Vec<f32>>>) -> anyhow::Result<OpSpec> {
    let (Some(rows), OpSpec::RlDeconv { v, config, psf: PsfSpec::Kernel { .. } }) = (supplied, &op) else {
        return Ok(op);
    };
    provided_psf_kernel(&rows)?;
    let size = rows.len();
    let kernel = Array2::from_shape_fn((size, size), |(y, x)| rows[y][x]);
    let hash = write_kernel(&scope.dir, &kernel)?;
    Ok(OpSpec::RlDeconv { v: *v, config: config.clone(), psf: PsfSpec::Kernel { hash, rows: size, cols: size } })
}

fn compute(scope: &Scope, op: &OpSpec, input_path: &str, progress: Option<&ProgressHandle>) -> anyhow::Result<(OpOutput, HduHeader)> {
    let loaded = LoadedInput::load(input_path, op)?;
    let kernel = stored_kernel(scope, op)?;
    let load: SlotLoader = &load_cached_full;
    let companions = OpCompanions { psf_kernel: kernel.as_ref(), load_slot: Some(load) };
    let out = run_op(op, &loaded.input().with_companions(companions), progress)?;
    let header = node_header(op, loaded.header(), &out, input_path);
    Ok((out, header))
}

fn node_output(id: &str, out: &OpOutput) -> NodeOutput {
    let (rows, cols) = out.planes[0].dim();
    let extras: &[&str] = if out.model.is_some() { &[MODEL_SUFFIX] } else { &[] };
    NodeOutput::named(id, [cols, rows], out.planes.len(), out.values, extras)
}

fn save_rgb_preview(planes: [&Array2<f32>; 3], values: OutputValues, png: &str) -> anyhow::Result<()> {
    let [r, g, b] = planes;
    if values == OutputValues::DisplayReferred {
        let unit = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        return helpers::render_rgb_preview_with_stf(r, g, b, unit, unit, unit, png, MAX_PREVIEW_DIM);
    }
    let stats = planes.map(compute_image_stats);
    let (stf, combined) = compute_linked_stf_with_stats(&stats[0], &stats[1], &stats[2], &AutoStfConfig::default());
    let linked = || make_stf_u8_fn(&stf, &combined);
    helpers::render_rgb_preview_with_stf(r, g, b, linked(), linked(), linked(), png, MAX_PREVIEW_DIM)
}

fn write_node_files(scope: &Scope, output: &NodeOutput, header: &HduHeader, out: &OpOutput) -> anyhow::Result<()> {
    fs::create_dir_all(&scope.dir)?;
    let fits = scope.file(&output.path);
    let png = scope.file(&output.png);
    match &out.planes[..] {
        [r, g, b] => {
            crate::core::cube::cache::GLOBAL_CUBE_CACHE.invalidate(&fits);
            let written = write_fits_rgb(&fits, r, g, b, Some(header));
            invalidate_written(&fits);
            written?;
            save_rgb_preview([r, g, b], out.values, &png)?;
        }
        [plane] => {
            write_derived_fits(&fits, plane, Some(header))?;
            save_output_preview(plane, out.values, &png)?;
        }
        planes => bail!("{} planes cannot be written as one history node", planes.len()),
    }
    if let (Some(model), Some(extra)) = (&out.model, output.extras.first()) {
        save_auto_stf_preview_png(model, &scope.file(extra))?;
    }
    Ok(())
}

fn remove_node_files(scope: &Scope, output: &NodeOutput) {
    for file in output.files() {
        let path = scope.dir.join(file);
        if let Err(e) = fs::remove_file(&path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("history: could not remove the node file {}: {e}", path.display());
            }
        }
    }
}

struct Recorded {
    node: Node,
    out: OpOutput,
}

fn replayable(scope: &Scope, doc: &HistoryDoc, parent: Option<&str>, op: &OpSpec) -> bool {
    let root_is_kept = parent.is_some() || (scope.rebased_base().is_none() && scope.source_unchanged(doc));
    let slots_are_kept = match op {
        OpSpec::Pixelmath { slots, .. } => slots.iter().all(|slot| scope.is_source_file(&slot.path)),
        _ => true,
    };
    root_is_kept && slots_are_kept
}

fn chain_stage(op: &OpSpec) -> Option<u8> {
    Some(match op {
        OpSpec::Background { .. } => 0,
        OpSpec::WaveletDenoise { .. } => 1,
        OpSpec::RlDeconv { .. } => 2,
        OpSpec::Arcsinh { .. } | OpSpec::Ghs { .. } | OpSpec::MaskedStretch { .. } => 3,
        OpSpec::Lhe { .. } | OpSpec::Hdrmt { .. } => 4,
        OpSpec::Tone { .. } => 5,
        OpSpec::Pixelmath { .. } => return None,
    })
}

fn check_stage_order(doc: &HistoryDoc, parent: Option<&str>, op: &OpSpec) -> anyhow::Result<()> {
    let (Some(stage), Some(parent)) = (chain_stage(op), parent) else {
        return Ok(());
    };
    if let Some(later) = doc.ancestry(parent).into_iter().rev().find(|n| chain_stage(&n.op).is_none_or(|s| s >= stage)) {
        bail!(
            "{} cannot follow {} ({}): along one branch every step comes after the ones before it; run it on an earlier step to start a new branch",
            op.name(),
            later.op.name(),
            later.id
        );
    }
    Ok(())
}

fn record_node(
    scope: &Scope,
    doc: &mut HistoryDoc,
    parent: Option<String>,
    op: OpSpec,
    input_path: &str,
    progress: Option<&ProgressHandle>,
) -> anyhow::Result<Recorded> {
    let t0 = Instant::now();
    if parent.is_none() && !scope.source_unchanged(doc) {
        history::restamp(doc, ImageInfo::of(&scope.image_key, scope.base_path.as_deref())?);
    }
    let (out, mut header) = compute(scope, &op, input_path, progress)?;
    let id = history::next_node_id(&scope.dir, doc);
    stamp(&mut header, &scope.id, &id);
    let output = node_output(&id, &out);
    if let Err(e) = write_node_files(scope, &output, &header, &out) {
        remove_node_files(scope, &output);
        return Err(e);
    }
    let replayable = replayable(scope, doc, parent.as_deref(), &op);
    let elapsed_ms = t0.elapsed().as_millis() as u64;
    let new = NewNode { id: id.clone(), parent, op, mask: None, output: output.clone(), elapsed_ms, replayable };
    if let Err(e) = history::append(&scope.dir, doc, new) {
        remove_node_files(scope, &output);
        return Err(e.into());
    }
    let node = doc.node(&id).cloned().ok_or_else(|| HistoryError::UnknownNode(id.clone()))?;
    Ok(Recorded { node, out })
}

fn rebuild(scope: &Scope, doc: &mut HistoryDoc, target: &str) -> anyhow::Result<Vec<String>> {
    history::mark_missing_on_path(&scope.dir, doc, target);
    let plan = history::replay_plan(doc, target)?;
    if let Some(kept) = plan.steps.iter().find(|id| doc.node(id).is_some_and(|n| !n.replayable)) {
        bail!(
            "{kept} cannot be rebuilt: it read a file this history does not keep (a chain base, a changed source or a PixelMath slot), and its own files were removed outside AstroBurst"
        );
    }
    let mut input = match plan.from.as_deref() {
        Some(id) => scope.node_fits(doc, id)?,
        None if !scope.source_unchanged(doc) => bail!(SOURCE_CHANGED),
        None => scope.root_input().to_string(),
    };
    for id in &plan.steps {
        let node = doc.node(id).cloned().ok_or_else(|| HistoryError::UnknownNode(id.clone()))?;
        let (out, mut header) = compute(scope, &node.op, &input, None).with_context(|| format!("replaying {id}"))?;
        let replayed = node_output(id, &out);
        if (replayed.dims, replayed.planes) != (node.output.dims, node.output.planes) {
            bail!(
                "replaying {id} produced {:?} × {} planes but the history recorded {:?} × {}; the source changed since it was recorded",
                replayed.dims,
                replayed.planes,
                node.output.dims,
                node.output.planes
            );
        }
        stamp(&mut header, &scope.id, id);
        if let Err(e) = write_node_files(scope, &node.output, &header, &out) {
            remove_node_files(scope, &node.output);
            return Err(e.context(format!("replaying {id}")));
        }
        history::mark_restored(&scope.dir, doc, id)?;
        input = scope.file(&node.output.path);
    }
    Ok(plan.steps)
}

fn input_after(scope: &Scope, doc: &mut HistoryDoc, parent: Option<&str>) -> anyhow::Result<String> {
    match parent {
        Some(id) => {
            rebuild(scope, doc, id)?;
            scope.node_fits(doc, id)
        }
        None => Ok(scope.root_input().to_string()),
    }
}

fn node_present(scope: &Scope, node: &Node) -> bool {
    node.present && node.output.files().all(|file| scope.dir.join(file).exists())
}

fn node_view(scope: &Scope, node: &Node) -> Value {
    json!({
        "id": node.id,
        KEY_PARENT: node.parent,
        "op": node.op,
        RES_FITS_PATH: scope.file(&node.output.path),
        RES_PNG_PATH: scope.file(&node.output.png),
        "extras": node.output.extras.iter().map(|e| scope.file(e)).collect::<Vec<_>>(),
        RES_DIMENSIONS: node.output.dims,
        "planes": node.output.planes,
        "domain": node.output.domain,
        "present": node_present(scope, node),
    })
}

fn head_view(scope: &Scope, doc: &HistoryDoc) -> Value {
    let head = doc.head.as_deref().and_then(|id| doc.node(id));
    json!({
        KEY_IMAGE_ID: scope.id,
        KEY_HISTORY_DIR: scope.dir,
        KEY_HEAD: doc.head,
        KEY_NODE: head.map(|n| node_view(scope, n)),
        "root_path": scope.root_input(),
        "can_undo": doc.head.is_some(),
        "can_redo": doc.redo_target().is_some(),
        "node_count": doc.nodes.len(),
    })
}

fn run_response(scope: &Scope, recorded: &Recorded) -> Value {
    let Recorded { node, out } = recorded;
    let mut response = match &out.extras {
        Value::Object(extras) => extras.clone(),
        _ => Map::new(),
    };
    let fields = [
        (KEY_NODE, json!(node.id)),
        (KEY_PARENT, json!(node.parent)),
        (KEY_IMAGE_ID, json!(scope.id)),
        (KEY_HISTORY_DIR, json!(scope.dir)),
        (RES_FITS_PATH, json!(scope.file(&node.output.path))),
        (RES_PNG_PATH, json!(scope.file(&node.output.png))),
        (RES_MODEL_PNG, json!(node.output.extras.first().map(|e| scope.file(e)))),
        (RES_DIMENSIONS, json!(node.output.dims)),
        (RES_IS_RGB, json!(node.output.planes == 3)),
        ("domain", json!(node.output.domain)),
        (RES_ELAPSED_MS, json!(node.elapsed_ms)),
    ];
    for (key, value) in fields {
        response.insert(key.to_string(), value);
    }
    Value::Object(response)
}

pub(crate) struct RunRequest<'a> {
    pub image_key: &'a str,
    pub base_path: Option<&'a str>,
    pub input_path: &'a str,
    pub op: OpSpec,
    pub psf_kernel: Option<Vec<Vec<f32>>>,
}

pub(crate) fn run_recorded(root: &Path, request: RunRequest, progress: Option<&ProgressHandle>) -> anyhow::Result<Value> {
    let scope = Scope::new(root, request.image_key, request.base_path);
    let _guard = RunGuard::acquire(&scope.dir)?;
    let mut doc = scope.load_or_new()?;
    let parent = resolve_parent(&scope, &doc, request.input_path)?;
    check_stage_order(&doc, parent.as_deref(), &request.op)?;
    if let Some(parent) = parent.as_deref() {
        rebuild(&scope, &mut doc, parent)?;
    }
    let op = with_stored_kernel(&scope, request.op, request.psf_kernel)?;
    let recorded = record_node(&scope, &mut doc, parent, op, request.input_path, progress)?;
    Ok(run_response(&scope, &recorded))
}

pub(crate) enum HeadMove<'a> {
    Undo,
    Redo,
    To(Option<&'a str>),
}

pub(crate) fn move_head(root: &Path, image_key: &str, base_path: Option<&str>, head_move: HeadMove) -> anyhow::Result<Value> {
    let scope = Scope::new(root, image_key, base_path);
    let _guard = RunGuard::acquire(&scope.dir)?;
    let mut doc = scope.existing()?;
    let target = match head_move {
        HeadMove::Undo => {
            let head = doc.head.as_deref().ok_or_else(|| anyhow!(NOTHING_TO_UNDO))?;
            doc.node(head).and_then(|n| n.parent.clone())
        }
        HeadMove::Redo => Some(doc.redo_target().ok_or_else(|| anyhow!(NOTHING_TO_REDO))?.id.clone()),
        HeadMove::To(id) => id.map(str::to_string),
    };
    history::set_head(&scope.dir, &mut doc, target.as_deref())?;
    Ok(head_view(&scope, &doc))
}

pub(crate) fn rehydrate(root: &Path, image_key: &str, base_path: Option<&str>, node: &str) -> anyhow::Result<Value> {
    let scope = Scope::new(root, image_key, base_path);
    let _guard = RunGuard::acquire(&scope.dir)?;
    let mut doc = scope.existing()?;
    let rebuilt = rebuild(&scope, &mut doc, node)?;
    let target = doc.node(node).ok_or_else(|| HistoryError::UnknownNode(node.to_string()))?;
    Ok(json!({
        KEY_IMAGE_ID: scope.id,
        KEY_HEAD: doc.head,
        KEY_NODE: node_view(&scope, target),
        "rebuilt": rebuilt,
    }))
}

pub(crate) struct RerunRequest<'a> {
    pub image_key: &'a str,
    pub base_path: Option<&'a str>,
    pub node: &'a str,
    pub op: Option<OpSpec>,
    pub replay_downstream: bool,
    pub psf_kernel: Option<Vec<Vec<f32>>>,
}

pub(crate) fn rerun_progress_op(
    root: &Path,
    image_key: &str,
    base_path: Option<&str>,
    node: &str,
    requested: Option<&OpSpec>,
) -> Option<OpSpec> {
    let stored = || Scope::new(root, image_key, base_path).existing().ok()?.node(node).map(|n| n.op.clone());
    requested.cloned().or_else(stored)
}

pub(crate) fn rerun(root: &Path, request: RerunRequest, progress: Option<&ProgressHandle>) -> anyhow::Result<Value> {
    let scope = Scope::new(root, request.image_key, request.base_path);
    let _guard = RunGuard::acquire(&scope.dir)?;
    let mut doc = scope.existing()?;
    let original = doc.node(request.node).cloned().ok_or_else(|| HistoryError::UnknownNode(request.node.to_string()))?;
    if let Some(op) = request.op.as_ref().filter(|op| op.name() != original.op.name()) {
        bail!(
            "{} is a {} step and cannot be re-run as {}; a re-run keeps the kind of step and changes its settings",
            original.id,
            original.op.name(),
            op.name()
        );
    }
    let downstream = if request.replay_downstream { history::downstream_of(&doc, &original.id) } else { Vec::new() };
    let mut input = input_after(&scope, &mut doc, original.parent.as_deref())?;
    let first = with_stored_kernel(&scope, request.op.unwrap_or(original.op), request.psf_kernel)?;
    let previous_head = doc.head.clone();
    let mut parent = original.parent;
    let mut created = Vec::new();
    let mut first_run = Value::Null;
    for (index, op) in std::iter::once(first).chain(downstream.into_iter().map(|n| n.op)).enumerate() {
        let step_progress = if index == 0 { progress } else { None };
        let recorded = match record_node(&scope, &mut doc, parent.take(), op, &input, step_progress) {
            Ok(recorded) => recorded,
            Err(e) => {
                if !created.is_empty() {
                    if let Err(restore) = history::set_head(&scope.dir, &mut doc, previous_head.as_deref()) {
                        log::warn!("history: a failed re-run could not move the head back to {previous_head:?}: {restore}");
                    }
                }
                return Err(e);
            }
        };
        if index == 0 {
            first_run = run_response(&scope, &recorded);
        }
        input = scope.file(&recorded.node.output.path);
        parent = Some(recorded.node.id.clone());
        created.push(recorded.node.id);
    }
    let mut view = head_view(&scope, &doc);
    view["created"] = json!(created);
    view["run"] = first_run;
    Ok(view)
}

pub(crate) fn delete_branch(root: &Path, image_key: &str, base_path: Option<&str>, node: &str) -> anyhow::Result<Value> {
    let scope = Scope::new(root, image_key, base_path);
    let _guard = RunGuard::acquire(&scope.dir)?;
    let mut doc = scope.existing()?;
    let removed = history::remove_subtree(&scope.dir, &mut doc, node)?;
    for gone in &removed {
        remove_node_files(&scope, &gone.output);
    }
    let mut view = head_view(&scope, &doc);
    view["removed"] = json!(removed.iter().map(|n| n.id.as_str()).collect::<Vec<_>>());
    Ok(view)
}

fn read_by(scope: &Scope, op: &OpSpec, path: &str) -> bool {
    let key = |p: &str| path_key(&image_ref(p).path);
    let slots: &[_] = match op {
        OpSpec::Pixelmath { slots, .. } => slots,
        _ => &[],
    };
    let target = key(path);
    key(scope.root_input()) == target || slots.iter().any(|slot| key(&slot.path) == target)
}

pub(crate) fn copy_to_output(root: &Path, image_key: &str, base_path: Option<&str>, node: &str, output_dir: &str) -> anyhow::Result<Value> {
    let scope = Scope::new(root, image_key, base_path);
    let _guard = RunGuard::acquire(&scope.dir)?;
    let doc = scope.existing()?;
    let step = doc.node(node).ok_or_else(|| HistoryError::UnknownNode(node.to_string()))?;
    let output_dir = resolve_output_dir(output_dir)?;
    let stem = output_stem(scope.root_input());
    let suffix = abproc(&step.op);
    let named = |name: String| Path::new(&output_dir).join(name).to_string_lossy().into_owned();
    let preferred = named(format!("{stem}_{suffix}.fits"));
    let copy = if read_by(&scope, &step.op, &preferred) { named(format!("{stem}_{node}_{suffix}.fits")) } else { preferred };
    let copied = fs::copy(scope.file(&step.output.path), &copy).with_context(|| format!("cannot copy {node} to {copy}"));
    invalidate_written(&copy);
    copied?;
    Ok(json!({ KEY_NODE: node, RES_FITS_PATH: copy }))
}

pub(crate) fn kernel_rows(root: &Path, image_key: &str, base_path: Option<&str>, hash: &str, rows: usize, cols: usize) -> anyhow::Result<Value> {
    let scope = Scope::new(root, image_key, base_path);
    let kernel = read_kernel(&scope.dir, hash, rows, cols).with_context(|| format!("the PSF kernel {hash} of this history cannot be read"))?;
    let values: Vec<Vec<f32>> = kernel.rows().into_iter().map(|row| row.to_vec()).collect();
    Ok(json!({ "hash": hash, "kernel": values }))
}

fn refresh_presence(scope: &Scope, doc: &mut HistoryDoc) {
    let present: Vec<bool> = doc.nodes.iter().map(|node| node_present(scope, node)).collect();
    for (node, present) in doc.nodes.iter_mut().zip(present) {
        node.present = present;
    }
}

pub(crate) fn history_state(root: &Path, image_key: &str, base_path: Option<&str>) -> anyhow::Result<Value> {
    let scope = Scope::new(root, image_key, base_path);
    let busy = held_runs().contains_key(&run_key(&scope.dir));
    let (doc, read_only, fingerprint_matches) = match history::load(&scope.dir) {
        Ok(Some(mut doc)) => {
            refresh_presence(&scope, &mut doc);
            let matches = scope.source_unchanged(&doc);
            (serde_json::to_value(&doc)?, Value::Null, json!(matches))
        }
        Ok(None) => (Value::Null, Value::Null, json!(true)),
        Err(HistoryError::ReadOnly(reason)) => {
            let raw: Value = serde_json::from_slice(&fs::read(history::sidecar_path(&scope.dir))?)?;
            (raw, json!(reason), Value::Null)
        }
        Err(e) => return Err(e.into()),
    };
    Ok(json!({
        KEY_IMAGE_ID: scope.id,
        KEY_HISTORY_DIR: scope.dir,
        "root_path": scope.root_input(),
        "doc": doc,
        "read_only": read_only,
        "fingerprint_matches": fingerprint_matches,
        "busy": busy,
    }))
}

pub(crate) fn estimate(root: &Path, input_path: &str) -> anyhow::Result<Value> {
    let header = cached_header(input_path)?;
    let axis = |key: &str| {
        header
            .get_i64(key)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| anyhow!("{input_path} has no {key}; the size of the next step is unknown"))
    };
    let (cols, rows) = (axis("NAXIS1")?, axis("NAXIS2")?);
    let planes = if image_ref(input_path).is_auto() && header.get_i64("NAXIS3") == Some(3) { 3 } else { 1 };
    let (history_bytes, history_files) = if root.is_dir() { dir_info(root)? } else { (0, 0) };
    Ok(json!({
        "next_node_bytes": history::estimate_next_node_bytes([cols, rows], planes),
        RES_DIMENSIONS: [cols, rows],
        "planes": planes,
        "history_bytes": history_bytes,
        "history_files": history_files,
        RES_HISTORY_ROOT: root,
    }))
}

fn progress_for(app: &tauri::AppHandle, op: &OpSpec) -> Option<ProgressHandle> {
    let (event, total) = match op {
        OpSpec::Background { .. } => (PROGRESS_EVENT, PROGRESS_STEPS as u64),
        OpSpec::WaveletDenoise { config, .. } => (EVENT_WAVELET_PROGRESS, (config.num_scales * 2 + 1) as u64),
        OpSpec::RlDeconv { config, .. } => (EVENT_DECONV_PROGRESS, config.iterations as u64),
        OpSpec::Lhe { .. } => (EVENT_LHE_PROGRESS, 1),
        OpSpec::Hdrmt { .. } => (EVENT_HDR_PROGRESS, 1),
        _ => return None,
    };
    Some(ProgressHandle::new(app, event, total))
}

fn history_root_of(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| history::history_root(&dir))
        .map_err(|e| format!("the app data folder is unknown: {e}"))
}

#[tauri::command]
pub async fn run_op_cmd(
    app: tauri::AppHandle,
    image_key: String,
    base_path: Option<String>,
    input_path: String,
    op: OpSpec,
    psf_kernel: Option<Vec<Vec<f32>>>,
) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    let progress = progress_for(&app, &op);
    blocking_cmd!({
        let request = RunRequest { image_key: &image_key, base_path: base_path.as_deref(), input_path: &input_path, op, psf_kernel };
        run_recorded(&root, request, progress.as_ref())
    })
}

#[tauri::command]
pub async fn history_get_cmd(app: tauri::AppHandle, image_key: String, base_path: Option<String>) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    blocking_cmd!(history_state(&root, &image_key, base_path.as_deref()))
}

#[tauri::command]
pub async fn history_set_head_cmd(
    app: tauri::AppHandle,
    image_key: String,
    base_path: Option<String>,
    node: Option<String>,
) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    blocking_cmd!(move_head(&root, &image_key, base_path.as_deref(), HeadMove::To(node.as_deref())))
}

#[tauri::command]
pub async fn history_undo_cmd(app: tauri::AppHandle, image_key: String, base_path: Option<String>) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    blocking_cmd!(move_head(&root, &image_key, base_path.as_deref(), HeadMove::Undo))
}

#[tauri::command]
pub async fn history_redo_cmd(app: tauri::AppHandle, image_key: String, base_path: Option<String>) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    blocking_cmd!(move_head(&root, &image_key, base_path.as_deref(), HeadMove::Redo))
}

#[tauri::command]
pub async fn history_rerun_cmd(
    app: tauri::AppHandle,
    image_key: String,
    base_path: Option<String>,
    node: String,
    op: Option<OpSpec>,
    replay_downstream: Option<bool>,
    psf_kernel: Option<Vec<Vec<f32>>>,
) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    blocking_cmd!({
        let request = RerunRequest {
            image_key: &image_key,
            base_path: base_path.as_deref(),
            node: &node,
            op,
            replay_downstream: replay_downstream.unwrap_or(false),
            psf_kernel,
        };
        let shown = rerun_progress_op(&root, request.image_key, request.base_path, request.node, request.op.as_ref());
        let progress = shown.as_ref().and_then(|op| progress_for(&app, op));
        rerun(&root, request, progress.as_ref())
    })
}

#[tauri::command]
pub async fn history_delete_branch_cmd(
    app: tauri::AppHandle,
    image_key: String,
    base_path: Option<String>,
    node: String,
) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    blocking_cmd!(delete_branch(&root, &image_key, base_path.as_deref(), &node))
}

#[tauri::command]
pub async fn history_copy_to_output_cmd(
    app: tauri::AppHandle,
    image_key: String,
    base_path: Option<String>,
    node: String,
    output_dir: String,
) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    blocking_cmd!(copy_to_output(&root, &image_key, base_path.as_deref(), &node, &output_dir))
}

#[tauri::command]
pub async fn history_rehydrate_cmd(app: tauri::AppHandle, image_key: String, base_path: Option<String>, node: String) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    blocking_cmd!(rehydrate(&root, &image_key, base_path.as_deref(), &node))
}

#[tauri::command]
pub async fn history_estimate_cmd(app: tauri::AppHandle, input_path: String) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    blocking_cmd!(estimate(&root, &input_path))
}

#[tauri::command]
pub async fn history_kernel_cmd(
    app: tauri::AppHandle,
    image_key: String,
    base_path: Option<String>,
    hash: String,
    rows: usize,
    cols: usize,
) -> Result<Value, String> {
    let root = history_root_of(&app)?;
    blocking_cmd!(kernel_rows(&root, &image_key, base_path.as_deref(), &hash, rows, cols))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::{Path, PathBuf};

    use ndarray::Array2;

    use crate::cmd::common::{cached_header, extract_image_resolved, try_extract_rgb_resolved, HEADER_ABPROC, HEADER_DISPLAY_REFERRED};
    use crate::core::history::{self, id::image_id};
    use crate::core::ops::tone::{ToneCurveInput, ToneLevelsInput};
    use crate::core::ops::{run_op, OpInput, OpSpec, OpVersion};
    use crate::infra::fits::writer::{write_fits_mono, write_fits_rgb};
    use crate::types::constants::{RES_DIMENSIONS, RES_FITS_PATH, RES_IS_RGB, RES_PNG_PATH, RES_REPLAYABLE_NODES};

    struct Lcg(u64);

    impl Lcg {
        fn uniform(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    fn sky(seed: u64, scale: f32) -> Array2<f32> {
        let mut rng = Lcg(seed);
        let mut arr = Array2::from_shape_fn((48, 64), |(y, x)| 100.0 + 0.3 * x as f32 + 0.2 * y as f32 + 4.0 * rng.uniform());
        for (y, x, peak) in [(10usize, 12usize, 900.0f32), (30, 40, 1500.0), (20, 55, 600.0), (40, 8, 1200.0)] {
            for dy in 0..3 {
                for dx in 0..3 {
                    arr[[y + dy - 1, x + dx - 1]] += if dy == 1 && dx == 1 { peak } else { peak * 0.3 };
                }
            }
        }
        arr.mapv(|v| v * scale)
    }

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

        fn mono(&self, name: &str, arr: &Array2<f32>) -> String {
            let path = self.data.path().join(name).to_string_lossy().to_string();
            write_fits_mono(&path, arr, None).unwrap();
            path
        }

        fn rgb(&self, name: &str, planes: [&Array2<f32>; 3]) -> String {
            let path = self.data.path().join(name).to_string_lossy().to_string();
            write_fits_rgb(&path, planes[0], planes[1], planes[2], None).unwrap();
            path
        }

        fn image_dir(&self, key: &str) -> PathBuf {
            history::image_dir(&self.root(), &image_id(key, None))
        }

        fn doc(&self, key: &str) -> history::HistoryDoc {
            history::load(&self.image_dir(key)).unwrap().expect("no sidecar was written")
        }

        fn run(&self, key: &str, input: &str, op: OpSpec) -> anyhow::Result<serde_json::Value> {
            run_recorded(&self.root(), RunRequest { image_key: key, base_path: None, input_path: input, op, psf_kernel: None }, None)
        }
    }

    fn tone() -> OpSpec {
        OpSpec::Tone {
            v: OpVersion,
            levels: Some(ToneLevelsInput { black: 0.05, gamma: 1.0, white: 0.95 }),
            curve: Some(ToneCurveInput { points: vec![[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]] }),
        }
    }

    fn arcsinh(factor: f64) -> OpSpec {
        OpSpec::Arcsinh { v: OpVersion, factor }
    }

    fn text(header: &crate::types::header::HduHeader, key: &str) -> Option<String> {
        header.get(key).map(|v| v.trim().trim_matches('\'').trim().to_string())
    }

    fn assert_bitwise(actual: &Array2<f32>, expected: &Array2<f32>, what: &str) {
        assert_eq!(actual.dim(), expected.dim(), "{what}");
        let differing = actual.iter().zip(expected.iter()).filter(|(a, e)| a.to_bits() != e.to_bits()).count();
        assert_eq!(differing, 0, "{what}: {differing} pixels differ");
    }

    fn path_of(value: &serde_json::Value, key: &str) -> String {
        value[key].as_str().unwrap_or_else(|| panic!("{key} missing in {value}")).to_string()
    }

    #[test]
    fn run_op_cmd_writes_a_node_and_sets_head() {
        let fx = Fixture::new();
        let input = sky(1, 1.0);
        let key = fx.mono("m42.fits", &input);

        let value = fx.run(&key, &key, tone()).unwrap();
        let dir = fx.image_dir(&key);
        let doc = fx.doc(&key);
        assert_eq!(doc.nodes.len(), 1);
        assert_eq!(doc.head.as_deref(), Some("n0001"));
        assert_eq!(doc.nodes[0].parent, None);
        assert_eq!(serde_json::to_value(&doc.nodes[0].op).unwrap(), serde_json::to_value(tone()).unwrap());
        assert_eq!(doc.nodes[0].output.planes, 1);
        assert!(doc.nodes[0].present);

        assert_eq!(value["node"], "n0001");
        assert_eq!(value["image_id"], image_id(&key, None));
        assert_eq!(value[RES_DIMENSIONS], serde_json::json!([64, 48]));
        assert_eq!(value[RES_IS_RGB], false);
        let fits = path_of(&value, RES_FITS_PATH);
        assert_eq!(Path::new(&fits), dir.join("n0001.fits"));
        assert!(Path::new(&path_of(&value, RES_PNG_PATH)).exists());

        let header = cached_header(&fits).unwrap();
        assert_eq!(text(&header, history::HEADER_ABHIST).as_deref(), Some(image_id(&key, None).as_str()));
        assert_eq!(text(&header, history::HEADER_ABNODE).as_deref(), Some("n0001"));
        assert_eq!(text(&header, HEADER_ABPROC).as_deref(), Some("tone"));
        assert_eq!(text(&header, HEADER_DISPLAY_REFERRED).as_deref(), Some("T"));

        let expected = run_op(&tone(), &OpInput::mono(&input, None), None).unwrap();
        assert_bitwise(&extract_image_resolved(&fits).unwrap().arr, &expected.planes[0], "node pixels");
        assert!(!fx.app.path().join("output").exists(), "a recorded run wrote under output/");
    }

    #[test]
    fn run_op_cmd_on_an_rgb_fits_writes_a_three_plane_node() {
        let fx = Fixture::new();
        let mut r = sky(2, 1.0);
        r[[0, 0]] = f32::NAN;
        let g = sky(3, 1.4);
        let b = sky(4, 0.6);
        let key = fx.rgb("colour.fits", [&r, &g, &b]);

        let value = fx.run(&key, &key, tone()).unwrap();
        assert_eq!(value[RES_IS_RGB], true);
        let doc = fx.doc(&key);
        assert_eq!(doc.nodes[0].output.planes, 3);
        let written = try_extract_rgb_resolved(&path_of(&value, RES_FITS_PATH)).unwrap().expect("a three-plane node");
        let expected = run_op(&tone(), &OpInput::rgb([&r, &g, &b], None), None).unwrap();
        for (plane, (expected, name)) in [&written.r, &written.g, &written.b].into_iter().zip(expected.planes.iter().zip(["r", "g", "b"])) {
            assert_bitwise(plane, expected, name);
        }
        assert!(written.r[[0, 0]].is_nan());
        assert_eq!(text(&written.header, history::HEADER_ABNODE).as_deref(), Some("n0001"));
    }

    #[test]
    fn run_op_cmd_resolves_parent_from_input_path_and_refuses_foreign_inputs() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(5, 1.0));
        let first = fx.run(&key, &key, arcsinh(30.0)).unwrap();
        let first_fits = path_of(&first, RES_FITS_PATH);

        let second = fx.run(&key, &first_fits, tone()).unwrap();
        assert_eq!(second["node"], "n0002");
        assert_eq!(second["parent"], "n0001");

        let respelled = if cfg!(windows) { first_fits.replace('\\', "/").to_uppercase() } else { first_fits.replace("/n0001", "//n0001") };
        let third = fx.run(&key, &respelled, tone()).unwrap();
        assert_eq!(third["parent"], "n0001", "a respelled node path was not recognised");

        let from_source = fx.run(&key, &key, arcsinh(10.0)).unwrap();
        assert_eq!(from_source["node"], "n0004");
        assert!(from_source["parent"].is_null());

        let doc = fx.doc(&key);
        let parents: Vec<Option<&str>> = doc.nodes.iter().map(|n| n.parent.as_deref()).collect();
        assert_eq!(parents, [None, Some("n0001"), Some("n0001"), None]);
        assert_eq!(doc.head.as_deref(), Some("n0004"));

        let output = fx.app.path().join("output");
        std::fs::create_dir_all(&output).unwrap();
        let foreign = output.join("m42_arcsinh.fits").to_string_lossy().to_string();
        write_fits_mono(&foreign, &sky(6, 1.0), None).unwrap();
        let err = fx.run(&key, &foreign, tone()).unwrap_err().to_string();
        assert!(err.contains("is not a node of this image's history"), "{err}");

        let other = fx.mono("m43.fits", &sky(7, 1.0));
        fx.run(&other, &other, arcsinh(30.0)).unwrap();
        let other_node = fx.image_dir(&other).join("n0001.fits").to_string_lossy().to_string();
        let err = fx.run(&key, &other_node, tone()).unwrap_err().to_string();
        assert!(err.contains("is not a node of this image's history"), "{err}");
        assert_eq!(fx.doc(&key).nodes.len(), 4, "a refused run was recorded");
    }

    fn files_named(dir: &Path, name: &str) -> Vec<PathBuf> {
        let Ok(listing) = std::fs::read_dir(dir) else { return Vec::new() };
        listing
            .flatten()
            .flat_map(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    files_named(&path, name)
                } else if entry.file_name() == name {
                    vec![path]
                } else {
                    Vec::new()
                }
            })
            .collect()
    }

    #[tokio::test]
    async fn legacy_commands_write_no_history() {
        use crate::cmd::processing::{apply_arcsinh_stretch_cmd, apply_tone_cmd, run_extract_background};
        use crate::core::imaging::background::BackgroundConfig;
        use crate::types::constants::{wizard_bg_key, RES_CACHE_KEY, RES_CORRECTED_FITS};

        let _wizard = crate::infra::cache::lock_wizard_entries();
        let fx = Fixture::new();
        let input = sky(8, 1.0);
        let key = fx.mono("m42.fits", &input);
        let out = fx.app.path().join("output").to_string_lossy().to_string();
        std::fs::create_dir_all(&out).unwrap();

        let toned = apply_tone_cmd(key.clone(), out.clone(), Some(ToneLevelsInput { black: 0.05, gamma: 1.0, white: 0.95 }), None).await.unwrap();
        assert_eq!(Path::new(&path_of(&toned, RES_FITS_PATH)), Path::new(&out).join("m42_tone.fits"));

        let stretched = apply_arcsinh_stretch_cmd(key.clone(), out.clone(), 30.0).await.unwrap();
        let legacy_fits = path_of(&stretched, RES_FITS_PATH);
        assert_eq!(Path::new(&legacy_fits), Path::new(&out).join("m42_arcsinh.fits"));

        let config = BackgroundConfig { grid_size: 4, poly_degree: 2, ..BackgroundConfig::default() };
        let wizard = run_extract_background(&key, &out, &config, "subtract", 3, Some("bin-ha"), None).unwrap();
        crate::infra::cache::GLOBAL_IMAGE_CACHE.remove(&wizard_bg_key("bin-ha"));
        assert_eq!(wizard[RES_CACHE_KEY], wizard_bg_key("bin-ha"));
        assert_eq!(wizard[RES_CORRECTED_FITS], wizard_bg_key("bin-ha"));
        assert!(!Path::new(&out).join("m42_bg_corrected.fits").exists(), "the wizard shape wrote a FITS");

        assert!(!fx.root().exists(), "a legacy command created the history root");
        assert!(files_named(fx.app.path(), history::SIDECAR_NAME).is_empty());
        assert!(files_named(fx.data.path(), history::SIDECAR_NAME).is_empty());

        let recorded = fx.run(&key, &key, arcsinh(30.0)).unwrap();
        let node_pixels = extract_image_resolved(&path_of(&recorded, RES_FITS_PATH)).unwrap().arr;
        assert_bitwise(&node_pixels, &extract_image_resolved(&legacy_fits).unwrap().arr, "legacy and recorded arcsinh");
    }

    fn node_listing(dir: &Path) -> Vec<(String, std::time::SystemTime)> {
        let mut listing: Vec<(String, std::time::SystemTime)> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name() != history::SIDECAR_NAME)
            .map(|e| (e.file_name().to_string_lossy().to_string(), e.metadata().unwrap().modified().unwrap()))
            .collect();
        listing.sort();
        listing
    }

    fn three_steps(fx: &Fixture, key: &str) -> Vec<String> {
        let first = fx.run(key, key, wavelet([3.0, 2.5, 2.0, 1.5, 1.0])).unwrap();
        let second = fx.run(key, &path_of(&first, RES_FITS_PATH), arcsinh(30.0)).unwrap();
        let third = fx.run(key, &path_of(&second, RES_FITS_PATH), tone()).unwrap();
        [first, second, third].iter().map(|v| path_of(v, RES_FITS_PATH)).collect()
    }

    fn head_of(value: &serde_json::Value) -> Option<String> {
        value["head"].as_str().map(str::to_string)
    }

    #[test]
    fn undo_and_redo_move_the_head_without_running_an_op() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(9, 1.0));
        let fits = three_steps(&fx, &key);
        let dir = fx.image_dir(&key);
        let before = node_listing(&dir);
        let root = fx.root();

        let undone = move_head(&root, &key, None, HeadMove::Undo).unwrap();
        assert_eq!(head_of(&undone).as_deref(), Some("n0002"));
        assert_eq!(Path::new(&path_of(&undone["node"], RES_FITS_PATH)), Path::new(&fits[1]));
        assert_eq!(undone["can_redo"], true);
        assert_eq!(head_of(&move_head(&root, &key, None, HeadMove::Undo).unwrap()).as_deref(), Some("n0001"));
        let original = move_head(&root, &key, None, HeadMove::Undo).unwrap();
        assert_eq!(head_of(&original), None);
        assert!(original["node"].is_null());
        assert_eq!(original["can_undo"], false);
        assert_eq!(original["root_path"], key.as_str());
        let err = move_head(&root, &key, None, HeadMove::Undo).unwrap_err().to_string();
        assert!(err.contains("nothing to undo"), "{err}");

        for expected in ["n0001", "n0002", "n0003"] {
            assert_eq!(head_of(&move_head(&root, &key, None, HeadMove::Redo).unwrap()).as_deref(), Some(expected));
        }
        let err = move_head(&root, &key, None, HeadMove::Redo).unwrap_err().to_string();
        assert!(err.contains("nothing to redo"), "{err}");

        let doc = fx.doc(&key);
        assert_eq!(doc.nodes.len(), 3);
        assert_eq!(doc.head.as_deref(), Some("n0003"));
        assert_eq!(node_listing(&dir), before, "undo or redo touched a node file");
    }

    #[test]
    fn redo_follows_the_most_recent_child_and_set_head_goes_anywhere() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(10, 1.0));
        let fits = three_steps(&fx, &key);
        fx.run(&key, &fits[0], arcsinh(12.0)).unwrap();
        let root = fx.root();

        assert_eq!(head_of(&move_head(&root, &key, None, HeadMove::To(Some("n0002"))).unwrap()).as_deref(), Some("n0002"));
        assert_eq!(head_of(&move_head(&root, &key, None, HeadMove::Undo).unwrap()).as_deref(), Some("n0001"));
        assert_eq!(head_of(&move_head(&root, &key, None, HeadMove::Redo).unwrap()).as_deref(), Some("n0004"), "redo did not take the newest branch");
        assert_eq!(head_of(&move_head(&root, &key, None, HeadMove::To(None)).unwrap()), None);
        let err = move_head(&root, &key, None, HeadMove::To(Some("n0042"))).unwrap_err().to_string();
        assert!(err.contains("n0042"), "{err}");
        assert_eq!(fx.doc(&key).nodes.len(), 4);

        let fresh = fx.mono("m43.fits", &sky(11, 1.0));
        let err = move_head(&root, &fresh, None, HeadMove::Undo).unwrap_err().to_string();
        assert!(err.contains("no processing history"), "{err}");
    }

    #[test]
    fn set_head_is_refused_while_a_run_holds_the_image() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(12, 1.0));
        fx.run(&key, &key, arcsinh(30.0)).unwrap();
        let root = fx.root();

        let guard = RunGuard::acquire(&fx.image_dir(&key)).unwrap();
        for attempt in [HeadMove::To(None), HeadMove::Undo, HeadMove::Redo] {
            let err = move_head(&root, &key, None, attempt).unwrap_err().to_string();
            assert_eq!(err, IMAGE_BUSY);
        }
        let err = fx.run(&key, &key, tone()).unwrap_err().to_string();
        assert_eq!(err, IMAGE_BUSY, "a second run raced the one in flight");
        assert_eq!(fx.doc(&key).head.as_deref(), Some("n0001"));

        drop(guard);
        assert_eq!(head_of(&move_head(&root, &key, None, HeadMove::To(None)).unwrap()), None);
        assert_eq!(fx.doc(&key).head, None);
        assert!(RunGuard::acquire(&fx.image_dir(&key)).is_ok(), "a finished move kept the image locked");
    }

    fn background() -> OpSpec {
        use crate::core::imaging::background::BackgroundConfig;
        let config = BackgroundConfig { grid_size: 4, poly_degree: 2, ..BackgroundConfig::default() };
        OpSpec::Background { v: OpVersion, model: BackgroundModel::Polynomial(config) }
    }

    fn wavelet(thresholds: [f32; 5]) -> OpSpec {
        use crate::core::imaging::wavelet::WaveletConfig;
        OpSpec::WaveletDenoise { v: OpVersion, config: WaveletConfig { thresholds: thresholds.to_vec(), ..WaveletConfig::default() } }
    }

    fn deconv() -> OpSpec {
        use crate::types::stacking::RLConfig;
        OpSpec::RlDeconv { v: OpVersion, config: RLConfig { iterations: 5, ..RLConfig::default() }, psf: PsfSpec::Gaussian { sigma: 1.5, size: 7 } }
    }

    fn chain(fx: &Fixture, key: &str, ops: Vec<OpSpec>) -> Vec<String> {
        let mut input = key.to_string();
        ops.into_iter()
            .map(|op| {
                input = path_of(&fx.run(key, &input, op).unwrap(), RES_FITS_PATH);
                input.clone()
            })
            .collect()
    }

    fn assert_within_range(actual: &Array2<f32>, expected: &Array2<f32>, what: &str) {
        assert_eq!(actual.dim(), expected.dim(), "{what}");
        let finite = || expected.iter().copied().filter(|v| v.is_finite());
        let range = finite().fold(f32::NEG_INFINITY, f32::max) - finite().fold(f32::INFINITY, f32::min);
        let worst = actual
            .iter()
            .zip(expected.iter())
            .map(|(a, e)| match (a.is_finite(), e.is_finite()) {
                (true, true) => (a - e).abs(),
                _ if a.to_bits() == e.to_bits() || (a.is_nan() && e.is_nan()) => 0.0,
                _ => f32::INFINITY,
            })
            .fold(0.0f32, f32::max);
        assert!(worst <= 1e-6 * range, "{what}: max |delta| {worst} exceeds 1e-6 of the range {range}");
    }

    fn evict(dir: &Path, ids: &[&str]) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if ids.iter().any(|id| name.starts_with(id)) {
                std::fs::remove_file(entry.path()).unwrap();
            }
        }
    }

    #[test]
    fn rehydrate_rebuilds_evicted_nodes_within_1e_6_of_range() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(13, 1.0));
        let fits = chain(&fx, &key, vec![background(), wavelet([3.0, 2.5, 2.0, 1.5, 1.0]), deconv(), arcsinh(30.0)]);
        let kept: Vec<Array2<f32>> = fits.iter().map(|f| extract_image_resolved(f).unwrap().arr).collect();
        let dir = fx.image_dir(&key);
        evict(&dir, &["n0002", "n0003", "n0004"]);
        assert!(!Path::new(&fits[3]).exists());

        let value = rehydrate(&fx.root(), &key, None, "n0004").unwrap();
        assert_eq!(value["rebuilt"], serde_json::json!(["n0002", "n0003", "n0004"]));
        assert_eq!(value["node"]["present"], true);

        let doc = fx.doc(&key);
        assert_eq!(doc.nodes.len(), 4, "rehydration created nodes");
        assert_eq!(doc.head.as_deref(), Some("n0004"));
        assert!(doc.nodes.iter().all(|n| n.present), "a rebuilt node is still marked missing");
        for (index, (path, expected)) in fits.iter().zip(&kept).enumerate().skip(1) {
            assert_within_range(&extract_image_resolved(path).unwrap().arr, expected, &format!("n000{}", index + 1));
            assert!(Path::new(&path.replace(".fits", ".png")).exists());
        }
        assert_eq!(text(&cached_header(&fits[2]).unwrap(), history::HEADER_ABNODE).as_deref(), Some("n0003"));

        let again = rehydrate(&fx.root(), &key, None, "n0004").unwrap();
        assert_eq!(again["rebuilt"], serde_json::json!([]), "a present node was rebuilt");
    }

    #[test]
    fn rerun_with_replay_downstream_creates_a_branch() {
        use crate::types::constants::HEADER_DISPLAY_REFERRED_CARD;

        let fx = Fixture::new();
        let input = sky(14, 1.0);
        let key = fx.mono("m42.fits", &input);
        chain(&fx, &key, vec![background(), wavelet([3.0, 2.5, 2.0, 1.5, 1.0]), arcsinh(30.0), tone()]);
        let dir = fx.image_dir(&key);
        evict(&dir, &["n0001"]);

        let stronger = wavelet([4.0, 3.0, 2.0, 1.5, 1.0]);
        let request = RerunRequest { image_key: &key, base_path: None, node: "n0002", op: Some(stronger.clone()), replay_downstream: true, psf_kernel: None };
        let value = rerun(&fx.root(), request, None).unwrap();
        assert_eq!(value["created"], serde_json::json!(["n0005", "n0006", "n0007"]));
        assert_eq!(head_of(&value).as_deref(), Some("n0007"));

        let doc = fx.doc(&key);
        let node = |id: &str| doc.node(id).unwrap().clone();
        assert_eq!(node("n0005").parent.as_deref(), Some("n0001"));
        assert_eq!(node("n0006").parent.as_deref(), Some("n0005"));
        assert_eq!(node("n0007").parent.as_deref(), Some("n0006"));
        let op_json = |id: &str| serde_json::to_value(&node(id).op).unwrap();
        assert_eq!(op_json("n0005"), serde_json::to_value(&stronger).unwrap());
        assert_eq!(op_json("n0006"), op_json("n0003"));
        assert_eq!(op_json("n0007"), op_json("n0004"));
        let old: Vec<&str> = doc.ancestry("n0004").iter().map(|n| n.id.as_str()).collect();
        assert_eq!(old, ["n0001", "n0002", "n0003", "n0004"], "the old branch is no longer reachable");
        assert!(doc.nodes.iter().all(|n| n.present), "the evicted parent was not rebuilt before the re-run");

        let ran = |op: &OpSpec, plane: &Array2<f32>, header: Option<&crate::types::header::HduHeader>| {
            run_op(op, &OpInput::mono(plane, header), None).unwrap().planes.remove(0)
        };
        let mut display = crate::types::header::HduHeader::empty();
        display.set(HEADER_DISPLAY_REFERRED_CARD, "T".to_string());
        let by_hand = ran(&background(), &input, None);
        let by_hand = ran(&stronger, &by_hand, None);
        let by_hand = ran(&arcsinh(30.0), &by_hand, None);
        let by_hand = ran(&tone(), &by_hand, Some(&display));
        let n0007 = extract_image_resolved(&dir.join("n0007.fits").to_string_lossy()).unwrap().arr;
        assert_within_range(&n0007, &by_hand, "n0007 against a by-hand run");

        let single = RerunRequest { image_key: &key, base_path: None, node: "n0003", op: None, replay_downstream: false, psf_kernel: None };
        let value = rerun(&fx.root(), single, None).unwrap();
        assert_eq!(value["created"], serde_json::json!(["n0008"]));
        assert_eq!(fx.doc(&key).node("n0008").unwrap().parent.as_deref(), Some("n0002"));

        let missing = RerunRequest { image_key: &key, base_path: None, node: "n0042", op: None, replay_downstream: true, psf_kernel: None };
        assert!(rerun(&fx.root(), missing, None).unwrap_err().to_string().contains("n0042"));
    }

    #[test]
    fn rerun_downstream_on_the_head_re_applies_the_undone_steps() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(17, 1.0));
        chain(&fx, &key, vec![background(), wavelet([3.0, 2.5, 2.0, 1.5, 1.0]), arcsinh(30.0), tone()]);
        let root = fx.root();
        move_head(&root, &key, None, HeadMove::Undo).unwrap();
        move_head(&root, &key, None, HeadMove::Undo).unwrap();
        assert_eq!(fx.doc(&key).head.as_deref(), Some("n0002"));

        let stronger = wavelet([4.0, 3.0, 2.0, 1.5, 1.0]);
        let request = RerunRequest { image_key: &key, base_path: None, node: "n0002", op: Some(stronger), replay_downstream: true, psf_kernel: None };
        let value = rerun(&root, request, None).unwrap();
        assert_eq!(value["created"], serde_json::json!(["n0005", "n0006", "n0007"]), "the steps after the head were not re-applied");
        assert_eq!(head_of(&value).as_deref(), Some("n0007"));

        let doc = fx.doc(&key);
        let parent = |id: &str| doc.node(id).unwrap().parent.clone();
        assert_eq!((parent("n0005"), parent("n0006"), parent("n0007")), (Some("n0001".into()), Some("n0005".into()), Some("n0006".into())));
        let op_json = |id: &str| serde_json::to_value(&doc.node(id).unwrap().op).unwrap();
        assert_eq!(op_json("n0006"), op_json("n0003"));
        assert_eq!(op_json("n0007"), op_json("n0004"));
    }

    #[test]
    fn rerun_reports_the_first_created_node_like_a_run() {
        use crate::types::constants::RES_NOISE_ESTIMATE;

        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(15, 1.0));
        chain(&fx, &key, vec![background(), wavelet([3.0, 2.5, 2.0, 1.5, 1.0]), arcsinh(30.0)]);
        let dir = fx.image_dir(&key);

        let stronger = wavelet([4.0, 3.0, 2.0, 1.5, 1.0]);
        let request = RerunRequest { image_key: &key, base_path: None, node: "n0002", op: Some(stronger), replay_downstream: true, psf_kernel: None };
        let value = rerun(&fx.root(), request, None).unwrap();
        assert_eq!(value["created"], serde_json::json!(["n0004", "n0005"]));
        let first = &value["run"];
        assert_eq!(first["node"], "n0004");
        assert_eq!(first["parent"], "n0001");
        assert_eq!(Path::new(&path_of(first, RES_FITS_PATH)), dir.join("n0004.fits"));
        assert_eq!(Path::new(&path_of(first, RES_PNG_PATH)), dir.join("n0004.png"));
        assert!(first[RES_NOISE_ESTIMATE].is_number(), "the op extras of the re-run step are missing: {first}");
        assert_eq!(head_of(&value).as_deref(), Some("n0005"));
    }

    #[test]
    fn delete_branch_removes_its_nodes_and_files_and_keeps_the_rest() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(16, 1.0));
        let fits = three_steps(&fx, &key);
        fx.run(&key, &fits[0], arcsinh(12.0)).unwrap();
        let dir = fx.image_dir(&key);
        let root = fx.root();

        let value = delete_branch(&root, &key, None, "n0002").unwrap();
        assert_eq!(value["removed"], serde_json::json!(["n0002", "n0003"]));
        assert_eq!(head_of(&value).as_deref(), Some("n0004"));
        assert_eq!(value["node_count"], 2);
        for gone in ["n0002.fits", "n0002.png", "n0003.fits", "n0003.png"] {
            assert!(!dir.join(gone).exists(), "{gone} was left on disk");
        }
        for kept in ["n0001.fits", "n0004.fits", "n0004.png"] {
            assert!(dir.join(kept).exists(), "{kept} was removed");
        }

        let value = delete_branch(&root, &key, None, "n0004").unwrap();
        assert_eq!(head_of(&value).as_deref(), Some("n0001"), "the head did not move out of the deleted branch");
        assert_eq!(fx.doc(&key).nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["n0001"]);

        let err = delete_branch(&root, &key, None, "n0042").unwrap_err().to_string();
        assert!(err.contains("n0042"), "{err}");

        let held = RunGuard::acquire(&dir).unwrap();
        let err = delete_branch(&root, &key, None, "n0001").unwrap_err().to_string();
        assert!(err.contains(IMAGE_BUSY), "{err}");
        drop(held);
        let value = delete_branch(&root, &key, None, "n0001").unwrap();
        assert_eq!(head_of(&value), None);
        assert_eq!(value["node_count"], 0);
        assert!(!dir.join("n0001.fits").exists());
        assert!(history::sidecar_path(&dir).exists(), "deleting every node removed the sidecar");
    }

    #[test]
    fn a_supplied_psf_kernel_is_stored_once_by_hash_and_read_back_on_re_run() {
        use crate::core::history::kernels::{kernel_hash, kernel_path};
        use crate::types::stacking::RLConfig;

        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(15, 1.0));
        let rows = vec![vec![1.0f32, 2.0, 1.0], vec![2.0, 4.0, 2.0], vec![1.0, 2.0, 1.0]];
        let raw = Array2::from_shape_fn((3, 3), |(y, x)| rows[y][x]);
        let op = OpSpec::RlDeconv {
            v: OpVersion,
            config: RLConfig { iterations: 4, ..RLConfig::default() },
            psf: PsfSpec::Kernel { hash: String::new(), rows: 0, cols: 0 },
        };
        let request = RunRequest { image_key: &key, base_path: None, input_path: &key, op, psf_kernel: Some(rows.clone()) };
        let value = run_recorded(&fx.root(), request, None).unwrap();
        assert_eq!(value["psf_source"], "provided");

        let doc = fx.doc(&key);
        let OpSpec::RlDeconv { psf: PsfSpec::Kernel { hash, rows: 3, cols: 3 }, .. } = &doc.nodes[0].op else {
            panic!("{:?}", doc.nodes[0].op)
        };
        assert_eq!(*hash, kernel_hash(&raw));
        assert!(kernel_path(&fx.image_dir(&key), hash).unwrap().exists());

        let kernel = kernel_rows(&fx.root(), &key, None, hash, 3, 3).unwrap();
        assert_eq!(kernel["kernel"], serde_json::json!(rows));

        let request = RerunRequest { image_key: &key, base_path: None, node: "n0001", op: None, replay_downstream: false, psf_kernel: None };
        rerun(&fx.root(), request, None).unwrap();
        let fits = |id: &str| extract_image_resolved(&fx.image_dir(&key).join(format!("{id}.fits")).to_string_lossy()).unwrap().arr;
        assert_bitwise(&fits("n0002"), &fits("n0001"), "a re-run from the stored kernel");
    }

    #[test]
    fn history_state_reports_presence_read_only_and_fingerprint_without_writing() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(16, 1.0));
        let root = fx.root();
        let empty = history_state(&root, &key, None).unwrap();
        assert!(empty["doc"].is_null());
        assert_eq!(empty["image_id"], image_id(&key, None));

        fx.run(&key, &key, arcsinh(30.0)).unwrap();
        let dir = fx.image_dir(&key);
        std::fs::remove_file(dir.join("n0001.png")).unwrap();
        let sidecar = std::fs::read(history::sidecar_path(&dir)).unwrap();
        let state = history_state(&root, &key, None).unwrap();
        assert_eq!(state["doc"]["nodes"][0]["present"], false);
        assert_eq!(state["fingerprint_matches"], true);
        assert_eq!(state["busy"], false);
        assert!(state["read_only"].is_null());
        assert_eq!(std::fs::read(history::sidecar_path(&dir)).unwrap(), sidecar, "reading the history rewrote it");

        let guard = RunGuard::acquire(&fx.image_dir(&key)).unwrap();
        assert_eq!(history_state(&root, &key, None).unwrap()["busy"], true);
        drop(guard);

        write_fits_mono(&key, &sky(17, 2.0), None).unwrap();
        assert_eq!(history_state(&root, &key, None).unwrap()["fingerprint_matches"], false);

        let bumped = String::from_utf8(sidecar).unwrap().replace("\"version\":1", "\"version\":2");
        std::fs::write(history::sidecar_path(&dir), &bumped).unwrap();
        let read_only = history_state(&root, &key, None).unwrap();
        assert!(read_only["read_only"].as_str().unwrap().contains("read-only"), "{read_only}");
        assert_eq!(read_only["doc"]["version"], 2);
        assert!(read_only["fingerprint_matches"].is_null(), "a newer history was reported as a changed source: {read_only}");
        let err = fx.run(&key, &key, tone()).unwrap_err().to_string();
        assert!(err.contains("read-only"), "{err}");
    }

    #[test]
    fn estimate_reports_the_next_node_size_and_the_history_total() {
        let fx = Fixture::new();
        let mono = fx.mono("m42.fits", &sky(18, 1.0));
        let (r, g, b) = (sky(19, 1.0), sky(20, 1.0), sky(21, 1.0));
        let colour = fx.rgb("colour.fits", [&r, &g, &b]);
        let root = fx.root();

        let before = estimate(&root, &mono).unwrap();
        assert_eq!(before["next_node_bytes"], 64 * 48 * 4);
        assert_eq!(before[RES_DIMENSIONS], serde_json::json!([64, 48]));
        assert_eq!(before["history_bytes"], 0);
        assert_eq!(estimate(&root, &colour).unwrap()["next_node_bytes"], 64 * 48 * 4 * 3);

        fx.run(&mono, &mono, arcsinh(30.0)).unwrap();
        assert!(estimate(&root, &mono).unwrap()["history_bytes"].as_u64().unwrap() > 64 * 48 * 4);
    }

    fn pixelmath(expression: &str, slots: &[(&str, &str)]) -> OpSpec {
        use crate::core::ops::pixelmath::PixelMathSlot;
        let slots = slots.iter().map(|(name, path)| PixelMathSlot { name: name.to_string(), path: path.to_string() }).collect();
        OpSpec::Pixelmath { v: OpVersion, expression: expression.to_string(), slots, truncate: false, rescale: false }
    }

    #[test]
    fn copy_to_output_writes_the_step_under_output_and_never_over_a_file_it_read() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(18, 1.0));
        let out = fx.app.path().join("output");
        let out_dir = out.to_string_lossy().to_string();
        let first = fx.run(&key, &key, pixelmath("$T * 2", &[])).unwrap();

        let copied = copy_to_output(&fx.root(), &key, None, "n0001", &out_dir).unwrap();
        let copy = path_of(&copied, RES_FITS_PATH);
        assert_eq!(Path::new(&copy), out.join("m42_pixelmath.fits"));
        let node_pixels = extract_image_resolved(&path_of(&first, RES_FITS_PATH)).unwrap().arr;
        assert_bitwise(&extract_image_resolved(&copy).unwrap().arr, &node_pixels, "the copy of n0001");
        assert!(fx.image_dir(&key).join("n0001.fits").exists(), "the copy moved the step out of the history");

        let second = fx.run(&key, &key, pixelmath("$T + B", &[("B", &copy)])).unwrap();
        let node = second["node"].as_str().unwrap();
        let again = copy_to_output(&fx.root(), &key, None, node, &out_dir).unwrap();
        assert_eq!(Path::new(&path_of(&again, RES_FITS_PATH)), out.join("m42_n0002_pixelmath.fits"), "the copy overwrote a slot of the step");
        assert_bitwise(&extract_image_resolved(&copy).unwrap().arr, &node_pixels, "the slot file the step read");

        let err = copy_to_output(&fx.root(), &key, None, "n0042", &out_dir).unwrap_err().to_string();
        assert!(err.contains("n0042"), "{err}");
    }

    fn present_ids(doc: &history::HistoryDoc) -> Vec<&str> {
        doc.nodes.iter().filter(|n| n.present).map(|n| n.id.as_str()).collect()
    }

    async fn sweep_to_zero(fx: &Fixture) -> serde_json::Value {
        crate::cmd::output::cleanup_history_cmd(fx.root().to_string_lossy().to_string(), Some(0)).await.unwrap()
    }

    #[tokio::test]
    async fn a_pixelmath_node_that_reads_a_file_outside_the_history_is_never_evicted() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(22, 1.0));
        let flat = fx.mono("flat.fits", &sky(23, 0.5));
        let stretched = path_of(&fx.run(&key, &key, arcsinh(30.0)).unwrap(), RES_FITS_PATH);
        let divided = path_of(&fx.run(&key, &stretched, pixelmath("$T / a", &[("a", &flat)])).unwrap(), RES_FITS_PATH);
        let from_source = path_of(&fx.run(&key, &key, pixelmath("$T * a", &[("a", &key)])).unwrap(), RES_FITS_PATH);
        fx.run(&key, &key, arcsinh(10.0)).unwrap();
        std::fs::remove_file(&flat).unwrap();

        let res = sweep_to_zero(&fx).await;

        let doc = fx.doc(&key);
        assert_eq!(present_ids(&doc), ["n0002", "n0004"], "{res}");
        assert!(Path::new(&divided).exists(), "a step that read a file outside the history was evicted: {res}");
        assert!(!Path::new(&from_source).exists(), "a step that only read the source file was kept: {res}");
        assert_eq!(res[RES_REPLAYABLE_NODES], 2, "{res}");
    }

    #[tokio::test]
    async fn the_first_steps_of_a_rebased_history_survive_the_sweep_and_the_launch_wipe() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(24, 1.0));
        let output = fx.app.path().join("output");
        std::fs::create_dir_all(&output).unwrap();
        let base = output.join("m42_stack.fits").to_string_lossy().to_string();
        write_fits_mono(&base, &sky(25, 1.0), None).unwrap();
        let run = |input: &str, op: OpSpec| {
            run_recorded(&fx.root(), RunRequest { image_key: &key, base_path: Some(&base), input_path: input, op, psf_kernel: None }, None)
        };
        let first = path_of(&run(&base, wavelet([3.0, 2.5, 2.0, 1.5, 1.0])).unwrap(), RES_FITS_PATH);
        let second = path_of(&run(&first, arcsinh(30.0)).unwrap(), RES_FITS_PATH);
        let kept = extract_image_resolved(&second).unwrap().arr;
        run(&base, wavelet([4.0, 3.0, 2.0, 1.5, 1.0])).unwrap();

        let res = sweep_to_zero(&fx).await;
        let dir = history::image_dir(&fx.root(), &image_id(&key, Some(&base)));
        assert_eq!(present_ids(&history::load(&dir).unwrap().unwrap()), ["n0001", "n0003"], "{res}");

        crate::cmd::output::clear_output_dir(fx.app.path());
        assert!(!Path::new(&base).exists());
        let value = rehydrate(&fx.root(), &key, Some(&base), "n0002").unwrap();
        assert_eq!(value["rebuilt"], serde_json::json!(["n0002"]));
        assert_within_range(&extract_image_resolved(&second).unwrap().arr, &kept, "n0002 rebuilt after the launch wipe");
    }

    #[test]
    fn replay_from_a_rewritten_source_is_refused() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(26, 1.0));
        let fits = chain(&fx, &key, vec![wavelet([3.0, 2.5, 2.0, 1.5, 1.0]), arcsinh(30.0)]);
        let dir = fx.image_dir(&key);
        evict(&dir, &["n0001", "n0002"]);
        write_fits_mono(&key, &sky(27, 1.0), None).unwrap();

        let err = format!("{:#}", rehydrate(&fx.root(), &key, None, "n0002").unwrap_err());
        assert!(err.contains("changed since it was recorded"), "{err}");
        assert!(!dir.join("n0001.fits").exists(), "a node was rebuilt from the rewritten source");

        let err = format!("{:#}", fx.run(&key, &fits[1], tone()).unwrap_err());
        assert!(err.contains("changed since it was recorded"), "{err}");
        assert_eq!(fx.doc(&key).nodes.len(), 2, "a step was recorded on top of a replay from the rewritten source");
    }

    #[test]
    fn a_step_on_the_rewritten_original_restamps_the_history_and_keeps_the_old_steps_pinned() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(29, 1.0));
        chain(&fx, &key, vec![wavelet([3.0, 2.5, 2.0, 1.5, 1.0]), arcsinh(30.0)]);
        let root = fx.root();
        let fresh = sky(30, 1.0);
        write_fits_mono(&key, &fresh, None).unwrap();
        assert_eq!(history_state(&root, &key, None).unwrap()["fingerprint_matches"], false);

        let value = fx.run(&key, &key, arcsinh(12.0)).unwrap();
        assert_eq!(value["node"], "n0003");
        assert!(value["parent"].is_null());
        assert_eq!(history_state(&root, &key, None).unwrap()["fingerprint_matches"], true, "the history still reads as recorded from the old file");

        let doc = fx.doc(&key);
        let replayable: Vec<(&str, bool)> = doc.nodes.iter().map(|n| (n.id.as_str(), n.replayable)).collect();
        assert_eq!(replayable, [("n0001", false), ("n0002", false), ("n0003", true)], "a step of the old file could be replayed from the new one");
        for level in [history::PinLevel::Full, history::PinLevel::Floor] {
            let pinned = history::pinned_paths(&doc, level);
            assert!(pinned.contains(&"n0001.fits".to_string()) && pinned.contains(&"n0002.fits".to_string()), "{level:?} released a step of the old file: {pinned:?}");
        }

        let dir = fx.image_dir(&key);
        let kept = extract_image_resolved(&dir.join("n0003.fits").to_string_lossy()).unwrap().arr;
        evict(&dir, &["n0003"]);
        assert_eq!(rehydrate(&root, &key, None, "n0003").unwrap()["rebuilt"], serde_json::json!(["n0003"]));
        assert_within_range(&extract_image_resolved(&dir.join("n0003.fits").to_string_lossy()).unwrap().arr, &kept, "n0003 replayed from the new file");
    }

    #[test]
    fn a_rerun_that_fails_downstream_restores_the_head() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(28, 1.0));
        let flat = fx.mono("flat.fits", &sky(29, 0.5));
        let fits = chain(&fx, &key, vec![wavelet([3.0, 2.5, 2.0, 1.5, 1.0]), arcsinh(30.0)]);
        fx.run(&key, &fits[1], pixelmath("$T * a", &[("a", &flat)])).unwrap();
        std::fs::remove_file(&flat).unwrap();

        let request = RerunRequest {
            image_key: &key,
            base_path: None,
            node: "n0001",
            op: Some(wavelet([4.0, 3.0, 2.0, 1.5, 1.0])),
            replay_downstream: true,
            psf_kernel: None,
        };
        let err = format!("{:#}", rerun(&fx.root(), request, None).unwrap_err());
        assert!(err.contains("flat.fits"), "{err}");

        let doc = fx.doc(&key);
        assert_eq!(doc.head.as_deref(), Some("n0003"), "a failed re-run left the head on its partial branch");
        assert_eq!(doc.node("n0005").and_then(|n| n.parent.as_deref()), Some("n0004"));
        assert!(doc.node("n0006").is_none());
    }

    #[test]
    fn a_failed_replay_leaves_no_partial_node_and_reports_it_missing() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(30, 1.0));
        chain(&fx, &key, vec![wavelet([3.0, 2.5, 2.0, 1.5, 1.0]), arcsinh(30.0)]);
        let dir = fx.image_dir(&key);
        evict(&dir, &["n0002"]);
        std::fs::create_dir(dir.join("n0002.png")).unwrap();

        assert!(rehydrate(&fx.root(), &key, None, "n0002").is_err());
        assert!(!dir.join("n0002.fits").exists(), "a failed replay left a FITS behind");
        let state = history_state(&fx.root(), &key, None).unwrap();
        assert_eq!(state["doc"]["nodes"][1]["present"], false);
        let moved = move_head(&fx.root(), &key, None, HeadMove::To(Some("n0002"))).unwrap();
        assert_eq!(moved["node"]["present"], false);
    }

    #[test]
    fn replaying_a_node_only_checks_the_files_of_its_own_ancestry() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(31, 1.0));
        chain(&fx, &key, vec![wavelet([3.0, 2.5, 2.0, 1.5, 1.0]), arcsinh(30.0)]);
        fx.run(&key, &key, wavelet([4.0, 3.0, 2.0, 1.5, 1.0])).unwrap();
        let dir = fx.image_dir(&key);
        evict(&dir, &["n0002", "n0003"]);

        assert_eq!(rehydrate(&fx.root(), &key, None, "n0002").unwrap()["rebuilt"], serde_json::json!(["n0002"]));
        assert!(fx.doc(&key).node("n0003").unwrap().present, "replaying n0002 checked the files of another branch");
    }

    #[test]
    fn the_progress_of_a_rerun_follows_the_stored_step_when_no_op_is_given() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(34, 1.0));
        chain(&fx, &key, vec![wavelet([3.0, 2.5, 2.0, 1.5, 1.0]), arcsinh(30.0)]);
        let root = fx.root();
        let shown = |node: &str, requested: Option<&OpSpec>| rerun_progress_op(&root, &key, None, node, requested).map(|op| op.name());

        assert_eq!(shown("n0001", None), Some("wavelet_denoise"), "a re-run of the stored step would run without its progress events");
        assert_eq!(shown("n0001", Some(&wavelet([4.0, 3.0, 2.0, 1.5, 1.0]))), Some("wavelet_denoise"));
        assert_eq!(shown("n0042", None), None);
        assert_eq!(rerun_progress_op(&root, &fx.mono("m43.fits", &sky(35, 1.0)), None, "n0001", None).map(|op| op.name()), None);
    }

    #[test]
    fn a_rerun_keeps_the_kind_of_step_it_re_runs() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(32, 1.0));
        chain(&fx, &key, vec![wavelet([3.0, 2.5, 2.0, 1.5, 1.0]), arcsinh(30.0)]);

        let request = RerunRequest { image_key: &key, base_path: None, node: "n0001", op: Some(arcsinh(5.0)), replay_downstream: true, psf_kernel: None };
        let err = rerun(&fx.root(), request, None).unwrap_err().to_string();
        assert!(err.contains("wavelet_denoise") && err.contains("arcsinh"), "{err}");
        assert_eq!(fx.doc(&key).nodes.len(), 2, "a re-run of another kind was recorded");
    }

    #[test]
    fn a_step_earlier_than_its_branch_is_refused_and_pixelmath_takes_any_input() {
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(33, 1.0));
        let fits = chain(&fx, &key, vec![arcsinh(30.0), tone()]);

        for (input, op) in [(&fits[1], wavelet([3.0, 2.5, 2.0, 1.5, 1.0])), (&fits[0], arcsinh(10.0)), (&fits[1], tone())] {
            let err = fx.run(&key, input, op).unwrap_err().to_string();
            assert!(err.contains("cannot follow"), "{err}");
        }
        assert_eq!(fx.doc(&key).nodes.len(), 2, "an out-of-order step was recorded");

        let combined = path_of(&fx.run(&key, &fits[1], pixelmath("$T * 0.5", &[])).unwrap(), RES_FITS_PATH);
        let err = fx.run(&key, &combined, tone()).unwrap_err().to_string();
        assert!(err.contains("cannot follow"), "{err}");
        fx.run(&key, &combined, pixelmath("$T + 0.1", &[])).unwrap();
        let contrast = OpSpec::Lhe { v: OpVersion, config: crate::core::imaging::local_contrast::LheConfig { kernel_radius: 8, ..Default::default() } };
        fx.run(&key, &fits[0], contrast).unwrap();
        assert_eq!(fx.doc(&key).nodes.len(), 5);
    }

    #[test]
    #[ignore = "timing: checked locally in A10, three CI runners make the bound flaky"]
    fn recorded_run_overhead_on_a_200_node_history() {
        use crate::core::history::fixtures::write_bytes;
        let fx = Fixture::new();
        let key = fx.mono("m42.fits", &sky(35, 1.0));
        let first = path_of(&fx.run(&key, &key, wavelet([3.0, 2.5, 2.0, 1.5, 1.0])).unwrap(), RES_FITS_PATH);
        let dir = fx.image_dir(&key);
        let mut doc = fx.doc(&key);
        let template = doc.nodes[0].clone();
        for number in 2..=199u32 {
            let id = format!("n{number:04}");
            let output = history::NodeOutput::named(&id, [64, 48], 1, OutputValues::Linear, &[]);
            for file in output.files() {
                write_bytes(&dir.join(file), 64, 1);
            }
            let parent = if number % 7 == 2 { None } else { Some(format!("n{:04}", number - 1)) };
            doc.nodes.push(history::Node { id, parent, output, ..template.clone() });
        }
        history::save(&dir, &doc).unwrap();

        let started = Instant::now();
        history::mark_missing(&dir, &mut doc.clone());
        let every_node = started.elapsed();
        let started = Instant::now();
        rebuild(&Scope::new(&fx.root(), &key, None), &mut doc, "n0001").unwrap();
        let presence = started.elapsed();
        let started = Instant::now();
        fx.run(&key, &first, arcsinh(30.0)).unwrap();
        let recorded = started.elapsed();
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        println!(
            "recorder: 200-node history, presence of every node {:.2} ms, presence and replay plan on the path {:.2} ms, whole recorded 48x64 arcsinh run {:.2} ms",
            ms(every_node),
            ms(presence),
            ms(recorded)
        );
        assert!(presence < std::time::Duration::from_millis(15), "the presence check took {presence:?}");
    }

    #[test]
    fn legacy_command_sources_never_reach_the_history() {
        const SOURCES: [(&str, &str); 10] = [
            ("processing/tone.rs", include_str!("processing/tone.rs")),
            ("processing/curves.rs", include_str!("processing/curves.rs")),
            ("processing/stretch.rs", include_str!("processing/stretch.rs")),
            ("processing/wavelet.rs", include_str!("processing/wavelet.rs")),
            ("processing/deconvolution.rs", include_str!("processing/deconvolution.rs")),
            ("processing/local_contrast.rs", include_str!("processing/local_contrast.rs")),
            ("processing/hdr.rs", include_str!("processing/hdr.rs")),
            ("processing/background.rs", include_str!("processing/background.rs")),
            ("processing/dbe.rs", include_str!("processing/dbe.rs")),
            ("pixelmath.rs", include_str!("pixelmath.rs")),
        ];
        const ALLOWED_HISTORY_USE: &str = "core::history::kernels::kernel_hash";
        for (name, source) in SOURCES {
            let body = source.find("\nmod tests {").map_or(source, |at| &source[..at]);
            for forbidden in ["cmd::ops", "run_recorded", "RunRequest", "app_data_dir", "Manager", "SIDECAR_NAME"] {
                assert!(!body.contains(forbidden), "{name} references {forbidden}, which reaches the recorded history");
            }
            for (at, _) in body.match_indices("core::history") {
                assert!(body[at..].starts_with(ALLOWED_HISTORY_USE), "{name} reaches core::history beyond {ALLOWED_HISTORY_USE}");
            }
            for (at, _) in body.match_indices("cmd::{") {
                let group = &body[at + "cmd::{".len()..];
                let mut depth = 1usize;
                let end = group
                    .char_indices()
                    .find_map(|(i, c)| {
                        match c {
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            _ => {}
                        }
                        (depth == 0).then_some(i)
                    })
                    .unwrap_or(group.len());
                let names: Vec<&str> = group[..end].split(|c: char| !c.is_alphanumeric() && c != '_').collect();
                assert!(!names.contains(&"ops"), "{name} imports cmd::ops through a use group");
            }
        }
    }
}
