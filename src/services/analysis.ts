import { typedInvoke, withPreview } from "../infrastructure/tauri";
import { toUint8Array, parseFftBuffer } from "../infrastructure/tauri/parsers";
import type { HistogramData, FftData } from "../shared/types/analysis";
import type { StarDetectionResult } from "../shared/types/processing";
import type { ProcessResult } from "../shared/types/fits.types";

export function computeHistogram(path: string, excludeDq = false): Promise<HistogramData> {
  return typedInvoke<HistogramData>("compute_histogram", { path, excludeDq });
}

export async function computeFftSpectrum(path: string): Promise<FftData> {
  const raw = await typedInvoke<ArrayBuffer>("compute_fft_spectrum", { path });
  return parseFftBuffer(toUint8Array(raw));
}

export function detectStars(path: string, sigma = 5.0, maxStars = 200): Promise<StarDetectionResult> {
  return typedInvoke<StarDetectionResult>("detect_stars", { path, sigma, maxStars });
}

export function detectStarsComposite(sigma = 5.0, maxStars = 200): Promise<StarDetectionResult> {
  return typedInvoke<StarDetectionResult>("detect_stars_composite", { sigma, maxStars });
}

export interface SubframeMetrics {
  file_path: string;
  file_name: string;
  star_count: number;
  median_fwhm: number;
  median_eccentricity: number;
  median_snr: number;
  background_median: number;
  background_sigma: number;
  noise_ratio: number;
  noise_sigma: number;
  noise_fraction: number;
  weight: number;
  accepted: boolean;
}

export interface SubframeAnalysisResult {
  subframes: SubframeMetrics[];
  total: number;
  accepted: number;
  rejected: number;
  elapsed_ms: number;
}

export interface SubframeOptions {
  maxFwhm?: number;
  maxEccentricity?: number;
  minSnr?: number;
  minStars?: number;
  fwhmWeight?: number;
  eccentricityWeight?: number;
  snrWeight?: number;
  noiseWeight?: number;
}

export function analyzeSubframes(
  paths: string[],
  options: SubframeOptions = {},
): Promise<SubframeAnalysisResult> {
  return typedInvoke<SubframeAnalysisResult>("analyze_subframes_cmd", {
    paths,
    maxFwhm: options.maxFwhm,
    maxEccentricity: options.maxEccentricity,
    minSnr: options.minSnr,
    minStars: options.minStars,
    fwhmWeight: options.fwhmWeight,
    eccentricityWeight: options.eccentricityWeight,
    snrWeight: options.snrWeight,
    noiseWeight: options.noiseWeight,
  });
}

export function applyStfRender(
  path: string,
  outputDir: string | undefined,
  shadow: number,
  midtone: number,
  highlight: number,
): Promise<ProcessResult> {
  return withPreview<ProcessResult>("apply_stf_render", outputDir, { path, shadow, midtone, highlight });
}

export interface StarPhotometry {
  x: number;
  y: number;
  peak: number;
  net_flux: number;
  flux_err: number;
  mag_inst: number;
  snr: number;
  fwhm: number;
  aperture_radius: number;
  aperture_pixels: number;
  aperture_area: number;
  bg_mean: number;
  bg_sigma: number;
  bg_pixels: number;
  saturated: boolean;
  n_masked: number;
  n_saturated: number;
  err_used: boolean;
  aperture_correction: number | null;
  flux_total: number | null;
  plateau_radius: number | null;
  flux_jy: number | null;
  flux_err_jy: number | null;
  mag_ab: number | null;
  mag_ab_err: number | null;
  mag_ab_total: number | null;
  st_mag: number | null;
}

export type FluxConvention =
  | { kind: "jwst_mjy_sr"; pixar_sr: number; pixar_a2: number | null; derived_pixar: boolean }
  | { kind: "jwst_dn_per_sec"; photmjsr: number; pixar_sr: number }
  | {
      kind: "hst_counts";
      photflam: number;
      photplam: number;
      photzpt: number;
      per_second: boolean;
      exptime: number | null;
    }
  | { kind: "roman_dn_per_sec"; mjy_per_dn_s: number; pixar_sr: number }
  | { kind: "generic_zero_point"; zp: number; keyword: string; per_second: boolean; exptime: number | null };

export interface PhotCal {
  convention: FluxConvention;
  bunit: string | null;
  notes: string[];
  warnings: string[];
  label: string;
}

export interface PhotometryMeasurement {
  photometry: StarPhotometry;
  sky?: { ra: number; dec: number } | null;
  gaia?: { gmag?: number | null; bp_rp: number; separation_arcsec: number } | null;
  photcal: PhotCal | null;
  warnings: string[];
  masked: boolean;
  elapsed_ms: number;
}

export interface PhotometryOptions {
  apertureRadius?: number;
  gaiaMatch?: boolean;
  excludeDq?: boolean;
  gain?: number;
}

export function measurePhotometry(
  path: string,
  x: number,
  y: number,
  options: PhotometryOptions = {},
): Promise<PhotometryMeasurement> {
  return typedInvoke<PhotometryMeasurement>("measure_photometry_cmd", {
    path,
    x,
    y,
    apertureRadius: options.apertureRadius ?? null,
    gaiaMatch: options.gaiaMatch ?? true,
    excludeDq: options.excludeDq ?? false,
    gain: options.gain ?? null,
  });
}
