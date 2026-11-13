import { describe, expect, it } from "vitest";
import { DEPTH_LAYER_ID, DEPTH_LAYER_KIND, createDepthPainter, depthCircles } from "../depthOverlay";
import type { DepthAperture } from "../../shared/types/depth";
import type { Pt } from "../regionGeometry";

const at = (x: number, y: number, usable = true): DepthAperture => ({ x, y, sum: 0, formal_err: null, usable });
const identity = (p: Pt): Pt => p;

function recordingContext() {
  const calls: { name: string; args: unknown[]; strokeStyle: unknown; lineWidth: unknown }[] = [];
  const ctx: Record<string, unknown> = { strokeStyle: "", lineWidth: 0 };
  for (const name of ["save", "restore", "beginPath", "arc", "stroke", "setLineDash"]) {
    ctx[name] = (...args: unknown[]) => {
      calls.push({ name, args, strokeStyle: ctx.strokeStyle, lineWidth: ctx.lineWidth });
    };
  }
  return { calls, ctx: ctx as unknown as CanvasRenderingContext2D };
}

describe("depthCircles", () => {
  it("circles are mapped through toScreen and scaled", () => {
    const toScreen = (p: Pt): Pt => ({ x: p.x * 2 + 10, y: p.y * 2 + 20 });
    expect(depthCircles([{ x: 100, y: 50, sum: 0, formal_err: null, usable: true }], 4, toScreen, 2, 1000, 1000)).toEqual([
      { cx: 210, cy: 120, r: 8, usable: true },
    ]);
    expect(depthCircles([at(100, 50, false)], 0, toScreen, 2, 1000, 1000)).toEqual([{ cx: 210, cy: 120, r: 0.5, usable: false }]);
    expect(depthCircles([at(100, 50)], Number.NaN, toScreen, 2, 1000, 1000)).toEqual([{ cx: 210, cy: 120, r: 0.5, usable: true }]);
  });

  it("circles outside the canvas or without a position are left out", () => {
    expect(depthCircles([at(-10, 0)], 4, identity, 1, 200, 100)).toEqual([]);
    expect(depthCircles([at(-3, 0)], 4, identity, 1, 200, 100)).toEqual([{ cx: -3, cy: 0, r: 4, usable: true }]);
    expect(depthCircles([at(50, -5)], 4, identity, 1, 200, 100)).toEqual([]);
    expect(depthCircles([at(205, 50)], 4, identity, 1, 200, 100)).toEqual([]);
    expect(depthCircles([at(50, 105)], 4, identity, 1, 200, 100)).toEqual([]);
    expect(depthCircles([at(Number.NaN, 10), at(10, Number.NaN)], 4, identity, 1, 200, 100)).toEqual([]);
  });
});

describe("createDepthPainter", () => {
  it("the painter strokes usable apertures solid and unusable ones dashed", () => {
    const { calls, ctx } = recordingContext();
    createDepthPainter([at(20, 20), at(60, 40, false)], 4)({ ctx, toScreen: identity, width: 200, height: 100, screenPxPerImagePx: 1 });
    const named = (name: string) => calls.filter((call) => call.name === name);
    expect(named("save")).toHaveLength(1);
    expect(named("restore")).toHaveLength(1);
    expect(calls[0].name).toBe("save");
    expect(calls[calls.length - 1].name).toBe("restore");
    expect(named("arc").map((call) => call.args)).toEqual([
      [20, 20, 4, 0, Math.PI * 2],
      [60, 40, 4, 0, Math.PI * 2],
    ]);
    expect(named("setLineDash").map((call) => call.args[0])).toEqual([[], [3, 3]]);
    const strokes = named("stroke");
    expect(strokes.map((call) => call.strokeStyle)).toEqual(["#7dd3fc", "#fbbf24"]);
    expect(strokes.map((call) => call.lineWidth)).toEqual([1, 1]);
  });

  it("names its layer", () => {
    expect(DEPTH_LAYER_ID).toBe("depth-apertures");
    expect(DEPTH_LAYER_KIND).toBe("depth-apertures");
  });
});
