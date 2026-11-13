import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import { createElement, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { LayoutGrid } from "lucide-react";
import Stripe, { type StripeProps } from "../dock/Stripe";
import type { StripButtonDragProps } from "../dock/useDockDrag";
import { focusStripButton, nextRailSession, nextShown, rightRailToolsTrigger, settleBottom, settleSide, startResizeDrag, withActiveTools, type AnchorTools } from "../dock/dockGeometry";
import type { ResizeSession } from "../dock/ResizeHandle";
import { DOCK_TOOLS } from "../dock/toolRegistry";
import { hooks } from "../../test/hookRuntime";
import { dockStore } from "../../hooks/useDockLayout";
import { measurementLog } from "../../utils/measurementLog";
import { stripButtonHint } from "../../utils/dockOnboarding";
import { DEFAULT_TOOL_SHORTCUTS } from "../../utils/toolCatalog";
import {
  DEFAULT_DOCK_LAYOUT,
  DOCK_TOOL_IDS,
  DOCK_TOOL_META,
  dockReducer,
  type DockAnchor,
  type DockLayout,
  type DockSide,
  type DockToolId,
} from "../../utils/dockLayout";

vi.mock("react", async (original) => {
  const { reactWithFakeHooks } = await import("../../test/hookRuntime");
  return reactWithFakeHooks(await original<Record<string, unknown>>());
});

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

interface Rail {
  enabledTools: readonly DockToolId[];
  onBrowseTools?(): void;
}

function stripeProps(side: DockSide, layout: DockLayout = DEFAULT_DOCK_LAYOUT, hasFile = true, rail?: Rail): StripeProps {
  const base: StripeProps = { side, layout, hasFile, buttonProps: () => NO_DRAG, onContextMenu: () => {} };
  return rail ? { ...base, compact: true, enabledTools: rail.enabledTools, onBrowseTools: rail.onBrowseTools } : base;
}

function render(side: DockSide, layout: DockLayout = DEFAULT_DOCK_LAYOUT, hasFile = true, rail?: Rail): string {
  return renderToStaticMarkup(createElement(Stripe, stripeProps(side, layout, hasFile, rail)));
}

const RAIL: Rail = { enabledTools: DEFAULT_TOOL_SHORTCUTS, onBrowseTools() {} };

function memoryStorage(): Pick<Storage, "getItem" | "setItem" | "removeItem"> {
  const map = new Map<string, string>();
  return {
    getItem: (key) => map.get(key) ?? null,
    setItem: (key, value) => void map.set(key, value),
    removeItem: (key) => void map.delete(key),
  };
}

type Node = { type: unknown; props: Record<string, unknown> };

function elements(node: ReactNode): Node[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Record<string, unknown>>(node)) return [];
  return [{ type: node.type, props: node.props }, ...elements(node.props.children as ReactNode)];
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
    let layout = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "image", anchor: "left-bottom", index: 0 });
    layout = dockReducer(layout, { type: "reorder", anchor: "right-top", from: 0, to: 2 });
    expect(order(render("left", layout), "left-bottom")).toEqual(["image", "compose"]);
    expect(order(render("right", layout), "right-top")).toEqual(["astrometry", "photometry", "headers", "cube", "cubeprep", "sagui", "processing", "history", "masks", "stacking"]);
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

  it("presses an open tool that needs a file only while a file is open, the same rule that disables it", () => {
    const opened = dockReducer(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "open", tool: "stacking" }), { type: "open", tool: "config" });
    for (const rail of [undefined, RAIL]) {
      const left = render("left", DEFAULT_DOCK_LAYOUT, false, rail);
      const right = render("right", opened, false, rail);
      expect(pressed(left, "left-top")).toEqual(["files"]);
      expect(pressed(left, "left-bottom")).toEqual([]);
      expect(pressed(right, "right-top")).toEqual([]);
      expect(pressed(right, "right-bottom")).toEqual(["config"]);
      const buttons = [...GROUPS.left.flatMap((anchor) => group(left, anchor)), ...GROUPS.right.flatMap((anchor) => group(right, anchor))];
      expect(buttons.filter((b) => b.class.split(" ").includes("ab-dock-strip-btn-active")).map((b) => b["data-tool-id"])).toEqual(["files", "config"]);
      expect(pressed(render("left", DEFAULT_DOCK_LAYOUT, true, rail), "left-bottom")).toEqual(["compose"]);
      expect(pressed(render("right", opened, true, rail), "right-top")).toEqual(["stacking"]);
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

  it("marks every button with its tool, anchor, label, open or close hint and menu popup", () => {
    for (const side of ["left", "right"] as const) {
      const html = render(side);
      for (const anchor of GROUPS[side]) {
        for (const b of group(html, anchor)) {
          const meta = DOCK_TOOL_META[b["data-tool-id"] as DockToolId];
          expect(b.type).toBe("button");
          expect(b["data-anchor"]).toBe(anchor);
          expect(b["aria-haspopup"]).toBe("menu");
          expect(b.title).toBe(stripButtonHint(meta.label, { open: b["aria-pressed"] === "true", disabled: false }));
          expect(b["aria-label"]).toBe(meta.label);
        }
      }
    }
    expect(render("left")).toContain(">Compose</span>");
    expect(render("right")).toContain(">Settings</span>");
  });

  it("titles each button with what a click does while its name stays the tool label", () => {
    const leftBottom = group(render("left"), "left-bottom");
    expect(leftBottom[0]).toMatchObject({ "data-tool-id": "compose", "aria-label": "Compose", title: "Compose is open: click to close" });
    expect(group(render("left"), "left-top")[0]).toMatchObject({ "data-tool-id": "files", title: "Files is open: click to close" });
    const right = Object.fromEntries(group(render("right"), "right-bottom").map((b) => [b["data-tool-id"], b]));
    expect(right.config).toMatchObject({ "aria-label": "Settings", title: "Settings: click to open, click again to close" });
    expect(right.export.title).toBe("Export: click to open, click again to close");
    const opened = Object.fromEntries(group(render("right", dockReducer(DEFAULT_DOCK_LAYOUT, { type: "open", tool: "config" })), "right-bottom").map((b) => [b["data-tool-id"], b]));
    expect(opened.config.title).toBe("Settings is open: click to close");
  });

  it("asks for a file first on the tools that need one and keeps Settings usable without a file", () => {
    const compose = group(render("left", DEFAULT_DOCK_LAYOUT, false), "left-bottom")[0];
    expect(compose).toMatchObject({ "data-tool-id": "compose", "aria-disabled": "true", "aria-label": "Compose", title: "Compose: open a file first" });
    const right = Object.fromEntries(group(render("right", DEFAULT_DOCK_LAYOUT, false), "right-bottom").map((b) => [b["data-tool-id"], b]));
    expect(right.config["aria-disabled"]).toBeUndefined();
    expect(right.config.title).toBe("Settings: click to open, click again to close");
    expect(right.export).toMatchObject({ "aria-disabled": "true", title: "Export: open a file first" });
    expect(right.log.title).toBe("Log: click to open, click again to close");
  });

  it("renders both group wrappers even when a group is empty", () => {
    const layout = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "compose", anchor: "left-top" });
    const html = render("left", layout);
    expect(order(html, "left-bottom")).toEqual([]);
    expect(order(html, "left-top")).toEqual(["files", "info", "compose"]);
    expect(pressed(html, "left-top")).toEqual(["compose"]);
  });

  it("shows the five analysis tools on the right strip in the default order", () => {
    const html = render("right");
    expect(order(html, "right-top")).toEqual(["headers", "image", "astrometry", "photometry", "cube", "cubeprep", "sagui", "processing", "history", "masks", "stacking"]);
    expect(order(html, "right-bottom")).toEqual(["synth", "export", "config", "log"]);
    for (const id of ["image", "astrometry", "photometry", "cube", "log"]) expect(html).toContain(`data-tool-id="${id}"`);
    expect(html).not.toContain('data-tool-id="analysis"');
  });

  it("labels each analysis button with its full name, not the strip abbreviation", () => {
    const buttons = [...group(render("right"), "right-top"), ...group(render("right"), "right-bottom")];
    const byId = Object.fromEntries(buttons.map((b) => [b["data-tool-id"], b["aria-label"]]));
    expect(byId.astrometry).toBe("Astrometry");
    expect(byId.photometry).toBe("Photometry");
    expect(byId.image).toBe("Image");
    expect(byId.cube).toBe("Spectra");
  });

  it("shows the Masks button right after History, labelled Masks and off without a file", () => {
    const withFile = group(render("right"), "right-top");
    const ids = withFile.map((b) => b["data-tool-id"]);
    expect(ids.indexOf("masks")).toBe(ids.indexOf("history") + 1);
    const masks = withFile.find((b) => b["data-tool-id"] === "masks");
    expect(masks?.["aria-label"]).toBe("Masks");
    expect(masks?.["aria-disabled"]).toBeUndefined();
    const noFile = group(render("right", DEFAULT_DOCK_LAYOUT, false), "right-top").find((b) => b["data-tool-id"] === "masks");
    expect(noFile?.["aria-disabled"]).toBe("true");
  });

  it("keeps Log enabled without a file while the other analysis tools are disabled", () => {
    const html = render("right", DEFAULT_DOCK_LAYOUT, false);
    const byId = Object.fromEntries([...group(html, "right-top"), ...group(html, "right-bottom")].map((b) => [b["data-tool-id"], b]));
    expect(byId.log["aria-disabled"]).toBeUndefined();
    for (const id of ["image", "astrometry", "photometry", "cube"]) expect(byId[id]["aria-disabled"]).toBe("true");
  });
});

