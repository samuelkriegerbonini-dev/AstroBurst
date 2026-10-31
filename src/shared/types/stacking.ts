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
  dark_scale?: number;
  warnings?: string[];
  has_flat_dark?: boolean;
}

export interface StackNormalizationApplied {
  offset: number;
  scale: number;
}

export interface StackFrameOffset {
  dy: number;
  dx: number;
}

export interface StackFrameAlignment {
  path: string;
  method_used: string;
  confidence: number | null;
  included: boolean;
  star_match_rejection?: string | null;
}

export interface StackResult {
  png_path: string;
  fits_path?: string;
  previewUrl?: string;
  frame_count?: number;
  rejected_pixels?: number;
  offsets?: StackFrameOffset[];
  scale?: number;
  alignment?: StackFrameAlignment[];
  warnings?: string[];
  dimensions: [number, number];
  elapsed_ms: number;
  rejection?: RejectionMethod;
  combine?: CombineMethod;
  normalization?: NormalizationMethod;
  rejection_normalization?: RejectionNormalization;
  normalization_applied?: StackNormalizationApplied[];
  rejection_low_fits?: string | null;
  rejection_high_fits?: string | null;
  weights_applied?: (number | null)[];
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
  flat_dark_paths?: string[];
  sigma_low?: number;
  sigma_high?: number;
  max_iterations?: number;
  winsor_cutoff?: number;
  percentile_low?: number;
  percentile_high?: number;
  minmax_low?: number;
  minmax_high?: number;
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
  dark_group_temp_c?: number | null;
  dark_temp_delta_max_c?: number | null;
  warnings?: string[];
  excluded_frames?: number[];
}

export interface PipelineDarkGroup {
  temp_c: number | null;
  frames: number;
}

export interface PipelineStats {
  darks_combined: number;
  flats_combined: number;
  bias_combined: number;
  flat_darks_combined?: number;
  dark_groups?: PipelineDarkGroup[];
  channels: PipelineChannelStats[];
}

export interface PipelineChannelPreview {
  label: string;
  pixels_b64: string;
  width: number;
  height: number;
}

export interface PipelineMasterOutput {
  label: string;
  png_path: string;
  fits_path: string;
  dimensions: [number, number];
  input_path: string;
  previewUrl?: string;
}

export type PipelineRgbPalette = "RGB" | "SHO" | "HOO";

export interface PipelineResult {
  stats: PipelineStats;
  channel_previews: PipelineChannelPreview[];
  rgb_preview?: string;
  rgb_palette?: PipelineRgbPalette | null;
  elapsed_ms?: number;
  warnings?: string[];
  masters?: PipelineMasterOutput[];
  rgb_png_path?: string | null;
  rgb_dimensions?: [number, number] | null;
  rgbPreviewUrl?: string;
}

export interface CalibrateOptions {
  darkPaths?: string[];
  flatPaths?: string[];
  biasPaths?: string[];
  flatDarkPaths?: string[];
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

export type DrizzleAlignmentMethod = "phase_correlation" | "affine";

export const DRIZZLE_ALIGNMENT_METHODS: readonly { value: DrizzleAlignmentMethod; label: string }[] = [
  { value: "phase_correlation", label: "Phase Correlation" },
  { value: "affine", label: "Star-based (affine)" },
];

export interface DrizzleRgbOptions {
  scale?: number;
  pixfrac?: number;
  kernel?: "square" | "gaussian" | "lanczos3";
  align?: boolean;
  alignmentMethod?: DrizzleAlignmentMethod;
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
  warnings?: string[];
  elapsed_ms: number;
}

export const STACK_PROGRESS_EVENT = "stack-progress";
export const CALIBRATE_PROGRESS_EVENT = "calibrate-progress";
export const DRIZZLE_RGB_PROGRESS_EVENT = "drizzle-rgb-progress";
export const PIPELINE_PROGRESS_EVENT = "pipeline-progress";
export const MOSAIC_PROGRESS_EVENT = "mosaic-progress";

export type MosaicLevelModel = "offset" | "offset_scale";

export type MosaicCanvasRequest =
  | { mode: "auto"; north_up: boolean; reference: number | null }
  | { mode: "like"; path: string };

export type MosaicRejectRequest = { mode: "auto" } | { mode: "off" } | { mode: "on"; snr: number; scale: number };

export interface MosaicSolveRequest {
  online: boolean;
  use_twomass: boolean;
  max_sip_order: number | null;
}

export interface MosaicRequest {
  canvas: MosaicCanvasRequest;
  levels: MosaicLevelModel;
  references: (number | null)[];
  feather_px: number;
  dq_mask: boolean;
  reject: MosaicRejectRequest;
  solve: MosaicSolveRequest;
}

export interface MosaicRejectSpec {
  snr: number;
  scale: number;
}

export interface MosaicCanvasWindow {
  x0: number;
  y0: number;
  cols: number;
  rows: number;
}

export type MosaicWcsKind = "header" | "gwcs";

export interface MosaicTileSolve {
  solver: string;
  tier?: number | null;
  catalog?: string | null;
  matched?: number | null;
  predicted?: number | null;
  rms_px?: number | null;
  sip_order?: number | null;
  hint_source?: string | null;
  wcs_cards?: [string, string][];
}

export interface MosaicTileRow {
  name: string;
  path: string;
  width: number;
  height: number;
  planes: number;
  bunit: string | null;
  single_exposure: boolean;
  solve: MosaicTileSolve | null;
  coverage_frac?: number;
  scale_ratio?: number;
  rotation_deg?: number | null;
  wcs_kind?: MosaicWcsKind;
  window?: MosaicCanvasWindow;
}

export interface MosaicOverlapPair {
  i: number;
  j: number;
  frac: number;
}

export interface MosaicComponent {
  tiles: number[];
  reference: number;
}

export interface MosaicEstimate {
  cols: number;
  rows: number;
  megapixels: number;
  ram_bytes: number;
  canvas_cards: [string, string][];
  tiles: MosaicTileRow[];
  pairs: MosaicOverlapPair[];
  components: MosaicComponent[];
  isolated: number[];
  warnings: string[];
  refusal: string | null;
  default_reject: MosaicRejectSpec | null;
  reject: MosaicRejectSpec | null;
  planes: number;
}

export interface MosaicLevels {
  offsets: number[];
  scales: number[];
  unmatched: number[];
  unscaled: number[];
}

export interface MosaicPairResidual {
  i: number;
  j: number;
  dx: number | null;
  dy: number | null;
  matched: number;
  rms_px: number | null;
}

export interface MosaicTileRejection {
  tested_px: number;
  rejected_px: number;
  sigma: number | null;
}

export interface MosaicResult {
  fits_path: string;
  png_path: string;
  coverage_path: string | null;
  written: string[];
  dimensions: number[];
  planes: number;
  canvas: { cols: number; rows: number; megapixels: number; cards: [string, string][] };
  tiles: MosaicTileRow[];
  pairs: MosaicOverlapPair[];
  components: MosaicComponent[];
  isolated: number[];
  levels: MosaicLevels[];
  pair_residuals: MosaicPairResidual[];
  reject: MosaicRejectSpec | null;
  rejected_px: number;
  rejections: MosaicTileRejection[];
  warnings: string[];
  elapsed_ms: number;
  previewUrl?: string;
}
