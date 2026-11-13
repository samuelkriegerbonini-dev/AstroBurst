import { describe, expect, it, vi } from "vitest";
import {
  AIR_FRAME_FORMULA_NOTE,
  OPEN_SPECTRUM_CSV_LABEL,
  cubeToolContent,
  defaultFrameChoice,
  documentAxis,
  documentDetails,
  documentDqExclusions,
  documentFrameLine,
  documentFrameOrigin,
  documentSourceHdu,
  documentSourcePath,
  documentSummary,
  dqExclusionMask,
  openSpectrumCsvPaletteAction,
  seriesPickText,
  seriesText,
  wavelengthUnitAssumed,
} from "../spectrumDocument";
import { spectrumHdu } from "../fileKinds";
import type { SpectralAxisInfo } from "../../shared/types/spectral";
import type { Spectrum1d, Spectrum1dKind } from "../../shared/types/spectrum1d";

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
    const vacuumDoc = doc({ wavelength_frame: "vacuum", axis: { ...AIR_AXIS, kind: "wave" } });
    expect(documentFrameLine(doc(), "air")).toBe("air wavelengths, as the file reads");
    expect(documentFrameLine(vacuumDoc, "vacuum")).toBe("vacuum wavelengths, as the file reads");
    expect(documentFrameLine(doc(), "vacuum")).toBe("vacuum wavelengths, set here (the file reads as air)");
    expect(documentFrameLine(vacuumDoc, "air")).toBe("air wavelengths, set here (the file reads as vacuum)");
    expect(documentFrameLine(doc({ wavelength_frame: "not_wavelength" }), null)).toBe(
      "the spectral axis is not a wavelength: no air or vacuum frame applies",
    );
  });

  it("says the frame was assumed when no card of the file names it", () => {
    const assumedAir = doc({
      notes: [
        "CTYPE1 'LINEAR' with no JWST, HST or Roman TELESCOP and no AIRORVAC or VACUUM card naming the frame: air wavelengths assumed, the ground-based default",
      ],
    });
    expect(documentFrameLine(assumedAir, "air")).toBe("air wavelengths, assumed (no card of the file names the frame)");
    expect(documentFrameLine(assumedAir, "vacuum")).toBe("vacuum wavelengths, set here (air was assumed for the file)");
    const assumedVacuum = doc({
      wavelength_frame: "vacuum",
      axis: { ...AIR_AXIS, kind: "wave" },
      notes: ["CTYPE1 'WAVELENGTH' is not a FITS spectral code: treated as WAVE, vacuum wavelengths assumed"],
    });
    expect(documentFrameLine(assumedVacuum, "vacuum")).toBe("vacuum wavelengths, assumed (no card of the file names the frame)");
    const unitAssumed = doc({ notes: ["CTYPE1 'AWAV': air wavelengths", "CUNIT1 absent: Angstrom assumed"] });
    expect(documentFrameLine(unitAssumed, "air")).toBe("air wavelengths, as the file reads");
  });

  it("names the card or the rule that gave the file its frame", () => {
    const vacuum = { wavelength_frame: "vacuum" as const, axis: { ...AIR_AXIS, kind: "wave" as const } };
    expect(documentFrameOrigin(doc({ ...vacuum, notes: ["EXTRACT1D table of a space-mission pipeline: vacuum wavelengths"] }))).toBe(
      "EXTRACT1D table of a space-mission pipeline",
    );
    expect(documentFrameOrigin(doc({ ...vacuum, notes: ["CTYPE1 'WAVE': vacuum wavelengths, the FITS definition of WAVE"] }))).toBe(
      "CTYPE1 'WAVE'",
    );
    expect(documentFrameOrigin(doc({ notes: ["CTYPE1 'WAVELENGTH' with AIRORVAC 'air': air wavelengths"] }))).toBe(
      "CTYPE1 'WAVELENGTH' with AIRORVAC 'air'",
    );
    expect(documentFrameOrigin(doc({ notes: ["CSV file, column wavelength_air_um: air wavelengths"] }))).toBe(
      "CSV file, column wavelength_air_um",
    );
    expect(
      documentFrameOrigin(
        doc({
          notes: [
            "CTYPE1 'LINEAR' with no JWST, HST or Roman TELESCOP and no AIRORVAC or VACUUM card naming the frame: air wavelengths assumed, the ground-based default",
          ],
        }),
      ),
    ).toBeNull();
    expect(
      documentFrameOrigin(
        doc({ ...vacuum, notes: ["CTYPE1 'WAVELENGTH' is not a FITS spectral code: treated as WAVE, vacuum wavelengths assumed"] }),
      ),
    ).toBeNull();
    expect(documentFrameOrigin(doc())).toBeNull();
    expect(documentFrameOrigin(doc({ notes: ["CUNIT1 absent: Angstrom assumed", "flux in erg/s/cm2/A"] }))).toBeNull();
    expect(
      documentFrameOrigin(doc({ wavelength_frame: "not_wavelength", notes: ["CTYPE1 'AWAV': air wavelengths"] })),
    ).toBeNull();
    expect(
      documentFrameOrigin(doc({ notes: ["CUNIT1 absent: Angstrom assumed", "  CTYPE1 'AWAV' : air wavelengths", "CTYPE1 'WAVE': vacuum wavelengths"] })),
    ).toBe("CTYPE1 'AWAV'");
  });

  it("names the one refractive-index formula that converts air and vacuum for the plot and the line velocities", () => {
    expect(AIR_FRAME_FORMULA_NOTE).toBe("air and vacuum wavelengths are converted with Morton (2000), for the plot and for line velocities");
    expect(AIR_FRAME_FORMULA_NOTE).not.toContain("Greisen");
  });
});

