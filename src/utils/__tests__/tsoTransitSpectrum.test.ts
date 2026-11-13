import { describe, expect, it } from "vitest";
import { DEFAULT_LIGHT_CURVE_VIEW, type LightCurveDoc, type LightCurveViewState } from "../../shared/types/lightCurve";
import type { TransitFitResult, TransitFittedParameter, TransitParameterName } from "../../shared/types/transit";
import type { TsoTransitBin, TsoTransitFitRequest } from "../../shared/types/tsoTransit";
import type { DepthRow, DepthSpectrum } from "../tsoLightCurves";
import {
  BASELINE_NAMES,
  EMPTY_TRANSIT_DRAFT,
  FIT_COLOR,
  FIT_SERIES_LABEL,
  QUICK_SERIES_LABEL,
  TRANSIT_FIT_LABEL,
  draftFromWhiteLight,
  fitFromDraft,
  fitPlotSeries,
  fitTableRows,
  fixedLine,
  isBlankDraft,
  leftOutRows,
  onDifferentScales,
  whiteLightValues,
  type TransitDraft,
} from "../tsoTransitSpectrum";

function bin(lo: number, hi: number, extra: Partial<TsoTransitBin> = {}): TsoTransitBin {
  return {
    band_um: [lo, hi],
    n_points: 240,
    n_in_transit: 18,
    weighted: true,
    rp: 0.1,
    rp_err: 0.001,
    depth: 0.01,
    depth_err: 0.0002,
    u1: 0.2,
    u1_err: null,
    u2: 0.25,
    u2_err: null,
    chi2: 250.5,
    dof: 237,
    rms: 0.0003,
    grazing: false,
    npixels_ratio: 1,
    pixel_loss: false,
    error: null,
    notes: [],
    warnings: [],
    ...extra,
  };
}

function failed(lo: number, hi: number, error: string): TsoTransitBin {
  return bin(lo, hi, { rp: null, rp_err: null, depth: null, depth_err: null, u1: null, u2: null, chi2: null, rms: null, error });
}

const E1_DRAFT: TransitDraft = {
  periodText: "1.4079405",
  t0Text: "60061.507668",
  aRsText: "336",
  bText: "7.16",
  rpText: "7.73",
  u1Text: "0.2",
  u2Text: "0.25",
  limbFree: false,
  baselineOrder: 1,
  scaleErrors: false,
  fromWhiteLight: false,
};

const FIT: Required<TsoTransitFitRequest> = {
  period_days: 1.4079405,
  t0: 60061.507668,
  a_rs: 336,
  b: 7.16,
  rp_start: 7.73,
  u1: 0.2,
  u2: 0.25,
  limb_darkening: "fixed",
  baseline_order: 1,
  exclude_rows: [],
  scale_errors: false,
};

function fitted(name: TransitParameterName, value: number, err: number | null, free: boolean): TransitFittedParameter {
  return { name, value, err, free, at_bound: false };
}

const R: TransitFitResult = {
  model: "Mandel and Agol 2002, quadratic limb darkening, circular orbit",
  kind: "transit",
  period: 1.4079405,
  parameters: [
    fitted("t0", 60061.507668411155, 0.00001, true),
    fitted("rp", 7.72564861863833, 0.02, true),
    fitted("a_rs", 336, null, false),
    fitted("b", 7.16, null, false),
    fitted("u1", 0.2, null, false),
    fitted("u2", 0.25, null, false),
  ],
  baseline: { order: 1, reference_time: 60061.48, coefficients: [1, 0], errors: [null, null] },
  converged: true,
  depth: 59.68,
  depth_err: 0.3,
  chi2: 0,
  dof: 236,
  reduced_chi2: 0,
  weighted: true,
  error_scale: 1,
  rms: 0,
  rms_ppm: 0,
  n_points: 240,
  n_in_transit: 18,
  iterations: 5,
  free_names: ["t0", "rp", "c0", "c1"],
  time: [],
  model_flux: [],
  baseline_flux: [],
  residual: [],
  notes: [],
  warnings: [],
  curve_time: [],
  curve_flux: [],
  curve_baseline: [],
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
  time_averaging: { cadence_days: 0, bin_points: [], n_bins: [], rms_ppm: [], expected_ppm: [], beta: 1 },
  elapsed_ms: 0,
};

function entry(name: TransitParameterName, patch: Partial<TransitFittedParameter>): TransitFitResult {
  return { ...R, parameters: R.parameters.map((parameter) => (parameter.name === name ? { ...parameter, ...patch } : parameter)) };
}

