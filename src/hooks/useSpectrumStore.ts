import { useSyncExternalStore } from "react";
import type { ContinuumWindows, CubeSpectrum, RegionSpectrum } from "../shared/types/cube";
import type { SpectrumSource } from "../shared/types/spectral";

export interface SpectrumState {
  spectrum: number[];
  err: number[] | null;
  wavelengths: number[] | null;
  coord: { x: number; y: number } | null;
  loading: boolean;
  elapsed: number;
  error: string | null;
  region: RegionSpectrum | null;
  regionSource: SpectrumSource | null;
  regionContinuum: ContinuumWindows | null;
  regionLoading: boolean;
  regionError: string | null;
}

const EMPTY: SpectrumState = {
  spectrum: [],
  err: null,
  wavelengths: null,
  coord: null,
  loading: false,
  elapsed: 0,
  error: null,
  region: null,
  regionSource: null,
  regionContinuum: null,
  regionLoading: false,
  regionError: null,
};

type Listener = () => void;

export class SpectrumStore {
  private value: SpectrumState = EMPTY;
  private listeners = new Set<Listener>();
  private request = 0;

  getSnapshot = (): SpectrumState => this.value;

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private emit(next: SpectrumState) {
    this.value = next;
    this.listeners.forEach((l) => l());
  }

  begin(coord: { x: number; y: number }) {
    const request = ++this.request;
    this.emit({ ...EMPTY, coord, loading: true });
    return request;
  }

  commit(result: CubeSpectrum, elapsed: number, request: number = this.request) {
    if (request !== this.request) return;
    this.emit({
      ...EMPTY,
      spectrum: result.values ?? [],
      err: result.err ?? null,
      wavelengths: result.wavelengths?.length ? result.wavelengths : null,
      coord: { x: result.x, y: result.y },
      loading: false,
      elapsed,
      error: null,
      region: null,
      regionSource: null,
      regionContinuum: null,
    });
  }

  fail(error: string, request: number = this.request) {
    if (request !== this.request) return;
    this.emit({ ...this.value, loading: false, error });
  }

  beginRegion() {
    const request = ++this.request;
    this.emit({ ...EMPTY, regionLoading: true });
    return request;
  }

  commitRegion(result: RegionSpectrum, source: SpectrumSource, continuum: ContinuumWindows | null = null, request: number = this.request) {
    if (request !== this.request) return;
    this.emit({
      ...EMPTY,
      region: result,
      regionSource: source,
      regionContinuum: continuum,
      regionLoading: false,
      regionError: null,
      elapsed: result.elapsed_ms,
    });
  }

  failRegion(error: string, request: number = this.request) {
    if (request !== this.request) return;
    this.emit({ ...this.value, regionLoading: false, regionError: error });
  }

  clearRegion() {
    if (!this.value.region && !this.value.regionError && !this.value.regionLoading) return;
    this.request++;
    this.emit({ ...this.value, region: null, regionSource: null, regionContinuum: null, regionError: null, regionLoading: false });
  }

  reset() {
    this.request++;
    if (this.value === EMPTY) return;
    this.emit(EMPTY);
  }
}

const store = new SpectrumStore();

export function beginSpectrum(coord: { x: number; y: number }) {
  return store.begin(coord);
}

export function commitSpectrum(result: CubeSpectrum, elapsed: number, request: number) {
  store.commit(result, elapsed, request);
}

export function failSpectrum(error: string, request: number) {
  store.fail(error, request);
}

export function beginRegionSpectrum() {
  return store.beginRegion();
}

export function commitRegionSpectrum(result: RegionSpectrum, source: SpectrumSource, continuum: ContinuumWindows | null, request: number) {
  store.commitRegion(result, source, continuum, request);
}

export function failRegionSpectrum(error: string, request: number) {
  store.failRegion(error, request);
}

export function clearRegionSpectrum() {
  store.clearRegion();
}

export function resetSpectrum() {
  store.reset();
}

export function useSpectrum(): SpectrumState {
  return useSyncExternalStore(store.subscribe, store.getSnapshot);
}
