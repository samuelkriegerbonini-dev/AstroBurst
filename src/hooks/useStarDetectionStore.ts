import { useEffect, useSyncExternalStore } from "react";
import type { StarDetectionResult } from "../shared/types";

export interface StarDetectionState {
  scope: string | null;
  result: StarDetectionResult | null;
  loading: boolean;
  error: string | null;
}

export const EMPTY_STAR_DETECTION: StarDetectionState = Object.freeze({ scope: null, result: null, loading: false, error: null });

type Listener = () => void;

export class StarDetectionStore {
  private value: StarDetectionState = EMPTY_STAR_DETECTION;
  private listeners = new Set<Listener>();
  private seq = 0;
  private consumers = 0;

  getSnapshot = (): StarDetectionState => this.value;

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private emit(next: StarDetectionState) {
    this.value = next;
    this.listeners.forEach((l) => l());
  }

  retain(): () => void {
    this.consumers++;
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.consumers--;
      if (this.consumers === 0) this.reset();
    };
  }

  syncScope(scope: string): void {
    if (scope === this.value.scope) return;
    this.seq++;
    this.emit({ scope, result: null, loading: false, error: null });
  }

  begin(scope: string): number {
    const result = scope === this.value.scope ? this.value.result : null;
    this.seq++;
    this.emit({ scope, result, loading: true, error: null });
    return this.seq;
  }

  commit(seq: number, result: StarDetectionResult | null): void {
    if (seq !== this.seq) return;
    this.emit({ ...this.value, result, loading: false });
  }

  fail(seq: number, message: string): void {
    if (seq !== this.seq) return;
    this.emit({ ...this.value, error: message, loading: false });
  }

  reset(): void {
    this.seq++;
    if (this.value === EMPTY_STAR_DETECTION) return;
    this.emit(EMPTY_STAR_DETECTION);
  }
}

export const starDetectionStore = new StarDetectionStore();

export function scopedStarDetection(state: StarDetectionState, scope: string): StarDetectionState {
  return state.scope === scope ? state : EMPTY_STAR_DETECTION;
}

export function useStarDetection(scope: string): StarDetectionState {
  useEffect(() => starDetectionStore.retain(), []);
  useEffect(() => starDetectionStore.syncScope(scope), [scope]);
  const snapshot = useSyncExternalStore(starDetectionStore.subscribe, starDetectionStore.getSnapshot, starDetectionStore.getSnapshot);
  return scopedStarDetection(snapshot, scope);
}
