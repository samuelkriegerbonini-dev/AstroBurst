import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  DEFAULT_LIGHT_CURVE_VIEW,
  type LightCurveDoc,
  type LightCurveSource,
  type LightCurveViewState,
  type LightCurveWindow,
} from "../../shared/types/lightCurve";
import {
  NO_BASELINE_NOTE,
  NO_DEPTH_NOTE,
  NO_TIME_BINS_NOTE,
  activeChoice,
  binText,
  lightCurveModel,
  modelSeries,
  periodogramInput,
  quickLookOf,
  seriesChoices,
  valueAxisLabel,
  type LightCurveModel,
  type PointState,
} from "../lightCurveViewModel";

interface WhiteLightFixture {
  int_num: number[];
  bmjd_tdb: number[];
  flux: number[];
  flux_err: number[];
}

const V = DEFAULT_LIGHT_CURVE_VIEW;
const T = Array.from({ length: 10 }, (_, i) => 2460310 + i);
const FB = [10.1, 9.9, 10.1, 9.9, 5.1, 4.9, 10.1, 9.9, 10.1, 9.9];
const W: LightCurveWindow = { start: 2460314, end: 2460315 };
const TABLE_SOURCE: LightCurveSource = { kind: "table", path: "p", time_column: "t", value_column: "f" };
const FLAGGED = [[], ["saturated"], [], [], ["pixel loss"], [], [], ["centroid jump", "low-confidence drift"], [], []];
const MARGIN_VIEW: LightCurveViewState = { ...V, window: W, marginDays: 1.5 };
const HAND_VIEW: LightCurveViewState = { ...V, window: W, excluded: [0, 4] };
const THREE_POINT_BINS: LightCurveViewState = { ...V, window: W, display: "normalised", bin: { kind: "points", points: 3 } };

const FIXTURE: WhiteLightFixture = JSON.parse(
  readFileSync(fileURLToPath(new URL("../../../src-tauri/tests/fixtures/lightcurve/e1_white_light.json", import.meta.url)), "utf8"),
);
const DATA_DIR = process.env.ASTROBURST_SCIENCE_DATA_DIR;
const WHTLT_FILE = "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_whtlt.ecsv";
const GOLDEN_DEPTH = 0.9503918297804987;
const GOLDEN_ERR_SCATTER = 0.006329466289158166;
const GOLDEN_ERR_FORMAL = 0.00027501456081958594;

function doc(over: Partial<LightCurveDoc> = {}): LightCurveDoc {
  return {
    id: "white",
    label: "white",
    quantity: "flux",
    unit: "Jy",
    timeScale: "bjd_tdb",
    time: [...T],
    value: [...FB],
    err: FB.map(() => 0.2),
    pointRef: T.map((_, i) => i + 1),
    flags: T.map(() => []),
    source: TABLE_SOURCE,
    notes: [],
    ...over,
  };
}

function series(
  time: number[],
  value: (number | null)[],
  err: (number | null)[],
  over: Partial<LightCurveDoc> = {},
): LightCurveDoc {
  return doc({ time, value, err, pointRef: time.map((_, i) => i), flags: time.map(() => []), ...over });
}

function sawtooth(): LightCurveDoc {
  const time = Array.from({ length: 21 }, (_, i) => 2460310 + i / 100);
  const value = time.map((_, i) => 1 + 0.001 * (((7 * i) % 5) - 2));
  value[10] = 1.5;
  value[14] = 0.5;
  value[15] = 0.5;
  value[16] = 0.5;
  return series(time, value, time.map(() => null));
}

function sawtoothView(document: LightCurveDoc): LightCurveViewState {
  return { ...V, clipSigma: 5, clipWindowPoints: 5, window: { start: document.time[14], end: document.time[16] } };
}

function e1Doc(): LightCurveDoc {
  return doc({
    timeScale: "bmjd_tdb",
    time: [...FIXTURE.bmjd_tdb],
    value: [...FIXTURE.flux],
    err: [...FIXTURE.flux_err],
    pointRef: [...FIXTURE.int_num],
    flags: FIXTURE.flux.map(() => []),
  });
}

function e1View(time: readonly number[]): LightCurveViewState {
  return { ...V, window: { start: time[182], end: time[187] }, marginDays: 0.0044 };
}

function readWhiteLightEcsv(dir: string): { names: string[]; rows: number[][] } {
  const text = readFileSync(join(dir, "tso", "whtlt", WHTLT_FILE), "utf8");
  const lines = text.split(/\r?\n/).filter((line) => line.length > 0 && !line.startsWith("#"));
  return { names: lines[0].split(" "), rows: lines.slice(1).map((line) => line.split(" ").map(Number)) };
}

function expectRelative(actual: number | null | undefined, expected: number, tolerance: number, label: string) {
  expect(typeof actual, `${label} measured ${actual}`).toBe("number");
  const relative = Math.abs((actual as number) - expected) / Math.abs(expected);
  expect(relative, `${label} measured ${actual}, expected ${expected}`).toBeLessThanOrEqual(tolerance);
}

function expectClose(actual: readonly (number | null)[], expected: readonly (number | null)[]) {
  expect(actual).toHaveLength(expected.length);
  expected.forEach((value, i) => {
    if (value === null) expect(actual[i], `element ${i}`).toBeNull();
    else expect(actual[i], `element ${i}`).toBeCloseTo(value, 12);
  });
}

function positionsOf(states: readonly PointState[], wanted: PointState): number[] {
  return states.flatMap((state, i) => (state === wanted ? [i] : []));
}

function everyKept(count: number): PointState[] {
  return Array.from({ length: count }, () => "kept" as const);
}

