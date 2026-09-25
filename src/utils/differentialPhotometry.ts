import type { TimeSeriesFrame, TimeSeriesResult } from "../shared/types/analysis";

export type TimeAxis = "jd" | "index";

export interface LightCurvePoint {
  frameIndex: number;
  t: number;
  mag: number | null;
  err: number | null;
  flux: number | null;
  skipped: boolean;
}

export interface EnsembleFlux {
  flux: number;
  err: number;
}

export interface LightCurveExtra {
  airmass: number | null;
  fwhm: number | null;
  sky: number | null;
  dx: number | null;
  dy: number | null;
  targetErr: number | null;
  targetSnr: number | null;
  ensembleFlux: number | null;
  registered: boolean | null;
  saturated: boolean | null;
}

export const MAG_ERR_FACTOR = 1.0857;
export const DEFAULT_CENTROID_JUMP_PX = 2;

export const LIGHT_CURVE_CSV_COLUMNS = [
  "index",
  "file",
  "jd_mid",
  "time_source",
  "exptime",
  "filter",
  "airmass",
  "diff_mag",
  "diff_err",
  "target_flux",
  "target_err",
  "target_snr",
  "ensemble_flux",
  "fwhm",
  "bg_mean",
  "dx",
  "dy",
  "registered",
  "saturated",
  "skipped",
] as const;

function isFiniteNumber(v: number | null | undefined): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

export function ensembleFlux(frame: TimeSeriesFrame, compIdx: number[]): EnsembleFlux | null {
  if (compIdx.length === 0) return null;
  let flux = 0;
  let variance = 0;
  for (const idx of compIdx) {
    const comp = frame.targets[idx];
    if (!comp || !isFiniteNumber(comp.net_flux) || comp.net_flux <= 0) return null;
    flux += comp.net_flux;
    variance += isFiniteNumber(comp.flux_err) ? comp.flux_err * comp.flux_err : 0;
  }
  return { flux, err: Math.sqrt(variance) };
}

export function differentialMag(
  target: { net_flux: number; flux_err: number },
  ens: { flux: number; err: number },
): { mag: number; err: number } | null {
  if (!isFiniteNumber(target.net_flux) || target.net_flux <= 0) return null;
  if (!isFiniteNumber(ens.flux) || ens.flux <= 0) return null;
  const mag = -2.5 * Math.log10(target.net_flux / ens.flux);
  const targetErr = isFiniteNumber(target.flux_err) ? target.flux_err : 0;
  const ensErr = isFiniteNumber(ens.err) ? ens.err : 0;
  const err = MAG_ERR_FACTOR * Math.sqrt((targetErr / target.net_flux) ** 2 + (ensErr / ens.flux) ** 2);
  return Number.isFinite(mag) && Number.isFinite(err) ? { mag, err } : null;
}

export function hasCompleteTimeAxis(result: TimeSeriesResult): boolean {
  const measured = result.frames.filter((f) => f.skipped === null);
  return measured.length > 0 && measured.every((f) => isFiniteNumber(f.jd_mid));
}

export function resolveTimeAxis(result: TimeSeriesResult, requested: TimeAxis): TimeAxis {
  return requested === "jd" && hasCompleteTimeAxis(result) ? "jd" : "index";
}

function frameTime(frame: TimeSeriesFrame, axis: TimeAxis): number {
  if (axis === "index") return frame.index;
  return isFiniteNumber(frame.jd_mid) ? frame.jd_mid : NaN;
}

export interface TimelinePoint {
  frameIndex: number;
  t: number;
  skipped: boolean;
}

export function frameTimes(result: TimeSeriesResult, timeAxis: TimeAxis): TimelinePoint[] {
  const axis = resolveTimeAxis(result, timeAxis);
  return result.frames.map((frame) => ({ frameIndex: frame.index, t: frameTime(frame, axis), skipped: frame.skipped !== null }));
}

export function lightCurve(
  result: TimeSeriesResult,
  targetIdx: number,
  compIdx: number[],
  timeAxis: TimeAxis,
): LightCurvePoint[] {
  const axis = resolveTimeAxis(result, timeAxis);
  const comps = compIdx.filter((i) => i !== targetIdx);
  return result.frames.map((frame) => {
    const skipped = frame.skipped !== null;
    const target = frame.targets[targetIdx] ?? null;
    const flux = target && isFiniteNumber(target.net_flux) ? target.net_flux : null;
    const ens = skipped || !target ? null : ensembleFlux(frame, comps);
    const diff = target && ens ? differentialMag(target, ens) : null;
    return {
      frameIndex: frame.index,
      t: frameTime(frame, axis),
      mag: diff ? diff.mag : null,
      err: diff ? diff.err : null,
      flux,
      skipped,
    };
  });
}

export function checkStarCurve(
  result: TimeSeriesResult,
  checkIdx: number,
  compIdx: number[],
  timeAxis: TimeAxis = "jd",
): LightCurvePoint[] {
  return lightCurve(
    result,
    checkIdx,
    compIdx.filter((i) => i !== checkIdx),
    timeAxis,
  );
}

