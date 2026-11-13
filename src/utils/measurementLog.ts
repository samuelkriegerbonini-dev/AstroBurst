import type { BatchPhotometryResult, PhotCal, PhotometryMeasurement, UserZeroPoint } from "../services/analysis";
import type { PixelTableResult, TimeSeriesResult } from "../shared/types/analysis";
import type { SkySeparationResult } from "../shared/types/astrometry";
import type { CrossMatchResult } from "../shared/types/catalog";
import type { PvRun } from "../shared/types/pv";
import type { LineCut, RadialProfile, Region, RegionShape, RegionStats, RegionStatsEntry, SbProfile } from "../shared/types/regions";
import type { LineMeasurement, LineModel } from "../shared/types/spectral";
import type { ChannelStatisticsBody } from "../shared/types/statistics";
import type { MeasurementSource } from "./analysisTarget";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "./catalogCsv";
import { exportEntries, provenanceLines, summedPixelUnit, withProvenance } from "./csvProvenance";
import {
  bjdSources,
  checkReference,
  hasCompleteTimeAxis,
  lightCurve,
  referenceTimeSource,
  saturatedCompLabels,
  saturatedComps,
  seriesRms,
} from "./differentialPhotometry";
import { generateId } from "./format";
import { exportStem, parseImageRef } from "./imageRef";
import type { LineFitRun } from "./lineFit";
import { batchWarnings, calibratedMagnitude, calibrationText, measurementWarnings } from "./photometryPanel";
import { summedFluxUnit } from "./photometryTable";
import { radiusInArcsec, type RadiiInput, type RadiusUnit } from "./photometryUnits";
import { regionSkyOf } from "./regionCsv";
import { isExcludeRegion } from "./regionExclude";
import { firstSurfaceBrightness } from "./sbProfile";
import { hasDocumentEntry, type ComparisonCsvInput } from "./spectrumCompare";
import { spectrumExportSummary, type SpectrumExportInput } from "./spectrumExport";
import { finiteOrNull } from "./values";

export const MAX_LOG_ENTRIES = 5000;
export const MEASUREMENT_KINDS = [
  "photometry",
  "photometry_batch",
  "region_stats",
  "radial_profile",
  "sb_profile",
  "line_cut",
  "sky_separation",
  "line",
  "time_series",
  "pixel",
  "statistics",
  "catalog_crossmatch",
  "spectrum_export",
  "spectrum_compare",
  "pv",
  "line_fit",
  "tso_light_curve",
  "tso_depth_spectrum",
  "tso_transit_spectrum",
  "periodogram",
  "forced_photometry",
  "image_depth",
  "resolution_check",
  "catalog_compare",
  "sed",
  "layer_assignment",
  "transit_fit",
] as const;

const DEFAULT_CLIP_SIGMA = 3;
const DEFAULT_CLIP_ITERS = 5;
const AUTO = "auto";
const GAIA_CATALOGUE = "Gaia DR3";
const NO_GAIN = "none";
const DEFAULT_CSV_NAME = "astroburst_measurements.csv";
const CSV_SUFFIX = "_measurements.csv";
const NOTE_SEPARATOR = " | ";
const NULL_TEXT = "--";
const SUMMARY_DIGITS = 6;
const NO_STATISTICS_NOTE = "no statistics";
export const EXCLUDE_ROW_NOTE = "exclude region: measured on its own pixels, without the exclude mask";
const LOADED_CUBE_IMAGE = "loaded cube";
const SPECTRUM_FILE_IMAGE = "spectrum file";
const LOADED_FILE_IMAGE = "loaded file";
const ORIGINAL_IMAGE = "original";
const IMAGE_UNIT = "the image unit";
const PARAM_PREFIX = "p_";
const VALUE_PREFIX = "v_";
const FIXED_COLUMNS = ["timestamp_utc", "kind", "file", "image", "dq", "unit", "source", "notes"] as const;

export type MeasurementKind = (typeof MEASUREMENT_KINDS)[number];
export type DqHandling = "excluded" | "requested" | "off" | "not_applied" | "reported";
export type LogValue = number | string | boolean | null;
export type LogRecord = Readonly<Record<string, LogValue>>;

export interface MeasurementLogEntry {
  id: string;
  timestamp_utc: string;
  kind: MeasurementKind;
  file: string | null;
  image: string;
  dq: DqHandling;
  unit: string | null;
  source: string;
  params: LogRecord;
  values: LogRecord;
  notes: readonly string[];
}

export type MeasurementLogDraft = Omit<MeasurementLogEntry, "id" | "timestamp_utc">;

export interface MeasurementProvenance {
  file: string | null;
  image: string;
}

export const EMPTY_LOG: readonly MeasurementLogEntry[] = Object.freeze([]) as readonly MeasurementLogEntry[];

const LOG_SESSION_PREFIX = "Kept for this session only:";

export const LOG_SESSION_NOTE = `${LOG_SESSION_PREFIX} Save CSV to keep it.`;

function rowCount(n: number): string {
  return `${n} row${n === 1 ? "" : "s"}`;
}

export function logSessionNote(total: number, shown: number): string {
  if (shown === 0) return `${LOG_SESSION_PREFIX} no row is shown; clear the filters to save the ${rowCount(total)} with Save CSV.`;
  if (shown < total) return `${LOG_SESSION_PREFIX} Save CSV writes the ${shown} shown ${shown === 1 ? "row" : "rows"} of ${total}; clear the filters to keep them all.`;
  return `${LOG_SESSION_PREFIX} Save CSV to keep ${total === 1 ? "this 1 row" : `these ${total} rows`}.`;
}

export function clearConfirmText(total: number, shown: number): string {
  const rows = rowCount(total);
  return shown < total ? `Clear all ${rows}, not only the ${shown} shown?` : `Clear ${rows}?`;
}

type Listener = () => void;

export class MeasurementLogCore {
  private entries: readonly MeasurementLogEntry[] = EMPTY_LOG;
  private listeners = new Set<Listener>();
  private readonly now: () => Date;
  private readonly newId: () => string;

  constructor(now: () => Date = () => new Date(), newId: () => string = generateId) {
    this.now = now;
    this.newId = newId;
  }

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  getSnapshot = (): readonly MeasurementLogEntry[] => this.entries;

  append(draft: MeasurementLogDraft): MeasurementLogEntry {
    const entry = this.stamp(draft);
    this.commit([...this.entries, entry]);
    return entry;
  }

  appendAll(drafts: readonly MeasurementLogDraft[]): MeasurementLogEntry[] {
    if (drafts.length === 0) return [];
    const stamped = drafts.map((draft) => this.stamp(draft));
    this.commit([...this.entries, ...stamped]);
    return stamped;
  }

  clear(): void {
    if (this.entries.length === 0) return;
    this.entries = EMPTY_LOG;
    this.notify();
  }

  private stamp(draft: MeasurementLogDraft): MeasurementLogEntry {
    return Object.freeze({
      id: this.newId(),
      timestamp_utc: this.now().toISOString(),
      kind: draft.kind,
      file: draft.file,
      image: draft.image,
      dq: draft.dq,
      unit: draft.unit,
      source: draft.source,
      params: Object.freeze({ ...draft.params }),
      values: Object.freeze({ ...draft.values }),
      notes: Object.freeze([...draft.notes]),
    });
  }

  private commit(next: MeasurementLogEntry[]): void {
    this.entries = next.length > MAX_LOG_ENTRIES ? next.slice(next.length - MAX_LOG_ENTRIES) : next;
    this.notify();
  }

  private notify(): void {
    this.listeners.forEach((listener) => listener());
  }
}

export const measurementLog = new MeasurementLogCore();

