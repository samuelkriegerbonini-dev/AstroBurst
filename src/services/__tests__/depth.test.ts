import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { imageDepth } from "../depth";
import type { DepthAperture, ImageDepth } from "../../shared/types/depth";

describe("imageDepth", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue({});
  });

  it("imageDepth sends null for every option left out", async () => {
    await imageDepth("a.fits");
    expect(typedInvokeMock).toHaveBeenCalledWith("image_depth_cmd", {
      path: "a.fits",
      radiusArcsec: null,
      radiusPx: null,
      nApertures: null,
      nSigma: null,
      seed: null,
      detectNsigma: null,
      psfFwhmArcsec: null,
      excludeDq: null,
    });
  });

  it("imageDepth forwards the options by name", async () => {
    await imageDepth("a.fits", { radiusArcsec: 1.5, nApertures: 1000, seed: 7, psfFwhmArcsec: 1.1, excludeDq: false });
    expect(typedInvokeMock).toHaveBeenCalledWith("image_depth_cmd", {
      path: "a.fits",
      radiusArcsec: 1.5,
      radiusPx: null,
      nApertures: 1000,
      nSigma: null,
      seed: 7,
      detectNsigma: null,
      psfFwhmArcsec: 1.1,
      excludeDq: false,
    });
  });

  it("the type mirrors the optional members", () => {
    expectTypeOf<ImageDepth["limit_ab"]>().toEqualTypeOf<number | null>();
    expectTypeOf<ImageDepth["f_corr"]>().toEqualTypeOf<number | null>();
    expectTypeOf<ImageDepth["radius_arcsec"]>().toEqualTypeOf<number | null>();
    expectTypeOf<ImageDepth["calibration_origin"]>().toEqualTypeOf<string | null>();
    expectTypeOf<ImageDepth["apertures"]>().toEqualTypeOf<DepthAperture[]>();
    expectTypeOf<ImageDepth["detected_plane_pixels"]>().toEqualTypeOf<number | null | undefined>();
  });
});
