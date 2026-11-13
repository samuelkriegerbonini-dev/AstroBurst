import type { TimeSeriesFrame, TimeSeriesResult, TimeSeriesTarget } from "../shared/types/analysis";
import { apertureLine, bjdSources, lightCurve, lightCurveExtras, MAG_ERR_FACTOR, referenceStem, saturatedCompLabels } from "./differentialPhotometry";
import { fileBaseName, isFiniteNumber } from "./values";

export type ExoClockTime = "jd_utc" | "bjd_tdb" | "hjd_utc";

export const EXOCLOCK_TIMES: readonly ExoClockTime[] = ["jd_utc", "bjd_tdb", "hjd_utc"];

export interface LightCurveExportSource {
  result: TimeSeriesResult;
  targetIdx: number;
  compIdx: number[];
  leftOut?: readonly (string | null)[];
}

export const BJD_ACCURACY_NOTE = "computed values use a low-precision ephemeris, within 1 s of astropy";

export const NO_TARGET_BLOCKER = "no star has the target role";
export const NO_ROWS_BLOCKER = "no frame has a differential magnitude";
export const AREA_MIXED_BLOCKER =
  "AREA pixel-area map applied on some frames only (a step of a few hundredths of a magnitude in the light curve, see the series warning)";

const ETD_COLUMNS_LINE = "# JD_UT V-C err";
const EXOCLOCK_COLUMNS = ["rel_flux", "rel_flux_err", "airmass", "fwhm_px", "sky", "dx_px", "dy_px"] as const;
const EXOCLOCK_TIME_DIGITS = 8;
const FLUX_SIGNIFICANT_DIGITS = 10;
const PRINTF_PRECISION = 6;
const DEFAULT_POINT_LABEL = /^(?:p|star |point )[1-9]\d*$/i;
const DATE_ONLY_MARK = "date only";
const EXPOSURE_START_MARK = "EXPTIME missing";
const HALF_EXPOSURE_NOTE = /\/2 \((\S+) s\)/;

const TIME_NAMES: Record<ExoClockTime, string> = { jd_utc: "JD_UTC", bjd_tdb: "BJD_TDB", hjd_utc: "HJD_UTC" };
const LEFT_OUT_REASONS: ReadonlyMap<string, string> = new Map([
  ["flagged", "flagged"],
  ["manual", "by hand"],
  ["clipped", "clipped"],
]);
const LEFT_OUT_ORDER = ["flagged", "by hand", "clipped"] as const;

interface ExportRow {
  frame: TimeSeriesFrame;
  mag: number;
  err: number;
  extraIndex: number;
}

export function defaultPointLabel(index: number): string {
  return `P${index + 1}`;
}

export function isDefaultPointLabel(label: string): boolean {
  return DEFAULT_POINT_LABEL.test(label);
}

export function withCurrentLabels(result: TimeSeriesResult, current: readonly TimeSeriesTarget[]): TimeSeriesResult {
  const targets = result.targets.map((echo) => {
    const region = current.find((t) => t.x === echo.x && t.y === echo.y);
    return region ? { ...echo, label: region.label } : echo;
  });
  return { ...result, targets };
}

export function magToRelativeFlux(mag: number, err: number): { flux: number; err: number } {
  const flux = 10 ** (-0.4 * mag);
  return { flux, err: (flux * err) / MAG_ERR_FACTOR };
}


export function formatPrintfF(value: number): string {
  return value.toFixed(PRINTF_PRECISION);
}

export function formatPrintfG(value: number): string {
  if (value === 0) return Object.is(value, -0) ? "-0" : "0";
  const [mantissa, exponentText] = value.toExponential(PRINTF_PRECISION - 1).split("e");
  const exponent = Number(exponentText);
  if (exponent < -4 || exponent >= PRINTF_PRECISION) {
    const sign = exponent < 0 ? "-" : "+";
    return `${stripZeros(mantissa)}e${sign}${String(Math.abs(exponent)).padStart(2, "0")}`;
  }
  return stripZeros(value.toFixed(PRINTF_PRECISION - 1 - exponent));
}

function stripZeros(text: string): string {
  return text.includes(".") ? text.replace(/0+$/, "").replace(/\.$/, "") : text;
}

function significant(value: number): string {
  return String(Number(value.toPrecision(FLUX_SIGNIFICANT_DIGITS)));
}

function cell(value: number | null, digits: number): string {
  return isFiniteNumber(value) ? value.toFixed(digits) : "";
}

function significantCell(value: number | null): string {
  return isFiniteNumber(value) ? significant(value) : "";
}

function comps(source: LightCurveExportSource): number[] {
  return source.compIdx.filter((i) => i !== source.targetIdx);
}

export function leftOutReasons(states: readonly string[]): (string | null)[] {
  return states.map((state) => LEFT_OUT_REASONS.get(state) ?? null);
}

