import { describe, it, expect, beforeEach, vi } from "vitest";
import { fileStore } from "../useFileStore";
import { isSelectableStatus } from "../../utils/fileSelection";
import type { TableProbe } from "../../shared/types/tables";

const X1D_PROBE = { kind: "x1d", hdu: 1 } as const;

const LIGHT_CURVE_PROBE: TableProbe = {
  format: "ecsv",
  n_rows: 240,
  columns: [
    { name: "MJD_UTC_NRS1", unit: "d", datatype: "float64", numeric: true },
    { name: "whitelight_flux_NRS1", unit: "MJy", datatype: "float64", numeric: true },
    { name: "whitelight_flux_err_NRS1", unit: "MJy", datatype: "float64", numeric: true },
  ],
  tables: null,
  meta: null,
  comments: [],
  guess: { kind: "light_curve", reason: "time column MJD_UTC_NRS1 and value column whitelight_flux_NRS1" },
  notes: [],
};

const CSV_SPECTRUM_PROBE: TableProbe = {
  format: "csv",
  n_rows: 431,
  columns: [
    { name: "wavelength_vacuum_um", unit: null, datatype: "float64", numeric: true },
    { name: "flux", unit: null, datatype: "float64", numeric: true },
  ],
  tables: null,
  meta: null,
  comments: [],
  guess: { kind: "spectrum", reason: "columns wavelength_vacuum_um and flux: a spectrum exported by AstroBurst" },
  notes: [],
};

function flushed(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

describe("fileStore table files", () => {
  beforeEach(() => {
    fileStore.reset();
  });

  it("a table is listed as a table, counted apart from failures and spectra, and completes the batch", () => {
    fileStore.addFiles([
      { name: "a_whtlt.ecsv", path: "C:/d/a_whtlt.ecsv", size: 1 },
      { name: "b_i2d.fits", path: "C:/d/b_i2d.fits", size: 1 },
    ]);
    const [table, image] = fileStore.getFiles();
    fileStore.fileTable(table.id, LIGHT_CURVE_PROBE);
    fileStore.fileDone(image.id, null);
    expect(fileStore.getStats()).toMatchObject({ total: 2, done: 1, failed: 0, spectra: 0, tables: 1 });
    expect(fileStore.getIsComplete()).toBe(true);
    expect(fileStore.getProgress()).toBe(100);
    expect(fileStore.getDoneFiles().map((f) => f.id)).toEqual([image.id]);
    expect(fileStore.getFile(table.id)).toMatchObject({ status: "table", error: null, table: LIGHT_CURVE_PROBE });
  });

  it("a CSV spectrum is a spectrum row without a probe kind", () => {
    fileStore.addFiles([{ name: "spec.csv", path: "C:/d/spec.csv", size: 1 }]);
    const [row] = fileStore.getFiles();
    fileStore.fileCsvSpectrum(row.id, CSV_SPECTRUM_PROBE);
    const settled = fileStore.getFile(row.id);
    expect(settled).toMatchObject({ status: "spectrum", error: null, table: CSV_SPECTRUM_PROBE });
    expect(settled?.spectrum).toBeUndefined();
    expect(fileStore.getStats()).toMatchObject({ total: 1, done: 0, failed: 0, spectra: 1, tables: 0 });
    expect(fileStore.getIsComplete()).toBe(true);
  });

  it("a table opened from the archive or a sample selects itself when it settles", () => {
    fileStore.addFiles([{ name: "a_whtlt.ecsv", path: "C:/d/a_whtlt.ecsv", size: 1 }]);
    const [row] = fileStore.getFiles();
    const spy = vi.fn();
    fileStore.selectWhenLoaded("C:/d/a_whtlt.ecsv", spy);
    fileStore.fileTable(row.id, LIGHT_CURVE_PROBE);
    expect(fileStore.getSelected()).toBe(row.id);
    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy).toHaveBeenCalledWith(fileStore.getFile(row.id));
    expect(fileStore.getSelectOnLoadPath()).toBeNull();
  });

  it("the end of a batch selects the first spectrum, else the first table, when nothing is selected", async () => {
    fileStore.addFiles([
      { name: "a.ecsv", path: "C:/d/a.ecsv", size: 1 },
      { name: "b_x1d.fits", path: "C:/d/b_x1d.fits", size: 1 },
      { name: "c.csv", path: "C:/d/c.csv", size: 1 },
    ]);
    const [a, b, c] = fileStore.getFiles();
    fileStore.setProcessing(true);
    fileStore.fileTable(a.id, LIGHT_CURVE_PROBE);
    fileStore.fileSpectrum(b.id, X1D_PROBE);
    fileStore.fileCsvSpectrum(c.id, CSV_SPECTRUM_PROBE);
    expect(fileStore.getSelected()).toBeNull();
    await flushed();
    const heard = vi.fn();
    const stop = fileStore.subscribeToSelected(heard);
    fileStore.setProcessing(false);
    expect(fileStore.getSelected()).toBe(b.id);
    await flushed();
    expect(heard).toHaveBeenCalled();
    stop();

    fileStore.reset();
    fileStore.addFiles([{ name: "a.ecsv", path: "C:/d/a.ecsv", size: 1 }]);
    const [lone] = fileStore.getFiles();
    fileStore.setProcessing(true);
    fileStore.fileTable(lone.id, LIGHT_CURVE_PROBE);
    expect(fileStore.getSelected()).toBeNull();
    fileStore.setProcessing(false);
    expect(fileStore.getSelected()).toBe(lone.id);
  });

  it("the end of a batch keeps the image the batch selected and leaves a batch of errors without a selection", () => {
    fileStore.addFiles([
      { name: "a.ecsv", path: "C:/d/a.ecsv", size: 1 },
      { name: "b_i2d.fits", path: "C:/d/b_i2d.fits", size: 1 },
      { name: "c_x1d.fits", path: "C:/d/c_x1d.fits", size: 1 },
    ]);
    const [table, image, spectrum] = fileStore.getFiles();
    fileStore.setProcessing(true);
    fileStore.fileDone(image.id, null);
    fileStore.fileTable(table.id, LIGHT_CURVE_PROBE);
    fileStore.fileSpectrum(spectrum.id, X1D_PROBE);
    fileStore.setProcessing(false);
    expect(fileStore.getSelected()).toBe(image.id);

    fileStore.reset();
    fileStore.addFiles([
      { name: "bad.fits", path: "C:/d/bad.fits", size: 1 },
      { name: "worse.csv", path: "C:/d/worse.csv", size: 1 },
    ]);
    const [bad, worse] = fileStore.getFiles();
    fileStore.setProcessing(true);
    fileStore.fileError(bad.id, "No such file");
    fileStore.fileError(worse.id, "the CSV header row is missing");
    fileStore.setProcessing(false);
    expect(fileStore.getSelected()).toBeNull();
  });
});

