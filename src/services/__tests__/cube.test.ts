import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { getCubeSpectrum, getCubeSpectrumRegion, releaseCubes } from "../cube";

describe("releaseCubes", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("releases each source file once, whatever plane of it was loaded", async () => {
    typedInvokeMock.mockResolvedValue(undefined);
    await releaseCubes(["C:/d/cube_s3d.fits", "C:/d/cube_s3d.fits#hdu=1", "C:/d/other.fits"]);
    expect(typedInvokeMock.mock.calls).toEqual([
      ["release_cube_cmd", { path: "C:/d/cube_s3d.fits" }],
      ["release_cube_cmd", { path: "C:/d/other.fits" }],
    ]);
  });

  it("keeps releasing the other files when one release fails", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    typedInvokeMock.mockRejectedValueOnce(new Error("gone")).mockResolvedValueOnce(undefined);
    await expect(releaseCubes(["a.fits", "b.fits"])).resolves.toBeUndefined();
    expect(typedInvokeMock).toHaveBeenCalledTimes(2);
    warn.mockRestore();
  });
});

describe("cube spectrum services", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("getCubeSpectrum invokes get_cube_spectrum with path, x and y and keeps flux_jy", async () => {
    typedInvokeMock.mockResolvedValueOnce({ spectrum: [2], wavelengths: [1.02], is_spectral: true, flux_jy: [1e-6] });
    const calibrated = await getCubeSpectrum("C:/d/cube.fits", 3, 4);
    expect(typedInvokeMock).toHaveBeenCalledWith("get_cube_spectrum", { path: "C:/d/cube.fits", x: 3, y: 4 });
    expect(calibrated.values).toEqual([2]);
    expect(calibrated.flux_jy).toEqual([1e-6]);
    expect(calibrated.x).toBe(3);
    expect(calibrated.y).toBe(4);

    typedInvokeMock.mockResolvedValueOnce({ spectrum: [2], wavelengths: [1.02], is_spectral: true });
    const plain = await getCubeSpectrum("C:/d/cube.fits", 3, 4);
    expect(plain.flux_jy).toBeNull();
  });

  it("getCubeSpectrumRegion invokes get_cube_spectrum_region_cmd with path, shape and background", async () => {
    typedInvokeMock.mockResolvedValueOnce({ sum: [], mean: [] });
    await getCubeSpectrumRegion("C:/d/cube.fits", { shape: "circle", x: 20, y: 20, r: 3 }, null);
    expect(typedInvokeMock).toHaveBeenCalledWith("get_cube_spectrum_region_cmd", {
      path: "C:/d/cube.fits",
      shape: { shape: "circle", x: 20, y: 20, r: 3 },
      background: null,
    });
  });
});
