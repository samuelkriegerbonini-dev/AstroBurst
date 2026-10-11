import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("../../../services/tiles", () => ({ generateTiles: vi.fn(), generateTilesRgb: vi.fn() }));
vi.mock("../../../infrastructure/tauri", () => ({ getOutputDirTiles: vi.fn(), getPreviewUrl: vi.fn() }));

import DeepZoomViewer, { DEEP_ZOOM_OVERLAYS_NOTICE } from "../DeepZoomviewer";

describe("Deep Zoom overlay notice", () => {
  it("says that the grid, regions and apertures are not drawn", () => {
    expect(DEEP_ZOOM_OVERLAYS_NOTICE).toBe(
      "Deep Zoom shows the pixels only: the grid, regions, apertures and other overlays are not drawn here.",
    );
  });

  it("shows the notice on the tile view", () => {
    const html = renderToStaticMarkup(
      createElement(DeepZoomViewer, { filePath: "C:/data/f444w_i2d.fits", composite: false, imageWidth: 9000, imageHeight: 9000 }),
    );
    expect(html).toContain(DEEP_ZOOM_OVERLAYS_NOTICE);
    expect(html).toContain('role="note"');
  });
});
