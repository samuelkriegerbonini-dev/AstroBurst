import type { LightCurveViewState } from "../shared/types/lightCurve";
import type { TsoCurve, TsoFile, TsoLightCurves } from "../shared/types/tso";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "./catalogCsv";
import { PROVENANCE_FILE_SEPARATOR, provenanceLines, unitLines, withProvenance, type ProvenanceEntry } from "./csvProvenance";
import { QUICK_LOOK_LABEL } from "./lightCurveAnalysis";
import { tsoTimeName, type DepthRow, type DepthSpectrum } from "./tsoLightCurves";

export interface TsoExportContext {
  software: string;
  exportedUtc: string;
  files: readonly Pick<TsoFile, "file_name" | "sha256" | "level">[];
}

interface BandRow {
  band: string;
  row: DepthRow;
}

const DEPTH_RULE = "D = 1 - F_in / F_out; depth_err is the larger of depth_err_scatter and depth_err_formal";
const WHITE_BAND = "white";
const DAYS = "d";
const MICROMETRES = "um";
const FRACTION = "fraction";

function standardEntries(context: TsoExportContext): ProvenanceEntry[] {
  return [
    ["software", context.software],
    ["exported_utc", context.exportedUtc],
    ["file", context.files.map((f) => f.file_name).join(PROVENANCE_FILE_SEPARATOR)],
  ];
}

function seriesEntries(result: TsoLightCurves, product: string): ProvenanceEntry[] {
  return [
    ["product", product],
    ["series", result.label],
    ["detector", result.series.detector],
    ["sporder", result.series.sporder ?? "none"],
    ["mode", result.mode],
  ];
}

function sourceEntries(result: TsoLightCurves, context: TsoExportContext): ProvenanceEntry[] {
  return [
    ["set_id", result.set_id],
    ["source_level", [...new Set(context.files.map((f) => f.level))].join(PROVENANCE_FILE_SEPARATOR)],
    ["source_sha256", context.files.map((f) => f.sha256).join(PROVENANCE_FILE_SEPARATOR)],
  ];
}

function warningEntries(warnings: readonly string[]): ProvenanceEntry[] {
  return warnings.map((warning) => ["warning", warning]);
}

function rejectedText(result: TsoLightCurves): string {
  if (result.rejected_elements.length === 0) return "none";
  return result.rejected_elements.map((e) => `${e.index} at ${e.wavelength_um} um`).join(PROVENANCE_FILE_SEPARATOR);
}

function curveColumns(prefix: string, curve: TsoCurve): CsvColumn<number>[] {
  return [
    { header: `${prefix}_flux`, value: (i) => curve.flux[i] },
    { header: `${prefix}_flux_err`, value: (i) => curve.flux_err[i] },
    { header: `${prefix}_n_used`, value: (i) => curve.n_used[i] },
    { header: `${prefix}_npixels_mean`, value: (i) => curve.npixels_mean[i] },
  ];
}

export function tsoCurvesCsv(result: TsoLightCurves, context: TsoExportContext): string {
  const curves: [string, TsoCurve][] = [["white", result.white], ...result.bins.map((bin, k): [string, TsoCurve] => [`bin${k + 1}`, bin])];
  const [whiteLo, whiteHi] = result.white.band_um;
  const entries: ProvenanceEntry[] = [
    ...standardEntries(context),
    ...seriesEntries(result, "tso_light_curves"),
    ["time_scale", result.time_scale],
    ["flux_unit", result.flux_unit],
    ...sourceEntries(result, context),
    ["white_band_um", `${whiteLo} to ${whiteHi} (${result.white.limits_origin})`],
    ...result.bins.map((bin, k): ProvenanceEntry => [`bin${k + 1}_band_um`, `${bin.band_um[0]} to ${bin.band_um[1]}, upper edge excluded`]),
    ["dropped_nan_time", result.dropped.nan_time],
    ["dropped_repeated_time", result.dropped.repeated_time],
    ["rejected_elements", rejectedText(result)],
    ...warningEntries(result.warnings),
  ];
  const timeUnit = result.time_scale === "index" ? tsoTimeName(result.time_scale) : `${DAYS}, ${tsoTimeName(result.time_scale)}`;
  const units: Record<string, string> = { time: timeUnit, mjd_utc: `${DAYS}, ${tsoTimeName("mjd_utc")}` };
  for (const [prefix] of curves) {
    units[`${prefix}_flux`] = result.flux_unit;
    units[`${prefix}_flux_err`] = result.flux_unit;
  }
  const columns: CsvColumn<number>[] = [
    { header: "int_num", value: (i) => result.int_num[i] },
    { header: "row_index", value: (i) => result.row_index[i] },
    { header: "time", value: (i) => result.time[i] },
    { header: "mjd_utc", value: (i) => result.mjd_utc[i] },
    ...curves.flatMap(([prefix, curve]) => curveColumns(prefix, curve)),
  ];
  const rows = result.time.map((_, i) => i);
  return withProvenance([...provenanceLines(entries), ...unitLines(units)], buildCsv(columns, rows), CSV_LINE_END);
}