export function dqHandlingOf(requested: boolean, masked: boolean | null): DqHandling {
  if (!requested) return "off";
  return masked === true ? "excluded" : "requested";
}

export function profileDqHandling(excludeDq: boolean, data: { dq_excluded: number | null }): DqHandling {
  return dqHandlingOf(excludeDq, data.dq_excluded !== null);
}

export function imageLabelOf(source: MeasurementSource | null): string {
  return source?.text ?? ORIGINAL_IMAGE;
}

export function fileNameOf(path: string | null): string | null {
  if (path === null) return null;
  const name = parseImageRef(path).path.split(/[\\/]/).pop() ?? "";
  return name === "" ? null : name;
}

export { finiteOrNull } from "./values";

export function positiveOrNull(v: number | null | undefined): number | null {
  const finite = finiteOrNull(v);
  return finite !== null && finite > 0 ? finite : null;
}

export function medianOf(values: readonly (number | null | undefined)[]): number | null {
  const finite = values.filter((v): v is number => typeof v === "number" && Number.isFinite(v)).sort((a, b) => a - b);
  const n = finite.length;
  if (n === 0) return null;
  const mid = Math.floor(n / 2);
  return n % 2 === 1 ? finite[mid] : (finite[mid - 1] + finite[mid]) / 2;
}

export function spectrumUnitOf(fluxUnit: string, axisUnit: string): string | null {
  const suffix = ` x ${axisUnit}`;
  if (!fluxUnit.endsWith(suffix)) return null;
  const prefix = fluxUnit.slice(0, fluxUnit.length - suffix.length);
  return prefix === "" ? null : prefix;
}

export function lineUnitNote(fluxUnit: string, axisUnit: string): string {
  const spectrumUnit = spectrumUnitOf(fluxUnit, axisUnit) ?? "the flux unit without the axis factor";
  return `flux, flux_err, flux_err_pixel, flux_err_level, flux_err_slope in ${fluxUnit}; peak, continuum_level, continuum_sigma, fit_amplitude in the spectrum unit (${spectrumUnit}); continuum_slope in the spectrum unit per ${axisUnit}; centroid, sigma, fwhm, equivalent_width, fit_centre, fit_sigma, fit_centre_err, fit_sigma_err in ${axisUnit}; centroid_kms, sigma_kms, fwhm_kms, source_centroid_kms, source_sigma_kms, source_fwhm_kms in km/s; equivalent_width_err, centroid_err, sigma_err, fwhm_err, peak_axis in ${axisUnit}; fit_amplitude_err in the spectrum unit (${spectrumUnit}); every _err_kms value in km/s; flux_si, flux_si_err in W m^-2; flux_cgs, flux_cgs_err in erg s^-1 cm^-2; z_line, z_line_err dimensionless`;
}

type PhotometryUnits = Pick<PhotometryMeasurement, "photcal" | "bunit" | "summed_unit">;

function photometrySumUnit(res: PhotometryUnits): string | null {
  return res.summed_unit ?? summedFluxUnit(res.photcal);
}

function photometryPixelUnit(res: PhotometryUnits): string | null {
  return res.bunit ?? res.photcal?.bunit ?? null;
}

function photometryUnitNote(res: PhotometryUnits): string {
  const sumUnit = photometrySumUnit(res) ?? `${IMAGE_UNIT}, summed over the aperture pixels`;
  const pixelUnit = photometryPixelUnit(res) ?? IMAGE_UNIT;
  return `net_flux, flux_err, flux_total in ${sumUnit}; peak, bg_mean, bg_sigma in ${pixelUnit} (pixel values); flux_jy, flux_err_jy in Jy; mag_inst, mag_ab, mag_ab_err, mag_ab_total, mag, mag_err, gaia_gmag, gaia_bp_rp in mag (mag and mag_err in the system of the parameter mag_system); x, y, fwhm, ee50_radius, ee80_radius in px; ra, dec in deg; aperture_correction = flux_total / net_flux and ee_in_aperture = net_flux / flux_total, both dimensionless`;
}

export interface RadialLogRequest {
  x: number;
  y: number;
  maxRadius: number;
  background: [number, number] | null;
  exclude: readonly RegionShape[];
}

export interface SbLogRequest {
  shape: RegionShape;
  background: RegionShape | null;
  binWidth: number;
  exclude: readonly RegionShape[];
}

export function filterEntries(
  entries: readonly MeasurementLogEntry[],
  kind: MeasurementKind | "all",
  file: string | null,
): readonly MeasurementLogEntry[] {
  if (kind === "all" && file === null) return entries;
  return entries.filter((e) => (kind === "all" || e.kind === kind) && (file === null || e.file === file));
}

function unionKeys(entries: readonly MeasurementLogEntry[], pick: (e: MeasurementLogEntry) => LogRecord): string[] {
  const keys: string[] = [];
  const seen = new Set<string>();
  for (const entry of entries) {
    for (const key of Object.keys(pick(entry))) {
      if (seen.has(key)) continue;
      seen.add(key);
      keys.push(key);
    }
  }
  return keys;
}

export interface MeasurementLogCsvContext {
  exportedUtc: string;
}

const LOG_TIMESTAMPS_TEXT = "UTC, ISO 8601, the time each row was logged";
const LOG_UNITS_TEXT = "the unit column gives the unit of the main values of a row; the notes of the row name the values that have another unit";
const LOG_COLUMNS_TEXT = "p_ columns are parameters and v_ columns are values; an empty cell means the row has no such key; the notes of a row are joined by a vertical bar";

function logProvenanceLines(entries: readonly MeasurementLogEntry[], context: MeasurementLogCsvContext): string[] {
  const kinds = MEASUREMENT_KINDS.filter((kind) => entries.some((e) => e.kind === kind));
  return provenanceLines([
    ...exportEntries(entries.map((e) => e.file), context.exportedUtc),
    ["rows", entries.length],
    ["kinds", kinds.length > 0 ? kinds.join("; ") : null],
    ["timestamps", LOG_TIMESTAMPS_TEXT],
    ["units", LOG_UNITS_TEXT],
    ["columns", LOG_COLUMNS_TEXT],
  ]);
}

function pixelUnitOf(unit: string | null | undefined): string | null {
  const trimmed = unit?.trim() ?? "";
  return trimmed === "" ? null : trimmed;
}

function summedUnitNotes(keys: readonly string[], pixelUnit: string | null): string[] {
  const sumUnit = summedPixelUnit(pixelUnit);
  return sumUnit !== null && sumUnit !== pixelUnit ? [`${keys.join(", ")} in ${sumUnit}`] : [];
}

export function measurementLogCsv(entries: readonly MeasurementLogEntry[], context?: MeasurementLogCsvContext): string {
  const columns: CsvColumn<MeasurementLogEntry>[] = [
    { header: FIXED_COLUMNS[0], value: (e) => e.timestamp_utc },
    { header: FIXED_COLUMNS[1], value: (e) => e.kind },
    { header: FIXED_COLUMNS[2], value: (e) => e.file },
    { header: FIXED_COLUMNS[3], value: (e) => e.image },
    { header: FIXED_COLUMNS[4], value: (e) => e.dq },
    { header: FIXED_COLUMNS[5], value: (e) => e.unit },
    { header: FIXED_COLUMNS[6], value: (e) => e.source },
    { header: FIXED_COLUMNS[7], value: (e) => e.notes.join(NOTE_SEPARATOR) },
    ...unionKeys(entries, (e) => e.params).map((key) => ({ header: `${PARAM_PREFIX}${key}`, value: (e: MeasurementLogEntry) => e.params[key] })),
    ...unionKeys(entries, (e) => e.values).map((key) => ({ header: `${VALUE_PREFIX}${key}`, value: (e: MeasurementLogEntry) => e.values[key] })),
  ];
  const table = buildCsv(columns, [...entries]);
  return context === undefined ? table : withProvenance(logProvenanceLines(entries, context), table, CSV_LINE_END);
}

