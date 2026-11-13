import { useEffect, useRef, useState } from "react";
import { getHeader } from "../services/header";
import { colorbarUnit } from "../utils/colorbar";

export function useImageUnit(path: string | null, enabled: boolean): string | null {
  const [loaded, setLoaded] = useState<{ path: string; unit: string | null } | null>(null);
  const seqRef = useRef(0);
  const target = enabled ? path : null;

  useEffect(() => {
    const seq = ++seqRef.current;
    if (target === null) return;
    getHeader(target).then(
      (header) => {
        if (seqRef.current === seq) setLoaded({ path: target, unit: colorbarUnit(header.BUNIT, false) });
      },
      () => {
        if (seqRef.current === seq) setLoaded(null);
      },
    );
  }, [target]);

  return target !== null && loaded !== null && loaded.path === target ? loaded.unit : null;
}
