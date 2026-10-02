import { describe, it, expect } from "vitest";
import { DEFAULT_DOCK_LAYOUT, DOCK_ANCHORS, anchorSide, dockReducer, type DockLayout, type DockSizes } from "../dockLayout";
import {
  DRAG_THRESHOLD_PX,
  EMPTY_GROUP_SLOT_PX,
  GROUP_HIT_SLOP_PX,
  STRIP_HIT_SLOP_PX,
  bottomSplitLayout,
  clampBottomToViewer,
  clampColumnsToViewer,
  dragStarted,
  dropPreviewRect,
  dropTargetAt,
  emptyGroupSlotRect,
  insertionIndex,
  insertionMarkerRect,
  shrinkColumnsForDeficit,
  type DockGeometry,
  type DragSource,
  type Rect,
} from "../dockDrag";

const DOCK: Rect = { x: 0, y: 40, w: 1280, h: 700 };
const LEFT_STRIP: Rect = { x: 0, y: 40, w: 42, h: 700 };
const RIGHT_STRIP: Rect = { x: 1238, y: 40, w: 42, h: 700 };
const BTN = 38;
const PAD = 4;

function geometryFor(layout: DockLayout): DockGeometry {
  const groups = DOCK_ANCHORS.map((anchor) => {
    const strip = anchorSide(anchor) === "left" ? LEFT_STRIP : RIGHT_STRIP;
    const tools = layout.anchors[anchor];
    const h = tools.length * BTN;
    const y = anchor.endsWith("top") ? strip.y + PAD : strip.y + strip.h - PAD - h;
    return {
      anchor,
      rect: { x: strip.x, y, w: strip.w, h },
      items: tools.map((tool, i) => ({ tool, rect: { x: strip.x + 2, y: y + i * BTN, w: BTN, h: BTN } })),
    };
  });
  return { dock: DOCK, leftStrip: LEFT_STRIP, rightStrip: RIGHT_STRIP, groups };
}

function withActive(active: Partial<DockLayout["active"]>, base: DockLayout = DEFAULT_DOCK_LAYOUT): DockLayout {
  return { ...base, active: { ...base.active, ...active } };
}

function withSizes(sizes: Partial<DockSizes>, base: DockLayout = DEFAULT_DOCK_LAYOUT): DockLayout {
  return { ...base, sizes: { ...base.sizes, ...sizes } };
}

const RIGHT_BOTTOM_EMPTY = (["synth", "export", "config"] as const).reduce<DockLayout>(
  (layout, tool) => dockReducer(layout, { type: "move", tool, anchor: "left-top" }),
  DEFAULT_DOCK_LAYOUT,
);

const G = geometryFor(DEFAULT_DOCK_LAYOUT);
const ANALYSIS: DragSource = { tool: "analysis", from: "right-top", fromIndex: 1 };
const HEADERS: DragSource = { tool: "headers", from: "right-top", fromIndex: 0 };
const FILES: DragSource = { tool: "files", from: "left-top", fromIndex: 0 };
const INFO: DragSource = { tool: "info", from: "left-top", fromIndex: 1 };

describe("constants", () => {
  it("match the plan", () => {
    expect([DRAG_THRESHOLD_PX, STRIP_HIT_SLOP_PX, GROUP_HIT_SLOP_PX, EMPTY_GROUP_SLOT_PX]).toEqual([5, 16, 16, 24]);
  });
});

describe("dragStarted", () => {
  it("starts at the 5 px threshold", () => {
    const start = { x: 100, y: 100 };
    expect(dragStarted(start, { x: 104, y: 100 })).toBe(false);
    expect(dragStarted(start, { x: 105, y: 100 })).toBe(true);
    expect(dragStarted(start, { x: 106, y: 100 })).toBe(true);
    expect(dragStarted(start, { x: 103, y: 104 })).toBe(true);
    expect(dragStarted(start, { x: 97, y: 97 })).toBe(false);
    expect(dragStarted(start, { x: 106, y: 100 }, 10)).toBe(false);
  });
});

