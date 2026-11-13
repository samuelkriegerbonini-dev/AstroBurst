import type { LightCurveBin, LightCurveDoc, LightCurveQuickLook, LightCurveViewState } from "../shared/types/lightCurve";
import {
  MAG_ERR_EXACT,
  binInTime,
  boxDepth,
  fitBaseline,
  foldPhase,
  normalise,
  outOfTransitScatter,
  relativeFlux,
  sigmaClip,
  type Baseline,
  type BinBy,
  type BoxDepth,
  type ScatterSummary,
  type TimeWindow,
} from "./lightCurveAnalysis";
import { EXCLUDABLE_FLAGS, docTimeLabel, timeOffset } from "./lightCurveDoc";
import { finiteOrNull, isFiniteNumber } from "./values";

export type PointState = "kept" | "missing" | "flagged" | "manual" | "clipped";

export interface SeriesChoice {
  key: string;
  label: string;
  docIds: string[];
}

export interface BinnedSeries {
  x: number[];
  y: number[];
  yErr: (number | null)[];
  n: number[];
}

export interface FoldedSeries {
  phase: number[];
  y: (number | null)[];
  yErr: (number | null)[];
  index: number[];
}

export interface ModelBand {
  x0: number;
  x1: number;
  kind: "window" | "margin";
}

export interface ModelCounts {
  total: number;
  kept: number;
  missing: number;
  flagged: number;
  manual: number;
  clipped: number;
}

export interface LightCurveModel {
  doc: LightCurveDoc;
  offset: number;
  xLabel: string;
  yLabel: string;
  invertY: boolean;
  normalised: boolean;
  x: number[];
  y: (number | null)[];
  yErr: (number | null)[];
  state: PointState[];
  inWindow: boolean[];
  baseline: Baseline | null;
  scatter: ScatterSummary | null;
  depth: BoxDepth | null;
  binned: BinnedSeries | null;
  binnedScatter: ScatterSummary | null;
  bands: ModelBand[];
  counts: ModelCounts;
  notes: string[];
  folded: FoldedSeries | null;
}

export interface ModelSeries {
  kept: { x: number[]; y: (number | null)[]; yErr: (number | null)[] };
  excluded: { x: number[]; y: (number | null)[] } | null;
}

export const NO_BASELINE_NOTE =
  "no baseline: normalised flux needs points outside the transit window (two with different times for a linear baseline)";
export const NO_DEPTH_NOTE = "no depth: the transit window holds no point, or no point is left outside it";
export const NO_TIME_BINS_NOTE = "no bins: binning by minutes needs a time axis, and this curve is on an index axis";

export function seriesChoices(docs: readonly LightCurveDoc[]): SeriesChoice[] {
  const choices: SeriesChoice[] = [];
  const overlays = new Map<string, SeriesChoice>();
  for (const doc of docs) {
    if (doc.overlay === undefined) {
      choices.push({ key: doc.id, label: doc.label, docIds: [doc.id] });
      continue;
    }
    const shared = overlays.get(doc.overlay);
    if (shared !== undefined) {
      shared.docIds.push(doc.id);
      continue;
    }
    const choice: SeriesChoice = { key: `overlay:${doc.overlay}`, label: doc.overlay, docIds: [doc.id] };
    overlays.set(doc.overlay, choice);
    choices.push(choice);
  }
  return choices;
}

export function activeChoice(choices: readonly SeriesChoice[], selection: string | null): SeriesChoice | null {
  return choices.find((choice) => choice.key === selection) ?? choices[0] ?? null;
}

export function valueAxisLabel(doc: Pick<LightCurveDoc, "quantity" | "unit" | "source">, normalised: boolean): string {
  if (normalised) return "normalised flux";
  if (doc.quantity === "rel_flux") return "relative flux";
  if (doc.quantity === "flux") return doc.unit ? `flux (${doc.unit})` : "flux";
  const comps = doc.source.kind === "frames" ? doc.source.comps.length : 0;
  if (comps === 0) return "diff mag";
  return comps === 1 ? "diff mag (1 comp)" : `diff mag (sum of ${comps} comps)`;
}

export function binText(bin: LightCurveBin): string | null {
  if (bin.kind === "points") return `${bin.points} points`;
  if (bin.kind === "minutes") return `${bin.minutes} min`;
  return null;
}

function transitWindow(view: LightCurveViewState): TimeWindow | null {
  const window = view.window;
  if (window === null || !Number.isFinite(window.start) || !Number.isFinite(window.end)) return null;
  return window.start <= window.end ? window : null;
}

