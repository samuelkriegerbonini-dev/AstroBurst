import type { LightCurveBin, LightCurveWindow } from "../shared/types/lightCurve";
import type { PeriodogramSearch } from "../services/periodogram";
import { parseDecimalText } from "./decimalText";
import { formatSignificant } from "./formatSignificant";
import { MISSING_VALUE } from "./formatUncertainty";

export const WINDOW_NEEDS_BOTH = "The transit window needs both a start and an end.";

const SAME_EDGE_DAYS = 1e-9;
const MINUTES_PER_DAY = 1440;
const MARGIN_ERROR = "margin is not a number of at least 0";

export interface WindowDraft {
  start: string;
  end: string;
}

export interface WindowInput {
  window: LightCurveWindow | null;
  error: string | null;
  apply: boolean;
  hint: string | null;
}

export interface MarginInput {
  marginDays: number;
  error: string | null;
  apply: boolean;
}

function refusedWindow(error: string): WindowInput {
  return { window: null, error, apply: false, hint: null };
}

export function windowFromDraft(draft: WindowDraft, offset: number): WindowInput {
  const startBlank = draft.start.trim() === "";
  const endBlank = draft.end.trim() === "";
  const start = parseDecimalText(draft.start);
  const end = parseDecimalText(draft.end);
  if (!startBlank && start === null) return refusedWindow("transit start is not a number");
  if (!endBlank && end === null) return refusedWindow("transit end is not a number");
  if (start === null || end === null) {
    return { window: null, error: null, apply: true, hint: startBlank && endBlank ? null : WINDOW_NEEDS_BOTH };
  }
  if (start > end) return refusedWindow("transit end is before transit start");
  return { window: { start: start + offset, end: end + offset }, error: null, apply: true, hint: null };
}

const MAX_AXIS_DECIMALS = 20;

function axisText(value: number, offset: number): string {
  const relative = value - offset;
  for (let decimals = 0; decimals <= MAX_AXIS_DECIMALS; decimals++) {
    const text = String(Number(relative.toFixed(decimals)));
    if (Number(text) + offset === value) return text;
  }
  return String(relative);
}

export function draftFromWindow(window: LightCurveWindow | null, offset: number): WindowDraft {
  if (window === null) return { start: "", end: "" };
  return { start: axisText(window.start, offset), end: axisText(window.end, offset) };
}

export function sameWindow(a: LightCurveWindow | null, b: LightCurveWindow | null): boolean {
  if (a === null || b === null) return a === b;
  return Math.abs(a.start - b.start) <= SAME_EDGE_DAYS && Math.abs(a.end - b.end) <= SAME_EDGE_DAYS;
}

export function marginFromDraft(text: string, indexScale: boolean): MarginInput {
  if (text.trim() === "") return { marginDays: 0, error: null, apply: true };
  const value = parseDecimalText(text);
  if (value === null || value < 0) return { marginDays: 0, error: MARGIN_ERROR, apply: false };
  return { marginDays: indexScale ? value : value / MINUTES_PER_DAY, error: null, apply: true };
}

export function draftFromMargin(marginDays: number, indexScale: boolean): string {
  if (marginDays === 0) return "";
  return indexScale ? String(marginDays) : String(Number((marginDays * MINUTES_PER_DAY).toFixed(6)));
}

export type BinKind = "none" | "points" | "minutes";

export interface BinInput {
  bin: LightCurveBin;
  error: string | null;
  apply: boolean;
}

export interface ClipInput {
  clipSigma: number | null;
  clipWindowPoints: number;
  error: string | null;
  apply: boolean;
}

const BLANK_BIN_SIZE = 5;
const BLANK_CLIP_WINDOW = 11;
const POINTS_BIN_ERROR = "bin size must be a whole number of at least 2";
const MINUTES_BIN_ERROR = "bin size must be a number of minutes above 0";
const CLIP_SIGMA_ERROR = "clip sigma must be a number above 0";
const CLIP_WINDOW_ERROR = "clip window must be a whole number of at least 3";

function refusedBin(error: string): BinInput {
  return { bin: { kind: "none" }, error, apply: false };
}

export function binFromDraft(kind: BinKind, sizeText: string): BinInput {
  if (kind === "none") return { bin: { kind: "none" }, error: null, apply: true };
  const size = sizeText.trim() === "" ? BLANK_BIN_SIZE : parseDecimalText(sizeText);
  if (kind === "points") {
    if (size === null || !Number.isInteger(size) || size < 2) return refusedBin(POINTS_BIN_ERROR);
    return { bin: { kind: "points", points: size }, error: null, apply: true };
  }
  if (size === null || size <= 0) return refusedBin(MINUTES_BIN_ERROR);
  return { bin: { kind: "minutes", minutes: size }, error: null, apply: true };
}

export function draftFromBin(bin: LightCurveBin): { kind: BinKind; size: string } {
  if (bin.kind === "points") return { kind: "points", size: String(bin.points) };
  if (bin.kind === "minutes") return { kind: "minutes", size: String(bin.minutes) };
  return { kind: "none", size: "" };
}

