import { useSyncExternalStore } from "react";

export interface ToolBrowserStore {
  subscribe(cb: () => void): () => void;
  get(): boolean;
  open(): void;
  close(): void;
}

let open = false;
const listeners = new Set<() => void>();

function setOpen(next: boolean): void {
  if (next === open) return;
  open = next;
  listeners.forEach((l) => l());
}

export const toolBrowserStore: ToolBrowserStore = {
  subscribe(cb) {
    listeners.add(cb);
    return () => {
      listeners.delete(cb);
    };
  },
  get: () => open,
  open: () => setOpen(true),
  close: () => setOpen(false),
};

export function useToolBrowserOpen(): boolean {
  return useSyncExternalStore(toolBrowserStore.subscribe, toolBrowserStore.get, toolBrowserStore.get);
}
