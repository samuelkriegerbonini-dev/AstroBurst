import type { SolveHints, WcsInfo } from "../shared/types/astrometry";
import { parseDecimalText } from "./decimalText";
import { normalizeAngle } from "./regionGeometry";
import { parseAngle } from "./skyList";

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
export const MAX_OFFLINE_NEAR_RADIUS_DEG = 15;
const CENTRE_RA_ERROR = "Centre RA must be degrees from 0 to 360 written with a dot (e.g. 190.03), or hours as h:m:s (e.g. 12:40:07.2).";
const CENTRE_DEC_ERROR = "Centre Dec must be degrees from -90 to 90 written with a dot (e.g. -11.63), or d:m:s (e.g. -11:38:00).";
const SEARCH_RADIUS_ERROR = "Radius must be decimal degrees above 0 and up to 180, written with a dot (e.g. 1.5).";
const HINTED_SOLVE_STATUS = "Hinted solve around the field centre";
const BLIND_SOLVE_STATUS = "Blind solve: only astrometry.net solves without a centre; fill both centre fields to solve offline";
const NEGATIVE_SIGN = "-";

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
  if (value !== null) return value >= 0 && value <= MAX_CENTRE_RA_DEG ? value : null;
  if (text.trim().startsWith(NEGATIVE_SIGN)) return null;
  return parseAngle(text, "lon").value;
}

function parseDeclination(text: string): number | null {
  const value = parseDecimalText(text);
  if (value !== null) return Math.abs(value) <= MAX_CENTRE_DEC_DEG ? value : null;
  return parseAngle(text, "lat").value;
}

export function parseRadius(text: string): number | null {
  const value = parseDecimalText(text);
  return value !== null && value > 0 && value <= MAX_SEARCH_RADIUS_DEG ? value : null;
}