describe("insertionIndex", () => {
  const items: Rect[] = [44, 82, 120, 158].map((y) => ({ x: 0, y, w: BTN, h: BTN }));

  it("counts the item mid-points above the pointer", () => {
    expect(insertionIndex(30, items, null)).toBe(0);
    expect(insertionIndex(62, items, null)).toBe(0);
    expect(insertionIndex(64, items, null)).toBe(1);
    expect(insertionIndex(140, items, null)).toBe(3);
    expect(insertionIndex(400, items, null)).toBe(4);
    expect(insertionIndex(100, [], null)).toBe(0);
  });

  it("returns the index after removal when the dragged item is in the same group above the pointer", () => {
    expect(insertionIndex(140, items, 0)).toBe(2);
    expect(insertionIndex(400, items, 1)).toBe(3);
    expect(insertionIndex(140, items, 3)).toBe(3);
    expect(insertionIndex(70, items, 1)).toBe(1);
    expect(insertionIndex(30, items, 2)).toBe(0);
  });
});

describe("dropTargetAt", () => {
  it("targets the left top group over its second item", () => {
    expect(dropTargetAt({ x: 20, y: 90 }, G, ANALYSIS)).toEqual({ anchor: "left-top", index: 1 });
    expect(dropTargetAt({ x: 20, y: 110 }, G, ANALYSIS)).toEqual({ anchor: "left-top", index: 2 });
  });

  it("targets the nearer group from the spacer", () => {
    expect(dropTargetAt({ x: 20, y: 600 }, G, ANALYSIS)).toEqual({ anchor: "left-bottom", index: 0 });
    expect(dropTargetAt({ x: 20, y: 200 }, G, ANALYSIS)).toEqual({ anchor: "left-top", index: 2 });
    expect(dropTargetAt({ x: 20, y: 735 }, G, ANALYSIS)).toEqual({ anchor: "left-bottom", index: 1 });
  });

  it("targets an empty bottom group through its slot", () => {
    const g = geometryFor(RIGHT_BOTTOM_EMPTY);
    expect(dropTargetAt({ x: 1260, y: 720 }, g, ANALYSIS)).toEqual({ anchor: "right-bottom", index: 0 });
    expect(dropTargetAt({ x: 1260, y: 700 }, g, ANALYSIS)).toEqual({ anchor: "right-bottom", index: 0 });
    expect(dropTargetAt({ x: 1260, y: 300 }, g, ANALYSIS)).toEqual({ anchor: "right-top", index: 3 });
  });

  it("accepts points inside the slop next to a strip", () => {
    expect(dropTargetAt({ x: 52, y: 70 }, G, ANALYSIS)).toEqual({ anchor: "left-top", index: 1 });
    expect(dropTargetAt({ x: 57, y: 70 }, G, ANALYSIS)).toEqual({ anchor: "left-top", index: 1 });
    expect(dropTargetAt({ x: 1228, y: 50 }, G, ANALYSIS)).toEqual({ anchor: "right-top", index: 0 });
    expect(dropTargetAt({ x: 1222, y: 50 }, G, ANALYSIS)).toEqual({ anchor: "right-top", index: 0 });
  });

  it("returns null over the viewer and outside the dock", () => {
    expect(dropTargetAt({ x: 59, y: 70 }, G, ANALYSIS)).toBeNull();
    expect(dropTargetAt({ x: 1221, y: 50 }, G, ANALYSIS)).toBeNull();
    expect(dropTargetAt({ x: 640, y: 300 }, G, ANALYSIS)).toBeNull();
    expect(dropTargetAt({ x: 20, y: 760 }, G, ANALYSIS)).toBeNull();
    expect(dropTargetAt({ x: 20, y: 20 }, G, ANALYSIS)).toBeNull();
    expect(dropTargetAt({ x: -5, y: 100 }, G, ANALYSIS)).toBeNull();
    expect(dropTargetAt({ x: 1290, y: 100 }, G, ANALYSIS)).toBeNull();
  });

  it("picks the group under the pointer when the bottom group starts within the slop of the top group", () => {
    const crowded: DockGeometry = {
      ...G,
      groups: [
        {
          anchor: "left-top",
          rect: { x: 0, y: 44, w: 42, h: 76 },
          items: [
            { tool: "files", rect: { x: 2, y: 44, w: 38, h: 38 } },
            { tool: "info", rect: { x: 2, y: 82, w: 38, h: 38 } },
          ],
        },
        { anchor: "left-bottom", rect: { x: 0, y: 130, w: 42, h: 38 }, items: [{ tool: "compose", rect: { x: 2, y: 130, w: 38, h: 38 } }] },
      ],
    };
    expect(dropTargetAt({ x: 20, y: 130 }, crowded, ANALYSIS)).toEqual({ anchor: "left-bottom", index: 0 });
    expect(dropTargetAt({ x: 20, y: 133 }, crowded, ANALYSIS)).toEqual({ anchor: "left-bottom", index: 0 });
    expect(dropTargetAt({ x: 20, y: 136 }, crowded, ANALYSIS)).toEqual({ anchor: "left-bottom", index: 0 });
    expect(dropTargetAt({ x: 20, y: 118 }, crowded, ANALYSIS)).toEqual({ anchor: "left-top", index: 2 });
    expect(dropTargetAt({ x: 20, y: 124 }, crowded, ANALYSIS)).toEqual({ anchor: "left-top", index: 2 });
    expect(dropTargetAt({ x: 20, y: 126 }, crowded, ANALYSIS)).toEqual({ anchor: "left-bottom", index: 0 });
    const adjacent: DockGeometry = {
      ...crowded,
      groups: [
        crowded.groups[0],
        { anchor: "left-bottom", rect: { x: 0, y: 120, w: 42, h: 38 }, items: [{ tool: "compose", rect: { x: 2, y: 120, w: 38, h: 38 } }] },
      ],
    };
    expect(dropTargetAt({ x: 20, y: 122 }, adjacent, ANALYSIS)).toEqual({ anchor: "left-bottom", index: 0 });
    expect(dropTargetAt({ x: 20, y: 118 }, adjacent, ANALYSIS)).toEqual({ anchor: "left-top", index: 2 });
  });

  it("returns the index after removal inside the dragged item's own group", () => {
    expect(dropTargetAt({ x: 1260, y: 190 }, G, ANALYSIS)).toEqual({ anchor: "right-top", index: 3 });
    expect(dropTargetAt({ x: 1260, y: 90 }, G, ANALYSIS)).toEqual({ anchor: "right-top", index: 1 });
    expect(dropTargetAt({ x: 1260, y: 50 }, G, ANALYSIS)).toEqual({ anchor: "right-top", index: 0 });
  });
});