describe("documentDqExclusions", () => {
  it("excludes the rows the line measurement excludes: bit 0 for JWST and Roman, 0x3FFF for HST", () => {
    expect(documentDqExclusions(doc({ dq: [0, 1, 4, 5], dq_table: "jwst" }))).toEqual([false, true, false, true]);
    expect(documentDqExclusions(doc({ dq: [0, 1, 4, 5], dq_table: "roman" }))).toEqual([false, true, false, true]);
    expect(documentDqExclusions(doc({ dq: [0, 4, 0x4000, 0x2000], dq_table: "hst" }))).toEqual([false, true, false, true]);
  });

  it("excludes nothing without a DQ column or with an unknown flag table", () => {
    expect(documentDqExclusions(doc({ dq: null, dq_table: "jwst" }))).toBeNull();
    expect(documentDqExclusions(doc({ dq: [0, 1, 2], dq_table: "unknown" }))).toBeNull();
  });

  it("gives the exclusion mask of each flag table", () => {
    expect(dqExclusionMask("jwst")).toBe(1);
    expect(dqExclusionMask("roman")).toBe(1);
    expect(dqExclusionMask("hst")).toBe(0x3fff);
    expect(dqExclusionMask("unknown")).toBe(0);
  });
});

describe("documentDetails", () => {
  it("says what a pipeline spectrum is and where its errors and flags come from", () => {
    const x1d = doc({
      source: { kind: "x1d", path: "C:/data/jw_x1d.fits", hdu: 1, extver: 1 },
      cards: { INSTRUME: "NIRSPEC", GRATING: "G235H", FILTER: "F170LP", TARGNAME: "NGC 7469", SRCTYPE: "POINT" },
      flux: [1, 2, 3, 4, 5],
      err: [0.1, 0.1, 0.1, 0.1, 0.1],
      dq: [0, 1, 0, 4, 1],
      dq_table: "jwst",
    });
    expect(documentDetails(x1d)).toBe(
      "NIRSPEC G235H F170LP, target NGC 7469, source type POINT, errors: FLUX_ERROR column, DQ: jwst flags, 2 channels excluded",
    );
  });

  it("says as much about a ground-based array and a CSV as they carry", () => {
    expect(documentDetails(doc({ cards: { OBJECT: "HD 37795" }, err: null, dq: null }))).toBe("target HD 37795, no errors, no DQ");
    const csv = doc({
      source: { kind: "csv", path: "C:/data/wasp39.csv" },
      cards: { instrument: "NIRSPEC", disperser: "G395H", target: "WASP-39" },
      err: [0.1, 0.1, 0.1],
      dq: [0, 0, 1],
      dq_table: "unknown",
    });
    expect(documentDetails(csv)).toBe("NIRSPEC G395H, target WASP-39, errors: err column, DQ: unknown flag table, not used");
    expect(documentDetails(doc({ cards: {}, err: null, dq: null }))).toBe("no errors, no DQ");
  });

  it("names the error extension of an array and the error column of a time series", () => {
    expect(documentDetails(doc({ err: [0.1, 0.1, 0.1] }))).toBe("errors: ERR extension, no DQ");
    const series = doc({
      source: { kind: "x1dints", paths: ["C:/data/jw_x1dints.fits"], detector: "NRS1", sporder: null, pick: { kind: "median" } },
      cards: { INSTRUME: "NIRSPEC", DETECTOR: "NRS1" },
      err: [0.1, 0.1, 0.1],
      dq: [0, 0, 0],
      dq_table: "jwst",
    });
    expect(documentDetails(series)).toBe("NIRSPEC NRS1, errors: FLUX_ERROR column, DQ: jwst flags, 0 channels excluded");
    expect(documentDetails({ ...series, dq: [0, 1, 0] })).toBe("NIRSPEC NRS1, errors: FLUX_ERROR column, DQ: jwst flags, 1 channel excluded");
  });
});

