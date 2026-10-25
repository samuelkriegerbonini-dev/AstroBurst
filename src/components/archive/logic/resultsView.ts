import type {
  MastColumn,
  MastFilterGroup,
  MastFilterValues,
  MastMission,
  MastMissionId,
  MastPreset,
  MastQuery,
  MastResolved,
  MastRow,
  MastSearchPage,
} from "../../../shared/types/mast";
import type { ArchiveForm, SortKey } from "./buildQuery";
import { interpretQuery, type QueryKind } from "./interpretQuery";
import { sortRows, type ResultSet } from "./paging";
import type { ThumbQueue } from "./thumbQueue";
import { coordinatesText } from "./copyBuilders";
import { PURE_PARALLEL_NOTE, PURE_PARALLEL_PRESET, droppedText, presetById, presetLevel } from "./presetView";
import { OBJECT_KEY, hasObjectFilters, objectChips, objectLabels, offeredIds, parseObjectChip, type ObjectChipId } from "./objectFilters";
import { levelChipLabel, narrowsLevel } from "./levels";
import { CATEGORY_COLUMN } from "./missionLabels";

export interface TableSort {
  keys: SortKey[];
  by: "local" | "server";
}

export interface SummaryView {
  text: string;
  raDeg: number | null;
  decDeg: number | null;
  total: number;
  applied: string;
  preset: string | null;
  ignored: string | null;
  unknownNote: string | null;
}

export interface RunLike {
  phase: "resolving" | "searching";
  kind: QueryKind;
  text: string;
  name: string | null;
  resolved: MastResolved | null;
  startedAt: number;
  searchStartedAt: number;
}

export interface ProgressLine {
  key: "resolve" | "resolved" | "search";
  text: string;
  elapsed: string | null;
}

export interface SearchProgressView {
  lines: ProgressLine[];
  slowNote: string | null;
}

export type BaseChipId = "level" | "instrument" | "filter" | "dateFrom" | "dateTo" | "publicOnly";
export type ChipId = BaseChipId | ObjectChipId;

export interface FilterChip {
  id: ChipId;
  label: string;
}

export interface EmptyView {
  title: string;
  chips: FilterChip[];
  widenTo: number | null;
  clearFilters: boolean;
  allLevels: boolean;
  tryOn: { mission: MastMissionId; label: string } | null;
  classLine: string | null;
}

export const WIDEN_STEPS: readonly number[] = [3, 10, 30];
export const SLOW_NOTE = "MAST answers queries it has not seen recently slowly (25 to 80 s measured); repeats take 1 to 3 s.";
export const SLOW_AFTER_MS = 8000;
export const ELAPSED_AFTER_MS = 1000;
export const UNKNOWN_TEXT = "unknown";
const CLASS_LINE = "Category and Object type come from what each proposal declared; a field can be filed under another class.";
export const UNKNOWN_CLEAR_TEXT = "Remove object filters";

const UNKNOWN_TAIL = "Datasets with no proposal category, such as most MSA source spectra and parallel fields, never match";

const OPERATOR = /^(>=|<=|!=|<|>)/;
const DATASET_COLUMNS = new Set(["fileSetName", "sci_data_set_name", "iue_data_id", "Target", "target_id"]);
const CHIP_FILTER: Record<BaseChipId, MastMission["quickFilters"][number]["id"]> = {
  level: "level",
  instrument: "instrument",
  filter: "filter",
  dateFrom: "date",
  dateTo: "date",
  publicOnly: "public",
};
const OTHER_MISSION: Partial<Record<MastMissionId, MastMissionId>> = { jwst: "hst", hst: "jwst" };

export function appliedText(applied: MastSearchPage["applied"]): string {
  const parts = applied.conditions.flatMap((c) => Object.entries(c).flatMap(([k, v]) => (v.trim() === "" ? [] : [OPERATOR.test(v) ? `${k} ${v}` : `${k} = ${v}`])));
  return parts.length ? parts.join(", ") : "none";
}

