import { describe, it, expect } from "vitest";
import {
  ANALYSIS_SECTION,
  ANALYSIS_TOOL_IDS,
  CUBE_TOOL_EMPTY_TITLE,
  MIN_CHIP_SECTIONS,
  analysisRouteDue,
  analysisSections,
  analysisToolForFile,
  deepZoomAvailable,
  nextAnalysisRoute,
  rememberAnalysisChoice,
  showSectionChips,
  toolSections,
  type AnalysisRouteTool,
  type AnalysisSectionsInput,
} from "../analysisSections";
import { DEFAULT_DOCK_LAYOUT, DOCK_TOOL_IDS, dockReducer, type DockAction, type DockLayout } from "../dockLayout";

describe("deepZoomAvailable", () => {
  it("offers Deep Zoom only above 4096 pixels on either side", () => {
    expect(deepZoomAvailable(4096, 4096)).toBe(false);
    expect(deepZoomAvailable(4097, 100)).toBe(true);
    expect(deepZoomAvailable(100, 8192)).toBe(true);
    expect(deepZoomAvailable(undefined, undefined)).toBe(false);
  });
});

const everything: AnalysisSectionsInput = { hasHistogram: true, isCube: true, isRamp: true, showFft: true, showDeepZoom: true };
const nothing: AnalysisSectionsInput = { hasHistogram: false, isCube: false, isRamp: false, showFft: false, showDeepZoom: false };
const largeMono: AnalysisSectionsInput = { hasHistogram: true, isCube: false, isRamp: false, showFft: true, showDeepZoom: true };

const labels = (tool: Parameters<typeof toolSections>[0], input: AnalysisSectionsInput) => toolSections(tool, input).map((s) => s.label);

describe("analysis tools", () => {
  it("are the five dock tools that host analysis sections, in strip order", () => {
    expect(ANALYSIS_TOOL_IDS).toEqual(["image", "astrometry", "photometry", "cube", "log"]);
    for (const tool of ANALYSIS_TOOL_IDS) expect(DOCK_TOOL_IDS).toContain(tool);
  });
});

describe("toolSections", () => {
  it("lists the Image sections of a large mono frame in render order", () => {
    expect(labels("image", largeMono)).toEqual(["Histogram", "Stats", "Pixels", "Regions", "Profiles", "Contours", "FFT", "Deep Zoom"]);
  });

  it("drops the Image sections that are not rendered", () => {
    expect(labels("image", nothing)).toEqual(["Stats", "Pixels", "Regions", "Profiles", "Contours"]);
    expect(labels("image", { ...largeMono, showFft: false })).not.toContain("FFT");
    expect(labels("image", { ...largeMono, showDeepZoom: false })).not.toContain("Deep Zoom");
    expect(labels("image", { ...largeMono, hasHistogram: false })[0]).toBe("Stats");
  });

  it("lists the Astrometry sections whatever the file", () => {
    for (const input of [everything, nothing, largeMono]) {
      expect(labels("astrometry", input)).toEqual(["Stars", "Geometry", "Catalog", "Targets"]);
    }
  });

  it("lists the Photometry sections whatever the file", () => {
    for (const input of [everything, nothing, largeMono]) {
      expect(labels("photometry", input)).toEqual(["Photometry", "Table", "Series"]);
    }
  });

  it("gates the Cube sections on a ramp and on a cube", () => {
    expect(labels("cube", nothing)).toEqual([]);
    expect(labels("cube", { ...nothing, isCube: true })).toEqual(["Spectrum", "PV"]);
    expect(labels("cube", { ...nothing, isRamp: true })).toEqual(["Ramp"]);
    expect(labels("cube", everything)).toEqual(["Ramp", "Spectrum", "PV"]);
  });

  it("gives the Log tool only the log section", () => {
    for (const input of [everything, nothing]) expect(toolSections("log", input)).toEqual([ANALYSIS_SECTION.log]);
  });

  it("tags every listed section with the tool that renders it", () => {
    for (const tool of ANALYSIS_TOOL_IDS) {
      for (const section of toolSections(tool, everything)) expect(section.tool).toBe(tool);
    }
  });
});

