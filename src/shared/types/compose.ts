import type { StfParams } from "./fits.types";

export interface ChannelStats {
  median: number;
  mean: number;
  min: number;
  max: number;
}

export interface BlendResult {
  png_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  elapsed_ms: number;
  stats_r?: ChannelStats;
  stats_g?: ChannelStats;
  stats_b?: ChannelStats;
  stf_r?: StfParams;
  stf_g?: StfParams;
  stf_b?: StfParams;
  auto_stf?: StfParams;
}

export interface AlignedChannel {
  path: string;
  offset: [number, number];
  confidence?: number;
  method_used?: string;
  matched_stars?: number;
  inliers?: number;
  residual_px?: number;
}

export interface AlignResult {
  channels: AlignedChannel[];
  align_method: string;
  dimensions: [number, number];
  elapsed_ms: number;
}

export interface RestretchResult {
  png_path: string;
  previewUrl?: string;
  elapsed_ms: number;
}

export interface AutoWbResult {
  r_factor: number;
  g_factor: number;
  b_factor: number;
  ref_channel: string;
}

export interface CalibrateCompositeResult {
  png_path: string;
  previewUrl?: string;
  auto_stf?: StfParams;
  elapsed_ms: number;
}

export interface CalibrateAndScnrResult {
  png_path: string;
  wb_applied: boolean;
  r_factor: number;
  g_factor: number;
  b_factor: number;
  scnr_applied: boolean;
  auto_stf?: StfParams;
  elapsed_ms: number;
}

export interface ResetWbResult {
  png_path: string;
  reset: boolean;
  r_factor: number;
  g_factor: number;
  b_factor: number;
  auto_stf?: StfParams;
  elapsed_ms: number;
}

export interface ScnrOptions {
  enabled: boolean;
  method?: string;
  amount?: number;
}
