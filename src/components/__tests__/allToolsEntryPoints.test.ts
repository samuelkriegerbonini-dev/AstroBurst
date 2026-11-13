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
import DockShell from "../dock/DockShell";
import { openToolBrowserDialog } from "../dock/toolBrowserDialog";
import { rightRailToolsTrigger } from "../dock/dockGeometry";
import { toolBrowserStore } from "../dock/toolBrowserStore";
import { productFilterStore } from "../../hooks/useProductFilter";
import type { PaletteFile } from "../CommandPalette";
import type { ProcessResult } from "../../shared/types/fits.types";

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

describe("Open spectrum (CSV) from the command palette", () => {
  it("is not offered while no file is shown, where the Cube tool that would show the CSV cannot open", () => {
    fileStore.addFiles([{ name: "catalogue.fits", path: "/data/catalogue.fits", size: 1 }]);
    fileStore.fileError(fileStore.getFiles()[0].id, "no HDU in this file holds a 2D image");
    const ids = appPaletteActions().map((action) => action.id);
    expect(ids).toContain("reset-layout");
    expect(ids).not.toContain("open-spectrum-csv");
  });

  it("is offered while a file is shown", () => {
    loadProcessedFile();
    expect(appPaletteActions().map((action) => action.id)).toContain("open-spectrum-csv");
  });
});

describe("Keyboard shortcut sheet", () => {
  const renderApp = () => elements(hooks.render(App, {}) as ReactNode);
  const sheetIn = (nodes: Node[]) => nodes.find((node) => node.props["data-testid"] === "shortcuts-sheet");

  it("closes itself on Esc and keeps the key from the window listeners, so a polygon being drawn survives", () => {
    hooks.mount();
    const palette = renderApp().find((node) => node.type === CommandPalette);
    (palette?.props.actions as PaletteAction[]).find((action) => action.id === "shortcuts")?.run();
    const sheet = sheetIn(renderApp());
    expect(sheet).toBeDefined();
    const escape = { key: "Escape", stopPropagation: vi.fn() };
    (sheet?.props.onKeyDown as (e: typeof escape) => void)(escape);
    expect(escape.stopPropagation).toHaveBeenCalledTimes(1);
    expect(sheetIn(renderApp())).toBeUndefined();
  });

  it("names the close button of the sheet and gives it a button type and a 24 px target", () => {
    hooks.mount();
    const palette = renderApp().find((node) => node.type === CommandPalette);
    (palette?.props.actions as PaletteAction[]).find((action) => action.id === "shortcuts")?.run();
    const nodes = renderApp();
    expect(sheetIn(nodes)).toBeDefined();
    const close = nodes.find((node) => node.type === "button" && node.props["aria-label"] === "Close keyboard shortcuts");
    expect(close).toBeDefined();
    expect(close!.props.type).toBe("button");
    expect(close!.props.title).toBe("Close");
    expect(String(close!.props.className)).toContain("min-w-6");
    expect(String(close!.props.className)).toContain("min-h-6");
    (close!.props.onClick as () => void)();
    expect(sheetIn(renderApp())).toBeUndefined();
  });
});

describe("Tool rows of the command palette", () => {
  const toolRows = () => appPaletteActions().filter((action) => action.id.startsWith("tool-"));

  it("say Open for every tool while the Archive hides the workspace, and open the tool there", () => {
    loadProcessedFile();
    appModeStore.set("archive");
    const rows = toolRows();
    expect(rows.map((row) => row.label).filter((label) => !label.startsWith("Open "))).toEqual([]);
    rows.find((row) => row.id === "tool-files")?.run();
    expect(appModeStore.get()).toBe("workspace");
    expect(dockStore.get().active["left-top"]).toBe("files");
  });

  it("say Close for a tool on screen in the workspace and close it", () => {
    loadProcessedFile();
    const files = toolRows().find((row) => row.id === "tool-files");
    expect(files?.label).toBe("Close Files");
    files?.run();
    expect(dockStore.get().active["left-top"]).toBeNull();
  });
});

