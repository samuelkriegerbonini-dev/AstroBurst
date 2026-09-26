import { describe, it, expect } from "vitest";
import {
  astroFileFromPath,
  folderErrorReport,
  folderReport,
  isValidFitsFile,
  partitionIncoming,
  rejectionReport,
  SUPPORTED_EXTENSIONS,
  SUPPORTED_EXTENSIONS_LABEL,
} from "../validation";

const CALIB = "C:\\crds\\jwst_nircam_distortion_0001.asdf";

describe("partitionIncoming", () => {
  it("splits paths into accepted images, unsupported files and calibration references", () => {
    const result = partitionIncoming(["C:\\a\\m42.fits", "C:\\a\\notes.txt", CALIB, "/b/cube.asdf", "/b/c.fits.gz"]);
    expect(result.accepted).toEqual(["C:\\a\\m42.fits", "/b/cube.asdf"]);
    expect(result.skipped).toEqual(["C:\\a\\notes.txt", "/b/c.fits.gz"]);
    expect(result.calib).toEqual([CALIB]);
  });

  it("recognises a calibration reference by its file name even inside a folder path", () => {
    expect(partitionIncoming(["/data/refs/jwst_miri_flat_0042.asdf"]).calib).toHaveLength(1);
  });

  it("keeps the original items when a path accessor is given", () => {
    const items = [{ name: "a.fits", size: 3 }, { name: "b.png", size: 4 }];
    const result = partitionIncoming(items, (item) => item.name);
    expect(result.accepted).toEqual([items[0]]);
    expect(result.skipped).toEqual([items[1]]);
  });

  it("returns three empty lists for no input", () => {
    expect(partitionIncoming([])).toEqual({ accepted: [], skipped: [], calib: [] });
  });
});

describe("astroFileFromPath", () => {
  it("names the file after the last path segment on both separators", () => {
    expect(astroFileFromPath("C:\\data\\m42.fits")).toEqual({ name: "m42.fits", path: "C:\\data\\m42.fits", size: 0 });
    expect(astroFileFromPath("/data/m42.fits").name).toBe("m42.fits");
  });
});

describe("rejectionReport", () => {
  it("is null when every item was accepted", () => {
    expect(rejectionReport(partitionIncoming(["a.fits"]))).toBeNull();
  });

  it("counts skipped files and calibration references", () => {
    expect(rejectionReport(partitionIncoming(["a.txt", "b.txt", CALIB, "c.fits"]))).toEqual({ skipped: 2, calib: 1, message: null });
  });
});

describe("folderReport", () => {
  it("is null when the folder gave images and nothing notable was skipped", () => {
    expect(folderReport("C:\\obs", partitionIncoming(["C:\\obs\\a.fits", "C:\\obs\\readme.txt"]))).toBeNull();
  });

  it("says that subfolders are not scanned when nothing loadable sits at the top level", () => {
    expect(folderReport("C:\\mastDownload", partitionIncoming(["C:\\mastDownload\\manifest.html"]))).toEqual({
      skipped: 0,
      calib: 0,
      message: "No FITS/ASDF files directly in C:\\mastDownload (subfolders are not scanned)",
    });
  });

  it("reports the calibration references of a folder that holds nothing else", () => {
    expect(folderReport("C:\\crds", partitionIncoming([CALIB]))).toEqual({
      skipped: 0,
      calib: 1,
      message: "No loadable FITS/ASDF files directly in C:\\crds (subfolders are not scanned)",
    });
  });

  it("still reports skipped calibration references next to accepted images", () => {
    expect(folderReport("C:\\mix", partitionIncoming(["C:\\mix\\a.fits", CALIB]))).toEqual({ skipped: 0, calib: 1, message: null });
  });
});

describe("folderErrorReport", () => {
  it("carries the folder and the reason", () => {
    expect(folderErrorReport("C:\\locked", new Error("access denied")).message).toBe("Could not read C:\\locked: access denied");
    expect(folderErrorReport("/x", "forbidden path").message).toBe("Could not read /x: forbidden path");
  });
});

describe("isValidFitsFile", () => {
  it("accepts fpack tile-compressed files", () => {
    expect(isValidFitsFile("image.fits.fz")).toBe(true);
    expect(isValidFitsFile("C:\\data\\IMAGE.FIT.FZ")).toBe(true);
    expect(isValidFitsFile("/data/frame.fz")).toBe(true);
  });

  it("keeps rejecting gzip-compressed FITS, which the reader cannot decode", () => {
    expect(isValidFitsFile("image.fits.gz")).toBe(false);
  });

  it("keeps the existing formats", () => {
    for (const name of ["a.fits", "b.fit", "c.fts", "d.asdf", "e.zip"]) expect(isValidFitsFile(name)).toBe(true);
    expect(isValidFitsFile("notes.txt")).toBe(false);
  });

  it("offers fz in the open dialog filter and the drop hint", () => {
    expect(SUPPORTED_EXTENSIONS).toContain("fz");
    expect(SUPPORTED_EXTENSIONS_LABEL).toContain(".fz");
  });
});
