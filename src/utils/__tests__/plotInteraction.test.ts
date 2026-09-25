import { describe, it, expect } from "vitest";
import {
  clampDomain,
  errorBarExtent,
  logDomain,
  logTicks,
  nearestHit,
  panDomain,
  seriesToCsv,
  seriesYExtent,
  splitZeroPointPoints,
  zoomDomain,
  type Domain,
} from "../plotInteraction";
import { CSV_LINE_END } from "../catalogCsv";
import type { ProfileSeries } from "../../components/regions/ProfilePlot";

const EXTENT: Domain = [0, 100];
const identity = (v: number) => v;

describe("zoomDomain", () => {
  it("keeps the anchor value at the same fractional position when zooming in", () => {
    const before: Domain = [20, 60];
    const anchor = 30;
    const after = zoomDomain(before, EXTENT, anchor, 0.5);
    const tBefore = (anchor - before[0]) / (before[1] - before[0]);
    const tAfter = (anchor - after[0]) / (after[1] - after[0]);
    expect(after[1] - after[0]).toBeCloseTo(20, 9);
    expect(tAfter).toBeCloseTo(tBefore, 9);
  });

  it("never leaves the extent when zooming out near an edge", () => {
    const after = zoomDomain([0, 10], EXTENT, 1, 1.5);
    expect(after[0]).toBeGreaterThanOrEqual(EXTENT[0]);
    expect(after[1]).toBeLessThanOrEqual(EXTENT[1]);
    expect(after[1] - after[0]).toBeCloseTo(15, 9);
  });

  it("returns exactly the extent after repeated zoom-out", () => {
    let d: Domain = [40, 50];
    for (let i = 0; i < 50; i++) d = zoomDomain(d, EXTENT, 45, 1.15);
    expect(d).toEqual(EXTENT);
  });

  it("does not shrink below one part in a billion of the extent", () => {
    let d: Domain = [40, 50];
    for (let i = 0; i < 400; i++) d = zoomDomain(d, EXTENT, 45, 0.5);
    expect((d[1] - d[0]) / (100 * 1e-9)).toBeCloseTo(1, 6);
    expect(d[0]).toBeLessThanOrEqual(45);
    expect(d[1]).toBeGreaterThanOrEqual(45);
  });

  it("falls back to the extent when the domain or extent is degenerate", () => {
    expect(zoomDomain([5, 5], EXTENT, 5, 0.5)).toEqual(EXTENT);
    expect(zoomDomain([0, 10], [3, 3], 5, 0.5)).toEqual([3, 3]);
    expect(zoomDomain([0, 10], EXTENT, NaN, 0.5)[1] - zoomDomain([0, 10], EXTENT, NaN, 0.5)[0]).toBeCloseTo(5, 9);
  });
});

describe("panDomain", () => {
  it("shifts the domain by the delta inside the extent", () => {
    expect(panDomain([10, 20], EXTENT, 5)).toEqual([15, 25]);
  });

  it("clamps at the low end", () => {
    expect(panDomain([10, 20], EXTENT, -50)).toEqual([0, 10]);
  });

  it("clamps at the high end", () => {
    expect(panDomain([80, 90], EXTENT, 50)).toEqual([90, 100]);
  });

  it("ignores a non-finite delta", () => {
    expect(panDomain([10, 20], EXTENT, NaN)).toEqual([10, 20]);
  });
});

describe("clampDomain", () => {
  it("returns the extent when the domain is wider than it", () => {
    expect(clampDomain([-10, 200], EXTENT)).toEqual(EXTENT);
  });

  it("orders a reversed domain and keeps its width", () => {
    expect(clampDomain([30, 10], EXTENT)).toEqual([10, 30]);
  });
});

describe("logDomain", () => {
  it("returns the positive part of an extent that crosses zero", () => {
    const d = logDomain([-1, 100]);
    expect(d).not.toBeNull();
    expect(d![0]).toBeGreaterThan(0);
    expect(d![1]).toBe(100);
  });

  it("returns null when nothing is positive", () => {
    expect(logDomain([-5, -1])).toBeNull();
    expect(logDomain([0, 0])).toBeNull();
  });

  it("keeps a fully positive extent unchanged", () => {
    expect(logDomain([0.5, 20])).toEqual([0.5, 20]);
  });
});

describe("logTicks", () => {
  it("yields only decade ticks across a wide range", () => {
    expect(logTicks(1e-3, 5e3)).toEqual([1e-3, 1e-2, 1e-1, 1, 10, 100, 1000]);
  });

  it("adds 2 and 5 minors when fewer than three decades are spanned", () => {
    expect(logTicks(1, 50)).toEqual([1, 2, 5, 10, 20, 50]);
  });

  it("returns nothing for a non-positive range", () => {
    expect(logTicks(0, 10)).toEqual([]);
    expect(logTicks(-3, -1)).toEqual([]);
  });
});

describe("errorBarExtent", () => {
  it("widens the extent by the bars", () => {
    expect(errorBarExtent([1, 5, 3], [0.5, 2, 0.1])).toEqual([0.5, 7]);
  });

  it("ignores null bars and null values", () => {
    expect(errorBarExtent([1, null, 3], [null, 10, null])).toEqual([1, 3]);
  });

  it("works without bars and returns null without finite values", () => {
    expect(errorBarExtent([2, 4])).toEqual([2, 4]);
    expect(errorBarExtent([null, NaN])).toBeNull();
    expect(errorBarExtent([])).toBeNull();
  });
});

