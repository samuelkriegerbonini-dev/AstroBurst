import { describe, it, expect, beforeEach, vi, expectTypeOf } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock, withPreview: vi.fn() }));

import { computeScaleLimits, getColormapLut } from "../display";
import { DEFAULT_DISPLAY_SETTINGS, type ScaleLimits } from "../../shared/types/display";

describe("computeScaleLimits", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("pins compute_scale_limits_cmd and its argument object", async () => {
    const reply: ScaleLimits = { vmin: -1, vmax: 1, algorithm: "zscale", symmetric: true, centre: 0.5, notes: [] };
    typedInvokeMock.mockResolvedValue(reply);
    const res = await computeScaleLimits("/a.fits", { ...DEFAULT_DISPLAY_SETTINGS, limits: "zscale", symmetric: true, centre: 0.5 });
    expect(typedInvokeMock).toHaveBeenCalledWith("compute_scale_limits_cmd", {
      path: "/a.fits",
      algorithm: "zscale",
      vmin: null,
      vmax: null,
      percentile: [1, 99.5],
      zscaleContrast: 0.25,
      symmetric: true,
      centre: 0.5,
    });
    expect(res).toBe(reply);
    expectTypeOf<ScaleLimits["centre"]>().toEqualTypeOf<number | null>();
    expectTypeOf<ScaleLimits["notes"]>().toEqualTypeOf<string[]>();
  });

  it("sends symmetric off with the default centre from the factory settings", async () => {
    typedInvokeMock.mockResolvedValue({ vmin: 0, vmax: 1, algorithm: "minmax", symmetric: false, centre: null, notes: [] });
    await computeScaleLimits("/b.fits", DEFAULT_DISPLAY_SETTINGS);
    expect(typedInvokeMock).toHaveBeenCalledWith("compute_scale_limits_cmd", expect.objectContaining({ symmetric: false, centre: 0 }));
  });
});

describe("getColormapLut", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("pins get_colormap_lut_cmd, appends the no-data entry and caches per name", async () => {
    const rgba = Array.from({ length: 1024 }, (_, i) => (i * 3) & 255);
    typedInvokeMock.mockResolvedValue({ name: "bwr", rgba, nodata: [64, 64, 64], colormaps: ["gray", "bwr"] });
    const lut = await getColormapLut("bwr");
    expect(typedInvokeMock).toHaveBeenCalledWith("get_colormap_lut_cmd", { name: "bwr" });
    expect(lut.length).toBe(1028);
    expect(Array.from(lut.subarray(0, 4))).toEqual(rgba.slice(0, 4));
    expect(Array.from(lut.subarray(1024, 1028))).toEqual([64, 64, 64, 255]);

    const again = await getColormapLut("bwr");
    expect(again).toBe(lut);
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
  });
});
