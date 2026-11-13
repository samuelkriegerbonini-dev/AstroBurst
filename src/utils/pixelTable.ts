import type { PixelTableResult, PixelTableStats } from "../shared/types/analysis";
import type { PixelTableLayers } from "../shared/types/layers";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "./catalogCsv";
import { exportEntries, provenanceLines, unitLines, withProvenance, type ProvenanceEntry } from "./csvProvenance";

export type CellTone = "normal" | "max" | "min" | "nan" | "dq";

export type PixelTablePlane = "SCI" | "ERR" | "WAVELENGTH";

export const PIXEL_TABLE_PLANES: readonly PixelTablePlane[] = ["SCI", "ERR", "WAVELENGTH"];

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
const CELL_DIGITS = 3;
const WAVELENGTH_SIGNIFICANT = 6;

export function columnIndices(result: Pick<PixelTableResult, "x0" | "size">): number[] {
  return Array.from({ length: result.size }, (_, i) => result.x0 + i);
}

export function rowIndices(result: Pick<PixelTableResult, "y0" | "size">): number[] {
  return Array.from({ length: result.size }, (_, i) => result.y0 + i);
}

export interface PixelCsvContext {
  file: string | null;
  plane: PixelTablePlane;
  exportedUtc: string;
}

const PIXEL_COORDINATES_TEXT = "0-based, integer pixel centres; rows are y, columns are x";

function pixelCsvProvenance(result: PixelTableResult & PixelTableLayers, context: PixelCsvContext): string[] {
  const entries: ProvenanceEntry[] = [
    ...exportEntries(context.file, context.exportedUtc),
    ["plane", context.plane],
    ["centre_x", result.x],
    ["centre_y", result.y],
    ["pixel_coordinates", PIXEL_COORDINATES_TEXT],
  ];
  if (context.plane === "ERR" && result.err_source != null) entries.push(["err_source", result.err_source]);
  if (result.dq_table !== null) entries.push(["dq_table", result.dq_table]);
  const unit = context.plane === "WAVELENGTH" ? result.wavelength_unit : result.unit;
  return [...provenanceLines(entries), ...columnIndices(result).flatMap((x) => unitLines({ [String(x)]: unit }))];
}

export function pixelTableCsv(
  result: PixelTableResult & PixelTableLayers,
  grid: (number | null)[][] = result.values,
  context?: PixelCsvContext,
): string {
  const columns: CsvColumn<number>[] = [
    { header: CSV_CORNER_HEADER, value: (rowIndex) => result.y0 + rowIndex },
    ...columnIndices(result).map((x, col) => ({ header: String(x), value: (rowIndex: number) => grid[rowIndex]?.[col] ?? null })),
  ];
  const table = buildCsv(columns, Array.from({ length: result.size }, (_, i) => i));
  return context === undefined ? table : withProvenance(pixelCsvProvenance(result, context), table, CSV_LINE_END);
}

export function formatCell(v: number | null, digits: number): string {
  if (v === null || !Number.isFinite(v)) return NULL_TEXT;
  if (v === 0) return (0).toFixed(digits);
  const a = Math.abs(v);
  if (a < FIXED_MIN_ABS || a >= FIXED_MAX_ABS) return v.toExponential(Math.max(0, digits - 1));
  return v.toFixed(Math.max(digits, digits - 1 - Math.floor(Math.log10(a))));
}

function formatSignificant(v: number | null, significant: number): string {
  if (v === null || !Number.isFinite(v)) return NULL_TEXT;
  if (v === 0) return (0).toFixed(significant - 1);
  const a = Math.abs(v);
  if (a < FIXED_MIN_ABS || a >= FIXED_MAX_ABS) return v.toExponential(significant - 1);
  return v.toFixed(Math.max(0, significant - 1 - Math.floor(Math.log10(a))));
}

export function formatPlaneCell(v: number | null, plane: PixelTablePlane, digits: number = CELL_DIGITS): string {
  return plane === "WAVELENGTH" ? formatSignificant(v, WAVELENGTH_SIGNIFICANT) : formatCell(v, digits);
}

