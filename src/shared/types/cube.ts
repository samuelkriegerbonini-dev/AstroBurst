import type { SpectralAxisInfo, VelocityConvention } from "./spectral";

export interface PhotometricPlane {
  filter: string;
  dataset: string;
  instrument: string;
  pivot_um: number | null;
  pixscale_arcsec: number | null;
  bunit_in: string;
  fluxconv: number | null;
  psf_fwhm_arcsec: number | null;
  kernel: string;
}

export interface CubeDims {
  width: number;
  height: number;
  frames: number;
  bitpix?: number;
  bunit?: string | null;
  wavelengths?: number[];
  spectral_classification?: {
    is_spectral: boolean;
    is_photometric?: boolean;
    reason: string | null;
    axis_type?: string | null;
    axis_unit?: string | null;
    channel_count?: number;
  };
  spectral_axis?: SpectralAxisInfo | null;
  photometric_planes?: PhotometricPlane[] | null;
}

export interface CubeSpectrum {
  wavelengths: number[];
  values: number[];
  x: number;
  y: number;
  is_spectral?: boolean;
  flux_jy?: number[] | null;
  err?: number[] | null;
}

export interface NoiseFactor {
  factor: number;
  factor_se: number;
  n_channels: number;
  windows: ContinuumWindows;
  warning: string | null;
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
  err: number[] | null;
  err_scaled: number[] | null;
  mean_err: number[] | null;
  used_weight: number[];
  err_source: string;
  bg_err_per_pixel: number[] | null;
  flux_jy_err: number[] | null;
  n_dq_excluded: number[] | null;
  noise_factor: NoiseFactor | null;
  notes: string[];
  elapsed_ms: number;
}

export type NullableSeries = (number | null)[];

type RegionSeriesKey = "sum" | "mean";

type RegionOptionalSeriesKey =
  | "bg_per_pixel"
  | "flux_jy"
  | "err"
  | "err_scaled"
  | "mean_err"
  | "bg_err_per_pixel"
  | "flux_jy_err";

export type RegionSpectrumWire = Omit<RegionSpectrum, RegionSeriesKey | RegionOptionalSeriesKey> & {
  [K in RegionSeriesKey]: NullableSeries;
} & {
  [K in RegionOptionalSeriesKey]: NullableSeries | null;
};

export type CollapseRangeMode = "sum" | "mean" | "median";
export type CollapseZeroPolicy = "include_physical_zeros" | "exclude_zero_padding";
export type ScienceDqPolicy = "do_not_use" | "ignore";

export interface ScienceDqMetadata {
  dq_policy?: ScienceDqPolicy;
  dq_applied?: boolean;
  dq_hdu?: number | null;
  dq_excluded_samples?: number;
}

export interface ScientificProvenance {
  source?: { input_ref: string; sha256?: string };
  integrity?: string;
  integrity_limit?: string;
}

export const COLLAPSE_RANGE_MODES: readonly CollapseRangeMode[] = ["sum", "mean", "median"];

export interface CollapseRangeResult extends ScienceDqMetadata {
  provenance_path?: string;
  provenance?: ScientificProvenance;
  png_path: string;
  fits_path: string;
  z0: number;
  z1: number;
  mode: CollapseRangeMode;
  zero_policy: CollapseZeroPolicy;
  dimensions: [number, number];
  has_err: boolean;
  err_source: string;
  notes: string[];
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
  dq_policy?: ScienceDqPolicy;
}

export interface MomentMapFiles {
  png_path: string;
  fits_path: string;
  previewUrl?: string;
}

export type MomentKind = "m0" | "m1" | "m2";

export const MOMENT_KINDS: readonly MomentKind[] = ["m0", "m1", "m2"];

export interface MomentMapsResult extends ScienceDqMetadata {
  provenance_path?: string;
  provenance?: ScientificProvenance;
  m0: MomentMapFiles;
  m1: MomentMapFiles;
  m2: MomentMapFiles;
  m0_unit: string;
  velocity_unit: string;
  noise_per_channel: number | null;
  n_channels: number;
  notes: string[];
  dimensions: [number, number];
  has_err: boolean;
  err_source: string;
  noise_factor: number | null;
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
  dq_policy?: ScienceDqPolicy;
  resolving_power: number | null;
  components: LineFitComponents;
  redshift?: number | null;
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
  | "ncomp"
  | "source_flux"
  | "source_velocity"
  | "source_sigma_obs"
  | "source_sigma_corr"
  | "source_flux_err"
  | "source_v_err"
  | "source_sigma_err";

export type LineFitComponentPlane =
  `c${1 | 2}_${"" | "source_"}${"flux" | "velocity" | "sigma_obs" | "flux_err" | "v_err" | "sigma_err"}`;

export type LineFitPlane = LineFitBasePlane | LineFitComponentPlane;

export interface LineFitUnits {
  flux: string;
  velocity: string;
  sigma: string;
}

export interface LineFitResult extends ScienceDqMetadata {
  provenance_path?: string;
  provenance?: ScientificProvenance;
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
  source_velocity_kms?: number | null;
  source_v_err_kms?: number | null;
  source_sigma_kms?: number | null;
  source_sigma_err_kms?: number | null;
  source_sigma_corr_kms?: number | null;
  source_flux?: number | null;
  source_flux_err?: number | null;
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

export interface LineFitSpaxel extends ScienceDqMetadata {
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
