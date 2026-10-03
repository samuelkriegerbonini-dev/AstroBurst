import { describe, it, expect } from "vitest";
import {
  ANALYSIS_SECTION,
  ANALYSIS_TABS,
  CUBE_TAB_UNAVAILABLE_TITLE,
  analysisPanelAttributes,
  analysisSections,
  analysisTabSections,
  createAnalysisTabMemory,
  deepZoomAvailable,
  defaultAnalysisTab,
  isTabNavKey,
  nextAnalysisTab,
  resolveAnalysisTab,
  tabAvailability,
} from "../analysisSections";

describe("deepZoomAvailable", () => {
  it("offers Deep Zoom only above 4096 pixels on either side", () => {
    expect(deepZoomAvailable(4096, 4096)).toBe(false);
    expect(deepZoomAvailable(4097, 100)).toBe(true);
    expect(deepZoomAvailable(100, 8192)).toBe(true);
    expect(deepZoomAvailable(undefined, undefined)).toBe(false);
  });
});

const base = { hasHistogram: true, isCube: false, isRamp: false, showFft: true, showDeepZoom: true };

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
    const labels = analysisSections({ hasHistogram: false, isCube: false, isRamp: false, showFft: false, showDeepZoom: false }).map(
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
    expect(labels).not.toContain("Ramp");
  });

  it("puts the Ramp section right before Spectrum only for a ramp", () => {
    const labels = analysisSections({ ...base, isCube: true, isRamp: true, showFft: false }).map((s) => s.label);
    const spectrum = labels.indexOf("Spectrum");
    expect(labels.slice(spectrum - 1, spectrum + 2)).toEqual(["Ramp", "Spectrum", "PV"]);
    expect(labels.filter((l) => l === "Ramp")).toHaveLength(1);
    const ids = analysisSections({ ...base, isCube: true, isRamp: true }).map((s) => s.id);
    expect(ids).toContain("analysis-ramp");
    expect(new Set(ids).size).toBe(ids.length);
  });
});

const everything = { hasHistogram: true, isCube: true, isRamp: true, showFft: true, showDeepZoom: true };
const plain = { hasHistogram: true, isCube: false, isRamp: false, showFft: true, showDeepZoom: false };
const cube = { ...plain, isCube: true, showFft: false };
const rampOnly = { ...plain, isRamp: true };

describe("analysis tabs", () => {
  it("names the three tabs Image, Sources and Cube in that order", () => {
    expect(ANALYSIS_TABS.map((t) => [t.id, t.label])).toEqual([
      ["image", "Image"],
      ["sources", "Sources"],
      ["cube", "Cube"],
    ]);
    expect(CUBE_TAB_UNAVAILABLE_TITLE).toBe("Open a data cube or a ramp");
  });

  it("assigns every section to its tab and keeps the log out of the tabs", () => {
    const tabOf = Object.fromEntries(Object.entries(ANALYSIS_SECTION).map(([key, s]) => [key, s.tab]));
    expect(tabOf).toEqual({
      histogram: "image",
      statistics: "image",
      pixels: "image",
      regions: "image",
      profiles: "image",
      contours: "image",
      fft: "image",
      deepZoom: "image",
      stars: "sources",
      photometry: "sources",
      table: "sources",
      series: "sources",
      geometry: "sources",
      catalog: "sources",
      targets: "sources",
      ramp: "cube",
      spectrum: "cube",
      pv: "cube",
      log: null,
    });
    expect(ANALYSIS_SECTION.log.id).toBe("analysis-log");
  });

  it("lists each tab's sections in the column order", () => {
    const labels = (tab: "image" | "sources" | "cube") => analysisTabSections(everything, tab).map((s) => s.label);
    expect(labels("image")).toEqual(["Histogram", "Stats", "Pixels", "Regions", "Profiles", "Contours", "FFT", "Deep Zoom"]);
    expect(labels("sources")).toEqual(["Stars", "Photometry", "Table", "Series", "Geometry", "Catalog", "Targets"]);
    expect(labels("cube")).toEqual(["Ramp", "Spectrum", "PV"]);
  });

  it("covers every rendered section except the log exactly once across the tabs, in column order", () => {
    const all = analysisSections(everything).map((s) => s.id);
    const tabbed = ANALYSIS_TABS.flatMap((t) => analysisTabSections(everything, t.id).map((s) => s.id));
    expect(tabbed).toHaveLength(all.length - 1);
    expect(new Set(tabbed)).toEqual(new Set(all.filter((id) => id !== ANALYSIS_SECTION.log.id)));
    for (const t of ANALYSIS_TABS) {
      const idx = analysisTabSections(everything, t.id).map((s) => all.indexOf(s.id));
      expect(idx).toEqual([...idx].sort((a, b) => a - b));
    }
  });

  it("keeps optional sections out of their tab when they are not rendered", () => {
    const bare = { hasHistogram: false, isCube: false, isRamp: false, showFft: false, showDeepZoom: false };
    expect(analysisTabSections(bare, "image").map((s) => s.label)).toEqual(["Stats", "Pixels", "Regions", "Profiles", "Contours"]);
    expect(analysisTabSections(bare, "cube")).toEqual([]);
    expect(analysisTabSections(cube, "cube").map((s) => s.label)).toEqual(["Spectrum", "PV"]);
    expect(analysisTabSections(rampOnly, "cube").map((s) => s.label)).toEqual(["Ramp"]);
  });

  it("opens cubes and ramps on the Cube tab and everything else on Image", () => {
    expect(defaultAnalysisTab({ isCube: false, isRamp: false })).toBe("image");
    expect(defaultAnalysisTab({ isCube: false })).toBe("image");
    expect(defaultAnalysisTab({ isCube: true, isRamp: false })).toBe("cube");
    expect(defaultAnalysisTab({ isCube: false, isRamp: true })).toBe("cube");
    expect(defaultAnalysisTab({ isCube: true, isRamp: true })).toBe("cube");
  });

  it("enables the Cube tab only for a cube or a ramp", () => {
    expect(tabAvailability(plain)).toEqual({ image: true, sources: true, cube: false });
    expect(tabAvailability({ ...plain, hasHistogram: false, showFft: false })).toEqual({ image: true, sources: true, cube: false });
    expect(tabAvailability(cube)).toEqual({ image: true, sources: true, cube: true });
    expect(tabAvailability(rampOnly)).toEqual({ image: true, sources: true, cube: true });
  });
});

