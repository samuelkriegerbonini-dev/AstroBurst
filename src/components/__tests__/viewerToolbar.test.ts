import { describe, expect, it } from "vitest";
import { createElement, createRef } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../../context/CompositeContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../context/PreviewContext";
import GpuViewport, { type ViewportOriginal } from "../render/GpuViewport";
import AdvancedImageViewer from "../viewer/AdvancedImageViewer";
import type { ProcessedFile } from "../../shared/types";
import type { ProcessedResult } from "../../shared/types/preview";
import { pipelineViewOutput } from "../../utils/stackingOutputs";

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

const LIGHT_1 = "C:/data/r_1.fits";
const LIGHT_2 = "C:/data/g_2.fits";
const FILE_DIMS: [number, number] = [1024, 768];

function lightFile(id: string, path: string, dimensions: [number, number] = FILE_DIMS): ProcessedFile {
  return {
    id,
    name: path.split("/").pop() ?? path,
    path,
    sourcePath: path,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: `${path}.png`, previewUrl: `asset://${id}.png`, dimensions, elapsed_ms: 1, is_rgb: false },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function publish(key: string, result: ProcessedResult): void {
  function Publisher() {
    useRenderActions().publishProcessed(key, result);
    return null;
  }
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function renderCpuViewer(file: ProcessedFile, record: ProcessedResult, withOverlays = false): string {
  publish(fileKeyOf(file) as string, record);
  const fileDims = file.result?.dimensions ?? FILE_DIMS;
  const shown = record.dimensions ?? fileDims;
  const viewer = createElement(AdvancedImageViewer, {
    original: { url: `asset://${file.id}.png`, label: "Original", width: fileDims[0], height: fileDims[1] },
    processed: { url: record.previewUrl ?? "", label: record.label, width: shown[0], height: shown[1] },
    ...(withOverlays ? { overlayCanvasRef: createRef<HTMLCanvasElement>(), dqCanvasRef: createRef<HTMLCanvasElement>() } : {}),
  });
  const preview = createElement(PreviewProvider, { file, doneFiles: [file], children: viewer });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function pngOnly(overrides: Partial<ProcessedResult>): ProcessedResult {
  return { fitsPath: null, previewUrl: "asset://out/result.png", dimensions: FILE_DIMS, label: "Result", kind: "stacking", inputPath: LIGHT_1, ...overrides };
}

describe("CPU viewer on a PNG-only result", () => {
  it("sizes a PNG-only result by its own size and says it is on another pixel grid", () => {
    const html = renderCpuViewer(lightFile("drizzle", LIGHT_1), pngOnly({ dimensions: [2048, 1536], label: "Drizzle RGB · 6 frames" }));
    expect(html).toContain(">2048 x 1536</span>");
    expect(html).toContain(">PNG-only result on another pixel grid: no readout, clicks or regions</div>");
  });

  it("never shows the file's size for a PNG-only result published without one", () => {
    const html = renderCpuViewer(lightFile("nodims", LIGHT_1), pngOnly({ dimensions: null, kind: "processing" }));
    expect(html).not.toContain("1024 x 768");
    expect(html).toContain("another pixel grid");
  });

  it("flags a PNG-only stack of the file's size built on another frame", () => {
    const html = renderCpuViewer(lightFile("other-frame", LIGHT_2), pngOnly({ label: "Pipeline RGB · 6 frames" }));
    expect(html).toContain(">1024 x 768</span>");
    expect(html).toContain("another pixel grid");
  });

  it("keeps a Debayer preview with the file's size on the file grid, without the hint", () => {
    const html = renderCpuViewer(lightFile("debayer", LIGHT_1), pngOnly({ kind: "debayer", label: "Debayer" }));
    expect(html).toContain(">1024 x 768</span>");
    expect(html).not.toContain("another pixel grid");
  });

  it("keeps the file's size and the readout on a cube frame shown before its FITS", () => {
    const frame = pngOnly({ kind: "cube", dimensions: null, frameIndex: 4, label: "Frame 5 / 40" });
    const html = renderCpuViewer(lightFile("cube-frame", LIGHT_1), frame);
    expect(html).toContain(">1024 x 768</span>");
    expect(html).not.toContain("another pixel grid");
  });

  it("shows a Pipeline RGB above the preview limit at the masters' size on the R light it was built on", () => {
    const full: [number, number] = [4144, 2822];
    const master = (label: string) => ({ label, png_path: `C:/out/${label}.png`, fits_path: `C:/out/${label}.fits`, dimensions: full, input_path: `C:/data/${label.toLowerCase()}_1.fits` });
    const view = pipelineViewOutput(
      {
        stats: { darks_combined: 0, flats_combined: 0, bias_combined: 0, channels: [] },
        channel_previews: [],
        rgb_preview: "AAAA",
        masters: [master("R"), master("G"), master("B")],
        rgb_png_path: "C:/out/rgb.png",
        rgb_dimensions: [1382, 941],
        rgbPreviewUrl: "asset://out/rgb.png",
      },
      "RGB",
    );
    expect(view).not.toBeNull();
    const html = renderCpuViewer(lightFile("pipeline-rgb", LIGHT_1, full), { ...view!.output, label: view!.label });
    expect(html).toContain(">4144 x 2822</span>");
    expect(html).not.toContain("1382 x 941");
    expect(html).not.toContain("another pixel grid");
  });

  it("hides the star and DQ overlays of the file over a PNG-only result on another grid", () => {
    const off = renderCpuViewer(lightFile("overlay-off", LIGHT_1), pngOnly({ dimensions: [2048, 1536] }), true);
    expect(off).toMatch(/visibility:hidden[^>]*><canvas/);
    const on = renderCpuViewer(lightFile("overlay-on", LIGHT_1), pngOnly({ kind: "debayer", label: "Debayer" }), true);
    expect(on).toContain("<canvas");
    expect(on).not.toContain("visibility:hidden");
  });
});