describe("compact sidebar rail", () => {
  beforeEach(() => {
    vi.stubGlobal("localStorage", memoryStorage());
    hooks.mount();
  });

  afterEach(() => {
    hooks.mount();
    dockStore.dispatch({ type: "reset" });
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  function railButton(layout: DockLayout, tool: DockToolId): Node {
    const tree = elements(hooks.render(Stripe, stripeProps("right", layout, true, RAIL)) as ReactNode);
    return tree.find((node) => node.props["data-tool-id"] === tool)!;
  }

  it("shows pinned and open tools and hides closed unpinned ones", () => {
    const layout = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "open", tool: "image" });
    const rail: Rail = { enabledTools: ["files", "export"], onBrowseTools() {} };
    const left = render("left", layout, true, rail);
    const right = render("right", layout, true, rail);
    expect(right).toMatch(/^<div[^>]*data-compact="true"/);
    const buttons = [...group(left, "left-top"), ...group(left, "left-bottom"), ...group(right, "right-top"), ...group(right, "right-bottom")];
    const visible = Object.fromEntries(buttons.map((b) => [b["data-tool-id"], b["data-shortcut-visible"]]));
    expect(visible).toMatchObject({ files: "true", export: "true", image: "true", compose: "true", info: "false", headers: "false", log: "false" });
  });

  it("keeps a closed unpinned tool on the rail while its button holds focus", () => {
    dockStore.dispatch({ type: "open", tool: "sagui" });
    const opened = railButton(dockStore.get(), "sagui");
    expect(opened.props["data-shortcut-visible"]).toBe("true");
    (opened.props.onFocus as (() => void) | undefined)?.();
    (opened.props.onClick as () => void)();
    expect(dockStore.get().active["right-top"]).toBeNull();
    const focused = railButton(dockStore.get(), "sagui");
    expect(focused.props["data-shortcut-visible"]).toBe("true");
    (focused.props.onBlur as (() => void) | undefined)?.();
    expect(railButton(dockStore.get(), "sagui").props["data-shortcut-visible"]).toBe("false");
  });

  it("puts the All tools trigger on the right rail, and on the left rail only while that rail shows no tool", () => {
    const trigger = 'aria-label="Browse all tools"';
    expect(render("right", DEFAULT_DOCK_LAYOUT, true, RAIL)).toContain(trigger);
    expect(render("right", DEFAULT_DOCK_LAYOUT, true, RAIL)).toContain('data-tools-trigger=""');
    expect(render("left", DEFAULT_DOCK_LAYOUT, true, RAIL)).not.toContain(trigger);
    const closed = dockReducer(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "closeAnchor", anchor: "left-top" }), { type: "closeAnchor", anchor: "left-bottom" });
    const bare: Rail = { enabledTools: [], onBrowseTools() {} };
    expect(render("left", closed, true, bare)).toContain(trigger);
    expect(render("left", DEFAULT_DOCK_LAYOUT, true, bare)).not.toContain(trigger);
    expect(render("left", closed, true, { enabledTools: [] })).not.toContain(trigger);
    expect(render("right", closed, true, { enabledTools: [] })).not.toContain(trigger);
  });

  it("finds the All tools trigger by the attributes the right rail renders, not the left one", () => {
    const matches = (props: Record<string, unknown>, selector: string) => {
      const [, name, value] = /^\[([\w-]+)(?:="([^"]*)")?\]$/.exec(selector) ?? [];
      return name !== undefined && (value === undefined ? name in props : props[name] === value);
    };
    const trigger = {};
    const asked: string[] = [];
    const root = {
      querySelector: (selector: string) => {
        asked.push(selector);
        return trigger;
      },
    };
    expect(rightRailToolsTrigger(root as unknown as ParentNode)).toBe(trigger);
    const [rail, button] = asked[0].split(" ");
    const right = elements(hooks.render(Stripe, stripeProps("right", DEFAULT_DOCK_LAYOUT, true, RAIL)) as ReactNode);
    expect(matches(right[0].props, rail)).toBe(true);
    expect(right.filter((node) => matches(node.props, button)).map((node) => node.props["aria-label"])).toEqual(["Browse all tools"]);
    const closed = dockReducer(dockReducer(DEFAULT_DOCK_LAYOUT, { type: "closeAnchor", anchor: "left-top" }), { type: "closeAnchor", anchor: "left-bottom" });
    hooks.mount();
    const left = elements(hooks.render(Stripe, stripeProps("left", closed, true, { enabledTools: [], onBrowseTools() {} })) as ReactNode);
    expect(left.some((node) => matches(node.props, button))).toBe(true);
    expect(matches(left[0].props, rail)).toBe(false);
  });

  it("keeps the native menu off the rail itself", () => {
    const root = elements(hooks.render(Stripe, stripeProps("right", DEFAULT_DOCK_LAYOUT, true, RAIL)) as ReactNode)[0];
    expect(root.props["data-dock-strip"]).toBe("right");
    const preventDefault = vi.fn();
    (root.props.onContextMenu as ((e: { preventDefault(): void }) => void) | undefined)?.({ preventDefault });
    expect(preventDefault).toHaveBeenCalledTimes(1);
  });

  it("captions every rail button with its short label, compact or not", () => {
    for (const rail of [RAIL, undefined]) {
      const html = render("left", DEFAULT_DOCK_LAYOUT, true, rail) + render("right", DEFAULT_DOCK_LAYOUT, true, rail);
      const captions = Object.fromEntries([...html.matchAll(/<button[^>]*data-tool-id="([\w-]+)"[^>]*>.*?<span>([^<]*)<\/span>/g)].map((m) => [m[1], m[2]]));
      expect(DOCK_TOOL_IDS.map((tool) => captions[tool])).toEqual(["Files", "Info", "Compose", "Headers", "Image", "Astro", "Phot", "Spectra", "Prep", "SAGUI", "Process", "History", "Masks", "Stack", "Synth", "Export", "Settings", "Log"]);
      for (const tool of DOCK_TOOL_IDS) expect(captions[tool]).toBe(DOCK_TOOL_META[tool].shortLabel);
    }
  });

  it("gives every tool its own icon, apart from the All tools trigger", () => {
    const icons = DOCK_TOOL_IDS.map((tool) => DOCK_TOOLS[tool].icon);
    expect(new Set(icons).size).toBe(DOCK_TOOL_IDS.length);
    expect(icons).not.toContain(LayoutGrid);
  });
});

