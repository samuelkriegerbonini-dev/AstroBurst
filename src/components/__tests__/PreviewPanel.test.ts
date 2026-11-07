import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CUBE_SPECTRUM_HINT, PNG_ONLY_TITLE } from "../../utils/previewShell";
import { dockStore } from "../../hooks/useDockLayout";
import { DEFAULT_DOCK_LAYOUT, anchorOf, dockReducer } from "../../utils/dockLayout";
import { middleEllipsis } from "../../utils/filenameDisplay";

const previewLayoutCss = readFileSync(fileURLToPath(new URL("../preview/preview-layout.css", import.meta.url)), "utf8");

interface ViewState {
  label: string | null;
  isCube: boolean;
  isSpectralCube: boolean;
  fileDims: [number, number];
  displayedDims: [number, number];
  gpu: boolean;
  spectrum: boolean;
  reloads?: number;
  finishedAt?: number;
  imageRef?: string;
  fileHeader?: Record<string, string>;
  filePath?: string;
  fileName?: string;
  productPath?: string | null;
  productVersion?: number;
  previewOnly?: boolean;
  compositeUrl?: string | null;
  cubeFrame?: number;
}

const S3D_GRID: [number, number] = [53, 55];
const UNCAL_GRID: [number, number] = [2048, 3200];
const X1D_NAME = "jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits";
const X1D_PATH = `/data/${X1D_NAME}`;

const state: ViewState = { label: null, isCube: true, isSpectralCube: true, fileDims: S3D_GRID, displayedDims: S3D_GRID, gpu: false, spectrum: false };

const getHeaderMock = vi.hoisted(() => vi.fn<() => Promise<Record<string, string>>>());
vi.mock("../../services/header", () => ({ getHeader: getHeaderMock }));

vi.mock("react", async (original) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await original()));

const { hostProps, captureHost, revertMock, resetProcessedMock } = vi.hoisted(() => {
  const hostProps = new Map<string, Record<string, unknown>>();
  type Factory = (type: unknown, props: Record<string, unknown> | null, ...rest: unknown[]) => unknown;
  const captureHost = (factory: Factory): Factory => (type, props, ...rest) => {
    const id = props?.["data-testid"];
    if (typeof type === "string" && typeof id === "string") hostProps.set(id, props as Record<string, unknown>);
    return factory(type, props, ...rest);
  };
  return { hostProps, captureHost, revertMock: vi.fn(async () => {}), resetProcessedMock: vi.fn() };
});

vi.mock("react/jsx-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-runtime")>();
  return { ...actual, jsx: captureHost(actual.jsx as never), jsxs: captureHost(actual.jsxs as never) };
});

vi.mock("react/jsx-dev-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-dev-runtime")>();
  return { ...actual, jsxDEV: captureHost(actual.jsxDEV as never) };
});

vi.mock("../../hooks/useHistory", () => ({ useHistoryController: () => ({ revert: revertMock }) }));