function deepFrozenDoc(document: LightCurveDoc): LightCurveDoc {
  Object.freeze(document.time);
  Object.freeze(document.value);
  Object.freeze(document.err);
  Object.freeze(document.pointRef);
  document.flags.forEach((words) => Object.freeze(words));
  Object.freeze(document.flags);
  return Object.freeze(document);
}

function frozenView(view: LightCurveViewState): LightCurveViewState {
  return Object.freeze({ ...view, excluded: Object.freeze([...view.excluded]) as number[] });
}

function expectWindowCards(model: LightCurveModel) {
  expectRelative(model.scatter!.ppm, 10690.449676496984, 1e-12, "scatter.ppm");
  expect(model.scatter!.mmag).toBeCloseTo(11.607008258917608, 12);
  expectRelative(model.scatter!.meanFormalPpm, 20000, 1e-12, "scatter.meanFormalPpm");
  expect(model.scatter!.ratio).toBeCloseTo(0.5345224838248492, 12);
  expect(model.scatter!.n).toEqual(8);
  expect(model.depth!.depth).toBeCloseTo(0.5, 12);
  expect(model.depth!.errScatter).toBeCloseTo(0.01017700489198213, 12);
  expect(model.depth!.errFormal).toBeCloseTo(0.014577379737113252, 12);
  expect(model.depth!.err).toEqual(model.depth!.errFormal);
  expect(model.depth!.nIn).toEqual(2);
  expect(model.depth!.nOut).toEqual(8);
  expect(model.depth!.fOut).toBeCloseTo(1, 12);
  expect(model.depth!.fIn).toBeCloseTo(0.5, 12);
}

describe("seriesChoices, activeChoice and valueAxisLabel", () => {
  it("groups the documents of one overlay into one choice", () => {
    const docs = [
      doc({ id: "target", label: "target T" }),
      doc({ id: "check:3", label: "check star K" }),
      doc({ id: "raw:1", label: "C1", overlay: "raw comps" }),
      doc({ id: "raw:2", label: "C2", overlay: "raw comps" }),
    ];
    const choices = seriesChoices(docs);
    expect(choices).toEqual([
      { key: "target", label: "target T", docIds: ["target"] },
      { key: "check:3", label: "check star K", docIds: ["check:3"] },
      { key: "overlay:raw comps", label: "raw comps", docIds: ["raw:1", "raw:2"] },
    ]);
    expect(activeChoice(choices, null)).toBe(choices[0]);
    expect(activeChoice(choices, "gone")).toBe(choices[0]);
    expect(activeChoice(choices, "overlay:raw comps")).toBe(choices[2]);
    expect(activeChoice([], null)).toBeNull();
    const interleaved = seriesChoices([docs[2], docs[0], docs[3]]);
    expect(interleaved).toEqual([
      { key: "overlay:raw comps", label: "raw comps", docIds: ["raw:1", "raw:2"] },
      { key: "target", label: "target T", docIds: ["target"] },
    ]);
  });

  it("names the value axis after the quantity, its unit and the comparison ensemble", () => {
    const frames = (comps: string[]): LightCurveSource => ({ kind: "frames", reference: "ref.fits", target: "T", comps });
    expect(valueAxisLabel(doc(), false)).toEqual("flux (Jy)");
    expect(valueAxisLabel(doc({ unit: null }), false)).toEqual("flux");
    expect(valueAxisLabel(doc({ unit: "" }), false)).toEqual("flux");
    expect(valueAxisLabel(doc({ quantity: "rel_flux" }), false)).toEqual("relative flux");
    expect(valueAxisLabel(doc({ quantity: "diff_mag", unit: "mag" }), false)).toEqual("diff mag");
    expect(valueAxisLabel(doc({ quantity: "diff_mag", unit: "mag", source: frames([]) }), false)).toEqual("diff mag");
    expect(valueAxisLabel(doc({ quantity: "diff_mag", unit: "mag", source: frames(["C1"]) }), false)).toEqual("diff mag (1 comp)");
    expect(valueAxisLabel(doc({ quantity: "diff_mag", unit: "mag", source: frames(["C1", "C2"]) }), false)).toEqual(
      "diff mag (sum of 2 comps)",
    );
    expect(valueAxisLabel(doc(), true)).toEqual("normalised flux");
    expect(valueAxisLabel(doc({ quantity: "diff_mag", unit: "mag", source: frames(["C1"]) }), true)).toEqual("normalised flux");
    expect(valueAxisLabel(doc({ quantity: "rel_flux" }), true)).toEqual("normalised flux");
  });
});