export function leftOutSummary(reasons: readonly (string | null)[]): string | null {
  const given = reasons.filter((reason): reason is string => typeof reason === "string");
  if (given.length === 0) return null;
  const parts = LEFT_OUT_ORDER.flatMap((reason) => {
    const count = given.filter((r) => r === reason).length;
    return count > 0 ? [`${count} ${reason}`] : [];
  });
  const n = given.length;
  return `${n} ${n === 1 ? "frame" : "frames"} (${parts.join(", ")})`;
}

function measuredRows(source: LightCurveExportSource): ExportRow[] {
  const curve = lightCurve(source.result, source.targetIdx, comps(source), "index");
  return curve.flatMap((point, k) => {
    const frame = source.result.frames[point.frameIndex];
    if (!frame || point.skipped || point.mag === null || point.err === null) return [];
    return [{ frame, mag: point.mag, err: point.err, extraIndex: k }];
  });
}

function leftOutReason(source: LightCurveExportSource, row: ExportRow): string | null {
  return source.leftOut?.[row.frame.index] ?? null;
}

function exportRows(source: LightCurveExportSource): ExportRow[] {
  return measuredRows(source).filter((row) => leftOutReason(source, row) === null);
}

function utcTime(frame: TimeSeriesFrame): number | null {
  const value = frame.geometry?.jd_utc;
  return frame.geometry?.bjd_source !== "header" && isFiniteNumber(value) ? value : null;
}

function exportTime(frame: TimeSeriesFrame, time: ExoClockTime): number | null {
  if (time === "jd_utc") return utcTime(frame);
  const value = time === "bjd_tdb" ? frame.geometry?.bjd_tdb : frame.geometry?.hjd_utc;
  return isFiniteNumber(value) ? value : null;
}

function timeBlocker(frame: TimeSeriesFrame, time: ExoClockTime): string | null {
  if (exportTime(frame, time) !== null) return null;
  const name = TIME_NAMES[time];
  const refusal = frame.geometry && !isFiniteNumber(frame.geometry.jd_utc) ? frame.geometry.time_scale_notes[0] : undefined;
  if (refusal) return `no ${name} for ${frame.file_name} (${refusal})`;
  const reason = time !== "jd_utc" ? frame.geometry?.time_scale_notes.find((note) => note.includes(name)) : undefined;
  if (reason) return `no ${name} for ${frame.file_name} (${reason})`;
  if (time !== "bjd_tdb" && frame.geometry?.bjd_source === "header") {
    return `no ${name} for ${frame.file_name} (the header time is barycentric: TREFPOS/TIMEREF/BJDREF)`;
  }
  if (time !== "jd_utc") return `no ${name} for ${frame.file_name} (needs DATE-OBS and the target RA/Dec)`;
  return `no ${name} for ${frame.file_name} (DATE-OBS missing or not a valid date)`;
}

function hasFluxErrors(frame: TimeSeriesFrame, stars: number[]): boolean {
  return stars.every((i) => isFiniteNumber(frame.targets[i]?.flux_err));
}

function frameBlocker(row: ExportRow, time: ExoClockTime, source: LightCurveExportSource): string | null {
  const name = row.frame.file_name;
  const missingTime = timeBlocker(row.frame, time);
  if (missingTime) return missingTime;
  const timeSource = row.frame.time_source ?? "";
  if (timeSource.includes(DATE_ONLY_MARK)) return `no time of day for ${name} (DATE-OBS is a date only)`;
  if (timeSource.includes(EXPOSURE_START_MARK)) return `no EXPTIME for ${name} (the time is the exposure start, not mid-exposure)`;
  if (!(row.err > 0) || !hasFluxErrors(row.frame, [source.targetIdx, ...comps(source)])) return `no errors for ${name} (flux_err missing)`;
  const saturated = saturatedCompLabels(row.frame, source.result.targets, comps(source));
  if (saturated.length > 0) return `comparison star ${saturated[0]} saturated on ${name} (leave it out of the ensemble)`;
  return null;
}

function exposureSeconds(frame: TimeSeriesFrame): number | null {
  if (isFiniteNumber(frame.exptime)) return frame.exptime;
  const seconds = Number(frame.time_source?.match(HALF_EXPOSURE_NOTE)?.[1]);
  return Number.isFinite(seconds) ? seconds : null;
}

function targetLabel(source: LightCurveExportSource): string {
  return source.result.targets[source.targetIdx]?.label ?? "";
}

function exportBlocker(source: LightCurveExportSource, time: ExoClockTime): string | null {
  if (source.targetIdx < 0 || source.result.targets[source.targetIdx]?.role !== "target") return NO_TARGET_BLOCKER;
  const label = targetLabel(source);
  if (label.trim() === "" || isDefaultPointLabel(label)) {
    return `no target name: label the target's Point region with the star's designation (now "${label}")`;
  }
  const rows = exportRows(source);
  if (rows.length === 0) return NO_ROWS_BLOCKER;
  if (source.result.area_mixed) return AREA_MIXED_BLOCKER;
  for (const row of rows) {
    const blocker = frameBlocker(row, time, source);
    if (blocker) return blocker;
  }
  return null;
}

export function etdBlocker(source: LightCurveExportSource): string | null {
  return exportBlocker(source, "jd_utc");
}

