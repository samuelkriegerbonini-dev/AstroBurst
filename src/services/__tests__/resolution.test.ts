import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { imagePsfFwhm, resolutionCheck } from "../resolution";
import type { ImagePsfFwhm, ResolutionCheck, ResolutionVerdict } from "../../shared/types/resolution";

const CHECK: ResolutionCheck = {
  verdict: "marginal",
  size_basis: "fwhm",
  psf_fwhm_arcsec: 1.14,
  psf_reference: null,
  pixels_per_fwhm: 5.7,
  nyquist_sampled: true,
  psf_area_px: 36.8,
  intrinsic_size_arcsec: 0.37,
  snr_floor_fraction: 0.29,
  psf_floor_fraction: 0.25,
  floor_fraction: 0.29,
  size_floor_arcsec: 0.33,
  resolution_elements: 0.33,
  kpc_per_arcsec: 8.23125144264795,
  kpc_per_pixel: 1.65,
  angular_diameter_distance_mpc: 1697.8,
  psf_fwhm_kpc: 9.383626644618662,
  intrinsic_size_kpc: 3.1,
  size_floor_kpc: 2.7,
  scale_of_interest_in_psf: 0.11,
  snr: 50,
  redshift: 1,
  psf_fwhm_rel_err: 0.03,
  resolved_elements: 3,
  min_pixels_per_fwhm: 2,
  scale_of_interest_kpc: 1,
  h0_km_s_mpc: 67.66,
  omega_m: 0.30966,
  omega_nu_massive: 0.00144,
  omega_rad: 0.0000915,
  cosmology: "Planck18",
  notes: [],
  warnings: [],
};

describe("resolution service", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("resolutionCheck sends the request object untouched", async () => {
    typedInvokeMock.mockResolvedValue(CHECK);
    const res = await resolutionCheck({ psf_fwhm_arcsec: 1.14, redshift: 1 });
    expect(typedInvokeMock).toHaveBeenCalledWith("resolution_check_cmd", {
      request: { psf_fwhm_arcsec: 1.14, redshift: 1 },
    });
    expect(res).toBe(CHECK);
  });

  it("imagePsfFwhm sends null for an absent star limit", async () => {
    typedInvokeMock.mockResolvedValue({});
    await imagePsfFwhm("a.fits");
    expect(typedInvokeMock).toHaveBeenCalledWith("image_psf_fwhm_cmd", { path: "a.fits", maxStars: null });
    await imagePsfFwhm("a.fits", 50);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("image_psf_fwhm_cmd", { path: "a.fits", maxStars: 50 });
  });

  it("the types mirror the Rust option members", () => {
    expectTypeOf<ImagePsfFwhm["fwhm_arcsec"]>().toEqualTypeOf<number | null>();
    expectTypeOf<ImagePsfFwhm["fwhm_px_scatter"]>().toEqualTypeOf<number | null>();
    expectTypeOf<ImagePsfFwhm["pixel_scale_arcsec"]>().toEqualTypeOf<number | null>();
    expectTypeOf<ResolutionCheck["verdict"]>().toEqualTypeOf<ResolutionVerdict | null>();
  });
});
