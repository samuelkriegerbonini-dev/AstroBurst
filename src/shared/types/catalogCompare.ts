export type CatalogueFluxUnit = "nanomaggy" | "Jy" | "mJy" | "uJy" | "nJy" | "zero_point" | "ab_mag";
export type CompareCutOp = "<" | "<=" | "==" | "!=" | ">=" | ">";

export interface CompareMapping {
  id: string | null;
  ra: string;
  dec: string;
  flux: string;
  flux_unit: CatalogueFluxUnit;
  zero_point: number | null;
  flux_err: string | null;
  flux_ivar: string | null;
  class: string | null;
  missing_value: number | null;
}

export interface CompareCut {
  column: string;
  op: CompareCutOp;
  value: number | string;
}

export interface CompareAperture {
  radius_arcsec: number | null;
  radius_px: number | null;
  annulus_arcsec: [number, number] | null;
  annulus_px: [number, number] | null;
}

export interface ComparePreset {
  id: string;
  label: string;
  band: string;
  mapping: CompareMapping;
  aperture: CompareAperture;
  cuts: CompareCut[];
  min_snr: number | null;
  max_snr: number | null;
  min_neighbour_arcsec: number | null;
  notes: string[];
}

export interface CompareColumn {
  name: string;
  numeric: boolean;
  unit: string | null;
}

export interface CompareProbe {
  table_path: string;
  file_name: string;
  format: "csv" | "ecsv";
  n_rows: number;
  columns: CompareColumn[];
  presets: ComparePreset[];
  notes: string[];
}

export interface CompareRequest {
  band: { path: string; label: string | null; dataset: string | null };
  table_path: string;
  preset: string | null;
  catalogue_band: string | null;
  mapping: CompareMapping;
  aperture: CompareAperture;
  cuts: CompareCut[];
  min_snr: number | null;
  max_snr: number | null;
  min_neighbour_arcsec: number | null;
  exclude_dq: boolean | null;
}

export interface CompareRow {
  row: number;
  id: string;
  ra_deg: number;
  dec_deg: number;
  x: number | null;
  y: number | null;
  class: string | null;
  flux_cat_jy: number | null;
  flux_err_cat_jy: number | null;
  snr_cat: number | null;
  flux_native: number | null;
  flux_err_native: number | null;
  flux_jy: number | null;
  flux_err_jy: number | null;
  ratio: number | null;
  masked_fraction: number | null;
  used: boolean;
  reason: string | null;
}

export interface CompareBandInfo {
  file_name: string;
  label: string;
  filter: { dataset: string; filter: string; pivot_um: number; source: string } | null;
  calibration_origin: string | null;
  jy_per_native_unit: number | null;
  bunit: string | null;
  pixel_scale_arcsec: number | null;
  aperture_radius_px: number | null;
  annulus_px: [number, number] | null;
  err_origin: { role: string; source: string; assumed: boolean; n_no_data: number } | null;
  dq_source: string | null;
  warnings: string[];
}

export interface CompareApertureUsed {
  radius_px: number;
  radius_arcsec: number;
  annulus_px: [number, number] | null;
  pixel_scale_arcsec: number;
  local_sky: boolean;
}

export interface CompareReport {
  n_table: number;
  n_on_image: number;
  n_after_cuts: number;
  n_measured: number;
  n_used: number;
  ratio_median: number | null;
  ratio_nmad: number | null;
  ratio_median_err: number | null;
  n_ratio: number;
  ratio_offset_mag: number | null;
  err_ratio_median: number | null;
  err_ratio_nmad: number | null;
  n_err: number;
  implied_zero_point_ab: number | null;
  implied_zero_point_nmad: number | null;
  n_zero_point: number;
  rows: CompareRow[];
  rejected: { reason: string; count: number }[];
  aperture: CompareApertureUsed;
  band: CompareBandInfo;
  calibration_origin: string | null;
  preset: string | null;
  catalogue_band: string | null;
  table: { path: string; file_name: string; format: "csv" | "ecsv"; n_rows: number };
  notes: string[];
  warnings: string[];
  elapsed_ms: number;
}
