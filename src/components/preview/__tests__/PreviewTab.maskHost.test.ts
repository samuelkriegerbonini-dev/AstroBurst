import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DEFAULT_DISPLAY_SETTINGS } from "../../../shared/types/display";
import type { RawRgbPixelData, StfParams } from "../../../shared/types";

const FILE = "C:\\data\\m16_rgb.fits";
const FILE_PNG = "asset://localhost/m16_rgb.png";
const WIZARD_PNG = "asset://localhost/rgb_composite_1792793764416.png";
const STF: StfParams = { shadow: 0.02, midtone: 0.12, highlight: 1 };

const screen = vi.hoisted(() => ({ composite: null as string | null }));

vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({
    file: { path: FILE, name: "m16_rgb.fits", result: { dimensions: [128, 96], previewUrl: FILE_PNG, header: {}, is_rgb: true } },
  }),
  useHistContext: () => ({ stfParams: { shadow: 0, midtone: 0.5, highlight: 1 }, histData: null, histDataPath: null }),
  useCubeContext: () => ({ isCube: false, isSpectralCube: false }),
  useRenderContext: () => ({ processed: null }),
  useDisplayContext: () => ({ display: DEFAULT_DISPLAY_SETTINGS, limits: null, lut: null, limitsLoading: false }),
  useDisplayedImage: () => ({ path: FILE, dimensions: [128, 96], isProcessed: false, label: null, previewOnly: false, previewUrl: FILE_PNG }),
}));
vi.mock("../../../context/CompositeContext", () => ({
  useCompositePreview: () => ({ compositePreviewUrl: screen.composite }),
  useCompositeStf: () => ({
    compositeStfR: STF,
    compositeStfG: STF,
    compositeStfB: STF,
    compositeStfLinked: true,
    compositeAutoStfR: STF,
    compositeAutoStfG: STF,
    compositeAutoStfB: STF,
  }),
  useCompositeActions: () => ({ setCompositeStf: () => {}, setCompositeStfLinked: () => {} }),
}));
vi.mock("../../../utils/displayMemoryStore", () => ({
  useFileDisplayEntry: () => null,
  displayMemory: { update: () => {}, get: () => null },
}));
vi.mock("../../../context/ComposeWizardContext", () => ({ useWizardCompositeDims: () => [128, 96] }));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => FILE }));
vi.mock("../../render/GpuViewport", async () => {
  const react = await import("react");
  const { default: OverlayLayer } = await import("../../viewer/OverlayLayer");
  return {
    default: (props: { renderW: number; renderH: number; fitsW?: number; fitsH?: number; regionsEnabled?: boolean }) =>
      react.createElement(
        "div",
        { "data-regions": String(!!props.regionsEnabled) },
        react.createElement(OverlayLayer, {
          containerRef: { current: null },
          transform: { scale: 1, x: 0, y: 0 },
          renderW: props.renderW,
          renderH: props.renderH,
          fitsW: props.fitsW ?? props.renderW,
          fitsH: props.fitsH ?? props.renderH,
          enabled: !!props.regionsEnabled,
        }),
      ),
  };
});
vi.mock("../../render/GpuRenderer", () => ({ default: () => null }));
vi.mock("../../render/GpuRgbRenderer", () => ({ default: () => null }));
vi.mock("../DisplayControls", () => ({ default: () => null }));
vi.mock("../Colorbar", () => ({ default: () => null }));
vi.mock("../../ui/ZoomPanView", () => ({ default: () => null }));

import PreviewTab from "../PreviewTab";
import OverlayLayer from "../../viewer/OverlayLayer";
import { overlayHostKey } from "../../../context/OverlayLayerKeyContext";
import { MASK_LAYER_ID, MASK_LAYER_KIND, maskLayer } from "../../viewer/painters/maskPainter";
import { COMPOSITE_RUN_KEY } from "../../../utils/compositeChain";
import { overlayStore } from "../../../utils/overlayStore";

const PIXELS = { width: 128, height: 96, displayReferred: false } as unknown as RawRgbPixelData;

function renderView(composite: string | null): string {
  screen.composite = composite;
  return renderToStaticMarkup(
    createElement(PreviewTab, { useGpu: true, rawPixels: null, rgbRawPixels: PIXELS, onBackToFile: () => {}, starOverlayRef: { current: null } }),
  );
}