describe("fileStore spectrum files", () => {
  beforeEach(() => {
    fileStore.reset();
  });

  it("a spectrum x1d is listed as a spectrum with its probe, not counted as a failure, and still completes the batch", () => {
    fileStore.addFiles([
      { name: "a_x1d.fits", path: "C:/d/a_x1d.fits", size: 1 },
      { name: "b_s3d.fits", path: "C:/d/b_s3d.fits", size: 1 },
    ]);
    const [x1d, cube] = fileStore.getFiles();
    fileStore.fileSpectrum(x1d.id, X1D_PROBE);
    fileStore.fileDone(cube.id, null);
    expect(fileStore.getStats()).toMatchObject({ total: 2, done: 1, failed: 0, spectra: 1 });
    expect(fileStore.getFile(x1d.id)).toMatchObject({ status: "spectrum", error: null, spectrum: X1D_PROBE });
    expect(fileStore.getIsComplete()).toBe(true);
    expect(fileStore.getProgress()).toBe(100);
    expect(fileStore.getDoneFiles().map((f) => f.id)).toEqual([cube.id]);
  });

  it("an x1d-only batch completes with failed 0 and spectra N and selects nothing on its own", () => {
    fileStore.addFiles([
      { name: "a_x1d.fits", path: "C:/d/a_x1d.fits", size: 1 },
      { name: "b_x1d.fits", path: "C:/d/b_x1d.fits", size: 1 },
      { name: "c_x1d.fits", path: "C:/d/c_x1d.fits", size: 1 },
    ]);
    for (const f of fileStore.getFiles()) fileStore.fileSpectrum(f.id, X1D_PROBE);
    expect(fileStore.getStats()).toMatchObject({ total: 3, done: 0, failed: 0, spectra: 3 });
    expect(fileStore.getIsComplete()).toBe(true);
    expect(fileStore.getProgress()).toBe(100);
    expect(fileStore.getSelected()).toBeNull();
  });

  it("reset and addFiles start the spectra counter at zero and keep real errors in failed", () => {
    expect(fileStore.getStats()).toEqual({ total: 0, done: 0, failed: 0, spectra: 0, tables: 0, totalBytes: 0 });
    fileStore.addFiles([{ name: "bad.fits", path: "C:/d/bad.fits", size: 1 }]);
    const [bad] = fileStore.getFiles();
    fileStore.fileError(bad.id, "No such file");
    expect(fileStore.getStats()).toMatchObject({ total: 1, done: 0, failed: 1, spectra: 0 });
    expect(fileStore.getIsComplete()).toBe(true);
  });

  it("a spectrum row has a selectable status and is never requeued as a failure", () => {
    fileStore.addFiles([{ name: "a_x1d.fits", path: "C:/d/a_x1d.fits", size: 1 }]);
    const [x1d] = fileStore.getFiles();
    fileStore.fileSpectrum(x1d.id, X1D_PROBE);
    const settled = fileStore.getFile(x1d.id);
    expect(settled?.spectrum).toEqual(X1D_PROBE);
    expect(isSelectableStatus(settled?.status ?? "")).toBe(true);
    expect(fileStore.requeueFile(x1d.id)).toBe(false);
  });
});

