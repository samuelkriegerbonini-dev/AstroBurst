import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, createRef, isValidElement, type ComponentProps, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { hooks } from "../../test/hookRuntime";
import { CompositeProvider } from "../../context/CompositeContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../context/PreviewContext";
import GpuViewport, { type ViewportOriginal } from "../render/GpuViewport";
import AdvancedImageViewer from "../viewer/AdvancedImageViewer";
import type { ProcessedFile } from "../../shared/types";
import type { ProcessedResult } from "../../shared/types/preview";
import { EMPTY_CHAIN } from "../../utils/processingChain";
import { pipelineViewOutput } from "../../utils/stackingOutputs";
import { CANVAS_HINT_CLASS, CUBE_SPECTRUM_HINT, OTHER_GRID_CANVAS_HINT } from "../../utils/previewShell";
import { regionToolHint } from "../../utils/regionToolHints";
import { regionStore } from "../../utils/regionStore";

const viewerContext = vi.hoisted(() => ({ current: null as { file: ProcessedFile; processed: ProcessedResult; chain: typeof EMPTY_CHAIN } | null }));

vi.mock("react", async (original) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../context/PreviewContext", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../context/PreviewContext")>();
  return {
    ...actual,
    useFileContext: () => (viewerContext.current ? { file: viewerContext.current.file } : actual.useFileContext()),
    useRenderContext: () => (viewerContext.current ? { processed: viewerContext.current.processed, chain: viewerContext.current.chain } : actual.useRenderContext()),
  };
});

type GpuViewportProps = ComponentProps<typeof GpuViewport>;

function renderViewport(original: ViewportOriginal | null, fitsW = 5000, fitsH = 2812, label: string | null = null, extra: Partial<GpuViewportProps> = {}): string {
  const viewport = createElement(GpuViewport, {
    renderW: 1920,
    renderH: 1080,
    fitsW,
    fitsH,
    crosshairEnabled: true,
    original,
    label,
    children: null,
    ...extra,
  });
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: viewport });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

