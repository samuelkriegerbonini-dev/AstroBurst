import { describe, it, expect } from "vitest";
import {
  ANCHOR_LABELS,
  COLUMN_VW_CAP_DOUBLE,
  COLUMN_VW_CAP_SINGLE,
  DEFAULT_DOCK_LAYOUT,
  DOCK_ANCHORS,
  DOCK_SIZE_LIMITS,
  DOCK_TOOL_IDS,
  DOCK_TOOL_META,
  MIN_PREVIEW_H,
  MIN_PREVIEW_W,
  anchorOf,
  anchorOpenTool,
  anchorOrientation,
  anchorSide,
  clampSize,
  columnMin,
  columnRenderWidth,
  dockLayoutErrors,
  dockReducer,
  groupActive,
  indexOf,
  isToolOpen,
  keptToolFileKey,
  moveLabel,
  otherBottom,
  toolMountState,
  type DockAction,
  type DockAnchor,
  type DockLayout,
  type DockSizes,
  type ToolMountInput,
} from "../dockLayout";

function withActive(active: Partial<DockLayout["active"]>, base: DockLayout = DEFAULT_DOCK_LAYOUT): DockLayout {
  return { ...base, active: { ...base.active, ...active } };
}

function withSizes(sizes: Partial<DockSizes>, base: DockLayout = DEFAULT_DOCK_LAYOUT): DockLayout {
  return { ...base, sizes: { ...base.sizes, ...sizes } };
}

describe("DEFAULT_DOCK_LAYOUT", () => {
  it("is the owner's default: Files and Info left top, Compose bottom left, the seven right tools split top/bottom", () => {
    expect(DEFAULT_DOCK_LAYOUT.anchors).toEqual({
      "left-top": ["files", "info"],
      "left-bottom": ["compose"],
      "right-top": ["headers", "image", "astrometry", "photometry", "cube", "processing", "stacking"],
      "right-bottom": ["synth", "export", "config", "log"],
    });
    expect(anchorOf(DEFAULT_DOCK_LAYOUT, "log")).toBe("right-bottom");
  });

  it("starts with Files and Compose open and both right anchors closed", () => {
    expect(DEFAULT_DOCK_LAYOUT.active).toEqual({ "left-top": "files", "left-bottom": "compose", "right-top": null, "right-bottom": null });
  });

  it("copies today's sizes", () => {
    expect(DEFAULT_DOCK_LAYOUT.sizes).toEqual({ leftW: 300, rightW: 380, bottomH: 280, bottomSplit: 0.5 });
    for (const key of Object.keys(DOCK_SIZE_LIMITS) as (keyof DockSizes)[]) {
      expect(DEFAULT_DOCK_LAYOUT.sizes[key]).toBe(DOCK_SIZE_LIMITS[key].default);
    }
    expect([MIN_PREVIEW_W, MIN_PREVIEW_H, COLUMN_VW_CAP_SINGLE, COLUMN_VW_CAP_DOUBLE]).toEqual([320, 200, 60, 40]);
  });

  it("has no invariant errors", () => {
    expect(dockLayoutErrors(DEFAULT_DOCK_LAYOUT)).toEqual([]);
  });

  it("is frozen down to the anchor lists, so an in-place change throws instead of corrupting later resets", () => {
    const parts: object[] = [DEFAULT_DOCK_LAYOUT, DEFAULT_DOCK_LAYOUT.anchors, DEFAULT_DOCK_LAYOUT.active, DEFAULT_DOCK_LAYOUT.sizes];
    for (const anchor of DOCK_ANCHORS) parts.push(DEFAULT_DOCK_LAYOUT.anchors[anchor]);
    for (const part of parts) expect(Object.isFrozen(part)).toBe(true);
    const reset = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "reset" });
    expect(() => reset.anchors["left-top"].push("config")).toThrow(TypeError);
    expect(() => reset.anchors["right-top"].sort()).toThrow(TypeError);
    expect(() => { reset.active["right-top"] = "image"; }).toThrow(TypeError);
    expect(() => { reset.sizes.leftW = 500; }).toThrow(TypeError);
    expect(DEFAULT_DOCK_LAYOUT.anchors["left-top"]).toEqual(["files", "info"]);
    expect(DEFAULT_DOCK_LAYOUT.active["right-top"]).toBeNull();
    expect(DEFAULT_DOCK_LAYOUT.sizes.leftW).toBe(300);
  });
});

