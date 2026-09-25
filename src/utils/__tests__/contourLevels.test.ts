import { describe, it, expect } from "vitest";
import {
  boundsOffScreen,
  buildContourRequest,
  closedContourPolygons,
  contourHint,
  contourToPolygon,
  cullPolyline,
  polylineBounds,
  resolveBin,
  CONTOUR_BIN_CHOICES,
  CONTOUR_HINT_ON_SCREEN,
  CONTOUR_HINT_OTHER_SOURCE,
  DEFAULT_SIGMA_MULTIPLES_TEXT,
  formatLevelValue,
  levelColour,
  levelsText,
  parseLevelList,
  parseSigmaMultiples,
  suggestBin,
  SINGLE_CONTOUR_COLOUR,
  type ContourForm,
} from "../contourLevels";
import { describeMeasurementSource } from "../analysisTarget";
import type { Pt } from "../regionGeometry";
import type { ContourLevel } from "../../shared/types/contours";

const identity = (p: Pt): Pt => p;
const scaled = (p: Pt): Pt => ({ x: p.x * 2 + 100, y: p.y * 2 + 50 });

describe("parseLevelList and parseSigmaMultiples", () => {
  it("accepts comma and space separated finite numbers", () => {
    expect(parseLevelList("1, 2.5 3")).toEqual([1, 2.5, 3]);
    expect(parseLevelList(" 10;20\n30 ")).toEqual([10, 20, 30]);
    expect(parseSigmaMultiples("1, 2, 3, 5, 10")).toEqual([1, 2, 3, 5, 10]);
    expect(parseLevelList("-1e3 4e-2")).toEqual([-1000, 0.04]);
  });

  it("rejects text with a non-numeric token, an infinity or nothing at all", () => {
    expect(parseLevelList("a")).toBeNull();
    expect(parseLevelList("1, b")).toBeNull();
    expect(parseLevelList("Infinity")).toBeNull();
    expect(parseLevelList("")).toBeNull();
    expect(parseSigmaMultiples("   ")).toBeNull();
  });

  it("starts the default sigma levels at 3 sigma so they sit outside the sky noise", () => {
    const multiples = parseSigmaMultiples(DEFAULT_SIGMA_MULTIPLES_TEXT);
    expect(multiples).toEqual([3, 5, 10]);
    expect(Math.min(...(multiples as number[]))).toBeGreaterThanOrEqual(3);
  });
});

describe("suggestBin", () => {
  it("bins large images down to about the target size and keeps small ones at 1", () => {
    expect(suggestBin(8192, 8192)).toBe(4);
    expect(suggestBin(1024, 1024)).toBe(1);
    expect(suggestBin(2048, 2048)).toBe(1);
    expect(suggestBin(2049, 100)).toBe(2);
    expect(suggestBin(100000, 100000)).toBe(8);
    expect(suggestBin(0, 0)).toBe(1);
    expect(suggestBin(Number.NaN, 512)).toBe(1);
  });
});

describe("levelColour", () => {
  it("returns the teal token in single mode and a blue to amber ramp otherwise", () => {
    expect(levelColour(0, 5, "single")).toBe(SINGLE_CONTOUR_COLOUR);
    expect(levelColour(4, 5, "single")).toBe(SINGLE_CONTOUR_COLOUR);
    expect(levelColour(0, 5, "ramp")).toBe("hsl(220 90% 60%)");
    expect(levelColour(4, 5, "ramp")).toBe("hsl(40 90% 60%)");
    expect(levelColour(0, 1, "ramp")).toBe("hsl(220 90% 60%)");
    expect(levelColour(2, 5, "ramp")).toBe("hsl(130 90% 60%)");
  });
});

describe("cullPolyline", () => {
  it("keeps polylines whose screen bbox touches the canvas and drops the rest", () => {
    const inside: [number, number][] = [
      [10, 10],
      [20, 30],
    ];
    expect(cullPolyline(inside, identity, 100, 100)).toBe(false);
    const rightOf: [number, number][] = [
      [150, 10],
      [160, 30],
    ];
    expect(cullPolyline(rightOf, identity, 100, 100)).toBe(true);
    const above: [number, number][] = [
      [10, -30],
      [20, -10],
    ];
    expect(cullPolyline(above, identity, 100, 100)).toBe(true);
    const spanning: [number, number][] = [
      [-50, -50],
      [500, 500],
    ];
    expect(cullPolyline(spanning, identity, 100, 100)).toBe(false);
    expect(cullPolyline([], identity, 100, 100)).toBe(true);
  });

  it("applies the transform before testing", () => {
    const line: [number, number][] = [
      [0, 0],
      [10, 10],
    ];
    expect(cullPolyline(line, scaled, 90, 100)).toBe(true);
    expect(cullPolyline(line, scaled, 100, 40)).toBe(true);
    expect(cullPolyline(line, scaled, 130, 60)).toBe(false);
  });

  it("computes the bounds once and tests them against the canvas on their own", () => {
    const line: [number, number][] = [
      [3, 8],
      [-2, 4],
      [7, -1],
    ];
    const bounds = polylineBounds(line);
    expect(bounds).toEqual([-2, -1, 7, 8]);
    expect(boundsOffScreen(bounds, identity, 100, 100)).toBe(false);
    expect(boundsOffScreen(bounds, scaled, 90, 100)).toBe(true);
    expect(boundsOffScreen(polylineBounds([]), identity, 100, 100)).toBe(true);
    expect(boundsOffScreen([Number.NaN, 0, 1, 1], identity, 100, 100)).toBe(true);
  });
});

