export interface PixelCoord {
  x: number;
  y: number;
}

export interface ViewerTransform {
  scale: number;
  x: number;
  y: number;
}

export interface ViewerRect {
  left: number;
  top: number;
}

export function screenToImagePixel(
  clientX: number,
  clientY: number,
  containerRect: ViewerRect,
  transform: ViewerTransform,
  renderW: number,
  renderH: number,
  fitsW: number,
  fitsH: number,
): PixelCoord | null {
  const imgX = (clientX - containerRect.left - transform.x) / transform.scale;
  const imgY = (clientY - containerRect.top - transform.y) / transform.scale;
  if (imgX < 0 || imgX >= renderW || imgY < 0 || imgY >= renderH) return null;
  return {
    x: Math.floor((imgX / renderW) * fitsW),
    y: Math.floor((imgY / renderH) * fitsH),
  };
}

export interface ImageCoord {
  x: number;
  y: number;
}

export function screenToImageCoord(
  clientX: number,
  clientY: number,
  containerRect: ViewerRect,
  transform: ViewerTransform,
  renderW: number,
  renderH: number,
  fitsW: number,
  fitsH: number,
): ImageCoord | null {
  const imgX = (clientX - containerRect.left - transform.x) / transform.scale;
  const imgY = (clientY - containerRect.top - transform.y) / transform.scale;
  if (imgX < 0 || imgX >= renderW || imgY < 0 || imgY >= renderH) return null;
  return {
    x: (imgX / renderW) * fitsW,
    y: (imgY / renderH) * fitsH,
  };
}

export function imageCoordToScreen(
  x: number,
  y: number,
  transform: ViewerTransform,
  renderW: number,
  renderH: number,
  fitsW: number,
  fitsH: number,
): { x: number; y: number } {
  return {
    x: (x / fitsW) * renderW * transform.scale + transform.x,
    y: (y / fitsH) * renderH * transform.scale + transform.y,
  };
}

export function edgeToCentre(c: number): number {
  return c - 0.5;
}

export function centreToEdge(c: number): number {
  return c + 0.5;
}

export function screenPxPerImagePx(transform: ViewerTransform, renderW: number, fitsW: number): number {
  return (transform.scale * renderW) / fitsW;
}
