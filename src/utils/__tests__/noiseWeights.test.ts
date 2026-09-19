import { describe, it, expect } from "vitest";
import { noiseWeightsFromSigmas, combineFrameWeights, formatWeightRange } from "../noiseWeights";

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

describe("formatWeightRange", () => {
  it("prints the range with two decimals and the neutral count", () => {
    expect(formatWeightRange({ weights: [0.5, 1.5], min: 0.5, max: 1.5, missing: 0 })).toBe("0.50 - 1.50");
    expect(formatWeightRange({ weights: [1, 1], min: 1, max: 1, missing: 1 })).toBe("1.00 - 1.00 (1 frame without a noise estimate)");
    expect(formatWeightRange({ weights: [1, 1], min: 1, max: 1, missing: 2 })).toBe("1.00 - 1.00 (2 frames without a noise estimate)");
  });
});