describe("Files in the command palette", () => {
  interface AppPalette {
    files: PaletteFile[];
    onSelectFile: (id: string) => void;
  }

  function appPalette(): AppPalette {
    hooks.mount();
    hooks.render(App, {});
    const palette = elements(hooks.render(App, {}) as ReactNode).find((node) => node.type === CommandPalette);
    if (!palette) throw new Error("App renders no command palette");
    return { files: palette.props.files as PaletteFile[], onSelectFile: palette.props.onSelectFile as (id: string) => void };
  }

  function addFile(name: string): string {
    fileStore.addFiles([{ name, path: `/data/${name}`, size: 1 }]);
    return fileStore.getFiles().find((file) => file.name === name)!.id;
  }

  afterEach(() => {
    productFilterStore.reset();
  });

  it("lists a file under its instrument", () => {
    const id = addFile("jw02739001001_02105_00001_nrca1_cal.fits");
    fileStore.fileDone(id, { header: { INSTRUME: "NIRCAM", FILTER: "F200W" }, png_path: "", previewUrl: "", dimensions: [1, 1], elapsed_ms: 0 } as unknown as ProcessResult);
    const files = appPalette().files;
    expect(files).toHaveLength(1);
    expect(files[0].searchText).toContain("nircam");
    expect(files[0].hidden).toBe(false);
  });

  it("keeps a file the chips hide in the list, marked, and reveals it when chosen", () => {
    const cube = addFile("jw01266-o005_t001_nirspec_g395h-f290lp_s3d.fits");
    fileStore.fileDone(cube, null);
    const spectrum = addFile("jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits");
    fileStore.fileSpectrum(spectrum, { kind: "x1d", hdu: 1 });
    productFilterStore.toggleFilter("x1d");
    const palette = appPalette();
    expect(palette.files.map((f) => [f.name.slice(-8), f.hidden])).toEqual([["s3d.fits", true], ["x1d.fits", false]]);
    appModeStore.set("archive");
    palette.onSelectFile(cube);
    expect(fileStore.getSelected()).toBe(cube);
    expect(productFilterStore.getActiveFilters()).toEqual(["x1d", "s3d"]);
    expect(appModeStore.get()).toBe("workspace");
  });
});

describe("Hide file panel", () => {
  it("moves the focus to the Files button of the rail", () => {
    loadProcessedFile();
    hooks.mount();
    hooks.render(App, {});
    vi.advanceTimersByTime(600);
    const tree = elements(hooks.render(App, {}) as ReactNode);
    const shell = tree.find((node) => node.type === DockShell);
    expect(shell).toBeDefined();
    const list = (shell!.props.tools as { files: { props: { onToggle: () => void } } }).files;
    const found: Record<string, object[]> = {};
    const page = stubPage(found);
    const filesButton = focusable(page, "Files rail button");
    found['[data-dock-strip] [data-tool-id="files"]'] = [filesButton];
    list.props.onToggle();
    expect(dockStore.get().active["left-top"]).not.toBe("files");
    vi.mocked(requestAnimationFrame).mock.calls.at(-1)![0](0);
    expect(focusedName(page)).toBe("Files rail button");
  });
});

describe("Layer role in the Files list", () => {
  it("hands the role of the open response to the row", () => {
    const guess = { role: "inverse_variance", basis: "file name -invvar-", assumed: false };
    fileStore.addFiles([{ name: "legacysurvey-0532m280-invvar-r.fits.fz", path: "/data/legacysurvey-0532m280-invvar-r.fits.fz", size: 1 }]);
    const id = fileStore.getFiles()[0].id;
    fileStore.fileDone(id, { png_path: "", previewUrl: "", dimensions: [1, 1], elapsed_ms: 0, plane: { layer: guess } } as unknown as ProcessResult);
    hooks.mount();
    hooks.render(App, {});
    vi.advanceTimersByTime(600);
    const shell = elements(hooks.render(App, {}) as ReactNode).find((node) => node.type === DockShell);
    expect(shell).toBeDefined();
    const list = (shell!.props.tools as { files: { props: { files: { layer?: unknown }[] } } }).files;
    expect(list.props.files[0].layer).toEqual(guess);
  });
});