export function measurementLogCsvFileName(filePath: string | null): string {
  if (filePath === null) return DEFAULT_CSV_NAME;
  return `${exportStem(filePath)}${CSV_SUFFIX}`;
}

export function formatLogValue(v: LogValue): string {
  if (v === null) return NULL_TEXT;
  if (typeof v === "boolean") return v ? "true" : "false";
  if (typeof v === "string") return v;
  if (!Number.isFinite(v)) return NULL_TEXT;
  return Number.isInteger(v) ? String(v) : String(Number(v.toPrecision(SUMMARY_DIGITS)));
}

export function entrySummary(e: MeasurementLogEntry, max = 3): string {
  return Object.entries(e.values)
    .slice(0, max)
    .map(([key, value]) => `${key}=${formatLogValue(value)}`)
    .join(", ");
}

export interface RegionStatsMeasured {
  regions: Region[];
  excludeDq: boolean;
  sigma: number | null;
  maxiters: number | null;
  masked: boolean;
  dqExcluded: number | null;
  regionExcluded: number;
  elapsedMs: number;
  notes?: readonly string[];
  bunit?: string | null;
  summedUnit?: string | null;
}

export function regionLogReady(
  measured: RegionStatsMeasured | null,
  live: { regions: Region[]; excludeDq: boolean; sigma: number | null; maxiters: number | null },
): boolean {
  return (
    measured !== null &&
    measured.regions === live.regions &&
    measured.excludeDq === live.excludeDq &&
    measured.sigma === live.sigma &&
    measured.maxiters === live.maxiters
  );
}

export function profileLogReady(
  result: { requestKey: string; excludeDq: boolean } | null,
  requestKey: string | null,
  excludeDq: boolean,
): boolean {
  return result !== null && result.requestKey === requestKey && result.excludeDq === excludeDq;
}

export interface StatisticsLogInput {
  channels: { label: string; body: ChannelStatisticsBody }[];
  unit: string | null;
  masked: boolean;
  dqExcluded: number | null;
  elapsedMs: number;
  regionKind: RegionShape["shape"] | null;
  composite: boolean;
  noise: boolean;
  excludeDq: boolean;
  region: RegionShape | null;
}

export function statisticsLogReady(
  result: StatisticsLogInput | null,
  live: { composite: boolean; noise: boolean; excludeDq: boolean; region: RegionShape | null },
): boolean {
  return (
    result !== null &&
    result.composite === live.composite &&
    result.noise === live.noise &&
    result.region === live.region &&
    (result.composite || result.excludeDq === live.excludeDq)
  );
}

function requestedOrAuto(v: number | undefined): LogValue {
  return v === undefined ? AUTO : v;
}

function gainOrNone(v: number | undefined): LogValue {
  return v === undefined ? NO_GAIN : v;
}

function regionSource(label: string | null, shape: RegionShape, background: RegionShape | null): string {
  return JSON.stringify({ kind: "region", label, shape, background });
}

function pixelSource(x: number, y: number): string {
  return JSON.stringify({ kind: "pixel", x, y });
}

const IMAGE_SOURCE = JSON.stringify({ kind: "image" });
const FILE_SOURCE = JSON.stringify({ kind: "file" });

export interface PhotometryRequestLog {
  radiusUnit?: RadiusUnit;
  typedRadii?: RadiiInput;
  pixelScaleArcsec?: number | null;
  zeroPoint?: UserZeroPoint | null;
  limitSigma?: number;
  fCorr?: number | null;
}

export interface PhotometryLimitLog {
  limitJy: number | null;
  limitAb: number | null;
  limitNative: number | null;
}

export interface PhotometryLogOptions extends PhotometryRequestLog {
  apertureRadius?: number;
  annulusInner?: number;
  annulusOuter?: number;
  gain?: number;
  gaiaMatch: boolean;
  excludeDq: boolean;
  limit?: PhotometryLimitLog | null;
}

export const BATCH_ROWS_NOTE = "per-source rows are not kept in this log: save the table CSV to keep them";

const LIMIT_NOTE =
  "upper_limit_jy in Jy, upper_limit_ab in mag (AB), upper_limit_native in the unit of net_flux; a value is a limit when the flux is below limit_sigma x flux_err x f_corr";

function typedArcsec(unit: RadiusUnit, typed: number | undefined): LogValue {
  return unit === "arcsec" ? requestedOrAuto(typed) : null;
}

function radiusRequestParams(opts: PhotometryRequestLog): LogRecord {
  const unit = opts.radiusUnit ?? "px";
  return {
    radius_unit: unit,
    aperture_radius_arcsec: typedArcsec(unit, opts.typedRadii?.aperture),
    annulus_inner_arcsec: typedArcsec(unit, opts.typedRadii?.skyInner),
    annulus_outer_arcsec: typedArcsec(unit, opts.typedRadii?.skyOuter),
    pixel_scale_arcsec: positiveOrNull(opts.pixelScaleArcsec),
  };
}

function calibrationSource(photcal: PhotCal | null): LogValue {
  if (!photcal) return null;
  const convention = photcal.convention;
  return convention.kind === "generic_zero_point" && convention.keyword === "user" ? "user" : "header";
}

function calibrationParams(opts: PhotometryRequestLog, res: PhotometryUnits): LogRecord {
  return {
    zero_point: opts.zeroPoint?.zp ?? null,
    zero_point_system: opts.zeroPoint?.system ?? null,
    zero_point_per_second: opts.zeroPoint?.per_second ?? null,
    calibration: calibrationText(res.photcal),
    calibration_source: calibrationSource(res.photcal),
    mag_system: res.photcal ? (res.photcal.mag_system ?? "ab") : null,
    bunit: photometryPixelUnit(res),
    ...(opts.limitSigma === undefined ? {} : { limit_sigma: opts.limitSigma, f_corr: opts.fCorr ?? null }),
  };
}

function calibrationNotes(photcal: PhotCal | null): string[] {
  return (photcal?.notes ?? []).map((text) => `calibration: ${text}`);
}

function arcsecRequestNotes(opts: PhotometryRequestLog): string[] {
  const scale = positiveOrNull(opts.pixelScaleArcsec);
  if (opts.radiusUnit !== "arcsec" || scale === null) return [];
  return [`radii typed in arcsec and converted to pixels with ${String(scale)} arcsec/px, the square root of the pixel area`];
}

