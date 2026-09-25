import type { Pt } from "../../../utils/regionGeometry";
import type { OverlayPainter } from "../../../utils/overlayStore";

export const APERTURE_LAYER_ID = "photometry-apertures";
export const APERTURE_LAYER_KIND = "apertures";

const APERTURE_COLOR_FALLBACK = "#fbbf24";
const HIGHLIGHT_COLOR = "#ffffff";
const LABEL_COLOR_FALLBACK = "#e3e5e8";
const HALO_COLOR = "rgba(9, 9, 11, 0.85)";
const LABEL_FONT = "10px 'JetBrains Mono', monospace";
const APERTURE_LINE_WIDTH = 1.2;
const HIGHLIGHT_LINE_WIDTH = 2;
const ANNULUS_LINE_WIDTH = 0.8;
const ANNULUS_DASH: number[] = [3, 3];
const HALO_WIDTH = 3;
const LABEL_GAP_PX = 3;
const MIN_SCREEN_RADIUS_PX = 0.5;

export interface ApertureMarker {
  x: number;
  y: number;
  rAp: number;
  skyIn: number;
  skyOut: number;
  label: string;
}

export interface AperturePainterOptions {
  apertures: ApertureMarker[];
  highlightIndex: number | null;
}

export interface ApertureScreenGeometry {
  cx: number;
  cy: number;
  rAp: number;
  skyIn: number;
  skyOut: number;
  labelX: number;
  labelY: number;
}

function cssToken(name: string, fallback: string): string {
  if (typeof document === "undefined" || typeof getComputedStyle !== "function") return fallback;
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

function screenRadius(imagePx: number, screenPxPerImagePx: number): number {
  const r = imagePx * screenPxPerImagePx;
  return Number.isFinite(r) && r > MIN_SCREEN_RADIUS_PX ? r : MIN_SCREEN_RADIUS_PX;
}

export function apertureScreenGeometry(
  ap: ApertureMarker,
  toScreen: (p: Pt) => Pt,
  screenPxPerImagePx: number,
): ApertureScreenGeometry {
  const centre = toScreen({ x: ap.x, y: ap.y });
  const rAp = screenRadius(ap.rAp, screenPxPerImagePx);
  const skyIn = screenRadius(ap.skyIn, screenPxPerImagePx);
  const skyOut = screenRadius(ap.skyOut, screenPxPerImagePx);
  return {
    cx: centre.x,
    cy: centre.y,
    rAp,
    skyIn,
    skyOut,
    labelX: centre.x + rAp + LABEL_GAP_PX,
    labelY: centre.y,
  };
}

export function apertureOffCanvas(g: ApertureScreenGeometry, width: number, height: number): boolean {
  const reach = Math.max(g.rAp, g.skyIn, g.skyOut);
  return (
    !Number.isFinite(g.cx) ||
    !Number.isFinite(g.cy) ||
    g.cx + reach < 0 ||
    g.cy + reach < 0 ||
    g.cx - reach > width ||
    g.cy - reach > height
  );
}

export function createAperturePainter(options: AperturePainterOptions): OverlayPainter {
  const { apertures, highlightIndex } = options;
  return ({ ctx, toScreen, width, height, screenPxPerImagePx }) => {
    const apertureColor = cssToken("--ab-amber", APERTURE_COLOR_FALLBACK);
    const labelColor = cssToken("--ab-text-1", LABEL_COLOR_FALLBACK);
    ctx.save();
    ctx.font = LABEL_FONT;
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    for (let i = 0; i < apertures.length; i++) {
      const ap = apertures[i];
      const g = apertureScreenGeometry(ap, toScreen, screenPxPerImagePx);
      if (apertureOffCanvas(g, width, height)) continue;
      const highlighted = i === highlightIndex;
      const color = highlighted ? HIGHLIGHT_COLOR : apertureColor;

      ctx.setLineDash([]);
      ctx.strokeStyle = color;
      ctx.lineWidth = highlighted ? HIGHLIGHT_LINE_WIDTH : APERTURE_LINE_WIDTH;
      ctx.beginPath();
      ctx.arc(g.cx, g.cy, g.rAp, 0, Math.PI * 2);
      ctx.stroke();

      ctx.setLineDash(ANNULUS_DASH);
      ctx.lineWidth = highlighted ? APERTURE_LINE_WIDTH : ANNULUS_LINE_WIDTH;
      ctx.beginPath();
      ctx.arc(g.cx, g.cy, g.skyIn, 0, Math.PI * 2);
      ctx.stroke();
      ctx.beginPath();
      ctx.arc(g.cx, g.cy, g.skyOut, 0, Math.PI * 2);
      ctx.stroke();
      ctx.setLineDash([]);

      if (!ap.label) continue;
      ctx.lineWidth = HALO_WIDTH;
      ctx.strokeStyle = HALO_COLOR;
      ctx.strokeText(ap.label, g.labelX, g.labelY);
      ctx.fillStyle = highlighted ? HIGHLIGHT_COLOR : labelColor;
      ctx.fillText(ap.label, g.labelX, g.labelY);
    }
    ctx.setLineDash([]);
    ctx.restore();
  };
}
