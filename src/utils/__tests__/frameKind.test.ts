import { describe, expect, it } from "vitest";
import {
  classifyImagetyp,
  frameKindOf,
  imagetypLeftOutLine,
  isCalibrationKind,
  isStackedProduct,
  sortByFrameKind,
  stackedLeftOutLine,
  subframeLights,
  type FrameKind,
} from "../frameKind";

const RUST_CLASSIFY_IMAGETYP_TABLE: [string, FrameKind][] = [
  ["Bias", "bias"],
  ["ZERO", "bias"],
  ["DARK", "dark"],
  ["Dark Frame", "dark"],
  ["Flat Field", "flat"],
  ["Light Frame", "light"],
  ["object", "light"],
  ["ext", "unknown"],
];

function frame(name: string, header: Record<string, string> | null) {
  return { name, path: `C:/session/${name}`, result: header ? { header } : null };
}

function names(files: { name: string }[]): string[] {
  return files.map((f) => f.name);
}

describe("classifyImagetyp", () => {
  it("follows the table of classify_imagetyp in core/stacking/frame_cards.rs", () => {
    for (const [value, kind] of RUST_CLASSIFY_IMAGETYP_TABLE) {
      expect(classifyImagetyp(value), value).toBe(kind);
    }
    expect(classifyImagetyp(null)).toBe("unknown");
    expect(classifyImagetyp("Flat Dark")).toBe("flatdark");
  });

  it("checks bias first, then flat dark, flat, dark and light, as the Rust substring rules do", () => {
    expect(classifyImagetyp("Bias Dark")).toBe("bias");
    expect(classifyImagetyp("DARKFLAT")).toBe("flatdark");
    expect(classifyImagetyp("Master Flat")).toBe("flat");
    expect(classifyImagetyp("Master Dark")).toBe("dark");
    expect(classifyImagetyp("LIGHT")).toBe("light");
    expect(classifyImagetyp("science")).toBe("unknown");
  });

  it("lowercases ASCII letters only, as to_ascii_lowercase does", () => {
    expect(classifyImagetyp("DAR\u212A")).toBe("unknown");
  });
});

describe("frameKindOf", () => {
  it("reads IMAGETYP, then FRAMETYP, skipping blank cards and quotes as first_text does", () => {
    expect(frameKindOf({ IMAGETYP: "'Flat Dark'" })).toBe("flatdark");
    expect(frameKindOf({ FRAMETYP: "'Dark'", EXPTIME: "300" })).toBe("dark");
    expect(frameKindOf({ IMAGETYP: "'Flat Field'", FRAMETYP: "'Dark'" })).toBe("flat");
    expect(frameKindOf({ IMAGETYP: "''", FRAMETYP: "'Bias'" })).toBe("bias");
    expect(frameKindOf({ IMAGETYP: "'  '", FRAMETYP: "' Light '" })).toBe("light");
  });

  it("reads the unquoted values of a loaded file's header", () => {
    expect(frameKindOf({ IMAGETYP: "FLAT", FILTER: "R" })).toBe("flat");
    expect(frameKindOf({ IMAGETYP: "Light Frame" })).toBe("light");
  });

  it("is unknown without either card or without a header", () => {
    expect(frameKindOf({ FILTER: "R" })).toBe("unknown");
    expect(frameKindOf({ IMAGETYP: "EXT" })).toBe("unknown");
    expect(frameKindOf(null)).toBe("unknown");
    expect(frameKindOf(undefined)).toBe("unknown");
  });

  it("calls bias, dark, flat and flat dark calibration kinds", () => {
    expect((["bias", "dark", "flat", "flatdark"] as FrameKind[]).every(isCalibrationKind)).toBe(true);
    expect(isCalibrationKind("light")).toBe(false);
    expect(isCalibrationKind("unknown")).toBe(false);
  });
});

describe("sortByFrameKind", () => {
  it("keeps Light and Unknown frames as lights and sorts bias, darks, flats and flat darks out in order", () => {
    const sorted = sortByFrameKind([
      frame("light_1.fits", { IMAGETYP: "LIGHT", FILTER: "R" }),
      frame("flat_1.fits", { IMAGETYP: "FLAT", FILTER: "R" }),
      frame("dark_1.fits", { IMAGETYP: "DARK", FILTER: "R" }),
      frame("wfpc2.fits", { IMAGETYP: "EXT", FILTNAM1: "F656N" }),
      frame("bias_1.fits", { IMAGETYP: "BIAS" }),
      frame("flatdark_1.fits", { IMAGETYP: "Flat Dark" }),
      frame("jwst.fits", null),
      frame("dark_2.fits", { FRAMETYP: "Dark" }),
    ]);
    expect(names(sorted.lights)).toEqual(["light_1.fits", "wfpc2.fits", "jwst.fits"]);
    expect(names(sorted.calibration.dark)).toEqual(["dark_1.fits", "dark_2.fits"]);
    expect(names(sorted.calibration.flat)).toEqual(["flat_1.fits"]);
    expect(names(sorted.calibration.flatdark)).toEqual(["flatdark_1.fits"]);
    expect(names(sorted.calibration.bias)).toEqual(["bias_1.fits"]);
  });
});

