import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { hooks } from "../../test/hookRuntime";
import ResizeHandle, { RESIZE_HANDLE_TITLE, type ResizeHandleProps, type ResizeSession } from "../dock/ResizeHandle";
import { resizeKeyMove, resizeTarget } from "../dock/resizeHandleModel";

vi.mock("react", async (original) => {
  const { reactWithFakeHooks } = await import("../../test/hookRuntime");
  return reactWithFakeHooks(await original<Record<string, unknown>>());
});

const store = vi.hoisted(() => ({
  sizes: { leftW: 300, rightW: 380, bottomH: 280, bottomSplit: 0.5 },
  dispatch: vi.fn(),
}));

vi.mock("../../hooks/useDockLayout", () => ({
  dockStore: {
    subscribe: () => () => {},
    get: () => ({ sizes: store.sizes }),
    dispatch: store.dispatch,
  },
}));

type Root = { props: Record<string, unknown> };

const RIGHT: ResizeSession = { start: 380, min: 280, max: 640, direction: -1 as const, apply: vi.fn(), commit: (v: number) => v, frozen: [] };

function render(props: ResizeHandleProps): Root {
  return hooks.render(ResizeHandle as unknown as (p: ResizeHandleProps) => ReactNode, props) as unknown as Root;
}

function key(el: Root, k: string, shiftKey = false) {
  const preventDefault = vi.fn();
  (el.props.onKeyDown as (e: unknown) => void)({ key: k, shiftKey, preventDefault });
  return preventDefault;
}

function rightHandle(begin: () => ResizeSession | null = () => RIGHT, open = true): Root {
  return render({ sizeKey: "rightW", axis: "x", open, begin, onActiveChange: () => {} });
}

beforeEach(() => {
  hooks.mount();
  store.dispatch.mockReset();
});

describe("resizeKeyMove", () => {
  it("maps the keys of its axis to a move of the handle", () => {
    expect(resizeKeyMove("ArrowRight", false, "x")).toEqual({ kind: "step", pixels: 16 });
    expect(resizeKeyMove("ArrowLeft", true, "x")).toEqual({ kind: "step", pixels: -64 });
    expect(resizeKeyMove("ArrowDown", false, "y")).toEqual({ kind: "step", pixels: 16 });
    expect(resizeKeyMove("ArrowUp", false, "y")).toEqual({ kind: "step", pixels: -16 });
    expect(resizeKeyMove("Home", false, "x")).toEqual({ kind: "min" });
    expect(resizeKeyMove("End", true, "y")).toEqual({ kind: "max" });
    expect(resizeKeyMove("ArrowUp", false, "x")).toBeNull();
    expect(resizeKeyMove("ArrowLeft", false, "y")).toBeNull();
    expect(resizeKeyMove("Tab", false, "x")).toBeNull();
    expect(resizeKeyMove("a", false, "y")).toBeNull();
  });
});

describe("resizeTarget", () => {
  it("moves a size by the step, in the direction the drag would", () => {
    expect(resizeTarget({ kind: "step", pixels: -16 }, RIGHT)).toBe(396);
    expect(resizeTarget({ kind: "step", pixels: 16 }, { start: 300, min: 180, max: 640, direction: 1 })).toBe(316);
    expect(resizeTarget({ kind: "step", pixels: -16 }, { start: 280, min: 140, max: 600, direction: -1 })).toBe(296);
  });

  it("stops at the limits the drag has", () => {
    expect(resizeTarget({ kind: "step", pixels: -64 }, { ...RIGHT, start: 630 })).toBe(640);
    expect(resizeTarget({ kind: "step", pixels: 64 }, { ...RIGHT, start: 290 })).toBe(280);
    expect(resizeTarget({ kind: "min" }, RIGHT)).toBe(280);
    expect(resizeTarget({ kind: "max" }, RIGHT)).toBe(640);
    expect(resizeTarget({ kind: "max" }, { ...RIGHT, max: 200 })).toBe(280);
    expect(resizeTarget({ kind: "step", pixels: -64 }, { ...RIGHT, max: 200 })).toBe(280);
  });
});

