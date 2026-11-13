import type { TimeSeriesFrame, TimeSeriesResult } from "../shared/types/analysis";
import type { LightCurveDoc, LightCurveQuantity, LightCurveQuickLook, LightCurveSource, LightCurveTimeScale } from "../shared/types/lightCurve";
import {
  centroidJumps,
  frameMeasurementErrors,
  frameTimes,
  lightCurve,
  referenceTimeSource,
  resolveTimeAxis,
  timeAxisLabel,
  MAG_ERR_FACTOR,
  type TimeAxis,
} from "./differentialPhotometry";
import { QUICK_LOOK_LABEL } from "./lightCurveAnalysis";
import { periodStep } from "./lightCurveInputs";
import type { MeasurementLogDraft } from "./measurementLog";
import type { PeriodogramResult } from "../shared/types/periodogram";
import { fileBaseName, finiteOrNull, isFiniteNumber } from "./values";

export const BMJD_OFFSET = 2400000.5;
export const FLAG_SKIPPED = "skipped";
export const FLAG_SATURATED = "saturated";
export const FLAG_COMP_SATURATED = "comp saturated";
export const FLAG_CENTROID_JUMP = "centroid jump";
export const FLAG_LOW_CONFIDENCE_DRIFT = "low-confidence drift";
export const FLAG_MEASUREMENT_ERROR = "measurement error";
export const EXCLUDABLE_FLAGS: readonly string[] = [FLAG_SATURATED, FLAG_COMP_SATURATED, FLAG_CENTROID_JUMP];

export const TIME_SCALE_NAMES: Readonly<Record<LightCurveTimeScale, string>> = {
  bmjd_tdb: "BMJD (TDB)",
  bjd_tdb: "BJD_TDB",
  hjd_utc: "HJD (UTC)",
  jd_utc: "JD (UTC)",
  jd_header: "JD (header time, not barycentric)",
  mjd_utc: "MJD (UTC)",
  index: "frame index",
};

const FRAME_TIME_SCALES: Readonly<Record<TimeAxis, LightCurveTimeScale>> = { bjd: "bjd_tdb", jd: "jd_header", index: "index" };
const ZERO_OFFSET_SUFFIX = " - 0";

export function frameGroupKey(referencePath: string): string {
  return `frames:${referencePath}`;
}

export function tableGroupKey(path: string): string {
  return `table:${path}`;
}

function offsetLabel(name: string, scale: LightCurveTimeScale, offset: number): string {
  return scale === "index" || offset === 0 ? name : `${name} - ${offset}`;
}

export function timeScaleLabel(scale: LightCurveTimeScale, offset: number): string {
  return offsetLabel(TIME_SCALE_NAMES[scale], scale, offset);
}

export function timeOffset(doc: Pick<LightCurveDoc, "timeScale" | "time" | "flags">): number {
  if (doc.timeScale === "index") return 0;
  let earliest = Infinity;
  doc.time.forEach((t, i) => {
    if (Number.isFinite(t) && t < earliest && !doc.flags[i]?.includes(FLAG_SKIPPED)) earliest = t;
  });
  return Number.isFinite(earliest) ? Math.floor(earliest) : 0;
}

export function docTimeLabel(doc: Pick<LightCurveDoc, "timeScale" | "timeLabel">, offset: number): string {
  return offsetLabel(doc.timeLabel ?? TIME_SCALE_NAMES[doc.timeScale], doc.timeScale, offset);
}

export function convertBarycentric(doc: LightCurveDoc, to: "bmjd_tdb" | "bjd_tdb"): LightCurveDoc {
  if (doc.timeScale === to || (doc.timeScale !== "bmjd_tdb" && doc.timeScale !== "bjd_tdb")) return doc;
  const shift = to === "bjd_tdb" ? BMJD_OFFSET : -BMJD_OFFSET;
  return { ...doc, timeScale: to, time: doc.time.map((t) => t + shift) };
}

export interface FrameDocOptions {
  id?: string;
  label?: string;
  overlay?: string;
}

function frameTimeAxis(result: TimeSeriesResult, axis: TimeAxis): Pick<LightCurveDoc, "timeScale" | "timeLabel"> {
  const resolved = resolveTimeAxis(result, axis);
  const label = timeAxisLabel(resolved, 0, result.frames);
  return {
    timeScale: FRAME_TIME_SCALES[resolved],
    timeLabel: label.endsWith(ZERO_OFFSET_SUFFIX) ? label.slice(0, -ZERO_OFFSET_SUFFIX.length) : label,
  };
}

function starLabel(result: TimeSeriesResult, starIdx: number): string {
  return result.targets[starIdx]?.label ?? `#${starIdx}`;
}

function raisedFlags(checks: readonly (readonly [string, boolean])[]): string[] {
  return checks.flatMap(([flag, raised]) => (raised ? [flag] : []));
}

