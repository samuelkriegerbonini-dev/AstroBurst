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
  LineVelocity,
  MeasuredLineSource,
  SeriesSpectrumSource,
  SpectrumSource,
} from "../../shared/types/spectral";
import type { TsoPick } from "../../shared/types/tso";

describe("line measurement types", () => {
  it("accepts a spectrum file source next to the cube sources", () => {
    expectTypeOf<FileSpectrumSource>().toEqualTypeOf<{ kind: "file"; hdu: number | null }>();
    expectTypeOf<LineMeasurementSource>().toEqualTypeOf<SpectrumSource | FileSpectrumSource>();
    expectTypeOf<LineMeasurement["source"]>().toEqualTypeOf<MeasuredLineSource>();
  });

  it("accepts a spectrum picked from a time series without widening the cube and file sources", () => {
    expectTypeOf<SeriesSpectrumSource>().toEqualTypeOf<{
      kind: "series";
      paths: string[];
      detector: string;
      sporder: number | null;
      pick: TsoPick;
    }>();
    expectTypeOf<MeasuredLineSource>().toEqualTypeOf<LineMeasurementSource | SeriesSpectrumSource>();
    expectTypeOf<Parameters<typeof measureSpectralLine>[1]>().toEqualTypeOf<MeasuredLineSource>();
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
    expectTypeOf<MeasureLineOptions["redshift"]>().toEqualTypeOf<number | null | undefined>();
  });

  it("carries the source-frame velocities, the continuum chi2 and the nominal channel count of wave 0", () => {
    expectTypeOf<LineVelocity["redshift"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<LineVelocity["source_centroid_kms"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineVelocity["source_sigma_kms"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineVelocity["source_fwhm_kms"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineMeasurement["continuum_chi2_red"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineMeasurement["n_channels_nominal"]>().toEqualTypeOf<number | undefined>();
  });

  it("carries the errors of equivalent width, centroid, sigma and FWHM and the two reliability flags", () => {
    expectTypeOf<LineMeasurement["equivalent_width_err"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineMeasurement["centroid_err"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineMeasurement["sigma_err"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineMeasurement["fwhm_err"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineMeasurement["errors_reliable"]>().toEqualTypeOf<boolean | undefined>();
    expectTypeOf<LineMeasurement["width_errors_reliable"]>().toEqualTypeOf<boolean | undefined>();
    expectTypeOf<LineVelocity["centroid_err_kms"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineVelocity["sigma_err_kms"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineVelocity["fwhm_err_kms"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineVelocity["source_centroid_err_kms"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineVelocity["source_sigma_err_kms"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineVelocity["source_fwhm_err_kms"]>().toEqualTypeOf<number | null | undefined>();
  });

  it("carries the identity, the requested redshift, the wavelength frame, the peak wavelength and the redshift of the line", () => {
    expectTypeOf<MeasureLineOptions["lineId"]>().toEqualTypeOf<string | null | undefined>();
    expectTypeOf<MeasureLineOptions["lineLabel"]>().toEqualTypeOf<string | null | undefined>();
    expectTypeOf<LineMeasurement["line_id"]>().toEqualTypeOf<string | null | undefined>();
    expectTypeOf<LineMeasurement["line_label"]>().toEqualTypeOf<string | null | undefined>();
    expectTypeOf<LineMeasurement["redshift"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineMeasurement["wavelength_frame"]>().toEqualTypeOf<"air" | "vacuum" | null | undefined>();
    expectTypeOf<LineMeasurement["peak_axis"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineVelocity["z_line"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineVelocity["z_line_err"]>().toEqualTypeOf<number | null | undefined>();
  });

  it("carries the physical line flux and its basis", () => {
    expectTypeOf<LineMeasurement["flux_si"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineMeasurement["flux_si_err"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineMeasurement["flux_cgs"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineMeasurement["flux_cgs_err"]>().toEqualTypeOf<number | null | undefined>();
    expectTypeOf<LineMeasurement["flux_physical_basis"]>().toEqualTypeOf<string | undefined>();
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
      redshift: null,
      lineId: null,
      lineLabel: null,
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
      redshift: null,
      lineId: null,
      lineLabel: null,
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
      redshift: null,
      lineId: null,
      lineLabel: null,
    });
  });

  it("sends the identity of the picked line", async () => {
    typedInvokeMock.mockResolvedValue({ flux: 1 });
    await measureSpectralLine(
      "C:/d/qso.fits",
      { kind: "file", hdu: 0 },
      { z0: 10, z1: 30, restUm: 0.656461, redshift: 0.5, lineId: "h_alpha", lineLabel: "Hα" },
    );
    expect(typedInvokeMock).toHaveBeenCalledWith(
      "measure_spectral_line_cmd",
      expect.objectContaining({ lineId: "h_alpha", lineLabel: "Hα", redshift: 0.5 }),
    );
  });

  it("sends a series pick as the source", async () => {
    typedInvokeMock.mockResolvedValue({ flux: 1 });
    const source: SeriesSpectrumSource = {
      kind: "series",
      paths: ["C:/d/a_x1dints.fits", "C:/d/b_x1dints.fits"],
      detector: "NRS2",
      sporder: -1,
      pick: { kind: "mean", first: 3, last: 9 },
    };
    await measureSpectralLine("C:/d/a_x1dints.fits", source, { z0: 1170, z1: 1321 });
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("measure_spectral_line_cmd", {
      path: "C:/d/a_x1dints.fits",
      source: {
        kind: "series",
        paths: ["C:/d/a_x1dints.fits", "C:/d/b_x1dints.fits"],
        detector: "NRS2",
        sporder: -1,
        pick: { kind: "mean", first: 3, last: 9 },
      },
      z0: 1170,
      z1: 1321,
      continuum: null,
      restUm: null,
      convention: "optical",
      model: "none",
      velocityShiftKms: null,
      useErr: null,
      useDq: null,
      wavelengthFrame: null,
      redshift: null,
      lineId: null,
      lineLabel: null,
    });
    expect(typedInvokeMock.mock.calls[0][1].source).toBe(source);
  });

  it("sends the redshift of the line list as typed, zero included", async () => {
    typedInvokeMock.mockResolvedValue({ flux: 1 });
    const source: FileSpectrumSource = { kind: "file", hdu: 0 };
    await measureSpectralLine("C:/d/qso.fits", source, { z0: 10, z1: 30, restUm: 0.65646, redshift: 0.5 });
    expect(typedInvokeMock).toHaveBeenLastCalledWith("measure_spectral_line_cmd", expect.objectContaining({ redshift: 0.5 }));
    await measureSpectralLine("C:/d/qso.fits", source, { z0: 10, z1: 30, restUm: 0.65646, redshift: 0 });
    expect(typedInvokeMock).toHaveBeenLastCalledWith("measure_spectral_line_cmd", expect.objectContaining({ redshift: 0 }));
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
