import { useSyncExternalStore } from "react";
import type { CubeSpectrum, RegionSpectrum } from "../shared/types/cube";

export interface SpectrumState {
  spectrum: number[];
  wavelengths: number[] | null;
  coord: { x: number; y: number } | null;
  loading: boolean;
  elapsed: number;
  error: string | null;
  region: RegionSpectrum | null;
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
  regionLoading: false,
  regionError: null,
};

type Listener = () => void;

class SpectrumStore {
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
    this.emit({ ...this.value, coord, loading: true, error: null, region: null, regionError: null });
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
    });
  }

  fail(error: string) {
    this.emit({ ...this.value, loading: false, error });
  }

  beginRegion() {
    this.emit({ ...this.value, regionLoading: true, regionError: null });
  }

  commitRegion(result: RegionSpectrum) {
    this.emit({
      ...this.value,
      region: result,
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
    this.emit({ ...this.value, region: null, regionError: null, regionLoading: false });
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

export function commitRegionSpectrum(result: RegionSpectrum) {
  store.commitRegion(result);
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
