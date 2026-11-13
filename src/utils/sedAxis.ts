import type { PhotometricPlane, RegionSpectrum } from "../shared/types/cube";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "./catalogCsv";
import { provenanceLines, unitLines, withProvenance } from "./csvProvenance";
import { abMagErr, fnuJyToAb, parseSpectralFluxUnit } from "./fluxUnits";
import { linearScale, niceTicks } from "./plotScale";

export interface SedPoint {
  index: number;
  filter: string;
  pivotUm: number;
  value: number;
  err: number | null;
  limit?: boolean;
}

export interface SedAxis {
  min: number;
  max: number;
  log: boolean;
  ticks: number[];
}

export type SedSeriesKind = "pixel" | "region_sum" | "region_flux";

export interface SedSeries {
  label: string;
  values: readonly number[];
  errors: readonly (number | null)[] | null;
  unit: string | null;
  kind: SedSeriesKind;
  npix: number | null;
}

export type SedUnitKind = "flux_density" | "surface_brightness" | "other";

export interface SedCsvMeta {
  unit: string | null;
  source: string;
  file: string | null;
  exportedUtc: string;
  appVersion: string;
}

export interface SedSource {
  spectrum: readonly number[];
  err: readonly number[] | null;
  coord: { x: number; y: number } | null;
  region: Pick<RegionSpectrum, "sum" | "err" | "npix" | "flux_jy" | "flux_jy_err"> | null;
}

const LOG_AXIS_MIN_RATIO = 2;
const AXIS_PAD_FRACTION = 0.08;
const MIN_LOG_PAD_DEX = 0.02;
const MIN_LINEAR_PAD_FRACTION = 0.05;
const LINEAR_TICK_COUNT = 5;
const LOG_TICK_MANTISSAS = [1, 2, 5];
const TICK_PRECISION = 6;

function finiteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function sedPoints(
  planes: readonly PhotometricPlane[],
  values: readonly number[],
  errors: readonly (number | null)[] | null,
): SedPoint[] {
  const points: SedPoint[] = [];
  planes.forEach((plane, index) => {
    const pivot = plane.pivot_um;
    const value = values[index];
    if (!finiteNumber(pivot) || pivot <= 0 || !finiteNumber(value)) return;
    const err = errors?.[index];
    points.push({ index, filter: plane.filter, pivotUm: pivot, value, err: finiteNumber(err) && err >= 0 ? err : null });
  });
  return points.sort((a, b) => a.pivotUm - b.pivotUm || a.index - b.index);
}

function logTicks(min: number, max: number): number[] {
  const ticks: number[] = [];
  for (let exponent = Math.floor(Math.log10(min)); exponent <= Math.ceil(Math.log10(max)); exponent++) {
    for (const mantissa of LOG_TICK_MANTISSAS) {
      const tick = Number((mantissa * 10 ** exponent).toPrecision(TICK_PRECISION));
      if (tick >= min && tick <= max) ticks.push(tick);
    }
  }
  return ticks;
}

export function sedWavelengthAxis(points: readonly SedPoint[]): SedAxis | null {
  if (points.length === 0) return null;
  const pivots = points.map((point) => point.pivotUm);
  const lo = Math.min(...pivots);
  const hi = Math.max(...pivots);
  if (hi / lo >= LOG_AXIS_MIN_RATIO) {
    const [a, b] = [Math.log10(lo), Math.log10(hi)];
    const pad = Math.max((b - a) * AXIS_PAD_FRACTION, MIN_LOG_PAD_DEX);
    const min = 10 ** (a - pad);
    const max = 10 ** (b + pad);
    return { min, max, log: true, ticks: logTicks(min, max) };
  }
  const pad = hi > lo ? (hi - lo) * AXIS_PAD_FRACTION : hi * MIN_LINEAR_PAD_FRACTION;
  const min = lo - pad;
  const max = hi + pad;
  return { min, max, log: false, ticks: niceTicks(min, max, LINEAR_TICK_COUNT).filter((tick) => tick >= min && tick <= max) };
}

export function sedX(wavelengthUm: number, axis: SedAxis, width: number): number {
  if (axis.log) return linearScale([Math.log10(axis.min), Math.log10(axis.max)], [0, width])(Math.log10(wavelengthUm));
  return linearScale([axis.min, axis.max], [0, width])(wavelengthUm);
}

const JANSKY = "Jy";

export function sedSeries(source: SedSource): SedSeries | null {
  const region = source.region;
  if (region) {
    const pixels = `over ${region.npix.toFixed(1)} px`;
    if (region.flux_jy) {
      return { label: `region flux ${pixels}`, values: region.flux_jy, errors: region.flux_jy_err, unit: JANSKY, kind: "region_flux", npix: region.npix };
    }
    return { label: `region sum ${pixels}`, values: region.sum, errors: region.err, unit: null, kind: "region_sum", npix: region.npix };
  }
  if (source.coord && source.spectrum.length > 0) {
    return { label: `pixel (${source.coord.x}, ${source.coord.y})`, values: source.spectrum, errors: source.err, unit: null, kind: "pixel", npix: null };
  }
  return null;
}

const FLUX_DENSITY_UNIT = /^[nuµμmkM]?Jy(\/pix|\/pixel)?$/;
const SURFACE_BRIGHTNESS_UNIT = /(\/sr|\/steradian|sr-1|sr\*\*-1|\/arcsec2|\/arcsec\^2|\/arcsec\*\*2|arcsec-2)$/i;

