import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { hooks } from "../../test/hookRuntime";
import App from "../../App";
import { fileStore } from "../../hooks/useFileStore";
import { appModeStore } from "../../hooks/useAppMode";
import { dockStore } from "../../hooks/useDockLayout";
import { modifierKey, shortcutSheet } from "../../utils/shortcutSheet";
import type { DockToolId } from "../../utils/dockLayout";
import CommandPalette, { type PaletteAction } from "../CommandPalette";
import DockContextMenu, { DockMenuList } from "../dock/DockContextMenu";
import { openToolBrowserDialog } from "../dock/toolBrowserDialog";
import { rightRailToolsTrigger } from "../dock/dockGeometry";
import { toolBrowserStore } from "../dock/toolBrowserStore";

vi.mock("react", async (original) => {
  const { reactWithFakeHooks } = await import("../../test/hookRuntime");
  return reactWithFakeHooks(await original<Record<string, unknown>>());
});

vi.mock("react-dom", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  createPortal: (children: unknown) => children,
}));

type Node = { type: unknown; props: Record<string, unknown> };
type MenuProps = Parameters<typeof DockMenuList>[0];

const ALL_TOOLS = "All tools";

function elements(node: ReactNode): Node[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Record<string, unknown>>(node)) return [];
  return [{ type: node.type, props: node.props }, ...elements(node.props.children as ReactNode)];
}

function loadProcessedFile(): void {
  fileStore.addFiles([{ name: "m51.fits", path: "/data/m51.fits", size: 1 }]);
  fileStore.fileDone(fileStore.getFiles()[0].id, null);
}

function appPaletteActions(): PaletteAction[] {
  hooks.mount();
  hooks.render(App, {});
  const palette = elements(hooks.render(App, {}) as ReactNode).find((node) => node.type === CommandPalette);
  if (!palette) throw new Error("App renders no command palette");
  return palette.props.actions as PaletteAction[];
}

function allToolsAction(actions: PaletteAction[]): PaletteAction | undefined {
  return actions.find((action) => action.label === ALL_TOOLS);
}

function paletteAfterTyping(actions: PaletteAction[], query: string): Node[] {
  hooks.mount();
  const props = { open: true, onClose: () => {}, actions, files: [], selectedId: null, onSelectFile: () => {} };
  const render = () => elements(hooks.render(CommandPalette, props) as ReactNode);
  const combobox = render().find((node) => node.props.role === "combobox")!;
  (combobox.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: query } });
  return render();
}

function searchAndPressEnter(actions: PaletteAction[], query: string): void {
  const combobox = paletteAfterTyping(actions, query).find((node) => node.props.role === "combobox")!;
  (combobox.props.onKeyDown as (e: { key: string; preventDefault(): void }) => void)({ key: "Enter", preventDefault: () => {} });
}

function resultLabels(actions: PaletteAction[], query: string): string[] {
  return paletteAfterTyping(actions, query)
    .filter((node) => node.props.className === "ab-cmdp-item-label")
    .map((node) => String((node.props.children as unknown[])[0]));
}

function sidebarToolMenu(tool: DockToolId, railButton: object, onClose: () => void): MenuProps {
  const element = DockContextMenu({ request: { tool, trigger: railButton as HTMLButtonElement }, onClose });
  if (!isValidElement<Record<string, unknown>>(element)) throw new Error("Missing popup");
  hooks.mount();
  const popup = hooks.render(element.type as (props: Record<string, unknown>) => unknown, element.props) as ReactElement<{ children: ReactElement<MenuProps> }>;
  return popup.props.children.props;
}

const TOOLS_TRIGGER = '[data-dock-strip="right"] [data-tools-trigger]';

interface FakePage {
  body: { name: string };
  activeElement: unknown;
}

function stubPage(found: Record<string, object[]>): FakePage {
  const body = { name: "body", isConnected: true, getClientRects: () => [{}], focus: vi.fn() };
  const page = {
    body,
    activeElement: body as unknown,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    querySelector: (selector: string) => found[selector]?.[0] ?? null,
    querySelectorAll: (selector: string) => found[selector] ?? [],
  };
  vi.stubGlobal("document", page);
  return page;
}

function focusable(page: FakePage, name: string) {
  let rendered = true;
  const element = {
    name,
    isConnected: true,
    getBoundingClientRect: () => ({ left: 960, top: 100, right: 1000, bottom: 136, width: 40, height: 36 }),
    getClientRects: () => (rendered ? [{}] : []),
    setAttribute: vi.fn(),
    removeAttribute: vi.fn(),
    closest: () => ({}),
    scrollIntoView: vi.fn(),
    focus: vi.fn(() => {
      if (rendered) page.activeElement = element;
    }),
    hide() {
      rendered = false;
      if (page.activeElement === element) page.activeElement = page.body;
    },
  };
  return element;
}

const focusedName = (page: FakePage) => (page.activeElement as { name: string }).name;

