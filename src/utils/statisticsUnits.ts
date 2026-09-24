import type { ChannelStatistics, DataRange, StatisticsUnit } from "../shared/types/statistics";

export const UINT16_MAX = 65535;

export type StatisticKind = "count" | "fraction" | "location" | "scale" | "variance" | "sum";

export interface StatisticRow {
  key: keyof ChannelStatistics;
  label: string;
  kind: StatisticKind;
}

export const STATISTIC_ROWS: readonly StatisticRow[] = [
  { key: "count", label: "count", kind: "count" },
  { key: "fraction", label: "count (%)", kind: "fraction" },
  { key: "mean", label: "mean", kind: "location" },
  { key: "median", label: "median", kind: "location" },
  { key: "avg_dev", label: "avgDev", kind: "scale" },
  { key: "mad", label: "MAD", kind: "scale" },
  { key: "bwmv_sqrt", label: "sqrt(BWMV)", kind: "scale" },
  { key: "std_dev", label: "stdDev", kind: "scale" },
  { key: "variance", label: "variance", kind: "variance" },
  { key: "min", label: "minimum", kind: "location" },
  { key: "max", label: "maximum", kind: "location" },
  { key: "sum", label: "sum", kind: "sum" },
  { key: "nan_count", label: "NaN", kind: "count" },
  { key: "padding", label: "padding (0)", kind: "count" },
  { key: "excluded", label: "excluded", kind: "count" },
];

export interface UnitTransform {
  scale: number;
  offset: number;
}

export const UNIT_LABELS: Record<StatisticsUnit, string> = {
  raw: "raw",
  normalized: "[0, 1]",
  "16bit": "16-bit",
};

function finiteRange(range: DataRange): { min: number; max: number } | null {
  const { min, max } = range;
  if (min == null || max == null || !Number.isFinite(min) || !Number.isFinite(max)) return null;
  return { min, max };
}

export function fitsSixteenBit(range: DataRange): boolean {
  const r = finiteRange(range);
  return r !== null && r.min >= 0 && r.max <= 1;
}

export function unitTransform(unit: StatisticsUnit, range: DataRange): UnitTransform | null {
  if (unit === "raw") return { scale: 1, offset: 0 };
  if (unit === "16bit") return fitsSixteenBit(range) ? { scale: UINT16_MAX, offset: 0 } : null;
  const r = finiteRange(range);
  if (!r || !(r.max > r.min)) return null;
  const span = r.max - r.min;
  return { scale: 1 / span, offset: -r.min / span };
}

interface Converters {
  location: (v: number) => number;
  scale: (v: number) => number;
}

function converters(unit: StatisticsUnit, range: DataRange): Converters | null {
  if (unit === "raw") return { location: (v) => v, scale: (v) => v };
  if (unit === "16bit") {
    if (!fitsSixteenBit(range)) return null;
    return { location: (v) => v * UINT16_MAX, scale: (v) => v * UINT16_MAX };
  }
  const r = finiteRange(range);
  if (!r || !(r.max > r.min)) return null;
  const span = r.max - r.min;
  return { location: (v) => (v - r.min) / span, scale: (v) => v / span };
}

export function convertScaleValue(value: number, unit: StatisticsUnit, range: DataRange): number {
  const c = converters(unit, range);
  return c ? c.scale(value) : value;
}

export function convertStatistics(
  stats: ChannelStatistics,
  unit: StatisticsUnit,
  range: DataRange,
): ChannelStatistics {
  const c = converters(unit, range);
  if (!c) return stats;
  const sumOffset = c.location(0);
  const unitScale = c.scale(1);
  return {
    ...stats,
    mean: c.location(stats.mean),
    median: c.location(stats.median),
    min: c.location(stats.min),
    max: c.location(stats.max),
    avg_dev: c.scale(stats.avg_dev),
    mad: c.scale(stats.mad),
    bwmv_sqrt: c.scale(stats.bwmv_sqrt),
    std_dev: c.scale(stats.std_dev),
    variance: stats.variance * unitScale * unitScale,
    sum: c.scale(stats.sum) + stats.count * sumOffset,
  };
}

function trimZeros(s: string): string {
  if (!s.includes(".") || s.includes("e")) return s;
  return s.replace(/\.?0+$/, "");
}

export function formatStatistic(value: number, kind: StatisticKind, unit: StatisticsUnit): string {
  if (!Number.isFinite(value)) return "--";
  if (kind === "count") return String(Math.round(value));
  if (kind === "fraction") return `${(value * 100).toFixed(2)}%`;
  if (unit === "normalized") return value.toFixed(6);
  if (unit === "16bit") return value.toFixed(1);
  if (value === 0) return "0";
  const magnitude = Math.abs(value);
  if (magnitude < 1e-4 || magnitude >= 1e9) return value.toExponential(3);
  return trimZeros(value.toPrecision(6));
}

export interface ChannelColumn {
  label: string;
  stats: ChannelStatistics;
}

export function statisticsToCsv(
  channels: ChannelColumn[],
  unit: StatisticsUnit,
  range: DataRange = { min: null, max: null },
): string {
  const converted = channels.map((c) => convertStatistics(c.stats, unit, range));
  const header = ["statistic", ...channels.map((c) => c.label)].join(",");
  const rows = STATISTIC_ROWS.map((row) => [row.key, ...converted.map((s) => String(s[row.key]))].join(","));
  return [header, ...rows].join("\n");
}