export function sedUnitKind(bunit: string | null): SedUnitKind {
  if (bunit === null) return "other";
  const compact = bunit.replace(/\s+/g, "");
  if (FLUX_DENSITY_UNIT.test(compact)) return "flux_density";
  if (SURFACE_BRIGHTNESS_UNIT.test(compact)) return "surface_brightness";
  return "other";
}

export function sedYLabel(series: Pick<SedSeries, "unit" | "kind" | "npix"> | null, bunit: string | null): string {
  if (series === null) return "Flux";
  if (series.unit) return `Flux (${series.unit})`;
  const unit = bunit?.trim() || null;
  const kind = sedUnitKind(unit);
  if (series.kind === "region_sum" && series.npix !== null) {
    if (kind === "flux_density") return `Flux (${unit})`;
    const pixels = `over ${series.npix.toFixed(1)} px`;
    return unit ? `Sum of ${unit} ${pixels}` : `Sum ${pixels}`;
  }
  if (unit === null) return "Value";
  if (kind === "flux_density") return `Flux (${unit})`;
  if (kind === "surface_brightness") return `Surface brightness (${unit})`;
  return `Value (${unit})`;
}

const Y_PAD_FRACTION = 0.1;

export function sedYDomain(points: readonly SedPoint[]): [number, number] | null {
  if (points.length === 0) return null;
  const spread = (point: SedPoint) => (point.limit ? 0 : (point.err ?? 0));
  const lo = Math.min(...points.map((point) => point.value - spread(point)));
  const hi = Math.max(...points.map((point) => point.value + spread(point)));
  const pad = hi > lo ? (hi - lo) * Y_PAD_FRACTION : Math.max(Math.abs(hi) * Y_PAD_FRACTION, 1);
  return [lo - pad, hi + pad];
}

export type SedYScale = "linear" | "log" | "ab";

const lowEdge = (point: SedPoint) => point.value - (point.err ?? 0);
const highEdge = (point: SedPoint) => point.value + (point.err ?? 0);

export function sedLogAllowed(points: readonly SedPoint[]): boolean {
  return points.length > 0 && points.every((point) => lowEdge(point) > 0);
}

export function sedLogYAxis(points: readonly SedPoint[]): SedAxis | null {
  if (!sedLogAllowed(points)) return null;
  const a = Math.log10(Math.min(...points.map(lowEdge)));
  const b = Math.log10(Math.max(...points.map(highEdge)));
  const pad = Math.max((b - a) * AXIS_PAD_FRACTION, MIN_LOG_PAD_DEX);
  const min = 10 ** (a - pad);
  const max = 10 ** (b + pad);
  return { min, max, log: true, ticks: logTicks(min, max) };
}

export interface SedAbView {
  points: SedPoint[];
  dropped: number;
}

export function sedAbView(points: readonly SedPoint[], unit: string | null): SedAbView | null {
  if (unit === null) return null;
  const parsed = parseSpectralFluxUnit(unit);
  if (parsed.kind !== "flux_density_nu" || !(Number.isFinite(parsed.jy_per_unit) && parsed.jy_per_unit > 0)) return null;
  const jyPerUnit = parsed.jy_per_unit;
  const kept: SedPoint[] = [];
  let dropped = 0;
  for (const point of points) {
    const jy = point.value * jyPerUnit;
    const mag = fnuJyToAb(jy);
    if (mag === null) {
      dropped += 1;
      continue;
    }
    kept.push({ ...point, value: mag, err: point.err === null ? null : abMagErr(jy, point.err * jyPerUnit) });
  }
  return { points: kept, dropped };
}

const SED_CSV_COLUMNS: CsvColumn<SedPoint>[] = [
  { header: "filter", value: (point) => point.filter },
  { header: "pivot_um", value: (point) => point.pivotUm },
  { header: "flux", value: (point) => point.value },
  { header: "flux_err", value: (point) => point.err },
  { header: "limit", value: (point) => point.limit === true },
];

export function sedCsv(points: readonly SedPoint[], meta: SedCsvMeta): string {
  const fluxUnit = meta.unit ?? "image unit";
  const lines = [
    ...provenanceLines([
      ["software", `AstroBurst ${meta.appVersion}`],
      ["exported_utc", meta.exportedUtc],
      ["file", meta.file],
      ["source", meta.source],
      ["flux_unit", meta.unit ?? "not calibrated"],
    ]),
    ...unitLines({ pivot_um: "um", flux: fluxUnit, flux_err: fluxUnit }),
  ];
  return withProvenance(lines, buildCsv(SED_CSV_COLUMNS, [...points]), CSV_LINE_END);
}

export function sedCsvFileName(fileName: string | null): string {
  if (fileName === null) return "sed.csv";
  const dot = fileName.lastIndexOf(".");
  return `${dot > 0 ? fileName.slice(0, dot) : fileName}_sed.csv`;
}

const PSF_DECIMALS = 3;
const SAME_PSF_TOLERANCE = 1e-6;

export function psfFwhmSummary(planes: readonly PhotometricPlane[]): string | null {
  const fwhms = planes.map((plane) => plane.psf_fwhm_arcsec).filter((fwhm): fwhm is number => finiteNumber(fwhm) && fwhm > 0);
  if (fwhms.length === 0) return null;
  const min = Math.min(...fwhms);
  const max = Math.max(...fwhms);
  if (fwhms.length === planes.length && max - min <= SAME_PSF_TOLERANCE) return `PSF FWHM ${min.toFixed(PSF_DECIMALS)} arcsec on every plane`;
  return `PSF FWHM ${min.toFixed(PSF_DECIMALS)} to ${max.toFixed(PSF_DECIMALS)} arcsec across planes (${fwhms.length} of ${planes.length} planes)`;
}