describe("emptyGroupSlotRect", () => {
  it("places the slot at the top group's start or above the bottom group's end", () => {
    const g = geometryFor(RIGHT_BOTTOM_EMPTY);
    expect(emptyGroupSlotRect("right-bottom", g)).toEqual({ x: 1238, y: 712, w: 42, h: 24 });
    const leftEmpty = geometryFor((["files", "info"] as const).reduce<DockLayout>(
      (layout, tool) => dockReducer(layout, { type: "move", tool, anchor: "right-bottom" }),
      DEFAULT_DOCK_LAYOUT,
    ));
    expect(emptyGroupSlotRect("left-top", leftEmpty)).toEqual({ x: 0, y: 44, w: 42, h: 24 });
  });

  it("falls back to the strip edges when the group was not measured", () => {
    const g: DockGeometry = { ...G, groups: [] };
    expect(emptyGroupSlotRect("left-top", g)).toEqual({ x: 0, y: 40, w: 42, h: 24 });
    expect(emptyGroupSlotRect("right-bottom", g)).toEqual({ x: 1238, y: 716, w: 42, h: 24 });
  });
});

describe("insertionMarkerRect", () => {
  it("draws a 30 x 2 line centred in the strip at the insertion gap", () => {
    expect(insertionMarkerRect({ anchor: "left-top", index: 0 }, G, ANALYSIS)).toEqual({ x: 6, y: 43, w: 30, h: 2 });
    expect(insertionMarkerRect({ anchor: "left-top", index: 1 }, G, ANALYSIS)).toEqual({ x: 6, y: 81, w: 30, h: 2 });
    expect(insertionMarkerRect({ anchor: "left-top", index: 2 }, G, ANALYSIS)).toEqual({ x: 6, y: 119, w: 30, h: 2 });
  });

  it("skips the dragged item inside its own group", () => {
    expect(insertionMarkerRect({ anchor: "right-top", index: 1 }, G, ANALYSIS)).toEqual({ x: 1244, y: 100, w: 30, h: 2 });
    expect(insertionMarkerRect({ anchor: "right-top", index: 2 }, G, ANALYSIS)).toEqual({ x: 1244, y: 157, w: 30, h: 2 });
    expect(insertionMarkerRect({ anchor: "right-top", index: 3 }, G, ANALYSIS)).toEqual({ x: 1244, y: 195, w: 30, h: 2 });
  });

  it("centres the line in the slot of an empty group", () => {
    expect(insertionMarkerRect({ anchor: "right-bottom", index: 0 }, geometryFor(RIGHT_BOTTOM_EMPTY), ANALYSIS))
      .toEqual({ x: 1244, y: 723, w: 30, h: 2 });
  });
});

