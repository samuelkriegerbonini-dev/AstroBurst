import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { hooks } from "../../test/hookRuntime";
import { elementsOf, textOf } from "../../test/elementTree";
import { DEFAULT_DOCK_LAYOUT, DOCK_TOOL_IDS, DOCK_TOOL_META, dockReducer, type DockLayout, type DockToolId } from "../../utils/dockLayout";
import { DEFAULT_TOOL_SHORTCUTS, TOOL_DESCRIPTIONS } from "../../utils/toolCatalog";
import { dockStore } from "../../hooks/useDockLayout";
import { dockOnboardingStore } from "../dock/dockOnboardingStore";
import ToolBrowser from "../dock/ToolBrowser";
import { openToolBrowserDialog } from "../dock/toolBrowserDialog";

const { announceSpy } = vi.hoisted(() => ({ announceSpy: vi.fn<(text: string) => void>() }));

vi.mock("react", async (original) => {
  const { reactWithFakeHooks } = await import("../../test/hookRuntime");
  return reactWithFakeHooks(await original<Record<string, unknown>>());
});

vi.mock("../dock/useDockDrag", async (original) => ({ ...(await original<Record<string, unknown>>()), announce: announceSpy }));

interface BrowserProps {
  layout: DockLayout;
  hasFile: boolean;
  enabledTools: readonly DockToolId[];
  onToggleTool: Mock<(tool: DockToolId) => void>;
  onClose: Mock<() => void>;
}

interface Rendered {
  type: unknown;
  props: Record<string, unknown>;
}

const SIDEBAR: readonly DockToolId[] = ["image", "processing", "export"];
const NEEDS_FILE: readonly DockToolId[] = ["compose", "headers", "image", "astrometry", "photometry", "cube", "processing", "history", "masks", "stacking", "synth", "export"];

function props(over: Partial<BrowserProps> = {}): BrowserProps {
  return { layout: DEFAULT_DOCK_LAYOUT, hasFile: true, enabledTools: SIDEBAR, onToggleTool: vi.fn(), onClose: vi.fn(), ...over };
}

function render(browser: BrowserProps): Rendered[] {
  return elementsOf(hooks.render(ToolBrowser, browser));
}

function search(browser: BrowserProps, query: string): Rendered[] {
  const input = render(browser).find((node) => node.type === "input")!;
  (input.props.onChange as (event: unknown) => void)({ target: { value: query } });
  return render(browser);
}

function byProp(tree: Rendered[], name: string, value: unknown): Rendered {
  const found = tree.find((node) => node.props[name] === value);
  if (!found) throw new Error(`no element with ${name}=${String(value)}`);
  return found;
}

const card = (tree: Rendered[], tool: DockToolId) => byProp(tree, "data-browser-tool", tool);
const openButton = (tree: Rendered[], tool: DockToolId) => byProp(tree, "data-open-tool", tool);
const toggle = (tree: Rendered[], tool: DockToolId) => byProp(tree, "data-toggle-tool", tool);
const listed = (tree: Rendered[]) => tree.filter((node) => node.props["data-browser-tool"] !== undefined).map((node) => node.props["data-browser-tool"]);
const inCard = (tree: Rendered[], tool: DockToolId, className: string) => elementsOf(card(tree, tool).props.children).find((node) => node.props.className === className);
const dialogOf = (tree: Rendered[]) => tree.find((node) => node.type === "dialog")!;
const click = (node: Rendered) => (node.props.onClick as () => void)();

function described(tree: Rendered[], tool: DockToolId): string {
  const id = openButton(tree, tool).props["aria-describedby"];
  expect(typeof id).toBe("string");
  return textOf(byProp(tree, "id", id).props.children);
}

const readCss = (name: string) => readFileSync(fileURLToPath(new URL(`../dock/${name}`, import.meta.url)), "utf8");

function cssRule(css: string, selector: string): Map<string, string> {
  for (const match of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (!match[1].split(",").some((part) => part.trim().replace(/\s+/g, " ") === selector)) continue;
    return new Map([...match[2].matchAll(/([\w-]+)\s*:\s*([^;]+);/g)].map((declaration) => [declaration[1], declaration[2].trim()]));
  }
  throw new Error(`${selector} is not defined`);
}

