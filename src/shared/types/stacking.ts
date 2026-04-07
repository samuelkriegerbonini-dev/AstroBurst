export interface CalibrateResult {
  png_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  elapsed_ms: number;
}

export interface StackResult {
  png_path: string;
  fits_path?: string;
  previewUrl?: string;
  dimensions: [number, number];
  elapsed_ms: number;
  frames_stacked: number;
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
}

export interface PipelineChannelStats {
  label: string;
  lights_input: number;
  lights_after_rejection?: number[];
  mean: number;
  stddev: number;
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
}

export interface CalibrateOptions {
  darkPath?: string;
  flatPath?: string;
  biasPath?: string;
  darkPaths?: string[];
  flatPaths?: string[];
  biasPaths?: string[];
  darkExposureRatio?: number;
  normalize?: boolean;
}

export interface StackOptions {
  name?: string;
  method?: string;
  sigmaLow?: number;
  sigmaHigh?: number;
  maxIterations?: number;
  align?: boolean;
  drizzleScale?: number;
  weightMode?: string;
}