describe("dropPreviewRect", () => {
  it("is null for a reorder inside the source anchor", () => {
    expect(dropPreviewRect({ anchor: "right-top", index: 3 }, G, DEFAULT_DOCK_LAYOUT, ANALYSIS)).toBeNull();
  });

  it("covers the side column the tool will open in", () => {
    expect(dropPreviewRect({ anchor: "left-top", index: 0 }, G, DEFAULT_DOCK_LAYOUT, ANALYSIS)).toEqual({ x: 42, y: 40, w: 300, h: 700 });
    expect(dropPreviewRect({ anchor: "right-top", index: 0 }, G, DEFAULT_DOCK_LAYOUT, FILES)).toEqual({ x: 858, y: 40, w: 380, h: 700 });
  });

  it("widens to the moved tool's minimum", () => {
    expect(dropPreviewRect({ anchor: "left-top", index: 0 }, G, withSizes({ leftW: 180 }), ANALYSIS)).toEqual({ x: 42, y: 40, w: 280, h: 700 });
  });

  it("switches the cap from 60vw to 40vw when both side columns will be open", () => {
    const wide = withSizes({ leftW: 640 });
    expect(dropPreviewRect({ anchor: "left-top", index: 0 }, G, wide, ANALYSIS)).toEqual({ x: 42, y: 40, w: 640, h: 700 });
    expect(dropPreviewRect({ anchor: "left-top", index: 0 }, G, withActive({ "right-top": "processing" }, wide), ANALYSIS))
      .toEqual({ x: 42, y: 40, w: 512, h: 700 });
  });

  it("covers the full centre width when the moved tool closes its source column", () => {
    expect(dropPreviewRect({ anchor: "left-bottom", index: 1 }, G, withActive({ "right-top": "analysis" }), ANALYSIS))
      .toEqual({ x: 342, y: 460, w: 896, h: 280 });
    expect(dropPreviewRect({ anchor: "left-bottom", index: 1 }, G, withActive({ "right-top": "processing" }), ANALYSIS))
      .toEqual({ x: 342, y: 460, w: 516, h: 280 });
  });

  it("covers one half when the other bottom half stays open", () => {
    expect(dropPreviewRect({ anchor: "right-bottom", index: 0 }, G, DEFAULT_DOCK_LAYOUT, HEADERS)).toEqual({ x: 790, y: 460, w: 448, h: 280 });
    expect(dropPreviewRect({ anchor: "left-bottom", index: 0 }, G, withActive({ "right-bottom": "export" }), HEADERS))
      .toEqual({ x: 342, y: 460, w: 448, h: 280 });
    expect(dropPreviewRect({ anchor: "right-bottom", index: 0 }, G, withSizes({ bottomSplit: 0.3, bottomH: 200 }), HEADERS))
      .toEqual({ x: 622, y: 540, w: 616, h: 200 });
  });

  it("covers the full centre width when the other bottom half is closed", () => {
    expect(dropPreviewRect({ anchor: "right-bottom", index: 0 }, G, withActive({ "left-bottom": null }), HEADERS))
      .toEqual({ x: 342, y: 460, w: 896, h: 280 });
  });

  it("leaves out the panels that stay closed without a file", () => {
    const stored = withActive({ "right-top": "analysis" });
    expect(dropPreviewRect({ anchor: "right-bottom", index: 0 }, G, stored, INFO, false)).toEqual({ x: 342, y: 460, w: 896, h: 280 });
    expect(dropPreviewRect({ anchor: "right-bottom", index: 0 }, G, stored, INFO, true)).toEqual({ x: 622, y: 460, w: 236, h: 280 });
    expect(dropPreviewRect({ anchor: "right-bottom", index: 0 }, G, stored, INFO)).toEqual({ x: 622, y: 460, w: 236, h: 280 });
    const analysisLeft = withSizes({ rightW: 640 }, dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "analysis", anchor: "left-top" }));
    expect(dropPreviewRect({ anchor: "right-top", index: 0 }, G, analysisLeft, INFO, false)).toEqual({ x: 598, y: 40, w: 640, h: 700 });
    expect(dropPreviewRect({ anchor: "right-top", index: 0 }, G, analysisLeft, INFO, true)).toEqual({ x: 726, y: 40, w: 512, h: 700 });
  });

  it("shows no preview for a tool that needs a file while there is none", () => {
    expect(dropPreviewRect({ anchor: "left-top", index: 0 }, G, DEFAULT_DOCK_LAYOUT, ANALYSIS, false)).toBeNull();
    expect(dropPreviewRect({ anchor: "right-bottom", index: 0 }, G, DEFAULT_DOCK_LAYOUT, HEADERS, false)).toBeNull();
    expect(dropPreviewRect({ anchor: "right-top", index: 0 }, G, DEFAULT_DOCK_LAYOUT, FILES, false)).toEqual({ x: 858, y: 40, w: 380, h: 700 });
  });
});

