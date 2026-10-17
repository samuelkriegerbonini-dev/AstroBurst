import { useEffect, useState } from "react";
import { getWcsInfo } from "../../../services/astrometry";
import type { WcsInfo } from "../../../shared/types/astrometry";
import { FILE_STATUS } from "../../../utils/constants";
import { aroundFromWcs, type AroundTarget } from "../logic/aroundQuery";
import type { WcsCheck } from "./footprintsStore";

export interface WorkspaceFileRef {
  path: string;
  name: string;
}

export function workspaceFileRef(path: string | null, name: string | null, status: string | null): WorkspaceFileRef | null {
  return path !== null && name !== null && status !== FILE_STATUS.SPECTRUM ? { path, name } : null;
}

const infos = new Map<string, Promise<WcsInfo | null>>();

export function wcsInfoFor(path: string, read: (path: string) => Promise<WcsInfo> = getWcsInfo): Promise<WcsInfo | null> {
  const hit = infos.get(path);
  if (hit) return hit;
  const next = read(path).then(
    (wcs) => wcs,
    () => null,
  );
  infos.set(path, next);
  void next.then((wcs) => {
    if (!wcs && infos.get(path) === next) infos.delete(path);
  });
  return next;
}

export function aroundTargetFor(path: string, name: string, read: (path: string) => Promise<WcsInfo> = getWcsInfo): Promise<AroundTarget | null> {
  return wcsInfoFor(path, read).then((wcs) => aroundFromWcs(wcs, name));
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

export function useWcsCheck(path: string | null): WcsCheck {
  const [found, setFound] = useState<{ path: string; check: WcsCheck } | null>(null);
  useEffect(() => {
    if (path === null) return;
    let alive = true;
    void wcsInfoFor(path).then((wcs) => {
      const check: WcsCheck = wcs ? "present" : "absent";
      if (alive) setFound((prev) => (prev && prev.path === path && prev.check === check ? prev : { path, check }));
    });
    return () => {
      alive = false;
    };
  }, [path]);
  if (path === null) return "absent";
  return found && found.path === path ? found.check : "checking";
}
