import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock }));

import { exportRegions, regionStats, sbProfile } from "../regions";
import {
  REGION_SYSTEMS,
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
    typedInvokeMock.mockResolvedValue({ regions: [], masked: false, dq_excluded: null, elapsed_ms: 1 });
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
    });
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
    await sbProfile("/a.fits", shape, { binWidth: 2, background, excludeDq: true });
    expect(typedInvokeMock).toHaveBeenCalledWith("sb_profile_cmd", {
      path: "/a.fits",
      shape,
      binWidth: 2,
      background,
      excludeDq: true,
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