export function offlineNearRadius(radius: number | null): number | undefined {
  return radius === null ? undefined : Math.min(radius, MAX_OFFLINE_NEAR_RADIUS_DEG);
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

export type ScaleUnits = "arcsecperpix" | "arcminwidth" | "degwidth";

export interface HintTexts {
  scaleLow: string;
  scaleHigh: string;
  scaleUnits: ScaleUnits;
  centerRa: string;
  centerDec: string;
  radius: string;
}

export type HintField = keyof HintTexts;

export const DEFAULT_HINT_TEXTS: HintTexts = {
  scaleLow: DEFAULT_SCALE_LOW_TEXT,
  scaleHigh: DEFAULT_SCALE_HIGH_TEXT,
  scaleUnits: "arcsecperpix",
  centerRa: "",
  centerDec: "",
  radius: "",
};

const SCALE_HINT_FIELDS: readonly HintField[] = ["scaleLow", "scaleHigh", "scaleUnits"];
const POSITION_HINT_FIELDS: readonly HintField[] = ["centerRa", "centerDec", "radius"];

export function headerHintTexts(
  info: Pick<WcsInfo, "pixel_scale_arcsec" | "center_ra" | "center_dec" | "fov_arcmin">,
): Partial<HintTexts> {
  const hints: Partial<HintTexts> = {};
  const scale = scaleHintFromPixelScale(info.pixel_scale_arcsec);
  if (scale) {
    hints.scaleLow = scale.low;
    hints.scaleHigh = scale.high;
    hints.scaleUnits = "arcsecperpix";
  }
  const position = headerPositionHintTexts(info);
  if (position) {
    hints.centerRa = position.ra;
    hints.centerDec = position.dec;
    if (position.radius !== null) hints.radius = position.radius;
  }
  return hints;
}

export function mergeHintTexts(
  current: HintTexts,
  hints: Partial<HintTexts>,
  touched: ReadonlySet<HintField>,
): HintTexts {
  const next: HintTexts = { ...current };
  const take = (field: HintField) => {
    if (hints[field] !== undefined) Object.assign(next, { [field]: hints[field] });
  };
  if (!SCALE_HINT_FIELDS.some((field) => touched.has(field))) SCALE_HINT_FIELDS.forEach(take);
  POSITION_HINT_FIELDS.filter((field) => !touched.has(field)).forEach(take);
  return next;
}

export const WCS_HINT_SOURCE = "WCS";
export const TYPED_HINT_SOURCE = "typed";

export interface HintSources {
  centre: string | null;
  scale: string | null;
}

export const NO_HINT_SOURCES: HintSources = { centre: null, scale: null };

type HeaderHintInfo = Pick<WcsInfo, "pixel_scale_arcsec" | "center_ra" | "center_dec" | "fov_arcmin">;

function pickHints(hints: Partial<HintTexts>, fields: readonly HintField[]): Partial<HintTexts> {
  const picked: Partial<HintTexts> = {};
  for (const field of fields) {
    if (hints[field] !== undefined) Object.assign(picked, { [field]: hints[field] });
  }
  return picked;
}

export function solveHintTexts(hints: SolveHints): Partial<HintTexts> {
  const texts: Partial<HintTexts> = {};
  const scale = hints.scale;
  if (scale && scale.low_arcsec > 0 && scale.high_arcsec >= scale.low_arcsec && Number.isFinite(scale.high_arcsec)) {
    texts.scaleLow = roundSignificant(scale.low_arcsec, "down");
    texts.scaleHigh = roundSignificant(scale.high_arcsec, "up");
    texts.scaleUnits = "arcsecperpix";
  }
  const centre = hints.centre;
  if (centre && Number.isFinite(centre.ra) && Number.isFinite(centre.dec) && Math.abs(centre.dec) <= MAX_CENTRE_DEC_DEG) {
    texts.centerRa = normalizeAngle(centre.ra).toFixed(6);
    texts.centerDec = centre.dec.toFixed(6);
  }
  return texts;
}

export function panelHints(
  info: HeaderHintInfo | null,
  hints: SolveHints | null,
): { texts: Partial<HintTexts>; sources: HintSources } {
  const wcs = info ? headerHintTexts(info) : {};
  const header = hints ? solveHintTexts(hints) : {};
  const centreFromWcs = wcs.centerRa !== undefined;
  const scaleFromWcs = wcs.scaleLow !== undefined;
  const centreFromHints = header.centerRa !== undefined;
  const scaleFromHints = header.scaleLow !== undefined;
  const position = pickHints(centreFromWcs ? wcs : header, POSITION_HINT_FIELDS);
  return {
    texts: {
      ...pickHints(scaleFromHints ? header : wcs, SCALE_HINT_FIELDS),
      ...(position.centerRa === undefined ? position : { radius: DEFAULT_HINT_TEXTS.radius, ...position }),
    },
    sources: {
      centre: centreFromWcs ? WCS_HINT_SOURCE : centreFromHints ? (hints?.centre?.source ?? null) : null,
      scale: scaleFromHints ? (hints?.scale?.source ?? null) : scaleFromWcs ? WCS_HINT_SOURCE : null,
    },
  };
}

const CENTRE_HINT_FIELDS: readonly HintField[] = ["centerRa", "centerDec"];

export function editedHintSources(current: HintSources, field: HintField): HintSources {
  if (CENTRE_HINT_FIELDS.includes(field)) return { ...current, centre: TYPED_HINT_SOURCE };
  if (SCALE_HINT_FIELDS.includes(field)) return { ...current, scale: TYPED_HINT_SOURCE };
  return current;
}

export function mergeHintSources(current: HintSources, next: HintSources, touched: ReadonlySet<HintField>): HintSources {
  const centreTyped = CENTRE_HINT_FIELDS.some((field) => touched.has(field));
  const scaleTyped = SCALE_HINT_FIELDS.some((field) => touched.has(field));
  return {
    centre: centreTyped ? current.centre : next.centre,
    scale: scaleTyped ? current.scale : next.scale,
  };
}

function sourcePhrase(group: "centre" | "scale", source: string | null): string {
  if (source === TYPED_HINT_SOURCE) return `${group} typed`;
  if (source === WCS_HINT_SOURCE) return `${group} from the header WCS`;
  if (source !== null) return `${group} from ${source}`;
  return group === "centre" ? "no centre in the header" : "default scale range";
}

export function hintSourceLine(sources: HintSources): string | null {
  if (sources.centre === null && sources.scale === null) return null;
  if (sources.centre === WCS_HINT_SOURCE && sources.scale === WCS_HINT_SOURCE) return "Hints from the header WCS";
  return `Hints: ${sourcePhrase("centre", sources.centre)}; ${sourcePhrase("scale", sources.scale)}`;
}
