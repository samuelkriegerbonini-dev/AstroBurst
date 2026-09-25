import { describe, it, expect } from "vitest";
import {
  APERTURE_LAYER_ID,
  APERTURE_LAYER_KIND,
  apertureOffCanvas,
  apertureScreenGeometry,
  type ApertureMarker,
} from "../aperturePainter";
import type { Pt } from "../../../../utils/regionGeometry";

const toScreen = (p: Pt): Pt => ({ x: p.x * 2 + 10, y: p.y * 2 + 20 });

function marker(overrides: Partial<ApertureMarker> = {}): ApertureMarker {
  return { x: 100, y: 50, rAp: 5, skyIn: 10, skyOut: 15, label: "s1", ...overrides };
}

describe("apertureScreenGeometry", () => {
  it("maps the centre through toScreen and scales the radii by the screen pixel ratio", () => {
    const g = apertureScreenGeometry(marker(), toScreen, 2);
    expect(g.cx).toBe(210);
    expect(g.cy).toBe(120);
    expect(g.rAp).toBe(10);
    expect(g.skyIn).toBe(20);
    expect(g.skyOut).toBe(30);
    expect(g.labelX).toBe(210 + 10 + 3);
    expect(g.labelY).toBe(120);
  });

  it("never collapses a radius below half a screen pixel", () => {
    const g = apertureScreenGeometry(marker({ rAp: 0, skyIn: Number.NaN, skyOut: 15 }), toScreen, 0.01);
    expect(g.rAp).toBe(0.5);
    expect(g.skyIn).toBe(0.5);
    expect(g.skyOut).toBe(0.5);
  });
});

describe("apertureOffCanvas", () => {
  it("keeps an aperture whose outer annulus touches the canvas and culls one fully outside", () => {
    const inside = apertureScreenGeometry(marker({ x: -6, y: 0 }), (p) => p, 1);
    expect(apertureOffCanvas(inside, 200, 100)).toBe(false);
    const outside = apertureScreenGeometry(marker({ x: -16, y: 0 }), (p) => p, 1);
    expect(apertureOffCanvas(outside, 200, 100)).toBe(true);
    const below = apertureScreenGeometry(marker({ x: 50, y: 120 }), (p) => p, 1);
    expect(apertureOffCanvas(below, 200, 100)).toBe(true);
    const nan = apertureScreenGeometry(marker({ x: Number.NaN }), (p) => p, 1);
    expect(apertureOffCanvas(nan, 200, 100)).toBe(true);
  });
});

describe("layer identity", () => {
  it("exposes stable id and kind", () => {
    expect(APERTURE_LAYER_ID).toBe("photometry-apertures");
    expect(APERTURE_LAYER_KIND).toBe("apertures");
  });
});