function isSaturated(frame: TimeSeriesFrame, starIdx: number): boolean {
  return frame.targets[starIdx]?.saturated === true;
}

function driftIsUncertain(frame: TimeSeriesFrame): boolean {
  return frame.offset !== null && !frame.offset.registered;
}

function overlayOf(options: FrameDocOptions): Pick<LightCurveDoc, "overlay"> {
  return options.overlay === undefined ? {} : { overlay: options.overlay };
}

export function lightCurveDocFromFrames(
  result: TimeSeriesResult,
  targetIdx: number,
  compIdx: number[],
  axis: TimeAxis,
  options: FrameDocOptions = {},
): LightCurveDoc {
  const comps = compIdx.filter((i) => i !== targetIdx);
  const curve = lightCurve(result, targetIdx, compIdx, axis);
  const jumps = new Set(centroidJumps(result));
  const target = starLabel(result, targetIdx);
  const compLabels = comps.map((i) => starLabel(result, i));
  const timeSource = referenceTimeSource(result);
  return {
    id: options.id ?? "target",
    label: options.label ?? `target ${target}`,
    quantity: "diff_mag",
    unit: "mag",
    ...frameTimeAxis(result, axis),
    time: curve.map((p) => p.t),
    value: curve.map((p) => p.mag),
    err: curve.map((p) => p.err),
    pointRef: curve.map((p) => p.frameIndex),
    flags: result.frames.map((frame) =>
      raisedFlags([
        [FLAG_SKIPPED, frame.skipped !== null],
        [FLAG_SATURATED, isSaturated(frame, targetIdx)],
        [FLAG_COMP_SATURATED, comps.some((i) => isSaturated(frame, i))],
        [FLAG_CENTROID_JUMP, jumps.has(frame.index)],
        [FLAG_LOW_CONFIDENCE_DRIFT, driftIsUncertain(frame)],
        [FLAG_MEASUREMENT_ERROR, frameMeasurementErrors(frame, result.targets, targetIdx, comps).length > 0],
      ]),
    ),
    source: { kind: "frames", reference: fileBaseName(result.reference_path), target, comps: compLabels },
    notes: [
      `differential magnitude: ${target} minus the sum of ${comps.length} comparison ${comps.length === 1 ? "star" : "stars"} (${compLabels.join(", ")})`,
      ...(timeSource === null ? [] : [`time: ${timeSource}`]),
    ],
    ...overlayOf(options),
    magErrFactor: MAG_ERR_FACTOR,
  };
}

export function rawFluxDocFromFrames(result: TimeSeriesResult, starIdx: number, axis: TimeAxis, options: FrameDocOptions = {}): LightCurveDoc {
  const timeline = frameTimes(result, axis);
  const jumps = new Set(centroidJumps(result));
  const label = starLabel(result, starIdx);
  return {
    id: options.id ?? `raw:${starIdx}`,
    label: options.label ?? label,
    quantity: "flux",
    unit: null,
    ...frameTimeAxis(result, axis),
    time: timeline.map((p) => p.t),
    value: result.frames.map((frame) => finiteOrNull(frame.targets[starIdx]?.net_flux)),
    err: result.frames.map((frame) => finiteOrNull(frame.targets[starIdx]?.flux_err)),
    pointRef: timeline.map((p) => p.frameIndex),
    flags: result.frames.map((frame) =>
      raisedFlags([
        [FLAG_SKIPPED, frame.skipped !== null],
        [FLAG_SATURATED, isSaturated(frame, starIdx)],
        [FLAG_CENTROID_JUMP, jumps.has(frame.index)],
        [FLAG_LOW_CONFIDENCE_DRIFT, driftIsUncertain(frame)],
      ]),
    ),
    source: { kind: "frames", reference: fileBaseName(result.reference_path), target: label, comps: [] },
    notes: [`net aperture flux of ${label} in the image unit summed over the aperture`],
    ...overlayOf(options),
  };
}

export interface LightCurveTableColumn {
  name: string;
  unit: string | null;
  values: (number | null)[] | null;
}

export interface LightCurveTableInput {
  columns: readonly LightCurveTableColumn[];
}

export interface LightCurveTableMapping {
  path: string;
  timeColumn: string;
  timeScale: LightCurveTimeScale;
  valueColumn: string;
  quantity: LightCurveQuantity;
  errColumn?: string | null;
  label?: string;
}

interface NumericColumn {
  name: string;
  unit: string | null;
  values: (number | null)[];
}

function numericColumn(table: LightCurveTableInput, name: string): NumericColumn {
  const lower = name.toLowerCase();
  const column = table.columns.find((c) => c.name === name) ?? table.columns.find((c) => c.name.toLowerCase() === lower);
  if (!column) throw new Error(`column '${name}' is not in the table (columns: ${table.columns.map((c) => c.name).join(", ")})`);
  if (column.values === null) throw new Error(`column '${name}' holds text, not numbers`);
  return { name: column.name, unit: column.unit, values: column.values };
}

