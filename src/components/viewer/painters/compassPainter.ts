import { compassArrows, niceScaleBarLength, scaleBarPixels, screenDirection } from "../../../utils/compass";
import type { OverlayPainter } from "../../../utils/overlayStore";
import type { Pt } from "../../../utils/regionGeometry";

export const COMPASS_LAYER_ID = "compass";
export const COMPASS_LAYER_KIND = "compass";

const COLOR_FALLBACK = "#e3e5e8";
const HALO_COLOR = "rgba(9, 9, 11, 0.85)";
const LABEL_FONT = "10px 'JetBrains Mono', monospace";
const MARGIN_PX = 16;
const ARROW_LENGTH_PX = 28;
const ARROW_HEAD_PX = 5;
const LABEL_GAP_PX = 8;
const LINE_WIDTH = 1.5;
const HALO_WIDTH = 3;
const MAX_BAR_PX = 120;
const MIN_BAR_PX = 16;
const BAR_TICK_PX = 4;
const BAR_LABEL_GAP_PX = 4;
const PROBE_IMAGE_PX = 10;

export interface CompassPainterOptions {
  northVec: [number, number];
  eastVec: [number, number];
  pixelScaleArcsec: number;
  imageCentre: Pt;
}

function cssToken(name: string, fallback: string): string {
  if (typeof document === "undefined" || typeof getComputedStyle !== "function") return fallback;
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

function haloText(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, color: string): void {
  ctx.lineWidth = HALO_WIDTH;
  ctx.strokeStyle = HALO_COLOR;
  ctx.strokeText(text, x, y);
  ctx.fillStyle = color;
  ctx.fillText(text, x, y);
}

function drawArrow(ctx: CanvasRenderingContext2D, from: Pt, to: Pt): void {
  const dir = screenDirection(from, to);
  if (!dir) return;
  ctx.beginPath();
  ctx.moveTo(from.x, from.y);
  ctx.lineTo(to.x, to.y);
  ctx.moveTo(to.x, to.y);
  ctx.lineTo(to.x - dir.x * ARROW_HEAD_PX - dir.y * ARROW_HEAD_PX, to.y - dir.y * ARROW_HEAD_PX + dir.x * ARROW_HEAD_PX);
  ctx.moveTo(to.x, to.y);
  ctx.lineTo(to.x - dir.x * ARROW_HEAD_PX + dir.y * ARROW_HEAD_PX, to.y - dir.y * ARROW_HEAD_PX - dir.x * ARROW_HEAD_PX);
  ctx.stroke();
}

export function createCompassPainter(options: CompassPainterOptions): OverlayPainter {
  const { northVec, eastVec, pixelScaleArcsec, imageCentre } = options;
  return ({ ctx, toScreen, height, screenPxPerImagePx }) => {
    const color = cssToken("--ab-text-1", COLOR_FALLBACK);
    ctx.save();
    ctx.setLineDash([]);
    ctx.font = LABEL_FONT;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.lineWidth = LINE_WIDTH;
    ctx.strokeStyle = color;

    const base = toScreen(imageCentre);
    const northDir = screenDirection(
      base,
      toScreen({ x: imageCentre.x + northVec[0] * PROBE_IMAGE_PX, y: imageCentre.y + northVec[1] * PROBE_IMAGE_PX }),
    );
    const eastDir = screenDirection(
      base,
      toScreen({ x: imageCentre.x + eastVec[0] * PROBE_IMAGE_PX, y: imageCentre.y + eastVec[1] * PROBE_IMAGE_PX }),
    );

    const barY = height - MARGIN_PX;
    const origin = { x: MARGIN_PX + ARROW_LENGTH_PX + LABEL_GAP_PX, y: barY - MARGIN_PX - ARROW_LENGTH_PX - LABEL_GAP_PX };
    if (northDir && eastDir) {
      const arrows = compassArrows(origin, northDir, eastDir, ARROW_LENGTH_PX);
      ctx.strokeStyle = color;
      ctx.lineWidth = LINE_WIDTH;
      drawArrow(ctx, origin, arrows.northTip);
      drawArrow(ctx, origin, arrows.eastTip);
      haloText(ctx, "N", arrows.northTip.x + northDir.x * LABEL_GAP_PX, arrows.northTip.y + northDir.y * LABEL_GAP_PX, color);
      haloText(ctx, "E", arrows.eastTip.x + eastDir.x * LABEL_GAP_PX, arrows.eastTip.y + eastDir.y * LABEL_GAP_PX, color);
    }

    if (Number.isFinite(screenPxPerImagePx) && screenPxPerImagePx > 0) {
      const maxArcsec = (MAX_BAR_PX / screenPxPerImagePx) * pixelScaleArcsec;
      const nice = niceScaleBarLength(maxArcsec);
      const barPx = nice ? scaleBarPixels(nice.arcsec, pixelScaleArcsec, screenPxPerImagePx) : 0;
      if (nice && barPx >= MIN_BAR_PX) {
        const x0 = MARGIN_PX;
        const x1 = MARGIN_PX + barPx;
        ctx.strokeStyle = HALO_COLOR;
        ctx.lineWidth = LINE_WIDTH + HALO_WIDTH;
        ctx.beginPath();
        ctx.moveTo(x0, barY);
        ctx.lineTo(x1, barY);
        ctx.stroke();
        ctx.strokeStyle = color;
        ctx.lineWidth = LINE_WIDTH;
        ctx.beginPath();
        ctx.moveTo(x0, barY);
        ctx.lineTo(x1, barY);
        ctx.moveTo(x0, barY - BAR_TICK_PX);
        ctx.lineTo(x0, barY + BAR_TICK_PX);
        ctx.moveTo(x1, barY - BAR_TICK_PX);
        ctx.lineTo(x1, barY + BAR_TICK_PX);
        ctx.stroke();
        ctx.textBaseline = "bottom";
        haloText(ctx, nice.label, (x0 + x1) / 2, barY - BAR_TICK_PX - BAR_LABEL_GAP_PX, color);
      }
    }
    ctx.restore();
  };
}