beforeEach(() => {
  vi.useFakeTimers();
  hooks.mount();
  fileStore.reset();
  toolBrowserStore.close();
  appModeStore.set("workspace");
  dockStore.dispatch({ type: "reset" });
  vi.stubGlobal("window", { innerWidth: 1000, innerHeight: 800, addEventListener: vi.fn(), removeEventListener: vi.fn() });
  vi.stubGlobal("document", { body: {}, activeElement: null, addEventListener: vi.fn(), removeEventListener: vi.fn(), querySelector: () => null, querySelectorAll: () => [] });
  vi.stubGlobal("HTMLElement", class {});
  vi.stubGlobal("requestAnimationFrame", vi.fn(() => 1));
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
});

afterEach(() => {
  hooks.mount();
  vi.unstubAllGlobals();
  vi.useRealTimers();
  fileStore.reset();
  toolBrowserStore.close();
  appModeStore.set("workspace");
  dockStore.dispatch({ type: "reset" });
});

describe("All tools from the command palette", () => {
  it("is offered once files are in the workspace and opens the tool browser in the workspace, also from the Archive", () => {
    loadProcessedFile();
    const entry = allToolsAction(appPaletteActions());
    expect(entry).toBeDefined();
    appModeStore.set("archive");
    entry!.run();
    expect(appModeStore.get()).toBe("workspace");
    expect(toolBrowserStore.get()).toBe(true);
  });

  it.each(["tools", "sidebar", "pin"])("comes first for '%s', so Enter opens the tool browser", (query) => {
    loadProcessedFile();
    searchAndPressEnter(appPaletteActions(), query);
    expect(toolBrowserStore.get()).toBe(true);
  });

  it("is also listed for 'browse', beside Browse MAST archive", () => {
    loadProcessedFile();
    expect(resultLabels(appPaletteActions(), "browse")).toEqual(expect.arrayContaining([ALL_TOOLS, "Browse MAST archive"]));
  });

  it("is not offered in the empty workspace, where no sidebar is shown", () => {
    expect(allToolsAction(appPaletteActions())).toBeUndefined();
  });

  it("is the entry the shortcut sheet tells keyboard users to pick", () => {
    loadProcessedFile();
    const label = allToolsAction(appPaletteActions())?.label;
    for (const mac of [false, true]) {
      const general = shortcutSheet(mac).find((group) => group.id === "general")?.rows ?? [];
      expect(general.map((row) => row.keys)).toContain(`${modifierKey(mac)}K, then ${label}`);
    }
  });

  it("returns focus to the right rail's All tools trigger when the palette left focus on the page body, as after a click on the viewer or from the Archive", () => {
    loadProcessedFile();
    const entry = allToolsAction(appPaletteActions());
    const found: Record<string, object[]> = {};
    const page = stubPage(found);
    const toolsTrigger = focusable(page, "All tools trigger");
    const search = focusable(page, "tool search");
    found[TOOLS_TRIGGER] = [toolsTrigger];
    entry!.run();
    expect(toolBrowserStore.get()).toBe(true);
    const closeBrowser = openToolBrowserDialog({ showModal: vi.fn(), close: () => search.hide() }, search, page.activeElement as HTMLElement, rightRailToolsTrigger);
    expect(focusedName(page)).toBe("tool search");
    closeBrowser();
    expect(focusedName(page)).toBe("All tools trigger");
  });
});

describe("All tools from the sidebar tool menu", () => {
  it("keeps focus on the rail when the user removes the tool the menu came from and then closes the dialog", () => {
    const found: Record<string, object[]> = {};
    const page = stubPage(found);
    const astrometry = focusable(page, "Astrometry rail button");
    const toolsTrigger = focusable(page, "All tools trigger");
    const search = focusable(page, "tool search");
    found[TOOLS_TRIGGER] = [toolsTrigger];
    found['[data-dock-root] [data-tool-id="astrometry"]'] = [astrometry];
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (frame: FrameRequestCallback) => frames.push(frame));
    const onClose = vi.fn();
    sidebarToolMenu("astrometry", astrometry, onClose).onBrowseTools();
    expect(toolBrowserStore.get()).toBe(true);
    expect(onClose).toHaveBeenCalled();
    const closeBrowser = openToolBrowserDialog({ showModal: vi.fn(), close: () => search.hide() }, search, page.activeElement as HTMLElement, rightRailToolsTrigger);
    for (const frame of frames) frame(0);
    astrometry.hide();
    closeBrowser();
    expect(focusedName(page)).toBe("All tools trigger");
  });

  it("falls back to the rail button the menu came from when the right rail shows no All tools trigger", () => {
    const page = stubPage({});
    const image = focusable(page, "Image rail button");
    const onClose = vi.fn();
    sidebarToolMenu("image", image, onClose).onBrowseTools();
    expect(toolBrowserStore.get()).toBe(true);
    expect(image.focus).toHaveBeenCalledWith({ preventScroll: true });
    expect(focusedName(page)).toBe("Image rail button");
    expect(onClose).toHaveBeenCalled();
  });
});
