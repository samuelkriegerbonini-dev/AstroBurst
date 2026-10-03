import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock, withPreviewMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn(), withPreviewMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock, withPreview: withPreviewMock }));

import { skySeparation, worldToPixel, writeSolvedWcs, type PlateSolveResult } from "../astrometry";

describe("PlateSolveResult", () => {
  it("declares only the SolveResult fields plate_solve_cmd still serializes", () => {
    expectTypeOf<keyof PlateSolveResult>().toEqualTypeOf<
      | "center_ra"
      | "center_dec"
      | "orientation"
      | "pixel_scale_arcsec"
      | "field_of_view_w_arcmin"
      | "field_of_view_h_arcmin"
      | "wcs_cards"
    >();
    expectTypeOf<PlateSolveResult["wcs_cards"]>().toEqualTypeOf<[string, string][]>();
  });
});

describe("writeSolvedWcs", () => {
  beforeEach(() => withPreviewMock.mockReset());

  it("writeSolvedWcs pins write_solved_wcs_cmd", async () => {
    const result = { fits_path: "C:/out/light_wcs.fits", png_path: "C:/out/light_wcs.png" };
    withPreviewMock.mockResolvedValue(result);
    const res = await writeSolvedWcs("C:/d/light.fits", [["CTYPE1", "RA---TAN"]]);
    expect(withPreviewMock).toHaveBeenCalledWith("write_solved_wcs_cmd", undefined, {
      path: "C:/d/light.fits",
      wcsCards: [["CTYPE1", "RA---TAN"]],
    });
    expect(res).toBe(result);
  });

  it("passes an explicit output folder through", async () => {
    withPreviewMock.mockResolvedValue({});
    await writeSolvedWcs("C:/d/light.fits", [], "D:/keep");
    expect(withPreviewMock).toHaveBeenCalledWith("write_solved_wcs_cmd", "D:/keep", { path: "C:/d/light.fits", wcsCards: [] });
  });
});

describe("worldToPixel", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("invokes world_to_pixel_cmd with the path, the points and the frame", async () => {
    const result = { points: [[1, 2]], on_image: [true], frame: "galactic", naxis1: 10, naxis2: 10 };
    typedInvokeMock.mockResolvedValue(result);
    const res = await worldToPixel("/a.fits", [[236.95, 41.9]], "galactic");
    expect(typedInvokeMock).toHaveBeenCalledWith("world_to_pixel_cmd", {
      path: "/a.fits",
      points: [[236.95, 41.9]],
      frame: "galactic",
    });
    expect(res).toBe(result);
  });

  it("defaults the frame to icrs", async () => {
    typedInvokeMock.mockResolvedValue({ points: [], on_image: [], frame: "icrs", naxis1: 0, naxis2: 0 });
    await worldToPixel("/a.fits", []);
    expect(typedInvokeMock).toHaveBeenCalledWith("world_to_pixel_cmd", { path: "/a.fits", points: [], frame: "icrs" });
  });
});

describe("skySeparation", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("invokes sky_separation_cmd in pixel mode with the file path", async () => {
    typedInvokeMock.mockResolvedValue({ separation_arcsec: 10 });
    await skySeparation("/a.fits", [10, 20], [10, 30], true);
    expect(typedInvokeMock).toHaveBeenCalledWith("sky_separation_cmd", {
      path: "/a.fits",
      a: [10, 20],
      b: [10, 30],
      pixel: true,
    });
  });

  it("invokes sky_separation_cmd in sky mode with a null path", async () => {
    typedInvokeMock.mockResolvedValue({ separation_arcsec: 10 });
    await skySeparation(null, [10, 20], [11, 21], false);
    expect(typedInvokeMock).toHaveBeenCalledWith("sky_separation_cmd", {
      path: null,
      a: [10, 20],
      b: [11, 21],
      pixel: false,
    });
  });
});
