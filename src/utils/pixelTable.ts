import type { PixelTableResult, PixelTableStats } from "../shared/types/analysis";
import { buildCsv, type CsvColumn } from "./catalogCsv";

export type CellTone = "normal" | "max" | "min" | "nan" | "dq";

export type PixelTablePlane = "SCI" | "ERR";

export interface DisplayedPlane {
  plane: PixelTablePlane;
  grid: (number | null)[][];
  stats: PixelTableStats;
}

export const PIXEL_TABLE_SIZES = [3, 5, 7, 9, 11, 13, 15] as const;
export const DEFAULT_PIXEL_TABLE_SIZE = 7;

const CSV_CORNER_HEADER = "y\\x";
const FIXED_MIN_ABS = 1e-3;
const FIXED_MAX_ABS = 1e5;
const NULL_TEXT = "--";

export function columnIndices(result: Pick<PixelTableResult, "x0" | "size">): number[] {
  return Array.from({ length: result.size }, (_, i) => result.x0 + i);
}

export function rowIndices(result: Pick<PixelTableResult, "y0" | "size">): number[] {
  return Array.from({ length: result.size }, (_, i) => result.y0 + i);
}

export function pixelTableCsv(result: PixelTableResult, grid: (number | null)[][] = result.values): string {
  const columns: CsvColumn<number>[] = [
    { header: CSV_CORNER_HEADER, value: (rowIndex) => result.y0 + rowIndex },
    ...columnIndices(result).map((x, col) => ({ header: String(x), value: (rowIndex: number) => grid[rowIndex]?.[col] ?? null })),
  ];
  return buildCsv(columns, Array.from({ length: result.size }, (_, i) => i));
}

export function formatCell(v: number | null, digits: number): string {
  if (v === null || !Number.isFinite(v)) return NULL_TEXT;
  if (v === 0) return (0).toFixed(digits);
  const a = Math.abs(v);
  if (a < FIXED_MIN_ABS || a >= FIXED_MAX_ABS) return v.toExponential(Math.max(0, digits - 1));
  return v.toFixed(Math.max(digits, digits - 1 - Math.floor(Math.log10(a))));
}

export function displayedPlane(result: PixelTableResult, showErr: boolean): DisplayedPlane {
  if (showErr && result.err && result.err_stats) return { plane: "ERR", grid: result.err, stats: result.err_stats };
  return { plane: "SCI", grid: result.values, stats: result.stats };
}

export function cellTone(v: number | null, stats: PixelTableStats, dqNames: string | null): CellTone {
  if (dqNames) return "dq";
  if (v === null || !Number.isFinite(v)) return "nan";
  if (stats.max !== null && v === stats.max) return "max";
  if (stats.min !== null && v === stats.min) return "min";
  return "normal";
}
