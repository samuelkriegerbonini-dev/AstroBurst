import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock }));

import {
  fromSpectrum1dWire,
  isSpectrumCsvPath,
  probeSpectrum1d,
  readSpectrum1d,
  readSpectrumCsv,
  readSpectrumDocument,
} from "../spectrum1d";
import type { ProcessedFile } from "../../shared/types";
import type { SpectralAxisInfo } from "../../shared/types/spectral";
import type {
  Spectrum1d,
  Spectrum1dKind,
  Spectrum1dSource,
  Spectrum1dWire,
  SpectrumPrecision,
  WavelengthFrame,
} from "../../shared/types/spectrum1d";

function wire(extra: Partial<Spectrum1dWire> = {}): Spectrum1dWire {
  return {
    source: { kind: "x1d", path: "C:/d/jw_x1d.fits", hdu: 1, extver: 1 },
    axis: {
      kind: "wave",
      ctype: "WAVELENGTH",
      unit: "um",
      header_unit: "um",
      header_scale: 1,
      values: [2.87, 2.88, 2.89, 2.9],
      crval: 2.87,
      cdelt: 0.01,
      crpix: 1,
      rest_wavelength_um: null,
      rest_frequency_hz: null,
      specsys: null,
      velosys: null,
      notes: [],
      tabulated: true,
    },
    wavelength_frame: "vacuum",
    precision: "f64",
    flux: [1.5, null, 2.5, null],
    err: [0.1, null, 0.2, 0.3],
    dq: [0, 1, 0, 2147483649],
    dq_table: "jwst",
    flux_unit: "Jy",
    n_rows: 1,
    cards: { TELESCOP: "JWST", GRATING: "G395H" },
    notes: ["EXTRACT1D table of a space-mission pipeline: vacuum wavelengths"],
    ...extra,
  };
}

describe("spectrum1d types", () => {
  it("pins the document shapes the backend serialises", () => {
    expectTypeOf<Spectrum1dKind>().toEqualTypeOf<{ kind: "x1d"; hdu: number } | { kind: "wcs1d"; hdu: number }>();
    expectTypeOf<Spectrum1dSource>().toEqualTypeOf<
      | { kind: "x1d"; path: string; hdu: number; extver: number | null }
      | { kind: "wcs1d"; path: string; hdu: number }
      | { kind: "csv"; path: string }
    >();
    expectTypeOf<WavelengthFrame>().toEqualTypeOf<"air" | "vacuum" | "not_wavelength">();
    expectTypeOf<SpectrumPrecision>().toEqualTypeOf<"f64" | "f32_widened">();
    expectTypeOf<Spectrum1dWire["flux"]>().toEqualTypeOf<(number | null)[]>();
    expectTypeOf<Spectrum1dWire["err"]>().toEqualTypeOf<(number | null)[] | null>();
    expectTypeOf<Spectrum1dWire["axis"]["values"]>().toEqualTypeOf<(number | null)[]>();
    expectTypeOf<Spectrum1d["flux"]>().toEqualTypeOf<number[]>();
    expectTypeOf<Spectrum1d["err"]>().toEqualTypeOf<number[] | null>();
    expectTypeOf<Spectrum1d["axis"]>().toEqualTypeOf<SpectralAxisInfo>();
    expectTypeOf<Spectrum1d["dq"]>().toEqualTypeOf<number[] | null>();
    expectTypeOf<Spectrum1d["flux_unit"]>().toEqualTypeOf<string | null>();
    expectTypeOf<Spectrum1d["cards"]>().toEqualTypeOf<Record<string, string>>();
  });

  it("lets a file row keep the probe verdict", () => {
    expectTypeOf<ProcessedFile["spectrum"]>().toEqualTypeOf<Spectrum1dKind | undefined>();
  });
});

