import type { MastFilterValues, MastMissionId, MastQuery, MastQuickFilter, MastResolved } from "../../../shared/types/mast";
import { interpretQuery, type QueryKind } from "./interpretQuery";

export interface ArchiveForm {
  mission: MastMissionId;
  text: string;
  kindOverride: QueryKind | null;
  radiusText: string;
  filters: MastFilterValues;
  preset: string | null;
}

export interface SortKey {
  column: string;
  desc: boolean;
}

export type QuickFilterId = MastQuickFilter["id"];

export type SearchPlan =
  | { kind: "blocked"; field: "query" | "radius"; message: string }
  | { kind: "resolve"; name: string }
  | { kind: "search"; query: MastQuery };

export interface PlanOptions {
  resolved?: MastResolved | null;
  serverResolve?: boolean;
  offset?: number;
  sort?: SortKey[];
}

export type LevelStorage = Partial<Pick<Storage, "getItem" | "setItem">>;

export const PAGE_LIMIT = 200;
export const DEFAULT_RADIUS_ARCMIN = 3;
export const EMPTY_QUERY_HINT = "Add a target, coordinates, programme or dataset id";
export const RADIUS_HINT = "Radius: type a number of arcminutes, for example 3";
export const LEVEL_STORAGE_KEY = "astroburst.mast.level";

export const DEFAULT_FORM: ArchiveForm = {
  mission: "jwst",
  text: "",
  kindOverride: null,
  radiusText: String(DEFAULT_RADIUS_ARCMIN),
  filters: { level: "combined" },
  preset: null,
};

export function parseRadius(text: string): number | null {
  const m = /^\s*(\d+(?:\.\d*)?|\.\d+)\s*'?\s*$/.exec(text);
  if (!m) return null;
  const value = Number(m[1]);
  return Number.isFinite(value) && value > 0 ? value : null;
}

function cleanText(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

export function applicableFilters(filters: MastFilterValues, available: readonly QuickFilterId[]): MastFilterValues {
  const out: MastFilterValues = {};
  const has = (id: QuickFilterId) => available.includes(id);
  if (has("level") && filters.level) out.level = filters.level;
  const instrument = cleanText(filters.instrument);
  if (has("instrument") && instrument) out.instrument = instrument;
  const filter = cleanText(filters.filter);
  if (has("filter") && filter) out.filter = filter.toUpperCase();
  const dateFrom = cleanText(filters.dateFrom);
  const dateTo = cleanText(filters.dateTo);
  if (has("date") && dateFrom) out.dateFrom = dateFrom;
  if (has("date") && dateTo) out.dateTo = dateTo;
  if (has("public") && filters.publicOnly) out.publicOnly = true;
  return out;
}

function query(form: ArchiveForm, where: MastQuery["where"], available: readonly QuickFilterId[], opts: PlanOptions, extra: Partial<MastQuery> = {}): SearchPlan {
  return {
    kind: "search",
    query: {
      mission: form.mission,
      where,
      program: null,
      datasetId: null,
      filters: applicableFilters(form.filters, available),
      preset: form.preset,
      sort: (opts.sort ?? []).map((s) => ({ column: s.column, desc: s.desc })),
      offset: opts.offset ?? 0,
      limit: PAGE_LIMIT,
      ...extra,
    },
  };
}

export function planSearch(form: ArchiveForm, available: readonly QuickFilterId[], opts: PlanOptions = {}): SearchPlan {
  const meaning = interpretQuery(form.text, form.kindOverride);
  if (meaning.kind === "empty") return { kind: "blocked", field: "query", message: EMPTY_QUERY_HINT };
  if (meaning.error) return { kind: "blocked", field: "query", message: meaning.error };
  if (meaning.kind === "programme" && meaning.program !== null) {
    return query(form, { kind: "none" }, available, opts, { program: meaning.program });
  }
  if (meaning.kind === "dataset" && meaning.datasetId !== null) {
    return query(form, { kind: "none" }, [], opts, { datasetId: meaning.datasetId, preset: null });
  }
  const radius = parseRadius(form.radiusText);
  if (radius === null) return { kind: "blocked", field: "radius", message: RADIUS_HINT };
  if (meaning.kind === "coordinates" && meaning.raDeg !== null && meaning.decDeg !== null) {
    return query(form, { kind: "position", raDeg: meaning.raDeg, decDeg: meaning.decDeg, radiusArcmin: radius, label: null }, available, opts);
  }
  const name = meaning.name ?? meaning.text;
  if (opts.resolved) {
    const r = opts.resolved;
    return query(form, { kind: "position", raDeg: r.raDeg, decDeg: r.decDeg, radiusArcmin: radius, label: r.canonicalName }, available, opts);
  }
  if (opts.serverResolve) return query(form, { kind: "name", name, radiusArcmin: radius }, available, opts);
  return { kind: "resolve", name };
}

export function loadLevel(storage: LevelStorage | null): "combined" | "all" {
  try {
    const value = storage?.getItem?.(LEVEL_STORAGE_KEY);
    return value === "all" ? "all" : "combined";
  } catch {
    return "combined";
  }
}

export function saveLevel(storage: LevelStorage | null, level: "combined" | "all"): void {
  try {
    storage?.setItem?.(LEVEL_STORAGE_KEY, level);
  } catch {
    return;
  }
}

export function browserLevelStorage(): LevelStorage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

export function switchMission(form: ArchiveForm, mission: MastMissionId, storage: LevelStorage | null): ArchiveForm {
  const filters: MastFilterValues = {};
  if (form.filters.dateFrom) filters.dateFrom = form.filters.dateFrom;
  if (form.filters.dateTo) filters.dateTo = form.filters.dateTo;
  if (mission === "jwst") filters.level = loadLevel(storage);
  return { mission, text: form.text, kindOverride: form.kindOverride, radiusText: form.radiusText, filters, preset: null };
}
