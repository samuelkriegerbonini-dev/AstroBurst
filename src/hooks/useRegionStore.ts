import { useCallback, useSyncExternalStore } from "react";
import type { RegionShape, RegionTool } from "../shared/types/regions";
import { regionStore, EMPTY_DOC, type RegionDoc } from "../utils/regionStore";

export function useRegionDoc(fileKey: string | null): RegionDoc {
  const getSnapshot = useCallback(() => (fileKey ? regionStore.getDoc(fileKey) : EMPTY_DOC), [fileKey]);
  return useSyncExternalStore(regionStore.subscribe, getSnapshot, getSnapshot);
}

export function useRegionTool(): RegionTool {
  return useSyncExternalStore(regionStore.subscribe, regionStore.getTool, regionStore.getTool);
}

export function useRegionDraft(fileKey: string | null): RegionShape | null {
  const getSnapshot = useCallback(() => regionStore.getDraftFor(fileKey), [fileKey]);
  return useSyncExternalStore(regionStore.subscribe, getSnapshot, getSnapshot);
}
