import { describe, expect, it } from "vitest";
import { aroundFromWcs, aroundLabel } from "../logic/aroundQuery";
import type { WcsInfo } from "../../../shared/types/astrometry";

function wcs(over: Partial<WcsInfo> = {}): WcsInfo {
  return {
    center_ra: 274.7123,
    center_dec: -13.8301,
    pixel_scale_arcsec: 0.0628,
    fov_arcmin: [6.8, 5.1],
    field_of_view_w_arcmin: 6.8,
    field_of_view_h_arcmin: 5.1,
    naxis1: 6500,
    naxis2: 4870,
    ...over,
  };
}

describe("aroundFromWcs", () => {
  it("centres on the image and uses half the field diagonal as the radius", () => {
    expect(aroundFromWcs(wcs(), "673nmos.fits")).toEqual({ raDeg: 274.7123, decDeg: -13.8301, radiusArcmin: 4.3, label: "673nmos.fits" });
  });

  it("clamps the radius to [1', 30']", () => {
    expect(aroundFromWcs(wcs({ fov_arcmin: [0.5, 0.5], field_of_view_w_arcmin: 0.5, field_of_view_h_arcmin: 0.5 }), "a")?.radiusArcmin).toBe(1);
    expect(aroundFromWcs(wcs({ fov_arcmin: [120, 90], field_of_view_w_arcmin: 120, field_of_view_h_arcmin: 90 }), "a")?.radiusArcmin).toBe(30);
  });

  it("falls back to the width and height fields when fov_arcmin is missing", () => {
    const info = { ...wcs(), fov_arcmin: undefined as unknown as [number, number] };
    expect(aroundFromWcs(info, "a")?.radiusArcmin).toBe(4.3);
  });

  it("returns null without a usable celestial solution", () => {
    expect(aroundFromWcs(wcs({ center_ra: Number.NaN }), "a")).toBeNull();
    expect(aroundFromWcs(wcs({ center_dec: 95 }), "a")).toBeNull();
    expect(aroundFromWcs(wcs({ fov_arcmin: [0, 0], field_of_view_w_arcmin: 0, field_of_view_h_arcmin: 0 }), "a")).toBeNull();
    expect(aroundFromWcs(null, "a")).toBeNull();
  });

  it("wraps a negative RA into [0, 360)", () => {
    expect(aroundFromWcs(wcs({ center_ra: -0.5 }), "a")?.raDeg).toBeCloseTo(359.5, 9);
  });
});

describe("aroundLabel", () => {
  it("reads like the first-run button", () => {
    expect(aroundLabel({ raDeg: 274.7123, decDeg: -13.8301, radiusArcmin: 4.2, label: "673nmos.fits" })).toBe("Around 673nmos.fits  274.71 -13.83  r 4.2'");
  });
});
