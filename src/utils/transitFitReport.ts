import type { LightCurveDoc, LightCurveSource, LightCurveTimeScale } from "../shared/types/lightCurve";
import type { TransitFitResult, TransitFittedParameter, TransitKind, TransitParameterName } from "../shared/types/transit";
import { exportEntries, provenanceLines, unitLines, withProvenance, type ProvenanceEntry } from "./csvProvenance";
import { formatWithUncertainty } from "./formatUncertainty";
import { MEASUREMENT_KINDS, type MeasurementKind, type MeasurementLogDraft } from "./measurementLog";
import type { TransitFitInput } from "./transitFitSetup";
import { fileBaseName } from "./values";

export const TRANSIT_FIT_LABEL =
  "transit model fit (least squares): limb darkening as typed or fitted, not from a stellar model; no systematics model beyond the baseline polynomial; no MCMC";
export const ECLIPSE_FIT_LABEL = "eclipse model fit (least squares): uniform planet disc; no systematics model beyond the baseline polynomial; no MCMC";

const PPM = 1e6;
const HOURS_PER_DAY = 24;
const MINUTES_PER_DAY = 1440;
const FIXED_VALUE_DIGITS = 10;
const CSV_LINE_END = "\n";
const TRANSIT_FIT_KIND: string = "transit_fit";
const MID_ECLIPSE_LABEL = "Mid-eclipse time";
const PARAMETER_LABELS: Readonly<Record<TransitParameterName, string>> = {
  t0: "Mid-transit time",
  rp: "Rp/Rs",
  a_rs: "a/Rs",
  b: "Impact parameter b",
  u1: "Limb darkening u1",
  u2: "Limb darkening u2",
  flux_ratio: "Planet/star flux ratio",
};

export function transitFitLabel(kind: TransitKind): string {
  return kind === "eclipse" ? ECLIPSE_FIT_LABEL : TRANSIT_FIT_LABEL;
}

export interface TransitResultRow {
  key: string;
  label: string;
  value: string;
  unit: string;
  fit: "free" | "fixed" | "derived";
}

function fitState(parameter: TransitFittedParameter): "free" | "fixed" {
  return parameter.free ? "free" : "fixed";
}

function scaled(value: number | null, factor: number): number | null {
  return value === null ? null : value * factor;
}

function parameterRow(parameter: TransitFittedParameter, kind: TransitKind, offset: number, timeName: string): TransitResultRow {
  const isTime = parameter.name === "t0";
  const value = isTime ? parameter.value - offset : parameter.value;
  return {
    key: parameter.name,
    label: isTime && kind === "eclipse" ? MID_ECLIPSE_LABEL : PARAMETER_LABELS[parameter.name],
    value: parameter.free ? formatWithUncertainty(value, parameter.err) : String(Number(value.toPrecision(FIXED_VALUE_DIGITS))),
    unit: isTime ? timeName : "",
    fit: fitState(parameter),
  };
}

function derivedRow(key: string, label: string, value: number | null, err: number | null, unit: string): TransitResultRow {
  return { key, label, value: formatWithUncertainty(value, err), unit, fit: "derived" };
}

function baselineUnit(k: number, fluxUnit: string | null): string {
  if (k === 0) return fluxUnit ?? "";
  return `${fluxUnit ?? "1"} / d${k === 1 ? "" : k}`;
}

export function transitResultRows(result: TransitFitResult, offset: number, timeName: string, fluxUnit: string | null): TransitResultRow[] {
  const { derived, baseline } = result;
  return [
    ...result.parameters.map((parameter) => parameterRow(parameter, result.kind, offset, timeName)),
    result.kind === "eclipse"
      ? derivedRow("depth", "Eclipse depth", result.depth * PPM, scaled(result.depth_err, PPM), "ppm")
      : derivedRow("depth", "(Rp/Rs)^2", result.depth, result.depth_err, ""),
    derivedRow("inclination", "Inclination", derived.inclination_deg, derived.inclination_err, "deg"),
    derivedRow("t14", "Duration, contacts 1 to 4", scaled(derived.duration_t14_days, HOURS_PER_DAY), scaled(derived.duration_t14_err, HOURS_PER_DAY), "h"),
    derivedRow("t23", "Duration, contacts 2 to 3", scaled(derived.duration_t23_days, HOURS_PER_DAY), scaled(derived.duration_t23_err, HOURS_PER_DAY), "h"),
    derivedRow("density", "Mean stellar density", derived.stellar_density_cgs, derived.stellar_density_err, "g/cm3"),
    ...baseline.coefficients.map(
      (coefficient, k): TransitResultRow => ({
        key: `c${k}`,
        label: `Baseline c${k}`,
        value: formatWithUncertainty(coefficient, baseline.errors[k]),
        unit: baselineUnit(k, fluxUnit),
        fit: "free",
      }),
    ),
  ];
}