describe("seriesYExtent", () => {
  it("drops non-positive values in log mode and restricts to the visible x range", () => {
    const series = [{ x: [0, 1, 2, 3], y: [-1, 0.5, 10, 1000], yErr: [null, 0.2, null, null] }];
    expect(seriesYExtent(series, true, null)).toEqual([0.3, 1000]);
    expect(seriesYExtent(series, false, [0, 2])).toEqual([-1, 10]);
    expect(seriesYExtent(series, true, [5, 6])).toBeNull();
  });
});

describe("nearestHit", () => {
  const lineSeries = [
    { x: [0, 10, 20, 30], y: [1, 2, null, 4] },
    { x: [0, 10, 20, 30], y: [5, 6, 7, 8] },
  ];

  it("prefers the closest x in line mode and breaks ties by y", () => {
    const hit = nearestHit(lineSeries, identity, identity, 11, 5.8, 12, [false, false]);
    expect(hit).toEqual({ seriesIndex: 1, index: 1, x: 10, y: 6 });
  });

  it("ignores null y values", () => {
    const hit = nearestHit([lineSeries[0]], identity, identity, 20.2, 0, 12, [false]);
    expect(hit).toEqual({ seriesIndex: 0, index: 3, x: 30, y: 4 });
  });

  it("uses euclidean distance in points mode", () => {
    const pts = [{ x: [0, 10], y: [0, 100] }];
    const hit = nearestHit(pts, identity, identity, 9, 5, 100, [true]);
    expect(hit).toEqual({ seriesIndex: 0, index: 0, x: 0, y: 0 });
  });

  it("returns null beyond maxDistPx", () => {
    expect(nearestHit(lineSeries, identity, identity, 500, 0, 12, [false, false])).toBeNull();
    expect(nearestHit([{ x: [0], y: [0] }], identity, identity, 9, 9, 12, [true])).toBeNull();
  });

  it("skips points the scale cannot place", () => {
    const logSy = (v: number) => (v > 0 ? Math.log10(v) : NaN);
    const hit = nearestHit([{ x: [0, 1], y: [-1, 10] }], identity, logSy, 0.2, 0, 12, [false]);
    expect(hit).toEqual({ seriesIndex: 0, index: 1, x: 1, y: 10 });
  });
});

describe("seriesToCsv", () => {
  const shared: ProfileSeries[] = [
    { x: [1, 2, 3], y: [10, null, 30], color: "#fff", label: "mean, sky", yErr: [0.5, null, 1.5] },
    { x: [1, 2, 3], y: [11, 21, 31], color: "#fff", label: "median" },
  ];

  it("emits one row per x with blank nulls, quoted labels and CRLF line ends", () => {
    const csv = seriesToCsv(shared, "radius (px)");
    const lines = csv.split(CSV_LINE_END);
    expect(lines).toEqual(['radius (px),"mean, sky",median,"mean, sky_err"', "1,10,11,0.5", "2,,21,", "3,30,31,1.5", ""]);
  });

  it("appends error columns only for series with yErr", () => {
    const csv = seriesToCsv([shared[1]], "x");
    expect(csv.split(CSV_LINE_END)[0]).toBe("x,median");
  });

  it("writes one row per point per series when the series do not share x", () => {
    const a: ProfileSeries = { x: [1, 2], y: [5, 6], color: "#fff", label: "a" };
    const b: ProfileSeries = { x: [9], y: [7], color: "#fff", label: "b" };
    const lines = seriesToCsv([a, b], "x").split(CSV_LINE_END);
    expect(lines).toEqual(["x,a,b", "1,5,", "2,6,", "9,,7", ""]);
  });

  it("produces only the header for an empty series list", () => {
    expect(seriesToCsv([], "x")).toBe(`x${CSV_LINE_END}`);
  });
});

describe("splitZeroPointPoints", () => {
  it("separates points beyond three rms from the fitted ones and spans the line over all x", () => {
    const split = splitZeroPointPoints([10, 12, 14, null, 16], [-10, -8, -5, -4, -4.05], 20, 0.1);
    expect(split.fitted).toEqual({ x: [10, 12, 16], y: [-10, -8, -4.05] });
    expect(split.outliers).toEqual({ x: [14], y: [-5] });
    expect(split.line).toEqual({ x: [10, 16], y: [-10, -4] });
  });

  it("keeps every point as fitted and draws no line without a zero point", () => {
    const split = splitZeroPointPoints([10, 12], [-10, -5], null, null);
    expect(split.fitted).toEqual({ x: [10, 12], y: [-10, -5] });
    expect(split.outliers).toEqual({ x: [], y: [] });
    expect(split.line).toBeNull();
  });

  it("does not flag outliers when the rms is zero and returns no line without points", () => {
    expect(splitZeroPointPoints([10], [-10.5], 20, 0).outliers.x).toEqual([]);
    expect(splitZeroPointPoints([null], [null], 20, 0.1).line).toBeNull();
  });
});
