import type { DrizzleRgbOptions } from "../shared/types/stacking";
import type { FixedRejection, RejectionParameter, RejectionSettings } from "./stackingRejection";

export type DrizzleKernel = NonNullable<DrizzleRgbOptions["kernel"]>;

export interface DrizzleSliderSpec {
  min: number;
  max: number;
  step: number;
  initial: number;
}

export const DRIZZLE_SCALE: DrizzleSliderSpec = { min: 1, max: 4, step: 0.5, initial: 2 };

export const DRIZZLE_PIXFRAC: DrizzleSliderSpec = { min: 0.1, max: 1, step: 0.05, initial: 0.7 };

export const DRIZZLE_KERNELS: readonly { value: DrizzleKernel; label: string }[] = [
  { value: "square", label: "Square" },
  { value: "gaussian", label: "Gaussian" },
  { value: "lanczos3", label: "Lanczos3" },
];

export const DEFAULT_DRIZZLE_KERNEL: DrizzleKernel = "square";

const DRIZZLE_FIXED_VALUES: Record<Exclude<RejectionParameter, "sigmaLow" | "sigmaHigh">, number> = {
  maxIterations: 5,
  winsorCutoff: 5,
  percentileLow: 0.2,
  percentileHigh: 0.1,
  minmaxLow: 1,
  minmaxHigh: 1,
};

export const DRIZZLE_FIXED_REJECTION: FixedRejection = { note: "fixed in drizzle", values: DRIZZLE_FIXED_VALUES };

export const DRIZZLE_REJECTION_DEFAULTS: RejectionSettings = { rejection: "sigma_clip", sigmaLow: 3, sigmaHigh: 3, ...DRIZZLE_FIXED_VALUES };
