import type {
  MosaicEstimate,
  MosaicLevelModel,
  MosaicRejectRequest,
  MosaicRequest,
  MosaicResult,
  MosaicSolveRequest,
  MosaicTileRow,
  MosaicTileSolve,
  MosaicWcsKind,
} from "../shared/types/stacking";
import { detectChannel, type ChannelSource } from "./channelMapping";
import { formatCount } from "./formatCount";
import type { SessionMosaic } from "./mosaicStore";
import { laneForBin, MONO_LANE, PIPELINE_LANES, type ComposeMaster, type PipelineLane } from "./pipelineToCompose";
import { sipOrderCap, type SolveOptions } from "./plateSolveOptions";
import { solveTierLabel } from "./plateSolveResult";
import { samePath } from "./processingChain";
import { frameStem, toDims, type StackViewOutput } from "./stackingOutputs";

export const MIN_MOSAIC_PANELS = 2;
export const MOSAIC_RESIDUAL_WARNING_PX = 0.5;
export const NEW_CANVAS = "new";
export const CHOOSE_CANVAS = "choose";

export const MOSAIC_INTRO =
  "Joins overlapping panels into one image on a shared canvas, placed by their WCS. Use stacked panels, not raw subs: stack each pointing first, and run Background extraction on each panel before the mosaic, as the mosaic matches one level per panel and cannot remove a gradient inside a panel.";
export const MOSAIC_SOLVE_NOTE =
  "A panel without a WCS is plate solved on this computer from the pointing and pixel scale in its own header (Gaia DR3 online only when the Plate Solve options allow it, never astrometry.net); its file is left unchanged.";
export const LIKE_CANVAS_NOTE =
  "The mosaic takes the projection and pixel grid of that FITS, so mosaics of different filters line up for Compose.";

export type MosaicRejectMode = "auto" | "off" | "on";
export type MosaicBusy = "estimate" | "run" | null;

export interface MosaicSettings {
  canvasOf: string | null;
  northUp: boolean;
  axesOf: string | null;
  levels: MosaicLevelModel;
  references: readonly (number | null)[];
  featherPx: number;
  dqMask: boolean;
  reject: MosaicRejectMode;
  snr: number;
  scale: number;
}

export const DEFAULT_MOSAIC_SETTINGS: MosaicSettings = {
  canvasOf: null,
  northUp: false,
  axesOf: null,
  levels: "offset",
  references: [],
  featherPx: 64,
  dqMask: true,
  reject: "auto",
  snr: 3.5,
  scale: 1.2,
};

export const MOSAIC_LEVEL_OPTIONS: readonly { value: MosaicLevelModel; label: string }[] = [
  { value: "offset", label: "Offset" },
  { value: "offset_scale", label: "Offset + scale" },
];

export const MOSAIC_REJECT_OPTIONS: readonly { value: MosaicRejectMode; label: string }[] = [
  { value: "auto", label: "Auto (from the panel kind)" },
  { value: "off", label: "Off" },
  { value: "on", label: "On" },
];

const GIB = 2 ** 30;
const MIB = 2 ** 20;
const NO_VALUE = "--";
const STAGE_FIELDS = 4;
const POSITION = /^\d+\/\d+$/;
const PICK_PANELS = "Pick at least two panels.";
const WCS_KIND_NAMES: Record<MosaicWcsKind, string> = { header: "Header WCS", gwcs: "gWCS" };
const SOLVER_NAMES: Readonly<Record<string, string>> = { local: "Offline solve" };