export function elapsedText(ms: number): string {
  const seconds = Math.max(0, ms) / 1000;
  return seconds < 10 ? `${seconds.toFixed(1)} s` : `${Math.round(seconds)} s`;
}

function radiusOf(results: ResultSet, query: MastQuery | null): number | null {
  if (results.radiusArcmin !== null) return results.radiusArcmin;
  const where = query?.where;
  return where && where.kind !== "none" ? where.radiusArcmin : null;
}

function positionOf(results: ResultSet, query: MastQuery | null): { raDeg: number; decDeg: number } | null {
  if (results.searchPosition) return results.searchPosition;
  const where = query?.where;
  return where?.kind === "position" ? { raDeg: where.raDeg, decDeg: where.decDeg } : null;
}

function datasetsText(n: number): string {
  return `${n} ${n === 1 ? "dataset" : "datasets"}`;
}

export function reportedTotal(results: ResultSet): number {
  return results.totalReliable && results.totalReported !== null ? Math.max(results.totalReported, results.rows.length) : results.rows.length;
}

function headOf(results: ResultSet, resolved: MastResolved | null, query: MastQuery | null): string {
  const pos = positionOf(results, query);
  const coords = pos ? coordinatesText(pos.raDeg, pos.decDeg) : "";
  if (query?.program !== null && query?.program !== undefined) return `Programme ${query.program}`;
  if (query?.datasetId) return `Dataset ${query.datasetId}`;
  if (resolved) return `${resolved.query} → ${resolved.resolver} "${resolved.canonicalName}" ${coordinatesText(resolved.raDeg, resolved.decDeg)}`;
  if (query?.where.kind === "name") return [`"${query.where.name}" resolved by MAST`, coords].filter(Boolean).join(" ");
  return coords;
}

function countText(n: number): string {
  return n.toLocaleString("en-US");
}

function unknownNoteText(shown: number, unfilteredTotal: number | null): string {
  if (unfilteredTotal === null) return `${UNKNOWN_TAIL} Category or Object type.`;
  return `Showing ${countText(shown)} of ${countText(unfilteredTotal)} datasets from the same search without Category or Object type. ${UNKNOWN_TAIL}.`;
}

export function summaryView(input: {
  results: ResultSet;
  resolved: MastResolved | null;
  query: MastQuery | null;
  missionLabel: string;
  presets?: readonly MastPreset[];
  groups?: readonly MastFilterGroup[];
}): SummaryView {
  const { results, resolved, query } = input;
  const parts: string[] = [];
  const head = headOf(results, resolved, query);
  if (head) parts.push(head);
  const preset = presetById(input.presets ?? [], query?.preset);
  if (preset) parts.push(`Preset ${preset.label}`);
  const objects = query ? objectLabels(query.filters, input.groups ?? []) : [];
  if (objects.length) parts.push(`Filters: ${objects.join(", ")}`);
  const usesCone = !query || (query.program === null && !query.datasetId);
  const radius = radiusOf(results, query);
  if (usesCone && radius !== null) parts.push(`r ${radius}'${results.radiusClamped ? " (clamped)" : ""}`);
  const total = reportedTotal(results);
  parts.push(results.totalReliable ? datasetsText(total) : `${results.rows.length}${results.hasMore ? "+" : ""} datasets (total approximate)`);
  parts.push(`${elapsedText(results.elapsedMs)}${results.fromCache ? " (cached)" : ""}`);
  const applied = appliedText(results.applied);
  parts.push(`MAST applied: ${applied}`);
  if (results.levelAppliedLocally) parts.push(`AstroBurst applied: ${results.levelAppliedLocally}`);
  const pos = positionOf(results, query) ?? (resolved ? { raDeg: resolved.raDeg, decDeg: resolved.decDeg } : null);
  return {
    text: parts.join(" · "),
    raDeg: pos?.raDeg ?? null,
    decDeg: pos?.decDeg ?? null,
    total,
    applied,
    preset: preset?.id ?? null,
    ignored: droppedText(results.droppedConditions),
    unknownNote: hasObjectFilters(query?.filters) ? unknownNoteText(total, results.unfilteredTotal) : null,
  };
}