export interface TransitPlotSeries {
  time: number[];
  flux: number[];
  fluxErr: number[] | null;
  model: number[];
  residualPpm: number[];
  curveTime: number[];
  curveFlux: number[];
}

export function transitPlotSeries(result: TransitFitResult, input: TransitFitInput, offset: number): TransitPlotSeries | null {
  const points = result.model_flux.length;
  if (points === 0 || points !== input.time.length) return null;
  const overBaseline = (values: readonly number[]): number[] => values.map((value, i) => value / result.baseline_flux[i]);
  return {
    time: input.time.map((t) => t - offset),
    flux: overBaseline(input.flux),
    fluxErr: input.err === null ? null : overBaseline(input.err),
    model: overBaseline(result.model_flux),
    residualPpm: result.residual.map((residual, i) => (PPM * residual) / result.baseline_flux[i]),
    curveTime: result.curve_time.map((t) => t - offset),
    curveFlux: result.curve_flux.map((flux, i) => flux / result.curve_baseline[i]),
  };
}

export interface TimeAveragingRow {
  points: number;
  minutes: number | null;
  bins: number;
  rmsPpm: number;
  expectedPpm: number;
  ratio: number | null;
}

export function timeAveragingRows(result: TransitFitResult): TimeAveragingRow[] {
  const averaging = result.time_averaging;
  return averaging.bin_points.map((points, i) => {
    const rmsPpm = averaging.rms_ppm[i];
    const expectedPpm = averaging.expected_ppm[i];
    return {
      points,
      minutes: averaging.cadence_days > 0 ? points * averaging.cadence_days * MINUTES_PER_DAY : null,
      bins: averaging.n_bins[i],
      rmsPpm,
      expectedPpm,
      ratio: expectedPpm > 0 ? rmsPpm / expectedPpm : null,
    };
  });
}

export interface TransitCsvContext {
  fileName: string | null;
  series: string;
  timeName: string;
  timeScale: LightCurveTimeScale;
  fluxUnit: string | null;
  exportedUtc?: string;
}

const TIME_SCALE_COLUMNS: Readonly<Record<LightCurveTimeScale, string>> = {
  bmjd_tdb: "bmjd_tdb",
  bjd_tdb: "bjd_tdb",
  mjd_utc: "mjd_utc",
  jd_utc: "jd_utc",
  hjd_utc: "hjd_utc",
  jd_header: "jd_mid",
  index: "index",
};

const TRANSIT_FIT_VALUE_COLUMNS = ["flux", "flux_err", "baseline", "model", "residual", "norm_flux", "norm_model", "residual_ppm"] as const;

export function transitFitCsvColumns(timeScale: LightCurveTimeScale): string[] {
  return [TIME_SCALE_COLUMNS[timeScale], ...TRANSIT_FIT_VALUE_COLUMNS];
}

function yesOrNo(answer: boolean): string {
  return answer ? "yes" : "no";
}

function fitEntries(result: TransitFitResult, context: TransitCsvContext): ProvenanceEntry[] {
  const { derived, baseline } = result;
  return [
    ["series", context.series],
    ["fit", transitFitLabel(result.kind)],
    ["model", result.model],
    ["period_d", result.period],
    ...result.parameters.flatMap((parameter): ProvenanceEntry[] => [
      [parameter.name, parameter.value],
      [`${parameter.name}_err`, parameter.err],
      [`${parameter.name}_fit`, fitState(parameter)],
    ]),
    ["depth", result.depth],
    ["depth_err", result.depth_err],
    ["inclination_deg", derived.inclination_deg],
    ["inclination_err", derived.inclination_err],
    ["duration_t14_d", derived.duration_t14_days],
    ["duration_t14_err", derived.duration_t14_err],
    ["duration_t23_d", derived.duration_t23_days],
    ["duration_t23_err", derived.duration_t23_err],
    ["stellar_density_g_cm3", derived.stellar_density_cgs],
    ["stellar_density_err", derived.stellar_density_err],
    ["baseline_order", baseline.order],
    ["baseline_reference_time", baseline.reference_time],
    ...baseline.coefficients.flatMap((coefficient, k): ProvenanceEntry[] => [
      [`baseline_c${k}`, coefficient],
      [`baseline_c${k}_err`, baseline.errors[k]],
    ]),
    ["chi2", result.chi2],
    ["dof", result.dof],
    ["reduced_chi2", result.reduced_chi2],
    ["weighted", yesOrNo(result.weighted)],
    ["error_scale", result.error_scale],
    ["rms", result.rms],
    ["rms_ppm", result.rms_ppm],
    ["beta", result.time_averaging.beta],
    ["n_points", result.n_points],
    ["n_in_transit", result.n_in_transit],
    ["converged", yesOrNo(result.converged)],
    ["time", context.timeName],
    ...result.notes.map((note): ProvenanceEntry => ["note", note]),
    ...result.warnings.map((warning): ProvenanceEntry => ["warning", warning]),
  ];
}

