import type { GridEdge, WcsGrid } from "../../../shared/types/astrometry";
import type { OverlayPainter } from "../../../utils/overlayStore";
import type { Pt } from "../../../utils/regionGeometry";

export const GRID_LAYER_ID = "wcs-grid";
export const GRID_LAYER_KIND = "grid";

const LINE_COLOR_FALLBACK = "#14b8a6";
const LABEL_COLOR_FALLBACK = "#e3e5e8";
const HALO_COLOR = "rgba(9, 9, 11, 0.85)";
const LINE_ALPHA = 0.55;
const LINE_WIDTH = 1;
const HALO_WIDTH = 3;
const LABEL_FONT = "10px 'JetBrains Mono', monospace";
export const LABEL_HEIGHT = 10;
export const LABEL_PAD = 4;
const LABEL_GAP = 2;

export interface LabelBox {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export interface PlacedLabel {
  text: string;
  x: number;
  y: number;
  align: CanvasTextAlign;
  baseline: CanvasTextBaseline;
  box: LabelBox;
}

export interface ViewportCrossing {
  p: Pt;
  edge: GridEdge;
}

function cssToken(name: string, fallback: string): string {
  if (typeof document === "undefined" || typeof getComputedStyle !== "function") return fallback;
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

const VIEWPORT_TOLERANCE = 1;

function insideViewport(p: Pt, width: number, height: number): boolean {
  return (
    p.x >= -VIEWPORT_TOLERANCE &&
    p.x <= width + VIEWPORT_TOLERANCE &&
    p.y >= -VIEWPORT_TOLERANCE &&
    p.y <= height + VIEWPORT_TOLERANCE
  );
}

function clipEntry(a: Pt, b: Pt, width: number, height: number): ViewportCrossing | null {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const bounds: [number, number, GridEdge][] = [
    [-dx, a.x, "left"],
    [dx, width - a.x, "right"],
    [-dy, a.y, "top"],
    [dy, height - a.y, "bottom"],
  ];
  let t0 = 0;
  let t1 = 1;
  let edge: GridEdge | null = null;
  for (const [p, q, e] of bounds) {
    if (p === 0) {
      if (q < 0) return null;
      continue;
    }
    const t = q / p;
    if (p < 0) {
      if (t > t1) return null;
      if (t > t0) {
        t0 = t;
        edge = e;
      }
    } else {
      if (t < t0) return null;
      if (t < t1) t1 = t;
    }
  }
  if (edge === null) return null;
  return { p: { x: a.x + dx * t0, y: a.y + dy * t0 }, edge };
}

export function viewportCrossings(points: Pt[], width: number, height: number): ViewportCrossing[] {
  const found: ViewportCrossing[] = [];
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1];
    const b = points[i];
    const entry = clipEntry(a, b, width, height);
    if (entry) found.push(entry);
    const exit = clipEntry(b, a, width, height);
    if (exit) found.push(exit);
  }
  return found;
}

export function viewportCrossing(points: Pt[], width: number, height: number): ViewportCrossing | null {
  return viewportCrossings(points, width, height)[0] ?? null;
}

function boxesOverlap(a: LabelBox, b: LabelBox): boolean {
  return a.x0 < b.x1 + LABEL_GAP && b.x0 < a.x1 + LABEL_GAP && a.y0 < b.y1 + LABEL_GAP && b.y0 < a.y1 + LABEL_GAP;
}

