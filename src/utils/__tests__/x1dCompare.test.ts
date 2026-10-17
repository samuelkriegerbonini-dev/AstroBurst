import { describe, it, expect, vi } from "vitest";
import {
  APERTURE_NOTE,
  DOCUMENT_ENTRY_ID,
  TABLE_ENTRY_ID,
  TABLE_SERIES_COLOR,
  comparableX1dIds,
  documentAsTable,
  documentComparisonEntries,
  entryFluxUnit,
  findSiblingCube,
  interpolateOnto,
  readComparisonTable,
  resampleTable,
  tableEntry,
  tableLabel,
  usableRows,
  withTable,
  x1dStem,
} from "../x1dCompare";
import type { ComparisonEntry } from "../spectrumCompare";
import { vacuumUmAxis, type VacuumAxis } from "../spectrumExport";
import { airToVacuumUm } from "../spectralAxis";
import type { ProcessedFile } from "../../shared/types";
import type { SpectralAxisInfo, X1dSpectrum } from "../../shared/types/spectral";
import type { Spectrum1d } from "../../shared/types/spectrum1d";

function x1dOf(wavelength: number[], flux: number[], extra: Partial<X1dSpectrum> = {}): X1dSpectrum {
  return {
    path: "C:/d/jw01_nrs1_x1d.fits",
    hdu: 1,
    extver: 1,
    n_rows: wavelength.length,
    wavelength_um: wavelength,
    wavelength_unit: "um",
    flux,
    flux_error: flux.map((v) => v / 10),
    flux_unit: "Jy",
    surf_bright: flux.map((v) => v * 2),
    surf_bright_unit: "MJy/sr",
    background: null,
    npixels: null,
    dq: wavelength.map(() => 0),
    dq_table: "jwst",
    dq_flagged_rows: 0,
    srctype: "POINT",
    grating: "G235H",
    filter: "F170LP",
    detector: "NRS1",
    instrument: "NIRSPEC",
    target: "T",
    other_tables: [],
    notes: [],
    ...extra,
  };
}

