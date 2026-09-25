function niceStep(raw: number): number {
  if (!(raw > 0) || !Number.isFinite(raw)) return 1;
  const exp = Math.floor(Math.log10(raw));
  const base = 10 ** exp;
  const f = raw / base;
  let nf: number;
  if (f <= 1.5) nf = 1;
  else if (f <= 3) nf = 2;
  else if (f <= 7) nf = 5;
  else nf = 10;
  return nf * base;
}

export function niceTicks(min: number, max: number, count: number): number[] {
  if (!Number.isFinite(min) || !Number.isFinite(max)) return [];
  const lo = Math.min(min, max);
  const hi = Math.max(min, max);
  const n = Math.max(2, Math.floor(count));
  if (hi === lo) return [lo];
  const step = niceStep((hi - lo) / (n - 1));
  const start = Math.floor(lo / step) * step;
  const end = Math.ceil(hi / step) * step;
  const ticks: number[] = [];
  const decimals = Math.max(0, -Math.floor(Math.log10(step)) + 1);
  for (let k = 0; ; k++) {
    const v = Number((start + k * step).toFixed(decimals));
    if (v > end + step * 1e-9) break;
    ticks.push(v === 0 ? 0 : v);
    if (ticks.length > 1000) break;
  }
  return ticks;
}

export function linearScale(domain: [number, number], range: [number, number]): (v: number) => number {
  const [d0, d1] = domain;
  const [r0, r1] = range;
  const span = d1 - d0;
  if (span === 0 || !Number.isFinite(span)) return () => (r0 + r1) / 2;
  const k = (r1 - r0) / span;
  return (v: number) => r0 + (v - d0) * k;
}

const TICK_EXPONENTIAL_BELOW = 1e-2;
const TICK_EXPONENTIAL_ABOVE = 1e5;
const TICK_EXPONENTIAL_DIGITS = 2;
const TICK_EXTRA_DECIMALS = 2;
const TICK_MAX_DECIMALS = 6;

export function formatAxisTick(value: number, range: number): string {
  if (!Number.isFinite(value)) return "";
  if (value === 0) return "0";
  const span = Number.isFinite(range) && range > 0 ? range : Math.abs(value);
  const magnitude = Math.max(Math.abs(value), span);
  if (magnitude < TICK_EXPONENTIAL_BELOW || magnitude >= TICK_EXPONENTIAL_ABOVE) {
    return value.toExponential(TICK_EXPONENTIAL_DIGITS);
  }
  const decimals = Math.min(
    TICK_MAX_DECIMALS,
    Math.max(0, Math.ceil(-Math.log10(span)) + TICK_EXTRA_DECIMALS),
  );
  return value.toFixed(decimals);
}

const TICK_FIXED_MIN = 1e-3;
const STEP_LABEL_MAX_DECIMALS = 12;
const STEP_LOG_EPSILON = 1e-9;

export function formatTickLabels(ticks: number[]): string[] {
  let step = Infinity;
  let maxAbs = 0;
  for (let i = 0; i < ticks.length; i++) {
    if (Number.isFinite(ticks[i])) maxAbs = Math.max(maxAbs, Math.abs(ticks[i]));
    if (i === 0) continue;
    const d = Math.abs(ticks[i] - ticks[i - 1]);
    if (d > 0 && d < step) step = d;
  }
  if (!Number.isFinite(step)) return ticks.map((v) => formatAxisTick(v, NaN));
  const stepExp = Math.floor(Math.log10(step) + STEP_LOG_EPSILON);
  const exponential = maxAbs >= TICK_EXPONENTIAL_ABOVE || maxAbs < TICK_FIXED_MIN;
  return ticks.map((v) => {
    if (!Number.isFinite(v)) return "";
    if (v === 0) return "0";
    if (!exponential) return v.toFixed(Math.min(STEP_LABEL_MAX_DECIMALS, Math.max(0, -stepExp)));
    const digits = Math.floor(Math.log10(Math.abs(v)) + STEP_LOG_EPSILON) - stepExp;
    return v.toExponential(Math.min(STEP_LABEL_MAX_DECIMALS, Math.max(1, digits)));
  });
}

export function tickLabels(ticks: number[], format?: (v: number, step: number) => string): string[] {
  if (!format) return formatTickLabels(ticks);
  const step = ticks.length > 1 ? ticks[1] - ticks[0] : NaN;
  return ticks.map((t) => format(t, step));
}

export function formatLogTickLabel(v: number): string {
  if (!(v > 0) || !Number.isFinite(v)) return "";
  if (v >= TICK_EXPONENTIAL_ABOVE || v < TICK_FIXED_MIN) return v.toExponential(0);
  return String(Number(v.toPrecision(1)));
}

export function finiteExtent(values: (number | null)[]): [number, number] | null {
  let lo = Infinity;
  let hi = -Infinity;
  for (const v of values) {
    if (v === null || !Number.isFinite(v)) continue;
    if (v < lo) lo = v;
    if (v > hi) hi = v;
  }
  return lo <= hi ? [lo, hi] : null;
}
