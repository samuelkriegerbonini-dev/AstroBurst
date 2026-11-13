import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { WriteSolvedWcsResult } from "../../../shared/types/astrometry";
import { SOLVED_WCS_LABEL, solvedWcsFor } from "../../../utils/solvedWcs";
import { targetProjectionPath } from "../../../utils/skyList";

interface CapturedPanelProps {
  filePath?: string | null;
  solvePath?: string | null;
  solveBadge?: ReactNode;
  onWcsWritten?: (result: WriteSolvedWcsResult) => void;
}

interface SkyPanelProps {
  filePath: string | null;
  measurePath?: string | null;
  measureKey?: string | null;
}

interface ChipProps {
  sections: readonly { id: string }[];
  tip?: { tips: Readonly<Record<string, { title: string; body: string } | undefined>> };
}

const { captured, publishProcessed, skyPanels, measureOverride, chips } = vi.hoisted(() => ({
  captured: { props: null as CapturedPanelProps | null },
  publishProcessed: vi.fn(),
  skyPanels: { catalog: null as SkyPanelProps | null, targets: null as SkyPanelProps | null },
  measureOverride: { value: {} as Record<string, unknown> },
  chips: { props: null as ChipProps | null },
}));

const STACK_PATH = "/out/stack.fits";
const FRAME_PATH = "/data/frame1.fits";
const X1D_PATH = "/data/jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits";

vi.mock("../../../hooks/useAnalysisMeasure", () => ({
  useAnalysisMeasure: () => ({
    target: {},
    effectivePath: STACK_PATH,
    compositeOnScreen: false,
    rgbPath: null,
    regionKey: FRAME_PATH,
    measureKey: `${STACK_PATH}@1`,
    targetWidth: 100,
    targetHeight: 80,
    filePath: FRAME_PATH,
    fileKey: "file-key",
    detectionScope: `image:${STACK_PATH}`,
    starsOnMeasuredImage: true,
    ...measureOverride.value,
  }),
}));
vi.mock("../../../hooks/useAnalysisTarget", () => ({
  useMeasurementSource: (measuresComposite: boolean) =>
    measuresComposite ? { text: "composite-variant", tone: "composite", title: "" } : { text: "Stack", tone: "processed", title: "" },
}));
vi.mock("../../../context/PreviewContext", () => ({
  useRenderActions: () => ({ publishProcessed }),
  useStarOverlayContext: () => ({ starOverlayRef: { current: null } }),
}));
vi.mock("../../../hooks/useStarDetectionStore", () => ({
  starDetectionStore: { begin: () => 0, commit: () => {}, fail: () => {} },
  useStarDetection: () => ({ scope: null, result: null, loading: false, error: null }),
}));
vi.mock("../../../services/analysis", () => ({ detectStars: async () => null, detectStarsComposite: async () => null }));
vi.mock("../SectionChips", () => ({
  default: (props: ChipProps) => {
    chips.props = props;
    return null;
  },
  SectionTipNote: ({ sectionId }: { sectionId: string }) => createElement("b", { "data-tip-note": sectionId }),
}));
vi.mock("../ObservationGeometryPanel", () => ({ default: () => null }));
vi.mock("../CatalogPanel", () => ({
  default: (props: SkyPanelProps) => {
    skyPanels.catalog = props;
    return null;
  },
}));
vi.mock("../TargetsPanel", () => ({
  default: (props: SkyPanelProps) => {
    skyPanels.targets = props;
    return null;
  },
}));
vi.mock("../PlateSolvePanel", () => ({
  default: (props: CapturedPanelProps) => {
    captured.props = props;
    return createElement("i", { "data-file-path": props.filePath, "data-solve-path": props.solvePath }, props.solveBadge);
  },
}));

import AstrometryTool from "../AstrometryTool";

async function renderTool(): Promise<string> {
  renderToStaticMarkup(createElement(AstrometryTool));
  await new Promise((resolve) => setTimeout(resolve, 0));
  return renderToStaticMarkup(createElement(AstrometryTool));
}

describe("AstrometryTool plate solve target", () => {
  it("keeps the selected file as the panel identity and solves the measured image, with its badge", async () => {
    const html = await renderTool();
    expect(html).toContain(`data-file-path="${FRAME_PATH}"`);
    expect(html).toContain(`data-solve-path="${STACK_PATH}"`);
    expect(html).toMatch(/data-solve-path="[^"]*"><span[^>]*>Stack<\/span><\/i>/);
    expect(html).not.toContain("composite-variant");
  });

  it("records the measured image as the input of the published WCS file", async () => {
    await renderTool();
    publishProcessed.mockClear();
    captured.props?.onWcsWritten?.({
      fits_path: "/out/stack_wcs.fits",
      png_path: "/out/stack_wcs.png",
      previewUrl: "asset://stack_wcs.png",
      dimensions: [100, 80],
      center_ra: 83.82,
      center_dec: -5.39,
      pixel_scale_arcsec: 2,
      sip_present: false,
      elapsed_ms: 5,
    });
    expect(publishProcessed).toHaveBeenCalledWith(
      "file-key",
      expect.objectContaining({ fitsPath: "/out/stack_wcs.fits", label: SOLVED_WCS_LABEL, inputPath: STACK_PATH }),
    );
  });

  it("remembers the written WCS for the solved input, for SPCC to find on that grid", async () => {
    await renderTool();
    captured.props?.onWcsWritten?.({
      fits_path: "/out/stack_wcs_spcc.fits",
      png_path: "/out/stack_wcs_spcc.png",
      dimensions: [100, 80],
      center_ra: 83.82,
      center_dec: -5.39,
      pixel_scale_arcsec: 2,
      sip_present: false,
      elapsed_ms: 5,
    });
    expect(solvedWcsFor(STACK_PATH)).toBe("/out/stack_wcs_spcc.fits");
    expect(solvedWcsFor(FRAME_PATH)).toBeNull();
  });
});

