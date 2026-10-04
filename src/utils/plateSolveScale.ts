import type { WcsInfo } from "../shared/types/astrometry";
import { parseDecimalText } from "./decimalText";
import { normalizeAngle } from "./regionGeometry";

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

const MAX_CENTRE_RA_DEG = 360;
const MAX_CENTRE_DEC_DEG = 90;
const MAX_SEARCH_RADIUS_DEG = 180;
const CENTRE_RA_ERROR = "Centre RA must be decimal degrees from 0 to 360, written with a dot (e.g. 190.03).";
const CENTRE_DEC_ERROR = "Centre Dec must be decimal degrees from -90 to 90, written with a dot (e.g. -11.63).";
const SEARCH_RADIUS_ERROR = "Radius must be decimal degrees above 0 and up to 180, written with a dot (e.g. 1.5).";
const HINTED_SOLVE_STATUS = "Hinted solve around the field centre";
const BLIND_SOLVE_STATUS = "Blind solve: fill both centre fields to hint it";

function positiveNumber(text: string): number | null {
  const value = parseDecimalText(text);
  return value !== null && value > 0 ? value : null;
}

export function parseScaleRange(lowText: string, highText: string): ScaleRange {
  const low = positiveNumber(lowText);
  if (low === null) return { error: "Scale low must be a number above 0." };
  const high = positiveNumber(highText);
  if (high === null) return { error: "Scale high must be a number above 0." };
  if (low >= high) return { error: "Scale low must be below scale high." };
  return { low, high, error: null };
}

function parseRightAscension(text: string): number | null {
  const value = parseDecimalText(text);
  return value !== null && value >= 0 && value <= MAX_CENTRE_RA_DEG ? value : null;
}

function parseDeclination(text: string): number | null {
  const value = parseDecimalText(text);
  return value !== null && Math.abs(value) <= MAX_CENTRE_DEC_DEG ? value : null;
}

export function parseRadius(text: string): number | null {
  const value = parseDecimalText(text);
  return value !== null && value > 0 && value <= MAX_SEARCH_RADIUS_DEG ? value : null;
}

export interface PositionHint {
  centerRa: number | null;
  centerDec: number | null;
  radius: number | null;
  raInvalid: boolean;
  decInvalid: boolean;
  radiusInvalid: boolean;
  error: string | null;
}

function rejectedText(text: string, parsed: number | null): boolean {
  return parsed === null && text.trim() !== "";
}

function firstHintError(raInvalid: boolean, decInvalid: boolean, radiusInvalid: boolean): string | null {
  if (raInvalid) return CENTRE_RA_ERROR;
  if (decInvalid) return CENTRE_DEC_ERROR;
  if (radiusInvalid) return SEARCH_RADIUS_ERROR;
  return null;
}

export function parsePositionHint(raText: string, decText: string, radiusText: string): PositionHint {
  const centerRa = parseRightAscension(raText);
  const centerDec = parseDeclination(decText);
  const hinted = centerRa !== null && centerDec !== null;
  const radius = hinted ? parseRadius(radiusText) : null;
  const raInvalid = rejectedText(raText, centerRa);
  const decInvalid = rejectedText(decText, centerDec);
  const radiusInvalid = hinted && rejectedText(radiusText, radius);
  return {
    centerRa: hinted ? centerRa : null,
    centerDec: hinted ? centerDec : null,
    radius,
    raInvalid,
    decInvalid,
    radiusInvalid,
    error: firstHintError(raInvalid, decInvalid, radiusInvalid),
  };
}

export function positionHintStatus(hint: PositionHint): string | null {
  if (hint.error !== null) return null;
  return hint.centerRa !== null ? HINTED_SOLVE_STATUS : BLIND_SOLVE_STATUS;
}

export interface HeaderPositionHintTexts {
  ra: string;
  dec: string;
  radius: string | null;
}

function headerSearchRadiusText(fov: [number, number] | null | undefined): string | null {
  if (!fov || !Number.isFinite(fov[0]) || !Number.isFinite(fov[1])) return null;
  const text = (Math.hypot(fov[0], fov[1]) / 2 / 60).toFixed(4);
  return parseRadius(text) === null ? null : text;
}

export function headerPositionHintTexts(
  info: Pick<WcsInfo, "center_ra" | "center_dec" | "fov_arcmin">,
): HeaderPositionHintTexts | null {
  if (!Number.isFinite(info.center_ra) || !Number.isFinite(info.center_dec)) return null;
  return {
    ra: normalizeAngle(info.center_ra).toFixed(6),
    dec: info.center_dec.toFixed(6),
    radius: headerSearchRadiusText(info.fov_arcmin),
  };
}
