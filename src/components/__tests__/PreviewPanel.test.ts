import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CUBE_SPECTRUM_HINT } from "../../utils/previewShell";

interface ViewState {
  label: string | null;
  isCube: boolean;
  isSpectralCube: boolean;
  fileDims: [number, number];
  displayedDims: [number, number];
}

const S3D_GRID: [number, number] = [53, 55];
const UNCAL_GRID: [number, number] = [2048, 3200];

const state: ViewState = { label: null, isCube: true, isSpectralCube: true, fileDims: S3D_GRID, displayedDims: S3D_GRID };

vi.mock("../../context/PreviewContext", () => ({
  fileKeyOf: (file: { id: string; path: string } | null) => (file ? `${file.id}|${file.path}` : null),
  useFileContext: () => ({
    file: {
      id: "1",
      path: "/data/cube_s3d.fits",
      name: "cube_s3d.fits",
      result: { dimensions: state.fileDims, previewUrl: "asset://cube.png", header: {}, elapsed_ms: 10 },
    },
  }),
  useCubeContext: () => ({ isCube: state.isCube, isSpectralCube: state.isSpectralCube, ramp: null }),
  useDisplayedImage: () => ({
    path: state.label ? "/out/result.fits" : "/data/cube_s3d.fits",
    dimensions: state.displayedDims,
    isProcessed: state.label !== null,
    label: state.label,
    previewOnly: false,
    previewUrl: "asset://shown.png",
  }),
  useRawPixelsContext: () => ({
    rawPixels: null,
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
    processed: state.label ? { label: state.label, previewUrl: "asset://result.png", fitsPath: "/out/result.fits" } : null,
    processedVersion: 0,
    stfPreviewUrl: null,
    chain: { psfKernel: null },
  }),
  useRenderActions: () => ({ resetProcessed: () => {}, currentFileKey: () => null }),
  useStarOverlayContext: () => ({ starOverlayRef: { current: null } }),
}));
vi.mock("../../context/CompositeContext", () => ({
  useCompositePreview: () => ({ compositePreviewUrl: null, compositeVersion: 0, canShowParked: false }),
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
vi.mock("../../utils/gpuPreference", () => ({ loadGpuPreference: () => false, saveGpuPreference: () => {} }));
vi.mock("../viewer/AdvancedImageViewer", () => ({
  default: (props: { canvasHint?: string }) => createElement("div", { "data-canvas-hint": props.canvasHint ?? "" }),
}));
vi.mock("../preview/DqControls", () => ({ default: () => null }));
vi.mock("../preview/DqOverlayCanvas", () => ({ default: () => null }));
vi.mock("../preview/ViewerStatusStrip", () => ({ default: () => null }));
vi.mock("../preview/DisplayControls", () => ({ default: () => null }));
vi.mock("../preview/PreviewTab", () => ({ default: () => null }));

import PreviewPanel from "../PreviewPanel";

function render(view: Partial<ViewState>): string {
  Object.assign(state, { label: null, isCube: true, isSpectralCube: true, fileDims: S3D_GRID, displayedDims: S3D_GRID }, view);
  return renderToStaticMarkup(createElement(PreviewPanel));
}

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
