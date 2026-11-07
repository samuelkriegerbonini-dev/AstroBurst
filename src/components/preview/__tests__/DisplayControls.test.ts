import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, type ReactNode } from "react";
import { hooks } from "../../../test/hookRuntime";
import { findElement } from "../../../test/elementTree";
import { renderToStaticMarkup } from "react-dom/server";
import { DEFAULT_DISPLAY_SETTINGS, type DisplaySettings } from "../../../shared/types/display";
import { wcsOverlayStatus } from "../../../utils/wcsOverlayStatus";
import { LINEAR_HINT_TITLE, type LinearUndo } from "../../../utils/displayMemory";
import { SYMMETRIC_NONLINEAR_STRETCH_NOTE } from "../../../utils/displayTransfer";
import { GPU_WAITING_TITLE, PNG_ONLY_TITLE } from "../../../utils/previewShell";

const DISPLAY_CSS = readFileSync(fileURLToPath(new URL("../display-controls.css", import.meta.url)), "utf8");
const LINEAR_ONLY_CLASS = ["ab-display", "linear"].join("-");

vi.mock("react", async (importOriginal) => {
  const { reactWithFakeHooks } = await import("../../../test/hookRuntime");
  return reactWithFakeHooks(await importOriginal());
});

interface MemoryState {
  linear: LinearUndo | null;
  linearBlocked: string | null;
  sameGroup: boolean;
}

const NO_MEMORY: MemoryState = { linear: null, linearBlocked: null, sameGroup: true };

const displayState: { display: DisplaySettings; memory: MemoryState; rawPixelsLoading: boolean } = {
  display: DEFAULT_DISPLAY_SETTINGS,
  memory: NO_MEMORY,
  rawPixelsLoading: false,
};

const setDisplay = vi.fn((patch: Partial<DisplaySettings>) => { displayState.display = { ...displayState.display, ...patch }; });
const setSameGroup = vi.fn((sameGroup: boolean) => { displayState.memory = { ...displayState.memory, sameGroup }; });

vi.mock("../../../context/PreviewContext", () => ({
  useDisplayContext: () => ({
    display: displayState.display,
    setDisplay,
    limits: null,
    limitsLoading: false,
    limitsError: null,
    linear: displayState.memory.linear,
    linearBlocked: displayState.memory.linearBlocked,
    toggleLinear: () => {},
    resetDisplay: () => {},
    setAsDefault: () => {},
    sameGroup: displayState.memory.sameGroup,
    setSameGroup,
  }),
  useRawPixelsContext: () => ({ rawPixelsLoading: displayState.rawPixelsLoading }),
}));

import DisplayControls from "../DisplayControls";

function render(
  props: { renderOnlyDisabled?: boolean; renderOnlyTitle?: string },
  display: Partial<DisplaySettings> = {},
  memory: Partial<MemoryState> = {},
): string {
  displayState.display = { ...DEFAULT_DISPLAY_SETTINGS, ...display };
  displayState.memory = { ...NO_MEMORY, ...memory };
  return renderToStaticMarkup(createElement(DisplayControls, { vmin: 0, vmax: 1, ...props }));
}

function insideDisabledFieldset(markup: string, text: string): boolean {
  const at = markup.indexOf(text);
  expect(at).toBeGreaterThan(-1);
  return markup.lastIndexOf('<fieldset disabled=""', at) > markup.lastIndexOf("</fieldset>", at);
}

function describedBy(tag: string): string {
  const id = /aria-describedby="([^"]+)"/.exec(tag)?.[1];
  expect(id).toBeDefined();
  return id ?? "";
}

function spanWithId(markup: string, id: string): string {
  const at = markup.indexOf(`id="${id}"`);
  expect(at).toBeGreaterThan(-1);
  return markup.slice(markup.lastIndexOf("<", at), markup.indexOf("</span>", at) + "</span>".length);
}

function statusNote(markup: string): string {
  const at = markup.indexOf('role="status"');
  expect(at).toBeGreaterThan(-1);
  return markup.slice(markup.lastIndexOf("<", at), markup.indexOf("</span>", at));
}

interface CssBlock {
  prelude: string;
  body: string;
}

function cssBlocks(css: string): CssBlock[] {
  const blocks: CssBlock[] = [];
  let depth = 0;
  let preludeStart = 0;
  let bodyStart = 0;
  for (let i = 0; i < css.length; i++) {
    if (css[i] === "{" && depth++ === 0) bodyStart = i + 1;
    if (css[i] === "}" && --depth === 0) {
      blocks.push({ prelude: css.slice(preludeStart, bodyStart - 1).trim(), body: css.slice(bodyStart, i) });
      preludeStart = i + 1;
    }
  }
  return blocks;
}