describe("bottomSplitLayout", () => {
  const base = { areaW: 600, split: 0.5, bottomH: 280, leftMin: 280, rightMin: 280 };

  it("is empty with nothing open", () => {
    expect(bottomSplitLayout({ ...base, leftOpen: false, rightOpen: false })).toEqual({ mode: "none", left: null, right: null, total: 0 });
  });

  it("gives the full width to a single open half", () => {
    expect(bottomSplitLayout({ ...base, leftOpen: true, rightOpen: false }))
      .toEqual({ mode: "single", left: { x: 0, y: 0, w: 600, h: 280 }, right: null, total: 280 });
    expect(bottomSplitLayout({ ...base, leftOpen: false, rightOpen: true }))
      .toEqual({ mode: "single", left: null, right: { x: 0, y: 0, w: 600, h: 280 }, total: 280 });
  });

  it("splits by the ratio when both halves are open", () => {
    expect(bottomSplitLayout({ ...base, leftOpen: true, rightOpen: true }))
      .toEqual({ mode: "split", left: { x: 0, y: 0, w: 300, h: 280 }, right: { x: 300, y: 0, w: 300, h: 280 }, total: 280 });
    expect(bottomSplitLayout({ ...base, areaW: 1000, split: 0.333, leftOpen: true, rightOpen: true }).left?.w).toBe(333);
  });

  it("clamps the split so each half keeps its tool's minimum when the area is wide enough", () => {
    const low = bottomSplitLayout({ ...base, areaW: 700, split: 0.25, leftOpen: true, rightOpen: true });
    expect([low.left, low.right]).toEqual([{ x: 0, y: 0, w: 280, h: 280 }, { x: 280, y: 0, w: 420, h: 280 }]);
    const high = bottomSplitLayout({ ...base, areaW: 700, split: 0.75, leftOpen: true, rightOpen: true });
    expect([high.left?.w, high.right?.w]).toEqual([420, 280]);
    const mixed = bottomSplitLayout({ ...base, areaW: 500, split: 0.25, leftMin: 180, leftOpen: true, rightOpen: true });
    expect([mixed.left?.w, mixed.right?.w]).toEqual([180, 320]);
  });

  it("uses the ratio unclamped when the area is narrower than both minimums", () => {
    const narrow = bottomSplitLayout({ ...base, areaW: 500, split: 0.25, leftOpen: true, rightOpen: true });
    expect([narrow.left?.w, narrow.right?.w]).toEqual([125, 375]);
    const half = bottomSplitLayout({ ...base, areaW: 516, leftOpen: true, rightOpen: true });
    expect([half.left?.w, half.right?.w]).toEqual([258, 258]);
  });

  it("subtracts the handle width between the halves", () => {
    const split = bottomSplitLayout({ ...base, areaW: 700, handleW: 6, leftOpen: true, rightOpen: true });
    expect([split.left, split.right]).toEqual([{ x: 0, y: 0, w: 347, h: 280 }, { x: 353, y: 0, w: 347, h: 280 }]);
  });
});

