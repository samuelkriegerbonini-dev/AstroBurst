import { describe, expect, it } from "vitest";
import {
  DEFAULT_LIGHT_CURVE_VIEW,
  type LightCurveDoc,
  type LightCurveSource,
  type LightCurveViewState,
} from "../../shared/types/lightCurve";
import type { TransitFitResult, TransitFittedParameter, TransitParameterName } from "../../shared/types/transit";
import { lightCurveModel, type LightCurveModel } from "../lightCurveViewModel";
import {
  DEFAULT_TRANSIT_FIT_SETUP,
  fittedError,
  fittedValue,
  setupWithPeriod,
  setupWithStart,
  transitDraftLabel,
  transitDraftNames,
  transitFitInput,
  transitFitRequest,
  transitFitSignature,
  transitGeometryOf,
  transitStartFromQuickLook,
  type TransitDraftName,
  type TransitFitSetup,
  type TransitStart,
} from "../transitFitSetup";

const V = DEFAULT_LIGHT_CURVE_VIEW;
const T = Array.from({ length: 10 }, (_, i) => 2460310 + i);
const FB = [10.1, 9.9, 10.1, 9.9, 5.1, 4.9, 10.1, 9.9, 10.1, 9.9];
const TABLE_SOURCE: LightCurveSource = { kind: "table", path: "p", time_column: "t", value_column: "f" };
const OFFSET = 2460310;
const MAG_TIME = [2460310.5, 2460310.6, 2460310.7, 2460310.8];
const MAG_VALUE = [1.505149978319906, 1.505149978319906, 2.2577249674798594, 1.505149978319906];
const MAG_ERR = 0.013297055068698485;
const MAG_FLUX = [0.25, 0.25, 0.125, 0.25];
const MAG_FLUX_ERR = [0.003061862178478972, 0.003061862178478972, 0.001530931089239486, 0.003061862178478972];

const SETUP: TransitFitSetup = {
  ...DEFAULT_TRANSIT_FIT_SETUP,
  period: "3",
  t0: { text: "0.5", free: true },
  rp: { text: "0.1", free: true },
  a_rs: { text: "9", free: false },
  b: { text: "0.3", free: false },
  u1: { text: "0.4", free: false },
  u2: { text: "0.25", free: false },
};

const START: TransitStart = { t0: 2460310.4999394338, rp: 0.099820347, a_rs: 8.99742612, b: 0.296997238, flux_ratio: 0.0019958 };

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

function series(time: number[], value: (number | null)[], err: (number | null)[], over: Partial<LightCurveDoc> = {}): Partial<LightCurveDoc> {
  return { time, value, err, pointRef: time.map((_, i) => i), flags: time.map(() => []), ...over };
}

function model(over: Partial<LightCurveDoc> = {}, view: Partial<LightCurveViewState> = {}): LightCurveModel {
  return lightCurveModel(doc(over), { ...V, ...view });
}

function magnitudes(over: Partial<LightCurveDoc> = {}): Partial<LightCurveDoc> {
  return series(MAG_TIME, MAG_VALUE, MAG_TIME.map(() => MAG_ERR), {
    quantity: "diff_mag",
    unit: "mag",
    magErrFactor: 1.0857,
    source: { kind: "frames", reference: "ref.fits", target: "T", comps: ["C1", "C2"] },
    ...over,
  });
}

function fourPoints(): Partial<LightCurveDoc> {
  return series(T.slice(0, 4), FB.slice(0, 4), [0.2, 0.2, 0.2, 0.2]);
}

function frozenDoc(over: Partial<LightCurveDoc> = {}): Partial<LightCurveDoc> {
  const base = doc(over);
  return {
    ...over,
    time: Object.freeze(base.time) as number[],
    value: Object.freeze(base.value) as (number | null)[],
    err: Object.freeze(base.err) as (number | null)[],
    pointRef: Object.freeze(base.pointRef) as number[],
    flags: Object.freeze(base.flags.map((words) => Object.freeze(words) as string[])) as string[][],
  };
}