function userLevelLabel(filters: MastFilterValues, preset: MastPreset | null): string | null {
  return presetLevel(preset) === null ? levelChipLabel(filters.level) : null;
}

export function filterSummary(filters: MastFilterValues, groups: readonly MastFilterGroup[] = [], preset: MastPreset | null = null): string {
  const parts: string[] = [];
  const level = userLevelLabel(filters, preset);
  if (level) parts.push(level);
  if (filters.instrument) parts.push(filters.instrument);
  if (filters.filter) parts.push(`filter ${filters.filter}`);
  if (filters.dateFrom) parts.push(`from ${filters.dateFrom}`);
  if (filters.dateTo) parts.push(`to ${filters.dateTo}`);
  if (filters.publicOnly) parts.push("public only");
  parts.push(...objectLabels(filters, groups));
  return parts.join(", ");
}

function since(now: number, start: number): string | null {
  const ms = now - start;
  return ms >= ELAPSED_AFTER_MS ? elapsedText(ms) : null;
}

interface ProgressContext {
  missionLabel: string;
  radiusText: string;
  filters: MastFilterValues;
  now: number;
  groups?: readonly MastFilterGroup[];
  preset?: MastPreset | null;
}

function searchLine(run: RunLike, ctx: ProgressContext): ProgressLine {
  const where = run.kind === "programme" ? `programme ${run.text}` : run.kind === "dataset" ? `dataset ${run.text}` : `r ${ctx.radiusText.trim().replace(/'$/, "")}'`;
  const filters = filterSummary(ctx.filters, ctx.groups, ctx.preset ?? null);
  const text = [`Searching ${ctx.missionLabel}`, where, filters].filter(Boolean).join(" ");
  return { key: "search", text, elapsed: since(ctx.now, run.searchStartedAt) };
}

export function searchProgress(run: RunLike, ctx: ProgressContext): SearchProgressView {
  const lines: ProgressLine[] = [];
  const lookupName = run.name ?? run.text;
  if (run.phase === "resolving") {
    lines.push({ key: "resolve", text: `Resolving "${lookupName}" with MAST name lookup`, elapsed: since(ctx.now, run.startedAt) });
  } else {
    if (run.resolved) {
      const r = run.resolved;
      lines.push({ key: "resolve", text: `Resolving "${lookupName}" with MAST name lookup`, elapsed: elapsedText(r.elapsedMs) });
      const type = r.objectType ? ` ${r.objectType}` : "";
      lines.push({ key: "resolved", text: `= ${r.resolver} "${r.canonicalName}" ${coordinatesText(r.raDeg, r.decDeg)}${type}`, elapsed: null });
    }
    lines.push(searchLine(run, ctx));
  }
  return { lines, slowNote: ctx.now - run.startedAt >= SLOW_AFTER_MS ? SLOW_NOTE : null };
}

function chipsOf(filters: MastFilterValues, groups: readonly MastFilterGroup[], preset: MastPreset | null): FilterChip[] {
  const chips: FilterChip[] = [];
  const level = userLevelLabel(filters, preset);
  if (level) chips.push({ id: "level", label: level });
  if (filters.instrument) chips.push({ id: "instrument", label: filters.instrument });
  if (filters.filter) chips.push({ id: "filter", label: `Filter ${filters.filter}` });
  if (filters.dateFrom) chips.push({ id: "dateFrom", label: `From ${filters.dateFrom}` });
  if (filters.dateTo) chips.push({ id: "dateTo", label: `To ${filters.dateTo}` });
  if (filters.publicOnly) chips.push({ id: "publicOnly", label: "Public only" });
  for (const chip of objectChips(filters, groups)) chips.push({ id: chip.id, label: chip.label });
  return chips;
}

function chipFits(chip: FilterChip, other: MastMission, quick: ReadonlySet<string>): boolean {
  if (chip.id === "level") return true;
  const object = parseObjectChip(chip.id);
  if (!object) return quick.has(CHIP_FILTER[chip.id as BaseChipId]);
  const group = other.filterGroups.find((g) => OBJECT_KEY[g.id] === object.key);
  return offeredIds([object.option], group).length === 1;
}

