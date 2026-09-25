import type { PhotCal } from "../../services/analysis";

export type RegionSystem = "image" | "physical" | "fk5" | "icrs";

export const REGION_SYSTEMS: readonly RegionSystem[] = ["image", "physical", "fk5", "icrs"];

export type RegionShape =
  | { shape: "circle"; x: number; y: number; r: number }
  | { shape: "ellipse"; x: number; y: number; rx: number; ry: number; angle: number }
  | { shape: "box"; x: number; y: number; width: number; height: number; angle: number }
  | { shape: "annulus"; x: number; y: number; r_inner: number; r_outer: number }
  | { shape: "polygon"; points: [number, number][] }
  | { shape: "line"; x1: number; y1: number; x2: number; y2: number }
  | { shape: "point"; x: number; y: number };

export type RegionShapeKind = RegionShape["shape"];

export const REGION_SHAPE_KINDS: readonly RegionShapeKind[] = [
  "circle",
  "ellipse",
  "box",
  "annulus",
  "polygon",
  "line",
  "point",
];

export type RegionTool = "none" | "select" | RegionShapeKind;

export interface RegionProperties {
  color: string | null;
  width: number | null;
  text: string | null;
  dash: boolean | null;
  include: boolean;
}

export interface Region {
  id: string;
  shape: RegionShape;
  props: RegionProperties;
  backgroundId: string | null;
}

export interface RegionWire {
  shape: RegionShape;
  props: RegionProperties;
}

export interface PixelBounds {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export interface BackgroundEstimate {
  median: number;
  sigma: number;
  count: number;
}

export interface RegionStats {
  count: number;
  n_nan: number;
  n_excluded: number;
  n_padding: number;
  area: number;
  bounds: PixelBounds;
  clipped: boolean;
  sum: number;
  mean: number;
  median: number;
  mad: number;
  sigma: number;
  std: number;
  min: number;
  max: number;
  clipped_mean: number | null;
  clipped_median: number | null;
  clipped_sigma: number | null;
  n_rejected: number;
  background: BackgroundEstimate | null;
  net_sum: number | null;
  net_snr: number | null;
  sum_err?: number | null;
  weighted_mean?: number | null;
  calibrated?: RegionCalibrated | null;
  sky?: RegionSky | null;
}

export interface RegionSky {
  ra: number | null;
  dec: number | null;
  pa_sky_deg: number | null;
  area_arcsec2: number | null;
  geometric_area_arcsec2: number | null;
}

export type RegionFluxSource = "net" | "sum";

export interface RegionCalibrated {
  flux_source: RegionFluxSource;
  flux_native: number;
  flux_err_native: number | null;
  flux_jy: number;
  flux_err_jy: number | null;
  mag_ab: number | null;
  mag_ab_err: number | null;
  st_mag: number | null;
  area_arcsec2: number | null;
  geometric_area_arcsec2: number | null;
  sb_mag_arcsec2: number | null;
  ra: number | null;
  dec: number | null;
  pa_sky_deg: number | null;
}

export interface RegionStatsEntry {
  id: string;
  stats: RegionStats | null;
  error: string | null;
}

export interface RegionStatsResult {
  regions: RegionStatsEntry[];
  masked: boolean;
  dq_excluded: number | null;
  elapsed_ms: number;
  photcal?: PhotCal | null;
  calibration_warnings?: string[];
  pixel_area_arcsec2?: number | null;
}

export interface RadialBin {
  r: number;
  count: number;
  mean: number | null;
  median: number | null;
  std: number | null;
  cumulative_sum: number;
}

export interface RadialProfile {
  x: number;
  y: number;
  max_radius: number;
  background: BackgroundEstimate | null;
  bins: RadialBin[];
  masked: boolean;
  elapsed_ms: number;
}

export interface LineCut {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  length: number;
  n_samples: number;
  distance: number[];
  xs: number[];
  ys: number[];
  values: (number | null)[];
  masked: boolean;
  elapsed_ms: number;
}

export interface RegionImportResult {
  regions: RegionWire[];
  warnings: string[];
  has_wcs: boolean;
}

export interface RegionExportResult {
  reg_text: string;
  system: RegionSystem;
}

export interface SbBin {
  sma_inner: number;
  sma_outer: number;
  sma: number;
  count: number;
  cumulative_count: number;
  mean: number | null;
  median: number | null;
  std: number | null;
  cumulative_sum: number;
  sma_arcsec: number | null;
  mu_ab: number | null;
  mu_err: number | null;
  mag_ab_cumulative: number | null;
}

export interface SbProfile {
  x: number;
  y: number;
  sma_max: number;
  ellipticity: number;
  angle_deg: number;
  bin_width: number;
  background: BackgroundEstimate | null;
  bins: SbBin[];
  r50_px: number | null;
  r80_px: number | null;
  r90_px: number | null;
  petrosian_radius_px: number | null;
  pixel_scale_arcsec: number | null;
  pixel_area_arcsec2: number | null;
  sky_pa_deg: number | null;
  photcal: PhotCal | null;
  calibration_warnings: string[];
  total_mag_ab: number | null;
  notes: string[];
  masked: boolean;
  elapsed_ms: number;
}