describe("fromSpectrum1dWire", () => {
  it("maps null flux and err samples to NaN and keeps the finite ones", () => {
    const doc = fromSpectrum1dWire(wire());
    expect(doc.flux).toEqual([1.5, NaN, 2.5, NaN]);
    expect(doc.err).toEqual([0.1, NaN, 0.2, 0.3]);
  });

  it("keeps a missing error column null", () => {
    expect(fromSpectrum1dWire(wire({ err: null })).err).toBeNull();
  });

  it("maps null axis values and a null reference value or step to NaN", () => {
    const base = wire();
    const doc = fromSpectrum1dWire(wire({ axis: { ...base.axis, values: [null, 2.88, null, 2.9], crval: null, cdelt: null } }));
    expect(doc.axis.values).toEqual([NaN, 2.88, NaN, 2.9]);
    expect(doc.axis.crval).toBeNaN();
    expect(doc.axis.cdelt).toBeNaN();
    expect(doc.axis.crpix).toBe(1);
    expect(doc.axis.kind).toBe("wave");
    expect(doc.axis.tabulated).toBe(true);
  });

  it("passes the source, frame, precision, DQ bits, units, cards and notes through unchanged", () => {
    const raw = wire({
      source: { kind: "wcs1d", path: "C:/d/hd.fits", hdu: 0 },
      wavelength_frame: "air",
      precision: "f32_widened",
      dq_table: "unknown",
      flux_unit: null,
    });
    const doc = fromSpectrum1dWire(raw);
    expect(doc.source).toEqual({ kind: "wcs1d", path: "C:/d/hd.fits", hdu: 0 });
    expect(doc.wavelength_frame).toBe("air");
    expect(doc.precision).toBe("f32_widened");
    expect(doc.dq).toEqual([0, 1, 0, 2147483649]);
    expect(doc.dq_table).toBe("unknown");
    expect(doc.flux_unit).toBeNull();
    expect(doc.n_rows).toBe(1);
    expect(doc.cards).toEqual({ TELESCOP: "JWST", GRATING: "G395H" });
    expect(doc.notes).toEqual(["EXTRACT1D table of a space-mission pipeline: vacuum wavelengths"]);
  });

  it("does not mutate the payload it maps", () => {
    const raw = wire();
    fromSpectrum1dWire(raw);
    expect(raw.flux).toEqual([1.5, null, 2.5, null]);
    expect(raw.axis.values).toEqual([2.87, 2.88, 2.89, 2.9]);
  });
});

describe("readSpectrum1d", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("asks for the file with a null HDU by default and returns the mapped document", async () => {
    typedInvokeMock.mockResolvedValueOnce(wire());
    const doc = await readSpectrum1d("C:/d/jw_x1d.fits");
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("read_spectrum1d_cmd", { path: "C:/d/jw_x1d.fits", hdu: null });
    expect(doc.flux).toEqual([1.5, NaN, 2.5, NaN]);
  });

  it("passes an explicit HDU through", async () => {
    typedInvokeMock.mockResolvedValueOnce(wire());
    await readSpectrum1d("C:/d/hd.fits.gz", 2);
    expect(typedInvokeMock).toHaveBeenCalledWith("read_spectrum1d_cmd", { path: "C:/d/hd.fits.gz", hdu: 2 });
  });

  it("rejects with the backend refusal", async () => {
    typedInvokeMock.mockRejectedValueOnce(new Error("CTYPE1 = 'MULTISPE': IRAF multispec spectra are not supported"));
    await expect(readSpectrum1d("C:/d/ms.fits")).rejects.toThrow("IRAF multispec spectra are not supported");
  });
});

describe("readSpectrumCsv", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("reads the CSV through its own command and maps null samples to NaN", async () => {
    typedInvokeMock.mockResolvedValueOnce(wire({ source: { kind: "csv", path: "C:/d/spec.csv" } }));
    const doc = await readSpectrumCsv("C:/d/spec.csv");
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("read_spectrum_csv_cmd", { path: "C:/d/spec.csv" });
    expect(doc.source).toEqual({ kind: "csv", path: "C:/d/spec.csv" });
    expect(doc.flux).toEqual([1.5, NaN, 2.5, NaN]);
  });
});

describe("readSpectrumDocument", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue(wire());
  });

  it("tells a CSV path from a FITS path by its extension, in any case", () => {
    expect(isSpectrumCsvPath("C:/d/spec.csv")).toBe(true);
    expect(isSpectrumCsvPath("C:/d/SPEC.CSV")).toBe(true);
    expect(isSpectrumCsvPath("C:/d/jw_x1d.fits")).toBe(false);
    expect(isSpectrumCsvPath("C:/d/csv/jw_x1d.fits.gz")).toBe(false);
    expect(isSpectrumCsvPath("C:/d/spec.csv.fits")).toBe(false);
  });

  it("reads a CSV path with the CSV command and ignores the HDU", async () => {
    await readSpectrumDocument("C:/d/SPEC.CSV", 3);
    expect(typedInvokeMock).toHaveBeenCalledWith("read_spectrum_csv_cmd", { path: "C:/d/SPEC.CSV" });
  });

  it("reads any other path with the spectrum file command and its HDU", async () => {
    await readSpectrumDocument("C:/d/jw_x1d.fits", 1);
    expect(typedInvokeMock).toHaveBeenCalledWith("read_spectrum1d_cmd", { path: "C:/d/jw_x1d.fits", hdu: 1 });
  });
});

describe("probeSpectrum1d", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("returns the kind and HDU the backend found", async () => {
    typedInvokeMock.mockResolvedValueOnce({ kind: "wcs1d", hdu: 0 });
    await expect(probeSpectrum1d("C:/d/hd.fits")).resolves.toEqual({ kind: "wcs1d", hdu: 0 });
    expect(typedInvokeMock).toHaveBeenCalledWith("probe_spectrum1d_cmd", { path: "C:/d/hd.fits" });
  });

  it("returns null for a cube or an image", async () => {
    typedInvokeMock.mockResolvedValueOnce(null);
    await expect(probeSpectrum1d("C:/d/cube_s3d.fits")).resolves.toBeNull();
  });
});