function finite(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function baseName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

function pad(value: number, width = 2): string {
  return String(value).padStart(width, "0");
}

function runTimestamp(at: Date): string {
  const date = `${at.getFullYear()}${pad(at.getMonth() + 1)}${pad(at.getDate())}`;
  return `${date}-${pad(at.getHours())}${pad(at.getMinutes())}${pad(at.getSeconds())}-${pad(at.getMilliseconds(), 3)}`;
}

function plural(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

function tileName(tiles: readonly Pick<MosaicTileRow, "name">[], index: number): string {
  return tiles[index]?.name ?? `panel ${index + 1}`;
}

export function mosaicSolveRequest(options: Pick<SolveOptions, "onlineGaia" | "useTwomass" | "maxSipOrder">): MosaicSolveRequest {
  return { online: options.onlineGaia, use_twomass: options.useTwomass, max_sip_order: sipOrderCap(options) ?? null };
}

function rejectRequest(settings: MosaicSettings): MosaicRejectRequest {
  switch (settings.reject) {
    case "on":
      return { mode: "on", snr: settings.snr, scale: settings.scale };
    case "off":
      return { mode: "off" };
    case "auto":
      return { mode: "auto" };
  }
}

export function mosaicRequest(paths: readonly string[], settings: MosaicSettings, solve: MosaicSolveRequest): MosaicRequest {
  const axesOf = settings.axesOf;
  const axes = axesOf === null ? -1 : paths.findIndex((path) => samePath(path, axesOf));
  return {
    canvas:
      settings.canvasOf !== null
        ? { mode: "like", path: settings.canvasOf }
        : { mode: "auto", north_up: settings.northUp, reference: settings.northUp || axes < 0 ? null : axes },
    levels: settings.levels,
    references: [...settings.references],
    feather_px: settings.featherPx,
    dq_mask: settings.dqMask,
    reject: rejectRequest(settings),
    solve,
  };
}

export function withReference(references: readonly (number | null)[], component: number, tile: number): (number | null)[] {
  const next = Array.from({ length: Math.max(references.length, component + 1) }, (_, k) => references[k] ?? null);
  next[component] = tile;
  return next;
}

export function mosaicRequestKey(paths: readonly string[], request: MosaicRequest): string {
  return JSON.stringify([paths, request.canvas, request.levels, request.references, request.feather_px, request.dq_mask, request.reject]);
}

export interface MosaicEstimateRecord {
  key: string;
  paths: readonly string[];
  request: MosaicRequest;
  result: MosaicEstimate;
}

export interface MosaicGate {
  panels: number;
  busy: MosaicBusy;
  estimate: MosaicEstimateRecord | null;
  key: string;
}

export function estimateDisabledReason(gate: MosaicGate): string | null {
  if (gate.busy === "run") return "Wait for the mosaic run to finish.";
  if (gate.busy === "estimate") return null;
  return gate.panels < MIN_MOSAIC_PANELS ? PICK_PANELS : null;
}

export function runDisabledReason(gate: MosaicGate): string | null {
  if (gate.busy === "run") return null;
  if (gate.busy === "estimate") return "Wait for the estimate to finish.";
  if (gate.panels < MIN_MOSAIC_PANELS) return PICK_PANELS;
  if (!gate.estimate) return "Estimate first: Run unlocks once the estimate has checked the canvas, the memory and the panel groups.";
  if (gate.estimate.key !== gate.key) return "The panels or settings changed since the estimate: Estimate again.";
  if (gate.estimate.result.refusal) return "The estimate refused this mosaic: change what it names, then Estimate again.";
  return null;
}

export function memoryText(bytes: number): string {
  return bytes >= GIB ? `${(bytes / GIB).toFixed(1)} GB` : `${(bytes / MIB).toFixed(1)} MB`;
}

export function canvasSummary(estimate: Pick<MosaicEstimate, "cols" | "rows" | "megapixels" | "ram_bytes" | "planes">): string | null {
  if (estimate.cols <= 0 || estimate.rows <= 0) return null;
  const rgb = estimate.planes === 3 ? " · RGB" : "";
  return `Canvas ${formatCount(estimate.cols)} × ${formatCount(estimate.rows)} px · ${estimate.megapixels.toFixed(1)} MP · about ${memoryText(estimate.ram_bytes)} of memory${rgb}`;
}

export interface MosaicGroupView {
  component: number;
  number: number;
  panels: { index: number; name: string }[];
  reference: number;
}

export function groupViews(estimate: Pick<MosaicEstimate, "components" | "tiles">): MosaicGroupView[] {
  const groups: MosaicGroupView[] = [];
  estimate.components.forEach((component, k) => {
    if (component.tiles.length < 2) return;
    groups.push({
      component: k,
      number: groups.length + 1,
      panels: component.tiles.map((index) => ({ index, name: tileName(estimate.tiles, index) })),
      reference: component.reference,
    });
  });
  return groups;
}

export function estimateNotes(estimate: Pick<MosaicEstimate, "components" | "tiles" | "isolated">): string[] {
  const groups = groupViews(estimate).length;
  const notes = groups > 1 ? [`The panels form ${groups} groups that do not overlap: levels are matched within each group, not between groups.`] : [];
  for (const index of estimate.isolated) {
    notes.push(`Panel '${tileName(estimate.tiles, index)}' overlaps no other panel: it keeps its own level, not matched to the others.`);
  }
  return notes;
}

export interface MosaicTileView {
  index: number;
  name: string;
  group: string | null;
  reference: boolean;
  wcs: string;
  scale: string;
  rotation: string;
  coverage: string;
  solve: string | null;
  singleExposure: boolean;
}

function rotationText(degrees: number | null | undefined): string {
  if (!finite(degrees)) return NO_VALUE;
  const text = degrees.toFixed(1);
  return `${text === "-0.0" ? "0.0" : text}°`;
}

export function solveLine(solve: MosaicTileSolve): string {
  const parts = [SOLVER_NAMES[solve.solver] ?? solve.solver, solveTierLabel(solve.catalog, solve.tier)];
  if (finite(solve.matched)) parts.push(plural(solve.matched, "star"));
  if (finite(solve.rms_px)) parts.push(`RMS ${solve.rms_px.toFixed(2)} px`);
  return parts.filter((part): part is string => !!part).join(" · ");
}

export function tileViews(estimate: Pick<MosaicEstimate, "tiles" | "components" | "isolated">): MosaicTileView[] {
  const groups = groupViews(estimate);
  const isolated = new Set(estimate.isolated);
  return estimate.tiles.map((tile, index) => {
    const group = groups.find((g) => g.panels.some((panel) => panel.index === index));
    return {
      index,
      name: tile.name,
      group: group ? `Group ${group.number}` : isolated.has(index) ? "Isolated" : null,
      reference: group?.reference === index,
      wcs: tile.solve ? "Solved from its header" : tile.wcs_kind ? WCS_KIND_NAMES[tile.wcs_kind] : NO_VALUE,
      scale: finite(tile.scale_ratio) ? `${tile.scale_ratio.toFixed(2)}×` : NO_VALUE,
      rotation: rotationText(tile.rotation_deg),
      coverage: finite(tile.coverage_frac) ? `${(tile.coverage_frac * 100).toFixed(1)} % of the canvas` : NO_VALUE,
      solve: tile.solve ? solveLine(tile.solve) : null,
      singleExposure: tile.single_exposure,
    };
  });
}

export function rejectionLine(mode: MosaicRejectMode, estimate: Pick<MosaicEstimate, "reject" | "tiles">): string {
  const spec = estimate.reject;
  const params = spec ? ` (SNR ${spec.snr}, scale ${spec.scale})` : "";
  switch (mode) {
    case "off":
      return "Outlier rejection is off.";
    case "on":
      return `Outlier rejection is on${params} where three or more panels overlap; where only two overlap nothing is rejected.`;
    case "auto":
      if (spec) return `Auto: outlier rejection is on${params}, as every panel is a single exposure and three or more panels overlap.`;
      if (estimate.tiles.length > 0 && estimate.tiles.every((tile) => tile.single_exposure)) return "Auto: outlier rejection is off, as no three panels overlap.";
      return "Auto: outlier rejection is off for stacked panels; choose On to reject outliers where three or more panels overlap.";
  }
}

function stageFields(stage: string): string[] {
  const parts = stage.split(":");
  if (parts.length <= STAGE_FIELDS) return parts;
  return [...parts.slice(0, STAGE_FIELDS - 1), parts.slice(STAGE_FIELDS - 1).join(":")];
}

export function mosaicProgressText(stage: string, busy: "estimate" | "run"): string {
  if (!stage) return busy === "estimate" ? "Reading the panels..." : "Starting the mosaic...";
  const [step, , position = "", detail = ""] = stageFields(stage);
  const counted = POSITION.test(position);
  const at = counted ? ` (${position})` : "";
  const panel = detail || "a panel";
  switch (step) {
    case "solve":
      return `Solving ${panel}${at}`;
    case "overlaps":
      return `Measuring overlaps: ${panel}${at}`;
    case "levels":
      return "Matching levels";
    case "reject":
      return `Finding outliers: ${panel}${at}`;
    case "warp":
      return `Reprojecting ${panel}${at}`;
    case "residuals":
      return `Checking star positions${counted ? ` (${position} overlaps)` : ""}`;
    case "saving":
      return `Saving ${detail || "the mosaic"}`;
    case "complete":
      return "Finishing";
    default:
      return step;
  }
}

export interface MosaicResidualView {
  pair: string;
  shift: string;
  stars: string;
  warn: boolean;
}

export function residualViews(result: Pick<MosaicResult, "pair_residuals" | "tiles">): MosaicResidualView[] {
  return result.pair_residuals.map((residual) => {
    const pair = `${tileName(result.tiles, residual.i)} ↔ ${tileName(result.tiles, residual.j)}`;
    const { dx, dy } = residual;
    if (residual.matched === 0 || !finite(dx) || !finite(dy)) return { pair, shift: "not measured", stars: "no shared stars", warn: false };
    const shift = Math.hypot(dx, dy);
    return {
      pair,
      shift: `${shift.toFixed(2)} px (dx ${dx.toFixed(2)}, dy ${dy.toFixed(2)})`,
      stars: plural(residual.matched, "star"),
      warn: shift > MOSAIC_RESIDUAL_WARNING_PX,
    };
  });
}

export function mosaicOutputName(firstPanel: string, panels: number, at: Date): string {
  return `${frameStem(firstPanel)}_mosaic${panels}_${runTimestamp(at)}`;
}

export function mosaicViewOutput(result: Pick<MosaicResult, "fits_path" | "previewUrl" | "dimensions" | "planes">, panels: number): StackViewOutput {
  return {
    output: {
      fitsPath: result.fits_path,
      previewUrl: result.previewUrl ?? null,
      dimensions: toDims(result.dimensions),
      kind: "stacking",
      inputPath: result.fits_path,
      planes: result.planes,
    },
    label: `Mosaic · ${plural(panels, "panel")}`,
  };
}

export interface PanelFilterGroup {
  lane: PipelineLane | null;
  paths: string[];
}

function laneOfPath(path: string, files: readonly ChannelSource[]): PipelineLane | null {
  const file = files.find((candidate) => candidate.path !== undefined && samePath(candidate.path, path));
  return file ? laneForBin(detectChannel(file)) : null;
}

export function panelFilterGroups(paths: readonly string[], files: readonly ChannelSource[]): PanelFilterGroup[] {
  const groups: PanelFilterGroup[] = [];
  for (const path of paths) {
    const lane = laneOfPath(path, files);
    const group = groups.find((g) => g.lane === lane);
    if (group) group.paths.push(path);
    else groups.push({ lane, paths: [path] });
  }
  return groups;
}

export function mixedFilterLine(groups: readonly PanelFilterGroup[]): string | null {
  const known = groups.filter((group) => group.lane !== null);
  if (known.length < 2) return null;
  const counts = known.map((group) => `${group.lane} ${group.paths.length}`).join(", ");
  return `The picked panels hold ${known.length} filters (${counts}): a mosaic joins panels of one filter, so build one mosaic per filter.`;
}

export function mosaicChannelOf(paths: readonly string[], files: readonly ChannelSource[]): PipelineLane {
  const groups = panelFilterGroups(paths, files);
  return groups.length === 1 && groups[0].lane !== null ? groups[0].lane : MONO_LANE;
}

export interface MosaicCanvasChoice {
  value: string;
  label: string;
}

export function canvasChoices(mosaics: readonly Pick<SessionMosaic, "fitsPath" | "name">[], canvasOf: string | null): MosaicCanvasChoice[] {
  const choices: MosaicCanvasChoice[] = [
    { value: NEW_CANVAS, label: "New, from these panels" },
    ...mosaics.map((mosaic) => ({ value: mosaic.fitsPath, label: `Use the canvas of ${mosaic.name}` })),
  ];
  if (canvasOf !== null && !mosaics.some((mosaic) => samePath(mosaic.fitsPath, canvasOf))) {
    choices.push({ value: canvasOf, label: `Use the canvas of ${baseName(canvasOf)}` });
  }
  choices.push({ value: CHOOSE_CANVAS, label: "Use the canvas of another FITS..." });
  return choices;
}

export interface MosaicComposePlan {
  masters: ComposeMaster[];
  notes: string[];
}

export function composePlan(mosaics: readonly SessionMosaic[]): MosaicComposePlan {
  const newest = new Map<PipelineLane, SessionMosaic>();
  const rgb: string[] = [];
  let older = 0;
  for (const mosaic of mosaics) {
    if (mosaic.planes !== 1) rgb.push(mosaic.name);
    else if (newest.has(mosaic.label)) older += 1;
    else newest.set(mosaic.label, mosaic);
  }
  const masters = PIPELINE_LANES.flatMap((lane) => {
    const mosaic = newest.get(lane);
    return mosaic ? [{ label: lane, fits_path: mosaic.fitsPath }] : [];
  });
  const notes: string[] = [];
  if (older > 0) notes.push(`Compose takes the newest mosaic of each channel: ${older} older mosaic${older === 1 ? " is" : "s are"} left out.`);
  if (rgb.length > 0) notes.push(`RGB mosaics stay out of Compose: ${rgb.join(", ")}.`);
  return { masters, notes };
}
