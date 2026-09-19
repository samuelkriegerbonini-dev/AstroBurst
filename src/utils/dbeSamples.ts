import type { DbeConfig } from "../shared/types/dbe";
import type { RegionDoc } from "./regionStore";

export const DBE_MIN_SAMPLE_RADIUS = 1;
export const DBE_MAX_SAMPLE_RADIUS = 64;
export const DBE_MIN_TOLERANCE = 0.1;
export const DBE_MAX_TOLERANCE = 10;
export const DBE_MIN_AUTO_GRID = 2;
export const DBE_MAX_AUTO_GRID = 40;
export const DBE_MIN_AUTO_CELL_PX = 4;
export const DBE_MIN_SAMPLES = 4;

export const DEFAULT_DBE_CONFIG: DbeConfig = {
  sampleRadius: 5,
  tolerance: 1.0,
  smoothing: 0.25,
  autoGrid: 10,
  manualSamples: [],
  rejectStars: true,
  mode: "subtract",
  normalize: true,
  maxSamples: 400,
};

export interface ImageSize {
  width: number;
  height: number;
}

export interface DbeValidation {
  ok: boolean;
  errors: string[];
}

export function pointSamplesFromDoc(doc: RegionDoc): [number, number][] {
  const out: [number, number][] = [];
  for (const region of doc.regions) {
    if (region.shape.shape !== "point" || region.props.include === false) continue;
    const { x, y } = region.shape;
    if (!Number.isFinite(x) || !Number.isFinite(y)) continue;
    out.push([x, y]);
  }
  return out;
}

function inRange(v: number, lo: number, hi: number): boolean {
  return Number.isFinite(v) && v >= lo && v <= hi;
}

export function validateDbeParams(config: DbeConfig, imageSize?: ImageSize | null): DbeValidation {
  const errors: string[] = [];

  if (!Number.isInteger(config.sampleRadius) || !inRange(config.sampleRadius, DBE_MIN_SAMPLE_RADIUS, DBE_MAX_SAMPLE_RADIUS)) {
    errors.push(`Sample radius must be an integer between ${DBE_MIN_SAMPLE_RADIUS} and ${DBE_MAX_SAMPLE_RADIUS}`);
  }
  if (!inRange(config.tolerance, DBE_MIN_TOLERANCE, DBE_MAX_TOLERANCE)) {
    errors.push(`Tolerance must be between ${DBE_MIN_TOLERANCE} and ${DBE_MAX_TOLERANCE} sigma`);
  }
  if (!inRange(config.smoothing, 0, 1)) {
    errors.push("Smoothing must be between 0 and 1");
  }
  if (!Number.isInteger(config.maxSamples) || config.maxSamples < DBE_MIN_SAMPLES) {
    errors.push(`Maximum samples must be an integer of at least ${DBE_MIN_SAMPLES}`);
  }

  if (config.autoGrid !== null) {
    if (!Number.isInteger(config.autoGrid) || !inRange(config.autoGrid, DBE_MIN_AUTO_GRID, DBE_MAX_AUTO_GRID)) {
      errors.push(`Automatic grid must be an integer between ${DBE_MIN_AUTO_GRID} and ${DBE_MAX_AUTO_GRID}`);
    } else if (imageSize) {
      const cell = Math.floor(Math.min(imageSize.width, imageSize.height) / config.autoGrid);
      if (cell < DBE_MIN_AUTO_CELL_PX) {
        errors.push(`Automatic grid of ${config.autoGrid} leaves cells under ${DBE_MIN_AUTO_CELL_PX} px on a ${imageSize.width}x${imageSize.height} image`);
      }
    }
  }

  config.manualSamples.forEach(([x, y], i) => {
    if (!Number.isFinite(x) || !Number.isFinite(y)) {
      errors.push(`Point sample ${i + 1} has non-finite coordinates`);
      return;
    }
    if (imageSize && (x < 0 || y < 0 || x > imageSize.width - 1 || y > imageSize.height - 1)) {
      errors.push(`Point sample ${i + 1} at (${x}, ${y}) is outside the image`);
    }
  });

  if (config.autoGrid === null && config.manualSamples.length < DBE_MIN_SAMPLES) {
    errors.push(`Without an automatic grid the spline needs at least ${DBE_MIN_SAMPLES} point regions`);
  }

  return { ok: errors.length === 0, errors };
}
