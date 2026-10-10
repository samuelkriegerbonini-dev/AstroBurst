import type { ContinuumWindows } from "./cube";
import type { RegionShape } from "./regions";

export type SpectralAxisKind = "wave" | "awav" | "freq" | "vrad" | "vopt" | "velo" | "zopt" | "unknown";

export type VelocityConvention = "optical" | "radio" | "relativistic";

export type SpectralAxisMode = "wavelength_vac" | "wavelength_air" | "frequency" | "velocity";

export type CorrectionFrame = "none" | "heliocentric" | "barycentric";

export interface SpectralAxisInfo {
  kind: SpectralAxisKind;
  ctype: string;
  unit: string;
  header_unit: string;
  header_scale: number;
  values: number[];
  crval: number;
  cdelt: number;
  crpix: number;
  rest_wavelength_um: number | null;
  rest_frequency_hz: number | null;
  specsys: string | null;
  velosys: number | null;
  notes: string[];
  tabulated?: boolean;
}

export interface RadialVelocityCorrectionResult {
  barycentric_kms: number;
  heliocentric_kms: number;
  jd_mid: number;
  method: string;
  accuracy_kms: number;
  notes: string[];
  ra_deg: number;
  dec_deg: number;
  coordinate_source: string;
}

export interface RadialVelocityCorrectionError {
  error: string;
}

export type RadialVelocityCorrectionResponse = RadialVelocityCorrectionResult | RadialVelocityCorrectionError;

export type LineModel = "none" | "gaussian";

export const LINE_MODELS: readonly LineModel[] = ["none", "gaussian"];

export type SpectrumSource =
  | { kind: "pixel"; x: number; y: number }
  | { kind: "region"; shape: RegionShape; background: RegionShape | null };

export interface TableSource {
  kind: "table";
  path: string;
  hdu: number;
}

export interface X1dTableRef {
  hdu: number;
  extver: number | null;
  n_rows: number;
}

export interface X1dSpectrum {
  path: string;
  hdu: number;
  extver: number | null;
  n_rows: number;
  wavelength_um: number[];
  wavelength_unit: string;
  flux: number[];
  flux_error: number[] | null;
  flux_unit: string;
  surf_bright: number[] | null;
  surf_bright_unit: string | null;
  background: number[] | null;
  npixels: number[] | null;
  dq: number[] | null;
  dq_table: "jwst" | "roman" | "hst" | "unknown";
  dq_flagged_rows: number;
  srctype: string | null;
  grating: string | null;
  filter: string | null;
  detector: string | null;
  instrument: string | null;
  target: string | null;
  other_tables: X1dTableRef[];
  notes: string[];
}

export interface LineVelocity {
  centroid_kms: number | null;
  sigma_kms: number | null;
  fwhm_kms: number | null;
  rest_um: number;
  convention: VelocityConvention;
  shift_applied_kms: number;
  axis_frame: string | null;
}

export interface GaussianFitResult {
  amplitude: number | null;
  centre: number | null;
  sigma: number | null;
  amplitude_err: number | null;
  centre_err: number | null;
  sigma_err: number | null;
  chi2: number | null;
  dof: number;
  iterations: number;
  converged: boolean;
}

export interface LineMeasurement {
  z0: number;
  z1: number;
  n_channels: number;
  axis_unit: string;
  flux_unit: string;
  continuum_level: number | null;
  continuum_slope: number | null;
  continuum_sigma: number | null;
  continuum_reference: number | null;
  continuum_linear: boolean;
  continuum_channels: number;
  continuum_windows: ContinuumWindows;
  flux: number | null;
  flux_err: number | null;
  equivalent_width: number | null;
  centroid: number | null;
  sigma: number | null;
  fwhm: number | null;
  peak: number | null;
  peak_channel: number;
  snr: number | null;
  velocity: LineVelocity | null;
  fit: GaussianFitResult | null;
  notes: string[];
  elapsed_ms: number;
  source: SpectrumSource;
}
