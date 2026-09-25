import { describe, it, expect } from "vitest";
import { compassArrows, formatScaleBarLabel, niceScaleBarLength, scaleBarPixels, screenDirection } from "../compass";

describe("niceScaleBarLength", () => {
  it("picks the largest round length that fits", () => {
    expect(niceScaleBarLength(47)).toEqual({ arcsec: 30, label: '30"' });
    expect(niceScaleBarLength(60)).toEqual({ arcsec: 60, label: "1'" });
    expect(niceScaleBarLength(700)).toEqual({ arcsec: 600, label: "10'" });
    expect(niceScaleBarLength(9000)).toEqual({ arcsec: 7200, label: "2 deg" });
    expect(niceScaleBarLength(0.3)).toEqual({ arcsec: 0.2, label: '0.2"' });
  });

  it("returns null when nothing fits or the input is not usable", () => {
    expect(niceScaleBarLength(0.05)).toBeNull();
    expect(niceScaleBarLength(0)).toBeNull();
    expect(niceScaleBarLength(Number.NaN)).toBeNull();
  });

  it("formats labels without trailing zeros", () => {
    expect(formatScaleBarLabel(90)).toBe("1.5'");
    expect(formatScaleBarLabel(5400)).toBe("1.5 deg");
  });
});

describe("screenDirection and compassArrows", () => {
  it("normalises the screen vector between two mapped points", () => {
    expect(screenDirection({ x: 0, y: 0 }, { x: 0, y: -4 })).toEqual({ x: 0, y: -1 });
    expect(screenDirection({ x: 1, y: 1 }, { x: 1, y: 1 })).toBeNull();
    expect(screenDirection({ x: 0, y: 0 }, { x: Number.NaN, y: 1 })).toBeNull();
  });

  it("places arrow tips along the given directions", () => {
    const arrows = compassArrows({ x: 40, y: 100 }, { x: 0, y: -1 }, { x: -1, y: 0 }, 25);
    expect(arrows.northTip).toEqual({ x: 40, y: 75 });
    expect(arrows.eastTip).toEqual({ x: 15, y: 100 });
  });
});

describe("scaleBarPixels", () => {
  it("converts arcseconds to screen pixels through the pixel scale and zoom", () => {
    expect(scaleBarPixels(30, 0.5, 2)).toBe(120);
    expect(scaleBarPixels(30, 0, 2)).toBe(0);
  });
});
