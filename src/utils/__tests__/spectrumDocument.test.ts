import { describe, expect, it, vi } from "vitest";
import {
  AIR_FRAME_FORMULA_NOTE,
  OPEN_SPECTRUM_CSV_LABEL,
  cubeToolContent,
  defaultFrameChoice,
  documentAxis,
  documentFrameLine,
  documentSummary,
  openSpectrumCsvPaletteAction,
} from "../spectrumDocument";
import type { SpectralAxisInfo } from "../../shared/types/spectral";
import type { Spectrum1d } from "../../shared/types/spectrum1d";

const AIR_AXIS: SpectralAxisInfo = {
  kind: "awav",
  ctype: "LINEAR",
  unit: "um",
  header_unit: "angstroms",
  header_scale: 1e-4,
  values: [0.6555, 0.6556, 0.6557],
  crval: 0.6555,
  cdelt: 1e-4,
  crpix: 1,
  rest_wavelength_um: null,
  rest_frequency_hz: null,
  specsys: null,
  velosys: null,
  notes: [],
};

function doc(extra: Partial<Spectrum1d> = {}): Spectrum1d {
  return {
    source: { kind: "wcs1d", path: "C:/data/ha.fits", hdu: 0 },
    axis: AIR_AXIS,
    wavelength_frame: "air",
    precision: "f64",
    flux: [1, 2, 1],
    err: null,
    dq: null,
    dq_table: "unknown",
    flux_unit: "erg/s/cm2/A",
    n_rows: 3,
    cards: {},
    notes: [],
    ...extra,
  };
}

describe("document wavelength frame", () => {
  it("starts from the frame the file reads as, and has no choice for an axis that is not a wavelength", () => {
    expect(defaultFrameChoice("air")).toBe("air");
    expect(defaultFrameChoice("vacuum")).toBe("vacuum");
    expect(defaultFrameChoice("not_wavelength")).toBeNull();
    expect(defaultFrameChoice(undefined)).toBeNull();
  });

  it("reads the native axis as vacuum when the vacuum frame is chosen for an air file, and back", () => {
    const asVacuum = documentAxis(AIR_AXIS, "vacuum");
    expect(asVacuum.kind).toBe("wave");
    expect(asVacuum.values).toBe(AIR_AXIS.values);
    expect(documentAxis(AIR_AXIS, "air")).toBe(AIR_AXIS);
    const vacuumAxis: SpectralAxisInfo = { ...AIR_AXIS, kind: "wave", ctype: "WAVE" };
    expect(documentAxis(vacuumAxis, "air").kind).toBe("awav");
    expect(documentAxis(vacuumAxis, "vacuum")).toBe(vacuumAxis);
    expect(documentAxis(vacuumAxis, null)).toBe(vacuumAxis);
  });

  it("states the frame, and says when the choice differs from the file", () => {
    expect(documentFrameLine("air", "air")).toBe("air wavelengths, as the file reads");
    expect(documentFrameLine("vacuum", "vacuum")).toBe("vacuum wavelengths, as the file reads");
    expect(documentFrameLine("air", "vacuum")).toBe("vacuum wavelengths, set here (the file reads as air)");
    expect(documentFrameLine("vacuum", "air")).toBe("air wavelengths, set here (the file reads as vacuum)");
    expect(documentFrameLine("not_wavelength", null)).toBe("the spectral axis is not a wavelength: no air or vacuum frame applies");
  });

  it("names both refractive-index formulas for an air frame", () => {
    expect(AIR_FRAME_FORMULA_NOTE).toContain("Morton (2000)");
    expect(AIR_FRAME_FORMULA_NOTE).toContain("Greisen et al. 2006");
  });
});

describe("documentSummary", () => {
  it("names the channel count, the HDU layout and the flux unit", () => {
    expect(documentSummary(doc())).toBe("3 channels, 1D array in HDU 0, flux in erg/s/cm2/A");
    expect(
      documentSummary(
        doc({ source: { kind: "x1d", path: "C:/data/jw_x1d.fits", hdu: 1, extver: 1 }, flux_unit: "Jy", n_rows: 3400 }),
      ),
    ).toBe("3400 channels, EXTRACT1D table in HDU 1, flux in Jy");
    expect(documentSummary(doc({ flux_unit: null }))).toBe("3 channels, 1D array in HDU 0, flux in native units");
  });

  it("names a CSV spectrum as a CSV file", () => {
    expect(documentSummary(doc({ source: { kind: "csv", path: "C:/data/ha_spectrum.csv" }, flux_unit: "Jy" }))).toBe(
      "3 channels, CSV file, flux in Jy",
    );
  });
});

describe("cubeToolContent", () => {
  const none = { isCube: false, isRamp: false, isSpectrum: false, fileKey: "img|C:/i.fits", csv: null };

  it("is empty when the selected file gives the Cube tool nothing and no CSV is open", () => {
    expect(cubeToolContent(none)).toBe("empty");
    expect(cubeToolContent({ ...none, fileKey: null })).toBe("empty");
  });

  it("shows a cube or a ramp, and a selected 1D spectrum as a document", () => {
    expect(cubeToolContent({ ...none, isCube: true })).toBe("cube");
    expect(cubeToolContent({ ...none, isRamp: true })).toBe("cube");
    expect(cubeToolContent({ ...none, isSpectrum: true })).toBe("spectrum");
  });

  it("shows an open CSV when the selected file gives the Cube tool nothing", () => {
    expect(cubeToolContent({ ...none, csv: { path: "C:/s.csv", over: "other|C:/o.fits" } })).toBe("csv");
  });

  it("shows the CSV over the cube or spectrum that was selected when it was opened, and that file once another one is picked", () => {
    const csv = { path: "C:/s.csv", over: "cube|C:/c.fits" };
    expect(cubeToolContent({ ...none, isCube: true, fileKey: "cube|C:/c.fits", csv })).toBe("csv");
    expect(cubeToolContent({ ...none, isCube: true, fileKey: "cube2|C:/c2.fits", csv })).toBe("cube");
    expect(cubeToolContent({ ...none, isSpectrum: true, fileKey: "x1d|C:/x.fits", csv })).toBe("spectrum");
    expect(cubeToolContent({ ...none, fileKey: null, csv: { path: "C:/s.csv", over: null } })).toBe("csv");
  });
});

describe("openSpectrumCsvPaletteAction", () => {
  it("is found by spectrum and csv and runs the open flow", () => {
    const run = vi.fn();
    const action = openSpectrumCsvPaletteAction(run);
    expect(action.id).toBe("open-spectrum-csv");
    expect(action.label).toBe(`${OPEN_SPECTRUM_CSV_LABEL}...`);
    expect(OPEN_SPECTRUM_CSV_LABEL).toBe("Open spectrum (CSV)");
    expect(action.keywords).toEqual(expect.arrayContaining(["spectrum", "csv"]));
    action.run();
    expect(run).toHaveBeenCalledTimes(1);
  });
});
