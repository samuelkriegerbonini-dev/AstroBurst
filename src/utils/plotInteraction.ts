import type { PlotHit, ProfileSeries } from "../components/regions/ProfilePlot";
import { buildCsv, type CsvColumn } from "./catalogCsv";

export type Domain = [number, number];

export const MIN_DOMAIN_FRACTION = 1e-9;
export const LOG_FLOOR_FRACTION = 1e-3;
const LOG_MINOR_MAX_DECADES = 3;
const LOG_MINOR_MULTIPLIERS = [2, 5];

function ordered(domain: Domain): Domain {
  return domain[0] <= domain[1] ? [domain[0], domain[1]] : [domain[1], domain[0]];
}

export function clampDomain(domain: Domain, extent: Domain): Domain {
  const [e0, e1] = ordered(extent);
  if (!Number.isFinite(e0) || !Number.isFinite(e1)) return [e0, e1];
  const [d0, d1] = ordered(domain);
  if (!Number.isFinite(d0) || !Number.isFinite(d1)) return [e0, e1];
  const width = d1 - d0;
  if (width >= e1 - e0) return [e0, e1];
  if (d0 < e0) return [e0, e0 + width];
  if (d1 > e1) return [e1 - width, e1];
  return [d0, d1];
}

export function zoomDomain(domain: Domain, extent: Domain, anchor: number, factor: number): Domain {
  const [e0, e1] = ordered(extent);
  const extentWidth = e1 - e0;
  if (!(extentWidth > 0) || !Number.isFinite(extentWidth)) return [e0, e1];
  const [d0, d1] = ordered(domain);
  const width = d1 - d0;
  if (!(width > 0) || !Number.isFinite(width) || !(factor > 0) || !Number.isFinite(factor)) return [e0, e1];
  const minWidth = extentWidth * MIN_DOMAIN_FRACTION;
  const nextWidth = Math.min(extentWidth, Math.max(minWidth, width * factor));
  if (nextWidth >= extentWidth) return [e0, e1];
  const a = Number.isFinite(anchor) ? anchor : (d0 + d1) / 2;
  const t = Math.min(1, Math.max(0, (a - d0) / width));
  const n0 = a - t * nextWidth;
  return clampDomain([n0, n0 + nextWidth], [e0, e1]);
}

export function panDomain(domain: Domain, extent: Domain, delta: number): Domain {
  const [d0, d1] = ordered(domain);
  const shift = Number.isFinite(delta) ? delta : 0;
  return clampDomain([d0 + shift, d1 + shift], extent);
}

export function logDomain(extent: Domain): Domain | null {
  const [lo, hi] = ordered(extent);
  if (!Number.isFinite(hi) || hi <= 0) return null;
  const floor = Number.isFinite(lo) && lo > 0 ? lo : hi * LOG_FLOOR_FRACTION;
  return [floor, hi];
}

export function logTicks(lo: number, hi: number): number[] {
  const [a, b] = ordered([lo, hi]);
  if (!(a > 0) || !Number.isFinite(a) || !Number.isFinite(b)) return [];
  const la = Math.log10(a);
  const lb = Math.log10(b);
  const withMinors = lb - la < LOG_MINOR_MAX_DECADES;
  const ticks: number[] = [];
  const first = Math.floor(la);
  const last = Math.ceil(lb);
  for (let k = first; k <= last; k++) {
    const decade = 10 ** k;
    if (decade >= a && decade <= b) ticks.push(decade);
    if (!withMinors) continue;
    for (const m of LOG_MINOR_MULTIPLIERS) {
      const v = m * decade;
      if (v >= a && v <= b) ticks.push(v);
    }
  }
  ticks.sort((p, q) => p - q);
  return ticks;
}

export function errorBarExtent(y: (number | null)[], yErr?: (number | null)[]): Domain | null {
  let lo = Infinity;
  let hi = -Infinity;
  for (let i = 0; i < y.length; i++) {
    const v = y[i];
    if (v === null || v === undefined || !Number.isFinite(v)) continue;
    const e = yErr?.[i];
    const bar = e !== null && e !== undefined && Number.isFinite(e) ? Math.abs(e) : 0;
    if (v - bar < lo) lo = v - bar;
    if (v + bar > hi) hi = v + bar;
  }
  return lo <= hi ? [lo, hi] : null;
}