export function photometryEntry(prov: MeasurementProvenance, res: PhotometryMeasurement, opts: PhotometryLogOptions): MeasurementLogDraft {
  const p = res.photometry;
  const magnitude = calibratedMagnitude(p, res.photcal);
  return {
    kind: "photometry",
    file: prov.file,
    image: prov.image,
    dq: dqHandlingOf(opts.excludeDq, res.masked),
    unit: photometrySumUnit(res),
    source: pixelSource(p.x, p.y),
    params: {
      aperture_radius_px: requestedOrAuto(opts.apertureRadius),
      annulus_inner_px: requestedOrAuto(opts.annulusInner),
      annulus_outer_px: requestedOrAuto(opts.annulusOuter),
      gain: gainOrNone(opts.gain),
      gaia_match: opts.gaiaMatch,
      aperture_effective_px: finiteOrNull(p.aperture_radius),
      sky_inner_effective_px: finiteOrNull(p.sky_inner),
      sky_outer_effective_px: finiteOrNull(p.sky_outer),
      photcal: res.photcal?.label ?? null,
      ...radiusRequestParams(opts),
      aperture_effective_arcsec: radiusInArcsec(p.aperture_radius, opts.pixelScaleArcsec ?? null),
      ...calibrationParams(opts, res),
    },
    values: {
      x: finiteOrNull(p.x),
      y: finiteOrNull(p.y),
      ra: finiteOrNull(res.sky?.ra),
      dec: finiteOrNull(res.sky?.dec),
      net_flux: finiteOrNull(p.net_flux),
      flux_err: finiteOrNull(p.flux_err),
      snr: finiteOrNull(p.snr),
      peak: finiteOrNull(p.peak),
      mag_inst: finiteOrNull(p.mag_inst),
      mag_ab: finiteOrNull(p.mag_ab),
      mag_ab_err: finiteOrNull(p.mag_ab_err),
      mag_ab_total: finiteOrNull(p.mag_ab_total),
      flux_jy: finiteOrNull(p.flux_jy),
      flux_err_jy: finiteOrNull(p.flux_err_jy),
      flux_total: finiteOrNull(p.flux_total),
      aperture_correction: finiteOrNull(p.aperture_correction),
      ee50_radius: finiteOrNull(p.ee50_radius),
      ee80_radius: finiteOrNull(p.ee80_radius),
      bg_mean: finiteOrNull(p.bg_mean),
      bg_sigma: finiteOrNull(p.bg_sigma),
      fwhm: positiveOrNull(p.fwhm),
      saturated: p.saturated,
      n_masked: finiteOrNull(p.n_masked),
      err_used: p.err_used,
      gaia_gmag: finiteOrNull(res.gaia?.gmag),
      gaia_bp_rp: finiteOrNull(res.gaia?.bp_rp),
      gaia_separation_arcsec: finiteOrNull(res.gaia?.separation_arcsec),
      n_nonfinite: finiteOrNull(p.n_nonfinite),
      ee_in_aperture: finiteOrNull(p.ee_in_aperture),
      mag: finiteOrNull(magnitude.mag),
      mag_err: finiteOrNull(magnitude.magErr),
      ...(opts.limit === undefined
        ? {}
        : {
            upper_limit_jy: opts.limit?.limitJy ?? null,
            upper_limit_ab: opts.limit?.limitAb ?? null,
            upper_limit_native: opts.limit?.limitNative ?? null,
          }),
    },
    notes: [
      ...measurementWarnings(res),
      ...calibrationNotes(res.photcal),
      photometryUnitNote(res),
      ...arcsecRequestNotes(opts),
      ...(opts.limit === undefined ? [] : [LIMIT_NOTE]),
    ],
  };
}

export interface BatchLogOptions extends PhotometryRequestLog {
  apertureRadius?: number;
  annulusInner?: number;
  annulusOuter?: number;
  gain?: number;
  excludeDq: boolean;
  from: "stars" | "regions" | "pasted" | "sky";
  placement?: { requested: number; onImage: number } | null;
  nLimits?: number;
  saved?: { path: string; rows: number } | null;
}

function skyLeftOutNotes(placement: BatchLogOptions["placement"]): string[] {
  if (!placement) return [];
  const outside = placement.requested - placement.onImage;
  return outside > 0 ? [`${outside} of ${placement.requested} sky positions are outside the image and were left out`] : [];
}

function batchRowsNote(saved: BatchLogOptions["saved"]): string {
  if (!saved) return BATCH_ROWS_NOTE;
  return `per-source rows saved to ${fileNameOf(saved.path) ?? saved.path} (${rowCount(saved.rows)})`;
}

export function batchPhotometryEntry(prov: MeasurementProvenance, res: BatchPhotometryResult, opts: BatchLogOptions): MeasurementLogDraft {
  const measured = res.rows.flatMap((row) => (row.photometry ? [row.photometry] : []));
  return {
    kind: "photometry_batch",
    file: prov.file,
    image: prov.image,
    dq: dqHandlingOf(opts.excludeDq, res.masked),
    unit: photometrySumUnit(res),
    source: JSON.stringify({ kind: "points", n: res.rows.length, from: opts.from }),
    params: {
      aperture_radius_px: requestedOrAuto(opts.apertureRadius),
      annulus_inner_px: requestedOrAuto(opts.annulusInner),
      annulus_outer_px: requestedOrAuto(opts.annulusOuter),
      gain: gainOrNone(opts.gain),
      photcal: res.photcal?.label ?? null,
      ...radiusRequestParams(opts),
      ...calibrationParams(opts, res),
      ...(opts.saved ? { csv_path: opts.saved.path, csv_rows: opts.saved.rows } : {}),
    },
    values: {
      n_points: res.rows.length,
      n_measured: res.n_measured,
      n_failed: res.n_failed,
      n_saturated: measured.filter((p) => p.saturated).length,
      median_net_flux: medianOf(measured.map((p) => p.net_flux)),
      median_snr: medianOf(measured.map((p) => p.snr)),
      median_fwhm: medianOf(measured.map((p) => positiveOrNull(p.fwhm))),
      n_with_nonfinite: measured.filter((p) => p.n_nonfinite > 0).length,
      ...(opts.placement
        ? { n_sky_requested: opts.placement.requested, n_sky_outside: opts.placement.requested - opts.placement.onImage }
        : {}),
      ...(opts.nLimits === undefined ? {} : { n_limits: opts.nLimits }),
    },
    notes: [
      ...batchWarnings(res),
      ...calibrationNotes(res.photcal),
      ...skyLeftOutNotes(opts.placement),
      ...arcsecRequestNotes(opts),
      batchRowsNote(opts.saved),
    ],
  };
}

function regionStatsValues(s: RegionStats): LogRecord {
  const calibrated = s.calibrated ?? null;
  const sky = regionSkyOf(s);
  return {
    count: finiteOrNull(s.count),
    n_excluded: finiteOrNull(s.n_excluded),
    n_nan: finiteOrNull(s.n_nan),
    area_px: finiteOrNull(s.area),
    mean: finiteOrNull(s.mean),
    median: finiteOrNull(s.median),
    sigma: finiteOrNull(s.sigma),
    std: finiteOrNull(s.std),
    mad: finiteOrNull(s.mad),
    min: finiteOrNull(s.min),
    max: finiteOrNull(s.max),
    sum: finiteOrNull(s.sum),
    sum_err: finiteOrNull(s.sum_err),
    net_sum: finiteOrNull(s.net_sum),
    net_snr: finiteOrNull(s.net_snr),
    clipped_mean: finiteOrNull(s.clipped_mean),
    clipped_sigma: finiteOrNull(s.clipped_sigma),
    n_rejected: finiteOrNull(s.n_rejected),
    flux_source: calibrated?.flux_source ?? null,
    flux_jy: finiteOrNull(calibrated?.flux_jy),
    flux_err_jy: finiteOrNull(calibrated?.flux_err_jy),
    mag_ab: finiteOrNull(calibrated?.mag_ab),
    mag_ab_err: finiteOrNull(calibrated?.mag_ab_err),
    sb_mag_arcsec2: finiteOrNull(calibrated?.sb_mag_arcsec2),
    area_arcsec2: finiteOrNull(sky?.area_arcsec2),
    ra: finiteOrNull(sky?.ra),
    dec: finiteOrNull(sky?.dec),
    pa_sky_deg: finiteOrNull(sky?.pa_sky_deg),
    net_err: finiteOrNull(s.net_err),
    net_err_terms: s.net_err_terms ?? null,
    n_padding: finiteOrNull(s.n_padding),
    bg_median: finiteOrNull(s.background?.median),
    bg_sigma: finiteOrNull(s.background?.sigma),
    bg_n: finiteOrNull(s.background?.count),
  };
}