export function clipFromDraft(sigmaText: string, windowText: string): ClipInput {
  const sigma = sigmaText.trim() === "" ? null : parseDecimalText(sigmaText);
  if (sigmaText.trim() !== "" && (sigma === null || sigma <= 0)) {
    return { clipSigma: null, clipWindowPoints: BLANK_CLIP_WINDOW, error: CLIP_SIGMA_ERROR, apply: false };
  }
  const points = windowText.trim() === "" ? BLANK_CLIP_WINDOW : parseDecimalText(windowText);
  if (points === null || !Number.isInteger(points) || points < 3) {
    return { clipSigma: null, clipWindowPoints: BLANK_CLIP_WINDOW, error: CLIP_WINDOW_ERROR, apply: false };
  }
  return { clipSigma: sigma, clipWindowPoints: points, error: null, apply: true };
}

export interface PeriodRangeInput {
  minFrequency: number | null;
  maxFrequency: number | null;
  error: string | null;
}

export interface FoldInput {
  fold: { periodDays: number; t0: number } | null;
  error: string | null;
  apply: boolean;
}

const SHORTEST_PERIOD_ERROR = "shortest period must be a number of days above 0";
const LONGEST_PERIOD_ERROR = "longest period must be a number of days above 0";
const PERIOD_ORDER_ERROR = "the shortest period must be below the longest";
const FOLD_PERIOD_ERROR = "fold period must be a number of days above 0";
const FOLD_EPOCH_MISSING = "the fold needs an epoch";
const FOLD_EPOCH_ERROR = "fold epoch is not a number";
const FAP_EXPONENT_BELOW = 1e-4;

function positiveDays(text: string): number | null {
  const value = parseDecimalText(text);
  return value !== null && value > 0 ? value : null;
}

function refusedRange(error: string): PeriodRangeInput {
  return { minFrequency: null, maxFrequency: null, error };
}

export function periodRangeFromDraft(minPeriodText: string, maxPeriodText: string): PeriodRangeInput {
  const minBlank = minPeriodText.trim() === "";
  const maxBlank = maxPeriodText.trim() === "";
  const minPeriod = minBlank ? null : positiveDays(minPeriodText);
  const maxPeriod = maxBlank ? null : positiveDays(maxPeriodText);
  if (!minBlank && minPeriod === null) return refusedRange(SHORTEST_PERIOD_ERROR);
  if (!maxBlank && maxPeriod === null) return refusedRange(LONGEST_PERIOD_ERROR);
  if (minPeriod !== null && maxPeriod !== null && minPeriod >= maxPeriod) return refusedRange(PERIOD_ORDER_ERROR);
  return {
    minFrequency: maxPeriod === null ? null : 1 / maxPeriod,
    maxFrequency: minPeriod === null ? null : 1 / minPeriod,
    error: null,
  };
}

function refusedFold(error: string): FoldInput {
  return { fold: null, error, apply: false };
}

export function foldFromDraft(periodText: string, epochText: string, offset: number): FoldInput {
  const epochBlank = epochText.trim() === "";
  if (periodText.trim() === "" && epochBlank) return { fold: null, error: null, apply: true };
  const periodDays = positiveDays(periodText);
  if (periodDays === null) return refusedFold(FOLD_PERIOD_ERROR);
  if (epochBlank) return refusedFold(FOLD_EPOCH_MISSING);
  const epoch = parseDecimalText(epochText);
  if (epoch === null) return refusedFold(FOLD_EPOCH_ERROR);
  return { fold: { periodDays, t0: epoch + offset }, error: null, apply: true };
}

export function fapText(probability: number | null): string {
  if (probability === null) return MISSING_VALUE;
  if (probability === 0) return "0";
  return probability < FAP_EXPONENT_BELOW ? probability.toExponential(1) : formatSignificant(probability, 2);
}

export function periodStep(periodDays: number, frequencyStep: number): number {
  return periodDays * periodDays * frequencyStep;
}

type SearchLimits = Pick<PeriodogramSearch, "first_frequency" | "last_frequency" | "upper_limit" | "cadence_days" | "best_frequency">;

const RANGE_DIGITS = 4;
const ALIAS_SLACK = 1e-9;

function rangeDigits(value: number): string {
  return formatSignificant(value, RANGE_DIGITS);
}

export function periodogramRangeText(limits: SearchLimits): string {
  const searched = `Periods searched: ${rangeDigits(1 / limits.last_frequency)} to ${rangeDigits(1 / limits.first_frequency)} d.`;
  if (limits.upper_limit === "cadence_nyquist" && limits.cadence_days !== null) {
    const cadence = limits.cadence_days;
    return `${searched} The shortest is twice the regular cadence of ${rangeDigits(cadence)} d (Nyquist frequency ${rangeDigits(0.5 / cadence)} per day): shorter periods only repeat aliases of the sampling. Type a shortest period to search below it.`;
  }
  if (limits.upper_limit === "five_mean_nyquist") {
    return `${searched} The shortest is set by five times the mean Nyquist frequency (points / 2 baseline).`;
  }
  return searched;
}

export function periodogramAliasWarning(limits: SearchLimits): string | null {
  if (limits.cadence_days === null) return null;
  const nyquist = 0.5 / limits.cadence_days;
  if (limits.best_frequency <= nyquist * (1 + ALIAS_SLACK)) return null;
  return `The highest peak lies above ${rangeDigits(nyquist)} per day, the Nyquist frequency of the regular cadence: it can be an alias of the sampling, not a period of the source.`;
}
