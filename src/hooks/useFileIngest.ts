import type { AstroFile } from "../shared/types";
import { fileStore } from "./useFileStore";
import { overwrittenFileIds } from "../utils/overwrittenFiles";
import { astroFileFromPath } from "../utils/validation";

export interface IngestOptions {
  quiet?: boolean;
}

type IngestFn = (files: AstroFile[], options?: IngestOptions) => void;

let ingestFn: IngestFn | null = null;

export function registerFileIngest(fn: IngestFn): () => void {
  ingestFn = fn;
  return () => {
    if (ingestFn === fn) ingestFn = null;
  };
}

export function ingestFiles(files: AstroFile[], options?: IngestOptions): boolean {
  if (!ingestFn || files.length === 0) return false;
  ingestFn(files, options);
  return true;
}

export function ingestUnlessLoaded(path: string): boolean {
  if (overwrittenFileIds(fileStore.getFiles(), [path]).length > 0) return false;
  return ingestFiles([astroFileFromPath(path)], { quiet: true });
}