describe("ResizeHandle", () => {
  it("is a named, focusable separator with its value", () => {
    const el = rightHandle();
    expect(el.props.role).toBe("separator");
    expect(el.props.tabIndex).toBe(0);
    expect(el.props["aria-orientation"]).toBe("vertical");
    expect(el.props["aria-label"]).toBe("Right panel width");
    expect(el.props["aria-valuenow"]).toBe(380);
    expect(el.props["aria-valuemin"]).toBe(280);
    expect(el.props["aria-valuemax"]).toBe(640);
    expect(el.props["aria-valuetext"]).toBe("380 px");
    expect(el.props.title).toBe(RESIZE_HANDLE_TITLE);
    expect(el.props["data-dock-handle"]).toBe("rightW");
    expect(el.props["data-open"]).toBe("true");
    expect(el.props.className).toBe("ab-resize-handle");
  });

  it("names the bottom handle and the split in their own units", () => {
    const bottom = render({ sizeKey: "bottomH", axis: "y", open: true, begin: () => null, onActiveChange: () => {} });
    expect(bottom.props["aria-orientation"]).toBe("horizontal");
    expect(bottom.props["aria-label"]).toBe("Bottom panel height");
    expect(bottom.props["aria-valuetext"]).toBe("280 px");
    expect(bottom.props.className).toBe("ab-resize-handle-h");
    hooks.mount();
    const split = render({ sizeKey: "bottomSplit", axis: "x", open: true, begin: () => null, onActiveChange: () => {} });
    expect(split.props["aria-valuenow"]).toBe(50);
    expect(split.props["aria-valuemin"]).toBe(25);
    expect(split.props["aria-valuemax"]).toBe(75);
    expect(split.props["aria-valuetext"]).toBe("50 %");
    expect(split.props["aria-label"]).toBe("Bottom panel split");
  });

  it("resizes by 16 px with an arrow and by 64 px with Shift", () => {
    const el = rightHandle();
    const prevented = key(el, "ArrowLeft");
    expect(store.dispatch).toHaveBeenLastCalledWith({ type: "resize", key: "rightW", value: 396 });
    expect(prevented).toHaveBeenCalled();
    key(el, "ArrowLeft", true);
    expect(store.dispatch).toHaveBeenLastCalledWith({ type: "resize", key: "rightW", value: 444 });
  });

  it("jumps to the limits with Home and End", () => {
    const el = rightHandle();
    key(el, "Home");
    expect(store.dispatch).toHaveBeenLastCalledWith({ type: "resize", key: "rightW", value: 280 });
    key(el, "End");
    expect(store.dispatch).toHaveBeenLastCalledWith({ type: "resize", key: "rightW", value: 640 });
  });

  it("resets with Enter without measuring the dock", () => {
    const begin = vi.fn(() => RIGHT);
    const el = rightHandle(begin);
    const prevented = key(el, "Enter");
    expect(store.dispatch).toHaveBeenCalledWith({ type: "resize", key: "rightW", value: 380 });
    expect(prevented).toHaveBeenCalled();
    expect(begin).not.toHaveBeenCalled();
  });

  it("sends the split as the fraction the drag sends", () => {
    const el = render({
      sizeKey: "bottomSplit",
      axis: "x",
      open: true,
      begin: () => ({ start: 400, min: 200, max: 600, direction: 1, apply: vi.fn(), commit: (v: number) => v / 800, frozen: [] }),
      onActiveChange: () => {},
    });
    key(el, "ArrowRight");
    expect(store.dispatch).toHaveBeenCalledWith({ type: "resize", key: "bottomSplit", value: 0.52 });
  });

  it("leaves the page alone at a limit, for other keys, without a session and while closed", () => {
    const atMax = rightHandle(() => ({ ...RIGHT, start: 640 }));
    const preventedAtMax = key(atMax, "ArrowLeft");
    expect(store.dispatch).not.toHaveBeenCalled();
    expect(preventedAtMax).toHaveBeenCalled();

    const begin = vi.fn(() => RIGHT);
    const el = rightHandle(begin);
    expect(key(el, "Tab")).not.toHaveBeenCalled();
    expect(key(el, "ArrowUp")).not.toHaveBeenCalled();
    expect(begin).not.toHaveBeenCalled();

    const noSession = rightHandle(() => null);
    expect(key(noSession, "ArrowLeft")).not.toHaveBeenCalled();
    expect(store.dispatch).not.toHaveBeenCalled();

    const closedBegin = vi.fn(() => RIGHT);
    const closed = rightHandle(closedBegin, false);
    expect(closed.props.tabIndex).toBe(-1);
    expect(key(closed, "ArrowLeft")).not.toHaveBeenCalled();
    expect(key(closed, "Enter")).not.toHaveBeenCalled();
    expect(closedBegin).not.toHaveBeenCalled();
    expect(store.dispatch).not.toHaveBeenCalled();
  });

  it("keeps the mouse path", () => {
    const el = rightHandle();
    expect(typeof el.props.onMouseDown).toBe("function");
    expect(typeof el.props.onDoubleClick).toBe("function");
    (el.props.onDoubleClick as () => void)();
    expect(store.dispatch).toHaveBeenCalledWith({ type: "resize", key: "rightW", value: 380 });
  });
});
