import { describe, expect, it } from "vitest";
import { fitImageToCanvas, fitsPixelToCanvas, imagePointToCanvas } from "../starOverlay";
import { regionPointToScreen } from "../regionCoords";

describe("star overlay mapping", () => {
  it("letterboxes the image inside the canvas", () => {
    expect(fitImageToCanvas(200, 200, 100, 50)).toEqual({ scale: 2, ox: 0, oy: 50 });
  });

  it("draws a star centroid at the pixel centre, not at its top-left edge", () => {
    const fit = fitImageToCanvas(200, 100, 100, 50);
    expect(imagePointToCanvas(0, 0, fit)).toEqual({ x: 1, y: 1 });
    expect(imagePointToCanvas(10, 20, fit)).toEqual({ x: 21, y: 41 });
  });

  it("draws a 1-based FITS annotation at the centre of the same pixel as a 0-based star", () => {
    const fit = fitImageToCanvas(200, 100, 100, 50);
    expect(fitsPixelToCanvas(1, 1, fit)).toEqual(imagePointToCanvas(0, 0, fit));
    expect(fitsPixelToCanvas(11, 21, fit)).toEqual({ x: 21, y: 41 });
  });

  it("agrees with the region overlay mapping for an unzoomed full view", () => {
    const fit = fitImageToCanvas(400, 200, 400, 200);
    const star = imagePointToCanvas(123, 45, fit);
    const region = regionPointToScreen(
      { x: 123, y: 45 },
      { transform: { scale: 1, x: 0, y: 0 }, renderW: 400, renderH: 200, fitsW: 400, fitsH: 200 },
    );
    expect(star.x).toBeCloseTo(region.x, 9);
    expect(star.y).toBeCloseTo(region.y, 9);
  });

  it("guards against unknown image dimensions", () => {
    const fit = fitImageToCanvas(100, 100, 0, 0);
    expect(Number.isFinite(fit.scale)).toBe(true);
  });
});
