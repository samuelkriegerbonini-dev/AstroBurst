import { useCallback, useSyncExternalStore } from "react";
import type { ImageDepth } from "../shared/types/depth";
import { depthStore } from "./useDepthStore";

export function useImageDepth(path: string | null, measureKey: string | null): ImageDepth | null {
  const latest = useCallback(() => (path ? depthStore.latest(path, measureKey) : null), [path, measureKey]);
  return useSyncExternalStore(depthStore.subscribe, latest, latest);
}
