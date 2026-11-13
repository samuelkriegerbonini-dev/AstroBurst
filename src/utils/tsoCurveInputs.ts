import type { TsoCurvesDraft } from "../hooks/useTsoStore";
import type { TsoCurve, TsoLightCurves, TsoLightCurvesRequest, TsoLimitsOrigin, TsoRejectedElement } from "../shared/types/tso";
import { parseDecimalText } from "./decimalText";
import { formatSignificant } from "./formatSignificant";
import { MISSING_VALUE } from "./formatUncertainty";
import { MAX_TSO_BINS, bandText, binsFromEdgesText, equalWidthBins, tsoTimeName } from "./tsoLightCurves";
import { isFiniteNumber } from "./values";

export type ExcludeInput = { ok: true; ranges: [number, number][] } | { ok: false; error: string };

export interface CurveRequestInput {
  request: TsoLightCurvesRequest | null;
  error: string | null;
}

const NUMBER_PATTERN = String.raw`[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?`;
const RANGE_PATTERN = new RegExp(String.raw`^(${NUMBER_PATTERN})\s*(?:-|to)\s*(${NUMBER_PATTERN})$`);
const SHOWN_DIGITS = 6;
const DEFAULT_REJECTED_LINES = 5;

const ORIGIN_TEXT: Record<TsoLimitsOrigin, string> = {
  open: "open limits",
  user: "your limits",
  "file log": "limits from the processing log of the file",
  "built-in default": "built-in limits of this mode",
};

export function excludeRangesFromText(text: string): ExcludeInput {
  const ranges: [number, number][] = [];
  for (const piece of text.split(/[;,]/).map((p) => p.trim()).filter((p) => p !== "")) {
    const match = RANGE_PATTERN.exec(piece);
    const lo = match ? Number(match[1]) : NaN;
    const hi = match ? Number(match[2]) : NaN;
    if (!Number.isFinite(lo) || !Number.isFinite(hi)) {
      return { ok: false, error: `exclude range '${piece}' is not two numbers written as a-b` };
    }
    if (lo > hi) return { ok: false, error: `exclude range '${piece}': the first number must not be above the second` };
    ranges.push([lo, hi]);
  }
  return { ok: true, ranges };
}

function failed(error: string): CurveRequestInput {
  return { request: null, error };
}

export function requestFromDraft(draft: TsoCurvesDraft): CurveRequestInput {
  const min = parseDecimalText(draft.minText);
  const max = parseDecimalText(draft.maxText);
  if (min === null && draft.minText.trim() !== "") return failed("the lower limit is not a number");
  if (max === null && draft.maxText.trim() !== "") return failed("the upper limit is not a number");
  if (min !== null && max !== null && min >= max) return failed("the lower limit must be below the upper one");

  const edges = binsFromEdgesText(draft.binsText);
  if (!edges.ok) return failed(`bin edges: ${edges.error}`);
  const bins: [number, number][] = [...edges.bins, ...draft.extraBins.map(([lo, hi]): [number, number] => [lo, hi])];
  if (bins.length > MAX_TSO_BINS) return failed(`at most ${MAX_TSO_BINS} bins`);

  let excludeUm: [number, number][] = [];
  let rejectSigma: number | null = null;
  if (draft.mode === "masked") {
    const exclude = excludeRangesFromText(draft.excludeText);
    if (!exclude.ok) return failed(exclude.error);
    excludeUm = exclude.ranges;
    if (draft.rejectOn) {
      rejectSigma = parseDecimalText(draft.rejectSigmaText);
      if (rejectSigma === null || rejectSigma <= 0) return failed("the rejection threshold must be a number above 0");
    }
  }

  return {
    request: {
      mode: draft.mode,
      wavelength_min_um: min,
      wavelength_max_um: max,
      bins,
      exclude_um: excludeUm,
      reject_elements_sigma: rejectSigma,
    },
    error: null,
  };
}

export function equalEdgesText(lo: number | null, hi: number | null, count: number): string {
  if (lo === null || hi === null) return "";
  const bins = equalWidthBins(lo, hi, count);
  if (bins.length === 0) return "";
  return [bins[0][0], ...bins.map((bin) => bin[1])].map((edge) => String(Number(edge.toPrecision(SHOWN_DIGITS)))).join(", ");
}

export function limitsLine(curve: Pick<TsoCurve, "band_um" | "limits_origin">): string {
  return `White light summed over ${bandText(curve.band_um)} (${ORIGIN_TEXT[curve.limits_origin]})`;
}

export function droppedLine(dropped: { nan_time: number; repeated_time: number }): string | null {
  if (dropped.nan_time === 0 && dropped.repeated_time === 0) return null;
  return `The pipeline rule left out ${dropped.nan_time} integration(s) with no MJD-AVG and ${dropped.repeated_time} with a repeated MJD-AVG.`;
}

export function rejectedLines(elements: readonly TsoRejectedElement[], max: number = DEFAULT_REJECTED_LINES): string[] {
  const lines = elements
    .slice(0, max)
    .map((element) => `Rejected element at ${formatSignificant(element.wavelength_um, SHOWN_DIGITS)} um (channel ${element.index})`);
  if (elements.length > max) lines.push(`and ${elements.length - max} more`);
  return lines;
}

function pointNumber(value: number | null | undefined): string {
  return isFiniteNumber(value) ? formatSignificant(value, SHOWN_DIGITS) : MISSING_VALUE;
}

export function selectedPointLine(result: TsoLightCurves, pointRef: number): string | null {
  const k = result.int_num.indexOf(pointRef);
  if (k < 0) return null;
  const flux = pointNumber(result.white.flux[k]);
  const unit = result.flux_unit === "unknown" ? "" : ` ${result.flux_unit}`;
  return (
    `integration ${result.row_index[k] + 1} of the series (INT_NUM ${pointRef}), ` +
    `${tsoTimeName(result.time_scale)} ${result.time[k]}, white light ${flux}${unit}, ` +
    `${result.white.n_used[k]} elements, mean NPIXELS ${pointNumber(result.white.npixels_mean[k])}`
  );
}
