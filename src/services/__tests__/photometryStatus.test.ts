import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { photometryStatus } from "../photometryStatus";
import type { PhotCal } from "../analysis";
import type {
  PhotometryStatus,
  PhotometryStatusDq,
  PhotometryStatusErr,
  PhotometryStatusFilter,
} from "../../shared/types/photometryStatus";

const STATUS: PhotometryStatus = {
  photcal: null,
  calibration_origin: null,
  missing_reason: "no photometric calibration in the header",
  bunit: "adu",
  summed_unit: "adu",
  err: null,
  dq: null,
  pixel_scale_arcsec: 0.262,
  pixel_scale_axes_arcsec: [0.262, 0.262],
  filter: null,
  warnings: [],
};

describe("photometryStatus", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("pins photometry_status_cmd with DQ exclusion off by default", async () => {
    typedInvokeMock.mockResolvedValue(STATUS);
    const res = await photometryStatus("C:/d/a.fits");
    expect(typedInvokeMock).toHaveBeenCalledWith("photometry_status_cmd", {
      path: "C:/d/a.fits",
      excludeDq: false,
      zeroPoint: null,
    });
    expect(res).toBe(STATUS);
  });

  it("forwards the DQ exclusion flag", async () => {
    typedInvokeMock.mockResolvedValue(STATUS);
    await photometryStatus("C:/d/a.fits", { excludeDq: true });
    expect(typedInvokeMock).toHaveBeenCalledWith("photometry_status_cmd", {
      path: "C:/d/a.fits",
      excludeDq: true,
      zeroPoint: null,
    });
  });

  it("forwards a typed zero point in the wire shape", async () => {
    typedInvokeMock.mockResolvedValue({ ...STATUS, calibration_source: "user" });
    const res = await photometryStatus("C:/d/a.fits", { zeroPoint: { zp: 25, system: "ab", per_second: false } });
    expect(typedInvokeMock).toHaveBeenCalledWith("photometry_status_cmd", {
      path: "C:/d/a.fits",
      excludeDq: false,
      zeroPoint: { zp: 25, system: "ab", per_second: false },
    });
    expect(res.calibration_source).toBe("user");
  });

  it("types the optional members as nullable", () => {
    expectTypeOf<PhotometryStatus["pixel_scale_arcsec"]>().toEqualTypeOf<number | null>();
    expectTypeOf<PhotometryStatus["photcal"]>().toEqualTypeOf<PhotCal | null>();
    expectTypeOf<PhotometryStatus["err"]>().toEqualTypeOf<PhotometryStatusErr | null>();
    expectTypeOf<PhotometryStatus["dq"]>().toEqualTypeOf<PhotometryStatusDq | null>();
    expectTypeOf<PhotometryStatus["filter"]>().toEqualTypeOf<PhotometryStatusFilter | null>();
    expectTypeOf<PhotometryStatus["calibration_source"]>().toEqualTypeOf<"header" | "user" | null | undefined>();
  });
});
