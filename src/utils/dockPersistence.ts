import {
  DEFAULT_DOCK_LAYOUT,
  DOCK_ANCHORS,
  DOCK_SIZE_LIMITS,
  clampSize,
  dockLayoutErrors,
  isDockToolId,
  type DockAnchor,
  type DockLayout,
  type DockSizes,
  type DockToolId,
} from "./dockLayout";

export const DOCK_LAYOUT_STORAGE_KEY = "ab.layout.dock.v1";
export const DOCK_LAYOUT_VERSION = 1 as const;
export const LEGACY_SIZE_KEYS = ["ab.layout.sidebarW", "ab.layout.rightW", "ab.layout.bottomH"] as const;

export interface PersistedDockLayout {
  version: 1;
  anchors: Record<DockAnchor, string[]>;
  active: Record<DockAnchor, string | null>;
  sizes: Partial<Record<keyof DockSizes, number>>;
}

export const LEGACY_TOOL_ALIASES: Readonly<Record<string, readonly DockToolId[]>> = {
  analysis: ["image", "astrometry", "photometry", "cube"],
};

export const LEGACY_ACTIVE_ALIASES: Readonly<Record<string, DockToolId>> = { analysis: "image" };

function legacyToolAlias(id: unknown): readonly DockToolId[] | null {
  return typeof id === "string" && Object.hasOwn(LEGACY_TOOL_ALIASES, id) ? LEGACY_TOOL_ALIASES[id] : null;
}

function legacyActiveAlias(id: unknown): DockToolId | null {
  return typeof id === "string" && Object.hasOwn(LEGACY_ACTIVE_ALIASES, id) ? LEGACY_ACTIVE_ALIASES[id] : null;
}

const LEGACY_SIZE_TARGETS: Record<(typeof LEGACY_SIZE_KEYS)[number], keyof DockSizes> = {
  "ab.layout.sidebarW": "leftW",
  "ab.layout.rightW": "rightW",
  "ab.layout.bottomH": "bottomH",
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseDockLayout(raw: unknown, fallback: DockLayout = DEFAULT_DOCK_LAYOUT): DockLayout {
  if (!isRecord(raw) || raw.version !== DOCK_LAYOUT_VERSION || !isRecord(raw.anchors)) return fallback;
  const rawAnchors = raw.anchors;
  const rawLists = DOCK_ANCHORS.map((anchor) => {
    const list = rawAnchors[anchor];
    return Array.isArray(list) ? (list as unknown[]) : [];
  });
  const rawKnown = new Set(rawLists.flat().filter(isDockToolId));
  const seen = new Set<DockToolId>();
  const anchors = {} as Record<DockAnchor, DockToolId[]>;
  DOCK_ANCHORS.forEach((anchor, i) => {
    anchors[anchor] = [];
    for (const id of rawLists[i]) {
      const expanded = isDockToolId(id) ? [id] : (legacyToolAlias(id) ?? []).filter((alias) => !rawKnown.has(alias));
      for (const tool of expanded) {
        if (seen.has(tool)) continue;
        seen.add(tool);
        anchors[anchor].push(tool);
      }
    }
  });
  for (const anchor of DOCK_ANCHORS) {
    for (const id of DEFAULT_DOCK_LAYOUT.anchors[anchor]) {
      if (!seen.has(id)) anchors[anchor].push(id);
    }
  }
  const rawActive = isRecord(raw.active) ? raw.active : {};
  const active = {} as Record<DockAnchor, DockToolId | null>;
  for (const anchor of DOCK_ANCHORS) {
    const id = rawActive[anchor];
    const wanted = isDockToolId(id) ? id : legacyActiveAlias(id);
    active[anchor] = wanted !== null && anchors[anchor].includes(wanted) ? wanted : null;
  }
  const rawSizes = isRecord(raw.sizes) ? raw.sizes : {};
  const sizes = {} as DockSizes;
  for (const key of Object.keys(DOCK_SIZE_LIMITS) as (keyof DockSizes)[]) {
    const value = rawSizes[key];
    sizes[key] = typeof value === "number" && Number.isFinite(value) ? clampSize(key, value) : DOCK_SIZE_LIMITS[key].default;
  }
  const layout: DockLayout = { anchors, active, sizes };
  return dockLayoutErrors(layout).length === 0 ? layout : fallback;
}

export function serializeDockLayout(layout: DockLayout): string {
  const persisted: PersistedDockLayout = {
    version: DOCK_LAYOUT_VERSION,
    anchors: layout.anchors,
    active: layout.active,
    sizes: layout.sizes,
  };
  return JSON.stringify(persisted);
}

export function migrateLegacySizes(read: (key: string) => string | null): Partial<DockSizes> {
  const sizes: Partial<DockSizes> = {};
  for (const key of LEGACY_SIZE_KEYS) {
    const raw = read(key);
    if (raw === null || raw.trim() === "") continue;
    const value = Number(raw);
    if (!Number.isFinite(value)) continue;
    const target = LEGACY_SIZE_TARGETS[key];
    sizes[target] = clampSize(target, value);
  }
  return sizes;
}
