import type { ScaleLimits } from "../shared/types/display";
import { STRETCH_KIND, lutIndex, normalize, stretchValue, type DisplayTransfer } from "./displayTransfer";

export const COLORBAR_HEIGHT_PX = 14;
export const COLORBAR_TICK_TARGET = 5;
export const COLORBAR_MIN_LABEL_GAP_PX = 44;

const NICE_MANTISSAS: readonly number[] = [1, 2, 2.5, 5, 10];
const INVERT_ITERATIONS = 48;
const STRETCH_TICKS_MIN = 5;
const STRETCH_TICKS_MAX = 7;
const STRETCH_TICK_SPACING_PX = 100;
const STRETCH_TICK_SLACK = 0.25;
const MIN_SIGNIFICANT = 2;
const MAX_SIGNIFICANT = 12;
const LABEL_CHAR_PX = 5.4;
const LABEL_PAD_PX = 4;

export interface ColorbarTick {
  value: number;
  frac: number;
  label: string;
  kind: "value" | "centre";
}

type TickMark = Pick<ColorbarTick, "value" | "label" | "kind">;

function cleanDecimal(x: number): number {
  const v = Number(x.toPrecision(12));
  return v === 0 ? 0 : v;
}

export function niceStep(span: number, target: number): number {
  const raw = span / Math.max(target - 1, 1);
  if (!Number.isFinite(raw) || !(raw > 0)) return 0;
  let k = Math.floor(Math.log10(raw));
  if (raw / 10 ** k >= 10) k += 1;
  if (raw / 10 ** k < 1) k -= 1;
  const norm = raw / 10 ** k;
  const m = NICE_MANTISSAS.find((c) => c >= norm - 1e-9) ?? 10;
  return cleanDecimal(k >= 0 ? m * 10 ** k : m / 10 ** -k);
}

export function niceTicks(lo: number, hi: number, target: number): number[] {
  if (!Number.isFinite(lo) || !Number.isFinite(hi)) return [];
  if (!(hi > lo)) return [lo];
  const step = niceStep(hi - lo, target);
  const out = [lo];
  if (step > 0) {
    const tol = step * 1e-6;
    const first = Math.ceil(lo / step - 1e-9);
    const last = Math.floor(hi / step + 1e-9);
    for (let i = first; i <= last; i++) {
      const v = cleanDecimal(i * step);
      if (v - lo > tol && hi - v > tol) out.push(v);
    }
  }
  out.push(hi);
  return out;
}

export function tickFraction(value: number, t: DisplayTransfer): number {
  return stretchValue(normalize(value, t.vmin, t.vmax), t);
}

export function invertStretch(y: number, t: DisplayTransfer): number {
  if (y !== y) return NaN;
  const target = y < 0 ? 0 : y > 1 ? 1 : y;
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < INVERT_ITERATIONS; i++) {
    const mid = (lo + hi) / 2;
    if (stretchValue(mid, t) < target) lo = mid;
    else hi = mid;
  }
  return (lo + hi) / 2;
}

export function valueAtFraction(frac: number, t: DisplayTransfer): number {
  return t.vmin + (t.vmax - t.vmin) * invertStretch(frac, t);
}

export function formatTickValue(value: number, span: number): string {
  if (!Number.isFinite(value)) return "";
  const abs = Math.abs(value);
  if (abs !== 0 && (abs < 1e-3 || abs >= 1e6)) return value.toExponential(2);
  const magnitude = Number.isFinite(span) && span > 0 ? Math.floor(Math.log10(span)) : 0;
  return value.toFixed(Math.min(4, Math.max(0, 2 - magnitude)));
}

function plainNumber(value: number): string {
  const abs = Math.abs(value);
  return abs !== 0 && (abs < 1e-3 || abs >= 1e6) ? value.toExponential() : String(value);
}

function stretchTickCount(widthPx: number): number {
  const fit = Number.isFinite(widthPx) ? Math.floor(widthPx / STRETCH_TICK_SPACING_PX) + 1 : STRETCH_TICKS_MIN;
  return Math.min(STRETCH_TICKS_MAX, Math.max(STRETCH_TICKS_MIN, fit));
}

function roundedTick(target: number, slack: number, t: DisplayTransfer): TickMark | null {
  if (t.vmin < 0 && t.vmax > 0 && Math.abs(tickFraction(0, t) - target) <= slack) return { value: 0, label: "0", kind: "value" };
  const raw = valueAtFraction(target, t);
  if (!Number.isFinite(raw)) return null;
  for (let digits = MIN_SIGNIFICANT; digits <= MAX_SIGNIFICANT; digits++) {
    const value = Number(raw.toPrecision(digits)) || 0;
    if (value > t.vmin && value < t.vmax && Math.abs(tickFraction(value, t) - target) <= slack) {
      return { value, label: plainNumber(value), kind: "value" };
    }
  }
  return null;
}