function addEntry(i: number) {
  measurementLog.append({ kind: "pixel", file: "a.fits", image: "original", dq: "off", unit: null, source: "test", params: {}, values: { i }, notes: [] });
}

describe("Log strip count", () => {
  afterEach(() => {
    measurementLog.clear();
  });

  function logButton(html: string): { attrs: Record<string, string>; inner: string } {
    const match = /(<button[^>]*data-tool-id="log"[^>]*>)(.*?)<\/button>/.exec(html);
    if (!match) throw new Error("log button not rendered");
    return { attrs: attrs(match[1]), inner: match[2] };
  }

  it("shows no badge and the plain label while the log is empty", () => {
    measurementLog.clear();
    const { attrs: a, inner } = logButton(render("right"));
    expect(a.title).toBe("Log: click to open, click again to close");
    expect(a["aria-label"]).toBe("Log");
    expect(inner).not.toMatch(/data-log-count/);
  });

  it("names the entry count in the title and label and shows it as a hidden badge", () => {
    measurementLog.clear();
    for (let i = 0; i < 3; i++) addEntry(i);
    const { attrs: a, inner } = logButton(render("right"));
    expect(a.title).toBe("Log (3 entries): click to open, click again to close");
    expect(a["aria-label"]).toBe("Log (3 entries)");
    expect(inner).toMatch(/<span[^>]*data-log-count=""[^>]*aria-hidden="true"[^>]*>3<\/span>|<span[^>]*aria-hidden="true"[^>]*data-log-count=""[^>]*>3<\/span>/);
    expect(inner).toContain(">Log</span>");
  });

  it("uses the singular for one entry", () => {
    measurementLog.clear();
    addEntry(0);
    const { attrs: a } = logButton(render("right"));
    expect(a.title).toBe("Log (1 entry): click to open, click again to close");
    expect(a["aria-label"]).toBe("Log (1 entry)");
  });

  it("shows the Log on the default compact rail with its entry count", () => {
    measurementLog.clear();
    addEntry(0);
    addEntry(1);
    const { attrs: a, inner } = logButton(render("right", DEFAULT_DOCK_LAYOUT, true, RAIL));
    expect(a["data-shortcut-visible"]).toBe("true");
    expect(inner).toMatch(/data-log-count=""[^>]*>2<\/span>/);
  });

  it("caps the badge at 99+", () => {
    measurementLog.clear();
    for (let i = 0; i < 120; i++) addEntry(i);
    const { attrs: a, inner } = logButton(render("right"));
    expect(a["aria-label"]).toBe("Log (120 entries)");
    expect(inner).toMatch(/>99\+<\/span>/);
  });

  it("leaves every other tool's button untouched", () => {
    measurementLog.clear();
    for (let i = 0; i < 3; i++) addEntry(i);
    const html = render("right");
    for (const b of [...group(html, "right-top"), ...group(html, "right-bottom")]) {
      if (b["data-tool-id"] === "log") continue;
      const meta = DOCK_TOOL_META[b["data-tool-id"] as DockToolId];
      expect(b.title).toBe(stripButtonHint(meta.label, { open: false, disabled: false }));
      expect(b["aria-label"]).toBe(meta.label);
    }
    expect(html.match(/data-log-count/g)).toHaveLength(1);
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
    const shown = nextShown(tools({ "left-top": "files", "right-top": "image" }), tools({ "left-top": "files" }));
    expect(shown).toEqual(tools({ "left-top": "files", "right-top": "image" }));
    const open = tools({ "left-top": "files" });
    expect(settleSide(shown, open, "left-top")).toBe(shown);
    expect(settleBottom(shown, open)).toBe(shown);
    expect(settleSide(shown, open, "right-top")).toEqual(tools({ "left-top": "files" }));
  });

  it("follows a reopen before the close settles and ignores the late settle", () => {
    const closing = nextShown(tools({ "right-top": "image" }), NONE);
    const reopened = nextShown(closing, tools({ "right-top": "image" }));
    expect(reopened).toBe(closing);
    expect(settleSide(reopened, tools({ "right-top": "image" }), "right-top")).toBe(reopened);
    expect(nextShown(closing, tools({ "right-top": "headers" }))).toEqual(tools({ "right-top": "headers" }));
  });

  it("shows a tool moved away from a closing anchor at its new anchor at once and clears the old one on its own settle", () => {
    const closing = nextShown(tools({ "right-top": "image" }), NONE);
    const open = tools({ "left-bottom": "image" });
    const moved = nextShown(closing, open);
    expect(moved).toEqual(tools({ "right-top": "image", "left-bottom": "image" }));
    expect(settleBottom(moved, open)).toBe(moved);
    expect(settleSide(moved, open, "right-top")).toEqual(open);
  });

  it("returns the same record from a settle that has nothing to clear", () => {
    expect(settleSide(NONE, NONE, "left-top")).toBe(NONE);
    expect(settleBottom(NONE, NONE)).toBe(NONE);
  });
});

