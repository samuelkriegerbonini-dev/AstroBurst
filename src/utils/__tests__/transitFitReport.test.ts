import { describe, expect, it } from "vitest";
import type { LightCurveDoc, LightCurveSource } from "../../shared/types/lightCurve";
import type { TransitFitResult, TransitFittedParameter } from "../../shared/types/transit";
import { APP_VERSION } from "../constants";
import { formatWithUncertainty as F } from "../formatUncertainty";
import { MEASUREMENT_KINDS, measurementLogCsv } from "../measurementLog";
import {
  ECLIPSE_FIT_LABEL,
  TRANSIT_FIT_LABEL,
  timeAveragingRows,
  transitFitCsvColumns,
  transitFitCsvText,
  transitFitEntry,
  transitFitLabel,
  transitFitLogKind,
  transitPlotSeries,
  transitResultRows,
  type TransitCsvContext,
} from "../transitFitReport";
import type { TransitFitInput } from "../transitFitSetup";
import { planLightCurveTable } from "../lightCurveTable";

const OFFSET = 2460310;

const FIXED_GEOMETRY: TransitFittedParameter[] = [
  { name: "a_rs", value: 9, err: null, free: false, at_bound: false },
  { name: "b", value: 0.3, err: null, free: false, at_bound: false },
];

const RESULT: TransitFitResult = {
  model: "Mandel and Agol 2002, quadratic limb darkening, circular orbit",
  period: 3,
  parameters: [
    { name: "t0", value: 2460310.4999394, err: 0.0000828, free: true, at_bound: false },
    { name: "rp", value: 0.09982, err: 0.00048, free: true, at_bound: false },
    ...FIXED_GEOMETRY,
    { name: "u1", value: 0.4, err: null, free: false, at_bound: false },
    { name: "u2", value: 0.25, err: null, free: false, at_bound: false },
  ],
  baseline: { order: 1, reference_time: 2460310.4995, coefficients: [999.98, 19.92], errors: [0.026, 0.29] },
  depth: 0.009964,
  depth_err: 0.000095,
  chi2: 195.3,
  dof: 234,
  reduced_chi2: 0.8346,
  weighted: true,
  error_scale: 1,
  rms: 0.27,
  rms_ppm: 270.7,
  n_points: 3,
  n_in_transit: 1,
  iterations: 12,
  converged: true,
  free_names: ["t0", "rp", "c0", "c1"],
  time: [2460310.4, 2460310.5, 2460310.6],
  model_flux: [998, 988, 1002],
  baseline_flux: [998, 1000, 1002],
  residual: [0.5, -1, 0.25],
  notes: ["note a"],
  warnings: ["warn b"],
  correlation: null,
  derived: {
    inclination_deg: 88.09,
    inclination_err: null,
    duration_t14_days: 0.1126,
    duration_t14_err: 0.0005,
    duration_t23_days: null,
    duration_t23_err: null,
    stellar_density_cgs: 1.53,
    stellar_density_err: null,
  },
  time_averaging: { cadence_days: 0.001, bin_points: [1, 2], n_bins: [240, 120], rms_ppm: [270.7, 190], expected_ppm: [271.3, 192.2], beta: 1 },
  curve_time: [2460310.4, 2460310.6],
  curve_flux: [998, 1002],
  curve_baseline: [998, 1002],
  kind: "transit",
  elapsed_ms: 3,
};

const ECL: TransitFitResult = {
  ...RESULT,
  kind: "eclipse",
  parameters: [
    RESULT.parameters[0],
    RESULT.parameters[1],
    ...FIXED_GEOMETRY,
    { name: "flux_ratio", value: 0.002, err: 0.0000067, free: true, at_bound: false },
  ],
  depth: 0.001996,
  depth_err: 0.0000067,
};

const INPUT: TransitFitInput = { time: [2460310.4, 2460310.5, 2460310.6], flux: [998.5, 987, 1002.25], err: [0.3, 0.3, 0.3], pointIndex: [0, 1, 2] };
const INPUT_WITHOUT_ERRORS: TransitFitInput = { ...INPUT, err: null };
const WITHOUT_SERIES: TransitFitResult = { ...RESULT, model_flux: [] };

