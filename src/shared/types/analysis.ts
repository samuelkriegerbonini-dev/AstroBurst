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