describe("fileStore requeue", () => {
  beforeEach(() => {
    fileStore.reset();
  });

  it("requeueFile turns an errored row back into a queued one and takes it out of the failed count", () => {
    fileStore.addFiles([{ name: "pm_out.fits", path: "C:/out/pm_out.fits", size: 0 }]);
    const [row] = fileStore.getFiles();
    fileStore.fileStarted(row.id);
    fileStore.fileError(row.id, "No such file");
    expect(fileStore.getIsComplete()).toBe(true);
    const statsVersion = fileStore.getStatsVersion();

    expect(fileStore.requeueFile(row.id)).toBe(true);

    expect(fileStore.getFile(row.id)).toMatchObject({ status: "queued", error: null, startedAt: null, finishedAt: null });
    expect(fileStore.getStats()).toMatchObject({ total: 1, done: 0, failed: 0 });
    expect(fileStore.getStatsVersion()).toBeGreaterThan(statsVersion);
    expect(fileStore.getIsComplete()).toBe(false);
  });

  it("requeueFile leaves rows that did not fail alone", () => {
    fileStore.addFiles([
      { name: "a.fits", path: "C:/d/a.fits", size: 0 },
      { name: "b.fits", path: "C:/d/b.fits", size: 0 },
    ]);
    const [done, queued] = fileStore.getFiles();
    fileStore.fileDone(done.id, null);
    expect(fileStore.requeueFile(done.id)).toBe(false);
    expect(fileStore.requeueFile(queued.id)).toBe(false);
    expect(fileStore.requeueFile("missing")).toBe(false);
    expect(fileStore.getFile(done.id)?.status).toBe("done");
    expect(fileStore.getStats()).toMatchObject({ done: 1, failed: 0 });
  });

  it("addFiles does not queue a second row for a path that is already waiting in the queue", () => {
    fileStore.addFiles([{ name: "pm_out.fits", path: "C:\\out\\pm_out.fits", size: 0 }]);
    fileStore.addFiles([
      { name: "pm_out.fits", path: "c:/OUT/pm_out.fits", size: 0 },
      { name: "other.fits", path: "C:/out/other.fits", size: 0 },
    ]);
    expect(fileStore.getFiles().map((f) => f.path)).toEqual(["C:\\out\\pm_out.fits", "C:/out/other.fits"]);
    expect(fileStore.getStats().total).toBe(2);

    const [first] = fileStore.getFiles();
    fileStore.fileError(first.id, "No such file");
    fileStore.addFiles([{ name: "pm_out.fits", path: "C:/out/pm_out.fits", size: 0 }]);
    expect(fileStore.getFiles()).toHaveLength(3);
  });

  it("addFiles does not add a second row for a path already loaded, whatever the file holds, and names the rows it kept", () => {
    fileStore.addFiles([
      { name: "a_whtlt.ecsv", path: "C:\\d\\a_whtlt.ecsv", size: 1 },
      { name: "b_i2d.fits", path: "C:\\d\\b_i2d.fits", size: 1 },
      { name: "c_x1d.fits", path: "C:\\d\\c_x1d.fits", size: 1 },
      { name: "d.fits", path: "C:\\d\\d.fits", size: 1 },
    ]);
    const [table, image, spectrum, broken] = fileStore.getFiles();
    fileStore.fileTable(table.id, LIGHT_CURVE_PROBE);
    fileStore.fileDone(image.id, null);
    fileStore.fileSpectrum(spectrum.id, X1D_PROBE);
    fileStore.fileError(broken.id, "No such file");
    fileStore.switchImageRef(image.id, "C:\\d\\b_i2d.fits#2", null as never);

    const outcome = fileStore.addFiles([
      { name: "a_whtlt.ecsv", path: "c:/D/a_whtlt.ecsv", size: 1 },
      { name: "b_i2d.fits", path: "C:/d/b_i2d.fits", size: 1 },
      { name: "c_x1d.fits", path: "C:/d/c_x1d.fits", size: 1 },
      { name: "d.fits", path: "C:/d/d.fits", size: 1 },
      { name: "e.fits", path: "C:/d/e.fits", size: 1 },
      { name: "e.fits", path: "C:\\d\\e.fits", size: 1 },
    ]);

    expect(fileStore.getFiles().map((f) => f.sourcePath)).toEqual([
      "C:\\d\\a_whtlt.ecsv",
      "C:\\d\\b_i2d.fits",
      "C:\\d\\c_x1d.fits",
      "C:\\d\\d.fits",
      "C:/d/d.fits",
      "C:/d/e.fits",
    ]);
    expect(fileStore.getStats().total).toBe(6);
    expect(outcome.added).toBe(2);
    expect(outcome.loaded.map((f) => f.id)).toEqual([table.id, image.id, spectrum.id]);
    expect(fileStore.addFiles([{ name: "a_whtlt.ecsv", path: "C:/d/a_whtlt.ecsv", size: 1 }])).toEqual({ added: 0, loaded: [fileStore.getFile(table.id)] });
  });
});

