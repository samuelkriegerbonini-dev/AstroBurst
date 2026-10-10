import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

interface TargetState {
  path: string | null;
  dimensions: [number, number] | null;
  previewOnly: boolean;
  label: string | null;
}

const { target } = vi.hoisted(() => ({
  target: { path: null, dimensions: null, previewOnly: false, label: null } as TargetState,
}));

const CUBE_PATH = "/data/cube_s3d.fits";
const CUBE_GRID: [number, number] = [53, 55];

vi.mock("../../context/PreviewContext", () => ({
  useFileContext: () => ({
    file: { id: "f", name: "cube_s3d.fits", path: CUBE_PATH, result: { dimensions: CUBE_GRID, header: {}, elapsed_ms: 10 } },
  }),
  useHistContext: () => ({ histData: null, stfParams: { shadow: 0, midtone: 0.5, highlight: 1 } }),
}));
vi.mock("../../hooks/useMousePixelStore", () => ({ useMousePixel: () => ({ x: 20, y: 30 }) }));
vi.mock("../../hooks/useAnalysisTarget", () => ({
  useAnalysisTarget: () => ({
    composite: false,
    fileRgbView: false,
    rgbPath: null,
    path: target.path,
    dimensions: target.dimensions,
    displayed: {
      path: target.path,
      dimensions: target.dimensions,
      isProcessed: target.label !== null,
      label: target.label,
      previewOnly: target.previewOnly,
      previewUrl: "asset://shown.png",
    },
    fileIsRgb: false,
    fileName: "cube_s3d.fits",
  }),
  useMeasurementSource: () => (target.label ? { text: target.label, tone: "processed", title: "" } : null),
}));
vi.mock("../header/WcsReadout", () => ({
  default: (props: { filePath: string | null; imageWidth: number; imageHeight: number }) =>
    createElement("i", { "data-wcs-path": props.filePath, "data-w": props.imageWidth, "data-h": props.imageHeight }),
}));
vi.mock("../header/PixelReadout", () => ({ default: () => null }));
vi.mock("../analysis/MeasurementBadge", () => ({ default: () => null }));

import { InfoPanel } from "../file/SidebarPanels";

function render(view: Partial<TargetState>): string {
  Object.assign(target, { path: CUBE_PATH, dimensions: CUBE_GRID, previewOnly: false, label: null }, view);
  return renderToStaticMarkup(createElement(InfoPanel));
}

describe("InfoPanel sky readout follows the analysis target", () => {
  it("probes the PV diagram on screen with its own grid, not the cube it was cut from", () => {
    const html = render({ path: "/out/pv.fits", dimensions: [40, 3814], label: "PV diagram" });
    expect(html).toContain('data-wcs-path="/out/pv.fits"');
    expect(html).toContain('data-w="40"');
    expect(html).toContain('data-h="3814"');
  });

  it("probes the loaded cube with the cube grid", () => {
    const html = render({});
    expect(html).toContain(`data-wcs-path="${CUBE_PATH}"`);
    expect(html).toContain('data-w="53"');
  });

  it("hides the readout for a PNG-only result on another grid", () => {
    expect(render({ dimensions: [800, 600], previewOnly: true, label: "Stretch" })).not.toContain("data-wcs-path");
  });

  it("hides the readout without a measured path", () => {
    expect(render({ path: null, dimensions: null })).not.toContain("data-wcs-path");
  });
});
