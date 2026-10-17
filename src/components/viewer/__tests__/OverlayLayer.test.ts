import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const FILE = "C:\\data\\jw02739001001_02105_00001_nrca1_cal.fits";

vi.mock("../../../context/PreviewContext", () => ({
  useDisplayContext: () => ({ display: { grid: false, gridFrame: "icrs", gridDensity: 1, compass: false } }),
  useDisplayedImage: () => ({ path: null, dimensions: [2048, 2048], isProcessed: false, label: null, previewOnly: false, previewUrl: null }),
}));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => FILE }));

import OverlayLayer from "../OverlayLayer";
import { overlayStore } from "../../../utils/overlayStore";
import { FOOTPRINT_LAYER_ID, footprintLayer } from "../painters/footprintPainter";

const SQUARE = {
  datasetId: "jw02739-o001_t001_nircam",
  polygons: [
    [
      [0, 0],
      [10, 0],
      [10, 10],
    ] as [number, number][],
  ],
};

function render(enabled = true): string {
  return renderToStaticMarkup(
    createElement(OverlayLayer, {
      containerRef: { current: null },
      transform: { scale: 1, x: 0, y: 0 },
      renderW: 2048,
      renderH: 2048,
      fitsW: 2048,
      fitsH: 2048,
      enabled,
    }),
  );
}

afterEach(() => {
  overlayStore.remove(FILE, FOOTPRINT_LAYER_ID);
  overlayStore.remove(FILE, "gaia-catalog");
});

describe("viewer overlay canvas", () => {
  it("carries the driver hooks for the visible layers and the footprints", () => {
    overlayStore.add(FILE, { id: "gaia-catalog", kind: "catalog", visible: true, paint: () => {} });
    overlayStore.add(FILE, footprintLayer([SQUARE], SQUARE.datasetId));
    const canvas = render().match(/<canvas[^>]*>/)?.[0] ?? "";
    expect(canvas).toContain('data-testid="viewer-overlay"');
    expect(canvas).toContain('data-layers="gaia-catalog mast-footprints"');
    expect(canvas).toContain('data-footprints="1"');
    expect(canvas).toContain(`data-highlight="${SQUARE.datasetId}"`);
  });

  it("renders no canvas without a visible layer or when disabled", () => {
    expect(render()).toBe("");
    overlayStore.add(FILE, footprintLayer([SQUARE], null));
    expect(render(false)).toBe("");
  });
});
