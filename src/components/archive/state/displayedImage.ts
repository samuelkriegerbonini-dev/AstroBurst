import { useSyncExternalStore } from "react";
import { targetProjectionPath } from "../../../utils/skyList";
import type { WorkspaceImage } from "./footprintsStore";
import type { WorkspaceFileRef } from "./useAroundTarget";

export interface DisplayedImageRef {
  filePath: string;
  projectionPath: string;
}

export function displayedImageRef(filePath: string | null, displayedPath: string | null): DisplayedImageRef | null {
  const projectionPath = targetProjectionPath(filePath, displayedPath);
  return filePath === null || projectionPath === null ? null : { filePath, projectionPath };
}

export function footprintImage(file: WorkspaceFileRef | null, displayed: DisplayedImageRef | null): WorkspaceImage | null {
  if (file === null) return null;
  const projectionPath = displayed !== null && displayed.filePath === file.path ? displayed.projectionPath : file.path;
  return { path: file.path, name: file.name, projectionPath };
}

let current: DisplayedImageRef | null = null;
const listeners = new Set<() => void>();

function sameRef(a: DisplayedImageRef | null, b: DisplayedImageRef | null): boolean {
  return a === b || (a !== null && b !== null && a.filePath === b.filePath && a.projectionPath === b.projectionPath);
}

export const displayedImageStore = {
  get: (): DisplayedImageRef | null => current,
  set(next: DisplayedImageRef | null): void {
    if (sameRef(current, next)) return;
    current = next;
    [...listeners].forEach((listener) => listener());
  },
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  },
};

export function useDisplayedImageRef(): DisplayedImageRef | null {
  return useSyncExternalStore(displayedImageStore.subscribe, displayedImageStore.get, displayedImageStore.get);
}