function frozenSetup(setup: TransitFitSetup): TransitFitSetup {
  const names: TransitDraftName[] = ["t0", "rp", "a_rs", "b", "u1", "u2", "flux_ratio"];
  const drafts = Object.fromEntries(names.map((name) => [name, Object.freeze({ ...setup[name] })]));
  return Object.freeze({ ...setup, ...drafts });
}

function expectClose(actual: readonly number[] | null, expected: readonly number[]) {
  expect(actual).not.toBeNull();
  expect(actual).toHaveLength(expected.length);
  expected.forEach((value, i) => expect(actual![i], `element ${i}`).toBeCloseTo(value, 12));
}

function fitted(name: TransitParameterName, value: number, err: number | null): TransitFittedParameter {
  return { name, value, err, free: err !== null, at_bound: false };
}

const RESULT: TransitFitResult = {
  model: "quadratic limb darkening",
  period: 3,
  parameters: [
    fitted("t0", 2460310.5, 0.0001),
    fitted("rp", 0.1, 0.0005),
    fitted("a_rs", 9, 0.2),
    fitted("b", 0.3, 0.06),
    fitted("u1", 0.4, null),
    fitted("u2", 0.25, null),
  ],
  baseline: { order: 1, reference_time: 2460310.5, coefficients: [1, 0], errors: [0.0001, 0.0002] },
  depth: 0.01,
  depth_err: 0.0001,
  chi2: 10,
  dof: 4,
  reduced_chi2: 2.5,
  weighted: true,
  error_scale: 1,
  rms: 0.001,
  rms_ppm: 1000,
  n_points: 10,
  n_in_transit: 2,
  iterations: 5,
  converged: true,
  free_names: ["t0", "rp", "a_rs", "b"],
  time: [],
  model_flux: [],
  baseline_flux: [],
  residual: [],
  notes: [],
  warnings: [],
  correlation: null,
  derived: {
    inclination_deg: null,
    inclination_err: null,
    duration_t14_days: null,
    duration_t14_err: null,
    duration_t23_days: null,
    duration_t23_err: null,
    stellar_density_cgs: null,
    stellar_density_err: null,
  },
  time_averaging: { cadence_days: 1, bin_points: [], n_bins: [], rms_ppm: [], expected_ppm: [], beta: 1 },
  curve_time: [],
  curve_flux: [],
  curve_baseline: [],
  kind: "transit",
  elapsed_ms: 1,
};

