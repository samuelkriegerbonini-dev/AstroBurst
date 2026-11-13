import { describe, it, expect, vi } from "vitest";
import { readFileSync } from "node:fs";
import {
  astroFileFromPath,
  childPath,
  dialogExtensions,
  expandDroppedPaths,
  FITS_FILE_EXTENSIONS,
  folderErrorReport,
  folderReport,
  ingestDrop,
  isIngestibleFile,
  isTextTableFile,
  isValidFitsFile,
  partitionIncoming,
  rejectionReport,
  SUPPORTED_EXTENSIONS,
  SUPPORTED_EXTENSIONS_LABEL,
} from "../validation";

const CALIB = "C:\\crds\\jwst_nircam_distortion_0001.asdf";

describe("partitionIncoming", () => {
  it("splits paths into accepted images, unsupported files and calibration references", () => {
    const result = partitionIncoming(["C:\\a\\m42.fits", "C:\\a\\notes.txt", CALIB, "/b/cube.asdf", "/b/c.fits.gz", "/b/d.tar.gz"]);
    expect(result.accepted).toEqual(["C:\\a\\m42.fits", "/b/cube.asdf", "/b/c.fits.gz"]);
    expect(result.skipped).toEqual(["C:\\a\\notes.txt", "/b/d.tar.gz"]);
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

  it("keeps the calibration rule and accepts CSV tables and bzip2 FITS", () => {
    const result = partitionIncoming(["C:\\a\\m42.fits", "C:\\a\\notes.txt", "/b/C1.csv", "/b/frame-r-001462-4-0165.fits.bz2"]);
    expect(result.accepted).toEqual(["C:\\a\\m42.fits", "/b/C1.csv", "/b/frame-r-001462-4-0165.fits.bz2"]);
    expect(result.skipped).toEqual(["C:\\a\\notes.txt"]);
    expect(partitionIncoming(["/b/C1.csv", CALIB]).calib).toEqual([CALIB]);
  });
});

describe("isIngestibleFile", () => {
  it("accepts CSV and ECSV tables and bzip2 FITS", () => {
    for (const name of [
      "C1_ls_dr10_box_A.csv",
      "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_whtlt.ecsv",
      "frame-r-001462-4-0165.fits.bz2",
      "X.FIT.BZ2",
      "a.fts.bz2",
    ]) expect(isIngestibleFile(name), name).toBe(true);
    expect(isValidFitsFile("frame-r-001462-4-0165.fits.bz2")).toBe(true);
    expect(isValidFitsFile("C1_ls_dr10_box_A.csv")).toBe(false);
    expect(isTextTableFile("C4_sdss_dr17_field_1462_4_165.csv")).toBe(true);
    expect(isTextTableFile("a.fits")).toBe(false);
  });

  it("rejects other compressed tables and a bare bzip2", () => {
    for (const name of [
      "table.csv.gz",
      "table.csv.bz2",
      "notes.bz2",
      "x.tar.bz2",
      "frame.fz.bz2",
      "notes.txt",
      "jw03375-o001_20261013t072715_tso3_00001_asn.json",
    ]) expect(isIngestibleFile(name), name).toBe(false);
  });

  it("offers the tables and the bzip2 spellings, never a bare bz2", () => {
    for (const ext of ["csv", "ecsv", "fits.bz2", "fit.bz2", "fts.bz2"]) expect(SUPPORTED_EXTENSIONS).toContain(ext);
    expect(SUPPORTED_EXTENSIONS).not.toContain("bz2");
    expect(SUPPORTED_EXTENSIONS_LABEL).toContain(".ecsv");
    expect(SUPPORTED_EXTENSIONS_LABEL).toContain(".fits.bz2");
    expect(SUPPORTED_EXTENSIONS).toEqual(["fits", "fit", "fts", "fz", "asdf", "zip", "fits.gz", "fit.gz", "fts.gz", "fits.bz2", "fit.bz2", "fts.bz2", "csv", "ecsv"]);
  });
});

describe.skipIf(!process.env.ASTROBURST_SCIENCE_DATA_DIR)("the science data set", () => {
  it("accepts every data file of the manifest and skips only the two association files", () => {
    const manifest = readFileSync(`${process.env.ASTROBURST_SCIENCE_DATA_DIR}/MANIFEST.tsv`, "utf8");
    const names = manifest
      .split(/\r?\n/)
      .slice(1)
      .filter((line) => line.length > 0)
      .map((line) => line.split("\t")[1]);
    const result = partitionIncoming(names);
    expect(result.skipped).toEqual(["jw03375-o001_20261013t072715_tso3_00001_asn.json", "jw02327-o001_20260925t123033_tso3_00001_asn.json"]);
    expect(result.accepted.length).toBe(names.length - 2);
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

describe("dropped folders", () => {
  const nightEntries = [
    { name: "a.fits", isDirectory: false },
    { name: "b.fits.gz", isDirectory: false },
    { name: "notes.txt", isDirectory: false },
    { name: "raw", isDirectory: true },
    { name: null, isDirectory: false },
  ];
  const nightReader = () =>
    vi.fn(async (dir: string) => {
      if (dir !== "C:\\night") throw new Error("not a directory");
      return nightEntries;
    });

  it("joins a folder and a name with the separator of the folder", () => {
    expect(childPath("C:\\night", "a.fits")).toBe("C:\\night\\a.fits");
    expect(childPath("/data/night", "a.fits")).toBe("/data/night/a.fits");
    expect(childPath("/data/night/", "a.fits")).toBe("/data/night/a.fits");
  });

  it("reads a dropped folder like Select Folder", async () => {
    const before = partitionIncoming(["C:\\night"]);
    expect(before.accepted).toEqual([]);
    expect(before.skipped).toEqual(["C:\\night"]);
    expect(await ingestDrop(["C:\\night"], nightReader())).toEqual({
      accepted: ["C:\\night\\a.fits", "C:\\night\\b.fits.gz"],
      report: null,
    });
  });

  it("keeps dropped files as they are and never lists their folder", async () => {
    const readDir = nightReader();
    expect(await expandDroppedPaths(["C:\\a\\m42.fits", "C:\\a\\C1.csv"], readDir)).toEqual({
      paths: ["C:\\a\\m42.fits", "C:\\a\\C1.csv"],
      emptyFolders: [],
    });
    expect(readDir).not.toHaveBeenCalled();
  });

  it("counts a dropped file the app does not open as skipped and says which folder held nothing", async () => {
    const readDir = vi.fn(async (dir: string) => {
      if (dir === "C:\\night") return nightEntries;
      if (dir === "C:\\empty") return [{ name: "readme.txt", isDirectory: false }];
      if (dir === "C:\\crds") return [{ name: "jwst_nircam_distortion_0001.asdf", isDirectory: false }];
      throw new Error("not a directory");
    });
    expect(await ingestDrop(["C:\\night", "C:\\a\\notes.txt", "C:\\empty"], readDir)).toEqual({
      accepted: ["C:\\night\\a.fits", "C:\\night\\b.fits.gz"],
      report: { skipped: 1, calib: 0, message: "No FITS/ASDF files directly in C:\\empty (subfolders are not scanned)" },
    });
    const emptyOnly = await ingestDrop(["C:\\empty"], readDir);
    expect(emptyOnly.accepted).toEqual([]);
    expect(emptyOnly.report?.message).toBe(folderReport("C:\\empty", { accepted: [], skipped: [], calib: [] })?.message);
    expect(await ingestDrop(["C:\\crds"], readDir)).toEqual({ accepted: [], report: { skipped: 0, calib: 1, message: null } });
    expect((await expandDroppedPaths(["C:\\crds"], readDir)).emptyFolders).toEqual([]);
  });
});

describe("isValidFitsFile", () => {
  it("accepts fpack tile-compressed files", () => {
    expect(isValidFitsFile("image.fits.fz")).toBe(true);
    expect(isValidFitsFile("C:\\data\\IMAGE.FIT.FZ")).toBe(true);
    expect(isValidFitsFile("/data/frame.fz")).toBe(true);
  });

  it("accepts gzip-compressed FITS in its three spellings, which the reader decompresses", () => {
    expect(isValidFitsFile("image.fits.gz")).toBe(true);
    expect(isValidFitsFile("C:\\data\\IMAGE.FIT.GZ")).toBe(true);
    expect(isValidFitsFile("/data/frame.fts.gz")).toBe(true);
  });

  it("rejects every other gzip file, so a tarball never reaches the reader", () => {
    for (const name of ["archive.tar.gz", "table.csv.gz", "notes.gz", "cube.asdf.gz", "frame.fz.gz"]) expect(isValidFitsFile(name), name).toBe(false);
  });

  it("keeps the existing formats", () => {
    for (const name of ["a.fits", "b.fit", "c.fts", "d.asdf", "e.zip"]) expect(isValidFitsFile(name)).toBe(true);
    expect(isValidFitsFile("notes.txt")).toBe(false);
  });

  it("offers fz in the open dialog filter and the drop hint", () => {
    expect(SUPPORTED_EXTENSIONS).toContain("fz");
    expect(SUPPORTED_EXTENSIONS_LABEL).toContain(".fz");
  });

  it("offers the gzip FITS spellings, never a bare gz, in the dialog filter and the drop hint", () => {
    for (const ext of ["fits.gz", "fit.gz", "fts.gz"]) expect(SUPPORTED_EXTENSIONS).toContain(ext);
    expect(SUPPORTED_EXTENSIONS).not.toContain("gz");
    expect(SUPPORTED_EXTENSIONS_LABEL).toContain(".fits.gz");
  });
});

describe("dialogExtensions", () => {
  it("keeps the multi-dot gzip patterns as they are on Windows and Linux", () => {
    expect(dialogExtensions(SUPPORTED_EXTENSIONS, false)).toEqual([...SUPPORTED_EXTENSIONS]);
  });

  it("adds gz on macOS, whose open panel matches only the last extension of fits.gz", () => {
    const mac = dialogExtensions(SUPPORTED_EXTENSIONS, true);
    expect(mac).toEqual([...SUPPORTED_EXTENSIONS, "gz", "bz2"]);
  });

  it("adds bz2 on macOS when a bzip2 spelling is offered", () => {
    expect(dialogExtensions(["fits", "fits.bz2"], true)).toEqual(["fits", "fits.bz2", "bz2"]);
  });

  it("adds nothing on macOS when no gzip spelling is offered", () => {
    expect(dialogExtensions(["fits", "fit", "fts"], true)).toEqual(["fits", "fit", "fts"]);
  });

  it("lists the FITS spellings with their gzip forms for spectrum pickers", () => {
    expect(FITS_FILE_EXTENSIONS).toEqual(["fits", "fit", "fts", "fits.gz", "fit.gz", "fts.gz"]);
    for (const ext of FITS_FILE_EXTENSIONS) expect(isValidFitsFile(`a.${ext}`), ext).toBe(true);
  });
});
