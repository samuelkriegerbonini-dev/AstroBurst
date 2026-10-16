import { useEffect, useState } from "react";
import { getWcsInfo } from "../../../services/astrometry";
import type { WcsInfo } from "../../../shared/types/astrometry";
import { aroundFromWcs, type AroundTarget } from "../logic/aroundQuery";

export interface WorkspaceFileRef {
  path: string;
  name: string;
}

const solved = new Map<string, Promise<AroundTarget | null>>();

export function aroundTargetFor(path: string, name: string, read: (path: string) => Promise<WcsInfo> = getWcsInfo): Promise<AroundTarget | null> {
  const key = `${path}\n${name}`;
  const hit = solved.get(key);
  if (hit) return hit;
  const next = read(path)
    .then((wcs) => aroundFromWcs(wcs, name), () => null)
    .then((target) => {
      if (!target) solved.delete(key);
      return target;
    });
  solved.set(key, next);
  return next;
}

export function useAroundTarget(file: WorkspaceFileRef | null): AroundTarget | null {
  const path = file?.path ?? null;
  const name = file?.name ?? null;
  const key = path !== null && name !== null ? `${path}\n${name}` : null;
  const [found, setFound] = useState<{ key: string; target: AroundTarget | null } | null>(null);
  useEffect(() => {
    if (path === null || name === null) return;
    let alive = true;
    const at = `${path}\n${name}`;
    void aroundTargetFor(path, name).then((target) => {
      if (alive) setFound((prev) => (prev && prev.key === at && prev.target === target ? prev : { key: at, target }));
    });
    return () => {
      alive = false;
    };
  }, [path, name]);
  return found && found.key === key ? found.target : null;
}
