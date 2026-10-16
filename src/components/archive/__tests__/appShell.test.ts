import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { AudioWaveform, ChevronLeft, CloudOff, ExternalLink, KeyRound, LayoutGrid, Lock, LockOpen, Table } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import ModeSwitch from "../../ModeSwitch";
import AppFooter from "../../AppFooter";
import EmptyState from "../../EmptyState";
import { APP_VERSION } from "../../../utils/constants";
import type { AppMode } from "../../../hooks/useAppMode";

type AnyElement = ReactElement<Record<string, unknown> & { children?: ReactNode }>;

function elements(node: ReactNode): AnyElement[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement(node)) return [];
  const el = node as AnyElement;
  return [el, ...elements(el.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (!isValidElement(node)) return "";
  return textOf((node as AnyElement).props.children);
}

function buttons(node: ReactNode): AnyElement[] {
  return elements(node).filter((el) => el.type === "button");
}

function clickEvent() {
  return { preventDefault: vi.fn(), stopPropagation: vi.fn() };
}

function buttonTag(html: string, mode: AppMode): string {
  return new RegExp(`<button[^>]*data-mode="${mode}"[^>]*>`).exec(html)?.[0] ?? "";
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("ModeSwitch", () => {
  it("renders a Workspace and an Archive segment and marks the active one", () => {
    const html = renderToStaticMarkup(createElement(ModeSwitch, { mode: "workspace", onChange: () => {} }));
    expect(html).toContain('data-testid="mode-switch"');
    expect(html).toContain('role="group"');
    expect(html).toContain('aria-label="Mode"');
    expect(html.indexOf('data-mode="workspace"')).toBeLessThan(html.indexOf('data-mode="archive"'));

    const workspace = buttonTag(html, "workspace");
    const archive = buttonTag(html, "archive");
    expect(workspace).toContain('class="ab-pill"');
    expect(workspace).toContain('type="button"');
    expect(workspace).toContain('data-active="true"');
    expect(workspace).toContain('aria-pressed="true"');
    expect(archive).toContain('class="ab-pill"');
    expect(archive).toContain('data-active="false"');
    expect(archive).toContain('aria-pressed="false"');
    expect(html).toMatch(/>Workspace<\/button>/);
    expect(html).toMatch(/>Archive<\/button>/);
  });

  it("moves the active mark to Archive", () => {
    const html = renderToStaticMarkup(createElement(ModeSwitch, { mode: "archive", onChange: () => {} }));
    expect(buttonTag(html, "archive")).toContain('data-active="true"');
    expect(buttonTag(html, "workspace")).toContain('data-active="false"');
  });

  it("names the shortcut of each segment in its title", () => {
    vi.stubGlobal("navigator", { platform: "Win32" });
    const html = renderToStaticMarkup(createElement(ModeSwitch, { mode: "workspace", onChange: () => {} }));
    expect(buttonTag(html, "workspace")).toContain('title="Workspace (Ctrl+1)"');
    expect(buttonTag(html, "archive")).toContain('title="Archive (Ctrl+2)"');
  });

  it("uses the Command key in the titles on macOS", () => {
    vi.stubGlobal("navigator", { platform: "MacIntel" });
    const html = renderToStaticMarkup(createElement(ModeSwitch, { mode: "workspace", onChange: () => {} }));
    expect(buttonTag(html, "workspace")).toContain('title="Workspace (⌘1)"');
    expect(buttonTag(html, "archive")).toContain('title="Archive (⌘2)"');
  });

  it("asks for the other mode and ignores a click on the active segment", () => {
    const onChange = vi.fn();
    const tree = ModeSwitch({ mode: "workspace", onChange });
    const byMode = (mode: AppMode) => buttons(tree).find((b) => b.props["data-mode"] === mode);
    (byMode("workspace")?.props.onClick as () => void)();
    expect(onChange).not.toHaveBeenCalled();
    (byMode("archive")?.props.onClick as () => void)();
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith("archive");
  });
});

describe("AppFooter", () => {
  const props = (over: Partial<Parameters<typeof AppFooter>[0]> = {}) => ({
    mode: "workspace" as AppMode,
    onModeChange: () => {},
    onOpenPalette: () => {},
    onOpenShortcuts: () => {},
    ...over,
  });

  it("shows the logo, name and version, then the mode switch, then the mode slot, then Search", () => {
    const html = renderToStaticMarkup(
      createElement(AppFooter, props(), createElement("button", { type: "button" }, "New Batch")),
    );
    expect(html).toMatch(/<footer[^>]*data-testid="app-footer"/);
    const order = ['alt="AstroBurst Logo"', ">AstroBurst<", APP_VERSION, 'data-testid="mode-switch"', ">New Batch<", ">Search<"].map((s) =>
      html.indexOf(s),
    );
    expect(order.every((i) => i >= 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
  });

  it("keeps the Search Ctrl+K and shortcut sheet buttons", () => {
    const html = renderToStaticMarkup(createElement(AppFooter, props()));
    expect(html).toContain('title="Search everywhere (Ctrl+K or double Shift)"');
    expect(html).toContain('<kbd class="ab-cmdp-kbd">Ctrl+K</kbd>');
    expect(html).toContain('title="Keyboard shortcuts (?)"');
    expect(html).toContain('<kbd class="ab-cmdp-kbd">?</kbd>');
  });

  it("separates the mode slot from the switch only when there is a slot", () => {
    const separator = /class="w-px h-3"/g;
    expect(renderToStaticMarkup(createElement(AppFooter, props())).match(separator)).toBeNull();
    const withSlot = renderToStaticMarkup(createElement(AppFooter, props(), createElement("span", null, "MAST")));
    expect(withSlot.match(separator)).toHaveLength(1);
  });

  it("passes the mode to the switch and wires Search and the shortcut sheet", () => {
    const onModeChange = vi.fn();
    const onOpenPalette = vi.fn();
    const onOpenShortcuts = vi.fn();
    const tree = AppFooter(props({ mode: "archive", onModeChange, onOpenPalette, onOpenShortcuts }));
    const modeSwitch = elements(tree).find((el) => el.type === ModeSwitch);
    expect(modeSwitch?.props.mode).toBe("archive");
    expect(modeSwitch?.props.onChange).toBe(onModeChange);

    const byTitle = (title: string) => buttons(tree).find((b) => b.props.title === title);
    (byTitle("Search everywhere (Ctrl+K or double Shift)")?.props.onClick as () => void)();
    expect(onOpenPalette).toHaveBeenCalledTimes(1);
    expect(onOpenShortcuts).not.toHaveBeenCalled();
    (byTitle("Keyboard shortcuts (?)")?.props.onClick as () => void)();
    expect(onOpenShortcuts).toHaveBeenCalledTimes(1);
  });
});

describe("EmptyState archive entry", () => {
  const base = { onBrowseFiles: () => {}, onSelectFolder: () => {} };

  it("offers no archive button unless onBrowseArchive is passed", () => {
    const html = renderToStaticMarkup(createElement(EmptyState, base));
    expect(html).toContain("Browse Files");
    expect(html).toContain("Select Folder");
    expect(html).not.toContain("Browse the MAST archive");
    expect(html).not.toContain('data-testid="empty-browse-archive"');
  });

  it("adds Browse the MAST archive with the Ctrl+2 hint after Select Folder", () => {
    vi.stubGlobal("navigator", { platform: "Win32" });
    const html = renderToStaticMarkup(createElement(EmptyState, { ...base, onBrowseArchive: () => {} }));
    const tag = /<button[^>]*data-testid="empty-browse-archive"[^>]*>/.exec(html)?.[0] ?? "";
    expect(tag).toContain('type="button"');
    expect(tag).toContain('class="ab-btn-secondary"');
    const button = html.slice(html.indexOf(tag), html.indexOf("</button>", html.indexOf(tag)));
    expect(button).toContain("Browse the MAST archive");
    expect(button).toMatch(/<kbd[^>]*>Ctrl\+2<\/kbd>/);
    expect(html.indexOf("Select Folder")).toBeLessThan(html.indexOf("Browse the MAST archive"));
  });

  it("shows the Command key hint on macOS", () => {
    vi.stubGlobal("navigator", { platform: "MacIntel" });
    const html = renderToStaticMarkup(createElement(EmptyState, { ...base, onBrowseArchive: () => {} }));
    expect(html).toMatch(/<kbd[^>]*>⌘2<\/kbd>/);
    expect(html).not.toContain("Ctrl+2");
  });

  it("calls onBrowseArchive without letting the click reach the drop zone", () => {
    const onBrowseArchive = vi.fn();
    const onBrowseFiles = vi.fn();
    const tree = EmptyState({ ...base, onBrowseFiles, onBrowseArchive });
    const archive = buttons(tree).find((b) => textOf(b.props.children).includes("Browse the MAST archive"));
    const event = clickEvent();
    (archive?.props.onClick as (e: ReturnType<typeof clickEvent>) => void)(event);
    expect(onBrowseArchive).toHaveBeenCalledTimes(1);
    expect(onBrowseFiles).not.toHaveBeenCalled();
    expect(event.preventDefault).toHaveBeenCalledTimes(1);
    expect(event.stopPropagation).toHaveBeenCalledTimes(1);
  });
});

describe("ambient declarations for the archive", () => {
  it("declare the lucide icons the archive uses", () => {
    for (const icon of [Lock, LockOpen, KeyRound, Table, LayoutGrid, CloudOff, AudioWaveform, ChevronLeft, ExternalLink]) {
      expect(icon).toBeTruthy();
    }
  });

  it("declare openUrl, which opens a link through the opener plugin", async () => {
    const invoke = vi.fn(async () => undefined);
    vi.stubGlobal("window", { __TAURI_INTERNALS__: { invoke } });
    await openUrl("https://auth.mast.stsci.edu/token");
    expect(invoke).toHaveBeenCalledWith("plugin:opener|open_url", { url: "https://auth.mast.stsci.edu/token", with: undefined }, undefined);
  });
});
