import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DEFAULT_DISPLAY_SETTINGS, type DisplaySettings } from "../../../shared/types/display";
import { wcsOverlayStatus } from "../../../utils/wcsOverlayStatus";

const displayState: { display: DisplaySettings } = { display: DEFAULT_DISPLAY_SETTINGS };

vi.mock("../../../context/PreviewContext", () => ({
  useDisplayContext: () => ({
    display: displayState.display,
    setDisplay: () => {},
    limits: null,
    limitsLoading: false,
    limitsError: null,
  }),
}));

import DisplayControls from "../DisplayControls";

function render(props: { renderOnlyDisabled?: boolean; renderOnlyTitle?: string }, display: Partial<DisplaySettings> = {}): string {
  displayState.display = { ...DEFAULT_DISPLAY_SETTINGS, ...display };
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
