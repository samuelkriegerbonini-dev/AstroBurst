import type { LightCurveDoc, LightCurveViewState } from "../shared/types/lightCurve";
import type { TransitFitResult, TransitParameterName } from "../shared/types/transit";
import type { TsoTransitBin, TsoTransitFitRequest, TsoTransitSpectrum } from "../shared/types/tsoTransit";
import { parseDecimalText } from "./decimalText";
import { formatSignificant } from "./formatSignificant";
import { lightCurveModel, type PointState } from "./lightCurveViewModel";
import { transitGeometryOf } from "./transitFitSetup";
import { bandText, tsoTimeName, type DepthSpectrum } from "./tsoLightCurves";
import { finiteOrNull, isFiniteNumber } from "./values";

export const TRANSIT_FIT_LABEL =
  "transit model fit per bin: Mandel and Agol 2002, quadratic limb darkening, baseline polynomial; no orbital eccentricity, no limb darkening from stellar models, no systematics model, no MCMC";
export const FIT_COLOR = "#67e8f9";
export const QUICK_SERIES_LABEL = "quick look, 1 - F_in / F_out";
export const FIT_SERIES_LABEL = "model fit, (Rp/Rs)^2";
export const BASELINE_NAMES = ["constant", "linear", "quadratic"] as const;

const MISSING_TEXT = "--";
const DEPTH_DIGITS = 5;
const ERROR_DIGITS = 2;
const CHI2_DIGITS = 4;
const QUICK_DIGITS = 4;
const DRAFT_TEXTS = ["periodText", "t0Text", "aRsText", "bText", "rpText", "u1Text", "u2Text"] as const;
const ORBIT_ENTRIES: readonly TransitParameterName[] = ["t0", "rp", "a_rs", "b"];
const LEFT_OUT_STATES: readonly PointState[] = ["flagged", "manual", "clipped"];

export interface TransitDraft {
  periodText: string;
  t0Text: string;
  aRsText: string;
  bText: string;
  rpText: string;
  u1Text: string;
  u2Text: string;
  limbFree: boolean;
  baselineOrder: 0 | 1 | 2;
  scaleErrors: boolean;
  fromWhiteLight: boolean;
}

