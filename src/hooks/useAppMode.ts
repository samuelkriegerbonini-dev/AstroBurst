import { useSyncExternalStore } from "react";

export type AppMode = "workspace" | "archive";

export interface AppModeStore {
  get(): AppMode;
  set(mode: AppMode): void;
  subscribe(cb: () => void): () => void;
}

export function createAppModeStore(initial: AppMode = "workspace"): AppModeStore {
  let mode = initial;
  const listeners = new Set<() => void>();
  return {
    get: () => mode,
    set: (next: AppMode) => {
      if (next === mode) return;
      mode = next;
      listeners.forEach((l) => l());
    },
    subscribe: (cb: () => void) => {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
  };
}

export const appModeStore = createAppModeStore();

export function useAppMode(): AppMode {
  return useSyncExternalStore(appModeStore.subscribe, appModeStore.get);
}

const DIGIT_MODES: Record<string, AppMode> = {
  Digit1: "workspace",
  Numpad1: "workspace",
  Digit2: "archive",
  Numpad2: "archive",
};

export function modeFromShortcut(e: Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">): AppMode | null {
  if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return null;
  if (/^(Digit|Numpad)\d$/.test(e.code)) return DIGIT_MODES[e.code] ?? null;
  if (e.key === "1") return "workspace";
  if (e.key === "2") return "archive";
  return null;
}
