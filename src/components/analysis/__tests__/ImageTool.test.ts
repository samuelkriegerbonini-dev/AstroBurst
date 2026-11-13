import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hooks } from "../../../test/hookRuntime";
import { findElement } from "../../../test/elementTree";

type Fn = (props: Record<string, unknown>) => unknown;

const FILE = "C:/data/jw_i2d.fits";

const view = vi.hoisted(() => ({
  file: null as unknown as { path: string; result: { header: Record<string, unknown>; is_rgb: boolean } },
  hist: null as unknown as Record<string, unknown>,
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({ file: view.file }),
  useHistContext: () => view.hist,
  useCubeContext: () => ({ isCube: false, ramp: null }),
  useRenderContext: () => ({ processed: null }),
  useRawPixelsContext: () => ({ rgbRawPixels: null }),
  useDisplayContext: () => ({ display: { stretch: "mtf" } }),
  useDqContext: () => ({ excludeDq: false }),
}));
vi.mock("../../../context/ToolHostContext", () => ({
  useToolHost: () => ({ gpuDisplay: false, active: true }),
}));
vi.mock("../../../hooks/useAnalysisMeasure", () => ({
  useAnalysisMeasure: () => ({
    target: { displayed: { previewOnly: false }, fileRgbView: false },
    effectivePath: FILE,
    compositeOnScreen: false,
    rgbPath: null,
    regionKey: FILE,
    measureKey: "k",
    targetWidth: 512,
    targetHeight: 512,
    filePath: FILE,
  }),
}));
vi.mock("../useSectionTip", () => ({ useSectionTip: () => null }));
vi.mock("../../../services/analysis", () => ({ computeFftSpectrum: vi.fn(), computeHistogram: vi.fn() }));

import ImageTool from "../ImageTool";
import HistogramPanel from "../HistogramPanel";

function histogramProps(): Record<string, unknown> {
  const tree = hooks.render((ImageTool as unknown as { type: Fn }).type, {});
  const panel = findElement(tree, (el) => el.type === HistogramPanel);
  expect(panel).toBeDefined();
  return panel!.props as Record<string, unknown>;
}

beforeEach(() => {
  view.file = { path: FILE, result: { header: { BUNIT: "MJy/sr" }, is_rgb: false } };
  view.hist = {
    histData: { bins: [1, 4, 9, 4, 1], data_min: 0, data_max: 100, mean: 0.31, median: 0.214, sigma: 0.0081, auto_stf: null },
    histDataPath: FILE,
    stfParams: { shadow: 0, midtone: 0.5, highlight: 1 },
    setStfParams: () => {},
  };
});

afterEach(() => {
  hooks.mount();
});

describe("ImageTool histogram unit", () => {
  it("passes the data unit of the file to its histogram", () => {
    expect(histogramProps().unit).toBe("MJy/sr");
  });

  it("passes no unit while the histogram is of a product", () => {
    view.hist.histDataPath = "C:/out/jw_i2d_bg.fits";
    expect(histogramProps().unit).toBeNull();
  });

  it("passes no unit for a header without BUNIT", () => {
    view.file.result.header = {};
    expect(histogramProps().unit).toBeNull();
  });
});