export const EMPTY_TRANSIT_DRAFT: TransitDraft = Object.freeze({
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

export function isBlankDraft(draft: TransitDraft): boolean {
  return DRAFT_TEXTS.every((name) => draft[name].trim() === "");
}

export interface WhiteLightValues {
  periodDays: number;
  t0: number;
  aRs: number;
  b: number;
  rp: number;
  u1: number;
  u2: number;
  baselineOrder: 0 | 1 | 2;
}

function fittedBaselineOrder(order: number): 0 | 1 | 2 {
  if (order >= 2) return 2;
  return order >= 1 ? 1 : 0;
}

export function whiteLightValues(result: TransitFitResult | null): WhiteLightValues | null {
  const geometry = transitGeometryOf(result);
  if (result === null || geometry === null || !result.converged) return null;
  const { period, t0, rp, a_rs, b, u1, u2 } = geometry;
  if (![period, t0, rp, a_rs, b, u1, u2].every(isFiniteNumber) || period <= 0) return null;
  const orbitStoppedAtLimit = result.parameters.some(
    (parameter) => ORBIT_ENTRIES.includes(parameter.name) && parameter.free && parameter.at_bound,
  );
  if (orbitStoppedAtLimit) return null;
  return { periodDays: period, t0, aRs: a_rs, b, rp, u1, u2, baselineOrder: fittedBaselineOrder(result.baseline.order) };
}

export function draftFromWhiteLight(values: WhiteLightValues, previous: TransitDraft): TransitDraft {
  return {
    periodText: String(values.periodDays),
    t0Text: String(values.t0),
    aRsText: String(values.aRs),
    bText: String(values.b),
    rpText: String(values.rp),
    u1Text: String(values.u1),
    u2Text: String(values.u2),
    limbFree: previous.limbFree,
    baselineOrder: values.baselineOrder,
    scaleErrors: previous.scaleErrors,
    fromWhiteLight: true,
  };
}

export interface TransitFitInput {
  fit: Required<TsoTransitFitRequest> | null;
  error: string | null;
}

function refused(error: string): TransitFitInput {
  return { fit: null, error };
}

export function fitFromDraft(draft: TransitDraft, leftOut: readonly number[]): TransitFitInput {
  const period = parseDecimalText(draft.periodText);
  if (period === null || period <= 0) return refused("the period must be a number of days above 0");
  const t0 = parseDecimalText(draft.t0Text);
  if (t0 === null) return refused("t0 must be a number: a time in the scale of the series");
  const aRs = parseDecimalText(draft.aRsText);
  if (aRs === null || aRs <= 1) return refused("a/Rs must be a number above 1");
  const b = parseDecimalText(draft.bText);
  if (b === null || b < 0) return refused("b must be a number of 0 or above");
  if (b >= aRs) return refused("b must be below a/Rs");
  const rp = parseDecimalText(draft.rpText);
  if (rp === null || rp <= 0) return refused("Rp/Rs must be a number above 0");
  const u1 = parseDecimalText(draft.u1Text);
  const u2 = parseDecimalText(draft.u2Text);
  if (u1 === null || u2 === null) return refused("u1 and u2 must be numbers");
  const seriesRows = leftOut.filter((row) => Number.isInteger(row) && row >= 0);
  return {
    fit: {
      period_days: period,
      t0,
      a_rs: aRs,
      b,
      rp_start: rp,
      u1,
      u2,
      limb_darkening: draft.limbFree ? "free" : "fixed",
      baseline_order: draft.baselineOrder,
      exclude_rows: [...new Set(seriesRows)].sort((first, second) => first - second),
      scale_errors: draft.scaleErrors,
    },
    error: null,
  };
}

export function leftOutRows(white: LightCurveDoc | null, view: LightCurveViewState, rowIndex: readonly number[]): number[] {
  if (white === null) return [];
  const { state } = lightCurveModel(white, view);
  return rowIndex.filter((_, position) => LEFT_OUT_STATES.includes(state[position]));
}

export interface FitPlotSeries {
  x: number[];
  y: (number | null)[];
  yErr: (number | null)[];
  color: string;
  label: string;
  mode: "points";
}

function centreOf(bin: TsoTransitBin): number {
  return (bin.band_um[0] + bin.band_um[1]) / 2;
}

export function fitPlotSeries(spectrum: Pick<TsoTransitSpectrum, "bins">): FitPlotSeries | null {
  const points = spectrum.bins
    .flatMap((bin) => (isFiniteNumber(bin.depth) ? [{ x: centreOf(bin), y: bin.depth, yErr: finiteOrNull(bin.depth_err) }] : []))
    .sort((first, second) => first.x - second.x);
  if (points.length === 0) return null;
  return {
    x: points.map((point) => point.x),
    y: points.map((point) => point.y),
    yErr: points.map((point) => point.yErr),
    color: FIT_COLOR,
    label: FIT_SERIES_LABEL,
    mode: "points",
  };
}

export interface FitTableRow {
  key: string;
  band: string;
  depth: string;
  depthErr: string;
  rp: string;
  rpErr: string;
  chi2: string;
  points: string;
  quick: string;
  remark: string;
  warnings: string[];
  failed: boolean;
  flagged: boolean;
}

function numberText(value: number | null, digits: number): string {
  return isFiniteNumber(value) ? formatSignificant(value, digits) : MISSING_TEXT;
}

function quickDepthOf(quick: DepthSpectrum | null, band: readonly [number, number]): number | null {
  return quick?.rows.find((row) => row.band[0] === band[0] && row.band[1] === band[1])?.depth ?? null;
}

function remarkOf(bin: TsoTransitBin): string {
  if (bin.error !== null) return bin.error;
  const words: string[] = [];
  if (bin.grazing) words.push("grazing");
  if (bin.pixel_loss) words.push("pixel loss");
  if (bin.warnings.length > 0) words.push("fit warnings");
  return words.join(", ");
}

export function fitTableRows(spectrum: Pick<TsoTransitSpectrum, "bins">, quick: DepthSpectrum | null): FitTableRow[] {
  return spectrum.bins
    .map((bin, position) => ({ bin, position }))
    .sort((first, second) => centreOf(first.bin) - centreOf(second.bin))
    .map(({ bin, position }) => ({
      key: `fit:${position}`,
      band: bandText(bin.band_um),
      depth: numberText(bin.depth, DEPTH_DIGITS),
      depthErr: numberText(bin.depth_err, ERROR_DIGITS),
      rp: numberText(bin.rp, DEPTH_DIGITS),
      rpErr: numberText(bin.rp_err, ERROR_DIGITS),
      chi2: isFiniteNumber(bin.chi2) ? `${formatSignificant(bin.chi2, CHI2_DIGITS)} / ${bin.dof}` : MISSING_TEXT,
      points: `${bin.n_in_transit} / ${bin.n_points}`,
      quick: numberText(quickDepthOf(quick, bin.band_um), QUICK_DIGITS),
      remark: remarkOf(bin),
      warnings: [...bin.warnings],
      failed: bin.error !== null,
      flagged: bin.grazing || bin.pixel_loss || bin.warnings.length > 0,
    }));
}

export function fixedLine(spectrum: Pick<TsoTransitSpectrum, "fit" | "time_scale">): string {
  const { fit, time_scale } = spectrum;
  const limb = fit.limb_darkening === "free" ? "; u1 and u2 fitted in every bin" : `, u1 ${fit.u1}, u2 ${fit.u2}`;
  const leftOut = fit.exclude_rows.length > 0 ? `; ${fit.exclude_rows.length} integration(s) left out` : "";
  const scaled = fit.scale_errors ? "; errors scaled by sqrt(chi2 / dof)" : "";
  return `Fixed in every bin: period ${fit.period_days} d, t0 ${fit.t0} (${tsoTimeName(time_scale)}), a/Rs ${fit.a_rs}, b ${fit.b}${limb}; ${BASELINE_NAMES[fit.baseline_order]} baseline${leftOut}${scaled}`;
}

export function onDifferentScales(spectrum: Pick<TsoTransitSpectrum, "bins">): boolean {
  return spectrum.bins.some((bin) => bin.grazing);
}
