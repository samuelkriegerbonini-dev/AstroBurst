import type { ProfileSeries } from "../components/regions/ProfilePlot";
import type { ContinuumWindows, CubeSpectrum, RegionSpectrum } from "../shared/types/cube";
import type { Region } from "../shared/types/regions";
import type {
  CorrectionFrame,
  RadialVelocityCorrectionResult,
  SpectralAxisMode,
  SpectrumSource,
} from "../shared/types/spectral";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "./catalogCsv";
import { shapeSummary } from "./regionGeometry";
import {
  NO_JY_REASON,
  fluxUnitLabel,
  frameLines,
  linkedAnnulus,
  numericSeries,
  regionLabel,
  vacuumLines,
  type AxisColumn,
  type SpectrumView,
  type VacuumAxis,
} from "./spectrumExport";
import { windowsAreValid } from "./spectrumRange";

export type NormaliseMode = "none" | "peak" | "median" | "window" | "offset";

export const NORMALISE_MODES: readonly NormaliseMode[] = ["none", "peak", "median", "window", "offset"];
export const COMPARISON_PALETTE: readonly string[] = [
  "#7dd3fc",
  "#f472b6",
  "#fbbf24",
  "#34d399",
  "#a78bfa",
  "#fb923c",
  "#22d3ee",
  "#f87171",
];
export const PIXEL_SERIES_COLOR = "#e4e4e7";
export const COMPARE_DEBOUNCE_MS = 300;
export const PIXEL_ENTRY_ID = "pixel";
export const MAX_COMPARISON_REGIONS = 16;

const NORM_UNIT = "norm";
const OFFSET_UNIT_SUFFIX = " + offset";
const MIXED_UNITS_LABEL = "flux (mixed units)";
const FLUX_LABEL = "flux";
const DEFAULT_OFFSET_STEP = 1;
const CHANNEL_HEADER = "channel";
const VACUUM_HEADER = "wavelength_vacuum_um";
const EMPTY_FIELD = "-";
const DEFAULT_CSV_NAME = "spectra.csv";
const CSV_SUFFIX = "_spectra.csv";
const IMAGE_NOTE = "loaded cube; extracted from the cube on disk, not from the processed step";
const WINDOWS_UNSET_REASON = "continuum windows are not set";
const PEAK_REASON = "peak is not positive";
const MEDIAN_REASON = "median is not positive";
const CONTINUUM_MEDIAN_REASON = "continuum median is not positive";
const NO_RESULT_REASON = "no result for this entry";
const TOO_MANY_REGIONS_REASON = `more than ${MAX_COMPARISON_REGIONS} regions: not compared`;

export interface ComparisonEntry {
  id: string;
  kind: "region" | "pixel";
  label: string;
  color: string;
  source: SpectrumSource;
  regionText: string | null;
  backgroundId: string | null;
  region: RegionSpectrum | null;
  pixel: CubeSpectrum | null;
  error: string | null;
}

export interface ComparisonOmission {
  id: string;
  label: string;
  reason: string;
}

export interface ComparisonPlotted {
  id: string;
  label: string;
  unit: string;
  factor: number;
  offset: number;
}

export interface ComparisonSeriesResult {
  series: ProfileSeries[];
  plotted: ComparisonPlotted[];
  omitted: ComparisonOmission[];
  yLabel: string;
  step: number | null;
}

export interface LimitedCandidates {
  compared: Region[];
  omitted: ComparisonOmission[];
}

export function limitCandidates(candidates: Region[], regions: Region[]): LimitedCandidates {
  const compared = candidates.slice(0, MAX_COMPARISON_REGIONS);
  const omitted = candidates.slice(MAX_COMPARISON_REGIONS).map((region) => ({
    id: region.id,
    label: regionLabel(region, regions.findIndex((r) => r.id === region.id) + 1),
    reason: TOO_MANY_REGIONS_REASON,
  }));
  return { compared, omitted };
}

function errorText(reason: unknown): string {
  if (typeof reason === "object" && reason !== null && "message" in reason) return String(reason.message);
  return String(reason);
}