describe("lightCurveModel", () => {
  it("a flux document with a window gives the baseline, the scatter and the box depth", () => {
    const document = doc();
    const model = lightCurveModel(document, { ...V, window: W });
    expect(model.doc).toBe(document);
    expect(model.offset).toEqual(2460310);
    expect(model.x).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    expect(model.y).toEqual(FB);
    expect(model.yErr).toEqual(FB.map(() => 0.2));
    expect(model.state).toEqual(everyKept(10));
    expect(model.inWindow).toEqual([false, false, false, false, true, true, false, false, false, false]);
    expect(model.baseline!.kind).toEqual("mean");
    expect(model.baseline!.level).toBeCloseTo(10, 12);
    expect(model.baseline!.slope).toEqual(0);
    expect(model.baseline!.t0).toBeCloseTo(2460314.5, 9);
    expect(model.baseline!.nOut).toEqual(8);
    expectWindowCards(model);
    expect(model.normalised).toEqual(false);
    expect(model.invertY).toEqual(false);
    expect(model.yLabel).toEqual("flux (Jy)");
    expect(model.xLabel).toEqual("BJD_TDB - 2460310");
    expect(model.bands).toEqual([{ x0: 4, x1: 5, kind: "window" }]);
    expect(model.counts).toEqual({ total: 10, kept: 10, missing: 0, flagged: 0, manual: 0, clipped: 0 });
    expect(model.notes).toEqual([]);
    expect(model.binned).toBeNull();
    expect(model.binnedScatter).toBeNull();
    expect(model.folded).toBeNull();
  });

  it("the normalised display divides values and errors by the baseline and keeps the numbers of the cards", () => {
    const model = lightCurveModel(doc(), { ...V, window: W, display: "normalised" });
    expect(model.normalised).toEqual(true);
    expect(model.yLabel).toEqual("normalised flux");
    expect(model.invertY).toEqual(false);
    expectClose(model.y, [1.01, 0.99, 1.01, 0.99, 0.51, 0.49, 1.01, 0.99, 1.01, 0.99]);
    expectClose(model.yErr, FB.map(() => 0.02));
    expectWindowCards(model);
  });

  it("flagged points leave the analysis only for the excludable flags and only while the switch is on", () => {
    const model = lightCurveModel(doc({ flags: FLAGGED }), { ...V, window: W });
    expect(model.state[1]).toEqual("flagged");
    expect(model.state[7]).toEqual("flagged");
    expect(model.state[4]).toEqual("kept");
    expect(model.counts.kept).toEqual(8);
    expect(model.counts.flagged).toEqual(2);
    expect(model.baseline!.level).toBeCloseTo(10.033333333333333, 12);
    expect(model.baseline!.nOut).toEqual(6);
    expectRelative(model.scatter!.ppm, 10293.643445069554, 1e-12, "scatter.ppm");
    expect(model.depth!.depth).toBeCloseTo(0.5016611295681064, 12);
    expect(model.depth!.errScatter).toBeCloseTo(0.010184415921033517, 12);
    expect(model.depth!.errFormal).toBeCloseTo(0.014666955643517849, 12);
    expect(model.notes).toEqual(["1 of 2 in-transit points are flagged: pixel loss"]);
    expect(model.y).toEqual(FB);

    const switchedOff = lightCurveModel(doc({ flags: FLAGGED }), { ...V, window: W, excludeFlagged: false });
    expect(switchedOff.state).toEqual(everyKept(10));
    expect(switchedOff.depth!.depth).toBeCloseTo(0.5, 12);
    expect(switchedOff.notes).toEqual(["1 of 2 in-transit points are flagged: pixel loss"]);

    const bothInTransit = FLAGGED.map((words, i) => (i === 5 ? ["saturated", "pixel loss", "cosmic ray"] : words));
    const excludedInTransit = lightCurveModel(doc({ flags: bothInTransit }), { ...V, window: W });
    expect(excludedInTransit.state[5]).toEqual("flagged");
    expect(excludedInTransit.notes).toEqual(["2 of 2 in-transit points are flagged: pixel loss, saturated, cosmic ray"]);
  });

  it("points excluded by hand leave the analysis and are counted", () => {
    const model = lightCurveModel(doc(), HAND_VIEW);
    expect(model.state[0]).toEqual("manual");
    expect(model.state[4]).toEqual("manual");
    expect(model.counts.manual).toEqual(2);
    expect(model.counts.kept).toEqual(8);
    expect(model.baseline!.level).toBeCloseTo(9.985714285714286, 12);
    expect(model.baseline!.nOut).toEqual(7);
    expect(model.depth!.depth).toBeCloseTo(0.5092989985693848, 12);
    expect(model.depth!.nIn).toEqual(1);
    expect(model.depth!.errScatter).toBeNull();
    expect(model.depth!.errFormal).toBeCloseTo(0.020370174068905764, 12);
    expect(model.depth!.err).toEqual(model.depth!.errFormal);

    const flaggedFirst = lightCurveModel(doc({ flags: FLAGGED }), { ...V, window: W, excluded: [1, 4] });
    expect(flaggedFirst.state[1]).toEqual("flagged");
    expect(flaggedFirst.state[4]).toEqual("manual");
    expect(flaggedFirst.counts).toEqual({ total: 10, kept: 7, missing: 0, flagged: 2, manual: 1, clipped: 0 });
  });

  it("the margin takes the points next to the window out of the baseline and draws two margin bands", () => {
    const model = lightCurveModel(doc(), MARGIN_VIEW);
    expect(model.baseline!.nOut).toEqual(6);
    expect(model.baseline!.level).toBeCloseTo(10, 12);
    expectRelative(model.scatter!.ppm, 10954.451150103332, 1e-12, "scatter.ppm");
    expect(model.scatter!.n).toEqual(6);
    expect(model.depth!.depth).toBeCloseTo(0.5, 12);
    expect(model.depth!.errScatter).toBeCloseTo(0.01024695076595958, 12);
    expect(model.depth!.errFormal).toBeCloseTo(0.014719601443879746, 12);
    expect(model.depth!.nOut).toEqual(6);
    expect(model.bands).toEqual([
      { x0: 4, x1: 5, kind: "window" },
      { x0: 2.5, x1: 4, kind: "margin" },
      { x0: 5, x1: 6.5, kind: "margin" },
    ]);
    expect(model.state).toEqual(everyKept(10));
    expect(model.inWindow).toEqual([false, false, false, false, true, true, false, false, false, false]);

    const noMargin = lightCurveModel(doc(), { ...V, window: W, marginDays: NaN });
    expect(noMargin.baseline!.nOut).toEqual(8);
    expect(noMargin.bands).toEqual([{ x0: 4, x1: 5, kind: "window" }]);
    expect(lightCurveModel(doc(), { ...V, window: W, marginDays: -1 }).baseline!.nOut).toEqual(8);
  });

  it("a linear baseline removes a slope before the depth is read", () => {
    const value = [1.55, 1.65, 1.75, 1.85, 1, 1, 2.15, 2.25, 2.35, 2.45];
    const model = lightCurveModel(doc({ value, err: value.map(() => null) }), {
      ...V,
      window: W,
      baselineKind: "linear",
      display: "normalised",
    });
    expect(model.baseline!.level).toBeCloseTo(2, 12);
    expect(model.baseline!.slope).toBeCloseTo(0.1, 12);
    expect(model.baseline!.nOut).toEqual(8);
    for (const i of [0, 1, 2, 3, 6, 7, 8, 9]) expect(model.y[i], `y[${i}]`).toBeCloseTo(1, 10);
    expect(model.y[4]).toBeCloseTo(0.5128205128205129, 12);
    expect(model.y[5]).toBeCloseTo(0.4878048780487805, 12);
    expect(model.yErr).toEqual(value.map(() => null));
    expect(model.depth!.depth).toBeCloseTo(0.49968730456535326, 12);
    expect(model.depth!.errScatter).toBeCloseTo(0.012507817385866178, 12);
    expect(model.depth!.errFormal).toBeNull();
    expect(model.scatter!.ppm).toBeLessThan(1e-6);
    expect(model.scatter!.meanFormalPpm).toBeNull();

    const nativeBins = lightCurveModel(doc({ value, err: value.map(() => null) }), {
      ...V,
      window: W,
      baselineKind: "linear",
      bin: { kind: "points", points: 1 },
    });
    expect(nativeBins.binned!.y).toEqual(value);
    expect(nativeBins.binnedScatter!.n).toEqual(8);
    expect(nativeBins.binnedScatter!.ppm).toBeLessThan(1e-6);
  });

  it("without a window the baseline and the scatter use every kept point and there is no depth", () => {
    const document = doc();
    const model = lightCurveModel(document, V);
    expect(model.baseline!.level).toBeCloseTo(9, 12);
    expect(model.baseline!.nOut).toEqual(10);
    expectRelative(model.scatter!.ppm, 234535.41035537136, 1e-12, "scatter.ppm");
    expect(model.scatter!.n).toEqual(10);
    expect(model.depth).toBeNull();
    expect(model.bands).toEqual([]);
    expect(model.notes).toEqual([]);
    expect(model.inWindow).toEqual(T.map(() => false));
    expect(lightCurveModel(document, { ...V, window: { start: 2460315, end: 2460314 } })).toEqual(model);
    expect(lightCurveModel(document, { ...V, window: { start: NaN, end: 2460315 } })).toEqual(model);
    expect(lightCurveModel(document, { ...V, window: { start: 2460314, end: NaN } })).toEqual(model);
    expect(lightCurveModel(document, { ...V, window: { start: 2460314, end: Infinity }, marginDays: 1 })).toEqual(model);
  });

  it("a normalised display that cannot be fitted falls back to the values with a note", () => {
    const model = lightCurveModel(doc(), { ...V, window: { start: 2460310, end: 2460319 }, display: "normalised" });
    expect(model.baseline).toBeNull();
    expect(model.normalised).toEqual(false);
    expect(model.y).toEqual(FB);
    expect(model.yLabel).toEqual("flux (Jy)");
    expect(model.scatter).toBeNull();
    expect(model.depth).toBeNull();
    expect(model.notes).toEqual([NO_BASELINE_NOTE, NO_DEPTH_NOTE]);
    expect(NO_BASELINE_NOTE).toEqual(
      "no baseline: normalised flux needs points outside the transit window (two with different times for a linear baseline)",
    );
    expect(NO_DEPTH_NOTE).toEqual("no depth: the transit window holds no point, or no point is left outside it");
    expect(NO_TIME_BINS_NOTE).toEqual("no bins: binning by minutes needs a time axis, and this curve is on an index axis");
    const native = lightCurveModel(doc(), { ...V, window: { start: 2460310, end: 2460319 } });
    expect(native.notes).toEqual([NO_DEPTH_NOTE]);
  });

  it("missing times and values stay in the arrays as missing points", () => {
    const value: (number | null)[] = [...FB];
    value[2] = null;
    const time = [...T];
    time[8] = NaN;
    const model = lightCurveModel(doc({ value, time }), { ...V, window: W });
    expect(model.state[2]).toEqual("missing");
    expect(model.state[8]).toEqual("missing");
    expect(model.y[2]).toBeNull();
    expect(model.y[8]).toBeNull();
    expect(model.yErr[2]).toBeNull();
    expect(model.yErr[8]).toBeNull();
    expect(Number.isNaN(model.x[8])).toEqual(true);
    expect(model.x[2]).toEqual(2);
    expect(model.counts).toEqual({ total: 10, kept: 8, missing: 2, flagged: 0, manual: 0, clipped: 0 });
    expect(model.baseline!.level).toBeCloseTo(9.966666666666667, 12);
    expect(model.baseline!.nOut).toEqual(6);
    expect(model.depth!.depth).toBeCloseTo(0.4983277591973244, 12);

    const missingWins = lightCurveModel(doc({ value, time, flags: FLAGGED.map((words, i) => (i === 2 ? ["saturated"] : words)) }), {
      ...V,
      window: W,
      excluded: [8],
    });
    expect(missingWins.state[2]).toEqual("missing");
    expect(missingWins.state[8]).toEqual("missing");
    const noError = lightCurveModel(doc({ err: FB.map((_, i) => (i === 3 ? NaN : 0.2)) }), { ...V, window: W });
    expect(noError.yErr[3]).toBeNull();
    expect(noError.y[3]).toEqual(9.9);
    expect(noError.state[3]).toEqual("kept");

    const emptyInTransit: (number | null)[] = [...FB];
    emptyInTransit[5] = null;
    const oneLeft = lightCurveModel(doc({ value: emptyInTransit, flags: FLAGGED }), { ...V, window: W });
    expect(oneLeft.inWindow[5]).toEqual(false);
    expect(oneLeft.inWindow[4]).toEqual(true);
    expect(oneLeft.notes).toEqual(["1 of 1 in-transit points are flagged: pixel loss"]);
    const endless = lightCurveModel(doc({ time: T.map((time, i) => (i === 8 ? Infinity : time)) }), { ...V, window: W });
    expect(endless.state[8]).toEqual("missing");
    expect(Number.isNaN(endless.x[8])).toEqual(true);
  });

  it("clipping never touches the window or its margin", () => {
    const document = sawtooth();
    const model = lightCurveModel(document, sawtoothView(document));
    expect(positionsOf(model.state, "clipped")).toEqual([10]);
    expect(positionsOf(model.state, "kept")).toHaveLength(20);
    expect(model.counts.clipped).toEqual(1);
    expect(model.depth!.depth).toBeCloseTo(0.5000294100347039, 12);
    expect(model.depth!.nIn).toEqual(3);
    expect(model.depth!.nOut).toEqual(17);
    expectRelative(model.scatter!.ppm, 1477.6896262964538, 1e-12, "scatter.ppm");

    const noWindow = lightCurveModel(document, { ...sawtoothView(document), window: null });
    expect(positionsOf(noWindow.state, "clipped")).toEqual([10]);
    expect(positionsOf(noWindow.state, "kept")).toHaveLength(20);
    expect(noWindow.depth).toBeNull();

    const noClipping = lightCurveModel(document, { ...sawtoothView(document), clipSigma: null });
    expect(noClipping.state).toEqual(everyKept(21));
    expect(lightCurveModel(document, { ...sawtoothView(document), clipSigma: 0 }).state).toEqual(everyKept(21));

    const inMargin = lightCurveModel(document, {
      ...V,
      clipSigma: 5,
      clipWindowPoints: 5,
      window: { start: document.time[11], end: document.time[11] },
      marginDays: 0.015,
    });
    expect(inMargin.state).toEqual(everyKept(21));
    expect(inMargin.baseline!.nOut).toEqual(18);

    const outlierInWindow = lightCurveModel(document, {
      ...sawtoothView(document),
      window: { start: document.time[10], end: document.time[10] },
    });
    expect(outlierInWindow.state).toEqual(everyKept(21));
    expect(outlierInWindow.depth!.nIn).toEqual(1);

    const outlierByHand = lightCurveModel(document, { ...sawtoothView(document), window: null, excluded: [10] });
    expect(outlierByHand.state[10]).toEqual("manual");
    expect(outlierByHand.counts.clipped).toEqual(0);
  });

  it("a differential-magnitude document is analysed as relative flux and drawn as magnitudes", () => {
    const time = [2460310.5, 2460310.6, 2460310.7, 2460310.8];
    const value = [1.505149978319906, 1.505149978319906, 2.2577249674798594, 1.505149978319906];
    const document = series(time, value, time.map(() => 0.013297055068698485), {
      quantity: "diff_mag",
      unit: "mag",
      magErrFactor: 1.0857,
      source: { kind: "frames", reference: "ref.fits", target: "T", comps: ["C1", "C2"] },
    });
    const view: LightCurveViewState = { ...V, window: { start: 2460310.7, end: 2460310.7 } };
    const native = lightCurveModel(document, view);
    expect(native.y).toEqual(value);
    expect(native.yErr).toEqual(time.map(() => 0.013297055068698485));
    expect(native.invertY).toEqual(true);
    expect(native.yLabel).toEqual("diff mag (sum of 2 comps)");
    expect(native.baseline!.level).toBeCloseTo(0.25, 12);
    expect(native.depth!.depth).toBeCloseTo(0.5, 12);
    expect(native.depth!.errFormal).toBeCloseTo(0.007071067811865474, 12);
    expect(native.depth!.errScatter).toBeNull();
    expect(native.depth!.nIn).toEqual(1);
    expect(native.depth!.nOut).toEqual(3);

    const normalised = lightCurveModel(document, { ...view, display: "normalised" });
    expectClose(normalised.y, [1, 1, 0.5, 1]);
    expectClose(normalised.yErr, [0.012247448713915891, 0.012247448713915891, 0.0061237243569579455, 0.012247448713915891]);
    expect(normalised.invertY).toEqual(false);
    expect(normalised.yLabel).toEqual("normalised flux");

    const exactFactor = lightCurveModel({ ...document, magErrFactor: undefined }, { ...view, display: "normalised" });
    expect(exactFactor.yErr[0]).toBeCloseTo(0.01224704031276242, 12);
  });

  it("bins are built from the kept drawn values and report their own scatter", () => {
    const byPoints = lightCurveModel(doc(), THREE_POINT_BINS);
    expect(byPoints.binned!.x).toEqual([1, 4, 7, 9]);
    expectClose(byPoints.binned!.y, [1.0033333333333334, 0.6633333333333333, 1.0033333333333334, 0.99]);
    expectClose(byPoints.binned!.yErr, [0.011547005383792516, 0.011547005383792516, 0.011547005383792516, 0.02]);
    expect(byPoints.binned!.n).toEqual([3, 3, 3, 1]);
    expect(byPoints.binnedScatter!.n).toEqual(3);
    expectRelative(byPoints.binnedScatter!.ppm, 7706.566440795943, 1e-12, "binnedScatter.ppm");

    const byMinutes = lightCurveModel(doc(), { ...V, window: W, bin: { kind: "minutes", minutes: 2880 } });
    expect(byMinutes.binned!.x).toEqual([0.5, 2.5, 4.5, 6.5, 8.5]);
    expectClose(byMinutes.binned!.y, [10, 10, 5, 10, 10]);
    expectClose(byMinutes.binned!.yErr, [0, 0, 0, 0, 0].map(() => 0.14142135623730953));
    expect(byMinutes.binned!.n).toEqual([2, 2, 2, 2, 2]);
    expect(byMinutes.binnedScatter!.ppm).toBeCloseTo(0, 12);
    expect(byMinutes.binnedScatter!.n).toEqual(4);
    expect(byMinutes.notes).toEqual([]);

    const onIndex = doc({ timeScale: "index", time: T.map((_, i) => i) });
    const noBins = lightCurveModel(onIndex, { ...V, bin: { kind: "minutes", minutes: 5 } });
    expect(noBins.binned).toBeNull();
    expect(noBins.binnedScatter).toBeNull();
    expect(noBins.notes).toEqual([NO_TIME_BINS_NOTE]);
    expect(noBins.x).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    expect(noBins.xLabel).toEqual("frame index");
    expect(lightCurveModel(onIndex, { ...V, bin: { kind: "points", points: 5 } }).binned!.x).toEqual([2, 7]);

    const withoutExcluded = lightCurveModel(doc(), { ...V, bin: { kind: "points", points: 4 }, excluded: [0, 1] });
    expect(withoutExcluded.binned!.x).toEqual([3.5, 7.5]);
    expect(withoutExcluded.binned!.n).toEqual([4, 4]);
  });

  it("the fold sorts the kept points by phase", () => {
    const time = [2460310, 2460310.5, 2460311.25, 2460309.75, 2460307];
    const document = series(time, [1, 2, 3, 4, 5], time.map(() => null));
    const fold = { periodDays: 1, t0: 2460310 };
    expect(lightCurveModel(document, { ...V, fold }).folded).toEqual({
      phase: [0, 0, 0.25, 0.5, 0.75],
      y: [1, 5, 3, 2, 4],
      yErr: [null, null, null, null, null],
      index: [0, 4, 2, 1, 3],
    });
    expect(lightCurveModel(document, { ...V, fold: { periodDays: 0, t0: 2460310 } }).folded).toBeNull();
    expect(lightCurveModel(document, { ...V, fold: { periodDays: 1, t0: NaN } }).folded).toBeNull();
    expect(lightCurveModel({ ...document, timeScale: "index" }, { ...V, fold }).folded).toBeNull();
    const byHand = lightCurveModel(document, { ...V, fold, excluded: [4] }).folded!;
    expect(byHand.index).toEqual([0, 2, 1, 3]);
    expect(byHand.phase).toEqual([0, 0.25, 0.5, 0.75]);
    expect(byHand.y).toEqual([1, 3, 2, 4]);
  });
});

