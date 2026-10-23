import { describe, it, expect } from "vitest";
import {
  UINT16_MAX,
  STATISTIC_ROWS,
  fitsSixteenBit,
  unitTransform,
  convertScaleValue,
  convertStatistics,
  formatStatistic,
  statisticValue,
  statisticsToCsv,
} from "../statisticsUnits";
import { SIGMA_MAD_LABEL, SIGMA_MAD_TITLE } from "../analysisLabels";
import type { ChannelStatistics } from "../../shared/types/statistics";
import statisticsPanelSource from "../../components/analysis/StatisticsPanel.tsx?raw";

const stats: ChannelStatistics = {
  count: 5,
  total: 6,
  fraction: 5 / 6,
  mean: 22,
  median: 3,
  avg_dev: 20.2,
  mad: 1,
  bwmv_sqrt: 1.5,
  min: 1,
  max: 100,
  sum: 110,
  variance: 1911.5,
  std_dev: Math.sqrt(1911.5),
  nan_count: 1,
  padding: 0,
  excluded: 0,
};

describe("unitTransform", () => {
  it("is the identity for raw units", () => {
    expect(unitTransform("raw", { min: 1, max: 100 })).toEqual({ scale: 1, offset: 0 });
  });

  it("maps the data range onto [0, 1] for normalized units", () => {
    const t = unitTransform("normalized", { min: 1, max: 100 });
    expect(t).not.toBeNull();
    expect(1 * t!.scale + t!.offset).toBeCloseTo(0, 12);
    expect(100 * t!.scale + t!.offset).toBeCloseTo(1, 12);
    expect(50.5 * t!.scale + t!.offset).toBeCloseTo(0.5, 12);
  });

  it("handles negative background-subtracted ranges", () => {
    const t = unitTransform("normalized", { min: -4, max: 4 })!;
    expect(0 * t.scale + t.offset).toBeCloseTo(0.5, 12);
  });

  it("returns null for degenerate or non-finite ranges", () => {
    expect(unitTransform("normalized", { min: 3, max: 3 })).toBeNull();
    expect(unitTransform("normalized", { min: NaN, max: 1 })).toBeNull();
    expect(unitTransform("normalized", { min: null, max: null })).toBeNull();
  });

  it("scales by 65535 only when the data already lives in [0, 1]", () => {
    expect(unitTransform("16bit", { min: 0.01, max: 0.9 })).toEqual({ scale: UINT16_MAX, offset: 0 });
    expect(unitTransform("16bit", { min: 1, max: 100 })).toBeNull();
    expect(unitTransform("16bit", { min: -0.1, max: 0.5 })).toBeNull();
  });
});

describe("fitsSixteenBit", () => {
  it("accepts ranges inside [0, 1] and rejects everything else", () => {
    expect(fitsSixteenBit({ min: 0, max: 1 })).toBe(true);
    expect(fitsSixteenBit({ min: 0.2, max: 0.3 })).toBe(true);
    expect(fitsSixteenBit({ min: 0, max: 1.0001 })).toBe(false);
    expect(fitsSixteenBit({ min: -1e-3, max: 0.5 })).toBe(false);
    expect(fitsSixteenBit({ min: null, max: 1 })).toBe(false);
  });
});

describe("convertScaleValue", () => {
  it("scales a dispersion value without shifting it", () => {
    expect(convertScaleValue(9.9, "normalized", { min: 1, max: 100 })).toBeCloseTo(0.1, 12);
    expect(convertScaleValue(0.5, "16bit", { min: 0, max: 1 })).toBe(0.5 * UINT16_MAX);
    expect(convertScaleValue(2.5, "raw", { min: 1, max: 100 })).toBe(2.5);
    expect(convertScaleValue(2.5, "16bit", { min: 1, max: 100 })).toBe(2.5);
  });
});

