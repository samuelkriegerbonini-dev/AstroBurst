import { FILE_STATUS } from "./constants";

export type FileStatus = (typeof FILE_STATUS)[keyof typeof FILE_STATUS];

export interface AstroFile {
  name: string;
  path: string;
  size: number;
  lastModified?: number;
}

export interface ProcessedFile {
  id: string;
  name: string;
  path: string;
  size: number;
  status: FileStatus;
  result: ProcessResult | null;
  error: string | null;
  startedAt: number | null;
  finishedAt: number | null;
}

export interface ResampleResult {
  png_path: string;
  fits_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  original_dimensions: [number, number];
  wcs_updates: Record<string, any>;
  stats: {
    min: number;
    max: number;
    mean: number;
    sigma: number;
  };
}

export interface ProcessResult {
  png_path: string;
  previewUrl: string;
  dimensions: [number, number];
  elapsed_ms: number;
  header?: Record<string, string> | null;
  histogram?: HistogramData | null;
  stf?: StfParams | null;
  stats?: {
    min: number;
    max: number;
    mean: number;
    sigma: number;
    median: number;
    mad?: number;
  } | null;
  original_dimensions?: [number, number];
  wcs_updates?: Record<string, any>;
  resampled?: ResampleResult | null;
  resampledPath?: string | null;
}

export interface HistogramData {
  bins: number[];
  bin_count: number;
  data_min: number;
  data_max: number;
  median: number;
  mean: number;
  sigma: number;
  mad: number;
  total_pixels: number;
  auto_stf: StfParams;
  elapsed_ms?: number;
  bin_edges: number[];
}

export interface StfParams {
  shadow: number;
  midtone: number;
  highlight: number;
}

export interface StarDetectionResult {
  stars: Star[];
  count: number;
  background_median: number;
  background_sigma: number;
  threshold_sigma: number;
  image_width: number;
  image_height: number;
  elapsed_ms: number;
}

export interface Star {
  x: number;
  y: number;
  flux: number;
  fwhm: number;
  peak: number;
  npix: number;
  snr: number;
}

export interface RawPixelData {
  data: Float32Array;
  width: number;
  height: number;
  min: number;
  max: number;
}

export interface FftData {
  pixels: Uint8Array;
  width: number;
  height: number;
  dc_magnitude: number;
  max_magnitude: number;
  elapsed_ms: number;
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

export interface ChannelStats {
  median: number;
  mean: number;
  sigma: number;
  min: number;
  max: number;
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

export interface CubeInfo {
  naxis1: number;
  naxis2: number;
  naxis3: number;
  bitpix: number;
  bytes_per_pixel: number;
  frame_bytes: number;
  total_data_bytes: number;
  wavelengths: number[] | null;
}

export interface WcsInfo {
  center_ra: number;
  center_dec: number;
  center_str: string;
  pixel_scale_arcsec: number;
  fov_arcmin: [number, number];
  corners: Array<{ ra: number; dec: number }>;
}

export interface HeaderData {
  file_name: string;
  file_path: string;
  total_cards: number;
  cards: Array<{ key: string; value: string }>;
  categories: Record<string, Record<string, string>>;
  filter_detection: {
    filter: string;
    filter_id: string;
    hubble_channel: string;
    confidence: string;
    matched_keyword: string;
    matched_value: string;
  } | null;
  filename_hint: string | null;
}

export interface AppConfig {
  has_api_key: boolean;
  astrometry_api_url: string;
  default_output_dir: string;
  plate_solve_timeout_secs: number;
  plate_solve_max_stars: number;
  auto_stretch_target_bg: number;
  auto_stretch_shadow_k: number;
}

export interface QueueStats {
  total: number;
  done: number;
  failed: number;
  totalBytes: number;
}
