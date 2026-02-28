export const FILE_STATUS = {
  QUEUED: "queued",
  PROCESSING: "processing",
  DONE: "done",
  ERROR: "error",
} as const;

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

export interface ProcessResult {
  png_path: string;
  previewUrl: string;
  dimensions: [number, number];
  elapsed_ms: number;
  header?: Record<string, string> | null;
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
  elapsed_ms: number;
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

/**
 * @interface DrizzleRgbResult
 * @description Result from the integrated RGB drizzle pipeline
 *
 * This interface represents the output of the drizzle_rgb_cmd command,
 * which performs drizzle stacking on three color channels and composes
 * them into a single RGB image.
 *
 * @property {string} png_path - Absolute path to the output RGB PNG preview
 * @property {string} previewUrl - Tauri asset protocol URL for webview display
 * @property {string} fits_path - Absolute path to the output RGB FITS file
 * @property {[number, number]} input_dims - Original input dimensions [width, height]
 * @property {[number, number]} output_dims - Drizzled output dimensions [width, height]
 * @property {number} frame_count_r - Number of frames processed for red channel
 * @property {number} frame_count_g - Number of frames processed for green channel
 * @property {number} frame_count_b - Number of frames processed for blue channel
 * @property {number} rejected_pixels - Total number of sigma-rejected pixels
 * @property {number} elapsed_ms - Total processing time in milliseconds
 * @property {number} scale - Applied drizzle scale factor
 * @property {ChannelStats} stats_r - Red channel statistics (median, sigma, etc.)
 * @property {ChannelStats} stats_g - Green channel statistics
 * @property {ChannelStats} stats_b - Blue channel statistics
 */
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

/**
 * @interface ChannelStats
 * @description Per-channel statistics from drizzle processing
 */
export interface ChannelStats {
  median: number;
  mean: number;
  sigma: number;
  min: number;
  max: number;
}

/**
 * @interface DrizzleRgbOptions
 * @description Configuration options for the RGB drizzle pipeline
 *
 * @property {number} scale - Output scale factor (1.5, 2.0, or 3.0)
 * @property {number} pixfrac - Pixel fraction / drop shrink factor (0.1 to 1.0)
 * @property {string} kernel - Interpolation kernel: 'square' | 'gaussian' | 'lanczos3'
 * @property {number} sigmaLow - Lower sigma clipping threshold
 * @property {number} sigmaHigh - Upper sigma clipping threshold
 * @property {boolean} align - Enable sub-pixel alignment via ZNCC
 * @property {string} wbMode - White balance mode: 'auto' | 'none' | 'manual'
 * @property {boolean} scnrEnabled - Enable SCNR green removal
 * @property {string} scnrMethod - SCNR method: 'average' | 'maximum'
 * @property {number} scnrAmount - SCNR strength (0.0 to 1.0)
 */
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
