import { useSyncExternalStore } from "react";
import type { ContinuumWindows, CubeSpectrum, RegionSpectrum } from "../shared/types/cube";
import type { SpectrumSource } from "../shared/types/spectral";

export interface SpectrumState {
  spectrum: number[];
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
    this.emit({ ...this.value, coord, loading: true, error: null, region: null, regionSource: null, regionContinuum: null, regionError: null });
  }

  commit(result: CubeSpectrum, elapsed: number) {
    this.emit({
      ...this.value,
      spectrum: result.values ?? [],
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

  fail(error: string) {
    this.emit({ ...this.value, loading: false, error });
  }

  beginRegion() {
    this.emit({ ...this.value, regionLoading: true, regionError: null });
  }

  commitRegion(result: RegionSpectrum, source: SpectrumSource, continuum: ContinuumWindows | null = null) {
    this.emit({
      ...this.value,
      region: result,
      regionSource: source,
      regionContinuum: continuum,
      regionLoading: false,
      regionError: null,
      elapsed: result.elapsed_ms,
    });
  }

  failRegion(error: string) {
    this.emit({ ...this.value, regionLoading: false, regionError: error });
  }

  clearRegion() {
    if (!this.value.region && !this.value.regionError && !this.value.regionLoading) return;
    this.emit({ ...this.value, region: null, regionSource: null, regionContinuum: null, regionError: null, regionLoading: false });
  }

  reset() {
    if (this.value === EMPTY) return;
    this.emit(EMPTY);
  }
}

const store = new SpectrumStore();

export function beginSpectrum(coord: { x: number; y: number }) {
  store.begin(coord);
}

export function commitSpectrum(result: CubeSpectrum, elapsed: number) {
  store.commit(result, elapsed);
}

export function failSpectrum(error: string) {
  store.fail(error);
}

export function beginRegionSpectrum() {
  store.beginRegion();
}

export function commitRegionSpectrum(result: RegionSpectrum, source: SpectrumSource, continuum: ContinuumWindows | null = null) {
  store.commitRegion(result, source, continuum);
}

export function failRegionSpectrum(error: string) {
  store.failRegion(error);
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
