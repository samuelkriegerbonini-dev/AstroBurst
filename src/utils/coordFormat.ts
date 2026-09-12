import type { SkyFrame } from "../shared/types/astrometry";

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
