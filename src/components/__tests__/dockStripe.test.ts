import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import Stripe from "../dock/Stripe";
import type { StripButtonDragProps } from "../dock/useDockDrag";
import { nextShown, settleBottom, settleSide, startResizeDrag, type AnchorTools } from "../dock/dockGeometry";
import type { ResizeSession } from "../dock/ResizeHandle";
import { dockStore } from "../../hooks/useDockLayout";
import {
  DEFAULT_DOCK_LAYOUT,
  DOCK_TOOL_META,
  dockReducer,
  type DockAnchor,
  type DockLayout,
  type DockSide,
  type DockToolId,
} from "../../utils/dockLayout";

const NO_DRAG: StripButtonDragProps = {
  onPointerDown() {},
  onPointerMove() {},
  onPointerUp() {},
  onPointerCancel() {},
  onLostPointerCapture() {},
  onClickCapture() {},
};

const GROUPS: Record<DockSide, [DockAnchor, DockAnchor]> = {
  left: ["left-top", "left-bottom"],
  right: ["right-top", "right-bottom"],
};

function render(side: DockSide, layout: DockLayout = DEFAULT_DOCK_LAYOUT, hasFile = true): string {
  return renderToStaticMarkup(
    createElement(Stripe, { side, layout, hasFile, buttonProps: () => NO_DRAG, onContextMenu: () => {} }),
  );
}

function attrs(tag: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const m of tag.matchAll(/([\w-]+)="([^"]*)"/g)) out[m[1]] = m[2];
  return out;
}

function group(html: string, anchor: DockAnchor): Record<string, string>[] {
  const match = new RegExp(`<div data-dock-group="${anchor}"[^>]*>(.*?)</div>`).exec(html);
  if (!match) throw new Error(`group ${anchor} not rendered`);
  return [...match[1].matchAll(/<button[^>]*>/g)].map((m) => attrs(m[0]));
}

function order(html: string, anchor: DockAnchor): string[] {
  return group(html, anchor).map((b) => b["data-tool-id"]);
}

function pressed(html: string, anchor: DockAnchor): string[] {
  return group(html, anchor).filter((b) => b["aria-pressed"] === "true").map((b) => b["data-tool-id"]);
}

