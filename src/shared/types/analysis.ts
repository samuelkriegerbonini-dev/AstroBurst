import type { StfParams } from "./fits.types";
import type { DqProbe, ErrProbe } from "./dq";
import type { FrameGeometry, GeometryTarget } from "./geometry";
import type { StarPhotometry } from "../../services/analysis";

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
  grid_width?: number;
  grid_height?: number;
  windowed?: boolean;
  downsampled?: boolean;
  image_width?: number;
  image_height?: number;
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

export const TIME_SERIES_PROGRESS_EVENT = "time-series-progress";

export type TimeSeriesRole = "target" | "comp" | "check" | "ignore";

export interface TimeSeriesTarget {
  x: number;
  y: number;
  label: string;
  role: TimeSeriesRole;
}

export interface TimeSeriesFrameOffset {
  dx: number;
  dy: number;
  confidence: number;
  registered: boolean;
}

export interface TimeSeriesFrame {
  index: number;
  path: string;
  file_name: string;
  jd_mid: number | null;
  time_source: string | null;
  exptime: number | null;
  filter: string | null;
  airmass: number | null;
  geometry: FrameGeometry | null;
  offset: TimeSeriesFrameOffset | null;
  photcal_label: string | null;
  targets: (StarPhotometry | null)[];
  errors: (string | null)[];
  skipped: string | null;
}

export interface TimeSeriesResult {
  reference_path: string;
  targets: TimeSeriesTarget[];
  frames: TimeSeriesFrame[];
  n_frames: number;
  n_skipped: number;
  warnings: string[];
  geometry_target: GeometryTarget | null;
  geometry_notes: string[];
  elapsed_ms: number;
}

export interface PixelTableStats {
  min: number | null;
  max: number | null;
  mean: number | null;
  median: number | null;
  n_finite: number;
  n_nan: number;
}

export interface PixelTableResult {
  x: number;
  y: number;
  size: number;
  x0: number;
  y0: number;
  values: (number | null)[][];
  err: (number | null)[][] | null;
  dq: (number | null)[][] | null;
  dq_names: (string | null)[][] | null;
  dq_table: string | null;
  unit: string | null;
  stats: PixelTableStats;
  err_stats: PixelTableStats | null;
  elapsed_ms: number;
}
