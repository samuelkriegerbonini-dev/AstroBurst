import type { LightCurveWindow } from "../shared/types/lightCurve";
import type {
  TransitFitRequest,
  TransitFitResult,
  TransitKind,
  TransitParameterInput,
  TransitParameterName,
} from "../shared/types/transit";
import { parseDecimalText } from "./decimalText";
import { MAG_ERR_EXACT, relativeFlux } from "./lightCurveAnalysis";
import type { LightCurveModel } from "./lightCurveViewModel";
import { isFiniteNumber } from "./values";

export interface TransitFitInput {
  time: number[];
  flux: number[];
  err: number[] | null;
  pointIndex: number[];
}

export type TransitDraftName = "t0" | "rp" | "a_rs" | "b" | "u1" | "u2" | "flux_ratio";

export interface TransitParameterDraft {
  text: string;
  free: boolean;
}

export interface TransitFitSetup {
  kind: TransitKind;
  period: string;
  t0: TransitParameterDraft;
  rp: TransitParameterDraft;
  a_rs: TransitParameterDraft;
  b: TransitParameterDraft;
  u1: TransitParameterDraft;
  u2: TransitParameterDraft;
  flux_ratio: TransitParameterDraft;
  baselineOrder: 0 | 1 | 2;
  scaleErrors: boolean;
}

function frozenDraft(text: string, free: boolean): TransitParameterDraft {
  return Object.freeze({ text, free });
}

export const DEFAULT_TRANSIT_FIT_SETUP: TransitFitSetup = Object.freeze({
  kind: "transit",
  period: "",
  t0: frozenDraft("", true),
  rp: frozenDraft("", true),
  a_rs: frozenDraft("", true),
  b: frozenDraft("", true),
  u1: frozenDraft("0", false),
  u2: frozenDraft("0", false),
  flux_ratio: frozenDraft("0.001", true),
  baselineOrder: 1,
  scaleErrors: false,
});

const TRANSIT_DRAFT_NAMES: readonly TransitDraftName[] = Object.freeze(["t0", "rp", "a_rs", "b", "u1", "u2"]);
const ECLIPSE_DRAFT_NAMES: readonly TransitDraftName[] = Object.freeze(["t0", "rp", "a_rs", "b", "flux_ratio"]);

const DRAFT_LABELS: Record<TransitDraftName, string> = {
  t0: "mid-transit time",
  rp: "Rp/Rs",
  a_rs: "a/Rs",
  b: "impact parameter b",
  u1: "limb darkening u1",
  u2: "limb darkening u2",
  flux_ratio: "planet/star flux ratio",
};

const START_RADIUS_RATIO = 0.1;
const START_IMPACT = 0.3;
const START_FLUX_RATIO = 0.001;
const UNUSED_PARAMETER: TransitParameterInput = Object.freeze({ value: 0, free: false });

export function transitDraftNames(kind: TransitKind): readonly TransitDraftName[] {
  return kind === "eclipse" ? ECLIPSE_DRAFT_NAMES : TRANSIT_DRAFT_NAMES;
}

export function transitDraftLabel(name: TransitDraftName, kind: TransitKind): string {
  return name === "t0" && kind === "eclipse" ? "mid-eclipse time" : DRAFT_LABELS[name];
}

function usableError(value: number | null): value is number {
  return isFiniteNumber(value) && value > 0;
}

export function transitFitInput(model: LightCurveModel): TransitFitInput {
  const { doc, state } = model;
  const curve =
    doc.quantity === "diff_mag"
      ? relativeFlux(doc.value, doc.err, doc.magErrFactor ?? MAG_ERR_EXACT)
      : { flux: doc.value, err: doc.err };
  const time: number[] = [];
  const flux: number[] = [];
  const errors: (number | null)[] = [];
  const pointIndex: number[] = [];
  for (let i = 0; i < state.length; i++) {
    const t = doc.time[i];
    const f = curve.flux[i];
    if (state[i] !== "kept" || !isFiniteNumber(t) || !isFiniteNumber(f)) continue;
    time.push(t);
    flux.push(f);
    errors.push(curve.err[i]);
    pointIndex.push(i);
  }
  return { time, flux, err: errors.length > 0 && errors.every(usableError) ? errors : null, pointIndex };
}

export function transitFitSignature(input: TransitFitInput): string {
  const { time, flux, err } = input;
  const n = time.length;
  const sum = flux.reduce((total, value) => total + value, 0);
  return `${n}|${time[0] ?? ""}|${time[n - 1] ?? ""}|${sum.toPrecision(12)}|${err ? "e" : "n"}`;
}

export interface TransitRequestInput {
  request: TransitFitRequest | null;
  error: string | null;
}

function refused(error: string): TransitRequestInput {
  return { request: null, error };
}