export function entriesFrom(
  candidates: Region[],
  regions: Region[],
  settled: PromiseSettledResult<RegionSpectrum>[],
  pixel: { coord: { x: number; y: number }; result: PromiseSettledResult<CubeSpectrum> } | null,
): ComparisonEntry[] {
  const entries: ComparisonEntry[] = candidates.map((region, i) => {
    const result = settled[i];
    const ordinal = regions.findIndex((r) => r.id === region.id) + 1;
    const background = linkedAnnulus(regions, region);
    return {
      id: region.id,
      kind: "region",
      label: regionLabel(region, ordinal),
      color: region.props.color ?? COMPARISON_PALETTE[i % COMPARISON_PALETTE.length],
      source: { kind: "region", shape: region.shape, background },
      regionText: region.props.text,
      backgroundId: background ? region.backgroundId : null,
      region: result?.status === "fulfilled" ? result.value : null,
      pixel: null,
      error: result === undefined ? NO_RESULT_REASON : result.status === "rejected" ? errorText(result.reason) : null,
    };
  });
  if (pixel) {
    entries.push({
      id: PIXEL_ENTRY_ID,
      kind: "pixel",
      label: `pixel (${pixel.coord.x}, ${pixel.coord.y})`,
      color: PIXEL_SERIES_COLOR,
      source: { kind: "pixel", x: pixel.coord.x, y: pixel.coord.y },
      regionText: null,
      backgroundId: null,
      region: null,
      pixel: pixel.result.status === "fulfilled" ? pixel.result.value : null,
      error: pixel.result.status === "rejected" ? errorText(pixel.result.reason) : null,
    });
  }
  return entries;
}

function entryLength(entry: ComparisonEntry): number | null {
  if (entry.kind === "region") return entry.region ? entry.region.sum.length : null;
  return entry.pixel ? entry.pixel.values.length : null;
}

export function pruneEntries(entries: ComparisonEntry[], regions: Region[], channelCount: number): ComparisonEntry[] {
  const ids = new Set(regions.map((r) => r.id));
  return entries.filter((entry) => {
    if (entry.kind === "region" && !ids.has(entry.id)) return false;
    const length = entryLength(entry);
    return length === null || length === channelCount;
  });
}

export function entryValues(entry: ComparisonEntry, view: SpectrumView): number[] | null {
  if (entry.kind === "region") {
    const region = entry.region;
    if (!region) return null;
    const array = view === "sum" ? region.sum : view === "mean" ? region.mean : region.flux_jy;
    return array ? numericSeries(array) : null;
  }
  const pixel = entry.pixel;
  if (!pixel) return null;
  const array = view === "jy" ? (pixel.flux_jy ?? null) : pixel.values;
  return array ? numericSeries(array) : null;
}

export function finiteMedian(values: number[]): number {
  const finite = values.filter((v) => Number.isFinite(v)).sort((a, b) => a - b);
  const n = finite.length;
  if (n === 0) return NaN;
  const mid = n >> 1;
  return n % 2 === 1 ? finite[mid] : (finite[mid - 1] + finite[mid]) / 2;
}

function windowIndices(windows: ContinuumWindows): number[] {
  const indices = new Set<number>();
  for (const [a, b] of windows) for (let i = a; i <= b; i++) indices.add(i);
  return [...indices];
}

export function normaliseFactor(
  y: number[],
  mode: NormaliseMode,
  windows: ContinuumWindows | null,
): { factor: number } | { reason: string } {
  switch (mode) {
    case "none":
    case "offset":
      return { factor: 1 };
    case "peak": {
      let peak = -Infinity;
      for (const v of y) if (Number.isFinite(v) && v > peak) peak = v;
      return peak > 0 && Number.isFinite(peak) ? { factor: peak } : { reason: PEAK_REASON };
    }
    case "median": {
      const median = finiteMedian(y);
      return median > 0 ? { factor: median } : { reason: MEDIAN_REASON };
    }
    case "window": {
      if (!windowsAreValid(windows, y.length) || windows === null) return { reason: WINDOWS_UNSET_REASON };
      const median = finiteMedian(windowIndices(windows).map((i) => y[i]));
      return median > 0 ? { factor: median } : { reason: CONTINUUM_MEDIAN_REASON };
    }
  }
}

