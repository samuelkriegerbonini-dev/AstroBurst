import { describe, it, expect } from "vitest";
import {
  CSV_SPECTRUM_HINT,
  FITS_TABLE_NOTE_HINT,
  TABLE_NOTE_HINT,
  isSpectrumSeries,
  layerNotes,
  layerTag,
  previewableFile,
  seriesRowText,
  spectrumHdu,
  spectrumRowNote,
  tableNoteOf,
  tableRowAction,
  tableRowText,
  tableRowTitle,
} from "../fileKinds";
import { FILE_STATUS } from "../constants";
import type { Spectrum1dKind } from "../../shared/types/spectrum1d";
import type { TableProbe, TableProbeColumn, TableProbeHdu } from "../../shared/types/tables";

type SeriesKind = Extract<Spectrum1dKind, { kind: "x1dints" }>;

const E1: SeriesKind = { kind: "x1dints", layout: "rows", n_series: 1, n_integrations: 240, n_channels: 431 };
const B1: SeriesKind = { kind: "x1dints", layout: "extensions", n_series: 1, n_integrations: 10, n_channels: 388 };
const A3: SeriesKind = { kind: "x1dints", layout: "rows", n_series: 3, n_integrations: 5, n_channels: 2048 };

function columns(count: number): TableProbeColumn[] {
  return Array.from({ length: count }, (_, i) => ({ name: `c${i}`, unit: null, datatype: "float64", numeric: true }));
}

function hdus(count: number): TableProbeHdu[] {
  return Array.from({ length: count }, (_, i) => ({ hdu: i + 1, extname: null, n_rows: 500, n_columns: 1 }));
}

const C1_PROBE: TableProbe = {
  format: "csv",
  n_rows: 297,
  columns: columns(17),
  tables: null,
  meta: null,
  comments: [],
  guess: { kind: "catalogue", reason: "sky position columns ra and dec" },
  notes: [],
};

describe("tableRowText", () => {
  it("words a table row by its size and format", () => {
    expect(tableRowText({ format: "csv", n_rows: 297, columns: columns(17), tables: null })).toBe("Table, 297 rows x 17 columns (CSV)");
    expect(tableRowText({ format: "ecsv", n_rows: 240, columns: columns(3), tables: null })).toBe("Table, 240 rows x 3 columns (ECSV)");
    expect(tableRowText({ format: "ecsv", n_rows: 1, columns: columns(3), tables: null })).toBe("Table, 1 row x 3 columns (ECSV)");
    expect(tableRowText({ format: "fits_bintable", n_rows: 500, columns: columns(1), tables: hdus(3) })).toBe("Table, 500 rows x 1 column (FITS table), 3 tables");
    expect(tableRowText({ format: "fits_bintable", n_rows: 500, columns: columns(8), tables: hdus(1) })).toBe("Table, 500 rows x 8 columns (FITS table)");
    expect(tableRowText({ format: "csv", n_rows: 12345, columns: columns(2), tables: null })).toBe("Table, 12345 rows x 2 columns (CSV)");
  });

  it("titles a table row with the reason of its guess when there is one", () => {
    expect(tableRowTitle(C1_PROBE)).toBe("Table, 297 rows x 17 columns (CSV); sky position columns ra and dec");
    expect(tableRowTitle({ ...C1_PROBE, guess: { kind: "unknown", reason: "" } })).toBe("Table, 297 rows x 17 columns (CSV)");
  });

  it("names the app's CSV spectrum", () => {
    expect(CSV_SPECTRUM_HINT).toBe("1D spectrum, CSV file");
  });
});

describe("previewableFile", () => {
  it("keeps a table out of the image viewer and lets every other row through", () => {
    const table = { status: "table" as const, id: "t" };
    const image = { status: "done" as const, id: "i" };
    const spectrum = { status: "spectrum" as const, id: "s" };
    expect(previewableFile(table)).toBeNull();
    expect(previewableFile(null)).toBeNull();
    expect(previewableFile(image)).toBe(image);
    expect(previewableFile(spectrum)).toBe(spectrum);
  });
});

describe("isSpectrumSeries", () => {
  it("tells time-series spectra from other spectrum rows", () => {
    expect(isSpectrumSeries({ status: "spectrum", spectrum: E1 })).toBe(true);
    expect(isSpectrumSeries({ status: "spectrum", spectrum: { kind: "x1d", hdu: 1 } })).toBe(false);
    expect(isSpectrumSeries({ status: "spectrum" })).toBe(false);
    expect(isSpectrumSeries({ status: "done", spectrum: E1 })).toBe(false);
    expect(isSpectrumSeries(null)).toBe(false);
    expect(isSpectrumSeries(undefined)).toBe(false);
    expect(FILE_STATUS.TABLE).toBe("table");
  });
});

