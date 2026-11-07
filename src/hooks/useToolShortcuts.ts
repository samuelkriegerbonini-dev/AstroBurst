import { useSyncExternalStore } from "react";
import type { DockToolId } from "../utils/dockLayout";
import { DEFAULT_TOOL_SHORTCUTS, TOOL_SHORTCUTS_KEY, initialToolShortcuts, toggleToolShortcut } from "../utils/toolCatalog";

export type ToolShortcutsStorage = Pick<Storage, "getItem" | "setItem">;

export interface ToolShortcutsStore {
  subscribe(cb: () => void): () => void;
  get(): readonly DockToolId[];
  resetCount(): number;
  toggle(tool: DockToolId): void;
  reset(): void;
}

function browserStorage(): ToolShortcutsStorage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

export function createToolShortcutsStore(storage: () => ToolShortcutsStorage | null): ToolShortcutsStore {
  const listeners = new Set<() => void>();
  const save = (list: readonly DockToolId[]) => {
    try {
      storage()?.setItem(TOOL_SHORTCUTS_KEY, JSON.stringify(list));
    } catch { }
  };
  let shortcuts: readonly DockToolId[] = initialToolShortcuts(storage());
  let resets = 0;
  save(shortcuts);
  const publish = (next: readonly DockToolId[]) => {
    shortcuts = next;
    save(next);
    listeners.forEach((l) => l());
  };
  return {
    subscribe(cb) {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    get: () => shortcuts,
    resetCount: () => resets,
    toggle(tool) {
      publish(toggleToolShortcut(shortcuts, tool, shortcuts.includes(tool)));
    },
    reset() {
      resets += 1;
      publish([...DEFAULT_TOOL_SHORTCUTS]);
    },
  };
}

export const toolShortcutsStore = createToolShortcutsStore(browserStorage);

export function useToolShortcuts(): readonly DockToolId[] {
  return useSyncExternalStore(toolShortcutsStore.subscribe, toolShortcutsStore.get, toolShortcutsStore.get);
}

export function useToolShortcutResetCount(): number {
  return useSyncExternalStore(toolShortcutsStore.subscribe, toolShortcutsStore.resetCount, toolShortcutsStore.resetCount);
}
