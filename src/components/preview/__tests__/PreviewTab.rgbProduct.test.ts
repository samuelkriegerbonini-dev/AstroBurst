import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DEFAULT_DISPLAY_SETTINGS } from "../../../shared/types/display";
import type { RawRgbPixelData, StfParams } from "../../../shared/types";
import type { ProcessedResult } from "../../../shared/types/preview";

const RGB = "C:\\astrokit\\docs\\notCommit\\comparison-2026-10-26\\runtime-rgb.fits";
const RGB_PNG = "asset://localhost/runtime-rgb.png";
const CURVES = "C:\\history\\344da451b072ae3a\\n0001.fits";
const CURVES_PNG = "asset://localhost/n0001.png?v=7";
const FILE_STF: StfParams = { shadow: 0.02, midtone: 0.12, highlight: 1 };

const CURVES_RECORD: ProcessedResult = {
  fitsPath: CURVES,
  previewUrl: CURVES_PNG,
  dimensions: [128, 128],
  label: "Curves",
  kind: "processing",
  inputPath: RGB,
  planes: 3,
  domain: "display_referred",
};

const screen = vi.hoisted(() => ({ processed: null as ProcessedResult | null, composite: null as string | null }));

vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({
    file: { path: RGB, name: "runtime-rgb.fits", result: { dimensions: [128, 128], previewUrl: RGB_PNG, header: {}, is_rgb: true } },
  }),
  useHistContext: () => ({ stfParams: { shadow: 0, midtone: 0.5, highlight: 1 }, histData: null, histDataPath: null }),
  useCubeContext: () => ({ isCube: false, isSpectralCube: false }),
  useRenderContext: () => ({ processed: screen.processed }),
  useDisplayContext: () => ({ display: DEFAULT_DISPLAY_SETTINGS, limits: null, lut: null, limitsLoading: false }),
  useDisplayedImage: () => ({
    path: screen.processed?.fitsPath ?? RGB,
    dimensions: [128, 128],
    isProcessed: screen.processed !== null,
    label: screen.processed?.label ?? null,
    previewOnly: false,
    previewUrl: screen.processed?.previewUrl ?? RGB_PNG,
  }),
}));
vi.mock("../../../context/CompositeContext", () => ({
  useCompositePreview: () => ({ compositePreviewUrl: screen.composite }),
  useCompositeStf: () => ({
    compositeStfR: FILE_STF,
    compositeStfG: FILE_STF,
    compositeStfB: FILE_STF,
    compositeStfLinked: false,
    compositeAutoStfR: FILE_STF,
    compositeAutoStfG: FILE_STF,
    compositeAutoStfB: FILE_STF,
  }),
  useCompositeActions: () => ({ setCompositeStf: () => {}, setCompositeStfLinked: () => {} }),
}));
vi.mock("../../../utils/displayMemoryStore", () => ({
  useFileDisplayEntry: () => null,
  displayMemory: { update: () => {}, get: () => null },
}));
vi.mock("../../../context/ComposeWizardContext", () => ({ useWizardCompositeDims: () => null }));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => RGB }));
vi.mock("../../render/GpuViewport", () => ({
  default: (props: { label?: string | null; original?: { disabledReason: string | null } | null; regionsEnabled?: boolean; crosshairEnabled?: boolean; fitsW?: number; children?: unknown }) =>
    createElement(
      "div",
      {
        "data-viewport-label": props.label ?? "",
        "data-original": props.original ? (props.original.disabledReason ?? "usable") : "none",
        "data-regions": String(!!props.regionsEnabled),
        "data-crosshair": String(!!props.crosshairEnabled),
        "data-fits-w": String(props.fitsW ?? ""),
      },
      props.children as never,
    ),
}));
vi.mock("../../render/GpuRenderer", () => ({ default: () => createElement("i", { "data-mono-renderer": "" }) }));
vi.mock("../../render/GpuRgbRenderer", () => ({
  default: (props: { stfR: StfParams; linked: boolean }) =>
    createElement("i", { "data-rgb-renderer": `${props.stfR.shadow}/${props.stfR.midtone}/${props.stfR.highlight}/${props.linked ? "linked" : "per-channel"}` }),
}));
vi.mock("../DisplayControls", () => ({ default: () => null }));
vi.mock("../Colorbar", () => ({ default: () => null }));
vi.mock("../../ui/ZoomPanView", () => ({ default: (props: { src: string | null }) => createElement("i", { "data-zoom-src": props.src ?? "" }) }));

import PreviewTab from "../PreviewTab";

const BAKED_PIXELS = { width: 128, height: 128, displayReferred: true } as unknown as RawRgbPixelData;

function renderResult(view: { useGpu: boolean; pixels: RawRgbPixelData | null }): string {
  screen.processed = CURVES_RECORD;
  screen.composite = null;
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

describe("PreviewTab shows a processed RGB result on the RGB viewer (AB-ERR-04)", () => {
  it("draws the Curves result with the RGB renderer and the baked identity STF, not the file's STF", () => {
    const html = renderResult({ useGpu: true, pixels: BAKED_PIXELS });
    expect(html).toContain('data-rgb-renderer="0/0.5/1/linked"');
    expect(html).not.toContain("data-mono-renderer");
    expect(html).toContain("RGB · GPU");
    expect(html).toMatch(/>processed<\/span>/);
    expect(html).not.toContain(">STF</button>");
    expect(html).not.toContain("Back to file");
    expect(html).not.toContain('data-testid="rgb-linear"');
  });

  it("keeps the processed label, hold-to-compare, crosshair and regions of a result on the file grid", () => {
    const html = renderResult({ useGpu: true, pixels: BAKED_PIXELS });
    expect(html).toContain('data-viewport-label="Curves"');
    expect(html).toContain('data-original="usable"');
    expect(html).toContain('data-crosshair="true"');
    expect(html).toContain('data-regions="true"');
    expect(html).toContain('data-fits-w="128"');
  });

  it("shows the Curves preview, not the original or an empty view, until the RGB planes arrive", () => {
    const html = renderResult({ useGpu: true, pixels: null });
    expect(html).toContain(`data-zoom-src="${CURVES_PNG}"`);
    expect(html).not.toContain("data-rgb-renderer");
  });
});
