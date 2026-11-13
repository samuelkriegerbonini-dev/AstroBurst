import { MAX_APERTURE_RADIUS_PX, MAX_SKY_OUTER_RADIUS_PX, MIN_APERTURE_RADIUS_PX } from "./photometryTable";

export type RadiusUnit = "px" | "arcsec";
export const RADIUS_UNITS: readonly RadiusUnit[] = ["px", "arcsec"];
export const UNDERSAMPLED_FWHM_PX = 2;
export const MIN_SKY_INNER_RADIUS_PX = 1;
export const MAG_ERR_PER_RELATIVE_FLUX_ERR = 1.0857362047581294;

const NO_PIXEL_SCALE_ERROR =
  "Radii in arcsec need the pixel scale of a celestial WCS, which this image does not have. Switch the unit to px.";

function finite(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function usableScale(pixelScaleArcsec: number | null): pixelScaleArcsec is number {
  return finite(pixelScaleArcsec) && pixelScaleArcsec > 0;
}

export function radiusInPixels(value: number, unit: RadiusUnit, pixelScaleArcsec: number | null): number | null {
  if (!finite(value) || value <= 0) return null;
  if (unit === "px") return value;
  return usableScale(pixelScaleArcsec) ? value / pixelScaleArcsec : null;
}

export function radiusInArcsec(px: number | null | undefined, pixelScaleArcsec: number | null): number | null {
  return finite(px) && usableScale(pixelScaleArcsec) ? px * pixelScaleArcsec : null;
}

export interface RadiiInput { aperture?: number; skyInner?: number; skyOuter?: number }
export interface RadiiInPixels { apertureRadius?: number; annulusInner?: number; annulusOuter?: number }
export interface RadiiRequest { radii: RadiiInPixels; error: string | null }

function arcsecRadiiError(input: RadiiInput, scale: number): string | null {
  const s = scale.toPrecision(4);
  const { aperture, skyInner, skyOuter } = input;
  if (aperture !== undefined) {
    const px = radiusInPixels(aperture, "arcsec", scale);
    if (px === null || px < MIN_APERTURE_RADIUS_PX || px > MAX_APERTURE_RADIUS_PX) {
      return `Aperture radius ${String(aperture)} arcsec is ${(aperture / scale).toFixed(2)} px at ${s} arcsec/px; it must be between ${MIN_APERTURE_RADIUS_PX} and ${MAX_APERTURE_RADIUS_PX} px (${(MIN_APERTURE_RADIUS_PX * scale).toPrecision(4)} to ${(MAX_APERTURE_RADIUS_PX * scale).toPrecision(4)} arcsec).`;
    }
  }
  if (skyInner !== undefined) {
    const px = radiusInPixels(skyInner, "arcsec", scale);
    if (px === null || px < MIN_SKY_INNER_RADIUS_PX) {
      return `Sky inner radius ${String(skyInner)} arcsec is ${(skyInner / scale).toFixed(2)} px at ${s} arcsec/px; it must be at least ${MIN_SKY_INNER_RADIUS_PX} px (${(MIN_SKY_INNER_RADIUS_PX * scale).toPrecision(4)} arcsec).`;
    }
  }
  if (skyOuter !== undefined) {
    const px = radiusInPixels(skyOuter, "arcsec", scale);
    if (px === null || px > MAX_SKY_OUTER_RADIUS_PX) {
      return `Sky outer radius ${String(skyOuter)} arcsec is ${(skyOuter / scale).toFixed(1)} px at ${s} arcsec/px; it must be at most ${MAX_SKY_OUTER_RADIUS_PX} px (${(MAX_SKY_OUTER_RADIUS_PX * scale).toPrecision(4)} arcsec).`;
    }
  }
  return null;
}

export function radiiRequest(input: RadiiInput, unit: RadiusUnit, pixelScaleArcsec: number | null): RadiiRequest {
  const { aperture, skyInner, skyOuter } = input;
  if (unit === "px") {
    return { radii: { apertureRadius: aperture, annulusInner: skyInner, annulusOuter: skyOuter }, error: null };
  }
  if (aperture === undefined && skyInner === undefined && skyOuter === undefined) return { radii: {}, error: null };
  if (!usableScale(pixelScaleArcsec)) return { radii: {}, error: NO_PIXEL_SCALE_ERROR };
  const error = arcsecRadiiError(input, pixelScaleArcsec);
  if (error !== null) return { radii: {}, error };
  const radii: RadiiInPixels = {};
  if (aperture !== undefined) radii.apertureRadius = aperture / pixelScaleArcsec;
  if (skyInner !== undefined) radii.annulusInner = skyInner / pixelScaleArcsec;
  if (skyOuter !== undefined) radii.annulusOuter = skyOuter / pixelScaleArcsec;
  return { radii, error: null };
}

export function isUndersampled(fwhmPx: number | null | undefined): boolean {
  return finite(fwhmPx) && fwhmPx < UNDERSAMPLED_FWHM_PX;
}

export function skyLevelError(sigmaPerPixel: number | null | undefined, nPixels: number | null | undefined): number | null {
  if (!finite(sigmaPerPixel) || sigmaPerPixel < 0 || !finite(nPixels) || nPixels <= 0) return null;
  return sigmaPerPixel / Math.sqrt(nPixels);
}

export interface NativeZeroPoint { zp: number; exptimeS: number | null; exptimeApplied: boolean }
export interface SystemMagnitude { mag: number; magErr: number | null }

export function systemMagnitude(
  magInst: number | null | undefined,
  netFlux: number | null | undefined,
  fluxErr: number | null | undefined,
  zeroPoint: NativeZeroPoint,
): SystemMagnitude | null {
  const { zp, exptimeS, exptimeApplied } = zeroPoint;
  if (!finite(magInst) || !finite(zp)) return null;
  if (exptimeApplied && !(finite(exptimeS) && exptimeS > 0)) return null;
  const exposureTerm = exptimeApplied && finite(exptimeS) ? 2.5 * Math.log10(exptimeS) : 0;
  const magErr =
    finite(fluxErr) && fluxErr >= 0 && finite(netFlux) && netFlux > 0
      ? (MAG_ERR_PER_RELATIVE_FLUX_ERR * fluxErr) / netFlux
      : null;
  return { mag: zp + magInst + exposureTerm, magErr };
}
