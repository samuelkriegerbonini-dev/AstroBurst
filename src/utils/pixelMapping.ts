export interface PixelCoord {
  x: number;
  y: number;
}

export interface ViewerTransform {
  scale: number;
  x: number;
  y: number;
}

export function screenToImagePixel(
  clientX: number,
  clientY: number,
  containerRect: DOMRect,
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
