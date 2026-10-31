import { describe, expect, it } from "vitest";
import { frameMapping, sharedHistogramFrame } from "../histogramFrame";

const R = { min: 0.001, max: 1.0 };
const G = { min: 0.004, max: 0.6 };
const B = { min: 0.002, max: 0.4 };

describe("sharedHistogramFrame", () => {
  it("spans the lowest minimum and the highest maximum of the channels, as the composite GHS normalises", () => {
    expect(sharedHistogramFrame([R, G, B])).toEqual({ min: 0.001, max: 1.0 });
  });

  it("is a lone channel's own frame", () => {
    expect(sharedHistogramFrame([G])).toEqual(G);
  });

  it("is null without a finite, non-empty range", () => {
    expect(sharedHistogramFrame([])).toBeNull();
    expect(sharedHistogramFrame([{ min: 0.5, max: 0.5 }])).toBeNull();
    expect(sharedHistogramFrame([{ min: Number.NaN, max: 1 }])).toBeNull();
  });
});

describe("frameMapping", () => {
  it("moves a value from its channel's 0 to 100 axis to the same value on the shared axis", () => {
    const shared = { min: 0.001, max: 1.0 };
    const mapping = frameMapping(B, shared);
    expect(mapping).not.toBeNull();
    for (const v of [0.002, 0.01, 0.2, 0.4]) {
      const own = ((v - B.min) / (B.max - B.min)) * 100;
      const onShared = ((v - shared.min) / (shared.max - shared.min)) * 100;
      expect((mapping?.scale ?? Number.NaN) * own + (mapping?.offset ?? Number.NaN)).toBeCloseTo(onShared, 9);
    }
  });

  it("is the identity for the channel that sets the shared frame", () => {
    const mapping = frameMapping(R, R);
    expect(mapping?.scale).toBeCloseTo(1, 12);
    expect(mapping?.offset).toBeCloseTo(0, 12);
  });

  it("is null when the shared frame has no range or the channel frame is not finite", () => {
    expect(frameMapping(R, { min: 1, max: 1 })).toBeNull();
    expect(frameMapping({ min: Number.NaN, max: 1 }, R)).toBeNull();
  });
});