vi.mock("../../context/PreviewContext", () => ({
  fileKeyOf: (file: { id: string; path: string } | null) => (file ? `${file.id}|${file.path}` : null),
  useFileContext: () => ({
    file: state.spectrum
      ? { id: "spec", path: X1D_PATH, name: X1D_NAME, status: "spectrum", spectrum: { kind: "x1d", hdu: 1 }, result: null }
      : {
          id: "1",
          path: state.filePath ?? "/data/cube_s3d.fits",
          name: state.fileName ?? "cube_s3d.fits",
          reloads: state.reloads,
          finishedAt: state.finishedAt,
          imageRef: state.imageRef,
          result: { dimensions: state.fileDims, previewUrl: "asset://cube.png", header: state.fileHeader ?? {}, elapsed_ms: 10 },
        },
  }),
  useCubeContext: () => ({ isCube: state.isCube, isSpectralCube: state.isSpectralCube, isSpectrum: state.spectrum, ramp: null }),
  useDisplayedImage: () => ({
    path: state.spectrum ? null : state.label && state.productPath !== null ? state.productPath ?? "/out/result.fits" : state.filePath ?? "/data/cube_s3d.fits",
    dimensions: state.spectrum ? null : state.displayedDims,
    isProcessed: state.label !== null,
    label: state.label,
    previewOnly: state.previewOnly ?? false,
    previewUrl: state.spectrum ? null : "asset://shown.png",
  }),
  useRawPixelsContext: () => ({
    rawPixels: state.gpu ? ({ width: state.displayedDims[0], height: state.displayedDims[1] } as unknown) : null,
    rawPixelsLoading: false,
    rawPixelsError: null,
    loadRawPixels: () => {},
    clearRawPixels: () => {},
    rgbRawPixels: null,
    rgbRawPixelsLoading: false,
    loadRgbRawPixels: () => {},
    clearRgbRawPixels: () => {},
  }),
  useRenderContext: () => ({
    processed: state.label
      ? {
          label: state.label,
          previewUrl: "asset://result.png",
          fitsPath: state.productPath === undefined ? "/out/result.fits" : state.productPath,
          dimensions: state.displayedDims,
          ...(state.cubeFrame === undefined ? {} : { kind: "cube", inputPath: state.filePath ?? "/data/cube_s3d.fits", frameIndex: state.cubeFrame }),
        }
      : null,
    processedVersion: state.productVersion ?? 0,
    stfPreviewUrl: null,
    chain: { psfKernel: null },
  }),
  useRenderActions: () => ({ resetProcessed: resetProcessedMock, currentFileKey: () => null, setStfPreviewUrl: () => {} }),
  useHistContext: () => ({ histData: null, histDataPath: null, stfParams: { shadow: 0, midtone: 0.5, highlight: 1 } }),
  seedRgbFileView: () => {},
  useStarOverlayContext: () => ({ starOverlayRef: { current: null } }),
}));
vi.mock("../../context/CompositeContext", () => ({
  useCompositePreview: () => ({ compositePreviewUrl: state.compositeUrl ?? null, compositeVersion: 0, canShowParked: false }),
  useCompositeActions: () => ({
    initRgb: () => {},
    setCompositePreviewUrl: () => {},
    clearComposite: () => {},
    resetComposite: () => {},
    setCompositeStf: () => {},
    setCompositeAutoStf: () => {},
    setCompositeStfLinked: () => {},
    park: () => {},
    replaceParked: () => {},
    showParkedComposite: () => {},
  }),
}));
vi.mock("../../context/ComposeWizardContext", () => ({ useComposeWizardContext: () => ({ state: { compositeReady: false } }) }));
vi.mock("../../hooks/useMousePixelStore", () => ({
  useMousePixelActions: () => ({ handleLeave: () => {}, reset: () => {} }),
  setMousePixel: () => {},
  emitPixelClick: () => {},
  usePixelClick: () => null,
}));
vi.mock("../../hooks/useRampStore", () => ({ useRampIntegration: () => 0 }));
vi.mock("../../utils/gpuPreference", () => ({ loadGpuPreference: () => state.gpu, saveGpuPreference: () => {} }));
const viewerProps: { onCanvasPixelClick?: (x: number, y: number) => void; frameKeys?: unknown }[] = [];

vi.mock("../viewer/AdvancedImageViewer", () => ({
  default: (props: { canvasHint?: string; onCanvasPixelClick?: (x: number, y: number) => void }) => {
    viewerProps.push(props);
    return createElement("div", { "data-canvas-hint": props.canvasHint ?? "" });
  },
}));
vi.mock("../history/HistoryShortcuts", () => ({ default: () => createElement("i", { "data-history-shortcuts": "" }) }));
vi.mock("../preview/DqControls", () => ({ default: () => createElement("i", { "data-dq-controls": "" }) }));
vi.mock("../preview/DqOverlayCanvas", () => ({ default: () => null }));
vi.mock("../preview/ViewerStatusStrip", () => ({ default: () => createElement("i", { "data-status-strip": "" }) }));
const spectrumPanelProps: { name: string; onOpenCubeTool: () => void }[] = [];

