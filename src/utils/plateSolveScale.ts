export const DEFAULT_SCALE_LOW_TEXT = "0.1";
export const DEFAULT_SCALE_HIGH_TEXT = "10";

const HINT_LOW_FACTOR = 0.8;
const HINT_HIGH_FACTOR = 1.25;
const HINT_SIGNIFICANT_DIGITS = 3;
const ROUNDING_SLACK = 1e-9;

export type ScaleRange = { low: number; high: number; error: null } | { error: string };

function roundSignificant(value: number, direction: "down" | "up"): string {
  const step = 10 ** (Math.floor(Math.log10(value)) - (HINT_SIGNIFICANT_DIGITS - 1));
  const units = value / step;
  const rounded = direction === "down" ? Math.floor(units + ROUNDING_SLACK) : Math.ceil(units - ROUNDING_SLACK);
  return String(Number((rounded * step).toPrecision(HINT_SIGNIFICANT_DIGITS)));
}

export function scaleHintFromPixelScale(pixelScaleArcsec: number | null | undefined): { low: string; high: string } | null {
  if (pixelScaleArcsec == null || !Number.isFinite(pixelScaleArcsec) || pixelScaleArcsec <= 0) return null;
  return {
    low: roundSignificant(pixelScaleArcsec * HINT_LOW_FACTOR, "down"),
    high: roundSignificant(pixelScaleArcsec * HINT_HIGH_FACTOR, "up"),
  };
}

function positiveNumber(text: string): number | null {
  if (text.trim() === "") return null;
  const value = Number(text);
  return Number.isFinite(value) && value > 0 ? value : null;
}

export function parseScaleRange(lowText: string, highText: string): ScaleRange {
  const low = positiveNumber(lowText);
  if (low === null) return { error: "Scale low must be a number above 0." };
  const high = positiveNumber(highText);
  if (high === null) return { error: "Scale high must be a number above 0." };
  if (low >= high) return { error: "Scale low must be below scale high." };
  return { low, high, error: null };
}
