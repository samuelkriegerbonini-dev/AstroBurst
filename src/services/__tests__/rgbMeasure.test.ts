import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDirTiles: vi.fn(async () => "/out/tiles"),
}));

import { detectStarsComposite } from "../analysis";
import { computeStatisticsComposite } from "../statistics";
import { generateTilesRgb } from "../tiles";

const RGB = "D:/data/M31_osc.fits";

describe("composite measurements of an RGB FITS", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue({});
  });

  it("statistics name the selected RGB file so the backend measures its planes, not the shared slot", async () => {
    await computeStatisticsComposite(true, RGB);
    expect(typedInvokeMock).toHaveBeenCalledWith("compute_statistics_composite_cmd", { noise: true, path: RGB });
  });

  it("star detection names the selected RGB file", async () => {
    await detectStarsComposite(4, 150, RGB);
    expect(typedInvokeMock).toHaveBeenCalledWith("detect_stars_composite", { sigma: 4, maxStars: 150, path: RGB });
  });

  it("deep zoom tiles name the selected RGB file", async () => {
    await generateTilesRgb("/out/tiles/0", 256, RGB);
    expect(typedInvokeMock).toHaveBeenCalledWith("generate_tiles_rgb", { outputDir: "/out/tiles/0", tileSize: 256, path: RGB });
  });

  it("sends no path for the wizard composite", async () => {
    await computeStatisticsComposite(false);
    expect(typedInvokeMock).toHaveBeenCalledWith("compute_statistics_composite_cmd", { noise: false, path: null });
  });
});
