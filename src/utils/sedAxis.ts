import type { PhotometricPlane, RegionSpectrum } from "../shared/types/cube";
import { linearScale, niceTicks } from "./plotScale";

export interface SedPoint {
  index: number;
  filter: string;
  pivotUm: number;
  value: number;
  err: number | null;
}

export interface SedAxis {
  min: number;
  max: number;
  log: boolean;
  ticks: number[];
}

export interface SedSeries {
  label: string;
  values: readonly number[];
  errors: readonly (number | null)[] | null;
  unit: string | null;
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
    if (region.flux_jy) return { label: `region flux ${pixels}`, values: region.flux_jy, errors: region.flux_jy_err, unit: JANSKY };
    return { label: `region sum ${pixels}`, values: region.sum, errors: region.err, unit: null };
  }
  if (source.coord && source.spectrum.length > 0) {
    return { label: `pixel (${source.coord.x}, ${source.coord.y})`, values: source.spectrum, errors: source.err, unit: null };
  }
  return null;
}