describe("anchors", () => {
  it("uses the owner's labels while keeping the anchor ids", () => {
    expect(DOCK_ANCHORS).toEqual(["left-top", "left-bottom", "right-top", "right-bottom"]);
    expect(ANCHOR_LABELS).toEqual({
      "left-top": "Left Top",
      "right-top": "Right Top",
      "left-bottom": "Bottom Left",
      "right-bottom": "Bottom Right",
    });
  });

  it("builds the move labels from the anchor labels", () => {
    expect(DOCK_ANCHORS.map(moveLabel)).toEqual(["Move to Left Top", "Move to Bottom Left", "Move to Right Top", "Move to Bottom Right"]);
  });

  it("maps each anchor to its strip side and orientation", () => {
    expect(DOCK_ANCHORS.map(anchorSide)).toEqual(["left", "left", "right", "right"]);
    expect(DOCK_ANCHORS.map(anchorOrientation)).toEqual(["vertical", "horizontal", "vertical", "horizontal"]);
    expect(otherBottom("left-bottom")).toBe("right-bottom");
    expect(otherBottom("right-bottom")).toBe("left-bottom");
  });
});

describe("DOCK_TOOL_META", () => {
  it("has every tool exactly once under its own id", () => {
    expect(Object.keys(DOCK_TOOL_META).sort()).toEqual([...DOCK_TOOL_IDS].sort());
    for (const id of DOCK_TOOL_IDS) expect(DOCK_TOOL_META[id].id).toBe(id);
    expect(new Set(DOCK_TOOL_IDS).size).toBe(DOCK_TOOL_IDS.length);
  });

  it("keeps today's strip labels and panel labels", () => {
    expect(DOCK_TOOL_IDS.map((id) => DOCK_TOOL_META[id].shortLabel)).toEqual([
      "Files", "Info", "Comp", "Headers", "Image", "Astro", "Phot", "Cube", "Proc", "Stack", "Synth", "Export", "Config", "Log",
    ]);
    expect(DOCK_TOOL_IDS.map((id) => DOCK_TOOL_META[id].label)).toEqual([
      "Files", "Info", "Compose", "Headers", "Image", "Astrometry", "Photometry", "Cube", "Processing", "Stacking", "Synth", "Export", "Settings", "Log",
    ]);
    expect(DOCK_TOOL_META.config.keywords).toEqual(["Config"]);
  });

  it("lists the tools in strip order with the analysis tools in the slot Analysis had", () => {
    expect(DOCK_TOOL_IDS).toEqual([
      "files", "info", "compose", "headers", "image", "astrometry", "photometry", "cube", "processing", "stacking", "synth", "export", "config", "log",
    ]);
    expect((DOCK_TOOL_IDS as readonly string[]).includes("analysis")).toBe(false);
  });

  it("gives the analysis tools their accents", () => {
    expect((["image", "astrometry", "photometry", "cube", "log"] as const).map((id) => DOCK_TOOL_META[id].accent)).toEqual([
      "var(--ab-blue)", "var(--ab-violet)", "var(--ab-amber)", "var(--ab-cyan)", "#a1a1aa",
    ]);
  });

  it("keeps Analysis as a palette keyword of every analysis tool", () => {
    expect(DOCK_TOOL_META.image.keywords).toEqual(["Analysis", "Histogram", "STF", "Statistics", "Pixels", "Regions", "Profiles", "Contours", "FFT", "Deep Zoom"]);
    expect(DOCK_TOOL_META.astrometry.keywords).toEqual(["Analysis", "Stars", "Star detection", "Plate solve", "WCS", "Catalog", "Targets", "Geometry"]);
    expect(DOCK_TOOL_META.photometry.keywords).toEqual(["Analysis", "Aperture", "Photometry table", "Time series", "Light curve"]);
    expect(DOCK_TOOL_META.cube.keywords).toEqual(["Analysis", "Spectrum", "Spectroscopy", "Ramp", "PV", "Line fit", "Moments"]);
    expect(DOCK_TOOL_META.log.keywords).toEqual(["Analysis", "Measurement log", "Measurements", "CSV"]);
  });

  it("keeps the four analysis tools per file, the Log while open and only Files always mounted", () => {
    expect(DOCK_TOOL_IDS.filter((id) => DOCK_TOOL_META[id].mount === "perFile")).toEqual(["image", "astrometry", "photometry", "cube"]);
    expect(DOCK_TOOL_IDS.filter((id) => DOCK_TOOL_META[id].mount === "always")).toEqual(["files"]);
    expect(DOCK_TOOL_META.log.mount).toBe("whileOpen");
  });

  it("puts exactly the four per-file analysis tools in the analysis keep group", () => {
    expect(DOCK_TOOL_IDS.filter((id) => DOCK_TOOL_META[id].keepGroup === "analysis")).toEqual(["image", "astrometry", "photometry", "cube"]);
    for (const id of DOCK_TOOL_IDS) {
      if (DOCK_TOOL_META[id].keepGroup !== "analysis") expect(DOCK_TOOL_META[id].keepGroup, id).toBeUndefined();
    }
  });

  it("lets only Files, Info and Log work without a file", () => {
    expect(DOCK_TOOL_IDS.filter((id) => !DOCK_TOOL_META[id].needsFile)).toEqual(["files", "info", "log"]);
  });

  it("gives Files and Info the sidebar minimum and every other tool the right-column minimum", () => {
    expect(DOCK_TOOL_IDS.map((id) => DOCK_TOOL_META[id].minWidth)).toEqual([180, 180, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280]);
  });
});

