import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";
import type { UserZeroPoint } from "../services/analysis";
import { photometryStatus } from "../services/photometryStatus";
import type { PhotometryStatus } from "../shared/types/photometryStatus";
import { errorText } from "./useSpectrum1dStore";

export interface PhotometryStatusDoc {
  loading: boolean;
  status: PhotometryStatus | null;
  error: string | null;
}

export const EMPTY_PHOTOMETRY_STATUS: PhotometryStatusDoc = Object.freeze({ loading: false, status: null, error: null }) as PhotometryStatusDoc;

export const MAX_PHOTOMETRY_STATUS_DOCS = 8;

export const STATUS_DEBOUNCE_MS = 250;

export interface PhotometryStatusRequest {
  path: string;
  excludeDq: boolean;
  measureKey: string | null;
  zeroPoint?: UserZeroPoint | null;
}

export function photometryStatusKey(request: PhotometryStatusRequest): string {
  const key = `${request.excludeDq ? 1 : 0}|${request.measureKey ?? ""}|${request.path}`;
  const zeroPoint = request.zeroPoint;
  return zeroPoint == null ? key : `${key}|${String(zeroPoint.zp)},${zeroPoint.system},${zeroPoint.per_second}`;
}

export type PhotometryStatusReader = (request: PhotometryStatusRequest) => Promise<PhotometryStatus>;

type Listener = () => void;

interface StoredDoc {
  request: PhotometryStatusRequest;
  doc: PhotometryStatusDoc;
}

export class PhotometryStatusStore {
  private docs = new Map<string, StoredDoc>();
  private holders = new Map<string, number>();
  private listeners = new Set<Listener>();

  constructor(
    private readonly read: PhotometryStatusReader = (r) => photometryStatus(r.path, { excludeDq: r.excludeDq, zeroPoint: r.zeroPoint ?? null }),
  ) {}

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  getDoc(request: PhotometryStatusRequest): PhotometryStatusDoc {
    return this.docs.get(photometryStatusKey(request))?.doc ?? EMPTY_PHOTOMETRY_STATUS;
  }

  retain(request: PhotometryStatusRequest): () => void {
    const key = photometryStatusKey(request);
    this.holders.set(key, (this.holders.get(key) ?? 0) + 1);
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const left = (this.holders.get(key) ?? 1) - 1;
      if (left > 0) this.holders.set(key, left);
      else this.holders.delete(key);
    };
  }

  private write(key: string, request: PhotometryStatusRequest, doc: PhotometryStatusDoc): void {
    this.docs.delete(key);
    this.docs.set(key, { request, doc });
    for (const [candidate, stored] of this.docs) {
      if (this.docs.size <= MAX_PHOTOMETRY_STATUS_DOCS) break;
      if (candidate !== key && !stored.doc.loading && !this.holders.has(candidate)) this.docs.delete(candidate);
    }
    this.notify();
  }

  private notify(): void {
    this.listeners.forEach((l) => l());
  }

  async load(request: PhotometryStatusRequest): Promise<void> {
    const key = photometryStatusKey(request);
    const current = this.docs.get(key)?.doc;
    if (current && (current.loading || current.status !== null)) return;
    const loading: PhotometryStatusDoc = { loading: true, status: null, error: null };
    this.write(key, request, loading);
    let next: PhotometryStatusDoc;
    try {
      next = { loading: false, status: await this.read(request), error: null };
    } catch (e) {
      next = { loading: false, status: null, error: errorText(e) };
    }
    if (this.docs.get(key)?.doc === loading) this.write(key, request, next);
  }

  forget(path: string): void {
    const dropped = [...this.docs].filter(([, stored]) => stored.request.path === path);
    if (dropped.length === 0) return;
    for (const [key] of dropped) this.docs.delete(key);
    this.notify();
    for (const [key, stored] of dropped) {
      if (this.holders.has(key)) void this.load(stored.request);
    }
  }
}

export const photometryStatusStore = new PhotometryStatusStore();

export function usePhotometryStatus(
  path: string | null,
  excludeDq: boolean,
  measureKey: string | null = null,
  zeroPoint: UserZeroPoint | null = null,
): PhotometryStatusDoc {
  const request = useMemo(
    () => (path === null ? null : { path, excludeDq, measureKey, zeroPoint }),
    [path, excludeDq, measureKey, zeroPoint],
  );
  const getSnapshot = useCallback(() => (request === null ? EMPTY_PHOTOMETRY_STATUS : photometryStatusStore.getDoc(request)), [request]);
  useEffect(() => {
    if (request === null) return;
    const release = photometryStatusStore.retain(request);
    const timer = setTimeout(() => void photometryStatusStore.load(request), STATUS_DEBOUNCE_MS);
    return () => {
      clearTimeout(timer);
      release();
    };
  }, [request]);
  return useSyncExternalStore(photometryStatusStore.subscribe, getSnapshot, getSnapshot);
}