vi.mock("../preview/SpectrumViewerEmpty", () => ({
  default: (props: { name: string; onOpenCubeTool: () => void }) => {
    spectrumPanelProps.push(props);
    return createElement("i", { "data-spectrum-empty": props.name });
  },
}));
vi.mock("../preview/DisplayControls", () => ({ default: () => null }));
const tabProps: { onCubePixelClick?: (x: number, y: number) => void; frameKeys?: unknown }[] = [];

vi.mock("../preview/PreviewTab", () => ({
  default: (props: { onCubePixelClick?: (x: number, y: number) => void }) => {
    tabProps.push(props);
    return null;
  },
}));

import PreviewPanel from "../PreviewPanel";
import { FrameKeysFocusRing } from "../preview/ViewerFrameStepper";
import { hooks } from "../../test/hookRuntime";
import { cubeFrameRequesters, formatFrameLabel } from "../../utils/cubeNavigation";
import { beginSpectrum, commitSpectrum, resetSpectrum, useSpectrum, type SpectrumState } from "../../hooks/useSpectrumStore";

function render(view: Partial<ViewState>): string {
  Object.assign(state, { label: null, isCube: true, isSpectralCube: true, fileDims: S3D_GRID, displayedDims: S3D_GRID, gpu: false, spectrum: false, reloads: undefined, finishedAt: undefined, imageRef: undefined, fileHeader: undefined, filePath: undefined, fileName: undefined, productPath: undefined, productVersion: undefined, previewOnly: undefined, compositeUrl: undefined, cubeFrame: undefined }, view);
  return renderToStaticMarkup(createElement(PreviewPanel));
}

function renderCurrent(): string {
  return renderToStaticMarkup(hooks.render(PreviewPanel, {}) as ReturnType<typeof createElement>);
}

