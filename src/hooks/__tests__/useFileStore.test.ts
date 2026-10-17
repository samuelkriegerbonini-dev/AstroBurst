import { describe, it, expect, beforeEach } from "vitest";
import { fileStore } from "../useFileStore";
import { isSelectableStatus } from "../../utils/fileSelection";

const X1D_PROBE = { kind: "x1d", hdu: 1 } as const;

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
    expect(fileStore.getStats()).toEqual({ total: 0, done: 0, failed: 0, spectra: 0, totalBytes: 0 });
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
});
