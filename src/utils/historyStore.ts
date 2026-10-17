import { announce } from "../components/dock/useDockDrag";
import type { HistoryScope, HistoryState } from "../shared/types/ops";

export const NO_IMAGE_MESSAGE = "No image selected";

export interface HistoryBindings {
  undo: () => unknown;
  redo: () => unknown;
}

export interface HistorySnapshot {
  fileKey: string | null;
  scope: HistoryScope | null;
  state: HistoryState | null;
  error: string | null;
  pending: boolean;
}

export const EMPTY_HISTORY_SNAPSHOT: HistorySnapshot = Object.freeze({ fileKey: null, scope: null, state: null, error: null, pending: false });

let bound: HistoryBindings | null = null;
let snapshot: HistorySnapshot = EMPTY_HISTORY_SNAPSHOT;
const listeners = new Set<() => void>();

function dispatch(pick: (b: HistoryBindings) => () => unknown): void {
  if (!bound) {
    announce(NO_IMAGE_MESSAGE);
    return;
  }
  const result = pick(bound)();
  if (result instanceof Promise) result.catch(() => undefined);
}

export const historyStore = {
  bind(bindings: HistoryBindings): () => void {
    bound = bindings;
    return () => {
      if (bound === bindings) bound = null;
    };
  },
  unbind(): void {
    bound = null;
  },
  isBound: (): boolean => bound !== null,
  undo(): void {
    dispatch((b) => b.undo);
  },
  redo(): void {
    dispatch((b) => b.redo);
  },
  get: (): HistorySnapshot => snapshot,
  set(next: HistorySnapshot): void {
    if (next === snapshot) return;
    snapshot = next;
    for (const listener of listeners) listener();
  },
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  },
};
