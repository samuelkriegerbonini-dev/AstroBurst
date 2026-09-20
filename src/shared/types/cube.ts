import type { SpectralAxisInfo, VelocityConvention } from "./spectral";

export interface CubeDims {
  width: number;
  height: number;
  frames: number;
  frame_count: number;
  bitpix?: number;
  axis_labels?: string[];
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

export interface CubeProcessResult {
  collapsed_path: string;
  collapsed_median_path?: string;
  collapsedPreviewUrl?: string;
  collapsedMedianPreviewUrl?: string;
  frame_count: number;
  elapsed_ms: number;
}

export interface CubeSpectrum {
  wavelengths: number[];
  values: number[];
  x: number;
  y: number;
  unit?: string;
  is_spectral?: boolean;
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
