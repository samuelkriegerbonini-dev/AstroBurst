import type { CosmeticConfig } from "./cosmetic";

export type RejectionMethod =
  | "none"
  | "sigma_clip"
  | "winsorized_sigma_clip"
  | "linear_fit_clip"
  | "percentile_clip"
  | "min_max";

export type CombineMethod = "mean" | "median" | "min" | "max";

export type NormalizationMethod =
  | "none"
  | "additive"
  | "multiplicative"
  | "additive_scaling"
  | "multiplicative_scaling";

export type RejectionNormalization = "none" | "scale_offset";

export interface CalibrateResult {
  png_path: string;
  fits_path?: string;
  previewUrl?: string;
  has_bias?: boolean;
  has_dark?: boolean;
  has_flat?: boolean;
  stats?: { min: number; max: number; mean: number; sigma: number };
  dimensions: [number, number];
  elapsed_ms: number;
}

export interface StackNormalizationApplied {
  offset: number;
  scale: number;
}

export interface StackResult {
  png_path: string;
  fits_path?: string;
  previewUrl?: string;
  frame_count?: number;
  rejected_pixels?: number;
  offsets?: [number, number][];
  dimensions: [number, number];
  elapsed_ms: number;
  rejection?: RejectionMethod;
  combine?: CombineMethod;
  normalization?: NormalizationMethod;
  rejection_normalization?: RejectionNormalization;
  normalization_applied?: StackNormalizationApplied[];
  rejection_low_fits?: string | null;
  rejection_high_fits?: string | null;
}

export interface PipelineChannel {
  label: string;
  paths: string[];
}

export interface PipelineRequest {
  channels: PipelineChannel[];
  dark_paths: string[];
  flat_paths: string[];
  bias_paths: string[];
  sigma_low?: number;
  sigma_high?: number;
  normalize?: boolean;
  align?: boolean;
  rejection?: RejectionMethod;
  combine?: CombineMethod;
  cosmetic?: CosmeticConfig | null;
  dark_optimize?: boolean;
}

export interface PipelineChannelStats {
  label: string;
  lights_input: number;
  lights_after_rejection?: number[];
  mean: number;
  stddev: number;
  cosmetic_replaced?: number | null;
  dark_scale_min?: number | null;
  dark_scale_max?: number | null;
  dark_scale_mean?: number | null;
}

export interface PipelineStats {
  darks_combined: number;
  flats_combined: number;
  bias_combined: number;
  channels: PipelineChannelStats[];
}

export interface PipelineChannelPreview {
  label: string;
  pixels_b64: string;
  width: number;
  height: number;
}

export interface PipelineResult {
  stats: PipelineStats;
  channel_previews: PipelineChannelPreview[];
  rgb_preview?: string;
  elapsed_ms?: number;
  warnings?: string[];
}

export interface CalibrateOptions {
  darkPaths?: string[];
  flatPaths?: string[];
  biasPaths?: string[];
  darkExposureRatio?: number;
}

export interface StackOptions {
  name?: string;
  sigmaLow?: number;
  sigmaHigh?: number;
  maxIterations?: number;
  align?: boolean;
  alignMethod?: string;
  weights?: number[];
  rejection?: RejectionMethod;
  combine?: CombineMethod;
  normalization?: NormalizationMethod;
  rejectionNormalization?: RejectionNormalization;
  winsorCutoff?: number;
  percentileLow?: number;
  percentileHigh?: number;
  minmaxLow?: number;
  minmaxHigh?: number;
  rejectionMaps?: boolean;
}

export interface DrizzleRgbOptions {
  scale?: number;
  pixfrac?: number;
  kernel?: "square" | "gaussian" | "lanczos3";
  align?: boolean;
  alignmentMethod?: "phase_correlation" | "zncc";
  sigmaLow?: number;
  sigmaHigh?: number;
  rejection?: RejectionMethod;
  wbMode?: "auto" | "manual" | "none";
  wbR?: number;
  wbG?: number;
  wbB?: number;
  scnrEnabled?: boolean;
  scnrAmount?: number;
  scnrMethod?: "average" | "maximum";
  saveFits?: boolean;
}

export interface DrizzleRgbResult {
  png_path: string;
  fits_path?: string | null;
  previewUrl?: string;
  dimensions: [number, number];
  output_dims: [number, number];
  input_dims: [number, number];
  frame_count_r: number;
  frame_count_g: number;
  frame_count_b: number;
  rejected_pixels: number;
  scale: number;
  elapsed_ms: number;
}

export const STACK_PROGRESS_EVENT = "stack-progress";
export const CALIBRATE_PROGRESS_EVENT = "calibrate-progress";
export const DRIZZLE_RGB_PROGRESS_EVENT = "drizzle-rgb-progress";
