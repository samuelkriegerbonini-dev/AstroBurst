import { describe, it, expect } from "vitest";
import {
  niceTicks,
  linearScale,
  finiteExtent,
  formatAxisTick,
  formatTickLabels,
  formatLogTickLabel,
  tickLabels,
} from "../plotScale";

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

describe("formatAxisTick", () => {
  it("keeps Jy/beam scale spectra readable instead of collapsing to 0.0", () => {
    const yMin = -1.5e-4;
    const yMax = 3.2e-3;
    const range = yMax - yMin;
    const ticks = [0, 1, 2, 3, 4].map((i) => formatAxisTick(yMax - (i / 4) * range, range));
    expect(ticks).toEqual(["3.20e-3", "2.36e-3", "1.53e-3", "6.88e-4", "-1.50e-4"]);
    expect(new Set(ticks).size).toBe(ticks.length);
  });

  it("uses fixed notation with a precision derived from the range", () => {
    expect(formatAxisTick(1234, 2000)).toBe("1234");
    expect(formatAxisTick(0.25, 1)).toBe("0.25");
    expect(formatAxisTick(1.5, 0.04)).toBe("1.5000");
  });

  it("handles zero, huge values and non-finite input", () => {
    expect(formatAxisTick(0, 3e-3)).toBe("0");
    expect(formatAxisTick(4.2e6, 1e6)).toBe("4.20e+6");
    expect(formatAxisTick(NaN, 1)).toBe("");
    expect(formatAxisTick(2, 0)).toBe("2.00");
  });
});

describe("formatTickLabels", () => {
  it("labels a flat light curve axis at the tick step instead of repeating values", () => {
    expect(formatTickLabels(niceTicks(-1.2105, -1.1895, 5))).toEqual([
      "-1.215",
      "-1.210",
      "-1.205",
      "-1.200",
      "-1.195",
      "-1.190",
      "-1.185",
    ]);
  });

  it("keeps zoomed, sub-0.01, large-value and deep-zoom ticks distinct and readable back to the tick", () => {
    const axes: [number, number, number][] = [
      [15.0, 15.12, 6],
      [0, 0.0049, 6],
      [100000, 100500, 5],
      [-0.012, 0.009, 5],
      [1.0e-4, 1.1e-4, 5],
      [2, 2.01, 6],
      [15.2, 15.20000003, 6],
      [0, 0.0009, 5],
    ];
    for (const [lo, hi, n] of axes) {
      const ticks = niceTicks(lo, hi, n);
      const step = ticks[1] - ticks[0];
      const labels = formatTickLabels(ticks);
      expect(new Set(labels).size).toBe(ticks.length);
      labels.forEach((l, i) => expect(Math.abs(Number(l) - ticks[i])).toBeLessThanOrEqual(step * 1e-6));
    }
  });

  it("uses no more decimals than the step needs", () => {
    expect(formatTickLabels(niceTicks(0, 97, 5))).toEqual(["0", "20", "40", "60", "80", "100"]);
    expect(formatTickLabels(niceTicks(0, 0.0049, 6))).toEqual(["0", "0.001", "0.002", "0.003", "0.004", "0.005"]);
    expect(formatTickLabels([15, 15.02, 15.04])).toEqual(["15.00", "15.02", "15.04"]);
  });

  it("falls back to a range-free label for a single tick", () => {
    expect(formatTickLabels([5])).toEqual(["5.00"]);
    expect(formatTickLabels([])).toEqual([]);
  });
});

describe("formatLogTickLabel", () => {
  it("labels log ticks by value and never as 0.00", () => {
    expect([1e-5, 1e-3, 2e-3, 5e-3, 0.01, 1, 1000, 2e5].map(formatLogTickLabel)).toEqual([
      "1e-5",
      "0.001",
      "0.002",
      "0.005",
      "0.01",
      "1",
      "1000",
      "2e+5",
    ]);
  });

  it("returns an empty label for a non-positive value", () => {
    expect(formatLogTickLabel(0)).toBe("");
    expect(formatLogTickLabel(-1)).toBe("");
  });
});

describe("tickLabels", () => {
  it("passes the tick spacing to a custom formatter so it can size its decimals", () => {
    const steps: number[] = [];
    const labels = tickLabels([0.503, 0.5035, 0.504], (v, step) => {
      steps.push(step);
      return v.toFixed(Math.ceil(-Math.log10(step) - 1e-6));
    });
    expect(labels).toEqual(["0.5030", "0.5035", "0.5040"]);
    steps.forEach((s) => expect(s).toBeCloseTo(0.0005, 12));
  });

  it("passes NaN as the step when there is a single tick", () => {
    expect(tickLabels([0.5], (v, step) => `${v}:${step}`)).toEqual(["0.5:NaN"]);
  });

  it("uses the step-aware default when no formatter is given", () => {
    expect(tickLabels(niceTicks(-1.2105, -1.1895, 5))).toEqual(formatTickLabels(niceTicks(-1.2105, -1.1895, 5)));
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