describe("contourToPolygon", () => {
  it("decimates long contours to at most the vertex cap and copies short ones", () => {
    const ring: [number, number][] = Array.from({ length: 2000 }, (_, i) => [Math.cos(i), Math.sin(i)]);
    const polygon = contourToPolygon(ring);
    expect(polygon).not.toBeNull();
    expect(polygon!.length).toBeLessThanOrEqual(512);
    expect(polygon!.length).toBeGreaterThan(400);
    expect(polygon![0]).toEqual(ring[0]);
    const short: [number, number][] = [
      [0, 0],
      [1, 0],
      [1, 1],
    ];
    const copy = contourToPolygon(short);
    expect(copy).toEqual(short);
    expect(copy).not.toBe(short);
    expect(contourToPolygon(ring, 10)!.length).toBeLessThanOrEqual(10);
  });

  it("rejects contours with fewer than three vertices", () => {
    expect(
      contourToPolygon([
        [0, 0],
        [1, 1],
      ]),
    ).toBeNull();
    expect(contourToPolygon([])).toBeNull();
  });
});

describe("levelsText and formatLevelValue", () => {
  it("joins levels with commas and formats values compactly", () => {
    expect(levelsText([105, 110.5, 115])).toBe("105, 110.5, 115");
    expect(formatLevelValue(105)).toBe("105");
    expect(formatLevelValue(110.5)).toBe("110.5");
    expect(formatLevelValue(0.000012345)).toBe("1.234e-5");
    expect(formatLevelValue(1234567)).toBe("1.235e+6");
    expect(formatLevelValue(0)).toBe("0");
    expect(formatLevelValue(Number.NaN)).toBe("--");
  });

  it("keeps the integer zeros of levels between 1e4 and 1e5", () => {
    expect(formatLevelValue(10000)).toBe("10000");
    expect(formatLevelValue(20000)).toBe("20000");
    expect(formatLevelValue(12340)).toBe("12340");
    expect(formatLevelValue(-20000)).toBe("-20000");
    expect(formatLevelValue(9999.95)).toBe("10000");
    expect(formatLevelValue(1000)).toBe("1000");
    expect(formatLevelValue(100)).toBe("100");
    expect(formatLevelValue(0.0012)).toBe("0.0012");
    expect(formatLevelValue(110.5)).toBe("110.5");
  });
});

describe("contourHint", () => {
  const base = {
    measuresComposite: false,
    compositeOnScreen: false,
    measuresFilePlanes: false,
    fileName: "R.fits",
    processedLabel: null,
    previewOnly: false,
  };

  it("does not claim the image on screen when the selected mono file is traced under an RGB composite", () => {
    const hint = contourHint(describeMeasurementSource({ ...base, compositeOnScreen: true }));
    expect(hint).toBe(CONTOUR_HINT_OTHER_SOURCE);
    expect(hint).not.toContain("image on screen");
  });

  it("does not claim the image on screen when a PNG-only result such as Debayer is on screen", () => {
    const hint = contourHint(describeMeasurementSource({ ...base, processedLabel: "Debayer", previewOnly: true }));
    expect(hint).toBe(CONTOUR_HINT_OTHER_SOURCE);
    expect(hint).not.toContain("image on screen");
  });

  it("keeps the on-screen hint for the original file and for a processed FITS", () => {
    expect(contourHint(describeMeasurementSource(base))).toBe(CONTOUR_HINT_ON_SCREEN);
    expect(contourHint(describeMeasurementSource({ ...base, processedLabel: "Denoise" }))).toBe(CONTOUR_HINT_ON_SCREEN);
  });
});

