import type { StfParams } from "./fits.types";

export interface ChannelStats {
  median: number;
  mean: number;
  sigma: number;
  min: number;
  max: number;
}

export interface RgbComposeResult {
  png_path: string;
  previewUrl: string;
  width: number;
  height: number;
  stf_r: StfParams;
  stf_g: StfParams;
  stf_b: StfParams;
  elapsed_ms: number;
}

export interface DrizzleRgbResult {
  png_path: string;
  previewUrl: string;
  fits_path: string;
  input_dims: [number, number];
  output_dims: [number, number];
  frame_count_r: number;
  frame_count_g: number;
  frame_count_b: number;
  rejected_pixels: number;
  elapsed_ms: number;
  scale: number;
  stats_r: ChannelStats | null;
  stats_g: ChannelStats | null;
  stats_b: ChannelStats | null;
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
