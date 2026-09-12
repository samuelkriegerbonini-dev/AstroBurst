import { DEFAULT_DISPLAY_SETTINGS } from "../shared/types/display";

const PERCENTILE_GAP = 0.1;

function clampPercentile(v: number): number {
  return Math.min(100, Math.max(0, v));
}

export function normalizePercentiles(low: number, high: number): [number, number] {
  let lo = Number.isFinite(low) ? clampPercentile(low) : DEFAULT_DISPLAY_SETTINGS.percentileLow;
  let hi = Number.isFinite(high) ? clampPercentile(high) : DEFAULT_DISPLAY_SETTINGS.percentileHigh;
  if (lo > hi) [lo, hi] = [hi, lo];
  if (lo === hi) {
    if (lo + PERCENTILE_GAP <= 100) hi = lo + PERCENTILE_GAP;
    else lo = hi - PERCENTILE_GAP;
  }
  return [lo, hi];
}

export function normalizeUserLimits(lo: number | null, hi: number | null): [number | null, number | null] {
  const a = lo !== null && Number.isFinite(lo) ? lo : null;
  const b = hi !== null && Number.isFinite(hi) ? hi : null;
  if (a !== null && b !== null && a > b) return [b, a];
  return [a, b];
}
