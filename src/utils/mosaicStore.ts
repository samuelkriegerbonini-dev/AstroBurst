import { useSyncExternalStore } from "react";
import type { PipelineLane } from "./pipelineToCompose";
import { samePath } from "./processingChain";

export interface SessionMosaic {
  fitsPath: string;
  written: readonly string[];
  name: string;
  label: PipelineLane;
  planes: number;
  dimensions: [number, number] | null;
  panels: number;
}

let mosaics: readonly SessionMosaic[] = [];
const listeners = new Set<() => void>();

function notify(): void {
  for (const listener of listeners) listener();
}

export function subscribeMosaics(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function sessionMosaics(): readonly SessionMosaic[] {
  return mosaics;
}

export function recordMosaic(mosaic: SessionMosaic): void {
  mosaics = [mosaic, ...mosaics.filter((m) => !samePath(m.fitsPath, mosaic.fitsPath))];
  notify();
}

export function relabelMosaic(fitsPath: string, label: PipelineLane): void {
  mosaics = mosaics.map((m) => (samePath(m.fitsPath, fitsPath) ? { ...m, label } : m));
  notify();
}

export function mosaicOutputPaths(): string[] {
  return mosaics.flatMap((m) => m.written);
}

export function useSessionMosaics(): readonly SessionMosaic[] {
  return useSyncExternalStore(subscribeMosaics, sessionMosaics, sessionMosaics);
}
