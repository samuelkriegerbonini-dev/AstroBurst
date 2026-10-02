import { useSyncExternalStore } from "react";

let gpuDisplay = false;
const listeners = new Set<() => void>();

export const gpuDisplayStore = {
  subscribe(cb: () => void): () => void {
    listeners.add(cb);
    return () => { listeners.delete(cb); };
  },
  get(): boolean {
    return gpuDisplay;
  },
  set(value: boolean): void {
    if (value === gpuDisplay) return;
    gpuDisplay = value;
    listeners.forEach((l) => l());
  },
};

export function useGpuDisplay(): boolean {
  return useSyncExternalStore(gpuDisplayStore.subscribe, gpuDisplayStore.get, gpuDisplayStore.get);
}