function pendingHeader() {
  let resolve!: (header: Record<string, string>) => void;
  const promise = new Promise<Record<string, string>>((done) => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => {
  getHeaderMock.mockReset();
  getHeaderMock.mockResolvedValue({});
});

function spectrumSnapshot(): SpectrumState {
  const savedCells = hooks.run.cells;
  const savedCursor = hooks.run.cursor;
  hooks.run.cells = [];
  try {
    return hooks.render(useSpectrum, {}) as SpectrumState;
  } finally {
    hooks.mount();
    hooks.run.cells = savedCells;
    hooks.run.cursor = savedCursor;
  }
}

afterEach(() => {
  hooks.mount();
  resetSpectrum();
});

describe("PreviewPanel active product metadata", () => {
  const SOURCE_HEADER = { BITPIX: "16", BUNIT: "DN", EXTNAME: "RAW" };
  const PRODUCT_HEADER = { BITPIX: "-32", BUNIT: "DN/s", EXTNAME: "SCI" };
  const QUICK_SLOPE: Partial<ViewState> = { label: "Quick slope", fileDims: UNCAL_GRID, displayedDims: [2048, 2048], fileHeader: SOURCE_HEADER, filePath: "/data/uncal.fits#hdu=1" };

  it("shows the active Quick Slope dimensions before its header arrives without borrowing the UNCAL type, unit, HDU or time", () => {
    render(QUICK_SLOPE);
    const html = renderCurrent();
    expect(html).toContain("2048×2048");
    expect(html).not.toContain("2048×3200");
    expect(html).not.toContain("BITPIX 16");
    expect(html).not.toContain("HDU 1");
    expect(html).not.toContain("0.01s");
    expect(html).toContain("Loading metadata");
    expect(getHeaderMock).toHaveBeenCalledWith("/out/result.fits");
  });

  it("shows the product BITPIX and BUNIT and identifies its source after reading the active FITS", async () => {
    getHeaderMock.mockResolvedValue(PRODUCT_HEADER);
    render(QUICK_SLOPE);
    renderCurrent();
    await Promise.resolve();
    const html = renderCurrent();
    expect(html).toContain("BITPIX -32");
    expect(html).toContain("DN/s");
    expect(html).toContain("SCI");
    expect(html).toContain("Source: cube_s3d.fits");
    expect(html).not.toContain("BITPIX 16");
  });

  it("does not show another product's metadata when requests finish out of order", async () => {
    const first = pendingHeader();
    const second = pendingHeader();
    getHeaderMock.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    render(QUICK_SLOPE);
    renderCurrent();
    Object.assign(state, { productPath: "/out/ratio.fits", label: "Quick / rate ratio" });
    renderCurrent();
    second.resolve({ BITPIX: "-32", BUNIT: "ratio" });
    await Promise.resolve();
    expect(renderCurrent()).toContain("ratio");
    first.resolve(PRODUCT_HEADER);
    await Promise.resolve();
    expect(renderCurrent()).not.toContain("DN/s");
  });

  it.each([{ reloads: 1 }, { finishedAt: 123 }, { imageRef: "updated.fits" }])("reloads product metadata for a changed input revision %o", async (revision) => {
    const replacement = pendingHeader();
    getHeaderMock.mockResolvedValueOnce(PRODUCT_HEADER).mockReturnValueOnce(replacement.promise);
    render(QUICK_SLOPE);
    renderCurrent();
    await Promise.resolve();
    expect(renderCurrent()).toContain("DN/s");
    Object.assign(state, revision);
    const html = renderCurrent();
    expect(html).not.toContain("DN/s");
    expect(html).toContain("Loading metadata");
    expect(getHeaderMock).toHaveBeenCalledTimes(2);
  });

  it("keeps BITPIX and BUNIT on screen while a new version of the same product is read", async () => {
    const replacement = pendingHeader();
    getHeaderMock.mockResolvedValueOnce(PRODUCT_HEADER).mockReturnValueOnce(replacement.promise);
    render(QUICK_SLOPE);
    renderCurrent();
    await Promise.resolve();
    expect(renderCurrent()).toContain("DN/s");
    state.productVersion = 1;
    const stepped = renderCurrent();
    expect(stepped).toContain("BITPIX -32");
    expect(stepped).toContain("DN/s");
    expect(stepped).not.toContain("Loading metadata");
    expect(getHeaderMock).toHaveBeenCalledTimes(2);
    replacement.resolve({ ...PRODUCT_HEADER, BUNIT: "MJy/sr" });
    await Promise.resolve();
    expect(renderCurrent()).toContain("MJy/sr");
  });

  it("shows the header of a revisited frame at once and still re-reads it", async () => {
    const revisit = pendingHeader();
    getHeaderMock.mockResolvedValueOnce(PRODUCT_HEADER).mockResolvedValueOnce({ BITPIX: "-32", BUNIT: "Jy" }).mockReturnValueOnce(revisit.promise);
    render({ ...QUICK_SLOPE, productPath: "/out/frame_0.fits", productVersion: 1 });
    renderCurrent();
    await Promise.resolve();
    expect(renderCurrent()).toContain("DN/s");
    Object.assign(state, { productPath: "/out/frame_1.fits", productVersion: 2 });
    expect(renderCurrent()).toContain("Loading metadata");
    await Promise.resolve();
    expect(renderCurrent()).toContain(">Jy<");
    Object.assign(state, { productPath: "/out/frame_0.fits", productVersion: 3 });
    const revisited = renderCurrent();
    expect(revisited).toContain("DN/s");
    expect(revisited).not.toContain(">Jy<");
    expect(revisited).not.toContain("Loading metadata");
    expect(getHeaderMock).toHaveBeenLastCalledWith("/out/frame_0.fits");
    expect(getHeaderMock).toHaveBeenCalledTimes(3);
  });

  it("explains the PNG chip and the load-time chip", () => {
    expect(render({ ...QUICK_SLOPE, productPath: null, previewOnly: true })).toContain(`title="${PNG_ONLY_TITLE}">PNG</span>`);
    expect(render({})).toContain('title="Load time">0.01s</span>');
  });

  it("shortens long file names in the middle inside a flexible name span", () => {
    const name = "jw02739001001_02105_00001_nrcb1_rate.fits";
    const path = `/data/${name}`;
    const shown = middleEllipsis(name, 40);
    const source = render({ fileName: name, filePath: path });
    expect(source).toContain(`<span class="ab-preview-name text-[10px] font-mono text-zinc-400" title="${path}">${shown}</span>`);
    expect(source).not.toContain("max-w-[200px]");
    const product = render({ fileName: name, filePath: path, label: "Moment 0" });
    expect(product).toContain(`title="${path}">Source: ${shown}</span>`);
    expect(product).not.toContain("max-w-[200px]");
    const rule = /\.ab-preview-name\s*\{([^}]*)\}/.exec(previewLayoutCss)?.[1] ?? "";
    expect(rule).toMatch(/min-width:\s*0/);
    expect(rule).toMatch(/text-overflow:\s*ellipsis/);
    expect(rule).toMatch(/white-space:\s*nowrap/);
  });

  it("restores source metadata after reverting while a product header is pending", async () => {
    const product = pendingHeader();
    getHeaderMock.mockReturnValue(product.promise);
    render(QUICK_SLOPE);
    renderCurrent();
    Object.assign(state, { label: null, displayedDims: UNCAL_GRID });
    const html = renderCurrent();
    expect(html).toContain("2048×3200");
    expect(html).toContain("BITPIX 16");
    expect(html).toContain("HDU 1");
    product.resolve(PRODUCT_HEADER);
    await Promise.resolve();
    expect(renderCurrent()).not.toContain("DN/s");
  });

  it("reports a failed product header without displaying the source metadata", async () => {
    getHeaderMock.mockRejectedValue(new Error("Product is unreadable"));
    render(QUICK_SLOPE);
    renderCurrent();
    await Promise.resolve();
    const html = renderCurrent();
    expect(html).toContain("Metadata unavailable");
    expect(html).toContain("Product is unreadable");
    expect(html).not.toContain("BITPIX 16");
    expect(html).not.toContain("HDU 1");
  });

  it("keeps a PNG-only product free of FITS metadata", () => {
    render({ ...QUICK_SLOPE, productPath: null, previewOnly: true });
    const html = renderCurrent();
    expect(html).toContain("PNG");
    expect(html).not.toContain("BITPIX");
    expect(html).not.toContain("HDU 1");
    expect(html).not.toContain("0.01s");
    expect(getHeaderMock).not.toHaveBeenCalled();
  });

  it("does not attribute the selected file's FITS metadata or dimensions to a composite", () => {
    render({ fileHeader: SOURCE_HEADER, fileDims: UNCAL_GRID, compositeUrl: "asset://composite.png" });
    const html = renderCurrent();
    expect(html).toContain("Composite");
    expect(html).not.toContain("BITPIX");
    expect(html).not.toContain("2048×3200");
    expect(html).not.toContain("0.01s");
    expect(getHeaderMock).not.toHaveBeenCalled();
  });

  it("discards a product header after unmount", async () => {
    const product = pendingHeader();
    getHeaderMock.mockReturnValueOnce(product.promise).mockResolvedValueOnce({ BITPIX: "-64", BUNIT: "km/s" });
    render(QUICK_SLOPE);
    renderCurrent();
    hooks.mount();
    render({ ...QUICK_SLOPE, productPath: "/out/velocity.fits", label: "Velocity" });
    renderCurrent();
    await Promise.resolve();
    product.resolve(PRODUCT_HEADER);
    await Promise.resolve();
    const html = renderCurrent();
    expect(html).toContain("km/s");
    expect(html).not.toContain("DN/s");
  });
});