describe("modelSeries, quickLookOf and periodogramInput", () => {
  it("modelSeries splits the kept and the excluded points without changing their positions", () => {
    const model = lightCurveModel(doc(), HAND_VIEW);
    const split = modelSeries(model);
    expect(split.kept.y).toEqual([null, 9.9, 10.1, 9.9, null, 4.9, 10.1, 9.9, 10.1, 9.9]);
    expect(split.kept.yErr).toEqual([null, 0.2, 0.2, 0.2, null, 0.2, 0.2, 0.2, 0.2, 0.2]);
    expect(split.excluded!.y).toEqual([10.1, null, null, null, 5.1, null, null, null, null, null]);
    expect(split.kept.x).toEqual(model.x);
    expect(split.excluded!.x).toEqual(model.x);
    expect(modelSeries(lightCurveModel(doc(), { ...V, window: W })).excluded).toBeNull();

    const normalised = modelSeries(lightCurveModel(doc(), { ...HAND_VIEW, display: "normalised" }));
    expect(normalised.excluded!.y[0]).toBeCloseTo(10.1 / (69.9 / 7), 12);
    expect(normalised.excluded!.y[4]).toBeCloseTo(5.1 / (69.9 / 7), 12);
    expect(normalised.kept.y[0]).toBeNull();
    expect(normalised.kept.yErr[1]).toBeCloseTo(0.2 / (69.9 / 7), 12);

    const value: (number | null)[] = [...FB];
    value[2] = null;
    const onlyMissing = modelSeries(lightCurveModel(doc({ value }), V));
    expect(onlyMissing.excluded).toBeNull();
    expect(onlyMissing.kept.y[2]).toBeNull();

    const onlyFlagged = modelSeries(lightCurveModel(doc({ flags: FLAGGED }), V));
    expect(onlyFlagged.excluded!.y).toEqual([null, 9.9, null, null, null, null, null, 9.9, null, null]);

    const document = sawtooth();
    const clipped = modelSeries(lightCurveModel(document, sawtoothView(document)));
    expect(clipped.excluded!.y.flatMap((y, i) => (y === null ? [] : [i]))).toEqual([10]);
    expect(clipped.excluded!.y[10]).toEqual(1.5);
    expect(clipped.kept.y[10]).toBeNull();
  });

  it("quickLookOf copies the depth, the scatter and the choices behind them", () => {
    const document = doc();
    const look = quickLookOf(lightCurveModel(document, MARGIN_VIEW), MARGIN_VIEW)!;
    expect(look.docId).toEqual("white");
    expect(look.label).toEqual("white");
    expect(look.source).toEqual(TABLE_SOURCE);
    expect(look.quantity).toEqual("flux");
    expect(look.unit).toEqual("Jy");
    expect(look.timeScale).toEqual("bjd_tdb");
    expect(look.timeName).toEqual("BJD_TDB");
    expect(look.nPoints).toEqual(10);
    expect(look.window).toEqual(W);
    expect(look.marginDays).toEqual(1.5);
    expect(look.baselineKind).toEqual("mean");
    expect(look.depth).toBeCloseTo(0.5, 12);
    expect(look.errScatter).toBeCloseTo(0.01024695076595958, 12);
    expect(look.errFormal).toBeCloseTo(0.014719601443879746, 12);
    expect(look.err).toBeCloseTo(0.014719601443879746, 12);
    expect(look.nIn).toEqual(2);
    expect(look.nOut).toEqual(6);
    expectRelative(look.scatterPpm, 10954.451150103332, 1e-12, "scatterPpm");
    expectRelative(look.formalPpm, 20000, 1e-12, "formalPpm");
    expect(look.scatterRatio).toBeCloseTo(0.5477225575051666, 12);
    expect(look.binned).toBeNull();
    expect(look.nExcluded).toEqual(0);
    expect(look.nClipped).toEqual(0);
    expect(look.notes).toEqual([]);

    expect(quickLookOf(lightCurveModel(document, THREE_POINT_BINS), THREE_POINT_BINS)!.binned).toEqual("3 points");
    expect(binText({ kind: "minutes", minutes: 5 })).toEqual("5 min");
    expect(binText({ kind: "points", points: 3 })).toEqual("3 points");
    expect(binText({ kind: "none" })).toBeNull();
    expect(quickLookOf(lightCurveModel(document, V), V)).toBeNull();

    const badMargin: LightCurveViewState = { ...V, window: W, marginDays: NaN };
    expect(quickLookOf(lightCurveModel(document, badMargin), badMargin)!.marginDays).toEqual(0);
    const negativeMargin: LightCurveViewState = { ...V, window: W, marginDays: -1 };
    expect(quickLookOf(lightCurveModel(document, negativeMargin), negativeMargin)!.marginDays).toEqual(0);

    const counted: LightCurveViewState = { ...V, window: W, excluded: [0] };
    const countedLook = quickLookOf(lightCurveModel(doc({ flags: FLAGGED }), counted), counted)!;
    expect(countedLook.nExcluded).toEqual(3);
    expect(countedLook.nPoints).toEqual(7);
    expect(countedLook.notes).toEqual(["1 of 2 in-transit points are flagged: pixel loss"]);

    const single = lightCurveModel(doc(), HAND_VIEW);
    const singleLook = quickLookOf(single, HAND_VIEW)!;
    expect(singleLook.errScatter).toBeNull();
    expect(singleLook.err).toEqual(single.depth!.errFormal);

    const sawtoothDoc = sawtooth();
    const clippedLook = quickLookOf(lightCurveModel(sawtoothDoc, sawtoothView(sawtoothDoc)), sawtoothView(sawtoothDoc))!;
    expect(clippedLook.nClipped).toEqual(1);
    expect(clippedLook.nExcluded).toEqual(0);
    expect(clippedLook.formalPpm).toBeNull();
    expect(clippedLook.scatterRatio).toBeNull();
    expect(clippedLook.timeName).toEqual("BJD_TDB");
  });

  it("periodogramInput keeps finite kept points and gives errors only when every point has one", () => {
    const input = periodogramInput(lightCurveModel(doc(), HAND_VIEW));
    expect(input.time).toHaveLength(8);
    expect(input.time[0]).toEqual(2460311);
    expect(input.time).toEqual([2460311, 2460312, 2460313, 2460315, 2460316, 2460317, 2460318, 2460319]);
    expect(input.value).toEqual([9.9, 10.1, 9.9, 4.9, 10.1, 9.9, 10.1, 9.9]);
    expect(input.err).toEqual([0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2]);

    const oneMissing = doc({ err: FB.map((_, i) => (i === 3 ? null : 0.2)) });
    expect(periodogramInput(lightCurveModel(oneMissing, HAND_VIEW)).err).toBeNull();
    const oneZero = doc({ err: FB.map((_, i) => (i === 3 ? 0 : 0.2)) });
    expect(periodogramInput(lightCurveModel(oneZero, HAND_VIEW)).err).toBeNull();
    const missingOnExcluded = doc({ err: FB.map((_, i) => (i === 0 ? null : 0.2)) });
    expect(periodogramInput(lightCurveModel(missingOnExcluded, HAND_VIEW)).err).toHaveLength(8);

    expect(periodogramInput(lightCurveModel(series([], [], []), V))).toEqual({ time: [], value: [], err: null });
  });
});