describe("imagetypLeftOutLine", () => {
  const four = ["a", "b", "c", "d"];

  it("names the total and the count of every kind left out", () => {
    expect(imagetypLeftOutLine({ dark: four, flat: four, flatdark: [], bias: [] })).toBe(
      "8 frames left out by IMAGETYP: 4 darks, 4 flats",
    );
    expect(imagetypLeftOutLine({ dark: ["a", "b"], flat: [], flatdark: four, bias: ["e", "f", "g"] })).toBe(
      "9 frames left out by IMAGETYP: 2 darks, 4 flat darks, 3 bias",
    );
  });

  it("uses the singular for one frame", () => {
    expect(imagetypLeftOutLine({ dark: [], flat: [], flatdark: ["a"], bias: [] })).toBe(
      "1 frame left out by IMAGETYP: 1 flat dark",
    );
    expect(imagetypLeftOutLine({ dark: ["a"], flat: ["b"], flatdark: [], bias: ["c"] })).toBe(
      "3 frames left out by IMAGETYP: 1 dark, 1 flat, 1 bias",
    );
  });

  it("says nothing when no frame was left out", () => {
    expect(imagetypLeftOutLine({ dark: [], flat: [], flatdark: [], bias: [] })).toBeNull();
  });
});

describe("isStackedProduct", () => {
  it("knows a stack, a drizzle, a pipeline master and a rejection map by the cards the app writes", () => {
    expect(isStackedProduct({ IMAGETYP: "LIGHT", ABCOMB: "mean", NCOMBINE: "3" })).toBe(true);
    expect(isStackedProduct({ ABCOMB: "'median  '" })).toBe(true);
    expect(isStackedProduct({ ABPROC: "stacked" })).toBe(true);
    expect(isStackedProduct({ ABPROC: "'drizzled'" })).toBe(true);
    expect(isStackedProduct({ IMAGETYP: "LIGHT", ABPROC: "rejection" })).toBe(true);
  });

  it("leaves a light, a calibrated light and other processing outputs alone", () => {
    expect(isStackedProduct({ IMAGETYP: "LIGHT", FILTER: "R" })).toBe(false);
    expect(isStackedProduct({ ABPROC: "calibrated" })).toBe(false);
    expect(isStackedProduct({ ABPROC: "arcsinh" })).toBe(false);
    expect(isStackedProduct({ ABCOMB: "  " })).toBe(false);
    expect(isStackedProduct(null)).toBe(false);
  });
});

describe("stackedLeftOutLine", () => {
  it("counts the stacked results a list leaves out", () => {
    expect(stackedLeftOutLine(0, "All")).toBeNull();
    expect(stackedLeftOutLine(1, "All")).toBe("1 stacked result left out of All");
    expect(stackedLeftOutLine(2, "the lanes")).toBe("2 stacked results left out of the lanes");
  });
});

describe("subframeLights", () => {
  it("offers the lights to the analysis and names the calibration frames and stacked results it leaves out", () => {
    const files = [
      frame("light_1.fits", { IMAGETYP: "LIGHT" }),
      frame("flat_1.fits", { IMAGETYP: "FLAT" }),
      frame("light_1_stack3.fits", { IMAGETYP: "LIGHT", ABCOMB: "mean", ABPROC: "stacked" }),
      frame("light_2.fits", { IMAGETYP: "LIGHT" }),
    ];
    expect(subframeLights(files)).toEqual({
      paths: ["C:/session/light_1.fits", "C:/session/light_2.fits"],
      leftOutLine: "1 frame left out by IMAGETYP: 1 flat; 1 stacked result left out of the analysis",
    });
    expect(subframeLights(files.filter((f) => f.name !== "flat_1.fits")).leftOutLine).toBe("1 stacked result left out of the analysis");
    expect(subframeLights([files[0], files[3]]).leftOutLine).toBeNull();
  });
});
