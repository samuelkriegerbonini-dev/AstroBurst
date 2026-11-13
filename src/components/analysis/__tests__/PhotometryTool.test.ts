import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ANALYSIS_SECTION, toolSections } from "../../../utils/analysisSections";

const IMAGE_PATH = "C:/data/m31.fits";
const panelProps = vi.hoisted(() => ({
  photometry: null as Record<string, unknown> | null,
  forced: null as Record<string, unknown> | null,
  depth: null as Record<string, unknown> | null,
  resolution: null as Record<string, unknown> | null,
  compare: null as Record<string, unknown> | null,
}));

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
vi.mock("../ForcedPhotometryPanel", () => ({
  default: (props: Record<string, unknown>) => {
    panelProps.forced = props;
    return "[forced-panel]";
  },
}));
vi.mock("../DepthPanel", () => ({
  default: (props: Record<string, unknown>) => {
    panelProps.depth = props;
    return "[depth-panel]";
  },
}));
vi.mock("../ResolutionPanel", () => ({
  default: (props: Record<string, unknown>) => {
    panelProps.resolution = props;
    return "[resolution-panel]";
  },
}));
vi.mock("../CatalogComparePanel", () => ({
  default: (props: Record<string, unknown>) => {
    panelProps.compare = props;
    return "[compare-panel]";
  },
}));

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
  it("shows the section chips with guidance on Photometry, Depth, Resolution and Compare", async () => {
    const html = await render();
    expect(html).toContain('aria-label="Photometry panels"');
    const nav = html.slice(html.indexOf("<nav"), html.indexOf("</nav>"));
    const sections = toolSections("photometry", { hasHistogram: false, isCube: false, showFft: false, showDeepZoom: false });
    expect(sections).toHaveLength(7);
    for (const section of sections) {
      expect(nav).toContain(`>${section.label}</button>`);
    }
    const guided = [
      { id: ANALYSIS_SECTION.photometry.id, title: "Aperture photometry" },
      { id: ANALYSIS_SECTION.depth.id, title: "Image depth" },
      { id: ANALYSIS_SECTION.resolution.id, title: "Resolved or not" },
      { id: ANALYSIS_SECTION.compare.id, title: "Catalogue comparison" },
    ];
    for (const { id, title } of guided) {
      expect(nav).toContain(`data-section-help="${id}"`);
      expect(nav).toContain(`aria-label="Help: ${title}"`);
    }
    for (const section of [ANALYSIS_SECTION.table, ANALYSIS_SECTION.series, ANALYSIS_SECTION.forced]) {
      expect(nav).not.toContain(`data-section-help="${section.id}"`);
    }
  });

  it("keeps the photometry tip slot inside the Photometry section, ahead of its panel", async () => {
    const section = sectionOf(await render(), ANALYSIS_SECTION.photometry.id);
    expect(section).toContain(`data-section-tip="${ANALYSIS_SECTION.photometry.id}"`);
    expect(section).toContain('aria-live="polite"');
    expect(section.indexOf("data-section-tip")).toBeLessThan(section.indexOf("[photometry-panel]"));
  });

  it("mounts each survey panel in its own section in chip order", async () => {
    panelProps.forced = null;
    panelProps.depth = null;
    panelProps.resolution = null;
    panelProps.compare = null;
    const html = await render();
    const survey = [
      { section: ANALYSIS_SECTION.forced, marker: "[forced-panel]", props: () => panelProps.forced },
      { section: ANALYSIS_SECTION.depth, marker: "[depth-panel]", props: () => panelProps.depth },
      { section: ANALYSIS_SECTION.resolution, marker: "[resolution-panel]", props: () => panelProps.resolution },
      { section: ANALYSIS_SECTION.compare, marker: "[compare-panel]", props: () => panelProps.compare },
    ];
    let previous = html.indexOf(`id="${ANALYSIS_SECTION.series.id}"`);
    expect(previous).toBeGreaterThanOrEqual(0);
    for (const { section, marker, props } of survey) {
      expect(sectionOf(html, section.id)).toContain(marker);
      const at = html.indexOf(`id="${section.id}"`);
      expect(at, section.id).toBeGreaterThan(previous);
      previous = at;
      expect(props(), section.id).toEqual({ filePath: IMAGE_PATH, measureKey: `${IMAGE_PATH}@1` });
    }
    expect(html).not.toContain(`data-section-help="${ANALYSIS_SECTION.forced.id}"`);
  });

  it("keeps the depth, resolution and compare tip slots inside their sections, ahead of their panels", async () => {
    const html = await render();
    const slots = [
      { id: ANALYSIS_SECTION.depth.id, marker: "[depth-panel]" },
      { id: ANALYSIS_SECTION.resolution.id, marker: "[resolution-panel]" },
      { id: ANALYSIS_SECTION.compare.id, marker: "[compare-panel]" },
    ];
    for (const { id, marker } of slots) {
      const section = sectionOf(html, id);
      const tipAt = section.indexOf(`data-section-tip="${id}"`);
      expect(tipAt, id).toBeGreaterThanOrEqual(0);
      expect(tipAt, id).toBeLessThan(section.indexOf(marker));
    }
    expect(sectionOf(html, ANALYSIS_SECTION.forced.id)).not.toContain("data-section-tip");
  });
});

describe("PhotometryTool panel inputs", () => {
  it("gives the single-star panel the measure key, so a file rewritten under the same path clears its result", async () => {
    panelProps.photometry = null;
    await render();
    expect(panelProps.photometry).toEqual({ filePath: IMAGE_PATH, measureKey: `${IMAGE_PATH}@1` });
  });
});
