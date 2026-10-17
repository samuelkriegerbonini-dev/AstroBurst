import type { ContinuumWindows, NoiseFactor, RegionSpectrum } from "../shared/types/cube";
import type { SpectrumView } from "./spectrumExport";

export interface RegionViewErrors {
  err: number[] | null;
  scaled: number[] | null;
}

export interface ErrorBandRun {
  x: number[];
  lower: number[];
  upper: number[];
}

const DECIMALS = 2;

export function regionViewErrors(region: RegionSpectrum, view: SpectrumView): RegionViewErrors {
  const err = view === "mean" ? region.mean_err : view === "jy" ? region.flux_jy_err : region.err;
  if (err === null || region.err_scaled === null) return { err, scaled: null };
  if (view === "sum") return { err, scaled: region.err_scaled };
  const factor = region.noise_factor?.factor;
  return { err, scaled: factor === undefined ? null : err.map((e) => e * factor) };
}

export function noiseFactorLabel(errSource: string, factor: NoiseFactor): string {
  return `${errSource} x ${factor.factor.toFixed(DECIMALS)} +- ${factor.factor_se.toFixed(DECIMALS)} (${factor.n_channels} line-free channels)`;
}

function sameWindows(a: ContinuumWindows, b: ContinuumWindows | null): boolean {
  return b !== null && a.every((window, side) => window[0] === b[side][0] && window[1] === b[side][1]);
}

export function windowsText(windows: ContinuumWindows, joiner: string, dash: string): string {
  const [first, second] = windows;
  const one = (w: readonly [number, number]) => `${w[0]}${dash}${w[1]}`;
  const single = first[0] === second[0] && first[1] === second[1];
  return single ? one(first) : `${one(first)}${joiner}${one(second)}`;
}

export function noiseFactorStaleNote(factor: NoiseFactor, windowsToSend: ContinuumWindows | null): string | null {
  if (sameWindows(factor.windows, windowsToSend)) return null;
  const measured = `noise factor measured on continuum ${windowsText(factor.windows, " and ", "–")}`;
  if (windowsToSend === null) {
    return `${measured}; no valid continuum windows are set: set them, then press Region spectrum to measure it again (without windows it drops the factor)`;
  }
  return `${measured}; the windows changed since: press Region spectrum to measure it again`;
}

export type NoiseFactorState = "measured" | "not_measured" | "none";

export function regionErrorLine(
  errSource: string,
  factor: NoiseFactor | null,
  windowsSent: ContinuumWindows | null,
): { text: string; factor: NoiseFactorState } {
  if (factor !== null) return { text: noiseFactorLabel(errSource, factor), factor: "measured" };
  if (windowsSent === null) return { text: `error band: ${errSource}`, factor: "none" };
  return {
    text: `error band: ${errSource}; noise factor not measured on continuum ${windowsText(windowsSent, " and ", "–")} (see notes)`,
    factor: "not_measured",
  };
}

export type ErrorBandKind = "none" | "propagated" | "propagated+scaled";

export function errorBandKind(errors: RegionViewErrors | null): ErrorBandKind {
  if (errors === null || errors.err === null) return "none";
  return errors.scaled === null ? "propagated" : "propagated+scaled";
}

export function errorBandLegend(kind: ErrorBandKind): string | null {
  return kind === "propagated+scaled" ? "fill: propagated error; dashed: error x noise factor" : null;
}

export function errorBandExtent(
  x: readonly number[] | null,
  y: readonly number[],
  errors: RegionViewErrors | null,
): [number, number] | null {
  if (errors === null) return null;
  let low = Infinity;
  let high = -Infinity;
  for (const band of [errors.err, errors.scaled]) {
    for (const run of errorBandRuns(x, y, band)) {
      for (const value of run.lower) low = Math.min(low, value);
      for (const value of run.upper) high = Math.max(high, value);
    }
  }
  return low <= high ? [low, high] : null;
}

export function regionNotesShown(region: RegionSpectrum): string[] {
  const warning = region.noise_factor?.warning ?? null;
  return warning === null ? region.notes : region.notes.filter((note) => note !== warning);
}

export function errPlaneLabel(hasErr: boolean, errSource: string, noiseFactor: number | null): string {
  if (!hasErr) return `no ERR written (error source: ${errSource})`;
  const factor = noiseFactor !== null && Number.isFinite(noiseFactor) ? `, noise factor ${noiseFactor.toFixed(DECIMALS)} in ABNOISEF` : "";
  return `ERR written (${errSource})${factor}`;
}

export function errorBandRuns(
  x: readonly number[] | null,
  y: readonly number[],
  err: readonly number[] | null,
): ErrorBandRun[] {
  if (err === null || err.length !== y.length || (x !== null && x.length !== y.length)) return [];
  const runs: ErrorBandRun[] = [];
  let run: ErrorBandRun | null = null;
  for (let i = 0; i < y.length; i++) {
    const xi = x ? x[i] : i;
    const yi = y[i];
    const ei = err[i];
    if (!Number.isFinite(xi) || !Number.isFinite(yi) || !Number.isFinite(ei) || ei < 0) {
      run = null;
      continue;
    }
    if (run === null) {
      run = { x: [], lower: [], upper: [] };
      runs.push(run);
    }
    run.x.push(xi);
    run.lower.push(yi - ei);
    run.upper.push(yi + ei);
  }
  return runs;
}