function reducedMotionBlock(css: string): string {
  const start = css.indexOf("@media (prefers-reduced-motion: reduce)");
  if (start < 0) return "";
  let depth = 0;
  for (let i = css.indexOf("{", start); i < css.length; i += 1) {
    if (css[i] === "{") depth += 1;
    if (css[i] === "}") {
      depth -= 1;
      if (depth === 0) return css.slice(start, i + 1);
    }
  }
  return "";
}

beforeEach(() => {
  vi.useFakeTimers();
  hooks.mount();
  dockStore.dispatch({ type: "reset" });
  announceSpy.mockClear();
});

afterEach(() => {
  hooks.mount();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
  dockStore.dispatch({ type: "reset" });
});

describe("Tool browser catalog", () => {
  it("keeps every registered tool available exactly once with its full name", () => {
    const html = renderToStaticMarkup(createElement(ToolBrowser, props()));
    const ids = [...html.matchAll(/data-browser-tool="([^"]+)"/g)].map((match) => match[1]);
    expect(ids.sort()).toEqual([...DOCK_TOOL_IDS].sort());
    for (const tool of DOCK_TOOL_IDS) expect(html).toContain(DOCK_TOOL_META[tool].label);
    expect(html).toContain("Search tools");
  });

  it("lists each category's tools as list items instead of articles", () => {
    const tree = render(props());
    const lists = tree.filter((node) => node.type === "ul");
    expect(lists).toHaveLength(5);
    for (const list of lists) {
      expect(list.props.role).toBe("list");
      const cards = elementsOf(list.props.children).filter((node) => node.props["data-browser-tool"] !== undefined);
      expect(cards.length).toBeGreaterThan(0);
      expect(cards.every((node) => node.type === "li")).toBe(true);
    }
    expect(tree.filter((node) => node.type === "li")).toHaveLength(DOCK_TOOL_IDS.length);
    expect(tree.some((node) => node.type === "article")).toBe(false);
  });

  it("groups tools under the Inspect, Measure, Enhance, Combine and Files and output headings without an eyebrow line", () => {
    const tree = render(props());
    expect(tree.filter((node) => node.type === "section").map((node) => node.props["aria-label"])).toEqual(["Inspect", "Measure", "Enhance", "Combine", "Files and output"]);
    expect(renderToStaticMarkup(createElement(ToolBrowser, props()))).not.toContain("YOUR WORKSPACE");
  });

  it.each([
    ["plate solve", ["astrometry"]],
    ["PLATE SOLVE", ["astrometry"]],
    ["light curve", ["photometry"]],
    ["histogram", ["image"]],
    ["measure", ["astrometry", "photometry", "cube", "sagui", "log"]],
    ["files and output", ["files", "export", "config"]],
  ])("finds %j by name, description, keyword or category", (query, expected) => {
    expect(listed(search(props(), query))).toEqual(expected);
  });

  it("treats a blank query as no filter while still offering to clear it", () => {
    const tree = search(props(), "   ");
    expect(listed(tree)).toHaveLength(DOCK_TOOL_IDS.length);
    expect(tree.some((node) => node.props["aria-label"] === "Clear tool search")).toBe(true);
  });

  it("reports how many tools match in the status line", () => {
    const status = (tree: Rendered[]) => textOf(byProp(tree, "role", "status").props.children);
    expect(status(render(props()))).toBe(`${DOCK_TOOL_IDS.length} tools available`);
    expect(status(search(props(), "light curve"))).toBe("1 tool found");
    expect(status(search(props(), "process"))).toBe("3 tools found");
    expect(status(search(props(), "unmatched query"))).toBe("0 tools found");
  });

  it("offers a way to clear a search with no results", () => {
    const browser = props();
    const empty = search(browser, "unmatched query");
    expect(listed(empty)).toHaveLength(0);
    click(byProp(empty, "aria-label", "Clear tool search"));
    expect(listed(render(browser))).toHaveLength(DOCK_TOOL_IDS.length);
  });
});

