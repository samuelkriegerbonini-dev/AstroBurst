import type { OverlayPainter } from "../../../utils/overlayStore";
import type { Pt } from "../../../utils/regionGeometry";

export const TARGET_LAYER_ID = "targets";
export const TARGET_LAYER_KIND = "targets";

const TARGET_COLOR_FALLBACK = "#22d3ee";
const HIGHLIGHT_COLOR = "#ffffff";
const LABEL_COLOR_FALLBACK = "#e3e5e8";
const HALO_COLOR = "rgba(9, 9, 11, 0.85)";
const LABEL_FONT = "10px 'JetBrains Mono', monospace";
const DIAMOND_HALF_PX = 5;
const HIGHLIGHT_EXTRA_PX = 3;
const MARKER_LINE_WIDTH = 1.2;
const HIGHLIGHT_LINE_WIDTH = 2;
const HALO_WIDTH = 3;
const LABEL_GAP_PX = 3;

export interface PlacedTarget {
  x: number;
  y: number;
  label: string;
}

export interface TargetPainterOptions {
  targets: PlacedTarget[];
  highlightIndex: number | null;
}

function cssToken(name: string, fallback: string): string {
  if (typeof document === "undefined" || typeof getComputedStyle !== "function") return fallback;
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

export function diamondVertices(centre: Pt, half: number): [Pt, Pt, Pt, Pt] {
  return [
    { x: centre.x, y: centre.y - half },
    { x: centre.x + half, y: centre.y },
    { x: centre.x, y: centre.y + half },
    { x: centre.x - half, y: centre.y },
  ];
}

export function isOffscreen(p: Pt, width: number, height: number, margin: number): boolean {
  return !Number.isFinite(p.x) || !Number.isFinite(p.y) || p.x < -margin || p.y < -margin || p.x > width + margin || p.y > height + margin;
}

export function targetMarkerHalf(highlighted: boolean): number {
  return DIAMOND_HALF_PX + (highlighted ? HIGHLIGHT_EXTRA_PX : 0);
}

export function createTargetPainter(options: TargetPainterOptions): OverlayPainter {
  const { targets, highlightIndex } = options;
  return ({ ctx, toScreen, width, height }) => {
    const markerColor = cssToken("--ab-cyan", TARGET_COLOR_FALLBACK);
    const labelColor = cssToken("--ab-text-1", LABEL_COLOR_FALLBACK);
    ctx.save();
    ctx.setLineDash([]);
    ctx.font = LABEL_FONT;
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    for (let i = 0; i < targets.length; i++) {
      const target = targets[i];
      const highlighted = i === highlightIndex;
      const half = targetMarkerHalf(highlighted);
      const p = toScreen({ x: target.x, y: target.y });
      if (isOffscreen(p, width, height, half)) continue;
      const [top, right, bottom, left] = diamondVertices(p, half);
      ctx.strokeStyle = highlighted ? HIGHLIGHT_COLOR : markerColor;
      ctx.lineWidth = highlighted ? HIGHLIGHT_LINE_WIDTH : MARKER_LINE_WIDTH;
      ctx.beginPath();
      ctx.moveTo(top.x, top.y);
      ctx.lineTo(right.x, right.y);
      ctx.lineTo(bottom.x, bottom.y);
      ctx.lineTo(left.x, left.y);
      ctx.closePath();
      ctx.stroke();
      if (!target.label) continue;
      const tx = p.x + half + LABEL_GAP_PX;
      ctx.lineWidth = HALO_WIDTH;
      ctx.strokeStyle = HALO_COLOR;
      ctx.strokeText(target.label, tx, p.y);
      ctx.fillStyle = highlighted ? HIGHLIGHT_COLOR : labelColor;
      ctx.fillText(target.label, tx, p.y);
    }
    ctx.restore();
  };
}
