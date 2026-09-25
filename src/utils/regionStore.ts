import type { Region, RegionShape, RegionShapeKind, RegionTool } from "../shared/types";
import { loadRegions, saveRegions, type RegionStorage } from "./regionPersistence";

export interface RegionDoc {
  regions: Region[];
  selectedId: string | null;
  version: number;
}

export const SAVE_DEBOUNCE_MS = 200;

export const EMPTY_DOC: RegionDoc = Object.freeze({ regions: [], selectedId: null, version: 0 }) as RegionDoc;

type Listener = () => void;

export class RegionStoreCore {
  private docs = new Map<string, RegionDoc>();
  private listeners = new Set<Listener>();
  private tool: RegionTool = "none";
  private draft: { fileKey: string; shape: RegionShape } | null = null;
  private pending = new Set<string>();
  private deferred = new Set<string>();
  private timer: ReturnType<typeof setTimeout> | null = null;
  private readonly storage: RegionStorage | null;
  private readonly now: () => number;
  private lastSavedAt: number | null = null;

  constructor(storage: RegionStorage | null, now: () => number = () => Date.now()) {
    this.storage = storage;
    this.now = now;
  }

  subscribe = (l: Listener): (() => void) => {
    this.listeners.add(l);
    return () => {
      this.listeners.delete(l);
    };
  };

  private notify(): void {
    this.listeners.forEach((l) => l());
  }

  getDoc(fileKey: string): RegionDoc {
    let doc = this.docs.get(fileKey);
    if (!doc) {
      doc = { regions: loadRegions(fileKey, this.storage), selectedId: null, version: 0 };
      this.docs.set(fileKey, doc);
    }
    return doc;
  }

  getTool = (): RegionTool => this.tool;

  setTool(t: RegionTool): void {
    if (this.tool === t) return;
    this.tool = t;
    this.notify();
  }

  getDraftFor = (fileKey: string | null): RegionShape | null =>
    fileKey !== null && this.draft !== null && this.draft.fileKey === fileKey ? this.draft.shape : null;

  getDraftKey = (): string | null => this.draft?.fileKey ?? null;

  setDraft(fileKey: string, shape: RegionShape | null): void {
    if (shape === null) {
      this.clearDraft(fileKey);
      return;
    }
    if (this.draft !== null && this.draft.fileKey === fileKey && this.draft.shape === shape) return;
    this.draft = { fileKey, shape };
    this.notify();
  }

  clearDraft(fileKey?: string): void {
    if (this.draft === null) return;
    if (fileKey !== undefined && this.draft.fileKey !== fileKey) return;
    this.draft = null;
    this.notify();
  }

  private commit(fileKey: string, next: Omit<RegionDoc, "version">, persist: boolean): void {
    const prev = this.getDoc(fileKey);
    this.docs.set(fileKey, { ...next, version: prev.version + 1 });
    if (persist) this.schedulePersist(fileKey);
    this.notify();
  }

  private schedulePersist(fileKey: string): void {
    this.pending.add(fileKey);
    if (this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      this.flush();
    }, SAVE_DEBOUNCE_MS);
  }

  persistNow(fileKey: string): void {
    this.pending.add(fileKey);
    this.flush();
  }

  flush(): void {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    for (const key of this.deferred) this.pending.add(key);
    this.deferred.clear();
    if (this.pending.size === 0) return;
    for (const key of this.pending) {
      const doc = this.docs.get(key);
      if (doc) saveRegions(key, doc.regions, this.storage);
    }
    this.pending.clear();
    this.lastSavedAt = this.now();
  }

  add(fileKey: string, region: Region): void {
    const doc = this.getDoc(fileKey);
    if (doc.regions.some((r) => r.id === region.id)) return;
    this.commit(fileKey, { regions: [...doc.regions, region], selectedId: doc.selectedId }, true);
  }

  update(fileKey: string, id: string, patch: Partial<Region>, persist = true): void {
    const doc = this.getDoc(fileKey);
    const idx = doc.regions.findIndex((r) => r.id === id);
    if (idx < 0) return;
    const regions = doc.regions.slice();
    regions[idx] = { ...regions[idx], ...patch, id };
    this.commit(fileKey, { regions, selectedId: doc.selectedId }, persist);
    if (!persist) this.deferred.add(fileKey);
  }

  remove(fileKey: string, id: string): void {
    const doc = this.getDoc(fileKey);
    if (!doc.regions.some((r) => r.id === id)) return;
    const regions = doc.regions
      .filter((r) => r.id !== id)
      .map((r) => (r.backgroundId === id ? { ...r, backgroundId: null } : r));
    const selectedId = doc.selectedId === id ? null : doc.selectedId;
    this.commit(fileKey, { regions, selectedId }, true);
  }

  removeShapeKind(fileKey: string, kind: RegionShapeKind): void {
    const doc = this.getDoc(fileKey);
    const removed = new Set(doc.regions.filter((r) => r.shape.shape === kind).map((r) => r.id));
    if (removed.size === 0) return;
    const regions = doc.regions
      .filter((r) => !removed.has(r.id))
      .map((r) => (r.backgroundId !== null && removed.has(r.backgroundId) ? { ...r, backgroundId: null } : r));
    const selectedId = doc.selectedId !== null && removed.has(doc.selectedId) ? null : doc.selectedId;
    this.commit(fileKey, { regions, selectedId }, true);
  }

  select(fileKey: string, id: string | null): void {
    const doc = this.getDoc(fileKey);
    const target = id !== null && doc.regions.some((r) => r.id === id) ? id : null;
    if (doc.selectedId === target) return;
    this.commit(fileKey, { regions: doc.regions, selectedId: target }, false);
  }

  setBackground(fileKey: string, id: string, backgroundId: string | null): void {
    const doc = this.getDoc(fileKey);
    const valid = backgroundId !== null && backgroundId !== id && doc.regions.some((r) => r.id === backgroundId);
    this.update(fileKey, id, { backgroundId: valid ? backgroundId : null });
  }

  replaceAll(fileKey: string, regions: Region[]): void {
    const doc = this.getDoc(fileKey);
    const selectedId = regions.some((r) => r.id === doc.selectedId) ? doc.selectedId : null;
    this.commit(fileKey, { regions: regions.slice(), selectedId }, true);
  }

  clear(fileKey: string): void {
    this.commit(fileKey, { regions: [], selectedId: null }, true);
  }

  getLastSavedAt(): number | null {
    return this.lastSavedAt;
  }
}

export function shapeKindKey(regions: readonly Region[], kind: RegionShapeKind): string {
  return JSON.stringify(regions.filter((r) => r.shape.shape === kind).map((r) => r.id).sort());
}

function browserStorage(): RegionStorage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export const regionStore = new RegionStoreCore(browserStorage());

if (typeof window !== "undefined") {
  window.addEventListener("pagehide", () => regionStore.flush());
  if (typeof document !== "undefined") {
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "hidden") regionStore.flush();
    });
  }
}