const CONTEXT: TransitCsvContext = {
  fileName: "f0.fits",
  series: "target T",
  timeName: "BJD_TDB",
  timeScale: "bjd_tdb",
  fluxUnit: "Jy",
  exportedUtc: "2026-11-13T12:00:00.000Z",
};

const FRAMES_SOURCE: LightCurveSource = { kind: "frames", reference: "f0.fits", target: "T", comps: ["C1"] };

const HEADER_KEYS = [
  "software",
  "exported_utc",
  "file",
  "series",
  "fit",
  "model",
  "period_d",
  "t0",
  "t0_err",
  "t0_fit",
  "rp",
  "rp_err",
  "rp_fit",
  "a_rs",
  "a_rs_err",
  "a_rs_fit",
  "b",
  "b_err",
  "b_fit",
  "u1",
  "u1_err",
  "u1_fit",
  "u2",
  "u2_err",
  "u2_fit",
  "depth",
  "depth_err",
  "inclination_deg",
  "inclination_err",
  "duration_t14_d",
  "duration_t14_err",
  "duration_t23_d",
  "duration_t23_err",
  "stellar_density_g_cm3",
  "stellar_density_err",
  "baseline_order",
  "baseline_reference_time",
  "baseline_c0",
  "baseline_c0_err",
  "baseline_c1",
  "baseline_c1_err",
  "chi2",
  "dof",
  "reduced_chi2",
  "weighted",
  "error_scale",
  "rms",
  "rms_ppm",
  "beta",
  "n_points",
  "n_in_transit",
  "converged",
  "time",
  "note",
  "warning",
];

const UNIT_LINES = [
  "# unit bjd_tdb: d",
  "# unit flux: Jy",
  "# unit flux_err: Jy",
  "# unit baseline: Jy",
  "# unit model: Jy",
  "# unit residual: Jy",
  "# unit residual_ppm: ppm",
];

const HEADER_ROW = "bjd_tdb,flux,flux_err,baseline,model,residual,norm_flux,norm_model,residual_ppm";
const FIRST_ROW = ["2460310.4", "998.5", "0.3", "998", "998", "0.5", String(998.5 / 998), "1", String((1e6 * 0.5) / 998)].join(",");
const SECOND_ROW = "2460310.5,987,0.3,1000,988,-1,0.987,0.988,-1000";
const THIRD_ROW = ["2460310.6", "1002.25", "0.3", "1002", "1002", "0.25", String(1002.25 / 1002), "1", String((1e6 * 0.25) / 1002)].join(",");

function doc(source: LightCurveSource): LightCurveDoc {
  return {
    id: "target",
    label: "target T",
    quantity: "flux",
    unit: "Jy",
    timeScale: "bjd_tdb",
    time: [...INPUT.time],
    value: [...INPUT.flux],
    err: [0.3, 0.3, 0.3],
    pointRef: [0, 1, 2],
    flags: [[], [], []],
    source,
    notes: [],
  };
}

function withParameter(result: TransitFitResult, parameter: TransitFittedParameter): TransitFitResult {
  return { ...result, parameters: result.parameters.map((p) => (p.name === parameter.name ? parameter : p)) };
}

function expectClose(actual: readonly number[] | null | undefined, expected: readonly number[], digits: number): void {
  expect(actual).toHaveLength(expected.length);
  expected.forEach((value, i) => expect(actual?.[i]).toBeCloseTo(value, digits));
}

function deepFreeze<T>(value: T): T {
  if (typeof value === "object" && value !== null && !Object.isFrozen(value)) {
    Object.freeze(value);
    Object.values(value).forEach(deepFreeze);
  }
  return value;
}

