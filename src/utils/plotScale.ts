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
