import type { Pt } from "./regionGeometry";

const ARCSEC_PER_ARCMIN = 60;
const ARCSEC_PER_DEG = 3600;
const SCALE_BAR_LADDER_ARCSEC: readonly number[] = [
  0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30,
  ARCSEC_PER_ARCMIN, 2 * ARCSEC_PER_ARCMIN, 5 * ARCSEC_PER_ARCMIN, 10 * ARCSEC_PER_ARCMIN, 15 * ARCSEC_PER_ARCMIN, 30 * ARCSEC_PER_ARCMIN,
  ARCSEC_PER_DEG, 2 * ARCSEC_PER_DEG, 5 * ARCSEC_PER_DEG, 10 * ARCSEC_PER_DEG,
];

export interface ScaleBarLength {
  arcsec: number;
  label: string;
}

export function formatScaleBarLabel(arcsec: number): string {
  if (arcsec >= ARCSEC_PER_DEG) return `${Number((arcsec / ARCSEC_PER_DEG).toFixed(2))} deg`;
  if (arcsec >= ARCSEC_PER_ARCMIN) return `${Number((arcsec / ARCSEC_PER_ARCMIN).toFixed(2))}'`;
  return `${Number(arcsec.toFixed(2))}"`;
}

export function niceScaleBarLength(maxArcsec: number): ScaleBarLength | null {
  if (!Number.isFinite(maxArcsec) || maxArcsec <= 0) return null;
  let best: number | null = null;
  for (const candidate of SCALE_BAR_LADDER_ARCSEC) {
    if (candidate <= maxArcsec) best = candidate;
    else break;
  }
  if (best === null) return null;
  return { arcsec: best, label: formatScaleBarLabel(best) };
}

export function screenDirection(from: Pt, to: Pt): Pt | null {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const len = Math.hypot(dx, dy);
  if (!Number.isFinite(len) || len === 0) return null;
  return { x: dx / len, y: dy / len };
}

export interface CompassArrows {
  northTip: Pt;
  eastTip: Pt;
}

export function compassArrows(origin: Pt, northDir: Pt, eastDir: Pt, lengthPx: number): CompassArrows {
  return {
    northTip: { x: origin.x + northDir.x * lengthPx, y: origin.y + northDir.y * lengthPx },
    eastTip: { x: origin.x + eastDir.x * lengthPx, y: origin.y + eastDir.y * lengthPx },
  };
}

export function scaleBarPixels(arcsec: number, pixelScaleArcsec: number, screenPxPerImagePx: number): number {
  if (!Number.isFinite(pixelScaleArcsec) || pixelScaleArcsec <= 0) return 0;
  return (arcsec / pixelScaleArcsec) * screenPxPerImagePx;
}

export interface PixelScaleInfo {
  pixel_scale_arcsec: number;
  pixel_scale_x_arcsec?: number | null;
}

export function horizontalPixelScaleArcsec(info: PixelScaleInfo): number {
  const sx = info.pixel_scale_x_arcsec;
  return typeof sx === "number" && Number.isFinite(sx) && sx > 0 ? sx : info.pixel_scale_arcsec;
}

export function overlayWcsPath(fileKey: string | null, displayedPath: string | null): string | null {
  return fileKey === null ? null : (displayedPath ?? fileKey);
}
