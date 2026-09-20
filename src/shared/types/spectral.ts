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
}

export interface VelocityAxisResult {
  values_kms: number[];
  convention: VelocityConvention;
  rest_um: number;
  notes: string[];
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