export function regionStatsEntries(
  prov: MeasurementProvenance,
  measured: RegionStatsMeasured,
  stats: ReadonlyMap<string, RegionStatsEntry>,
  photcal: PhotCal | null,
): MeasurementLogDraft[] {
  const byId = new Map(measured.regions.map((r) => [r.id, r]));
  const dq = dqHandlingOf(measured.excludeDq, measured.dqExcluded !== null);
  const params: LogRecord = {
    sigma: measured.sigma ?? DEFAULT_CLIP_SIGMA,
    maxiters: measured.maxiters ?? DEFAULT_CLIP_ITERS,
    dq_excluded_px: measured.dqExcluded,
    region_excluded_px: measured.regionExcluded,
    photcal: photcal?.label ?? null,
  };
  const ownParams: LogRecord = { ...params, region_excluded_px: 0 };
  const pixelUnit = photcal?.bunit ?? measured.bunit ?? null;
  const sumUnit = measured.summedUnit ?? summedFluxUnit(photcal);
  const sumUnitNotes = sumUnit !== null && sumUnit !== pixelUnit ? [`sum, sum_err, net_sum, net_err in ${sumUnit}`] : [];
  const commandNotes = measured.notes ?? [];
  return measured.regions.map((region) => {
    const entry = stats.get(region.id);
    const s = entry?.stats ?? null;
    const background = region.backgroundId ? (byId.get(region.backgroundId)?.shape ?? null) : null;
    const own = isExcludeRegion(region);
    const notes = s ? [] : [entry?.error ?? NO_STATISTICS_NOTE];
    return {
      kind: "region_stats",
      file: prov.file,
      image: prov.image,
      dq,
      unit: pixelUnit,
      source: regionSource(region.props.text, region.shape, background),
      params: own ? ownParams : params,
      values: s ? regionStatsValues(s) : {},
      notes: [...notes, ...(own ? [EXCLUDE_ROW_NOTE] : []), ...(s ? sumUnitNotes : []), ...commandNotes],
    };
  });
}

export function radialProfileEntry(
  prov: MeasurementProvenance,
  data: RadialProfile,
  excludeDq: boolean,
  req: RadialLogRequest,
  region: Region,
  pixelUnit?: string | null,
): MeasurementLogDraft {
  const last = data.bins.length > 0 ? data.bins[data.bins.length - 1] : null;
  const unit = pixelUnitOf(pixelUnit);
  return {
    kind: "radial_profile",
    file: prov.file,
    image: prov.image,
    dq: profileDqHandling(excludeDq, data),
    unit,
    source: regionSource(region.props.text, region.shape, null),
    params: {
      x: finiteOrNull(data.x),
      y: finiteOrNull(data.y),
      max_radius_px: finiteOrNull(data.max_radius),
      background_inner_px: req.background ? finiteOrNull(req.background[0]) : null,
      background_outer_px: req.background ? finiteOrNull(req.background[1]) : null,
      region_excluded_px: data.region_excluded,
      exclude_regions: req.exclude.length,
    },
    values: {
      n_bins: data.bins.length,
      background_median: finiteOrNull(data.background?.median),
      background_sigma: finiteOrNull(data.background?.sigma),
      background_count: finiteOrNull(data.background?.count),
      cumulative_sum_last: finiteOrNull(last?.cumulative_sum),
    },
    notes: summedUnitNotes(["cumulative_sum_last"], unit),
  };
}

export function sbProfileEntry(
  prov: MeasurementProvenance,
  sb: SbProfile,
  excludeDq: boolean,
  req: SbLogRequest,
  region: Region,
): MeasurementLogDraft {
  return {
    kind: "sb_profile",
    file: prov.file,
    image: prov.image,
    dq: profileDqHandling(excludeDq, sb),
    unit: sb.photcal?.bunit ?? null,
    source: regionSource(region.props.text, region.shape, req.background),
    params: {
      x: finiteOrNull(sb.x),
      y: finiteOrNull(sb.y),
      sma_max_px: finiteOrNull(sb.sma_max),
      ellipticity: finiteOrNull(sb.ellipticity),
      angle_deg: finiteOrNull(sb.angle_deg),
      bin_width_px: finiteOrNull(sb.bin_width),
      photcal: sb.photcal?.label ?? null,
      region_excluded_px: sb.region_excluded,
      exclude_regions: req.exclude.length,
    },
    values: {
      r50_px: finiteOrNull(sb.r50_px),
      r80_px: finiteOrNull(sb.r80_px),
      r90_px: finiteOrNull(sb.r90_px),
      petrosian_radius_px: finiteOrNull(sb.petrosian_radius_px),
      total_mag_ab: finiteOrNull(sb.total_mag_ab),
      mu_0_mag_arcsec2: firstSurfaceBrightness(sb),
      sky_pa_deg: finiteOrNull(sb.sky_pa_deg),
      pixel_scale_arcsec: finiteOrNull(sb.pixel_scale_arcsec),
      n_bins: sb.bins.length,
      background_median: finiteOrNull(sb.background?.median),
      background_sigma: finiteOrNull(sb.background?.sigma),
      background_count: finiteOrNull(sb.background?.count),
    },
    notes: [...sb.notes, ...sb.calibration_warnings],
  };
}

export function lineCutEntry(
  prov: MeasurementProvenance,
  cut: LineCut,
  excludeDq: boolean,
  region: Region,
  exclude: readonly RegionShape[],
  pixelUnit?: string | null,
): MeasurementLogDraft {
  const finite = cut.values.filter((v): v is number => typeof v === "number" && Number.isFinite(v));
  return {
    kind: "line_cut",
    file: prov.file,
    image: prov.image,
    dq: profileDqHandling(excludeDq, cut),
    unit: pixelUnitOf(pixelUnit),
    source: regionSource(region.props.text, region.shape, null),
    params: {
      x1: finiteOrNull(cut.x1),
      y1: finiteOrNull(cut.y1),
      x2: finiteOrNull(cut.x2),
      y2: finiteOrNull(cut.y2),
      region_excluded_px: cut.region_excluded,
      exclude_regions: exclude.length,
    },
    values: {
      length_px: finiteOrNull(cut.length),
      n_samples: cut.n_samples,
      value_min: finite.length > 0 ? Math.min(...finite) : null,
      value_max: finite.length > 0 ? Math.max(...finite) : null,
    },
    notes: [],
  };
}

export function skySeparationEntry(
  prov: MeasurementProvenance,
  line: Extract<RegionShape, { shape: "line" }>,
  sky: SkySeparationResult,
): MeasurementLogDraft {
  return {
    kind: "sky_separation",
    file: prov.file,
    image: prov.image,
    dq: "not_applied",
    unit: "arcsec",
    source: regionSource(null, line, null),
    params: { x1: line.x1, y1: line.y1, x2: line.x2, y2: line.y2 },
    values: {
      a_ra: finiteOrNull(sky.a_sky[0]),
      a_dec: finiteOrNull(sky.a_sky[1]),
      b_ra: finiteOrNull(sky.b_sky[0]),
      b_dec: finiteOrNull(sky.b_sky[1]),
      separation_arcsec: finiteOrNull(sky.separation_arcsec),
      separation_arcmin: finiteOrNull(sky.separation_arcmin),
      separation_deg: finiteOrNull(sky.separation_deg),
      position_angle_deg: finiteOrNull(sky.position_angle_deg),
      pixel_length: finiteOrNull(sky.pixel_length),
      pixel_scale_arcsec: finiteOrNull(sky.pixel_scale_arcsec),
    },
    notes: [],
  };
}