export function displayedPlane(result: PixelTableResult, plane: PixelTablePlane): DisplayedPlane {
  if (plane === "ERR" && result.err && result.err_stats) return { plane: "ERR", grid: result.err, stats: result.err_stats };
  if (plane === "WAVELENGTH" && result.wavelength && result.wavelength_stats) {
    return { plane: "WAVELENGTH", grid: result.wavelength, stats: result.wavelength_stats };
  }
  return { plane: "SCI", grid: result.values, stats: result.stats };
}

export function planeAvailable(result: PixelTableResult, plane: PixelTablePlane): boolean {
  return displayedPlane(result, plane).plane === plane;
}

export interface PixelCellReadings {
  value: number | null;
  err: number | null | undefined;
  wavelength: number | null | undefined;
  dqNames: string | null;
}

export function pixelCellTitle(
  x: number,
  y: number,
  cell: PixelCellReadings,
  units: Pick<PixelTableResult, "unit" | "wavelength_unit">,
  digits: number,
): string {
  const parts = [`(${x}, ${y}): ${formatCell(cell.value, digits)}${units.unit && cell.value !== null ? ` ${units.unit}` : ""}`];
  if (cell.err !== undefined) parts.push(`ERR ${formatCell(cell.err, digits)}`);
  if (cell.wavelength !== undefined) {
    const unit = units.wavelength_unit && cell.wavelength !== null ? ` ${units.wavelength_unit}` : "";
    parts.push(`λ ${formatPlaneCell(cell.wavelength, "WAVELENGTH")}${unit}`);
  }
  if (cell.dqNames) parts.push(`DQ ${cell.dqNames}`);
  return parts.join("\n");
}

export function cellTone(v: number | null, stats: PixelTableStats, dqNames: string | null): CellTone {
  if (dqNames) return "dq";
  if (v === null || !Number.isFinite(v)) return "nan";
  if (stats.max !== null && v === stats.max) return "max";
  if (stats.min !== null && v === stats.min) return "min";
  return "normal";
}

export const PIXEL_TABLE_KEY: readonly { tone: Exclude<CellTone, "normal">; label: string }[] = [
  { tone: "dq", label: "DQ flagged *" },
  { tone: "max", label: "max" },
  { tone: "min", label: "min" },
  { tone: "nan", label: "no value" },
];

export const DQ_CELL_GLYPH = "*";

const UNTOUCHED_PIPELINE_ERROR = /^(EXTNAME ERR|ASDF array [^,]*), [^:]*: as is$/;

export function errorOriginLine(source: string | null | undefined): string | null {
  if (!source || UNTOUCHED_PIPELINE_ERROR.test(source)) return null;
  return `± from ${source}`;
}

export function cellMark(tone: CellTone, dqNames: string | null, centre: boolean): { glyph: string | null; srText: string | null } {
  const flagged = tone === "dq";
  const parts = [...(flagged ? [`DQ: ${dqNames}`] : []), ...(centre ? ["clicked pixel"] : [])];
  return { glyph: flagged ? DQ_CELL_GLYPH : null, srText: parts.length > 0 ? parts.join(", ") : null };
}

export function centreSummary(result: Pick<PixelTableResult, "x" | "y" | "size" | "dq_names">): string {
  const half = Math.floor(result.size / 2);
  const names = result.dq_names?.[half]?.[half];
  return `clicked (${result.x}, ${result.y})${names ? `: DQ ${names}` : ""}`;
}

export function layerNotes(layers: PixelTableLayers): { origin: string | null; gaps: string[] } {
  const gaps = [
    ...(layers.err_unavailable ? [`ERR not used: ${layers.err_unavailable}`] : []),
    ...(layers.dq_unavailable ? [`DQ not used: ${layers.dq_unavailable}`] : []),
  ];
  return { origin: layers.err_source ? `ERR ${layers.err_source}` : null, gaps };
}
