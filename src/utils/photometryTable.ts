import type { StarPhotometry } from "../services/analysis";
import type { PlotReferenceLine } from "../components/regions/ProfilePlot";
import { buildCsv, type CsvCell, type CsvColumn } from "./catalogCsv";
import { parseImageRef } from "./imageRef";

export interface PhotometryTableRow {
  index: number;
  label: string;
  photometry: StarPhotometry | null;
  sky: { ra: number; dec: number } | null;
  error: string | null;
}

export const PHOTOMETRY_COLUMN_KEYS = [
  "index",
  "label",
  "x",
  "y",
  "ra",
  "dec",
  "net_flux",
  "flux_err",
  "snr",
  "mag_inst",
  "mag_ab",
  "mag_ab_err",
  "flux_jy",
  "flux_err_jy",
  "fwhm",
  "aperture_radius",
  "sky_inner",
  "sky_outer",
  "bg_mean",
  "bg_sigma",
  "aperture_correction",
  "flux_total",
  "mag_ab_total",
  "ee50_radius",
  "ee80_radius",
  "saturated",
  "n_masked",
  "err_used",
  "error",
] as const;

export type PhotometryColumnKey = (typeof PHOTOMETRY_COLUMN_KEYS)[number];
export type SortDirection = "asc" | "desc";

export const PEAK_SEARCH_RADIUS_PX = 8;
export const MIN_APERTURE_RADIUS_PX = 2;
export const MAX_APERTURE_RADIUS_PX = 60;
export const MAX_SKY_OUTER_RADIUS_PX = 512;
export const APERTURE_RANGE_HINT = `Aperture radius must be between ${MIN_APERTURE_RADIUS_PX} and ${MAX_APERTURE_RADIUS_PX} px.`;
const PEAK_SEARCH_BOX_PX = 2 * PEAK_SEARCH_RADIUS_PX + 1;
export const BATCH_SEMANTICS_TEXT = `Each position snaps to the brightest pixel in the ${PEAK_SEARCH_BOX_PX} x ${PEAK_SEARCH_BOX_PX} px box around it (${PEAK_SEARCH_RADIUS_PX} px each way) and is then recentred on that light, so a brighter neighbour beyond ${PEAK_SEARCH_RADIUS_PX} px can take over the input; check the measured x/y. Two inputs on one star measure the same star (flagged as duplicates). The aperture radius is fixed for every source (leave it blank for 1.5 x FWHM per star). When the curve of growth has no plateau before the sky annulus, the correction, total flux and EE radii are empty.`;
export const STARS_ELSEWHERE_NOTICE =
  "The detected stars came from the RGB composite, not from the image this table measures, so they are not offered here; go back to the file and detect again.";

const POSITION_SEPARATOR = /[\s,;]+/;
const COMMENT_PREFIX = "#";

function finiteOrNull(v: number | null | undefined): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

export function rowValue(row: PhotometryTableRow, key: PhotometryColumnKey): CsvCell {
  const p = row.photometry;
  switch (key) {
    case "index":
      return row.index;
    case "label":
      return row.label;
    case "error":
      return row.error;
    case "ra":
      return row.sky?.ra ?? null;
    case "dec":
      return row.sky?.dec ?? null;
    case "saturated":
      return p ? p.saturated : null;
    case "err_used":
      return p ? p.err_used : null;
    case "n_masked":
      return p ? p.n_masked : null;
    default:
      return p ? finiteOrNull(p[key]) : null;
  }
}

export const PHOTOMETRY_CSV_COLUMNS: CsvColumn<PhotometryTableRow>[] = PHOTOMETRY_COLUMN_KEYS.map((key) => ({
  header: key,
  value: (row) => rowValue(row, key),
}));

export function photometryTableCsv(rows: PhotometryTableRow[]): string {
  return buildCsv(PHOTOMETRY_CSV_COLUMNS, rows);
}

export function parsePositions(text: string): { points: [number, number][]; errors: string[] } {
  const points: [number, number][] = [];
  const errors: string[] = [];
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (line === "" || line.startsWith(COMMENT_PREFIX)) continue;
    const parts = line.split(POSITION_SEPARATOR).filter((part) => part !== "");
    const x = parts.length >= 2 ? Number(parts[0]) : Number.NaN;
    const y = parts.length >= 2 ? Number(parts[1]) : Number.NaN;
    if (parts.length !== 2 || !Number.isFinite(x) || !Number.isFinite(y)) {
      errors.push(`line ${i + 1}: expected "x y" or "x,y", got "${line}"`);
      continue;
    }
    points.push([x, y]);
  }
  return { points, errors };
}

function cellKey(cx: number, cy: number): string {
  return `${cx}:${cy}`;
}