describe("wavelengthUnitAssumed", () => {
  it("knows when the wavelength unit was a default", () => {
    const missingNote = "CUNIT1 missing: assumed the FITS default 'm' for WAVE";
    const irafNote = "WAT1 has no units= and CUNIT1 is missing: Angstrom assumed";
    const frameNote = "CTYPE1 = 'LINEAR': air wavelengths assumed, the ground-based default";
    expect(wavelengthUnitAssumed(doc({ wavelength_unit_source: "assumed" }))).toBe(true);
    expect(wavelengthUnitAssumed(doc({ wavelength_unit_source: "file", notes: [irafNote] }))).toBe(false);
    expect(wavelengthUnitAssumed(doc({ wavelength_unit_source: "user", axis: { ...AIR_AXIS, notes: [missingNote] } }))).toBe(false);
    expect(wavelengthUnitAssumed(doc({ axis: { ...AIR_AXIS, notes: [missingNote] } }))).toBe(true);
    expect(wavelengthUnitAssumed(doc({ notes: [irafNote] }))).toBe(true);
    expect(wavelengthUnitAssumed(doc({ notes: [frameNote] }))).toBe(false);
  });
});

describe("documentSummary", () => {
  it("names the channel count, the HDU layout and the flux unit", () => {
    expect(documentSummary(doc())).toBe("3 channels, 1D array in HDU 0, flux in erg/s/cm2/A");
    expect(
      documentSummary(
        doc({
          source: { kind: "x1d", path: "C:/data/jw_x1d.fits", hdu: 1, extver: 1 },
          flux: Array.from({ length: 3400 }, () => 1),
          flux_unit: "Jy",
          n_rows: 3400,
        }),
      ),
    ).toBe("3400 channels, EXTRACT1D table in HDU 1, flux in Jy");
    expect(documentSummary(doc({ flux_unit: null }))).toBe("3 channels, 1D array in HDU 0, flux in native units");
  });

  it("counts the plotted samples of a one-row vector table, not its rows", () => {
    expect(
      documentSummary(
        doc({ source: { kind: "x1d", path: "C:/data/jw_x1dints.fits", hdu: 1, extver: 1 }, flux: [1, 2, 3], n_rows: 1, flux_unit: "Jy" }),
      ),
    ).toBe("3 channels, EXTRACT1D table in HDU 1, flux in Jy");
  });

  it("names a CSV spectrum as a CSV file", () => {
    expect(documentSummary(doc({ source: { kind: "csv", path: "C:/data/ha_spectrum.csv" }, flux_unit: "Jy" }))).toBe(
      "3 channels, CSV file, flux in Jy",
    );
  });

  it("names a pick of a time series by detector, order and integrations", () => {
    const paths = ["C:/d/jw03375-o001_x1dints.fits"];
    const series = (source: Spectrum1d["source"]) => doc({ source, flux: [1, 2, 3, 4], flux_unit: "Jy" });
    expect(documentSummary(series({ kind: "x1dints", paths, detector: "NRS1", sporder: -1, pick: { kind: "median" } }))).toBe(
      "4 channels, time-series spectra, NRS1: median of all integrations, flux in Jy",
    );
    expect(
      documentSummary(series({ kind: "x1dints", paths, detector: "NIS", sporder: 2, pick: { kind: "integration", index: 2 } })),
    ).toBe("4 channels, time-series spectra, NIS, order 2: integration 3, flux in Jy");
    expect(
      documentSummary(
        series({ kind: "x1dints", paths, detector: "MIRIMAGE", sporder: 1, pick: { kind: "mean", first: 0, last: 63 } }),
      ),
    ).toBe("4 channels, time-series spectra, MIRIMAGE: mean of integrations 1 to 64, flux in Jy");
    expect(seriesPickText({ kind: "mean", first: 2, last: 2 })).toBe("mean of integrations 3 to 3");
  });

  it("prints an order only where it tells series apart", () => {
    expect(seriesText("NIS", 1)).toBe("NIS, order 1");
    expect(seriesText("NRCALONG", 1)).toBe("NRCALONG");
    expect(seriesText("NRS1", 0)).toBe("NRS1");
    expect(seriesText("NRS2", -1)).toBe("NRS2");
    expect(seriesText("NIS", null)).toBe("NIS");
    expect(seriesText("NIS", 3)).toBe("NIS, order 3");
  });
});