function linearMarks(t: DisplayTransfer, span: number): TickMark[] {
  return niceTicks(t.vmin, t.vmax, COLORBAR_TICK_TARGET).map((value) => ({ value, label: formatTickValue(value, span), kind: "value" }));
}

function endLabels(t: DisplayTransfer, span: number): [string, string] {
  const lo = formatTickValue(t.vmin, span);
  const hi = formatTickValue(t.vmax, span);
  if (lo !== hi) return [lo, hi];
  for (let digits = MIN_SIGNIFICANT; digits <= MAX_SIGNIFICANT; digits++) {
    const a = plainNumber(Number(t.vmin.toPrecision(digits)) || 0);
    const b = plainNumber(Number(t.vmax.toPrecision(digits)) || 0);
    if (a !== b) return [a, b];
  }
  return [lo, hi];
}

function stretchMarks(t: DisplayTransfer, widthPx: number, span: number): TickMark[] {
  const [loLabel, hiLabel] = endLabels(t, span);
  const first: TickMark = { value: t.vmin, label: loLabel, kind: "value" };
  const last: TickMark = { value: t.vmax, label: hiLabel, kind: "value" };
  const intervals = stretchTickCount(widthPx) - 1;
  const slack = STRETCH_TICK_SLACK / intervals;
  const marks = [first];
  for (let k = 1; k < intervals; k++) {
    const mark = roundedTick(k / intervals, slack, t);
    if (mark === null || mark.label === first.label || mark.label === last.label) continue;
    if (mark.value > marks[marks.length - 1].value) marks.push(mark);
  }
  marks.push(last);
  return marks;
}

function labelExtent(px: number, label: string, index: number, last: number): [number, number] {
  const width = label.length * LABEL_CHAR_PX;
  if (index === 0) return [px, px + width];
  if (index === last) return [px - width, px];
  return [px - width / 2, px + width / 2];
}

export function colorbarTicks(t: DisplayTransfer, widthPx: number, centre: number | null): ColorbarTick[] {
  if (!Number.isFinite(t.vmin) || !Number.isFinite(t.vmax)) return [];
  const span = t.vmax - t.vmin;
  const marks = t.stretchKind === STRETCH_KIND.linear || !(span > 0) ? linearMarks(t, span) : stretchMarks(t, widthPx, span);
  if (centre !== null && Number.isFinite(centre) && centre >= t.vmin && centre <= t.vmax) {
    const tol = Math.abs(span) * 1e-9;
    const hit = marks.find((m) => Math.abs(m.value - centre) <= tol);
    if (hit) hit.kind = "centre";
    else {
      marks.push({ value: centre, label: formatTickValue(centre, span), kind: "centre" });
      marks.sort((a, b) => a.value - b.value);
    }
  }
  const ticks: ColorbarTick[] = marks.map((m) => ({ value: m.value, frac: tickFraction(m.value, t), label: m.label, kind: m.kind }));
  const last = ticks.length - 1;
  const forced = (i: number) => i === 0 || i === last || ticks[i].kind === "centre";
  let prevPx = -Infinity;
  let prevEnd = -Infinity;
  for (let i = 0; i <= last; i++) {
    const px = ticks[i].frac * widthPx;
    const [start, end] = labelExtent(px, ticks[i].label, i, last);
    if (forced(i)) {
      prevPx = px;
      prevEnd = end;
      continue;
    }
    let next = i + 1;
    while (!forced(next)) next++;
    const nextPx = ticks[next].frac * widthPx;
    const [nextStart] = labelExtent(nextPx, ticks[next].label, next, last);
    const spaced = px - prevPx >= COLORBAR_MIN_LABEL_GAP_PX && nextPx - px >= COLORBAR_MIN_LABEL_GAP_PX;
    if (spaced && start >= prevEnd + LABEL_PAD_PX && end + LABEL_PAD_PX <= nextStart) {
      prevPx = px;
      prevEnd = end;
    } else ticks[i].label = "";
  }
  return ticks;
}

export function colorbarCentre(
  symmetricRequested: boolean,
  limits: Pick<ScaleLimits, "symmetric" | "centre"> | null,
): number | null {
  return symmetricRequested && limits?.symmetric === true ? limits.centre : null;
}

export function colorbarPixels(lut: Uint8Array, widthPx: number, invert: boolean): Uint8ClampedArray {
  const w = Math.max(0, Math.floor(widthPx));
  const out = new Uint8ClampedArray(w * 4);
  for (let x = 0; x < w; x++) {
    const src = lutIndex(w > 1 ? x / (w - 1) : 0, invert) * 4;
    const off = x * 4;
    out[off] = lut[src];
    out[off + 1] = lut[src + 1];
    out[off + 2] = lut[src + 2];
    out[off + 3] = lut[src + 3];
  }
  return out;
}

export function colorbarUnit(bunit: string | null | undefined, isProcessed: boolean): string | null {
  if (isProcessed || typeof bunit !== "string") return null;
  const text = bunit.trim().replace(/^'(.*)'$/, "$1").trim();
  return text === "" ? null : text;
}