function cssRules(selector: string, css = DISPLAY_CSS): string {
  return cssBlocks(css)
    .filter((block) => block.prelude.split(",").map((s) => s.trim()).includes(selector))
    .map((block) => block.body)
    .join("\n");
}

function mediaCss(query: string): string {
  return cssBlocks(DISPLAY_CSS)
    .filter((block) => block.prelude === `@media ${query}`)
    .map((block) => block.body)
    .join("\n");
}

describe("DisplayControls without GPU rendering", () => {
  afterEach(() => {
    wcsOverlayStatus.set("grid", null);
    wcsOverlayStatus.set("compass", null);
    displayState.rawPixelsLoading = false;
  });

  it("disables stretch, limits, symmetric, colormap and invert with the reason", () => {
    const html = render({ renderOnlyDisabled: true });
    for (const label of [">stretch<", ">limits<", ">symmetric<", ">cmap<", ">invert<"]) {
      expect(insideDisabledFieldset(html, label)).toBe(true);
    }
    expect(html.match(/title="needs GPU rendering"/g)?.length).toBeGreaterThanOrEqual(3);
  });

  it("keeps grid and compass usable", () => {
    const html = render({ renderOnlyDisabled: true });
    expect(insideDisabledFieldset(html, ">grid<")).toBe(false);
    expect(insideDisabledFieldset(html, ">compass<")).toBe(false);
    expect(html).not.toContain("resolved display limits");
  });

  it("gives the disabled controls the reason it is passed instead of the default", () => {
    const html = render({ renderOnlyDisabled: true, renderOnlyTitle: "waiting for the GPU image" });
    expect(html.match(/title="waiting for the GPU image"/g)?.length).toBeGreaterThanOrEqual(3);
    expect(html).not.toContain("needs GPU rendering");
    expect(insideDisabledFieldset(html, ">stretch<")).toBe(true);
    expect(insideDisabledFieldset(html, ">grid<")).toBe(false);
  });

  it("ignores the reason while the controls are enabled", () => {
    expect(render({ renderOnlyTitle: "waiting for the GPU image" })).not.toContain("waiting for the GPU image");
  });

  it("leaves every control enabled on the GPU display", () => {
    const html = render({});
    expect(html).not.toContain('<fieldset disabled=""');
    expect(html).not.toContain("needs GPU rendering");
    expect(html).toContain("resolved display limits");
  });

  it("disables the symmetric, invert and colorbar toggles themselves and keeps grid, compass and same filter clickable", () => {
    const html = render({ renderOnlyDisabled: true });
    for (const id of ["symmetric", "invert", "colorbar"]) {
      expect(openingTag(html, `display-${id}`)).toContain('disabled=""');
    }
    for (const id of ["grid", "compass", "same-group"]) {
      expect(openingTag(html, `display-${id}`)).not.toContain("disabled");
    }
  });

  it("names every control", () => {
    const html = render({});
    expect(openingTag(html, "display-controls")).toContain('aria-label="Image display"');
    for (const name of ["Stretch curve", "Display limits", "Color map"]) {
      expect(html).toMatch(new RegExp(`<select[^>]*aria-label="${name}"`));
    }
    const toggles: [string, string][] = [
      ["symmetric", "Symmetric"],
      ["invert", "Invert"],
      ["colorbar", "Colorbar"],
      ["grid", "Grid"],
      ["compass", "Compass"],
      ["same-group", "Same filter"],
    ];
    for (const [id, name] of toggles) {
      expect(openingTag(html, `display-${id}`)).toContain(`aria-label="${name}"`);
    }
  });

  it("says inline why the controls are off, right after Linear, with a spinner only while the GPU image loads", () => {
    displayState.rawPixelsLoading = true;
    const waiting = render({ renderOnlyDisabled: true, renderOnlyTitle: GPU_WAITING_TITLE });
    const note = statusNote(waiting);
    expect(note).toContain(GPU_WAITING_TITLE);
    expect(note).toContain("animate-spin");
    expect(note).toContain('class="ab-display-note"');
    const at = waiting.indexOf('role="status"');
    expect(at).toBeGreaterThan(waiting.indexOf('data-testid="display-linear"'));
    expect(at).toBeLessThan(waiting.indexOf('<fieldset disabled=""'));
    expect(waiting.match(/role="status"/g)).toHaveLength(1);
    displayState.rawPixelsLoading = false;
    const png = statusNote(render({ renderOnlyDisabled: true, renderOnlyTitle: PNG_ONLY_TITLE }));
    expect(png).toContain(PNG_ONLY_TITLE);
    expect(png).not.toContain("animate-spin");
    expect(statusNote(render({ renderOnlyDisabled: true }))).toContain("needs GPU rendering");
    expect(render({})).not.toContain('role="status"');
  });

  it("takes the spinner from the GPU image load, not from the wording of the reason", () => {
    displayState.rawPixelsLoading = true;
    const detailed = statusNote(render({ renderOnlyDisabled: true, renderOnlyTitle: `${GPU_WAITING_TITLE} (48 MB)` }));
    expect(detailed).toContain(`${GPU_WAITING_TITLE} (48 MB)`);
    expect(detailed).toContain("animate-spin");
    expect(render({})).not.toContain("animate-spin");
    displayState.rawPixelsLoading = false;
    expect(statusNote(render({ renderOnlyDisabled: true, renderOnlyTitle: GPU_WAITING_TITLE }))).not.toContain("animate-spin");
  });
});