function placeAt(text: string, p: Pt, edge: GridEdge, textWidth: number): PlacedLabel {
  switch (edge) {
    case "left":
      return {
        text, x: p.x + LABEL_PAD, y: p.y, align: "left", baseline: "middle",
        box: { x0: p.x + LABEL_PAD, y0: p.y - LABEL_HEIGHT / 2, x1: p.x + LABEL_PAD + textWidth, y1: p.y + LABEL_HEIGHT / 2 },
      };
    case "right":
      return {
        text, x: p.x - LABEL_PAD, y: p.y, align: "right", baseline: "middle",
        box: { x0: p.x - LABEL_PAD - textWidth, y0: p.y - LABEL_HEIGHT / 2, x1: p.x - LABEL_PAD, y1: p.y + LABEL_HEIGHT / 2 },
      };
    case "top":
      return {
        text, x: p.x, y: p.y + LABEL_PAD, align: "center", baseline: "top",
        box: { x0: p.x - textWidth / 2, y0: p.y + LABEL_PAD, x1: p.x + textWidth / 2, y1: p.y + LABEL_PAD + LABEL_HEIGHT },
      };
    default:
      return {
        text, x: p.x, y: p.y - LABEL_PAD, align: "center", baseline: "bottom",
        box: { x0: p.x - textWidth / 2, y0: p.y - LABEL_PAD - LABEL_HEIGHT, x1: p.x + textWidth / 2, y1: p.y - LABEL_PAD },
      };
  }
}

export function layoutGridLabels(
  grid: WcsGrid,
  toScreen: (p: Pt) => Pt,
  width: number,
  height: number,
  measure: (text: string) => number,
): PlacedLabel[] {
  const placed: PlacedLabel[] = [];
  for (const label of grid.labels) {
    const onEdge = toScreen({ x: label.x, y: label.y });
    let anchor: ViewportCrossing | null = insideViewport(onEdge, width, height) ? { p: onEdge, edge: label.edge } : null;
    if (!anchor) {
      for (const line of grid.lines) {
        if (line.kind !== label.kind || line.label !== label.text) continue;
        const crossings = viewportCrossings(line.points.map(([x, y]) => toScreen({ x, y })), width, height);
        anchor = crossings.find((c) => c.edge === label.edge) ?? crossings[0] ?? null;
        if (anchor) break;
      }
    }
    if (!anchor) continue;
    const candidate = placeAt(label.text, anchor.p, anchor.edge, measure(label.text));
    const { box } = candidate;
    if (box.x0 < 0 || box.y0 < 0 || box.x1 > width || box.y1 > height) continue;
    if (placed.some((other) => boxesOverlap(other.box, box))) continue;
    placed.push(candidate);
  }
  return placed;
}

function strokePolyline(ctx: CanvasRenderingContext2D, points: Pt[], width: number, height: number): void {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const p of points) {
    if (p.x < minX) minX = p.x;
    if (p.y < minY) minY = p.y;
    if (p.x > maxX) maxX = p.x;
    if (p.y > maxY) maxY = p.y;
  }
  if (maxX < 0 || maxY < 0 || minX > width || minY > height) return;
  ctx.beginPath();
  ctx.moveTo(points[0].x, points[0].y);
  for (let i = 1; i < points.length; i++) ctx.lineTo(points[i].x, points[i].y);
  ctx.stroke();
}

export function createGridPainter(grid: WcsGrid): OverlayPainter {
  return ({ ctx, toScreen, width, height }) => {
    const lineColor = cssToken("--ab-teal", LINE_COLOR_FALLBACK);
    const labelColor = cssToken("--ab-text-1", LABEL_COLOR_FALLBACK);
    ctx.save();
    ctx.setLineDash([]);
    ctx.lineJoin = "round";
    ctx.lineCap = "round";
    ctx.lineWidth = LINE_WIDTH;
    ctx.strokeStyle = lineColor;
    ctx.globalAlpha = LINE_ALPHA;
    for (const line of grid.lines) {
      if (line.points.length < 2) continue;
      strokePolyline(ctx, line.points.map(([x, y]) => toScreen({ x, y })), width, height);
    }
    ctx.globalAlpha = 1;
    ctx.font = LABEL_FONT;
    const labels = layoutGridLabels(grid, toScreen, width, height, (text) => ctx.measureText(text).width);
    for (const label of labels) {
      ctx.textAlign = label.align;
      ctx.textBaseline = label.baseline;
      ctx.lineWidth = HALO_WIDTH;
      ctx.strokeStyle = HALO_COLOR;
      ctx.strokeText(label.text, label.x, label.y);
      ctx.fillStyle = labelColor;
      ctx.fillText(label.text, label.x, label.y);
    }
    ctx.restore();
  };
}
