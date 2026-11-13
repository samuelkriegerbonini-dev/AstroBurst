import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  MAG_ERR_EXACT,
  QUICK_LOOK_LABEL,
  binInTime,
  boxDepth,
  fitBaseline,
  foldPhase,
  normalise,
  outOfTransitScatter,
  relativeFlux,
  sigmaClip,
  type Baseline,
  type TimeWindow,
} from "../lightCurveAnalysis";

interface WhiteLightFixture {
  int_num: number[];
  bmjd_tdb: number[];
  mjd_utc: number[];
  flux: number[];
  flux_err: number[];
}

const T10 = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
const TRANSIT: TimeWindow = { start: 4, end: 5 };
const WINGS: TimeWindow[] = [
  { start: 0, end: 1 },
  { start: 8, end: 9 },
];
const FB = [10.1, 9.9, 10.1, 9.9, 5.1, 4.9, 10.1, 9.9, 10.1, 9.9];
const ERR_02 = FB.map(() => 0.2);
const NO_ERR: null[] = FB.map(() => null);
const SQRT_THIRD = 0.5773502691896258;

const FIXTURE: WhiteLightFixture = JSON.parse(
  readFileSync(fileURLToPath(new URL("../../../src-tauri/tests/fixtures/lightcurve/e1_white_light.json", import.meta.url)), "utf8"),
);
const DATA_DIR = process.env.ASTROBURST_SCIENCE_DATA_DIR;
const WHTLT_FILE = "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_whtlt.ecsv";
const GOLDEN_DEPTH = 0.9503918297804987;
const GOLDEN_ERR_SCATTER = 0.006329466289158166;

function e1Window(t: readonly number[]): TimeWindow {
  return { start: t[182], end: t[187] };
}

