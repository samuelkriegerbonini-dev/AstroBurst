import { useCallback, useSyncExternalStore } from "react";
import { overlayStore, EMPTY_OVERLAY_DOC, type OverlayDoc } from "../utils/overlayStore";

export function useOverlayDoc(fileKey: string | null): OverlayDoc {
  const getSnapshot = useCallback(() => (fileKey ? overlayStore.getDoc(fileKey) : EMPTY_OVERLAY_DOC), [fileKey]);
  return useSyncExternalStore(overlayStore.subscribe, getSnapshot, getSnapshot);
}
