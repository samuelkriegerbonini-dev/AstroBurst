import type { LightCurveDoc, LightCurveViewState, LightCurveWindow } from "../shared/types/lightCurve";
import type { SeriesKey, TsoCurve, TsoLightCurves, TsoTimeScale } from "../shared/types/tso";
import { formatSignificant } from "./formatSignificant";
import { QUICK_LOOK_LABEL } from "./lightCurveAnalysis";
import { TIME_SCALE_NAMES } from "./lightCurveDoc";
import { lightCurveModel } from "./lightCurveViewModel";
import { medianOf } from "./measurementLog";
import { finiteOrNull, isFiniteNumber } from "./values";

export const PIXEL_LOSS_FLAG = "pixel loss";
export const PIXEL_LOSS_RATIO = 0.9;
export const WHITE_DOC_ID = "white";
export const BINS_OVERLAY = "bins";
export const INDEX_TIME_LABEL = "integration number";
export const MAX_TSO_BINS = 512;
export const DEFAULT_REJECT_SIGMA = 100;

const BAND_DIGITS = 4;
const RATIO_DIGITS = 3;
const DEPTH_RULE_NOTE = "depth D = 1 - F_in / F_out per bin; the error is the larger of the scatter and the formal estimate";
const NO_WINDOW_NOTE = "set a transit window in the light-curve view to get depths";

export type BinsInput = { ok: true; bins: [number, number][] } | { ok: false; error: string };

export interface DepthRow {
  docId: string;
  label: string;
  band: [number, number];
  centre: number;
  halfWidth: number;
  depth: number | null;
  errScatter: number | null;
  errFormal: number | null;
  err: number | null;
  nIn: number;
  nOut: number;
  pixelRatio: number | null;
  pixelLoss: boolean;
}

export interface DepthSpectrum {
  white: DepthRow | null;
  rows: DepthRow[];
  window: LightCurveWindow | null;
  marginDays: number;
  notes: string[];
  warnings: string[];
}

export function tsoGroupKey(setId: string, key: SeriesKey): string {
  return `tso:${setId}:${key.detector}:${key.sporder ?? ""}`;
}

export function binDocId(index: number): string {
  return `bin:${index}`;
}

export function bandText(band: readonly [number, number]): string {
  return `${formatSignificant(band[0], BAND_DIGITS)} to ${formatSignificant(band[1], BAND_DIGITS)} um`;
}

export function tsoTimeName(scale: TsoTimeScale): string {
  return scale === "index" ? INDEX_TIME_LABEL : TIME_SCALE_NAMES[scale];
}

export function pixelLossFlags(npixelsMean: readonly (number | null)[]): boolean[] {
  const median = medianOf(npixelsMean);
  return npixelsMean.map((n) => median !== null && isFiniteNumber(n) && n < PIXEL_LOSS_RATIO * median);
}

function elementNotes(nUsed: readonly number[]): string[] {
  if (nUsed.length === 0) return [];
  const fewest = nUsed.reduce((a, b) => Math.min(a, b));
  const most = nUsed.reduce((a, b) => Math.max(a, b));
  const range = fewest === most ? `${fewest}` : `${fewest} to ${most}`;
  const empty = nUsed.filter((n) => n === 0).length;
  const notes = [`${range} elements summed per integration`];
  if (empty > 0) notes.push(`${empty} integration(s) with no element summed are left empty`);
  return notes;
}

function curveDoc(
  result: TsoLightCurves,
  paths: readonly string[],
  curve: TsoCurve,
  identity: Pick<LightCurveDoc, "id" | "label" | "overlay">,
  sumNote: string,
): LightCurveDoc {
  const value = result.time.map((_, i) => (curve.n_used[i] === 0 ? null : finiteOrNull(curve.flux[i])));
  const err = value.map((v, i) => {
    const e = curve.flux_err[i];
    return v !== null && isFiniteNumber(e) && e > 0 ? e : null;
  });
  const lost = pixelLossFlags(curve.npixels_mean);
  const nLost = lost.filter(Boolean).length;
  const notes = [sumNote, ...elementNotes(curve.n_used)];
  if (nLost > 0) {
    notes.push(`${nLost} integration(s) flagged ${PIXEL_LOSS_FLAG}: mean NPIXELS below ${PIXEL_LOSS_RATIO} of its median`);
  }
  return {
    ...identity,
    quantity: "flux",
    unit: result.flux_unit === "unknown" ? null : result.flux_unit,
    timeScale: result.time_scale,
    ...(result.time_scale === "index" ? { timeLabel: INDEX_TIME_LABEL } : {}),
    time: [...result.time],
    value,
    err,
    pointRef: [...result.int_num],
    flags: result.time.map((_, i) => (lost[i] ? [PIXEL_LOSS_FLAG] : [])),
    source: {
      kind: "tso",
      paths: [...paths],
      detector: result.series.detector,
      sporder: result.series.sporder,
      band_um: [curve.band_um[0], curve.band_um[1]],
      mode: result.mode,
    },
    notes,
  };
}

export function lightCurveDocsFromTso(result: TsoLightCurves, paths: readonly string[]): LightCurveDoc[] {
  const sumOver = (curve: TsoCurve) => `${result.mode} mode: sum of FLUX over ${bandText(curve.band_um)}`;
  const white = curveDoc(
    result,
    paths,
    result.white,
    { id: WHITE_DOC_ID, label: `white light ${bandText(result.white.band_um)}` },
    `${sumOver(result.white)} (${result.white.limits_origin} limits)`,
  );
  const bins = result.bins.map((bin, k) =>
    curveDoc(
      result,
      paths,
      bin,
      { id: binDocId(k), label: bandText(bin.band_um), overlay: BINS_OVERLAY },
      `${sumOver(bin)}, upper edge excluded`,
    ),
  );
  return [white, ...bins];
}

