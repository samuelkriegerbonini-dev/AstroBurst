import { describe, it, expect } from "vitest";
import { cellTone, columnIndices, formatCell, pixelTableCsv, rowIndices } from "../pixelTable";
import { CSV_LINE_END } from "../catalogCsv";
import type { PixelTableResult, PixelTableStats } from "../../shared/types/analysis";

const STATS: PixelTableStats = { min: 1, max: 9, mean: 4.5, median: 4, n_finite: 6, n_nan: 0 };

function result(overrides: Partial<PixelTableResult> = {}): PixelTableResult {
  return {
    x: 0,
    y: 1,
    size: 3,
    x0: -1,
    y0: 0,
    values: [
      [null, 1, 2],
      [null, 4, 5],
      [null, 8, 9],
    ],
    err: null,
    dq: null,
    dq_names: null,
    dq_table: null,
    unit: null,
    stats: STATS,
    elapsed_ms: 1,
    ...overrides,
  };
}

describe("pixelTableCsv", () => {
  it("writes the column x indices as the header and the y index first on every row, blanks for null cells", () => {
    const lines = pixelTableCsv(result()).split(CSV_LINE_END);
    expect(lines[0]).toBe("x\\y,-1,0,1");
    expect(lines[1]).toBe("0,,1,2");
    expect(lines[2]).toBe("1,,4,5");
    expect(lines[3]).toBe("2,,8,9");
    expect(lines[4]).toBe("");
    expect(lines).toHaveLength(5);
  });

  it("can export a different grid of the same shape, such as the ERR plane", () => {
    const err = [
      [null, 0.5, 1],
      [null, 2, 2.5],
      [null, 4, 4.5],
    ];
    const lines = pixelTableCsv(result({ err }), err).split(CSV_LINE_END);
    expect(lines[1]).toBe("0,,0.5,1");
    expect(lines[3]).toBe("2,,4,4.5");
  });

  it("derives the axis indices from the origin and size", () => {
    expect(columnIndices({ x0: 10, size: 5 })).toEqual([10, 11, 12, 13, 14]);
    expect(rowIndices({ y0: -2, size: 3 })).toEqual([-2, -1, 0]);
  });
});

describe("formatCell", () => {
  it("uses fixed notation inside [1e-3, 1e5) and exponential outside", () => {
    expect(formatCell(12.3456, 2)).toBe("12.35");
    expect(formatCell(0.001, 3)).toBe("0.001");
    expect(formatCell(99999.5, 1)).toBe("99999.5");
    expect(formatCell(1e5, 2)).toBe("1.0e+5");
    expect(formatCell(0.0005, 2)).toBe("5.0e-4");
    expect(formatCell(-2.5e7, 3)).toBe("-2.50e+7");
    expect(formatCell(-0.5, 2)).toBe("-0.50");
  });

  it("prints zero in fixed notation and dashes for null or non-finite values", () => {
    expect(formatCell(0, 2)).toBe("0.00");
    expect(formatCell(null, 2)).toBe("--");
    expect(formatCell(Number.NaN, 2)).toBe("--");
    expect(formatCell(Number.POSITIVE_INFINITY, 2)).toBe("--");
  });
});

describe("cellTone", () => {
  it("flags DQ before anything else, then null, max and min", () => {
    expect(cellTone(9, STATS, "SATURATED")).toBe("dq");
    expect(cellTone(null, STATS, "DO_NOT_USE")).toBe("dq");
    expect(cellTone(null, STATS, null)).toBe("nan");
    expect(cellTone(9, STATS, null)).toBe("max");
    expect(cellTone(1, STATS, null)).toBe("min");
    expect(cellTone(4, STATS, null)).toBe("normal");
  });

  it("treats a constant grid as all max and an empty grid as plain", () => {
    const flat: PixelTableStats = { ...STATS, min: 3, max: 3 };
    expect(cellTone(3, flat, null)).toBe("max");
    const empty: PixelTableStats = { min: null, max: null, mean: null, median: null, n_finite: 0, n_nan: 9 };
    expect(cellTone(2, empty, null)).toBe("normal");
  });
});