describe("Tool browser open state", () => {
  it("marks a tool open only while its panel is shown, not while it waits for a file", () => {
    const layout = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "open", tool: "image" });
    const noFile = render(props({ layout, hasFile: false }));
    for (const tool of ["image", "compose"] as const) {
      expect(card(noFile, tool).props["data-open"]).toBeUndefined();
      expect(inCard(noFile, tool, "ab-tool-browser-open-state")).toBeUndefined();
    }
    hooks.mount();
    const withFile = render(props({ layout, hasFile: true }));
    for (const tool of ["image", "compose", "files"] as const) {
      expect(card(withFile, tool).props["data-open"]).toBe(true);
      expect(inCard(withFile, tool, "ab-tool-browser-open-state")).toBeDefined();
    }
    expect(card(withFile, "astrometry").props["data-open"]).toBeUndefined();
  });

  it("describes every Open button with the tool's description", () => {
    const tree = render(props());
    const ids = DOCK_TOOL_IDS.map((tool) => openButton(tree, tool).props["aria-describedby"]);
    expect(new Set(ids).size).toBe(DOCK_TOOL_IDS.length);
    for (const tool of DOCK_TOOL_IDS) {
      expect(openButton(tree, tool).props["aria-label"]).toBe(`Open ${DOCK_TOOL_META[tool].label}`);
      expect(described(tree, tool)).toContain(TOOL_DESCRIPTIONS[tool]);
    }
  });

  it("tells assistive technology which tools are open through the description instead of the badge", () => {
    const tree = render(props());
    expect(inCard(tree, "files", "ab-tool-browser-open-state")!.props["aria-hidden"]).toBe("true");
    expect(described(tree, "files")).toContain("Currently open.");
    expect(described(tree, "compose")).toContain("Currently open.");
    expect(described(tree, "image")).not.toContain("Currently open.");
  });

  it("blocks Open without a file only for the tools that need one, with a visible reason", () => {
    const noFile = render(props({ hasFile: false }));
    expect(DOCK_TOOL_IDS.filter((tool) => openButton(noFile, tool).props["aria-disabled"] === true)).toEqual(NEEDS_FILE);
    for (const tool of DOCK_TOOL_IDS) {
      expect(openButton(noFile, tool).props.disabled).toBeUndefined();
      expect(described(noFile, tool).includes("Open a file first")).toBe(NEEDS_FILE.includes(tool));
      expect(toggle(noFile, tool).props.disabled).toBeUndefined();
    }
    hooks.mount();
    const withFile = render(props({ hasFile: true }));
    expect(DOCK_TOOL_IDS.filter((tool) => openButton(withFile, tool).props["aria-disabled"] !== undefined)).toEqual([]);
    expect(DOCK_TOOL_IDS.filter((tool) => described(withFile, tool).includes("Open a file first"))).toEqual([]);
  });

  it("ignores Open on a tool that needs a file: nothing opens, the dialog stays and nothing is announced", () => {
    const browser = props({ hasFile: false });
    click(openButton(render(browser), "image"));
    vi.runAllTimers();
    expect(dockStore.get()).toBe(DEFAULT_DOCK_LAYOUT);
    expect(browser.onClose).not.toHaveBeenCalled();
    expect(announceSpy).not.toHaveBeenCalled();
  });

  it("opens a tool at its saved dock location and keeps an already open tool open", () => {
    const stripToggled = vi.spyOn(dockOnboardingStore, "stripToggled");
    const browser = props();
    const tree = render(browser);
    click(openButton(tree, "image"));
    expect(dockStore.get().active["right-top"]).toBe("image");
    click(openButton(tree, "files"));
    expect(dockStore.get().active["left-top"]).toBe("files");
    expect(stripToggled.mock.calls).toEqual([["image"], ["files"]]);
    expect(browser.onClose).toHaveBeenCalledTimes(2);
  });

  it("announces where a tool opened once the modal is gone", () => {
    click(openButton(render(props()), "image"));
    vi.advanceTimersByTime(149);
    expect(announceSpy).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(announceSpy.mock.calls).toEqual([["Image opened in Right Top"]]);
  });

  it("says a tool is already open instead of claiming it opened again", () => {
    click(openButton(render(props()), "files"));
    vi.advanceTimersByTime(150);
    expect(announceSpy.mock.calls).toEqual([["Files is already open"]]);
  });
});

