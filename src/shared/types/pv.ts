import type { CorrectionFrame, SpectralAxisMode, VelocityConvention } from "./spectral";

export interface PvLine {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export type PvOffsetUnit = "arcsec" | "pixel";

export type PvSpectralUnit = "um" | "GHz" | "km/s" | "ch";

export interface PvSummary {
  n_offsets: number;
  n_channels: number;
  n_across: number;
  n_off_image: number;
  offset_step: number;
  offset_unit: PvOffsetUnit;
  slit_length_px: number;
  slit_length: number;
  slit_pa_deg: number | null;
  pixel_scale_arcsec: number | null;
  ridge_gradient: number | null;
  ridge_gradient_unit: string;
  ridge_span: number | null;
  n_ridge_valid: number;
  peak_value: number | null;
  peak_offset: number | null;
  peak_spectral: number | null;
  bunit: string | null;
}

export interface PvDiagramResult {
  fits_path: string;
  png_path: string;
  dimensions: [number, number];
  offsets: number[];
  offset_unit: PvOffsetUnit;
  offset_step: number;
  spectral_values: number[];
  spectral_unit: PvSpectralUnit;
  spectral_mode: SpectralAxisMode;
  spectral_shift_kms: number;
  spectral_rest_um: number | null;
  spectral_convention: VelocityConvention | null;
  ridge: (number | null)[];
  ridge_channel: (number | null)[];
  peak_channel: (number | null)[];
  peak_value: (number | null)[];
  peak_spectral: (number | null)[];
  xs: number[];
  ys: number[];
  line: PvLine;
  step_px: number;
  width_px: number;
  z0: number;
  z1: number;
  n_across: number;
  summary: PvSummary;
  notes: string[];
  elapsed_ms: number;
  previewUrl?: string;
}

export interface PvRunParams {
  filePath: string;
  regionId: string;
  line: PvLine;
  stepPx: number;
  widthPx: number;
  z0: number;
  z1: number;
  mode: SpectralAxisMode;
  restUm: number | null;
  convention: VelocityConvention;
  correction: CorrectionFrame;
  velocityShiftKms: number | null;
}

export interface PvRun {
  params: PvRunParams;
  result: PvDiagramResult;
}