function csvTable(result: TransitFitResult, input: TransitFitInput, timeScale: LightCurveTimeScale): string {
  const series = transitPlotSeries(result, input, 0);
  const { err } = input;
  const rows =
    series === null
      ? []
      : input.time.map((t, i) =>
          [
            t,
            input.flux[i],
            err === null ? "" : err[i],
            result.baseline_flux[i],
            result.model_flux[i],
            result.residual[i],
            series.flux[i],
            series.model[i],
            series.residualPpm[i],
          ]
            .map(String)
            .join(","),
        );
  return [transitFitCsvColumns(timeScale).join(","), ...rows].map((line) => line + CSV_LINE_END).join("");
}

export function transitFitCsvText(result: TransitFitResult, input: TransitFitInput, context: TransitCsvContext): string {
  const { fluxUnit, timeScale } = context;
  const lines = [
    ...provenanceLines(exportEntries(context.fileName, context.exportedUtc)),
    ...provenanceLines(fitEntries(result, context)),
    ...unitLines({ [TIME_SCALE_COLUMNS[timeScale]]: "d", flux: fluxUnit, flux_err: fluxUnit, baseline: fluxUnit, model: fluxUnit, residual: fluxUnit, residual_ppm: "ppm" }),
  ];
  return withProvenance(lines, csvTable(result, input, timeScale), CSV_LINE_END);
}

export function transitFitLogKind(): MeasurementKind {
  return MEASUREMENT_KINDS.find((kind) => kind === TRANSIT_FIT_KIND) ?? "time_series";
}

function sourceFile(source: LightCurveSource): string {
  if (source.kind === "frames") return source.reference;
  return fileBaseName(source.kind === "table" ? source.path : (source.paths[0] ?? ""));
}

export function transitFitEntry(doc: LightCurveDoc, result: TransitFitResult, timeName: string): MeasurementLogDraft {
  const { derived } = result;
  return {
    kind: transitFitLogKind(),
    file: sourceFile(doc.source),
    image: `${result.n_points} points`,
    dq: "not_applied",
    unit: "fraction",
    source: JSON.stringify({ kind: "light_curve", doc: doc.label, from: doc.source }),
    params: {
      series: doc.label,
      fit: result.kind,
      model: result.model,
      time_scale: timeName,
      period_days: result.period,
      free: result.free_names.join(" "),
      baseline_order: result.baseline.order,
      weighted: result.weighted,
      error_scale: result.error_scale,
      converged: result.converged,
    },
    values: {
      depth: result.depth,
      depth_err: result.depth_err,
      ...Object.fromEntries(
        result.parameters.flatMap((parameter): [string, number | null][] => [
          [parameter.name, parameter.value],
          [`${parameter.name}_err`, parameter.err],
        ]),
      ),
      inclination_deg: derived.inclination_deg,
      inclination_err: derived.inclination_err,
      duration_t14_days: derived.duration_t14_days,
      duration_t14_err: derived.duration_t14_err,
      duration_t23_days: derived.duration_t23_days,
      duration_t23_err: derived.duration_t23_err,
      stellar_density_cgs: derived.stellar_density_cgs,
      stellar_density_err: derived.stellar_density_err,
      chi2: result.chi2,
      dof: result.dof,
      reduced_chi2: result.reduced_chi2,
      rms_ppm: result.rms_ppm,
      beta: result.time_averaging.beta,
      n_points: result.n_points,
      n_in_transit: result.n_in_transit,
    },
    notes: [
      transitFitLabel(result.kind),
      ...result.notes,
      ...result.warnings.map((warning) => `warning: ${warning}`),
      `times in days, ${timeName}; durations in days; inclination in degrees; density in g/cm3`,
    ],
  };
}