export function lineEntry(
  filePath: string,
  result: LineMeasurement,
  model: LineModel,
  dqRequested: boolean | null = null,
): MeasurementLogDraft {
  const velocity = result.velocity;
  const fit = result.fit;
  const terms = result.flux_err_terms;
  return {
    kind: "line",
    file: fileNameOf(filePath),
    image: result.source.kind === "file" || result.source.kind === "series" ? SPECTRUM_FILE_IMAGE : LOADED_CUBE_IMAGE,
    dq: dqRequested === null ? "not_applied" : dqHandlingOf(dqRequested, result.n_dq_excluded > 0),
    unit: result.flux_unit,
    source: JSON.stringify(result.source),
    params: {
      z0: result.z0,
      z1: result.z1,
      n_channels: result.n_channels,
      continuum_windows: JSON.stringify(result.continuum_windows),
      continuum_linear: result.continuum_linear,
      continuum_channels: result.continuum_channels,
      model,
      axis_unit: result.axis_unit,
      spectrum_unit: spectrumUnitOf(result.flux_unit, result.axis_unit),
      rest_um: finiteOrNull(velocity?.rest_um),
      convention: velocity?.convention ?? null,
      shift_applied_kms: finiteOrNull(velocity?.shift_applied_kms),
      axis_frame: velocity?.axis_frame ?? null,
      err_used: result.err_used,
      err_source: result.err_source,
      redshift: finiteOrNull(velocity?.redshift),
      n_channels_nominal: finiteOrNull(result.n_channels_nominal),
      line_id: result.line_id ?? null,
      line_label: result.line_label ?? null,
      redshift_requested: finiteOrNull(result.redshift),
      wavelength_frame: result.wavelength_frame ?? null,
      errors_reliable: result.errors_reliable ?? null,
      width_errors_reliable: result.width_errors_reliable ?? null,
      flux_physical_basis: result.flux_physical_basis ?? null,
    },
    values: {
      flux: finiteOrNull(result.flux),
      flux_err: finiteOrNull(result.flux_err),
      flux_err_pixel: finiteOrNull(terms?.pixel),
      flux_err_level: finiteOrNull(terms?.level),
      flux_err_slope: finiteOrNull(terms?.slope),
      n_dq_excluded: result.n_dq_excluded,
      n_nonfinite: result.n_nonfinite,
      n_bad_err: result.n_bad_err,
      equivalent_width: finiteOrNull(result.equivalent_width),
      centroid: finiteOrNull(result.centroid),
      sigma: finiteOrNull(result.sigma),
      fwhm: finiteOrNull(result.fwhm),
      peak: finiteOrNull(result.peak),
      peak_channel: finiteOrNull(result.peak_channel),
      snr: finiteOrNull(result.snr),
      continuum_level: finiteOrNull(result.continuum_level),
      continuum_slope: finiteOrNull(result.continuum_slope),
      continuum_sigma: finiteOrNull(result.continuum_sigma),
      centroid_kms: finiteOrNull(velocity?.centroid_kms),
      sigma_kms: finiteOrNull(velocity?.sigma_kms),
      fwhm_kms: finiteOrNull(velocity?.fwhm_kms),
      fit_amplitude: finiteOrNull(fit?.amplitude),
      fit_centre: finiteOrNull(fit?.centre),
      fit_sigma: finiteOrNull(fit?.sigma),
      fit_centre_err: finiteOrNull(fit?.centre_err),
      fit_sigma_err: finiteOrNull(fit?.sigma_err),
      fit_chi2: finiteOrNull(fit?.chi2),
      fit_dof: finiteOrNull(fit?.dof),
      fit_converged: fit ? fit.converged : null,
      source_centroid_kms: finiteOrNull(velocity?.source_centroid_kms),
      source_sigma_kms: finiteOrNull(velocity?.source_sigma_kms),
      source_fwhm_kms: finiteOrNull(velocity?.source_fwhm_kms),
      continuum_chi2_red: finiteOrNull(result.continuum_chi2_red),
      equivalent_width_err: finiteOrNull(result.equivalent_width_err),
      centroid_err: finiteOrNull(result.centroid_err),
      sigma_err: finiteOrNull(result.sigma_err),
      fwhm_err: finiteOrNull(result.fwhm_err),
      centroid_err_kms: finiteOrNull(velocity?.centroid_err_kms),
      sigma_err_kms: finiteOrNull(velocity?.sigma_err_kms),
      fwhm_err_kms: finiteOrNull(velocity?.fwhm_err_kms),
      source_centroid_err_kms: finiteOrNull(velocity?.source_centroid_err_kms),
      source_sigma_err_kms: finiteOrNull(velocity?.source_sigma_err_kms),
      source_fwhm_err_kms: finiteOrNull(velocity?.source_fwhm_err_kms),
      z_line: finiteOrNull(velocity?.z_line),
      z_line_err: finiteOrNull(velocity?.z_line_err),
      peak_axis: finiteOrNull(result.peak_axis),
      flux_si: finiteOrNull(result.flux_si),
      flux_si_err: finiteOrNull(result.flux_si_err),
      flux_cgs: finiteOrNull(result.flux_cgs),
      flux_cgs_err: finiteOrNull(result.flux_cgs_err),
      fit_amplitude_err: finiteOrNull(fit?.amplitude_err),
      fit_iterations: finiteOrNull(fit?.iterations),
    },
    notes: [...result.notes, lineUnitNote(result.flux_unit, result.axis_unit)],
  };
}

export interface TimeSeriesLogOptions {
  apertureRadius: number;
  annulusInner?: number;
  annulusOuter?: number;
  gain?: number;
  excludeDq: boolean;
  trackDrift: boolean;
  compIdx?: readonly number[];
}

function extremeFrame(frames: TimeSeriesResult["frames"], pick: (f: TimeSeriesResult["frames"][number]) => number | null, better: (a: number, b: number) => boolean) {
  let best: { frame: TimeSeriesResult["frames"][number]; value: number } | null = null;
  for (const frame of frames) {
    const value = pick(frame);
    if (value === null || !Number.isFinite(value)) continue;
    if (best === null || better(value, best.value)) best = { frame, value };
  }
  return best;
}

