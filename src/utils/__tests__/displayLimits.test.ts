import { describe, it, expect } from "vitest";
import { normalizePercentiles, normalizeUserLimits } from "../displayLimits";

describe("normalizePercentiles", () => {
  it("passes ordered values through", () => {
    expect(normalizePercentiles(1, 99.5)).toEqual([1, 99.5]);
  });

  it("swaps when low > high", () => {
    expect(normalizePercentiles(99, 1)).toEqual([1, 99]);
  });

  it("clamps to 0..100", () => {
    expect(normalizePercentiles(-5, 120)).toEqual([0, 100]);
    expect(normalizePercentiles(120, -5)).toEqual([0, 100]);
  });

  it("nudges high when equal", () => {
    expect(normalizePercentiles(50, 50)).toEqual([50, 50.1]);
    expect(normalizePercentiles(100, 100)).toEqual([99.9, 100]);
  });

  it("replaces non-finite values with the defaults", () => {
    expect(normalizePercentiles(NaN, 99.5)).toEqual([1, 99.5]);
    expect(normalizePercentiles(1, Infinity)).toEqual([1, 99.5]);
  });
});

describe("normalizeUserLimits", () => {
  it("swaps when both finite and lo > hi", () => {
    expect(normalizeUserLimits(5, 1)).toEqual([1, 5]);
  });

  it("keeps ordered values", () => {
    expect(normalizeUserLimits(1, 5)).toEqual([1, 5]);
    expect(normalizeUserLimits(1, 1)).toEqual([1, 1]);
  });

  it("passes nulls through", () => {
    expect(normalizeUserLimits(null, null)).toEqual([null, null]);
    expect(normalizeUserLimits(5, null)).toEqual([5, null]);
    expect(normalizeUserLimits(null, 1)).toEqual([null, 1]);
  });

  it("turns non-finite numbers into null", () => {
    expect(normalizeUserLimits(NaN, 1)).toEqual([null, 1]);
    expect(normalizeUserLimits(2, Infinity)).toEqual([2, null]);
  });
});
