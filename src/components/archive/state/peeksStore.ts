import { useSyncExternalStore } from "react";
import type { MastError, MastMissionId, MastPeek } from "../../../shared/types/mast";
import { drizzled, peekKey } from "../logic/peekView";

export type PeekStage = "loading" | "ready" | "failed";

export interface PeekEntry {
  key: string;
  mission: MastMissionId;
  datasetId: string;
  filename: string;
  attempt: number;
  stage: PeekStage;
  peek: MastPeek | null;
  error: MastError | null;
  order: number;
}

export interface PeeksState {
  entries: Readonly<Record<string, PeekEntry>>;
  order: number;
}

export type PeeksAction =
  | { type: "started"; key: string; mission: MastMissionId; datasetId: string; filename: string; attempt: number }
  | { type: "loaded"; key: string; attempt: number; peek: MastPeek }
  | { type: "failed"; key: string; attempt: number; error: MastError }
  | { type: "dropped"; key: string; attempt: number };

export function createPeeksState(): PeeksState {
  return { entries: {}, order: 0 };
}

function current(state: PeeksState, key: string, attempt: number): PeekEntry | null {
  const entry = state.entries[key];
  return entry && entry.attempt === attempt ? entry : null;
}

function withEntry(state: PeeksState, entry: PeekEntry): PeeksState {
  return { ...state, entries: { ...state.entries, [entry.key]: entry } };
}

export function peeksReducer(state: PeeksState, action: PeeksAction): PeeksState {
  switch (action.type) {
    case "started": {
      const previous = state.entries[action.key];
      return withEntry(state, {
        key: action.key,
        mission: action.mission,
        datasetId: action.datasetId,
        filename: action.filename,
        attempt: action.attempt,
        stage: "loading",
        peek: previous?.peek ?? null,
        error: null,
        order: previous?.order ?? 0,
      });
    }
    case "loaded": {
      const entry = current(state, action.key, action.attempt);
      if (!entry) return state;
      const order = state.order + 1;
      return { ...withEntry(state, { ...entry, stage: "ready", peek: action.peek, error: null, order }), order };
    }
    case "failed": {
      const entry = current(state, action.key, action.attempt);
      if (!entry) return state;
      return withEntry(state, { ...entry, stage: "failed", error: action.error });
    }
    case "dropped": {
      const entry = current(state, action.key, action.attempt);
      if (!entry) return state;
      if (entry.peek) return withEntry(state, { ...entry, stage: "ready" });
      const entries = { ...state.entries };
      delete entries[action.key];
      return { ...state, entries };
    }
  }
}

export function peekEntry(state: PeeksState, mission: MastMissionId, datasetId: string, filename: string): PeekEntry | null {
  return state.entries[peekKey(mission, datasetId, filename)] ?? null;
}

function readyPeeks(state: PeeksState, mission: MastMissionId, datasetId: string): PeekEntry[] {
  return Object.values(state.entries)
    .filter((entry) => entry.mission === mission && entry.datasetId === datasetId && entry.stage === "ready" && entry.peek !== null)
    .sort((a, b) => b.order - a.order);
}

export function latestPeek(state: PeeksState, mission: MastMissionId, datasetId: string): MastPeek | null {
  return readyPeeks(state, mission, datasetId)[0]?.peek ?? null;
}

function drizzleRank(entry: PeekEntry): number {
  return drizzled(entry.filename) ? 1 : 0;
}

export function datasetPeek(state: PeeksState, mission: MastMissionId, datasetId: string): MastPeek | null {
  const shaped = Object.values(state.entries)
    .filter((entry) => entry.mission === mission && entry.datasetId === datasetId && entry.peek !== null && entry.peek.complete && entry.peek.footprints.length > 0)
    .sort((a, b) => drizzleRank(b) - drizzleRank(a) || b.order - a.order);
  return shaped[0]?.peek ?? null;
}

export interface PeeksStore {
  getState(): PeeksState;
  dispatch(action: PeeksAction): void;
  subscribe(listener: () => void): () => void;
}

export function createPeeksStore(initial: PeeksState = createPeeksState()): PeeksStore {
  let state = initial;
  const listeners = new Set<() => void>();
  return {
    getState: () => state,
    dispatch(action) {
      const next = peeksReducer(state, action);
      if (next === state) return;
      state = next;
      [...listeners].forEach((listener) => listener());
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

export const peeksStore = createPeeksStore();

export function usePeeks(store: PeeksStore = peeksStore): PeeksState {
  return useSyncExternalStore(store.subscribe, store.getState, store.getState);
}