export function seriesRms(points: LightCurvePoint[]): number | null {
  const mags = points.map((p) => p.mag).filter(isFiniteNumber);
  if (mags.length < 2) return null;
  const mean = mags.reduce((a, b) => a + b, 0) / mags.length;
  const variance = mags.reduce((a, m) => a + (m - mean) ** 2, 0) / mags.length;
  return Math.sqrt(variance);
}

export function jdZero(points: TimelinePoint[]): number {
  const times = points.filter((p) => !p.skipped).map((p) => p.t).filter(isFiniteNumber);
  return times.length > 0 ? Math.floor(Math.min(...times)) : 0;
}

function offsetOf(frame: TimeSeriesFrame): { dx: number; dy: number } {
  const o = frame.offset;
  if (o && o.registered && isFiniteNumber(o.dx) && isFiniteNumber(o.dy)) return { dx: o.dx, dy: o.dy };
  return { dx: 0, dy: 0 };
}

export function centroidJumps(result: TimeSeriesResult, thresholdPx = DEFAULT_CENTROID_JUMP_PX): number[] {
  const flagged = new Set<number>();
  const measured = result.frames.filter((f) => f.skipped === null);
  for (let k = 1; k < measured.length; k++) {
    const prev = measured[k - 1];
    const cur = measured[k];
    const prevOff = offsetOf(prev);
    const curOff = offsetOf(cur);
    const n = Math.min(prev.targets.length, cur.targets.length);
    for (let i = 0; i < n; i++) {
      const a = prev.targets[i];
      const b = cur.targets[i];
      if (!a || !b) continue;
      const dx = b.x - curOff.dx - (a.x - prevOff.dx);
      const dy = b.y - curOff.dy - (a.y - prevOff.dy);
      const distance = Math.hypot(dx, dy);
      if (Number.isFinite(distance) && distance > thresholdPx) {
        flagged.add(cur.index);
        break;
      }
    }
  }
  return [...flagged].sort((a, b) => a - b);
}

export function lightCurveExtras(result: TimeSeriesResult, targetIdx: number, compIdx: number[]): LightCurveExtra[] {
  const comps = compIdx.filter((i) => i !== targetIdx);
  return result.frames.map((frame) => {
    const target = frame.targets[targetIdx] ?? null;
    const ens = frame.skipped === null ? ensembleFlux(frame, comps) : null;
    return {
      airmass: isFiniteNumber(frame.airmass) ? frame.airmass : null,
      fwhm: target && isFiniteNumber(target.fwhm) ? target.fwhm : null,
      sky: target && isFiniteNumber(target.bg_mean) ? target.bg_mean : null,
      dx: frame.offset && isFiniteNumber(frame.offset.dx) ? frame.offset.dx : null,
      dy: frame.offset && isFiniteNumber(frame.offset.dy) ? frame.offset.dy : null,
      targetErr: target && isFiniteNumber(target.flux_err) ? target.flux_err : null,
      targetSnr: target && isFiniteNumber(target.snr) ? target.snr : null,
      ensembleFlux: ens ? ens.flux : null,
      registered: frame.offset ? frame.offset.registered : null,
      saturated: target ? target.saturated : null,
    };
  });
}

function csvCell(v: number | string | boolean | null | undefined, digits?: number): string {
  if (v === null || v === undefined) return "";
  if (typeof v === "boolean") return v ? "true" : "false";
  if (typeof v === "number") {
    if (!Number.isFinite(v)) return "";
    return digits === undefined ? String(v) : v.toFixed(digits);
  }
  return /[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
}

export function lightCurveCsv(result: TimeSeriesResult, rows: LightCurvePoint[], extra: LightCurveExtra[]): string {
  const lines = [LIGHT_CURVE_CSV_COLUMNS.join(",")];
  rows.forEach((row, k) => {
    const frame = result.frames[row.frameIndex];
    if (!frame) return;
    const e = extra[k];
    lines.push(
      [
        csvCell(frame.index),
        csvCell(frame.file_name),
        csvCell(frame.jd_mid, 6),
        csvCell(frame.time_source),
        csvCell(frame.exptime),
        csvCell(frame.filter),
        csvCell(frame.airmass, 4),
        csvCell(row.mag, 5),
        csvCell(row.err, 5),
        csvCell(row.flux, 3),
        csvCell(e?.targetErr ?? null, 3),
        csvCell(e?.targetSnr ?? null, 2),
        csvCell(e?.ensembleFlux ?? null, 3),
        csvCell(e?.fwhm ?? null, 3),
        csvCell(e?.sky ?? null, 3),
        csvCell(e?.dx ?? null, 3),
        csvCell(e?.dy ?? null, 3),
        csvCell(e?.registered ?? null),
        csvCell(e?.saturated ?? null),
        csvCell(frame.skipped),
      ].join(","),
    );
  });
  return lines.join("\n") + "\n";
}

export function lightCurveCsvFileName(referencePath: string): string {
  const base = referencePath.split(/[\\/]/).pop() ?? "frames";
  const stem = base.replace(/#.*$/, "").replace(/\.[^.]+$/, "") || "frames";
  return `${stem}_lightcurve.csv`;
}
