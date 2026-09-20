export type GaiaBand = "G" | "BP" | "RP";

export const GAIA_BANDS: readonly GaiaBand[] = ["G", "BP", "RP"];

export type CatalogCsvKind = "catalog" | "sources" | "matches";

export interface CatalogRow {
  id: string;
  ra: number;
  dec: number;
  ra_epoch: number;
  dec_epoch: number;
  pm_ra_masyr: number | null;
  pm_dec_masyr: number | null;
  g: number | null;
  bp: number | null;
  rp: number | null;
  bp_rp: number | null;
  parallax_mas: number | null;
}

export interface PlacedCatalogRow extends CatalogRow {
  x: number | null;
  y: number | null;
  on_image: boolean;
}

export interface ConeSearchResult {
  rows: PlacedCatalogRow[];
  epoch_year: number | null;
  n_total: number;
  n_on_image: number;
  radius_arcmin: number;
  center_ra: number;
  center_dec: number;
  elapsed_ms: number;
  warnings: string[];
}

export interface ConeSearchOptions {
  radiusArcmin?: number | null;
  magLimit?: number | null;
  maxRows?: number | null;
}

export interface MeasuredSource {
  x: number;
  y: number;
  ra: number;
  dec: number;
  flux: number;
  fwhm: number;
  snr: number;
  mag_inst: number;
  mag_ab: number | null;
  saturated: boolean;
}

export interface MatchedCatalogRow {
  id: string;
  ra: number;
  dec: number;
  g: number | null;
  bp: number | null;
  rp: number | null;
  bp_rp: number | null;
}

export interface CrossMatchEntry {
  star: MeasuredSource;
  row: MatchedCatalogRow;
  sep_arcsec: number;
  d_ra_arcsec: number;
  d_dec_arcsec: number;
}

export interface AstrometrySummary {
  median_d_ra_arcsec: number;
  median_d_dec_arcsec: number;
  rms_arcsec: number;
  n: number;
}

export interface ZeroPointFit {
  zp: number;
  zp_err: number;
  colour_coeff: number | null;
  rms: number;
  n_used: number;
  n_rejected: number;
  band: GaiaBand;
  colour_term_used: boolean;
}

export interface CrossMatchResult {
  matches: CrossMatchEntry[];
  sources: MeasuredSource[];
  astrometry: AstrometrySummary | null;
  zero_point: ZeroPointFit | null;
  photcal_present: boolean;
  epoch_year: number | null;
  n_detected: number;
  n_catalog: number;
  band: GaiaBand;
  match_radius_arcsec: number;
  elapsed_ms: number;
  warnings: string[];
}

export interface CrossMatchOptions {
  sigma?: number | null;
  maxStars?: number | null;
  radiusArcsec?: number | null;
  band?: GaiaBand | null;
  colourTerm?: boolean | null;
  apertureRadius?: number | null;
}

export interface CatalogExportResult {
  output_path: string;
  n: number;
}