describe("shrinkColumnsForDeficit", () => {
  const both = { leftOpen: true, rightOpen: true, leftMin: 280, rightMin: 280 };

  it("takes from the wider open column first", () => {
    expect(shrinkColumnsForDeficit({ ...both, leftW: 500, rightW: 300, deficit: 150 })).toEqual({ leftW: 350, rightW: 300 });
    expect(shrinkColumnsForDeficit({ ...both, leftW: 300, rightW: 500, deficit: 150 })).toEqual({ leftW: 300, rightW: 350 });
  });

  it("then takes from both columns equally", () => {
    expect(shrinkColumnsForDeficit({ ...both, leftW: 500, rightW: 300, deficit: 220 })).toEqual({ leftW: 290, rightW: 290 });
    expect(shrinkColumnsForDeficit({ ...both, leftW: 400, rightW: 400, deficit: 3 })).toEqual({ leftW: 398, rightW: 399 });
  });

  it("never goes below the given minimums", () => {
    expect(shrinkColumnsForDeficit({ ...both, leftW: 500, rightW: 300, deficit: 1000 })).toEqual({ leftW: 280, rightW: 280 });
    expect(shrinkColumnsForDeficit({ ...both, leftMin: 180, rightMin: 380, leftW: 400, rightW: 400, deficit: 100 })).toEqual({ leftW: 320, rightW: 380 });
    expect(shrinkColumnsForDeficit({ ...both, leftMin: 450, leftW: 500, rightW: 300, deficit: 150 })).toEqual({ leftW: 450, rightW: 280 });
  });

  it("shrinks only the open column when one side is closed", () => {
    expect(shrinkColumnsForDeficit({ ...both, rightOpen: false, leftMin: 180, leftW: 300, rightW: 380, deficit: 50 })).toEqual({ leftW: 250, rightW: 380 });
    expect(shrinkColumnsForDeficit({ ...both, rightOpen: false, leftMin: 180, leftW: 300, rightW: 380, deficit: 500 })).toEqual({ leftW: 180, rightW: 380 });
    expect(shrinkColumnsForDeficit({ ...both, leftOpen: false, leftW: 300, rightW: 500, deficit: 100 })).toEqual({ leftW: 300, rightW: 400 });
  });

  it("changes nothing without a deficit or an open column", () => {
    expect(shrinkColumnsForDeficit({ ...both, leftW: 300, rightW: 400, deficit: 0 })).toEqual({ leftW: 300, rightW: 400 });
    expect(shrinkColumnsForDeficit({ ...both, leftW: 300, rightW: 400, deficit: -20 })).toEqual({ leftW: 300, rightW: 400 });
    expect(shrinkColumnsForDeficit({ ...both, leftOpen: false, rightOpen: false, leftW: 300, rightW: 400, deficit: 100 })).toEqual({ leftW: 300, rightW: 400 });
  });
});

