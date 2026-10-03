import { useCallback, useSyncExternalStore } from "react";
import type { X1dSpectrum } from "../shared/types/spectral";
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
  tablePath: string | null;
  tableHdu: number | null;
  table: X1dSpectrum | null;
  tableError: string | null;
  tableLoading: boolean;
}

export interface TableRequest {
  path: string;
  hdu: number | null;
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
  tablePath: null,
  tableHdu: null,
  table: null,
  tableError: null,
  tableLoading: false,
}) as ComparisonDoc;

export const MAX_COMPARISON_DOCS = 8;

type Listener = () => void;

export type ComparisonPatch = Partial<
  Omit<
    ComparisonDoc,
    | "seq"
    | "loading"
    | "entries"
    | "regionVersion"
    | "pixelKey"
    | "pendingRegionVersion"
    | "pendingPixelKey"
    | "table"
    | "tableError"
    | "tableLoading"
  >
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

  requestTable(filePath: string, tablePath: string, hdu: number | null = null): void {
    this.write(filePath, {
      ...this.getDoc(filePath),
      enabled: true,
      tablePath,
      tableHdu: hdu,
      table: null,
      tableError: null,
      tableLoading: false,
    });
  }

  beginTable(filePath: string): TableRequest | null {
    const doc = this.getDoc(filePath);
    if (doc.tablePath === null) return null;
    this.write(filePath, { ...doc, table: null, tableError: null, tableLoading: true });
    return { path: doc.tablePath, hdu: doc.tableHdu };
  }

  private awaiting(filePath: string, request: TableRequest): ComparisonDoc | null {
    const doc = this.docs.get(filePath);
    if (!doc || !doc.tableLoading || doc.tablePath !== request.path || doc.tableHdu !== request.hdu) return null;
    return doc;
  }

  commitTable(filePath: string, request: TableRequest, x1d: X1dSpectrum): boolean {
    const doc = this.awaiting(filePath, request);
    if (!doc) return false;
    this.write(filePath, { ...doc, table: x1d, tableError: null, tableLoading: false });
    return true;
  }

  failTable(filePath: string, request: TableRequest, error: string): boolean {
    const doc = this.awaiting(filePath, request);
    if (!doc) return false;
    this.write(filePath, { ...doc, table: null, tableError: error, tableLoading: false });
    return true;
  }

  clearTable(filePath: string): void {
    this.write(filePath, {
      ...this.getDoc(filePath),
      tablePath: null,
      tableHdu: null,
      table: null,
      tableError: null,
      tableLoading: false,
    });
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
