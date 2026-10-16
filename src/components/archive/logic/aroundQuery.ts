import type { WcsInfo } from "../../../shared/types/astrometry";

export interface AroundTarget {
  raDeg: number;
  decDeg: number;
  radiusArcmin: number;
  label: string;
}

export const AROUND_MIN_ARCMIN = 1;
export const AROUND_MAX_ARCMIN = 30;

function fieldOf(wcs: WcsInfo): [number, number] | null {
  const fov = Array.isArray(wcs.fov_arcmin) ? wcs.fov_arcmin : [wcs.field_of_view_w_arcmin, wcs.field_of_view_h_arcmin];
  const [w, h] = fov;
  return Number.isFinite(w) && Number.isFinite(h) && w > 0 && h > 0 ? [w, h] : null;
}

export function aroundFromWcs(wcs: WcsInfo | null | undefined, label: string): AroundTarget | null {
  if (!wcs) return null;
  const ra = wcs.center_ra;
  const dec = wcs.center_dec;
  if (!Number.isFinite(ra) || !Number.isFinite(dec) || dec < -90 || dec > 90) return null;
  const field = fieldOf(wcs);
  if (!field) return null;
  const halfDiagonal = Math.hypot(field[0], field[1]) / 2;
  const clamped = Math.min(AROUND_MAX_ARCMIN, Math.max(AROUND_MIN_ARCMIN, halfDiagonal));
  const radiusArcmin = Math.min(AROUND_MAX_ARCMIN, Math.ceil(clamped * 10 - 1e-9) / 10);
  const raDeg = ((ra % 360) + 360) % 360;
  return { raDeg, decDeg: dec, radiusArcmin, label };
}

export function aroundLabel(target: AroundTarget): string {
  return `Around ${target.label}  ${target.raDeg.toFixed(2)} ${target.decDeg.toFixed(2)}  r ${target.radiusArcmin}'`;
}

export function aroundQueryText(target: AroundTarget): string {
  return `${target.raDeg.toFixed(5)} ${target.decDeg.toFixed(5)}`;
}