describe("ANALYSIS_SECTION", () => {
  it("keeps today's element ids and labels", () => {
    expect(Object.fromEntries(Object.entries(ANALYSIS_SECTION).map(([key, s]) => [key, [s.id, s.label]]))).toEqual({
      histogram: ["analysis-histogram", "Histogram"],
      stars: ["analysis-stars", "Stars"],
      photometry: ["analysis-photometry", "Photometry"],
      table: ["analysis-table", "Table"],
      series: ["analysis-series", "Series"],
      geometry: ["analysis-geometry", "Geometry"],
      catalog: ["analysis-catalog", "Catalog"],
      targets: ["analysis-targets", "Targets"],
      statistics: ["analysis-statistics", "Stats"],
      pixels: ["analysis-pixels", "Pixels"],
      regions: ["analysis-regions", "Regions"],
      profiles: ["analysis-profiles", "Profiles"],
      contours: ["analysis-contours", "Contours"],
      fft: ["analysis-fft", "FFT"],
      ramp: ["analysis-ramp", "Ramp"],
      spectrum: ["analysis-spectrum", "Spectrum"],
      pv: ["analysis-pv", "PV"],
      deepZoom: ["analysis-deep-zoom", "Deep Zoom"],
      log: ["analysis-log", "Log"],
    });
  });

  it("puts every section in exactly one tool", () => {
    const all = analysisSections(everything);
    for (const section of Object.values(ANALYSIS_SECTION)) {
      const owners = ANALYSIS_TOOL_IDS.filter((tool) => toolSections(tool, everything).includes(section));
      expect(owners, section.id).toEqual([section.tool]);
      expect(all.filter((s) => s === section), section.id).toHaveLength(1);
    }
    expect(all).toHaveLength(Object.keys(ANALYSIS_SECTION).length);
  });

  it("gives every section a distinct element id across all tools", () => {
    const ids = analysisSections(everything).map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) expect(id).toMatch(/^analysis-[a-z-]+$/);
  });
});

describe("analysisSections", () => {
  it("concatenates the tools' sections in tool order", () => {
    expect(analysisSections(largeMono)).toEqual(ANALYSIS_TOOL_IDS.flatMap((tool) => toolSections(tool, largeMono)));
    expect(analysisSections(nothing).map((s) => s.label)).toEqual([
      "Stats", "Pixels", "Regions", "Profiles", "Contours",
      "Stars", "Geometry", "Catalog", "Targets",
      "Photometry", "Table", "Series",
      "Log",
    ]);
  });
});

describe("showSectionChips", () => {
  it("shows the chip nav from four sections up", () => {
    expect(MIN_CHIP_SECTIONS).toBe(4);
    const s = Object.values(ANALYSIS_SECTION);
    expect(showSectionChips([])).toBe(false);
    expect(showSectionChips(s.slice(0, 3))).toBe(false);
    expect(showSectionChips(s.slice(0, 4))).toBe(true);
    expect(showSectionChips(s.slice(0, 8))).toBe(true);
  });

  it("gives chips to Image and Astrometry and never to Photometry, Cube or Log", () => {
    expect(showSectionChips(toolSections("image", nothing))).toBe(true);
    expect(showSectionChips(toolSections("astrometry", nothing))).toBe(true);
    for (const tool of ["photometry", "cube", "log"] as const) expect(showSectionChips(toolSections(tool, everything))).toBe(false);
  });
});

describe("analysisToolForFile", () => {
  it("opens Cube for a cube or a ramp and Image for anything else", () => {
    expect(analysisToolForFile({ isCube: true })).toBe("cube");
    expect(analysisToolForFile({ isCube: false, isRamp: true })).toBe("cube");
    expect(analysisToolForFile({ isCube: true, isRamp: true })).toBe("cube");
    expect(analysisToolForFile({ isCube: false, isRamp: false })).toBe("image");
    expect(analysisToolForFile({ isCube: false })).toBe("image");
  });
});

