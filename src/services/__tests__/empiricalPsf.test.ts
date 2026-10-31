import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock, announceMock } = vi.hoisted(() => ({
  typedInvokeMock: vi.fn(),
  announceMock: vi.fn(),
}));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

vi.mock("../../infrastructure/tauri/outputEvents", () => ({
  announceCommandOutputs: announceMock,
}));

import { buildEmpiricalPsf } from "../processing";
import type { EmpiricalPsfReport } from "../../shared/types/processing";

function report(overrides: Partial<EmpiricalPsfReport> = {}): EmpiricalPsfReport {
  return {
    source: "regions",
    filter: "F444W",
    dataset: "JWST",
    pixel_scale_arcsec: 0.04,
    normalization_radius_px: 20,
    psf_size: 51,
    fwhm_px: 3.6,
    fwhm_arcsec: 0.144,
    stars_used: 1,
    stars_rejected: 0,
    stars: [],
    kernel: [[1]],
    profile: [1],
    oversampling: null,
    oversampled_size: null,
    output_path: null,
    oversampled_path: null,
    warnings: [],
    elapsed_ms: 4,
    ...overrides,
  };
}

describe("buildEmpiricalPsf", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    announceMock.mockReset();
  });

  it("sends the tagged star source, the snake_case recipe and camelCase arguments", async () => {
    typedInvokeMock.mockResolvedValueOnce(report());
    await buildEmpiricalPsf("C:/d/field.fits", { kind: "regions", path: "C:/d/stars_f444w.reg" }, {
      recipe: { cutout_size: 31, psf_size: 21, normalization_radius: 0.48, normalization_radius_unit: "arcsec", oversampling: 2 },
      outputDir: "C:/out",
    });
    expect(typedInvokeMock).toHaveBeenCalledWith("build_empirical_psf_cmd", {
      path: "C:/d/field.fits",
      source: { kind: "regions", path: "C:/d/stars_f444w.reg" },
      options: { cutout_size: 31, psf_size: 21, normalization_radius: 0.48, normalization_radius_unit: "arcsec", oversampling: 2 },
      dataset: null,
      filter: null,
      pixelScaleArcsec: null,
      outputDir: "C:/out",
    });
  });

  it("leaves the recipe to the backend defaults and writes nothing without an output folder", async () => {
    typedInvokeMock.mockResolvedValueOnce(report());
    await buildEmpiricalPsf("C:/d/field.fits", { kind: "auto", num_stars: 12 });
    expect(typedInvokeMock).toHaveBeenCalledWith("build_empirical_psf_cmd", {
      path: "C:/d/field.fits",
      source: { kind: "auto", num_stars: 12 },
      options: null,
      dataset: null,
      filter: null,
      pixelScaleArcsec: null,
      outputDir: null,
    });
    expect(announceMock).not.toHaveBeenCalled();
  });

  it("announces the written PSF and its oversampled copy", async () => {
    const written = report({ output_path: "C:/out/empirical_psf_jwst_f444w.fits", oversampled_path: "C:/out/empirical_psf_jwst_f444w_os2.fits" });
    typedInvokeMock.mockResolvedValueOnce(written);
    const result = await buildEmpiricalPsf("C:/d/field.fits", { kind: "ra_dec", text: "53.16 -27.79" }, { outputDir: "C:/out" });
    expect(result).toBe(written);
    expect(announceMock.mock.calls).toEqual([
      [{ output_path: "C:/out/empirical_psf_jwst_f444w.fits" }],
      [{ output_path: "C:/out/empirical_psf_jwst_f444w_os2.fits" }],
    ]);
  });
});
