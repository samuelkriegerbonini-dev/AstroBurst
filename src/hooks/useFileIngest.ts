import type { AstroFile } from "../shared/types";
import { fileStore } from "./useFileStore";
import { FILE_STATUS } from "../utils/constants";
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
  if (!ingestFn) return false;
  const rows = overwrittenFileIds(fileStore.getFiles(), [path]).flatMap((id) => fileStore.getFile(id) ?? []);
  if (rows.some((row) => row.status !== FILE_STATUS.ERROR)) return false;
  for (const row of rows) fileStore.requeueFile(row.id);
  return ingestFiles([astroFileFromPath(path)], { quiet: true });
}
