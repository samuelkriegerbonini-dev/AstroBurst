import type { ContourLevel } from "../../../shared/types/contours";
import type { OverlayPainter } from "../../../utils/overlayStore";
import {
  boundsOffScreen,
  levelColour,
  polylineBounds,
  SINGLE_CONTOUR_COLOUR,
  type ContourColourMode,
  type PolylineBounds,
} from "../../../utils/contourLevels";

export const CONTOUR_LAYER_ID = "contours";
export const CONTOUR_LAYER_KIND = "contours";

const HIGHLIGHT_COLOUR = "#ffffff";
const HIGHLIGHT_LINE_WIDTH = 2;
const TEAL_TOKEN = "--ab-teal";
const MIN_SCREEN_STEP_PX = 1;

export interface ContourPainterOptions {
  levels: ContourLevel[];
  hidden: ReadonlySet<number>;
  highlightIndex: number | null;
  colourMode: ContourColourMode;
  lineWidth: number;
}

function cssToken(name: string, fallback: string): string {
  if (typeof document === "undefined" || typeof getComputedStyle !== "function") return fallback;
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

export function paintOrder(count: number, hidden: ReadonlySet<number>, highlightIndex: number | null): number[] {
  const order: number[] = [];
  for (let i = 0; i < count; i++) {
    if (!hidden.has(i) && i !== highlightIndex) order.push(i);
  }
  if (highlightIndex !== null && highlightIndex >= 0 && highlightIndex < count && !hidden.has(highlightIndex)) {
    order.push(highlightIndex);
  }
  return order;
}

export function createContourPainter(options: ContourPainterOptions): OverlayPainter {
  const { levels, hidden, highlightIndex, colourMode, lineWidth } = options;
  const order = paintOrder(levels.length, hidden, highlightIndex);
  const bounds: PolylineBounds[][] = levels.map((level, index) => (hidden.has(index) ? [] : level.polylines.map(polylineBounds)));
  const minStepSq = MIN_SCREEN_STEP_PX * MIN_SCREEN_STEP_PX;
  return ({ ctx, toScreen, width, height }) => {
    const single = cssToken(TEAL_TOKEN, SINGLE_CONTOUR_COLOUR);
    ctx.save();
    ctx.setLineDash([]);
    ctx.lineJoin = "round";
    ctx.lineCap = "round";
    for (const index of order) {
      const level = levels[index];
      const highlighted = index === highlightIndex;
      ctx.strokeStyle = highlighted
        ? HIGHLIGHT_COLOUR
        : colourMode === "single"
          ? single
          : levelColour(index, levels.length, colourMode);
      ctx.lineWidth = highlighted ? HIGHLIGHT_LINE_WIDTH : lineWidth;
      const levelBounds = bounds[index];
      ctx.beginPath();
      level.polylines.forEach((polyline, pi) => {
        if (polyline.length < 2 || boundsOffScreen(levelBounds[pi], toScreen, width, height)) return;
        const first = toScreen({ x: polyline[0][0], y: polyline[0][1] });
        ctx.moveTo(first.x, first.y);
        let lastX = first.x;
        let lastY = first.y;
        const lastIndex = polyline.length - 1;
        for (let k = 1; k <= lastIndex; k++) {
          const p = toScreen({ x: polyline[k][0], y: polyline[k][1] });
          const dx = p.x - lastX;
          const dy = p.y - lastY;
          if (k < lastIndex && dx * dx + dy * dy < minStepSq) continue;
          ctx.lineTo(p.x, p.y);
          lastX = p.x;
          lastY = p.y;
        }
        if (level.closed[pi]) ctx.lineTo(first.x, first.y);
      });
      ctx.stroke();
    }
    ctx.restore();
  };
}
