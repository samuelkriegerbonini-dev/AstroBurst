export const MISSING_VALUE = "--";
export const PLUS_MINUS = "±";

export interface UncertaintyFormatOptions {
  unit?: string;
  errDigits?: 1 | 2;
}

const MAX_SHARED_SPAN = 12;
const FIXED_FROM = 1e-3;
const FIXED_BELOW = 1e5;
const INTEGER_FROM = 1e3;
const ROUND_TRIP_DECIMALS = 16;

function fixedText(x: number, decimals: number): string {
  const text = x.toFixed(decimals);
  return text.startsWith("-") && !/[1-9]/.test(text) ? text.slice(1) : text;
}

function exponentialText(x: number, decimals: number): string {
  return x.toExponential(decimals).replace("e+", "e");
}

function exponentAfterE(text: string): number {
  return Number(text.slice(text.indexOf("e") + 1));
}

function roundedExponent(x: number, decimals: number): number {
  return exponentAfterE(x.toExponential(decimals));
}

function shortestExponent(x: number): number {
  return exponentAfterE(Math.abs(x).toExponential());
}

function shiftDecimal(x: number, places: number): number {
  if (x === 0) return 0;
  const text = x.toExponential(ROUND_TRIP_DECIMALS);
  return Number(`${text.slice(0, text.indexOf("e"))}e${exponentAfterE(text) + places}`);
}

function fourSignificant(x: number): string {
  if (x === 0) return "0";
  const magnitude = Math.abs(x);
  if (magnitude >= FIXED_FROM && magnitude < INTEGER_FROM) return fixedText(x, 3 - roundedExponent(x, 3));
  if (magnitude >= INTEGER_FROM && magnitude < FIXED_BELOW) return fixedText(x, 0);
  return exponentialText(x, 3);
}

function isUsableError(err: number | null | undefined): err is number {
  return err != null && Number.isFinite(err) && err > 0;
}

function pair(valueText: string, errText: string): string {
  return `${valueText} ${PLUS_MINUS} ${errText}`;
}

function coarseText(x: number, decimals: number): string {
  const text = fixedText(shiftDecimal(x, decimals), 0);
  return text === "0" ? text : text + "0".repeat(-decimals);
}

function valueWithError(value: number, err: number, digits: 1 | 2): string {
  const errExponent = roundedExponent(err, digits - 1);
  const decimals = digits - 1 - errExponent;
  const largest = Math.max(Math.abs(value), err);
  const topExponent = Math.max(shortestExponent(largest), errExponent);
  if (topExponent - errExponent > MAX_SHARED_SPAN) {
    return pair(fourSignificant(value), exponentialText(err, digits - 1));
  }
  if (largest >= FIXED_FROM && largest < FIXED_BELOW) {
    return decimals >= 0
      ? pair(fixedText(value, decimals), fixedText(err, decimals))
      : pair(coarseText(value, decimals), coarseText(err, decimals));
  }
  const places = decimals + topExponent;
  const mantissas = pair(fixedText(shiftDecimal(value, -topExponent), places), fixedText(shiftDecimal(err, -topExponent), places));
  return `(${mantissas})e${topExponent}`;
}

export function formatWithUncertainty(
  value: number | null | undefined,
  err: number | null | undefined,
  options?: UncertaintyFormatOptions,
): string {
  if (value == null || !Number.isFinite(value)) return MISSING_VALUE;
  const body = isUsableError(err) ? valueWithError(value, err, options?.errDigits === 1 ? 1 : 2) : fourSignificant(value);
  return options?.unit ? `${body} ${options.unit}` : body;
}

function janskyPrefix(reference: number): { unit: string; places: number } {
  if (reference >= 1 || reference === 0) return { unit: "Jy", places: 0 };
  if (reference >= 1e-3) return { unit: "mJy", places: 3 };
  if (reference >= 1e-6) return { unit: "uJy", places: 6 };
  return { unit: "nJy", places: 9 };
}

export function formatFluxDensityJy(valueJy: number | null | undefined, errJy: number | null | undefined): string {
  if (valueJy == null || !Number.isFinite(valueJy)) return MISSING_VALUE;
  const err = isUsableError(errJy) ? errJy : null;
  const { unit, places } = janskyPrefix(valueJy === 0 && err !== null ? err : Math.abs(valueJy));
  return formatWithUncertainty(shiftDecimal(valueJy, places), err === null ? null : shiftDecimal(err, places), { unit });
}