describe("dockReducer move to another anchor", () => {
  it("appends to the target, opens the tool there and leaves the source's open tool alone", () => {
    const next = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "image", anchor: "left-bottom" });
    expect(next.anchors["left-bottom"]).toEqual(["compose", "image"]);
    expect(next.anchors["right-top"]).toEqual(["headers", "astrometry", "photometry", "cube", "processing", "stacking"]);
    expect(next.active).toEqual({ "left-top": "files", "left-bottom": "image", "right-top": null, "right-bottom": null });
    expect(dockLayoutErrors(next)).toEqual([]);
  });

  it("inserts at the given index and closes the source when the moved tool was open there", () => {
    const next = dockReducer(withActive({ "right-top": "image" }), { type: "move", tool: "image", anchor: "left-top", index: 1 });
    expect(next.anchors["left-top"]).toEqual(["files", "image", "info"]);
    expect(next.active["left-top"]).toBe("image");
    expect(next.active["right-top"]).toBeNull();
  });

  it("keeps the source's other open tool open", () => {
    const next = dockReducer(withActive({ "right-top": "processing" }), { type: "move", tool: "image", anchor: "right-bottom", index: 0 });
    expect(next.anchors["right-bottom"]).toEqual(["image", "synth", "export", "config", "log"]);
    expect(next.active["right-top"]).toBe("processing");
    expect(next.active["right-bottom"]).toBe("image");
  });

  it("clamps the index into the target list", () => {
    expect(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "files", anchor: "right-bottom", index: 99 }).anchors["right-bottom"])
      .toEqual(["synth", "export", "config", "log", "files"]);
    expect(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "files", anchor: "right-bottom", index: -5 }).anchors["right-bottom"])
      .toEqual(["files", "synth", "export", "config", "log"]);
  });

  it("can empty an anchor", () => {
    const next = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "compose", anchor: "right-bottom" });
    expect(next.anchors["left-bottom"]).toEqual([]);
    expect(next.active["left-bottom"]).toBeNull();
    expect(next.active["right-bottom"]).toBe("compose");
    expect(dockLayoutErrors(next)).toEqual([]);
  });

  it("does not mutate its input", () => {
    const before = JSON.stringify(DEFAULT_DOCK_LAYOUT);
    dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "image", anchor: "left-top", index: 0 });
    expect(JSON.stringify(DEFAULT_DOCK_LAYOUT)).toBe(before);
  });
});