export function flagDuplicates(rows: PhotometryTableRow[], tolerancePx = 1): Set<number> {
  const flagged = new Set<number>();
  if (!(tolerancePx > 0) || !Number.isFinite(tolerancePx)) return flagged;
  const cells = new Map<string, { x: number; y: number }[]>();
  const tolerance2 = tolerancePx * tolerancePx;
  for (const row of rows) {
    const p = row.photometry;
    if (!p || !Number.isFinite(p.x) || !Number.isFinite(p.y)) continue;
    const cx = Math.floor(p.x / tolerancePx);
    const cy = Math.floor(p.y / tolerancePx);
    let duplicate = false;
    for (let dy = -1; dy <= 1 && !duplicate; dy++) {
      for (let dx = -1; dx <= 1 && !duplicate; dx++) {
        const seen = cells.get(cellKey(cx + dx, cy + dy));
        if (!seen) continue;
        duplicate = seen.some((q) => (q.x - p.x) ** 2 + (q.y - p.y) ** 2 <= tolerance2);
      }
    }
    if (duplicate) {
      flagged.add(row.index);
      continue;
    }
    const key = cellKey(cx, cy);
    const bucket = cells.get(key);
    if (bucket) bucket.push({ x: p.x, y: p.y });
    else cells.set(key, [{ x: p.x, y: p.y }]);
  }
  return flagged;
}

function sortableValue(cell: CsvCell): number | string | null {
  if (cell === null || cell === undefined) return null;
  if (typeof cell === "boolean") return cell ? 1 : 0;
  if (typeof cell === "number") return Number.isFinite(cell) ? cell : null;
  return cell;
}

export function sortRows(rows: PhotometryTableRow[], key: PhotometryColumnKey, dir: SortDirection): PhotometryTableRow[] {
  const sign = dir === "asc" ? 1 : -1;
  const decorated = rows.map((row, position) => ({ row, position, value: sortableValue(rowValue(row, key)) }));
  decorated.sort((a, b) => {
    if (a.value === null && b.value === null) return a.position - b.position;
    if (a.value === null) return 1;
    if (b.value === null) return -1;
    const cmp =
      typeof a.value === "number" && typeof b.value === "number"
        ? a.value - b.value
        : String(a.value).localeCompare(String(b.value));
    return cmp !== 0 ? sign * cmp : a.position - b.position;
  });
  return decorated.map((d) => d.row);
}

export function medianSnr(rows: PhotometryTableRow[]): number | null {
  const values = rows
    .map((row) => row.photometry?.snr)
    .filter((v): v is number => typeof v === "number" && Number.isFinite(v))
    .sort((a, b) => a - b);
  const n = values.length;
  if (n === 0) return null;
  return n % 2 === 1 ? values[(n - 1) / 2] : (values[n / 2 - 1] + values[n / 2]) / 2;
}

export function photometryCsvFileName(filePath: string): string {
  const base = parseImageRef(filePath).path.split(/[\\/]/).pop() ?? "image";
  const stem = base.replace(/\.(fits?|fts|asdf)(\.gz)?$/i, "");
  return `${stem}_photometry.csv`;
}

export function apertureRadiusInRange(radius: number | undefined): boolean {
  return radius === undefined || (radius >= MIN_APERTURE_RADIUS_PX && radius <= MAX_APERTURE_RADIUS_PX);
}

export function plateauCaption(plateauRadius: number | null | undefined): string {
  return plateauRadius != null && Number.isFinite(plateauRadius) ? ` (growth curve plateau at r=${plateauRadius.toFixed(2)} px)` : "";
}

const APERTURE_LINE_COLOR = "#fbbf24";
const SKY_LINE_COLOR = "#a1a1aa";
const PLATEAU_LINE_COLOR = "#ffffff";

export function growthReferenceLines(phot: StarPhotometry): PlotReferenceLine[] {
  const candidates: { value: number | null | undefined; label: string; color: string; dashed: boolean }[] = [
    { value: phot.aperture_radius, label: "r_ap", color: APERTURE_LINE_COLOR, dashed: false },
    { value: phot.sky_inner, label: "sky in", color: SKY_LINE_COLOR, dashed: false },
    { value: phot.sky_outer, label: "sky out", color: SKY_LINE_COLOR, dashed: false },
    { value: phot.plateau_radius, label: "plateau", color: PLATEAU_LINE_COLOR, dashed: true },
    { value: phot.ee50_radius, label: "EE50", color: PLATEAU_LINE_COLOR, dashed: true },
  ];
  return candidates
    .filter((c): c is typeof c & { value: number } => typeof c.value === "number" && Number.isFinite(c.value))
    .map((c) => ({ axis: "x", value: c.value, label: c.label, color: c.color, dashed: c.dashed }));
}