function withOrder(order: number): TransitFitResult {
  return { ...R, baseline: { ...R.baseline, order } };
}

function whiteDoc(patch: Partial<LightCurveDoc> = {}): LightCurveDoc {
  return {
    id: "white",
    label: "white light 0.6 to 5.3 um",
    quantity: "flux",
    unit: "Jy",
    timeScale: "bmjd_tdb",
    time: [60000.1, 60000.2, 60000.3, 60000.4, 60000.5, 60000.6],
    value: [10, 10, 10, 10, 10, 10],
    err: [0.5, 0.5, 0.5, 0.5, 0.5, 0.5],
    pointRef: [1, 2, 3, 4, 5, 6],
    flags: [[], [], ["pixel loss"], [], [], []],
    source: { kind: "tso", paths: ["series_x1dints.fits"], detector: "NRS1", sporder: null, band_um: [0.6, 5.3], mode: "pipeline" },
    notes: [],
    ...patch,
  };
}

const ROWS = [0, 2, 3, 4, 6, 7];
const BY_HAND: LightCurveViewState = { ...DEFAULT_LIGHT_CURVE_VIEW, excluded: [4, 1] };

function quickRow(band: [number, number], depth: number | null): DepthRow {
  return {
    docId: `bin:${band[0]}`,
    label: `${band[0]} to ${band[1]} um`,
    band,
    centre: (band[0] + band[1]) / 2,
    halfWidth: (band[1] - band[0]) / 2,
    depth,
    errScatter: null,
    errFormal: null,
    err: null,
    nIn: 18,
    nOut: 222,
    pixelRatio: null,
    pixelLoss: false,
  };
}

function quickLook(rows: DepthRow[]): DepthSpectrum {
  return { white: null, rows, window: null, marginDays: 0, notes: [], warnings: [] };
}

