import type { WcsCard, WriteSolvedWcsResult } from "../shared/types/astrometry";
import type { GaiaRefineDecision, GaiaRefineResult, SolveCredit } from "../services/astrometry";
import { formatDuration, type SolveField } from "./plateSolveResult";
import { effectiveBinFiles, spccInputs, type WizardState } from "./wizard";
import { normalizeOutputPath } from "./processingChain";

export const SOLVED_WCS_LABEL = "Plate-solved WCS";
export const NO_SOLVE_YET = "Plate solve the image first.";
export const NO_WCS_FILE = "astrometry.net returned no WCS file for this job; solve again to write the WCS.";

export function wcsWriteBlocker(result: { wcs_cards?: readonly (readonly [string, string])[] } | null): string | null {
  if (!result) return NO_SOLVE_YET;
  if (!result.wcs_cards || result.wcs_cards.length === 0) return NO_WCS_FILE;
  return null;
}

export const WCS_ALREADY_WRITTEN = "The image on screen already carries this solved WCS.";

export function wcsRewriteBlocker(writtenPath: string | null, solvePath: string | null): string | null {
  return solvePath !== null && writtenPath === solvePath ? WCS_ALREADY_WRITTEN : null;
}

function baseName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

export function wcsSourceLabel(wcsPath: string): string {
  return `WCS from ${baseName(wcsPath)}`;
}

export function solvedWcsSummary(result: WriteSolvedWcsResult): string {
  const centre = `${result.center_ra.toFixed(5)}, ${result.center_dec.toFixed(5)} deg`;
  const scale = `${result.pixel_scale_arcsec.toPrecision(4)}"/px`;
  return `Wrote ${baseName(result.fits_path)}: centre ${centre}, ${scale}${result.sip_present ? ", SIP" : ""}`;
}

export const GAIA_REFINE_CONSENT =
  "Refine with Gaia DR3 sends the field centre and radius to VizieR (CDS) and downloads the Gaia DR3 star list of this field; the image stays on this computer. Gaia DR3 data: ESA/Gaia/DPAC, CC BY-NC 3.0 IGO.";
export const NO_WCS_TO_REFINE = "Plate solve the image first, or open an image whose header has a celestial WCS.";
const EPOCH_ERROR = "Epoch: type a decimal year between 1900 and 2100, or leave it empty to use the header date";
const MIN_EPOCH_YEAR = 1900;
const MAX_EPOCH_YEAR = 2100;
const ARCSEC_PER_DEG = 3600;
const ASSUMED_EPOCH_UNCERTAINTY_YR = 10;
const MS_PER_S = 1000;

const REFINE_HEADINGS: Readonly<Record<GaiaRefineDecision, string>> = {
  applied: "Refined with Gaia DR3",
  no_significant_correction: "No significant correction",
  refused: "Not refined",
};

const REFINED_WCS_NAMES: Readonly<Record<string, string>> = {
  solve: "Plate solve",
  header: "Header WCS",
  gwcs: "gWCS (JWST distortion model)",
};

const REFINE_MODEL_NAMES: Readonly<Record<string, string>> = {
  shift: "Shift",
  rshift: "Shift + rotation",
  rscale: "Shift + rotation + scale",
  general: "General linear",
};

