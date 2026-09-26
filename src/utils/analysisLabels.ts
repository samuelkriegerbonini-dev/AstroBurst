export const FFT_FREQUENCY_UNIT = "cyc/px";
export const MEAN_LABEL = "μ";
export const SIGMA_MAD_LABEL = "σ(MAD)";
export const SIGMA_MAD_TITLE = "1.4826 × MAD (robust)";

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
