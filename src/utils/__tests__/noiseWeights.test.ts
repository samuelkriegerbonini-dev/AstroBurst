import { describe, it, expect } from "vitest";
import { noiseWeightsFromSigmas, combineFrameWeights, effectiveWeightRange, effectiveWeightsOf, noiseWeightHint } from "../noiseWeights";

describe("noiseWeightsFromSigmas", () => {
  it("weights frames by inverse variance and normalises to mean one", () => {
    const summary = noiseWeightsFromSigmas([1, 2, 4]);
    const raw = [1, 1 / 4, 1 / 16];
    const mean = raw.reduce((s, w) => s + w, 0) / raw.length;
    summary.weights.forEach((w, i) => expect(w).toBeCloseTo(raw[i] / mean, 10));
    expect(summary.weights.reduce((s, w) => s + w, 0) / 3).toBeCloseTo(1, 10);
    expect(summary.min).toBeCloseTo(Math.min(...summary.weights), 10);
    expect(summary.max).toBeCloseTo(Math.max(...summary.weights), 10);
    expect(summary.missing).toBe(0);
    expect(summary.weights[0]).toBeGreaterThan(summary.weights[1]);
  });

  it("gives frames without a usable sigma a neutral weight and counts them", () => {
    const summary = noiseWeightsFromSigmas([2, null, 2, Number.NaN, 0, -1]);
    expect(summary.weights).toEqual([1, 1, 1, 1, 1, 1]);
    expect(summary.missing).toBe(4);
  });

  it("falls back to unit weights when no frame has a sigma", () => {
    const summary = noiseWeightsFromSigmas([null, null]);
    expect(summary.weights).toEqual([1, 1]);
    expect(summary.min).toBe(1);
    expect(summary.max).toBe(1);
    expect(summary.missing).toBe(2);
    expect(noiseWeightsFromSigmas([]).weights).toEqual([]);
  });
});

describe("combineFrameWeights", () => {
  it("multiplies noise weights with subframe weights when both exist", () => {
    expect(combineFrameWeights([0.5, 2], [2, 0.25])).toEqual([1, 0.5]);
  });

  it("returns the noise weights alone without subframe weights", () => {
    expect(combineFrameWeights(undefined, [0.5, 2])).toEqual([0.5, 2]);
  });

  it("treats a missing subframe entry as one", () => {
    expect(combineFrameWeights([0.5], [2, 3])).toEqual([1, 3]);
  });
});

describe("effectiveWeightRange", () => {
  it("drops excluded frames and renormalises the backend weights to mean one", () => {
    expect(effectiveWeightRange([0.4, null, 0.4, 1.2])).toBe("0.60 - 1.80");
  });

  it("needs at least two stacked frames", () => {
    expect(effectiveWeightRange([null, 0.5])).toBeNull();
    expect(effectiveWeightRange([])).toBeNull();
  });
});

describe("noiseWeightHint", () => {
  it("names the effective range and the number of frames stacked", () => {
    expect(noiseWeightHint("0.71 - 1.32", 12, 0)).toBe(
      "Effective frame weights 0.71 - 1.32 for the 12 frames stacked (noise weights divided by the normalization scale², normalised to mean 1).",
    );
  });

  it("says when the subframe weights were multiplied in", () => {
    expect(noiseWeightHint("0.71 - 1.32", 12, 3)).toBe(
      "Effective frame weights 0.71 - 1.32 for the 12 frames stacked (noise weights divided by the normalization scale², normalised to mean 1), multiplied by the subframe weights.",
    );
  });

  it("is the idle sentence before a stack has reported weights", () => {
    expect(noiseWeightHint(null, 0, 0)).toBe("Noise weights are measured when you stack.");
    expect(noiseWeightHint(null, 3, 2)).toBe("Noise weights are measured when you stack.");
  });

  it("keeps the pinned sentence when every frame had a noise estimate", () => {
    expect(noiseWeightHint("0.60 - 1.80", 3, 0, { missing: 0, total: 3 })).toBe(
      "Effective frame weights 0.60 - 1.80 for the 3 frames stacked (noise weights divided by the normalization scale², normalised to mean 1).",
    );
  });

  it("names the frames that had no noise estimate and kept the mean noise weight", () => {
    expect(noiseWeightHint("0.71 - 1.32", 12, 3, { missing: 2, total: 12 })).toBe(
      "Effective frame weights 0.71 - 1.32 for the 12 frames stacked (noise weights divided by the normalization scale², normalised to mean 1), multiplied by the subframe weights. 2 of the 12 frames had no noise estimate and kept the mean noise weight.",
    );
  });

  it("says noise weighting was not applied when no frame had a noise estimate", () => {
    expect(noiseWeightHint("0.95 - 1.05", 3, 0, { missing: 3, total: 3 })).toBe(
      "Effective frame weights 0.95 - 1.05 for the 3 frames stacked (noise weights divided by the normalization scale², normalised to mean 1). No frame had a noise estimate, so noise weighting was not applied.",
    );
  });
});

describe("effectiveWeightsOf", () => {
  const noise = noiseWeightsFromSigmas([1, null, 2, 2]);

  it("keeps the stack-time subframe weights and noise coverage with the backend weights", () => {
    expect(effectiveWeightsOf([0.4, null, 0.4, 1.2], [1, 0.5, 0.8, 1], noise)).toEqual({
      range: "0.60 - 1.80",
      frames: 3,
      subframeCount: 2,
      noise: { missing: 1, total: 4 },
    });
  });

  it("counts no subframe weighting without subframe weights", () => {
    expect(effectiveWeightsOf([0.4, null, 0.4, 1.2], undefined, noise)?.subframeCount).toBe(0);
    expect(effectiveWeightsOf([0.4, null, 0.4, 1.2], [1, 1, 1, 1], noise)?.subframeCount).toBe(0);
  });

  it("reports nothing when fewer than two frames were stacked", () => {
    expect(effectiveWeightsOf([0.4, null, null, null], [1, 0.5, 0.8, 1], noise)).toBeNull();
  });
});
