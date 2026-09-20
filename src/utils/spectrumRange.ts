import type { ContinuumWindows } from "../shared/types/cube";

export interface ChannelRange {
  z0: number;
  z1: number;
}

export interface PlotMapping {
  width: number;
  padLeft: number;
  padRight: number;
  xMin: number;
  xMax: number;
  xValues: number[] | null;
  n: number;
}

export const DEFAULT_CONTINUUM_WIDTH = 5;

const MIN_RANGE = 1e-10;

function plotWidth(m: PlotMapping): number {
  return Math.max(m.width - m.padLeft - m.padRight, 1);
}

function xRange(m: PlotMapping): number {
  return Math.max(m.xMax - m.xMin, MIN_RANGE);
}

export function channelAxisValue(z: number, m: PlotMapping): number {
  if (m.xValues && m.xValues.length === m.n) return m.xValues[z];
  return z;
}

export function axisValueToPixel(x: number, m: PlotMapping): number {
  return m.padLeft + ((x - m.xMin) / xRange(m)) * plotWidth(m);
}

export function pixelToAxisValue(px: number, m: PlotMapping): number {
  return m.xMin + ((px - m.padLeft) / plotWidth(m)) * xRange(m);
}

export function nearestChannel(xValue: number, m: PlotMapping): number | null {
  if (m.n <= 0 || !Number.isFinite(xValue)) return null;
  if (!m.xValues || m.xValues.length !== m.n) {
    return Math.min(m.n - 1, Math.max(0, Math.round(xValue)));
  }
  let best = -1;
  let bestDistance = Infinity;
  for (let i = 0; i < m.n; i++) {
    const v = m.xValues[i];
    if (!Number.isFinite(v)) continue;
    const d = Math.abs(v - xValue);
    if (d < bestDistance) {
      bestDistance = d;
      best = i;
    }
  }
  return best < 0 ? null : best;
}

export function clampRange(range: ChannelRange, n: number): ChannelRange | null {
  if (n <= 0 || !Number.isFinite(range.z0) || !Number.isFinite(range.z1)) return null;
  const a = Math.min(n - 1, Math.max(0, Math.round(range.z0)));
  const b = Math.min(n - 1, Math.max(0, Math.round(range.z1)));
  return { z0: Math.min(a, b), z1: Math.max(a, b) };
}

export function channelRangeFromDrag(startPx: number, endPx: number, m: PlotMapping): ChannelRange | null {
  const a = nearestChannel(pixelToAxisValue(startPx, m), m);
  const b = nearestChannel(pixelToAxisValue(endPx, m), m);
  if (a === null || b === null) return null;
  return clampRange({ z0: a, z1: b }, m.n);
}

export function channelWidthAt(values: number[], i: number): number {
  const n = values.length;
  if (n < 2 || i < 0 || i >= n) return 0;
  if (i === 0) return Math.abs(values[1] - values[0]);
  if (i === n - 1) return Math.abs(values[n - 1] - values[n - 2]);
  return Math.abs(values[i + 1] - values[i - 1]) / 2;
}

export function rangeBounds(range: ChannelRange, m: PlotMapping): { lo: number; hi: number } {
  const a = channelAxisValue(range.z0, m);
  const b = channelAxisValue(range.z1, m);
  return { lo: Math.min(a, b), hi: Math.max(a, b) };
}

export function rangePixelSpan(range: ChannelRange, m: PlotMapping): { x0: number; x1: number } {
  const values = m.xValues && m.xValues.length === m.n ? m.xValues : null;
  const halfA = values ? channelWidthAt(values, range.z0) / 2 : 0.5;
  const halfB = values ? channelWidthAt(values, range.z1) / 2 : 0.5;
  const a = channelAxisValue(range.z0, m);
  const b = channelAxisValue(range.z1, m);
  const lo = Math.min(a - halfA, a + halfA, b - halfB, b + halfB);
  const hi = Math.max(a - halfA, a + halfA, b - halfB, b + halfB);
  const x0 = Math.max(m.padLeft, axisValueToPixel(lo, m));
  const x1 = Math.min(m.width - m.padRight, axisValueToPixel(hi, m));
  return { x0: Math.min(x0, x1), x1: Math.max(x0, x1) };
}

export function formatRangeLabel(range: ChannelRange, m: PlotMapping, unit: string): string {
  const channels = range.z0 === range.z1 ? `ch ${range.z0}` : `ch ${range.z0}–${range.z1}`;
  if (!m.xValues || m.xValues.length !== m.n) return channels;
  const { lo, hi } = rangeBounds(range, m);
  const decimals = unit === "km/s" ? 1 : 4;
  return `${channels} (${lo.toFixed(decimals)}–${hi.toFixed(decimals)} ${unit})`;
}

export function windowsAreValid(windows: ContinuumWindows | null, n: number): boolean {
  if (!windows) return false;
  return windows.every(
    ([a, b]) => Number.isInteger(a) && Number.isInteger(b) && a >= 0 && a <= b && b < n,
  );
}

export function defaultContinuumWindows(
  range: ChannelRange,
  n: number,
  width: number = DEFAULT_CONTINUUM_WIDTH,
): ContinuumWindows | null {
  const clamped = clampRange(range, n);
  if (!clamped || width < 1) return null;
  const left: [number, number] | null =
    clamped.z0 > 0 ? [Math.max(0, clamped.z0 - width), clamped.z0 - 1] : null;
  const right: [number, number] | null =
    clamped.z1 < n - 1 ? [clamped.z1 + 1, Math.min(n - 1, clamped.z1 + width)] : null;
  if (left && right) return [left, right];
  if (left) return [left, left];
  if (right) return [right, right];
  return null;
}

export function parseChannelInput(text: string, n: number): number | null {
  const value = Number(text.trim());
  if (!Number.isInteger(value) || value < 0 || value >= n) return null;
  return value;
}