export function offsetStepAuto(rows: number[][]): number {
  let best = 0;
  for (const row of rows) {
    let min = Infinity;
    let max = -Infinity;
    for (const v of row) {
      if (!Number.isFinite(v)) continue;
      if (v < min) min = v;
      if (v > max) max = v;
    }
    if (max > min && max - min > best) best = max - min;
  }
  return best > 0 ? best : DEFAULT_OFFSET_STEP;
}

function seriesUnit(mode: NormaliseMode, fluxUnit: string): string {
  if (mode === "none") return fluxUnit;
  if (mode === "offset") return `${fluxUnit}${OFFSET_UNIT_SUFFIX}`;
  return NORM_UNIT;
}

function sharedFluxUnit(units: string[]): string {
  const distinct = new Set(units);
  if (distinct.size === 0) return FLUX_LABEL;
  return distinct.size === 1 ? units[0] : MIXED_UNITS_LABEL;
}

function yLabelFor(mode: NormaliseMode, fluxUnits: string[], step: number | null): string {
  switch (mode) {
    case "none":
      return sharedFluxUnit(fluxUnits);
    case "peak":
      return "flux / peak";
    case "median":
      return "flux / median";
    case "window":
      return "flux / continuum median";
    case "offset":
      return `${sharedFluxUnit(fluxUnits)} + k × ${step ?? DEFAULT_OFFSET_STEP}`;
  }
}

export function comparisonSeries(
  entries: ComparisonEntry[],
  hidden: ReadonlySet<string>,
  axisValues: number[],
  view: SpectrumView,
  mode: NormaliseMode,
  windows: ContinuumWindows | null,
  offsetStep: number | null,
  bunit: string | null,
  skipped: readonly ComparisonOmission[] = [],
): ComparisonSeriesResult {
  const omitted: ComparisonOmission[] = [];
  const prepared: { entry: ComparisonEntry; values: number[]; factor: number; fluxUnit: string }[] = [];
  for (const entry of entries) {
    if (hidden.has(entry.id)) continue;
    const omit = (reason: string) => omitted.push({ id: entry.id, label: entry.label, reason });
    if (entry.error !== null) {
      omit(entry.error);
      continue;
    }
    const values = entryValues(entry, view);
    if (values === null) {
      omit(view === "jy" ? NO_JY_REASON : NO_RESULT_REASON);
      continue;
    }
    if (values.length !== axisValues.length) {
      omit(`spectrum has ${values.length} channels, axis has ${axisValues.length}`);
      continue;
    }
    const norm = normaliseFactor(values, mode, windows);
    if ("reason" in norm) {
      omit(norm.reason);
      continue;
    }
    prepared.push({ entry, values, factor: norm.factor, fluxUnit: fluxUnitLabel(bunit, view, entry.kind) });
  }
  const step =
    mode === "offset"
      ? offsetStep !== null && Number.isFinite(offsetStep)
        ? offsetStep
        : offsetStepAuto(prepared.map((p) => p.values))
      : null;
  const series: ProfileSeries[] = [];
  const plotted: ComparisonPlotted[] = [];
  prepared.forEach(({ entry, values, factor, fluxUnit }, k) => {
    const offset = step === null ? 0 : k * step;
    const y = values.map((v) => {
      const out = v / factor + offset;
      return Number.isFinite(out) ? out : null;
    });
    series.push({ x: axisValues, y, color: entry.color, label: entry.label });
    plotted.push({ id: entry.id, label: entry.label, unit: seriesUnit(mode, fluxUnit), factor, offset });
  });
  omitted.push(...skipped);
  return { series, plotted, omitted, yLabel: yLabelFor(mode, prepared.map((p) => p.fluxUnit), step), step };
}

