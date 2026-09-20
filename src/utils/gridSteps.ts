import type { SkyFrame } from "../shared/types/astrometry";
import { GRID_DENSITY_DEFAULT, GRID_DENSITY_MAX, GRID_DENSITY_MIN } from "../shared/types/display";
import { frameLonInHours } from "./coordFormat";

export type StepTable = "hours" | "sexagesimal" | "decimal";

export const HOUR_STEPS_SECONDS: readonly number[] = [
  21600, 10800, 7200, 3600, 1800, 1200, 900, 600, 300, 120, 60, 30, 20, 10, 5, 2, 1,
];

export const SEXAGESIMAL_STEPS_ARCSEC: readonly number[] = [
  162000, 108000, 72000, 54000, 36000, 18000, 7200, 3600, 1800, 1200, 900, 600, 300, 120, 60, 30, 20, 15, 10, 5, 2, 1,
];

export const DECIMAL_STEPS_DEG: readonly number[] = [
  45, 30, 20, 15, 10, 5, 2, 1, 0.5, 0.2, 0.1, 0.05, 0.02, 0.01, 0.005, 0.002, 0.001, 0.0005, 0.0002, 0.0001,
];

function remEuclid(value: number, modulus: number): number {
  const r = value % modulus;
  return r < 0 ? r + modulus : r;
}

function pad2(n: number): string {
  return n < 10 ? `0${n}` : String(n);
}

export function clampGridDensity(density: number): number {
  if (!Number.isFinite(density)) return GRID_DENSITY_DEFAULT;
  return Math.min(GRID_DENSITY_MAX, Math.max(GRID_DENSITY_MIN, Math.round(density)));
}

export function targetLines(density: number): number {
  return 3 + 2 * clampGridDensity(density);
}

export function lonStepTable(frame: SkyFrame): StepTable {
  return frameLonInHours(frame) ? "hours" : "decimal";
}

export function stepValuesDeg(table: StepTable): number[] {
  switch (table) {
    case "hours":
      return HOUR_STEPS_SECONDS.map((s) => s / 240);
    case "sexagesimal":
      return SEXAGESIMAL_STEPS_ARCSEC.map((s) => s / 3600);
    default:
      return [...DECIMAL_STEPS_DEG];
  }
}

export function chooseStep(table: StepTable, extentDeg: number, target: number): number {
  const steps = stepValuesDeg(table);
  const limit = target + 1e-9;
  let chosen = steps[0];
  for (const step of steps) {
    if (Number.isFinite(extentDeg) && extentDeg / step <= limit) {
      chosen = step;
    } else if (Number.isFinite(extentDeg)) {
      break;
    }
  }
  return chosen;
}

export function decimalPlaces(stepDeg: number): number {
  if (!Number.isFinite(stepDeg) || stepDeg <= 0) return 0;
  for (let places = 0; places <= 6; places++) {
    const scaled = stepDeg * Math.pow(10, places);
    if (Math.abs(scaled - Math.round(scaled)) < 1e-6) return places;
  }
  return 6;
}

export function formatLonHours(deg: number, stepDeg: number): string {
  const stepSeconds = Math.round(stepDeg * 240);
  const total = Math.round(remEuclid(deg, 360) * 240) % 86400;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return stepSeconds < 60 ? `${pad2(h)}h${pad2(m)}m${pad2(s)}s` : `${pad2(h)}h${pad2(m)}m`;
}

export function formatLonDecimal(deg: number, stepDeg: number): string {
  const decimals = decimalPlaces(stepDeg);
  let wrapped = remEuclid(deg, 360);
  const scale = Math.pow(10, decimals);
  if (Math.round(wrapped * scale) / scale >= 360) wrapped = 0;
  return `${wrapped.toFixed(decimals)}°`;
}

export function formatLonLabel(frame: SkyFrame, deg: number, stepDeg: number): string {
  return frameLonInHours(frame) ? formatLonHours(deg, stepDeg) : formatLonDecimal(deg, stepDeg);
}

export function formatLatLabel(deg: number, stepDeg: number): string {
  const stepArcsec = Math.round(stepDeg * 3600);
  const total = Math.round(Math.abs(deg) * 3600);
  const sign = deg < 0 && total > 0 ? "-" : "+";
  const d = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return stepArcsec < 60 ? `${sign}${pad2(d)}°${pad2(m)}'${pad2(s)}"` : `${sign}${pad2(d)}°${pad2(m)}'`;
}
