import { useEffect, useRef, useState } from "react";
import { checkPointingOverlap } from "../services/astrometry";
import type { PointingOverlapPair, PointingOverlapResult } from "../shared/types/astrometry";

const DEBOUNCE_MS = 400;
const resultCache = new Map<string, PointingOverlapResult>();

export interface PointingOverlapState {
  disjointPairs: PointingOverlapPair[];
  checking: boolean;
}

export function usePointingOverlap(paths: string[]): PointingOverlapState {
  const [disjointPairs, setDisjointPairs] = useState<PointingOverlapPair[]>([]);
  const [checking, setChecking] = useState(false);
  const sigRef = useRef("");

  const sig = paths.length >= 2 ? [...paths].sort().join("|") : "";

  useEffect(() => {
    if (!sig) {
      sigRef.current = "";
      setDisjointPairs([]);
      setChecking(false);
      return;
    }
    if (sigRef.current === sig) return;
    sigRef.current = sig;

    const cached = resultCache.get(sig);
    if (cached) {
      setDisjointPairs(cached.pairs.filter((p) => p.status === "disjoint"));
      setChecking(false);
      return;
    }

    setChecking(true);
    const timer = window.setTimeout(() => {
      const requestSig = sig;
      checkPointingOverlap(requestSig.split("|"))
        .then((res) => {
          resultCache.set(requestSig, res);
          if (sigRef.current !== requestSig) return;
          setDisjointPairs(res.pairs.filter((p) => p.status === "disjoint"));
        })
        .catch(() => {
          if (sigRef.current !== requestSig) return;
          setDisjointPairs([]);
        })
        .finally(() => {
          if (sigRef.current !== requestSig) return;
          setChecking(false);
        });
    }, DEBOUNCE_MS);

    return () => window.clearTimeout(timer);
  }, [sig]);

  return { disjointPairs, checking };
}
