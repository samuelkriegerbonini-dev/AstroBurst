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

export function effectiveWeightRange(weightsApplied: readonly (number | null)[]): string | null {
  const stacked = weightsApplied.filter((w): w is number => typeof w === "number" && Number.isFinite(w));
  if (stacked.length < 2) return null;
  const mean = stacked.reduce((sum, w) => sum + w, 0) / stacked.length;
  if (!(mean > 0)) return null;
  const normalised = stacked.map((w) => w / mean);
  return `${Math.min(...normalised).toFixed(2)} - ${Math.max(...normalised).toFixed(2)}`;
}

export interface NoiseCoverage {
  missing: number;
  total: number;
}

export interface EffectiveWeights {
  range: string;
  frames: number;
  subframeCount: number;
  noise: NoiseCoverage;
}

export function weightedFrameCount(subframe: readonly number[] | undefined): number {
  return subframe ? subframe.filter((w) => w !== 1.0).length : 0;
}

export function effectiveWeightsOf(
  weightsApplied: readonly (number | null)[],
  subframe: readonly number[] | undefined,
  noise: NoiseWeightSummary,
): EffectiveWeights | null {
  const range = effectiveWeightRange(weightsApplied);
  if (range === null) return null;
  return {
    range,
    frames: weightsApplied.filter((w) => typeof w === "number" && Number.isFinite(w)).length,
    subframeCount: weightedFrameCount(subframe),
    noise: { missing: noise.missing, total: noise.weights.length },
  };
}

export function noiseWeightHint(range: string | null, frames: number, subframeCount: number, noise?: NoiseCoverage): string {
  if (range === null) return "Noise weights are measured when you stack.";
  const subframes = subframeCount > 0 ? ", multiplied by the subframe weights" : "";
  const sentence = `Effective frame weights ${range} for the ${frames} frames stacked (noise weights divided by the normalization scale², normalised to mean 1)${subframes}.`;
  if (!noise || noise.missing <= 0) return sentence;
  if (noise.missing >= noise.total) return `${sentence} No frame had a noise estimate, so noise weighting was not applied.`;
  return `${sentence} ${noise.missing} of the ${noise.total} frames had no noise estimate and kept the mean noise weight.`;
}
