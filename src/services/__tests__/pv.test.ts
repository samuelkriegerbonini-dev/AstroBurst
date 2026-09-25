import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { withPreviewMock } = vi.hoisted(() => ({ withPreviewMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: vi.fn(),
  withPreview: withPreviewMock,
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { computePvDiagram } from "../pv";
import type { PvDiagramResult } from "../../shared/types/pv";

describe("computePvDiagram", () => {
  beforeEach(() => {
    withPreviewMock.mockReset();
  });

  it("sends the slit, sampling, range and axis mode with camelCase keys", async () => {
    withPreviewMock.mockResolvedValue({ fits_path: "x" });
    await computePvDiagram("C:/d/cube.fits", "C:/out", {
      line: { x0: 2, y0: 16, x1: 29, y1: 16 },
      stepPx: 1,
      widthPx: 3,
      z0: 0,
      z1: 39,
      mode: "velocity",
      restUm: 1.02,
      convention: "optical",
      velocityShiftKms: -7.25,
    });
    expect(withPreviewMock).toHaveBeenCalledTimes(1);
    expect(withPreviewMock).toHaveBeenCalledWith("pv_diagram_cmd", "C:/out", {
      path: "C:/d/cube.fits",
      line: { x0: 2, y0: 16, x1: 29, y1: 16 },
      stepPx: 1,
      widthPx: 3,
      z0: 0,
      z1: 39,
      mode: "velocity",
      restUm: 1.02,
      convention: "optical",
      velocityShiftKms: -7.25,
    });
  });

  it("sends null for an absent rest wavelength and shift", async () => {
    withPreviewMock.mockResolvedValue({ fits_path: "x" });
    await computePvDiagram("cube.fits", undefined, {
      line: { x0: 0, y0: 0, x1: 5, y1: 5 },
      stepPx: 0.5,
      widthPx: 0,
      z0: 3,
      z1: 9,
      mode: "wavelength_vac",
      restUm: null,
      convention: "radio",
      velocityShiftKms: null,
    });
    expect(withPreviewMock).toHaveBeenCalledWith("pv_diagram_cmd", undefined, {
      path: "cube.fits",
      line: { x0: 0, y0: 0, x1: 5, y1: 5 },
      stepPx: 0.5,
      widthPx: 0,
      z0: 3,
      z1: 9,
      mode: "wavelength_vac",
      restUm: null,
      convention: "radio",
      velocityShiftKms: null,
    });
  });

  it("returns the resolved result untouched", async () => {
    const resolved = { fits_path: "C:/out/cube_pv.fits", previewUrl: "asset://pv.png", ridge: [1, null] };
    withPreviewMock.mockResolvedValue(resolved);
    const result = await computePvDiagram("cube.fits", "C:/out", {
      line: { x0: 0, y0: 0, x1: 5, y1: 5 },
      stepPx: 1,
      widthPx: 1,
      z0: 0,
      z1: 1,
      mode: "frequency",
      restUm: null,
      convention: "optical",
      velocityShiftKms: null,
    });
    expect(result).toBe(resolved);
  });

  it("types the ridge as nullable numbers", () => {
    expectTypeOf<PvDiagramResult["ridge"]>().toEqualTypeOf<(number | null)[]>();
    expectTypeOf<PvDiagramResult["spectral_convention"]>().toEqualTypeOf<"optical" | "radio" | "relativistic" | null>();
  });
});
