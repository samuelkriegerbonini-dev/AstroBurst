import { useCallback, useEffect, useSyncExternalStore } from "react";
import { readSpectrumDocument } from "../services/spectrum1d";
import type { Spectrum1d } from "../shared/types/spectrum1d";
import { OPEN_SPECTRUM_CSV_LABEL, type OpenSpectrumCsv } from "../utils/spectrumDocument";

export interface Spectrum1dDoc {
  loading: boolean;
  spectrum: Spectrum1d | null;
  error: string | null;
}

export const EMPTY_SPECTRUM1D_DOC: Spectrum1dDoc = Object.freeze({ loading: false, spectrum: null, error: null }) as Spectrum1dDoc;

export const MAX_SPECTRUM1D_DOCS = 8;

type Listener = () => void;

type Spectrum1dReader = (path: string, hdu: number | null) => Promise<Spectrum1d>;

function docKey(path: string, hdu: number | null): string {
  return `${hdu ?? ""}|${path}`;
}

function errorText(reason: unknown): string {
  if (typeof reason === "object" && reason !== null && "message" in reason) return String(reason.message);
  return String(reason);
}

export class Spectrum1dStore {
  private docs = new Map<string, Spectrum1dDoc>();
  private holders = new Map<string, number>();
  private listeners = new Set<Listener>();

  constructor(private readonly read: Spectrum1dReader = readSpectrumDocument) {}

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  getDoc(path: string, hdu: number | null): Spectrum1dDoc {
    return this.docs.get(docKey(path, hdu)) ?? EMPTY_SPECTRUM1D_DOC;
  }

  retain(path: string, hdu: number | null): () => void {
    const key = docKey(path, hdu);
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

  private evictable(key: string, doc: Spectrum1dDoc): boolean {
    return !doc.loading && !this.holders.has(key);
  }

  private write(key: string, next: Spectrum1dDoc): void {
    this.docs.delete(key);
    this.docs.set(key, next);
    for (const [candidate, doc] of this.docs) {
      if (this.docs.size <= MAX_SPECTRUM1D_DOCS) break;
      if (candidate !== key && this.evictable(candidate, doc)) this.docs.delete(candidate);
    }
    this.listeners.forEach((l) => l());
  }

  async load(path: string, hdu: number | null): Promise<void> {
    const key = docKey(path, hdu);
    const current = this.docs.get(key);
    if (current && (current.loading || current.spectrum !== null)) return;
    this.write(key, { loading: true, spectrum: null, error: null });
    try {
      const spectrum = await this.read(path, hdu);
      this.write(key, { loading: false, spectrum, error: null });
    } catch (e) {
      this.write(key, { loading: false, spectrum: null, error: errorText(e) });
    }
  }

  async reread(path: string, hdu: number | null): Promise<void> {
    const key = docKey(path, hdu);
    const current = this.docs.get(key);
    if (!current || current.loading) return;
    this.docs.delete(key);
    if (this.holders.has(key)) await this.load(path, hdu);
  }
}

export const spectrum1dStore = new Spectrum1dStore();

export function useSpectrum1dDoc(path: string | null, hdu: number | null): Spectrum1dDoc {
  const getSnapshot = useCallback(
    () => (path === null ? EMPTY_SPECTRUM1D_DOC : spectrum1dStore.getDoc(path, hdu)),
    [path, hdu],
  );
  useEffect(() => {
    if (path === null) return;
    const release = spectrum1dStore.retain(path, hdu);
    void spectrum1dStore.load(path, hdu);
    return release;
  }, [path, hdu]);
  return useSyncExternalStore(spectrum1dStore.subscribe, getSnapshot, getSnapshot);
}

export const SPECTRUM_CSV_FILTER = { name: "Spectrum CSV", extensions: ["csv"] };

export type SpectrumCsvPicker = () => Promise<string | null>;

export class SpectrumCsvStore {
  private current: OpenSpectrumCsv | null = null;
  private listeners = new Set<Listener>();

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  get = (): OpenSpectrumCsv | null => this.current;

  open(path: string, over: string | null): void {
    this.current = { path, over };
    this.listeners.forEach((l) => l());
  }

  close(): void {
    if (this.current === null) return;
    this.current = null;
    this.listeners.forEach((l) => l());
  }
}

export const spectrumCsvStore = new SpectrumCsvStore();

export async function pickSpectrumCsvFile(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({ multiple: false, directory: false, filters: [SPECTRUM_CSV_FILTER], title: OPEN_SPECTRUM_CSV_LABEL });
  return typeof picked === "string" ? picked : null;
}

export async function openSpectrumCsv(
  over: string | null,
  pick: SpectrumCsvPicker = pickSpectrumCsvFile,
  store: SpectrumCsvStore = spectrumCsvStore,
  documents: Pick<Spectrum1dStore, "reread"> = spectrum1dStore,
): Promise<boolean> {
  let path: string | null;
  try {
    path = await pick();
  } catch (e) {
    console.error("[AstroBurst] Spectrum CSV dialog error:", e);
    return false;
  }
  if (path === null) return false;
  void documents.reread(path, null);
  store.open(path, over);
  return true;
}

export function useOpenSpectrumCsv(): OpenSpectrumCsv | null {
  return useSyncExternalStore(spectrumCsvStore.subscribe, spectrumCsvStore.get, spectrumCsvStore.get);
}
