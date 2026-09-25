import { useCallback, useSyncExternalStore } from "react";
import type { ComparisonEntry, NormaliseMode } from "../utils/spectrumCompare";

export interface ComparisonDoc {
  enabled: boolean;
  entries: ComparisonEntry[];
  loading: boolean;
  regionVersion: number | null;
  pixelKey: string | null;
  pendingRegionVersion: number | null;
  pendingPixelKey: string | null;
  normalise: NormaliseMode;
  offsetStep: number | null;
  includePixel: boolean;
  hidden: string[];
  seq: number;
}

export const EMPTY_COMPARISON_DOC: ComparisonDoc = Object.freeze({
  enabled: false,
  entries: [],
  loading: false,
  regionVersion: null,
  pixelKey: null,
  pendingRegionVersion: null,
  pendingPixelKey: null,
  normalise: "none",
  offsetStep: null,
  includePixel: true,
  hidden: [],
  seq: 0,
}) as ComparisonDoc;

export const MAX_COMPARISON_DOCS = 8;

type Listener = () => void;

export type ComparisonPatch = Partial<
  Omit<ComparisonDoc, "seq" | "loading" | "entries" | "regionVersion" | "pixelKey" | "pendingRegionVersion" | "pendingPixelKey">
>;

export class SpectrumComparisonStore {
  private docs = new Map<string, ComparisonDoc>();
  private listeners = new Set<Listener>();

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  getDoc(filePath: string): ComparisonDoc {
    return this.docs.get(filePath) ?? EMPTY_COMPARISON_DOC;
  }

  private write(filePath: string, next: ComparisonDoc): void {
    this.docs.delete(filePath);
    this.docs.set(filePath, next);
    while (this.docs.size > MAX_COMPARISON_DOCS) {
      const oldest = this.docs.keys().next().value;
      if (oldest === undefined) break;
      this.docs.delete(oldest);
    }
    this.notify();
  }

  private notify(): void {
    this.listeners.forEach((l) => l());
  }

  patch(filePath: string, patch: ComparisonPatch): void {
    this.write(filePath, { ...this.getDoc(filePath), ...patch });
  }

  begin(filePath: string, regionVersion: number, pixelKey: string | null): number {
    const doc = this.getDoc(filePath);
    const seq = doc.seq + 1;
    this.write(filePath, { ...doc, seq, loading: true, pendingRegionVersion: regionVersion, pendingPixelKey: pixelKey });
    return seq;
  }

  commit(filePath: string, seq: number, entries: ComparisonEntry[]): boolean {
    const doc = this.getDoc(filePath);
    if (seq !== doc.seq) return false;
    this.write(filePath, {
      ...doc,
      entries,
      loading: false,
      regionVersion: doc.pendingRegionVersion,
      pixelKey: doc.pendingPixelKey,
      pendingRegionVersion: null,
      pendingPixelKey: null,
    });
    return true;
  }

  invalidate(filePath: string): void {
    const doc = this.docs.get(filePath);
    if (!doc) return;
    this.write(filePath, { ...doc, seq: doc.seq + 1, loading: false, pendingRegionVersion: null, pendingPixelKey: null });
  }

  forget(filePath: string): void {
    if (this.docs.delete(filePath)) this.notify();
  }
}

export const spectrumComparisonStore = new SpectrumComparisonStore();

export function useSpectrumComparison(filePath: string | null): ComparisonDoc {
  const getSnapshot = useCallback(
    () => (filePath ? spectrumComparisonStore.getDoc(filePath) : EMPTY_COMPARISON_DOC),
    [filePath],
  );
  return useSyncExternalStore(spectrumComparisonStore.subscribe, getSnapshot, getSnapshot);
}