describe("clampColumnsToViewer", () => {
  const view = { rootW: 1250, stripsW: 84, leftOpen: true, rightOpen: true, leftMin: 180, rightMin: 280, minCentre: 320 };

  it("takes only the deficit of the settled layout", () => {
    expect(clampColumnsToViewer({ ...view, leftW: 500, rightW: 380 })).toEqual({ leftW: 466, rightW: 380 });
  });

  it("takes nothing more on the frames that follow while the columns still animate", () => {
    let widths = { leftW: 500, rightW: 380 };
    for (let frame = 0; frame < 12; frame++) widths = clampColumnsToViewer({ ...view, ...widths }) ?? widths;
    expect(widths).toEqual({ leftW: 466, rightW: 380 });
    expect(clampColumnsToViewer({ ...view, leftW: 467, rightW: 380 })).toBeNull();
  });

  it("ignores the width of a closed column", () => {
    expect(clampColumnsToViewer({ ...view, rightOpen: false, leftW: 900, rightW: 380 })).toEqual({ leftW: 846, rightW: 380 });
    expect(clampColumnsToViewer({ ...view, rightOpen: false, leftW: 640, rightW: 380 })).toBeNull();
  });

  it("measures an overflowing layout from the root width", () => {
    expect(clampColumnsToViewer({ ...view, rootW: 900, leftW: 500, rightW: 500 })).toEqual({ leftW: 216, rightW: 280 });
  });

  it("returns null with no open column or with both columns at their minimums", () => {
    expect(clampColumnsToViewer({ ...view, leftOpen: false, rightOpen: false, leftW: 900, rightW: 900 })).toBeNull();
    expect(clampColumnsToViewer({ ...view, rootW: 600, leftW: 180, rightW: 280 })).toBeNull();
  });
});

describe("clampBottomToViewer", () => {
  const area = { open: true, minViewport: 200, minBottom: 140 };

  it("takes only the deficit of the settled height", () => {
    expect(clampBottomToViewer({ ...area, viewportH: 120, outerH: 280, bottomH: 280 })).toBe(200);
  });

  it("counts the part of the height transition that has not run yet", () => {
    expect(clampBottomToViewer({ ...area, viewportH: 400, outerH: 50, bottomH: 280 })).toBe(250);
    expect(clampBottomToViewer({ ...area, viewportH: 140, outerH: 260, bottomH: 200 })).toBeNull();
  });

  it("stops at the bottom floor", () => {
    expect(clampBottomToViewer({ ...area, viewportH: 50, outerH: 280, bottomH: 280 })).toBe(140);
    expect(clampBottomToViewer({ ...area, viewportH: 50, outerH: 140, bottomH: 140 })).toBeNull();
  });

  it("does nothing while the area is closed", () => {
    expect(clampBottomToViewer({ ...area, open: false, viewportH: 50, outerH: 280, bottomH: 280 })).toBeNull();
  });
});