describe("DisplayControls colorbar toggle", () => {
  it("is on by default", () => {
    expect(DEFAULT_DISPLAY_SETTINGS.colorbar).toBe(true);
    expect(openingTag(render({}), "display-colorbar")).toContain('aria-pressed="true"');
  });

  it("reflects a colorbar switched off", () => {
    expect(openingTag(render({}, { colorbar: false }), "display-colorbar")).toContain('aria-pressed="false"');
  });

  it("sits with the render-only controls, disabled without GPU rendering", () => {
    expect(insideDisabledFieldset(render({ renderOnlyDisabled: true }), ">colorbar<")).toBe(true);
    expect(insideDisabledFieldset(render({}), ">colorbar<")).toBe(false);
  });
});

describe("DisplayControls labels and notes", () => {
  afterEach(() => {
    wcsOverlayStatus.set("grid", null);
    wcsOverlayStatus.set("compass", null);
  });

  it("names its reset button Reset display", () => {
    expect(render({})).toContain("Reset display</button>");
  });

  it("shows an amber no-WCS note with the reason when a ticked grid could not be drawn", () => {
    wcsOverlayStatus.set("grid", "no celestial WCS in the header");
    const html = render({}, { grid: true });
    expect(html).toContain("(no WCS)");
    expect(html).toContain('title="grid: no celestial WCS in the header"');
  });

  it("shows no note for an overlay that is not ticked", () => {
    wcsOverlayStatus.set("compass", "the WCS has no usable orientation");
    expect(render({}, { compass: false })).not.toContain("(no WCS)");
  });

  it("says under mtf that Symmetric sets the stretch to linear, inside its fieldset and linked to the button", () => {
    const html = render({}, { stretch: "mtf" });
    const symmetric = openingTag(html, "display-symmetric");
    expect(spanWithId(html, describedBy(symmetric))).toContain("(sets stretch to linear)");
    const start = html.indexOf(symmetric);
    const at = html.indexOf("(sets stretch to linear)");
    expect(at).toBeGreaterThan(start);
    expect(html.slice(start, at)).not.toContain("</fieldset>");
    expect(at).toBeLessThan(html.indexOf('data-testid="display-invert"'));
  });

  it("drops the mtf hint off mtf and on the CPU viewer", () => {
    const linear = render({}, { stretch: "linear" });
    expect(linear).not.toContain("(sets stretch to linear)");
    expect(openingTag(linear, "display-symmetric")).not.toContain("aria-describedby");
    const cpu = render({ renderOnlyDisabled: true }, { stretch: "mtf" });
    expect(cpu).not.toContain("(sets stretch to linear)");
    expect(openingTag(cpu, "display-symmetric")).not.toContain("aria-describedby");
  });

  it("puts the no-WCS note at the end of the grid and compass group and links it to the ticked toggle", () => {
    wcsOverlayStatus.set("grid", "no celestial WCS in the header");
    const html = render({}, { grid: true });
    const compassAt = html.indexOf('data-testid="display-compass"');
    const noteAt = html.indexOf("(no WCS)");
    expect(noteAt).toBeGreaterThan(compassAt);
    expect(html.slice(compassAt, noteAt)).not.toContain("</div>");
    expect(noteAt).toBeLessThan(html.indexOf('class="ab-display-memory"'));
    const note = spanWithId(html, describedBy(openingTag(html, "display-grid")));
    expect(note).toContain("(no WCS)");
    expect(note).toContain("grid: no celestial WCS in the header</span>");
    expect(openingTag(html, "display-compass")).not.toContain("aria-describedby");
  });

  it("links the no-WCS note to Compass only while its WCS error is set", () => {
    wcsOverlayStatus.set("compass", "the WCS has no usable orientation");
    const html = render({}, { compass: true });
    const note = spanWithId(html, describedBy(openingTag(html, "display-compass")));
    expect(note).toContain("compass: the WCS has no usable orientation");
    expect(openingTag(html, "display-grid")).not.toContain("aria-describedby");
  });

  it("puts the symmetric stretch note after the centre field, inside the symmetric fieldset", () => {
    const html = render({}, { stretch: "asinh", symmetric: true });
    const noteAt = html.indexOf(`(${SYMMETRIC_NONLINEAR_STRETCH_NOTE})`);
    expect(noteAt).toBeGreaterThan(html.indexOf('aria-label="Symmetric limits centre"'));
    expect(noteAt).toBeLessThan(html.indexOf('data-testid="display-invert"'));
  });

  it("shows the resolved range at the end of the limits fieldset", () => {
    const html = render({}, { stretch: "linear", limits: "zscale" });
    const limitsAt = html.indexOf('aria-label="Display limits"');
    const rangeAt = html.indexOf('title="resolved display limits"');
    expect(rangeAt).toBeGreaterThan(html.indexOf('title="zscale contrast"'));
    expect(html.slice(limitsAt, rangeAt)).not.toContain("</fieldset>");
    expect(rangeAt).toBeLessThan(html.indexOf('aria-label="Color map"'));
  });
});