export function transitFitRequest(input: TransitFitInput, setup: TransitFitSetup, offset: number): TransitRequestInput {
  const n = input.time.length;
  if (n === 0) return refused("no kept point to fit");
  if (setup.period.trim() === "") return refused("type the period in days: it is given, never fitted");
  const period = parseDecimalText(setup.period);
  if (period === null || period <= 0) return refused("period must be a number of days above 0");
  const names = transitDraftNames(setup.kind);
  const typed = new Map<TransitDraftName, TransitParameterInput>();
  for (const name of names) {
    const draft = setup[name];
    const label = transitDraftLabel(name, setup.kind);
    if (draft.text.trim() === "") return refused(`${label} needs a value`);
    const value = parseDecimalText(draft.text);
    if (value === null) return refused(`${label} is not a number`);
    typed.set(name, { value: name === "t0" ? value + offset : value, free: draft.free });
  }
  const k = names.filter((name) => setup[name].free).length + setup.baselineOrder + 1;
  if (n <= k) return refused(`${n} points for ${k} free parameters: the fit needs more points than free parameters`);
  const parameter = (name: TransitDraftName): TransitParameterInput => typed.get(name) ?? UNUSED_PARAMETER;
  return {
    request: {
      kind: setup.kind,
      time: input.time,
      flux: input.flux,
      err: input.err,
      period,
      t0: parameter("t0"),
      rp: parameter("rp"),
      a_rs: parameter("a_rs"),
      b: parameter("b"),
      u1: parameter("u1"),
      u2: parameter("u2"),
      flux_ratio: typed.get("flux_ratio") ?? null,
      baseline_order: setup.baselineOrder,
      scale_errors: setup.scaleErrors,
    },
    error: null,
  };
}

export interface TransitStart {
  t0: number;
  rp: number;
  a_rs: number;
  b: number;
  flux_ratio: number;
}

export function transitStartFromQuickLook(
  window: LightCurveWindow | null,
  depth: number | null,
  periodDays: number | null,
): TransitStart | null {
  if (window === null || !isFiniteNumber(periodDays)) return null;
  const duration = window.end - window.start;
  if (!(duration > 0) || duration >= periodDays / 2) return null;
  const rp = isFiniteNumber(depth) && depth > 0 && depth < 1 ? Math.sqrt(depth) : START_RADIUS_RATIO;
  const b = START_IMPACT;
  const sine = Math.sin((Math.PI * duration) / periodDays);
  return {
    t0: (window.start + window.end) / 2,
    rp,
    a_rs: Math.sqrt(b * b + ((1 + rp) ** 2 - b * b) / (sine * sine)),
    b,
    flux_ratio: isFiniteNumber(depth) && depth > 0 && depth < 0.5 ? depth / (1 - depth) : START_FLUX_RATIO,
  };
}

function withText(draft: TransitParameterDraft, text: string): TransitParameterDraft {
  return { ...draft, text };
}

function significantText(value: number, digits: number): string {
  return String(Number(value.toPrecision(digits)));
}

export function setupWithStart(setup: TransitFitSetup, start: TransitStart, offset: number): TransitFitSetup {
  return {
    ...setup,
    t0: withText(setup.t0, String(Number((start.t0 - offset).toFixed(8)))),
    rp: withText(setup.rp, significantText(start.rp, 6)),
    a_rs: withText(setup.a_rs, significantText(start.a_rs, 6)),
    b: withText(setup.b, significantText(start.b, 6)),
    flux_ratio: setup.kind === "eclipse" ? withText(setup.flux_ratio, significantText(start.flux_ratio, 4)) : setup.flux_ratio,
  };
}

export function setupWithPeriod(setup: TransitFitSetup, periodDays: number | null): TransitFitSetup {
  if (setup.period.trim() !== "" || !isFiniteNumber(periodDays) || periodDays <= 0) return setup;
  return { ...setup, period: String(periodDays) };
}

export interface TransitGeometry {
  period: number;
  t0: number;
  rp: number;
  a_rs: number;
  b: number;
  u1: number;
  u2: number;
}

export function fittedValue(result: TransitFitResult, name: TransitParameterName): number | null {
  return result.parameters.find((parameter) => parameter.name === name)?.value ?? null;
}

export function fittedError(result: TransitFitResult, name: TransitParameterName): number | null {
  return result.parameters.find((parameter) => parameter.name === name)?.err ?? null;
}

export function transitGeometryOf(result: TransitFitResult | null): TransitGeometry | null {
  if (result === null || result.kind !== "transit") return null;
  const t0 = fittedValue(result, "t0");
  const rp = fittedValue(result, "rp");
  const a_rs = fittedValue(result, "a_rs");
  const b = fittedValue(result, "b");
  const u1 = fittedValue(result, "u1");
  const u2 = fittedValue(result, "u2");
  if (t0 === null || rp === null || a_rs === null || b === null || u1 === null || u2 === null) return null;
  return { period: result.period, t0, rp, a_rs, b, u1, u2 };
}
