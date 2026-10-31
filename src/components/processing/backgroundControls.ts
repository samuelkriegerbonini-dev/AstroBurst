export type BackgroundOperation = "gradient" | "deband";
export type BackgroundModel = "polynomial" | "spline";

export interface ControlRange {
  min: number;
  max: number;
  step: number;
}

export interface BackgroundControls {
  gridSize?: ControlRange;
  polyDegree?: ControlRange;
  sigmaClip?: ControlRange;
  iterations?: ControlRange;
  axis: boolean;
  applyAs: boolean;
  dbe: boolean;
}

const GRID_SIZE: ControlRange = { min: 3, max: 24, step: 1 };
const POLY_DEGREE: ControlRange = { min: 1, max: 5, step: 1 };
const SIGMA_CLIP: ControlRange = { min: 1, max: 5, step: 0.1 };
const GRADIENT_ITERATIONS: ControlRange = { min: 1, max: 10, step: 1 };
const DEBAND_ITERATIONS: ControlRange = { min: 1, max: 5, step: 1 };

export function backgroundControlsFor(operation: BackgroundOperation, model: BackgroundModel = "polynomial"): BackgroundControls {
  if (operation === "deband") return { sigmaClip: SIGMA_CLIP, iterations: DEBAND_ITERATIONS, axis: true, applyAs: false, dbe: false };
  if (model === "spline") return { axis: false, applyAs: true, dbe: true };
  return { gridSize: GRID_SIZE, polyDegree: POLY_DEGREE, sigmaClip: SIGMA_CLIP, iterations: GRADIENT_ITERATIONS, axis: false, applyAs: true, dbe: false };
}

export function withinRange(value: number, range: ControlRange): number {
  return Math.min(range.max, Math.max(range.min, value));
}
