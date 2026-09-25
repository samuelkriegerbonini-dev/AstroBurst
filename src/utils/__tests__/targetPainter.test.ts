import { describe, it, expect } from "vitest";
import {
  TARGET_LAYER_ID,
  diamondVertices,
  isOffscreen,
  targetMarkerHalf,
} from "../../components/viewer/painters/targetPainter";

describe("diamondVertices", () => {
  it("returns top, right, bottom and left points around the centre", () => {
    expect(diamondVertices({ x: 10, y: 20 }, 5)).toEqual([
      { x: 10, y: 15 },
      { x: 15, y: 20 },
      { x: 10, y: 25 },
      { x: 5, y: 20 },
    ]);
  });
});

describe("isOffscreen", () => {
  it("culls points beyond the margin and non-finite points", () => {
    expect(isOffscreen({ x: 50, y: 50 }, 100, 100, 5)).toBe(false);
    expect(isOffscreen({ x: -4, y: 50 }, 100, 100, 5)).toBe(false);
    expect(isOffscreen({ x: -6, y: 50 }, 100, 100, 5)).toBe(true);
    expect(isOffscreen({ x: 50, y: 106 }, 100, 100, 5)).toBe(true);
    expect(isOffscreen({ x: Number.NaN, y: 50 }, 100, 100, 5)).toBe(true);
  });
});

describe("targetMarkerHalf", () => {
  it("grows the highlighted marker and keeps the layer id stable", () => {
    expect(targetMarkerHalf(false)).toBe(5);
    expect(targetMarkerHalf(true)).toBe(8);
    expect(TARGET_LAYER_ID).toBe("targets");
  });
});
