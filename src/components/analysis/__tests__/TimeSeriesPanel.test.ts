import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { GAIN_TITLE } from "../../../utils/photometryGain";
import timeSeriesPanelSource from "../TimeSeriesPanel.tsx?raw";

vi.mock("../../../context/PreviewContext", () => ({
  useDqContext: () => ({ excludeDq: false }),
  useFileContext: () => ({ file: { path: "C:/data/502nmos.fits", result: { dimensions: [1600, 1600] } } }),
  useRenderContext: () => ({ processed: null }),
}));
vi.mock("../../../hooks/useRegionStore", () => ({ useRegionDoc: () => ({ regions: [] }) }));
vi.mock("../../../hooks/useProgress", () => ({ useProgress: () => ({ percent: 0, active: false, reset: () => {} }) }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({
  useAnalysisTarget: () => ({ composite: false, displayed: { isProcessed: false, label: "502nmos.fits" } }),
}));
vi.mock("../../../hooks/useFrameSet", () => ({
  useMatchingFrames: () => [],
  orderWithReferenceFirst: () => [],
  pathOf: (f: { path: string }) => f.path,
}));
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { selectFile: () => {} } }));

const { default: TimeSeriesPanel } = await import("../TimeSeriesPanel");

function render(): string {
  return renderToStaticMarkup(createElement(TimeSeriesPanel, { filePath: "C:/data/502nmos.fits" }));
}

describe("TimeSeriesPanel fields (followups-9, followups-10, science0925-19)", () => {
  it("renders the aperture, sky and gain fields as decimal text boxes, never native number inputs", () => {
    const markup = render();
    expect(markup).not.toContain('type="number"');
    expect(markup.match(/inputMode="decimal"/g)).toHaveLength(4);
    expect(markup).toContain('value="5"');
  });

  it("marks the gain field with the photometry gain title and shows the header-gain caption slot under it", () => {
    const markup = render();
    expect(markup).toContain(`title="${GAIN_TITLE}"`);
    expect(markup).toContain("data-gain-source");
    expect(markup).toContain("data-gain-caption");
  });
});

describe("TimeSeriesPanel table cells (UX-07)", () => {
  it("prints the sky level through the shared flux formatter", () => {
    expect(timeSeriesPanelSource).toMatch(/data-column="bg_mean">\s*\{fluxCellText\(target\?\.bg_mean\)\}/);
  });

  it("writes the differential magnitude error with ± on one line, as the photometry table does", () => {
    expect(timeSeriesPanelSource.match(/\+\/-/g) ?? []).toEqual([]);
    const cell = timeSeriesPanelSource.match(
      /<td className="([^"]*)">\s*\{point && point\.mag !== null \? `\$\{fmt\(point\.mag, 4\)\} ± \$\{fmt\(point\.err, 4\)\}` : "--"\}/,
    );
    expect(cell).not.toBeNull();
    expect(cell![1].split(" ")).toContain("whitespace-nowrap");
  });
});
