export const VIEWER_ZOOM_MIN = 0.1;
export const VIEWER_ZOOM_MAX = 32;
export const FIT_SCALE_CAP = 16;

const WHEEL_NOTCH_PX = 100;
const WHEEL_NOTCH_FACTOR = 1.15;
const WHEEL_K = Math.log(WHEEL_NOTCH_FACTOR) / WHEEL_NOTCH_PX;
const WHEEL_LINE_PX = WHEEL_NOTCH_PX / 3;
const WHEEL_PAGE_PX = WHEEL_NOTCH_PX;
const DOM_DELTA_LINE = 1;
const DOM_DELTA_PAGE = 2;
const PRESET_TOLERANCE = 0.01;

export function fitsPerRenderPx(fitsW: number | undefined, renderW: number): number {
  if (fitsW === undefined || !Number.isFinite(fitsW) || fitsW <= 0) return 1;
  if (!Number.isFinite(renderW) || renderW <= 0) return 1;
  return fitsW / renderW;
}

export function renderScaleForFits(fitsScale: number, fitsPerRender: number): number {
  return fitsScale * fitsPerRender;
}

export function zoomPercentLabel(renderScale: number, fitsPerRender: number): string {
  const pct = (renderScale / fitsPerRender) * 100;
  return pct < 10 ? `${pct.toFixed(1)}%` : `${Math.round(pct)}%`;
}

export function isZoomPresetActive(renderScale: number, preset: number, fitsPerRender: number): boolean {
  return Math.abs(renderScale / fitsPerRender / preset - 1) < PRESET_TOLERANCE;
}

export function clampViewerScale(scale: number, fitsPerRender: number): number {
  if (!Number.isFinite(scale)) return VIEWER_ZOOM_MIN;
  const max = VIEWER_ZOOM_MAX * Math.max(1, fitsPerRender);
  return Math.max(VIEWER_ZOOM_MIN, Math.min(max, scale));
}

export function fitScale(
  containerW: number,
  containerH: number,
  renderW: number,
  renderH: number,
  fitsPerRender: number,
): number {
  return Math.min(containerW / renderW, containerH / renderH, FIT_SCALE_CAP * fitsPerRender);
}

export function wheelZoomFactor(deltaY: number, deltaMode: number): number {
  const px =
    deltaMode === DOM_DELTA_LINE ? deltaY * WHEEL_LINE_PX : deltaMode === DOM_DELTA_PAGE ? deltaY * WHEEL_PAGE_PX : deltaY;
  if (!Number.isFinite(px) || px === 0) return 1;
  return Math.exp(-px * WHEEL_K);
}

export function imageRenderingFor(renderScale: number): "pixelated" | "auto" {
  return renderScale > 1 ? "pixelated" : "auto";
}

export function previewTextureBadge(renderW: number, fitsW: number | undefined): string | null {
  if (fitsW === undefined || !Number.isFinite(fitsW) || renderW <= 0 || fitsW <= renderW) return null;
  return `preview ${renderW} px (${(fitsW / renderW).toFixed(1)}:1)`;
}

export const ZOOM_PAN_MIN = 0.25;

export const ZOOM_PAN_MAX = 16;

export const ACTUAL_SIZE_SCALE = 1;

export function clampZoomPanScale(scale: number, fitScale: number): number {
  const floor = fitScale > 0 ? Math.min(ZOOM_PAN_MIN, fitScale) : ZOOM_PAN_MIN;
  return Math.max(floor, Math.min(ZOOM_PAN_MAX, scale));
}

export type WheelZoomMode = "always" | "modifier";

export function wheelGestureZooms(
  mode: WheelZoomMode,
  modifiers: { ctrlKey: boolean; metaKey: boolean },
): boolean {
  return mode === "always" || modifiers.ctrlKey || modifiers.metaKey;
}

export interface PanOffset {
  x: number;
  y: number;
}

export function panForZoom(
  scale: number,
  pan: PanOffset,
  nextScale: number,
  anchorX: number,
  anchorY: number,
): PanOffset {
  const ratio = nextScale / scale;
  return { x: anchorX - ratio * (anchorX - pan.x), y: anchorY - ratio * (anchorY - pan.y) };
}

export function actualSizeView(
  scale: number,
  pan: PanOffset,
  fitScale: number,
  containerW: number,
  containerH: number,
): { scale: number; pan: PanOffset } {
  const next = clampZoomPanScale(ACTUAL_SIZE_SCALE, fitScale);
  return { scale: next, pan: panForZoom(scale, pan, next, containerW / 2, containerH / 2) };
}