function tryOnOf(mission: MastMission, missions: readonly MastMission[], chips: readonly FilterChip[]): EmptyView["tryOn"] {
  const otherId = OTHER_MISSION[mission.id];
  const other = otherId ? missions.find((m) => m.id === otherId) : undefined;
  if (!other || other.availability !== "open" || !other.capabilities.search) return null;
  const quick = new Set<string>(other.quickFilters.map((f) => f.id));
  return chips.every((c) => chipFits(c, other, quick)) ? { mission: other.id, label: `Try on ${other.label}` } : null;
}

function classLineOf(filters: MastFilterValues, preset: MastPreset | null): string | null {
  if (!hasObjectFilters(filters)) return null;
  return preset?.id === PURE_PARALLEL_PRESET ? PURE_PARALLEL_NOTE : CLASS_LINE;
}

export function emptyView(input: { query: MastQuery | null; results: ResultSet; missions: readonly MastMission[]; mission: MastMission; presets?: readonly MastPreset[] }): EmptyView {
  const { query, results, missions, mission } = input;
  const label = mission.label;
  const radius = radiusOf(results, query);
  const pos = positionOf(results, query);
  let title = `No ${label} datasets`;
  let widenTo: number | null = null;
  if (query?.program !== null && query?.program !== undefined) title = `No ${label} datasets in programme ${query.program}`;
  else if (query?.datasetId) title = `No ${label} dataset called ${query.datasetId}`;
  else if (radius !== null) {
    title = pos ? `No ${label} datasets within ${radius}' of ${coordinatesText(pos.raDeg, pos.decDeg)}` : `No ${label} datasets within ${radius}'`;
    widenTo = WIDEN_STEPS.find((step) => step > radius) ?? null;
  }
  const filters = query?.filters ?? {};
  const preset = presetById(input.presets ?? [], query?.preset);
  const chips = chipsOf(filters, mission.filterGroups, preset);
  const objects = objectLabels(filters, mission.filterGroups);
  return {
    title: objects.length ? `${title} with ${objects.join(", ")}` : title,
    chips,
    widenTo,
    clearFilters: chips.some((c) => c.id !== "level"),
    allLevels: mission.id === "jwst" && narrowsLevel(filters.level) && presetLevel(preset) === null,
    tryOn: tryOnOf(mission, missions, chips),
    classLine: classLineOf(filters, preset),
  };
}

export function isLevelSearch(form: Pick<ArchiveForm, "mission" | "filters" | "preset">, presets: readonly MastPreset[]): boolean {
  return form.mission === "jwst" && narrowsLevel(form.filters.level) && presetLevel(presetById(presets, form.preset)) === null;
}

export function notResolvedView(name: string, elapsedMs: number): { title: string; programmeOffer: boolean } {
  return {
    title: `MAST name lookup found no object called "${name}" (${elapsedText(elapsedMs)})`,
    programmeOffer: interpretQuery(name, "programme").error === null,
  };
}

export function footerMastText(total: number, thumbs: number, queued: number): string {
  if (total === 0 && queued === 0) return "MAST idle";
  const head = `MAST ${total} in flight`;
  if (thumbs === 0 && queued === 0) return head;
  return `${head} (${thumbs} ${thumbs === 1 ? "thumbnail" : "thumbnails"}, ${queued} queued)`;
}

export function queuedThumbs(q: ThumbQueue): number {
  const seen = new Set<string>();
  for (const key of [...q.selected, ...q.visible, ...q.ahead]) {
    if (!q.inflight.includes(key) && !(key in q.done)) seen.add(key);
  }
  return seen.size;
}

export function isDatasetColumn(column: Pick<MastColumn, "name">): boolean {
  return DATASET_COLUMNS.has(column.name);
}