describe("session tools", () => {
  it("adds every open tool and keeps the same set while nothing new opens", () => {
    const seen = withActiveTools(new Set(), tools({ "left-top": "files", "right-top": "sagui" }));
    expect([...seen].sort()).toEqual(["files", "sagui"]);
    expect(withActiveTools(seen, tools({ "right-top": "sagui" }))).toBe(seen);
    expect(withActiveTools(seen, NONE)).toBe(seen);
    expect([...withActiveTools(seen, tools({ "right-bottom": "log" }))].sort()).toEqual(["files", "log", "sagui"]);
  });

  it("keeps the same session until a tool opens, and starts over from the open tools after a shortcut reset", () => {
    const files = tools({ "left-top": "files" });
    const start = nextRailSession(null, 0, files);
    expect([...start.tools]).toEqual(["files"]);
    expect(nextRailSession(start, 0, files)).toBe(start);
    const opened = nextRailSession(start, 0, tools({ "left-top": "files", "right-top": "sagui" }));
    expect([...opened.tools].sort()).toEqual(["files", "sagui"]);
    expect(nextRailSession(opened, 0, files)).toBe(opened);
    const defaults = tools({ "left-top": "files", "left-bottom": "compose" });
    const reset = nextRailSession(opened, 1, defaults);
    expect([...reset.tools].sort()).toEqual(["compose", "files"]);
    expect(nextRailSession(reset, 1, defaults)).toBe(reset);
    const empty = nextRailSession(null, 2, NONE);
    expect(empty.tools.size).toBe(0);
    expect(nextRailSession(empty, 2, NONE)).toBe(empty);
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

describe("focusStripButton", () => {
  function fakeButton(inStrip: boolean) {
    const calls: string[] = [];
    const selectors: string[] = [];
    return {
      calls,
      selectors,
      button: {
        focus(options?: FocusOptions) {
          calls.push(`focus preventScroll=${options?.preventScroll}`);
        },
        closest(selector: string) {
          selectors.push(selector);
          return inStrip ? {} : null;
        },
        scrollIntoView(options?: ScrollIntoViewOptions) {
          calls.push(`scroll block=${options?.block}`);
        },
      },
    };
  }

  it("focuses without scrolling, then brings a strip button into view", () => {
    const fake = fakeButton(true);
    focusStripButton(fake.button);
    expect(fake.calls).toEqual(["focus preventScroll=true", "scroll block=nearest"]);
    expect(fake.selectors).toEqual([".ab-dock-strip"]);
  });

  it("only focuses a button outside the strip", () => {
    const fake = fakeButton(false);
    focusStripButton(fake.button);
    expect(fake.calls).toEqual(["focus preventScroll=true"]);
  });

  it("ignores a missing button", () => {
    expect(() => focusStripButton(null)).not.toThrow();
    expect(() => focusStripButton(undefined)).not.toThrow();
  });
});
