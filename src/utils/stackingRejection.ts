import type { CombineMethod, NormalizationMethod, RejectionMethod } from "../shared/types/stacking";

export interface SelectOption<T extends string> {
  value: T;
  label: string;
}

export interface StackSettings {
  sigmaLow: number;
  sigmaHigh: number;
  maxIterations: number;
  align: boolean;
  alignMethod?: string;
  rejection: RejectionMethod;
  combine: CombineMethod;
  normalization: NormalizationMethod;
  winsorCutoff: number;
  percentileLow: number;
  percentileHigh: number;
  minmaxLow: number;
  minmaxHigh: number;
  rejectionMaps: boolean;
}

export const DEFAULT_STACK_SETTINGS: StackSettings = {
  sigmaLow: 4.0,
  sigmaHigh: 3.0,
  maxIterations: 5,
  align: true,
  alignMethod: "phase_correlation",
  rejection: "sigma_clip",
  combine: "mean",
  normalization: "additive_scaling",
  winsorCutoff: 5.0,
  percentileLow: 0.2,
  percentileHigh: 0.1,
  minmaxLow: 1,
  minmaxHigh: 1,
  rejectionMaps: false,
};

export const REJECTION_OPTIONS: SelectOption<RejectionMethod>[] = [
  { value: "none", label: "No rejection" },
  { value: "sigma_clip", label: "Sigma clipping" },
  { value: "winsorized_sigma_clip", label: "Winsorized sigma clipping" },
  { value: "linear_fit_clip", label: "Linear fit clipping" },
  { value: "percentile_clip", label: "Percentile clipping" },
  { value: "min_max", label: "Min/max" },
];

export const COMBINE_OPTIONS: SelectOption<CombineMethod>[] = [
  { value: "mean", label: "Mean (average)" },
  { value: "median", label: "Median" },
  { value: "min", label: "Minimum" },
  { value: "max", label: "Maximum" },
];

export const NORMALIZATION_OPTIONS: SelectOption<NormalizationMethod>[] = [
  { value: "none", label: "None" },
  { value: "additive", label: "Additive" },
  { value: "multiplicative", label: "Multiplicative" },
  { value: "additive_scaling", label: "Additive with scaling" },
  { value: "multiplicative_scaling", label: "Multiplicative with scaling" },
];

export const PERCENTILE_MIN_FRAMES = 3;
export const SIGMA_RECOMMENDED_FRAMES = 5;
export const LINEAR_FIT_MIN_FRAMES = 5;
export const LINEAR_FIT_RECOMMENDED_FRAMES = 8;

export type HintSeverity = "ok" | "warn";

export interface FrameCountHint {
  severity: HintSeverity;
  text: string;
}

export function rejectionUsesSigma(method: RejectionMethod): boolean {
  return method === "sigma_clip" || method === "winsorized_sigma_clip" || method === "linear_fit_clip";
}

export function rejectionFrameHint(
  method: RejectionMethod,
  frameCount: number,
  minmaxLow = 1,
  minmaxHigh = 1,
): FrameCountHint {
  switch (method) {
    case "none":
      return { severity: "ok", text: "No pixel rejection: every frame contributes to each output pixel." };
    case "sigma_clip":
    case "winsorized_sigma_clip":
      return frameCount < SIGMA_RECOMMENDED_FRAMES
        ? { severity: "warn", text: `Sigma clipping works best with at least ${SIGMA_RECOMMENDED_FRAMES} frames (have ${frameCount}).` }
        : { severity: "ok", text: `Robust sigma estimated per pixel from ${frameCount} frames.` };
    case "linear_fit_clip":
      if (frameCount < LINEAR_FIT_MIN_FRAMES) {
        return { severity: "warn", text: `Linear fit needs at least ${LINEAR_FIT_MIN_FRAMES} frames (have ${frameCount}); sigma clipping is used instead.` };
      }
      return frameCount < LINEAR_FIT_RECOMMENDED_FRAMES
        ? { severity: "warn", text: `Linear fit works best with at least ${LINEAR_FIT_RECOMMENDED_FRAMES} frames (have ${frameCount}).` }
        : { severity: "ok", text: `Fits a line through the sorted stack of ${frameCount} frames and rejects residual outliers.` };
    case "percentile_clip":
      return frameCount < PERCENTILE_MIN_FRAMES
        ? { severity: "warn", text: `Percentile clipping needs at least ${PERCENTILE_MIN_FRAMES} frames (have ${frameCount}); nothing is rejected below that.` }
        : { severity: "ok", text: "Single pass around the per-pixel median; good for small stacks." };
    case "min_max": {
      const needed = minmaxLow + minmaxHigh + 1;
      return frameCount < needed
        ? { severity: "warn", text: `Min/max needs more than ${minmaxLow + minmaxHigh} frames to drop ${minmaxLow} low and ${minmaxHigh} high (have ${frameCount}).` }
        : { severity: "ok", text: `Drops the ${minmaxLow} lowest and ${minmaxHigh} highest values at every pixel.` };
    }
  }
}

export function subframeWeightsFor(
  paths: string[],
  weightsByPath: Record<string, number> | undefined,
): number[] | undefined {
  if (!weightsByPath) return undefined;
  if (!paths.some((p) => weightsByPath[p] !== undefined)) return undefined;
  return paths.map((p) => weightsByPath[p] ?? 1.0);
}
