import type { ProcessedFile } from "../shared/types";
import type { FileSpectrumSource, X1dSpectrum } from "../shared/types/spectral";
import type { Spectrum1d, Spectrum1dKind } from "../shared/types/spectrum1d";
import { parseImageRef } from "./imageRef";
import type { ComparisonEntry } from "./spectrumCompare";
import { fileBaseName, fluxUnitLabel, vacuumUmAxis, type SpectrumView, type VacuumAxis } from "./spectrumExport";

export type SiblingCandidate = Pick<ProcessedFile, "path" | "status">;

export interface ListedFile extends SiblingCandidate {
  id: string;
  spectrum?: Spectrum1dKind;
}

export const TABLE_ENTRY_ID = "x1d";
export const TABLE_SERIES_COLOR = "#f0abfc";
export const DOCUMENT_ENTRY_ID = "document";
export const DOCUMENT_SERIES_COLOR = "#e4e4e7";
export const TABLE_ONLY_HINT = "1D spectrum, EXTRACT1D table";
export const APERTURE_NOTE =
  "the pipeline aperture differs from your region: compare shapes and line positions, not absolute levels";

const X1D_NAME_RE = /_x1d(\.fits|\.fit|\.fts)(\.gz)?$/i;
const FITS_EXTENSION_RE = /\.(fits|fit|fts)(\.gz)?$/i;
const CUBE_SUFFIX = "_s3d";
const EXACT_HIT_RELATIVE = 1e-9;
const TABLE_REJECTION = "BINTABLE extension, holds no image pixels";
const EXTRACT1D = "EXTRACT1D";
const DEFAULT_SB_UNIT = "MJy/sr";
const DEFAULT_FLUX_UNIT = "Jy";
const TABLE_LABEL = "x1d";
const NATIVE_FLUX_UNIT = "native";
const MICRON_UNIT = "um";
const CSV_TABLE_HDU = 0;
const UNKNOWN_DQ_TABLE = "unknown";

export interface ResampledTable {
  flux: number[];
  fluxErr: (number | null)[];
  surfBright: number[] | null;
  rowsUsed: number;
  droppedDq: number;
  overlap: [number, number] | null;
}

export interface TableEntryData {
  path: string;
  hdu: number;
  nRows: number;
  fluxUnit: string;
  sbUnit: string | null;
  resampled: ResampledTable;
}

export interface UsableRows {
  wavelength: number[];
  flux: number[];
  err: (number | null)[];
  sb: (number | null)[];
  flagged: boolean[];
  droppedDq: number;
}