describe("dockReducer move inside the same anchor", () => {
  const open = withActive({ "right-top": "processing" });

  it("never changes the open tool and equals reorder", () => {
    const moved = dockReducer(open, { type: "move", tool: "headers", anchor: "right-top", index: 3 });
    expect(moved.anchors["right-top"]).toEqual(["image", "astrometry", "photometry", "headers", "cube", "processing", "stacking"]);
    expect(moved.active).toEqual(open.active);
    expect(moved).toEqual(dockReducer(open, { type: "reorder", anchor: "right-top", from: 0, to: 3 }));
  });

  it("does not open a closed dragged tool", () => {
    const moved = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "stacking", anchor: "right-top", index: 0 });
    expect(moved.anchors["right-top"]).toEqual(["stacking", "headers", "image", "astrometry", "photometry", "cube", "processing"]);
    expect(moved.active["right-top"]).toBeNull();
  });

  it("returns the same object without an index", () => {
    expect(dockReducer(open, { type: "move", tool: "headers", anchor: "right-top" })).toBe(open);
  });

  it("returns the same object when the index is the current one", () => {
    expect(dockReducer(open, { type: "move", tool: "image", anchor: "right-top", index: 1 })).toBe(open);
  });

  it("clamps the index to the last position", () => {
    expect(dockReducer(open, { type: "move", tool: "headers", anchor: "right-top", index: 99 }).anchors["right-top"])
      .toEqual(["image", "astrometry", "photometry", "cube", "processing", "stacking", "headers"]);
  });
});

describe("dockReducer reorder", () => {
  it("moves an item inside one anchor without touching the open tools", () => {
    const layout = withActive({ "right-top": "image" });
    const next = dockReducer(layout, { type: "reorder", anchor: "right-top", from: 6, to: 0 });
    expect(next.anchors["right-top"]).toEqual(["stacking", "headers", "image", "astrometry", "photometry", "cube", "processing"]);
    expect(next.active).toEqual(layout.active);
  });

  it("returns the same object out of bounds and when from equals to", () => {
    for (const [from, to] of [[-1, 0], [0, 7], [7, 0], [1, 1], [0.5, 2]]) {
      expect(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "reorder", anchor: "right-top", from, to })).toBe(DEFAULT_DOCK_LAYOUT);
    }
  });
});

describe("dockReducer open state", () => {
  it("toggles a tool in its own anchor and replaces the other open tool", () => {
    const opened = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "toggle", tool: "headers" });
    expect(opened.active["right-top"]).toBe("headers");
    const replaced = dockReducer(opened, { type: "toggle", tool: "image" });
    expect(replaced.active["right-top"]).toBe("image");
    expect(dockReducer(replaced, { type: "toggle", tool: "image" }).active["right-top"]).toBeNull();
  });

  it("opens idempotently", () => {
    const opened = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "open", tool: "info" });
    expect(opened.active["left-top"]).toBe("info");
    expect(dockReducer(opened, { type: "open", tool: "info" })).toBe(opened);
  });

  it("hides only the open tool", () => {
    expect(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "hide", tool: "info" })).toBe(DEFAULT_DOCK_LAYOUT);
    expect(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "hide", tool: "files" }).active["left-top"]).toBeNull();
  });

  it("closes an anchor and returns the same object when it is already closed", () => {
    expect(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "closeAnchor", anchor: "left-bottom" }).active["left-bottom"]).toBeNull();
    expect(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "closeAnchor", anchor: "right-top" })).toBe(DEFAULT_DOCK_LAYOUT);
  });

  it("follows a tool to its new anchor", () => {
    const moved = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "headers", anchor: "left-bottom" });
    const hidden = dockReducer(moved, { type: "hide", tool: "headers" });
    expect(hidden.active["left-bottom"]).toBeNull();
    expect(dockReducer(hidden, { type: "toggle", tool: "headers" }).active["left-bottom"]).toBe("headers");
  });
});