describe("spectrumHdu", () => {
  it("reads the HDU of a probe kind that has one", () => {
    expect(spectrumHdu({ kind: "x1d", hdu: 2 })).toBe(2);
    expect(spectrumHdu({ kind: "wcs1d", hdu: 0 })).toBe(0);
    expect(spectrumHdu(E1)).toBeNull();
    expect(spectrumHdu(null)).toBeNull();
    expect(spectrumHdu(undefined)).toBeNull();
  });
});

describe("seriesRowText", () => {
  it("words a time-series row by its integrations, channels and series", () => {
    expect(seriesRowText(E1)).toBe("Time-series spectra, 240 integrations x 431 channels, 1 series");
    expect(seriesRowText(A3)).toBe("Time-series spectra, 5 integrations x 2048 channels, 3 series");
    expect(seriesRowText({ n_series: 1, n_integrations: 1, n_channels: 1 })).toBe("Time-series spectra, 1 integration x 1 channel, 1 series");
    expect(seriesRowText({ n_series: 2, n_integrations: 0, n_channels: 1271 })).toBe("Time-series spectra, 0 integrations x 1271 channels, 2 series");
  });
});

describe("spectrumRowNote", () => {
  it("notes every kind of spectrum row", () => {
    expect(spectrumRowNote({ spectrum: E1 })).toEqual({
      text: "Time-series spectra, 240 integrations x 431 channels, 1 series",
      title: "Time-series spectra, 240 integrations x 431 channels, 1 series; one table row per integration",
      series: true,
    });
    expect(spectrumRowNote({ spectrum: B1 })).toEqual({
      text: "Time-series spectra, 10 integrations x 388 channels, 1 series",
      title: "Time-series spectra, 10 integrations x 388 channels, 1 series; one EXTRACT1D table per integration",
      series: true,
    });
    expect(spectrumRowNote({ spectrum: A3 })).toEqual({
      text: "Time-series spectra, 5 integrations x 2048 channels, 3 series",
      title: "Time-series spectra, 5 integrations x 2048 channels, 3 series; one table row per integration; counts of the first series",
      series: true,
    });
    const oneTable = { text: "1D spectrum, EXTRACT1D table", title: "1D spectrum, EXTRACT1D table, HDU 1", series: false };
    expect(spectrumRowNote({ spectrum: { kind: "x1d", hdu: 1 } })).toEqual(oneTable);
    expect(spectrumRowNote({ spectrum: { kind: "x1d", hdu: 1, n_tables: 1 } })).toEqual(oneTable);
    expect(spectrumRowNote({ spectrum: { kind: "x1d", hdu: 2, n_tables: 3 } })).toEqual({
      text: "1D spectra, 3 EXTRACT1D tables",
      title: "1D spectra, 3 EXTRACT1D tables, HDU 2 opens first",
      series: false,
    });
    expect(spectrumRowNote({ spectrum: { kind: "wcs1d", hdu: 0 } })).toEqual({ text: "1D spectrum, 1D array", title: "1D spectrum, 1D array, HDU 0", series: false });
    expect(spectrumRowNote({ table: { format: "csv" } })).toEqual({ text: "1D spectrum, CSV file", title: "1D spectrum, CSV file", series: false });
    expect(spectrumRowNote({})).toEqual({ text: "1D spectrum", title: "1D spectrum", series: false });
    expect(spectrumRowNote({ table: { format: "ecsv" } })).toEqual({ text: "1D spectrum", title: "1D spectrum", series: false });
  });
});

const WHTLT: TableProbe = {
  format: "ecsv",
  n_rows: 240,
  columns: ["MJD_UTC_NRS1", "BJD_TDB_NRS1", "whitelight_flux_NRS1"].map((name) => ({ name, unit: null, datatype: "float64", numeric: true })),
  tables: null,
  meta: null,
  comments: [],
  guess: { kind: "light_curve", reason: "time column MJD_UTC_NRS1 and value column whitelight_flux_NRS1" },
  notes: [],
};

const FITSCAT: TableProbe = {
  format: "fits_bintable",
  n_rows: 500,
  columns: columns(8),
  tables: [{ hdu: 1, extname: "CATALOG", n_rows: 500, n_columns: 8 }],
  meta: null,
  comments: [],
  guess: { kind: "catalogue", reason: "sky position columns RA and DEC" },
  notes: [],
};

const WHTLT_NAME = "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_whtlt.ecsv";