function finiteOrNull(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function signed(value: number, digits: number): string {
  return `${value >= 0 ? "+" : ""}${value.toFixed(digits)}`;
}

export const GAIA_REFINE_CANCELLED_NOTE = "Gaia DR3 refinement cancelled; the WCS is unchanged";

export interface GaiaRefineProgress {
  stage: string;
  current: number;
  total: number;
}

const REFINE_STAGE_TEXTS: Readonly<Record<string, (progress: GaiaRefineProgress) => string>> = {
  load: () => "Reading the image and its WCS",
  detect: () => "Detecting stars; the Gaia DR3 star list is fetched at the same time",
  measure: ({ current, total }) => (total > 0 ? `Measuring stars: ${current} of ${total}` : "Measuring stars"),
  vizier: ({ current, total }) => `Waiting for VizieR: ${formatDuration(current * MS_PER_S)} / ${formatDuration(total * MS_PER_S)}`,
  fit: () => "Fitting the tie to Gaia DR3",
  represent: () => "Computing the refined WCS cards",
};

export function gaiaRefineProgressText(progress: GaiaRefineProgress | null): string | null {
  if (progress === null || !Object.hasOwn(REFINE_STAGE_TEXTS, progress.stage)) return null;
  return REFINE_STAGE_TEXTS[progress.stage](progress);
}

export function creditedSolveTier(
  solve: { catalog?: string | null; credit?: SolveCredit | null } | null,
  refined: GaiaRefineResult | null,
): string | null {
  if (!solve?.credit || !solve.catalog) return null;
  return refined === null || refined.wcs_kind === "solve" ? solve.catalog : null;
}

export function gaiaRefineBlocker(hasWcs: boolean): string | null {
  return hasWcs ? null : NO_WCS_TO_REFINE;
}

export interface RefineSource {
  solveCards: readonly WcsCard[];
  solvedPath: string | null;
  path: string;
  fileWcsKind: string | null | undefined;
}

export function refineSolveCards({ solveCards, solvedPath, path, fileWcsKind }: RefineSource): readonly WcsCard[] | null {
  if (solveCards.length === 0 || solvedPath === null || fileWcsKind === "gwcs") return null;
  return normalizeOutputPath(solvedPath) === normalizeOutputPath(path) ? solveCards : null;
}

export function appliedRefinement(result: GaiaRefineResult | null): GaiaRefineResult | null {
  return result !== null && result.decision === "applied" && result.wcs_cards.length > 0 ? result : null;
}

export function gaiaRefineHeading(result: GaiaRefineResult): string {
  return REFINE_HEADINGS[result.decision] ?? result.decision;
}

function rmsText(result: GaiaRefineResult): string | null {
  const holdoutBefore = finiteOrNull(result.holdout_rms_before_mas);
  const holdoutAfter = finiteOrNull(result.holdout_rms_after_mas);
  if (holdoutBefore !== null && holdoutAfter !== null) {
    return `${holdoutBefore.toFixed(1)} mas before, ${holdoutAfter.toFixed(1)} mas after (hold-out)`;
  }
  const before = finiteOrNull(result.rms_before_mas);
  const after = finiteOrNull(result.rms_after_mas);
  if (before === null && after === null) return null;
  const parts = [before === null ? null : `${before.toFixed(1)} mas before`, after === null ? null : `${after.toFixed(1)} mas after`];
  return parts.filter((p): p is string => p !== null).join(", ");
}

function shiftText(result: GaiaRefineResult): string | null {
  const east = finiteOrNull(result.shift_east_mas);
  const north = finiteOrNull(result.shift_north_mas);
  if (east === null || north === null) return null;
  const errEast = finiteOrNull(result.shift_err_east_mas);
  const errNorth = finiteOrNull(result.shift_err_north_mas);
  const sigma = errEast !== null && errNorth !== null ? ` (1σ ${errEast.toFixed(1)}, ${errNorth.toFixed(1)})` : "";
  return `${signed(east, 1)} mas E, ${signed(north, 1)} mas N${sigma}`;
}

function rotationText(result: GaiaRefineResult): string | null {
  const rotation = finiteOrNull(result.rotation_deg);
  if (rotation === null) return null;
  const err = finiteOrNull(result.rotation_err_deg);
  const sigma = err === null ? "" : ` (1σ ${err.toFixed(5)}°)`;
  return `${signed(rotation, 5)}° (${signed(rotation * ARCSEC_PER_DEG, 2)}")${sigma}`;
}

function scaleText(result: GaiaRefineResult): string | null {
  const ppm = finiteOrNull(result.scale_ppm);
  if (ppm === null) return null;
  const err = finiteOrNull(result.scale_err_ppm);
  return `${signed(ppm, 1)} ppm${err === null ? "" : ` (1σ ${err.toFixed(1)})`}`;
}

function epochText(result: GaiaRefineResult): string {
  const year = finiteOrNull(result.epoch_year);
  if (year === null || result.epoch_source === "none") return "J2016.0 (no date in the header)";
  return `${year.toFixed(3)} (${result.epoch_source})`;
}

function modelText(model: string | null | undefined): string | null {
  if (!model) return null;
  const name = REFINE_MODEL_NAMES[model];
  return name ? `${name} (${model})` : model;
}

export function gaiaRefineFields(result: GaiaRefineResult): SolveField[] {
  const entries: (SolveField | null)[] = [
    {
      key: "refine-matched",
      label: "Matched",
      value: `${result.n_used} used of ${result.n_matched} matched (${result.n_catalog} Gaia DR3 stars)`,
    },
    ((value) => (value ? { key: "refine-rms", label: "RMS", value } : null))(rmsText(result)),
    ((value) => (value ? { key: "refine-shift", label: "Shift", value } : null))(shiftText(result)),
    ((value) => (value ? { key: "refine-rotation", label: "Rotation", value } : null))(rotationText(result)),
    ((value) => (value ? { key: "refine-scale", label: "Scale", value } : null))(scaleText(result)),
    ((value) => (value ? { key: "refine-model", label: "Model", value } : null))(modelText(result.model)),
    { key: "refine-epoch", label: "Epoch", value: epochText(result) },
    ((term) =>
      term === null ? null : { key: "refine-epoch-term", label: "Epoch term", value: `${term.toFixed(1)} mas for an epoch uncertain by ${ASSUMED_EPOCH_UNCERTAINTY_YR} yr` })(
      finiteOrNull(result.epoch_systematic_mas),
    ),
    ((name) => (name ? { key: "refine-wcs", label: "Refined WCS", value: name } : null))(
      result.wcs_kind ? (REFINED_WCS_NAMES[result.wcs_kind] ?? result.wcs_kind) : null,
    ),
    result.representation ? { key: "refine-representation", label: "WCS update", value: result.representation } : null,
    { key: "refine-catalog", label: "Catalogue", value: `${result.catalog} (${result.catalog_source})` },
  ];
  return entries.filter((entry): entry is SolveField => entry !== null);
}

export interface EpochYearInput {
  value: number | null;
  error: string | null;
}

export function parseEpochYear(text: string): EpochYearInput {
  const trimmed = text.trim();
  if (trimmed === "") return { value: null, error: null };
  const value = Number(trimmed.replace(",", "."));
  if (!Number.isFinite(value) || value < MIN_EPOCH_YEAR || value > MAX_EPOCH_YEAR) return { value: null, error: EPOCH_ERROR };
  return { value, error: null };
}

export interface SolveIdentity {
  regionKey: string | null;
  solvePath: string | null;
}

export function shouldResetSolve(prev: SolveIdentity, next: SolveIdentity, writtenPath: string | null): boolean {
  return prev.regionKey !== next.regionKey || (prev.solvePath !== next.solvePath && next.solvePath !== writtenPath);
}

const solvedWcsByInput = new Map<string, string>();
const solvedWcsListeners = new Set<() => void>();
const wcsRevisions = new Map<string, number>();
let solvedWcsWrites = 0;

function notifySolvedWcs(): void {
  for (const listener of solvedWcsListeners) listener();
}

function advanceWcsRevision(path: string): void {
  const key = normalizeOutputPath(path);
  wcsRevisions.set(key, (wcsRevisions.get(key) ?? 0) + 1);
}

export function wcsRevision(path: string | null): number {
  return path === null ? 0 : (wcsRevisions.get(normalizeOutputPath(path)) ?? 0);
}

export function rememberSolvedWcs(inputPath: string, fitsPath: string): void {
  solvedWcsByInput.set(normalizeOutputPath(inputPath), fitsPath);
  solvedWcsWrites += 1;
  advanceWcsRevision(inputPath);
  advanceWcsRevision(fitsPath);
  notifySolvedWcs();
}

export function solvedWcsFor(gridPath: string): string | null {
  return solvedWcsByInput.get(normalizeOutputPath(gridPath)) ?? null;
}

export function subscribeSolvedWcs(listener: () => void): () => void {
  solvedWcsListeners.add(listener);
  return () => {
    solvedWcsListeners.delete(listener);
  };
}

export function solvedWcsVersion(): number {
  return solvedWcsWrites;
}

export interface SpccWcsSource {
  gridPath: string | null;
  reason: string | null;
}

function alignedOntoReason(binLabel: string): string {
  return `SPCC can use a plate-solved WCS only on the R channel's own grid; R was aligned onto ${binLabel}. Make R the alignment reference and run Align again.`;
}

function solvedOnRGrid(state: WizardState, binId: string, gridPath: string | null): string | null {
  const stacked = state.stackedPaths[binId];
  return (stacked ? solvedWcsFor(stacked) : null) ?? (gridPath ? solvedWcsFor(gridPath) : null);
}

export function spccWcsSource(state: WizardState): SpccWcsSource {
  const r = spccInputs(state).r;
  const bin = r ? state.bins.find((b) => b.id === r.binId) : undefined;
  if (!bin) return { gridPath: null, reason: null };
  const gridPath = effectiveBinFiles(state, bin)[0] ?? null;
  if (Object.keys(state.alignedPaths).length > 0) {
    const active = state.bins.filter((b) => b.files.length > 0);
    const reference = active.find((b) => b.id === state.alignRefBinId) ?? active[0];
    if (reference && reference.id !== bin.id) {
      const solved = solvedOnRGrid(state, bin.id, gridPath) !== null;
      return { gridPath: null, reason: solved ? alignedOntoReason(reference.shortLabel) : null };
    }
  }
  return { gridPath, reason: null };
}

export function spccSolvedWcsPath(state: WizardState, source: SpccWcsSource = spccWcsSource(state)): string | null {
  const binId = spccInputs(state).r?.binId;
  return source.gridPath && binId ? solvedOnRGrid(state, binId, source.gridPath) : null;
}
