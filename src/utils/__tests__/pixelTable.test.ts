import { describe, it, expect } from "vitest";
import {
  DEFAULT_PIXEL_TABLE_SIZE,
  PIXEL_TABLE_SIZES,
  cellTone,
  columnIndices,
  displayedPlane,
  formatCell,
  pixelTableCsv,
  rowIndices,
} from "../pixelTable";
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
    err_stats: null,
    elapsed_ms: 1,
    ...overrides,
  };
}

describe("pixelTableCsv", () => {
  it("writes the column x indices as the header and the y index first on every row, blanks for null cells", () => {
    const lines = pixelTableCsv(result()).split(CSV_LINE_END);
    expect(lines[0]).toBe("y\\x,-1,0,1");
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

  it("labels the corner row axis before the backslash and column axis after it, matching the rows and header it writes", () => {
    const r = result();
    const lines = pixelTableCsv(r).split(CSV_LINE_END);
    const [corner, ...headers] = lines[0].split(",");
    const [rowAxis, colAxis] = corner.split("\\");
    expect(rowAxis).toBe("y");
    expect(colAxis).toBe("x");
    expect(headers).toEqual(columnIndices(r).map(String));
    expect(lines.slice(1, 1 + r.size).map((l) => l.split(",")[0])).toEqual(rowIndices(r).map(String));
  });
});

describe("PIXEL_TABLE_SIZES", () => {
  it("offers every odd grid size the pixel_table_cmd boundary accepts (3 to 15)", () => {
    const backendAccepted = Array.from({ length: 7 }, (_, i) => 3 + 2 * i);
    expect([...PIXEL_TABLE_SIZES]).toEqual(backendAccepted);
    expect(PIXEL_TABLE_SIZES).toContain(DEFAULT_PIXEL_TABLE_SIZE);
  });
});

describe("displayedPlane", () => {
  const ERR_STATS: PixelTableStats = { min: 0, max: 4.5, mean: 2.25, median: 2.25, n_finite: 6, n_nan: 0 };
  const ERR = [
    [null, 0, 0.5],
    [null, 2, 2.5],
    [null, 4, 4.5],
  ];

  it("shows the ERR grid with the ERR plane's own stats when ERR is requested and present", () => {
    const r = result({ err: ERR, err_stats: ERR_STATS });
    expect(displayedPlane(r, true)).toEqual({ plane: "ERR", grid: ERR, stats: ERR_STATS });
  });

  it("shows the science grid and science stats when ERR is off or the image has no ERR plane", () => {
    const r = result({ err: ERR, err_stats: ERR_STATS });
    expect(displayedPlane(r, false)).toEqual({ plane: "SCI", grid: r.values, stats: STATS });
    const plain = result();
    expect(displayedPlane(plain, true)).toEqual({ plane: "SCI", grid: plain.values, stats: STATS });
  });
});

describe("formatCell", () => {
  it("uses fixed notation inside [1e-3, 1e5) and exponential outside", () => {
    expect(formatCell(12.3456, 2)).toBe("12.35");
    expect(formatCell(0.001, 3)).toBe("0.00100");
    expect(formatCell(99999.5, 1)).toBe("99999.5");
    expect(formatCell(1e5, 2)).toBe("1.0e+5");
    expect(formatCell(0.0005, 2)).toBe("5.0e-4");
    expect(formatCell(-2.5e7, 3)).toBe("-2.50e+7");
    expect(formatCell(-0.5, 2)).toBe("-0.50");
  });

  it("keeps at least the requested significant figures inside the fixed range, matching the exponential branch below 1e-3", () => {
    expect(formatCell(0.00149, 3)).toBe("0.00149");
    expect(formatCell(0.00101, 3)).toBe("0.00101");
    expect(formatCell(0.00449, 3)).toBe("0.00449");
    expect(formatCell(0.0123, 3)).toBe("0.0123");
    expect(formatCell(-0.00149, 3)).toBe("-0.00149");
    expect(formatCell(0.00099, 3)).toBe("9.90e-4");
    expect(formatCell(12.3456, 3)).toBe("12.346");
    expect(formatCell(0.00149, 4)).toBe("0.001490");
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
