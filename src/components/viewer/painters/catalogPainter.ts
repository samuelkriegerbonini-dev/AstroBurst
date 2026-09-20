import type { PlacedCatalogRow } from "../../../shared/types/catalog";
import type { OverlayPainter } from "../../../utils/overlayStore";

export const CATALOG_LAYER_ID = "gaia-catalog";
export const CATALOG_LAYER_KIND = "catalog";

const CATALOG_COLOR_FALLBACK = "#22d3ee";
const MATCHED_COLOR = "#fbbf24";
const HIGHLIGHT_COLOR = "#ffffff";
const LABEL_COLOR_FALLBACK = "#e3e5e8";
const HALO_COLOR = "rgba(9, 9, 11, 0.85)";
const LABEL_FONT = "10px 'JetBrains Mono', monospace";
const MARKER_LINE_WIDTH = 1.2;
const HIGHLIGHT_LINE_WIDTH = 2;
const HIGHLIGHT_EXTRA_PX = 3;
const HALO_WIDTH = 3;
const LABEL_GAP_PX = 3;
const MIN_RADIUS_PX = 3;
const MAX_RADIUS_PX = 9;
const BRIGHT_MAG = 8;
const FAINT_MAG = 18;
const LABEL_MAX_CHARS = 8;
const LABEL_TAIL_CHARS = 6;

export interface CatalogPainterOptions {
  rows: PlacedCatalogRow[];
  matchedIds: ReadonlySet<string>;
  labels: boolean;
  highlightId: string | null;
}

function cssToken(name: string, fallback: string): string {
  if (typeof document === "undefined" || typeof getComputedStyle !== "function") return fallback;
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

export function markerRadiusPx(g: number | null): number {
  if (g === null || !Number.isFinite(g)) return MIN_RADIUS_PX;
  const t = (g - BRIGHT_MAG) / (FAINT_MAG - BRIGHT_MAG);
  const clamped = Math.min(1, Math.max(0, t));
  return MAX_RADIUS_PX - clamped * (MAX_RADIUS_PX - MIN_RADIUS_PX);
}

export function shortLabel(id: string): string {
  return id.length <= LABEL_MAX_CHARS ? id : `…${id.slice(-LABEL_TAIL_CHARS)}`;
}

export function createCatalogPainter(options: CatalogPainterOptions): OverlayPainter {
  const { rows, matchedIds, labels, highlightId } = options;
  return ({ ctx, toScreen, width, height }) => {
    const catalogColor = cssToken("--ab-cyan", CATALOG_COLOR_FALLBACK);
    const labelColor = cssToken("--ab-text-1", LABEL_COLOR_FALLBACK);
    ctx.save();
    ctx.setLineDash([]);
    ctx.font = LABEL_FONT;
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    for (const row of rows) {
      if (row.x === null || row.y === null) continue;
      const p = toScreen({ x: row.x, y: row.y });
      const highlighted = row.id === highlightId;
      const radius = markerRadiusPx(row.g) + (highlighted ? HIGHLIGHT_EXTRA_PX : 0);
      if (p.x < -radius || p.y < -radius || p.x > width + radius || p.y > height + radius) continue;
      const matched = matchedIds.has(row.id);
      ctx.strokeStyle = highlighted ? HIGHLIGHT_COLOR : matched ? MATCHED_COLOR : catalogColor;
      ctx.lineWidth = highlighted ? HIGHLIGHT_LINE_WIDTH : MARKER_LINE_WIDTH;
      ctx.beginPath();
      ctx.arc(p.x, p.y, radius, 0, Math.PI * 2);
      ctx.stroke();
      if (!labels && !highlighted) continue;
      const text = shortLabel(row.id);
      const tx = p.x + radius + LABEL_GAP_PX;
      ctx.lineWidth = HALO_WIDTH;
      ctx.strokeStyle = HALO_COLOR;
      ctx.strokeText(text, tx, p.y);
      ctx.fillStyle = highlighted ? HIGHLIGHT_COLOR : labelColor;
      ctx.fillText(text, tx, p.y);
    }
    ctx.restore();
  };
}
