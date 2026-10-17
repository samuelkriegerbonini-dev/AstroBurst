import type { Pt } from "../../../utils/regionGeometry";
import type { OverlayLayer, OverlayPainter } from "../../../utils/overlayStore";

export const FOOTPRINT_LAYER_ID = "mast-footprints";
export const FOOTPRINT_LAYER_KIND = "footprints";

const FOOTPRINT_COLOR_FALLBACK = "#a78bfa";
const HIGHLIGHT_COLOR = "#ffffff";
const HALO_COLOR = "rgba(9, 9, 11, 0.85)";
const LABEL_FONT = "10px 'JetBrains Mono', monospace";
const LINE_WIDTH = 1.2;
const HIGHLIGHT_LINE_WIDTH = 2.4;
const HALO_WIDTH = 3;
const LABEL_GAP_PX = 4;
const CULL_MARGIN_PX = 2;

export interface FootprintOutline {
  datasetId: string;
  polygons: readonly (readonly [number, number])[][];
}

export interface FootprintPainterOptions {
  footprints: readonly FootprintOutline[];
  highlight: string | null;
}

export interface FootprintOverlayLayer extends OverlayLayer {
  footprints: number;
  highlight: string | null;
}

function cssToken(name: string, fallback: string): string {
  if (typeof document === "undefined" || typeof getComputedStyle !== "function") return fallback;
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

export function screenPath(polygon: readonly (readonly [number, number])[], toScreen: (p: Pt) => Pt): Pt[] {
  return polygon.map(([x, y]) => toScreen({ x, y }));
}

export function pathOffCanvas(path: readonly Pt[], width: number, height: number): boolean {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const p of path) {
    if (!Number.isFinite(p.x) || !Number.isFinite(p.y)) return true;
    minX = Math.min(minX, p.x);
    minY = Math.min(minY, p.y);
    maxX = Math.max(maxX, p.x);
    maxY = Math.max(maxY, p.y);
  }
  return path.length === 0 || maxX < -CULL_MARGIN_PX || maxY < -CULL_MARGIN_PX || minX > width + CULL_MARGIN_PX || minY > height + CULL_MARGIN_PX;
}

function tracePath(ctx: CanvasRenderingContext2D, path: readonly Pt[]): void {
  ctx.beginPath();
  ctx.moveTo(path[0].x, path[0].y);
  for (let i = 1; i < path.length; i++) ctx.lineTo(path[i].x, path[i].y);
  ctx.closePath();
  ctx.stroke();
}

function topLeft(paths: readonly Pt[][]): Pt {
  let best = paths[0][0];
  for (const path of paths) for (const p of path) if (p.y < best.y || (p.y === best.y && p.x < best.x)) best = p;
  return best;
}

export function createFootprintPainter({ footprints, highlight }: FootprintPainterOptions): OverlayPainter {
  return ({ ctx, toScreen, width, height }) => {
    const color = cssToken("--ab-violet", FOOTPRINT_COLOR_FALLBACK);
    const drawn = (footprint: FootprintOutline) =>
      footprint.polygons.map((polygon) => screenPath(polygon, toScreen)).filter((path) => !pathOffCanvas(path, width, height));
    ctx.save();
    ctx.setLineDash([]);
    ctx.strokeStyle = color;
    ctx.lineWidth = LINE_WIDTH;
    for (const footprint of footprints) {
      if (footprint.datasetId === highlight) continue;
      for (const path of drawn(footprint)) tracePath(ctx, path);
    }
    const paths = highlight === null ? [] : footprints.filter((f) => f.datasetId === highlight).flatMap(drawn);
    if (highlight !== null && paths.length > 0) {
      ctx.strokeStyle = HIGHLIGHT_COLOR;
      ctx.lineWidth = HIGHLIGHT_LINE_WIDTH;
      for (const path of paths) tracePath(ctx, path);
      const anchor = topLeft(paths);
      ctx.font = LABEL_FONT;
      ctx.textAlign = "left";
      ctx.textBaseline = "bottom";
      ctx.lineWidth = HALO_WIDTH;
      ctx.strokeStyle = HALO_COLOR;
      ctx.strokeText(highlight, anchor.x, anchor.y - LABEL_GAP_PX);
      ctx.fillStyle = HIGHLIGHT_COLOR;
      ctx.fillText(highlight, anchor.x, anchor.y - LABEL_GAP_PX);
    }
    ctx.restore();
  };
}

export function footprintLayer(footprints: readonly FootprintOutline[], highlight: string | null): FootprintOverlayLayer {
  const drawn = footprints.filter((f) => f.polygons.length > 0);
  const lit = highlight !== null && drawn.some((f) => f.datasetId === highlight) ? highlight : null;
  return {
    id: FOOTPRINT_LAYER_ID,
    kind: FOOTPRINT_LAYER_KIND,
    visible: true,
    paint: createFootprintPainter({ footprints: drawn, highlight: lit }),
    footprints: drawn.reduce((sum, f) => sum + f.polygons.length, 0),
    highlight: lit,
  };
}

export function isFootprintLayer(layer: OverlayLayer): layer is FootprintOverlayLayer {
  const candidate = layer as Partial<FootprintOverlayLayer>;
  return layer.id === FOOTPRINT_LAYER_ID && typeof candidate.footprints === "number";
}