describe("CUBE_TOOL_EMPTY_TITLE", () => {
  it("tells what the Cube tool needs", () => {
    expect(CUBE_TOOL_EMPTY_TITLE).toBe("Open a data cube or a ramp");
  });
});

const after = (...actions: DockAction[]): DockLayout => actions.reduce(dockReducer, DEFAULT_DOCK_LAYOUT);
const memoryOf = (entries: [string, AnalysisRouteTool][] = []) => new Map<string, AnalysisRouteTool>(entries);
const plain = { isCube: false, isRamp: false };
const cube = { isCube: true, isRamp: false };
const rampOnly = { isCube: false, isRamp: true };

describe("nextAnalysisRoute", () => {
  it("swaps Image for Cube in the shared anchor when a cube is selected", () => {
    const layout = after({ type: "open", tool: "image" });
    expect(nextAnalysisRoute({ layout, fileKey: "c", ...cube, memory: memoryOf() })).toEqual([{ anchor: "right-top", tool: "cube" }]);
  });

  it("routes a ramp to Cube like a cube", () => {
    const layout = after({ type: "open", tool: "image" });
    expect(nextAnalysisRoute({ layout, fileKey: "r", ...rampOnly, memory: memoryOf() })).toEqual([{ anchor: "right-top", tool: "cube" }]);
  });

  it("swaps Cube for Image when a plain image is selected", () => {
    const layout = after({ type: "open", tool: "cube" });
    expect(nextAnalysisRoute({ layout, fileKey: "p", ...plain, memory: memoryOf() })).toEqual([{ anchor: "right-top", tool: "image" }]);
  });

  it("does nothing when the desired tool is already open", () => {
    expect(nextAnalysisRoute({ layout: after({ type: "open", tool: "image" }), fileKey: "p", ...plain, memory: memoryOf() })).toEqual([]);
    expect(nextAnalysisRoute({ layout: after({ type: "open", tool: "cube" }), fileKey: "c", ...cube, memory: memoryOf() })).toEqual([]);
  });

  it("never opens a tool when neither Image nor Cube is open", () => {
    expect(nextAnalysisRoute({ layout: DEFAULT_DOCK_LAYOUT, fileKey: "c", ...cube, memory: memoryOf() })).toEqual([]);
    expect(nextAnalysisRoute({ layout: after({ type: "open", tool: "photometry" }), fileKey: "c", ...cube, memory: memoryOf() })).toEqual([]);
    expect(nextAnalysisRoute({ layout: after({ type: "open", tool: "astrometry" }), fileKey: "p", ...plain, memory: memoryOf([["p", "cube"]]) })).toEqual([]);
  });

  it("does nothing without a selected file", () => {
    expect(nextAnalysisRoute({ layout: after({ type: "open", tool: "image" }), fileKey: null, ...cube, memory: memoryOf() })).toEqual([]);
  });

  it("prefers the last explicit choice for the file over the default", () => {
    expect(nextAnalysisRoute({ layout: after({ type: "open", tool: "image" }), fileKey: "c", ...cube, memory: memoryOf([["c", "image"]]) })).toEqual([]);
    expect(nextAnalysisRoute({ layout: after({ type: "open", tool: "cube" }), fileKey: "c", ...cube, memory: memoryOf([["c", "image"]]) })).toEqual([
      { anchor: "right-top", tool: "image" },
    ]);
  });

  it("keys the remembered choice by file", () => {
    const layout = after({ type: "open", tool: "image" });
    expect(nextAnalysisRoute({ layout, fileKey: "d", ...cube, memory: memoryOf([["c", "image"]]) })).toEqual([{ anchor: "right-top", tool: "cube" }]);
  });

  it("falls back to Image when Cube is remembered but the file is neither a cube nor a ramp", () => {
    expect(nextAnalysisRoute({ layout: after({ type: "open", tool: "image" }), fileKey: "p", ...plain, memory: memoryOf([["p", "cube"]]) })).toEqual([]);
    expect(nextAnalysisRoute({ layout: after({ type: "open", tool: "cube" }), fileKey: "p", ...plain, memory: memoryOf([["p", "cube"]]) })).toEqual([
      { anchor: "right-top", tool: "image" },
    ]);
  });

  it("opens the desired tool in its own anchor and leaves the other anchor alone", () => {
    const split = after({ type: "move", tool: "cube", anchor: "left-bottom" }, { type: "open", tool: "compose" }, { type: "open", tool: "image" });
    expect(nextAnalysisRoute({ layout: split, fileKey: "c", ...cube, memory: memoryOf() })).toEqual([{ anchor: "left-bottom", tool: "cube" }]);
    const both = dockReducer(split, { type: "open", tool: "cube" });
    expect(nextAnalysisRoute({ layout: both, fileKey: "c", ...cube, memory: memoryOf() })).toEqual([]);
    expect(nextAnalysisRoute({ layout: both, fileKey: "p", ...plain, memory: memoryOf() })).toEqual([]);
  });

  it("routes from an open Cube in another anchor to Image in its own anchor", () => {
    const layout = after({ type: "move", tool: "cube", anchor: "left-bottom" });
    expect(layout.active["right-top"]).toBe(null);
    expect(nextAnalysisRoute({ layout, fileKey: "p", ...plain, memory: memoryOf() })).toEqual([{ anchor: "right-top", tool: "image" }]);
  });
});

