import { describe, it, expect } from "vitest";
import { niceTicks, linearScale, finiteExtent } from "../plotScale";

describe("niceTicks", () => {
  it("chooses round steps covering the domain", () => {
    expect(niceTicks(0, 97, 5)).toEqual([0, 20, 40, 60, 80, 100]);
    expect(niceTicks(0, 1, 5)).toEqual([0, 0.2, 0.4, 0.6, 0.8, 1]);
    expect(niceTicks(-3, 3, 4)).toEqual([-4, -2, 0, 2, 4]);
  });

  it("handles degenerate and reversed inputs", () => {
    expect(niceTicks(5, 5, 5)).toEqual([5]);
    expect(niceTicks(97, 0, 5)).toEqual([0, 20, 40, 60, 80, 100]);
    expect(niceTicks(NaN, 1, 5)).toEqual([]);
  });
});

describe("linearScale", () => {
  it("maps the domain ends to the range ends", () => {
    const s = linearScale([10, 20], [0, 100]);
    expect(s(10)).toBe(0);
    expect(s(20)).toBe(100);
    expect(s(15)).toBe(50);
    const inv = linearScale([0, 1], [200, 0]);
    expect(inv(0)).toBe(200);
    expect(inv(1)).toBe(0);
  });

  it("returns the range midpoint for a degenerate domain", () => {
    expect(linearScale([3, 3], [0, 10])(3)).toBe(5);
  });
});

describe("finiteExtent", () => {
  it("ignores nulls and non-finite values", () => {
    expect(finiteExtent([null, 3, NaN, -1, Infinity, 7])).toEqual([-1, 7]);
  });

  it("returns null when nothing is finite", () => {
    expect(finiteExtent([])).toBeNull();
    expect(finiteExtent([null, NaN])).toBeNull();
  });
});