describe("AstrometryTool section tips", () => {
  const STARS = "analysis-stars";
  const CATALOG = "analysis-catalog";
  const TARGETS = "analysis-targets";

  it("guides the stars, solve and WCS flow, then the catalog and targets that depend on it", async () => {
    await renderTool();
    const tips = chips.props?.tip?.tips;
    expect(chips.props?.sections.map((s) => s.id)).toEqual([STARS, "analysis-geometry", CATALOG, TARGETS]);
    expect(tips?.[STARS]?.title).toBe("Detect stars and solve the field");
    for (const step of ["Detect Stars", "Plate Solve", "Write WCS to image"]) expect(tips?.[STARS]?.body).toContain(step);
    expect(tips?.[STARS]?.body.indexOf("Detect Stars")).toBeLessThan(tips?.[STARS]?.body.indexOf("Plate Solve") ?? -1);
    expect(tips?.[STARS]?.body.indexOf("Plate Solve")).toBeLessThan(tips?.[STARS]?.body.indexOf("Write WCS to image") ?? -1);
    expect(tips?.[CATALOG]?.title).toBe("Compare with Gaia DR3");
    for (const step of ["WCS", "Search Gaia DR3", "Cross-match detected stars"]) expect(tips?.[CATALOG]?.body).toContain(step);
    expect(tips?.[TARGETS]?.title).toBe("Place targets on the image");
    for (const step of ["WCS", "one per line", "Place"]) expect(tips?.[TARGETS]?.body).toContain(step);
    expect(tips?.["analysis-geometry"]).toBeUndefined();
  });

  it("says Plate Solve and the cross-match find their own sources, so Detect Stars feeds neither", async () => {
    const { DEFAULT_DETECTION_SIGMA, DEFAULT_MAX_STARS } = await vi.importActual<typeof import("../CatalogPanel")>("../CatalogPanel");
    await renderTool();
    const tips = chips.props?.tip?.tips;
    expect(tips?.[STARS]?.body).toContain("Plate Solve fits sky coordinates to the image and finds its own sources");
    expect(tips?.[STARS]?.body).not.toMatch(/from them|your detections/);
    expect(tips?.[CATALOG]?.body).toContain(
      `Cross-match detected stars finds sources in the image (${DEFAULT_DETECTION_SIGMA} sigma, up to ${DEFAULT_MAX_STARS}) and pairs them with Gaia DR3`,
    );
    expect(tips?.[CATALOG]?.body).not.toContain("your detections");
  });

  it("renders each tip slot inside its own section", async () => {
    const html = await renderTool();
    for (const id of [STARS, CATALOG, TARGETS]) {
      const at = html.indexOf(`id="${id}"`);
      const note = html.indexOf(`data-tip-note="${id}"`);
      expect(at).toBeGreaterThanOrEqual(0);
      expect(note).toBeGreaterThan(at);
      expect(note).toBeLessThan(html.indexOf("</section>", at));
    }
    expect(html).not.toContain('data-tip-note="analysis-geometry"');
  });
});

describe("AstrometryTool catalog and targets panels", () => {
  afterEach(() => {
    measureOverride.value = {};
    skyPanels.catalog = null;
    skyPanels.targets = null;
  });

  it("gives them no file and no projection for a selected 1D spectrum, which has no image to project on", async () => {
    measureOverride.value = { effectivePath: null, regionKey: X1D_PATH, filePath: X1D_PATH, measureKey: null, detectionScope: "image:" };
    await renderTool();
    for (const panel of [skyPanels.catalog, skyPanels.targets]) {
      expect(panel).not.toBeNull();
      expect(panel?.filePath).toBeNull();
      expect(targetProjectionPath(panel?.filePath ?? null, panel?.measurePath ?? null)).toBeNull();
    }
  });

  it("keeps the selected file as their identity and projects through the measured image (control)", async () => {
    await renderTool();
    for (const panel of [skyPanels.catalog, skyPanels.targets]) {
      expect(panel?.filePath).toBe(FRAME_PATH);
      expect(panel?.measurePath).toBe(STACK_PATH);
    }
  });

  it("gives the catalog panel the version key of the measured image, so a step that rewrites the same path clears the cross-match", async () => {
    await renderTool();
    expect(skyPanels.catalog?.measureKey).toBe(`${STACK_PATH}@1`);
  });
});
