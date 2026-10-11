import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DEFAULT_DISPLAY_SETTINGS } from "../../../shared/types/display";
import { CUBE_SPECTRUM_HINT } from "../../../utils/previewShell";
import { STF_LOCK_RGB_CPU } from "../../../utils/analysisTarget";
import { RGB_LINEAR_BAKED, type FileDisplayEntry } from "../../../utils/displayMemory";
import type { RawRgbPixelData } from "../../../shared/types";

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

interface RgbState {
  isRgb: boolean;
  compositeUrl: string | null;
  entry: FileDisplayEntry | null;
}

const NO_RGB: RgbState = { isRgb: false, compositeUrl: null, entry: null };
const rgb: RgbState = { ...NO_RGB };

vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({
    file: {
      path: "/data/cube_s3d.fits",
      name: "cube_s3d.fits",
      result: { dimensions: state.fileDims, previewUrl: "asset://cube.png", header: {}, is_rgb: rgb.isRgb },
    },
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
  useCompositePreview: () => ({ compositePreviewUrl: rgb.compositeUrl }),
  useCompositeStf: () => ({}),
  useCompositeActions: () => ({ setCompositeStf: () => {}, setCompositeStfLinked: () => {} }),
}));
vi.mock("../../../utils/displayMemoryStore", () => ({
  useFileDisplayEntry: (path: string | null) => (path === null ? null : rgb.entry),
  displayMemory: { update: () => {}, get: () => null },
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
  Object.assign(rgb, NO_RGB);
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

const RGB_PIXELS = { width: 2, height: 2, displayReferred: false } as unknown as RawRgbPixelData;
const IDENTITY = { shadow: 0, midtone: 0.5, highlight: 1 };

function renderRgb(view: { fileView: boolean; useGpu: boolean; pixels: RawRgbPixelData | null; entry?: FileDisplayEntry | null }): string {
  Object.assign(state, { label: null, isCube: false, isSpectralCube: false, fileDims: S3D_GRID, displayedDims: S3D_GRID });
  Object.assign(rgb, {
    isRgb: view.fileView,
    compositeUrl: view.fileView ? "asset://cube.png" : "asset://compose_rgb.png",
    entry: view.entry ?? null,
  });
  return renderToStaticMarkup(
    createElement(PreviewTab, {
      useGpu: view.useGpu,
      rawPixels: null,
      rgbRawPixels: view.pixels,
      onBackToFile: () => {},
      starOverlayRef: { current: null },
    }),
  );
}

function rgbLinearTag(markup: string): string {
  const at = markup.indexOf('data-testid="rgb-linear"');
  expect(at).toBeGreaterThan(-1);
  return markup.slice(markup.lastIndexOf("<", at), markup.indexOf(">", at) + 1);
}

describe("PreviewTab RGB Linear", () => {
  it("the RGB file view on the GPU shows an enabled rgb-linear button", () => {
    const html = renderRgb({ fileView: true, useGpu: true, pixels: RGB_PIXELS });
    const tag = rgbLinearTag(html);
    expect(tag).not.toContain('disabled=""');
    expect(tag).toContain('aria-pressed="false"');
    expect(tag).toContain(
      'title="Linear: show each channel linearly from its own minimum to its maximum (per-channel STF reset). Click again to go back"',
    );
    expect(html).toMatch(/data-testid="rgb-linear"[^>]*>Linear<\/button>/);
    expect(html.indexOf('data-testid="rgb-linear"')).toBeGreaterThan(html.indexOf(">STF</button>"));
    const saved = { r: IDENTITY, g: IDENTITY, b: IDENTITY, linked: true };
    const on = rgbLinearTag(renderRgb({ fileView: true, useGpu: true, pixels: RGB_PIXELS, entry: { rgbLinear: saved } }));
    expect(on).toContain('aria-pressed="true"');
    expect(on).toContain('title="Back to the previous channel STF"');
  });

  it("the RGB file view on the CPU disables rgb-linear with STF_LOCK_RGB_CPU", () => {
    for (const html of [
      renderRgb({ fileView: true, useGpu: false, pixels: null }),
      renderRgb({ fileView: true, useGpu: true, pixels: null }),
      renderRgb({ fileView: true, useGpu: false, pixels: RGB_PIXELS }),
    ]) {
      const tag = rgbLinearTag(html);
      expect(tag).toContain('disabled=""');
      expect(tag).toContain(`title="${STF_LOCK_RGB_CPU}"`);
    }
  });

  it("a display-referred RGB view disables rgb-linear", () => {
    const baked = { ...RGB_PIXELS, displayReferred: true } as RawRgbPixelData;
    const tag = rgbLinearTag(renderRgb({ fileView: true, useGpu: true, pixels: baked }));
    expect(tag).toContain('disabled=""');
    expect(tag).toContain(`title="${RGB_LINEAR_BAKED}"`);
  });

  it("the Compose composite has no rgb-linear button", () => {
    const html = renderRgb({ fileView: false, useGpu: true, pixels: RGB_PIXELS });
    expect(html).toContain("RGB Composite");
    expect(html).not.toContain("rgb-linear");
  });
});
