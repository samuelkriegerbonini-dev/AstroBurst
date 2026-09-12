import {
  centreToEdge,
  edgeToCentre,
  imageCoordToScreen,
  screenPxPerImagePx,
  screenToImageCoord,
  type ViewerRect,
  type ViewerTransform,
} from "./pixelMapping";
import type { Pt } from "./regionGeometry";

export interface RegionMapping {
  transform: ViewerTransform;
  renderW: number;
  renderH: number;
  fitsW: number;
  fitsH: number;
}

export function resolveRegionHost(container: HTMLElement | null, canvas: HTMLElement | null): HTMLElement | null {
  if (!canvas) return null;
  return container ?? canvas.parentElement ?? null;
}

export function isRegionMappingUsable(m: RegionMapping): boolean {
  const t = m.transform;
  return (
    Number.isFinite(t.scale) &&
    t.scale > 0 &&
    Number.isFinite(t.x) &&
    Number.isFinite(t.y) &&
    m.renderW > 0 &&
    m.renderH > 0 &&
    m.fitsW > 0 &&
    m.fitsH > 0
  );
}

export function regionPointToScreen(p: Pt, m: RegionMapping): Pt {
  return imageCoordToScreen(
    centreToEdge(p.x),
    centreToEdge(p.y),
    m.transform,
    m.renderW,
    m.renderH,
    m.fitsW,
    m.fitsH,
  );
}

export function screenToRegionPoint(clientX: number, clientY: number, rect: ViewerRect, m: RegionMapping): Pt | null {
  if (!isRegionMappingUsable(m)) return null;
  const c = screenToImageCoord(clientX, clientY, rect, m.transform, m.renderW, m.renderH, m.fitsW, m.fitsH);
  return c ? { x: edgeToCentre(c.x), y: edgeToCentre(c.y) } : null;
}

export function regionToleranceImagePx(screenPx: number, m: RegionMapping): number {
  const perImagePx = screenPxPerImagePx(m.transform, m.renderW, m.fitsW);
  return perImagePx > 0 && Number.isFinite(perImagePx) ? screenPx / perImagePx : screenPx;
}
