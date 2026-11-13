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
import { pointerModeStore } from "../../utils/pointerModeStore";
import { forgetViewerView } from "../../hooks/useViewerTransform";

const viewerContext = vi.hoisted(() => ({ current: null as { file: ProcessedFile; processed: ProcessedResult | null; chain: typeof EMPTY_CHAIN } | null }));

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
  onPointerDown?: (event: unknown) => void;
  onPointerMove?: (event: unknown) => void;
  onPointerUp?: () => void;
  ref?: (element: unknown) => void;
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

interface CanvasSize {
  width: number;
  height: number;
}

const anyCpuViewer = (AdvancedImageViewer as unknown as { type: (props: ComponentProps<typeof AdvancedImageViewer>) => ReactNode }).type;

describe("viewer keeps the view", () => {
  const SIZE_800x600: CanvasSize = { width: 800, height: 600 };
  const SIZE_1000x800: CanvasSize = { width: 1000, height: 800 };
  let frames: (() => void)[] = [];

  beforeEach(() => {
    hooks.mount();
    frames = [];
    vi.stubGlobal("requestAnimationFrame", (frame: () => void) => frames.push(frame));
    vi.stubGlobal("cancelAnimationFrame", () => {});
    vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  });

  afterEach(() => {
    hooks.mount();
    viewerContext.current = null;
    pointerModeStore.set("pan");
    forgetViewerView();
    vi.unstubAllGlobals();
  });

  function flushFrames(): void {
    for (const frame of frames.splice(0)) frame();
  }

  function fakeContainer({ width, height }: CanvasSize) {
    return {
      clientWidth: width,
      clientHeight: height,
      getBoundingClientRect: () => ({ left: 0, top: 0, width, height }),
      addEventListener: () => {},
      removeEventListener: () => {},
    };
  }

  function loadImages(view: () => ReactNode, natural: number): void {
    for (const img of elementsOf(view(), "img")) img.onLoad?.({ currentTarget: { naturalWidth: natural, naturalHeight: natural } });
    view();
  }

  function mountGpu(view: () => ReactNode, size: CanvasSize): void {
    canvasOf(view())?.ref?.(fakeContainer(size));
    view();
    flushFrames();
  }

  function mountCpu(view: () => ReactNode, size: CanvasSize, natural: number): void {
    canvasOf(view())?.ref?.(fakeContainer(size));
    loadImages(view, natural);
    flushFrames();
  }

  function click(tree: ReactNode, match: (button: ToolbarProps) => boolean): void {
    elementsOf(tree, "button").find(match)?.onClick?.();
  }

  function dragWithMiddleButton(view: () => ReactNode, dx: number, dy: number): void {
    canvasOf(view())?.onPointerDown?.({ button: 1, clientX: 400, clientY: 300, pointerId: 1, currentTarget: { setPointerCapture: () => {} } });
    canvasOf(view())?.onPointerMove?.({ clientX: 400 + dx, clientY: 300 + dy });
    canvasOf(view())?.onPointerUp?.();
    view();
  }

  const isPreset = (label: string) => (button: ToolbarProps) => button.children === label;
  const isCrosshair = (button: ToolbarProps) => button.title === "Crosshair";
  const crosshairPressed = (tree: ReactNode) => elementsOf(tree, "button").find(isCrosshair)?.["aria-pressed"];
  const gpuZoom = (tree: ReactNode) => elementsOf(tree, "span").find((span) => span.className?.includes("ab-viewer-zoom-readout"))?.children;
  const cpuZoom = (tree: ReactNode) => elementsOf(tree, "span").find((span) => span.className === "ab-viewer-status-item")?.children;
  const layerTransform = (tree: ReactNode) => elementsOf(tree, "div").find((div) => typeof div.style?.transform === "string")?.style?.transform as string | undefined;

  function fitsCentre(tree: ReactNode, size: CanvasSize, fitsPerRender: number): [number, number] {
    const [x, y, scale] = (/translate\((\S+)px, (\S+)px\) scale\((\S+)\)/.exec(layerTransform(tree) ?? "") ?? []).slice(1).map(Number);
    return [((size.width / 2 - x) / scale) * fitsPerRender, ((size.height / 2 - y) / scale) * fitsPerRender];
  }

  it("keeps 4x and Crosshair in the GPU viewer across the remount that a cube frame step or a file switch causes", () => {
    const props: GpuViewportProps = { renderW: 2048, renderH: 2048, fitsW: 4096, fitsH: 4096, crosshairEnabled: true, children: null };
    const view = () => hooks.render(gpuViewport, props) as ReactNode;
    mountGpu(view, SIZE_800x600);
    expect(gpuZoom(view())).toBe("15%");
    click(view(), isPreset("4x"));
    click(view(), isCrosshair);
    expect({ zoom: gpuZoom(view()), crosshair: crosshairPressed(view()) }).toEqual({ zoom: "400%", crosshair: true });
    hooks.mount();
    mountGpu(view, SIZE_800x600);
    expect({ zoom: gpuZoom(view()), crosshair: crosshairPressed(view()) }).toEqual({ zoom: "400%", crosshair: true });
  });

  it("keeps the FITS zoom, the centre and Crosshair while the CPU viewer stands in for a PNG-first cube frame and the GPU viewer comes back", () => {
    const gpuProps: GpuViewportProps = { renderW: 2048, renderH: 2048, fitsW: 4096, fitsH: 4096, crosshairEnabled: true, children: null };
    const gpu = () => hooks.render(gpuViewport, gpuProps) as ReactNode;
    mountGpu(gpu, SIZE_800x600);
    click(gpu(), isPreset("4x"));
    click(gpu(), isCrosshair);
    dragWithMiddleButton(gpu, 800, 400);
    expect(fitsCentre(gpu(), SIZE_800x600, 2)).toEqual([1848, 1948]);
    hooks.mount();
    const file = lightFile("cube-step", LIGHT_1, [4096, 4096]);
    const frame = pngOnly({ kind: "cube", dimensions: null, frameIndex: 5, label: "Frame 6 / 40", previewUrl: "asset://out/frame6.png" });
    viewerContext.current = { file, processed: frame, chain: EMPTY_CHAIN };
    const cpu = () => hooks.render(cpuViewer, cpuViewerProps(file, frame)) as ReactNode;
    mountCpu(cpu, SIZE_800x600, 1024);
    expect({ zoom: cpuZoom(cpu()), crosshair: crosshairPressed(cpu()), centre: fitsCentre(cpu(), SIZE_800x600, 4) }).toEqual({ zoom: "400%", crosshair: true, centre: [1848, 1948] });
    hooks.mount();
    mountGpu(gpu, SIZE_800x600);
    expect({ zoom: gpuZoom(gpu()), crosshair: crosshairPressed(gpu()), centre: fitsCentre(gpu(), SIZE_800x600, 2) }).toEqual({ zoom: "400%", crosshair: true, centre: [1848, 1948] });
  });

  it("keeps 400% in the CPU viewer when a stretch change re-renders the same grid under a new URL", () => {
    viewerContext.current = { file: lightFile("stf", LIGHT_1, [4096, 4096]), processed: null, chain: EMPTY_CHAIN };
    const stretched = (t: number) => ({ url: `asset://stf.png?t=${t}`, label: "Original · STF", width: 4096, height: 4096 });
    const props = { original: { url: "asset://stf.png?_v=stf", label: "Original", width: 4096, height: 4096 }, processed: stretched(1) };
    const view = () => hooks.render(anyCpuViewer, props) as ReactNode;
    mountCpu(view, SIZE_800x600, 2048);
    expect(cpuZoom(view())).toBe("15%");
    click(view(), isPreset("4x"));
    expect(cpuZoom(view())).toBe("400%");
    props.processed = stretched(2);
    loadImages(view, 2048);
    flushFrames();
    expect(cpuZoom(view())).toBe("400%");
  });

  it("keeps the 4x view and its centre in the CPU viewer when the next file has the same 512 x 512 grid", () => {
    const band = (name: string) => ({ url: `asset://legacy_${name}.png?_v=${name}`, label: "Original", width: 512, height: 512 });
    viewerContext.current = { file: lightFile("g", "C:/data/legacy_g.fits", [512, 512]), processed: null, chain: EMPTY_CHAIN };
    const props = { original: band("g"), processed: null };
    const view = () => hooks.render(anyCpuViewer, props) as ReactNode;
    mountCpu(view, SIZE_1000x800, 512);
    expect(layerTransform(view())).toBe("translate(100px, 0px) scale(1.5625)");
    click(view(), isPreset("4x"));
    expect(layerTransform(view())).toBe("translate(-524px, -624px) scale(4)");
    viewerContext.current = { file: lightFile("r", "C:/data/legacy_r.fits", [512, 512]), processed: null, chain: EMPTY_CHAIN };
    props.original = band("r");
    loadImages(view, 512);
    flushFrames();
    expect(layerTransform(view())).toBe("translate(-524px, -624px) scale(4)");
  });

  it("fits the CPU viewer to the window when the next file has another grid", () => {
    viewerContext.current = { file: lightFile("g", "C:/data/legacy_g.fits", [512, 512]), processed: null, chain: EMPTY_CHAIN };
    const props = { original: { url: "asset://legacy_g.png?_v=g", label: "Original", width: 512, height: 512 }, processed: null };
    const view = () => hooks.render(anyCpuViewer, props) as ReactNode;
    mountCpu(view, SIZE_1000x800, 512);
    click(view(), isPreset("4x"));
    viewerContext.current = { file: lightFile("wide", "C:/data/wide.fits", [1024, 1024]), processed: null, chain: EMPTY_CHAIN };
    props.original = { url: "asset://wide.png?_v=wide", label: "Original", width: 1024, height: 1024 };
    loadImages(view, 1024);
    flushFrames();
    expect(layerTransform(view())).toBe("translate(100px, 0px) scale(0.78125)");
  });

  it("pans with the left button in a GPU viewer that has no pointer buttons, whatever mode another viewer picked", () => {
    pointerModeStore.set("crosshair");
    const view = () => hooks.render(gpuViewport, { renderW: 512, renderH: 512, crosshairEnabled: false, children: null }) as ReactNode;
    mountGpu(view, SIZE_800x600);
    expect(elementsOf(view(), "button").find(isCrosshair)).toBeUndefined();
    expect(canvasOf(view())?.style?.cursor).toBe("grab");
  });
});
