import { describe, expect, it } from "vitest";
import { arcsecPerScreenPx, clampView, fitScale, fitView, isFit, maxScale, panBy, zoomAt, zoomTo } from "../logic/zoom";

const IMG = { w: 4000, h: 2000 };
const BOX = { w: 1000, h: 800 };
const MAX = maxScale(IMG);

describe("limits", () => {
  it("stops at 1:1 of the 4096 px copy", () => {
    expect(maxScale({ w: 14341, h: 8583 })).toBeCloseTo(4096 / 14341, 12);
    expect(maxScale({ w: 2048, h: 1226 })).toBe(1);
    expect(MAX).toBe(1);
  });

  it("fits the whole image and never enlarges past the limit", () => {
    expect(fitScale(IMG, BOX, MAX)).toBe(0.25);
    expect(fitScale({ w: 500, h: 300 }, BOX, 1)).toBe(1);
  });
});

describe("fit and zoom", () => {
  it("centres the fitted image", () => {
    expect(fitView(IMG, BOX, MAX)).toEqual({ scale: 0.25, x: 0, y: 150 });
    expect(fitView({ w: 500, h: 300 }, BOX, 1)).toEqual({ scale: 1, x: 250, y: 250 });
  });

  it("zooms at the cursor and keeps the point under it", () => {
    const fit = fitView(IMG, BOX, MAX);
    expect(zoomAt(fit, 2, { x: 500, y: 400 }, IMG, BOX, MAX)).toEqual({ scale: 0.5, x: -500, y: -100 });
    expect(zoomAt(fit, 10, { x: 500, y: 400 }, IMG, BOX, MAX).scale).toBe(1);
    expect(zoomAt(fit, 0.1, { x: 500, y: 400 }, IMG, BOX, MAX)).toEqual(fit);
  });

  it("goes to an exact scale around the centre", () => {
    expect(zoomTo(fitView(IMG, BOX, MAX), 1, IMG, BOX, MAX)).toEqual({ scale: 1, x: -1500, y: -600 });
  });

  it("pans without leaving a gap at the edges", () => {
    const zoomed = { scale: 0.5, x: -500, y: -100 };
    expect(panBy(zoomed, 600, 0, IMG, BOX, MAX)).toEqual({ scale: 0.5, x: 0, y: -100 });
    expect(panBy(zoomed, -1000, -500, IMG, BOX, MAX)).toEqual({ scale: 0.5, x: -1000, y: -200 });
    expect(panBy(fitView(IMG, BOX, MAX), 0, 90, IMG, BOX, MAX)).toEqual({ scale: 0.25, x: 0, y: 150 });
  });

  it("clamps a stale view after the stage changes size", () => {
    expect(clampView({ scale: 0.1, x: 40, y: 40 }, IMG, BOX, MAX)).toEqual({ scale: 0.25, x: 0, y: 150 });
    expect(isFit(fitView(IMG, BOX, MAX), IMG, BOX, MAX)).toBe(true);
    expect(isFit({ scale: 0.5, x: -500, y: -100 }, IMG, BOX, MAX)).toBe(false);
  });
});

describe("scale bar input", () => {
  it("turns source arcseconds per pixel into arcseconds per screen pixel", () => {
    expect(arcsecPerScreenPx(0.031, 0.25)).toBeCloseTo(0.124, 12);
    expect(arcsecPerScreenPx(0.031, 0)).toBe(Number.POSITIVE_INFINITY);
  });
});
