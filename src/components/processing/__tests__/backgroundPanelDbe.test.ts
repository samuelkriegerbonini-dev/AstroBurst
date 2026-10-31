import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { choices, flips, interceptDuringRender } = vi.hoisted(() => {
  const choices = new Map<string, string[]>();
  const flips = new Map<string, boolean>();
  type Factory = (type: unknown, props: Record<string, unknown> | null, ...rest: unknown[]) => unknown;
  const interceptDuringRender = (factory: Factory): Factory => (type, props, ...rest) => {
    const id = props?.["data-testid"];
    const value = typeof type === "string" && typeof id === "string" ? choices.get(id)?.shift() : undefined;
    if (value !== undefined) (props?.onChange as (e: { target: { value: string } }) => void)({ target: { value } });
    const label = props?.label;
    if (typeof type !== "string" && typeof label === "string" && flips.has(label)) {
      const checked = flips.get(label) as boolean;
      flips.delete(label);
      (props?.onChange as (v: boolean) => void)(checked);
    }
    return factory(type, props, ...rest);
  };
  return { choices, flips, interceptDuringRender };
});

vi.mock("react/jsx-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-runtime")>();
  return { ...actual, jsx: interceptDuringRender(actual.jsx as never), jsxs: interceptDuringRender(actual.jsxs as never) };
});

vi.mock("react/jsx-dev-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-dev-runtime")>();
  return { ...actual, jsxDEV: interceptDuringRender(actual.jsxDEV as never) };
});

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf } from "../../../context/PreviewContext";
import { regionStore } from "../../../utils/regionStore";
import BackgroundPanel from "../BackgroundPanel";
import type { CompositeInputView } from "../compositeProps";
import type { ProcessedFile } from "../../../shared/types";
import type { Region } from "../../../shared/types/regions";

const PATH = "C:/data/673nmos.fits";
const COMPOSITE_INPUT: CompositeInputView = { input: "base", previewUrl: "asset://out/blend.png", label: "Composite" };
const NOT_USED = "Point regions are not used: turn on 'Use point regions as samples' or Auto Grid";
const NOT_USED_ON_COMPOSITE = "Point regions are not used on the composite: turn Auto Grid on";
const POINT_REGIONS_ON_COMPOSITE = "Point regions belong to the file's pixel grid, not the composite";
const POINT_REGIONS_SWITCH = /<button[^>]*role="switch"[^>]*aria-label="Use point regions as samples"[^>]*>/;

function fileOf(id: string): ProcessedFile {
  return {
    id,
    name: "673nmos.fits",
    path: PATH,
    sourcePath: PATH,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/673.png", previewUrl: "asset://C:/out/673.png", dimensions: [1600, 1600], elapsed_ms: 1, is_rgb: false },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function pointRegion(id: string, x: number, y: number): Region {
  return { id, shape: { shape: "point", x, y }, props: { color: null, width: null, text: null, dash: null, include: true }, backgroundId: null };
}

function render(file: ProcessedFile, overrides: Record<string, unknown> = {}): string {
  const panel = createElement(BackgroundPanel, {
    selectedFile: file,
    outputDir: "C:/out",
    fileKey: fileKeyOf(file),
    compositeMode: false,
    compositeInput: null,
    onCompositeDone: () => {},
    fileName: file.name,
    disabledReason: null,
    ...overrides,
  });
  const preview = createElement(PreviewProvider, { file, doneFiles: [file], children: panel });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function renderSpline(id: string, options: { gridOn?: boolean; pointRegionsOn?: boolean; overrides?: Record<string, unknown> } = {}): string {
  choices.set("background-model", ["spline"]);
  if (options.gridOn === false) flips.set("Auto Grid", false);
  if (options.pointRegionsOn === false) flips.set("Use point regions as samples", false);
  return render(fileOf(id), options.overrides);
}

function runButton(html: string): { disabled: boolean; describedBy: string[] } {
  const attrs = /<button([^>]*class="ab-run-btn[^"]*"[^>]*)>/.exec(html)?.[1];
  if (attrs === undefined) throw new Error("run button not rendered");
  return {
    disabled: /data-disabled="true"/.test(attrs),
    describedBy: (/aria-describedby="([^"]*)"/.exec(attrs)?.[1] ?? "").split(/\s+/).filter(Boolean),
  };
}

function textOf(markup: string): string {
  return markup.replace(/<!-- -->/g, "").replace(/<[^>]*>/g, "").replace(/&#x27;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, "&");
}

function notes(html: string): { id: string | null; text: string; live: boolean }[] {
  return [...html.matchAll(/(<div aria-live="polite"[^>]*>)?<(div|p)([^>]*role="note"[^>]*)>([\s\S]*?)<\/\2>/g)].map((m) => ({
    id: /id="([^"]+)"/.exec(m[3])?.[1] ?? null,
    text: textOf(m[4]),
    live: m[1] !== undefined,
  }));
}

