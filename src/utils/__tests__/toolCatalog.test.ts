import { describe, expect, it } from "vitest";
import { DOCK_TOOL_IDS } from "../dockLayout";
import { DEFAULT_TOOL_SHORTCUTS, TOOL_CATEGORIES, TOOL_DESCRIPTIONS, initialToolShortcuts, parseToolShortcuts, toggleToolShortcut, toolCategoryColor } from "../toolCatalog";

function storageWith(entries: Record<string, string>) {
  const data = new Map(Object.entries(entries));
  return { getItem: (key: string) => data.get(key) ?? null };
}

const throwingStorage = {
  getItem(): string | null {
    throw new Error("SecurityError");
  },
};

describe("Tool catalog", () => {
  it("categorizes every tool exactly once with a description", () => {
    expect(TOOL_CATEGORIES.flatMap((category) => [...category.tools]).sort()).toEqual([...DOCK_TOOL_IDS].sort());
    for (const tool of DOCK_TOOL_IDS) {
      expect(TOOL_DESCRIPTIONS[tool].length).toBeGreaterThan(20);
      expect(toolCategoryColor(tool)).toMatch(/^#[0-9a-f]{6}$/);
    }
  });

  it("describes the measurement tools by what they measure", () => {
    expect(TOOL_DESCRIPTIONS.astrometry).toBe("Solve sky coordinates, detect sources and cross-match them with Gaia and other catalogues.");
    expect(TOOL_DESCRIPTIONS.photometry).toBe("Measure aperture flux and magnitudes, multi-band photometry, image depth and source size, and build light curves.");
    expect(TOOL_DESCRIPTIONS.cube).toBe("Measure 1D spectra (x1d, CSV), time-series spectra and IFU cubes: line fits, white light and moment maps.");
    expect(TOOL_DESCRIPTIONS.cubeprep).toBe("Match units, pixel grids and PSFs across bands for multiband work.");
    expect(TOOL_DESCRIPTIONS.sagui).toBe("Segment multiband images into regions and measure their spectral energy distributions.");
  });

  it("names the categories by what the tools do", () => {
    expect(TOOL_CATEGORIES.map((category) => [category.label, [...category.tools]])).toEqual([
      ["Inspect", ["image", "headers", "info"]],
      ["Measure", ["astrometry", "photometry", "cube", "sagui", "log"]],
      ["Enhance", ["processing", "masks", "history"]],
      ["Combine", ["stacking", "compose", "cubeprep", "synth"]],
      ["Files and output", ["files", "export", "config"]],
    ]);
  });

  it("starts with common shortcuts when preferences are absent or corrupt", () => {
    for (const raw of [null, "invalid", "null", "{}", "true"]) expect(parseToolShortcuts(raw)).toEqual(DEFAULT_TOOL_SHORTCUTS);
  });

  it("pins the measurement tools and the Log by default, in canonical order", () => {
    expect(DEFAULT_TOOL_SHORTCUTS).toEqual(["files", "info", "compose", "headers", "image", "astrometry", "photometry", "cube", "processing", "history", "export", "config", "log"]);
    expect(DOCK_TOOL_IDS.filter((tool) => DEFAULT_TOOL_SHORTCUTS.includes(tool))).toEqual(DEFAULT_TOOL_SHORTCUTS);
  });

  it("preserves intentionally hidden shortcuts including an empty sidebar", () => {
    expect(parseToolShortcuts("[]")).toEqual([]);
    expect(parseToolShortcuts('["cube","config"]')).toEqual(["cube", "config"]);
  });

  it("returns a stored list in canonical order, the order enabling a shortcut produces", () => {
    expect(parseToolShortcuts('["log","image","files"]')).toEqual(["files", "image", "log"]);
    expect(toggleToolShortcut(["image", "files"], "headers", false)).toEqual(["files", "headers", "image"]);
  });

  it("ignores unknown and duplicate stored shortcuts", () => {
    expect(parseToolShortcuts('["image",null,"removed-tool","image",3,"log"]')).toEqual(["image", "log"]);
  });

  it("enables and disables a shortcut without modifying other selections", () => {
    const before = ["files", "image"] as const;
    const enabled = toggleToolShortcut(before, "cube", false);
    expect(enabled).toEqual(["files", "image", "cube"]);
    expect(toggleToolShortcut(enabled, "cube", true)).toEqual(before);
    expect(before).toEqual(["files", "image"]);
  });

  it("can disable an open tool that was not previously pinned", () => {
    expect(toggleToolShortcut(["files"], "astrometry", true)).toEqual(["files"]);
  });
});

describe("initialToolShortcuts", () => {
  it("gives a fresh profile the default shortcuts", () => {
    expect(initialToolShortcuts(storageWith({}))).toEqual(DEFAULT_TOOL_SHORTCUTS);
  });

  it("seeds every tool for a profile upgraded from a release without shortcuts", () => {
    expect(initialToolShortcuts(storageWith({ "ab.layout.dock.v1": "{}" }))).toEqual([...DOCK_TOOL_IDS]);
  });

  it("keeps the stored shortcuts once there are any, with or without a saved layout", () => {
    expect(initialToolShortcuts(storageWith({ "ab.tools.shortcuts.v1": '["config","cube"]', "ab.layout.dock.v1": "{}" }))).toEqual(["cube", "config"]);
    expect(initialToolShortcuts(storageWith({ "ab.tools.shortcuts.v1": "[]" }))).toEqual([]);
  });

  it("falls back to the defaults when storage is missing or throws", () => {
    expect(initialToolShortcuts(null)).toEqual(DEFAULT_TOOL_SHORTCUTS);
    expect(initialToolShortcuts(throwingStorage)).toEqual(DEFAULT_TOOL_SHORTCUTS);
  });
});
