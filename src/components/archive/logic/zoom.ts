export interface Size {
  w: number;
  h: number;
}

export interface Point {
  x: number;
  y: number;
}

export interface ZoomView {
  scale: number;
  x: number;
  y: number;
}

export const ZOOM_STEP = 1.25;
export const LARGE_EDGE_PX = 4096;

export function maxScale(source: Size): number {
  const edge = Math.max(source.w, source.h);
  return edge > 0 ? Math.min(1, LARGE_EDGE_PX / edge) : 1;
}

export function fitScale(img: Size, box: Size, max: number): number {
  if (img.w <= 0 || img.h <= 0 || box.w <= 0 || box.h <= 0) return max;
  return Math.min(box.w / img.w, box.h / img.h, max);
}

function axis(offset: number, length: number, boxLength: number): number {
  if (length <= boxLength) return (boxLength - length) / 2;
  return Math.min(0, Math.max(boxLength - length, offset));
}

export function clampView(view: ZoomView, img: Size, box: Size, max: number): ZoomView {
  const fit = fitScale(img, box, max);
  const scale = Math.min(max, Math.max(fit, view.scale));
  return { scale, x: axis(view.x, img.w * scale, box.w), y: axis(view.y, img.h * scale, box.h) };
}

export function fitView(img: Size, box: Size, max: number): ZoomView {
  return clampView({ scale: fitScale(img, box, max), x: 0, y: 0 }, img, box, max);
}

export function zoomAt(view: ZoomView, factor: number, point: Point, img: Size, box: Size, max: number): ZoomView {
  const fit = fitScale(img, box, max);
  const scale = Math.min(max, Math.max(fit, view.scale * factor));
  const ix = (point.x - view.x) / view.scale;
  const iy = (point.y - view.y) / view.scale;
  return clampView({ scale, x: point.x - ix * scale, y: point.y - iy * scale }, img, box, max);
}

export function zoomTo(view: ZoomView, scale: number, img: Size, box: Size, max: number): ZoomView {
  return zoomAt(view, scale / view.scale, { x: box.w / 2, y: box.h / 2 }, img, box, max);
}

export function panBy(view: ZoomView, dx: number, dy: number, img: Size, box: Size, max: number): ZoomView {
  return clampView({ ...view, x: view.x + dx, y: view.y + dy }, img, box, max);
}

export function isFit(view: ZoomView, img: Size, box: Size, max: number): boolean {
  return Math.abs(view.scale - fitScale(img, box, max)) < 1e-9;
}

export function arcsecPerScreenPx(sourceArcsecPerPx: number, scale: number): number {
  return scale > 0 ? sourceArcsecPerPx / scale : Number.POSITIVE_INFINITY;
}
