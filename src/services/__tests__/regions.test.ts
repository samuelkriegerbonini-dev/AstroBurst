import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock }));

import { exportRegions } from "../regions";
import { REGION_SYSTEMS, type RegionStats } from "../../shared/types/regions";

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
