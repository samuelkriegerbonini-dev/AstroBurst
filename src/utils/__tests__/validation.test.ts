import { describe, it, expect } from "vitest";
import { isValidFitsFile, SUPPORTED_EXTENSIONS, SUPPORTED_EXTENSIONS_LABEL } from "../validation";

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
