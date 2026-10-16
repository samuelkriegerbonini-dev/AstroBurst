import type { MastQuickWcs } from "../../../shared/types/mast";

export interface ScaleBar {
  px: number;
  label: string;
}

export function pixelScaleText(wcs: MastQuickWcs): string {
  const s = wcs.pixelScaleArcsec;
  const digits = s < 1 ? 3 : s < 10 ? 2 : 1;
  return `${s.toFixed(digits)}"/px`;
}

function angleText(arcmin: number): string {
  return `${arcmin.toFixed(1)}'`;
}

export function fieldText(wcs: MastQuickWcs): string {
  const [w, h] = wcs.fieldArcmin;
  if (Math.max(w, h) < 1) return `field ${Math.round(w * 60)}" x ${Math.round(h * 60)}"`;
  return `field ${angleText(w)} x ${angleText(h)}`;
}

export function orientationText(wcs: MastQuickWcs, place: "pane" | "rail"): string | null {
  if (wcs.orientation === "cd_matrix") return "orientation from the CD matrix";
  return place === "rail" ? "orientation not given" : null;
}

const STEPS_ARCSEC = [0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 50, 60, 120, 300, 600, 1200, 3000, 3600, 7200, 18000, 36000];

function lengthLabel(arcsec: number): string {
  if (arcsec < 60) return `${arcsec}"`;
  if (arcsec < 3600) return `${arcsec / 60}'`;
  return `${arcsec / 3600}°`;
}

export function scaleBar(arcsecPerScreenPx: number, maxPx: number): ScaleBar | null {
  if (!Number.isFinite(arcsecPerScreenPx) || arcsecPerScreenPx <= 0 || maxPx <= 0) return null;
  let best: number | null = null;
  for (const step of STEPS_ARCSEC) {
    if (step / arcsecPerScreenPx <= maxPx) best = step;
  }
  if (best === null) return null;
  return { px: best / arcsecPerScreenPx, label: lengthLabel(best) };
}
