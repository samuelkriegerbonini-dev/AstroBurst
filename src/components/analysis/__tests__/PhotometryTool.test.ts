import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ANALYSIS_SECTION } from "../../../utils/analysisSections";

const IMAGE_PATH = "C:/data/m31.fits";
const panelProps = vi.hoisted(() => ({ photometry: null as Record<string, unknown> | null }));

vi.mock("../../../hooks/useAnalysisMeasure", () => ({
  useAnalysisMeasure: () => ({
    effectivePath: IMAGE_PATH,
    regionKey: IMAGE_PATH,
    measureKey: `${IMAGE_PATH}@1`,
    detectionScope: `image:${IMAGE_PATH}`,
    starsOnMeasuredImage: true,
  }),
}));
vi.mock("../../../hooks/useStarDetectionStore", () => ({ useStarDetection: () => ({ result: null }) }));
vi.mock("../PhotometryPanel", () => ({
  default: (props: Record<string, unknown>) => {
    panelProps.photometry = props;
    return "[photometry-panel]";
  },
}));
vi.mock("../PhotometryTablePanel", () => ({ default: () => "[photometry-table]" }));
vi.mock("../TimeSeriesPanel", () => ({ default: () => "[time-series]" }));

import PhotometryTool from "../PhotometryTool";

async function render(): Promise<string> {
  renderToStaticMarkup(createElement(PhotometryTool));
  await new Promise((resolve) => setTimeout(resolve, 0));
  return renderToStaticMarkup(createElement(PhotometryTool));
}

function sectionOf(html: string, id: string): string {
  const at = html.indexOf(`id="${id}"`);
  return at < 0 ? "" : html.slice(at, html.indexOf("</section>", at));
}

describe("PhotometryTool section chips", () => {
  it("shows the three section chips with photometry guidance", async () => {
    const html = await render();
    expect(html).toContain('aria-label="Photometry panels"');
    const nav = html.slice(html.indexOf("<nav"), html.indexOf("</nav>"));
    for (const section of [ANALYSIS_SECTION.photometry, ANALYSIS_SECTION.table, ANALYSIS_SECTION.series]) {
      expect(nav).toContain(`>${section.label}</button>`);
    }
    expect(nav).toContain(`data-section-help="${ANALYSIS_SECTION.photometry.id}"`);
    expect(nav).toContain('aria-label="Help: Aperture photometry"');
    expect(nav).not.toContain(`data-section-help="${ANALYSIS_SECTION.table.id}"`);
    expect(nav).not.toContain(`data-section-help="${ANALYSIS_SECTION.series.id}"`);
  });

  it("keeps the photometry tip slot inside the Photometry section, ahead of its panel", async () => {
    const section = sectionOf(await render(), ANALYSIS_SECTION.photometry.id);
    expect(section).toContain(`data-section-tip="${ANALYSIS_SECTION.photometry.id}"`);
    expect(section).toContain('aria-live="polite"');
    expect(section.indexOf("data-section-tip")).toBeLessThan(section.indexOf("[photometry-panel]"));
  });
});

describe("PhotometryTool panel inputs", () => {
  it("gives the single-star panel the measure key, so a file rewritten under the same path clears its result", async () => {
    panelProps.photometry = null;
    await render();
    expect(panelProps.photometry).toEqual({ filePath: IMAGE_PATH, measureKey: `${IMAGE_PATH}@1` });
  });
});
