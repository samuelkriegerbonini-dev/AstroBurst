import { describe, it, expect, vi } from "vitest";
import { CONTOUR_LAYER_ID, CONTOUR_LAYER_KIND, createContourPainter, paintOrder } from "../contourPainter";
import type { ContourLevel } from "../../../../shared/types/contours";
import type { OverlayPaintContext } from "../../../../utils/overlayStore";
import type { Pt } from "../../../../utils/regionGeometry";

const toScreen = (p: Pt): Pt => ({ x: p.x * 2 + 10, y: p.y * 2 + 20 });

function level(value: number, polylines: [number, number][][], closed: boolean[]): ContourLevel {
  return { value, polylines, closed, n_points: polylines.reduce((n, p) => n + p.length, 0) };
}

function fakeContext() {
  const ctx = {
    save: vi.fn(),
    restore: vi.fn(),
    setLineDash: vi.fn(),
    beginPath: vi.fn(),
    closePath: vi.fn(),
    moveTo: vi.fn(),
    lineTo: vi.fn(),
    stroke: vi.fn(),
    strokeStyle: "",
    lineWidth: 0,
    lineJoin: "miter",
    lineCap: "butt",
  };
  const strokes: { style: string; width: number }[] = [];
  ctx.stroke.mockImplementation(() => strokes.push({ style: ctx.strokeStyle, width: ctx.lineWidth }));
  return { ctx, strokes };
}

function paint(painter: ReturnType<typeof createContourPainter>, ctx: unknown, width = 200, height = 200) {
  painter({ ctx, toScreen, width, height, screenPxPerImagePx: 2 } as unknown as OverlayPaintContext);
}

describe("contour painter", () => {
  it("exports stable layer identifiers", () => {
    expect(CONTOUR_LAYER_ID).toBe("contours");
    expect(CONTOUR_LAYER_KIND).toBe("contours");
  });

  it("draws the highlighted level last and skips hidden ones", () => {
    expect(paintOrder(4, new Set([1]), 0)).toEqual([2, 3, 0]);
    expect(paintOrder(3, new Set(), null)).toEqual([0, 1, 2]);
    expect(paintOrder(3, new Set([2]), 2)).toEqual([0, 1]);
    expect(paintOrder(3, new Set(), 7)).toEqual([0, 1, 2]);
  });

  it("strokes visible polylines through toScreen, closes rings and highlights in white", () => {
    const levels = [
      level(
        1,
        [
          [
            [0, 0],
            [5, 0],
            [5, 5],
          ],
        ],
        [true],
      ),
      level(
        2,
        [
          [
            [1, 1],
            [2, 2],
          ],
        ],
        [false],
      ),
      level(3, [[[3, 3]]], [false]),
    ];
    const { ctx, strokes } = fakeContext();
    const painter = createContourPainter({ levels, hidden: new Set([2]), highlightIndex: 1, colourMode: "ramp", lineWidth: 1.5 });
    paint(painter, ctx);
    expect(ctx.save).toHaveBeenCalledTimes(1);
    expect(ctx.restore).toHaveBeenCalledTimes(1);
    expect(ctx.setLineDash).toHaveBeenCalledWith([]);
    expect(ctx.moveTo).toHaveBeenNthCalledWith(1, 10, 20);
    expect(ctx.lineTo).toHaveBeenNthCalledWith(1, 20, 20);
    expect(ctx.lineTo).toHaveBeenNthCalledWith(2, 20, 30);
    expect(ctx.closePath).toHaveBeenCalledTimes(1);
    expect(strokes).toEqual([
      { style: "hsl(220 90% 60%)", width: 1.5 },
      { style: "#ffffff", width: 2 },
    ]);
  });

  it("uses the teal fallback in single mode and culls polylines off the canvas", () => {
    const levels = [
      level(
        1,
        [
          [
            [500, 500],
            [600, 600],
          ],
          [
            [1, 1],
            [2, 2],
          ],
        ],
        [false, false],
      ),
    ];
    const { ctx, strokes } = fakeContext();
    const painter = createContourPainter({ levels, hidden: new Set(), highlightIndex: null, colourMode: "single", lineWidth: 1 });
    paint(painter, ctx);
    expect(ctx.moveTo).toHaveBeenCalledTimes(1);
    expect(ctx.moveTo).toHaveBeenCalledWith(12, 22);
    expect(strokes).toEqual([{ style: "#14b8a6", width: 1 }]);
  });
});
