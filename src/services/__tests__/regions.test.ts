import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock }));

import { exportRegions, lineCut, radialProfile, regionStats, sbProfile } from "../regions";
import {
  REGION_SYSTEMS,
  type LineCut,
  type RadialProfile,
  type RegionCalibrated,
  type RegionSky,
  type RegionStats,
  type RegionStatsResult,
  type SbBin,
  type SbProfile,
} from "../../shared/types/regions";
import type { PhotCal } from "../analysis";

describe("region export systems", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue({ reg_text: "", system: "physical" });
  });

  it("offers every system RegionSystem::parse accepts, including physical for cutouts with LTV/LTM", () => {
    expect(REGION_SYSTEMS).toEqual(["image", "physical", "fk5", "icrs"]);
  });

  it("sends physical to the export command", async () => {
    await exportRegions("/a.fits", [], "physical");
    expect(typedInvokeMock).toHaveBeenCalledWith("regions_export_cmd", {
      path: "/a.fits",
      regions: [],
      system: "physical",
      sexagesimal: true,
    });
  });
});

describe("RegionStats payload", () => {
  it("types the clipped statistics as nullable, since an all-rejected clip serializes NaN as null", () => {
    expectTypeOf<RegionStats["clipped_mean"]>().toEqualTypeOf<number | null>();
    expectTypeOf<RegionStats["clipped_median"]>().toEqualTypeOf<number | null>();
    expectTypeOf<RegionStats["clipped_sigma"]>().toEqualTypeOf<number | null>();
    expectTypeOf<RegionStats>().toHaveProperty("n_padding");
  });
});

describe("regionStats", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue({ regions: [], masked: false, dq_excluded: null, region_excluded: 0, elapsed_ms: 1 });
  });

  it("forwards sigma and maxiters to the command", async () => {
    const shape = { shape: "circle" as const, x: 1, y: 2, r: 3 };
    await regionStats("/a.fits", [{ id: "r1", shape }], { excludeDq: true, sigma: 2.5, maxiters: 7 });
    expect(typedInvokeMock).toHaveBeenCalledWith("region_stats_cmd", {
      path: "/a.fits",
      regions: [{ id: "r1", shape, background: null }],
      excludeDq: true,
      sigma: 2.5,
      maxiters: 7,
      exclude: [],
    });
  });

  it("sends null for sigma and maxiters when they are unset so the backend defaults apply", async () => {
    await regionStats("/a.fits", []);
    expect(typedInvokeMock).toHaveBeenCalledWith("region_stats_cmd", {
      path: "/a.fits",
      regions: [],
      excludeDq: false,
      sigma: null,
      maxiters: null,
      exclude: [],
    });
  });

  it("sends the exclude shapes so their pixels are left out of every region", async () => {
    const shape = { shape: "circle" as const, x: 32, y: 32, r: 20 };
    const hole = { shape: "circle" as const, x: 30, y: 30, r: 5 };
    await regionStats("/a.fits", [{ id: "r1", shape }], { exclude: [hole] });
    expect(typedInvokeMock).toHaveBeenCalledWith("region_stats_cmd", {
      path: "/a.fits",
      regions: [{ id: "r1", shape, background: null }],
      excludeDq: false,
      sigma: null,
      maxiters: null,
      exclude: [hole],
    });
  });

  it("types the per-call count of pixels removed by exclude regions", () => {
    expectTypeOf<RegionStatsResult["region_excluded"]>().toEqualTypeOf<number>();
    expectTypeOf<RadialProfile["region_excluded"]>().toEqualTypeOf<number>();
    expectTypeOf<LineCut["region_excluded"]>().toEqualTypeOf<number>();
    expectTypeOf<SbProfile["region_excluded"]>().toEqualTypeOf<number>();
  });

  it("types the DQ count of the profile commands apart from masked, as with_mask emits it", () => {
    expectTypeOf<RadialProfile["dq_excluded"]>().toEqualTypeOf<number | null>();
    expectTypeOf<LineCut["dq_excluded"]>().toEqualTypeOf<number | null>();
    expectTypeOf<SbProfile["dq_excluded"]>().toEqualTypeOf<number | null>();
  });
});

describe("radialProfile and lineCut", () => {
  const hole = { shape: "box" as const, x: 5, y: 5, width: 2, height: 2, angle: 0 };

  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue({ bins: [], masked: false, dq_excluded: null, region_excluded: 0, elapsed_ms: 1 });
  });

  it("sends the exclude shapes and an empty list by default", async () => {
    await radialProfile("/a.fits", 10, 20, 8, { background: [10, 14], excludeDq: true, exclude: [hole] });
    expect(typedInvokeMock).toHaveBeenLastCalledWith("radial_profile_cmd", {
      path: "/a.fits",
      x: 10,
      y: 20,
      maxRadius: 8,
      background: [10, 14],
      excludeDq: true,
      exclude: [hole],
    });
    await radialProfile("/a.fits", 1, 2, 3);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("radial_profile_cmd", {
      path: "/a.fits",
      x: 1,
      y: 2,
      maxRadius: 3,
      background: null,
      excludeDq: false,
      exclude: [],
    });
  });

  it("lineCut sends the exclude shapes after excludeDq and an empty list by default", async () => {
    await lineCut("/a.fits", 0, 1, 9, 8, true, [hole]);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("line_cut_cmd", { path: "/a.fits", x1: 0, y1: 1, x2: 9, y2: 8, excludeDq: true, exclude: [hole] });
    await lineCut("/a.fits", 0, 1, 9, 8);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("line_cut_cmd", { path: "/a.fits", x1: 0, y1: 1, x2: 9, y2: 8, excludeDq: false, exclude: [] });
  });
});

