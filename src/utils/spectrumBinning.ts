export const BINNING_BASIS =
  "flux = mean of the samples used in a bin; err = sqrt(sum err^2) / n_used, the channels treated as independent";

export interface BinnedSpectrum {
  factor: number;
  channels: number;
  first: number[];
  last: number[];
  nUsed: number[];
  flux: number[];
  err: number[] | null;
  nExcluded: number;
  nNonFinite: number;
}

export function effectiveBinFactor(factor: number, channels: number): number {
  if (!Number.isFinite(factor)) return 1;
  return Math.min(Math.max(Math.floor(factor), 1), Math.max(channels, 1));
}

export function binCount(channels: number, factor: number): number {
  return Math.ceil(channels / effectiveBinFactor(factor, channels));
}

export function binSpectrum(
  flux: readonly number[],
  err: readonly number[] | null,
  excluded: readonly boolean[] | null,
  factor: number,
): BinnedSpectrum {
  const channels = flux.length;
  const f = effectiveBinFactor(factor, channels);
  const bins = binCount(channels, f);
  const errors = err !== null && err.length === channels ? err : null;
  const mask = excluded !== null && excluded.length === channels ? excluded : null;
  const first = new Array<number>(bins);
  const last = new Array<number>(bins);
  const nUsed = new Array<number>(bins);
  const binnedFlux = new Array<number>(bins);
  const binnedErr = errors === null ? null : new Array<number>(bins);
  let nExcluded = 0;
  let nNonFinite = 0;
  for (let k = 0; k < bins; k++) {
    const start = k * f;
    const end = Math.min(channels, start + f);
    let used = 0;
    let fluxSum = 0;
    let squaredErrSum = 0;
    for (let i = start; i < end; i++) {
      if (mask !== null && mask[i]) {
        nExcluded++;
        continue;
      }
      const value = flux[i];
      if (!Number.isFinite(value)) {
        nNonFinite++;
        continue;
      }
      used++;
      fluxSum += value;
      if (errors !== null) {
        const e = errors[i];
        squaredErrSum += Number.isFinite(e) && e > 0 ? e * e : NaN;
      }
    }
    first[k] = start;
    last[k] = end - 1;
    nUsed[k] = used;
    binnedFlux[k] = used > 0 ? fluxSum / used : NaN;
    if (binnedErr !== null) binnedErr[k] = used > 0 ? Math.sqrt(squaredErrSum) / used : NaN;
  }
  return { factor: f, channels, first, last, nUsed, flux: binnedFlux, err: binnedErr, nExcluded, nNonFinite };
}

export function binAxis(values: readonly number[], factor: number): number[] {
  const channels = values.length;
  const f = effectiveBinFactor(factor, channels);
  const bins = binCount(channels, f);
  const centres = new Array<number>(bins);
  for (let k = 0; k < bins; k++) {
    const start = k * f;
    const end = Math.min(channels, start + f);
    let finite = 0;
    let total = 0;
    for (let i = start; i < end; i++) {
      const value = values[i];
      if (!Number.isFinite(value)) continue;
      finite++;
      total += value;
    }
    centres[k] = finite > 0 ? total / finite : NaN;
  }
  return centres;
}

export function binningNote(binned: BinnedSpectrum): string {
  if (binned.factor === 1) return "";
  const bins = binned.flux.length;
  const lastWidth = bins > 0 ? binned.last[bins - 1] - binned.first[bins - 1] + 1 : binned.factor;
  const shortLast =
    lastWidth < binned.factor ? `, the last holds ${lastWidth} ${lastWidth === 1 ? "channel" : "channels"}` : "";
  return `binned by ${binned.factor}: ${bins} ${bins === 1 ? "bin" : "bins"}${shortLast}; mean of the unmasked samples, errors assume independent channels`;
}
