import type { ContinuumWindows } from "../shared/types/cube";
import type { ChannelRange } from "./spectrumRange";

export const GUARD_GAP_MIN_CHANNELS = 2;
export const GUARD_GAP_LINE_FRACTION = 0.25;
export const AUTO_WINDOWS_RULE =
  "two windows as wide as the line range, each separated from it by a guard gap of a quarter of the line width (at least 2 channels); near an edge of the axis the gap shrinks first, then the window";

export function guardGapChannels(lineWidth: number): number {
  return Math.max(GUARD_GAP_MIN_CHANNELS, Math.ceil(lineWidth * GUARD_GAP_LINE_FRACTION));
}

export function guardedContinuumWindows(range: ChannelRange, n: number): ContinuumWindows | null {
  const { z0, z1 } = range;
  if (!Number.isInteger(z0) || !Number.isInteger(z1) || !Number.isInteger(n)) return null;
  if (z0 < 0 || z0 > z1 || z1 >= n) return null;
  const lineWidth = z1 - z0 + 1;
  const guardGap = guardGapChannels(lineWidth);
  const fittingGap = (available: number) => Math.min(guardGap, Math.max(0, available - lineWidth));
  const availableLow = z0;
  const availableHigh = n - 1 - z1;
  const lowGap = fittingGap(availableLow);
  const highGap = fittingGap(availableHigh);
  const low: [number, number] | null =
    availableLow > 0 ? [Math.max(0, z0 - lowGap - lineWidth), z0 - lowGap - 1] : null;
  const high: [number, number] | null =
    availableHigh > 0 ? [z1 + highGap + 1, Math.min(n - 1, z1 + highGap + lineWidth)] : null;
  if (low && high) return [low, high];
  if (low) return [low, low];
  if (high) return [high, high];
  return null;
}
