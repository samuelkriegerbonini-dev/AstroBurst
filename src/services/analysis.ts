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

export function applyStfRender(
  path: string,
  outputDir: string | undefined,
  shadow: number,
  midtone: number,
  highlight: number,
): Promise<ProcessResult> {
  return withPreview<ProcessResult>("apply_stf_render", outputDir, { path, shadow, midtone, highlight });
}
