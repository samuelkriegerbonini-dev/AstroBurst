import { isFiniteNumber } from "./values";

export const DEFAULT_LIMIT_SIGMA = 3;
export const AB_ZERO_POINT_JY = 8.9;

export interface UpperLimit {
  isLimit: boolean;
  limitJy: number | null;
  limitAb: number | null;
  effectiveErrJy: number | null;
}

export function abFromJy(fluxJy: number | null | undefined): number | null {
  return isFiniteNumber(fluxJy) && fluxJy > 0 ? -2.5 * Math.log10(fluxJy) + AB_ZERO_POINT_JY : null;
}

export function effectiveError(errJy: number | null | undefined, fCorr?: number | null): number | null {
  if (typeof errJy !== "number") return null;
  const effective = errJy * (fCorr ?? 1);
  return Number.isFinite(effective) && effective > 0 ? effective : null;
}

export function upperLimit(
  fluxJy: number | null | undefined,
  errJy: number | null | undefined,
  k: number,
  fCorr?: number | null,
): UpperLimit {
  const effective = effectiveError(errJy, fCorr);
  if (effective !== null && Number.isFinite(k) && k > 0 && isFiniteNumber(fluxJy) && fluxJy < k * effective) {
    const limitJy = k * effective;
    return { isLimit: true, limitJy, limitAb: abFromJy(limitJy), effectiveErrJy: effective };
  }
  return { isLimit: false, limitJy: null, limitAb: null, effectiveErrJy: effective };
}
