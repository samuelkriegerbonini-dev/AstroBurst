import { useSyncExternalStore } from "react";

export type InflightKind = "thumb" | "other";

export interface InflightSnapshot {
  total: number;
  thumbs: number;
}

export interface InflightCounter {
  begin(kind: InflightKind): () => void;
  snapshot(): InflightSnapshot;
  subscribe(listener: () => void): () => void;
}

export function createInflight(): InflightCounter {
  let snap: InflightSnapshot = { total: 0, thumbs: 0 };
  const listeners = new Set<() => void>();
  const change = (delta: number, kind: InflightKind) => {
    snap = { total: snap.total + delta, thumbs: snap.thumbs + (kind === "thumb" ? delta : 0) };
    [...listeners].forEach((l) => l());
  };
  return {
    begin(kind) {
      change(1, kind);
      let open = true;
      return () => {
        if (!open) return;
        open = false;
        change(-1, kind);
      };
    },
    snapshot: () => snap,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

export const archiveInflight = createInflight();

export function useInflight(counter: InflightCounter = archiveInflight): InflightSnapshot {
  return useSyncExternalStore(counter.subscribe, counter.snapshot, counter.snapshot);
}
