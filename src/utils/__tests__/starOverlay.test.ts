import { describe, expect, it } from "vitest";
import {
  VIEW_SCALE_ATTRIBUTE,
  fitImageToCanvas,
  fitsPixelToCanvas,
  imagePointToCanvas,
  overlayLayers,
  overlayStrokeScale,
  viewScaleAttributes,
} from "../starOverlay";
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

describe("star overlay on a downsampled composite texture", () => {
  it("maps a star of a full-resolution composite onto the texture within half a texture pixel", () => {
    const fit = fitImageToCanvas(2048, 683, 3000, 1000);
    for (const [x, y] of [[0, 0], [2999, 999], [1500, 500]]) {
      const p = imagePointToCanvas(x, y, fit);
      expect(Math.abs(p.x - ((x + 0.5) * 2048) / 3000)).toBeLessThan(0.5);
      expect(Math.abs(p.y - ((y + 0.5) * 683) / 1000)).toBeLessThan(0.5);
    }
  });
});

describe("overlayStrokeScale", () => {
  it("thickens strokes by the zoom-out factor so they keep their on-screen size", () => {
    expect(overlayStrokeScale(512, 2048, true)).toBe(4);
  });

  it("leaves strokes unchanged at 100 % and when zoomed in", () => {
    expect(overlayStrokeScale(2048, 2048, true)).toBe(1);
    expect(overlayStrokeScale(8192, 2048, true)).toBe(1);
  });

  it("falls back to unscaled strokes while the canvas has no size", () => {
    expect(overlayStrokeScale(0, 2048, true)).toBe(1);
    expect(overlayStrokeScale(512, 0, true)).toBe(1);
    expect(overlayStrokeScale(Number.NaN, 2048, true)).toBe(1);
  });

  it("keeps strokes unscaled on a host that does not publish its view scale, since it never redraws on zoom", () => {
    expect(overlayStrokeScale(1024, 4096, false)).toBe(1);
    expect(overlayStrokeScale(512, 2048, false)).toBe(1);
  });
});

describe("viewScaleAttributes", () => {
  it("publishes the view scale rounded to three decimals under the attribute the overlay observes", () => {
    expect(viewScaleAttributes(0.3214567)).toEqual({ [VIEW_SCALE_ATTRIBUTE]: "0.321" });
    expect(viewScaleAttributes(2)).toEqual({ [VIEW_SCALE_ATTRIBUTE]: "2" });
  });

  it("uses a data attribute, so React writes it without touching the canvas style", () => {
    expect(VIEW_SCALE_ATTRIBUTE.startsWith("data-")).toBe(true);
  });
});

describe("overlayLayers", () => {
  const base = {
    canvasMounted: true,
    showStars: true,
    starCount: 12,
    showAnnotations: true,
    annotationCount: 3,
    annotationsOnView: true,
  };

  it("draws stars and plate-solve labels on a view in the solved file's pixel grid", () => {
    expect(overlayLayers(base)).toEqual({ drawStars: true, drawAnnotations: true, unavailable: false });
  });

  it("does not draw plate-solve labels on a view measured on another grid, such as the wizard composite", () => {
    expect(overlayLayers({ ...base, annotationsOnView: false })).toMatchObject({ drawStars: true, drawAnnotations: false });
  });

  it("follows the eye and label toggles", () => {
    expect(overlayLayers({ ...base, showStars: false, showAnnotations: false })).toMatchObject({
      drawStars: false,
      drawAnnotations: false,
    });
  });

  it("reports a missing layer only for content that would be drawn on this view", () => {
    expect(overlayLayers({ ...base, canvasMounted: false }).unavailable).toBe(true);
    expect(overlayLayers({ ...base, canvasMounted: false, starCount: 0 }).unavailable).toBe(true);
    expect(overlayLayers({ ...base, canvasMounted: false, starCount: 0, annotationsOnView: false }).unavailable).toBe(false);
    expect(overlayLayers({ ...base, canvasMounted: false, starCount: 0, annotationCount: 0 }).unavailable).toBe(false);
  });
});
