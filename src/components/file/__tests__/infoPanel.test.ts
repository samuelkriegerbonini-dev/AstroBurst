import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { current } = vi.hoisted(() => ({
  current: { header: {} as Record<string, string> },
}));

vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({ file: { id: "f", name: "x.fits", path: "C:/heavy/x.fits", result: { header: current.header } } }),
  useHistContext: () => ({ histData: null, stfParams: { shadow: 0, midtone: 0.5, highlight: 1 } }),
}));
vi.mock("../../../hooks/useMousePixelStore", () => ({ useMousePixel: () => null }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({
  useAnalysisTarget: () => ({ path: null }),
  useMeasurementSource: () => null,
}));
vi.mock("../../header/WcsReadout", () => ({ default: () => null }));
vi.mock("../../header/PixelReadout", () => ({ default: () => null }));
vi.mock("../../analysis/MeasurementBadge", () => ({ default: () => null }));

import { InfoPanel } from "../SidebarPanels";

function renderWithHeader(header: Record<string, string>): string {
  current.header = header;
  return renderToStaticMarkup(createElement(InfoPanel));
}

describe("InfoPanel filter rows", () => {
  it("shows the PUPIL row next to the raw FILTER row of a NIRCam pupil-wheel exposure", () => {
    const html = renderWithHeader({ TELESCOP: "JWST", INSTRUME: "NIRCAM", FILTER: "F444W", PUPIL: "F470N" });
    expect(html).toContain("FILTER: F444W");
    expect(html).toContain("PUPIL: F470N");
  });

  it("shows an empty pupil slot as it is in the header", () => {
    const html = renderWithHeader({ INSTRUME: "NIRCAM", FILTER: "F444W", PUPIL: "CLEAR" });
    expect(html).toContain("FILTER: F444W");
    expect(html).toContain("PUPIL: CLEAR");
  });

  it("adds no PUPIL row when the header has no PUPIL", () => {
    const html = renderWithHeader({ INSTRUME: "MIRI", FILTER: "F770W" });
    expect(html).toContain("FILTER: F770W");
    expect(html).not.toContain("PUPIL");
  });
});
