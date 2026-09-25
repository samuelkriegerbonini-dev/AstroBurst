import type { BatchPhotometryResult, PhotCal, PhotometryMeasurement } from "../services/analysis";
import type { PixelTableResult, TimeSeriesResult } from "../shared/types/analysis";
import type { SkySeparationResult } from "../shared/types/astrometry";
import type { CrossMatchResult } from "../shared/types/catalog";
import type { PvRun } from "../shared/types/pv";
import type { LineCut, RadialProfile, Region, RegionShape, RegionStats, RegionStatsEntry, SbProfile } from "../shared/types/regions";
import type { LineMeasurement, LineModel } from "../shared/types/spectral";
import type { ChannelStatisticsBody } from "../shared/types/statistics";
import type { MeasurementSource } from "./analysisTarget";
import { buildCsv, type CsvColumn } from "./catalogCsv";
import { checkReference, hasCompleteTimeAxis, lightCurve, referenceTimeSource, seriesRms } from "./differentialPhotometry";
import { generateId } from "./format";
import { exportStem, parseImageRef } from "./imageRef";
import { regionSkyOf } from "./regionCsv";
import { firstSurfaceBrightness } from "./sbProfile";
import type { ComparisonCsvInput } from "./spectrumCompare";
import { spectrumExportSummary, type SpectrumExportInput } from "./spectrumExport";

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
] as const;

const DEFAULT_CLIP_SIGMA = 3;
const DEFAULT_CLIP_ITERS = 5;
const AUTO = "auto";
const NO_GAIN = "none";
const DEFAULT_CSV_NAME = "astroburst_measurements.csv";
const CSV_SUFFIX = "_measurements.csv";
const NOTE_SEPARATOR = " | ";
const NULL_TEXT = "--";
const SUMMARY_DIGITS = 6;
const NO_STATISTICS_NOTE = "no statistics";
const LOADED_CUBE_IMAGE = "loaded cube";
const LOADED_FILE_IMAGE = "loaded file";
const ORIGINAL_IMAGE = "original";
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

export function imageLabelOf(source: MeasurementSource | null): string {
  return source?.text ?? ORIGINAL_IMAGE;
}

export function fileNameOf(path: string | null): string | null {
  if (path === null) return null;
  const name = parseImageRef(path).path.split(/[\\/]/).pop() ?? "";
  return name === "" ? null : name;
}

export function finiteOrNull(v: number | null | undefined): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

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
  return `flux, flux_err in ${fluxUnit}; peak, continuum_level, continuum_sigma, fit_amplitude in the spectrum unit (${spectrumUnit}); continuum_slope in the spectrum unit per ${axisUnit}; centroid, sigma, fwhm, equivalent_width, fit_centre, fit_sigma, fit_centre_err, fit_sigma_err in ${axisUnit}; centroid_kms, sigma_kms, fwhm_kms in km/s`;
}

export interface RadialLogRequest {
  x: number;
  y: number;
  maxRadius: number;
  background: [number, number] | null;
}