describe("transit fit setup", () => {
  it("the fit input is the kept points with their flux and errors", () => {
    const input = transitFitInput(model({}, { excluded: [0, 4] }));
    expect(input.time).toEqual([2460311, 2460312, 2460313, 2460315, 2460316, 2460317, 2460318, 2460319]);
    expect(input.flux).toEqual([9.9, 10.1, 9.9, 4.9, 10.1, 9.9, 10.1, 9.9]);
    expect(input.err).toEqual([0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2]);
    expect(input.pointIndex).toEqual([1, 2, 3, 5, 6, 7, 8, 9]);

    const oneErrorMissing = FB.map((_, i) => (i === 3 ? null : 0.2));
    const withoutOneError = transitFitInput(model({ err: oneErrorMissing }, { excluded: [0, 4] }));
    expect(withoutOneError.err).toBeNull();
    expect(withoutOneError.flux).toEqual(input.flux);
    expect(transitFitInput(model({ err: FB.map((_, i) => (i === 3 ? 0 : 0.2)) })).err).toBeNull();
    expect(transitFitInput(model({ err: oneErrorMissing }, { excluded: [3] })).err).toEqual(FB.slice(1).map(() => 0.2));

    const withoutOneValue = transitFitInput(model({ value: FB.map((value, i) => (i === 2 ? null : value)) }));
    expect(withoutOneValue.pointIndex).toEqual([0, 1, 3, 4, 5, 6, 7, 8, 9]);
    expect(withoutOneValue.time).toEqual(T.filter((_, i) => i !== 2));
    expect(withoutOneValue.flux).toEqual(FB.filter((_, i) => i !== 2));
    expect(withoutOneValue.err).toHaveLength(9);

    const relative = transitFitInput(model({ quantity: "rel_flux", unit: null }));
    expect(relative.flux).toEqual(FB);
    expect(relative.err).toEqual(FB.map(() => 0.2));

    const keptByHand = model({ time: T.map((time, i) => (i === 1 ? NaN : time)) });
    expect(keptByHand.state[1]).toEqual("missing");
    expect(transitFitInput({ ...keptByHand, state: T.map(() => "kept" as const) }).pointIndex).toEqual([0, 2, 3, 4, 5, 6, 7, 8, 9]);

    const empty = transitFitInput(model(series([], [], [])));
    expect(empty).toEqual({ time: [], flux: [], err: null, pointIndex: [] });
  });

  it("a magnitude curve enters as relative flux", () => {
    const input = transitFitInput(model(magnitudes()));
    expectClose(input.flux, MAG_FLUX);
    expectClose(input.err, MAG_FLUX_ERR);
    expect(input.time).toEqual(MAG_TIME);
    expect(input.pointIndex).toEqual([0, 1, 2, 3]);

    const normalised = transitFitInput(
      model(magnitudes(), { display: "normalised", window: { start: 2460310.7, end: 2460310.7 } }),
    );
    expect(normalised).toEqual(input);

    const exactFactor = transitFitInput(model(magnitudes({ magErrFactor: undefined })));
    expect(exactFactor.err![0]).toBeCloseTo((0.25 * MAG_ERR * Math.LN10) / 2.5, 12);
    expect(exactFactor.err![0]).not.toBeCloseTo(MAG_FLUX_ERR[0], 9);

    const overflowing = transitFitInput(model(magnitudes({ value: [1.505149978319906, -1000, 2.2577249674798594, 1.505149978319906] })));
    expect(overflowing.pointIndex).toEqual([0, 2, 3]);
    expectClose(overflowing.flux, [0.25, 0.125, 0.25]);
  });

  it("the signature follows the points and the errors", () => {
    const all = transitFitInput(model());
    const excluded = transitFitInput(model({}, { excluded: [0, 4] }));
    const noErrors = transitFitInput(model({ err: FB.map(() => null) }));
    expect(transitFitSignature(all)).toEqual("10|2460310|2460319|90.0000000000|e");
    expect(transitFitSignature(excluded)).toEqual("8|2460311|2460319|74.8000000000|e");
    expect(transitFitSignature(transitFitInput(model()))).toEqual(transitFitSignature(all));
    expect(transitFitSignature(noErrors)).toEqual("10|2460310|2460319|90.0000000000|n");
    expect(transitFitSignature(transitFitInput(model(series([], [], []))))).toEqual("0|||0.00000000000|n");
    const otherFlux = transitFitInput(model({ value: FB.map((value, i) => (i === 5 ? 4.8 : value)) }));
    expect(transitFitSignature(otherFlux)).toEqual("10|2460310|2460319|89.9000000000|e");
  });

  it("the request carries the typed values, t0 in absolute time, and the fixed flags", () => {
    const input = transitFitInput(model());
    const { request, error } = transitFitRequest(input, SETUP, OFFSET);
    expect(error).toBeNull();
    expect(request).toEqual({
      kind: "transit",
      time: input.time,
      flux: input.flux,
      err: input.err,
      period: 3,
      t0: { value: 2460310.5, free: true },
      rp: { value: 0.1, free: true },
      a_rs: { value: 9, free: false },
      b: { value: 0.3, free: false },
      u1: { value: 0.4, free: false },
      u2: { value: 0.25, free: false },
      flux_ratio: null,
      baseline_order: 1,
      scale_errors: false,
    });
    expect(request!.time).toEqual(T);
    expect(request!.flux).toEqual(FB);
    expect(request!.err).toEqual(FB.map(() => 0.2));

    const other = transitFitRequest(
      transitFitInput(model({ err: FB.map(() => null) })),
      { ...SETUP, period: " 1.5 ", u1: { text: "0.4", free: true }, flux_ratio: { text: "", free: true }, baselineOrder: 2, scaleErrors: true },
      0,
    );
    expect(other.error).toBeNull();
    expect(other.request!.period).toEqual(1.5);
    expect(other.request!.t0).toEqual({ value: 0.5, free: true });
    expect(other.request!.u1).toEqual({ value: 0.4, free: true });
    expect(other.request!.flux_ratio).toBeNull();
    expect(other.request!.err).toBeNull();
    expect(other.request!.baseline_order).toEqual(2);
    expect(other.request!.scale_errors).toEqual(true);
  });

  it("every input that can not be used is named, in order", () => {
    expect(transitDraftNames("transit")).toEqual(["t0", "rp", "a_rs", "b", "u1", "u2"]);
    expect(transitDraftNames("eclipse")).toEqual(["t0", "rp", "a_rs", "b", "flux_ratio"]);
    expect(transitDraftNames("transit")).toBe(transitDraftNames("transit"));
    expect(transitDraftNames("transit").map((name) => transitDraftLabel(name, "transit"))).toEqual([
      "mid-transit time",
      "Rp/Rs",
      "a/Rs",
      "impact parameter b",
      "limb darkening u1",
      "limb darkening u2",
    ]);
    expect(transitDraftNames("eclipse").map((name) => transitDraftLabel(name, "eclipse"))).toEqual([
      "mid-eclipse time",
      "Rp/Rs",
      "a/Rs",
      "impact parameter b",
      "planet/star flux ratio",
    ]);

    const input = transitFitInput(model());
    const refusal = (setup: TransitFitSetup, from = input) => {
      const outcome = transitFitRequest(from, setup, OFFSET);
      expect(outcome.request).toBeNull();
      return outcome.error;
    };
    expect(refusal({ ...SETUP, period: "" })).toEqual("type the period in days: it is given, never fitted");
    expect(refusal({ ...SETUP, period: "  " })).toEqual("type the period in days: it is given, never fitted");
    expect(refusal({ ...SETUP, period: "abc" })).toEqual("period must be a number of days above 0");
    expect(refusal({ ...SETUP, period: "0" })).toEqual("period must be a number of days above 0");
    expect(refusal({ ...SETUP, period: "-2" })).toEqual("period must be a number of days above 0");
    expect(refusal({ ...SETUP, t0: { text: "", free: true } })).toEqual("mid-transit time needs a value");
    expect(refusal({ ...SETUP, rp: { text: "x", free: true } })).toEqual("Rp/Rs is not a number");
    expect(refusal({ ...SETUP, a_rs: { text: " ", free: false } })).toEqual("a/Rs needs a value");
    expect(refusal({ ...SETUP, b: { text: "0,3", free: false } })).toEqual("impact parameter b is not a number");
    expect(refusal({ ...SETUP, u1: { text: "", free: false } })).toEqual("limb darkening u1 needs a value");
    expect(refusal({ ...SETUP, u2: { text: "", free: false } })).toEqual("limb darkening u2 needs a value");
    expect(refusal({ ...SETUP, period: "", t0: { text: "", free: true } })).toEqual(
      "type the period in days: it is given, never fitted",
    );
    expect(refusal({ ...SETUP, t0: { text: "", free: true }, rp: { text: "x", free: true } })).toEqual(
      "mid-transit time needs a value",
    );
    expect(refusal({ ...SETUP, rp: { text: "x", free: true }, u2: { text: "", free: false } })).toEqual("Rp/Rs is not a number");

    const four = transitFitInput(model(fourPoints()));
    expect(four.time).toHaveLength(4);
    expect(refusal(SETUP, four)).toEqual("4 points for 4 free parameters: the fit needs more points than free parameters");
    expect(refusal({ ...SETUP, baselineOrder: 2 }, four)).toEqual(
      "4 points for 5 free parameters: the fit needs more points than free parameters",
    );
    expect(refusal({ ...SETUP, baselineOrder: 0, u1: { text: "0.4", free: true }, u2: { text: "0.25", free: true } }, four)).toEqual(
      "4 points for 5 free parameters: the fit needs more points than free parameters",
    );
    expect(refusal({ ...SETUP, rp: { text: "x", free: true } }, four)).toEqual("Rp/Rs is not a number");
    const constantBaseline = transitFitRequest(four, { ...SETUP, baselineOrder: 0 }, OFFSET);
    expect(constantBaseline.error).toBeNull();
    expect(constantBaseline.request!.baseline_order).toEqual(0);
    expect(constantBaseline.request!.time).toEqual(T.slice(0, 4));

    const empty = transitFitInput(model(series([], [], [])));
    expect(refusal(SETUP, empty)).toEqual("no kept point to fit");
    expect(refusal({ ...SETUP, period: "" }, empty)).toEqual("no kept point to fit");
  });

  it("an eclipse request uses the flux ratio and no limb darkening", () => {
    const input = transitFitInput(model());
    const eclipse: TransitFitSetup = {
      ...SETUP,
      kind: "eclipse",
      flux_ratio: { text: "0.002", free: true },
      u1: { text: "", free: false },
    };
    const { request, error } = transitFitRequest(input, eclipse, OFFSET);
    expect(error).toBeNull();
    expect(request!.kind).toEqual("eclipse");
    expect(request!.flux_ratio).toEqual({ value: 0.002, free: true });
    expect(request!.u1).toEqual({ value: 0, free: false });
    expect(request!.u2).toEqual({ value: 0, free: false });
    expect(request!.t0).toEqual({ value: 2460310.5, free: true });
    expect(request!.rp).toEqual({ value: 0.1, free: true });
    expect(request!.a_rs).toEqual({ value: 9, free: false });
    expect(request!.b).toEqual({ value: 0.3, free: false });

    const blankTime = transitFitRequest(input, { ...eclipse, t0: { text: "", free: true } }, OFFSET);
    expect(blankTime).toEqual({ request: null, error: "mid-eclipse time needs a value" });
    const blankRatio = transitFitRequest(input, { ...eclipse, flux_ratio: { text: "", free: true } }, OFFSET);
    expect(blankRatio).toEqual({ request: null, error: "planet/star flux ratio needs a value" });
    const unreadRatio = transitFitRequest(input, { ...eclipse, flux_ratio: { text: "1e", free: true } }, OFFSET);
    expect(unreadRatio).toEqual({ request: null, error: "planet/star flux ratio is not a number" });

    const four = transitFitInput(model(fourPoints()));
    const freeLimbDarkening: TransitFitSetup = { ...eclipse, baselineOrder: 0, u1: { text: "", free: true }, u2: { text: "0.25", free: true } };
    expect(transitFitRequest(four, freeLimbDarkening, OFFSET).error).toEqual(
      "4 points for 4 free parameters: the fit needs more points than free parameters",
    );
    const fixedRatio = transitFitRequest(four, { ...freeLimbDarkening, flux_ratio: { text: "0.002", free: false } }, OFFSET);
    expect(fixedRatio.error).toBeNull();
    expect(fixedRatio.request!.u1).toEqual({ value: 0, free: false });
    expect(fixedRatio.request!.u2).toEqual({ value: 0, free: false });
    expect(fixedRatio.request!.flux_ratio).toEqual({ value: 0.002, free: false });
  });

  it("starting values come from the window, the box depth and the period", () => {
    const window = { start: 10, end: 10.112612441416733 };
    const start = transitStartFromQuickLook(window, 0.01, 3)!;
    expect(start.t0).toBeCloseTo(10.056306220708366, 12);
    expect(start.rp).toBeCloseTo(0.1, 12);
    expect(start.b).toBeCloseTo(0.3, 12);
    expect(start.a_rs).toBeCloseTo(9, 9);
    expect(start.flux_ratio).toBeCloseTo(0.010101010101010102, 12);

    for (const depth of [null, 0, 1, 1.2, NaN, -0.01]) {
      const fallback = transitStartFromQuickLook(window, depth, 3)!;
      expect(fallback.rp, `depth ${depth}`).toBeCloseTo(0.1, 12);
      expect(fallback.flux_ratio, `depth ${depth}`).toBeCloseTo(0.001, 12);
      expect(fallback.a_rs, `depth ${depth}`).toBeCloseTo(9, 9);
    }
    const deep = transitStartFromQuickLook(window, 0.6, 3)!;
    expect(deep.rp).toBeCloseTo(Math.sqrt(0.6), 12);
    expect(deep.flux_ratio).toBeCloseTo(0.001, 12);
    expect(transitStartFromQuickLook(window, 0.5, 3)!.flux_ratio).toBeCloseTo(0.001, 12);
    expect(transitStartFromQuickLook(window, 0.25, 3)!.flux_ratio).toBeCloseTo(1 / 3, 12);

    expect(transitStartFromQuickLook(null, 0.01, 3)).toBeNull();
    expect(transitStartFromQuickLook(window, 0.01, null)).toBeNull();
    expect(transitStartFromQuickLook(window, 0.01, 0)).toBeNull();
    expect(transitStartFromQuickLook(window, 0.01, NaN)).toBeNull();
    expect(transitStartFromQuickLook(window, 0.01, -3)).toBeNull();
    expect(transitStartFromQuickLook({ start: 10, end: 10 }, 0.01, 3)).toBeNull();
    expect(transitStartFromQuickLook({ start: 10.2, end: 10 }, 0.01, 3)).toBeNull();
    expect(transitStartFromQuickLook({ start: NaN, end: 10 }, 0.01, 3)).toBeNull();
    expect(transitStartFromQuickLook({ start: 10, end: 11.6 }, 0.01, 3)).toBeNull();
    expect(transitStartFromQuickLook({ start: 10, end: 11.5 }, 0.01, 3)).toBeNull();
    expect(transitStartFromQuickLook({ start: 10, end: 11.4 }, 0.01, 3)).not.toBeNull();

    const real = transitStartFromQuickLook({ start: 60061.50491134814, end: 60061.50881847316 }, 0.9503918297804987, 1.4079405)!;
    expect(real.t0).toBeCloseTo(60061.50686491065, 9);
    expect(real.rp).toBeCloseTo(0.974880418195226, 9);
    expect(real.a_rs).toBeCloseTo(223.90005538847618, 9);
    expect(real.b).toBeCloseTo(0.3, 12);
    expect(real.flux_ratio).toBeCloseTo(0.001, 12);
  });

  it("the starting values are written into the drafts in axis units", () => {
    expect(DEFAULT_TRANSIT_FIT_SETUP).toEqual({
      kind: "transit",
      period: "",
      t0: { text: "", free: true },
      rp: { text: "", free: true },
      a_rs: { text: "", free: true },
      b: { text: "", free: true },
      u1: { text: "0", free: false },
      u2: { text: "0", free: false },
      flux_ratio: { text: "0.001", free: true },
      baselineOrder: 1,
      scaleErrors: false,
    });
    expect(Object.isFrozen(DEFAULT_TRANSIT_FIT_SETUP)).toBe(true);
    for (const name of ["t0", "rp", "a_rs", "b", "u1", "u2", "flux_ratio"] as const) {
      expect(Object.isFrozen(DEFAULT_TRANSIT_FIT_SETUP[name]), name).toBe(true);
    }

    const before = JSON.stringify(SETUP);
    const transit = setupWithStart(SETUP, START, OFFSET);
    expect(transit.t0).toEqual({ text: "0.49993943", free: true });
    expect(transit.rp).toEqual({ text: "0.0998203", free: true });
    expect(transit.a_rs).toEqual({ text: "8.99743", free: false });
    expect(transit.b).toEqual({ text: "0.296997", free: false });
    expect(transit.flux_ratio).toEqual({ text: "0.001", free: true });
    expect(transit.period).toEqual("3");
    expect(transit.u1).toEqual({ text: "0.4", free: false });
    expect(transit.u2).toEqual({ text: "0.25", free: false });
    expect(transit.kind).toEqual("transit");
    expect(transit.baselineOrder).toEqual(1);
    expect(transit.scaleErrors).toEqual(false);
    expect(transit).not.toBe(SETUP);
    expect(JSON.stringify(SETUP)).toEqual(before);

    const eclipse = setupWithStart({ ...SETUP, kind: "eclipse", flux_ratio: { text: "0.001", free: false } }, START, OFFSET);
    expect(eclipse.flux_ratio).toEqual({ text: "0.001996", free: false });
    expect(eclipse.t0.text).toEqual("0.49993943");
    expect(eclipse.kind).toEqual("eclipse");
    expect(setupWithStart(SETUP, START, 0).t0.text).toEqual("2460310.49993943");

    const typed = setupWithPeriod(DEFAULT_TRANSIT_FIT_SETUP, 1.4079405);
    expect(typed.period).toEqual("1.4079405");
    expect(typed.t0).toBe(DEFAULT_TRANSIT_FIT_SETUP.t0);
    expect(DEFAULT_TRANSIT_FIT_SETUP.period).toEqual("");
    expect(setupWithPeriod({ ...SETUP, period: "  " }, 5).period).toEqual("5");
    expect(setupWithPeriod(SETUP, 5)).toBe(SETUP);
    expect(setupWithPeriod(DEFAULT_TRANSIT_FIT_SETUP, null)).toBe(DEFAULT_TRANSIT_FIT_SETUP);
    expect(setupWithPeriod(DEFAULT_TRANSIT_FIT_SETUP, 0)).toBe(DEFAULT_TRANSIT_FIT_SETUP);
    expect(setupWithPeriod(DEFAULT_TRANSIT_FIT_SETUP, NaN)).toBe(DEFAULT_TRANSIT_FIT_SETUP);
    expect(setupWithPeriod(DEFAULT_TRANSIT_FIT_SETUP, -1)).toBe(DEFAULT_TRANSIT_FIT_SETUP);
  });

  it("the geometry of a fit", () => {
    expect(fittedValue(RESULT, "rp")).toEqual(0.1);
    expect(fittedError(RESULT, "rp")).toEqual(0.0005);
    expect(fittedValue(RESULT, "u1")).toEqual(0.4);
    expect(fittedError(RESULT, "u1")).toBeNull();
    expect(fittedValue(RESULT, "flux_ratio")).toBeNull();
    expect(fittedError(RESULT, "flux_ratio")).toBeNull();
    expect(transitGeometryOf(RESULT)).toEqual({ period: 3, t0: 2460310.5, rp: 0.1, a_rs: 9, b: 0.3, u1: 0.4, u2: 0.25 });
    expect(transitGeometryOf(null)).toBeNull();
    expect(transitGeometryOf({ ...RESULT, kind: "eclipse" })).toBeNull();
    for (const missing of ["t0", "rp", "a_rs", "b", "u1", "u2"]) {
      const without = { ...RESULT, parameters: RESULT.parameters.filter((parameter) => parameter.name !== missing) };
      expect(transitGeometryOf(without), `without ${missing}`).toBeNull();
    }
  });

  it("the helpers do not change their arguments", () => {
    const setup = frozenSetup(SETUP);
    const kept = transitFitInput(model(frozenDoc(), { excluded: [0, 4] }));
    expect(kept.pointIndex).toEqual([1, 2, 3, 5, 6, 7, 8, 9]);
    expect(transitFitInput(model(frozenDoc(magnitudes()))).pointIndex).toEqual([0, 1, 2, 3]);

    const input = transitFitInput(model(frozenDoc()));
    const frozenInput = Object.freeze({
      time: Object.freeze(input.time) as number[],
      flux: Object.freeze(input.flux) as number[],
      err: Object.freeze(input.err) as number[],
      pointIndex: Object.freeze(input.pointIndex) as number[],
    });
    expect(transitFitSignature(frozenInput)).toEqual("10|2460310|2460319|90.0000000000|e");
    expect(transitFitRequest(frozenInput, setup, OFFSET).request!.t0).toEqual({ value: 2460310.5, free: true });
    expect(transitFitRequest(frozenInput, frozenSetup({ ...SETUP, kind: "eclipse" }), OFFSET).error).toBeNull();

    expect(setupWithStart(setup, Object.freeze({ ...START }), OFFSET).t0.text).toEqual("0.49993943");
    expect(setupWithStart(frozenSetup({ ...SETUP, kind: "eclipse" }), START, OFFSET).flux_ratio.text).toEqual("0.001996");
    expect(setupWithPeriod(frozenSetup({ ...SETUP, period: "" }), 2).period).toEqual("2");
    expect(setupWithPeriod(DEFAULT_TRANSIT_FIT_SETUP, 2).period).toEqual("2");
    expect(setup).toEqual(SETUP);
  });
});