describe("PreviewPanel input revision", () => {
  it.each([{ reloads: 1 }, { finishedAt: 123 }, { imageRef: "updated.fits" }])("clears a pixel spectrum when the same input changes %o", (change) => {
    render({});
    hooks.mount();
    hooks.render(PreviewPanel, {});
    const request = beginSpectrum({ x: 1, y: 2 });
    commitSpectrum({ values: [1, 2], wavelengths: [], x: 1, y: 2, is_spectral: true, flux_jy: null }, 1, request);
    expect(spectrumSnapshot().spectrum).toEqual([1, 2]);
    Object.assign(state, change);
    hooks.render(PreviewPanel, {});
    expect(spectrumSnapshot().spectrum).toEqual([]);
    expect(spectrumSnapshot().coord).toBeNull();
  });

  it("does not clear a current spectrum on an unrelated viewer rerender", () => {
    render({});
    hooks.mount();
    hooks.render(PreviewPanel, {});
    const request = beginSpectrum({ x: 1, y: 2 });
    commitSpectrum({ values: [1, 2], wavelengths: [], x: 1, y: 2, is_spectral: true, flux_jy: null }, 1, request);
    state.label = "Moment 0";
    hooks.render(PreviewPanel, {});
    expect(spectrumSnapshot().spectrum).toEqual([1, 2]);
  });

  it("rejects a pending pixel spectrum after the same input is reloaded", () => {
    render({});
    hooks.mount();
    hooks.render(PreviewPanel, {});
    const request = beginSpectrum({ x: 1, y: 2 });
    state.reloads = 1;
    hooks.render(PreviewPanel, {});
    commitSpectrum({ values: [1, 2], wavelengths: [], x: 1, y: 2, is_spectral: true, flux_jy: null }, 1, request);
    expect(spectrumSnapshot().spectrum).toEqual([]);
    expect(spectrumSnapshot().loading).toBe(false);
  });
});

