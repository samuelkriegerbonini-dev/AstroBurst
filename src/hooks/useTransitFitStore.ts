import { useCallback, useSyncExternalStore } from "react";
import type { TransitFitRequest, TransitFitResult } from "../shared/types/transit";
import { DEFAULT_TRANSIT_FIT_SETUP, type TransitFitSetup } from "../utils/transitFitSetup";

export interface TransitFitRecord {
  setup: TransitFitSetup;
  request: TransitFitRequest | null;
  result: TransitFitResult | null;
  signature: string | null;
  fittedUtc: string | null;
}

export const EMPTY_TRANSIT_FIT: TransitFitRecord = Object.freeze({
  setup: DEFAULT_TRANSIT_FIT_SETUP,
  request: null,
  result: null,
  signature: null,
  fittedUtc: null,
});

export const MAX_TRANSIT_FITS = 64;

export function transitFitKey(group: string, docId: string): string {
  return `${group}#${docId}`;
}

type Listener = () => void;

export class TransitFitStore {
  private records = new Map<string, TransitFitRecord>();
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

  private write(key: string, record: TransitFitRecord): void {
    this.records.delete(key);
    this.records.set(key, Object.freeze(record));
    for (const oldest of this.records.keys()) {
      if (this.records.size <= MAX_TRANSIT_FITS) break;
      this.records.delete(oldest);
    }
    this.notify();
  }

  get(group: string, docId: string): TransitFitRecord {
    return this.records.get(transitFitKey(group, docId)) ?? EMPTY_TRANSIT_FIT;
  }

  setSetup(group: string, docId: string, setup: TransitFitSetup): void {
    this.write(transitFitKey(group, docId), { ...this.get(group, docId), setup });
  }

  setResult(
    group: string,
    docId: string,
    fit: { request: TransitFitRequest; result: TransitFitResult; signature: string; fittedUtc: string },
  ): void {
    const { request, result, signature, fittedUtc } = fit;
    this.write(transitFitKey(group, docId), { setup: this.get(group, docId).setup, request, result, signature, fittedUtc });
  }

  clearResult(group: string, docId: string): void {
    const key = transitFitKey(group, docId);
    const stored = this.records.get(key);
    if (!stored) return;
    this.write(key, { ...EMPTY_TRANSIT_FIT, setup: stored.setup });
  }

  removeGroup(group: string): void {
    const prefix = transitFitKey(group, "");
    const keys = [...this.records.keys()].filter((key) => key.startsWith(prefix));
    if (keys.length === 0) return;
    keys.forEach((key) => this.records.delete(key));
    this.notify();
  }

  size(): number {
    return this.records.size;
  }
}

export const transitFitStore = new TransitFitStore();

export function useTransitFit(group: string | null, docId: string | null): TransitFitRecord {
  const getSnapshot = useCallback(
    () => (group === null || docId === null ? EMPTY_TRANSIT_FIT : transitFitStore.get(group, docId)),
    [group, docId],
  );
  return useSyncExternalStore(transitFitStore.subscribe, getSnapshot, getSnapshot);
}