export function timeSeriesEntry(res: TimeSeriesResult, opts: TimeSeriesLogOptions): MeasurementLogDraft {
  const targetIdx = res.targets.findIndex((t) => t.role === "target");
  const allComps = res.targets.map((t, i) => (t.role === "comp" ? i : -1)).filter((i) => i >= 0);
  const inUse = opts.compIdx;
  const compIdx = inUse ? allComps.filter((i) => inUse.includes(i)) : allComps;
  const leftOut = allComps.filter((i) => !compIdx.includes(i));
  const leftOutLabels = leftOut.map((i) => res.targets[i].label);
  const checkIdx = res.targets.map((t, i) => (t.role === "check" ? i : -1)).filter((i) => i >= 0);
  const targetCurve = targetIdx >= 0 ? lightCurve(res, targetIdx, compIdx, "index") : [];
  const reference = checkReference(res, compIdx, checkIdx, "index");
  const used = res.frames.filter((f) => f.skipped === null);
  const timed = hasCompleteTimeAxis(res);
  const first = timed ? extremeFrame(used, (f) => f.jd_mid, (a, b) => a < b) : null;
  const last = timed ? extremeFrame(used, (f) => f.jd_mid, (a, b) => a > b) : null;
  const bjdComplete = used.length > 0 && used.every((f) => finiteOrNull(f.geometry?.bjd_tdb) !== null);
  const bjdFirst = bjdComplete ? extremeFrame(used, (f) => f.geometry?.bjd_tdb ?? null, (a, b) => a < b) : null;
  const bjdLast = bjdComplete ? extremeFrame(used, (f) => f.geometry?.bjd_tdb ?? null, (a, b) => a > b) : null;
  const referenceName = fileNameOf(res.reference_path);
  return {
    kind: "time_series",
    file: referenceName,
    image: `${res.n_frames} frames`,
    dq: dqHandlingOf(opts.excludeDq, null),
    unit: "mag",
    source: JSON.stringify({
      kind: "frames",
      n: res.n_frames,
      reference: referenceName,
      targets: res.targets.map((t, i) => ({ x: t.x, y: t.y, label: t.label, role: t.role, ...(leftOut.includes(i) ? { left_out: true } : {}) })),
    }),
    params: {
      n_frames: res.n_frames,
      n_skipped: res.n_skipped,
      aperture_radius_px: opts.apertureRadius,
      annulus_inner_px: requestedOrAuto(opts.annulusInner),
      annulus_outer_px: requestedOrAuto(opts.annulusOuter),
      gain: gainOrNone(opts.gain),
      track_drift: opts.trackDrift,
      target_label: targetIdx >= 0 ? res.targets[targetIdx].label : null,
      n_comp: compIdx.length,
      n_check: checkIdx.length,
      first_frame: first ? first.frame.file_name : (res.frames[0]?.file_name ?? null),
      last_frame: last ? last.frame.file_name : (res.frames[res.frames.length - 1]?.file_name ?? null),
      bjd_source: bjdSources(used),
      comps_left_out: leftOutLabels.length > 0 ? leftOutLabels.join("; ") : null,
      target_ra_deg: res.geometry_target ? res.geometry_target.ra_deg : null,
      target_dec_deg: res.geometry_target ? res.geometry_target.dec_deg : null,
      target_source: res.geometry_target ? res.geometry_target.source : null,
    },
    values: {
      n_used: targetCurve.filter((p) => p.mag !== null).length,
      target_rms_mag: seriesRms(targetCurve),
      check_rms_mag: reference ? seriesRms(reference.curve) : null,
      check_label: reference ? (res.targets[reference.index]?.label ?? null) : null,
      jd_mid_first: first ? first.value : null,
      jd_mid_last: last ? last.value : null,
      time_span_days: first && last ? last.value - first.value : null,
      time_source: timed ? referenceTimeSource(res) : null,
      bjd_tdb_first: bjdFirst ? bjdFirst.value : null,
      bjd_tdb_last: bjdLast ? bjdLast.value : null,
      n_frames_comp_saturated: used.filter((f) => saturatedCompLabels(f, res.targets, compIdx).length > 0).length,
    },
    notes: [
      ...res.warnings,
      ...saturatedComps(res).map(
        (comp) => `comparison star ${comp.label} saturated on ${comp.frames} ${comp.frames === 1 ? "frame" : "frames"}${leftOut.includes(comp.index) ? ", left out of the ensemble" : ""}`,
      ),
      ...(leftOut.length > 0 ? [`ensemble: ${compIdx.length} of ${allComps.length} comparison stars (left out: ${leftOutLabels.join(", ")})`] : []),
    ],
  };
}

export function pixelEntry(prov: MeasurementProvenance, result: PixelTableResult): MeasurementLogDraft {
  const half = Math.floor(result.size / 2);
  return {
    kind: "pixel",
    file: prov.file,
    image: prov.image,
    dq: "reported",
    unit: result.unit,
    source: pixelSource(result.x, result.y),
    params: { size: result.size, dq_table: result.dq_table },
    values: {
      x: result.x,
      y: result.y,
      value: finiteOrNull(result.values[half]?.[half]),
      err: finiteOrNull(result.err?.[half]?.[half]),
      dq_value: finiteOrNull(result.dq?.[half]?.[half]),
      dq_names: result.dq_names?.[half]?.[half] ?? null,
      wavelength: finiteOrNull(result.wavelength?.[half]?.[half]),
      wavelength_unit: result.wavelength_unit ?? null,
      window_min: finiteOrNull(result.stats.min),
      window_max: finiteOrNull(result.stats.max),
      window_mean: finiteOrNull(result.stats.mean),
      window_median: finiteOrNull(result.stats.median),
      n_finite: result.stats.n_finite,
      n_nan: result.stats.n_nan,
    },
    notes: [],
  };
}

export function statisticsEntry(prov: MeasurementProvenance, input: StatisticsLogInput): MeasurementLogDraft {
  const prefixed = input.channels.length > 1;
  const values: Record<string, LogValue> = {};
  for (const channel of input.channels) {
    const prefix = prefixed ? `${channel.label.toLowerCase()}_` : "";
    const s = channel.body.statistics;
    const noise = channel.body.noise;
    values[`${prefix}count`] = finiteOrNull(s.count);
    values[`${prefix}mean`] = finiteOrNull(s.mean);
    values[`${prefix}median`] = finiteOrNull(s.median);
    values[`${prefix}std_dev`] = finiteOrNull(s.std_dev);
    values[`${prefix}mad`] = finiteOrNull(s.mad);
    values[`${prefix}avg_dev`] = finiteOrNull(s.avg_dev);
    values[`${prefix}bwmv_sqrt`] = finiteOrNull(s.bwmv_sqrt);
    values[`${prefix}min`] = finiteOrNull(s.min);
    values[`${prefix}max`] = finiteOrNull(s.max);
    values[`${prefix}sum`] = finiteOrNull(s.sum);
    values[`${prefix}nan_count`] = finiteOrNull(s.nan_count);
    values[`${prefix}excluded`] = finiteOrNull(s.excluded);
    values[`${prefix}noise_sigma`] = finiteOrNull(noise?.sigma);
    values[`${prefix}noise_fraction`] = finiteOrNull(noise?.fraction);
  }
  return {
    kind: "statistics",
    file: prov.file,
    image: prov.image,
    dq: input.composite ? "not_applied" : dqHandlingOf(input.excludeDq, input.masked),
    unit: input.unit,
    source: input.region ? regionSource(null, input.region, null) : IMAGE_SOURCE,
    params: { noise: input.noise, region_kind: input.regionKind, dq_excluded_px: input.dqExcluded },
    values,
    notes: [
      ...input.channels.flatMap((c) => (c.body.noise_note ? [prefixed ? `${c.label}: ${c.body.noise_note}` : c.body.noise_note] : [])),
      ...summedUnitNotes(
        input.channels.map((c) => (prefixed ? `${c.label.toLowerCase()}_sum` : "sum")),
        pixelUnitOf(input.unit),
      ),
    ],
  };
}