describe("RegionCalibrated payload", () => {
  it("mirrors the calibrated block keys of the Rust region statistics", () => {
    expectTypeOf<RegionCalibrated["flux_source"]>().toEqualTypeOf<"net" | "sum">();
    expectTypeOf<RegionCalibrated["flux_jy"]>().toEqualTypeOf<number>();
    expectTypeOf<RegionCalibrated["mag_ab"]>().toEqualTypeOf<number | null>();
    expectTypeOf<RegionCalibrated["sb_mag_arcsec2"]>().toEqualTypeOf<number | null>();
    expectTypeOf<RegionCalibrated["pa_sky_deg"]>().toEqualTypeOf<number | null>();
    expectTypeOf<RegionCalibrated>().toHaveProperty("flux_err_native");
    expectTypeOf<RegionCalibrated>().toHaveProperty("geometric_area_arcsec2");
    expectTypeOf<RegionCalibrated>().toHaveProperty("area_arcsec2");
    expectTypeOf<RegionCalibrated>().toHaveProperty("ra");
    expectTypeOf<RegionCalibrated>().toHaveProperty("dec");
    expectTypeOf<RegionCalibrated>().toHaveProperty("st_mag");
    expectTypeOf<RegionStats["calibrated"]>().toEqualTypeOf<RegionCalibrated | null | undefined>();
    expectTypeOf<RegionStatsResult["photcal"]>().toEqualTypeOf<PhotCal | null | undefined>();
    expectTypeOf<RegionStatsResult["calibration_warnings"]>().toEqualTypeOf<string[] | undefined>();
  });

  it("types the WCS-only sky block that is sent with or without flux calibration", () => {
    expectTypeOf<RegionStats["sky"]>().toEqualTypeOf<RegionSky | null | undefined>();
    expectTypeOf<RegionSky>().toEqualTypeOf<{
      ra: number | null;
      dec: number | null;
      pa_sky_deg: number | null;
      area_arcsec2: number | null;
      geometric_area_arcsec2: number | null;
    }>();
  });
});

describe("sbProfile", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue({ bins: [], masked: false, elapsed_ms: 1 });
  });

  it("sends the region shape, bin width and background region of any kind to the command", async () => {
    const shape = { shape: "ellipse" as const, x: 10, y: 20, rx: 8, ry: 4, angle: 30 };
    const background = { shape: "box" as const, x: 40, y: 40, width: 6, height: 6, angle: 0 };
    const hole = { shape: "circle" as const, x: 12, y: 21, r: 2 };
    await sbProfile("/a.fits", shape, { binWidth: 2, background, excludeDq: true, exclude: [hole] });
    expect(typedInvokeMock).toHaveBeenCalledWith("sb_profile_cmd", {
      path: "/a.fits",
      shape,
      binWidth: 2,
      background,
      excludeDq: true,
      exclude: [hole],
    });
  });

  it("sends null for the bin width and background when unset so the backend defaults apply", async () => {
    const shape = { shape: "circle" as const, x: 1, y: 2, r: 3 };
    await sbProfile("/a.fits", shape);
    expect(typedInvokeMock).toHaveBeenCalledWith("sb_profile_cmd", {
      path: "/a.fits",
      shape,
      binWidth: null,
      background: null,
      excludeDq: false,
      exclude: [],
    });
  });

  it("types the calibrated bin columns and derived radii as nullable", () => {
    expectTypeOf<SbBin["mu_ab"]>().toEqualTypeOf<number | null>();
    expectTypeOf<SbBin["mu_err"]>().toEqualTypeOf<number | null>();
    expectTypeOf<SbBin["std"]>().toEqualTypeOf<number | null>();
    expectTypeOf<SbBin["sma_arcsec"]>().toEqualTypeOf<number | null>();
    expectTypeOf<SbProfile["r50_px"]>().toEqualTypeOf<number | null>();
    expectTypeOf<SbProfile["petrosian_radius_px"]>().toEqualTypeOf<number | null>();
    expectTypeOf<SbProfile["sky_pa_deg"]>().toEqualTypeOf<number | null>();
    expectTypeOf<SbProfile["photcal"]>().toEqualTypeOf<PhotCal | null>();
    expectTypeOf<SbProfile>().toHaveProperty("notes");
  });
});
