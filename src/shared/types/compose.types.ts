export interface ChannelStats {
  median: number;
  mean: number;
  min: number;
  max: number;
}

export interface DimensionCrop {
  original_r: [number, number] | null;
  original_g: [number, number] | null;
  original_b: [number, number] | null;
  cropped_to: [number, number];
}

export type AlignMethod = "phase_correlation" | "affine";

export interface RgbComposeResult {
  png_path: string;
  previewUrl: string;
  dimensions: [number, number];
  stats_r: ChannelStats;
  stats_g: ChannelStats;
  stats_b: ChannelStats;
  offset_g: [number, number];
  offset_b: [number, number];
  scnr_applied: boolean;
  dimension_crop: DimensionCrop | null;
  resampled: boolean;
  lrgb_applied: boolean;
  elapsed_ms: number;
}

export interface DrizzleRgbResult {
  png_path: string;
  previewUrl: string;
  fits_path: string;
  dimensions: [number, number];
  input_dims: [number, number];
  output_dims: [number, number];
  frame_count_r: number;
  frame_count_g: number;
  frame_count_b: number;
  rejected_pixels: number;
  elapsed_ms: number;
  scale: number;
}

export interface DrizzleRgbOptions {
  scale?: number;
  pixfrac?: number;
  kernel?: "square" | "gaussian" | "lanczos3";
  sigmaLow?: number;
  sigmaHigh?: number;
  align?: boolean;
  wbMode?: "auto" | "none" | "manual";
  scnrEnabled?: boolean;
  scnrMethod?: "average" | "maximum";
  scnrAmount?: number;
}