describe("document sources", () => {
  it("gives the path and the HDU of every source kind", () => {
    const x1d: Spectrum1d["source"] = { kind: "x1d", path: "C:/d/a_x1d.fits", hdu: 2, extver: 1 };
    const wcs1d: Spectrum1d["source"] = { kind: "wcs1d", path: "C:/d/b.fits", hdu: 0 };
    const csv: Spectrum1d["source"] = { kind: "csv", path: "C:/d/c.csv" };
    const series: Spectrum1d["source"] = {
      kind: "x1dints",
      paths: ["C:/d/jw03375-o001_x1dints.fits", "C:/d/jw03375-o001_seg002_x1dints.fits"],
      detector: "NRS1",
      sporder: -1,
      pick: { kind: "median" },
    };
    expect(documentSourcePath(x1d)).toBe("C:/d/a_x1d.fits");
    expect(documentSourcePath(wcs1d)).toBe("C:/d/b.fits");
    expect(documentSourcePath(csv)).toBe("C:/d/c.csv");
    expect(documentSourcePath(series)).toBe("C:/d/jw03375-o001_x1dints.fits");
    expect(documentSourcePath({ ...series, paths: [] })).toBe("");
    expect(documentSourceHdu(x1d)).toBe(2);
    expect(documentSourceHdu(wcs1d)).toBe(0);
    expect(documentSourceHdu(csv)).toBeNull();
    expect(documentSourceHdu(series)).toBeNull();
  });

  it("a time-series kind has no HDU of its own", () => {
    const kind: Spectrum1dKind = { kind: "x1dints", layout: "rows", n_series: 2, n_integrations: 8, n_channels: 1271 };
    expect(spectrumHdu(kind)).toBeNull();
    expect(spectrumHdu({ kind: "x1d", hdu: 2, n_tables: 3 })).toBe(2);
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

  it("names the Spectra tool and is found by spectra and by cube", () => {
    const action = openSpectrumCsvPaletteAction(() => {});
    expect(action.hint).toBe("Spectra tool");
    expect(action.keywords).toEqual(expect.arrayContaining(["spectra", "spectrum", "csv", "cube"]));
  });
});
