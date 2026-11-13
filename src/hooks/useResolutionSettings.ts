import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { getWcsInfo } from "../services/astrometry";
import { getHeader } from "../services/header";
import { imagePsfFwhm } from "../services/resolution";
import type { ImagePsfFwhm, ResolutionSettings, SeeingCard } from "../shared/types/resolution";
import { DEFAULT_RESOLUTION_SETTINGS, sanitizeResolutionSettings, seeingCardsFromHeader } from "../utils/resolutionText";

export const RESOLUTION_STORAGE_KEY = "ab.resolution.v1";
export const MAX_IMAGE_PSF_DOCS = 8;

type SettingsStorage = Pick<Storage, "getItem" | "setItem">;
type Listener = () => void;

function browserStorage(): SettingsStorage | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}

function storedSettings(storage: SettingsStorage | null): ResolutionSettings {
  try {
    return sanitizeResolutionSettings(JSON.parse(storage?.getItem(RESOLUTION_STORAGE_KEY) ?? "null"));
  } catch {
    return { ...DEFAULT_RESOLUTION_SETTINGS };
  }
}

function sameSettings(a: ResolutionSettings, b: ResolutionSettings): boolean {
  return (Object.keys(a) as (keyof ResolutionSettings)[]).every((key) => Object.is(a[key], b[key]));
}

export class ResolutionSettingsStore {
  private value: ResolutionSettings;
  private readonly storage: SettingsStorage | null;
  private readonly listeners = new Set<Listener>();

  constructor(storage: SettingsStorage | null = browserStorage()) {
    this.storage = storage;
    this.value = storedSettings(storage);
  }

  getSnapshot = (): ResolutionSettings => this.value;

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  update(patch: Partial<ResolutionSettings>): void {
    this.apply(sanitizeResolutionSettings({ ...this.value, ...patch }));
  }

  reset(): void {
    this.store(DEFAULT_RESOLUTION_SETTINGS);
    this.apply({ ...DEFAULT_RESOLUTION_SETTINGS });
  }

  private apply(next: ResolutionSettings): void {
    if (sameSettings(next, this.value)) return;
    this.value = next;
    this.store(next);
    this.listeners.forEach((listener) => listener());
  }

  private store(settings: ResolutionSettings): void {
    try {
      this.storage?.setItem(RESOLUTION_STORAGE_KEY, JSON.stringify(settings));
    } catch {
      return;
    }
  }
}

export const resolutionSettingsStore = new ResolutionSettingsStore();

export function useResolutionSettings(): ResolutionSettings {
  return useSyncExternalStore(resolutionSettingsStore.subscribe, resolutionSettingsStore.getSnapshot, resolutionSettingsStore.getSnapshot);
}

export interface ImagePsfState {
  psf: ImagePsfFwhm | null;
  loading: boolean;
  error: string | null;
}

export const EMPTY_IMAGE_PSF: ImagePsfState = Object.freeze({ psf: null, loading: false, error: null });

export class ImagePsfStore {
  private readonly states = new Map<string, ImagePsfState>();
  private readonly runs = new Map<string, number>();
  private readonly listeners = new Set<Listener>();
  private readonly measurePsf: (path: string) => Promise<ImagePsfFwhm>;
  private seq = 0;

  constructor(measure: (path: string) => Promise<ImagePsfFwhm> = imagePsfFwhm) {
    this.measurePsf = measure;
  }

  getSnapshot = (path: string | null): ImagePsfState => (path === null ? EMPTY_IMAGE_PSF : (this.states.get(path) ?? EMPTY_IMAGE_PSF));

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  async measure(path: string): Promise<void> {
    const run = ++this.seq;
    this.runs.set(path, run);
    this.set(path, { psf: this.getSnapshot(path).psf, loading: true, error: null });
    try {
      const psf = await this.measurePsf(path);
      if (this.runs.get(path) === run) this.set(path, { psf, loading: false, error: null });
    } catch (error) {
      if (this.runs.get(path) === run) {
        this.set(path, { psf: null, loading: false, error: error instanceof Error ? error.message : String(error) });
      }
    }
  }

  forget(path: string): void {
    this.runs.delete(path);
    if (this.states.delete(path)) this.emit();
  }

  private set(path: string, state: ImagePsfState): void {
    this.states.set(path, state);
    for (const oldest of this.states.keys()) {
      if (this.states.size <= MAX_IMAGE_PSF_DOCS) break;
      this.states.delete(oldest);
      this.runs.delete(oldest);
    }
    this.emit();
  }

  private emit(): void {
    this.listeners.forEach((listener) => listener());
  }
}

export const imagePsfStore = new ImagePsfStore();

export function useImagePsf(path: string | null): ImagePsfState {
  const snapshot = () => imagePsfStore.getSnapshot(path);
  return useSyncExternalStore(imagePsfStore.subscribe, snapshot, snapshot);
}

function isPositiveScale(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value > 0;
}

function usePathValue<T>(path: string | null, load: (path: string) => Promise<T>, empty: T): T {
  const [loaded, setLoaded] = useState<{ path: string; value: T } | null>(null);
  const seqRef = useRef(0);
  useEffect(() => {
    const seq = ++seqRef.current;
    if (path === null) return;
    load(path).then(
      (value) => {
        if (seqRef.current === seq) setLoaded({ path, value });
      },
      () => {
        if (seqRef.current === seq) setLoaded(null);
      },
    );
  }, [path, load]);
  return loaded !== null && loaded.path === path ? loaded.value : empty;
}

async function wcsPixelScale(path: string): Promise<number | null> {
  const { pixel_scale_arcsec: scale } = await getWcsInfo(path);
  return isPositiveScale(scale) ? scale : null;
}

const NO_SEEING_CARDS: readonly SeeingCard[] = Object.freeze([]);

async function headerSeeing(path: string): Promise<readonly SeeingCard[]> {
  return seeingCardsFromHeader(await getHeader(path));
}

export function useWcsPixelScale(path: string | null): number | null {
  return usePathValue(path, wcsPixelScale, null);
}

export function useHeaderSeeing(path: string | null): readonly SeeingCard[] {
  return usePathValue(path, headerSeeing, NO_SEEING_CARDS);
}