const HINT_ATTRIBUTE = `data-canvas-hint="${CUBE_SPECTRUM_HINT}"`;

describe("PreviewPanel spectrum hint on the CPU viewer", () => {
  it("shows the hint on a spectral cube and on a map on its spatial grid", () => {
    expect(render({})).toContain(HINT_ATTRIBUTE);
    expect(render({ label: "Moment m1 · 25 ch" })).toContain(HINT_ATTRIBUTE);
  });

  it("hides the hint on a PV diagram, whose pixels are offset by channel and not spaxels", () => {
    expect(render({ label: "PV diagram", displayedDims: [44, 3814] })).toContain('data-canvas-hint=""');
  });

  it("hides the hint on a ramp group frame and on its quick slope, where a click plots the pixel ramp", () => {
    expect(render({ isSpectralCube: false, fileDims: UNCAL_GRID, displayedDims: UNCAL_GRID })).toContain('data-canvas-hint=""');
    expect(render({ isSpectralCube: false, fileDims: UNCAL_GRID, displayedDims: [2048, 2048], label: "Quick slope (DN/s)" })).toContain('data-canvas-hint=""');
  });

  it("shows no hint on a plain image", () => {
    expect(render({ isCube: false, isSpectralCube: false })).toContain('data-canvas-hint=""');
  });
});

describe("PreviewPanel spectrum click on the CPU viewer", () => {
  function canvasClickAfterRender(view: Partial<ViewState>) {
    viewerProps.length = 0;
    render(view);
    return viewerProps[viewerProps.length - 1]?.onCanvasPixelClick;
  }

  it("passes no spectrum click handler on a PV diagram, whose grid is not the cube's spatial grid", () => {
    expect(canvasClickAfterRender({ label: "PV diagram", displayedDims: [40, 3814] })).toBeUndefined();
  });

  it("passes the spectrum click handler on the cube's spatial grid", () => {
    expect(typeof canvasClickAfterRender({ displayedDims: S3D_GRID })).toBe("function");
  });
});

describe("PreviewPanel spectrum click on the GPU viewer", () => {
  function tabClickAfterRender(view: Partial<ViewState>) {
    tabProps.length = 0;
    viewerProps.length = 0;
    render({ gpu: true, ...view });
    expect(tabProps.length).toBeGreaterThan(0);
    expect(viewerProps.length).toBe(0);
    return tabProps[tabProps.length - 1].onCubePixelClick;
  }

  it("passes no spectrum click handler on a PV diagram, whose grid is not the cube's spatial grid", () => {
    expect(tabClickAfterRender({ label: "PV diagram", displayedDims: [40, 3814] })).toBeUndefined();
  });

  it("passes the spectrum click handler on the cube's spatial grid", () => {
    expect(typeof tabClickAfterRender({ displayedDims: S3D_GRID })).toBe("function");
  });
});

describe("PreviewPanel history shortcuts", () => {
  it("mounts the undo and redo keys inside the preview on both viewers", () => {
    expect(render({})).toContain("data-history-shortcuts");
    expect(render({ gpu: true })).toContain("data-history-shortcuts");
  });
});