function finiteOrNull(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function usableRows(x1d: X1dSpectrum): UsableRows {
  const nodes: { wavelength: number; flux: number; err: number | null; sb: number | null; flagged: boolean }[] = [];
  let droppedDq = 0;
  x1d.wavelength_um.forEach((raw, i) => {
    const dq = x1d.dq?.[i];
    const flagged = typeof dq === "number" && dq !== 0;
    if (flagged) droppedDq += 1;
    const wavelength = finiteOrNull(raw);
    if (wavelength === null) return;
    nodes.push(
      flagged
        ? { wavelength, flux: NaN, err: null, sb: null, flagged }
        : {
            wavelength,
            flux: finiteOrNull(x1d.flux[i]) ?? NaN,
            err: finiteOrNull(x1d.flux_error?.[i]),
            sb: finiteOrNull(x1d.surf_bright?.[i]),
            flagged,
          },
    );
  });
  nodes.sort((a, b) => a.wavelength - b.wavelength);
  return {
    wavelength: nodes.map((r) => r.wavelength),
    flux: nodes.map((r) => r.flux),
    err: nodes.map((r) => r.err),
    sb: nodes.map((r) => r.sb),
    flagged: nodes.map((r) => r.flagged),
    droppedDq,
  };
}

function lastNotAbove(xs: readonly number[], target: number): number {
  let lo = 0;
  let hi = xs.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (xs[mid] <= target) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

type Blend = (y0: number, y1: number, t: number) => number;

const linearBlend: Blend = (y0, y1, t) => y0 + t * (y1 - y0);
const quadratureBlend: Blend = (s0, s1, t) => Math.hypot((1 - t) * s0, t * s1);

export function interpolateOnto(
  axisUm: readonly number[],
  xs: readonly number[],
  ys: readonly (number | null)[],
  blend: Blend = linearBlend,
): (number | null)[] {
  const n = xs.length;
  return axisUm.map((target) => {
    if (n === 0 || !Number.isFinite(target)) return null;
    const j = lastNotAbove(xs, target);
    if (isExactHit(target, xs[j])) return finiteOrNull(ys[j]);
    if (j + 1 < n && isExactHit(target, xs[j + 1])) return finiteOrNull(ys[j + 1]);
    if (target < xs[0] || target > xs[n - 1]) return null;
    const y0 = finiteOrNull(ys[j]);
    const y1 = finiteOrNull(ys[j + 1]);
    if (y0 === null || y1 === null) return null;
    const t = (target - xs[j]) / (xs[j + 1] - xs[j]);
    return blend(y0, y1, t);
  });
}

function isExactHit(target: number, row: number): boolean {
  return Math.abs(target - row) <= EXACT_HIT_RELATIVE * Math.abs(row);
}

function overlapOf(axisUm: readonly number[], wavelength: readonly number[]): [number, number] | null {
  const finite = axisUm.filter((v) => Number.isFinite(v));
  if (finite.length === 0 || wavelength.length === 0) return null;
  const lo = Math.max(Math.min(...finite), wavelength[0]);
  const hi = Math.min(Math.max(...finite), wavelength[wavelength.length - 1]);
  return lo <= hi ? [lo, hi] : null;
}

export function resampleTable(x1d: X1dSpectrum, cubeVacuumUm: readonly number[]): ResampledTable {
  const rows = usableRows(x1d);
  const unflagged = rows.wavelength.filter((_, i) => !rows.flagged[i]);
  const toNumbers = (values: (number | null)[]) => values.map((v) => v ?? NaN);
  return {
    flux: toNumbers(interpolateOnto(cubeVacuumUm, rows.wavelength, rows.flux)),
    fluxErr: interpolateOnto(cubeVacuumUm, rows.wavelength, rows.err, quadratureBlend),
    surfBright: x1d.surf_bright === null ? null : toNumbers(interpolateOnto(cubeVacuumUm, rows.wavelength, rows.sb)),
    rowsUsed: unflagged.length,
    droppedDq: rows.droppedDq,
    overlap: overlapOf(cubeVacuumUm, unflagged),
  };
}

function cardText(value: string | null): string | null {
  const trimmed = value?.trim() ?? "";
  return trimmed === "" ? null : trimmed;
}

export function tableLabel(x1d: X1dSpectrum): string {
  const detector = cardText(x1d.detector)?.toLowerCase() ?? null;
  const optics = [cardText(x1d.grating), cardText(x1d.filter)].filter((part): part is string => part !== null).join("/");
  const parts = [detector, optics === "" ? null : optics].filter((part): part is string => part !== null);
  return parts.length === 0 ? TABLE_LABEL : `${TABLE_LABEL} (${parts.join(", ")})`;
}

export function tableEntry(x1d: X1dSpectrum, vacuum: VacuumAxis, channelCount: number): ComparisonEntry {
  const axis = vacuum.values;
  const error =
    axis === null
      ? vacuum.reason
      : axis.length !== channelCount
        ? `vacuum axis has ${axis.length} channels, cube has ${channelCount}`
        : null;
  return {
    id: TABLE_ENTRY_ID,
    kind: "table",
    label: tableLabel(x1d),
    color: TABLE_SERIES_COLOR,
    source: { kind: "table", path: x1d.path, hdu: x1d.hdu },
    regionText: null,
    backgroundId: null,
    region: null,
    pixel: null,
    table:
      axis === null || error !== null
        ? null
        : {
            path: x1d.path,
            hdu: x1d.hdu,
            nRows: x1d.n_rows,
            fluxUnit: x1d.flux_unit,
            sbUnit: x1d.surf_bright_unit,
            resampled: resampleTable(x1d, axis),
          },
    error,
  };
}

export function withTable(
  entries: ComparisonEntry[],
  x1d: X1dSpectrum | null,
  vacuum: VacuumAxis,
  channelCount: number,
): ComparisonEntry[] {
  return x1d === null ? entries : [...entries, tableEntry(x1d, vacuum, channelCount)];
}

function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export function x1dStem(path: string): string | null {
  const name = baseName(parseImageRef(path).path);
  return X1D_NAME_RE.test(name) ? name.replace(X1D_NAME_RE, "") : null;
}

function loadedCubes<T extends SiblingCandidate>(files: readonly T[]): Map<string, T> {
  const cubes = new Map<string, T>();
  for (const file of files) {
    if (file.status !== "done") continue;
    const name = baseName(parseImageRef(file.path).path);
    if (!FITS_EXTENSION_RE.test(name)) continue;
    const key = name.replace(FITS_EXTENSION_RE, "").toLowerCase();
    if (key.endsWith(CUBE_SUFFIX) && !cubes.has(key)) cubes.set(key, file);
  }
  return cubes;
}

function siblingIn<T>(cubes: ReadonlyMap<string, T>, x1dPath: string): T | null {
  const stem = x1dStem(x1dPath);
  return stem ? (cubes.get(`${stem.toLowerCase()}${CUBE_SUFFIX}`) ?? null) : null;
}

export function findSiblingCube<T extends SiblingCandidate>(files: readonly T[], x1dPath: string): T | null {
  return siblingIn(loadedCubes(files), x1dPath);
}

export function tableOnlyFileHint(error: string | null | undefined): string | null {
  if (!error) return null;
  return error.includes(TABLE_REJECTION) && error.includes(EXTRACT1D) ? TABLE_ONLY_HINT : null;
}

export function comparableX1dIds(files: readonly ListedFile[]): Set<string> {
  const ids = new Set<string>();
  const tables = files.filter((file) => file.status === "spectrum" && file.spectrum?.kind === "x1d");
  if (tables.length === 0) return ids;
  const cubes = loadedCubes(files);
  for (const file of tables) if (siblingIn(cubes, file.path) !== null) ids.add(file.id);
  return ids;
}

export function documentAsTable(spectrum: Spectrum1d): X1dSpectrum {
  const source = spectrum.source;
  const vacuum = vacuumUmAxis(spectrum.axis, null, "optical", spectrum.flux.length);
  if (vacuum.values === null) throw new Error(`${fileBaseName(source.path)}: ${vacuum.reason}`);
  const card = (key: string) => cardText(spectrum.cards[key] ?? null);
  const dq = spectrum.dq_table === UNKNOWN_DQ_TABLE ? null : spectrum.dq;
  return {
    path: source.path,
    hdu: source.kind === "csv" ? CSV_TABLE_HDU : source.hdu,
    extver: source.kind === "x1d" ? source.extver : null,
    n_rows: spectrum.n_rows,
    wavelength_um: vacuum.values,
    wavelength_unit: MICRON_UNIT,
    flux: spectrum.flux,
    flux_error: spectrum.err,
    flux_unit: spectrum.flux_unit ?? NATIVE_FLUX_UNIT,
    surf_bright: null,
    surf_bright_unit: null,
    background: null,
    npixels: null,
    dq,
    dq_table: spectrum.dq_table,
    dq_flagged_rows: dq?.filter((bits) => bits !== 0).length ?? 0,
    srctype: card("SRCTYPE"),
    grating: card("GRATING"),
    filter: card("FILTER"),
    detector: card("DETECTOR"),
    instrument: card("INSTRUME"),
    target: card("TARGNAME") ?? card("OBJECT"),
    other_tables: [],
    notes: spectrum.notes,
  };
}

export function documentComparisonEntries(
  spectrum: Spectrum1d,
  source: FileSpectrumSource,
  table: X1dSpectrum | null,
  vacuum: VacuumAxis,
  channelCount: number,
): ComparisonEntry[] {
  const own: ComparisonEntry = {
    id: DOCUMENT_ENTRY_ID,
    kind: "document",
    label: fileBaseName(spectrum.source.path),
    color: DOCUMENT_SERIES_COLOR,
    source,
    regionText: null,
    backgroundId: null,
    region: null,
    pixel: null,
    table: null,
    document: { flux: spectrum.flux, fluxUnit: spectrum.flux_unit ?? NATIVE_FLUX_UNIT },
    error: null,
  };
  if (table === null) return [own];
  return [own, { ...tableEntry(table, vacuum, channelCount), label: fileBaseName(table.path) }];
}

export interface ComparisonReaders {
  x1d: (path: string, hdu: number | null) => Promise<X1dSpectrum>;
  document: (path: string, hdu: number | null) => Promise<Spectrum1d>;
}

export async function readComparisonTable(
  path: string,
  hdu: number | null,
  documentMode: boolean,
  readers: ComparisonReaders,
): Promise<X1dSpectrum> {
  return documentMode ? documentAsTable(await readers.document(path, hdu)) : readers.x1d(path, hdu);
}

export function entryFluxUnit(entry: ComparisonEntry, bunit: string | null, view: SpectrumView): string {
  if (entry.kind === "document") return entry.document?.fluxUnit ?? NATIVE_FLUX_UNIT;
  if (entry.kind === "table") {
    const table = entry.table;
    if (view === "mean") return table?.sbUnit ?? DEFAULT_SB_UNIT;
    return table?.fluxUnit ?? DEFAULT_FLUX_UNIT;
  }
  return fluxUnitLabel(bunit, view, entry.kind);
}
