import { describe, it, expect } from "vitest";
import {
  APERTURE_NOTE,
  TABLE_ENTRY_ID,
  TABLE_SERIES_COLOR,
  entryFluxUnit,
  findSiblingCube,
  interpolateOnto,
  openTableBlocker,
  resampleTable,
  siblingX1dPath,
  tableBlockers,
  tableEntry,
  tableLabel,
  usableRows,
  withTable,
  x1dStem,
} from "../x1dCompare";
import type { ComparisonEntry } from "../spectrumCompare";
import type { VacuumAxis } from "../spectrumExport";
import type { ProcessedFile } from "../../shared/types";
import type { X1dSpectrum } from "../../shared/types/spectral";

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
  it("siblingX1dPath maps an s3d or cal path to its x1d and keeps an x1d path", () => {
    expect(siblingX1dPath("C:/d/jw01_nrs1_s3d.fits#hdu=1")).toBe("C:/d/jw01_nrs1_x1d.fits");
    expect(siblingX1dPath("C:\\d\\jw01_nrs1_cal.fits")).toBe("C:\\d\\jw01_nrs1_x1d.fits");
    expect(siblingX1dPath("C:/d/jw01_nrs1_S3D.FIT")).toBe("C:/d/jw01_nrs1_x1d.FIT");
    expect(siblingX1dPath("C:/d/jw01_nrs1_x1d.fits")).toBe("C:/d/jw01_nrs1_x1d.fits");
    expect(siblingX1dPath("C:/d/jw01_nrs1_x1d.fits#hdu=1")).toBe("C:/d/jw01_nrs1_x1d.fits");
    expect(siblingX1dPath("C:/d/jw01_nrs1_rate.fits")).toBeNull();
    expect(siblingX1dPath("C:/d/cube.fits")).toBeNull();
  });

  it("x1dStem strips the folder and the _x1d suffix", () => {
    expect(x1dStem("C:/d/jw01266005001_02103_00001_nrs1_x1d.fits")).toBe("jw01266005001_02103_00001_nrs1");
    expect(x1dStem("C:\\d\\a_x1d.FTS")).toBe("a");
    expect(x1dStem("C:/d/a_s3d.fits")).toBeNull();
    expect(x1dStem("C:/d/spectrum.fits")).toBeNull();
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

  it("tableBlockers maps every x1d error row to its blocker and skips the other rows", () => {
    const x1dError =
      "Failed to load C:/d/jw01_nrs1_x1d.fits: HDU 1 (EXTRACT1D): BINTABLE extension, holds no image pixels; no HDU in this file holds a 2D image.";
    const files = [
      { ...fileOf("x", "C:/d/jw01_nrs1_x1d.fits", "error"), error: x1dError },
      { ...fileOf("y", "C:/d/jw02_nrs1_x1d.fits", "error"), error: x1dError.replace("jw01", "jw02") },
      { ...fileOf("e", "C:/d/broken.fits", "error"), error: "Failed to open C:/d/broken.fits: No such file" },
      fileOf("s", "C:/d/jw01_nrs1_s3d.fits#hdu=1"),
    ];
    const blockers = tableBlockers(files);
    expect([...blockers.keys()]).toEqual(["x", "y"]);
    expect(blockers.get("x")).toBeNull();
    expect(blockers.get("y")).toBe("load jw02_nrs1_s3d.fits first");
  });

  it("tableBlockers resolves many x1d rows against many loaded files", () => {
    const error = (i: number) =>
      `Failed to load C:/d/jw${i}_nrs1_x1d.fits: HDU 1 (EXTRACT1D): BINTABLE extension, holds no image pixels; no HDU in this file holds a 2D image.`;
    const files: ProcessedFile[] = [];
    for (let i = 0; i < 300; i++) files.push({ ...fileOf(`x${i}`, `C:/d/jw${i}_nrs1_x1d.fits`, "error"), error: error(i) });
    for (let i = 0; i < 700; i++) files.push(fileOf(`r${i}`, `C:/d/jw${i}_nrs1_rate.fits`));
    for (let i = 0; i < 300; i += 2) files.push(fileOf(`s${i}`, `C:/d/jw${i}_nrs1_s3d.fits#hdu=1`));
    files.push(fileOf("c1", "C:/d/jw1_nrs1_cal.fits"));
    const blockers = tableBlockers(files);
    expect(blockers.size).toBe(300);
    for (let i = 0; i < 300; i++) expect(blockers.get(`x${i}`)).toBe(i % 2 === 0 ? null : `load jw${i}_nrs1_s3d.fits first`);
  });

  it("openTableBlocker names the cube to load, ignores a loaded 2D cal and is null once the s3d is loaded", () => {
    const x1d = "C:/d/jw01266005001_02103_00001_nrs1_x1d.fits";
    expect(openTableBlocker([], x1d)).toBe("load jw01266005001_02103_00001_nrs1_s3d.fits first");
    expect(openTableBlocker([fileOf("c", "C:/d/jw01266005001_02103_00001_nrs1_cal.fits")], x1d)).toBe(
      "load jw01266005001_02103_00001_nrs1_s3d.fits first",
    );
    expect(openTableBlocker([fileOf("s", "C:/d/jw01266005001_02103_00001_nrs1_s3d.fits#hdu=1")], x1d)).toBeNull();
    expect(openTableBlocker([], "C:/d/spectrum.fits")).toBe(
      "spectrum.fits is not named <cube>_x1d.fits: use Pick x1d… in the Spectroscopy panel of its cube",
    );
  });
});