describe("buildContourRequest", () => {
  const base: ContourForm = {
    mode: "sigma",
    levelsText: "",
    nLevelsText: "5",
    loText: "0",
    hiText: "100",
    sigmaText: "1, 2, 3, 5, 10",
    smoothText: "1",
    binChoice: "auto",
    imageWidth: 8192,
    imageHeight: 4096,
    excludeDq: true,
  };

  it("builds sigma, list and generated requests with the auto bin from the image size", () => {
    expect(buildContourRequest(base)).toEqual({
      ok: true,
      options: { mode: "sigma", smoothSigma: 1, bin: 4, excludeDq: true, sigmaMultiples: [1, 2, 3, 5, 10] },
    });
    expect(buildContourRequest({ ...base, mode: "list", levelsText: "10 20", binChoice: "2" })).toEqual({
      ok: true,
      options: { mode: "list", smoothSigma: 1, bin: 2, excludeDq: true, levels: [10, 20] },
    });
    expect(buildContourRequest({ ...base, mode: "log", loText: "1", hiText: "1000", nLevelsText: "4" })).toEqual({
      ok: true,
      options: { mode: "log", smoothSigma: 1, bin: 4, excludeDq: true, nLevels: 4, lo: 1, hi: 1000 },
    });
  });

  it("explains every rejected form in a sentence", () => {
    const errorOf = (form: ContourForm) => {
      const r = buildContourRequest(form);
      return r.ok ? null : r.error;
    };
    expect(errorOf({ ...base, smoothText: "x" })).toMatch(/Smoothing sigma/);
    expect(errorOf({ ...base, smoothText: "21" })).toMatch(/Smoothing sigma/);
    expect(errorOf({ ...base, mode: "list", levelsText: "" })).toMatch(/Levels must be/);
    expect(errorOf({ ...base, sigmaText: "a" })).toMatch(/Sigma multiples/);
    expect(errorOf({ ...base, mode: "linear", nLevelsText: "0" })).toMatch(/number of levels/);
    expect(errorOf({ ...base, mode: "linear", nLevelsText: "2.5" })).toMatch(/number of levels/);
    expect(errorOf({ ...base, mode: "linear", loText: "" })).toMatch(/Low and high/);
    expect(errorOf({ ...base, mode: "linear", loText: "5", hiText: "1" })).toMatch(/Low must be less/);
    expect(errorOf({ ...base, mode: "log", loText: "0", hiText: "1" })).toMatch(/greater than 0/);
    expect(errorOf({ ...base, mode: "sqrt", loText: "-1", hiText: "1" })).toMatch(/at least 0/);
  });

  it("resolves explicit bin choices and clamps them", () => {
    expect(resolveBin("8", 100, 100)).toBe(8);
    expect(resolveBin("1", 100000, 100)).toBe(1);
    expect(resolveBin("auto", 100000, 100)).toBe(8);
    expect(CONTOUR_BIN_CHOICES).toEqual(["auto", "1", "2", "4", "8"]);
  });
});

describe("closedContourPolygons", () => {
  const ring: [number, number][] = [
    [0, 0],
    [4, 0],
    [4, 4],
    [0, 4],
  ];
  const levels: ContourLevel[] = [
    { value: 1, polylines: [ring, [[0, 0], [1, 1]]], closed: [true, false], n_points: 6 },
    { value: 2, polylines: [ring, ring], closed: [true, true], n_points: 8 },
  ];

  it("keeps only closed polylines of visible levels and caps the count", () => {
    const all = closedContourPolygons(levels, new Set());
    expect(all.polygons.map((p) => p.value)).toEqual([2, 2, 1]);
    expect(all.skipped).toBe(0);
    const hidden = closedContourPolygons(levels, new Set([0]));
    expect(hidden.polygons.map((p) => p.value)).toEqual([2, 2]);
    const capped = closedContourPolygons(levels, new Set(), 2);
    expect(capped.polygons).toHaveLength(2);
    expect(capped.skipped).toBe(1);
    expect(capped.polygons[0].points).toEqual(ring);
    expect(capped.polygons[0].points).not.toBe(ring);
  });

  it("keeps the highest level and the largest rings first when the cap is reached", () => {
    const square = (x: number, y: number, s: number): [number, number][] => [
      [x, y],
      [x + s, y],
      [x + s, y + s],
      [x, y + s],
    ];
    const noise = Array.from({ length: 250 }, (_, i) => square(i * 2, 0, 1));
    const halo = square(1000, 1000, 20);
    const core = square(1005, 1005, 6);
    const ranked: ContourLevel[] = [
      { value: 1, polylines: [...noise, halo], closed: [...noise.map(() => true), true], n_points: 0 },
      { value: 5, polylines: [core], closed: [true], n_points: 0 },
    ];
    const pick = closedContourPolygons(ranked, new Set());
    expect(pick.polygons).toHaveLength(200);
    expect(pick.skipped).toBe(52);
    expect(pick.polygons[0].value).toBe(5);
    expect(pick.polygons[1].points[0]).toEqual([1000, 1000]);
    expect(pick.polygons.filter((p) => p.value === 1)).toHaveLength(199);
  });
});
