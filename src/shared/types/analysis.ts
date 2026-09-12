import type { StfParams } from "./fits.types";
import type { DqProbe, ErrProbe } from "./dq";

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
  masked?: boolean;
  dq_excluded?: number | null;
}

export interface RawPixelData {
  data: Float32Array;
  width: number;
  height: number;
  min: number;
  max: number;
}

export interface RawRgbChannel {
  data: Float32Array;
  min: number;
  max: number;
}

export interface RawRgbPixelData {
  width: number;
  height: number;
  displayReferred?: boolean;
  r: RawRgbChannel;
  g: RawRgbChannel;
  b: RawRgbChannel;
}

export interface FftData {
  pixels: Uint8Array;
  width: number;
  height: number;
  dc_magnitude: number;
  max_magnitude: number;
  elapsed_ms: number;
}

export interface PixelNeighborhood {
  min: number | null;
  max: number | null;
  mean: number | null;
  median: number | null;
  n_pixels: number;
  n_nan: number;
}

export interface PixelProbeResult {
  x: number;
  y: number;
  value: number | null;
  unit: string | null;
  box: number;
  neighborhood: PixelNeighborhood;
  dq: DqProbe | null;
  err: ErrProbe | null;
}
