import { describe, it, expect } from "vitest";
import { screenToImagePixel, type ViewerTransform } from "../pixelMapping";

const rect = { left: 0, top: 0 } as DOMRect;
const identity: ViewerTransform = { scale: 1, x: 0, y: 0 };

describe("screenToImagePixel", () => {
  it("maps a client point inside a 100x100 render to floored pixel coords with the identity transform", () => {
    expect(screenToImagePixel(10, 20, rect, identity, 100, 100, 100, 100)).toEqual({ x: 10, y: 20 });
    expect(screenToImagePixel(10.9, 20.4, rect, identity, 100, 100, 100, 100)).toEqual({ x: 10, y: 20 });
  });

  it("returns null outside [0, renderW) x [0, renderH)", () => {
    expect(screenToImagePixel(-1, 20, rect, identity, 100, 100, 100, 100)).toBeNull();
    expect(screenToImagePixel(10, -0.5, rect, identity, 100, 100, 100, 100)).toBeNull();
    expect(screenToImagePixel(100, 20, rect, identity, 100, 100, 100, 100)).toBeNull();
    expect(screenToImagePixel(10, 100, rect, identity, 100, 100, 100, 100)).toBeNull();
    expect(screenToImagePixel(99.99, 99.99, rect, identity, 100, 100, 100, 100)).toEqual({ x: 99, y: 99 });
  });

  it("inverts scale and translation of the viewer transform", () => {
    const transform: ViewerTransform = { scale: 2, x: 10, y: 20 };
    expect(screenToImagePixel(30, 60, rect, transform, 100, 100, 100, 100)).toEqual({ x: 10, y: 20 });
    expect(screenToImagePixel(9, 60, rect, transform, 100, 100, 100, 100)).toBeNull();
  });

  it("subtracts the container rect origin", () => {
    const offsetRect = { left: 5, top: 7 } as DOMRect;
    expect(screenToImagePixel(15, 27, offsetRect, identity, 100, 100, 100, 100)).toEqual({ x: 10, y: 20 });
  });

  it("scales render coords to FITS coords by the render->fits ratio with floor", () => {
    expect(screenToImagePixel(10, 20, rect, identity, 100, 100, 400, 400)).toEqual({ x: 40, y: 80 });
    expect(screenToImagePixel(10.3, 20.7, rect, identity, 100, 100, 400, 400)).toEqual({ x: 41, y: 82 });
    expect(screenToImagePixel(99.9, 99.9, rect, identity, 100, 100, 400, 400)).toEqual({ x: 399, y: 399 });
  });
});
