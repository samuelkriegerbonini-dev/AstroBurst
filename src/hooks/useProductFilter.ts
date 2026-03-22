import { useSyncExternalStore, useCallback } from "react";

type Listener = () => void;
export type FilterMode = "or" | "and";

const JWST_PRODUCT_RE = /[_-](i2d|segm|cal|rate|rateints|x1d|s2d|s3d|uncal|crf|bsub|srctype|outlier|tweakreg|skymatch|whtlt)(?:\.fits|\.fit|\.fts|\.asdf)$/i;

export function extractProductType(filename: string): string | null {
  const match = filename.match(JWST_PRODUCT_RE);
  return match ? match[1].toLowerCase() : null;
}

export function detectProductTypes(filenames: string[]): string[] {
  const types = new Set<string>();
  for (const name of filenames) {
    const pt = extractProductType(name);
    if (pt) types.add(pt);
  }
  return Array.from(types).sort();
}

function singleMatch(filename: string, filter: string): boolean {
  const pt = extractProductType(filename);
  if (pt === filter) return true;
  return filename.toLowerCase().includes(filter.toLowerCase());
}

export function matchesActiveFilters(filename: string, filters: string[], mode: FilterMode): boolean {
  if (filters.length === 0) return true;
  return mode === "or"
    ? filters.some((f) => singleMatch(filename, f))
    : filters.every((f) => singleMatch(filename, f));
}

export interface FilterState {
  activeFilters: string[];
  customChips: string[];
  mode: FilterMode;
}

class ProductFilterStore {
  private state: FilterState = {
    activeFilters: [],
    customChips: [],
    mode: "or",
  };

  private listeners = new Set<Listener>();

  subscribe = (listener: Listener) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): FilterState => this.state;
  getActiveFilters = (): string[] => this.state.activeFilters;
  getMode = (): FilterMode => this.state.mode;

  private notify() {
    this.state = { ...this.state };
    this.listeners.forEach((l) => l());
  }

  toggleFilter(filter: string) {
    const idx = this.state.activeFilters.indexOf(filter);
    if (idx >= 0) {
      this.state.activeFilters = this.state.activeFilters.filter((f) => f !== filter);
    } else {
      this.state.activeFilters = [...this.state.activeFilters, filter];
    }
    this.notify();
  }

  setMode(mode: FilterMode) {
    if (this.state.mode === mode) return;
    this.state.mode = mode;
    this.notify();
  }

  toggleMode() {
    this.state.mode = this.state.mode === "or" ? "and" : "or";
    this.notify();
  }

  clearAll() {
    if (this.state.activeFilters.length === 0) return;
    this.state.activeFilters = [];
    this.notify();
  }

  addCustomChip(text: string) {
    const normalized = text.trim().toLowerCase();
    if (!normalized) return;
    if (!this.state.customChips.includes(normalized)) {
      this.state.customChips = [...this.state.customChips, normalized];
    }
    if (!this.state.activeFilters.includes(normalized)) {
      this.state.activeFilters = [...this.state.activeFilters, normalized];
    }
    this.notify();
  }

  removeCustomChip(text: string) {
    this.state.customChips = this.state.customChips.filter((c) => c !== text);
    this.state.activeFilters = this.state.activeFilters.filter((f) => f !== text);
    this.notify();
  }

  reset() {
    this.state = { activeFilters: [], customChips: [], mode: "or" };
    this.notify();
  }
}

export const productFilterStore = new ProductFilterStore();

export function useActiveFilters(): string[] {
  return useSyncExternalStore(
    productFilterStore.subscribe,
    () => productFilterStore.getActiveFilters(),
  );
}

export function useFilterMode(): FilterMode {
  return useSyncExternalStore(
    productFilterStore.subscribe,
    () => productFilterStore.getMode(),
  );
}

export function useProductFilterState(): FilterState {
  return useSyncExternalStore(
    productFilterStore.subscribe,
    productFilterStore.getSnapshot,
  );
}

export function useProductFilterActions() {
  const toggleFilter = useCallback((filter: string) => {
    productFilterStore.toggleFilter(filter);
  }, []);

  const toggleMode = useCallback(() => {
    productFilterStore.toggleMode();
  }, []);

  const clearAll = useCallback(() => {
    productFilterStore.clearAll();
  }, []);

  const addCustomChip = useCallback((text: string) => {
    productFilterStore.addCustomChip(text);
  }, []);

  const removeCustomChip = useCallback((text: string) => {
    productFilterStore.removeCustomChip(text);
  }, []);

  const reset = useCallback(() => {
    productFilterStore.reset();
  }, []);

  return { toggleFilter, toggleMode, clearAll, addCustomChip, removeCustomChip, reset };
}
