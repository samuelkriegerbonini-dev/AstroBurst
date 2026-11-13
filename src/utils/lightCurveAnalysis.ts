import { MAD_TO_SIGMA } from "./statisticsUnits";

export const QUICK_LOOK_LABEL = "quick look, no limb darkening or systematics model";
export const MAG_ERR_EXACT = 2.5 / Math.LN10;

export interface TimeWindow {
  start: number;
  end: number;
}

export type BaselineKind = "mean" | "median" | "linear";

export interface Baseline {
  kind: BaselineKind;
  level: number;
  slope: number;
  t0: number;
  nOut: number;
}

export interface NullableCurve {
  flux: (number | null)[];
  err: (number | null)[];
}

export type BinBy = { widthDays: number } | { points: number };

export interface BinnedCurve {
  t: number[];
  flux: number[];
  err: (number | null)[];
  n: number[];
  weighted: boolean;
}

export interface ScatterSummary {
  ppm: number;
  mmag: number;
  meanFormalPpm: number | null;
  ratio: number | null;
  n: number;
}

export interface BoxDepth {
  depth: number;
  errScatter: number | null;
  errFormal: number | null;
  err: number | null;
  nIn: number;
  nOut: number;
  fIn: number;
  fOut: number;
}

interface Point {
  index: number;
  t: number;
  flux: number;
}

function isFiniteNumber(v: number | null | undefined): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

function isPositive(v: number): boolean {
  return v > 0;
}

function isNotNegative(v: number): boolean {
  return v >= 0;
}

function sum(values: readonly number[]): number {
  let total = 0;
  for (const v of values) total += v;
  return total;
}

function mean(values: readonly number[]): number {
  return sum(values) / values.length;
}

