import type { TimeSeriesFrame, TimeSeriesResult, TimeSeriesTarget } from "../shared/types/analysis";
import { lightCurve, lightCurveExtras, MAG_ERR_FACTOR, referenceStem } from "./differentialPhotometry";
import { fileBaseName, isFiniteNumber } from "./values";

export type ExoClockTime = "jd_utc" | "bjd_tdb" | "hjd_utc";

export const EXOCLOCK_TIMES: readonly ExoClockTime[] = ["jd_utc", "bjd_tdb", "hjd_utc"];

export interface LightCurveExportSource {
  result: TimeSeriesResult;
  targetIdx: number;
  compIdx: number[];
}

export const NO_TARGET_BLOCKER = "no star has the target role";
export const NO_ROWS_BLOCKER = "no frame has a differential magnitude";
export const AREA_MIXED_BLOCKER =
  "AREA pixel-area map applied on some frames only (a step of a few hundredths of a magnitude in the light curve, see the series warning)";

const ETD_COLUMNS_LINE = "# JD_UT V-C err";
const EXOCLOCK_COLUMNS = ["rel_flux", "rel_flux_err", "airmass", "fwhm_px", "sky", "dx_px", "dy_px"] as const;
const EXOCLOCK_TIME_DIGITS = 8;
const FLUX_SIGNIFICANT_DIGITS = 10;
const PRINTF_PRECISION = 6;
const DEFAULT_POINT_LABEL = /^P[1-9]\d*$/;
const DATE_ONLY_MARK = "date only";
const EXPOSURE_START_MARK = "EXPTIME missing";
const HALF_EXPOSURE_NOTE = /\/2 \((\S+) s\)/;

const TIME_NAMES: Record<ExoClockTime, string> = { jd_utc: "JD_UTC", bjd_tdb: "BJD_TDB", hjd_utc: "HJD_UTC" };

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

function comps(source: LightCurveExportSource): number[] {
  return source.compIdx.filter((i) => i !== source.targetIdx);
}

function exportRows(source: LightCurveExportSource): ExportRow[] {
  const curve = lightCurve(source.result, source.targetIdx, comps(source), "index");
  return curve.flatMap((point, k) => {
    const frame = source.result.frames[point.frameIndex];
    if (!frame || point.skipped || point.mag === null || point.err === null) return [];
    return [{ frame, mag: point.mag, err: point.err, extraIndex: k }];
  });
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
  if (time !== "bjd_tdb" && frame.geometry?.bjd_source === "header") {
    return `no ${name} for ${frame.file_name} (the header time is barycentric: TIMEREF/BJDREF)`;
  }
  if (time !== "jd_utc") return `no ${name} for ${frame.file_name} (needs DATE-OBS and the target RA/Dec)`;
  return `no ${name} for ${frame.file_name} (DATE-OBS missing)`;
}

function hasFluxErrors(frame: TimeSeriesFrame, stars: number[]): boolean {
  return stars.every((i) => isFiniteNumber(frame.targets[i]?.flux_err));
}

function frameBlocker(row: ExportRow, time: ExoClockTime, stars: number[]): string | null {
  const name = row.frame.file_name;
  const missingTime = timeBlocker(row.frame, time);
  if (missingTime) return missingTime;
  const timeSource = row.frame.time_source ?? "";
  if (timeSource.includes(DATE_ONLY_MARK)) return `no time of day for ${name} (DATE-OBS is a date only)`;
  if (timeSource.includes(EXPOSURE_START_MARK)) return `no EXPTIME for ${name} (the time is the exposure start, not mid-exposure)`;
  if (!(row.err > 0) || !hasFluxErrors(row.frame, stars)) return `no errors for ${name} (flux_err missing)`;
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
  const stars = [source.targetIdx, ...comps(source)];
  for (const row of rows) {
    const blocker = frameBlocker(row, time, stars);
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

function apertureLine(source: LightCurveExportSource, rows: ExportRow[]): string {
  const reference = source.result.frames.find((f) => f.path === source.result.reference_path);
  const star = reference?.targets[source.targetIdx] ?? rows[0].frame.targets[source.targetIdx];
  if (!star) return "# aperture px: unknown";
  return `# aperture px: ${star.aperture_radius}; sky annulus px: ${star.sky_inner} to ${star.sky_outer}`;
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
  const omitted = result.frames.length - rows.length;
  const lines = [
    "# AstroBurst differential light curve for ExoClock",
    `# target: ${targetLabel(source)}`,
    `# comparison stars: ${comps(source).map((i) => result.targets[i]?.label ?? `#${i}`).join(", ")}`,
    `# reference frame: ${fileBaseName(result.reference_path)}`,
    `# time: ${time} = ${TIME_NAMES[time]} at mid-exposure`,
    `# flux: target / sum of comparison fluxes, not normalised; rel_flux_err = rel_flux * diff_err / ${MAG_ERR_FACTOR}`,
    `# filter: ${distinct(measured.map((r) => r.frame.filter))}`,
    `# exposure s: ${distinct(measured.map((r) => exposureSeconds(r.frame)))}`,
    apertureLine(source, measured),
    `# rows: ${rows.length} of ${result.frames.length} frames (${omitted} omitted: skipped or no differential magnitude)`,
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
        cell(extra?.sky ?? null, 3),
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
