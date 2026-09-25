import { describe, it, expect } from "vitest";
import {
  clampDomain,
  emptyPlotMessage,
  errorBarExtent,
  errorBarSpan,
  logDomain,
  logHiddenCount,
  logTicks,
  nearestHit,
  panDomain,
  savePngWith,
  seriesToCsv,
  seriesYExtent,
  splitZeroPointPoints,
  zoomDomain,
  zoomScopeKey,
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

  it("returns exactly the extent when zooming out from the full extent at any anchor", () => {
    const ext: Domain = [0.001, 0.0035];
    for (let i = 0; i <= 1000; i++) {
      const a = ext[0] + ((ext[1] - ext[0]) * i) / 1000;
      expect(zoomDomain(ext, ext, a, 1.15)).toEqual(ext);
    }
  });
});

describe("zoomScopeKey", () => {
  const jd = [0.50017, 0.50087, 0.50156, 0.50226, 0.50295, 0.50365, 0.50434, 0.50503];

  it("changes when the x quantity changes from JD offset to frame index", () => {
    expect(zoomScopeKey([{ x: jd }], "JD - 2460310")).not.toBe(zoomScopeKey([{ x: jd.map((_, k) => k) }], "frame index"));
  });
  it("changes when the SB axis switches from arcsec to px", () => {
    const px = Array.from({ length: 30 }, (_, i) => i + 0.5);
    expect(zoomScopeKey([{ x: px.map((v) => v * 0.05) }], "semi-major axis (arcsec)")).not.toBe(
      zoomScopeKey([{ x: px }], "semi-major axis (px)"),
    );
  });

  it("changes when a new region brings a different radial extent under the same label", () => {
    const bins = (n: number) => [{ x: Array.from({ length: n }, (_, i) => i + 0.5), y: Array(n).fill(1) }];
    expect(zoomScopeKey(bins(50), "radius (px)")).not.toBe(zoomScopeKey(bins(15), "radius (px)"));
  });

  it("stays the same when only y changes on the same frames", () => {
    const diff: ProfileSeries[] = [{ x: jd, y: jd.map(() => 0.01), color: "#fff", label: "diff mag" }];
    const raw: ProfileSeries[] = [{ x: jd, y: jd.map(() => -10.5), color: "#fff", label: "raw comps" }];
    expect(zoomScopeKey(diff, "JD - 2460310")).toBe(zoomScopeKey(raw, "JD - 2460310"));
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

  it("gives a point within reach priority over a line vertex at the same x", () => {
    const series = [
      { x: [12, 14], y: [124.96, 90] },
      { x: [12, 17.5], y: [124.57, 40] },
    ];
    const hit = nearestHit(series, identity, identity, 13, 126.96, 12, [true, false]);
    expect(hit).toEqual({ seriesIndex: 0, index: 0, x: 12, y: 124.96 });
  });

  it("does not let a later line series displace a point hit", () => {
    const series = [{ x: [0], y: [0] }, { x: [0], y: [50] }, { x: [30], y: [0] }];
    expect(nearestHit(series, identity, identity, 1, 2, 12, [true, false, true])).toEqual({ seriesIndex: 0, index: 0, x: 0, y: 0 });
  });

  it("falls back to the line hit when no point is within reach", () => {
    const series = [{ x: [100], y: [100] }, { x: [0, 50], y: [0, 50] }];
    expect(nearestHit(series, identity, identity, 3, 40, 12, [true, false])).toEqual({ seriesIndex: 1, index: 0, x: 0, y: 0 });
  });

  it("skips points outside the zoomed x domain", () => {
    const zoom: Domain = [10, 20];
    const sx = (v: number) => 52 + ((v - zoom[0]) / (zoom[1] - zoom[0])) * 238;
    const bins = [{ x: [9.5, 10.5, 11.5], y: [900, 30, 20] }];
    expect(nearestHit(bins, sx, identity, 45, 60, 12, [false], zoom)).toBeNull();
    expect(nearestHit(bins, sx, identity, 60, 60, 12, [false], zoom)).toEqual({ seriesIndex: 0, index: 1, x: 10.5, y: 30 });
  });

  it("does not select a hidden frame from inside the plot edge in points mode", () => {
    const zoom: Domain = [4.2, 10];
    const sx = (v: number) => 52 + ((v - zoom[0]) / (zoom[1] - zoom[0])) * 238;
    const frames = [{ x: [3, 4, 5], y: [12, 12, 12] }];
    expect(nearestHit(frames, sx, identity, 53, 12, 12, [true], zoom)).toBeNull();
    expect(nearestHit(frames, sx, identity, 53, 12, 12, [true])).toEqual({ seriesIndex: 0, index: 1, x: 4, y: 12 });
  });
});

describe("errorBarSpan", () => {
  it("keeps the upper half of a log-axis bar whose lower end is not positive and marks the lower end as clipped", () => {
    const span = errorBarSpan(0.4, 0.7, true);
    expect(span).not.toBeNull();
    expect(span?.lower).toBeNull();
    expect(span?.upper).toBeCloseTo(1.1, 12);
    expect(errorBarSpan(0.5, -0.5, true)).toEqual({ lower: null, upper: 1 });
  });

  it("keeps both ends of a log-axis bar that stays positive", () => {
    expect(errorBarSpan(50, 2, true)).toEqual({ lower: 48, upper: 52 });
  });

  it("draws no bar for a non-positive value on a log axis", () => {
    expect(errorBarSpan(-0.1, 0.7, true)).toBeNull();
    expect(errorBarSpan(0, 0.7, true)).toBeNull();
  });

  it("keeps a negative lower end on a linear axis and uses the absolute error", () => {
    const span = errorBarSpan(0.4, -0.7, false);
    expect(span?.lower).toBeCloseTo(-0.3, 12);
    expect(span?.upper).toBeCloseTo(1.1, 12);
  });

  it("draws no bar for a non-finite value or error", () => {
    expect(errorBarSpan(NaN, 1, false)).toBeNull();
    expect(errorBarSpan(1, Infinity, false)).toBeNull();
  });
});

describe("log y empty state and hidden count", () => {
  it("reports no positive values instead of no data for an all-negative magnitude series", () => {
    const s = [{ x: [0, 1, 2], y: [-10.1, -10.0, -10.2] }];
    expect(emptyPlotMessage(s, true)).toBe("no positive values for log y");
    expect(emptyPlotMessage(s, false)).toBe("no data");
    expect(emptyPlotMessage([{ x: [], y: [] }], true)).toBe("no data");
  });

  it("counts finite values at or below zero hidden by a log axis", () => {
    const s = [{ x: [0, 1, 2, 3, 4], y: [-0.01, 0.02, 0, null, 0.005] }, { x: [0], y: [NaN] }];
    expect(logHiddenCount(s)).toEqual({ hidden: 2, total: 4 });
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

  it("applies the fitted colour term so stars on the colour relation are fitted and lie on the line", () => {
    const cat = [12, 13, 14, 15];
    const bpRp = [0.5, 1, 2, 2.5];
    const inst = cat.map((g, i) => g - 25 - 0.3 * bpRp[i]);
    const split = splitZeroPointPoints(cat, inst, 25, 0.02, bpRp, 0.3);
    expect(split.outliers.x).toEqual([]);
    expect(split.noColour.x).toEqual([]);
    expect(split.fitted.x).toEqual(cat);
    split.fitted.y.forEach((y, i) => expect(y).toBeCloseTo(cat[i] - 25, 12));
    expect(split.line).toEqual({ x: [12, 15], y: [-13, -10] });
  });

  it("sets rows without BP-RP aside, uncorrected, when a colour term was fitted", () => {
    const split = splitZeroPointPoints([12, 13, 14], [-13.3, -12, -11.6], 25, 0.02, [1, null, NaN], 0.3);
    expect(split.fitted.x).toEqual([12]);
    expect(split.fitted.y[0]).toBeCloseTo(-13, 12);
    expect(split.outliers.x).toEqual([]);
    expect(split.noColour).toEqual({ x: [13, 14], y: [-12, -11.6] });
    expect(split.line).toEqual({ x: [12, 12], y: [-13, -13] });
  });

  it("still flags a real outlier after the colour correction", () => {
    const split = splitZeroPointPoints([14, 15], [14 - 25 - 0.3 + 0.5, 15 - 25 - 0.3], 25, 0.02, [1, 1], 0.3);
    expect(split.outliers.x).toEqual([14]);
    expect(split.fitted.x).toEqual([15]);
  });

  it("ignores the colour column when no colour term was fitted", () => {
    const split = splitZeroPointPoints([12, 13], [-13, -12], 25, 0.02, [1, null], null);
    expect(split.fitted).toEqual({ x: [12, 13], y: [-13, -12] });
    expect(split.noColour).toEqual({ x: [], y: [] });
  });
});

describe("savePngWith", () => {
  it("reports the write error instead of dropping it", async () => {
    const out = await savePngWith(
      async () => "C:/data/plot.png",
      async () => new Uint8Array([137, 80, 78, 71]),
      async () => {
        throw new Error("The process cannot access the file (os error 32)");
      },
    );
    expect(out).toEqual({ kind: "failed", message: "The process cannot access the file (os error 32)" });
  });

  it("reports a string rejection from the dialog as a failure", async () => {
    const out = await savePngWith(
      () => Promise.reject("dialog plugin unavailable"),
      async () => new Uint8Array([1]),
      async () => {},
    );
    expect(out).toEqual({ kind: "failed", message: "dialog plugin unavailable" });
  });

  it("reports a PNG encoding failure as a failure, not a silent return", async () => {
    let wrote = false;
    const out = await savePngWith(
      async () => "C:/data/plot.png",
      async () => null,
      async () => {
        wrote = true;
      },
    );
    expect(out.kind).toBe("failed");
    expect(wrote).toBe(false);
  });

  it("treats a cancelled dialog as cancelled and never writes", async () => {
    let wrote = false;
    const out = await savePngWith(
      async () => null,
      async () => new Uint8Array([1]),
      async () => {
        wrote = true;
      },
    );
    expect(out).toEqual({ kind: "cancelled" });
    expect(wrote).toBe(false);
  });

  it("returns the saved path on success", async () => {
    const out = await savePngWith(async () => "C:/data/plot.png", async () => new Uint8Array([1]), async () => {});
    expect(out).toEqual({ kind: "saved", path: "C:/data/plot.png" });
  });
});
