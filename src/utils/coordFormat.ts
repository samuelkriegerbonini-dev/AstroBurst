import type { SkyFrame } from "../shared/types/astrometry";
import { isFiniteNumber } from "./values";

export type CoordFormat = "sexagesimal" | "decimal";

const NON_FINITE = "—";

function pad2(n: number): string {
  return n < 10 ? `0${n}` : String(n);
}

function padSeconds(s: number, precision: number): string {
  const text = s.toFixed(precision);
  return s < 10 ? `0${text}` : text;
}

interface Sexagesimal {
  whole: number;
  minutes: number;
  seconds: number;
}

function splitSexagesimal(units: number, precision: number, wrap: number | null): Sexagesimal {
  const scale = Math.pow(10, precision);
  let total = Math.round(units * 3600 * scale) / scale;
  if (wrap !== null) {
    const limit = wrap * 3600;
    total = ((total % limit) + limit) % limit;
  }
  const whole = Math.floor(total / 3600);
  const rest = total - whole * 3600;
  const minutes = Math.floor(rest / 60);
  const seconds = Math.max(0, rest - minutes * 60);
  return { whole, minutes, seconds };
}

function wrap360(deg: number): number {
  const r = deg % 360;
  return r < 0 ? r + 360 : r;
}

export function formatLon(deg: number, opts: { hours: boolean; format: CoordFormat; precision?: number }): string {
  if (!Number.isFinite(deg)) return NON_FINITE;
  const lon = wrap360(deg);
  if (opts.format === "decimal") {
    const precision = opts.precision ?? 6;
    let text = lon.toFixed(precision);
    if (Number(text) >= 360) text = (0).toFixed(precision);
    return `${text}°`;
  }
  if (opts.hours) {
    const precision = opts.precision ?? 2;
    const { whole, minutes, seconds } = splitSexagesimal(lon / 15, precision, 24);
    return `${pad2(whole)}h${pad2(minutes)}m${padSeconds(seconds, precision)}s`;
  }
  const precision = opts.precision ?? 1;
  const { whole, minutes, seconds } = splitSexagesimal(lon, precision, 360);
  return `${whole}°${pad2(minutes)}'${padSeconds(seconds, precision)}"`;
}

export function formatLat(deg: number, opts: { format: CoordFormat; precision?: number }): string {
  if (!Number.isFinite(deg)) return NON_FINITE;
  const sign = deg < 0 || Object.is(deg, -0) ? "-" : "+";
  const abs = Math.abs(deg);
  if (opts.format === "decimal") {
    const precision = opts.precision ?? 6;
    return `${sign}${abs.toFixed(precision)}°`;
  }
  const precision = opts.precision ?? 1;
  const { whole, minutes, seconds } = splitSexagesimal(abs, precision, null);
  return `${sign}${pad2(whole)}°${pad2(minutes)}'${padSeconds(seconds, precision)}"`;
}

export interface CentreText {
  ra: string;
  dec: string;
  raDeg: string;
  decDeg: string;
}

export interface SolveCentre {
  center_ra: number;
  center_dec: number;
  pixel_scale_arcsec?: number | null;
  field_of_view_w_arcmin?: number | null;
  field_of_view_h_arcmin?: number | null;
}

const DEGREE_SIGN = "°";
const DECIMAL_UNIT_AND_FRAME = "deg ICRS";
const SCALE_SIGNIFICANT_DIGITS = 4;

export function solveCentreText(raDeg: number, decDeg: number): CentreText {
  return {
    ra: formatLon(raDeg, { hours: true, format: "sexagesimal", precision: 2 }),
    dec: formatLat(decDeg, { format: "sexagesimal", precision: 1 }),
    raDeg: formatLon(raDeg, { hours: false, format: "decimal", precision: 6 }),
    decDeg: formatLat(decDeg, { format: "decimal", precision: 6 }),
  };
}

export function pixelScaleText(arcsecPerPx: number | null | undefined): string | null {
  if (!isFiniteNumber(arcsecPerPx) || arcsecPerPx <= 0) return null;
  const decimals = Math.max(0, SCALE_SIGNIFICANT_DIGITS - 1 - Math.floor(Math.log10(arcsecPerPx)));
  return `${arcsecPerPx.toFixed(decimals)}"/px`;
}

export function fieldOfViewText(widthArcmin: number | null | undefined, heightArcmin: number | null | undefined): string | null {
  if (!isFiniteNumber(widthArcmin) || !isFiniteNumber(heightArcmin)) return null;
  if (Math.max(widthArcmin, heightArcmin) < 1) return `${Math.round(widthArcmin * 60)}" x ${Math.round(heightArcmin * 60)}"`;
  return `${widthArcmin.toFixed(1)}' x ${heightArcmin.toFixed(1)}'`;
}

export function skyPositionLine(raDeg: number, decDeg: number): string {
  const text = solveCentreText(raDeg, decDeg);
  return `${text.raDeg.replace(DEGREE_SIGN, "")} ${text.decDeg.replace(DEGREE_SIGN, "")} ${DECIMAL_UNIT_AND_FRAME} | ${text.ra} ${text.dec}`;
}

export function solveSummaryLine(centre: SolveCentre): string {
  const fov = fieldOfViewText(centre.field_of_view_w_arcmin, centre.field_of_view_h_arcmin);
  return [skyPositionLine(centre.center_ra, centre.center_dec), pixelScaleText(centre.pixel_scale_arcsec), fov && `FOV ${fov}`]
    .filter((part): part is string => Boolean(part))
    .join(" | ");
}

export function frameAxisLabels(frame: SkyFrame): [string, string] {
  switch (frame) {
    case "galactic":
      return ["l", "b"];
    case "ecliptic":
      return ["λ", "β"];
    default:
      return ["RA", "Dec"];
  }
}

export function frameLonInHours(frame: SkyFrame): boolean {
  return frame === "icrs" || frame === "fk5";
}