describe("convertStatistics", () => {
  it("shifts location statistics, scales dispersion statistics and leaves counts alone", () => {
    const range = { min: 1, max: 100 };
    const out = convertStatistics(stats, "normalized", range);
    expect(out.min).toBeCloseTo(0, 12);
    expect(out.max).toBeCloseTo(1, 12);
    expect(out.median).toBeCloseTo(2 / 99, 12);
    expect(out.mean).toBeCloseTo(21 / 99, 12);
    expect(out.mad).toBeCloseTo(1 / 99, 12);
    expect(out.avg_dev).toBeCloseTo(20.2 / 99, 12);
    expect(out.bwmv_sqrt).toBeCloseTo(1.5 / 99, 12);
    expect(out.std_dev).toBeCloseTo(stats.std_dev / 99, 12);
    expect(out.variance).toBeCloseTo(1911.5 / (99 * 99), 12);
    expect(out.sum).toBeCloseTo(105 / 99, 12);
    expect(out.count).toBe(5);
    expect(out.total).toBe(6);
    expect(out.fraction).toBe(5 / 6);
    expect(out.nan_count).toBe(1);
    expect(out.excluded).toBe(0);
  });

  it("falls back to raw values when the unit cannot be applied", () => {
    expect(convertStatistics(stats, "16bit", { min: 1, max: 100 })).toEqual(stats);
    expect(convertStatistics(stats, "normalized", { min: 3, max: 3 })).toEqual(stats);
  });

  it("multiplies by 65535 for 16-bit units", () => {
    const small: ChannelStatistics = { ...stats, min: 0, max: 0.5, median: 0.25, mad: 0.1, sum: 1.25 };
    const out = convertStatistics(small, "16bit", { min: 0, max: 0.5 });
    expect(out.median).toBeCloseTo(0.25 * UINT16_MAX, 9);
    expect(out.mad).toBeCloseTo(0.1 * UINT16_MAX, 9);
    expect(out.max).toBeCloseTo(0.5 * UINT16_MAX, 9);
  });
});

describe("formatStatistic", () => {
  it("prints counts as integers and values with unit-appropriate precision", () => {
    expect(formatStatistic(5, "count", "raw")).toBe("5");
    expect(formatStatistic(0.123456789, "location", "normalized")).toBe("0.123457");
    expect(formatStatistic(12345.678, "location", "16bit")).toBe("12345.7");
    expect(formatStatistic(1234.5678, "location", "raw")).toBe("1234.57");
    expect(formatStatistic(0.00001234, "scale", "raw")).toBe("1.234e-5");
    expect(formatStatistic(NaN, "location", "raw")).toBe("--");
    expect(formatStatistic(5 / 6, "fraction", "raw")).toBe("83.33%");
  });
});

describe("statisticsToCsv", () => {
  it("writes one row per statistic with a column per channel", () => {
    const csv = statisticsToCsv([{ label: "K", stats }], "raw");
    const lines = csv.split("\n");
    expect(lines[0]).toBe("statistic,K");
    expect(lines.length).toBe(STATISTIC_ROWS.length + 1);
    expect(lines.find((l) => l.startsWith("median,"))).toBe("median,3");
    expect(lines.find((l) => l.startsWith("count,"))).toBe("count,5");
  });

  it("reports the zero padding the backend leaves out, so the partition of a padded frame adds up", () => {
    const padded: ChannelStatistics = { ...stats, total: 10, fraction: 5 / 10, nan_count: 1, padding: 4, excluded: 0 };
    const accounted = STATISTIC_ROWS
      .filter((row) => row.kind === "count")
      .reduce((sum, row) => sum + statisticValue(padded, row.key), 0);
    expect(accounted).toBe(padded.total);
    const csv = statisticsToCsv([{ label: "K", stats: padded }], "raw").split("\n");
    expect(csv.find((l) => l.startsWith("padding,"))).toBe("padding,4");
  });

  it("converts units and handles three channels", () => {
    const csv = statisticsToCsv(
      [
        { label: "R", stats },
        { label: "G", stats },
        { label: "B", stats },
      ],
      "normalized",
      { min: 1, max: 100 },
    );
    const lines = csv.split("\n");
    expect(lines[0]).toBe("statistic,R,G,B");
    const minLine = lines.find((l) => l.startsWith("min,"))!;
    expect(minLine.split(",").slice(1).map(Number)).toEqual([0, 0, 0]);
    const maxLine = lines.find((l) => l.startsWith("max,"))!;
    expect(maxLine.split(",").slice(1).map(Number)).toEqual([1, 1, 1]);
  });
});

