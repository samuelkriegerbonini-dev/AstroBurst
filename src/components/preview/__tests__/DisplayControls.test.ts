import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DEFAULT_DISPLAY_SETTINGS, type DisplaySettings } from "../../../shared/types/display";
import { wcsOverlayStatus } from "../../../utils/wcsOverlayStatus";
import { LINEAR_HINT_TITLE, type LinearUndo } from "../../../utils/displayMemory";
import { PNG_ONLY_TITLE } from "../../../utils/previewShell";

interface MemoryState {
  linear: LinearUndo | null;
  linearBlocked: string | null;
  sameGroup: boolean;
}

const NO_MEMORY: MemoryState = { linear: null, linearBlocked: null, sameGroup: true };

const displayState: { display: DisplaySettings; memory: MemoryState } = { display: DEFAULT_DISPLAY_SETTINGS, memory: NO_MEMORY };

vi.mock("../../../context/PreviewContext", () => ({
  useDisplayContext: () => ({
    display: displayState.display,
    setDisplay: () => {},
    limits: null,
    limitsLoading: false,
    limitsError: null,
    linear: displayState.memory.linear,
    linearBlocked: displayState.memory.linearBlocked,
    toggleLinear: () => {},
    resetDisplay: () => {},
    setAsDefault: () => {},
    sameGroup: displayState.memory.sameGroup,
    setSameGroup: () => {},
  }),
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

describe("DisplayControls without GPU rendering", () => {
  afterEach(() => {
    wcsOverlayStatus.set("grid", null);
    wcsOverlayStatus.set("compass", null);
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
});

function colorbarCheckbox(markup: string): string {
  const at = markup.indexOf(">colorbar<");
  expect(at).toBeGreaterThan(-1);
  return markup.slice(markup.lastIndexOf("<label", at), at);
}

describe("DisplayControls colorbar toggle", () => {
  it("is on by default", () => {
    expect(DEFAULT_DISPLAY_SETTINGS.colorbar).toBe(true);
    expect(colorbarCheckbox(render({}))).toContain('checked=""');
  });

  it("reflects a colorbar switched off", () => {
    expect(colorbarCheckbox(render({}, { colorbar: false }))).not.toContain('checked=""');
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
    expect(openingTag(on, "display-same-group")).toContain('checked=""');
    expect(openingTag(render({}, {}, { sameGroup: false }), "display-same-group")).not.toContain("checked");
    const label = on.slice(on.lastIndexOf("<label", on.indexOf('data-testid="display-same-group"')), on.indexOf("</label>", on.indexOf('data-testid="display-same-group"')));
    expect(label).toContain(`title="${SAME_GROUP_TITLE}"`);
    expect(label).toContain(">same filter</span>");
    expect(openingTag(on, "display-set-default")).toContain(`title="${SET_DEFAULT_TITLE}"`);
    expect(on).toMatch(/data-testid="display-set-default"[^>]*>Set as default<\/button>/);
    expect(openingTag(on, "display-reset")).toContain(`title="${RESET_TITLE}"`);
    expect(on).toMatch(/data-testid="display-reset"[^>]*>(<svg.*?<\/svg>)?Reset display<\/button>/);
    const order = ["resolved display limits", 'data-testid="display-same-group"', 'data-testid="display-set-default"', 'data-testid="display-reset"'].map((t) => on.indexOf(t));
    expect(order.every((at, i) => at > -1 && (i === 0 || at > order[i - 1]))).toBe(true);
    const cpu = render({ renderOnlyDisabled: true });
    for (const id of ["display-same-group", "display-set-default", "display-reset"]) {
      expect(insideDisabledFieldset(cpu, `data-testid="${id}"`)).toBe(false);
    }
  });
});