describe("Stripe", () => {
  it("renders each strip as a labelled group carrying the strip side", () => {
    const left = render("left");
    const right = render("right");
    expect(left).toMatch(/^<div[^>]*role="group"[^>]*aria-label="Left tools"/);
    expect(left).toMatch(/^<div[^>]*data-dock-strip="left"/);
    expect(right).toMatch(/^<div[^>]*role="group"[^>]*aria-label="Right tools"/);
    expect(right).toMatch(/^<div[^>]*data-dock-strip="right"/);
  });

  it("lists the buttons of each group in the layout order, top group first", () => {
    for (const side of ["left", "right"] as const) {
      const html = render(side);
      const [top, bottom] = GROUPS[side];
      expect(order(html, top)).toEqual(DEFAULT_DOCK_LAYOUT.anchors[top]);
      expect(order(html, bottom)).toEqual(DEFAULT_DOCK_LAYOUT.anchors[bottom]);
      expect(html.indexOf(`data-dock-group="${top}"`)).toBeLessThan(html.indexOf(`data-dock-group="${bottom}"`));
    }
  });

  it("follows a moved and reordered layout", () => {
    let layout = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "analysis", anchor: "left-bottom", index: 0 });
    layout = dockReducer(layout, { type: "reorder", anchor: "right-top", from: 0, to: 2 });
    expect(order(render("left", layout), "left-bottom")).toEqual(["analysis", "compose"]);
    expect(order(render("right", layout), "right-top")).toEqual(["processing", "stacking", "headers"]);
  });

  it("presses exactly the active tool of each group and nothing in a closed group", () => {
    const left = render("left");
    expect(pressed(left, "left-top")).toEqual(["files"]);
    expect(pressed(left, "left-bottom")).toEqual(["compose"]);
    const right = render("right");
    expect(pressed(right, "right-top")).toEqual([]);
    expect(pressed(right, "right-bottom")).toEqual([]);
    const opened = dockReducer(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "open", tool: "stacking" }), { type: "open", tool: "export" });
    const html = render("right", opened);
    expect(pressed(html, "right-top")).toEqual(["stacking"]);
    expect(pressed(html, "right-bottom")).toEqual(["export"]);
    for (const anchor of GROUPS.right) {
      expect(group(html, anchor).every((b) => b["aria-pressed"] === "true" || b["aria-pressed"] === "false")).toBe(true);
    }
  });

  it("disables only the tools that need a file while there is none, and keeps them in place", () => {
    for (const side of ["left", "right"] as const) {
      for (const anchor of GROUPS[side]) {
        const withFile = group(render(side, DEFAULT_DOCK_LAYOUT, true), anchor);
        const noFile = group(render(side, DEFAULT_DOCK_LAYOUT, false), anchor);
        expect(withFile.some((b) => "aria-disabled" in b)).toBe(false);
        expect(noFile.map((b) => b["data-tool-id"])).toEqual(DEFAULT_DOCK_LAYOUT.anchors[anchor]);
        for (const b of noFile) {
          const needsFile = DOCK_TOOL_META[b["data-tool-id"] as DockToolId].needsFile;
          expect(b["aria-disabled"]).toBe(needsFile ? "true" : undefined);
        }
      }
    }
  });

  it("marks every button with its tool, anchor, label and menu popup", () => {
    for (const side of ["left", "right"] as const) {
      const html = render(side);
      for (const anchor of GROUPS[side]) {
        for (const b of group(html, anchor)) {
          const meta = DOCK_TOOL_META[b["data-tool-id"] as DockToolId];
          expect(b.type).toBe("button");
          expect(b["data-anchor"]).toBe(anchor);
          expect(b["aria-haspopup"]).toBe("menu");
          expect(b.title).toBe(meta.label);
        }
      }
    }
    expect(render("left")).toContain(">Comp</span>");
    expect(render("right")).toContain(">Config</span>");
  });

  it("renders both group wrappers even when a group is empty", () => {
    const layout = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "compose", anchor: "left-top" });
    const html = render("left", layout);
    expect(order(html, "left-bottom")).toEqual([]);
    expect(order(html, "left-top")).toEqual(["files", "info", "compose"]);
    expect(pressed(html, "left-top")).toEqual(["compose"]);
  });
});

const NONE: AnchorTools = { "left-top": null, "left-bottom": null, "right-top": null, "right-bottom": null };

function tools(over: Partial<AnchorTools>): AnchorTools {
  return { ...NONE, ...over };
}