describe("tsoTransitSpectrum", () => {
  it("starts blank and knows a blank draft", () => {
    expect(EMPTY_TRANSIT_DRAFT).toEqual({
      periodText: "",
      t0Text: "",
      aRsText: "",
      bText: "",
      rpText: "",
      u1Text: "",
      u2Text: "",
      limbFree: false,
      baselineOrder: 1,
      scaleErrors: false,
      fromWhiteLight: false,
    });
    expect(Object.isFrozen(EMPTY_TRANSIT_DRAFT)).toBe(true);
    expect(isBlankDraft(EMPTY_TRANSIT_DRAFT)).toBe(true);
    expect(isBlankDraft({ ...EMPTY_TRANSIT_DRAFT, u2Text: " " })).toBe(true);
    expect(isBlankDraft({ ...EMPTY_TRANSIT_DRAFT, limbFree: true, baselineOrder: 2, scaleErrors: true, fromWhiteLight: true })).toBe(true);
    expect(isBlankDraft({ ...EMPTY_TRANSIT_DRAFT, bText: "0" })).toBe(false);
    expect(isBlankDraft({ ...EMPTY_TRANSIT_DRAFT, periodText: "1.4" })).toBe(false);
    expect(isBlankDraft({ ...EMPTY_TRANSIT_DRAFT, u2Text: "0.25" })).toBe(false);
    expect(isBlankDraft(E1_DRAFT)).toBe(false);
  });

  it("takes the values of a white-light fit without rounding them, and refuses one it cannot use", () => {
    const values = whiteLightValues(R);
    expect(values).toEqual({
      periodDays: 1.4079405,
      t0: 60061.507668411155,
      aRs: 336,
      b: 7.16,
      rp: 7.72564861863833,
      u1: 0.2,
      u2: 0.25,
      baselineOrder: 1,
    });
    const previous: TransitDraft = { ...EMPTY_TRANSIT_DRAFT, limbFree: true, scaleErrors: true };
    expect(draftFromWhiteLight(values!, previous)).toEqual({
      periodText: "1.4079405",
      t0Text: "60061.507668411155",
      aRsText: "336",
      bText: "7.16",
      rpText: "7.72564861863833",
      u1Text: "0.2",
      u2Text: "0.25",
      limbFree: true,
      baselineOrder: 1,
      scaleErrors: true,
      fromWhiteLight: true,
    });
    expect(previous.fromWhiteLight).toBe(false);
    expect(draftFromWhiteLight(values!, E1_DRAFT)).toMatchObject({ limbFree: false, scaleErrors: false, fromWhiteLight: true });

    expect(whiteLightValues(withOrder(2))!.baselineOrder).toBe(2);
    expect(whiteLightValues(withOrder(0))!.baselineOrder).toBe(0);
    expect(whiteLightValues(withOrder(3))!.baselineOrder).toBe(2);
    expect(whiteLightValues(withOrder(-1))!.baselineOrder).toBe(0);
    expect(draftFromWhiteLight(whiteLightValues(withOrder(2))!, EMPTY_TRANSIT_DRAFT).baselineOrder).toBe(2);

    expect(whiteLightValues(null)).toBeNull();
    expect(whiteLightValues({ ...R, kind: "eclipse" })).toBeNull();
    expect(whiteLightValues({ ...R, parameters: R.parameters.filter((parameter) => parameter.name !== "rp") })).toBeNull();
    expect(whiteLightValues(entry("rp", { value: NaN }))).toBeNull();
    expect(whiteLightValues(entry("u2", { value: Infinity }))).toBeNull();
    expect(whiteLightValues({ ...R, period: 0 })).toBeNull();
    expect(whiteLightValues({ ...R, period: -1.4079405 })).toBeNull();
    expect(whiteLightValues({ ...R, period: NaN })).toBeNull();
    expect(whiteLightValues({ ...R, converged: false })).toBeNull();
    expect(whiteLightValues(entry("t0", { free: true, at_bound: true }))).toBeNull();
    expect(whiteLightValues(entry("rp", { free: true, at_bound: true }))).toBeNull();
    expect(whiteLightValues(entry("a_rs", { free: true, at_bound: true }))).toBeNull();
    expect(whiteLightValues(entry("b", { free: true, at_bound: true }))).toBeNull();

    expect(whiteLightValues(entry("u1", { free: true, at_bound: true }))).toEqual(values);
    expect(whiteLightValues(entry("b", { free: false, at_bound: true }))).toEqual(values);
    expect(whiteLightValues(entry("t0", { free: true, at_bound: false }))).toEqual(values);
  });

  it("reads a draft into the request of the command", () => {
    expect(fitFromDraft(E1_DRAFT, [])).toEqual({
      fit: {
        period_days: 1.4079405,
        t0: 60061.507668,
        a_rs: 336,
        b: 7.16,
        rp_start: 7.73,
        u1: 0.2,
        u2: 0.25,
        limb_darkening: "fixed",
        baseline_order: 1,
        exclude_rows: [],
        scale_errors: false,
      },
      error: null,
    });
    expect(fitFromDraft(E1_DRAFT, [])).toEqual({ fit: FIT, error: null });

    const asked = fitFromDraft({ ...E1_DRAFT, limbFree: true, baselineOrder: 2, scaleErrors: true }, []);
    expect(asked.error).toBeNull();
    expect(asked.fit).toEqual({ ...FIT, limb_darkening: "free", baseline_order: 2, scale_errors: true });

    const leftOut = [7, 3, 7, -1, 2.5];
    expect(fitFromDraft(E1_DRAFT, leftOut).fit!.exclude_rows).toEqual([3, 7]);
    expect(leftOut).toEqual([7, 3, 7, -1, 2.5]);
    expect(fitFromDraft(E1_DRAFT, [10, 9, 100, 0, NaN]).fit!.exclude_rows).toEqual([0, 9, 10, 100]);

    const fromWhiteLight = fitFromDraft(draftFromWhiteLight(whiteLightValues(R)!, EMPTY_TRANSIT_DRAFT), []);
    expect(fromWhiteLight.fit).toEqual({ ...FIT, t0: 60061.507668411155, rp_start: 7.72564861863833 });
  });

  it("names the first thing that is wrong in the orbit", () => {
    const errorOf = (patch: Partial<TransitDraft>) => {
      const input = fitFromDraft({ ...E1_DRAFT, ...patch }, []);
      expect(input.fit).toBeNull();
      return input.error;
    };
    const period = "the period must be a number of days above 0";
    const t0 = "t0 must be a number: a time in the scale of the series";
    const aRs = "a/Rs must be a number above 1";
    const bRange = "b must be a number of 0 or above";
    const bOrbit = "b must be below a/Rs";
    const rp = "Rp/Rs must be a number above 0";
    const limb = "u1 and u2 must be numbers";

    expect(fitFromDraft(EMPTY_TRANSIT_DRAFT, [])).toEqual({ fit: null, error: period });
    expect(errorOf({ periodText: "0" })).toBe(period);
    expect(errorOf({ periodText: "-1.4" })).toBe(period);
    expect(errorOf({ periodText: "two days" })).toBe(period);
    expect(errorOf({ t0Text: "x" })).toBe(t0);
    expect(errorOf({ t0Text: "" })).toBe(t0);
    expect(errorOf({ aRsText: "-1" })).toBe(aRs);
    expect(errorOf({ aRsText: "1" })).toBe(aRs);
    expect(errorOf({ aRsText: "" })).toBe(aRs);
    expect(errorOf({ bText: "-0.1" })).toBe(bRange);
    expect(errorOf({ bText: "" })).toBe(bRange);
    expect(errorOf({ bText: "400" })).toBe(bOrbit);
    expect(errorOf({ bText: "336" })).toBe(bOrbit);
    expect(errorOf({ rpText: "" })).toBe(rp);
    expect(errorOf({ rpText: "0" })).toBe(rp);
    expect(errorOf({ rpText: "-7.73" })).toBe(rp);
    expect(errorOf({ u1Text: "0,2" })).toBe(limb);
    expect(errorOf({ u2Text: "" })).toBe(limb);

    expect(errorOf({ periodText: "0", t0Text: "x", aRsText: "1" })).toBe(period);
    expect(errorOf({ t0Text: "x", aRsText: "1", bText: "-1" })).toBe(t0);
    expect(errorOf({ aRsText: "1", bText: "-1", rpText: "" })).toBe(aRs);
    expect(errorOf({ bText: "-1", rpText: "", u1Text: "" })).toBe(bRange);
    expect(errorOf({ bText: "400", rpText: "", u1Text: "" })).toBe(bOrbit);
    expect(errorOf({ rpText: "", u1Text: "" })).toBe(rp);

    const central = fitFromDraft({ ...E1_DRAFT, bText: "0" }, []);
    expect(central.error).toBeNull();
    expect(central.fit).toEqual({ ...FIT, b: 0 });
    const beforeTheEpoch = fitFromDraft({ ...E1_DRAFT, t0Text: " -0.25 ", u1Text: "0", u2Text: "-0.1" }, []);
    expect(beforeTheEpoch.fit).toEqual({ ...FIT, t0: -0.25, u1: 0, u2: -0.1 });
  });

  it("lists the integrations the view leaves out, as positions in the series", () => {
    const white = whiteDoc();
    expect(leftOutRows(white, BY_HAND, ROWS)).toEqual([2, 6]);

    const saturated = whiteDoc({ flags: [["saturated"], [], ["pixel loss"], [], [], []] });
    expect(leftOutRows(saturated, BY_HAND, ROWS)).toEqual([0, 2, 6]);
    expect(leftOutRows(saturated, { ...BY_HAND, excludeFlagged: false }, ROWS)).toEqual([2, 6]);

    expect(leftOutRows(white, DEFAULT_LIGHT_CURVE_VIEW, ROWS)).toEqual([]);
    expect(leftOutRows(null, BY_HAND, ROWS)).toEqual([]);
    expect(leftOutRows(white, BY_HAND, [0, 2])).toEqual([2]);
    expect(leftOutRows(white, BY_HAND, [...ROWS, 8, 9])).toEqual([2, 6]);
    expect(leftOutRows(white, BY_HAND, [])).toEqual([]);

    const noFlux = whiteDoc({ value: [10, 10, 10, null, 10, 10] });
    expect(leftOutRows(noFlux, { ...DEFAULT_LIGHT_CURVE_VIEW, excluded: [3, 5] }, ROWS)).toEqual([7]);

    const spike = whiteDoc({
      time: [60000.1, 60000.2, 60000.3, 60000.4, 60000.5, 60000.6, 60000.7],
      value: [10, 10.1, 9.9, 10, 30, 10.1, 9.9],
      err: [0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5],
      pointRef: [1, 2, 3, 4, 5, 6, 7],
      flags: [[], [], [], [], [], [], []],
    });
    const clipping: LightCurveViewState = { ...DEFAULT_LIGHT_CURVE_VIEW, clipSigma: 5, clipWindowPoints: 3 };
    expect(leftOutRows(spike, clipping, [0, 2, 3, 4, 6, 7, 9])).toEqual([6]);
    expect(leftOutRows(spike, { ...clipping, excluded: [0] }, [0, 2, 3, 4, 6, 7, 9])).toEqual([0, 6]);
    expect(leftOutRows(spike, DEFAULT_LIGHT_CURVE_VIEW, [0, 2, 3, 4, 6, 7, 9])).toEqual([]);

    expect(white).toEqual(whiteDoc());
    expect(ROWS).toEqual([0, 2, 3, 4, 6, 7]);
  });

  it("draws the fitted depths against the bin centres in wavelength order", () => {
    const bins = [bin(3, 4, { depth: 64.34, depth_err: 0.21 }), bin(0.6, 1, { depth: 59.29, depth_err: null }), failed(1, 2, "x")];
    expect(fitPlotSeries({ bins })).toEqual({
      x: [0.8, 3.5],
      y: [59.29, 64.34],
      yErr: [null, 0.21],
      color: "#67e8f9",
      label: "model fit, (Rp/Rs)^2",
      mode: "points",
    });
    expect(bins.map((b) => b.band_um[0])).toEqual([3, 0.6, 1]);
    expect(fitPlotSeries({ bins: [failed(1, 2, "x"), failed(2, 3, "y")] })).toBeNull();
    expect(fitPlotSeries({ bins: [] })).toBeNull();
    expect(fitPlotSeries({ bins: [bin(1, 2)] })).toEqual({
      x: [1.5],
      y: [0.01],
      yErr: [0.0002],
      color: FIT_COLOR,
      label: FIT_SERIES_LABEL,
      mode: "points",
    });
  });

  it("words the rows of the fit table beside the quick look", () => {
    const bins = [
      bin(1, 2, {
        rp: 7.9615132223691125,
        rp_err: 0.018147660280474452,
        depth: 63.38569278995821,
        depth_err: 0.2889656745561202,
        chi2: 1954234.6936146035,
        grazing: true,
        pixel_loss: true,
      }),
      bin(0.6, 1, {
        rp: 7.699960796108718,
        rp_err: 0.020304906635130987,
        depth: 59.28939626161121,
        depth_err: 0.3126939701183128,
        chi2: 468409.872724232,
        grazing: true,
        warnings: ["w"],
      }),
      failed(2, 3, "no integration has a flux in this band"),
    ];
    const quick = quickLook([quickRow([0.6, 1], 0.9673703022163279), quickRow([2, 3], null)]);
    const rows = fitTableRows({ bins }, quick);
    expect(rows).toEqual([
      {
        key: "fit:1",
        band: "0.6 to 1 um",
        depth: "59.289",
        depthErr: "0.31",
        rp: "7.7",
        rpErr: "0.02",
        chi2: "468410 / 237",
        points: "18 / 240",
        quick: "0.9674",
        remark: "grazing, fit warnings",
        warnings: ["w"],
        failed: false,
        flagged: true,
      },
      {
        key: "fit:0",
        band: "1 to 2 um",
        depth: "63.386",
        depthErr: "0.29",
        rp: "7.9615",
        rpErr: "0.018",
        chi2: "1954235 / 237",
        points: "18 / 240",
        quick: "--",
        remark: "grazing, pixel loss",
        warnings: [],
        failed: false,
        flagged: true,
      },
      {
        key: "fit:2",
        band: "2 to 3 um",
        depth: "--",
        depthErr: "--",
        rp: "--",
        rpErr: "--",
        chi2: "--",
        points: "18 / 240",
        quick: "--",
        remark: "no integration has a flux in this band",
        warnings: [],
        failed: true,
        flagged: false,
      },
    ]);
    expect(bins.map((b) => b.band_um[0])).toEqual([1, 0.6, 2]);
    expect(rows[0].warnings).not.toBe(bins[1].warnings);

    expect(fitTableRows({ bins }, null).map((row) => row.quick)).toEqual(["--", "--", "--"]);
    expect(fitTableRows({ bins }, quickLook([quickRow([0.6, 1.5], 0.5), quickRow([0.5, 1], 0.5)])).map((row) => row.quick)).toEqual([
      "--",
      "--",
      "--",
    ]);

    const [warned] = fitTableRows({ bins: [bin(1, 2, { warnings: ["w"] })] }, null);
    expect(warned.remark).toBe("fit warnings");
    expect(warned.flagged).toBe(true);
    expect(warned.failed).toBe(false);

    const [plain] = fitTableRows(
      { bins: [bin(1, 2, { depth_err: null, rp_err: null, chi2: 250.537, dof: -3 })] },
      quickLook([quickRow([1, 2], 0.0123456)]),
    );
    expect(plain).toEqual({
      key: "fit:0",
      band: "1 to 2 um",
      depth: "0.01",
      depthErr: "--",
      rp: "0.1",
      rpErr: "--",
      chi2: "250.5 / -3",
      points: "18 / 240",
      quick: "0.01235",
      remark: "",
      warnings: [],
      failed: false,
      flagged: false,
    });

    const [every] = fitTableRows({ bins: [bin(1, 2, { grazing: true, pixel_loss: true, warnings: ["a", "b"] })] }, null);
    expect(every.remark).toBe("grazing, pixel loss, fit warnings");
    expect(every.warnings).toEqual(["a", "b"]);
    const [lost] = fitTableRows({ bins: [bin(1, 2, { pixel_loss: true })] }, null);
    expect(lost.remark).toBe("pixel loss");
    expect(lost.flagged).toBe(true);
    const [grazing] = fitTableRows({ bins: [bin(1, 2, { grazing: true })] }, null);
    expect(grazing.remark).toBe("grazing");
    expect(grazing.flagged).toBe(true);

    expect(fitTableRows({ bins: [bin(1, 3), bin(1.5, 2.5), bin(0.5, 1)] }, null).map((row) => row.key)).toEqual(["fit:2", "fit:0", "fit:1"]);
    expect(fitTableRows({ bins: [] }, quick)).toEqual([]);
  });

  it("says what was fixed in every bin", () => {
    expect(fixedLine({ fit: FIT, time_scale: "bmjd_tdb" })).toBe(
      "Fixed in every bin: period 1.4079405 d, t0 60061.507668 (BMJD (TDB)), a/Rs 336, b 7.16, u1 0.2, u2 0.25; linear baseline",
    );
    expect(
      fixedLine({
        fit: { ...FIT, limb_darkening: "free", baseline_order: 0, exclude_rows: [3, 7], scale_errors: true },
        time_scale: "mjd_utc",
      }),
    ).toBe(
      "Fixed in every bin: period 1.4079405 d, t0 60061.507668 (MJD (UTC)), a/Rs 336, b 7.16; u1 and u2 fitted in every bin; constant baseline; 2 integration(s) left out; errors scaled by sqrt(chi2 / dof)",
    );
    expect(fixedLine({ fit: { ...FIT, t0: 60061.507668411155, baseline_order: 2, exclude_rows: [5] }, time_scale: "bmjd_tdb" })).toBe(
      "Fixed in every bin: period 1.4079405 d, t0 60061.507668411155 (BMJD (TDB)), a/Rs 336, b 7.16, u1 0.2, u2 0.25; quadratic baseline; 1 integration(s) left out",
    );
    expect(fixedLine({ fit: { ...FIT, scale_errors: true }, time_scale: "bmjd_tdb" })).toBe(
      "Fixed in every bin: period 1.4079405 d, t0 60061.507668 (BMJD (TDB)), a/Rs 336, b 7.16, u1 0.2, u2 0.25; linear baseline; errors scaled by sqrt(chi2 / dof)",
    );
    expect(BASELINE_NAMES).toEqual(["constant", "linear", "quadratic"]);
  });

  it("knows when the two series cannot share a scale", () => {
    expect(onDifferentScales({ bins: [bin(1, 2), bin(2, 3, { grazing: true })] })).toBe(true);
    expect(onDifferentScales({ bins: [bin(1, 2), bin(2, 3, { pixel_loss: true, warnings: ["w"] })] })).toBe(false);
    expect(onDifferentScales({ bins: [] })).toBe(false);

    for (const phrase of [
      "Mandel and Agol 2002",
      "quadratic limb darkening",
      "no orbital eccentricity",
      "no limb darkening from stellar models",
      "no systematics model",
      "no MCMC",
    ]) {
      expect(TRANSIT_FIT_LABEL).toContain(phrase);
    }
    expect(TRANSIT_FIT_LABEL).toBe(
      "transit model fit per bin: Mandel and Agol 2002, quadratic limb darkening, baseline polynomial; no orbital eccentricity, no limb darkening from stellar models, no systematics model, no MCMC",
    );
    expect(QUICK_SERIES_LABEL).toBe("quick look, 1 - F_in / F_out");
    expect(FIT_SERIES_LABEL).toBe("model fit, (Rp/Rs)^2");
    expect(FIT_COLOR).toBe("#67e8f9");
  });
});
