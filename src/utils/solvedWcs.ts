import type { WriteSolvedWcsResult } from "../shared/types/astrometry";
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

export interface SolveIdentity {
  regionKey: string | null;
  solvePath: string | null;
}

export function shouldResetSolve(prev: SolveIdentity, next: SolveIdentity, writtenPath: string | null): boolean {
  return prev.regionKey !== next.regionKey || (prev.solvePath !== next.solvePath && next.solvePath !== writtenPath);
}

const solvedWcsByInput = new Map<string, string>();
const solvedWcsListeners = new Set<() => void>();
let solvedWcsWrites = 0;

export function rememberSolvedWcs(inputPath: string, fitsPath: string): void {
  solvedWcsByInput.set(normalizeOutputPath(inputPath), fitsPath);
  solvedWcsWrites += 1;
  for (const listener of solvedWcsListeners) listener();
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
