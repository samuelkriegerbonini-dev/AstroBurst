import type { GeometryOverrides, ObservationGeometryResult } from "../shared/types/geometry";

export const SITE_OVERRIDE_STORAGE_KEY = "astroburst.geometry.site.v1";
export const NO_TIME_HINT =
  "No observation time in the header (MJD-AVG, EXPMID, DATE-AVG, DATE-OBS, MJD-OBS or BJDREF with TSTART/TSTOP): no time scales or geometry can be computed.";
export const NO_SITE_HINT = "No observatory location in the header: enter the site to get altitude, azimuth, airmass and the Moon altitude.";
export const NO_TARGET_HINT = "No target coordinates (WCS or RA/DEC keywords): enter RA and Dec in degrees to get BJD_TDB and HJD_UTC.";

export const SITE_NEEDS_BOTH = "site override needs both latitude and longitude";
export const TARGET_NEEDS_BOTH = "target override needs both RA and Dec";

const LAT_LIMIT_DEG = 90;
const LON_MIN_DEG = -180;
const LON_MAX_DEG = 360;
const HEIGHT_MIN_M = -500;
const HEIGHT_MAX_M = 10000;
const RA_MIN_DEG = 0;
const RA_MAX_DEG = 360;
const DEC_LIMIT_DEG = 90;
const JD_DIGITS = 6;
const ANGLE_DIGITS = 4;
const AIRMASS_DIGITS = 4;
const PERCENT_DIGITS = 1;
const HOUR_ANGLE_DIGITS = 4;
const TENTHS_PER_HOUR = 36000;
const TENTHS_PER_MINUTE = 600;
const HOURS_PER_TURN = 24;
const DEGREES_PER_HOUR = 15;
const NULL_VALUE = "--";

export interface SiteOverride {
  lat: number;
  lon: number;
  height: number;
}

export interface TargetOverride {
  ra: number;
  dec: number;
}

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export interface GeometryRow {
  key: string;
  label: string;
  value: string;
  unit: string;
  hint?: string;
}

