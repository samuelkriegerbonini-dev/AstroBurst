import { GRID_DENSITY_DEFAULT, GRID_DENSITY_MAX, GRID_DENSITY_MIN } from "../shared/types/display";

export function clampGridDensity(density: number): number {
  if (!Number.isFinite(density)) return GRID_DENSITY_DEFAULT;
  return Math.min(GRID_DENSITY_MAX, Math.max(GRID_DENSITY_MIN, Math.round(density)));
}
