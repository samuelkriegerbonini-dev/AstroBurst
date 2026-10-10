import { describe, expect, it, vi } from "vitest";
import { createElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { WriteSolvedWcsResult } from "../../../shared/types/astrometry";
import { SOLVED_WCS_LABEL } from "../../../utils/solvedWcs";

interface CapturedPanelProps {
  filePath?: string | null;
  solvePath?: string | null;
  solveBadge?: ReactNode;
  onWcsWritten?: (result: WriteSolvedWcsResult) => void;
}

const { captured, publishProcessed } = vi.hoisted(() => ({
  captured: { props: null as CapturedPanelProps | null },
  publishProcessed: vi.fn(),
}));

const STACK_PATH = "/out/stack.fits";
const FRAME_PATH = "/data/frame1.fits";

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
vi.mock("../SectionChips", () => ({ default: () => null }));
vi.mock("../ObservationGeometryPanel", () => ({ default: () => null }));
vi.mock("../CatalogPanel", () => ({ default: () => null }));
vi.mock("../TargetsPanel", () => ({ default: () => null }));
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
});
