import { afterEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_DOCK_LAYOUT, dockReducer, type DockToolId } from "../dockLayout";
import { dropTargetAt, insertionMarkerRect, type DockGeometry, type DragSource } from "../dockDrag";
import { measureDockGeometry } from "../../components/dock/dockGeometry";

const processing: DragSource = { tool: "processing", from: "right-top", fromIndex: 7 };
const image: DragSource = { tool: "image", from: "right-top", fromIndex: 1 };
const files: DragSource = { tool: "files", from: "left-top", fromIndex: 0 };
const visible: DockToolId[] = ["image", "processing", "history"];

const geometry: DockGeometry = {
  dock: { x: 0, y: 0, w: 1200, h: 800 },
  viewportW: 1200,
  leftStrip: { x: 0, y: 0, w: 44, h: 800 },
  rightStrip: { x: 1156, y: 0, w: 44, h: 800 },
  groups: [
    {
      anchor: "right-top",
      rect: { x: 1156, y: 60, w: 44, h: 112 },
      totalItems: DEFAULT_DOCK_LAYOUT.anchors["right-top"].length,
      items: visible.map((tool, index) => ({
        tool,
        index: DEFAULT_DOCK_LAYOUT.anchors["right-top"].indexOf(tool),
        rect: { x: 1158, y: 60 + index * 38, w: 40, h: 36 },
      })),
    },
    { anchor: "right-bottom", rect: { x: 1156, y: 796, w: 44, h: 0 }, totalItems: 4, items: [] },
  ],
};

function movedAt(y: number, source: DragSource) {
  const target = dropTargetAt({ x: 1178, y }, geometry, source);
  if (!target) throw new Error("No drop target");
  return { target, layout: dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: source.tool, ...target }) };
}

describe("Compact dock dragging", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("does not move a shortcut when dragging six pixels down within its own slot", () => {
    for (const [source, centre] of [[image, 78], [processing, 116]] as const) {
      const { target, layout } = movedAt(centre + 6, source);
      expect(target).toEqual({ anchor: "right-top", index: source.fromIndex });
      expect(layout).toBe(DEFAULT_DOCK_LAYOUT);
    }
  });

  it("reorders against visible neighbours while retaining every hidden tool", () => {
    const upward = movedAt(70, processing);
    expect(upward.target.index).toBe(1);
    expect(upward.layout.anchors["right-top"].filter((tool) => visible.includes(tool))).toEqual(["processing", "image", "history"]);
    const downward = movedAt(164, processing);
    expect(downward.target.index).toBe(8);
    expect(downward.layout.anchors["right-top"].filter((tool) => visible.includes(tool))).toEqual(["image", "history", "processing"]);
    for (const { layout } of [upward, downward]) {
      expect(layout.anchors["right-top"].filter((tool) => !visible.includes(tool))).toEqual(DEFAULT_DOCK_LAYOUT.anchors["right-top"].filter((tool) => !visible.includes(tool)));
    }
  });

  it("places a cross-anchor drop between the visible targets using their full indices", () => {
    const { target, layout } = movedAt(100, files);
    expect(target).toEqual({ anchor: "right-top", index: 7 });
    expect(layout.anchors["left-top"]).toEqual(["info"]);
    expect(layout.anchors["right-top"].slice(6, 9)).toEqual(["sagui", "files", "processing"]);
  });

  it("keeps hidden suffix tools after a drop following the last visible shortcut", () => {
    const { target, layout } = movedAt(170, files);
    expect(target.index).toBe(9);
    expect(layout.anchors["right-top"].slice(-4)).toEqual(["history", "files", "masks", "stacking"]);
  });

  it("appends to a group whose existing shortcuts are all hidden", () => {
    const { target, layout } = movedAt(790, files);
    expect(target).toEqual({ anchor: "right-bottom", index: 4 });
    expect(layout.anchors["right-bottom"]).toEqual(["synth", "export", "config", "log", "files"]);
  });

  it("draws the insertion marker in the visible gap for the mapped full index", () => {
    expect(insertionMarkerRect({ anchor: "right-top", index: 7 }, geometry, files)).toEqual({ x: 1163, y: 96, w: 30, h: 2 });
    expect(insertionMarkerRect({ anchor: "right-top", index: 1 }, geometry, processing)).toEqual({ x: 1163, y: 59, w: 30, h: 2 });
    expect(insertionMarkerRect({ anchor: "right-top", index: 8 }, geometry, processing)).toEqual({ x: 1163, y: 171, w: 30, h: 2 });
  });

  it("measures only visible buttons and retains their canonical positions", () => {
    const rect = (top: number, width: number, height: number) => ({ left: 1158, top, width, height });
    const buttons = DEFAULT_DOCK_LAYOUT.anchors["right-top"].map((tool) => ({
      dataset: { toolId: tool },
      getBoundingClientRect: () => {
        const index = visible.indexOf(tool);
        return index < 0 ? rect(0, 0, 0) : rect(60 + index * 38, 40, 36);
      },
    }));
    const group = {
      dataset: { dockGroup: "right-top" },
      getBoundingClientRect: () => rect(60, 44, 112),
      querySelectorAll: () => buttons,
    };
    const root = {
      getBoundingClientRect: () => ({ left: 0, top: 0, width: 1200, height: 800 }),
      querySelector: () => null,
      querySelectorAll: () => [group],
    } as unknown as HTMLElement;
    vi.stubGlobal("window", { innerWidth: 1280 });
    expect(measureDockGeometry(root).viewportW).toBe(1280);
    const measured = measureDockGeometry(root).groups[0];
    expect(measured.totalItems).toBe(11);
    expect(measured.items.map(({ tool, index }) => ({ tool, index }))).toEqual([
      { tool: "image", index: 1 },
      { tool: "processing", index: 7 },
      { tool: "history", index: 8 },
    ]);
  });
});