describe("edges of the model", () => {
  it("an empty document and a single point give an empty model, not an exception", () => {
    const empty = lightCurveModel(series([], [], []), V);
    expect(empty.x).toEqual([]);
    expect(empty.y).toEqual([]);
    expect(empty.state).toEqual([]);
    expect(empty.baseline).toBeNull();
    expect(empty.scatter).toBeNull();
    expect(empty.depth).toBeNull();
    expect(empty.counts.total).toEqual(0);
    expect(empty.offset).toEqual(0);
    expect(modelSeries(empty)).toEqual({ kept: { x: [], y: [], yErr: [] }, excluded: null });

    const single = lightCurveModel(series([2460310], [5], [null]), V);
    expect(single.baseline!.level).toEqual(5);
    expect(single.scatter).toBeNull();
    expect(single.depth).toBeNull();
    expect(single.x).toEqual([0]);
  });

  it("the model does not change the document or the view", () => {
    const sawtoothDoc = sawtooth();
    const cases: [LightCurveDoc, LightCurveViewState][] = [
      [doc(), { ...V, window: W }],
      [doc({ flags: FLAGGED.map((words) => [...words]) }), { ...V, window: W }],
      [sawtoothDoc, sawtoothView(sawtoothDoc)],
      [doc(), THREE_POINT_BINS],
      [doc(), { ...HAND_VIEW, fold: { periodDays: 3, t0: 2460310 } }],
    ];
    for (const [document, view] of cases) {
      const before = JSON.stringify([document, view]);
      const frozenDocument = deepFrozenDoc(document);
      const frozen = frozenView(view);
      const model = lightCurveModel(frozenDocument, frozen);
      expect(() => modelSeries(model)).not.toThrow();
      expect(() => quickLookOf(model, frozen)).not.toThrow();
      expect(() => periodogramInput(model)).not.toThrow();
      expect(JSON.stringify([frozenDocument, frozen])).toEqual(before);
    }
  });
});

