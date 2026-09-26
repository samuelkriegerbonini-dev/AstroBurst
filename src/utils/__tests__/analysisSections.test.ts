import { describe, it, expect } from "vitest";
import { analysisSections, deepZoomAvailable } from "../analysisSections";

describe("deepZoomAvailable", () => {
  it("offers Deep Zoom only above 4096 pixels on either side", () => {
    expect(deepZoomAvailable(4096, 4096)).toBe(false);
    expect(deepZoomAvailable(4097, 100)).toBe(true);
    expect(deepZoomAvailable(100, 8192)).toBe(true);
    expect(deepZoomAvailable(undefined, undefined)).toBe(false);
  });
});

const base = { hasHistogram: true, isCube: false, showFft: true, showDeepZoom: true };

describe("analysisSections", () => {
  it("lists every panel of a large mono frame in column order", () => {
    expect(analysisSections(base).map((s) => s.label)).toEqual([
      "Histogram",
      "Stars",
      "Photometry",
      "Table",
      "Series",
      "Geometry",
      "Catalog",
      "Targets",
      "Stats",
      "Pixels",
      "Regions",
      "Profiles",
      "Contours",
      "FFT",
      "Deep Zoom",
      "Log",
    ]);
  });

  it("gives every section a distinct element id", () => {
    const ids = analysisSections({ ...base, isCube: true }).map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) expect(id).toMatch(/^analysis-[a-z-]+$/);
  });

  it("drops the panels that are not rendered", () => {
    const labels = analysisSections({ hasHistogram: false, isCube: false, showFft: false, showDeepZoom: false }).map(
      (s) => s.label,
    );
    expect(labels).not.toContain("Histogram");
    expect(labels).not.toContain("FFT");
    expect(labels).not.toContain("Deep Zoom");
    expect(labels).not.toContain("Spectrum");
    expect(labels[0]).toBe("Stars");
    expect(labels[labels.length - 1]).toBe("Log");
  });

  it("adds the cube panels after Contours for a cube", () => {
    const labels = analysisSections({ ...base, isCube: true, showFft: false }).map((s) => s.label);
    const contours = labels.indexOf("Contours");
    expect(labels.slice(contours + 1, contours + 3)).toEqual(["Spectrum", "PV"]);
  });
});