describe("robust sigma row (UX-35)", () => {
  function csvRow(csv: string, key: string): number[] | undefined {
    const line = csv.split("\n").find((l) => l.startsWith(`${key},`));
    return line?.split(",").slice(1).map(Number);
  }

  function expectRelativelyEqual(actual: number, expected: number) {
    expect(Math.abs(actual - expected) / Math.abs(expected)).toBeLessThan(1e-12);
  }

  it("writes a sigma_mad row equal to 1.4826 × the mad row", () => {
    const csv = statisticsToCsv(
      [
        { label: "R", stats },
        { label: "G", stats: { ...stats, mad: 0.0123 } },
        { label: "B", stats: { ...stats, mad: 2.7e-7 } },
      ],
      "raw",
    );
    const mad = csvRow(csv, "mad")!;
    const sigma = csvRow(csv, "sigma_mad");
    expect(sigma).toBeDefined();
    expect(sigma).toHaveLength(3);
    sigma!.forEach((value, i) => expectRelativelyEqual(value, 1.4826 * mad[i]));
  });

  it("converts the robust sigma like MAD in normalized and 16-bit units", () => {
    const normalized = statisticsToCsv([{ label: "K", stats }], "normalized", { min: 1, max: 100 });
    expectRelativelyEqual(csvRow(normalized, "sigma_mad")![0], (1.4826 * 1) / 99);
    const small: ChannelStatistics = { ...stats, min: 0, max: 0.5, mad: 0.1 };
    const sixteen = statisticsToCsv([{ label: "K", stats: small }], "16bit", { min: 0, max: 0.5 });
    expectRelativelyEqual(csvRow(sixteen, "sigma_mad")![0], 1.4826 * 0.1 * UINT16_MAX);
  });

  it("puts the row right after MAD with the shared label and title, as a scale statistic", () => {
    const madAt = STATISTIC_ROWS.findIndex((row) => row.key === "mad");
    const row = STATISTIC_ROWS[madAt + 1];
    expect(row.key).toBe("sigma_mad");
    expect(row.label).toBe(SIGMA_MAD_LABEL);
    expect(row.title).toBe(SIGMA_MAD_TITLE);
    expect(row.kind).toBe("scale");
    const lines = statisticsToCsv([{ label: "K", stats }], "raw").split("\n");
    expect(lines.findIndex((l) => l.startsWith("sigma_mad,"))).toBe(lines.findIndex((l) => l.startsWith("mad,")) + 1);
  });

  it("reads table values by row key, deriving the robust sigma from MAD", () => {
    expect(statisticValue(stats, "median")).toBe(3);
    expect(statisticValue({ ...stats, mad: 2 }, "sigma_mad")).toBe(1.4826 * 2);
    expect(statisticValue({ ...stats, mad: Number.NaN }, "sigma_mad")).toBeNaN();
  });
});

describe("StatisticsPanel table wiring (UX-35)", () => {
  it("reads every cell through statisticValue, so the sigma_mad row reaches the table", () => {
    expect(statisticsPanelSource).toContain("formatStatistic(statisticValue(s, row.key), row.kind, unit)");
    expect(statisticsPanelSource).not.toMatch(/s\[row\.key\]/);
  });

  it("tags each row with its key and gives the label cell the row title", () => {
    expect(statisticsPanelSource).toContain("data-statistic={row.key}");
    expect(statisticsPanelSource).toMatch(/<td className="[^"]*" title=\{row\.title\}>\s*\{row\.label\}/);
  });
});
