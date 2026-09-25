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
    expect(ctx.closePath).not.toHaveBeenCalled();
    expect(ctx.lineTo).toHaveBeenNthCalledWith(3, 10, 20);
    expect(ctx.lineTo).toHaveBeenCalledTimes(4);
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

  it("closes every ring with a segment back to its first vertex inside one shared path, never with closePath", () => {
    const levels = [
      level(
        1,
        [
          [
            [0, 0],
            [5, 0],
            [5, 5],
          ],
          [
            [10, 10],
            [12, 10],
            [12, 12],
          ],
        ],
        [true, true],
      ),
    ];
    const { ctx, strokes } = fakeContext();
    paint(createContourPainter({ levels, hidden: new Set(), highlightIndex: null, colourMode: "ramp", lineWidth: 1 }), ctx);
    expect(ctx.closePath).not.toHaveBeenCalled();
    expect(ctx.beginPath).toHaveBeenCalledTimes(1);
    expect(ctx.moveTo).toHaveBeenNthCalledWith(1, 10, 20);
    expect(ctx.lineTo).toHaveBeenNthCalledWith(3, 10, 20);
    expect(ctx.moveTo).toHaveBeenNthCalledWith(2, 30, 40);
    expect(ctx.lineTo).toHaveBeenNthCalledWith(6, 30, 40);
    expect(ctx.lineTo).toHaveBeenCalledTimes(6);
    expect(strokes).toHaveLength(1);
  });

  const ring = (cx: number, cy: number, r: number, n: number): [number, number][] =>
    Array.from({ length: n }, (_, k) => [cx + r * Math.cos((2 * Math.PI * k) / n), cy + r * Math.sin((2 * Math.PI * k) / n)]);

  function paintAt(levels: ContourLevel[], scale: number, width: number, height: number) {
    const { ctx } = fakeContext();
    createContourPainter({ levels, hidden: new Set(), highlightIndex: null, colourMode: "ramp", lineWidth: 1 })({
      ctx,
      toScreen: (p: Pt) => ({ x: p.x * scale, y: p.y * scale }),
      width,
      height,
      screenPxPerImagePx: scale,
    } as unknown as OverlayPaintContext);
    return ctx;
  }

  it("emits about one vertex per screen pixel at fit zoom and keeps sub-pixel rings as dots", () => {
    const small = Array.from({ length: 1000 }, (_, i) => ring((i % 40) * 5 + 2, Math.floor(i / 40) * 5 + 2, 1, 12));
    const big = ring(1000, 1000, 200, 4000);
    const ctx = paintAt([level(1, [...small, big], [...small.map(() => true), true])], 0.39, 1200, 800);
    expect(ctx.moveTo).toHaveBeenCalledTimes(1001);
    expect(ctx.closePath).not.toHaveBeenCalled();
    expect(ctx.lineTo.mock.calls.length).toBeLessThan(3000);
    expect(ctx.lineTo.mock.calls.length).toBeGreaterThanOrEqual(2 * 1000 + 400);
  });

  it("does not decimate when vertices are more than a screen pixel apart", () => {
    const big = ring(1000, 1000, 200, 4000);
    const ctx = paintAt([level(1, [big], [true])], 20, 100000, 100000);
    expect(ctx.lineTo).toHaveBeenCalledTimes(4000);
  });

  it("culls polylines entirely off the canvas from bounds computed when the painter is built", () => {
    const inside = ring(50, 50, 10, 64);
    const outside = ring(5000, 5000, 10, 64);
    const ctx = paintAt([level(1, [outside, inside], [true, true])], 1, 200, 200);
    expect(ctx.moveTo).toHaveBeenCalledTimes(1);
    expect(ctx.moveTo).toHaveBeenCalledWith(inside[0][0], inside[0][1]);
  });
});
