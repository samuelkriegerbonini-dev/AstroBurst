import { safeInvoke, withPreview } from "../infrastructure/tauri";
import { toUint8Array, parseFftBuffer } from "../infrastructure/tauri/parsers";

export function computeHistogram(path: string) {
  return safeInvoke("compute_histogram", { path });
}

export async function computeFftSpectrum(path: string) {
  const raw = await safeInvoke("compute_fft_spectrum", { path });
  return parseFftBuffer(toUint8Array(raw));
}

export function detectStars(path: string, sigma = 5.0, maxStars = 200) {
  return safeInvoke("detect_stars", { path, sigma, maxStars });
}

export function applyStfRender(
  path: string,
  outputDir: string | undefined,
  shadow: number,
  midtone: number,
  highlight: number,
) {
  return withPreview("apply_stf_render", outputDir, { path, shadow, midtone, highlight });
}
