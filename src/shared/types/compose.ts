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
  stf_linked?: boolean;
  stf_note?: string | null;
}

export interface AlignedChannel {
  path?: string;
  cache_key?: string;
  offset: [number, number];
  confidence?: number;
  method_used?: string;
  matched_stars?: number;
  inliers?: number;
  residual_px?: number;
  registered?: boolean;
  reprojected?: boolean;
  wcs_scale_ratio?: number | null;
  wcs_rotation_deg?: number | null;
  prefilter_k?: number | null;
  residual_measured?: boolean;
}

export interface AlignResult {
  channels: AlignedChannel[];
  align_method: string;
  dimensions: [number, number];
  elapsed_ms: number;
  reference_index?: number;
  reference_rule?: "finest_wcs" | "selected" | "first";
  run_token?: string | null;
  warnings?: string[];
}

export interface ChannelOverlayPreview {
  png_path: string;
  previewUrl: string;
  channel_previews: string[];
  channelPreviewUrls: string[];
  dimensions: [number, number];
  preview_dimensions: [number, number];
  elapsed_ms: number;
}

export interface CropBounds {
  dimensions: [number, number];
  crop_top: number;
  crop_bottom: number;
  crop_left: number;
  crop_right: number;
  auto_detected: boolean;
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
  empty_channels?: string[];
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
