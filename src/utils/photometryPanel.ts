import type { BatchPhotometryResult, MagSystem, PhotCal, PhotometryMeasurement, StarPhotometry, UserZeroPoint } from "../services/analysis";
import { decimalFieldState, type DecimalFieldOptions } from "./decimalText";
import type { NoiseFactorOffer } from "./forcedPhotometryRequest";
import { formatFluxDensityJy, formatWithUncertainty } from "./formatUncertainty";
import { radiusInArcsec, radiusInPixels, systemMagnitude, type NativeZeroPoint, type RadiusUnit } from "./photometryUnits";
import { DEFAULT_LIMIT_SIGMA, upperLimit } from "./upperLimits";
import { isFiniteNumber } from "./values";

export const GAIA_MATCH_PREF_KEY = "astroburst.photometry.gaiaMatch.v1";

export function loadGaiaMatchPreference(storage: Pick<Storage, "getItem"> = localStorage): boolean {
  try {
    return storage.getItem(GAIA_MATCH_PREF_KEY) === "true";
  } catch {
    return false;
  }
}

export function saveGaiaMatchPreference(value: boolean, storage: Pick<Storage, "setItem"> = localStorage): void {
  try {
    storage.setItem(GAIA_MATCH_PREF_KEY, value ? "true" : "false");
  } catch {
  }
}

export function gaiaFailureWarning(error: unknown): string {
  return `Gaia query failed: ${error instanceof Error ? error.message : String(error)}`;
}

export function withWarning(result: PhotometryMeasurement, warning: string): PhotometryMeasurement {
  return result.warnings.includes(warning) ? result : { ...result, warnings: [...result.warnings, warning] };
}

export function measurementWarnings(result: PhotometryMeasurement): string[] {
  return [...new Set([...result.warnings, ...result.photometry.warnings])];
}

export function batchWarnings(result: BatchPhotometryResult): string[] {
  return [...new Set([...result.warnings, ...result.rows.flatMap((row) => row.photometry?.warnings ?? [])])];
}

export function replaceNewest(
  history: PhotometryMeasurement[],
  previous: PhotometryMeasurement,
  next: PhotometryMeasurement,
): PhotometryMeasurement[] {
  return history[0] === previous ? [next, ...history.slice(1)] : history;
}

export interface ClickLogGate {
  begin(): number;
  isCurrent(seq: number): boolean;
  hold(seq: number, append: () => void): void;
  commit(seq: number, append: () => void): void;
  supersede(): void;
}

export function createClickLogGate(): ClickLogGate {
  let current = 0;
  let held: (() => void) | null = null;
  const flush = () => {
    const append = held;
    held = null;
    append?.();
  };
  return {
    begin() {
      flush();
      return ++current;
    },
    isCurrent: (seq) => seq === current,
    hold(seq, append) {
      if (seq === current) held = append;
    },
    commit(seq, append) {
      if (seq !== current) return;
      held = null;
      append();
    },
    supersede() {
      current++;
      flush();
    },
  };
}

export function detectedStarsLabel(shown: number, total: number | null | undefined): string {
  const capped = shown > 0 && typeof total === "number" && Number.isFinite(total) && total > shown;
  return capped ? `Detected stars (${shown} of ${total})` : `Detected stars (${shown})`;
}

export function convertedRadiusText(text: string, from: RadiusUnit, to: RadiusUnit, pixelScaleArcsec: number | null): string {
  const value = Number(text);
  if (from === to || !Number.isFinite(value) || value <= 0) return text;
  const converted = to === "arcsec" ? radiusInArcsec(value, pixelScaleArcsec) : radiusInPixels(value, "arcsec", pixelScaleArcsec);
  return converted === null ? text : String(Number(converted.toPrecision(4)));
}

const MAGNITUDE_TITLES: Record<MagSystem, string> = {
  ab: "AB mag",
  unknown: "AB mag (system assumed)",
  vega: "Vega mag",
  st: "ST mag",
};

const MAGNITUDE_COLUMN_HEADERS: Record<MagSystem, string> = { ab: "AB", unknown: "AB", vega: "Vega", st: "ST" };

export function magnitudeTitle(system: MagSystem | null | undefined): string {
  return MAGNITUDE_TITLES[system ?? "ab"];
}

export function magnitudeColumnHeader(system: MagSystem | null | undefined): string {
  return MAGNITUDE_COLUMN_HEADERS[system ?? "ab"];
}

export function nativeZeroPointOf(photcal: PhotCal | null | undefined): NativeZeroPoint | null {
  const convention = photcal?.convention;
  if (convention?.kind !== "generic_zero_point") return null;
  return { zp: convention.zp, exptimeS: convention.exptime ?? null, exptimeApplied: convention.exptime_applied === true };
}