describe("fileStore reload of a rewritten file", () => {
  const PATH = "C:/data/mast/JWST/2739/xc1_i2d.fits";

  beforeEach(() => {
    fileStore.reset();
  });

  it("fileReloaded counts each reload on the row and keeps its id, path and selection", () => {
    fileStore.addFiles([{ name: "xc1_i2d.fits", path: PATH, size: 0 }]);
    const [row] = fileStore.getFiles();
    fileStore.fileDone(row.id, { previewUrl: "v1.png", dimensions: [256, 256] } as never);
    expect(fileStore.getFile(row.id)?.reloads ?? 0).toBe(0);

    fileStore.fileReloaded(row.id, { previewUrl: "v2.png?v=1", dimensions: [256, 256] } as never);
    expect(fileStore.getFile(row.id)).toMatchObject({ id: row.id, path: PATH, reloads: 1 });
    fileStore.fileReloaded(row.id, { previewUrl: "v2.png?v=2", dimensions: [256, 256] } as never);
    expect(fileStore.getFile(row.id)?.reloads).toBe(2);
    expect(fileStore.getSelected()).toBe(row.id);
  });

  it("other updates of the row do not count as a reload", () => {
    fileStore.addFiles([{ name: "xc1_i2d.fits", path: PATH, size: 0 }]);
    const [row] = fileStore.getFiles();
    fileStore.fileStarted(row.id);
    fileStore.fileDone(row.id, { previewUrl: "v1.png", dimensions: [256, 256] } as never);
    fileStore.fileResampled(row.id, { fits_path: "C:/out/xc1_resampled.fits" } as never);
    expect(fileStore.getFile(row.id)?.reloads ?? 0).toBe(0);
  });
});
