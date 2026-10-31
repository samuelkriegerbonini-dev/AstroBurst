import { useEffect, useState } from "react";
import { getWcsInfo } from "../services/astrometry";
import { headerWcsProbe, type HeaderWcsProbe } from "../utils/solvedWcs";

export function useHeaderWcs(path: string | null): HeaderWcsProbe | null {
  const [probe, setProbe] = useState<HeaderWcsProbe | null>(null);

  useEffect(() => {
    if (path === null) return;
    let live = true;
    getWcsInfo(path).then(
      (info) => {
        if (live) setProbe(headerWcsProbe(path, info));
      },
      () => {
        if (live) setProbe(headerWcsProbe(path, null));
      },
    );
    return () => {
      live = false;
    };
  }, [path]);

  return probe !== null && probe.path === path ? probe : null;
}
