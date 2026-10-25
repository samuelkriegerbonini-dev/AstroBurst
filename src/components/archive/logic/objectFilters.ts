import type { MastFilterGroup, MastFilterValues, ObjectCategoryId, ObjectTypeId } from "../../../shared/types/mast";

export type ObjectGroupId = MastFilterGroup["id"];
export type ObjectKey = "categories" | "objectTypes";
export type ObjectChipId = `category:${ObjectCategoryId}` | `objectType:${ObjectTypeId}`;
export type ObjectFilters = Pick<MastFilterValues, ObjectKey>;

export interface ObjectChip {
  id: ObjectChipId;
  group: ObjectGroupId;
  option: string;
  label: string;
}

const OBJECT_GROUPS = {
  category: { key: "categories", prefix: "category" },
  object_type: { key: "objectTypes", prefix: "objectType" },
} as const satisfies Record<ObjectGroupId, { key: ObjectKey; prefix: string }>;

export const OBJECT_KEY: Record<ObjectGroupId, ObjectKey> = { category: OBJECT_GROUPS.category.key, object_type: OBJECT_GROUPS.object_type.key };
export const NO_OBJECT_FILTERS: ObjectFilters = { categories: undefined, objectTypes: undefined };

const GROUP_OF_PREFIX = new Map<string, (typeof OBJECT_GROUPS)[ObjectGroupId]>(Object.values(OBJECT_GROUPS).map((g) => [g.prefix, g]));

export function offeredIds(ids: readonly string[] | undefined, group: MastFilterGroup | undefined): string[] {
  if (!ids?.length || !group) return [];
  const wanted = new Set(ids);
  return group.options.map((o) => o.id).filter((id) => wanted.has(id));
}

export function offeredObjectFilters(filters: MastFilterValues, groups: readonly MastFilterGroup[]): ObjectFilters {
  const out: Partial<Record<ObjectKey, string[]>> = {};
  for (const group of groups) {
    const key = OBJECT_KEY[group.id];
    const ids = offeredIds(filters[key], group);
    if (ids.length) out[key] = ids;
  }
  return out as ObjectFilters;
}

export function hasObjectFilters(filters: MastFilterValues | undefined): boolean {
  return !!(filters?.categories?.length || filters?.objectTypes?.length);
}

export function objectChips(filters: MastFilterValues, groups: readonly MastFilterGroup[]): ObjectChip[] {
  return groups.flatMap((group) =>
    offeredIds(filters[OBJECT_KEY[group.id]], group).map((option) => ({
      id: `${OBJECT_GROUPS[group.id].prefix}:${option}` as ObjectChipId,
      group: group.id,
      option,
      label: group.options.find((o) => o.id === option)?.label ?? option,
    })),
  );
}

export function objectLabels(filters: MastFilterValues, groups: readonly MastFilterGroup[]): string[] {
  return objectChips(filters, groups).map((c) => c.label);
}

export function parseObjectChip(id: string): { key: ObjectKey; option: string } | null {
  const at = id.indexOf(":");
  const group = at > 0 ? GROUP_OF_PREFIX.get(id.slice(0, at)) : undefined;
  const option = id.slice(at + 1);
  return group && option ? { key: group.key, option } : null;
}

export function chipAfterRemoval<T>(chips: readonly T[], index: number): T | null {
  return chips[index + 1] ?? chips[index - 1] ?? null;
}

export function withoutOption(filters: MastFilterValues, key: ObjectKey, option: string): ObjectFilters {
  return { [key]: (filters[key] ?? []).filter((id) => id !== option) } as ObjectFilters;
}

export function toggledOption(filters: MastFilterValues, group: MastFilterGroup, option: string): ObjectFilters {
  const key = OBJECT_KEY[group.id];
  const current = filters[key] ?? [];
  const next = current.some((id) => id === option) ? current.filter((id) => id !== option) : [...current, option];
  return { [key]: offeredIds(next, group) } as ObjectFilters;
}

export function objectChangeText(before: MastFilterValues, after: MastFilterValues, groups: readonly MastFilterGroup[]): string | null {
  const was = objectChips(before, groups);
  const now = objectChips(after, groups);
  const added = now.filter((c) => !was.some((w) => w.id === c.id));
  const removed = was.filter((c) => !now.some((n) => n.id === c.id));
  if (added.length + removed.length === 0) return null;
  if (added.length === 1 && removed.length === 0) return `Filter ${added[0].label} added`;
  if (removed.length === 1 && added.length === 0) return `Filter ${removed[0].label} removed`;
  return now.length ? `Filters ${now.map((c) => c.label).join(", ")}` : "Object filters cleared";
}