export function exoClockBlocker(source: LightCurveExportSource, time: ExoClockTime): string | null {
  return exportBlocker(source, time);
}

export function exportBlockerNotes(etd: string | null, exoClock: string | null): { key: string; text: string }[] {
  if (etd && etd === exoClock) return [{ key: "etd-exoclock", text: `ETD and ExoClock: ${etd}` }];
  const notes: { key: string; text: string }[] = [];
  if (etd) notes.push({ key: "etd", text: `ETD: ${etd}` });
  if (exoClock) notes.push({ key: "exoclock", text: `ExoClock: ${exoClock}` });
  return notes;
}

function sortedRows(source: LightCurveExportSource, time: ExoClockTime): { row: ExportRow; t: number }[] {
  return exportRows(source)
    .map((row) => ({ row, t: exportTime(row.frame, time) as number }))
    .sort((a, b) => a.t - b.t);
}

function assertExportable(blocker: string | null): void {
  if (blocker) throw new Error(blocker);
}

export function etdText(source: LightCurveExportSource): string {
  assertExportable(etdBlocker(source));
  const rows = sortedRows(source, "jd_utc");
  const lines = [`#JD_UT (+ ${Math.trunc(rows[0].t)})`, ETD_COLUMNS_LINE];
  for (const { row, t } of rows) lines.push(`${formatPrintfF(t)} ${formatPrintfG(row.mag)} ${formatPrintfG(row.err)}`);
  return lines.join("\n");
}

function distinct(values: (string | number | null)[]): string {
  const seen = [...new Set(values.filter((v) => v !== null && v !== "").map(String))];
  return seen.length > 0 ? seen.join(", ") : "unknown";
}

function offsetCell(frame: TimeSeriesFrame, value: number | null, referencePath: string): string {
  if (frame.offset === null && frame.path === referencePath) return cell(0, 3);
  return frame.offset?.registered ? cell(value, 3) : "";
}

export function exoClockCsv(source: LightCurveExportSource, time: ExoClockTime): string {
  assertExportable(exoClockBlocker(source, time));
  const { result } = source;
  const rows = sortedRows(source, time);
  const extras = lightCurveExtras(result, source.targetIdx, comps(source));
  const measured = rows.map(({ row }) => row);
  const all = measuredRows(source);
  const frames = result.frames.length;
  const leftOut = all.length > rows.length ? leftOutSummary(all.map((row) => leftOutReason(source, row))) : null;
  const bjdComputed = time === "bjd_tdb" && measured.some((r) => r.frame.geometry?.bjd_source === "computed");
  const lines = [
    "# AstroBurst differential light curve for ExoClock",
    `# target: ${targetLabel(source)}`,
    `# comparison stars: ${comps(source).map((i) => result.targets[i]?.label ?? `#${i}`).join(", ")}`,
    `# reference frame: ${fileBaseName(result.reference_path)}`,
    `# time: ${time} = ${TIME_NAMES[time]} at mid-exposure`,
    ...(time === "bjd_tdb" ? [`# bjd_tdb source: ${bjdSources(measured.map((r) => r.frame)) ?? "unknown"}`] : []),
    ...(bjdComputed ? [`# bjd_tdb accuracy: ${BJD_ACCURACY_NOTE}`] : []),
    `# flux: target / sum of comparison fluxes, not normalised; rel_flux_err = rel_flux * diff_err / ${MAG_ERR_FACTOR}`,
    `# filter: ${distinct(measured.map((r) => r.frame.filter))}`,
    `# exposure s: ${distinct(measured.map((r) => exposureSeconds(r.frame)))}`,
    apertureLine(result, source.targetIdx),
    `# rows: ${rows.length} of ${frames} frames (${frames - all.length} omitted: skipped or no differential magnitude)`,
    ...(leftOut ? [`# left out: ${leftOut}`] : []),
    [time, ...EXOCLOCK_COLUMNS].join(","),
  ];
  for (const { row, t } of rows) {
    const flux = magToRelativeFlux(row.mag, row.err);
    const extra = extras[row.extraIndex];
    lines.push(
      [
        t.toFixed(EXOCLOCK_TIME_DIGITS),
        significant(flux.flux),
        significant(flux.err),
        cell(extra?.airmass ?? null, 4),
        cell(extra?.fwhm ?? null, 3),
        significantCell(extra?.sky ?? null),
        offsetCell(row.frame, extra?.dx ?? null, result.reference_path),
        offsetCell(row.frame, extra?.dy ?? null, result.reference_path),
      ].join(","),
    );
  }
  return lines.join("\n") + "\n";
}

function fileStem(source: LightCurveExportSource): string {
  const stem = referenceStem(source.result.reference_path);
  const target = targetLabel(source).trim().replace(/[^A-Za-z0-9._-]+/g, "_") || "target";
  return `${target}_${stem}`;
}

export function etdFileName(source: LightCurveExportSource): string {
  return `${fileStem(source)}_etd.txt`;
}

export function exoClockFileName(source: LightCurveExportSource, time: ExoClockTime): string {
  return `${fileStem(source)}_exoclock_${time}.csv`;
}
