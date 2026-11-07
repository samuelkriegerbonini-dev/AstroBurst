import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import { hooks } from "../../test/hookRuntime";
import { dockStore } from "../../hooks/useDockLayout";
import { createToolShortcutsStore, toolShortcutsStore } from "../../hooks/useToolShortcuts";
import { DEFAULT_TOOL_SHORTCUTS, TOOL_SHORTCUTS_KEY } from "../../utils/toolCatalog";
import type { DockToolId } from "../../utils/dockLayout";
import DockShell from "../dock/DockShell";
import Stripe from "../dock/Stripe";
import ToolBrowser from "../dock/ToolBrowser";
import DockContextMenu from "../dock/DockContextMenu";
import ResizeHandle, { type ResizeSession } from "../dock/ResizeHandle";
import SideColumn from "../dock/SideColumn";
import { toolBrowserStore } from "../dock/toolBrowserStore";

vi.mock("react", async (original) => {
  const { reactWithFakeHooks } = await import("../../test/hookRuntime");
  return reactWithFakeHooks(await original<Record<string, unknown>>());
});

vi.mock("../../context/PreviewContext", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  useFileContext: () => ({ file: { id: "1", path: "/data/m51.fits", name: "m51.fits" } }),
  useCubeContext: () => ({ isCube: false, isSpectrum: false, ramp: null, cubeFileKey: null }),
}));

type Node = { type: unknown; props: Record<string, unknown> };

function elements(node: ReactNode): Node[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Record<string, unknown>>(node)) return [];
  return [{ type: node.type, props: node.props }, ...elements(node.props.children as ReactNode)];
}

function shell(): Node[] {
  return elements(hooks.render(DockShell, { tools: {}, children: null }) as ReactNode);
}

function strip(tree: Node[], side: "left" | "right"): Node {
  return tree.find((node) => node.type === Stripe && node.props.side === side)!;
}

function railTools(tree: Node[]): readonly DockToolId[] {
  return strip(tree, "right").props.enabledTools as readonly DockToolId[];
}

function openBrowser(): Node {
  (strip(shell(), "right").props.onBrowseTools as () => void)();
  return shell().find((node) => node.type === ToolBrowser)!;
}

function memoryStorage() {
  const map = new Map<string, string>();
  return {
    map,
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => {
      map.set(key, value);
    },
    removeItem: (key: string) => {
      map.delete(key);
    },
  };
}

let storage: ReturnType<typeof memoryStorage>;

beforeEach(() => {
  storage = memoryStorage();
  vi.stubGlobal("localStorage", storage);
  hooks.mount();
  toolBrowserStore.close();
  toolShortcutsStore.reset();
  dockStore.dispatch({ type: "reset" });
});

afterEach(() => {
  hooks.mount();
  toolBrowserStore.close();
  toolShortcutsStore.reset();
  dockStore.dispatch({ type: "reset" });
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("DockShell sidebar shortcuts", () => {
  it("unpins an open pinned Image without dispatching anything, so its panel stays open", () => {
    dockStore.dispatch({ type: "open", tool: "image" });
    const browser = openBrowser();
    const dispatch = vi.spyOn(dockStore, "dispatch");
    (browser.props.onToggleTool as (tool: DockToolId) => void)("image");
    expect(dispatch).not.toHaveBeenCalled();
    expect(dockStore.get().active["right-top"]).toBe("image");
    expect(toolShortcutsStore.get()).not.toContain("image");
  });

  it("pins a closed tool without dispatching anything", () => {
    const browser = openBrowser();
    const dispatch = vi.spyOn(dockStore, "dispatch");
    (browser.props.onToggleTool as (tool: DockToolId) => void)("stacking");
    expect(dispatch).not.toHaveBeenCalled();
    expect(toolShortcutsStore.get()).toContain("stacking");
    expect(railTools(shell())).toContain("stacking");
  });

  it("round-trips the shortcut list through ab.tools.shortcuts.v1", () => {
    const browser = openBrowser();
    const toggle = browser.props.onToggleTool as (tool: DockToolId) => void;
    toggle("stacking");
    toggle("files");
    const saved = JSON.parse(storage.map.get(TOOL_SHORTCUTS_KEY) ?? "null") as DockToolId[];
    expect(saved).toEqual(["info", "compose", "headers", "image", "astrometry", "photometry", "cube", "processing", "history", "stacking", "export", "config", "log"]);
    expect(toolShortcutsStore.get()).toEqual(saved);
    expect(createToolShortcutsStore(() => storage).get()).toEqual(saved);
  });

  it("follows the shortcut store when it changes outside the dialog", () => {
    shell();
    toolShortcutsStore.toggle("synth");
    expect(railTools(shell())).toContain("synth");
    toolShortcutsStore.reset();
    expect(railTools(shell())).not.toContain("synth");
  });

  it("keeps a tool opened this session on the rail after it closes, without pinning it", () => {
    shell();
    dockStore.dispatch({ type: "open", tool: "sagui" });
    shell();
    dockStore.dispatch({ type: "hide", tool: "sagui" });
    const tree = shell();
    expect(dockStore.get().active["right-top"]).toBeNull();
    expect(railTools(tree)).toContain("sagui");
    expect(strip(tree, "left").props.enabledTools).toEqual(railTools(tree));
    expect(toolShortcutsStore.get()).not.toContain("sagui");
    const menu = tree.find((node) => node.type === DockContextMenu)!;
    expect(menu.props.enabledTools).toEqual(railTools(tree));
    const browser = openBrowser();
    expect(browser.props.enabledTools).toEqual(toolShortcutsStore.get());
  });

  it("drops the tools opened this session from the rail on Reset layout, but not on a shortcut toggle", () => {
    shell();
    dockStore.dispatch({ type: "open", tool: "sagui" });
    shell();
    dockStore.dispatch({ type: "hide", tool: "sagui" });
    toolShortcutsStore.toggle("stacking");
    expect(railTools(shell())).toEqual(expect.arrayContaining(["sagui", "stacking"]));
    dockStore.resetAll();
    const reset = shell();
    expect(railTools(reset)).toEqual(DEFAULT_TOOL_SHORTCUTS);
    expect(strip(reset, "left").props.enabledTools).toEqual(DEFAULT_TOOL_SHORTCUTS);
    dockStore.dispatch({ type: "open", tool: "synth" });
    shell();
    dockStore.dispatch({ type: "hide", tool: "synth" });
    expect(railTools(shell())).toContain("synth");
  });
});

describe("DockShell column resize", () => {
  it("caps a column drag at the share of the window width the CSS clamp allows", () => {
    dockStore.dispatch({ type: "open", tool: "sagui" });
    const tree = shell();
    const column = tree.find((node) => node.type === SideColumn && node.props.anchor === "right-top")!;
    (column.props.outerRef as { current: unknown }).current = { style: {} };
    (column.props.innerRef as { current: unknown }).current = { offsetWidth: 380, style: {} };
    const centre = tree.find((node) => node.props["data-dock-centre"] === "")!;
    (centre.props.ref as { current: unknown }).current = { clientWidth: 2000 };
    vi.stubGlobal("window", { innerWidth: 1000 });
    const begin = (nodes: typeof tree) => (nodes.find((node) => node.type === ResizeHandle && node.props.sizeKey === "rightW")!.props.begin as () => ResizeSession | null)();
    expect(begin(tree)).toMatchObject({ start: 380, min: 320, max: 400 });
    dockStore.dispatch({ type: "hide", tool: "files" });
    expect(begin(shell())).toMatchObject({ start: 380, min: 320, max: 600 });
  });
});
