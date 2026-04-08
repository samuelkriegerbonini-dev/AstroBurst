import { typedInvoke, withPreview } from "../infrastructure/tauri";
import { toUint8Array, parseFftBuffer } from "../infrastructure/tauri/parsers";
import type { HistogramData, FftData } from "../shared/types/analysis";
import type { StarDetectionResult } from "../shared/types/processing";
import type { ProcessResult } from "../shared/types/fits.types";

export function computeHistogram(path: string): Promise<HistogramData> {
  return typedInvoke<HistogramData>("compute_histogram", { path });
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
