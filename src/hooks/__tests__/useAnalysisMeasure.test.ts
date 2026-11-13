import { beforeEach, describe, expect, it, vi } from "vitest";
import { hooks } from "../../test/hookRuntime";
import { useAnalysisMeasure } from "../useAnalysisMeasure";

const P = "C:/data/l2_r.fits";

const view = vi.hoisted(() => ({
  file: { path: "C:/data/l2_r.fits", reloads: 0 },
  target: { path: "C:/data/l2_r.fits", composite: false, rgbPath: null, dimensions: [512, 512] },
}));

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../context/PreviewContext", () => ({
  fileKeyOf: () => "k",
  useFileContext: () => ({ file: view.file }),
  useRenderContext: () => ({ processed: null, processedVersion: 0 }),
}));
vi.mock("../../context/CompositeContext", () => ({ useCompositePreview: () => ({ compositeVersion: 0 }) }));
vi.mock("../useAnalysisTarget", () => ({ useAnalysisTarget: () => view.target }));
vi.mock("../useRegionKey", () => ({ useRegionKey: () => P }));

const read = () => hooks.render(useAnalysisMeasure as never, undefined) as { measureKey: string | null; detectionScope: string };

beforeEach(() => {
  hooks.mount();
  view.file = { path: P, reloads: 0 };
});

describe("useAnalysisMeasure", () => {
  it("gives a new measure key and a new detection scope after the file is read again", () => {
    const first = read();
    expect(first.measureKey).toBe("C:/data/l2_r.fits");
    view.file = { ...view.file, reloads: 1 };
    const second = read();
    expect(second.measureKey).toBe("C:/data/l2_r.fits#1");
    expect(second.detectionScope).not.toBe(first.detectionScope);
  });

  it("keeps the key while nothing was read again", () => {
    expect(read()).toBe(read());
  });
});