const LIGHT_CURVE_ACTION = { kind: "light_curve", label: "Open as light curve", title: "Open this table as a light curve in the Series section of Photometry" };

describe("tableRowAction", () => {
  it("offers one action per table kind and none for a FITS table", () => {
    expect(tableRowAction(WHTLT)).toEqual(LIGHT_CURVE_ACTION);
    expect(tableRowAction(C1_PROBE)).toEqual({ kind: "catalogue", label: "Use as catalogue", title: "Use this table as the catalogue of the Compare section of Photometry" });
    expect(tableRowAction(FITSCAT)).toBeNull();
    expect(tableRowAction({ format: "ecsv", guess: { kind: "unknown", reason: "" } })).toBeNull();
    expect(tableRowAction({ format: "ecsv", guess: { kind: "spectrum", reason: "columns wavelength and flux" } })).toBeNull();
  });
});

describe("tableNoteOf", () => {
  it("builds the viewer note of a selected table and of nothing else", () => {
    expect(tableNoteOf({ id: "t", name: WHTLT_NAME, status: "table", table: WHTLT })).toEqual({
      id: "t",
      name: WHTLT_NAME,
      text: "Table, 240 rows x 3 columns (ECSV)",
      reason: "time column MJD_UTC_NRS1 and value column whitelight_flux_NRS1",
      hint: "It has no image to show here.",
      action: LIGHT_CURVE_ACTION,
    });
    expect(TABLE_NOTE_HINT).toBe("It has no image to show here.");
    expect(FITS_TABLE_NOTE_HINT).toBe("Binary tables of a FITS file are listed only: save the table as CSV or ECSV to use it.");
    expect(tableNoteOf({ id: "f", name: "cat.fits", status: "table", table: FITSCAT })).toEqual({
      id: "f",
      name: "cat.fits",
      text: "Table, 500 rows x 8 columns (FITS table)",
      reason: "sky position columns RA and DEC",
      hint: FITS_TABLE_NOTE_HINT,
      action: null,
    });
    expect(tableNoteOf(null)).toBeNull();
    expect(tableNoteOf({ id: "d", name: "m42.fits", status: "done" })).toBeNull();
    expect(tableNoteOf({ id: "s", name: "spectrum.csv", status: "spectrum", table: { ...C1_PROBE, guess: { kind: "spectrum", reason: "" } } })).toBeNull();
    expect(tableNoteOf({ id: "t", name: "lost.csv", status: "table" })).toBeNull();
  });
});

describe("layer rows", () => {
  it("tags a layer by its role and says where the role came from", () => {
    expect(layerTag({ role: "inverse_variance", basis: "file name -invvar-", assumed: false })).toEqual({
      text: "inverse variance",
      title: "inverse variance layer, from file name -invvar-",
    });
    expect(layerTag({ role: "inverse_variance", basis: "EXTNAME WGT", assumed: true })).toEqual({
      text: "inverse variance (assumed)",
      title: "inverse variance (assumed) layer, from EXTNAME WGT",
    });
    expect(layerTag({ role: "sigma", basis: "EXTNAME RMS", assumed: false })?.text).toBe("sigma (error)");
    expect(layerTag({ role: "variance", basis: "EXTNAME VARIANCE", assumed: false })?.text).toBe("variance");
    expect(layerTag({ role: "mask", basis: "EXTNAME MASK", assumed: false })?.text).toBe("mask");
    expect(layerTag({ role: "science", basis: "EXTNAME SCI", assumed: false })).toBeNull();
    expect(layerTag({ role: "other", basis: "EXTNAME AREA", assumed: false })).toBeNull();
    expect(layerTag(null)).toBeNull();
    expect(layerTag(undefined)).toBeNull();
  });

  it("lists why a layer was not used, error first", () => {
    expect(
      layerNotes({
        err_unavailable: "error layer HDU 3 has 100 x 90 pixels, the image 100 x 100",
        dq_unavailable: "mask layer HDU 2 is a floating-point plane (BITPIX -32): flag bits cannot be read exactly",
      }),
    ).toEqual([
      "Error layer not used: error layer HDU 3 has 100 x 90 pixels, the image 100 x 100",
      "Mask layer not used: mask layer HDU 2 is a floating-point plane (BITPIX -32): flag bits cannot be read exactly",
    ]);
    expect(layerNotes({ err_unavailable: null, dq_unavailable: "" })).toEqual([]);
    expect(layerNotes({})).toEqual([]);
    expect(layerNotes(null)).toEqual([]);
    expect(layerNotes(undefined)).toEqual([]);
  });
});
