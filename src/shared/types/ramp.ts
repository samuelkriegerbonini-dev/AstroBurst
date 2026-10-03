import type { CubeDims } from "./cube";

export type RampKind = "jwst_groups" | "roman_resultants";

export type TgroupSource = "tgroup" | "tframe_product" | "none";

export type RampSource = "fits" | "asdf";

export interface Irs2Header {
  nrs_norm: number;
  nrs_ref: number;
  noutputs: number;
  fast_axis: number | null;
  slow_axis: number | null;
}

export interface RampInfo {
  kind: RampKind;
  nints: number;
  ngroups: number;
  nframes: number;
  groupgap: number;
  tframe_s: number | null;
  tgroup_s: number | null;
  tgroup_source: TgroupSource;
  group_times_s: number[] | null;
  readpatt: string | null;
  instrument: string | null;
  detector: string | null;
  exp_type: string | null;
  datamodl: string | null;
  irs2: Irs2Header | null;
  frame_width: number;
  frame_height: number;
}

export type CubeInfoWithRamp = CubeDims & { ramp?: RampInfo | null };

export interface RampOpenInfo {
  source: RampSource;
  ramp: RampInfo | null;
  frames: number;
  width: number;
  height: number;
  array_key: string | null;
}

export interface RampPixelSeries {
  x: number;
  y: number;
  integration: number;
  ngroups: number;
  nints: number;
  values: number[];
  group_times_s: number[] | null;
  unit: string;
}

export interface TablePreviewColumn {
  name: string;
  unit: string | null;
  values: (number | null)[];
}

export interface TablePreview {
  hdu: number;
  extname: string;
  n_rows: number;
  shown_rows: number;
  columns: TablePreviewColumn[];
  omitted_columns: string[];
}

export interface RampTables {
  group: TablePreview | null;
  int_times: TablePreview | null;
}

export interface RampFrameResult {
  frame_index: number;
  output_path: string;
  fits_path: string | null;
}

export type RefCorrection = "auto" | "amplifier" | "off";

export const REF_CORRECTIONS: readonly RefCorrection[] = ["auto", "amplifier", "off"];

export interface QuickSlopeParams {
  sat_dn: number;
  jump_k: number;
  scale_floor_dn: number;
  min_groups_ols: number;
  ref_correction: RefCorrection;
  ref_window_rows: number | null;
}

export const DEFAULT_QUICK_SLOPE_PARAMS: QuickSlopeParams = {
  sat_dn: 62258,
  jump_k: 5,
  scale_floor_dn: 3,
  min_groups_ols: 4,
  ref_correction: "auto",
  ref_window_rows: 200,
};

export interface FlagCounts {
  saturated: number;
  jump_det: number;
  do_not_use: number;
}

export interface QuickSlopeResult {
  fits_path: string;
  png_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  integration: number;
  nints: number;
  ngroups: number;
  tgroup_s: number;
  tgroup_source: TgroupSource;
  detector: string | null;
  readpatt: string | null;
  ref_corrected: boolean;
  stripped: boolean;
  params: QuickSlopeParams;
  band_scales_dn: number[];
  counts: FlagCounts;
  rate_sibling: string | null;
  warnings: string[];
  elapsed_ms: number;
}

export interface PixelFit {
  slope: number | null;
  ngood: number;
  dq: number;
  noise: number | null;
  n_usable: number;
  first_saturated: number | null;
}

export interface RampPixelFit {
  x: number;
  y: number;
  integration: number;
  group_times_s: number[];
  raw: number[];
  corrected: number[];
  ref_offsets: number[] | null;
  band_scale_dn: number;
  fit: PixelFit;
  flagged_diffs: number[];
  amplifier: number | null;
  science_row: number | null;
}

export interface CompareBin {
  lo: number | null;
  hi: number | null;
  n: number;
  median_rate: number;
  median_err: number | null;
  median_delta: number;
  median_rel: number | null;
  p16_delta: number;
  p84_delta: number;
}

export interface AmpBins {
  amplifier: number;
  science_rows: [number, number];
  bins: CompareBin[];
}

export interface ShiftCheck {
  best_dy: number;
  corr_at_zero: number | null;
  corr_best: number | null;
  second_dy: number;
  second_corr: number | null;
  passed: boolean;
}

export interface FlagConfusion {
  tp: number;
  fp: number;
  fn_: number;
  recall: number | null;
  precision: number | null;
}

export interface Histogram {
  edges: number[];
  counts: number[];
}

export interface RateComparison {
  qslope_path: string;
  rate_path: string;
  detector: string | null;
  good_pixels: number;
  quick_flagged_in_good: number;
  bins: CompareBin[];
  amp_bins: AmpBins[];
  zero_shift: ShiftCheck;
  jump: FlagConfusion;
  saturated: FlagConfusion;
  saturated_rule: string;
  saturated_any_group: FlagConfusion;
  rel_hist: Histogram;
  delta_hist: Histogram;
  science_rows: [number, number] | null;
  ratio_png_path: string;
  ratio_fits_path: string;
  ratioPreviewUrl?: string;
  elapsed_ms: number;
}
