import { formatCount } from "./formatCount";

export const SYNTH_PIXEL_BUDGET = 1_073_741_824;
const BYTES_PER_PIXEL = 4;
const BYTE_UNITS: [number, string][] = [
  [1e9, "GB"],
  [1e6, "MB"],
  [1e3, "kB"],
];

export interface SynthStackEstimate {
  pixels: number;
  bytes: number;
  overBudgetReason: string | null;
}

export function synthStackEstimate(width: number, height: number, frames: number): SynthStackEstimate {
  const framePixels = width * height;
  const pixels = framePixels * frames;
  const maxFrames = framePixels > 0 ? Math.floor(SYNTH_PIXEL_BUDGET / framePixels) : frames;
  const overBudgetReason =
    pixels > SYNTH_PIXEL_BUDGET
      ? `${frames} frames of ${width}x${height} pixels exceed the ${formatCount(SYNTH_PIXEL_BUDGET)}-pixel stack limit: at most ${maxFrames} frames at this size.`
      : null;
  return { pixels, bytes: pixels * BYTES_PER_PIXEL, overBudgetReason };
}

export function formatByteSize(bytes: number): string {
  for (const [scale, unit] of BYTE_UNITS) {
    if (bytes >= scale) return `${(bytes / scale).toFixed(1)} ${unit}`;
  }
  return `${Math.round(bytes)} B`;
}
