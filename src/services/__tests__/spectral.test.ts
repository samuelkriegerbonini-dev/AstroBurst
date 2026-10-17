import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { getSpectralAxis, measureSpectralLine, readX1dSpectrum, type MeasureLineOptions } from "../spectral";
import type {
  FileSpectrumSource,
  FluxErrorTerms,
  LineMeasurement,
  LineMeasurementSource,
  SpectrumSource,
} from "../../shared/types/spectral";

describe("line measurement types", () => {
  it("accepts a spectrum file source next to the cube sources", () => {
    expectTypeOf<FileSpectrumSource>().toEqualTypeOf<{ kind: "file"; hdu: number | null }>();
    expectTypeOf<LineMeasurementSource>().toEqualTypeOf<SpectrumSource | FileSpectrumSource>();
    expectTypeOf<LineMeasurement["source"]>().toEqualTypeOf<LineMeasurementSource>();
  });

  it("carries the error and exclusion fields every measurement response holds", () => {
    expectTypeOf<FluxErrorTerms>().toEqualTypeOf<{ pixel: number; level: number; slope: number }>();
    expectTypeOf<LineMeasurement["err_used"]>().toEqualTypeOf<boolean>();
    expectTypeOf<LineMeasurement["flux_err_terms"]>().toEqualTypeOf<FluxErrorTerms | null>();
    expectTypeOf<LineMeasurement["n_dq_excluded"]>().toEqualTypeOf<number>();
    expectTypeOf<LineMeasurement["n_nonfinite"]>().toEqualTypeOf<number>();
    expectTypeOf<LineMeasurement["n_bad_err"]>().toEqualTypeOf<number>();
    expectTypeOf<LineMeasurement["err_source"]>().toEqualTypeOf<string>();
  });

  it("takes the ERR, DQ and air or vacuum options", () => {
    expectTypeOf<MeasureLineOptions["useErr"]>().toEqualTypeOf<boolean | null | undefined>();
    expectTypeOf<MeasureLineOptions["useDq"]>().toEqualTypeOf<boolean | null | undefined>();
    expectTypeOf<MeasureLineOptions["wavelengthFrame"]>().toEqualTypeOf<"air" | "vacuum" | null | undefined>();
  });
});

describe("measureSpectralLine", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("sends the pixel source with the optical convention and no model by default", async () => {
    typedInvokeMock.mockResolvedValue({ flux: 1 });
    const source: SpectrumSource = { kind: "pixel", x: 16, y: 16 };
    await measureSpectralLine("C:/d/cube.fits", source, { z0: 8, z1: 32 });
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("measure_spectral_line_cmd", {
      path: "C:/d/cube.fits",
      source,
      z0: 8,
      z1: 32,
      continuum: null,
      restUm: null,
      convention: "optical",
      model: "none",
      velocityShiftKms: null,
      useErr: null,
      useDq: null,
      wavelengthFrame: null,
    });
  });

  it("passes the region source, windows, rest wavelength, model and frame shift through", async () => {
    typedInvokeMock.mockResolvedValue({ flux: 1 });
    const source: SpectrumSource = {
      kind: "region",
      shape: { shape: "circle", x: 16, y: 16, r: 6 },
      background: { shape: "annulus", x: 16, y: 16, r_inner: 9, r_outer: 13 },
    };
    const result = await measureSpectralLine("cube.fits", source, {
      z0: 8,
      z1: 32,
      continuum: [
        [0, 5],
        [34, 39],
      ],
      restUm: 1.02,
      convention: "radio",
      model: "gaussian",
      velocityShiftKms: -12.5,
    });
    expect(result).toEqual({ flux: 1 });
    expect(typedInvokeMock).toHaveBeenCalledWith("measure_spectral_line_cmd", {
      path: "cube.fits",
      source,
      z0: 8,
      z1: 32,
      continuum: [
        [0, 5],
        [34, 39],
      ],
      restUm: 1.02,
      convention: "radio",
      model: "gaussian",
      velocityShiftKms: -12.5,
      useErr: null,
      useDq: null,
      wavelengthFrame: null,
    });
  });

  it("sends the file source with the ERR, DQ and wavelength frame choices", async () => {
    typedInvokeMock.mockResolvedValue({ flux: 1 });
    const source: FileSpectrumSource = { kind: "file", hdu: 1 };
    await measureSpectralLine("C:/d/hd.fits", source, {
      z0: 100,
      z1: 120,
      restUm: 0.656461,
      useErr: false,
      useDq: true,
      wavelengthFrame: "vacuum",
    });
    expect(typedInvokeMock).toHaveBeenCalledWith("measure_spectral_line_cmd", {
      path: "C:/d/hd.fits",
      source: { kind: "file", hdu: 1 },
      z0: 100,
      z1: 120,
      continuum: null,
      restUm: 0.656461,
      convention: "optical",
      model: "none",
      velocityShiftKms: null,
      useErr: false,
      useDq: true,
      wavelengthFrame: "vacuum",
    });
  });
});

describe("getSpectralAxis", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("asks the backend for the header axis of the given path", async () => {
    typedInvokeMock.mockResolvedValue({ kind: "wave" });
    await getSpectralAxis("cube.fits");
    expect(typedInvokeMock).toHaveBeenCalledWith("spectral_axis_cmd", { path: "cube.fits" });
  });
});

describe("readX1dSpectrum", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("sends the cube path with its plane fragment and a null HDU by default", async () => {
    typedInvokeMock.mockResolvedValue({ n_rows: 3 });
    const result = await readX1dSpectrum("a_s3d.fits#hdu=1");
    expect(result).toEqual({ n_rows: 3 });
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("read_x1d_spectrum_cmd", { path: "a_s3d.fits#hdu=1", hdu: null });
  });

  it("passes an explicit EXTRACT1D HDU through", async () => {
    typedInvokeMock.mockResolvedValue({ n_rows: 3 });
    await readX1dSpectrum("C:/d/b_x1d.fits", 3);
    expect(typedInvokeMock).toHaveBeenCalledWith("read_x1d_spectrum_cmd", { path: "C:/d/b_x1d.fits", hdu: 3 });
  });
});