describe("dockReducer resize and reset", () => {
  it("clamps every size to its limits", () => {
    const resize = (key: keyof DockSizes, value: number) => dockReducer(DEFAULT_DOCK_LAYOUT, { type: "resize", key, value }).sizes[key];
    expect(resize("leftW", 50)).toBe(180);
    expect(resize("leftW", 900)).toBe(640);
    expect(resize("rightW", 100)).toBe(280);
    expect(resize("rightW", 700)).toBe(640);
    expect(resize("bottomH", 10)).toBe(140);
    expect(resize("bottomH", 1000)).toBe(600);
    expect(resize("bottomSplit", 0.1)).toBe(0.25);
    expect(resize("bottomSplit", 0.9)).toBe(0.75);
  });

  it("rounds pixel sizes to integers and the split to three decimals", () => {
    expect(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "resize", key: "leftW", value: 300.6 }).sizes.leftW).toBe(301);
    expect(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "resize", key: "bottomSplit", value: 1 / 3 }).sizes.bottomSplit).toBe(0.333);
  });

  it("returns the same object when the clamped size does not change", () => {
    expect(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "resize", key: "rightW", value: 380.2 })).toBe(DEFAULT_DOCK_LAYOUT);
    expect(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "resize", key: "rightW", value: 380, persist: false })).toBe(DEFAULT_DOCK_LAYOUT);
  });

  it("maps NaN to the default and infinities to the limits", () => {
    expect(clampSize("bottomH", Number.NaN)).toBe(280);
    expect(clampSize("bottomH", Infinity)).toBe(600);
    expect(clampSize("bottomH", -Infinity)).toBe(140);
  });

  it("resets to the default layout", () => {
    const changed = dockReducer(withSizes({ leftW: 500 }, withActive({ "right-top": "image" })), { type: "move", tool: "files", anchor: "right-bottom" });
    expect(dockReducer(changed, { type: "reset" })).toBe(DEFAULT_DOCK_LAYOUT);
  });
});

describe("layout queries", () => {
  it("finds a tool's anchor, index and open state", () => {
    expect(anchorOf(DEFAULT_DOCK_LAYOUT, "processing")).toBe("right-top");
    expect(indexOf(DEFAULT_DOCK_LAYOUT, "processing")).toBe(5);
    expect(anchorOf(DEFAULT_DOCK_LAYOUT, "config")).toBe("right-bottom");
    expect(indexOf(DEFAULT_DOCK_LAYOUT, "config")).toBe(2);
    expect(isToolOpen(DEFAULT_DOCK_LAYOUT, "files")).toBe(true);
    expect(isToolOpen(DEFAULT_DOCK_LAYOUT, "info")).toBe(false);
    expect(isToolOpen(DEFAULT_DOCK_LAYOUT, "compose")).toBe(true);
    const moved = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "info", anchor: "right-top", index: 0 });
    expect(anchorOf(moved, "info")).toBe("right-top");
    expect(indexOf(moved, "info")).toBe(0);
    expect(isToolOpen(moved, "info")).toBe(true);
  });

  it("picks the open tool's minimum width for a column and the limit when it is closed", () => {
    expect(columnMin(DEFAULT_DOCK_LAYOUT, "left-top")).toBe(180);
    expect(columnMin(DEFAULT_DOCK_LAYOUT, "right-top")).toBe(280);
    expect(columnMin(withActive({ "left-top": null }), "left-top")).toBe(180);
    const imageLeft = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "image", anchor: "left-top" });
    expect(columnMin(imageLeft, "left-top")).toBe(280);
    const filesRight = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "files", anchor: "right-top" });
    expect(columnMin(filesRight, "right-top")).toBe(280);
  });

  it("renders a column at least at its minimum, at most at its maximum and the viewport cap", () => {
    const base = { min: 280, max: 640, dockW: 1280, bothOpen: false };
    expect(columnRenderWidth({ ...base, size: 180 })).toBe(280);
    expect(columnRenderWidth({ ...base, size: 400 })).toBe(400);
    expect(columnRenderWidth({ ...base, size: 640 })).toBe(640);
    expect(columnRenderWidth({ ...base, size: 640, bothOpen: true })).toBe(512);
    expect(columnRenderWidth({ ...base, size: 900, dockW: 3000 })).toBe(640);
    expect(columnRenderWidth({ ...base, size: 640, dockW: 1000 })).toBe(600);
    expect(columnRenderWidth({ ...base, size: 640, dockW: 1000, bothOpen: true })).toBe(400);
  });

  it("does not open a tool that needs a file when there is none", () => {
    expect(anchorOpenTool(DEFAULT_DOCK_LAYOUT, "left-top", false)).toBe("files");
    expect(anchorOpenTool(DEFAULT_DOCK_LAYOUT, "left-bottom", false)).toBeNull();
    expect(anchorOpenTool(DEFAULT_DOCK_LAYOUT, "left-bottom", true)).toBe("compose");
    expect(anchorOpenTool(DEFAULT_DOCK_LAYOUT, "right-top", true)).toBeNull();
    expect(anchorOpenTool(withActive({ "left-top": "info" }), "left-top", false)).toBe("info");
  });
});

