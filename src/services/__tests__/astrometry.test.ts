import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock, withPreviewMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn(), withPreviewMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock, withPreview: withPreviewMock }));

import {
  GAIA_REFINE_PROGRESS_EVENT,
  LOCAL_NO_SOLUTION_CODE,
  PLATE_SOLVE_PROGRESS_EVENT,
  cancelGaiaRefine,
  cancelPlateSolve,
  getSolveHints,
  plateSolve,
  refineWithGaia,
  skySeparation,
  worldToPixel,
  writeSolvedWcs,
  type PlateSolveResult,
} from "../astrometry";

describe("PlateSolveResult", () => {
  it("declares the SolveResult fields plate_solve_cmd serializes, the local solver's with serde(default), and the credit it adds", () => {
    expectTypeOf<keyof PlateSolveResult>().toEqualTypeOf<
      | "center_ra"
      | "center_dec"
      | "orientation"
      | "pixel_scale_arcsec"
      | "field_of_view_w_arcmin"
      | "field_of_view_h_arcmin"
      | "wcs_cards"
      | "annotations"
      | "solver"
      | "tier"
      | "catalog"
      | "matched"
      | "predicted"
      | "rms_px"
      | "sip_order"
      | "sip_max_err_px"
      | "hint_source"
      | "elapsed_ms"
      | "parity"
      | "warnings"
      | "credit"
    >();
    expectTypeOf<PlateSolveResult["credit"]>().toEqualTypeOf<{ text: string; citation: string } | null | undefined>();
    expectTypeOf<PlateSolveResult["wcs_cards"]>().toEqualTypeOf<[string, string][]>();
    expectTypeOf<PlateSolveResult["orientation"]>().toEqualTypeOf<number | null | undefined>();
  });
});

describe("plateSolve", () => {
  beforeEach(() => typedInvokeMock.mockReset().mockResolvedValue({}));

  it("routes to the local solver with its near-search radius when the panel asks for it", async () => {
    await plateSolve("C:/d/light.fits", { solver: "local", centerRa: 274.7, centerDec: -13.82, radius: 2.5, nearRadiusDeg: 2.5 });
    expect(typedInvokeMock).toHaveBeenCalledWith("plate_solve_cmd", {
      path: "C:/d/light.fits",
      apiKey: null,
      scaleLower: null,
      scaleUpper: null,
      scaleUnits: null,
      downsampleFactor: null,
      centerRa: 274.7,
      centerDec: -13.82,
      radius: 2.5,
      solver: "local",
      nearRadiusDeg: 2.5,
    });
  });

  it("leaves the solver and the near radius to the command's defaults when they are not given", async () => {
    await plateSolve("C:/d/light.fits");
    expect(typedInvokeMock.mock.calls[0][1]).toMatchObject({ solver: null, nearRadiusDeg: null });
  });

  it("names the error code of a local no-solution", () => {
    expect(LOCAL_NO_SOLUTION_CODE).toBe("local_no_solution");
  });
});

describe("getSolveHints", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("invokes solve_hints_cmd with the image path", async () => {
    const hints = { centre: { ra: 274.7, dec: -13.8, source: "OBJCTRA/OBJCTDEC" }, scale: null, warnings: [] };
    typedInvokeMock.mockResolvedValue(hints);
    await expect(getSolveHints("C:/d/light.fits")).resolves.toBe(hints);
    expect(typedInvokeMock).toHaveBeenCalledWith("solve_hints_cmd", { path: "C:/d/light.fits" });
  });
});

describe("cancelPlateSolve", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("cancels the running solve through the plate-solve-progress event plate_solve_cmd registers", async () => {
    typedInvokeMock.mockResolvedValueOnce(true);
    await expect(cancelPlateSolve()).resolves.toBe(true);
    expect(PLATE_SOLVE_PROGRESS_EVENT).toBe("plate-solve-progress");
    expect(typedInvokeMock).toHaveBeenCalledWith("cancel_progress_cmd", { event: "plate-solve-progress" });
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

describe("refineWithGaia", () => {
  beforeEach(() => typedInvokeMock.mockReset().mockResolvedValue({}));

  it("pins gaia_refine_wcs_cmd with the solve's cards, a typed epoch and the model", async () => {
    await refineWithGaia("C:/d/light.fits", { wcsCards: [["CRVAL1", "80.65"]], epochYear: 1990.5, model: "rshift" });
    expect(typedInvokeMock).toHaveBeenCalledWith("gaia_refine_wcs_cmd", {
      path: "C:/d/light.fits",
      wcsCards: [["CRVAL1", "80.65"]],
      epochYear: 1990.5,
      model: "rshift",
    });
  });

  it("lets the command read the file's own WCS, the header epoch and the default model when nothing is given", async () => {
    await refineWithGaia("C:/d/light.fits", { wcsCards: [] });
    expect(typedInvokeMock).toHaveBeenCalledWith("gaia_refine_wcs_cmd", {
      path: "C:/d/light.fits",
      wcsCards: null,
      epochYear: null,
      model: null,
    });
  });
});

describe("writeSolvedWcs with a Gaia DR3 tie", () => {
  beforeEach(() => withPreviewMock.mockReset().mockResolvedValue({}));

  it("passes the tie's HISTORY lines for the written copy", async () => {
    await writeSolvedWcs("C:/d/light.fits", [["CRVAL1", "80.65"]], undefined, ["Gaia DR3 tie: rshift"]);
    expect(withPreviewMock).toHaveBeenCalledWith("write_solved_wcs_cmd", undefined, {
      path: "C:/d/light.fits",
      wcsCards: [["CRVAL1", "80.65"]],
      gaiaHistory: ["Gaia DR3 tie: rshift"],
    });
  });

  it("sends no HISTORY argument for a plain solve", async () => {
    await writeSolvedWcs("C:/d/light.fits", [], undefined, []);
    expect(withPreviewMock).toHaveBeenCalledWith("write_solved_wcs_cmd", undefined, { path: "C:/d/light.fits", wcsCards: [] });
  });

  it("passes the solve's catalogue so a Gaia-tier solve is credited in HISTORY", async () => {
    await writeSolvedWcs("C:/d/light.fits", [["CRVAL1", "80.65"]], undefined, null, "tier1-gaia-siril");
    expect(withPreviewMock).toHaveBeenCalledWith("write_solved_wcs_cmd", undefined, {
      path: "C:/d/light.fits",
      wcsCards: [["CRVAL1", "80.65"]],
      solveCatalog: "tier1-gaia-siril",
    });
    withPreviewMock.mockClear();
    await writeSolvedWcs("C:/d/light.fits", [["CRVAL1", "80.65"]], undefined, null, null);
    expect(withPreviewMock.mock.calls[0][2]).not.toHaveProperty("solveCatalog");
  });
});

describe("cancelGaiaRefine", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("cancels a running refinement through the gaia-refine-progress event gaia_refine_wcs_cmd registers", async () => {
    typedInvokeMock.mockResolvedValueOnce(true);
    expect(GAIA_REFINE_PROGRESS_EVENT).toBe("gaia-refine-progress");
    await expect(cancelGaiaRefine()).resolves.toBe(true);
    expect(typedInvokeMock).toHaveBeenCalledWith("cancel_progress_cmd", { event: "gaia-refine-progress" });
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
