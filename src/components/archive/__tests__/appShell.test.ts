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
import type { SampleControls } from "../../../hooks/useSampleData";

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

describe("EmptyState sample data entry", () => {
  const base = { onBrowseFiles: () => {}, onSelectFolder: () => {} };
  const TOTAL = 30_896_640;
  const CREDIT = "HST/WFPC2 narrowband frames of the Eagle Nebula (M16): NASA, ESA, J. Hester and P. Scowen (Arizona State University), data via STScI";

  function sample(over: Partial<SampleControls> = {}): SampleControls {
    return {
      id: "hst-m16-wfpc2",
      title: "HST/WFPC2 narrowband frames of M16 (Eagle Nebula)",
      credit: CREDIT,
      state: "absent",
      receivedBytes: 0,
      totalBytes: TOTAL,
      file: null,
      error: null,
      download: vi.fn(async () => {}),
      cancel: vi.fn(async () => {}),
      open: vi.fn(async () => {}),
      retry: vi.fn(async () => {}),
      ...over,
    };
  }

  function render(over: Partial<SampleControls> = {}) {
    return renderToStaticMarkup(createElement(EmptyState, { ...base, onBrowseArchive: () => {}, sample: sample(over) }));
  }

  function testId(html: string, id: string): string {
    const tag = new RegExp(`<(\\w+)[^>]*data-testid="${id}"[^>]*>`).exec(html);
    if (!tag) return "";
    const at = html.indexOf(tag[0]);
    return html.slice(at, html.indexOf(`</${tag[1]}>`, at));
  }

  it("offers no sample data block unless the sample set is passed", () => {
    const html = renderToStaticMarkup(createElement(EmptyState, base));
    expect(html).not.toContain("sample data");
    expect(html).not.toContain('data-testid="empty-open-sample"');
    expect(html).not.toContain("HST narrowband frames of M16");
  });

  it("offers Download sample data with the set size after the archive button when the frames are absent", () => {
    vi.stubGlobal("navigator", { platform: "Win32" });
    const html = render();
    const tag = /<button[^>]*data-testid="empty-open-sample"[^>]*>/.exec(html)?.[0] ?? "";
    expect(tag).toContain('type="button"');
    expect(tag).toContain('class="ab-btn-secondary"');
    expect(testId(html, "empty-open-sample")).toContain("Download sample data (31 MB)");
    expect(html.indexOf("Browse the MAST archive")).toBeLessThan(html.indexOf("Download sample data"));
    expect(html).not.toContain('data-testid="empty-sample-progress"');
    expect(html).not.toContain('data-testid="empty-sample-error"');
  });

  it("says the frames are downloaded once and kept in the data folder, with the credit", () => {
    const html = render();
    expect(html).toContain("HST narrowband frames of M16");
    expect(html).toContain("downloaded once and kept in the data folder");
    expect(html).toContain(CREDIT);
  });

  it("offers to resume with the received amount when a download stopped part way", () => {
    const html = render({ state: "partial", receivedBytes: 10_298_880 });
    expect(testId(html, "empty-open-sample")).toContain("Resume sample download (10 MB of 31 MB)");
  });

  it("gives the percentage when the received amount would round to the whole set", () => {
    expect(testId(render({ state: "partial", receivedBytes: 30_600_000 }), "empty-open-sample")).toContain("Resume sample download (99% of 31 MB)");
  });

  it("offers a plain download when a partial set has nothing received yet", () => {
    const button = testId(render({ state: "partial", receivedBytes: 0 }), "empty-open-sample");
    expect(button).toContain("Download sample data (31 MB)");
    expect(button).not.toContain("Resume");
  });

  it("shows a progress bar with the percentage and Cancel instead of the button while downloading", () => {
    const html = render({ state: "downloading", receivedBytes: 15_448_320, file: "656nmos.fits" });
    expect(html).not.toContain('data-testid="empty-open-sample"');
    const bar = /<div[^>]*data-testid="empty-sample-progress"[^>]*>/.exec(html)?.[0] ?? "";
    expect(bar).toContain('role="progressbar"');
    expect(bar).toContain('aria-valuemin="0"');
    expect(bar).toContain('aria-valuemax="100"');
    expect(bar).toContain('aria-valuenow="50"');
    expect(html).toContain("50%");
    expect(html.indexOf("Browse the MAST archive")).toBeLessThan(html.indexOf('data-testid="empty-sample-progress"'));
    const cancel = /<button[^>]*data-testid="empty-sample-cancel"[^>]*>/.exec(html)?.[0] ?? "";
    expect(cancel).toContain('type="button"');
    expect(testId(html, "empty-sample-cancel")).toContain("Cancel");
  });

  it("names the received amount on the bar and the download on Cancel for screen readers", () => {
    const html = render({ state: "downloading", receivedBytes: 15_448_320, file: "656nmos.fits" });
    const bar = /<div[^>]*data-testid="empty-sample-progress"[^>]*>/.exec(html)?.[0] ?? "";
    expect(bar).toContain('aria-valuetext="15 MB of 31 MB"');
    const cancel = /<button[^>]*data-testid="empty-sample-cancel"[^>]*>/.exec(html)?.[0] ?? "";
    expect(cancel).toContain('aria-label="Cancel sample download"');
  });

  it("holds the bar below 100% until the last byte arrives", () => {
    expect(render({ state: "downloading", receivedBytes: TOTAL - 1 })).toContain('aria-valuenow="99"');
    expect(render({ state: "downloading", receivedBytes: TOTAL })).toContain('aria-valuenow="100"');
    expect(render({ state: "downloading", receivedBytes: 0, totalBytes: 0 })).toContain('aria-valuenow="0"');
  });

  it("offers Open sample data once the frames are kept", () => {
    const html = render({ state: "ready", receivedBytes: TOTAL });
    expect(testId(html, "empty-open-sample")).toContain("Open sample data");
    expect(html).not.toContain("Download sample data");
  });

  it("shows the error with Retry", () => {
    const html = render({ state: "partial", receivedBytes: 10_298_880, error: "656nmos.fits: the server answered HTTP 404" });
    const error = /<p[^>]*data-testid="empty-sample-error"[^>]*>/.exec(html)?.[0] ?? "";
    expect(error).toContain('role="alert"');
    expect(testId(html, "empty-sample-error")).toContain("656nmos.fits: the server answered HTTP 404");
    expect(testId(html, "empty-open-sample")).toContain("Retry sample download");
  });

  it("keeps Open sample data for kept frames whose open failed, with the error beside it", () => {
    const html = render({ state: "ready", receivedBytes: TOTAL, error: "the Data folder setting could not be read: x" });
    expect(testId(html, "empty-open-sample")).toContain("Open sample data");
    expect(html).not.toContain("Retry sample download");
    expect(testId(html, "empty-sample-error")).toContain("the Data folder setting could not be read: x");
  });

  it.each([
    ["absent", {}, "download"],
    ["partial", { state: "partial", receivedBytes: 10_298_880 }, "download"],
    ["ready", { state: "ready", receivedBytes: TOTAL }, "open"],
    ["failed", { state: "partial", receivedBytes: 10_298_880, error: "network unreachable" }, "retry"],
    ["ready after a failed open", { state: "ready", receivedBytes: TOTAL, error: "not readable" }, "open"],
  ] as const)("runs the %s action without letting the click reach the drop zone", (_label, over, action) => {
    const onBrowseFiles = vi.fn();
    const set = sample(over);
    const tree = EmptyState({ ...base, onBrowseFiles, sample: set });
    const main = buttons(tree).find((b) => b.props["data-testid"] === "empty-open-sample");
    const event = clickEvent();
    (main?.props.onClick as (e: ReturnType<typeof clickEvent>) => void)(event);
    for (const name of ["download", "open", "retry", "cancel"] as const) {
      expect(set[name]).toHaveBeenCalledTimes(name === action ? 1 : 0);
    }
    expect(onBrowseFiles).not.toHaveBeenCalled();
    expect(event.preventDefault).toHaveBeenCalledTimes(1);
    expect(event.stopPropagation).toHaveBeenCalledTimes(1);
  });

  it("cancels a running download without letting the click reach the drop zone", () => {
    const onBrowseFiles = vi.fn();
    const set = sample({ state: "downloading", receivedBytes: 1_000 });
    const tree = EmptyState({ ...base, onBrowseFiles, sample: set });
    const cancel = buttons(tree).find((b) => b.props["data-testid"] === "empty-sample-cancel");
    const event = clickEvent();
    (cancel?.props.onClick as (e: ReturnType<typeof clickEvent>) => void)(event);
    expect(set.cancel).toHaveBeenCalledTimes(1);
    expect(set.download).not.toHaveBeenCalled();
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
