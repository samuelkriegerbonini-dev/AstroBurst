import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

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
import type {
  CollapseRangeResult,
  ContinuumWindows,
  LineFitConfig,
  MomentMapsResult,
  NoiseFactor,
  RegionSpectrum,
  RegionSpectrumWire,
} from "../../shared/types/cube";

describe("cube error product types", () => {
  it("carries the propagated error, DQ counts and noise factor of a region spectrum", () => {
    expectTypeOf<NoiseFactor>().toEqualTypeOf<{
      factor: number;
      factor_se: number;
      n_channels: number;
      windows: ContinuumWindows;
      warning: string | null;
    }>();
    expectTypeOf<RegionSpectrumWire["sum"]>().toEqualTypeOf<(number | null)[]>();
    expectTypeOf<RegionSpectrumWire["mean"]>().toEqualTypeOf<(number | null)[]>();
    expectTypeOf<RegionSpectrumWire["bg_per_pixel"]>().toEqualTypeOf<(number | null)[] | null>();
    expectTypeOf<RegionSpectrumWire["flux_jy"]>().toEqualTypeOf<(number | null)[] | null>();
    expectTypeOf<RegionSpectrumWire["err"]>().toEqualTypeOf<(number | null)[] | null>();
    expectTypeOf<RegionSpectrumWire["err_scaled"]>().toEqualTypeOf<(number | null)[] | null>();
    expectTypeOf<RegionSpectrumWire["mean_err"]>().toEqualTypeOf<(number | null)[] | null>();
    expectTypeOf<RegionSpectrumWire["bg_err_per_pixel"]>().toEqualTypeOf<(number | null)[] | null>();
    expectTypeOf<RegionSpectrumWire["flux_jy_err"]>().toEqualTypeOf<(number | null)[] | null>();
    expectTypeOf<RegionSpectrum["sum"]>().toEqualTypeOf<number[]>();
    expectTypeOf<RegionSpectrum["err"]>().toEqualTypeOf<number[] | null>();
    expectTypeOf<RegionSpectrum["err_scaled"]>().toEqualTypeOf<number[] | null>();
    expectTypeOf<RegionSpectrum["mean_err"]>().toEqualTypeOf<number[] | null>();
    expectTypeOf<RegionSpectrum["used_weight"]>().toEqualTypeOf<number[]>();
    expectTypeOf<RegionSpectrum["err_source"]>().toEqualTypeOf<string>();
    expectTypeOf<RegionSpectrum["bg_err_per_pixel"]>().toEqualTypeOf<number[] | null>();
    expectTypeOf<RegionSpectrum["flux_jy_err"]>().toEqualTypeOf<number[] | null>();
    expectTypeOf<RegionSpectrum["n_dq_excluded"]>().toEqualTypeOf<number[] | null>();
    expectTypeOf<RegionSpectrum["noise_factor"]>().toEqualTypeOf<NoiseFactor | null>();
    expectTypeOf<RegionSpectrum["notes"]>().toEqualTypeOf<string[]>();
  });

  it("says whether a collapse or a moment run wrote an ERR plane", () => {
    expectTypeOf<CollapseRangeResult["has_err"]>().toEqualTypeOf<boolean>();
    expectTypeOf<CollapseRangeResult["err_source"]>().toEqualTypeOf<string>();
    expectTypeOf<CollapseRangeResult["notes"]>().toEqualTypeOf<string[]>();
    expectTypeOf<MomentMapsResult["has_err"]>().toEqualTypeOf<boolean>();
    expectTypeOf<MomentMapsResult["err_source"]>().toEqualTypeOf<string>();
    expectTypeOf<MomentMapsResult["noise_factor"]>().toEqualTypeOf<number | null>();
  });
});

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

  it("getCubeSpectrumRegion invokes get_cube_spectrum_region_cmd with path, shape, background and no continuum by default", async () => {
    typedInvokeMock.mockResolvedValueOnce({ sum: [], mean: [] });
    await getCubeSpectrumRegion("C:/d/cube.fits", { shape: "circle", x: 20, y: 20, r: 3 }, null);
    expect(typedInvokeMock).toHaveBeenCalledWith("get_cube_spectrum_region_cmd", {
      path: "C:/d/cube.fits",
      shape: { shape: "circle", x: 20, y: 20, r: 3 },
      background: null,
      continuum: null,
    });
  });

  it("getCubeSpectrumRegion sends the continuum windows the noise factor is measured on", async () => {
    typedInvokeMock.mockResolvedValueOnce({ sum: [], mean: [] });
    const windows: ContinuumWindows = [
      [0, 9],
      [30, 39],
    ];
    const annulus = { shape: "annulus" as const, x: 20, y: 20, r_inner: 5, r_outer: 8 };
    await getCubeSpectrumRegion("C:/d/cube.fits", { shape: "circle", x: 20, y: 20, r: 3 }, annulus, windows);
    expect(typedInvokeMock).toHaveBeenCalledWith("get_cube_spectrum_region_cmd", {
      path: "C:/d/cube.fits",
      shape: { shape: "circle", x: 20, y: 20, r: 3 },
      background: annulus,
      continuum: windows,
    });
  });

  it("getCubeSpectrumRegion maps every NaN channel the backend sends as null to NaN and keeps the error fields", async () => {
    const factor: NoiseFactor = {
      factor: 1.31,
      factor_se: 0.05,
      n_channels: 12,
      windows: [
        [0, 5],
        [34, 39],
      ],
      warning: null,
    };
    const wire: RegionSpectrumWire = {
      sum: [1, null, 3],
      mean: [0.1, null, 0.3],
      npix: 10,
      n_bg: 40,
      bg_per_pixel: [0.01, null, 0.03],
      wavelengths: [2, 2.001, 2.002],
      unit: "um",
      bg_subtracted: true,
      flux_jy: [1e-5, null, 3e-5],
      err: [0.2, null, 0.4],
      err_scaled: [0.262, null, 0.524],
      mean_err: [0.02, null, 0.04],
      used_weight: [10, 0, 10],
      err_source: "ERR HDU 2",
      bg_err_per_pixel: [0.001, null, 0.003],
      flux_jy_err: [2e-6, null, 4e-6],
      n_dq_excluded: [0, 3, 1],
      noise_factor: factor,
      notes: ["error propagated from the ERR cube (HDU 2)"],
      elapsed_ms: 7,
    };
    typedInvokeMock.mockResolvedValueOnce(wire);
    const region = await getCubeSpectrumRegion("C:/d/cube.fits", { shape: "circle", x: 20, y: 20, r: 3 }, null);
    expect(region.sum).toEqual([1, NaN, 3]);
    expect(region.mean).toEqual([0.1, NaN, 0.3]);
    expect(region.bg_per_pixel).toEqual([0.01, NaN, 0.03]);
    expect(region.flux_jy).toEqual([1e-5, NaN, 3e-5]);
    expect(region.err).toEqual([0.2, NaN, 0.4]);
    expect(region.err_scaled).toEqual([0.262, NaN, 0.524]);
    expect(region.mean_err).toEqual([0.02, NaN, 0.04]);
    expect(region.bg_err_per_pixel).toEqual([0.001, NaN, 0.003]);
    expect(region.flux_jy_err).toEqual([2e-6, NaN, 4e-6]);
    expect(region.used_weight).toEqual([10, 0, 10]);
    expect(region.err_source).toBe("ERR HDU 2");
    expect(region.n_dq_excluded).toEqual([0, 3, 1]);
    expect(region.noise_factor).toEqual(factor);
    expect(region.notes).toEqual(["error propagated from the ERR cube (HDU 2)"]);
    expect(region.wavelengths).toEqual([2, 2.001, 2.002]);
  });

  it("getCubeSpectrumRegion keeps absent error arrays absent for a cube without ERR", async () => {
    typedInvokeMock.mockResolvedValueOnce({
      sum: [1],
      mean: [0.1],
      bg_per_pixel: null,
      flux_jy: null,
      err: null,
      err_scaled: null,
      mean_err: null,
      bg_err_per_pixel: null,
      flux_jy_err: null,
      err_source: "none",
    });
    const region = await getCubeSpectrumRegion("C:/d/cube.fits", { shape: "circle", x: 20, y: 20, r: 3 }, null);
    expect(region.err).toBeNull();
    expect(region.err_scaled).toBeNull();
    expect(region.mean_err).toBeNull();
    expect(region.flux_jy_err).toBeNull();
    expect(region.bg_err_per_pixel).toBeNull();
    expect(region.bg_per_pixel).toBeNull();
    expect(region.flux_jy).toBeNull();
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
