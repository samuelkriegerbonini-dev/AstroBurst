import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { getObservationGeometry } from "../geometry";
import type { FrameGeometry, ObservationGeometryResult } from "../../shared/types/geometry";

const RESULT: ObservationGeometryResult = {
  geometry: {
    jd_utc: 2461120.9236,
    jd_tt: 2461120.9244,
    jd_tdb: 2461120.9244,
    bjd_tdb: 2461120.9251,
    hjd_utc: 2461120.9243,
    bjd_source: "computed",
    lst_deg: 10,
    hour_angle_deg: 0,
    altitude_deg: 70,
    azimuth_deg: 180,
    airmass_computed: 1.06,
    airmass_formula: "Kasten & Young 1989",
    parallactic_angle_deg: 0,
    sun_altitude_deg: -40,
    moon_altitude_deg: 12,
    moon_illumination: 0.5,
    moon_separation_deg: 90,
    time_scale_notes: [],
  },
  target: { ra_deg: 150, dec_deg: 2, source: "WCS at image centre (49.5, 49.5)" },
  site: { lon_deg: -155.47, lat_deg: 19.82, height_m: 0, source: "SITELAT/SITELONG" },
  time_source: "DATE-OBS + EXPTIME/2 (600 s)",
  airmass_header: null,
  notes: [],
  elapsed_ms: 1,
};

describe("getObservationGeometry", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("pins observation_geometry_cmd with nulls for absent overrides", async () => {
    typedInvokeMock.mockResolvedValue(RESULT);
    const res = await getObservationGeometry("C:/d/frame.fits");
    expect(typedInvokeMock).toHaveBeenCalledWith("observation_geometry_cmd", {
      path: "C:/d/frame.fits",
      targetRa: null,
      targetDec: null,
      siteLat: null,
      siteLon: null,
      siteHeight: null,
    });
    expect(res).toBe(RESULT);
  });

  it("forwards target and site overrides", async () => {
    typedInvokeMock.mockResolvedValue(RESULT);
    await getObservationGeometry("C:/d/frame.fits", { targetRa: 150.5, targetDec: -2.25, siteLat: 19.82, siteLon: -155.47, siteHeight: 4200 });
    expect(typedInvokeMock).toHaveBeenCalledWith("observation_geometry_cmd", {
      path: "C:/d/frame.fits",
      targetRa: 150.5,
      targetDec: -2.25,
      siteLat: 19.82,
      siteLon: -155.47,
      siteHeight: 4200,
    });
  });

  it("types bjd_tdb as number or null", () => {
    expectTypeOf<FrameGeometry["bjd_tdb"]>().toEqualTypeOf<number | null>();
    expectTypeOf<FrameGeometry["bjd_source"]>().toEqualTypeOf<"computed" | "header" | null>();
    expect(RESULT.geometry.bjd_tdb).not.toBeNull();
  });
});
