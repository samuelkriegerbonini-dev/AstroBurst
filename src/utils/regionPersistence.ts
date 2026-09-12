import type { Region, RegionProperties } from "../shared/types/regions";
import { isRegionShape } from "./regionGeometry";

export const STORAGE_PREFIX = "astroburst.regions.v1:";

export type RegionStorage = Pick<Storage, "getItem" | "setItem">;

export const DEFAULT_REGION_PROPS: RegionProperties = {
  color: null,
  width: null,
  text: null,
  dash: null,
  include: true,
};

export function normalizeProps(raw: unknown): RegionProperties {
  if (!raw || typeof raw !== "object") return { ...DEFAULT_REGION_PROPS };
  const o = raw as Record<string, unknown>;
  return {
    color: typeof o.color === "string" ? o.color : null,
    width: typeof o.width === "number" && Number.isFinite(o.width) ? o.width : null,
    text: typeof o.text === "string" ? o.text : null,
    dash: typeof o.dash === "boolean" ? o.dash : null,
    include: typeof o.include === "boolean" ? o.include : true,
  };
}

export function serializeRegions(regions: Region[]): string {
  return JSON.stringify(regions);
}

export function parseRegions(text: string | null): Region[] {
  if (!text) return [];
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return [];
  }
  if (!Array.isArray(raw)) return [];
  const out: Region[] = [];
  const seen = new Set<string>();
  for (const entry of raw) {
    if (!entry || typeof entry !== "object") continue;
    const o = entry as Record<string, unknown>;
    if (typeof o.id !== "string" || o.id.length === 0 || seen.has(o.id)) continue;
    if (!isRegionShape(o.shape)) continue;
    seen.add(o.id);
    out.push({
      id: o.id,
      shape: o.shape,
      props: normalizeProps(o.props),
      backgroundId: typeof o.backgroundId === "string" ? o.backgroundId : null,
    });
  }
  const ids = new Set(out.map((r) => r.id));
  return out.map((r) => (r.backgroundId && !ids.has(r.backgroundId) ? { ...r, backgroundId: null } : r));
}

export function loadRegions(fileKey: string, storage: RegionStorage | null): Region[] {
  if (!storage) return [];
  try {
    return parseRegions(storage.getItem(STORAGE_PREFIX + fileKey));
  } catch {
    return [];
  }
}

export function saveRegions(fileKey: string, regions: Region[], storage: RegionStorage | null): void {
  if (!storage) return;
  try {
    storage.setItem(STORAGE_PREFIX + fileKey, serializeRegions(regions));
  } catch {
    return;
  }
}