function noteWith(html: string, text: string): { id: string | null; text: string; live: boolean } {
  const found = notes(html).find((n) => n.text === text);
  if (!found) throw new Error(`no note "${text}": ${JSON.stringify(notes(html))}`);
  return found;
}

beforeEach(() => {
  choices.clear();
  flips.clear();
  regionStore.clear(PATH);
});

describe("BackgroundPanel DBE checks", () => {
  it("disables Run with the grid off and no point regions, and says so in a live note that describes Run", () => {
    const html = renderSpline("dbe-none", { gridOn: false });
    const note = noteWith(html, "4 point regions needed, 0 used");
    expect(note.live).toBe(true);
    expect(note.id).toBeTruthy();
    const run = runButton(html);
    expect(run.disabled).toBe(true);
    expect(run.describedBy).toContain(note.id);
  });

  it("enables Run with the grid off once four point regions are used", () => {
    [[100, 100], [1500, 100], [100, 1500], [1500, 1500]].forEach(([x, y], i) => regionStore.add(PATH, pointRegion(`p${i}`, x, y)));
    const html = renderSpline("dbe-four", { gridOn: false });
    expect(html).not.toContain("point regions needed");
    expect(runButton(html)).toEqual({ disabled: false, describedBy: [] });
  });

  it("names the toggle when four point regions exist but are not used", () => {
    [[100, 100], [1500, 100], [100, 1500], [1500, 1500]].forEach(([x, y], i) => regionStore.add(PATH, pointRegion(`q${i}`, x, y)));
    const html = renderSpline("dbe-toggle-off", { gridOn: false, pointRegionsOn: false });
    const note = noteWith(html, NOT_USED);
    expect(runButton(html).describedBy).toContain(note.id);
    expect(runButton(html).disabled).toBe(true);
  });

  it("puts the Auto Grid switch before a 2..30 slider, with Run enabled and no note at the defaults", () => {
    const html = renderSpline("dbe-defaults");
    const switchAt = html.search(/<button[^>]*role="switch"[^>]*aria-checked="true"[^>]*aria-label="Auto Grid"/);
    const sliderLabel = /<label class="ab-slider-label" for="([^"]+)">Auto Grid \(per axis\)/.exec(html);
    expect(switchAt).toBeGreaterThan(-1);
    expect(sliderLabel).not.toBeNull();
    expect(switchAt).toBeLessThan(sliderLabel?.index ?? -1);
    const range = new RegExp(`<input[^>]*id="${sliderLabel?.[1]}"[^>]*>`).exec(html)?.[0] ?? "";
    expect(range).toContain('min="2"');
    expect(range).toContain('max="30"');
    expect(range).toContain('value="10"');
    expect(runButton(html)).toEqual({ disabled: false, describedBy: [] });
    expect(notes(html).filter((n) => /point region|Automatic grid/i.test(n.text))).toEqual([]);
  });

  it("hides the slider while the switch is off", () => {
    const html = renderSpline("dbe-switch-off", { gridOn: false });
    expect(html).toMatch(/<button[^>]*role="switch"[^>]*aria-checked="false"[^>]*aria-label="Auto Grid"/);
    expect(html).not.toContain("Auto Grid (per axis)");
  });

  it("on the composite, asks for Auto Grid instead of point regions", () => {
    regionStore.add(PATH, pointRegion("c0", 10, 10));
    const html = renderSpline("dbe-composite", { gridOn: false, overrides: { compositeMode: true, compositeInput: COMPOSITE_INPUT } });
    const note = noteWith(html, NOT_USED_ON_COMPOSITE);
    expect(runButton(html).disabled).toBe(true);
    expect(runButton(html).describedBy).toContain(note.id);
  });

  it("on the composite, says under the point-regions switch why it is off, and the switch points to that line", () => {
    const html = renderSpline("dbe-composite-switch", { overrides: { compositeMode: true, compositeInput: COMPOSITE_INPUT } });
    const note = noteWith(html, POINT_REGIONS_ON_COMPOSITE);
    const tag = POINT_REGIONS_SWITCH.exec(html)?.[0] ?? "";
    expect(note.id).toBeTruthy();
    expect(tag).toContain('disabled=""');
    expect(tag).toContain(`aria-describedby="${note.id}"`);
    expect(html).not.toContain(`title="${POINT_REGIONS_ON_COMPOSITE.replace("'", "&#x27;")}"`);
    expect(runButton(html)).toEqual({ disabled: false, describedBy: [] });
  });

  it("on a file, the point-regions switch has no composite line and describes nothing", () => {
    const html = renderSpline("dbe-file-switch");
    expect(textOf(html)).not.toContain(POINT_REGIONS_ON_COMPOSITE);
    expect(POINT_REGIONS_SWITCH.exec(html)?.[0]).not.toContain("aria-describedby");
  });

  it("keeps the polynomial model free of DBE notes", () => {
    const html = render(fileOf("dbe-polynomial"));
    expect(html).not.toContain("point regions needed");
    expect(runButton(html)).toEqual({ disabled: false, describedBy: [] });
  });
});