describe("display-controls.css", () => {
  it("draws a state box on every toggle, hollow when off and filled when pressed, with a solid accent border", () => {
    expect(cssRules(".ab-display-toggle::before")).toMatch(/content:\s*""/);
    expect(cssRules(".ab-display-toggle::before")).toMatch(/border:\s*1px solid currentColor/);
    expect(cssRules(".ab-display-toggle::before")).not.toMatch(/background/);
    expect(cssRules('.ab-display-toggle[aria-pressed="true"]::before')).toMatch(/background:\s*currentColor/);
    expect(cssRules('.ab-display-toggle[aria-pressed="true"]')).toMatch(/border-color:\s*var\(--ab-accent\)/);
  });

  it("keeps the on state visible under forced colors: a system color fill and a Highlight border, GrayText while disabled", () => {
    const forced = mediaCss("(forced-colors: active)");
    expect(cssRules('.ab-display-toggle[aria-pressed="true"]::before', forced)).toMatch(/background:\s*CanvasText/);
    expect(cssRules('.ab-display-toggle[aria-pressed="true"]', forced)).toMatch(/border-color:\s*Highlight/);
    expect(cssRules('.ab-display-toggle[aria-pressed="true"]:disabled::before', forced)).toMatch(/background:\s*GrayText/);
    expect(cssRules('.ab-display-toggle[aria-pressed="true"]:disabled', forced)).toMatch(/border-color:\s*GrayText/);
    expect(cssRules('.ab-display-toggle[aria-pressed="true"]::before')).not.toMatch(/CanvasText/);
  });

  it("has no Linear-only rule left", () => {
    expect(DISPLAY_CSS).not.toContain(`.${LINEAR_ONLY_CLASS}`);
  });

  it("keeps only the memory group right-aligned", () => {
    expect(cssRules(".ab-display-range")).not.toMatch(/margin-left/);
    expect(cssRules(".ab-display-memory")).toMatch(/margin-left:\s*auto/);
  });

  it("styles the inline hint and the notes", () => {
    expect(cssRules(".ab-display-hint")).toMatch(/color:\s*var\(--ab-text-3\)/);
    expect(cssRules(".ab-display-note")).toMatch(/color:\s*var\(--ab-amber\)/);
  });
});