export interface SbLogRequest {
  shape: RegionShape;
  background: RegionShape | null;
  binWidth: number;
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

export function measurementLogCsv(entries: readonly MeasurementLogEntry[]): string {
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
  return buildCsv(columns, [...entries]);
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
  elapsedMs: number;
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

interface PhotometryLogOptions {
  apertureRadius?: number;
  annulusInner?: number;
  annulusOuter?: number;
  gain?: number;
  gaiaMatch: boolean;
  excludeDq: boolean;
}

export function photometryEntry(prov: MeasurementProvenance, res: PhotometryMeasurement, opts: PhotometryLogOptions): MeasurementLogDraft {
  const p = res.photometry;
  return {
    kind: "photometry",
    file: prov.file,
    image: prov.image,
    dq: dqHandlingOf(opts.excludeDq, res.masked),
    unit: res.photcal?.bunit ?? null,
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
    },
    notes: [...res.warnings],
  };
}

interface BatchLogOptions {
  apertureRadius?: number;
  annulusInner?: number;
  annulusOuter?: number;
  gain?: number;
  excludeDq: boolean;
  from: "stars" | "regions" | "pasted";
}

export function batchPhotometryEntry(prov: MeasurementProvenance, res: BatchPhotometryResult, opts: BatchLogOptions): MeasurementLogDraft {
  const measured = res.rows.flatMap((row) => (row.photometry ? [row.photometry] : []));
  return {
    kind: "photometry_batch",
    file: prov.file,
    image: prov.image,
    dq: dqHandlingOf(opts.excludeDq, res.masked),
    unit: res.photcal?.bunit ?? null,
    source: JSON.stringify({ kind: "points", n: res.rows.length, from: opts.from }),
    params: {
      aperture_radius_px: requestedOrAuto(opts.apertureRadius),
      annulus_inner_px: requestedOrAuto(opts.annulusInner),
      annulus_outer_px: requestedOrAuto(opts.annulusOuter),
      gain: gainOrNone(opts.gain),
      photcal: res.photcal?.label ?? null,
    },
    values: {
      n_points: res.rows.length,
      n_measured: res.n_measured,
      n_failed: res.n_failed,
      n_saturated: measured.filter((p) => p.saturated).length,
      median_net_flux: medianOf(measured.map((p) => p.net_flux)),
      median_snr: medianOf(measured.map((p) => p.snr)),
      median_fwhm: medianOf(measured.map((p) => positiveOrNull(p.fwhm))),
    },
    notes: [...res.warnings],
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
  };
}

export function regionStatsEntries(
  prov: MeasurementProvenance,
  measured: RegionStatsMeasured,
  stats: ReadonlyMap<string, RegionStatsEntry>,
  photcal: PhotCal | null,
): MeasurementLogDraft[] {
  const byId = new Map(measured.regions.map((r) => [r.id, r]));
  const dq = dqHandlingOf(measured.excludeDq, measured.masked);
  const params: LogRecord = {
    sigma: measured.sigma ?? DEFAULT_CLIP_SIGMA,
    maxiters: measured.maxiters ?? DEFAULT_CLIP_ITERS,
    dq_excluded_px: measured.dqExcluded,
    photcal: photcal?.label ?? null,
  };
  return measured.regions.map((region) => {
    const entry = stats.get(region.id);
    const s = entry?.stats ?? null;
    const background = region.backgroundId ? (byId.get(region.backgroundId)?.shape ?? null) : null;
    return {
      kind: "region_stats",
      file: prov.file,
      image: prov.image,
      dq,
      unit: photcal?.bunit ?? null,
      source: regionSource(region.props.text, region.shape, background),
      params,
      values: s ? regionStatsValues(s) : {},
      notes: s ? [] : [entry?.error ?? NO_STATISTICS_NOTE],
    };
  });
}

export function radialProfileEntry(
  prov: MeasurementProvenance,
  data: RadialProfile,
  excludeDq: boolean,
  req: RadialLogRequest,
  region: Region,
): MeasurementLogDraft {
  const last = data.bins.length > 0 ? data.bins[data.bins.length - 1] : null;
  return {
    kind: "radial_profile",
    file: prov.file,
    image: prov.image,
    dq: dqHandlingOf(excludeDq, data.masked),
    unit: null,
    source: regionSource(region.props.text, region.shape, null),
    params: {
      x: finiteOrNull(data.x),
      y: finiteOrNull(data.y),
      max_radius_px: finiteOrNull(data.max_radius),
      background_inner_px: req.background ? finiteOrNull(req.background[0]) : null,
      background_outer_px: req.background ? finiteOrNull(req.background[1]) : null,
    },
    values: {
      n_bins: data.bins.length,
      background_median: finiteOrNull(data.background?.median),
      background_sigma: finiteOrNull(data.background?.sigma),
      background_count: finiteOrNull(data.background?.count),
      cumulative_sum_last: finiteOrNull(last?.cumulative_sum),
    },
    notes: [],
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
    dq: dqHandlingOf(excludeDq, sb.masked),
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

export function lineCutEntry(prov: MeasurementProvenance, cut: LineCut, excludeDq: boolean, region: Region): MeasurementLogDraft {
  const finite = cut.values.filter((v): v is number => typeof v === "number" && Number.isFinite(v));
  return {
    kind: "line_cut",
    file: prov.file,
    image: prov.image,
    dq: dqHandlingOf(excludeDq, cut.masked),
    unit: null,
    source: regionSource(region.props.text, region.shape, null),
    params: { x1: finiteOrNull(cut.x1), y1: finiteOrNull(cut.y1), x2: finiteOrNull(cut.x2), y2: finiteOrNull(cut.y2) },
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

export function lineEntry(filePath: string, result: LineMeasurement, model: LineModel): MeasurementLogDraft {
  const velocity = result.velocity;
  const fit = result.fit;
  return {
    kind: "line",
    file: fileNameOf(filePath),
    image: LOADED_CUBE_IMAGE,
    dq: "not_applied",
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
    },
    values: {
      flux: finiteOrNull(result.flux),
      flux_err: finiteOrNull(result.flux_err),
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
    },
    notes: [...result.notes, lineUnitNote(result.flux_unit, result.axis_unit)],
  };
}

interface TimeSeriesLogOptions {
  apertureRadius: number;
  annulusInner?: number;
  annulusOuter?: number;
  gain?: number;
  excludeDq: boolean;
  trackDrift: boolean;
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
  const compIdx = res.targets.map((t, i) => (t.role === "comp" ? i : -1)).filter((i) => i >= 0);
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
      targets: res.targets.map((t) => ({ x: t.x, y: t.y, label: t.label, role: t.role })),
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
    },
    notes: [...res.warnings],
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
    notes: input.channels.flatMap((c) => (c.body.noise_note ? [prefixed ? `${c.label}: ${c.body.noise_note}` : c.body.noise_note] : [])),
  };
}

export function crossMatchEntry(
  filePath: string,
  res: CrossMatchResult,
  opts: { sigma: number; maxStars: number; colourTerm: boolean },
): MeasurementLogDraft {
  const zp = res.zero_point;
  const astrometry = res.astrometry;
  return {
    kind: "catalog_crossmatch",
    file: fileNameOf(filePath),
    image: LOADED_FILE_IMAGE,
    dq: "not_applied",
    unit: "mag",
    source: FILE_SOURCE,
    params: {
      band: res.band,
      match_radius_arcsec: finiteOrNull(res.match_radius_arcsec),
      colour_term: opts.colourTerm,
      detection_sigma: opts.sigma,
      max_stars: opts.maxStars,
      epoch_year: finiteOrNull(res.epoch_year),
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
    },
    notes: [...res.warnings],
  };
}

export function spectrumExportEntry(filePath: string, input: SpectrumExportInput, savedPath: string | null): MeasurementLogDraft {
  const summary = spectrumExportSummary(input);
  const fluxUnit = summary.params.flux_unit;
  return {
    kind: "spectrum_export",
    file: fileNameOf(filePath),
    image: LOADED_CUBE_IMAGE,
    dq: "not_applied",
    unit: typeof fluxUnit === "string" ? fluxUnit : null,
    source: JSON.stringify(input.source),
    params: { ...summary.params, saved_path: savedPath },
    values: { ...summary.values },
    notes: input.pixelFluxJyError !== null ? [input.pixelFluxJyError] : [],
  };
}

export function spectrumCompareEntry(filePath: string, csvInput: ComparisonCsvInput, savedPath: string | null): MeasurementLogDraft {
  return {
    kind: "spectrum_compare",
    file: fileNameOf(filePath),
    image: LOADED_CUBE_IMAGE,
    dq: "not_applied",
    unit: null,
    source: FILE_SOURCE,
    params: {
      normalisation: csvInput.mode,
      offset_step: csvInput.mode === "offset" ? finiteOrNull(csvInput.result.step) : null,
      view: csvInput.view,
      saved_path: savedPath,
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
