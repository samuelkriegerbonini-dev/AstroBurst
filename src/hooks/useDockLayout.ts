import { useSyncExternalStore } from "react";
import { DEFAULT_DOCK_LAYOUT, dockReducer, type DockAction, type DockLayout, type DockSizes } from "../utils/dockLayout";
import {
  DOCK_LAYOUT_STORAGE_KEY,
  LEGACY_SIZE_KEYS,
  migrateLegacySizes,
  parseDockLayout,
  serializeDockLayout,
} from "../utils/dockPersistence";

function loadInitial(): DockLayout {
  try {
    const raw = localStorage.getItem(DOCK_LAYOUT_STORAGE_KEY);
    if (raw !== null) return parseDockLayout(JSON.parse(raw));
    const legacy = migrateLegacySizes((key) => localStorage.getItem(key));
    if (Object.keys(legacy).length === 0) return DEFAULT_DOCK_LAYOUT;
    return { ...DEFAULT_DOCK_LAYOUT, sizes: { ...DEFAULT_DOCK_LAYOUT.sizes, ...legacy } };
  } catch {
    return DEFAULT_DOCK_LAYOUT;
  }
}

function write(layout: DockLayout): void {
  try {
    localStorage.setItem(DOCK_LAYOUT_STORAGE_KEY, serializeDockLayout(layout));
  } catch { }
}

function persists(action: DockAction): boolean {
  return action.type !== "resize" || action.persist !== false;
}

let layout = loadInitial();
let savedSizes: DockSizes = layout.sizes;
const listeners = new Set<() => void>();

function savedSizesAfter(action: DockAction, next: DockLayout): DockSizes {
  if (action.type === "reset") return next.sizes;
  if (action.type !== "resize" || !persists(action)) return savedSizes;
  const value = next.sizes[action.key];
  return savedSizes[action.key] === value ? savedSizes : { ...savedSizes, [action.key]: value };
}

export const dockStore = {
  subscribe(cb: () => void): () => void {
    listeners.add(cb);
    return () => { listeners.delete(cb); };
  },
  get(): DockLayout {
    return layout;
  },
  dispatch(action: DockAction): void {
    const next = dockReducer(layout, action);
    const nextSaved = savedSizesAfter(action, next);
    const changed = next !== layout;
    if (!changed && nextSaved === savedSizes) return;
    layout = next;
    savedSizes = nextSaved;
    if (persists(action)) write({ ...next, sizes: savedSizes });
    if (changed) listeners.forEach((l) => l());
  },
  resetAll(): void {
    dockStore.dispatch({ type: "reset" });
    for (const key of [DOCK_LAYOUT_STORAGE_KEY, ...LEGACY_SIZE_KEYS]) {
      try {
        localStorage.removeItem(key);
      } catch { }
    }
  },
};

export function useDockLayout(): DockLayout {
  return useSyncExternalStore(dockStore.subscribe, dockStore.get, dockStore.get);
}
