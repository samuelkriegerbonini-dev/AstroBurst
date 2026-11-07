import { createElement, isValidElement, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hooks } from "../../test/hookRuntime";
import { dockStore } from "../../hooks/useDockLayout";
import { DEFAULT_DOCK_LAYOUT, type DockToolId } from "../../utils/dockLayout";
import DockContextMenu, { DockMenuList } from "../dock/DockContextMenu";

const { announceSpy } = vi.hoisted(() => ({ announceSpy: vi.fn<(text: string) => void>() }));

vi.mock("react", async (original) => {
  const { reactWithFakeHooks } = await import("../../test/hookRuntime");
  return reactWithFakeHooks(await original<Record<string, unknown>>());
});

vi.mock("react-dom", async (original) => ({
  ...await original<Record<string, unknown>>(),
  createPortal: (children: unknown) => children,
}));

vi.mock("../dock/useDockDrag", async (original) => ({ ...(await original<Record<string, unknown>>()), announce: announceSpy }));

const enabledTools: readonly DockToolId[] = ["image", "processing", "history"];
const group = () => dockStore.get().anchors["right-top"];
const trigger = {
  isConnected: true,
  getBoundingClientRect: () => ({ left: 960, top: 100, right: 1000, bottom: 136, width: 40, height: 36 }),
  setAttribute: vi.fn(),
  removeAttribute: vi.fn(),
};

type MenuProps = Parameters<typeof DockMenuList>[0];

function menu(tool: DockToolId, enabled?: readonly DockToolId[], onClose: () => void = () => {}, button: object = trigger): MenuProps {
  const element = DockContextMenu({ request: { tool, trigger: button as unknown as HTMLButtonElement }, onClose, enabledTools: enabled });
  if (!isValidElement<Record<string, unknown>>(element)) throw new Error("Missing popup");
  const popup = hooks.render(element.type as (props: Record<string, unknown>) => unknown, element.props) as ReactElement<{ children: ReactElement<MenuProps> }>;
  return popup.props.children.props;
}

function disabled(props: MenuProps, label: string): boolean {
  const html = renderToStaticMarkup(createElement(DockMenuList, props));
  const button = [...html.matchAll(/<button\b([^>]*)>([\s\S]*?)<\/button>/g)].find((match) => match[2].replace(/<[^>]*>/g, "").trim() === label);
  if (!button) throw new Error(`Missing ${label}`);
  return button[1].includes('aria-disabled="true"');
}

beforeEach(() => {
  hooks.mount();
  dockStore.dispatch({ type: "reset" });
  announceSpy.mockClear();
  vi.stubGlobal("document", { body: {}, addEventListener: vi.fn(), removeEventListener: vi.fn() });
  vi.stubGlobal("window", { innerWidth: 1000, innerHeight: 800, addEventListener: vi.fn(), removeEventListener: vi.fn() });
  vi.stubGlobal("requestAnimationFrame", vi.fn(() => 1));
});

afterEach(() => {
  hooks.mount();
  vi.unstubAllGlobals();
  dockStore.dispatch({ type: "reset" });
});