function e1Baseline(t: readonly number[]): TimeWindow[] {
  return [
    { start: t[0], end: t[169] },
    { start: t[200], end: t[239] },
  ];
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

function clippedIndices(mask: readonly boolean[]): number[] {
  return mask.flatMap((clipped, i) => (clipped ? [i] : []));
}

function sawtoothWithOutlier(): (number | null)[] {
  const flux: (number | null)[] = Array.from({ length: 21 }, (_, i) => 1 + 0.001 * (((7 * i) % 5) - 2));
  flux[10] = 1.5;
  return flux;
}

const T21 = Array.from({ length: 21 }, (_, i) => i);

function readWhiteLightEcsv(dir: string): { names: string[]; rows: number[][] } {
  const text = readFileSync(join(dir, "tso", "whtlt", WHTLT_FILE), "utf8");
  const lines = text.split(/\r?\n/).filter((line) => line.length > 0 && !line.startsWith("#"));
  return { names: lines[0].split(" "), rows: lines.slice(1).map((line) => line.split(" ").map(Number)) };
}

function bitDifferences(actual: readonly number[], expected: readonly number[]): number[] {
  const length = Math.max(actual.length, expected.length);
  return Array.from({ length }, (_, i) => i).filter((i) => !Object.is(actual[i], expected[i]));
}

describe("relativeFlux and the constants", () => {
  it("relativeFlux turns magnitudes into fluxes with the error of first order", () => {
    const curve = relativeFlux([0, 2.5, -2.5, null], [0.010857362047581294, 0.1, null, 0.1]);
    expectClose(curve.flux, [1, 0.1, 10, null]);
    expectClose(curve.err, [0.01, 0.009210340371976187, null, null]);
  });

  it("relativeFlux with the factor of the image series reproduces the ExoClock golden", () => {
    const curve = relativeFlux([1.505149978319906], [0.013297055068698485], 1.0857);
    expect(Number((curve.flux[0] as number).toPrecision(10))).toBe(0.25);
    expect(Number((curve.err[0] as number).toPrecision(10))).toBe(0.003061862178);
  });

  it("the exported constants hold the label of decision D7 and 2.5 / ln 10", () => {
    expect(QUICK_LOOK_LABEL).toBe("quick look, no limb darkening or systematics model");
    expect(MAG_ERR_EXACT).toBe(1.0857362047581294);
  });
});

describe("fitBaseline and normalise", () => {
  it("fitBaseline takes the mean of the points outside the window", () => {
    const flux = [1, 1, 1, 1, 0.5, 0.5, 1, 1, 1, 1];
    expect(fitBaseline(T10, flux, TRANSIT, "mean")).toEqual({ kind: "mean", level: 1, slope: 0, t0: 4.5, nOut: 8 });
    const whole = fitBaseline(T10, flux, null, "mean");
    expect(whole!.level).toBeCloseTo(0.9, 12);
    expect(whole!.nOut).toEqual(10);
  });

  it("fitBaseline fits a straight line in time outside the window", () => {
    const flux = [1.55, 1.65, 1.75, 1.85, 1, 1, 2.15, 2.25, 2.35, 2.45];
    const line = fitBaseline(T10, flux, TRANSIT, "linear");
    expect(line!.kind).toEqual("linear");
    expect(line!.level).toBeCloseTo(2, 12);
    expect(line!.slope).toBeCloseTo(0.1, 12);
    expect(line!.t0).toBeCloseTo(4.5, 12);
    expect(line!.nOut).toEqual(8);
  });

  it("fitBaseline with the median ignores one wild point", () => {
    const flux = [1, 1, 1, 9, 0.5, 0.5, 1, 1, 1, 1];
    const median = fitBaseline(T10, flux, TRANSIT, "median");
    expect(median!.level).toBeCloseTo(1, 12);
    expect(median!.slope).toEqual(0);
    expect(fitBaseline(T10, flux, TRANSIT, "mean")!.level).toBeCloseTo(2, 12);
  });

  it("fitBaseline keeps only the points inside the baseline windows", () => {
    const flux = [1, 1, 1, 9, 0.5, 0.5, 1, 1, 1, 1];
    const wings = fitBaseline(T10, flux, TRANSIT, "mean", WINGS);
    expect(wings!.level).toBeCloseTo(1, 12);
    expect(wings!.nOut).toEqual(4);
    expect(wings!.t0).toBeCloseTo(4.5, 12);
  });

  it("fitBaseline returns null when nothing can be fitted", () => {
    expect(fitBaseline([0, 1], [null, null], null, "mean")).toBeNull();
    expect(fitBaseline([0, 1], [1, null], null, "linear")).toBeNull();
    expect(fitBaseline([3, 3], [1, 2], null, "linear")).toBeNull();
  });

  it("normalise divides flux and error by the baseline model", () => {
    const rising: Baseline = { kind: "linear", level: 2, slope: 0.1, t0: 4.5, nOut: 8 };
    const curve = normalise([9, 0, 2], [2.45, null, 5], [0.0245, 0.1, null], rising);
    expectClose(curve.flux, [1, null, 2.857142857142857]);
    expectClose(curve.err, [0.01, null, null]);
    const negative: Baseline = { kind: "linear", level: 1, slope: -1, t0: 0, nOut: 2 };
    const lost = normalise([2], [5], [1], negative);
    expect(lost.flux).toEqual([null]);
    expect(lost.err).toEqual([null]);
  });
});

describe("binInTime", () => {
  const T7 = [0, 1, 2, 3, 4, 5, 6];
  const F7 = [1, 2, 3, 4, 5, 6, 7];

  it("binInTime weights by inverse variance when every point has an error", () => {
    const binned = binInTime(T7, F7, [1, 1, 2, 1, 1, 1, 1], { points: 3 });
    expectClose(binned.t, [1, 4, 6]);
    expectClose(binned.flux, [1.6666666666666667, 5, 7]);
    expectClose(binned.err, [0.6666666666666666, SQRT_THIRD, 1]);
    expect(binned.n).toEqual([3, 3, 1]);
    expect(binned.weighted).toEqual(true);
  });

  it("binInTime falls back to the plain mean and the scatter when one error is missing", () => {
    const binned = binInTime(T7, F7, [1, 1, null, 1, 1, 1, 1], { points: 3 });
    expectClose(binned.flux, [2, 5, 7]);
    expectClose(binned.err, [SQRT_THIRD, SQRT_THIRD, null]);
    expect(binned.weighted).toEqual(false);
    expect(binInTime(T7, F7, [1, 1, 0, 1, 1, 1, 1], { points: 3 }).weighted).toEqual(false);
  });

  it("binInTime groups by a width in days and leaves empty bins out", () => {
    const binned = binInTime([0, 0.5, 1.9, 2.0, 7.1], [1, 2, 3, 4, 5], [null, null, null, null, null], { widthDays: 2 });
    expectClose(binned.t, [0.8, 2, 7.1]);
    expectClose(binned.flux, [2, 4, 5]);
    expectClose(binned.err, [SQRT_THIRD, null, null]);
    expect(binned.n).toEqual([3, 1, 1]);
  });

  it("binInTime sorts by time and skips missing points", () => {
    const binned = binInTime([2, 0, 1, NaN, 3], [3, 1, 2, 9, null], [1, 1, 1, 1, 1], { points: 2 });
    expectClose(binned.t, [0.5, 2]);
    expectClose(binned.flux, [1.5, 3]);
    expectClose(binned.err, [0.7071067811865475, 1]);
    expect(binned.n).toEqual([2, 1]);
  });

  it("binInTime returns empty arrays for no points and for a width that is not positive", () => {
    for (const binned of [binInTime([], [], [], { points: 2 }), binInTime([0, 1], [1, 2], [1, 1], { widthDays: 0 })]) {
      expect(binned.t).toEqual([]);
      expect(binned.flux).toEqual([]);
      expect(binned.err).toEqual([]);
      expect(binned.n).toEqual([]);
    }
  });
});

describe("sigmaClip", () => {
  it("sigmaClip flags one outlier against the running median", () => {
    const flux = sawtoothWithOutlier();
    const mask = sigmaClip(T21, flux, 5, 5);
    expect(mask).toHaveLength(21);
    expect(clippedIndices(mask)).toEqual([10]);
    expect(clippedIndices(sigmaClip(T21, flux, 1.4, 5))).toEqual([10]);
    expect(clippedIndices(sigmaClip(T21, flux, 1.3, 5))).toEqual([0, 2, 5, 7, 8, 10, 15, 17]);
    flux[3] = null;
    const withGap = sigmaClip(T21, flux, 5, 5);
    expect(clippedIndices(withGap)).toEqual([10]);
    expect(withGap[3]).toEqual(false);
  });

  it("sigmaClip clips nothing when the robust scatter is zero or the series is too short", () => {
    const flat = T21.map(() => 1);
    flat[10] = 1.5;
    expect(sigmaClip(T21, flat, 5, 5)).toEqual(T21.map(() => false));
    expect(sigmaClip([0, 1], [1, 5], 3, 5)).toEqual([false, false]);
    expect(sigmaClip(T21, sawtoothWithOutlier(), 0, 5)).toEqual(T21.map(() => false));
  });
});

describe("outOfTransitScatter", () => {
  const flux = [1.001, 0.999, 1.001, 0.999, 0.5, 0.5, 1.001, 0.999, 1.001, 0.999];

  it("outOfTransitScatter reports the sample scatter in ppm and mmag against the formal error", () => {
    const scatter = outOfTransitScatter(T10, flux, flux.map(() => 0.001), TRANSIT);
    expectRelative(scatter!.ppm, 1069.0449676496, 1e-9, "ppm");
    expectRelative(scatter!.mmag, 1.1607008258917, 1e-9, "mmag");
    expectRelative(scatter!.meanFormalPpm, 1000, 1e-9, "meanFormalPpm");
    expectRelative(scatter!.ratio, 1.0690449676496, 1e-9, "ratio");
    expect(scatter!.n).toEqual(8);
  });

  it("outOfTransitScatter has no formal error without errors and no result below two points", () => {
    const scatter = outOfTransitScatter(T10, flux, flux.map(() => null), TRANSIT);
    expectRelative(scatter!.ppm, 1069.0449676496, 1e-9, "ppm");
    expect(scatter!.meanFormalPpm).toBeNull();
    expect(scatter!.ratio).toBeNull();
    const zeroError = outOfTransitScatter(T10, flux, [0, ...flux.slice(1).map(() => 0.001)], TRANSIT);
    expect(zeroError!.meanFormalPpm).toBeNull();
    expect(zeroError!.ratio).toBeNull();
    expect(outOfTransitScatter([0], [1], [1], null)).toBeNull();
  });
});

describe("boxDepth", () => {
  it("boxDepth gives the depth with the scatter and the formal error", () => {
    const box = boxDepth(T10, FB, ERR_02, TRANSIT);
    expect(box!.fIn).toBeCloseTo(5, 12);
    expect(box!.fOut).toBeCloseTo(10, 12);
    expect(box!.depth).toBeCloseTo(0.5, 12);
    expect(box!.errScatter).toBeCloseTo(0.010177004891982, 12);
    expect(box!.errFormal).toBeCloseTo(0.014577379737113, 12);
    expect(box!.err).toBe(box!.errFormal);
    expect(box!.nIn).toEqual(2);
    expect(box!.nOut).toEqual(8);
  });

  it("boxDepth uses only the baseline windows when they are given", () => {
    const box = boxDepth(T10, FB, ERR_02, TRANSIT, WINGS);
    expect(box!.depth).toBeCloseTo(0.5, 12);
    expect(box!.nOut).toEqual(4);
    expect(box!.errScatter).toBeCloseTo(0.010408329997331, 12);
    expect(box!.errFormal).toBeCloseTo(0.015, 12);
  });

  it("boxDepth keeps the estimate that exists when the other cannot be made", () => {
    const onePoint: TimeWindow = { start: 4, end: 4 };
    const formalOnly = boxDepth(T10, FB, ERR_02, onePoint);
    expect(formalOnly!.nIn).toEqual(1);
    expect(formalOnly!.depth).toBeCloseTo(0.4593639575971732, 12);
    expect(formalOnly!.errScatter).toBeNull();
    expect(formalOnly!.errFormal).toBeCloseTo(0.021542935209213, 12);
    expect(formalOnly!.err).toBe(formalOnly!.errFormal);
    const neither = boxDepth(T10, FB, NO_ERR, onePoint);
    expect(neither!.errScatter).toBeNull();
    expect(neither!.errFormal).toBeNull();
    expect(neither!.err).toBeNull();
    const scatterOnly = boxDepth(T10, FB, [null, ...ERR_02.slice(1)], TRANSIT);
    expect(scatterOnly!.errFormal).toBeNull();
    expect(scatterOnly!.errScatter).toBeCloseTo(0.010177004891982, 12);
    expect(scatterOnly!.err).toBe(scatterOnly!.errScatter);
    const zeroErrors = boxDepth(T10, FB, FB.map(() => 0), TRANSIT);
    expect(zeroErrors!.errFormal).toEqual(0);
    expect(zeroErrors!.err).toBe(zeroErrors!.errScatter);
  });

  it("boxDepth returns null without points on one side and stays finite when the in-transit mean is zero", () => {
    expect(boxDepth(T10, FB, ERR_02, { start: 20, end: 21 })).toBeNull();
    expect(boxDepth(T10, FB, ERR_02, { start: 0, end: 9 })).toBeNull();
    const total = boxDepth(T10, [10.1, 9.9, 10.1, 9.9, 0.1, -0.1, 10.1, 9.9, 10.1, 9.9], ERR_02, TRANSIT);
    expect(total!.depth).toBeCloseTo(1, 12);
    expect(total!.errScatter).toBeCloseTo(0.01, 12);
    expect(total!.errFormal).toBeCloseTo(0.014142135623731, 12);
  });
});

describe("foldPhase and purity", () => {
  it("foldPhase maps every time to a phase in [0, 1)", () => {
    expectClose(foldPhase([10, 10.5, 11.25, 9.75, 7], 10, 1), [0, 0.5, 0.25, 0.75, 0]);
    expectClose(foldPhase([0, 1, 2.5, 6], 1, 2.5), [0.6, 0, 0.6, 0]);
    for (const phase of [...foldPhase([10 - 1e-15], 10, 1), ...foldPhase([-1e-17], 0, 1)]) {
      expect(phase).toBeGreaterThanOrEqual(0);
      expect(phase).toBeLessThan(1);
    }
    expect(foldPhase([1, NaN], 0, 2)[1]).toBeNaN();
    expect(foldPhase([1, 2], 0, 0)).toEqual([NaN, NaN]);
  });

  it("the functions do not change their inputs", () => {
    const t = Object.freeze([...T10]);
    const flux = Object.freeze([...FB]);
    const err = Object.freeze([...ERR_02]);
    const window = Object.freeze({ ...TRANSIT });
    const line: Baseline = { kind: "linear", level: 10, slope: 0, t0: 4.5, nOut: 8 };
    expect(() => relativeFlux(flux, err)).not.toThrow();
    expect(() => fitBaseline(t, flux, window, "median")).not.toThrow();
    expect(() => normalise(t, flux, err, line)).not.toThrow();
    expect(() => binInTime(t, flux, err, { points: 3 })).not.toThrow();
    expect(() => sigmaClip(t, flux, 3, 5)).not.toThrow();
    expect(() => outOfTransitScatter(t, flux, err, window)).not.toThrow();
    expect(() => boxDepth(t, flux, err, window)).not.toThrow();
    expect(() => foldPhase(t, 0, 2)).not.toThrow();
    expect(t).toEqual(T10);
    expect(flux).toEqual(FB);
    expect(err).toEqual(ERR_02);
  });
});

describe("the committed white light of WD 1856+534 b (JWST NIRSpec PRISM, level 3)", () => {
  const t = FIXTURE.bmjd_tdb;

  it("the white light of WD 1856+534 b gives the depth and both errors of the quick-look golden", () => {
    for (const column of [FIXTURE.int_num, FIXTURE.bmjd_tdb, FIXTURE.mjd_utc, FIXTURE.flux, FIXTURE.flux_err]) {
      expect(column).toHaveLength(240);
    }
    const box = boxDepth(t, FIXTURE.flux, FIXTURE.flux_err, e1Window(t), e1Baseline(t));
    expectRelative(box!.depth, GOLDEN_DEPTH, 1e-10, "depth");
    expectRelative(box!.errScatter, GOLDEN_ERR_SCATTER, 1e-10, "errScatter");
    expectRelative(box!.errFormal, 0.00027501456081958594, 1e-10, "errFormal");
    expect(box!.err).toBe(box!.errScatter);
    expect(box!.nIn).toEqual(6);
    expect(box!.nOut).toEqual(210);
    expectRelative(box!.fIn, 0.008133413871789402, 1e-10, "fIn");
    expectRelative(box!.fOut, 0.1639531116709505, 1e-10, "fOut");
  });

  it("the same curve gives the out-of-transit scatter and the baselines of the reference run", () => {
    const scatter = outOfTransitScatter(t, FIXTURE.flux, FIXTURE.flux_err, e1Window(t), e1Baseline(t));
    expectRelative(scatter!.ppm, 927.3509315525, 1e-9, "ppm");
    expectRelative(scatter!.mmag, 1.0068584809027, 1e-9, "mmag");
    expectRelative(scatter!.meanFormalPpm, 512.8350502783, 1e-9, "meanFormalPpm");
    expectRelative(scatter!.ratio, 1.808283055242, 1e-9, "ratio");
    expect(scatter!.n).toEqual(210);
    const mean = fitBaseline(t, FIXTURE.flux, e1Window(t), "mean", e1Baseline(t));
    expectRelative(mean!.level, 0.16395311167095059, 1e-9, "mean level");
    expectRelative(mean!.t0, 60061.48132169474, 1e-9, "t0");
    const median = fitBaseline(t, FIXTURE.flux, e1Window(t), "median", e1Baseline(t));
    expectRelative(median!.level, 0.1639706008070289, 1e-9, "median level");
    const line = fitBaseline(t, FIXTURE.flux, e1Window(t), "linear", e1Baseline(t));
    expectRelative(line!.slope, -0.0004124872721696969, 1e-9, "linear slope");
  });
});

describe.skipIf(!DATA_DIR)("the archived white light of WD 1856+534 b (ASTROBURST_SCIENCE_DATA_DIR)", () => {
  it("real_e1_whtlt_equals_the_committed_white_light_bit_for_bit", () => {
    const { names, rows } = readWhiteLightEcsv(DATA_DIR as string);
    expect(names).toEqual(["MJD_UTC_NRS1", "BJD_TDB_NRS1", "whitelight_flux_NRS1"]);
    expect(rows).toHaveLength(240);
    expect(bitDifferences(rows.map((row) => row[2]), FIXTURE.flux), "rows where whitelight_flux_NRS1 differs").toEqual([]);
    expect(bitDifferences(rows.map((row) => row[1]), FIXTURE.bmjd_tdb), "rows where BJD_TDB_NRS1 differs").toEqual([]);
    expect(bitDifferences(rows.map((row) => row[0]), FIXTURE.mjd_utc), "rows where MJD_UTC_NRS1 differs").toEqual([]);
  });

  it("real_e1_whtlt_box_depth_matches_the_quicklook_golden", () => {
    const { rows } = readWhiteLightEcsv(DATA_DIR as string);
    const t = rows.map((row) => row[1]);
    const flux = rows.map((row) => row[2]);
    const box = boxDepth(t, flux, flux.map(() => null), e1Window(t), e1Baseline(t));
    expectRelative(box!.depth, GOLDEN_DEPTH, 1e-10, "depth");
    expectRelative(box!.errScatter, GOLDEN_ERR_SCATTER, 1e-10, "errScatter");
    expect(box!.errFormal).toBeNull();
    expect(box!.nIn).toEqual(6);
    expect(box!.nOut).toEqual(210);
  });
});
