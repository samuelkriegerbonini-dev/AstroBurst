import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("../../render/DeepZoomviewer", () => ({ default: () => null }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({ useMeasurementSource: () => null }));

import TileViewerPanel from "../TileViewerPanel";

function renderPanel(imageWidth: number, imageHeight: number): string {
  return renderToStaticMarkup(
    createElement(TileViewerPanel, {
      filePath: "C:/data/mosaic.fits",
      composite: false,
      rgbPath: null,
      imageWidth,
      imageHeight,
    }),
  );
}

describe("TileViewerPanel header", () => {
  it("renders the image size with a real multiplication sign", () => {
    const html = renderPanel(8000, 6000);
    expect(html).toContain("8000\u00d76000");
    expect(html).not.toContain("\\u00d7");
  });

  it("renders nothing for an image that does not need tiles", () => {
    expect(renderPanel(1024, 768)).toBe("");
  });
});
