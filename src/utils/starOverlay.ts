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