function baselineMargin(view: LightCurveViewState): number {
  return Number.isFinite(view.marginDays) && view.marginDays > 0 ? view.marginDays : 0;
}

function isInside(time: number, span: TimeWindow): boolean {
  return span.start <= time && time <= span.end;
}

function keptOnly(values: readonly (number | null)[], state: readonly PointState[]): (number | null)[] {
  return state.map((pointState, i) => (pointState === "kept" ? values[i] : null));
}

function binRule(bin: LightCurveBin, onIndexAxis: boolean): BinBy | null {
  if (bin.kind === "points") return { points: bin.points };
  if (bin.kind === "minutes" && !onIndexAxis) return { widthDays: bin.minutes / 1440 };
  return null;
}

function bandsOf(window: TimeWindow | null, margin: number, offset: number): ModelBand[] {
  if (window === null) return [];
  const x0 = window.start - offset;
  const x1 = window.end - offset;
  const bands: ModelBand[] = [{ x0, x1, kind: "window" }];
  if (margin > 0) {
    bands.push({ x0: window.start - margin - offset, x1: x0, kind: "margin" });
    bands.push({ x0: x1, x1: window.end + margin - offset, kind: "margin" });
  }
  return bands;
}

function foldedSeries(
  doc: LightCurveDoc,
  fold: LightCurveViewState["fold"],
  state: readonly PointState[],
  y: readonly (number | null)[],
  yErr: readonly (number | null)[],
): FoldedSeries | null {
  if (fold === null || doc.timeScale === "index") return null;
  if (!Number.isFinite(fold.t0) || !Number.isFinite(fold.periodDays) || !(fold.periodDays > 0)) return null;
  const phase = foldPhase(doc.time, fold.t0, fold.periodDays);
  const index = state.flatMap((pointState, i) => (pointState === "kept" ? [i] : []));
  index.sort((a, b) => phase[a] - phase[b]);
  return { phase: index.map((i) => phase[i]), y: index.map((i) => y[i]), yErr: index.map((i) => yErr[i]), index };
}

export function lightCurveModel(doc: LightCurveDoc, view: LightCurveViewState): LightCurveModel {
  const t = doc.time;
  const onIndexAxis = doc.timeScale === "index";
  const offset = timeOffset(doc);
  const window = transitWindow(view);
  const margin = baselineMargin(view);
  const baselineSpans: TimeWindow[] | undefined =
    window === null || margin === 0
      ? undefined
      : [
          { start: -Infinity, end: window.start - margin },
          { start: window.end + margin, end: Infinity },
        ];

  const byHand = new Set(view.excluded);
  const chosen: PointState[] = t.map((time, i) => {
    if (!isFiniteNumber(time) || !isFiniteNumber(doc.value[i])) return "missing";
    if (view.excludeFlagged && doc.flags[i]?.some((flag) => EXCLUDABLE_FLAGS.includes(flag))) return "flagged";
    return byHand.has(i) ? "manual" : "kept";
  });
  const inWindow = t.map((time, i) => window !== null && chosen[i] !== "missing" && isInside(time, window));

  const analysis =
    doc.quantity === "diff_mag"
      ? relativeFlux(doc.value, doc.err, doc.magErrFactor ?? MAG_ERR_EXACT)
      : { flux: doc.value.map((value) => finiteOrNull(value)), err: doc.err.map((error) => finiteOrNull(error)) };

  const sigma = view.clipSigma;
  const clipped =
    isFiniteNumber(sigma) && sigma > 0
      ? sigmaClip(
          t,
          analysis.flux.map((flux, i) => {
            const fitsBaseline = baselineSpans === undefined || baselineSpans.some((span) => isInside(t[i], span));
            return chosen[i] === "kept" && !inWindow[i] && fitsBaseline ? flux : null;
          }),
          sigma,
          view.clipWindowPoints,
        )
      : null;
  const state = clipped === null ? chosen : chosen.map((pointState, i) => (clipped[i] ? "clipped" : pointState));

  const baseline = fitBaseline(t, keptOnly(analysis.flux, state), window, view.baselineKind, baselineSpans);
  const curve = baseline === null ? analysis : normalise(t, analysis.flux, analysis.err, baseline);
  const keptFlux = keptOnly(curve.flux, state);
  const keptErr = keptOnly(curve.err, state);
  const scatter = outOfTransitScatter(t, keptFlux, keptErr, window, baselineSpans);
  const depth = window === null ? null : boxDepth(t, keptFlux, keptErr, window, baselineSpans);

  const normalised = view.display === "normalised" && baseline !== null;
  const y = normalised ? curve.flux : state.map((pointState, i) => (pointState === "missing" ? null : doc.value[i]));
  const yErr = normalised ? curve.err : state.map((pointState, i) => (pointState === "missing" ? null : finiteOrNull(doc.err[i])));

  const by = binRule(view.bin, onIndexAxis);
  const drawnBins = by === null ? null : binInTime(t, keptOnly(y, state), keptOnly(yErr, state), by);
  const analysisBins = by === null ? null : binInTime(t, keptFlux, keptErr, by);

  const countOf = (wanted: PointState) => state.filter((pointState) => pointState === wanted).length;
  const flaggedInTransit = doc.flags.filter((words, i) => inWindow[i] && words.length > 0);
  const notes: string[] = [];
  if (view.display === "normalised" && baseline === null) notes.push(NO_BASELINE_NOTE);
  if (flaggedInTransit.length > 0) {
    const words = [...new Set(flaggedInTransit.flat())].join(", ");
    notes.push(`${flaggedInTransit.length} of ${inWindow.filter(Boolean).length} in-transit points are flagged: ${words}`);
  }
  if (window !== null && depth === null) notes.push(NO_DEPTH_NOTE);
  if (view.bin.kind === "minutes" && onIndexAxis) notes.push(NO_TIME_BINS_NOTE);

  return {
    doc,
    offset,
    xLabel: docTimeLabel(doc, offset),
    yLabel: valueAxisLabel(doc, normalised),
    invertY: !normalised && doc.quantity === "diff_mag",
    normalised,
    x: t.map((time) => (isFiniteNumber(time) ? time - offset : NaN)),
    y,
    yErr,
    state,
    inWindow,
    baseline,
    scatter,
    depth,
    binned:
      drawnBins === null
        ? null
        : { x: drawnBins.t.map((time) => time - offset), y: drawnBins.flux, yErr: drawnBins.err, n: drawnBins.n },
    binnedScatter:
      analysisBins === null
        ? null
        : outOfTransitScatter(analysisBins.t, analysisBins.flux, analysisBins.err, window, baselineSpans),
    bands: bandsOf(window, margin, offset),
    counts: {
      total: state.length,
      kept: countOf("kept"),
      missing: countOf("missing"),
      flagged: countOf("flagged"),
      manual: countOf("manual"),
      clipped: countOf("clipped"),
    },
    notes,
    folded: foldedSeries(doc, view.fold, state, y, yErr),
  };
}