describe("PreviewPanel Revert to original", () => {
  it("moves the history head to the original instead of dropping the chain", () => {
    hostProps.clear();
    revertMock.mockClear();
    resetProcessedMock.mockClear();
    render({ label: "Wavelet Denoise", isCube: false, isSpectralCube: false });
    const revert = hostProps.get("revert-original");
    if (!revert) throw new Error("Revert to original is not shown while a result is on screen");
    expect(String(revert.title)).toContain("the history keeps every step");
    (revert.onClick as () => void)();
    expect(revertMock).toHaveBeenCalledTimes(1);
    expect(resetProcessedMock).not.toHaveBeenCalled();
  });
});

describe("PreviewPanel frame stepper", () => {
  const CUBE_PATH = "/data/cube_s3d.fits";
  const FRAME: Partial<ViewState> = { label: "ch 5 (0–39)", cubeFrame: 5 };
  const request = vi.fn<(idx: number) => void>();
  let unregister: () => void = () => {};

  beforeEach(() => {
    request.mockReset();
    unregister = cubeFrameRequesters.register({
      filePath: CUBE_PATH,
      totalFrames: 40,
      playing: false,
      failure: null,
      labelFor: (idx) => formatFrameLabel(idx, 40, null, ""),
      stepOrigin: (displayed) => displayed,
      request,
    });
  });

  afterEach(() => unregister());

  type ElementProps = Record<string, unknown> & { children?: unknown };
  interface Located {
    path: number[];
    type: unknown;
    props: ElementProps;
  }

  function locate(node: unknown, match: (type: unknown, props: ElementProps) => boolean, path: number[] = []): Located | null {
    if (!isValidElement<ElementProps>(node)) return null;
    if (match(node.type, node.props)) return { path, type: node.type, props: node.props };
    const children = Array.isArray(node.props.children) ? node.props.children : [node.props.children];
    for (let i = 0; i < children.length; i++) {
      const found = locate(children[i], match, [...path, i]);
      if (found) return found;
    }
    return null;
  }

  const isViewport = (_type: unknown, props: ElementProps) => props["data-dock-viewport"] !== undefined;
  const isRing = (type: unknown) => type === FrameKeysFocusRing;

  function headerActions(html: string): string {
    const at = html.indexOf('class="ab-preview-actions"');
    expect(at).toBeGreaterThan(-1);
    return html.slice(at, html.indexOf('data-dock-viewport=""'));
  }

  it("puts a frame stepper first in the header actions while a cube frame is on screen", () => {
    const actions = headerActions(render(FRAME));
    expect(actions).toContain('title="Previous frame"');
    expect(actions).toContain(">ch 5 (0–39)</span>");
    expect(actions).toContain('title="Next frame"');
    expect(actions.indexOf('title="Next frame"')).toBeLessThan(actions.indexOf("data-dq-controls"));
  });

  it("shows no frame stepper on the original cube or on a map of it", () => {
    expect(render({})).not.toContain('title="Previous frame"');
    expect(render({ label: "Moment 0" })).not.toContain('title="Previous frame"');
  });

  it("keeps the frame keys on the viewport, which stays mounted when a PNG-first frame swaps the GPU viewer for the CPU viewer", () => {
    render({ ...FRAME, gpu: true });
    hooks.mount();
    const gpu = hooks.render(PreviewPanel, {});
    tabProps.length = 0;
    renderToStaticMarkup(gpu as ReturnType<typeof createElement>);
    expect(tabProps).toHaveLength(1);
    expect(tabProps[0].frameKeys, "the GPU canvas leaves the page in the swap").toBeUndefined();
    const onGpu = locate(gpu, isViewport);
    state.previewOnly = true;
    const cpu = hooks.render(PreviewPanel, {});
    viewerProps.length = 0;
    renderToStaticMarkup(cpu as ReturnType<typeof createElement>);
    expect(viewerProps).toHaveLength(1);
    expect(viewerProps[0].frameKeys, "the CPU canvas leaves the page when the FITS arrives").toBeUndefined();
    const onCpu = locate(cpu, isViewport);
    expect(onCpu?.type).toBe("div");
    expect(onCpu?.path, "the same element in both viewers, so focus stays on it").toEqual(onGpu?.path);
    for (const viewport of [onGpu, onCpu]) {
      expect(viewport?.props.tabIndex).toBe(0);
      expect(viewport?.props["aria-keyshortcuts"]).toContain("ArrowRight");
    }
    const self = {};
    (onCpu?.props.onKeyDown as (event: unknown) => void)({ key: "ArrowRight", shiftKey: false, target: self, currentTarget: self, preventDefault: () => {} });
    expect(request).toHaveBeenCalledWith(6);
  });

  it("draws the viewport's focus ring as a layer inside it and hands it the viewport's own outline", () => {
    render(FRAME);
    hooks.mount();
    const tree = hooks.render(PreviewPanel, {});
    const viewport = locate(tree, isViewport);
    expect(String(viewport?.props.className).split(" ")).toEqual(expect.arrayContaining(["group/frame-keys", "relative"]));
    expect(viewport?.props.style).toEqual({ outline: "none" });
    const ring = locate(tree, isRing);
    expect(ring?.path.slice(0, viewport?.path.length)).toEqual(viewport?.path);
    expect(renderToStaticMarkup(tree as ReturnType<typeof createElement>)).toContain("group-focus-visible/frame-keys:block");
  });

  it("makes the viewport no focus stop and draws no ring without a cube frame on screen", () => {
    render({ label: "Moment 0" });
    hooks.mount();
    const tree = hooks.render(PreviewPanel, {});
    const viewport = locate(tree, isViewport);
    expect(viewport?.props.tabIndex).toBeUndefined();
    expect(viewport?.props.onKeyDown).toBeUndefined();
    expect(viewport?.props.style).toBeUndefined();
    expect(locate(tree, isRing)).toBeNull();
  });
});

