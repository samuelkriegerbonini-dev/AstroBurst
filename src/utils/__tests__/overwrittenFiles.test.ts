import { describe, it, expect } from "vitest";
import { overwrittenFileIds, writtenFitsPaths } from "../overwrittenFiles";

describe("writtenFitsPaths", () => {
  it("collects the FITS a single run wrote", () => {
    expect(writtenFitsPaths({ png_path: "/o/M42_arcsinh.png", fits_path: "/o/M42_arcsinh.fits" })).toEqual(["/o/M42_arcsinh.fits"]);
    expect(writtenFitsPaths({ r_path: "/o/f_R.fits", g_path: "/o/f_G.fits", b_path: "/o/f_B.fits" })).toEqual([
      "/o/f_R.fits",
      "/o/f_G.fits",
      "/o/f_B.fits",
    ]);
  });

  it("collects the FITS of every item of a batch but never the batch inputs", () => {
    const batch = {
      results: [
        { path: "/raw/a.fits", fits_path: "/o/a_cosmetic.fits" },
        { path: "/raw/b.fits", error: "unreadable" },
      ],
    };
    expect(writtenFitsPaths(batch)).toEqual(["/o/a_cosmetic.fits"]);
  });

  it("finds nothing in a result that wrote no FITS", () => {
    expect(writtenFitsPaths({ png_path: "/o/a.png" })).toEqual([]);
    expect(writtenFitsPaths(null)).toEqual([]);
  });
});

describe("overwrittenFileIds", () => {
  const files = [
    { id: "m42", path: "C:/raw/M42.fits" },
    { id: "out", path: "C:\\Users\\me\\output\\M42_arcsinh.fits" },
    { id: "plane", path: "C:/o/cube.fits#hdu=1" },
  ];

  it("matches a loaded file that a step rewrote, whatever the separator or case", () => {
    expect(overwrittenFileIds(files, ["c:/users/me/output/M42_arcsinh.fits"])).toEqual(["out"]);
  });

  it("matches a loaded plane when its whole file was rewritten", () => {
    expect(overwrittenFileIds(files, ["C:/o/cube.fits"])).toEqual(["plane"]);
  });

  it("leaves every other loaded file alone", () => {
    expect(overwrittenFileIds(files, ["C:/o/M42_deconv.fits"])).toEqual([]);
    expect(overwrittenFileIds(files, [])).toEqual([]);
  });
});
