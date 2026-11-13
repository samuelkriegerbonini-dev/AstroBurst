import type { PhotCal, UserZeroPoint } from "../../services/analysis";

export const FORCED_PHOTOMETRY_PROGRESS_EVENT = "forced-photometry-progress";
export const FORCED_MAX_BANDS = 12;
export const FORCED_MAX_POSITIONS = 5000;

export type ForcedCorrection = "none" | "growth_curve";

export interface ForcedBandInput {
  path: string;
  label?: string | null;
  dataset?: string | null;
}

export interface ForcedSkyPosition {
  id: string;
  ra_deg: number;
  dec_deg: number;
}

export interface ForcedAperture {
  radius_arcsec?: number | null;
  radius_px?: number | null;
  annulus_arcsec?: [number, number] | null;
  annulus_px?: [number, number] | null;
  correction: ForcedCorrection;
  reference_radius_arcsec?: number | null;
}

export interface ForcedRunOptions {
  exclude_dq?: boolean | null;
  gain?: number | null;
  limit_sigma?: number | null;
  f_corr?: (number | null)[] | null;
  zero_points?: (UserZeroPoint | null)[] | null;
}

export interface ForcedFilter {
  dataset: string;
  filter: string;
  pivot_um: number;
  source: string;
}

export interface ForcedErrOrigin {
  role: string;
  source: string;
  assumed: boolean;
  n_no_data: number;
}

export interface ForcedApertureCorrection {
  radius_px: number;
  reference_radius_px: number;
  factor: number;
  scatter: number;
  n_stars: number;
  n_candidates: number;
}

export interface ForcedFlagName {
  value: number;
  name: string;
}

export interface ForcedBand {
  index: number;
  path: string;
  file_name: string;
  label: string;
  error: string | null;
  filter: ForcedFilter | null;
  photcal: PhotCal | null;
  calibration_origin: string | null;
  jy_per_native_unit: number | null;
  bunit: string | null;
  pixel_scale_arcsec: number | null;
  aperture_radius_px: number | null;
  annulus_px: [number, number] | null;
  aperture_correction: ForcedApertureCorrection | null;
  err_origin: ForcedErrOrigin | null;
  dq_source: string | null;
  f_corr: number | null;
  n_measured: number;
  n_failed: number;
  warnings: string[];
  flag_names?: ForcedFlagName[];
  mag_system?: string | null;
}

export interface ForcedCell {
  x: number | null;
  y: number | null;
  on_image: boolean;
  error: string | null;
  flux_native: number | null;
  flux_err_native: number | null;
  flux_jy: number | null;
  flux_err_jy: number | null;
  flux_err_corr_jy: number | null;
  flux_total_jy: number | null;
  mag_ab: number | null;
  mag_ab_err: number | null;
  upper_limit_jy: number | null;
  upper_limit_ab: number | null;
  snr: number | null;
  masked_fraction: number | null;
  n_nonfinite: number;
  n_masked: number;
  flags_or: number | null;
  saturated: boolean;
  fwhm_px: number | null;
}

export interface ForcedSource {
  id: string;
  ra_deg: number;
  dec_deg: number;
  cells: (ForcedCell | null)[];
}

export interface ForcedPhotometryRequest {
  bands: ForcedBandInput[];
  positions: ForcedSkyPosition[];
  aperture: ForcedAperture;
  options?: ForcedRunOptions;
}

export interface ForcedPhotometryResult {
  bands: ForcedBand[];
  sources: ForcedSource[];
  aperture: ForcedAperture;
  limit_sigma: number;
  elapsed_ms: number;
  warnings: string[];
}
