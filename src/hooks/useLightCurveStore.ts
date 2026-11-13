import { useCallback, useEffect, useSyncExternalStore } from "react";
import { DEFAULT_LIGHT_CURVE_VIEW, type LightCurveDoc, type LightCurveViewState } from "../shared/types/lightCurve";

export const MAX_LIGHT_CURVE_GROUPS = 16;
export const EMPTY_LIGHT_CURVES: readonly LightCurveDoc[] = Object.freeze([]);

type Listener = () => void;

function sameTimeBase(a: LightCurveDoc, b: LightCurveDoc): boolean {
  return a.timeScale === b.timeScale && a.time.length === b.time.length;
}

export class LightCurveStore {
  private docs = new Map<string, readonly LightCurveDoc[]>();
  private views = new Map<string, LightCurveViewState>();
  private holders = new Map<string, number>();
  private groupLists = new Map<string, readonly string[]>();
  private activeTable: string | null = null;
  private listeners = new Set<Listener>();

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private notify(): void {
    this.listeners.forEach((l) => l());
  }

  private drop(group: string): void {
    this.docs.delete(group);
    this.views.delete(group);
    this.groupLists.clear();
    if (this.activeTable === group) this.activeTable = null;
  }

  private resetTimeBoundView(group: string): void {
    const view = this.views.get(group);
    if (!view) return;
    const { window, marginDays, excluded, fold } = DEFAULT_LIGHT_CURVE_VIEW;
    this.views.set(group, Object.freeze({ ...view, window, marginDays, excluded, fold }));
  }

  put(group: string, docs: LightCurveDoc[]): void {
    if (docs.length === 0) {
      this.remove(group);
      return;
    }
    const before = this.docs.get(group);
    if (before && !sameTimeBase(before[0], docs[0])) this.resetTimeBoundView(group);
    this.docs.delete(group);
    this.docs.set(group, Object.freeze(docs));
    this.groupLists.clear();
    for (const candidate of this.docs.keys()) {
      if (this.docs.size <= MAX_LIGHT_CURVE_GROUPS) break;
      if (candidate !== group && !this.holders.has(candidate)) this.drop(candidate);
    }
    this.notify();
  }

  get(group: string): readonly LightCurveDoc[] {
    return this.docs.get(group) ?? EMPTY_LIGHT_CURVES;
  }

  remove(group: string): void {
    if (!this.docs.has(group)) return;
    this.drop(group);
    this.notify();
  }

  groups(prefix = ""): readonly string[] {
    let list = this.groupLists.get(prefix);
    if (!list) {
      list = Object.freeze([...this.docs.keys()].filter((group) => group.startsWith(prefix)));
      this.groupLists.set(prefix, list);
    }
    return list;
  }

  retain(group: string): () => void {
    this.holders.set(group, (this.holders.get(group) ?? 0) + 1);
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const left = (this.holders.get(group) ?? 1) - 1;
      if (left > 0) this.holders.set(group, left);
      else this.holders.delete(group);
    };
  }

  getView(group: string): LightCurveViewState {
    return this.views.get(group) ?? DEFAULT_LIGHT_CURVE_VIEW;
  }

  patchView(group: string, patch: Partial<LightCurveViewState>): void {
    if (!this.docs.has(group)) return;
    this.views.set(group, Object.freeze({ ...this.getView(group), ...patch }));
    this.notify();
  }

  getActiveTable(): string | null {
    return this.activeTable;
  }

  setActiveTable(group: string | null): void {
    if (this.activeTable === group) return;
    this.activeTable = group;
    this.notify();
  }
}

export const lightCurveStore = new LightCurveStore();

function useRetainedGroup(group: string | null): void {
  useEffect(() => {
    if (group === null) return;
    return lightCurveStore.retain(group);
  }, [group]);
}

export function useLightCurves(group: string | null): readonly LightCurveDoc[] {
  const getSnapshot = useCallback(() => (group === null ? EMPTY_LIGHT_CURVES : lightCurveStore.get(group)), [group]);
  useRetainedGroup(group);
  return useSyncExternalStore(lightCurveStore.subscribe, getSnapshot, getSnapshot);
}

export function useLightCurveView(group: string | null): LightCurveViewState {
  const getSnapshot = useCallback(() => (group === null ? DEFAULT_LIGHT_CURVE_VIEW : lightCurveStore.getView(group)), [group]);
  useRetainedGroup(group);
  return useSyncExternalStore(lightCurveStore.subscribe, getSnapshot, getSnapshot);
}

export function useLightCurveGroups(prefix: string): readonly string[] {
  const getSnapshot = useCallback(() => lightCurveStore.groups(prefix), [prefix]);
  return useSyncExternalStore(lightCurveStore.subscribe, getSnapshot, getSnapshot);
}

function activeTableSnapshot(): string | null {
  return lightCurveStore.getActiveTable();
}

export function useActiveLightCurveTable(): string | null {
  return useSyncExternalStore(lightCurveStore.subscribe, activeTableSnapshot, activeTableSnapshot);
}
