import { useSyncExternalStore } from "react";
import type { CubeSpectrum } from "../shared/types/cube";

export interface SpectrumState {
  spectrum: number[];
  wavelengths: number[] | null;
  coord: { x: number; y: number } | null;
  loading: boolean;
  elapsed: number;
  error: string | null;
}

const EMPTY: SpectrumState = {
  spectrum: [],
  wavelengths: null,
  coord: null,
  loading: false,
  elapsed: 0,
  error: null,
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
    this.emit({ ...this.value, coord, loading: true, error: null });
  }

  commit(result: CubeSpectrum, elapsed: number) {
    this.emit({
      spectrum: result.values ?? [],
      wavelengths: result.wavelengths?.length ? result.wavelengths : null,
      coord: { x: result.x, y: result.y },
      loading: false,
      elapsed,
      error: null,
    });
  }

  fail(error: string) {
    this.emit({ ...this.value, loading: false, error });
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

export function resetSpectrum() {
  store.reset();
}

export function useSpectrum(): SpectrumState {
  return useSyncExternalStore(store.subscribe, store.getSnapshot);
}