export function tableColumns(mission: MastMission | undefined, results: ResultSet): MastColumn[] {
  const byName = new Map(results.columns.map((c) => [c.name, c]));
  const defaults = mission?.defaultColumns ?? [];
  const chosen = defaults.flatMap((d) => {
    const answered = byName.get(d.name);
    return answered ? [{ ...d, kind: answered.kind, sortable: answered.sortable, unit: d.unit ?? answered.unit }] : [];
  });
  return chosen.length ? chosen : [...results.columns];
}

export function columnWidth(column: MastColumn, first = false): number {
  if (first || isDatasetColumn(column)) return 212;
  if (column.kind === "datetime") return 140;
  if (column.kind === "int" || column.kind === "float") return 84;
  return 112;
}

export function fitColumns(columns: readonly MastColumn[], width: number): MastColumn[] {
  const out: MastColumn[] = [];
  let used = 0;
  for (const [i, c] of columns.entries()) {
    const w = columnWidth(c, i === 0);
    if (i > 0 && used + w > width) break;
    out.push(c);
    used += w;
  }
  return out;
}

export function isUnknownCell(column: Pick<MastColumn, "name">, value: string | number | boolean | null | undefined): boolean {
  return column.name === CATEGORY_COLUMN && (value === null || value === undefined || value === "");
}

export function cellText(value: string | number | boolean | null | undefined, column: Pick<MastColumn, "kind">): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "boolean") return value ? "yes" : "no";
  if (column.kind === "datetime" && typeof value === "string") return value.replace("T", " ").slice(0, 16);
  if (column.kind === "float" && typeof value === "number") return floatText(value);
  return String(value);
}

function floatText(value: number): string {
  if (Number.isInteger(value) || !Number.isFinite(value)) return String(value);
  return Math.abs(value) >= 1000 ? value.toFixed(1) : String(Number(value.toPrecision(4)));
}

export function displayRows(results: ResultSet | null, sort: TableSort): MastRow[] {
  if (!results) return [];
  if (sort.by === "local" && sort.keys.length) return sortRows(results.rows, sort.keys, results.columns);
  return results.rows;
}

export function rowKeys(rows: readonly MastRow[]): string[] {
  const used = new Set<string>();
  return rows.map((r) => {
    let key = r.datasetId;
    for (let n = 2; used.has(key); n += 1) key = `${r.datasetId}#${n}`;
    used.add(key);
    return key;
  });
}

const keyMaps = new WeakMap<readonly MastRow[], Map<MastRow, string>>();

export function rowKeyMap(rows: readonly MastRow[]): Map<MastRow, string> {
  const known = keyMaps.get(rows);
  if (known) return known;
  const keys = rowKeys(rows);
  const map = new Map(rows.map((r, i) => [r, keys[i]]));
  keyMaps.set(rows, map);
  return map;
}

export function rovingRow(rows: readonly MastRow[], keyOf: ReadonlyMap<MastRow, string>, selectedId: string | null, selectedKey: string | null): number {
  if (selectedId === null) return 0;
  if (selectedKey !== null) {
    const exact = rows.findIndex((r) => r.datasetId === selectedId && keyOf.get(r) === selectedKey);
    if (exact >= 0) return exact;
  }
  return Math.max(0, rows.findIndex((r) => r.datasetId === selectedId));
}

function sortLabel(column: string, columns: readonly MastColumn[]): string {
  return (columns.find((c) => c.name === column)?.label ?? column).toUpperCase();
}

export function sortText(sort: TableSort, columns: readonly MastColumn[], results: ResultSet): string {
  if (sort.keys.length === 0) return "";
  const keys = sort.keys.map((k) => `${sortLabel(k.column, columns)} ${k.desc ? "desc" : "asc"}`).join(", ");
  if (sort.by === "server") return `Sorted by MAST: ${keys}`;
  return `Sorted locally by ${keys} (all ${results.rows.length} rows loaded)`;
}

export function rowsText(first: number, last: number, results: ResultSet): string {
  const n = results.rows.length;
  if (n === 0) return "No rows";
  return `Rows ${first}-${last} of ${n}${results.hasMore ? "+" : ""}`;
}
