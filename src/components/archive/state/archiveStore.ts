import { useSyncExternalStore } from "react";
import { archiveReducer, createInitialState, type ArchiveAction, type ArchiveState } from "./archiveReducer";

export interface ArchiveStore {
  getState(): ArchiveState;
  dispatch(action: ArchiveAction): void;
  subscribe(listener: () => void): () => void;
}

export function createArchiveStore(initial: ArchiveState = createInitialState()): ArchiveStore {
  let state = initial;
  const listeners = new Set<() => void>();
  return {
    getState: () => state,
    dispatch(action) {
      const next = archiveReducer(state, action);
      if (next === state) return;
      state = next;
      [...listeners].forEach((l) => l());
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

export const archiveStore = createArchiveStore();

export function useArchiveState(store: ArchiveStore = archiveStore): ArchiveState {
  return useSyncExternalStore(store.subscribe, store.getState, store.getState);
}