export interface CalibratedMagnitude {
  mag: number | null;
  magErr: number | null;
  system: MagSystem | null;
}

export function calibratedMagnitude(phot: StarPhotometry, photcal: PhotCal | null | undefined): CalibratedMagnitude {
  if (!photcal) return { mag: null, magErr: null, system: null };
  const system = photcal.mag_system ?? "ab";
  if (system === "ab" || system === "unknown") return { mag: phot.mag_ab, magErr: phot.mag_ab_err, system };
  const zeroPoint = nativeZeroPointOf(photcal);
  const measured = zeroPoint === null ? null : systemMagnitude(phot.mag_inst, phot.net_flux, phot.flux_err, zeroPoint);
  return measured === null ? { mag: null, magErr: null, system } : { mag: measured.mag, magErr: measured.magErr, system };
}

export function calibrationText(photcal: PhotCal | null | undefined): string | null {
  if (!photcal) return null;
  return photcal.origin ? photcal.origin : photcal.label;
}

export const ZERO_POINT_FIELD: DecimalFieldOptions = { optional: true, min: -100, max: 100 };
export const ZERO_POINT_TITLE = "Zero point in magnitudes of one pixel unit; used because the header has no calibration";
export const ZERO_POINT_CAPTION =
  "The header has no calibration: a typed zero point is applied and reported as such. Per second divides counts by EXPTIME. A zero point from the Gaia cross-match is a Vega zero point.";

export function userZeroPoint(text: string, system: "ab" | "vega", perSecond: boolean): UserZeroPoint | undefined {
  const { value } = decimalFieldState(text, ZERO_POINT_FIELD);
  return value === null ? undefined : { zp: value, system, per_second: perSecond };
}

export interface PhotometryLimit {
  sigma: number;
  fCorr: number | null;
  isLimit: boolean;
  limitJy: number | null;
  limitAb: number | null;
  limitNative: number | null;
}

export function appliedDepthFactor(offer: NoiseFactorOffer, accepted: number | null): number | null {
  return offer.kind === "usable" && accepted !== null && accepted === offer.fCorr ? accepted : null;
}

export function depthRadiusText(offer: NoiseFactorOffer): string | null {
  if (offer.kind === "none") return null;
  return `r = ${String(Number(offer.radius.toPrecision(4)))} ${offer.unit}`;
}

export function depthNoiseText(offer: NoiseFactorOffer, accepted: number | null): string | null {
  if (offer.kind === "none") return null;
  const radius = depthRadiusText(offer);
  if (offer.kind === "other_radius") {
    return `The Depth section measured this image at ${radius}, not with this aperture: its f_corr is not offered. Measure the depth again with this radius to get one.`;
  }
  const factor = offer.fCorr.toFixed(2);
  if (accepted === offer.fCorr) {
    return `f_corr ${factor} from the Depth section (${radius}) multiplies the errors of the next measurements before the limit test.`;
  }
  return `Depth section: f_corr ${factor} for this image at ${radius}, the radius of this aperture. Not applied: click Use to multiply the errors by it before the limit test.`;
}

export function photometryLimit(phot: StarPhotometry, fCorr: number | null): PhotometryLimit {
  const sigma = DEFAULT_LIMIT_SIGMA;
  const factor = phot.err_used ? fCorr : null;
  const native = upperLimit(phot.net_flux, phot.flux_err, sigma, factor ?? undefined);
  const limitNative = native.isLimit ? native.limitJy : null;
  if (isFiniteNumber(phot.flux_jy) && isFiniteNumber(phot.flux_err_jy)) {
    const jy = upperLimit(phot.flux_jy, phot.flux_err_jy, sigma, factor ?? undefined);
    return { sigma, fCorr: factor, isLimit: jy.isLimit, limitJy: jy.limitJy, limitAb: jy.limitAb, limitNative };
  }
  return { sigma, fCorr: factor, isLimit: native.isLimit, limitJy: null, limitAb: null, limitNative };
}

export function limitSentence(limit: PhotometryLimit, summedUnit: string | null): string | null {
  if (!limit.isLimit) return null;
  const bound =
    limit.limitAb !== null
      ? `AB > ${limit.limitAb.toFixed(2)} (flux below ${formatFluxDensityJy(limit.limitJy, null)})`
      : `net flux below ${formatWithUncertainty(limit.limitNative, null)}${summedUnit === null ? "" : ` ${summedUnit}`}`;
  const factor = limit.fCorr === null ? "" : `; errors multiplied by f_corr ${limit.fCorr.toFixed(2)} from the Depth section`;
  return `not detected at ${limit.sigma} sigma: ${bound}${factor}`;
}
