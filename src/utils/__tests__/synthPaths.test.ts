import { describe, it, expect } from "vitest";
import { synthOutputPaths, synthStackOutputPaths } from "../synthPaths";
import { isValidFitsFile } from "../validation";

describe("synthOutputPaths", () => {
  it("keeps a .fits name and derives the siblings from its stem", () => {
    expect(synthOutputPaths("C:\\Users\\u\\synthetic.fits")).toEqual({
      fits: "C:\\Users\\u\\synthetic.fits",
      catalog: "C:\\Users\\u\\synthetic_catalog.csv",
      groundTruth: "C:\\Users\\u\\synthetic_groundtruth.fits",
    });
  });

  it("adds .fits to an extension-less name and gives each output its own file", () => {
    expect(synthOutputPaths("/home/u/field1")).toEqual({
      fits: "/home/u/field1.fits",
      catalog: "/home/u/field1_catalog.csv",
      groundTruth: "/home/u/field1_groundtruth.fits",
    });
  });

  it("strips .fts, .fit and .FITS when naming the siblings", () => {
    expect(synthOutputPaths("/home/u/field1.fts")).toEqual({
      fits: "/home/u/field1.fts",
      catalog: "/home/u/field1_catalog.csv",
      groundTruth: "/home/u/field1_groundtruth.fits",
    });
    expect(synthOutputPaths("/home/u/a.fit").groundTruth).toBe("/home/u/a_groundtruth.fits");
    expect(synthOutputPaths("C:/d/synthetic.FITS").fits).toBe("C:/d/synthetic.FITS");
    expect(synthOutputPaths("C:/d/synthetic.FITS").catalog).toBe("C:/d/synthetic_catalog.csv");
  });

  it("treats a non-FITS suffix as part of the name instead of replacing it", () => {
    expect(synthOutputPaths("/home/u/field1.fits.gz")).toEqual({
      fits: "/home/u/field1.fits.gz.fits",
      catalog: "/home/u/field1.fits.gz_catalog.csv",
      groundTruth: "/home/u/field1.fits.gz_groundtruth.fits",
    });
  });

  it("never returns the same path twice and always names an image the app can open", () => {
    const chosen = [
      "/home/u/field1",
      "/home/u/field1.fts",
      "/home/u/field1.fits.gz",
      "/home/u/x_catalog.csv",
      "/home/u/x_groundtruth.fits",
      "/home/u/.fits",
      "C:\\d\\synthetic.fit",
    ];
    for (const path of chosen) {
      const out = synthOutputPaths(path);
      expect(new Set([out.fits, out.catalog, out.groundTruth]).size, path).toBe(3);
      expect(isValidFitsFile(out.fits), out.fits).toBe(true);
      expect(isValidFitsFile(out.groundTruth), out.groundTruth).toBe(true);
    }
  });
});

describe("synthStackOutputPaths", () => {
  it("names the stack catalog and ground truth after the prefix inside the folder", () => {
    expect(synthStackOutputPaths("C:\\Users\\u\\run", "synth")).toEqual({
      catalog: "C:\\Users\\u\\run\\synth_catalog.csv",
      groundTruth: "C:\\Users\\u\\run\\synth_groundtruth.fits",
    });
    expect(synthStackOutputPaths("/home/u/run", "f")).toEqual({
      catalog: "/home/u/run/f_catalog.csv",
      groundTruth: "/home/u/run/f_groundtruth.fits",
    });
  });

  it("does not double a trailing separator", () => {
    expect(synthStackOutputPaths("/home/u/run/", "synth").catalog).toBe("/home/u/run/synth_catalog.csv");
    expect(synthStackOutputPaths("C:\\run\\", "synth").groundTruth).toBe("C:\\run\\synth_groundtruth.fits");
  });
});
