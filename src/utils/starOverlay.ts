import { centreToEdge } from "./pixelMapping";

export interface CanvasFit {
  scale: number;
  ox: number;
  oy: number;
}

export function fitImageToCanvas(canvasW: number, canvasH: number, imageW: number, imageH: number): CanvasFit {
  const iw = imageW > 0 ? imageW : 1;
  const ih = imageH > 0 ? imageH : 1;
  const scale = Math.min(canvasW / iw, canvasH / ih);
  return { scale, ox: (canvasW - iw * scale) / 2, oy: (canvasH - ih * scale) / 2 };
}

export function imagePointToCanvas(x: number, y: number, fit: CanvasFit): { x: number; y: number } {
  return { x: fit.ox + centreToEdge(x) * fit.scale, y: fit.oy + centreToEdge(y) * fit.scale };
}

export function fitsPixelToCanvas(x: number, y: number, fit: CanvasFit): { x: number; y: number } {
  return imagePointToCanvas(x - 1, y - 1, fit);
}

export const VIEW_SCALE_ATTRIBUTE = "data-view-scale";

export function viewScaleAttributes(scale: number): Record<typeof VIEW_SCALE_ATTRIBUTE, string> {
  return { [VIEW_SCALE_ATTRIBUTE]: String(Math.round(scale * 1000) / 1000) };
}

export function overlayStrokeScale(screenWidth: number, backingWidth: number, publishesViewScale: boolean): number {
  if (!publishesViewScale) return 1;
  if (!(screenWidth > 0) || !(backingWidth > 0) || !Number.isFinite(screenWidth) || !Number.isFinite(backingWidth)) return 1;
  return Math.max(1, backingWidth / screenWidth);
}

export interface OverlayLayersInput {
  canvasMounted: boolean;
  showStars: boolean;
  starCount: number;
  showAnnotations: boolean;
  annotationCount: number;
  annotationsOnView: boolean;
}

export interface OverlayLayers {
  drawStars: boolean;
  drawAnnotations: boolean;
  unavailable: boolean;
}

export function overlayLayers(input: OverlayLayersInput): OverlayLayers {
  const annotationsApply = input.annotationsOnView && input.annotationCount > 0;
  return {
    drawStars: input.showStars && input.starCount > 0,
    drawAnnotations: input.showAnnotations && annotationsApply,
    unavailable: !input.canvasMounted && (input.starCount > 0 || annotationsApply),
  };
}