describe("the committed white light of WD 1856+534 b", () => {
  it("the white light of WD 1856+534 b gives the golden depth through the model", () => {
    const document = e1Doc();
    const view = e1View(document.time);
    const model = lightCurveModel(document, view);
    expectRelative(model.depth!.depth, GOLDEN_DEPTH, 1e-10, "depth");
    expectRelative(model.depth!.errScatter, GOLDEN_ERR_SCATTER, 1e-10, "errScatter");
    expectRelative(model.depth!.errFormal, GOLDEN_ERR_FORMAL, 1e-10, "errFormal");
    expect(model.depth!.nIn).toEqual(6);
    expect(model.depth!.nOut).toEqual(210);
    expectRelative(model.scatter!.ppm, 927.3509315525, 1e-9, "scatter.ppm");
    expectRelative(model.scatter!.mmag, 1.0068584809027, 1e-9, "scatter.mmag");
    expectRelative(model.scatter!.meanFormalPpm, 512.8350502783, 1e-9, "scatter.meanFormalPpm");
    expectRelative(model.scatter!.ratio, 1.808283055242, 1e-9, "scatter.ratio");
    expectRelative(model.baseline!.level, 0.16395311167095059, 1e-9, "baseline.level");
    expect(model.baseline!.nOut).toEqual(210);
    expect(model.offset).toEqual(60061);
    expect(model.x[0]).toBeCloseTo(0.44217334569839295, 9);
    expect(model.xLabel).toEqual("BMJD (TDB) - 60061");
    expect(model.counts.kept).toEqual(240);
    expect(positionsOf(model.inWindow.map((inside) => (inside ? "kept" : "missing")), "kept")).toEqual([182, 183, 184, 185, 186, 187]);
    expect(model.bands).toHaveLength(3);
    expect(model.bands[1].x1 - model.bands[1].x0).toBeCloseTo(0.0044, 9);

    const normalised = lightCurveModel(document, { ...view, display: "normalised" });
    expectRelative(normalised.y[185], 0.03126827293006477, 1e-10, "normalised y[185]");
    expectRelative(normalised.y[0], 0.9984532090502577, 1e-10, "normalised y[0]");

    const linear = lightCurveModel(document, { ...view, baselineKind: "linear" });
    expectRelative(linear.depth!.depth, 0.9503885316521851, 1e-9, "linear depth");
    expectRelative(linear.baseline!.slope, -0.0004124872721696969, 1e-9, "linear slope");

    const noMargin = lightCurveModel(document, { ...view, marginDays: 0 });
    expect(noMargin.depth!.nOut).toEqual(234);
    expectRelative(noMargin.depth!.depth, 0.9495337325685955, 1e-10, "depth without the margin");
  });

  it("clipping and binning on the same curve", () => {
    const document = e1Doc();
    const view = e1View(document.time);
    const clipped = lightCurveModel(document, { ...view, clipSigma: 5 });
    expect(positionsOf(clipped.state, "clipped")).toEqual([125, 210, 212]);
    expect(clipped.scatter!.n).toEqual(207);
    expectRelative(clipped.scatter!.ppm, 635.2015069795015, 1e-9, "clipped scatter.ppm");
    expectRelative(clipped.depth!.depth, 0.9503957291573791, 1e-9, "clipped depth");

    const binned = lightCurveModel(document, { ...view, display: "normalised", bin: { kind: "points", points: 10 } });
    expect(binned.binned!.x).toHaveLength(24);
    expect(binned.binned!.n).toEqual(Array.from({ length: 24 }, () => 10));
    expectRelative(binned.binned!.y[0], 0.999936467184672, 1e-9, "binned y[0]");
    expectRelative(binned.binned!.yErr[0], 0.00016215528303869324, 1e-9, "binned yErr[0]");
    expect(binned.binnedScatter!.n).toEqual(21);
    expectRelative(binned.binnedScatter!.ppm, 305.24859610888905, 1e-9, "binnedScatter.ppm");
  });
});

describe.skipIf(!DATA_DIR)("the archived white light of WD 1856+534 b (ASTROBURST_SCIENCE_DATA_DIR)", () => {
  it("real_e1_whtlt_table_gives_the_golden_depth_through_the_model", () => {
    const { names, rows } = readWhiteLightEcsv(DATA_DIR as string);
    expect(rows).toHaveLength(240);
    const column = (name: string) => rows.map((row) => row[names.indexOf(name)]);
    const time = column("BJD_TDB_NRS1");
    const document = series(time, column("whitelight_flux_NRS1"), time.map(() => null), { timeScale: "bmjd_tdb" });
    const model = lightCurveModel(document, e1View(time));
    expectRelative(model.depth!.depth, GOLDEN_DEPTH, 1e-10, "depth");
    expectRelative(model.depth!.errScatter, GOLDEN_ERR_SCATTER, 1e-10, "errScatter");
    expect(model.depth!.errFormal).toBeNull();
    expect(model.depth!.err).toEqual(model.depth!.errScatter);
    expect(model.depth!.nIn).toEqual(6);
    expect(model.depth!.nOut).toEqual(210);
    expect(model.scatter!.meanFormalPpm).toBeNull();
  });
});