describe("Tool browser sidebar toggle", () => {
  it("shows pinned tools as in the sidebar and leaves an open unpinned tool out", () => {
    const tree = render(props());
    expect(DOCK_TOOL_IDS.filter((tool) => card(tree, tool).props["data-selected"] === true)).toEqual(["image", "processing", "export"]);
    expect(card(tree, "files").props["data-open"]).toBe(true);
    expect(card(tree, "files").props["data-selected"]).toBe(false);
    expect(textOf(inCard(tree, "files", "ab-tool-browser-sidebar-state")!.props.children)).toBe("Not in sidebar");
    expect(textOf(inCard(tree, "image", "ab-tool-browser-sidebar-state")!.props.children)).toBe("In sidebar");
  });

  it("shows the default sidebar tools as in the sidebar", () => {
    const tree = render(props({ enabledTools: DEFAULT_TOOL_SHORTCUTS }));
    expect(DOCK_TOOL_IDS.filter((tool) => card(tree, tool).props["data-selected"] === true)).toEqual([
      "files", "info", "compose", "headers", "image", "astrometry", "photometry", "cube", "processing", "history", "export", "config", "log",
    ]);
  });

  it("names the toggle with its visible action first and the tool after it, without a pressed state", () => {
    const tree = render(props());
    expect(toggle(tree, "image").props["aria-label"]).toBe("Remove from sidebar: Image");
    expect(toggle(tree, "files").props["aria-label"]).toBe("Add to sidebar: Files");
    for (const tool of DOCK_TOOL_IDS) {
      const button = toggle(tree, tool);
      const visible = textOf(button.props.children);
      expect(visible).toBe(SIDEBAR.includes(tool) ? "Remove from sidebar" : "Add to sidebar");
      expect(button.props["aria-label"]).toBe(`${visible}: ${DOCK_TOOL_META[tool].label}`);
      expect(button.props).not.toHaveProperty("aria-pressed");
    }
  });

  it("changes only the pin: no panel opens or closes and the dialog stays", () => {
    const browser = props({ hasFile: false });
    const tree = render(browser);
    const added = toggle(tree, "astrometry");
    expect(added.type).toBe("button");
    click(added);
    click(toggle(tree, "files"));
    click(toggle(tree, "image"));
    expect(browser.onToggleTool.mock.calls).toEqual([["astrometry"], ["files"], ["image"]]);
    expect(dockStore.get()).toBe(DEFAULT_DOCK_LAYOUT);
    expect(browser.onClose).not.toHaveBeenCalled();
  });
});