export function crossMatchEntry(
  filePath: string,
  res: CrossMatchResult,
  opts: { sigma: number; maxStars: number; colourTerm: boolean },
): MeasurementLogDraft {
  const zp = res.zero_point;
  const astrometry = res.astrometry;
  const fixedRadius = res.aperture?.kind === "fixed" ? res.aperture.radius_px : null;
  const autoAperture = res.aperture?.kind === "auto" ? res.aperture : null;
  const query = res.catalog_query ?? null;
  return {
    kind: "catalog_crossmatch",
    file: fileNameOf(filePath),
    image: LOADED_FILE_IMAGE,
    dq: "not_applied",
    unit: "mag (Vega)",
    source: FILE_SOURCE,
    params: {
      band: res.band,
      match_radius_arcsec: finiteOrNull(res.match_radius_arcsec),
      colour_term: opts.colourTerm,
      detection_sigma: finiteOrNull(res.detection_sigma) ?? opts.sigma,
      max_stars: finiteOrNull(res.max_stars) ?? opts.maxStars,
      epoch_year: finiteOrNull(res.epoch_year),
      aperture_radius_px: fixedRadius ?? (autoAperture ? AUTO : null),
      aperture_effective_px: fixedRadius ?? finiteOrNull(autoAperture?.median_radius_px),
      aperture_fwhm_factor: autoAperture ? finiteOrNull(autoAperture.fwhm_factor) : null,
      aperture_min_px: fixedRadius ?? finiteOrNull(autoAperture?.min_radius_px),
      aperture_max_px: fixedRadius ?? finiteOrNull(autoAperture?.max_radius_px),
      catalogue: GAIA_CATALOGUE,
      catalogue_table: res.catalog_table ?? null,
      catalogue_source: res.catalog_source ?? null,
      magnitude_system: "Vega",
      cone_radius_arcmin: finiteOrNull(query?.radius_arcmin),
      g_limit_mag: finiteOrNull(query?.mag_limit),
      max_rows: finiteOrNull(query?.max_rows),
    },
    values: {
      n_detected: res.n_detected,
      n_catalog: res.n_catalog,
      n_matches: res.matches.length,
      zp: finiteOrNull(zp?.zp),
      zp_err: finiteOrNull(zp?.zp_err),
      colour_coeff: finiteOrNull(zp?.colour_coeff),
      zp_rms: finiteOrNull(zp?.rms),
      n_used: finiteOrNull(zp?.n_used),
      n_rejected: finiteOrNull(zp?.n_rejected),
      n_without_colour: finiteOrNull(zp?.n_without_colour),
      colour_term_used: zp ? zp.colour_term_used : null,
      median_d_ra_arcsec: finiteOrNull(astrometry?.median_d_ra_arcsec),
      median_d_dec_arcsec: finiteOrNull(astrometry?.median_d_dec_arcsec),
      astrometry_rms_arcsec: finiteOrNull(astrometry?.rms_arcsec),
      astrometry_n: finiteOrNull(astrometry?.n),
      photcal_present: res.photcal_present,
      n_measured: res.sources.length,
      row_cap_reached: query ? query.row_cap_reached : null,
      cone_smaller_than_field: query ? query.cone_smaller_than_field : null,
    },
    notes: [...res.warnings],
  };
}

export function spectrumExportEntry(filePath: string, input: SpectrumExportInput, savedPath: string | null): MeasurementLogDraft {
  const summary = spectrumExportSummary(input);
  const fluxUnit = summary.params.flux_unit;
  const isFile = input.source.kind === "file";
  const region = input.source.kind === "region" ? input.region : null;
  const regionDq = region?.n_dq_excluded ?? null;
  const dq: DqHandling =
    typeof summary.params.bin_factor === "number"
      ? dqHandlingOf(summary.params.bin_dq_requested === true, Number(summary.params.bin_dq_excluded) > 0)
      : isFile && summary.params.dq_column === true
        ? "reported"
        : regionDq !== null
          ? dqHandlingOf(true, regionDq.some((n) => n > 0))
          : "not_applied";
  const warning = region?.noise_factor?.warning ?? null;
  return {
    kind: "spectrum_export",
    file: fileNameOf(filePath),
    image: isFile ? SPECTRUM_FILE_IMAGE : LOADED_CUBE_IMAGE,
    dq,
    unit: typeof fluxUnit === "string" ? fluxUnit : null,
    source: JSON.stringify(input.source),
    params: { ...summary.params, saved_path: savedPath, ...(isFile ? { document_path: filePath } : {}) },
    values: { ...summary.values },
    notes: [...(input.pixelFluxJyError !== null ? [input.pixelFluxJyError] : []), ...(warning !== null ? [warning] : [])],
  };
}

export function spectrumCompareEntry(filePath: string, csvInput: ComparisonCsvInput, savedPath: string | null): MeasurementLogDraft {
  return {
    kind: "spectrum_compare",
    file: fileNameOf(filePath),
    image: hasDocumentEntry(csvInput.entries) ? SPECTRUM_FILE_IMAGE : LOADED_CUBE_IMAGE,
    dq: "not_applied",
    unit: null,
    source: FILE_SOURCE,
    params: {
      normalisation: csvInput.mode,
      offset_step: csvInput.mode === "offset" ? finiteOrNull(csvInput.result.step) : null,
      view: csvInput.view,
      saved_path: savedPath,
      x1d_path: csvInput.table?.path ?? null,
      x1d_hdu: csvInput.table?.hdu ?? null,
      x1d_dq_rows_dropped: csvInput.table?.resampled.droppedDq ?? null,
    },
    values: { n_series: csvInput.result.plotted.length },
    notes: csvInput.result.omitted.map((o) => `${o.label}: ${o.reason}`),
  };
}

export function pvEntry(run: PvRun): MeasurementLogDraft {
  const { params, result } = run;
  const line = params.line;
  const ridge = result.ridge.filter((v): v is number => typeof v === "number" && Number.isFinite(v));
  return {
    kind: "pv",
    file: fileNameOf(params.filePath),
    image: LOADED_CUBE_IMAGE,
    dq: "not_applied",
    unit: result.spectral_unit,
    source: regionSource(null, { shape: "line", x1: line.x0, y1: line.y0, x2: line.x1, y2: line.y1 }, null),
    params: {
      step_px: params.stepPx,
      width_px: params.widthPx,
      z0: params.z0,
      z1: params.z1,
      mode: params.mode,
      convention: params.convention,
      velocity_shift_kms: finiteOrNull(params.velocityShiftKms),
      correction: params.correction,
      rest_um: finiteOrNull(result.spectral_rest_um),
      offset_unit: result.offset_unit,
      spectral_unit: result.spectral_unit,
      fits_path: result.fits_path,
    },
    values: {
      n_offsets: result.summary.n_offsets,
      n_channels: result.summary.n_channels,
      ridge_min: ridge.length > 0 ? Math.min(...ridge) : null,
      ridge_max: ridge.length > 0 ? Math.max(...ridge) : null,
    },
    notes: [...result.notes],
  };
}

export function lineFitEntry(run: LineFitRun): MeasurementLogDraft {
  const { config, result } = run;
  return {
    kind: "line_fit",
    file: fileNameOf(run.filePath),
    image: LOADED_CUBE_IMAGE,
    dq: dqHandlingOf(config.use_dq, result.dq_hdu !== null),
    unit: result.units.flux,
    source: JSON.stringify({ kind: "cube", z0: result.z0, z1: result.z1 }),
    params: {
      z0: result.z0,
      z1: result.z1,
      n_channels: result.n_channels,
      rest_um: finiteOrNull(result.rest_um),
      convention: result.convention,
      continuum_windows: JSON.stringify(result.continuum_windows),
      weighting: result.weighting,
      err_hdu: finiteOrNull(result.err_hdu),
      dq_hdu: finiteOrNull(result.dq_hdu),
      use_err: config.use_err,
      use_dq: config.use_dq,
      emission_only: config.emission_only,
      snr_threshold: finiteOrNull(result.snr_threshold),
      resolving_power: finiteOrNull(result.resolving_power),
      components: result.components,
      line: run.lineLabel,
      sigma_label: result.sigma_label,
      fits_velocity: result.planes.velocity?.fits_path ?? null,
      fits_flux: result.planes.flux?.fits_path ?? null,
      redshift: finiteOrNull(config.redshift ?? 0),
      fits_source_velocity: result.planes.source_velocity?.fits_path ?? null,
      fits_source_flux: result.planes.source_flux?.fits_path ?? null,
    },
    values: {
      n_fit: result.n_fit,
      n_masked: result.n_masked,
      n_const_continuum: result.n_const_continuum,
      n_two_components: result.n_two_components,
      median_chi2_red: finiteOrNull(result.median_chi2_red),
      elapsed_ms: result.elapsed_ms,
    },
    notes: [...result.notes],
  };
}