function headerKeys(lines: readonly string[]): (string | undefined)[] {
  return lines.filter((line) => line.startsWith("# ") && !line.startsWith("# unit ")).map((line) => /^# ([^:]+):/.exec(line)?.[1]);
}

describe("transitFitReport", () => {
  it("the result table lists the parameters, the derived quantities and the baseline with units", () => {
    const rows = transitResultRows(RESULT, OFFSET, "BJD_TDB - 2460310", "Jy");
    expect(rows.map((row) => row.key)).toEqual(["t0", "rp", "a_rs", "b", "u1", "u2", "depth", "inclination", "t14", "t23", "density", "c0", "c1"]);
    expect(rows.map((row) => row.label)).toEqual([
      "Mid-transit time",
      "Rp/Rs",
      "a/Rs",
      "Impact parameter b",
      "Limb darkening u1",
      "Limb darkening u2",
      "(Rp/Rs)^2",
      "Inclination",
      "Duration, contacts 1 to 4",
      "Duration, contacts 2 to 3",
      "Mean stellar density",
      "Baseline c0",
      "Baseline c1",
    ]);
    expect(rows.map((row) => row.unit)).toEqual(["BJD_TDB - 2460310", "", "", "", "", "", "", "deg", "h", "h", "g/cm3", "Jy", "Jy / d"]);
    expect(rows.map((row) => row.fit)).toEqual([
      "free",
      "free",
      "fixed",
      "fixed",
      "fixed",
      "fixed",
      "derived",
      "derived",
      "derived",
      "derived",
      "derived",
      "free",
      "free",
    ]);
    expect(rows.map((row) => row.value)).toEqual([
      F(2460310.4999394 - 2460310, 0.0000828),
      F(0.09982, 0.00048),
      "9",
      "0.3",
      "0.4",
      "0.25",
      F(0.009964, 0.000095),
      F(88.09, null),
      F(0.1126 * 24, 0.0005 * 24),
      "--",
      F(1.53, null),
      F(999.98, 0.026),
      F(19.92, 0.29),
    ]);
    expect(rows[9].value).toBe("--");

    const unitless = transitResultRows(RESULT, OFFSET, "BJD_TDB - 2460310", null);
    expect(unitless.slice(11).map((row) => row.unit)).toEqual(["", "1 / d"]);

    const quadratic: TransitFitResult = {
      ...RESULT,
      baseline: { order: 2, reference_time: 2460310.4995, coefficients: [999.98, 19.92, -3.1], errors: [0.026, 0.29, null] },
    };
    const quadraticRows = transitResultRows(quadratic, OFFSET, "BJD_TDB - 2460310", "Jy").slice(11);
    expect(quadraticRows.map((row) => row.key)).toEqual(["c0", "c1", "c2"]);
    expect(quadraticRows.map((row) => row.label)).toEqual(["Baseline c0", "Baseline c1", "Baseline c2"]);
    expect(quadraticRows.map((row) => row.unit)).toEqual(["Jy", "Jy / d", "Jy / d2"]);
    expect(quadraticRows.map((row) => row.fit)).toEqual(["free", "free", "free"]);
    expect(quadraticRows[2].value).toBe(F(-3.1, null));
    expect(transitResultRows(quadratic, OFFSET, "BJD_TDB - 2460310", null).slice(11).map((row) => row.unit)).toEqual(["", "1 / d", "1 / d2"]);

    const typed = withParameter(RESULT, { name: "a_rs", value: 336.1234567, err: null, free: false, at_bound: false });
    expect(transitResultRows(typed, OFFSET, "BJD_TDB - 2460310", "Jy")[2].value).toBe("336.1234567");
    const longer = withParameter(RESULT, { name: "a_rs", value: 336.123456789, err: null, free: false, at_bound: false });
    expect(transitResultRows(longer, OFFSET, "BJD_TDB - 2460310", "Jy")[2].value).toBe("336.1234568");

    const fixedTime = withParameter(RESULT, { name: "t0", value: 2460310.4999394, err: null, free: false, at_bound: false });
    const fixedTimeRow = transitResultRows(fixedTime, OFFSET, "BJD_TDB - 2460310", "Jy")[0];
    expect(fixedTimeRow.fit).toBe("fixed");
    expect(fixedTimeRow.value).toBe(String(Number((2460310.4999394 - 2460310).toPrecision(10))));
    expect(transitResultRows(fixedTime, 0, "BJD_TDB", "Jy")[0].value).toBe("2460310.5");
    expect(transitResultRows(RESULT, 0, "BJD_TDB", "Jy")[0].value).toBe(F(2460310.4999394, 0.0000828));
    expect(transitResultRows(RESULT, 0, "BJD_TDB", "Jy")[1].value).toBe(F(0.09982, 0.00048));
  });

  it("an eclipse result names its time, its flux ratio and its depth in ppm", () => {
    const rows = transitResultRows(ECL, OFFSET, "BJD_TDB - 2460310", "Jy");
    expect(rows.map((row) => row.key)).toEqual(["t0", "rp", "a_rs", "b", "flux_ratio", "depth", "inclination", "t14", "t23", "density", "c0", "c1"]);
    expect(rows[0].label).toBe("Mid-eclipse time");
    expect(rows[0].unit).toBe("BJD_TDB - 2460310");
    expect(rows[4]).toEqual({ key: "flux_ratio", label: "Planet/star flux ratio", value: F(0.002, 0.0000067), unit: "", fit: "free" });
    expect(rows[5]).toEqual({ key: "depth", label: "Eclipse depth", value: F(0.001996 * 1e6, 0.0000067 * 1e6), unit: "ppm", fit: "derived" });

    const withoutDepthError = transitResultRows({ ...ECL, depth_err: null }, OFFSET, "BJD_TDB - 2460310", "Jy");
    expect(withoutDepthError[5].value).toBe(F(0.001996 * 1e6, null));

    expect(transitFitLabel("eclipse")).toBe(ECLIPSE_FIT_LABEL);
    expect(transitFitLabel("transit")).toBe(TRANSIT_FIT_LABEL);
    expect(TRANSIT_FIT_LABEL).toBe(
      "transit model fit (least squares): limb darkening as typed or fitted, not from a stellar model; no systematics model beyond the baseline polynomial; no MCMC",
    );
    expect(ECLIPSE_FIT_LABEL).toBe("eclipse model fit (least squares): uniform planet disc; no systematics model beyond the baseline polynomial; no MCMC");
  });

  it("the plot series are the data and the model over the fitted baseline", () => {
    const series = transitPlotSeries(RESULT, INPUT, OFFSET);
    expect(series).not.toBeNull();
    expectClose(series?.time, [0.4, 0.5, 0.6], 6);
    expect(series?.flux).toEqual([998.5 / 998, 0.987, 1002.25 / 1002]);
    expectClose(series?.fluxErr, [0.3 / 998, 0.0003, 0.3 / 1002], 15);
    expect(series?.model).toEqual([1, 0.988, 1]);
    expectClose(series?.residualPpm, [501.00200400801606, -1000, 249.500998003992], 9);
    expectClose(series?.curveTime, [0.4, 0.6], 6);
    expect(series?.curveFlux).toEqual([1, 1]);

    const absolute = transitPlotSeries(RESULT, INPUT, 0);
    expect(absolute?.time).toEqual([2460310.4, 2460310.5, 2460310.6]);
    expect(absolute?.curveTime).toEqual([2460310.4, 2460310.6]);

    expect(transitPlotSeries(RESULT, INPUT_WITHOUT_ERRORS, OFFSET)?.fluxErr).toBeNull();
    expect(transitPlotSeries(WITHOUT_SERIES, INPUT, OFFSET)).toBeNull();
    const twoPoints: TransitFitInput = { time: [2460310.4, 2460310.5], flux: [998.5, 987], err: [0.3, 0.3], pointIndex: [0, 1] };
    expect(transitPlotSeries(RESULT, twoPoints, OFFSET)).toBeNull();
    const noPoints: TransitFitInput = { time: [], flux: [], err: null, pointIndex: [] };
    expect(transitPlotSeries(WITHOUT_SERIES, noPoints, OFFSET)).toBeNull();
  });

  it("the time-averaging rows give the bin length in minutes and the ratio to white noise", () => {
    const rows = timeAveragingRows(RESULT);
    const expected = [
      { points: 1, minutes: 1.44, bins: 240, rmsPpm: 270.7, expectedPpm: 271.3, ratio: 270.7 / 271.3 },
      { points: 2, minutes: 2.88, bins: 120, rmsPpm: 190, expectedPpm: 192.2, ratio: 190 / 192.2 },
    ];
    expect(rows).toHaveLength(2);
    expected.forEach((want, i) => {
      expect(Object.keys(rows[i])).toEqual(["points", "minutes", "bins", "rmsPpm", "expectedPpm", "ratio"]);
      expect(rows[i].points).toBe(want.points);
      expect(rows[i].bins).toBe(want.bins);
      expect(rows[i].minutes).toBeCloseTo(want.minutes, 12);
      expect(rows[i].rmsPpm).toBeCloseTo(want.rmsPpm, 12);
      expect(rows[i].expectedPpm).toBeCloseTo(want.expectedPpm, 12);
      expect(rows[i].ratio).toBeCloseTo(want.ratio, 12);
    });

    const withoutCadence = timeAveragingRows({ ...RESULT, time_averaging: { ...RESULT.time_averaging, cadence_days: 0 } });
    expect(withoutCadence.map((row) => row.minutes)).toEqual([null, null]);
    expect(withoutCadence.map((row) => row.points)).toEqual([1, 2]);

    const withoutExpectation = timeAveragingRows({ ...RESULT, time_averaging: { ...RESULT.time_averaging, expected_ppm: [0, 192.2] } });
    expect(withoutExpectation[0].ratio).toBeNull();
    expect(withoutExpectation[0].expectedPpm).toBe(0);
    expect(withoutExpectation[1].ratio).toBeCloseTo(190 / 192.2, 12);

    const empty = timeAveragingRows({
      ...RESULT,
      time_averaging: { cadence_days: 0.001, bin_points: [], n_bins: [], rms_ppm: [], expected_ppm: [], beta: 1 },
    });
    expect(empty).toEqual([]);
  });

  it("the CSV starts with the provenance, carries the whole fit and names its units", () => {
    const text = transitFitCsvText(RESULT, INPUT, CONTEXT);
    const lines = text.split("\n");
    expect(headerKeys(lines)).toEqual(HEADER_KEYS);
    expect(lines[0]).toBe(`# software: AstroBurst ${APP_VERSION}`);
    [
      "# exported_utc: 2026-11-13T12:00:00.000Z",
      "# file: f0.fits",
      "# series: target T",
      `# fit: ${TRANSIT_FIT_LABEL}`,
      "# model: Mandel and Agol 2002, quadratic limb darkening, circular orbit",
      "# period_d: 3",
      "# t0: 2460310.4999394",
      "# t0_err: 0.0000828",
      "# t0_fit: free",
      "# a_rs: 9",
      "# a_rs_err:",
      "# a_rs_fit: fixed",
      "# depth: 0.009964",
      "# depth_err: 0.000095",
      "# inclination_deg: 88.09",
      "# inclination_err:",
      "# duration_t14_d: 0.1126",
      "# duration_t14_err: 0.0005",
      "# duration_t23_d:",
      "# duration_t23_err:",
      "# stellar_density_g_cm3: 1.53",
      "# stellar_density_err:",
      "# baseline_order: 1",
      "# baseline_reference_time: 2460310.4995",
      "# baseline_c0: 999.98",
      "# baseline_c0_err: 0.026",
      "# baseline_c1: 19.92",
      "# baseline_c1_err: 0.29",
      "# chi2: 195.3",
      "# dof: 234",
      "# reduced_chi2: 0.8346",
      "# weighted: yes",
      "# error_scale: 1",
      "# rms: 0.27",
      "# rms_ppm: 270.7",
      "# beta: 1",
      "# n_points: 3",
      "# n_in_transit: 1",
      "# converged: yes",
      "# time: BJD_TDB",
      "# note: note a",
      "# warning: warn b",
    ].forEach((line) => expect(lines).toContain(line));
    expect(lines.slice(0, HEADER_KEYS.length).every((line) => line.startsWith("# ") && !line.startsWith("# unit "))).toBe(true);
    expect(lines.slice(HEADER_KEYS.length)).toEqual([...UNIT_LINES, HEADER_ROW, FIRST_ROW, SECOND_ROW, THIRD_ROW, ""]);
    expect(transitFitCsvColumns("bjd_tdb").join(",")).toBe(HEADER_ROW);
    expect(text.endsWith(`${THIRD_ROW}\n`)).toBe(true);
    expect(text.includes("\r")).toBe(false);

    const eclipseLines = transitFitCsvText(ECL, INPUT, CONTEXT).split("\n");
    expect(eclipseLines).toContain(`# fit: ${ECLIPSE_FIT_LABEL}`);
    expect(eclipseLines).toContain("# flux_ratio: 0.002");
    expect(eclipseLines).toContain("# flux_ratio_err: 0.0000067");
    expect(eclipseLines).toContain("# flux_ratio_fit: free");
    expect(headerKeys(eclipseLines)).not.toContain("u1");
  });

  it("the saved CSV names its time column by the time scale, so the table reader opens it as a light curve", () => {
    const cases = [
      { scale: "bmjd_tdb", column: "bmjd_tdb", shift: -2400000.5 },
      { scale: "bjd_tdb", column: "bjd_tdb", shift: 0 },
      { scale: "mjd_utc", column: "mjd_utc", shift: -2400000.5 },
      { scale: "jd_utc", column: "jd_utc", shift: 0 },
      { scale: "hjd_utc", column: "hjd_utc", shift: 0 },
      { scale: "jd_header", column: "jd_mid", shift: 0 },
    ] as const;
    for (const { scale, column, shift } of cases) {
      const shifted = { ...INPUT, err: null, time: INPUT.time.map((t) => t + shift) };
      const lines = transitFitCsvText(RESULT, shifted, { ...CONTEXT, timeScale: scale }).split("\n");
      expect(lines).toContain(`# unit ${column}: d`);
      const table = lines.filter((line) => line !== "" && !line.startsWith("#")).map((line) => line.split(","));
      const [header, ...rows] = table;
      expect(header).toEqual(transitFitCsvColumns(scale));
      const columns = header.map((name, k) => ({
        name,
        unit: null,
        values: rows.map((row) => (row[k] === "" ? null : Number(row[k]))),
      }));
      const plan = planLightCurveTable({ columns });
      expect(plan.time).toEqual({ column, scale, reason: null });
      expect(plan.values.map((value) => [value.column, value.quantity, value.errColumn])).toEqual([["flux", "flux", "flux_err"]]);
      expect(plan.notes).toEqual([]);
    }
  });

  it("a fit without errors or without its series still exports", () => {
    const withoutErrors = transitFitCsvText(RESULT, INPUT_WITHOUT_ERRORS, CONTEXT).split("\n");
    const rows = withoutErrors.slice(withoutErrors.indexOf(HEADER_ROW) + 1, -1);
    expect(rows).toHaveLength(3);
    expect(rows[1]).toBe("2460310.5,987,,1000,988,-1,0.987,0.988,-1000");
    rows.forEach((row) => expect(row.split(",")[2]).toBe(""));

    const unitless = transitFitCsvText(RESULT, INPUT, { ...CONTEXT, fluxUnit: null }).split("\n");
    expect(unitless.filter((line) => line.startsWith("# unit "))).toEqual(["# unit bjd_tdb: d", "# unit residual_ppm: ppm"]);

    const withoutSeries = transitFitCsvText(WITHOUT_SERIES, INPUT, CONTEXT);
    expect(withoutSeries.endsWith(`# unit residual_ppm: ppm\n${HEADER_ROW}\n`)).toBe(true);
    expect(headerKeys(withoutSeries.split("\n"))).toEqual(HEADER_KEYS);

    const stamped = transitFitCsvText(RESULT, INPUT, { fileName: null, series: "target T", timeName: "BJD_TDB", timeScale: "bjd_tdb", fluxUnit: "Jy" }).split("\n");
    expect(stamped[1]).toMatch(/^# exported_utc: \d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/);
    expect(stamped[2]).toBe("# file:");

    const unweighted = transitFitCsvText({ ...RESULT, weighted: false, converged: false, notes: ["n1", "n2"], warnings: [] }, INPUT, CONTEXT).split("\n");
    expect(unweighted).toContain("# weighted: no");
    expect(unweighted).toContain("# converged: no");
    expect(headerKeys(unweighted).slice(-3)).toEqual(["time", "note", "note"]);
    expect(unweighted).toContain("# note: n1");
    expect(unweighted).toContain("# note: n2");

    const scaled = transitFitCsvText(
      { ...RESULT, weighted: false, error_scale: 1.5, time_averaging: { ...RESULT.time_averaging, beta: 1.25 } },
      INPUT,
      CONTEXT,
    ).split("\n");
    expect(scaled).toContain("# weighted: no");
    expect(scaled).toContain("# converged: yes");
    expect(scaled).toContain("# error_scale: 1.5");
    expect(scaled).toContain("# beta: 1.25");
  });

  it("the log entry records the fit, its label and what was free", () => {
    const framesDoc = doc(FRAMES_SOURCE);
    const entry = transitFitEntry(framesDoc, RESULT, "BJD_TDB");
    expect(entry.kind).toBe(transitFitLogKind());
    expect(entry.file).toBe("f0.fits");
    expect(entry.image).toBe("3 points");
    expect(entry.dq).toBe("not_applied");
    expect(entry.unit).toBe("fraction");
    expect(JSON.parse(entry.source)).toEqual({ kind: "light_curve", doc: "target T", from: framesDoc.source });
    expect(Object.keys(entry.params)).toEqual([
      "series",
      "fit",
      "model",
      "time_scale",
      "period_days",
      "free",
      "baseline_order",
      "weighted",
      "error_scale",
      "converged",
    ]);
    expect(entry.params).toEqual({
      series: "target T",
      fit: "transit",
      model: "Mandel and Agol 2002, quadratic limb darkening, circular orbit",
      time_scale: "BJD_TDB",
      period_days: 3,
      free: "t0 rp c0 c1",
      baseline_order: 1,
      weighted: true,
      error_scale: 1,
      converged: true,
    });
    expect(Object.keys(entry.values)).toEqual([
      "depth",
      "depth_err",
      "t0",
      "t0_err",
      "rp",
      "rp_err",
      "a_rs",
      "a_rs_err",
      "b",
      "b_err",
      "u1",
      "u1_err",
      "u2",
      "u2_err",
      "inclination_deg",
      "inclination_err",
      "duration_t14_days",
      "duration_t14_err",
      "duration_t23_days",
      "duration_t23_err",
      "stellar_density_cgs",
      "stellar_density_err",
      "chi2",
      "dof",
      "reduced_chi2",
      "rms_ppm",
      "beta",
      "n_points",
      "n_in_transit",
    ]);
    expect(entry.values).toEqual({
      depth: 0.009964,
      depth_err: 0.000095,
      t0: 2460310.4999394,
      t0_err: 0.0000828,
      rp: 0.09982,
      rp_err: 0.00048,
      a_rs: 9,
      a_rs_err: null,
      b: 0.3,
      b_err: null,
      u1: 0.4,
      u1_err: null,
      u2: 0.25,
      u2_err: null,
      inclination_deg: 88.09,
      inclination_err: null,
      duration_t14_days: 0.1126,
      duration_t14_err: 0.0005,
      duration_t23_days: null,
      duration_t23_err: null,
      stellar_density_cgs: 1.53,
      stellar_density_err: null,
      chi2: 195.3,
      dof: 234,
      reduced_chi2: 0.8346,
      rms_ppm: 270.7,
      beta: 1,
      n_points: 3,
      n_in_transit: 1,
    });
    expect(entry.notes).toEqual([
      TRANSIT_FIT_LABEL,
      "note a",
      "warning: warn b",
      "times in days, BJD_TDB; durations in days; inclination in degrees; density in g/cm3",
    ]);

    const tableDoc = doc({ kind: "table", path: "C:/d/w39_whtlt.ecsv", time_column: "t", value_column: "f" });
    expect(transitFitEntry(tableDoc, RESULT, "BJD_TDB").file).toBe("w39_whtlt.ecsv");

    const tso = { kind: "tso", detector: "NRS1", sporder: null, band_um: null, mode: "pipeline" } as const;
    expect(transitFitEntry(doc({ ...tso, paths: ["C:\\d\\jw_x1dints.fits", "C:\\d\\other.fits"] }), RESULT, "BJD_TDB").file).toBe("jw_x1dints.fits");
    expect(transitFitEntry(doc({ ...tso, paths: [] }), RESULT, "BJD_TDB").file).toBe("");

    const scaled = transitFitEntry(
      framesDoc,
      { ...RESULT, weighted: false, error_scale: 1.5, time_averaging: { ...RESULT.time_averaging, beta: 1.25 } },
      "BJD_TDB",
    );
    expect(scaled.params.weighted).toBe(false);
    expect(scaled.params.converged).toBe(true);
    expect(scaled.params.error_scale).toBe(1.5);
    expect(scaled.values.beta).toBe(1.25);

    const eclipse = transitFitEntry(framesDoc, ECL, "BMJD (TDB)");
    expect(eclipse.params.fit).toBe("eclipse");
    expect(eclipse.params.time_scale).toBe("BMJD (TDB)");
    expect(eclipse.values.flux_ratio).toBe(0.002);
    expect(eclipse.values.flux_ratio_err).toBe(0.0000067);
    expect(Object.keys(eclipse.values)).not.toContain("u1");
    expect(eclipse.notes[0]).toBe(ECLIPSE_FIT_LABEL);
    expect(eclipse.notes[eclipse.notes.length - 1]).toBe("times in days, BMJD (TDB); durations in days; inclination in degrees; density in g/cm3");
  });

  it("the log kind is transit_fit when the log knows it and time_series until then", () => {
    expect(transitFitLogKind()).toBe((MEASUREMENT_KINDS as readonly string[]).includes("transit_fit") ? "transit_fit" : "time_series");
    expect(transitFitLogKind()).toBe("transit_fit");

    const entry = transitFitEntry(doc(FRAMES_SOURCE), RESULT, "BJD_TDB");
    const csv = measurementLogCsv([{ ...entry, id: "fit-1", timestamp_utc: "2026-11-13T12:00:00.000Z" }]);
    const header = csv.split(/\r?\n/)[0].split(",");
    expect(header).toContain("p_period_days");
    expect(header).toContain("v_depth");
  });

  it("the functions do not change their arguments", () => {
    const result = deepFreeze(structuredClone(RESULT));
    const input = deepFreeze(structuredClone(INPUT));
    const context = deepFreeze({ ...CONTEXT });
    const frozenDoc = deepFreeze(doc(FRAMES_SOURCE));
    expect(Object.isFrozen(result.parameters[0])).toBe(true);
    expect(Object.isFrozen(result.time_averaging.bin_points)).toBe(true);
    expect(Object.isFrozen(input.time)).toBe(true);

    expect(transitResultRows(result, OFFSET, "BJD_TDB - 2460310", "Jy")).toEqual(transitResultRows(RESULT, OFFSET, "BJD_TDB - 2460310", "Jy"));
    expect(transitPlotSeries(result, input, OFFSET)).toEqual(transitPlotSeries(RESULT, INPUT, OFFSET));
    expect(timeAveragingRows(result)).toEqual(timeAveragingRows(RESULT));
    expect(transitFitCsvText(result, input, context)).toBe(transitFitCsvText(RESULT, INPUT, CONTEXT));
    expect(transitFitEntry(frozenDoc, result, "BJD_TDB")).toEqual(transitFitEntry(doc(FRAMES_SOURCE), RESULT, "BJD_TDB"));
  });
});