describe("Compact dock context menu", () => {
  it("moves up past hidden tools to the previous visible shortcut in one action, announces it and closes", () => {
    const onClose = vi.fn();
    menu("processing", enabledTools, onClose).onReorder(-1);
    expect(group()).toEqual(["headers", "processing", "image", "astrometry", "photometry", "cube", "cubeprep", "sagui", "history", "masks", "stacking"]);
    expect(announceSpy.mock.calls).toEqual([["Processing moved up"]]);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("moves down past hidden tools to the next visible shortcut in one action, announces it and closes", () => {
    const onClose = vi.fn();
    menu("image", enabledTools, onClose).onReorder(1);
    expect(group()).toEqual(["headers", "astrometry", "photometry", "cube", "cubeprep", "sagui", "processing", "image", "history", "masks", "stacking"]);
    expect(announceSpy.mock.calls).toEqual([["Image moved down"]]);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("disables moves beyond the first and last visible shortcuts", () => {
    const first = menu("image", enabledTools);
    expect(disabled(first, "Move up")).toBe(true);
    expect(disabled(first, "Move down")).toBe(false);
    hooks.mount();
    const last = menu("history", enabledTools);
    expect(disabled(last, "Move up")).toBe(false);
    expect(disabled(last, "Move down")).toBe(true);
  });

  it("includes an active unpinned tool as a visible neighbor", () => {
    dockStore.dispatch({ type: "open", tool: "cube" });
    menu("processing", enabledTools).onReorder(-1);
    expect(group()).toEqual(["headers", "image", "astrometry", "photometry", "processing", "cube", "cubeprep", "sagui", "history", "masks", "stacking"]);
    expect(dockStore.get().active["right-top"]).toBe("cube");
    expect(announceSpy.mock.calls).toEqual([["Processing moved up"]]);
  });

  it("counts the requested tool as visible when it is neither pinned nor open", () => {
    const onClose = vi.fn();
    const props = menu("astrometry", enabledTools, onClose);
    expect(disabled(props, "Move up")).toBe(false);
    expect(disabled(props, "Move down")).toBe(false);
    props.onReorder(-1);
    expect(group()).toEqual(["headers", "astrometry", "image", "photometry", "cube", "cubeprep", "sagui", "processing", "history", "masks", "stacking"]);
    expect(announceSpy.mock.calls).toEqual([["Astrometry moved up"]]);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("disables both directions for a lone visible tool and ignores unavailable moves without closing or announcing", () => {
    const onClose = vi.fn();
    const props = menu("processing", ["processing"], onClose);
    expect(disabled(props, "Move up")).toBe(true);
    expect(disabled(props, "Move down")).toBe(true);
    props.onReorder(-1);
    props.onReorder(1);
    expect(dockStore.get()).toBe(DEFAULT_DOCK_LAYOUT);
    expect(announceSpy).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("preserves adjacent full-layout reordering when no shortcuts are filtered", () => {
    menu("processing").onReorder(-1);
    expect(group()).toEqual(["headers", "image", "astrometry", "photometry", "cube", "cubeprep", "processing", "sagui", "history", "masks", "stacking"]);
    expect(announceSpy.mock.calls).toEqual([["Processing moved up"]]);
  });

  it("announces that Reset restored the layout and the sidebar shortcuts, then closes", () => {
    dockStore.dispatch({ type: "open", tool: "cube" });
    const onClose = vi.fn();
    menu("image", enabledTools, onClose).onReset();
    expect(dockStore.get()).toBe(DEFAULT_DOCK_LAYOUT);
    expect(announceSpy.mock.calls).toEqual([["Layout and sidebar shortcuts reset"]]);
    expect(onClose).toHaveBeenCalledOnce();
  });
});

describe("Compact dock context menu focus return", () => {
  const TOOLS_TRIGGER = '[data-dock-strip="right"] [data-tools-trigger]';

  function railButton(rendered: boolean, connected = true) {
    return { ...trigger, isConnected: connected, getClientRects: () => (rendered ? [{}] : []), focus: vi.fn(), closest: () => ({}), scrollIntoView: vi.fn() };
  }

  function closeAndRunFrames(tool: DockToolId, menuTrigger: object, toolButtons: object[], toolsTrigger: object, act: (props: MenuProps) => void): void {
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", vi.fn((frame: FrameRequestCallback) => frames.push(frame)));
    const found: Record<string, object[]> = { [TOOLS_TRIGGER]: [toolsTrigger], [`[data-dock-root] [data-tool-id="${tool}"]`]: toolButtons };
    const body = {};
    vi.stubGlobal("document", {
      body,
      activeElement: body,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      querySelector: (selector: string) => found[selector]?.[0] ?? null,
      querySelectorAll: (selector: string) => found[selector] ?? [],
    });
    act(menu(tool, [...enabledTools, tool], () => {}, menuTrigger));
    for (const frame of frames) frame(0);
  }

  it("moves focus to the right rail's All tools trigger when Reset hides the tool the menu came from", () => {
    const hidden = railButton(false);
    const toolsTrigger = railButton(true);
    closeAndRunFrames("masks", hidden, [hidden], toolsTrigger, (props) => props.onReset());
    expect(hidden.focus).not.toHaveBeenCalled();
    expect(toolsTrigger.focus).toHaveBeenCalledWith({ preventScroll: true });
  });

  it("returns focus to the tool's rail button when it is still shown", () => {
    const shown = railButton(true);
    const toolsTrigger = railButton(true);
    closeAndRunFrames("image", shown, [shown], toolsTrigger, (props) => props.onReset());
    expect(shown.focus).toHaveBeenCalledWith({ preventScroll: true });
    expect(toolsTrigger.focus).not.toHaveBeenCalled();
  });

  it("follows a moved tool to its new rail button", () => {
    const detached = railButton(false, false);
    const moved = railButton(true);
    const toolsTrigger = railButton(true);
    closeAndRunFrames("image", detached, [moved], toolsTrigger, (props) => props.onMove("left-top"));
    expect(dockStore.get().anchors["left-top"]).toContain("image");
    expect(detached.focus).not.toHaveBeenCalled();
    expect(moved.focus).toHaveBeenCalledWith({ preventScroll: true });
    expect(toolsTrigger.focus).not.toHaveBeenCalled();
  });
});