describe("PreviewPanel with a 1D spectrum selected", () => {
  const SPECTRUM_VIEW: Partial<ViewState> = { spectrum: true, isCube: false, isSpectralCube: false };
  const GPU_TOGGLE = 'data-testid="gpu-toggle"';

  it("shows the spectrum panel instead of an image viewer and hides the DQ controls, the GPU toggle and the status strip", () => {
    spectrumPanelProps.length = 0;
    viewerProps.length = 0;
    tabProps.length = 0;
    const html = render(SPECTRUM_VIEW);
    expect(html).toContain(`data-spectrum-empty="${X1D_NAME}"`);
    expect(spectrumPanelProps.at(-1)?.name).toBe(X1D_NAME);
    expect(viewerProps).toHaveLength(0);
    expect(tabProps).toHaveLength(0);
    expect(html).not.toContain("data-dq-controls");
    expect(html).not.toContain(GPU_TOGGLE);
    expect(html).not.toContain("data-status-strip");
  });

  it("keeps the DQ controls, the GPU toggle and the status strip on an image (control)", () => {
    const html = render({ isCube: false, isSpectralCube: false });
    expect(html).not.toContain("data-spectrum-empty");
    expect(html).toContain("data-dq-controls");
    expect(html).toContain(GPU_TOGGLE);
    expect(html).toContain("data-status-strip");
  });

  it("opens the Cube tool from the spectrum panel and keeps it open when routing already opened it", () => {
    const dispatch = vi.spyOn(dockStore, "dispatch").mockImplementation(() => {});
    try {
      spectrumPanelProps.length = 0;
      render(SPECTRUM_VIEW);
      const openCubeTool = spectrumPanelProps.at(-1)?.onOpenCubeTool;
      if (!openCubeTool) throw new Error("the spectrum panel got no Open the Cube tool handler");
      openCubeTool();
      openCubeTool();
      expect(dispatch).toHaveBeenCalledTimes(2);
      const routed = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "open", tool: "cube" });
      let layout = routed;
      for (const [action] of dispatch.mock.calls) {
        layout = dockReducer(layout, action);
        expect(layout.active[anchorOf(routed, "cube")]).toBe("cube");
      }
      const closed = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "open", tool: "image" });
      const fromImage = dockReducer(closed, dispatch.mock.calls[0][0]);
      expect(fromImage.active[anchorOf(closed, "cube")]).toBe("cube");
    } finally {
      dispatch.mockRestore();
    }
  });
});