function median(values: readonly number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

function sampleStd(values: readonly number[], centre: number): number {
  return Math.sqrt(sum(values.map((v) => (v - centre) ** 2)) / (values.length - 1));
}

function scatterErrorOfMean(values: readonly number[], centre: number): number | null {
  return values.length < 2 ? null : sampleStd(values, centre) / Math.sqrt(values.length);
}

function byTime(a: Point, b: Point): number {
  return a.t - b.t;
}

function fluxesOf(points: readonly Point[]): number[] {
  return points.map((p) => p.flux);
}

function usablePoints(t: readonly number[], flux: readonly (number | null)[]): Point[] {
  const points: Point[] = [];
  for (let index = 0; index < flux.length; index++) {
    const ti = t[index];
    const fi = flux[index];
    if (isFiniteNumber(ti) && isFiniteNumber(fi)) points.push({ index, t: ti, flux: fi });
  }
  return points;
}

function isInside(t: number, window: TimeWindow): boolean {
  return window.start <= t && t <= window.end;
}

function outOfTransitPoints(
  t: readonly number[],
  flux: readonly (number | null)[],
  window: TimeWindow | null,
  baseline: readonly TimeWindow[] | undefined,
): Point[] {
  const restricted = baseline !== undefined && baseline.length > 0;
  return usablePoints(t, flux).filter(
    (p) => (window === null || !isInside(p.t, window)) && (!restricted || baseline.some((b) => isInside(p.t, b))),
  );
}

function errorsWhenEvery(
  points: readonly Point[],
  err: readonly (number | null)[],
  accepts: (e: number) => boolean,
): number[] | null {
  const errors: number[] = [];
  for (const p of points) {
    const e = err[p.index];
    if (!isFiniteNumber(e) || !accepts(e)) return null;
    errors.push(e);
  }
  return errors;
}

function formalErrorOfMean(points: readonly Point[], err: readonly (number | null)[]): number | null {
  const errors = errorsWhenEvery(points, err, isNotNegative);
  return errors === null ? null : Math.sqrt(sum(errors.map((e) => e * e))) / errors.length;
}

function baselineModel(baseline: Baseline, t: number): number {
  return baseline.level + baseline.slope * (t - baseline.t0);
}

export function relativeFlux(
  mag: readonly (number | null)[],
  err: readonly (number | null)[],
  magErrFactor: number = MAG_ERR_EXACT,
): NullableCurve {
  const flux: (number | null)[] = [];
  const fluxErr: (number | null)[] = [];
  for (let i = 0; i < mag.length; i++) {
    const m = mag[i];
    const e = err[i];
    if (!isFiniteNumber(m)) {
      flux.push(null);
      fluxErr.push(null);
      continue;
    }
    const f = 10 ** (-0.4 * m);
    flux.push(f);
    fluxErr.push(isFiniteNumber(e) ? (f * e) / magErrFactor : null);
  }
  return { flux, err: fluxErr };
}

export function fitBaseline(
  t: readonly number[],
  flux: readonly (number | null)[],
  window: TimeWindow | null,
  kind: BaselineKind,
  baseline?: readonly TimeWindow[],
): Baseline | null {
  const points = outOfTransitPoints(t, flux, window, baseline);
  const nOut = points.length;
  if (nOut === 0) return null;
  const fluxes = fluxesOf(points);
  const t0 = mean(points.map((p) => p.t));
  if (kind === "median") return { kind, level: median(fluxes), slope: 0, t0, nOut };
  const level = mean(fluxes);
  if (kind === "mean") return { kind, level, slope: 0, t0, nOut };
  if (nOut < 2 || points.every((p) => p.t === points[0].t)) return null;
  const covariance = sum(points.map((p) => (p.t - t0) * (p.flux - level)));
  const spread = sum(points.map((p) => (p.t - t0) ** 2));
  return { kind, level, slope: covariance / spread, t0, nOut };
}

export function normalise(
  t: readonly number[],
  flux: readonly (number | null)[],
  err: readonly (number | null)[],
  baseline: Baseline,
): NullableCurve {
  const normFlux: (number | null)[] = [];
  const normErr: (number | null)[] = [];
  for (let i = 0; i < flux.length; i++) {
    const f = flux[i];
    const e = err[i];
    const model = baselineModel(baseline, t[i]);
    if (!isFiniteNumber(f) || !Number.isFinite(model) || !(model > 0)) {
      normFlux.push(null);
      normErr.push(null);
      continue;
    }
    normFlux.push(f / model);
    normErr.push(isFiniteNumber(e) ? e / model : null);
  }
  return { flux: normFlux, err: normErr };
}

function groupsOf(points: readonly Point[], by: BinBy): Point[][] {
  const groups: Point[][] = [];
  if ("points" in by) {
    const size = by.points >= 1 ? Math.floor(by.points) : 1;
    for (let i = 0; i < points.length; i += size) groups.push(points.slice(i, i + size));
    return groups;
  }
  if (!(by.widthDays > 0)) return groups;
  let current = NaN;
  for (const p of points) {
    const slot = Math.floor((p.t - points[0].t) / by.widthDays);
    if (slot !== current) {
      groups.push([]);
      current = slot;
    }
    groups[groups.length - 1].push(p);
  }
  return groups;
}

export function binInTime(
  t: readonly number[],
  flux: readonly (number | null)[],
  err: readonly (number | null)[],
  by: BinBy,
): BinnedCurve {
  const points = usablePoints(t, flux).sort(byTime);
  const weighted = points.length > 0 && errorsWhenEvery(points, err, isPositive) !== null;
  const binned: BinnedCurve = { t: [], flux: [], err: [], n: [], weighted };
  for (const group of groupsOf(points, by)) {
    const fluxes = fluxesOf(group);
    binned.t.push(mean(group.map((p) => p.t)));
    binned.n.push(group.length);
    const errors = weighted ? errorsWhenEvery(group, err, isPositive) : null;
    if (errors === null) {
      const centre = mean(fluxes);
      binned.flux.push(centre);
      binned.err.push(scatterErrorOfMean(fluxes, centre));
      continue;
    }
    const weights = errors.map((e) => 1 / (e * e));
    const totalWeight = sum(weights);
    binned.flux.push(sum(weights.map((w, i) => w * fluxes[i])) / totalWeight);
    binned.err.push(1 / Math.sqrt(totalWeight));
  }
  return binned;
}

export function sigmaClip(
  t: readonly number[],
  flux: readonly (number | null)[],
  sigma: number,
  windowPoints: number,
): boolean[] {
  const clipped = flux.map(() => false);
  const points = usablePoints(t, flux).sort(byTime);
  if (points.length < 3 || !(sigma > 0)) return clipped;
  const half = Math.floor((windowPoints > 3 ? windowPoints : 3) / 2);
  const fluxes = fluxesOf(points);
  const residuals = fluxes.map((f, k) => f - median(fluxes.slice(Math.max(0, k - half), k + half + 1)));
  const centre = median(residuals);
  const deviations = residuals.map((r) => Math.abs(r - centre));
  const scale = MAD_TO_SIGMA * median(deviations);
  if (!(scale > 0)) return clipped;
  points.forEach((p, k) => {
    clipped[p.index] = deviations[k] > sigma * scale;
  });
  return clipped;
}

export function outOfTransitScatter(
  t: readonly number[],
  flux: readonly (number | null)[],
  err: readonly (number | null)[],
  window: TimeWindow | null,
  baseline?: readonly TimeWindow[],
): ScatterSummary | null {
  const points = outOfTransitPoints(t, flux, window, baseline);
  const n = points.length;
  if (n < 2) return null;
  const fluxes = fluxesOf(points);
  const centre = mean(fluxes);
  if (centre === 0) return null;
  const rel = sampleStd(fluxes, centre) / Math.abs(centre);
  const ppm = 1e6 * rel;
  const mmag = 1000 * MAG_ERR_EXACT * rel;
  const errors = errorsWhenEvery(points, err, isPositive);
  if (errors === null) return { ppm, mmag, meanFormalPpm: null, ratio: null, n };
  const meanFormalPpm = (1e6 * mean(errors)) / Math.abs(centre);
  return { ppm, mmag, meanFormalPpm, ratio: ppm / meanFormalPpm, n };
}

export function boxDepth(
  t: readonly number[],
  flux: readonly (number | null)[],
  err: readonly (number | null)[],
  window: TimeWindow,
  baseline?: readonly TimeWindow[],
): BoxDepth | null {
  const inside = usablePoints(t, flux).filter((p) => isInside(p.t, window));
  const outside = outOfTransitPoints(t, flux, window, baseline);
  if (inside.length === 0 || outside.length === 0) return null;
  const insideFluxes = fluxesOf(inside);
  const outsideFluxes = fluxesOf(outside);
  const fIn = mean(insideFluxes);
  const fOut = mean(outsideFluxes);
  if (fOut === 0 || !Number.isFinite(fOut)) return null;
  const ratio = fIn / fOut;
  const depthError = (sIn: number | null, sOut: number | null): number | null =>
    sIn === null || sOut === null ? null : Math.sqrt(sIn ** 2 + ratio ** 2 * sOut ** 2) / Math.abs(fOut);
  const errScatter = depthError(scatterErrorOfMean(insideFluxes, fIn), scatterErrorOfMean(outsideFluxes, fOut));
  const errFormal = depthError(formalErrorOfMean(inside, err), formalErrorOfMean(outside, err));
  const larger = errScatter !== null && errFormal !== null ? Math.max(errScatter, errFormal) : (errScatter ?? errFormal);
  return {
    depth: 1 - ratio,
    errScatter,
    errFormal,
    err: larger,
    nIn: inside.length,
    nOut: outside.length,
    fIn,
    fOut,
  };
}

export function foldPhase(t: readonly number[], t0: number, periodDays: number): number[] {
  const foldable = Number.isFinite(t0) && Number.isFinite(periodDays) && periodDays > 0;
  return t.map((ti) => {
    if (!foldable || !isFiniteNumber(ti)) return NaN;
    const x = (ti - t0) / periodDays;
    const phase = x - Math.floor(x);
    return phase < 1 ? phase : 0;
  });
}
