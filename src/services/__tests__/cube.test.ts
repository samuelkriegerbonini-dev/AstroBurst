import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock, getOutputDirMock, getPreviewUrlMock } = vi.hoisted(() => ({
  typedInvokeMock: vi.fn(),
  getOutputDirMock: vi.fn(),
  getPreviewUrlMock: vi.fn(),
}));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: getOutputDirMock,
  getPreviewUrl: getPreviewUrlMock,
}));

import { fitCubeLines, getCubeSpectrum, getCubeSpectrumRegion, inspectLineFitSpaxel, releaseCubes } from "../cube";
import type { LineFitConfig } from "../../shared/types/cube";

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

describe("line-fit services", () => {
  const cfg: LineFitConfig = {
    z0: 530,
    z1: 560,
    rest_um: 1.875613,
    convention: "optical",
    continuum: [
      [500, 520],
      [568, 588],
    ],
    snr_threshold: 3,
    emission_only: true,
    use_err: true,
    use_dq: true,
    resolving_power: null,
    components: "one",
  };

  beforeEach(() => {
    typedInvokeMock.mockReset();
    getOutputDirMock.mockReset();
    getPreviewUrlMock.mockReset();
  });

  it("fitCubeLines invokes cube_line_fit_cmd with the resolved output dir and resolves a previewUrl on every plane", async () => {
    getOutputDirMock.mockResolvedValue("C:/app/output");
    getPreviewUrlMock.mockImplementation((png: string) => Promise.resolve(`asset://${png}`));
    typedInvokeMock.mockResolvedValueOnce({
      planes: {
        flux: { png_path: "C:/app/output/a_linefit_flux_530-560.png", fits_path: "C:/app/output/a_linefit_flux_530-560.fits" },
        velocity: { png_path: "C:/app/output/a_linefit_velocity_530-560.png", fits_path: "C:/app/output/a_linefit_velocity_530-560.fits" },
      },
      plane_order: ["flux", "velocity"],
      n_fit: 5,
    });
    const result = await fitCubeLines("a_s3d.fits#hdu=1", undefined, cfg);
    expect(typedInvokeMock).toHaveBeenCalledWith("cube_line_fit_cmd", { path: "a_s3d.fits#hdu=1", outputDir: "C:/app/output", config: cfg });
    expect(result.planes.flux.previewUrl).toBe("asset://C:/app/output/a_linefit_flux_530-560.png");
    expect(result.planes.velocity.previewUrl).toBe("asset://C:/app/output/a_linefit_velocity_530-560.png");
    expect(result.planes.velocity.fits_path).toBe("C:/app/output/a_linefit_velocity_530-560.fits");
    expect(result.n_fit).toBe(5);

    typedInvokeMock.mockResolvedValueOnce({ planes: {}, plane_order: [] });
    await fitCubeLines("a_s3d.fits#hdu=1", "D:/maps", cfg);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("cube_line_fit_cmd", { path: "a_s3d.fits#hdu=1", outputDir: "D:/maps", config: cfg });
    expect(getOutputDirMock).toHaveBeenCalledTimes(1);
  });

  it("inspectLineFitSpaxel invokes cube_line_fit_spaxel_cmd with path, config, x and y", async () => {
    typedInvokeMock.mockResolvedValueOnce({ x: 18, y: 32 });
    const spaxel = await inspectLineFitSpaxel("a_s3d.fits#hdu=1", cfg, 18, 32);
    expect(typedInvokeMock).toHaveBeenCalledWith("cube_line_fit_spaxel_cmd", { path: "a_s3d.fits#hdu=1", config: cfg, x: 18, y: 32 });
    expect(spaxel.x).toBe(18);
  });
});