export function seriesYExtent(
  series: { x: number[]; y: (number | null)[]; yErr?: (number | null)[] }[],
  logY: boolean,
  xDomain: Domain | null,
): Domain | null {
  let lo = Infinity;
  let hi = -Infinity;
  const consider = (v: number) => {
    if (!Number.isFinite(v)) return;
    if (logY && v <= 0) return;
    if (v < lo) lo = v;
    if (v > hi) hi = v;
  };
  for (const s of series) {
    for (let i = 0; i < s.y.length; i++) {
      const v = s.y[i];
      if (v === null || v === undefined || !Number.isFinite(v)) continue;
      const x = s.x[i];
      if (xDomain && Number.isFinite(x) && (x < xDomain[0] || x > xDomain[1])) continue;
      consider(v);
      const e = s.yErr?.[i];
      if (e !== null && e !== undefined && Number.isFinite(e)) {
        consider(v - Math.abs(e));
        consider(v + Math.abs(e));
      }
    }
  }
  return lo <= hi ? [lo, hi] : null;
}

export function nearestHit(
  series: { x: number[]; y: (number | null)[] }[],
  sx: (v: number) => number,
  sy: (v: number) => number,
  px: number,
  py: number,
  maxDistPx: number,
  pointsMode: boolean[],
  xDomain: Domain | null = null,
): PlotHit | null {
  let best: PlotHit | null = null;
  let bestPrimary = Infinity;
  let bestSecondary = Infinity;
  let bestIsPoint = false;
  for (let si = 0; si < series.length; si++) {
    const s = series[si];
    const euclid = pointsMode[si] === true;
    if (bestIsPoint && !euclid) continue;
    const n = Math.min(s.x.length, s.y.length);
    for (let i = 0; i < n; i++) {
      const y = s.y[i];
      if (y === null || y === undefined || !Number.isFinite(y)) continue;
      const x = s.x[i];
      if (xDomain && (x < xDomain[0] || x > xDomain[1])) continue;
      const cx = sx(x);
      const cy = sy(y);
      if (!Number.isFinite(cx) || !Number.isFinite(cy)) continue;
      const dx = Math.abs(cx - px);
      const dy = Math.abs(cy - py);
      const primary = euclid ? Math.hypot(dx, dy) : dx;
      const secondary = euclid ? 0 : dy;
      if (primary > maxDistPx) continue;
      const pointDisplacesLine = euclid && !bestIsPoint;
      if (pointDisplacesLine || primary < bestPrimary || (primary === bestPrimary && secondary < bestSecondary)) {
        bestPrimary = primary;
        bestSecondary = secondary;
        bestIsPoint = euclid;
        best = { seriesIndex: si, index: i, x, y };
      }
    }
  }
  return best;
}

export interface ErrorBarSpan {
  lower: number | null;
  upper: number;
}

export function errorBarSpan(v: number, e: number, logY: boolean): ErrorBarSpan | null {
  if (!Number.isFinite(v) || !Number.isFinite(e)) return null;
  if (logY && v <= 0) return null;
  const bar = Math.abs(e);
  const lower = v - bar;
  return { lower: logY && lower <= 0 ? null : lower, upper: v + bar };
}

export function zoomScopeKey(series: { x: number[] }[], xLabel: string): string {
  const ext = finiteExtentOf(series.flatMap((s) => s.x));
  return ext ? `${xLabel}|${ext[0]}|${ext[1]}` : `${xLabel}|`;
}

export const NO_DATA_MESSAGE = "no data";
export const NO_POSITIVE_LOG_MESSAGE = "no positive values for log y";

export function logHiddenCount(series: { y: (number | null)[] }[]): { hidden: number; total: number } {
  let hidden = 0;
  let total = 0;
  for (const s of series) {
    for (const v of s.y) {
      if (v === null || v === undefined || !Number.isFinite(v)) continue;
      total++;
      if (v <= 0) hidden++;
    }
  }
  return { hidden, total };
}

export function emptyPlotMessage(
  series: { x: number[]; y: (number | null)[]; yErr?: (number | null)[] }[],
  logY: boolean,
): string {
  if (logY && seriesYExtent(series, false, null) !== null && seriesYExtent(series, true, null) === null) {
    return NO_POSITIVE_LOG_MESSAGE;
  }
  return NO_DATA_MESSAGE;
}

export type PngSaveOutcome = { kind: "saved"; path: string } | { kind: "cancelled" } | { kind: "failed"; message: string };

export const PNG_ENCODE_FAILED_MESSAGE = "The plot could not be encoded as a PNG image.";

export async function savePngWith(
  pickTarget: () => Promise<string | null>,
  encode: () => Promise<Uint8Array | null>,
  write: (path: string, bytes: Uint8Array) => Promise<void>,
): Promise<PngSaveOutcome> {
  try {
    const target = await pickTarget();
    if (!target) return { kind: "cancelled" };
    const bytes = await encode();
    if (!bytes) return { kind: "failed", message: PNG_ENCODE_FAILED_MESSAGE };
    await write(target, bytes);
    return { kind: "saved", path: target };
  } catch (e: unknown) {
    return { kind: "failed", message: e instanceof Error ? e.message : String(e) };
  }
}

