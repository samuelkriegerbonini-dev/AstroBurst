import type { StfParams } from "./fits.types";

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