describe("DisplayControls decimal fields", () => {
  const layouts: Partial<DisplaySettings>[] = [
    { stretch: "asinh", limits: "percentile", symmetric: true, centre: -300 },
    { stretch: "power", limits: "user", userLo: -2.5, userHi: null },
    { stretch: "linear", limits: "zscale" },
  ];

  it("renders every stretch, limit and centre field as a decimal text box, never type=number", () => {
    const pages = layouts.map((layout) => render({}, layout));
    for (const html of pages) expect(html).not.toContain('type="number"');
    expect(pages.map((html) => html.match(/<input[^>]*inputMode="decimal"/g)?.length ?? 0)).toEqual([4, 3, 1]);
  });

  it("shows the bound values as dot decimals and an unset user limit as a blank optional field", () => {
    const symmetric = render({}, layouts[0]);
    expect(symmetric).toMatch(/<input[^>]*aria-label="Symmetric limits centre"[^>]*value="-300"/);
    const user = render({}, layouts[1]);
    expect(user).toMatch(/<input[^>]*value="-2.5"/);
    expect(user).toMatch(/<input[^>]*placeholder="max"[^>]*value=""|<input[^>]*value=""[^>]*placeholder="max"/);
    expect(user).not.toContain("aria-invalid");
  });
});

function openingTag(markup: string, testId: string): string {
  const at = markup.indexOf(`data-testid="${testId}"`);
  expect(at).toBeGreaterThan(-1);
  return markup.slice(markup.lastIndexOf("<", at), markup.indexOf(">", at) + 1);
}

const LINEAR_OFF =
  "Linear: show the data linearly from its minimum to its maximum (stretch linear, limits minmax, STF reset). Click again to go back";
const SAME_GROUP_TITLE =
  "Same display for frames of one instrument + filter: a file without its own display takes the last display set on a file with the same INSTRUME and filter (PUPIL when it names a filter). Off: such files start from the default display";
const SET_DEFAULT_TITLE =
  "Start files without their own display (and without a matching instrument + filter) from this stretch, limits, colormap, invert and symmetric setting; user lo/hi limits are not carried over";
const RESET_TITLE = "Reset this file&#x27;s display (stretch, limits, colormap, invert, symmetric, STF) and the colorbar, grid and compass";

describe("DisplayControls Linear and per-file display", () => {
  it("Linear is the first control and is outside the disabled fieldsets on the CPU viewer", () => {
    const html = render({ renderOnlyDisabled: true });
    expect(html.slice(0, html.indexOf(">") + 1)).toContain('data-testid="display-controls"');
    const tag = openingTag(html, "display-linear");
    const start = html.indexOf(tag);
    expect(html.slice(0, start)).not.toMatch(/<(select|input|button|label)\b/);
    expect(start).toBeLessThan(html.indexOf('<fieldset disabled=""'));
    expect(insideDisabledFieldset(html, 'data-testid="display-linear"')).toBe(false);
    expect(tag).toContain('type="button"');
    expect(tag).toContain('aria-pressed="false"');
    expect(tag).not.toContain('disabled=""');
    expect(tag).toContain(`title="${LINEAR_OFF}"`);
    expect(html.slice(start + tag.length)).toMatch(/^Linear<\/button>/);
  });

  it('Linear shows aria-pressed="true" and "Back to asinh with zscale limits" when Linear is on', () => {
    const on = openingTag(render({}, { stretch: "linear", limits: "minmax" }, { linear: { stretch: "asinh", limits: "zscale" } }), "display-linear");
    expect(on).toContain('aria-pressed="true"');
    expect(on).toContain('title="Back to asinh with zscale limits"');
    const fromMtf = openingTag(render({}, { stretch: "linear", limits: "minmax" }, { linear: { stretch: "mtf", limits: "minmax" } }), "display-linear");
    expect(fromMtf).toContain('title="Back to mtf"');
  });

  it("styles Linear as a display toggle with no colour utilities of its own", () => {
    for (const linear of [null, { stretch: "asinh", limits: "zscale" } as LinearUndo]) {
      expect(openingTag(render({}, {}, { linear }), "display-linear")).toContain('class="ab-display-toggle disabled:opacity-40"');
    }
  });

  it("Linear is disabled with the reason the context gives", () => {
    const hinted = openingTag(render({}, {}, { linearBlocked: LINEAR_HINT_TITLE }), "display-linear");
    expect(hinted).toContain('disabled=""');
    expect(hinted).toContain(`title="${LINEAR_HINT_TITLE}"`);
    const png = openingTag(render({ renderOnlyDisabled: true, renderOnlyTitle: PNG_ONLY_TITLE }, {}, { linearBlocked: PNG_ONLY_TITLE }), "display-linear");
    expect(png).toContain('disabled=""');
    expect(png).toContain(`title="${PNG_ONLY_TITLE}"`);
  });

  it("same filter reflects sameGroup; Set as default and Reset display carry their test ids and titles", () => {
    const on = render({}, {}, { sameGroup: true });
    expect(openingTag(on, "display-same-group")).toContain('aria-pressed="true"');
    expect(openingTag(render({}, {}, { sameGroup: false }), "display-same-group")).toContain('aria-pressed="false"');
    const label = on.slice(on.lastIndexOf("<button", on.indexOf('data-testid="display-same-group"')), on.indexOf("</button>", on.indexOf('data-testid="display-same-group"')));
    expect(label).toContain(`title="${SAME_GROUP_TITLE}"`);
    expect(label).toContain(">same filter</span>");
    expect(openingTag(on, "display-set-default")).toContain(`title="${SET_DEFAULT_TITLE}"`);
    expect(on).toMatch(/data-testid="display-set-default"[^>]*>Set as default<\/button>/);
    expect(openingTag(on, "display-reset")).toContain(`title="${RESET_TITLE}"`);
    expect(on).toMatch(/data-testid="display-reset"[^>]*>(<svg.*?<\/svg>)?Reset display<\/button>/);
    const order = [
      'aria-label="Display limits"',
      "resolved display limits",
      'aria-label="Color map"',
      'data-testid="display-same-group"',
      'data-testid="display-set-default"',
      'data-testid="display-reset"',
    ].map((t) => on.indexOf(t));
    expect(order.every((at, i) => at > -1 && (i === 0 || at > order[i - 1]))).toBe(true);
    const cpu = render({ renderOnlyDisabled: true });
    for (const id of ["display-same-group", "display-set-default", "display-reset"]) {
      expect(insideDisabledFieldset(cpu, `data-testid="${id}"`)).toBe(false);
    }
  });
});


