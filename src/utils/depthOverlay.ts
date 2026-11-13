import type { DepthAperture } from "../shared/types/depth";
import type { OverlayPainter } from "./overlayStore";
import type { Pt } from "./regionGeometry";

export const DEPTH_LAYER_ID = "depth-apertures";
export const DEPTH_LAYER_KIND = "depth-apertures";

const USABLE_COLOR = "#7dd3fc";
const UNUSABLE_COLOR = "#fbbf24";
const LINE_WIDTH = 1;
const SOLID: number[] = [];
const UNUSABLE_DASH: number[] = [3, 3];
const MIN_SCREEN_RADIUS_PX = 0.5;

export interface DepthCircle {
  cx: number;
  cy: number;
  r: number;
  usable: boolean;
}

export function depthCircles(
  apertures: readonly DepthAperture[],
  radiusPx: number,
  toScreen: (p: Pt) => Pt,
  screenPxPerImagePx: number,
  width: number,
  height: number,
): DepthCircle[] {
  const scaled = radiusPx * screenPxPerImagePx;
  const r = Number.isFinite(scaled) ? Math.max(scaled, MIN_SCREEN_RADIUS_PX) : MIN_SCREEN_RADIUS_PX;
  const circles: DepthCircle[] = [];
  for (const aperture of apertures) {
    const centre = toScreen({ x: aperture.x, y: aperture.y });
    if (!Number.isFinite(centre.x) || !Number.isFinite(centre.y)) continue;
    if (centre.x + r < 0 || centre.y + r < 0 || centre.x - r > width || centre.y - r > height) continue;
    circles.push({ cx: centre.x, cy: centre.y, r, usable: aperture.usable });
  }
  return circles;
}

export function createDepthPainter(apertures: readonly DepthAperture[], radiusPx: number): OverlayPainter {
  return ({ ctx, toScreen, width, height, screenPxPerImagePx }) => {
    const circles = depthCircles(apertures, radiusPx, toScreen, screenPxPerImagePx, width, height);
    ctx.save();
    ctx.lineWidth = LINE_WIDTH;
    for (const circle of circles) {
      ctx.setLineDash(circle.usable ? SOLID : UNUSABLE_DASH);
      ctx.strokeStyle = circle.usable ? USABLE_COLOR : UNUSABLE_COLOR;
      ctx.beginPath();
      ctx.arc(circle.cx, circle.cy, circle.r, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.restore();
  };
}
