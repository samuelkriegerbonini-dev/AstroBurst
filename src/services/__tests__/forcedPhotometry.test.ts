import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { cancelForcedPhotometry, forcedPhotometry } from "../forcedPhotometry";
import type {
  ForcedApertureCorrection,
  ForcedBand,
  ForcedCell,
  ForcedFlagName,
  ForcedPhotometryResult,
  ForcedRunOptions,
  ForcedSource,
} from "../../shared/types/forcedPhotometry";
import type { UserZeroPoint } from "../analysis";

const RESULT: ForcedPhotometryResult = {
  bands: [],
  sources: [],
  aperture: { radius_arcsec: 2, radius_px: null, annulus_arcsec: null, annulus_px: null, correction: "none", reference_radius_arcsec: null },
  limit_sigma: 3,
  elapsed_ms: 1,
  warnings: [],
};

const REQUEST = {
  bands: [{ path: "C:/d/r.fits" }],
  positions: [{ id: "a", ra_deg: 53.1, dec_deg: -28.1 }],
  aperture: { radius_arcsec: 2, correction: "none" as const },
};

describe("forcedPhotometry", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("pins forced_photometry_cmd with its four arguments and null options", async () => {
    typedInvokeMock.mockResolvedValue(RESULT);
    const res = await forcedPhotometry(REQUEST);
    expect(typedInvokeMock).toHaveBeenCalledWith("forced_photometry_cmd", {
      bands: [{ path: "C:/d/r.fits" }],
      positions: [{ id: "a", ra_deg: 53.1, dec_deg: -28.1 }],
      aperture: { radius_arcsec: 2, correction: "none" },
      options: null,
    });
    expect(res).toBe(RESULT);
  });

  it("sends the options object when one is given", async () => {
    typedInvokeMock.mockResolvedValue(RESULT);
    await forcedPhotometry({ ...REQUEST, options: { exclude_dq: true, limit_sigma: 5, f_corr: [1.6, null] } });
    expect(typedInvokeMock).toHaveBeenCalledWith("forced_photometry_cmd", {
      bands: [{ path: "C:/d/r.fits" }],
      positions: [{ id: "a", ra_deg: 53.1, dec_deg: -28.1 }],
      aperture: { radius_arcsec: 2, correction: "none" },
      options: { exclude_dq: true, limit_sigma: 5, f_corr: [1.6, null] },
    });
  });

  it("sends the typed zero points inside the options, one entry per band", async () => {
    typedInvokeMock.mockResolvedValue(RESULT);
    const zeroPoints: ForcedRunOptions["zero_points"] = [{ zp: 25, system: "ab", per_second: false }, null];
    await forcedPhotometry({ ...REQUEST, options: { zero_points: zeroPoints } });
    expect(typedInvokeMock).toHaveBeenCalledWith("forced_photometry_cmd", {
      bands: [{ path: "C:/d/r.fits" }],
      positions: [{ id: "a", ra_deg: 53.1, dec_deg: -28.1 }],
      aperture: { radius_arcsec: 2, correction: "none" },
      options: { zero_points: [{ zp: 25, system: "ab", per_second: false }, null] },
    });
    expectTypeOf<ForcedRunOptions["zero_points"]>().toEqualTypeOf<(UserZeroPoint | null)[] | null | undefined>();
  });

  it("cancels through the generic progress command with the event name", async () => {
    typedInvokeMock.mockResolvedValue(true);
    await expect(cancelForcedPhotometry()).resolves.toBe(true);
    expect(typedInvokeMock).toHaveBeenCalledWith("cancel_progress_cmd", { event: "forced-photometry-progress" });
  });

  it("mirrors the nullable cells, the band error and the optional fields of a later backend", () => {
    expectTypeOf<ForcedSource["cells"]>().toEqualTypeOf<(ForcedCell | null)[]>();
    expectTypeOf<ForcedBand["error"]>().toEqualTypeOf<string | null>();
    expectTypeOf<ForcedApertureCorrection["n_candidates"]>().toEqualTypeOf<number>();
    expectTypeOf<ForcedBand["flag_names"]>().toEqualTypeOf<ForcedFlagName[] | undefined>();
    expectTypeOf<ForcedBand["mag_system"]>().toEqualTypeOf<string | null | undefined>();
  });
});
