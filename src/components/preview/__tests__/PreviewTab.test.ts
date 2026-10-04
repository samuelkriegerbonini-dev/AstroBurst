import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DEFAULT_DISPLAY_SETTINGS } from "../../../shared/types/display";
import { CUBE_SPECTRUM_HINT } from "../../../utils/previewShell";

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

vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({
    file: { path: "/data/cube_s3d.fits", name: "cube_s3d.fits", result: { dimensions: state.fileDims, previewUrl: "asset://cube.png", header: {} } },
  }),
  useHistContext: () => ({ stfParams: { shadow: 0, midtone: 0.5, highlight: 1 }, histData: null, histDataPath: null }),
  useCubeContext: () => ({ isCube: state.isCube, isSpectralCube: state.isSpectralCube }),
  useRenderContext: () => ({ processed: state.label ? { label: state.label } : null }),
  useDisplayContext: () => ({ display: DEFAULT_DISPLAY_SETTINGS, limits: null, lut: null, limitsLoading: false }),
  useDisplayedImage: () => ({
    path: state.label ? "/out/result.fits" : "/data/cube_s3d.fits",
    dimensions: state.displayedDims,
    isProcessed: state.label !== null,
    label: state.label,
    previewOnly: false,
    previewUrl: "asset://shown.png",
  }),
}));
vi.mock("../../../context/CompositeContext", () => ({
  useCompositePreview: () => ({ compositePreviewUrl: null }),
  useCompositeStf: () => ({}),
  useCompositeActions: () => ({ setCompositeStf: () => {}, setCompositeStfLinked: () => {} }),
}));
vi.mock("../../../context/ComposeWizardContext", () => ({ useWizardCompositeDims: () => null }));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => null }));
vi.mock("../../render/GpuViewport", () => ({
  default: (props: { label?: string | null; original?: { disabledReason: string | null } | null }) =>
    createElement("div", { "data-viewport-label": props.label ?? "", "data-original": props.original ? (props.original.disabledReason ?? "usable") : "none" }),
}));
vi.mock("../../render/GpuRenderer", () => ({ default: () => null }));
vi.mock("../../render/GpuRgbRenderer", () => ({ default: () => null }));
vi.mock("../DisplayControls", () => ({ default: () => null }));
vi.mock("../Colorbar", () => ({ default: () => null }));
vi.mock("../../ui/ZoomPanView", () => ({ default: () => null }));

import PreviewTab from "../PreviewTab";

function render(view: Partial<ViewState>): string {
  Object.assign(state, { label: null, isCube: true, isSpectralCube: true, fileDims: S3D_GRID, displayedDims: S3D_GRID }, view);
  return renderToStaticMarkup(
    createElement(PreviewTab, {
      useGpu: true,
      rawPixels: { data: new Float32Array(4), width: 2, height: 2, min: 0, max: 1 },
      onCubePixelClick: () => {},
      onBackToFile: () => {},
      starOverlayRef: { current: null },
    }),
  );
}

describe("PreviewTab processed label", () => {
  const label = "Moment m1 · 25 ch";

  it("hands the processed label to the GPU viewport instead of pinning it over the bottom-left compass corner", () => {
    const html = render({ label });
    expect(html).toContain(`data-viewport-label="${label}"`);
    expect(html).not.toContain(`>${label}</div>`);
  });

  it("passes no label for the loaded file", () => {
    expect(render({})).toContain('data-viewport-label=""');
  });
});

describe("PreviewTab hold-to-compare original", () => {
  it("lets the viewer hold the original over a result on the file grid and blocks it on another grid", () => {
    expect(render({ label: "Moment m1 · 25 ch" })).toContain('data-original="usable"');
    expect(render({ label: "PV diagram", displayedDims: [44, 3814] })).toContain('data-original="The original is 53×55 and the result 44×3814 px; hold-to-compare needs the same pixel grid"');
  });
});

describe("PreviewTab spectrum hint on the GPU viewer", () => {
  it("shows the hint on a spectral cube and on a map on its spatial grid", () => {
    expect(render({})).toContain(CUBE_SPECTRUM_HINT);
    expect(render({ label: "Moment m1 · 25 ch" })).toContain(CUBE_SPECTRUM_HINT);
  });

  it("hides the hint on a PV diagram, whose pixels are offset by channel and not spaxels", () => {
    expect(render({ label: "PV diagram", displayedDims: [44, 3814] })).not.toContain(CUBE_SPECTRUM_HINT);
  });

  it("hides the hint on a ramp group frame and on its quick slope, where a click plots the pixel ramp", () => {
    expect(render({ isSpectralCube: false, fileDims: UNCAL_GRID, displayedDims: UNCAL_GRID })).not.toContain(CUBE_SPECTRUM_HINT);
    expect(render({ isSpectralCube: false, fileDims: UNCAL_GRID, displayedDims: [2048, 2048], label: "Quick slope (DN/s)" })).not.toContain(CUBE_SPECTRUM_HINT);
  });
});