function fileOf(id: string, path: string, status: ProcessedFile["status"] = "done"): ProcessedFile {
  const sourcePath = path.split("#")[0];
  return {
    id,
    name: sourcePath.split("/").pop() ?? path,
    path,
    sourcePath,
    imageRef: path === sourcePath ? null : path,
    size: 1,
    status,
    result: null,
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function regionEntryOf(): ComparisonEntry {
  return {
    id: "r1",
    kind: "region",
    label: "R1",
    color: "#111111",
    source: { kind: "region", shape: { shape: "circle", x: 1, y: 1, r: 1 }, background: null },
    regionText: null,
    backgroundId: null,
    region: null,
    pixel: null,
    table: null,
    error: null,
  };
}

describe("interpolateOnto", () => {
  it("interpolates linearly inside the table range and returns the row value on an exact match", () => {
    const out = interpolateOnto([1.5, 2, 2.25], [1, 2, 3], [10, 20, 40]);
    expect(out[0]).toBeCloseTo(15, 12);
    expect(out[1]).toBe(20);
    expect(out[2]).toBeCloseTo(25, 12);
  });

  it("returns null outside the table range and for non-finite axis values", () => {
    expect(interpolateOnto([0.5, 3.5, NaN, 1, 3], [1, 2, 3], [10, 20, 30])).toEqual([null, null, null, 10, 30]);
    expect(interpolateOnto([1, 2], [], [])).toEqual([null, null]);
  });

  it("returns null when either neighbour is null or NaN", () => {
    const out = interpolateOnto([1.5, 2.5, 3.5, 3], [1, 2, 3, 4], [10, null, 30, NaN]);
    expect(out).toEqual([null, null, null, 30]);
  });

  it("accepts a descending target axis", () => {
    const out = interpolateOnto([3, 2.5, 1], [1, 2, 3], [10, 20, 30]);
    expect(out[0]).toBe(30);
    expect(out[1]).toBeCloseTo(25, 12);
    expect(out[2]).toBe(10);
  });

  it("works on unsorted rows once usableRows has sorted them by wavelength", () => {
    const x1d = x1dOf([3, 1, 2], [30, 10, 20]);
    const rows = usableRows(x1d);
    expect(rows.wavelength).toEqual([1, 2, 3]);
    expect(rows.flux).toEqual([10, 20, 30]);
    expect(rows.err).toEqual([1, 2, 3]);
    expect(rows.sb).toEqual([20, 40, 60]);
    expect(interpolateOnto([1.5, 2.5], rows.wavelength, rows.flux)).toEqual([15, 25]);
  });
});

describe("usableRows", () => {
  it("keeps DQ-flagged rows as empty nodes, drops rows without a finite wavelength and counts every DQ drop", () => {
    const x1d = x1dOf([1, 2, 3, 4, 5, 6], [10, 20, 30, 40, 50, 60], {
      dq: [0, 1, 0, 0, 4, 2],
      wavelength_um: [1, 2, null as unknown as number, 4, 5, null as unknown as number],
      surf_bright: null,
    });
    const rows = usableRows(x1d);
    expect(rows.wavelength).toEqual([1, 2, 4, 5]);
    expect(rows.flux).toEqual([10, NaN, 40, NaN]);
    expect(rows.err).toEqual([1, null, 4, null]);
    expect(rows.sb).toEqual([null, null, null, null]);
    expect(rows.flagged).toEqual([false, true, false, true]);
    expect(rows.droppedDq).toBe(3);
  });

  it("keeps every row when the table has no DQ column and turns null fluxes into NaN", () => {
    const x1d = x1dOf([1, 2], [null as unknown as number, 5], { dq: null, flux_error: [null as unknown as number, 1] });
    const rows = usableRows(x1d);
    expect(rows.wavelength).toEqual([1, 2]);
    expect(rows.flux[0]).toBeNaN();
    expect(rows.flux[1]).toBe(5);
    expect(rows.err).toEqual([null, 1]);
    expect(rows.flagged).toEqual([false, false]);
    expect(rows.droppedDq).toBe(0);
  });
});

describe("resampleTable", () => {
  it("never fills a channel at or next to a dq != 0 row and reports droppedDq, rowsUsed and the overlap in vacuum um", () => {
    const x1d = x1dOf([1, 2, 3, 4], [10, 999, 30, 40], { dq: [0, 1, 0, 0] });
    const resampled = resampleTable(x1d, [0.5, 1.5, 2, 3.5, 4.5]);
    expect(resampled.droppedDq).toBe(1);
    expect(resampled.rowsUsed).toBe(3);
    expect(resampled.overlap).toEqual([1, 4]);
    expect(resampled.flux[0]).toBeNaN();
    expect(resampled.flux[1]).toBeNaN();
    expect(resampled.flux[2]).toBeNaN();
    expect(resampled.flux[3]).toBeCloseTo(35, 12);
    expect(resampled.flux[4]).toBeNaN();
    expect(resampled.fluxErr[0]).toBeNull();
    expect(resampled.fluxErr[1]).toBeNull();
    expect(resampled.fluxErr[2]).toBeNull();
    expect(resampled.fluxErr[3]).toBeCloseTo(2.5, 12);
    expect(resampled.fluxErr[4]).toBeNull();
    expect(resampled.surfBright?.[2]).toBeNaN();
    expect(resampled.surfBright?.[3]).toBeCloseTo(70, 12);
    expect(resampled.flux).toHaveLength(5);
  });

  it("propagates FLUX_ERROR in quadrature between two rows and keeps a row's own error on an exact hit", () => {
    const x1d = x1dOf([1, 2, 3], [40, 40, 30]);
    const resampled = resampleTable(x1d, [1, 1.25, 1.5, 2, 2.5]);
    expect(resampled.fluxErr[0]).toBe(4);
    expect(resampled.fluxErr[1]).toBeCloseTo(4 * Math.sqrt(0.75 ** 2 + 0.25 ** 2), 12);
    expect(resampled.fluxErr[2]).toBeCloseTo(4 * Math.SQRT1_2, 12);
    expect(resampled.fluxErr[3]).toBe(4);
    expect(resampled.fluxErr[4]).toBeCloseTo(Math.hypot(0.5 * 4, 0.5 * 3), 12);
    expect(resampled.flux[1]).toBeCloseTo(40, 12);
    expect(resampled.flux[4]).toBeCloseTo(35, 12);
  });

  it("leaves every channel between the last good row before an interior DQ run and the first good row after it empty", () => {
    const x1d = x1dOf([1, 2, 3, 4, 5, 6], [10, 20, 0, 0, 50, 60], { dq: [0, 0, 1, 1, 0, 0] });
    const axis = [1, 1.5, 2, 2.0001, 2.5, 3, 3.5, 4, 4.5, 4.9999, 5, 5.5, 6];
    const resampled = resampleTable(x1d, axis);
    expect(resampled.droppedDq).toBe(2);
    expect(resampled.rowsUsed).toBe(4);
    axis.forEach((um, i) => {
      if (um > 2 && um < 5) expect(resampled.flux[i]).toBeNaN();
      else expect(Number.isFinite(resampled.flux[i])).toBe(true);
    });
    expect(resampled.flux[1]).toBeCloseTo(15, 12);
    expect(resampled.flux[2]).toBe(20);
    expect(resampled.flux[10]).toBe(50);
    expect(resampled.flux[11]).toBeCloseTo(55, 12);
  });

  it("treats an axis value within 1e-9 relative of a row as that row, so the first good channel after a gap keeps its value", () => {
    const x1d = x1dOf([2, 2.001, 2.002, 2.003], [20, 0, 22, 23], { dq: [0, 1, 0, 0] });
    const axis = [2 - 2e-15, 2.001, 2.002 - 2e-15, 2.003 + 2e-15, 2.002 - 1e-6];
    expect(axis[0]).toBeLessThan(2);
    expect(axis[2]).toBeLessThan(2.002);
    expect(axis[3]).toBeGreaterThan(2.003);
    const resampled = resampleTable(x1d, axis);
    expect(resampled.flux[0]).toBe(20);
    expect(resampled.flux[1]).toBeNaN();
    expect(resampled.flux[2]).toBe(22);
    expect(resampled.flux[3]).toBe(23);
    expect(resampled.flux[4]).toBeNaN();
    expect(resampled.fluxErr[2]).toBeCloseTo(2.2, 12);
  });

  it("keeps the overlap and rowsUsed on the unflagged rows when the table ends in a DQ run", () => {
    const x1d = x1dOf([1, 2, 3, 4, 5], [10, 20, 30, 0, 0], { dq: [0, 0, 0, 1, 1] });
    const resampled = resampleTable(x1d, [1, 2, 3, 4, 5, 6]);
    expect(resampled.rowsUsed).toBe(3);
    expect(resampled.droppedDq).toBe(2);
    expect(resampled.overlap).toEqual([1, 3]);
    expect(resampled.flux.slice(0, 3)).toEqual([10, 20, 30]);
    expect(resampled.flux.slice(3).every((v) => Number.isNaN(v))).toBe(true);
  });

  it("has no surface brightness without a SURF_BRIGHT column and no overlap for disjoint ranges", () => {
    const x1d = x1dOf([1, 2], [1, 2], { surf_bright: null });
    const resampled = resampleTable(x1d, [5, 6, 7]);
    expect(resampled.surfBright).toBeNull();
    expect(resampled.overlap).toBeNull();
    expect(resampled.flux.every((v) => Number.isNaN(v))).toBe(true);
    expect(resampled.fluxErr).toEqual([null, null, null]);
  });

  it("puts a 1.66-3.17 um x1d onto a 2.87-5.27 um cube axis only inside 2.87-3.17", () => {
    const rows = 151;
    const wavelength = Array.from({ length: rows }, (_, i) => 1.66 + (i * (3.17 - 1.66)) / (rows - 1));
    const x1d = x1dOf(wavelength, wavelength.map(() => 1));
    const channels = 241;
    const axis = Array.from({ length: channels }, (_, i) => 2.87 + (i * (5.27 - 2.87)) / (channels - 1));
    const resampled = resampleTable(x1d, axis);
    expect(resampled.flux).toHaveLength(channels);
    axis.forEach((um, i) => {
      const value = resampled.flux[i];
      if (um >= 2.87 && um <= 3.17 - 1e-9) expect(value).toBeCloseTo(1, 9);
      if (um > 3.17 + 1e-9) expect(value).toBeNaN();
    });
    expect(resampled.flux.filter((v) => Number.isFinite(v)).length).toBeGreaterThan(20);
    expect(resampled.overlap?.[0]).toBeCloseTo(2.87, 9);
    expect(resampled.overlap?.[1]).toBeCloseTo(3.17, 9);
  });
});

describe("labels and entries", () => {
  it("tableLabel names detector, grating and filter and omits missing parts", () => {
    expect(tableLabel(x1dOf([1], [1], { detector: "NRS1", grating: "G235H", filter: "F170LP" }))).toBe("x1d (nrs1, G235H/F170LP)");
    expect(tableLabel(x1dOf([1], [1], { detector: null }))).toBe("x1d (G235H/F170LP)");
    expect(tableLabel(x1dOf([1], [1], { filter: null }))).toBe("x1d (nrs1, G235H)");
    expect(tableLabel(x1dOf([1], [1], { grating: null, filter: null }))).toBe("x1d (nrs1)");
    expect(tableLabel(x1dOf([1], [1], { detector: null, grating: null, filter: null }))).toBe("x1d");
  });

  it("tableEntry builds a table entry resampled on the vacuum axis", () => {
    const x1d = x1dOf([1, 2, 3], [10, 20, 30], { dq: [0, 2, 0], hdu: 3, n_rows: 3 });
    const vacuum: VacuumAxis = { values: [1, 2, 3], restUm: null, restOrigin: null, reason: null };
    const entry = tableEntry(x1d, vacuum, 3);
    expect(entry.id).toBe(TABLE_ENTRY_ID);
    expect(entry.kind).toBe("table");
    expect(entry.label).toBe("x1d (nrs1, G235H/F170LP)");
    expect(entry.color).toBe(TABLE_SERIES_COLOR);
    expect(entry.source).toEqual({ kind: "table", path: "C:/d/jw01_nrs1_x1d.fits", hdu: 3 });
    expect(entry.region).toBeNull();
    expect(entry.pixel).toBeNull();
    expect(entry.error).toBeNull();
    expect(entry.table?.path).toBe("C:/d/jw01_nrs1_x1d.fits");
    expect(entry.table?.hdu).toBe(3);
    expect(entry.table?.nRows).toBe(3);
    expect(entry.table?.fluxUnit).toBe("Jy");
    expect(entry.table?.sbUnit).toBe("MJy/sr");
    expect(entry.table?.resampled.droppedDq).toBe(1);
    expect(entry.table?.resampled.flux).toEqual([10, NaN, 30]);
    expect(entry.table?.resampled.fluxErr).toEqual([1, null, 3]);
  });

  it("tableEntry carries the vacuum reason as its error when the cube has no vacuum axis", () => {
    const x1d = x1dOf([1, 2], [1, 2]);
    const vacuum: VacuumAxis = { values: null, restUm: null, restOrigin: null, reason: "velocity axis without a rest wavelength" };
    const entry = tableEntry(x1d, vacuum, 2);
    expect(entry.error).toBe("velocity axis without a rest wavelength");
    expect(entry.table).toBeNull();
    const short: VacuumAxis = { values: [1, 2], restUm: null, restOrigin: null, reason: null };
    expect(tableEntry(x1d, short, 3).error).toBe("vacuum axis has 2 channels, cube has 3");
  });

  it("withTable appends the table entry after the cube entries and returns them unchanged without a table", () => {
    const vacuum: VacuumAxis = { values: [1, 2], restUm: null, restOrigin: null, reason: null };
    const cube = [regionEntryOf()];
    expect(withTable(cube, null, vacuum, 2)).toBe(cube);
    const merged = withTable(cube, x1dOf([1, 2], [5, 6]), vacuum, 2);
    expect(merged.map((e) => e.id)).toEqual(["r1", TABLE_ENTRY_ID]);
    expect(merged[1].table?.resampled.flux).toEqual([5, 6]);
    expect(cube).toHaveLength(1);
  });

  it("entryFluxUnit reports the x1d units for a table entry and the cube units otherwise", () => {
    const x1d = x1dOf([1, 2], [1, 2]);
    const vacuum: VacuumAxis = { values: [1, 2], restUm: null, restOrigin: null, reason: null };
    const entry = tableEntry(x1d, vacuum, 2);
    expect(entryFluxUnit(entry, "MJy/sr", "mean")).toBe("MJy/sr");
    expect(entryFluxUnit(entry, "MJy/sr", "jy")).toBe("Jy");
    expect(entryFluxUnit(entry, "MJy/sr", "sum")).toBe("Jy");
    const noSb = tableEntry(x1dOf([1, 2], [1, 2], { surf_bright_unit: null }), vacuum, 2);
    expect(entryFluxUnit(noSb, null, "mean")).toBe("MJy/sr");
    expect(entryFluxUnit(regionEntryOf(), "MJy/sr", "sum")).toBe("MJy/sr x pix");
    expect(entryFluxUnit(regionEntryOf(), "MJy/sr", "jy")).toBe("Jy");
    expect(APERTURE_NOTE).toContain("pipeline aperture");
  });
});

describe("sibling paths", () => {
  it("x1dStem strips the folder and the _x1d suffix", () => {
    expect(x1dStem("C:/d/jw01266005001_02103_00001_nrs1_x1d.fits")).toBe("jw01266005001_02103_00001_nrs1");
    expect(x1dStem("C:\\d\\a_x1d.FTS")).toBe("a");
    expect(x1dStem("C:/d/a_s3d.fits")).toBeNull();
    expect(x1dStem("C:/d/spectrum.fits")).toBeNull();
  });

  it("x1dStem strips a gzip suffix after the FITS spelling", () => {
    expect(x1dStem("C:/d/jw01_nrs1_x1d.fits.gz")).toBe("jw01_nrs1");
    expect(x1dStem("C:\\d\\a_x1d.FTS.GZ#hdu=1")).toBe("a");
    expect(x1dStem("C:/d/a_x1d.fit.gz")).toBe("a");
    expect(x1dStem("C:/d/a_x1d.tar.gz")).toBeNull();
    expect(x1dStem("C:/d/a_x1d.gz")).toBeNull();
  });

  it("findSiblingCube pairs gzip and plain names in every combination", () => {
    const gzCube = fileOf("g", "C:/d/jw01_nrs1_s3d.fits.gz#hdu=1");
    const plainCube = fileOf("p", "C:/d/jw01_nrs1_s3d.fits#hdu=1");
    expect(findSiblingCube([gzCube], "C:/d/jw01_nrs1_x1d.fits.gz")).toBe(gzCube);
    expect(findSiblingCube([gzCube], "C:/d/jw01_nrs1_x1d.fits")).toBe(gzCube);
    expect(findSiblingCube([plainCube], "C:/d/jw01_nrs1_x1d.fits.gz")).toBe(plainCube);
    expect(findSiblingCube([fileOf("t", "C:/d/jw01_nrs1_s3d.tar.gz")], "C:/d/jw01_nrs1_x1d.fits.gz")).toBeNull();
  });

  it("comparableX1dIds offers the cube comparison for a gzip x1d row", () => {
    const files: ProcessedFile[] = [
      { ...fileOf("x", "C:/d/jw01_nrs1_x1d.fits.gz", "spectrum"), spectrum: { kind: "x1d", hdu: 1 } },
      fileOf("s", "C:/d/jw01_nrs1_s3d.fits.gz#hdu=1"),
    ];
    expect([...comparableX1dIds(files)]).toEqual(["x"]);
  });

  it("findSiblingCube matches the first loaded s3d, never the 2D cal, and ignores files not done", () => {
    const x1d = "C:/d/jw01_nrs1_x1d.fits";
    const s3d = fileOf("s", "C:/d/jw01_nrs1_s3d.fits#hdu=1");
    const s3dAgain = fileOf("t", "C:/e/JW01_NRS1_S3D.FITS#hdu=1");
    const cal = fileOf("c", "C:/d/jw01_nrs1_cal.fits");
    const other = fileOf("o", "C:/d/jw02_nrs1_s3d.fits#hdu=1");
    expect(findSiblingCube([other, cal, s3d], x1d)).toBe(s3d);
    expect(findSiblingCube([s3dAgain, s3d], x1d)).toBe(s3dAgain);
    expect(findSiblingCube([other, cal], x1d)).toBeNull();
    expect(findSiblingCube([other], x1d)).toBeNull();
    expect(findSiblingCube([fileOf("p", "C:/d/jw01_nrs1_s3d.fits", "processing")], x1d)).toBeNull();
    expect(findSiblingCube([s3d], "C:/d/spectrum.fits")).toBeNull();
  });

  it("comparableX1dIds lists the x1d spectrum rows whose s3d cube is loaded and skips every other row", () => {
    const x1dRow = (id: string, path: string): ProcessedFile => ({ ...fileOf(id, path, "spectrum"), spectrum: { kind: "x1d", hdu: 1 } });
    const files = [
      x1dRow("x", "C:/d/jw01_nrs1_x1d.fits"),
      x1dRow("y", "C:/d/jw02_nrs1_x1d.fits"),
      { ...fileOf("w", "C:/d/jw01_nrs1_x1d.fits", "spectrum"), spectrum: { kind: "wcs1d", hdu: 0 } as const },
      fileOf("n", "C:/d/jw01_nrs1_x1d.fits", "spectrum"),
      { ...fileOf("e", "C:/d/jw01_nrs1_x1d.fits", "error"), error: "Failed to load C:/d/jw01_nrs1_x1d.fits: HDU 1 (EXTRACT1D): BINTABLE extension, holds no image pixels" },
      x1dRow("z", "C:/d/spectrum.fits"),
      fileOf("c", "C:/d/jw02_nrs1_cal.fits"),
      fileOf("s", "C:/d/jw01_nrs1_s3d.fits#hdu=1"),
    ];
    expect([...comparableX1dIds(files)]).toEqual(["x"]);
    expect(comparableX1dIds([fileOf("s", "C:/d/jw01_nrs1_s3d.fits#hdu=1")]).size).toBe(0);
  });

  it("comparableX1dIds resolves many x1d rows against many loaded files", () => {
    const files: ProcessedFile[] = [];
    for (let i = 0; i < 300; i++) files.push({ ...fileOf(`x${i}`, `C:/d/jw${i}_nrs1_x1d.fits`, "spectrum"), spectrum: { kind: "x1d", hdu: 1 } });
    for (let i = 0; i < 700; i++) files.push(fileOf(`r${i}`, `C:/d/jw${i}_nrs1_rate.fits`));
    for (let i = 0; i < 300; i += 2) files.push(fileOf(`s${i}`, `C:/d/jw${i}_nrs1_s3d.fits#hdu=1`));
    files.push(fileOf("c1", "C:/d/jw1_nrs1_cal.fits"));
    const ids = comparableX1dIds(files);
    expect(ids.size).toBe(150);
    for (let i = 0; i < 300; i++) expect(ids.has(`x${i}`)).toBe(i % 2 === 0);
  });
});

function spectrumOf(axis: SpectralAxisInfo, flux: number[], extra: Partial<Spectrum1d> = {}): Spectrum1d {
  return {
    source: { kind: "x1d", path: "C:/d/jw01_g395h_x1d.fits", hdu: 1, extver: 1 },
    axis,
    wavelength_frame: axis.kind === "awav" ? "air" : "vacuum",
    precision: "f64",
    flux,
    err: flux.map(() => 0.5),
    dq: null,
    dq_table: "unknown",
    flux_unit: "Jy",
    n_rows: flux.length,
    cards: {},
    notes: [],
    ...extra,
  };
}

function axisOf(kind: SpectralAxisInfo["kind"], values: number[], extra: Partial<SpectralAxisInfo> = {}): SpectralAxisInfo {
  return {
    kind,
    ctype: "WAVELENGTH",
    unit: "um",
    header_unit: "um",
    header_scale: 1,
    values,
    crval: values[0] ?? 0,
    cdelt: values.length > 1 ? values[1] - values[0] : 0,
    crpix: 1,
    rest_wavelength_um: null,
    rest_frequency_hz: null,
    specsys: null,
    velosys: null,
    notes: [],
    ...extra,
  };
}

describe("documentAsTable", () => {
  it("turns a CSV spectrum into a table on vacuum wavelengths with its errors, DQ, unit and optics", () => {
    const doc = spectrumOf(axisOf("wave", [2.0, 2.1, 2.2]), [1, 2, 3], {
      source: { kind: "csv", path: "C:/d/g235h_spectrum.csv" },
      err: [0.1, 0.2, 0.3],
      dq: [0, 1, 0],
      dq_table: "jwst",
      cards: { GRATING: "G235H", FILTER: "F170LP", DETECTOR: "NRS1", INSTRUME: "NIRSPEC", TARGNAME: "T", SRCTYPE: "POINT" },
      notes: ["CSV file, column wavelength_vacuum_um: vacuum wavelengths"],
    });
    expect(documentAsTable(doc)).toEqual({
      path: "C:/d/g235h_spectrum.csv",
      hdu: 0,
      extver: null,
      n_rows: 3,
      wavelength_um: [2.0, 2.1, 2.2],
      wavelength_unit: "um",
      flux: [1, 2, 3],
      flux_error: [0.1, 0.2, 0.3],
      flux_unit: "Jy",
      surf_bright: null,
      surf_bright_unit: null,
      background: null,
      npixels: null,
      dq: [0, 1, 0],
      dq_table: "jwst",
      dq_flagged_rows: 1,
      srctype: "POINT",
      grating: "G235H",
      filter: "F170LP",
      detector: "NRS1",
      instrument: "NIRSPEC",
      target: "T",
      other_tables: [],
      notes: ["CSV file, column wavelength_vacuum_um: vacuum wavelengths"],
    });
  });

  it("keeps the HDU of a FITS spectrum and names a missing flux unit as native", () => {
    const x1d = documentAsTable(spectrumOf(axisOf("wave", [2.0, 2.1]), [1, 2], { source: { kind: "x1d", path: "C:/d/a_x1d.fits", hdu: 2, extver: 2 } }));
    expect(x1d.hdu).toBe(2);
    expect(x1d.extver).toBe(2);
    const wcs1d = documentAsTable(spectrumOf(axisOf("wave", [2.0, 2.1]), [1, 2], { source: { kind: "wcs1d", path: "C:/d/b.fits", hdu: 0 }, flux_unit: null }));
    expect(wcs1d.hdu).toBe(0);
    expect(wcs1d.flux_unit).toBe("native");
  });

  it("converts an air axis to vacuum wavelengths so it resamples onto a vacuum axis", () => {
    const air = [0.6562, 0.6563, 0.6564];
    const table = documentAsTable(spectrumOf(axisOf("awav", air), [1, 2, 3]));
    expect(table.wavelength_um).toEqual(air.map(airToVacuumUm));
  });

  it("keeps every row of a spectrum whose DQ table is unknown, as the line measurement does", () => {
    const axis = [2.0, 2.1, 2.2];
    const doc = spectrumOf(axisOf("wave", axis), [1, 2, 3], {
      source: { kind: "csv", path: "C:/d/no_table_spectrum.csv" },
      dq: [0, 4, 0],
      dq_table: "unknown",
    });
    const table = documentAsTable(doc);
    expect(table.dq).toBeNull();
    expect(table.dq_flagged_rows).toBe(0);
    const resampled = resampleTable(table, axis);
    expect(resampled.flux).toEqual([1, 2, 3]);
    expect(resampled.droppedDq).toBe(0);
    const known = resampleTable(documentAsTable({ ...doc, dq_table: "jwst", dq: [0, 1, 0] }), axis);
    expect(known.flux[1]).toBeNaN();
    expect(known.droppedDq).toBe(1);
  });

  it("refuses a spectrum whose axis has no vacuum wavelength, naming the file", () => {
    const velocity = spectrumOf(axisOf("vopt", [0, 10, 20], { unit: "km/s" }), [1, 2, 3], { source: { kind: "csv", path: "C:/d/v.csv" } });
    expect(() => documentAsTable(velocity)).toThrow(/v\.csv: .*rest wavelength/);
  });
});

describe("documentComparisonEntries", () => {
  const AXIS = Array.from({ length: 10 }, (_, i) => 2.0 + i * 0.001);

  function documentOnAxis(): Spectrum1d {
    return spectrumOf(axisOf("wave", AXIS), AXIS.map((_, i) => 10 + i));
  }

  it("puts the spectrum first and resamples a second x1d onto the spectrum's own vacuum axis", () => {
    const second = x1dOf([1.9995, 2.0025, 2.0055, 2.0085, 2.0115], [1, 4, 7, 10, 13]);
    const doc = documentOnAxis();
    const vacuum = vacuumUmAxis(doc.axis, null, "optical", AXIS.length);
    const entries = documentComparisonEntries(doc, { kind: "file", hdu: 1 }, second, vacuum, AXIS.length);
    expect(entries.map((e) => e.id)).toEqual([DOCUMENT_ENTRY_ID, TABLE_ENTRY_ID]);
    expect(entries[0]).toMatchObject({
      kind: "document",
      label: "jw01_g395h_x1d.fits",
      source: { kind: "file", hdu: 1 },
      document: { flux: doc.flux, fluxUnit: "Jy" },
      error: null,
    });
    const table = entries[1];
    expect(table.kind).toBe("table");
    expect(table.label).toBe("jw01_nrs1_x1d.fits");
    expect(table.error).toBeNull();
    const resampled = table.table?.resampled.flux ?? [];
    expect(resampled).toHaveLength(AXIS.length);
    AXIS.forEach((w, i) => expect(resampled[i]).toBeCloseTo(1 + 1000 * (w - 1.9995), 9));
    expect(table.table?.resampled).toEqual(resampleTable(second, AXIS));
  });

  it("shows the spectrum alone before a second spectrum is picked", () => {
    const doc = documentOnAxis();
    const entries = documentComparisonEntries(doc, { kind: "file", hdu: 1 }, null, vacuumUmAxis(doc.axis, null, "optical", AXIS.length), AXIS.length);
    expect(entries.map((e) => e.kind)).toEqual(["document"]);
  });

  it("reports the document unit for its entry", () => {
    const doc = spectrumOf(axisOf("wave", AXIS), AXIS.map(() => 1), { flux_unit: "erg/s/cm2/A" });
    const [entry] = documentComparisonEntries(doc, { kind: "file", hdu: 0 }, null, vacuumUmAxis(doc.axis, null, "optical", AXIS.length), AXIS.length);
    expect(entryFluxUnit(entry, "MJy/sr", "sum")).toBe("erg/s/cm2/A");
  });
});

describe("readComparisonTable", () => {
  const x1d = x1dOf([2.0, 2.1], [1, 2]);

  it("reads the pipeline x1d of a cube with the x1d reader", async () => {
    const readers = { x1d: vi.fn(async () => x1d), document: vi.fn() };
    await expect(readComparisonTable("C:/d/jw_x1d.fits", 2, false, readers)).resolves.toBe(x1d);
    expect(readers.x1d).toHaveBeenCalledWith("C:/d/jw_x1d.fits", 2);
    expect(readers.document).not.toHaveBeenCalled();
  });

  it("reads any x1d, 1D FITS or CSV spectrum next to a spectrum file with the document reader and converts it", async () => {
    const doc = spectrumOf(axisOf("wave", [2.0, 2.1]), [5, 6], { source: { kind: "csv", path: "C:/d/s.csv" } });
    const readers = { x1d: vi.fn(), document: vi.fn(async () => doc) };
    const table = await readComparisonTable("C:/d/s.csv", null, true, readers);
    expect(readers.document).toHaveBeenCalledWith("C:/d/s.csv", null);
    expect(readers.x1d).not.toHaveBeenCalled();
    expect(table).toEqual(documentAsTable(doc));
  });
});