export function lightCurveDocFromTable(table: LightCurveTableInput, mapping: LightCurveTableMapping): LightCurveDoc {
  const time = numericColumn(table, mapping.timeColumn);
  const value = numericColumn(table, mapping.valueColumn);
  const err = mapping.errColumn ? numericColumn(table, mapping.errColumn) : null;
  const rows = Math.max(time.values.length, value.values.length);
  const kept = Array.from({ length: rows }, (_, row) => row).flatMap((row) => {
    const t = time.values[row];
    const v = value.values[row];
    return isFiniteNumber(t) && isFiniteNumber(v) ? [{ row, t, v }] : [];
  });
  return {
    id: value.name,
    label: mapping.label ?? `${value.name} (${fileBaseName(mapping.path)})`,
    quantity: mapping.quantity,
    unit: value.unit,
    timeScale: mapping.timeScale,
    time: kept.map((point) => point.t),
    value: kept.map((point) => point.v),
    err: kept.map((point) => finiteOrNull(err?.values[point.row])),
    pointRef: kept.map((point) => point.row),
    flags: kept.map(() => []),
    source: { kind: "table", path: mapping.path, time_column: time.name, value_column: value.name },
    notes: [
      ...(kept.length < rows ? [`${kept.length} of ${rows} rows kept (${rows - kept.length} without a finite time or value)`] : []),
      ...(err ? [`errors from column ${err.name}`] : []),
    ],
  };
}

function quickLookFile(source: LightCurveSource): string {
  if (source.kind === "frames") return source.reference;
  return fileBaseName(source.kind === "table" ? source.path : (source.paths[0] ?? ""));
}

export function quickLookEntry(q: LightCurveQuickLook): MeasurementLogDraft {
  return {
    kind: "time_series",
    file: quickLookFile(q.source),
    image: `${q.nPoints} points`,
    dq: "not_applied",
    unit: "fraction",
    source: JSON.stringify({ kind: "light_curve", doc: q.label, from: q.source }),
    params: {
      series: q.label,
      quantity: q.quantity,
      time_scale: q.timeName,
      window_start: q.window.start,
      window_end: q.window.end,
      baseline_margin_days: q.marginDays,
      baseline_kind: q.baselineKind,
      binned: q.binned,
      n_excluded: q.nExcluded,
      n_clipped: q.nClipped,
    },
    values: {
      box_depth: q.depth,
      box_depth_err: q.err,
      box_depth_err_scatter: q.errScatter,
      box_depth_err_formal: q.errFormal,
      n_in: q.nIn,
      n_out: q.nOut,
      oot_scatter_ppm: q.scatterPpm,
      oot_formal_ppm: q.formalPpm,
      oot_scatter_ratio: q.scatterRatio,
    },
    notes: [
      QUICK_LOOK_LABEL,
      "depth D = 1 - F_in / F_out; the error is the larger of the scatter and the formal estimate",
      `times in days, ${q.timeName}`,
      ...q.notes,
    ],
  };
}

export interface PeriodogramLogInput {
  doc: LightCurveDoc;
  valueName: string;
  result: PeriodogramResult;
  minFrequency: number | null;
  maxFrequency: number | null;
  fold: { periodDays: number; t0: number } | null;
}

export function periodogramEntry(input: PeriodogramLogInput): MeasurementLogDraft {
  const { doc, result, fold } = input;
  return {
    kind: "periodogram",
    file: quickLookFile(doc.source),
    image: `${result.n_points} points`,
    dq: "not_applied",
    unit: "d",
    source: JSON.stringify({ kind: "light_curve", doc: doc.label, from: doc.source }),
    params: {
      series: doc.label,
      quantity: input.valueName,
      time_scale: docTimeLabel(doc, 0),
      min_frequency_per_day: input.minFrequency,
      max_frequency_per_day: input.maxFrequency,
      weighted: result.weighted,
      normalization: result.normalization,
      fap_method: result.fap_method,
      fold_period_days: fold === null ? null : fold.periodDays,
      fold_epoch: fold === null ? null : fold.t0,
    },
    values: {
      best_period_days: result.best_period,
      best_frequency_per_day: result.best_frequency,
      best_power: result.best_power,
      false_alarm_probability: result.false_alarm_probability,
      period_step_days: periodStep(result.best_period, result.frequency_step),
      n_points: result.n_points,
      n_dropped: result.n_dropped,
      n_frequencies: result.n_frequencies,
      frequency_step_per_day: result.frequency_step,
      baseline_days: result.baseline_days,
    },
    notes: [...result.notes, "periods in days, frequencies in cycles per day"],
  };
}
