import { describe, it, expect, beforeEach } from "vitest";
import { fileStore } from "../useFileStore";

const X1D_REJECTION =
  "Failed to load C:/d/jw_x1d.fits: HDU 1 (EXTRACT1D): BINTABLE extension, holds no image pixels; " +
  "no HDU in this file holds a 2D image. " +
  "HDUs: [0] EXTNAME=(none) XTENSION=PRIMARY NAXIS=0 -- header only, no pixel data; " +
  "[1] EXTNAME=EXTRACT1D XTENSION=BINTABLE NAXIS=2 [232x1024] -- BINTABLE extension, holds no image pixels";

describe("fileStore table-only files", () => {
  beforeEach(() => {
    fileStore.reset();
  });

  it("a table-only x1d is listed as a table, not counted as a failure, and still completes the batch", () => {
    fileStore.addFiles([
      { name: "a_x1d.fits", path: "C:/d/a_x1d.fits", size: 1 },
      { name: "b_s3d.fits", path: "C:/d/b_s3d.fits", size: 1 },
    ]);
    const [x1d, cube] = fileStore.getFiles();
    fileStore.fileTable(x1d.id, X1D_REJECTION);
    fileStore.fileDone(cube.id, null);
    expect(fileStore.getStats()).toMatchObject({ total: 2, done: 1, failed: 0, tables: 1 });
    expect(fileStore.getFile(x1d.id)).toMatchObject({ status: "table", error: X1D_REJECTION });
    expect(fileStore.getIsComplete()).toBe(true);
    expect(fileStore.getProgress()).toBe(100);
    expect(fileStore.getDoneFiles().map((f) => f.id)).toEqual([cube.id]);
  });

  it("an x1d-only batch completes with failed 0 and tables N", () => {
    fileStore.addFiles([
      { name: "a_x1d.fits", path: "C:/d/a_x1d.fits", size: 1 },
      { name: "b_x1d.fits", path: "C:/d/b_x1d.fits", size: 1 },
      { name: "c_x1d.fits", path: "C:/d/c_x1d.fits", size: 1 },
    ]);
    for (const f of fileStore.getFiles()) fileStore.fileTable(f.id, X1D_REJECTION);
    expect(fileStore.getStats()).toMatchObject({ total: 3, done: 0, failed: 0, tables: 3 });
    expect(fileStore.getIsComplete()).toBe(true);
    expect(fileStore.getProgress()).toBe(100);
    expect(fileStore.getSelected()).toBeNull();
  });

  it("reset and addFiles start the tables counter at zero and keep real errors in failed", () => {
    expect(fileStore.getStats()).toEqual({ total: 0, done: 0, failed: 0, tables: 0, totalBytes: 0 });
    fileStore.addFiles([{ name: "bad.fits", path: "C:/d/bad.fits", size: 1 }]);
    const [bad] = fileStore.getFiles();
    fileStore.fileError(bad.id, "No such file");
    expect(fileStore.getStats()).toMatchObject({ total: 1, done: 0, failed: 1, tables: 0 });
    expect(fileStore.getIsComplete()).toBe(true);
  });
});