function isFiniteNumber(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

function latitudeValid(lat: number): boolean {
  return Math.abs(lat) <= LAT_LIMIT_DEG;
}

function longitudeValid(lon: number): boolean {
  return lon >= LON_MIN_DEG && lon <= LON_MAX_DEG;
}

function heightValid(height: number): boolean {
  return height >= HEIGHT_MIN_M && height <= HEIGHT_MAX_M;
}

export function loadSiteOverride(storage: StorageLike | null): SiteOverride | null {
  if (!storage) return null;
  try {
    const text = storage.getItem(SITE_OVERRIDE_STORAGE_KEY);
    if (!text) return null;
    const raw = JSON.parse(text) as Partial<Record<keyof SiteOverride, unknown>>;
    const lat = raw.lat;
    const lon = raw.lon;
    const height = raw.height === undefined ? 0 : raw.height;
    if (!isFiniteNumber(lat) || !isFiniteNumber(lon) || !isFiniteNumber(height)) return null;
    if (!latitudeValid(lat) || !longitudeValid(lon) || !heightValid(height)) return null;
    return { lat, lon, height };
  } catch {
    return null;
  }
}

export function saveSiteOverride(storage: StorageLike | null, site: SiteOverride | null): void {
  if (!storage) return;
  try {
    if (site) storage.setItem(SITE_OVERRIDE_STORAGE_KEY, JSON.stringify(site));
    else storage.removeItem(SITE_OVERRIDE_STORAGE_KEY);
  } catch {
  }
}

function parseDecimal(text: string): number | null {
  const trimmed = text.trim();
  if (!/^[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?$/.test(trimmed)) return null;
  const v = Number(trimmed);
  return Number.isFinite(v) ? v : null;
}

export function parseSiteInputs(lat: string, lon: string, height: string): { site: SiteOverride | null; error: string | null } {
  const latBlank = lat.trim() === "";
  const lonBlank = lon.trim() === "";
  const heightBlank = height.trim() === "";
  if (latBlank && lonBlank && heightBlank) return { site: null, error: null };
  if (latBlank || lonBlank) return { site: null, error: SITE_NEEDS_BOTH };
  const latValue = parseDecimal(lat);
  if (latValue === null || !latitudeValid(latValue)) {
    return { site: null, error: `latitude ${lat.trim()} must be a decimal between -${LAT_LIMIT_DEG} and ${LAT_LIMIT_DEG} degrees` };
  }
  const lonValue = parseDecimal(lon);
  if (lonValue === null || !longitudeValid(lonValue)) {
    return { site: null, error: `longitude ${lon.trim()} must be a decimal between ${LON_MIN_DEG} and ${LON_MAX_DEG} degrees (east positive)` };
  }
  const heightValue = heightBlank ? 0 : parseDecimal(height);
  if (heightValue === null || !heightValid(heightValue)) {
    return { site: null, error: `height ${height.trim()} must be a decimal between ${HEIGHT_MIN_M} and ${HEIGHT_MAX_M} m` };
  }
  return { site: { lat: latValue, lon: lonValue, height: heightValue }, error: null };
}

export function parseTargetInputs(ra: string, dec: string): { target: TargetOverride | null; error: string | null } {
  const raBlank = ra.trim() === "";
  const decBlank = dec.trim() === "";
  if (raBlank && decBlank) return { target: null, error: null };
  if (raBlank || decBlank) return { target: null, error: TARGET_NEEDS_BOTH };
  const raValue = parseDecimal(ra);
  if (raValue === null || raValue < RA_MIN_DEG || raValue >= RA_MAX_DEG) {
    return { target: null, error: `RA ${ra.trim()} must be a decimal of at least ${RA_MIN_DEG} and below ${RA_MAX_DEG} degrees` };
  }
  const decValue = parseDecimal(dec);
  if (decValue === null || Math.abs(decValue) > DEC_LIMIT_DEG) {
    return { target: null, error: `Dec ${dec.trim()} must be a decimal between -${DEC_LIMIT_DEG} and ${DEC_LIMIT_DEG} degrees` };
  }
  return { target: { ra: raValue, dec: decValue }, error: null };
}

export function overridesFor(target: TargetOverride | null, site: SiteOverride | null): GeometryOverrides {
  return {
    targetRa: target ? target.ra : null,
    targetDec: target ? target.dec : null,
    siteLat: site ? site.lat : null,
    siteLon: site ? site.lon : null,
    siteHeight: site ? site.height : null,
  };
}

export function formatHours(deg: number): string {
  const hours = ((deg / DEGREES_PER_HOUR) % HOURS_PER_TURN + HOURS_PER_TURN) % HOURS_PER_TURN;
  const tenths = Math.round(hours * TENTHS_PER_HOUR) % (HOURS_PER_TURN * TENTHS_PER_HOUR);
  const h = Math.floor(tenths / TENTHS_PER_HOUR);
  const m = Math.floor((tenths % TENTHS_PER_HOUR) / TENTHS_PER_MINUTE);
  const s = (tenths % TENTHS_PER_MINUTE) / 10;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${s.toFixed(1).padStart(4, "0")}`;
}

function fixed(v: number | null | undefined, digits: number): string {
  return isFiniteNumber(v) ? v.toFixed(digits) : NULL_VALUE;
}

function row(key: string, label: string, value: string, unit: string, hint?: string): GeometryRow {
  return hint === undefined ? { key, label, value, unit } : { key, label, value, unit, hint };
}

export function geometryRows(result: ObservationGeometryResult): GeometryRow[] {
  const g = result.geometry;
  const lst = isFiniteNumber(g.lst_deg) ? `${g.lst_deg.toFixed(ANGLE_DIGITS)} deg (${formatHours(g.lst_deg)} h)` : NULL_VALUE;
  const hourAngle = isFiniteNumber(g.hour_angle_deg) ? (g.hour_angle_deg / DEGREES_PER_HOUR).toFixed(HOUR_ANGLE_DIGITS) : NULL_VALUE;
  const illumination = isFiniteNumber(g.moon_illumination) ? (g.moon_illumination * 100).toFixed(PERCENT_DIGITS) : NULL_VALUE;
  return [
    row("jd_utc", "JD_UTC", fixed(g.jd_utc, JD_DIGITS), "d"),
    row("jd_tt", "JD_TT", fixed(g.jd_tt, JD_DIGITS), "d"),
    row("jd_tdb", "JD_TDB", fixed(g.jd_tdb, JD_DIGITS), "d"),
    row("bjd_tdb", "BJD_TDB", fixed(g.bjd_tdb, JD_DIGITS), "d", isFiniteNumber(g.bjd_tdb) && g.bjd_source ? g.bjd_source : undefined),
    row("hjd_utc", "HJD_UTC", fixed(g.hjd_utc, JD_DIGITS), "d"),
    row("lst", "Local sidereal time", lst, ""),
    row("hour_angle", "Hour angle", hourAngle, "h"),
    row("altitude", "Altitude", fixed(g.altitude_deg, ANGLE_DIGITS), "deg"),
    row("azimuth", "Azimuth", fixed(g.azimuth_deg, ANGLE_DIGITS), "deg"),
    row("airmass_computed", "Airmass computed", fixed(g.airmass_computed, AIRMASS_DIGITS), "", g.airmass_formula),
    row("airmass_header", "Airmass header", fixed(result.airmass_header, AIRMASS_DIGITS), "", "header AIRMASS"),
    row("parallactic_angle", "Parallactic angle", fixed(g.parallactic_angle_deg, ANGLE_DIGITS), "deg"),
    row("sun_altitude", "Sun altitude", fixed(g.sun_altitude_deg, ANGLE_DIGITS), "deg"),
    row("moon_altitude", "Moon altitude", fixed(g.moon_altitude_deg, ANGLE_DIGITS), "deg"),
    row("moon_illumination", "Moon illumination", illumination, "%"),
    row("moon_separation", "Moon separation", fixed(g.moon_separation_deg, ANGLE_DIGITS), "deg"),
  ];
}

export function geometryText(result: ObservationGeometryResult): string {
  const lines = geometryRows(result).map((r) => `${r.label}: ${r.value}${r.unit ? ` ${r.unit}` : ""}`);
  if (result.target) lines.push(`Target: RA ${result.target.ra_deg} Dec ${result.target.dec_deg} deg from ${result.target.source}`);
  if (result.site) {
    lines.push(`Site: lat ${result.site.lat_deg} lon ${result.site.lon_deg} deg height ${result.site.height_m} m from ${result.site.source}`);
  }
  if (result.time_source) lines.push(`Time source: ${result.time_source}`);
  for (const note of result.notes) lines.push(`Note: ${note}`);
  for (const note of result.geometry.time_scale_notes) lines.push(`Note: ${note}`);
  return lines.join("\n") + "\n";
}
