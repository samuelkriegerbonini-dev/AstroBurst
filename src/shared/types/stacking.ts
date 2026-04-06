export interface CalibrateResult {
  png_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  elapsed_ms: number;
}

export interface StackResult {
  png_path: string;
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

export interface PipelineResult {
  channels: Array<{
    label: string;
    output_path: string;
    frames_calibrated: number;
  }>;
  elapsed_ms: number;
}

export interface CalibrateOptions {
  darkPath?: string;
  flatPath?: string;
  biasPath?: string;
  normalize?: boolean;
}

export interface StackOptions {
  name?: string;
  method?: string;
  sigmaLow?: number;
  sigmaHigh?: number;
  drizzleScale?: number;
  weightMode?: string;
}
