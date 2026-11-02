import type { ScienceDqMetadata } from "../shared/types/cube";

export const FFT_FREQUENCY_UNIT = "cyc/px";
export const MEAN_LABEL = "μ";
export const SIGMA_MAD_LABEL = "σ(MAD)";
export const SIGMA_MAD_TITLE = "1.4826 × MAD (robust)";

export function scienceDqSummary(result: ScienceDqMetadata): string | null {
  if (!result.dq_policy) return null;
  if (result.dq_policy === "ignore") return "DQ ignored by choice.";
  if (result.dq_applied === false) return "DO_NOT_USE masking was requested but not applied; see notes.";
  if (result.dq_applied !== true) return "DO_NOT_USE masking was requested; application was not reported.";
  const count = result.dq_excluded_samples === undefined ? "excluded sample count unavailable" : `${result.dq_excluded_samples} samples excluded`;
  return `DQ: DO_NOT_USE (bit 0), HDU ${result.dq_hdu ?? "unknown"}, ${count}.`;
}

export function skyRangeLabel(range: { lo: number; hi: number }): string {
  return `sky ${range.lo.toPrecision(4)}–${range.hi.toPrecision(4)}`;
}

export function starCountLabel(shown: number, detected: number | null | undefined): string {
  if (detected == null || !Number.isFinite(detected) || detected <= shown) return String(shown);
  return `${shown} of ${detected} (brightest)`;
}

export interface FftSize {
  width: number;
  height: number;
  imageWidth: number;
  imageHeight: number;
  downsampled: boolean;
}

export function fftSizeLabel({ width, height, imageWidth, imageHeight, downsampled }: FftSize): string {
  const shown = `${width}×${height}`;
  return downsampled ? `${shown} (from ${imageWidth}×${imageHeight})` : shown;
}

export function fftGridTitle({ gridWidth, gridHeight }: { gridWidth: number; gridHeight: number }): string {
  return `FFT grid ${gridWidth}×${gridHeight}: the image zero-padded to the next power of two on each axis`;
}

export function fftFrequencyReadout(fx: string, fy: string): string {
  return `freq (${fx}, ${fy}) ${FFT_FREQUENCY_UNIT}`;
}
