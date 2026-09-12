import { describe, it, expect } from "vitest";
import {
  screenToImagePixel,
  screenToImageCoord,
  imageCoordToScreen,
  edgeToCentre,
  centreToEdge,
  screenPxPerImagePx,
  type ViewerTransform,
} from "../pixelMapping";

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

describe("screenToImageCoord / imageCoordToScreen", () => {
  it("returns continuous edge-based coordinates and null outside the render area", () => {
    expect(screenToImageCoord(10.4, 20.25, rect, identity, 100, 100, 100, 100)).toEqual({ x: 10.4, y: 20.25 });
    expect(screenToImageCoord(-0.01, 20, rect, identity, 100, 100, 100, 100)).toBeNull();
    expect(screenToImageCoord(100, 20, rect, identity, 100, 100, 100, 100)).toBeNull();
    expect(screenToImageCoord(10, 100, rect, identity, 100, 100, 100, 100)).toBeNull();
  });

  it("round-trips through imageCoordToScreen for identity, scaled+offset and render/fits ratio", () => {
    const cases: { t: ViewerTransform; renderW: number; fitsW: number }[] = [
      { t: identity, renderW: 100, fitsW: 100 },
      { t: { scale: 2, x: 10, y: 20 }, renderW: 100, fitsW: 100 },
      { t: identity, renderW: 100, fitsW: 400 },
      { t: { scale: 0.25, x: -3.5, y: 7.25 }, renderW: 100, fitsW: 400 },
    ];
    for (const { t, renderW, fitsW } of cases) {
      const p = { x: 10.3 + t.x, y: 17.7 + t.y };
      const img = screenToImageCoord(p.x, p.y, rect, t, renderW, renderW, fitsW, fitsW);
      expect(img).not.toBeNull();
      const back = imageCoordToScreen(img!.x, img!.y, t, renderW, renderW, fitsW, fitsW);
      expect(Math.abs(back.x - p.x)).toBeLessThan(1e-9);
      expect(Math.abs(back.y - p.y)).toBeLessThan(1e-9);
    }
  });

  it("subtracts the container rect origin", () => {
    const offsetRect = { left: 5, top: 7 } as DOMRect;
    expect(screenToImageCoord(15.5, 27.5, offsetRect, identity, 100, 100, 100, 100)).toEqual({ x: 10.5, y: 20.5 });
  });
});

describe("edgeToCentre / centreToEdge", () => {
  it("shifts by half a pixel and inverts", () => {
    expect(edgeToCentre(10.5)).toBe(10);
    expect(centreToEdge(edgeToCentre(10.5))).toBe(10.5);
    expect(centreToEdge(10)).toBe(10.5);
  });

  it("agrees with screenToImagePixel at the screen centre of pixel (10,10)", () => {
    const t: ViewerTransform = { scale: 4, x: 12, y: -6 };
    const centre = imageCoordToScreen(centreToEdge(10), centreToEdge(10), t, 100, 100, 100, 100);
    const probe = screenToImagePixel(centre.x, centre.y, rect, t, 100, 100, 100, 100);
    expect(probe).toEqual({ x: 10, y: 10 });
    const cont = screenToImageCoord(centre.x, centre.y, rect, t, 100, 100, 100, 100)!;
    expect(edgeToCentre(cont.x)).toBeCloseTo(10, 9);
    expect(edgeToCentre(cont.y)).toBeCloseTo(10, 9);
  });
});

describe("screenPxPerImagePx", () => {
  it("multiplies the scale by the render/fits ratio", () => {
    expect(screenPxPerImagePx({ scale: 2, x: 0, y: 0 }, 100, 400)).toBe(0.5);
    expect(screenPxPerImagePx(identity, 100, 100)).toBe(1);
  });
});
