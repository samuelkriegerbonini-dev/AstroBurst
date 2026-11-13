import { describe, expect, it } from "vitest";
import { DEFAULT_BAND_COLOR, visibleBands } from "../plotBands";

describe("visibleBands", () => {
  it("orders, clips and filters the bands for the visible domain", () => {
    const bands = [
      { x0: 3, x1: 2, label: "in transit" },
      { x0: -5, x1: 0.5, color: "red" },
      { x0: 9, x1: 12 },
      { x0: 4, x1: 4 },
      { x0: NaN, x1: 2 },
      { x0: 20, x1: 30 },
      { x0: 1, x1: 2, label: "" },
    ];
    const expected = [
      { x0: 2, x1: 3, color: DEFAULT_BAND_COLOR, label: "in transit" },
      { x0: 0, x1: 0.5, color: "red", label: null },
      { x0: 9, x1: 10, color: DEFAULT_BAND_COLOR, label: null },
      { x0: 1, x1: 2, color: DEFAULT_BAND_COLOR, label: null },
    ];
    expect(visibleBands(bands, [0, 10])).toEqual(expected);
    expect(visibleBands(bands, [10, 0])).toEqual(expected);
    expect(visibleBands([], [0, 1])).toEqual([]);
  });
});
