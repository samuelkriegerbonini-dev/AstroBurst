import { describe, it, expect } from "vitest";
import { pixelFromRect } from "../useMousePixelStore";

const DIMS: [number, number] = [4000, 3000];

describe("pixelFromRect", () => {
  it("maps a point inside the element to a FITS pixel of the given dimensions", () => {
    const rect = { left: 100, top: 50, width: 400, height: 300 };
    expect(pixelFromRect(300, 200, rect, DIMS)).toEqual({ x: 2000, y: 1500 });
    expect(pixelFromRect(100, 50, rect, DIMS)).toEqual({ x: 0, y: 0 });
  });

  it("returns null outside the element", () => {
    const rect = { left: 100, top: 50, width: 400, height: 300 };
    expect(pixelFromRect(99, 60, rect, DIMS)).toBeNull();
    expect(pixelFromRect(500, 60, rect, DIMS)).toBeNull();
    expect(pixelFromRect(200, 350, rect, DIMS)).toBeNull();
  });

  it("returns null for a hidden element with an empty rect instead of a non-finite coordinate", () => {
    const hidden = { left: 0, top: 0, width: 0, height: 0 };
    expect(pixelFromRect(10, 10, hidden, DIMS)).toBeNull();
    expect(pixelFromRect(0, 0, hidden, DIMS)).toBeNull();
  });

  it("returns null without dimensions", () => {
    expect(pixelFromRect(10, 10, { left: 0, top: 0, width: 100, height: 100 }, null)).toBeNull();
  });
});
