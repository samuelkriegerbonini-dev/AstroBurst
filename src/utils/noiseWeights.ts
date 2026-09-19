export interface NoiseWeightSummary {
  weights: number[];
  min: number;
  max: number;
  missing: number;
}

function usableSigma(sigma: number | null | undefined): sigma is number {
  return typeof sigma === "number" && Number.isFinite(sigma) && sigma > 0;
}

export function noiseWeightsFromSigmas(sigmas: (number | null | undefined)[]): NoiseWeightSummary {
  const raw = sigmas.map((sigma) => (usableSigma(sigma) ? 1 / (sigma * sigma) : null));
  const valid = raw.filter((w): w is number => w !== null);
  const missing = raw.length - valid.length;
  if (valid.length === 0) {
    return { weights: sigmas.map(() => 1), min: 1, max: 1, missing };
  }
  const mean = valid.reduce((sum, w) => sum + w, 0) / valid.length;
  const weights = raw.map((w) => (w === null ? 1 : w / mean));
  return { weights, min: Math.min(...weights), max: Math.max(...weights), missing };
}

export function combineFrameWeights(subframe: number[] | undefined, noise: number[]): number[] {
  if (!subframe) return noise;
  return noise.map((w, i) => w * (subframe[i] ?? 1));
}

export function formatWeightRange(summary: NoiseWeightSummary): string {
  const range = `${summary.min.toFixed(2)} - ${summary.max.toFixed(2)}`;
  if (summary.missing === 0) return range;
  const noun = summary.missing === 1 ? "frame" : "frames";
  return `${range} (${summary.missing} ${noun} without a noise estimate)`;
}
