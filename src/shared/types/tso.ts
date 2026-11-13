export type TsoLayout = "rows" | "extensions";

export interface X1dIntsKind {
  kind: "x1dints";
  layout: TsoLayout;
  n_series: number;
  n_integrations: number;
  n_channels: number;
}

export interface X1dTableInfo {
  hdu: number;
  extver: number | null;
  n_rows: number;
  n_channels: number;
  sporder: number | null;
  detector: string | null;
  slit_name: string | null;
  source_id: number | null;
  source_name: string | null;
  int_num: number | null;
  srctype: string | null;
  label: string;
}

export interface Spectrum1dTables {
  tables: X1dTableInfo[];
}

export interface SeriesKey {
  detector: string;
  sporder: number | null;
}

export type TsoTimeScale = "bmjd_tdb" | "mjd_utc" | "index";

export interface TsoWhiteLightLog {
  min_um: number | null;
  max_um: number | null;
}

export interface TsoFile {
  path: string;
  file_name: string;
  sha256: string;
  level: number;
  datamodel: string | null;
  cal_ver: string | null;
  exp_type: string | null;
  layout: TsoLayout;
  nints: number | null;
  intstart: number | null;
  intend: number | null;
  exsegnum: number | null;
  exsegtot: number | null;
  instrument: string | null;
  detector: string | null;
  filter: string | null;
  pupil: string | null;
  grating: string | null;
  subarray: string | null;
  target: string | null;
  expstart: number | null;
  outlier_detection: string | null;
  whitelight_log: TsoWhiteLightLog | null;
  whtlt_path: string | null;
  n_extract1d: number;
  data_bytes: number;
}

export interface TsoExposureCompleteness {
  exposure: string;
  nints: number | null;
  found: number;
  missing: [number, number][];
  complete: boolean | null;
}

export interface TsoSeriesSummary {
  key: SeriesKey;
  label: string;
  n_integrations: number;
  n_channels: number;
  flux_unit: string;
  time_scale: TsoTimeScale;
  time_source: string;
  time_first: number | null;
  time_last: number | null;
  wavelength_min_um: number | null;
  wavelength_max_um: number | null;
  common_grid: boolean;
  outlier_detection: string | null;
  completeness: TsoExposureCompleteness[];
  notes: string[];
  warnings: string[];
}

export interface TsoOpenResult {
  set_id: string;
  files: TsoFile[];
  series: TsoSeriesSummary[];
  notes: string[];
  warnings: string[];
}

export type TsoPick =
  | { kind: "integration"; index: number }
  | { kind: "median" }
  | { kind: "mean"; first: number; last: number };

export type TsoCurveMode = "pipeline" | "masked";

export type TsoLimitsOrigin = "open" | "user" | "file log" | "built-in default";

export interface TsoCurve {
  band_um: [number, number];
  limits_origin: TsoLimitsOrigin;
  flux: (number | null)[];
  flux_err: (number | null)[];
  n_used: number[];
  npixels_mean: (number | null)[];
}

export interface TsoLightCurvesRequest {
  mode: TsoCurveMode;
  wavelength_min_um?: number | null;
  wavelength_max_um?: number | null;
  bins?: [number, number][];
  exclude_um?: [number, number][];
  reject_elements_sigma?: number | null;
}

export interface TsoRejectedElement {
  index: number;
  wavelength_um: number;
  reason: string;
}

export interface TsoLightCurves {
  set_id: string;
  label: string;
  series: SeriesKey;
  mode: TsoCurveMode;
  time_scale: TsoTimeScale;
  time: number[];
  mjd_utc: (number | null)[];
  int_num: number[];
  row_index: number[];
  flux_unit: string;
  white: TsoCurve;
  bins: TsoCurve[];
  dropped: { nan_time: number; repeated_time: number };
  rejected_elements: TsoRejectedElement[];
  notes: string[];
  warnings: string[];
}

export type WhtltSchema = "legacy" | "current";

export interface WhtltSeries {
  column: string;
  detector: string | null;
  sporder: number | null;
  mjd_utc: number[];
  bmjd_tdb: (number | null)[];
  flux: number[];
  flux_unit: string | null;
}

export interface WhtltImport {
  path: string;
  file_name: string;
  schema: WhtltSchema;
  series: WhtltSeries[];
  meta: Record<string, unknown> | null;
  notes: string[];
}

export interface WhtltComparison {
  series: SeriesKey | null;
  label: string | null;
  column: string;
  n_whtlt: number;
  n_joined: number;
  max_rel_diff: number | null;
  within_1e4: boolean;
  limits_um: [number, number] | null;
  limits_origin: TsoLimitsOrigin | null;
  notes: string[];
}
