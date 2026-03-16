import { safeInvoke, withPreview } from "../infrastructure/tauri";
import { parseRawPixelBuffer } from "../infrastructure/tauri/parsers";

export function processFits(path: string, outputDir?: string) {
  return withPreview("process_fits", outputDir, { path });
}

export function processFitsFull(path: string, outputDir?: string) {
  return withPreview("process_fits_full", outputDir, { path });
}

export async function getRawPixelsBinary(path: string) {
  const buffer = await safeInvoke("get_raw_pixels_binary", { path });
  return parseRawPixelBuffer(buffer);
}

export async function getRawPixelsPreview(path: string, maxDim = 2048) {
  const buffer = await safeInvoke("get_raw_pixels_preview", { path, maxDim });
  return parseRawPixelBuffer(buffer);
}

export function resampleFits(
  path: string,
  targetWidth: number,
  targetHeight: number,
  outputDir?: string,
) {
  return withPreview("resample_fits_cmd", outputDir, { path, targetWidth, targetHeight });
}