describe("remembered analysis tab", () => {
  it("uses the default when nothing is remembered and falls back when the remembered tab is unavailable", () => {
    expect(resolveAnalysisTab(undefined, plain)).toBe("image");
    expect(resolveAnalysisTab(undefined, cube)).toBe("cube");
    expect(resolveAnalysisTab("sources", cube)).toBe("sources");
    expect(resolveAnalysisTab("cube", plain)).toBe("image");
    expect(resolveAnalysisTab("cube", rampOnly)).toBe("cube");
  });

  it("remembers the chosen tab per file key", () => {
    const memory = createAnalysisTabMemory();
    expect(memory.resolve("1|a.fits", plain)).toBe("image");
    expect(memory.resolve("2|cube.fits", cube)).toBe("cube");
    memory.remember("1|a.fits", "sources");
    memory.remember("2|cube.fits", "image");
    expect(memory.resolve("1|a.fits", plain)).toBe("sources");
    expect(memory.resolve("2|cube.fits", cube)).toBe("image");
    expect(memory.resolve("3|b.fits", plain)).toBe("image");
  });

  it("falls back to the default without forgetting a tab that becomes available again", () => {
    const memory = createAnalysisTabMemory();
    memory.remember("k", "cube");
    expect(memory.resolve("k", plain)).toBe("image");
    expect(memory.resolve("k", cube)).toBe("cube");
  });

  it("follows the default until the user picks a tab, so a late cube detection still lands on Cube", () => {
    const memory = createAnalysisTabMemory();
    expect(memory.resolve("k", plain)).toBe("image");
    expect(memory.resolve("k", cube)).toBe("cube");
  });

  it("keeps separate memories independent and treats a missing file key as one slot", () => {
    const a = createAnalysisTabMemory();
    const b = createAnalysisTabMemory();
    a.remember(null, "sources");
    expect(a.resolve(null, plain)).toBe("sources");
    expect(b.resolve(null, plain)).toBe("image");
  });
});

describe("tab keyboard navigation", () => {
  const all = { image: true, sources: true, cube: true };
  const noCube = { image: true, sources: true, cube: false };

  it("recognises only the roving-tabindex keys", () => {
    expect(["ArrowLeft", "ArrowRight", "Home", "End"].every(isTabNavKey)).toBe(true);
    expect(isTabNavKey("ArrowUp")).toBe(false);
    expect(isTabNavKey("Enter")).toBe(false);
  });

  it("wraps with the arrows and jumps with Home and End", () => {
    expect(nextAnalysisTab("image", "ArrowRight", all)).toBe("sources");
    expect(nextAnalysisTab("cube", "ArrowRight", all)).toBe("image");
    expect(nextAnalysisTab("image", "ArrowLeft", all)).toBe("cube");
    expect(nextAnalysisTab("sources", "Home", all)).toBe("image");
    expect(nextAnalysisTab("image", "End", all)).toBe("cube");
  });

  it("skips the disabled Cube tab", () => {
    expect(nextAnalysisTab("sources", "ArrowRight", noCube)).toBe("image");
    expect(nextAnalysisTab("image", "ArrowLeft", noCube)).toBe("sources");
    expect(nextAnalysisTab("image", "End", noCube)).toBe("sources");
  });

  it("moves from a focused disabled tab to its enabled neighbours", () => {
    expect(nextAnalysisTab("cube", "ArrowLeft", noCube)).toBe("sources");
    expect(nextAnalysisTab("cube", "ArrowRight", noCube)).toBe("image");
  });
});

describe("analysis tab panel attributes", () => {
  it("links each panel to its tab and hides only the inactive ones", () => {
    expect(analysisPanelAttributes("cube", "sources")).toEqual({
      role: "tabpanel",
      id: "analysis-panel-cube",
      "aria-labelledby": "analysis-tab-cube",
      "data-analysis-panel": "cube",
      hidden: true,
    });
    expect(analysisPanelAttributes("sources", "sources").hidden).toBe(false);
  });
});
