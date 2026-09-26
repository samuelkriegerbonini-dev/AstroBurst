import { describe, expect, it } from "vitest";
import { REGION_SHAPE_KINDS } from "../../shared/types/regions";
import { regionLayerSwallowsClick, viewportClickRoute } from "../regionClick";

describe("regionLayerSwallowsClick", () => {
  it("lets a Select-mode click on empty sky reach the viewer", () => {
    expect(regionLayerSwallowsClick("select", false)).toBe(false);
  });

  it("keeps a Select-mode click whose press the layer consumed (a region, a handle, or the drawing that just switched to Select)", () => {
    expect(regionLayerSwallowsClick("select", true)).toBe(true);
  });

  it("keeps every click while a drawing tool is active", () => {
    for (const tool of REGION_SHAPE_KINDS) {
      expect(regionLayerSwallowsClick(tool, false)).toBe(true);
    }
  });

  it("never keeps a click when no region tool is active", () => {
    expect(regionLayerSwallowsClick("none", true)).toBe(false);
    expect(regionLayerSwallowsClick("none", false)).toBe(false);
  });
});

describe("viewportClickRoute", () => {
  it("drops a Select-mode click on the gutter outside the image instead of mapping it to a spaxel", () => {
    expect(regionLayerSwallowsClick("select", false)).toBe(false);
    expect(viewportClickRoute("crosshair", false, true, true)).toBe("none");
    expect(viewportClickRoute("pan", false, true, true)).toBe("none");
  });

  it("sends an on-image crosshair click to the pixel handler", () => {
    expect(viewportClickRoute("crosshair", true, true, true)).toBe("pixel");
  });

  it("sends an on-image pan click to the canvas pixel handler, which receives image coordinates", () => {
    expect(viewportClickRoute("pan", true, true, true)).toBe("canvas");
    expect(viewportClickRoute("crosshair", true, false, true)).toBe("canvas");
  });

  it("does nothing when the matching handler is absent", () => {
    expect(viewportClickRoute("pan", true, true, false)).toBe("none");
    expect(viewportClickRoute("crosshair", true, false, false)).toBe("none");
  });
});
