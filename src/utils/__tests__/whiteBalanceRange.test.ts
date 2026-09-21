import { describe, it, expect } from "vitest";
import {
  wbSliderBounds,
  wbFactorsOutOfRange,
  WB_APPLY_MIN,
  WB_APPLY_MAX,
} from "../whiteBalanceRange";

describe("wbSliderBounds", () => {
  it("keeps the default range for neutral factors", () => {
    expect(wbSliderBounds(1, 1, 1)).toEqual({ min: 0.1, max: 3 });
  });

  it("grows with the factors", () => {
    expect(wbSliderBounds(1, 1, 4).max).toBe(6);
    expect(wbSliderBounds(1, 1, 8).max).toBe(12);
  });

  it("does not let a degenerate factor blow the slider away", () => {
    const bounds = wbSliderBounds(63671007156.37, 1, 63671007156.37);
    expect(bounds.max).toBe(WB_APPLY_MAX);
    expect(bounds.min).toBe(0.1);
  });

  it("never drops the floor below the applicable minimum", () => {
    expect(wbSliderBounds(0.0001, 1, 1).min).toBe(WB_APPLY_MIN);
    expect(wbSliderBounds(-5, 1, 1).min).toBe(WB_APPLY_MIN);
  });

  it("falls back to the default range when nothing is finite", () => {
    expect(wbSliderBounds(NaN, Infinity, NaN)).toEqual({ min: 0.1, max: 3 });
  });

  it("stays reachable for every factor the backend accepts", () => {
    for (const factor of [WB_APPLY_MIN, 0.02, 0.5, 1, 12, 25, 60, WB_APPLY_MAX]) {
      const bounds = wbSliderBounds(factor, 1, 1);
      expect(wbFactorsOutOfRange(factor, 1, 1)).toEqual([]);
      expect(bounds.min).toBeLessThanOrEqual(factor);
      expect(bounds.max).toBeGreaterThanOrEqual(factor);
    }
  });
});

describe("wbFactorsOutOfRange", () => {
  it("reports nothing for usable factors", () => {
    expect(wbFactorsOutOfRange(1, 1, 2.5)).toEqual([]);
  });

  it("names every channel outside the applicable range", () => {
    expect(wbFactorsOutOfRange(63671007156.37, 1, 63671007156.37)).toEqual(["R", "B"]);
    expect(wbFactorsOutOfRange(0, 1, 1)).toEqual(["R"]);
    expect(wbFactorsOutOfRange(1, NaN, 1)).toEqual(["G"]);
  });

  it("accepts the exact bounds the backend accepts", () => {
    expect(wbFactorsOutOfRange(WB_APPLY_MIN, WB_APPLY_MAX, 1)).toEqual([]);
  });
});
