import type { DrizzleRgbResult } from "../shared/types/stacking";
import { formatCount } from "./formatCount";
import { decimalFieldState, firstFieldError, type DecimalFieldOptions } from "./decimalText";
import { WB_APPLY_MAX, WB_APPLY_MIN } from "./whiteBalanceRange";

export interface DrizzleResultItem {
  label: string;
  value: string | number;
}

export function drizzleResultItems(result: DrizzleRgbResult, kernelAtRun: string): DrizzleResultItem[] {
  return [
    { label: "Output", value: result.output_dims ? `${result.output_dims[0]}×${result.output_dims[1]}` : "--" },
    { label: "Scale", value: result.scale ? `${result.scale.toFixed(1)}x` : "--" },
    { label: "Kernel", value: kernelAtRun },
    { label: "R frames", value: result.frame_count_r },
    { label: "G frames", value: result.frame_count_g },
    { label: "B frames", value: result.frame_count_b },
    { label: "Rejected", value: result.rejected_pixels ? formatCount(result.rejected_pixels) : "0" },
  ];
}

export function isCancelledRun(message: string): boolean {
  return /cancel/i.test(message);
}

export const DRIZZLE_GAIN_FIELD: DecimalFieldOptions = { min: WB_APPLY_MIN, max: WB_APPLY_MAX };

export interface DrizzleGains {
  wbR: number;
  wbG: number;
  wbB: number;
}

export interface GainTexts {
  r: string;
  g: string;
  b: string;
}

export function manualGains(texts: GainTexts): { gains: DrizzleGains | null; error: string | null } {
  const r = decimalFieldState(texts.r, DRIZZLE_GAIN_FIELD);
  const g = decimalFieldState(texts.g, DRIZZLE_GAIN_FIELD);
  const b = decimalFieldState(texts.b, DRIZZLE_GAIN_FIELD);
  const error = firstFieldError([
    ["R gain", r],
    ["G gain", g],
    ["B gain", b],
  ]);
  if (r.value === null || g.value === null || b.value === null) return { gains: null, error };
  return { gains: { wbR: r.value, wbG: g.value, wbB: b.value }, error };
}
