import type {
  CatalogCsvKind,
  CrossMatchEntry,
  MeasuredSource,
  PlacedCatalogRow,
} from "../shared/types/catalog";

export type CsvCell = string | number | boolean | null | undefined;

export interface CsvColumn<T> {
  header: string;
  value: (item: T) => CsvCell;
}

export const CSV_LINE_END = "\r\n";

const NEEDS_QUOTING = /[",\r\n]/;

export function csvEscape(text: string): string {
  return NEEDS_QUOTING.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

export function csvCell(value: CsvCell): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "number") return Number.isFinite(value) ? String(value) : "";
  if (typeof value === "boolean") return value ? "true" : "false";
  return csvEscape(value);
}

export function buildCsv<T>(columns: CsvColumn<T>[], items: T[]): string {
  const lines: string[] = [columns.map((c) => csvEscape(c.header)).join(",")];
  for (const item of items) {
    lines.push(columns.map((c) => csvCell(c.value(item))).join(","));
  }
  return lines.join(CSV_LINE_END) + CSV_LINE_END;
}

export const CATALOG_CSV_COLUMNS: CsvColumn<PlacedCatalogRow>[] = [
  { header: "id", value: (r) => r.id },
  { header: "ra", value: (r) => r.ra },
  { header: "dec", value: (r) => r.dec },
  { header: "ra_epoch", value: (r) => r.ra_epoch },
  { header: "dec_epoch", value: (r) => r.dec_epoch },
  { header: "pm_ra_masyr", value: (r) => r.pm_ra_masyr },
  { header: "pm_dec_masyr", value: (r) => r.pm_dec_masyr },
  { header: "parallax_mas", value: (r) => r.parallax_mas },
  { header: "g", value: (r) => r.g },
  { header: "bp", value: (r) => r.bp },
  { header: "rp", value: (r) => r.rp },
  { header: "bp_rp", value: (r) => r.bp_rp },
  { header: "x", value: (r) => r.x },
  { header: "y", value: (r) => r.y },
  { header: "on_image", value: (r) => r.on_image },
];

export const SOURCES_CSV_COLUMNS: CsvColumn<MeasuredSource>[] = [
  { header: "x", value: (s) => s.x },
  { header: "y", value: (s) => s.y },
  { header: "ra", value: (s) => s.ra },
  { header: "dec", value: (s) => s.dec },
  { header: "flux", value: (s) => s.flux },
  { header: "mag_inst", value: (s) => s.mag_inst },
  { header: "mag_ab", value: (s) => s.mag_ab },
  { header: "fwhm", value: (s) => s.fwhm },
  { header: "snr", value: (s) => s.snr },
  { header: "saturated", value: (s) => s.saturated },
];

export const MATCHES_CSV_COLUMNS: CsvColumn<CrossMatchEntry>[] = [
  { header: "id", value: (m) => m.row.id },
  { header: "x", value: (m) => m.star.x },
  { header: "y", value: (m) => m.star.y },
  { header: "ra", value: (m) => m.star.ra },
  { header: "dec", value: (m) => m.star.dec },
  { header: "flux", value: (m) => m.star.flux },
  { header: "mag_inst", value: (m) => m.star.mag_inst },
  { header: "mag_ab", value: (m) => m.star.mag_ab },
  { header: "fwhm", value: (m) => m.star.fwhm },
  { header: "snr", value: (m) => m.star.snr },
  { header: "cat_ra", value: (m) => m.row.ra },
  { header: "cat_dec", value: (m) => m.row.dec },
  { header: "g", value: (m) => m.row.g },
  { header: "bp", value: (m) => m.row.bp },
  { header: "rp", value: (m) => m.row.rp },
  { header: "bp_rp", value: (m) => m.row.bp_rp },
  { header: "sep_arcsec", value: (m) => m.sep_arcsec },
  { header: "d_ra_arcsec", value: (m) => m.d_ra_arcsec },
  { header: "d_dec_arcsec", value: (m) => m.d_dec_arcsec },
];

export function catalogRowsCsv(rows: PlacedCatalogRow[]): string {
  return buildCsv(CATALOG_CSV_COLUMNS, rows);
}

export function sourcesCsv(sources: MeasuredSource[]): string {
  return buildCsv(SOURCES_CSV_COLUMNS, sources);
}

export function matchesCsv(matches: CrossMatchEntry[]): string {
  return buildCsv(MATCHES_CSV_COLUMNS, matches);
}

const DEFAULT_STEM = "catalog";

export function catalogCsvFileName(filePath: string | null, kind: CatalogCsvKind): string {
  const withoutFragment = (filePath ?? "").split("#")[0];
  const base = withoutFragment.split(/[\\/]/).pop() ?? "";
  const dot = base.lastIndexOf(".");
  const stem = (dot > 0 ? base.slice(0, dot) : base) || DEFAULT_STEM;
  return `${stem}_${kind}.csv`;
}