describe("dock shown state (close animation)", () => {
  it("shows what is open and keeps the same record while nothing changes", () => {
    const open = tools({ "left-top": "files", "left-bottom": "compose" });
    const shown = nextShown(NONE, open);
    expect(shown).toEqual(open);
    expect(nextShown(shown, open)).toBe(shown);
  });

  it("clears a bottom half at once when it closes beside an open half", () => {
    const shown = tools({ "left-bottom": "compose", "right-bottom": "synth" });
    expect(nextShown(shown, tools({ "left-bottom": "compose" }))).toEqual(tools({ "left-bottom": "compose" }));
  });

  it("keeps the last bottom half until the area height settles", () => {
    const shown = tools({ "left-bottom": "compose" });
    const kept = nextShown(shown, NONE);
    expect(kept).toBe(shown);
    expect(settleBottom(kept, NONE)).toEqual(NONE);
  });

  it("keeps both halves when they close together until the area settles", () => {
    const shown = tools({ "left-bottom": "compose", "right-bottom": "synth" });
    expect(nextShown(shown, NONE)).toBe(shown);
    expect(settleBottom(shown, NONE)).toEqual(NONE);
  });

  it("does not bring back Synth, closed earlier, when Compose then closes", () => {
    const synthClosed = nextShown(tools({ "left-bottom": "compose", "right-bottom": "synth" }), tools({ "left-bottom": "compose" }));
    const composeClosed = nextShown(synthClosed, NONE);
    expect(composeClosed).toEqual(tools({ "left-bottom": "compose" }));
    expect(settleBottom(composeClosed, NONE)).toEqual(NONE);
  });

  it("drops a closing last half at once when the other half opens before the area settles", () => {
    const closing = nextShown(tools({ "right-bottom": "synth" }), NONE);
    expect(closing).toEqual(tools({ "right-bottom": "synth" }));
    expect(nextShown(closing, tools({ "left-bottom": "compose" }))).toEqual(tools({ "left-bottom": "compose" }));
  });

  it("ignores an area settle while a half is open", () => {
    const shown = tools({ "left-bottom": "compose" });
    expect(settleBottom(shown, tools({ "left-bottom": "compose" }))).toBe(shown);
  });

  it("keeps a side anchor until its own width transition ends", () => {
    const shown = nextShown(tools({ "left-top": "files", "right-top": "analysis" }), tools({ "left-top": "files" }));
    expect(shown).toEqual(tools({ "left-top": "files", "right-top": "analysis" }));
    const open = tools({ "left-top": "files" });
    expect(settleSide(shown, open, "left-top")).toBe(shown);
    expect(settleBottom(shown, open)).toBe(shown);
    expect(settleSide(shown, open, "right-top")).toEqual(tools({ "left-top": "files" }));
  });

  it("follows a reopen before the close settles and ignores the late settle", () => {
    const closing = nextShown(tools({ "right-top": "analysis" }), NONE);
    const reopened = nextShown(closing, tools({ "right-top": "analysis" }));
    expect(reopened).toBe(closing);
    expect(settleSide(reopened, tools({ "right-top": "analysis" }), "right-top")).toBe(reopened);
    expect(nextShown(closing, tools({ "right-top": "headers" }))).toEqual(tools({ "right-top": "headers" }));
  });

  it("shows a tool moved away from a closing anchor at its new anchor at once and clears the old one on its own settle", () => {
    const closing = nextShown(tools({ "right-top": "analysis" }), NONE);
    const open = tools({ "left-bottom": "analysis" });
    const moved = nextShown(closing, open);
    expect(moved).toEqual(tools({ "right-top": "analysis", "left-bottom": "analysis" }));
    expect(settleBottom(moved, open)).toBe(moved);
    expect(settleSide(moved, open, "right-top")).toEqual(open);
  });

  it("returns the same record from a settle that has nothing to clear", () => {
    expect(settleSide(NONE, NONE, "left-top")).toBe(NONE);
    expect(settleBottom(NONE, NONE)).toBe(NONE);
  });
});

type MoveListener = (ev: { clientX: number; clientY: number }) => void;

interface FakeWindow {
  addEventListener(type: string, fn: MoveListener): void;
  removeEventListener(type: string, fn: MoveListener): void;
  fire(type: string, at?: number): void;
  count(): number;
}

function fakeWindow(): FakeWindow {
  const listeners = new Map<string, Set<MoveListener>>();
  return {
    addEventListener(type, fn) {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type)?.add(fn);
    },
    removeEventListener(type, fn) {
      listeners.get(type)?.delete(fn);
    },
    fire(type, at = 0) {
      for (const fn of [...(listeners.get(type) ?? [])]) fn({ clientX: at, clientY: at });
    },
    count() {
      let n = 0;
      for (const set of listeners.values()) n += set.size;
      return n;
    },
  };
}