describe("dockLayoutErrors", () => {
  it("reports duplicated, missing and unknown tools", () => {
    const dup: DockLayout = { ...DEFAULT_DOCK_LAYOUT, anchors: { ...DEFAULT_DOCK_LAYOUT.anchors, "left-bottom": ["compose", "files"] } };
    expect(dockLayoutErrors(dup)).toEqual(["tool files appears 2 times"]);
    const missing: DockLayout = { ...DEFAULT_DOCK_LAYOUT, anchors: { ...DEFAULT_DOCK_LAYOUT.anchors, "right-bottom": ["synth", "export"] } };
    expect(dockLayoutErrors(missing)).toEqual(["tool config appears 0 times", "tool log appears 0 times"]);
    const unknown = { ...DEFAULT_DOCK_LAYOUT, anchors: { ...DEFAULT_DOCK_LAYOUT.anchors, "left-bottom": ["compose", "nope"] } } as unknown as DockLayout;
    expect(dockLayoutErrors(unknown)).toEqual(["unknown tool nope at left-bottom"]);
  });

  it("reports an open tool that is not docked in that anchor", () => {
    expect(dockLayoutErrors(withActive({ "right-top": "files" }))).toEqual(["active tool files at right-top is not docked there"]);
  });

  it("reports sizes outside the limits", () => {
    expect(dockLayoutErrors(withSizes({ bottomH: 100 }))).toEqual(["size bottomH 100 outside 140..600"]);
    expect(dockLayoutErrors(withSizes({ bottomSplit: Number.NaN }))).toHaveLength(1);
  });
});

function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const SIZE_KEYS = ["leftW", "rightW", "bottomH", "bottomSplit"] as const;
const RANDOM_SIZES = [Number.NaN, -50, 0, 0.1, 0.5, 0.9, 150, 333.7, 1000, Infinity, -Infinity];

function randomAction(rand: () => number, layout: DockLayout): DockAction {
  const pick = <T>(xs: readonly T[]): T => xs[Math.floor(rand() * xs.length)];
  const tool = pick(DOCK_TOOL_IDS);
  const anchor = pick(DOCK_ANCHORS);
  const index = Math.floor(rand() * 14) - 2;
  if (rand() < 0.02) return { type: "reset" };
  switch (Math.floor(rand() * 9)) {
    case 0: return { type: "move", tool, anchor, index };
    case 1: return { type: "move", tool, anchor };
    case 2: return { type: "move", tool, anchor: anchorOf(layout, tool), index };
    case 3: return { type: "reorder", anchor, from: index, to: Math.floor(rand() * 14) - 2 };
    case 4: return { type: "toggle", tool };
    case 5: return { type: "open", tool };
    case 6: return { type: "hide", tool };
    case 7: return { type: "closeAnchor", anchor };
    default: return { type: "resize", key: pick(SIZE_KEYS), value: pick(RANDOM_SIZES), persist: rand() < 0.5 };
  }
}

describe("dockReducer invariants", () => {
  it("hold after every action of a seeded run of 500 random actions", () => {
    for (const seed of [1, 42, 20261002]) {
      const rand = mulberry32(seed);
      let layout: DockLayout = DEFAULT_DOCK_LAYOUT;
      let changes = 0;
      for (let step = 0; step < 500; step++) {
        const action = randomAction(rand, layout);
        const before = JSON.stringify(layout);
        const next = dockReducer(layout, action);
        const context = `seed ${seed} step ${step} ${JSON.stringify(action)}`;
        expect(dockLayoutErrors(next), context).toEqual([]);
        expect(JSON.stringify(layout), `${context} mutated its input`).toBe(before);
        if (action.type !== "reset" && JSON.stringify(next) === before) expect(next, `${context} changed identity without a change`).toBe(layout);
        if (action.type === "move" && action.anchor === anchorOf(layout, action.tool)) {
          expect(next.active, context).toEqual(layout.active);
          if (action.index === undefined) expect(next, context).toBe(layout);
        }
        if (action.type === "move" && action.anchor !== anchorOf(layout, action.tool)) {
          expect(next.active[action.anchor], context).toBe(action.tool);
          expect(anchorOf(next, action.tool), context).toBe(action.anchor);
        }
        if (action.type === "reorder") expect(next.active, context).toEqual(layout.active);
        if (next !== layout) changes++;
        layout = next;
      }
      expect(changes).toBeGreaterThan(200);
    }
  });
});

