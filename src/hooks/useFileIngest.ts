import type { AstroFile } from "../shared/types";

type IngestFn = (files: AstroFile[]) => void;

let ingestFn: IngestFn | null = null;

export function registerFileIngest(fn: IngestFn): () => void {
  ingestFn = fn;
  return () => {
    if (ingestFn === fn) ingestFn = null;
  };
}

export function ingestFiles(files: AstroFile[]): boolean {
  if (!ingestFn || files.length === 0) return false;
  ingestFn(files);
  return true;
}