describe("Tool browser modal", () => {
  it("closes once on Escape through the cancel event and stops the native close", () => {
    const browser = props();
    const event = { preventDefault: vi.fn() };
    (dialogOf(render(browser)).props.onCancel as (event: unknown) => void)(event);
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(browser.onClose).toHaveBeenCalledOnce();
  });

  it("closes once from the Close button and once from an Open action", () => {
    const closeButton = props();
    click(byProp(render(closeButton), "aria-label", "Close tool browser"));
    expect(closeButton.onClose).toHaveBeenCalledOnce();
    hooks.mount();
    const openAction = props();
    click(openButton(render(openAction), "astrometry"));
    expect(openAction.onClose).toHaveBeenCalledOnce();
  });

  it("closes on a backdrop click only when the press also started on the backdrop", () => {
    const browser = props();
    const dialog = dialogOf(render(browser));
    const node = {};
    const inner = {};
    const press = (target: object) => (dialog.props.onPointerDown as (event: unknown) => void)({ target, currentTarget: node });
    const release = (target: object) => (dialog.props.onClick as (event: unknown) => void)({ target, currentTarget: node });
    press(inner);
    release(node);
    expect(browser.onClose).not.toHaveBeenCalled();
    press(node);
    release(inner);
    release(node);
    expect(browser.onClose).not.toHaveBeenCalled();
    press(node);
    release(node);
    expect(browser.onClose).toHaveBeenCalledOnce();
  });

  it("keeps key presses and releases inside the modal so global shortcuts stay inactive", () => {
    const dialog = dialogOf(render(props()));
    for (const handler of ["onKeyDown", "onKeyUp"]) {
      const event = { key: "Shift", stopPropagation: vi.fn(), preventDefault: vi.fn() };
      (dialog.props[handler] as (event: unknown) => void)(event);
      expect(event.stopPropagation).toHaveBeenCalledOnce();
      expect(event.preventDefault).not.toHaveBeenCalled();
    }
  });

  it("opens the native modal before focusing the search field and returns focus to a connected opener", () => {
    vi.stubGlobal("document", { body: {} });
    const calls: string[] = [];
    const dialog = { showModal: () => calls.push("showModal"), close: () => calls.push("close") };
    const input = { focus: () => calls.push("focus search") };
    const opener = { isConnected: true, getClientRects: () => [{}], focus: vi.fn(() => calls.push("focus opener")) };
    const railTrigger = vi.fn(() => ({ focus: vi.fn() }));
    const cleanup = openToolBrowserDialog(dialog, input, opener, railTrigger);
    expect(calls).toEqual(["showModal", "focus search"]);
    cleanup();
    expect(calls).toEqual(["showModal", "focus search", "close", "focus opener"]);
    expect(opener.focus).toHaveBeenCalledWith({ preventScroll: true });
    expect(railTrigger).not.toHaveBeenCalled();
  });

  it("returns focus to the right rail's All tools trigger after closing when the opener left the document", () => {
    vi.stubGlobal("document", { body: {} });
    const calls: string[] = [];
    const dialog = { showModal: vi.fn(), close: () => calls.push("close") };
    const opener = { isConnected: false, getClientRects: () => [], focus: vi.fn() };
    const railTrigger = { focus: vi.fn(() => calls.push("focus rail trigger")) };
    openToolBrowserDialog(dialog, null, opener, () => railTrigger)();
    expect(calls).toEqual(["close", "focus rail trigger"]);
    expect(opener.focus).not.toHaveBeenCalled();
    expect(railTrigger.focus).toHaveBeenCalledWith({ preventScroll: true });
  });

  it("returns focus to the right rail's All tools trigger when the opener is still in the document but hidden by the time the dialog closes", () => {
    vi.stubGlobal("document", { body: {} });
    let rects: object[] = [{}];
    const opener = { isConnected: true, getClientRects: () => rects, focus: vi.fn() };
    const railTrigger = { focus: vi.fn() };
    const close = openToolBrowserDialog({ showModal: vi.fn(), close: vi.fn() }, null, opener, () => railTrigger);
    rects = [];
    close();
    expect(opener.focus).not.toHaveBeenCalled();
    expect(railTrigger.focus).toHaveBeenCalledWith({ preventScroll: true });
  });

  it("returns focus to the right rail's All tools trigger when only the page body had focus as the dialog opened", () => {
    const body = { isConnected: true, getClientRects: () => [{}], focus: vi.fn() };
    vi.stubGlobal("document", { body });
    const railTrigger = { focus: vi.fn() };
    openToolBrowserDialog({ showModal: vi.fn(), close: vi.fn() }, null, body, () => railTrigger)();
    expect(body.focus).not.toHaveBeenCalled();
    expect(railTrigger.focus).toHaveBeenCalledWith({ preventScroll: true });
  });
});