describe("resize handle drag", () => {
  let win: FakeWindow;
  let body: { style: { cursor?: string; userSelect?: string } };
  let dispatch: MockInstance<typeof dockStore.dispatch>;

  beforeEach(() => {
    win = fakeWindow();
    body = { style: {} };
    vi.stubGlobal("window", win);
    vi.stubGlobal("document", { body });
    dispatch = vi.spyOn(dockStore, "dispatch").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  function drag(over: Partial<ResizeSession> = {}, axis: "x" | "y" = "x", sizeKey: "leftW" | "bottomH" | "bottomSplit" = "leftW") {
    const frozen = { style: { transition: "" } };
    const handle = { dataset: {} as Record<string, string> };
    const session: ResizeSession = {
      start: 512,
      min: 180,
      max: 900,
      direction: 1,
      apply: vi.fn<(value: number) => void>(),
      commit: vi.fn((value: number) => value),
      frozen: [frozen as unknown as HTMLElement],
      ...over,
    };
    const onActiveChange = vi.fn<(active: boolean) => void>();
    startResizeDrag({ sizeKey, axis, origin: 100, handle: handle as unknown as HTMLElement, session, onActiveChange });
    return { session, handle, frozen, onActiveChange };
  }

  it("does not save anything for a click without movement, and still cleans up", () => {
    const { session, handle, frozen, onActiveChange } = drag();
    expect(body.style.cursor).toBe("col-resize");
    expect(handle.dataset.dragging).toBe("true");
    expect(frozen.style.transition).toBe("none");
    win.fire("mouseup");
    expect(dispatch).not.toHaveBeenCalled();
    expect(session.apply).not.toHaveBeenCalled();
    expect(session.commit).not.toHaveBeenCalled();
    expect(win.count()).toBe(0);
    expect(body.style.cursor).toBe("");
    expect(body.style.userSelect).toBe("");
    expect(handle.dataset.dragging).toBeUndefined();
    expect(frozen.style.transition).toBe("");
    expect(onActiveChange.mock.calls).toEqual([[true], [false]]);
  });

  it("does not count a mousemove at the press point as a drag, even when the start lies past the limits", () => {
    const inside = drag();
    win.fire("mousemove", 100);
    win.fire("mouseup", 100);
    expect(inside.session.apply).not.toHaveBeenCalled();
    const past = drag({ start: 700, max: 600 });
    win.fire("mousemove", 100);
    win.fire("mouseup", 100);
    expect(past.session.apply).not.toHaveBeenCalled();
    expect(dispatch).not.toHaveBeenCalled();
  });

  it("does not count a move the limits clamp back to the start as a drag", () => {
    const { session } = drag({ start: 180, min: 180 });
    win.fire("mousemove", 40);
    win.fire("mouseup", 40);
    expect(session.apply).not.toHaveBeenCalled();
    expect(dispatch).not.toHaveBeenCalled();
  });

  it("applies every step of a real drag and saves the committed value once", () => {
    const { session } = drag();
    win.fire("mousemove", 140);
    win.fire("mousemove", 160);
    win.fire("mouseup", 160);
    expect(session.apply).toHaveBeenNthCalledWith(1, 552);
    expect(session.apply).toHaveBeenNthCalledWith(2, 572);
    expect(dispatch).toHaveBeenCalledTimes(1);
    expect(dispatch).toHaveBeenCalledWith({ type: "resize", key: "leftW", value: 572 });
    win.fire("mousemove", 300);
    expect(session.apply).toHaveBeenCalledTimes(2);
  });

  it("saves after a drag that returns to the start, so the store matches what was applied", () => {
    const { session } = drag();
    win.fire("mousemove", 140);
    win.fire("mousemove", 100);
    win.fire("mouseup", 100);
    expect(session.apply).toHaveBeenLastCalledWith(512);
    expect(dispatch).toHaveBeenCalledWith({ type: "resize", key: "leftW", value: 512 });
  });

  it("drags the bottom area on y with the inverted direction and the split through commit", () => {
    const area = drag({ start: 280, min: 140, max: 600, direction: -1 }, "y", "bottomH");
    expect(body.style.cursor).toBe("row-resize");
    win.fire("mousemove", 50);
    win.fire("mouseup", 50);
    expect(area.session.apply).toHaveBeenCalledWith(330);
    expect(dispatch).toHaveBeenLastCalledWith({ type: "resize", key: "bottomH", value: 330 });
    const split = drag({ start: 400, min: 100, max: 700, commit: (value: number) => value / 800 }, "x", "bottomSplit");
    win.fire("mousemove", 140);
    win.fire("mouseup", 140);
    expect(split.session.apply).toHaveBeenCalledWith(440);
    expect(dispatch).toHaveBeenLastCalledWith({ type: "resize", key: "bottomSplit", value: 0.55 });
  });
});