describe("rememberAnalysisChoice", () => {
  it("remembers Image or Cube when the user opens it for the selected file", () => {
    const memory = memoryOf();
    const closed = DEFAULT_DOCK_LAYOUT;
    const image = dockReducer(closed, { type: "open", tool: "image" });
    rememberAnalysisChoice(memory, "a", closed.active, image.active);
    expect(memory.get("a")).toBe("image");
    const cubeOpen = dockReducer(image, { type: "toggle", tool: "cube" });
    rememberAnalysisChoice(memory, "a", image.active, cubeOpen.active);
    expect(memory.get("a")).toBe("cube");
    expect(memory.size).toBe(1);
  });

  it("ignores closing, moving an already open tool and opening other tools", () => {
    const memory = memoryOf([["a", "cube"]]);
    const image = after({ type: "open", tool: "image" });
    rememberAnalysisChoice(memory, "a", image.active, dockReducer(image, { type: "hide", tool: "image" }).active);
    rememberAnalysisChoice(memory, "a", image.active, dockReducer(image, { type: "move", tool: "image", anchor: "left-bottom" }).active);
    rememberAnalysisChoice(memory, "a", image.active, dockReducer(image, { type: "open", tool: "photometry" }).active);
    rememberAnalysisChoice(memory, "a", image.active, image.active);
    expect(memory.get("a")).toBe("cube");
  });

  it("remembers nothing without a selected file", () => {
    const memory = memoryOf();
    rememberAnalysisChoice(memory, null, DEFAULT_DOCK_LAYOUT.active, after({ type: "open", tool: "image" }).active);
    expect(memory.size).toBe(0);
  });
});

describe("analysisRouteDue", () => {
  it("decides once per file, only after the cube flags describe that file", () => {
    expect(analysisRouteDue({ fileKey: "b", flagsKey: "a", routedKey: "a" })).toBe(false);
    expect(analysisRouteDue({ fileKey: "b", flagsKey: "b", routedKey: "a" })).toBe(true);
    expect(analysisRouteDue({ fileKey: "b", flagsKey: "b", routedKey: "b" })).toBe(false);
    expect(analysisRouteDue({ fileKey: "b", flagsKey: "b", routedKey: null })).toBe(true);
    expect(analysisRouteDue({ fileKey: null, flagsKey: "a", routedKey: "a" })).toBe(false);
    expect(analysisRouteDue({ fileKey: null, flagsKey: null, routedKey: null })).toBe(false);
  });
});