interface ToggleProps {
  "data-testid"?: string;
  "aria-pressed"?: boolean;
  onClick?: () => void;
}

function toggleButton(testId: string): ToggleProps {
  const component = DisplayControls as unknown as { type: (props: { vmin: number; vmax: number }) => ReactNode };
  const tree = hooks.render(component.type, { vmin: 0, vmax: 1 });
  const found = findElement<ToggleProps>(tree, (element) => element.type === "button" && element.props["data-testid"] === testId);
  if (!found) throw new Error(`Missing toggle ${testId}`);
  return found.props;
}

describe("DisplayControls compact toggles", () => {
  beforeEach(() => {
    displayState.display = DEFAULT_DISPLAY_SETTINGS;
    displayState.memory = NO_MEMORY;
  });

  afterEach(() => {
    hooks.mount();
    vi.clearAllMocks();
  });

  it.each(["symmetric", "invert", "colorbar", "grid", "compass"] as const)("toggles %s on and off with the existing display setting", (key) => {
    hooks.mount();
    displayState.display = { ...DEFAULT_DISPLAY_SETTINGS, [key]: false };
    const off = toggleButton(`display-${key}`);
    expect(off["aria-pressed"]).toBe(false);
    off.onClick?.();
    expect(setDisplay).toHaveBeenLastCalledWith({ [key]: true });
    const on = toggleButton(`display-${key}`);
    expect(on["aria-pressed"]).toBe(true);
    on.onClick?.();
    expect(setDisplay).toHaveBeenLastCalledWith({ [key]: false });
    expect(toggleButton(`display-${key}`)["aria-pressed"]).toBe(false);
  });

  it("toggles shared filter display on and off through its existing memory callback", () => {
    hooks.mount();
    displayState.memory = { ...NO_MEMORY, sameGroup: false };
    toggleButton("display-same-group").onClick?.();
    expect(setSameGroup).toHaveBeenLastCalledWith(true);
    expect(toggleButton("display-same-group")["aria-pressed"]).toBe(true);
    toggleButton("display-same-group").onClick?.();
    expect(setSameGroup).toHaveBeenLastCalledWith(false);
    expect(toggleButton("display-same-group")["aria-pressed"]).toBe(false);
  });

  it("exposes every toggle directly without expanding a separate options card", () => {
    const html = render({});
    expect(html).not.toContain("<details");
    for (const id of ["symmetric", "invert", "colorbar", "grid", "compass", "same-group"]) {
      expect(openingTag(html, `display-${id}`)).toContain('<button type="button"');
    }
  });
});
