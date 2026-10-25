import type { MastColumn, MastMissionId, MastRow, MastSearchPage } from "../../../shared/types/mast";
import type { SortKey } from "./buildQuery";

export const PAGE_SIZE = 200;
export const MAX_ROWS = 2000;
export const SORT_BLOCKED_HINT = "MAST cannot sort by this column while more rows exist";

export interface ResultSet {
  mission: MastMissionId;
  columns: MastColumn[];
  rows: MastRow[];
  hasMore: boolean;
  truncated: boolean;
  totalReported: number | null;
  totalReliable: boolean;
  radiusArcmin: number | null;
  radiusClamped: boolean;
  searchPosition: MastSearchPage["searchPosition"];
  applied: MastSearchPage["applied"];
  droppedConditions: string[];
  requestBody: unknown;
  messages: string[];
  unresolvedTargets: string[];
  elapsedMs: number;
  fromCache: boolean;
  levelAppliedLocally: string | null;
  unfilteredTotal: number | null;
}

export type SortMode = "local" | "server" | "none";

function fromPage(page: MastSearchPage, rows: MastRow[]): ResultSet {
  const capped = rows.slice(0, MAX_ROWS);
  const truncated = rows.length >= MAX_ROWS && (page.hasMore || rows.length > MAX_ROWS);
  return {
    mission: page.mission,
    columns: page.columns,
    rows: capped,
    hasMore: page.hasMore && capped.length < MAX_ROWS,
    truncated,
    totalReported: page.totalReported,
    totalReliable: page.totalReliable,
    radiusArcmin: page.radiusArcmin,
    radiusClamped: page.radiusClamped,
    searchPosition: page.searchPosition,
    applied: page.applied,
    droppedConditions: page.droppedConditions,
    requestBody: page.requestBody,
    messages: page.messages,
    unresolvedTargets: page.unresolvedTargets,
    elapsedMs: page.elapsedMs,
    fromCache: page.fromCache,
    levelAppliedLocally: page.levelAppliedLocally,
    unfilteredTotal: page.unfilteredTotal,
  };
}

function rowIdentity(row: MastRow): string {
  return `${row.datasetId} ${JSON.stringify(row.values)}`;
}

export function mergePage(prev: ResultSet | null, page: MastSearchPage): ResultSet {
  if (page.offset === 0 || !prev) return fromPage(page, page.rows);
  if (page.offset !== nextOffset(prev)) return prev;
  const seen = new Set(prev.rows.map(rowIdentity));
  const added = page.rows.filter((r) => !seen.has(rowIdentity(r)));
  const merged = fromPage(page, [...prev.rows, ...added]);
  return {
    ...merged,
    columns: prev.columns.length ? prev.columns : page.columns,
    requestBody: prev.requestBody,
    applied: prev.applied,
    droppedConditions: prev.droppedConditions,
    elapsedMs: prev.elapsedMs,
    fromCache: prev.fromCache,
    levelAppliedLocally: prev.levelAppliedLocally,
    unfilteredTotal: prev.unfilteredTotal,
  };
}

export function nextOffset(set: ResultSet): number | null {
  if (!set.hasMore || set.rows.length >= MAX_ROWS) return null;
  const offset = Math.ceil(set.rows.length / PAGE_SIZE) * PAGE_SIZE;
  return offset <= MAX_ROWS - PAGE_SIZE ? offset : null;
}

export function allRowsLoaded(set: ResultSet): boolean {
  return !set.hasMore;
}

export function sortMode(set: ResultSet, column: MastColumn): SortMode {
  if (allRowsLoaded(set)) return "local";
  return column.sortable ? "server" : "none";
}

export function nextSort(keys: readonly SortKey[], column: string, additive: boolean): SortKey[] {
  const existing = keys.find((k) => k.column === column);
  const cycled: SortKey | null = !existing ? { column, desc: false } : existing.desc ? null : { column, desc: true };
  if (!additive) return cycled ? [cycled] : [];
  if (!existing) return [...keys, cycled as SortKey];
  return keys.flatMap((k) => (k.column !== column ? [k] : cycled ? [cycled] : []));
}

export function cellValue(row: MastRow, column: string): string | number | boolean | null {
  const v = row.values[column];
  return v === undefined ? null : v;
}

const collator = new Intl.Collator("en", { numeric: true, sensitivity: "base" });

function timeOf(v: string | number | boolean): number {
  if (typeof v === "number") return v;
  const t = Date.parse(String(v).replace(" ", "T"));
  return Number.isNaN(t) ? Number.NEGATIVE_INFINITY : t;
}

function compareValues(a: string | number | boolean, b: string | number | boolean, kind: MastColumn["kind"]): number {
  if (kind === "int" || kind === "float") {
    const x = typeof a === "number" ? a : Number(a);
    const y = typeof b === "number" ? b : Number(b);
    if (Number.isFinite(x) && Number.isFinite(y)) return x - y;
  }
  if (kind === "datetime") return timeOf(a) - timeOf(b);
  return collator.compare(String(a), String(b));
}

export function sortRows(rows: readonly MastRow[], keys: readonly SortKey[], columns: readonly MastColumn[]): MastRow[] {
  if (keys.length === 0) return [...rows];
  const specs = keys.map((k) => ({ ...k, kind: columns.find((c) => c.name === k.column)?.kind ?? "text" }));
  const indexed = rows.map((row, index) => ({ row, index }));
  indexed.sort((p, q) => {
    for (const spec of specs) {
      const a = cellValue(p.row, spec.column);
      const b = cellValue(q.row, spec.column);
      if (a === null && b === null) continue;
      if (a === null) return 1;
      if (b === null) return -1;
      const c = compareValues(a, b, spec.kind);
      if (c !== 0) return spec.desc ? -c : c;
    }
    return p.index - q.index;
  });
  return indexed.map((x) => x.row);
}