describe("Rail and dialog stylesheets", () => {
  const dockCss = readCss("dock.css");
  const browserCss = readCss("tool-browser.css");
  const RAIL = ".ab-dock-strip[data-workspace-rail]";
  const BUTTON = `${RAIL} .ab-dock-strip-btn`;

  it("marks the open rail tool with an inset bar on its panel side and a 50% border, so the cue does not rely on hue", () => {
    expect(cssRule(dockCss, `${RAIL}[data-side="left"] .ab-dock-strip-btn-active`).get("box-shadow")).toMatch(/^inset -2px 0 0 var\(--strip-accent\b/);
    expect(cssRule(dockCss, `${RAIL}[data-side="right"] .ab-dock-strip-btn-active`).get("box-shadow")).toMatch(/^inset 2px 0 0 var\(--strip-accent\b/);
    const border = cssRule(dockCss, `${BUTTON}.ab-dock-strip-btn-active`).get("border-color");
    expect(border).toMatch(/^color-mix\(in srgb, var\(--strip-accent\b/);
    expect(border).toMatch(/ 50%, transparent\)$/);
  });

  it("dims a disabled rail tool to at least 0.55 opacity and drops its open cue", () => {
    const disabled = cssRule(dockCss, `${BUTTON}[aria-disabled="true"]`);
    expect(Number(disabled.get("opacity"))).toBeGreaterThanOrEqual(0.55);
    expect(disabled.get("box-shadow")).toBe("none");
  });

  it("keeps rail icons neutral at rest and gives them the tool hue on hover, keyboard focus and when open", () => {
    expect(cssRule(dockCss, `${BUTTON} svg`).get("color")).toBe("var(--ab-text-2)");
    for (const state of [`${BUTTON}:hover svg`, `${BUTTON}:focus-visible svg`, `${RAIL} .ab-dock-strip-btn-active svg`]) {
      expect(cssRule(dockCss, state).get("color")).toMatch(/^var\(--strip-accent\b/);
    }
  });

  it("sizes the rail at 48px with 44px buttons and 9px captions", () => {
    expect(cssRule(dockCss, RAIL).get("width")).toBe("48px");
    const button = cssRule(dockCss, BUTTON);
    expect(button.get("width")).toBe("44px");
    expect(button.get("font-size")).toBe("9px");
  });

  it("outlines the All tools trigger without the fill that made it look pressed", () => {
    const trigger = cssRule(dockCss, `${RAIL} .ab-tool-browser-trigger`);
    expect([...trigger.values()].join(" ")).not.toContain("--ab-accent-bg");
    expect(trigger.get("border-color")).toBe("var(--ab-accent-border)");
  });

  it("hides tools that are not rail shortcuts from the rail stylesheet", () => {
    expect(cssRule(dockCss, '.ab-dock-strip[data-compact="true"] .ab-dock-strip-btn[data-shortcut-visible="false"]').get("display")).toBe("none");
  });

  it("keeps only dialog rules in tool-browser.css", () => {
    expect(browserCss).not.toContain(".ab-dock-strip");
    expect(browserCss).not.toContain("prefers-reduced-motion");
    expect(browserCss).not.toContain("aria-pressed");
    expect(browserCss).not.toContain("ab-tool-browser-eyebrow");
    expect(cssRule(browserCss, '.ab-tool-browser-card[data-selected="false"] .ab-tool-browser-toggle').get("color")).toBe("var(--tool-category-color)");
  });

  it("dims only the icon and title of a card that needs a file, so its description stays readable", () => {
    expect(browserCss).not.toContain(":disabled");
    expect(cssRule(browserCss, '.ab-tool-browser-open[aria-disabled="true"]').has("opacity")).toBe(false);
    for (const part of [".ab-tool-browser-icon", ".ab-tool-browser-card-heading"]) {
      expect(Number(cssRule(browserCss, `.ab-tool-browser-open[aria-disabled="true"] ${part}`).get("opacity"))).toBeLessThan(1);
    }
  });

  it("leaves reduced motion to the global rule and keeps only the static onboarding arrow swap", () => {
    const block = reducedMotionBlock(dockCss);
    expect(block).not.toMatch(/animation\s*:/);
    expect(cssRule(block, ".ab-dock-onboarding-static-arrow").get("display")).toBe("inline");
    expect(cssRule(block, ".ab-dock-onboarding-cursor").get("display")).toBe("none");
  });

  it("draws the onboarding demo rail from tokens with an All tools slot", () => {
    expect(dockCss).not.toMatch(/#52525b|#71717a/i);
    for (const selector of [".ab-dock-onboarding-strip", ".ab-dock-onboarding-slot", ".ab-dock-onboarding-source", ".ab-dock-onboarding-demo-trigger"]) {
      expect([...cssRule(dockCss, selector).values()].join(" ")).toMatch(/var\(--ab-/);
    }
  });
});