export function modelSeries(model: LightCurveModel): ModelSeries {
  const { x, y, yErr, state, counts } = model;
  const kept = { x, y: keptOnly(y, state), yErr: keptOnly(yErr, state) };
  if (counts.flagged + counts.manual + counts.clipped === 0) return { kept, excluded: null };
  const leftOut = state.map((pointState, i) => (pointState === "kept" || pointState === "missing" ? null : y[i]));
  return { kept, excluded: { x, y: leftOut } };
}

export function quickLookOf(model: LightCurveModel, view: LightCurveViewState): LightCurveQuickLook | null {
  const { doc, depth, scatter, counts } = model;
  const window = transitWindow(view);
  if (depth === null || window === null) return null;
  return {
    docId: doc.id,
    label: doc.label,
    source: doc.source,
    quantity: doc.quantity,
    unit: doc.unit,
    timeScale: doc.timeScale,
    timeName: docTimeLabel(doc, 0),
    nPoints: counts.kept,
    window,
    marginDays: baselineMargin(view),
    baselineKind: view.baselineKind,
    depth: depth.depth,
    errScatter: depth.errScatter,
    errFormal: depth.errFormal,
    err: depth.err,
    nIn: depth.nIn,
    nOut: depth.nOut,
    scatterPpm: scatter === null ? null : scatter.ppm,
    formalPpm: scatter === null ? null : scatter.meanFormalPpm,
    scatterRatio: scatter === null ? null : scatter.ratio,
    binned: binText(view.bin),
    nExcluded: counts.flagged + counts.manual,
    nClipped: counts.clipped,
    notes: model.notes,
  };
}

export function periodogramInput(model: LightCurveModel): { time: number[]; value: number[]; err: number[] | null } {
  const time: number[] = [];
  const value: number[] = [];
  const err: number[] = [];
  model.state.forEach((pointState, i) => {
    const drawn = model.y[i];
    if (pointState !== "kept" || !isFiniteNumber(drawn)) return;
    time.push(model.doc.time[i]);
    value.push(drawn);
    const error = model.yErr[i];
    if (isFiniteNumber(error) && error > 0) err.push(error);
  });
  return { time, value, err: err.length > 0 && err.length === value.length ? err : null };
}
