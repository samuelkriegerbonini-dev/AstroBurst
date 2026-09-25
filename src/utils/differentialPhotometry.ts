import type { TimeSeriesFrame, TimeSeriesResult, TimeSeriesRole, TimeSeriesTarget } from "../shared/types/analysis";

export type TimeAxis = "jd" | "bjd" | "index";

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
  centroidJump: boolean | null;
  errors: string | null;
}

export interface CheckReference {
  index: number;
  kind: "check" | "comp";
  curve: LightCurvePoint[];
}

export const MAG_ERR_FACTOR = 1.0857;
export const DEFAULT_CENTROID_JUMP_PX = 2;
export const MAX_TIME_SERIES_TARGETS = 64;
export const FRAME_JD_DIGITS = 5;
export const RELATIVE_JD_DIGITS = 4;
export const NO_TARGET_HINT = "Mark one star as the target.";
export const NO_COMP_HINT = "Mark at least one star as comp; the differential light curve divides the target by the comparison stars.";

const JD_TICK_MIN_DIGITS = 3;
const JD_READOUT_DIGITS = 6;
const JD_MAX_DIGITS = 12;
const STEP_LOG_TOLERANCE = 1e-6;

export const LIGHT_CURVE_CSV_COLUMNS = [
  "index",
  "file",
  "jd_mid",
  "time_source",
  "exptime",
  "filter",
  "airmass",
  "bjd_tdb",
  "hjd_utc",
  "airmass_computed",
  "altitude_deg",
  "parallactic_angle_deg",
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
  "centroid_jump",
  "skipped",
  "errors",
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

export function hasCompleteBjdAxis(result: TimeSeriesResult): boolean {
  const measured = result.frames.filter((f) => f.skipped === null);
  return measured.length > 0 && measured.every((f) => isFiniteNumber(f.geometry?.bjd_tdb));
}

export interface TimeAxisAvailability {
  jd: boolean;
  bjd: boolean;
  any: boolean;
}

export function timeAxisAvailability(result: TimeSeriesResult): TimeAxisAvailability {
  const jd = hasCompleteTimeAxis(result);
  const bjd = hasCompleteBjdAxis(result);
  return { jd, bjd, any: jd || bjd };
}

export function resolveTimeAxis(result: TimeSeriesResult, requested: TimeAxis): TimeAxis {
  if (requested === "bjd" && hasCompleteBjdAxis(result)) return "bjd";
  return requested !== "index" && hasCompleteTimeAxis(result) ? "jd" : "index";
}

function frameTime(frame: TimeSeriesFrame, axis: TimeAxis): number {
  if (axis === "index") return frame.index;
  const value = axis === "bjd" ? frame.geometry?.bjd_tdb : frame.jd_mid;
  return isFiniteNumber(value) ? value : NaN;
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

export function checkReference(result: TimeSeriesResult, compIdx: number[], checkIdx: number[], timeAxis: TimeAxis): CheckReference | null {
  if (checkIdx.length > 0) return { index: checkIdx[0], kind: "check", curve: lightCurve(result, checkIdx[0], compIdx, timeAxis) };
  if (compIdx.length >= 2) return { index: compIdx[0], kind: "comp", curve: checkStarCurve(result, compIdx[0], compIdx, timeAxis) };
  return null;
}

export function checkRmsLabel(reference: Pick<CheckReference, "index" | "kind"> | null, targets: TimeSeriesTarget[]): string {
  const label = reference ? targets[reference.index]?.label : undefined;
  if (!reference || label === undefined) return "Check-star rms";
  return reference.kind === "check" ? `Check-star rms (${label})` : `${label} vs other comps rms`;
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

function correctedCentroid(frame: TimeSeriesFrame | null, targetIdx: number): { x: number; y: number } | null {
  const star = frame?.targets[targetIdx] ?? null;
  if (!frame || !star || !isFiniteNumber(star.x) || !isFiniteNumber(star.y)) return null;
  const off = offsetOf(frame);
  return { x: star.x - off.dx, y: star.y - off.dy };
}

function driftKnown(frame: TimeSeriesFrame, referencePath: string): boolean {
  return frame.path === referencePath || (frame.offset !== null && frame.offset.registered);
}

function beyond(a: { x: number; y: number }, b: { x: number; y: number }, thresholdPx: number): boolean {
  const distance = Math.hypot(a.x - b.x, a.y - b.y);
  return Number.isFinite(distance) && distance > thresholdPx;
}

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = sorted.length >> 1;
  return sorted.length % 2 === 1 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

function strayFromCommonMotion(displacements: { x: number; y: number }[], thresholdPx: number): boolean {
  return displacements.some((own, k) => {
    const others = displacements.filter((_, j) => j !== k);
    if (others.length === 0) return false;
    const common = { x: median(others.map((d) => d.x)), y: median(others.map((d) => d.y)) };
    return beyond(own, common, thresholdPx);
  });
}

export function centroidJumps(result: TimeSeriesResult, thresholdPx = DEFAULT_CENTROID_JUMP_PX): number[] {
  const measured = result.frames.filter((f) => f.skipped === null);
  const reference = measured.find((f) => f.path === result.reference_path) ?? null;
  const anchors = new Map<number, { x: number; y: number }>();
  result.targets.forEach((_, i) => {
    const anchor = correctedCentroid(reference, i);
    if (anchor) anchors.set(i, anchor);
  });
  const flagged: number[] = [];
  for (const frame of measured) {
    const known = driftKnown(frame, result.reference_path);
    const displacements: { x: number; y: number }[] = [];
    let jump = false;
    for (let i = 0; i < frame.targets.length; i++) {
      const here = correctedCentroid(frame, i);
      if (!here) continue;
      const anchor = anchors.get(i);
      if (!anchor) {
        if (known) anchors.set(i, here);
      } else if (known) {
        jump = jump || beyond(here, anchor, thresholdPx);
      } else {
        displacements.push({ x: here.x - anchor.x, y: here.y - anchor.y });
      }
    }
    if (jump || strayFromCommonMotion(displacements, thresholdPx)) flagged.push(frame.index);
  }
  return flagged;
}

export function frameMeasurementErrors(
  frame: TimeSeriesFrame,
  targets: readonly TimeSeriesTarget[],
  targetIdx: number,
  compIdx: readonly number[],
): string[] {
  if (frame.skipped !== null) return [];
  const used = [targetIdx, ...compIdx.filter((i) => i !== targetIdx)].filter((i) => i >= 0);
  return used.flatMap((i) => {
    const error = frame.errors[i];
    return error ? [`${targets[i]?.label ?? `#${i}`}: ${error}`] : [];
  });
}

export function lightCurveExtras(result: TimeSeriesResult, targetIdx: number, compIdx: number[]): LightCurveExtra[] {
  const comps = compIdx.filter((i) => i !== targetIdx);
  const jumps = new Set(centroidJumps(result));
  return result.frames.map((frame) => {
    const target = frame.targets[targetIdx] ?? null;
    const ens = frame.skipped === null ? ensembleFlux(frame, comps) : null;
    const errors = frameMeasurementErrors(frame, result.targets, targetIdx, comps);
    const computedAirmass = frame.geometry?.airmass_computed;
    return {
      airmass: isFiniteNumber(frame.airmass) ? frame.airmass : isFiniteNumber(computedAirmass) ? computedAirmass : null,
      fwhm: target && isFiniteNumber(target.fwhm) ? target.fwhm : null,
      sky: target && isFiniteNumber(target.bg_mean) ? target.bg_mean : null,
      dx: frame.offset && isFiniteNumber(frame.offset.dx) ? frame.offset.dx : null,
      dy: frame.offset && isFiniteNumber(frame.offset.dy) ? frame.offset.dy : null,
      targetErr: target && isFiniteNumber(target.flux_err) ? target.flux_err : null,
      targetSnr: target && isFiniteNumber(target.snr) ? target.snr : null,
      ensembleFlux: ens ? ens.flux : null,
      registered: frame.offset ? frame.offset.registered : null,
      saturated: target ? target.saturated : null,
      centroidJump: frame.skipped === null ? jumps.has(frame.index) : null,
      errors: errors.length > 0 ? errors.join("; ") : null,
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
        csvCell(frame.geometry?.bjd_tdb ?? null, 6),
        csvCell(frame.geometry?.hjd_utc ?? null, 6),
        csvCell(frame.geometry?.airmass_computed ?? null, 4),
        csvCell(frame.geometry?.altitude_deg ?? null, 3),
        csvCell(frame.geometry?.parallactic_angle_deg ?? null, 2),
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
        csvCell(e?.centroidJump ?? null),
        csvCell(frame.skipped),
        csvCell(e?.errors ?? null),
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

export function resultCoversPath(result: Pick<TimeSeriesResult, "frames"> | null, path: string | null): boolean {
  return !!result && !!path && result.frames.some((f) => f.path === path);
}

export function measuredTargets(targets: TimeSeriesTarget[]): TimeSeriesTarget[] {
  return targets.filter((t) => t.role !== "ignore");
}

export function timeSeriesRoleHint(roles: readonly TimeSeriesRole[]): string | null {
  if (!roles.includes("target")) return NO_TARGET_HINT;
  if (!roles.includes("comp")) return NO_COMP_HINT;
  return null;
}

export function inFrameOrder(result: TimeSeriesResult, framePaths: string[]): TimeSeriesResult {
  const rank = new Map<string, number>();
  framePaths.forEach((p, i) => {
    if (!rank.has(p)) rank.set(p, i);
  });
  const last = framePaths.length;
  const frames = [...result.frames]
    .sort((a, b) => (rank.get(a.path) ?? last) - (rank.get(b.path) ?? last))
    .map((frame, index) => ({ ...frame, index }));
  return { ...result, frames };
}

export function frameJdLabel(value: number | null | undefined, axis: TimeAxis, jd0: number): string {
  if (!isFiniteNumber(value)) return "--";
  return axis !== "index" ? (value - jd0).toFixed(RELATIVE_JD_DIGITS) : value.toFixed(FRAME_JD_DIGITS);
}

export function jdOffsetLabel(value: number, step?: number): string {
  const digits =
    step !== undefined && Number.isFinite(step) && step > 0
      ? Math.min(JD_MAX_DIGITS, Math.max(JD_TICK_MIN_DIGITS, Math.ceil(-Math.log10(step) - STEP_LOG_TOLERANCE)))
      : JD_READOUT_DIGITS;
  return value.toFixed(digits);
}

export function timeAxisLabel(axis: TimeAxis, jd0: number): string {
  if (axis === "bjd") return `BJD_TDB - ${jd0}`;
  return axis === "jd" ? `JD (header time, not barycentric) - ${jd0}` : "frame index";
}

export function referenceTimeSource(result: TimeSeriesResult): string | null {
  return result.frames.find((f) => f.path === result.reference_path)?.time_source ?? null;
}

export function frameFilesNotice(input: { compositeOnScreen: boolean; processedLabel: string | null }): string | null {
  if (input.compositeOnScreen) return "Measured on the loaded frame files, not on the RGB view on screen";
  if (input.processedLabel) return `Measured on the loaded frame files; ${input.processedLabel} on screen is not used`;
  return null;
}
