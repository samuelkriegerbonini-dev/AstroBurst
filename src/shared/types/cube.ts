import type { SpectralAxisInfo, VelocityConvention } from "./spectral";

export interface CubeDims {
  width: number;
  height: number;
  frames: number;
  bitpix?: number;
  wavelengths?: number[];
  spectral_classification?: {
    is_spectral: boolean;
    reason: string | null;
    axis_type?: string | null;
    axis_unit?: string | null;
    channel_count?: number;
  };
  spectral_axis?: SpectralAxisInfo | null;
}

export interface CubeSpectrum {
  wavelengths: number[];
  values: number[];
  x: number;
  y: number;
  is_spectral?: boolean;
  flux_jy?: number[] | null;
}

export interface RegionSpectrum {
  sum: number[];
  mean: number[];
  npix: number;
  n_bg: number;
  bg_per_pixel: number[] | null;
  wavelengths: number[] | null;
  unit: string;
  bg_subtracted: boolean;
  flux_jy: number[] | null;
  elapsed_ms: number;
}

export type CollapseRangeMode = "sum" | "mean" | "median";

export const COLLAPSE_RANGE_MODES: readonly CollapseRangeMode[] = ["sum", "mean", "median"];

export interface CollapseRangeResult {
  png_path: string;
  fits_path: string;
  z0: number;
  z1: number;
  mode: CollapseRangeMode;
  dimensions: [number, number];
  elapsed_ms: number;
  previewUrl?: string;
}

export type ContinuumWindows = [[number, number], [number, number]];

export interface MomentConfig {
  z0: number;
  z1: number;
  rest_um: number | null;
  convention: VelocityConvention;
  continuum: ContinuumWindows | null;
  snr_threshold: number;
  mask_below_threshold: boolean;
}

export interface MomentMapFiles {
  png_path: string;
  fits_path: string;
  previewUrl?: string;
}

export type MomentKind = "m0" | "m1" | "m2";

export const MOMENT_KINDS: readonly MomentKind[] = ["m0", "m1", "m2"];

export interface MomentMapsResult {
  m0: MomentMapFiles;
  m1: MomentMapFiles;
  m2: MomentMapFiles;
  m0_unit: string;
  velocity_unit: string;
  noise_per_channel: number | null;
  n_channels: number;
  notes: string[];
  dimensions: [number, number];
  elapsed_ms: number;
}

export type LineFitComponents = "one" | "two" | "auto";

export const LINE_FIT_COMPONENTS: readonly LineFitComponents[] = ["one", "two", "auto"];

export type LineFitWeighting = "err" | "continuum";

export interface LineFitConfig {
  z0: number;
  z1: number;
  rest_um: number | null;
  convention: VelocityConvention;
  continuum: ContinuumWindows | null;
  snr_threshold: number;
  emission_only: boolean;
  use_err: boolean;
  use_dq: boolean;
  resolving_power: number | null;
  components: LineFitComponents;
}

export type LineFitBasePlane =
  | "flux"
  | "velocity"
  | "sigma_obs"
  | "sigma_corr"
  | "flux_err"
  | "v_err"
  | "sigma_err"
  | "chi2_red"
  | "snr"
  | "mask"
  | "ncomp";

export type LineFitComponentPlane = `c${1 | 2}_${"flux" | "velocity" | "sigma_obs" | "flux_err" | "v_err" | "sigma_err"}`;

export type LineFitPlane = LineFitBasePlane | LineFitComponentPlane;

export interface LineFitUnits {
  flux: string;
  velocity: string;
  sigma: string;
}

export interface LineFitResult {
  planes: Record<string, MomentMapFiles>;
  plane_order: LineFitPlane[];
  units: LineFitUnits;
  sigma_label: string;
  weighting: LineFitWeighting;
  err_hdu: number | null;
  dq_hdu: number | null;
  z0: number;
  z1: number;
  n_channels: number;
  rest_um: number | null;
  convention: VelocityConvention;
  components: LineFitComponents;
  continuum_windows: ContinuumWindows;
  resolving_power: number | null;
  snr_threshold: number;
  n_fit: number;
  n_masked: number;
  n_const_continuum: number;
  n_two_components: number;
  median_chi2_red: number | null;
  notes: string[];
  dimensions: [number, number];
  elapsed_ms: number;
}

export interface LineFitComponentFit {
  amplitude: number;
  centre: number;
  sigma: number;
  amplitude_err: number | null;
  centre_err: number | null;
  sigma_err: number | null;
  velocity_kms: number;
  v_err_kms: number | null;
  sigma_kms: number;
  sigma_err_kms: number | null;
  sigma_corr_kms: number | null;
  flux: number;
  flux_err: number | null;
  snr: number | null;
}

export interface LineFitContinuum {
  intercept: number;
  slope: number;
  x_ref: number;
  sigma: number | null;
  linear: boolean;
  channels: number;
}

export interface LineFitSpaxelModel {
  channel: number[];
  continuum: number[];
  total: number[];
  components: number[][];
}

export interface LineFitSpaxel {
  x: number;
  y: number;
  z0: number;
  z1: number;
  continuum_windows: ContinuumWindows;
  span: [number, number];
  axis: number[];
  axis_unit: string;
  flux: (number | null)[];
  err: (number | null)[] | null;
  channels: number[];
  dropped_dq: number[];
  dropped_err: number[];
  continuum: LineFitContinuum | null;
  weighting: LineFitWeighting;
  single: LineFitComponentFit | null;
  chi2: number | null;
  dof: number;
  chi2_red: number | null;
  converged: boolean;
  iterations: number;
  components: LineFitComponentFit[];
  ncomp: 0 | 1 | 2;
  delta_bic: number | null;
  mask: number;
  model: LineFitSpaxelModel | null;
  notes: string[];
}