interface CsvRow {
  x: number;
  values: (number | null)[];
  errs: (number | null)[];
}

function sameX(series: ProfileSeries[]): boolean {
  if (series.length === 0) return true;
  const ref = series[0].x;
  for (const s of series) {
    if (s.x.length !== ref.length) return false;
    for (let i = 0; i < ref.length; i++) if (!Object.is(s.x[i], ref[i])) return false;
  }
  return true;
}

function cellOf(values: (number | null)[] | undefined, i: number): number | null {
  const v = values?.[i];
  return v === null || v === undefined || !Number.isFinite(v) ? null : v;
}

export function seriesToCsv(series: ProfileSeries[], xLabel: string): string {
  const columns: CsvColumn<CsvRow>[] = [{ header: xLabel, value: (r) => r.x }];
  series.forEach((s, si) => {
    columns.push({ header: s.label, value: (r) => r.values[si] });
  });
  series.forEach((s, si) => {
    if (s.yErr) columns.push({ header: `${s.label}_err`, value: (r) => r.errs[si] });
  });
  const rows: CsvRow[] = [];
  if (sameX(series)) {
    const n = series[0]?.x.length ?? 0;
    for (let i = 0; i < n; i++) {
      rows.push({
        x: series[0].x[i],
        values: series.map((s) => cellOf(s.y, i)),
        errs: series.map((s) => cellOf(s.yErr, i)),
      });
    }
  } else {
    series.forEach((s, si) => {
      for (let i = 0; i < s.x.length; i++) {
        const values: (number | null)[] = series.map(() => null);
        const errs: (number | null)[] = series.map(() => null);
        values[si] = cellOf(s.y, i);
        errs[si] = cellOf(s.yErr, i);
        rows.push({ x: s.x[i], values, errs });
      }
    });
  }
  return buildCsv(columns, rows);
}

function finiteExtentOf(values: number[]): Domain | null {
  let lo = Infinity;
  let hi = -Infinity;
  for (const v of values) {
    if (!Number.isFinite(v)) continue;
    if (v < lo) lo = v;
    if (v > hi) hi = v;
  }
  return lo <= hi ? [lo, hi] : null;
}

export const PLOT_PNG_BACKGROUND = "#0a0a0f";
const PLOT_PNG_MIN_SCALE = 2;

export function plotPngScale(devicePixelRatio: number | undefined): number {
  if (devicePixelRatio === undefined || !Number.isFinite(devicePixelRatio) || devicePixelRatio <= 0) return PLOT_PNG_MIN_SCALE;
  return Math.max(PLOT_PNG_MIN_SCALE, devicePixelRatio);
}

function summaryNumber(v: number): string {
  const a = Math.abs(v);
  if (a === 0) return "0";
  if (a >= 1e5 || a < 1e-3) return v.toExponential(3);
  return String(Number(v.toPrecision(5)));
}

function seriesSentence(s: { x: number[]; y: (number | null)[]; label: string }, xLabel: string, yLabel: string): string {
  let count = 0;
  let x0 = Infinity;
  let x1 = -Infinity;
  let y0 = Infinity;
  let y1 = -Infinity;
  const n = Math.min(s.x.length, s.y.length);
  for (let i = 0; i < n; i++) {
    const x = s.x[i];
    const y = s.y[i];
    if (y === null || !Number.isFinite(x) || !Number.isFinite(y)) continue;
    count++;
    x0 = Math.min(x0, x);
    x1 = Math.max(x1, x);
    y0 = Math.min(y0, y);
    y1 = Math.max(y1, y);
  }
  if (count === 0) return `${s.label}: no data.`;
  return `${s.label}: ${count} points, ${xLabel} ${summaryNumber(x0)} to ${summaryNumber(x1)}, ${yLabel} ${summaryNumber(y0)} to ${summaryNumber(y1)}.`;
}

export function plotSummary(
  series: readonly { x: number[]; y: (number | null)[]; label: string }[],
  xLabel: string,
  yLabel: string,
  selected: { seriesIndex: number; index: number } | null = null,
): string {
  if (series.length === 0) return "No data.";
  const text = [`${series.length} series.`, ...series.map((s) => seriesSentence(s, xLabel, yLabel))].join(" ");
  if (!selected) return text;
  const picked = series[selected.seriesIndex];
  const x = picked?.x[selected.index];
  const y = picked?.y[selected.index];
  if (!picked || x === undefined || y === undefined || y === null || !Number.isFinite(x) || !Number.isFinite(y)) return text;
  return `${text} Selected point of ${picked.label}: ${xLabel} ${summaryNumber(x)}, ${yLabel} ${summaryNumber(y)}.`;
}