const DEPTH_COLUMNS: CsvColumn<BandRow>[] = [
  { header: "band", value: (r) => r.band },
  { header: "band_lo_um", value: (r) => r.row.band[0] },
  { header: "band_hi_um", value: (r) => r.row.band[1] },
  { header: "centre_um", value: (r) => r.row.centre },
  { header: "depth", value: (r) => r.row.depth },
  { header: "depth_err", value: (r) => r.row.err },
  { header: "depth_err_scatter", value: (r) => r.row.errScatter },
  { header: "depth_err_formal", value: (r) => r.row.errFormal },
  { header: "n_in", value: (r) => r.row.nIn },
  { header: "n_out", value: (r) => r.row.nOut },
  { header: "npixels_ratio", value: (r) => r.row.pixelRatio },
  { header: "pixel_loss", value: (r) => r.row.pixelLoss },
];

const DEPTH_UNITS: Record<string, string> = {
  band_lo_um: MICROMETRES,
  band_hi_um: MICROMETRES,
  centre_um: MICROMETRES,
  depth: FRACTION,
  depth_err: FRACTION,
  depth_err_scatter: FRACTION,
  depth_err_formal: FRACTION,
};

export function tsoDepthSpectrumCsv(
  spectrum: DepthSpectrum,
  result: TsoLightCurves,
  view: Pick<LightCurveViewState, "baselineKind">,
  context: TsoExportContext,
): string {
  const entries: ProvenanceEntry[] = [
    ...standardEntries(context),
    ...seriesEntries(result, "tso_depth_spectrum"),
    ["quick_look", QUICK_LOOK_LABEL],
    ["depth", DEPTH_RULE],
    ["window_start", spectrum.window?.start ?? null],
    ["window_end", spectrum.window?.end ?? null],
    ["window_time", tsoTimeName(result.time_scale)],
    ["baseline_margin_days", spectrum.marginDays],
    ["baseline_kind", view.baselineKind],
    ...sourceEntries(result, context),
    ...warningEntries(spectrum.warnings),
  ];
  const rows: BandRow[] = [
    ...(spectrum.white === null ? [] : [{ band: WHITE_BAND, row: spectrum.white }]),
    ...spectrum.rows.map((row) => ({ band: row.label, row })),
  ];
  return withProvenance([...provenanceLines(entries), ...unitLines(DEPTH_UNITS)], buildCsv(DEPTH_COLUMNS, rows), CSV_LINE_END);
}

function exportFileName(result: Pick<TsoLightCurves, "label" | "mode">, firstFileName: string, suffix: string): string {
  const name = firstFileName.replace(/\.gz$/, "");
  const dot = name.lastIndexOf(".");
  const stem = dot > 0 ? name.slice(0, dot) : name;
  if (stem === "") return `tso_${suffix}`;
  const series = result.label.toLowerCase().replace(/[^a-z0-9]+/g, "_");
  return `${stem}_${series}_${result.mode}_${suffix}`;
}

export function tsoCurvesCsvFileName(result: Pick<TsoLightCurves, "label" | "mode">, firstFileName: string): string {
  return exportFileName(result, firstFileName, "lightcurves.csv");
}

export function tsoDepthSpectrumCsvFileName(result: Pick<TsoLightCurves, "label" | "mode">, firstFileName: string): string {
  return exportFileName(result, firstFileName, "depth_spectrum.csv");
}