describe("GPU viewer toolbar", () => {
  it("has no Reset View button, since Fit to window does the same", () => {
    const html = renderViewport(null);
    expect(html).toContain('title="Fit to window"');
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

  it("names the icon-only zoom buttons with aria-labels", () => {
    const html = renderViewport(null);
    for (const name of ["Zoom in", "Zoom out", "Fit to window"]) expect(html).toContain(`aria-label="${name}"`);
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
    expect(html).toContain(">PNG-only result on another pixel grid: no readout, clicks or regions</span></div>");
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

type ToolbarProps = {
  children?: ReactNode;
  className?: string;
  title?: string;
  onClick?: () => void;
  onLoad?: (event: unknown) => void;
  onKeyDown?: unknown;
  tabIndex?: number;
  role?: string;
  style?: Record<string, unknown>;
  "aria-pressed"?: boolean;
  "aria-label"?: string;
  "aria-keyshortcuts"?: string;
};

function elementsOf(node: ReactNode, type: string): ToolbarProps[] {
  if (Array.isArray(node)) return node.flatMap((child) => elementsOf(child, type));
  if (!isValidElement<ToolbarProps>(node)) return [];
  return [...(node.type === type ? [node.props] : []), ...elementsOf(node.props.children, type)];
}

function cpuViewerProps(file: ProcessedFile, record: ProcessedResult) {
  const fileDims = file.result?.dimensions ?? FILE_DIMS;
  const shown = record.dimensions ?? fileDims;
  return {
    original: { url: `asset://${file.id}.png`, label: "Original", width: fileDims[0], height: fileDims[1] },
    processed: { url: record.previewUrl ?? "", label: record.label, width: shown[0], height: shown[1] },
  };
}

const cpuViewer = (AdvancedImageViewer as unknown as { type: (props: ReturnType<typeof cpuViewerProps>) => ReactNode }).type;

function loadedCpuViewer(file: ProcessedFile, record: ProcessedResult, natural: [number, number], extra: Partial<Omit<ComponentProps<typeof AdvancedImageViewer>, "original" | "processed">> = {}): () => ReactNode {
  viewerContext.current = { file, processed: record, chain: EMPTY_CHAIN };
  const props = { ...cpuViewerProps(file, record), ...extra };
  const view = () => hooks.render(cpuViewer, props) as ReactNode;
  for (const img of elementsOf(view(), "img")) img.onLoad?.({ currentTarget: { naturalWidth: natural[0], naturalHeight: natural[1] } });
  return view;
}

function textureBadgeOf(tree: ReactNode): ToolbarProps | undefined {
  return elementsOf(tree, "span").find((span) => span.className === "ab-viewer-preview-badge");
}

describe("CPU viewer toolbar", () => {
  beforeEach(() => {
    hooks.mount();
    vi.stubGlobal("requestAnimationFrame", vi.fn());
  });

  afterEach(() => {
    hooks.mount();
    viewerContext.current = null;
    vi.unstubAllGlobals();
  });

  it("explains the preview texture of a large PNG-only result without sending it to Deep Zoom", () => {
    const view = loadedCpuViewer(lightFile("big-drizzle", LIGHT_1), pngOnly({ dimensions: [8000, 6000], label: "Drizzle RGB · 6 frames" }), [1920, 1440]);
    const badge = textureBadgeOf(view());
    expect(badge?.children).toBe("preview 1920 px (4.2:1)");
    expect(badge?.title).toBe("The viewer shows a downsampled preview texture (1920×1440 of 8000×6000 FITS pixels).");
  });

  it("still points a large FITS-backed result to Deep Zoom", () => {
    const record: ProcessedResult = { ...pngOnly({ dimensions: [8000, 6000], kind: "processing" }), fitsPath: "C:/out/result.fits" };
    const view = loadedCpuViewer(lightFile("big-fits", LIGHT_1, [8000, 6000]), record, [1920, 1440]);
    expect(textureBadgeOf(view())?.title).toContain("For full-resolution pixels open Image > Deep Zoom.");
  });

  it("reports the status bar toggle state through aria-pressed", () => {
    const view = loadedCpuViewer(lightFile("eye", LIGHT_1), pngOnly({ kind: "debayer", label: "Debayer" }), [1024, 768]);
    const eye = () => elementsOf(view(), "button").find((button) => button.title === "Toggle status bar");
    expect(eye()?.["aria-pressed"]).toBe(true);
    eye()?.onClick?.();
    expect(eye()?.["aria-pressed"]).toBe(false);
  });

  it("names the icon-only zoom buttons with aria-labels", () => {
    const html = renderCpuViewer(lightFile("zoom-names", LIGHT_1), pngOnly({ kind: "debayer", label: "Debayer" }));
    for (const name of ["Zoom in", "Zoom out", "Fit to window"]) expect(html).toContain(`title="${name}" aria-label="${name}"`);
  });
});

function hintChipMarkup(html: string): string[] {
  const chips: string[] = [];
  for (let at = html.indexOf(`class="${CANVAS_HINT_CLASS}`); at > -1; at = html.indexOf(`class="${CANVAS_HINT_CLASS}`, at + 1)) {
    chips.push(divMarkupWithAttribute(html.slice(html.lastIndexOf("<div", at)), `class="${CANVAS_HINT_CLASS}`));
  }
  return chips;
}

function hintLines(chip: string): string[] {
  return [...chip.matchAll(/<span[^>]*>([^<]*)<\/span>/g)].map((match) => match[1]);
}

describe("GPU viewer canvas hint chip", () => {
  afterEach(() => regionStore.setTool("none"));

  it("stacks the active region tool's hint under the cube hint in one bottom-right chip of the canvas", () => {
    regionStore.setTool("polygon");
    const html = renderViewport(null, 1920, 1080, null, { regionsEnabled: true, canvasHint: CUBE_SPECTRUM_HINT });
    expect(divMarkupWithAttribute(html, 'class="ab-viewer-canvas"')).toContain(`class="${CANVAS_HINT_CLASS}`);
    const chips = hintChipMarkup(html);
    expect(chips).toHaveLength(1);
    expect(hintLines(chips[0])).toEqual([CUBE_SPECTRUM_HINT, regionToolHint("polygon")]);
  });

  it("keeps the cube hint alone while no region tool is active", () => {
    const chips = hintChipMarkup(renderViewport(null, 1920, 1080, null, { regionsEnabled: true, canvasHint: CUBE_SPECTRUM_HINT }));
    expect(chips.map(hintLines)).toEqual([[CUBE_SPECTRUM_HINT]]);
  });

  it("shows the region tool hint alone when the view has no cube hint", () => {
    regionStore.setTool("select");
    const chips = hintChipMarkup(renderViewport(null, 1920, 1080, null, { regionsEnabled: true }));
    expect(chips.map(hintLines)).toEqual([[regionToolHint("select")]]);
  });

  it("gives no region hint while regions are off for the view, and no chip without any hint", () => {
    regionStore.setTool("polygon");
    expect(hintChipMarkup(renderViewport(null, 1920, 1080, null, { canvasHint: CUBE_SPECTRUM_HINT })).map(hintLines)).toEqual([[CUBE_SPECTRUM_HINT]]);
    expect(hintChipMarkup(renderViewport(null, 1920, 1080, null))).toEqual([]);
  });

  it("lets a long region hint wrap inside the canvas instead of being clipped, while the cube hint stays on one line", () => {
    regionStore.setTool("annulus");
    const chip = hintChipMarkup(renderViewport(null, 1920, 1080, null, { regionsEnabled: true, canvasHint: CUBE_SPECTRUM_HINT }))[0];
    const tag = /<div[^>]*>/.exec(chip)?.[0] ?? "";
    expect(tag).toContain("flex-col");
    expect(tag).toContain("max-w-[calc(100%-1rem)]");
    expect(chip).toContain(`<span>${CUBE_SPECTRUM_HINT}</span>`);
    expect(chip).toContain(`<span class="whitespace-normal">${regionToolHint("annulus")}</span>`);
  });
});

const gpuViewport = (GpuViewport as unknown as { type: (props: GpuViewportProps) => ReactNode }).type;

function canvasOf(tree: ReactNode): ToolbarProps | undefined {
  return elementsOf(tree, "div").find((div) => div.className === "ab-viewer-canvas");
}

describe("viewer canvas focus and region hint", () => {
  beforeEach(() => {
    hooks.mount();
    vi.stubGlobal("requestAnimationFrame", vi.fn());
  });

  afterEach(() => {
    hooks.mount();
    viewerContext.current = null;
    regionStore.setTool("none");
    vi.unstubAllGlobals();
  });

  it("keeps the GPU and CPU canvases out of the tab order and free of an inset focus outline, which the image layers would paint over", () => {
    const gpu = canvasOf(hooks.render(gpuViewport, { renderW: 64, renderH: 64, children: null }) as ReactNode);
    hooks.mount();
    const frame = pngOnly({ kind: "cube", dimensions: null, frameIndex: 4, label: "ch 4 (0–39)" });
    const cpu = canvasOf(loadedCpuViewer(lightFile("frame-canvas", LIGHT_1), frame, [1024, 768])());
    for (const canvas of [gpu, cpu]) {
      expect(canvas).toBeDefined();
      expect(canvas?.tabIndex).toBeUndefined();
      expect(canvas?.onKeyDown).toBeUndefined();
      expect(canvas?.style).not.toHaveProperty("outlineOffset");
    }
  });

  it("stacks the active region tool's hint under the cube hint on the CPU canvas", () => {
    regionStore.setTool("annulus");
    const view = loadedCpuViewer(lightFile("cpu-hint", LIGHT_1), pngOnly({ kind: "debayer", label: "Debayer" }), [1024, 768], { canvasHint: CUBE_SPECTRUM_HINT });
    const chips = elementsOf(canvasOf(view())?.children, "div").filter((div) => div.className?.startsWith(CANVAS_HINT_CLASS));
    expect(chips).toHaveLength(1);
    expect(elementsOf(chips[0].children, "span").map((span) => span.children)).toEqual([CUBE_SPECTRUM_HINT, regionToolHint("annulus")]);
  });

  it("gives no region hint over a PNG-only result on another grid, where regions are off", () => {
    regionStore.setTool("annulus");
    const view = loadedCpuViewer(lightFile("cpu-other-grid", LIGHT_1), pngOnly({ dimensions: [2048, 1536] }), [2048, 1536]);
    const chips = elementsOf(canvasOf(view())?.children, "div").filter((div) => div.className?.startsWith(CANVAS_HINT_CLASS));
    expect(chips.map((chip) => elementsOf(chip.children, "span").map((span) => span.children))).toEqual([[OTHER_GRID_CANVAS_HINT]]);
  });
});
