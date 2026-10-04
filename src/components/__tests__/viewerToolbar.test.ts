import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../../context/CompositeContext";
import { PreviewProvider } from "../../context/PreviewContext";
import GpuViewport, { type ViewportOriginal } from "../render/GpuViewport";

function renderViewport(original: ViewportOriginal | null, fitsW = 5000, fitsH = 2812, label: string | null = null): string {
  const viewport = createElement(GpuViewport, {
    renderW: 1920,
    renderH: 1080,
    fitsW,
    fitsH,
    crosshairEnabled: true,
    original,
    label,
    children: null,
  });
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: viewport });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

describe("GPU viewer toolbar", () => {
  it("has no Reset View button, since Fit to Window does the same", () => {
    const html = renderViewport(null);
    expect(html).toContain('title="Fit to Window"');
    expect(html).not.toContain("Reset View");
  });

  it("shows Pan and Crosshair as two always-visible buttons with Pan pressed by default", () => {
    const html = renderViewport(null);
    expect(html).toMatch(/title="Pan" aria-pressed="true"/);
    expect(html).toMatch(/title="Crosshair" aria-pressed="false"/);
  });

  it("labels 1:1 as text so it differs from the box region icon", () => {
    expect(renderViewport(null)).toMatch(/title="1:1 Pixel[^"]*">1:1<\/button>/);
  });

  it("puts the zoom readout and the preview badge before the zoom presets", () => {
    const html = renderViewport(null);
    const readout = html.indexOf("ab-viewer-zoom-readout");
    const badge = html.indexOf("preview 1920 px (2.6:1)");
    const firstPreset = html.indexOf("ab-viewer-zoom-preset");
    expect(readout).toBeGreaterThan(-1);
    expect(badge).toBeGreaterThan(readout);
    expect(firstPreset).toBeGreaterThan(badge);
  });

  it("has no preview badge when the texture holds every FITS pixel", () => {
    expect(renderViewport(null, 1920, 1080)).not.toContain("ab-viewer-preview-badge");
  });

  it("points the preview badge to Deep Zoom only when the image has a side above the Deep Zoom threshold", () => {
    expect(renderViewport(null, 5000, 2812)).toMatch(/class="ab-viewer-preview-badge" title="[^"]*open Image &gt; Deep Zoom\."/);
    const belowThreshold = renderViewport(null, 3000, 1688);
    expect(belowThreshold).toMatch(/class="ab-viewer-preview-badge" title="The viewer shows a downsampled preview texture \(1920×1080 of 3000×1688 FITS pixels\)\."/);
    expect(belowThreshold).not.toContain("Deep Zoom");
  });

  it("offers a hold-to-show Original button only when an original is passed", () => {
    expect(renderViewport(null)).not.toContain(">Original</button>");
    const html = renderViewport({ url: "asset://original.png", disabledReason: null });
    expect(html).toMatch(/title="Hold to show the original[^"]*" aria-pressed="false"/);
    expect(html).toContain('src="asset://original.png"');
  });

  it("disables the Original button with the reason when the grids differ", () => {
    const html = renderViewport({ url: "asset://original.png", disabledReason: "different grid" });
    expect(html).toMatch(/disabled="" title="different grid"/);
    expect(html).not.toContain('src="asset://original.png"');
  });
});

function classesOfElementWithText(html: string, text: string): string[] {
  const at = html.indexOf(`>${text}</div>`);
  expect(at).toBeGreaterThan(-1);
  const open = html.lastIndexOf("<div", at);
  return (/class="([^"]*)"/.exec(html.slice(open, at))?.[1] ?? "").split(" ");
}

function divMarkupWithAttribute(html: string, attribute: string): string {
  const at = html.indexOf(attribute);
  expect(at).toBeGreaterThan(-1);
  const start = html.lastIndexOf("<div", at);
  const tag = /<div[\s>]|<\/div>/g;
  tag.lastIndex = start;
  let depth = 0;
  for (let match = tag.exec(html); match; match = tag.exec(html)) {
    depth += match[0] === "</div>" ? -1 : 1;
    if (depth === 0) return html.slice(start, tag.lastIndex);
  }
  return html.slice(start);
}

describe("GPU viewer processed label", () => {
  const label = "Moment m1 · 25 ch";

  it("shows the processed label at the top right of the image area, away from the compass and scale bar painted bottom left", () => {
    const html = renderViewport(null, 1920, 1080, label);
    expect(divMarkupWithAttribute(html, 'class="ab-viewer-canvas"')).toContain(`>${label}</div>`);
    const classes = classesOfElementWithText(html, label);
    expect(classes).toEqual(expect.arrayContaining(["absolute", "top-2", "right-2", "pointer-events-none"]));
    expect(classes.some((c) => c.startsWith("bottom-") || c.startsWith("left-"))).toBe(false);
  });

  it("keeps the processed label over the result while the Original button is not held", () => {
    const html = renderViewport({ url: "asset://original.png", disabledReason: null }, 1920, 1080, label);
    expect(html).toMatch(/title="Hold to show the original[^"]*" aria-pressed="false"/);
    expect(divMarkupWithAttribute(html, 'class="ab-viewer-canvas"')).toContain(`>${label}</div>`);
  });

  it("shows no processed label when none is passed", () => {
    expect(renderViewport(null, 1920, 1080)).not.toContain("Showing a processed result");
    expect(renderViewport(null, 1920, 1080, "")).not.toContain("Showing a processed result");
  });
});