function mountState(patch: Partial<ToolMountInput>) {
  const tool = patch.tool ?? "image";
  return toolMountState({
    tool,
    activeTool: null,
    shownTool: null,
    anchorMounted: false,
    hasFile: true,
    fileKey: "a",
    keptFileKey: null,
    mount: DOCK_TOOL_META[tool].mount,
    needsFile: DOCK_TOOL_META[tool].needsFile,
    ...patch,
  });
}

const HIDDEN = { mounted: false, visible: false, active: false };

describe("toolMountState", () => {
  it("mounts nothing before any tool is opened", () => {
    expect(mountState({})).toEqual(HIDDEN);
    expect(mountState({ tool: "headers" })).toEqual(HIDDEN);
  });

  it("shows analysis as active while it is the open tool", () => {
    expect(mountState({ activeTool: "image", shownTool: "image", anchorMounted: true, keptFileKey: "a" }))
      .toEqual({ mounted: true, visible: true, active: true });
  });

  it("mounts analysis on the render that opens it, before the kept file key catches up", () => {
    expect(mountState({ activeTool: "image", shownTool: "image", anchorMounted: true, keptFileKey: null }))
      .toEqual({ mounted: true, visible: true, active: true });
  });

  it("keeps analysis mounted but hidden and inactive while another tool of its anchor is shown", () => {
    const input = { activeTool: "headers", shownTool: "headers", anchorMounted: true, keptFileKey: "a" } as const;
    expect(mountState(input)).toEqual({ mounted: true, visible: false, active: false });
    expect(mountState({ ...input, tool: "headers" })).toEqual({ mounted: true, visible: true, active: true });
  });

  it("keeps a tool visible but inactive during the close animation", () => {
    expect(mountState({ shownTool: "image", anchorMounted: true, keptFileKey: "a" })).toEqual({ mounted: true, visible: true, active: false });
    expect(mountState({ tool: "headers", shownTool: "headers", anchorMounted: true })).toEqual({ mounted: true, visible: true, active: false });
  });

  it("keeps analysis mounted after the anchor has closed and unmounts other tools", () => {
    expect(mountState({ shownTool: "image", anchorMounted: false, keptFileKey: "a" })).toEqual({ mounted: true, visible: false, active: false });
    expect(mountState({ tool: "headers", shownTool: "headers", anchorMounted: false })).toEqual(HIDDEN);
  });

  it("drops the hidden analysis when another file is loaded", () => {
    expect(mountState({ activeTool: "headers", shownTool: "headers", anchorMounted: true, fileKey: "b", keptFileKey: "a" }).mounted).toBe(false);
    expect(mountState({ shownTool: "image", anchorMounted: false, fileKey: "b", keptFileKey: "a" }).mounted).toBe(false);
  });

  it("keeps the open analysis mounted across a file change", () => {
    expect(mountState({ activeTool: "image", shownTool: "image", anchorMounted: true, fileKey: "b", keptFileKey: "a" }))
      .toEqual({ mounted: true, visible: true, active: true });
  });

  it("mounts nothing that needs a file while there is none, even when it is the open tool", () => {
    for (const tool of ["image", "compose", "headers"] as const) {
      expect(mountState({ tool, activeTool: tool, shownTool: tool, anchorMounted: true, hasFile: false, fileKey: null })).toEqual(HIDDEN);
    }
  });

  it("never keeps a whileOpen tool once it is hidden", () => {
    expect(mountState({ tool: "processing", activeTool: "stacking", shownTool: "stacking", anchorMounted: true, keptFileKey: "a" })).toEqual(HIDDEN);
  });

  it("keeps Files mounted while hidden and without a file", () => {
    expect(mountState({ tool: "files", activeTool: "info", shownTool: "info", anchorMounted: true, hasFile: false, fileKey: null }))
      .toEqual({ mounted: true, visible: false, active: false });
    expect(mountState({ tool: "files", hasFile: false, fileKey: null })).toEqual({ mounted: true, visible: false, active: false });
  });

  it("shows Info without a file", () => {
    expect(mountState({ tool: "info", activeTool: "info", shownTool: "info", anchorMounted: true, hasFile: false, fileKey: null }))
      .toEqual({ mounted: true, visible: true, active: true });
  });

  it("lets the open tool of each anchor be active at once", () => {
    const layout = withActive({ "right-top": "image", "right-bottom": "export" });
    const active = DOCK_TOOL_IDS.filter((tool) => {
      const anchor: DockAnchor = anchorOf(layout, tool);
      return mountState({ tool, activeTool: layout.active[anchor], shownTool: layout.active[anchor], anchorMounted: true }).active;
    });
    expect(active).toEqual(["files", "compose", "image", "export"]);
  });
});

