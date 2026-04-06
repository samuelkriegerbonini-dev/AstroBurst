import { typedInvoke, withPreview } from "../infrastructure/tauri";
import { parseRawPixelBuffer } from "../infrastructure/tauri/parsers";
import type { ProcessResult, ResampleResult } from "../shared/types/fits.types";

export function processFits(path: string, outputDir?: string): Promise<ProcessResult> {
  return withPreview<ProcessResult>("process_fits", outputDir, { path });
}

export function processFitsFull(path: string, outputDir?: string): Promise<ProcessResult> {
  return withPreview<ProcessResult>("process_fits_full", outputDir, { path });
}

export interface RawPixelsResult {
  width: number;
  height: number;
  dataMin: number;
  dataMax: number;
  pixels: Float32Array;
}

export async function getRawPixelsPreview(path: string, maxDim = 2048): Promise<RawPixelsResult> {
  const buffer = await typedInvoke<ArrayBuffer>("get_raw_pixels_preview", { path, maxDim });
  return parseRawPixelBuffer(buffer);
}

export function resampleFits(
  path: string,
  targetWidth: number,
  targetHeight: number,
  outputDir?: string,
): Promise<ResampleResult> {
  return withPreview<ResampleResult>("resample_fits_cmd", outputDir, { path, targetWidth, targetHeight });
}
