const ARCSEC_PER_ARCMIN = 60;
const ARCSEC_PER_DEG = 3600;
const ARCSEC_DIGITS = 2;
const ARCMIN_DIGITS = 2;
const DEG_DIGITS = 3;
const PA_DIGITS = 1;
const FULL_TURN_DEG = 360;
const NON_FINITE = "--";

export function formatSeparation(arcsec: number): string {
  if (!Number.isFinite(arcsec)) return NON_FINITE;
  const abs = Math.abs(arcsec);
  if (abs < ARCSEC_PER_ARCMIN) return `${abs.toFixed(ARCSEC_DIGITS)}"`;
  if (abs < ARCSEC_PER_DEG) return `${(abs / ARCSEC_PER_ARCMIN).toFixed(ARCMIN_DIGITS)}'`;
  return `${(abs / ARCSEC_PER_DEG).toFixed(DEG_DIGITS)} deg`;
}

export function normalizePositionAngle(deg: number): number {
  const wrapped = ((deg % FULL_TURN_DEG) + FULL_TURN_DEG) % FULL_TURN_DEG;
  return Number(wrapped.toFixed(PA_DIGITS)) >= FULL_TURN_DEG ? 0 : wrapped;
}

export function formatPositionAngle(deg: number): string {
  if (!Number.isFinite(deg)) return NON_FINITE;
  return `${normalizePositionAngle(deg).toFixed(PA_DIGITS)} deg E of N`;
}

export function pixelLength(x1: number, y1: number, x2: number, y2: number): number {
  return Math.hypot(x2 - x1, y2 - y1);
}
