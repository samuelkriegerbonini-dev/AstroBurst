import type { CubePrepProgress } from "./cubeprep";

export type SaguiSedScaling = "median_centre" | "median_mad";

export interface SaguiConfig {
  input_path: string;
  output_dir: string;
  rscript_path: string;
  library_path: string;
  n_regions: number;
  starlet: boolean;
  starlet_scales: number[];
  starlet_threshold: number;
  dq_exclude_bits: number;
  psf_target: string;
  mode: "ward" | "sparse";
  knn_k: number;
  sed_scaling?: SaguiSedScaling;
}

export interface SaguiBand {
  filter: string;
  pivot_um: number;
  psf_fwhm_arcsec?: number | null;
  pixscale_arcsec?: number | null;
  psf_area_px?: number | null;
}

export interface SaguiPlaneDqFlags {
  filter: string;
  table: string;
  flagged_finite: Record<string, number>;
}

export interface SaguiInputInfo {
  width: number;
  height: number;
  bands: SaguiBand[];
  unit: string;
  has_err: boolean;
  has_dq: boolean;
  has_noise_correlation?: boolean;
  dq_flags?: SaguiPlaneDqFlags[];
  estimated_memory_bytes: number;
  warnings: string[];
  redshift?: number | null;
}

export interface SaguiRuntimeInfo {
  available: boolean;
  ready: boolean;
  r_version: string | null;
  sagui_version: string | null;
  sagui_commit: string | null;
  issues: string[];
}

export interface SaguiRegionalBand extends SaguiBand {
  flux: number;
  error: number | null;
  error_correlated?: number | null;
}

export interface SaguiRegion {
  region: number;
  n_pixels: number;
  n_components?: number;
  min_snr?: number | null;
  min_snr_lower_bound?: number | null;
  bands: SaguiRegionalBand[];
}

export interface SaguiResult {
  run_id: string;
  unit: string;
  output_dir: string;
  labels_path: string;
  mask_path: string;
  table_path: string;
  manifest_path: string;
  width: number;
  height: number;
  labels: number[];
  regions: SaguiRegion[];
  warnings: string[];
}

export type SaguiProgress = CubePrepProgress;

export interface SaguiSavedResult {
  result: SaguiResult;
  config: SaguiConfig;
}

export interface SaguiInstallation {
  library_path: string;
  rscript_path: string;
  manifest_path: string;
  warnings: string[];
}
