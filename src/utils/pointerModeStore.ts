import { useSyncExternalStore } from "react";

export type PointerMode = "pan" | "crosshair";

let pointerMode: PointerMode = "pan";
const listeners = new Set<() => void>();

export const pointerModeStore = {
  subscribe(cb: () => void): () => void {
    listeners.add(cb);
    return () => {
      listeners.delete(cb);
    };
  },
  get(): PointerMode {
    return pointerMode;
  },
  set(value: PointerMode): void {
    if (value === pointerMode) return;
    pointerMode = value;
    listeners.forEach((l) => l());
  },
};

export function usePointerMode(): PointerMode {
  return useSyncExternalStore(pointerModeStore.subscribe, pointerModeStore.get, pointerModeStore.get);
}