export interface ComparisonCsvInput {
  fileName: string;
  view: SpectrumView;
  mode: NormaliseMode;
  windows: ContinuumWindows | null;
  axis: AxisColumn;
  axisKnown: boolean;
  vacuum: VacuumAxis;
  channelCount: number;
  entries: ComparisonEntry[];
  result: ComparisonSeriesResult;
  specsys: string | null;
  axisMode: SpectralAxisMode;
  correction: CorrectionFrame;
  correctionResult: RadialVelocityCorrectionResult | null;
  exportedAtUtc: string;
}

function normaliseLine(input: ComparisonCsvInput): string {
  if (input.mode === "window" && input.windows) {
    const [[a0, b0], [a1, b1]] = input.windows;
    return `normalise: window; windows: [${a0}-${b0}],[${a1}-${b1}]`;
  }
  if (input.mode === "offset") return `normalise: offset; offset_step: ${input.result.step ?? DEFAULT_OFFSET_STEP}`;
  return `normalise: ${input.mode}`;
}

function seriesLine(entry: ComparisonEntry, plotted: ComparisonPlotted): string {
  const source = entry.source;
  const where = source.kind === "pixel" ? `pixel (${source.x}, ${source.y})` : shapeSummary(source.shape);
  const background =
    source.kind === "region" && source.background ? `annulus ${entry.backgroundId ?? EMPTY_FIELD}` : "none";
  const region = entry.region;
  const npix = region ? String(region.npix) : EMPTY_FIELD;
  const nBg = region ? String(region.n_bg) : EMPTY_FIELD;
  const subtracted = region ? String(region.bg_subtracted) : EMPTY_FIELD;
  return `series: ${plotted.label}; ${entry.kind}; ${where}; background: ${background}; npix: ${npix}; n_bg: ${nBg}; bg_subtracted: ${subtracted}; unit: ${plotted.unit}; factor: ${plotted.factor}; offset: ${plotted.offset}`;
}

export function comparisonCsv(input: ComparisonCsvInput): string {
  const byId = new Map(input.entries.map((entry) => [entry.id, entry]));
  const lines = [
    `file: ${input.fileName}`,
    `image: ${input.fileName} (${IMAGE_NOTE})`,
    `view: ${input.view}`,
    normaliseLine(input),
    ...frameLines(input.specsys, input.axisMode, input.correction, input.correctionResult),
    ...vacuumLines(input.vacuum),
    `exported_utc: ${input.exportedAtUtc}`,
  ];
  input.result.plotted.forEach((plotted) => {
    const entry = byId.get(plotted.id);
    if (entry) lines.push(seriesLine(entry, plotted));
  });
  for (const omission of input.result.omitted) lines.push(`omitted: ${omission.label}; ${omission.reason}`);

  const columns: CsvColumn<number>[] = [{ header: CHANNEL_HEADER, value: (i) => i }];
  const vacuumValues = input.vacuum.values;
  if (vacuumValues !== null) columns.push({ header: VACUUM_HEADER, value: (i) => vacuumValues[i] });
  const axisValues = input.axis.values;
  if (input.axisKnown && axisValues !== null && input.axis.header !== CHANNEL_HEADER) {
    columns.push({ header: input.axis.header, value: (i) => axisValues[i] });
  }
  input.result.plotted.forEach((plotted, k) => {
    const y = input.result.series[k]?.y ?? [];
    columns.push({ header: `${plotted.label} [${plotted.unit}]`, value: (i) => y[i] ?? null });
  });
  const rows = Array.from({ length: input.channelCount }, (_, i) => i);
  return lines.map((line) => `# ${line}`).join(CSV_LINE_END) + CSV_LINE_END + buildCsv(columns, rows);
}

export function comparisonCsvFileName(filePath: string | null): string {
  const withoutFragment = (filePath ?? "").split("#")[0];
  const base = withoutFragment.split(/[\\/]/).pop() ?? "";
  const dot = base.lastIndexOf(".");
  const stem = dot > 0 ? base.slice(0, dot) : base;
  return stem ? `${stem}${CSV_SUFFIX}` : DEFAULT_CSV_NAME;
}
