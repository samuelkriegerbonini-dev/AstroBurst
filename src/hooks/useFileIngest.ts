import type { AstroFile, ProcessedFile } from "../shared/types";
import { fileStore } from "./useFileStore";
import { appModeStore } from "./useAppMode";
import { processedFilterable, productFilterStore } from "./useProductFilter";
import { FILE_STATUS } from "../utils/constants";
import { overwrittenFileIds } from "../utils/overwrittenFiles";
import { astroFileFromPath } from "../utils/validation";

export interface IngestOptions {
  quiet?: boolean;
  keepMode?: boolean;
}

export function showWorkspaceUnlessKept(options?: IngestOptions): void {
  if (!options?.keepMode) appModeStore.set("workspace");
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

function rowsHolding(path: string): ProcessedFile[] {
  return overwrittenFileIds(fileStore.getFiles(), [path]).flatMap((id) => fileStore.getFile(id) ?? []);
}

const notFailed = (row: ProcessedFile) => row.status !== FILE_STATUS.ERROR;

export function ingestUnlessLoaded(path: string, options: IngestOptions = {}): boolean {
  if (!ingestFn) return false;
  const rows = rowsHolding(path);
  if (rows.some(notFailed)) return false;
  for (const row of rows) fileStore.requeueFile(row.id);
  return ingestFiles([astroFileFromPath(path)], { quiet: true, ...options });
}

function revealInFiles(file: ProcessedFile): void {
  productFilterStore.reveal(processedFilterable(file));
}

export function showFile(file: ProcessedFile): void {
  revealInFiles(file);
  fileStore.selectFile(file.id);
}

export function openUnlessLoaded(path: string): boolean {
  const added = ingestUnlessLoaded(path);
  if (added) fileStore.selectWhenLoaded(path, revealInFiles);
  return added;
}

export function showOrOpen(path: string): boolean {
  const loaded = rowsHolding(path).find(notFailed);
  if (!loaded) return openUnlessLoaded(path);
  showFile(loaded);
  return true;
}
