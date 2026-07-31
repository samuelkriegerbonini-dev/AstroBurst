import { useSyncExternalStore } from "react";

export type ToolId = "compose" | "processing" | "stacking" | "synth" | "config" | "export" | "headers" | "analysis";
export type RightToolId = Exclude<ToolId, "compose">;

export const RIGHT_TOOLS: { id: RightToolId; label: string }[] = [
  { id: "headers", label: "Headers" },
  { id: "analysis", label: "Analysis" },
  { id: "processing", label: "Processing" },
  { id: "stacking", label: "Stacking" },
  { id: "synth", label: "Synth" },
  { id: "export", label: "Export" },
  { id: "config", label: "Settings" },
];

let rightTool: RightToolId | null = null;
const listeners = new Set<() => void>();

function emit() {
  listeners.forEach((l) => l());
}

export const rightToolStore = {
  subscribe(cb: () => void): () => void {
    listeners.add(cb);
    return () => { listeners.delete(cb); };
  },
  get: (): RightToolId | null => rightTool,
  set(id: RightToolId | null) {
    if (id === rightTool) return;
    rightTool = id;
    emit();
  },
  toggle(id: RightToolId) {
    rightToolStore.set(rightTool === id ? null : id);
  },
};

export function useRightTool(): RightToolId | null {
  return useSyncExternalStore(rightToolStore.subscribe, rightToolStore.get);
}
