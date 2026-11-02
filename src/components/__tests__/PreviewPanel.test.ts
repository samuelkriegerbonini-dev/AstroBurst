import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CUBE_SPECTRUM_HINT } from "../../utils/previewShell";
import { dockStore } from "../../hooks/useDockLayout";
import { DEFAULT_DOCK_LAYOUT, anchorOf, dockReducer } from "../../utils/dockLayout";

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
  productPath?: string | null;
  productVersion?: number;
  previewOnly?: boolean;
  compositeUrl?: string | null;
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
          name: "cube_s3d.fits",
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
    processed: state.label ? { label: state.label, previewUrl: "asset://result.png", fitsPath: state.productPath === undefined ? "/out/result.fits" : state.productPath, dimensions: state.displayedDims } : null,
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
const viewerProps: { onCanvasPixelClick?: (x: number, y: number) => void }[] = [];

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
const tabProps: { onCubePixelClick?: (x: number, y: number) => void }[] = [];

vi.mock("../preview/PreviewTab", () => ({
  default: (props: { onCubePixelClick?: (x: number, y: number) => void }) => {
    tabProps.push(props);
    return null;
  },
}));

import PreviewPanel from "../PreviewPanel";
import { hooks } from "../../test/hookRuntime";
import { beginSpectrum, commitSpectrum, resetSpectrum, useSpectrum, type SpectrumState } from "../../hooks/useSpectrumStore";

function render(view: Partial<ViewState>): string {
  Object.assign(state, { label: null, isCube: true, isSpectralCube: true, fileDims: S3D_GRID, displayedDims: S3D_GRID, gpu: false, spectrum: false, reloads: undefined, finishedAt: undefined, imageRef: undefined, fileHeader: undefined, filePath: undefined, productPath: undefined, productVersion: undefined, previewOnly: undefined, compositeUrl: undefined }, view);
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

  it.each([{ productVersion: 1 }, { reloads: 1 }, { finishedAt: 123 }, { imageRef: "updated.fits" }])("reloads product metadata for a changed revision %o", async (revision) => {
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