describe("keptToolFileKey", () => {
  it("records the file the tool was active on and keeps it while the tool is hidden", () => {
    expect(keptToolFileKey(true, "a", null)).toBe("a");
    expect(keptToolFileKey(true, "b", "a")).toBe("b");
    expect(keptToolFileKey(false, "a", "a")).toBe("a");
  });

  it("forgets the file as soon as another file is loaded", () => {
    expect(keptToolFileKey(false, "b", "a")).toBeNull();
    expect(keptToolFileKey(false, null, "a")).toBeNull();
  });

  it("does not mount a hidden analysis on returning to a file after A -> B -> A", () => {
    let kept = keptToolFileKey(true, "a", null);
    kept = keptToolFileKey(false, "a", kept);
    kept = keptToolFileKey(false, "b", kept);
    kept = keptToolFileKey(false, "a", kept);
    expect(kept).toBeNull();
    expect(mountState({ activeTool: "headers", shownTool: "headers", anchorMounted: true, fileKey: "a", keptFileKey: kept }).mounted).toBe(false);
  });

  it("lets a hidden kept tool follow the file while a tool of its keep group is open", () => {
    const kept = keptToolFileKey(false, "b", "a", true);
    expect(kept).toBe("b");
    expect(mountState({ tool: "photometry", activeTool: "image", shownTool: "image", anchorMounted: true, fileKey: "b", keptFileKey: kept }))
      .toEqual({ mounted: true, visible: false, active: false });
  });

  it("never mounts a tool that was not opened, even while its group is open", () => {
    expect(keptToolFileKey(false, "b", null, true)).toBeNull();
  });

  it("drops the kept key without a file, even while its group is open", () => {
    expect(keptToolFileKey(false, null, "a", true)).toBeNull();
  });

  it("drops the kept key on a file change while no tool of its group is open", () => {
    expect(keptToolFileKey(false, "b", "a", false)).toBeNull();
    expect(keptToolFileKey(false, "b", "a")).toBeNull();
  });

  it("follows the file while the tool itself is open, whatever its group", () => {
    expect(keptToolFileKey(true, "b", "a", true)).toBe("b");
  });
});

describe("groupActive", () => {
  const NONE = { "left-top": null, "left-bottom": null, "right-top": null, "right-bottom": null } as const;

  it("is true while some anchor's open tool belongs to the group", () => {
    expect(groupActive({ ...NONE, "right-top": "image" }, "analysis")).toBe(true);
    expect(groupActive({ ...NONE, "left-bottom": "cube" }, "analysis")).toBe(true);
    expect(groupActive({ ...NONE, "left-top": "files", "right-bottom": "photometry" }, "analysis")).toBe(true);
  });

  it("is false while no open tool belongs to the group", () => {
    expect(groupActive({ ...NONE, "right-top": "headers" }, "analysis")).toBe(false);
    expect(groupActive(NONE, "analysis")).toBe(false);
    expect(groupActive({ ...NONE, "right-bottom": "log" }, "analysis")).toBe(false);
    expect(groupActive({ ...NONE, "right-top": "image" }, "other")).toBe(false);
  });
});