function overlayCanvas(html: string): string {
  return html.match(/<canvas[^>]*data-testid="viewer-overlay"[^>]*>/)?.[0] ?? "";
}

function registerLayers(): void {
  overlayStore.add(COMPOSITE_RUN_KEY, { id: MASK_LAYER_ID, kind: MASK_LAYER_KIND, visible: true, paint: () => {} });
  overlayStore.add(FILE, { id: "gaia-catalog", kind: "catalog", visible: true, paint: () => {} });
}

afterEach(() => {
  overlayStore.clear(COMPOSITE_RUN_KEY);
  overlayStore.clear(FILE);
});

describe("overlay host of the wizard composite view", () => {
  it("overlayHostKey_is_the_composite_key_on_the_wizard_composite_view", () => {
    expect(overlayHostKey({ enabled: false, regionKey: FILE, layerKey: COMPOSITE_RUN_KEY })).toBe(COMPOSITE_RUN_KEY);
    expect(overlayHostKey({ enabled: true, regionKey: FILE, layerKey: COMPOSITE_RUN_KEY })).toBe(COMPOSITE_RUN_KEY);
    expect(overlayHostKey({ enabled: true, regionKey: FILE, layerKey: null })).toBe(FILE);
    expect(overlayHostKey({ enabled: false, regionKey: FILE, layerKey: null })).toBeNull();

    registerLayers();
    const wizard = renderView(WIZARD_PNG);
    expect(wizard).toContain("RGB Composite");
    expect(wizard).toContain('data-regions="false"');
    expect(overlayCanvas(wizard)).toContain(`data-layers="${MASK_LAYER_ID}"`);

    const rgbFile = renderView(FILE_PNG);
    expect(rgbFile).not.toContain("RGB Composite");
    expect(rgbFile).toContain('data-regions="true"');
    expect(overlayCanvas(rgbFile)).toContain('data-layers="gaia-catalog"');
  });

  it("an_overlay_layer_given_a_layer_key_draws_only_that_keys_layers_with_regions_off", () => {
    registerLayers();
    const props = {
      containerRef: { current: null },
      transform: { scale: 1, x: 0, y: 0 },
      renderW: 128,
      renderH: 96,
      fitsW: 128,
      fitsH: 96,
      enabled: false,
    };
    expect(renderToStaticMarkup(createElement(OverlayLayer, props))).toBe("");
    const keyed = renderToStaticMarkup(createElement(OverlayLayer, { ...props, layerKey: COMPOSITE_RUN_KEY }));
    expect(overlayCanvas(keyed)).toContain(`data-layers="${MASK_LAYER_ID}"`);
    overlayStore.clear(COMPOSITE_RUN_KEY);
    expect(renderToStaticMarkup(createElement(OverlayLayer, { ...props, layerKey: COMPOSITE_RUN_KEY }))).toBe("");
  });

  it("composite_mask_canvas_carries_the_mask_id_and_its_marked_cell_count_for_the_driver", () => {
    const surface = () => ({}) as CanvasImageSource;
    const preview = { width: 4, height: 1, cells: new Uint8Array([0, 3, 200, 255]) };
    const layer = maskLayer({ maskId: "m01", preview, dims: [128, 96], style: { opacity: 0.45, tint: [255, 0, 0], displayInvert: false } }, surface);
    if (!layer) throw new Error("mask layer was not built");
    overlayStore.add(COMPOSITE_RUN_KEY, layer);
    overlayStore.add(FILE, { id: "gaia-catalog", kind: "catalog", visible: true, paint: () => {} });
    const canvas = overlayCanvas(renderView(WIZARD_PNG));
    expect(canvas).toContain(`data-layers="${MASK_LAYER_ID}"`);
    expect(canvas).toContain('data-mask="m01"');
    expect(canvas).toContain('data-mask-cells="3"');
    const fileCanvas = overlayCanvas(renderView(FILE_PNG));
    expect(fileCanvas).toContain('data-layers="gaia-catalog"');
    expect(fileCanvas).not.toContain("data-mask");
  });
});