function contiguousBins(edges: readonly number[]): [number, number][] {
  return edges.slice(1).map((upper, k) => [edges[k], upper]);
}

export function equalWidthBins(lo: number, hi: number, count: number): [number, number][] {
  if (!Number.isFinite(lo) || !Number.isFinite(hi) || lo >= hi) return [];
  if (!Number.isInteger(count) || count < 1 || count > MAX_TSO_BINS) return [];
  const width = (hi - lo) / count;
  return contiguousBins(Array.from({ length: count + 1 }, (_, k) => (k === count ? hi : lo + k * width)));
}

export function binsFromEdgesText(text: string): BinsInput {
  const tokens = text.split(/[\s,;]+/).filter((token) => token !== "");
  if (tokens.length === 0) return { ok: true, bins: [] };
  const edges = tokens.map(Number);
  const notNumber = edges.findIndex((edge) => !Number.isFinite(edge));
  if (notNumber >= 0) return { ok: false, error: `edge '${tokens[notNumber]}' is not a number` };
  if (edges.length === 1) return { ok: false, error: "two or more edges make a bin" };
  const notAbove = edges.findIndex((edge, k) => k > 0 && edge <= edges[k - 1]);
  if (notAbove >= 0) {
    return { ok: false, error: `edges must increase: ${tokens[notAbove]} is not above ${tokens[notAbove - 1]}` };
  }
  if (edges.length - 1 > MAX_TSO_BINS) return { ok: false, error: `at most ${MAX_TSO_BINS} bins` };
  return { ok: true, bins: contiguousBins(edges) };
}

function meanOf(values: readonly number[]): number {
  return values.reduce((total, v) => total + v, 0) / values.length;
}

export function windowPixelRatio(
  time: readonly number[],
  npixelsMean: readonly (number | null)[],
  window: LightCurveWindow,
  marginDays: number,
): number | null {
  const margin = marginDays > 0 ? marginDays : 0;
  const inside: number[] = [];
  const outside: number[] = [];
  time.forEach((t, i) => {
    const n = npixelsMean[i];
    if (!isFiniteNumber(n)) return;
    if (window.start <= t && t <= window.end) inside.push(n);
    else if (t <= window.start - margin || t >= window.end + margin) outside.push(n);
  });
  if (inside.length === 0 || outside.length === 0) return null;
  const baseline = meanOf(outside);
  return baseline === 0 ? null : meanOf(inside) / baseline;
}

function depthRow(
  doc: LightCurveDoc,
  curve: TsoCurve,
  view: LightCurveViewState,
  window: LightCurveWindow | null,
  marginDays: number,
): DepthRow {
  const depth = lightCurveModel(doc, view).depth;
  const [lo, hi] = curve.band_um;
  const pixelRatio = window === null ? null : windowPixelRatio(doc.time, curve.npixels_mean, window, marginDays);
  return {
    docId: doc.id,
    label: doc.label,
    band: [lo, hi],
    centre: (lo + hi) / 2,
    halfWidth: (hi - lo) / 2,
    depth: depth?.depth ?? null,
    errScatter: depth?.errScatter ?? null,
    errFormal: depth?.errFormal ?? null,
    err: depth?.err ?? null,
    nIn: depth?.nIn ?? 0,
    nOut: depth?.nOut ?? 0,
    pixelRatio,
    pixelLoss: pixelRatio !== null && pixelRatio < PIXEL_LOSS_RATIO,
  };
}

export function depthSpectrum(docs: readonly LightCurveDoc[], view: LightCurveViewState, curves: TsoLightCurves): DepthSpectrum {
  const asked = view.window;
  const window =
    asked !== null && Number.isFinite(asked.start) && Number.isFinite(asked.end) && asked.start <= asked.end ? asked : null;
  const marginDays = Number.isFinite(view.marginDays) && view.marginDays > 0 ? view.marginDays : 0;
  const curveOfDoc = new Map<string, TsoCurve>([
    [WHITE_DOC_ID, curves.white],
    ...curves.bins.map((bin, k): [string, TsoCurve] => [binDocId(k), bin]),
  ]);
  const measured = docs.flatMap((doc) => {
    const curve = curveOfDoc.get(doc.id);
    return curve === undefined ? [] : [depthRow(doc, curve, view, window, marginDays)];
  });
  const ratios = measured.flatMap((row) => (row.pixelRatio === null ? [] : [row.pixelRatio]));
  const nLost = measured.filter((row) => row.pixelLoss).length;
  const notes = [QUICK_LOOK_LABEL, DEPTH_RULE_NOTE];
  if (window === null) notes.push(NO_WINDOW_NOTE);
  const warnings =
    nLost === 0
      ? []
      : [
          `mean NPIXELS inside the window is below ${PIXEL_LOSS_RATIO} of the baseline in ${nLost} of ${ratios.length} curves (lowest ratio ${formatSignificant(Math.min(...ratios), RATIO_DIGITS)}): the extraction lost pixels during the event, so these depths are biased high`,
        ];
  return {
    white: measured.find((row) => row.docId === WHITE_DOC_ID) ?? null,
    rows: measured.filter((row) => row.docId !== WHITE_DOC_ID),
    window,
    marginDays,
    notes,
    warnings,
  };
}
