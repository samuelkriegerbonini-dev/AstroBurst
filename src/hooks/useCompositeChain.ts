import { useEffect, useSyncExternalStore } from "react";
import { useCompositePreview } from "../context/CompositeContext";
import { compositeChainState } from "../services/compositeChain";
import type { CompositeChain } from "../shared/types/compositeChain";
import { COMPOSITE_RUN_KEY, hasCompositeReset, isCompositeChainStale } from "../utils/compositeChain";
import { compositeChainStore } from "../utils/compositeChainStore";
import { useRunLocked } from "./useProcessingRun";

export function useCompositeChain(): CompositeChain {
  return useSyncExternalStore(compositeChainStore.subscribe, compositeChainStore.get, compositeChainStore.get);
}

export function useCompositeChainSync(): void {
  const { compositeVersion } = useCompositePreview();
  const compositeRunning = useRunLocked(COMPOSITE_RUN_KEY);
  useEffect(() => {
    if (compositeRunning || !hasCompositeReset(compositeChainStore.get())) return;
    let cancelled = false;
    compositeChainState()
      .then((state) => {
        if (cancelled) return;
        if (isCompositeChainStale(compositeChainStore.get(), state)) compositeChainStore.reset();
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [compositeVersion, compositeRunning]);
}
