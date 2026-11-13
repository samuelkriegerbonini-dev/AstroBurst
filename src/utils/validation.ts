import type { AstroFile } from "../shared/types";

const FITS_SPELLINGS = [".fits", ".fit", ".fts"];
const GZIP_SUFFIX = ".gz";
const BZIP2_SUFFIX = ".bz2";
const GZIP_FITS_EXTENSIONS = FITS_SPELLINGS.map((ext) => `${ext}${GZIP_SUFFIX}`);
const BZIP2_FITS_EXTENSIONS = FITS_SPELLINGS.map((ext) => `${ext}${BZIP2_SUFFIX}`);
const VALID_EXTENSIONS = [...FITS_SPELLINGS, ".fz", ".asdf", ".zip", ...GZIP_FITS_EXTENSIONS, ...BZIP2_FITS_EXTENSIONS];
const TEXT_TABLE_EXTENSIONS = [".csv", ".ecsv"];
const INGESTIBLE_EXTENSIONS = [...VALID_EXTENSIONS, ...TEXT_TABLE_EXTENSIONS];

export const SUPPORTED_EXTENSIONS: readonly string[] = INGESTIBLE_EXTENSIONS.map((ext) => ext.slice(1));

export const SUPPORTED_EXTENSIONS_LABEL = INGESTIBLE_EXTENSIONS.join(" ");

export const FITS_FILE_EXTENSIONS: readonly string[] = [...FITS_SPELLINGS, ...GZIP_FITS_EXTENSIONS].map((ext) => ext.slice(1));

export function dialogExtensions(extensions: readonly string[], mac: boolean): string[] {
  if (!mac) return [...extensions];
  const lastExtensions = [GZIP_SUFFIX, BZIP2_SUFFIX].filter((suffix) => extensions.some((ext) => ext.endsWith(suffix)));
  return [...extensions, ...lastExtensions.map((suffix) => suffix.slice(1))];
}

function endsWithAny(nameOrPath: string, extensions: readonly string[]): boolean {
  const lower = nameOrPath.toLowerCase();
  return extensions.some((ext) => lower.endsWith(ext));
}

export function isValidFitsFile(nameOrPath: string): boolean {
  return endsWithAny(nameOrPath, VALID_EXTENSIONS);
}

export function isTextTableFile(nameOrPath: string): boolean {
  return endsWithAny(nameOrPath, TEXT_TABLE_EXTENSIONS);
}

export function isIngestibleFile(nameOrPath: string): boolean {
  return isValidFitsFile(nameOrPath) || isTextTableFile(nameOrPath);
}

const CALIB_REF_RE =
  /^jwst_[a-z]+_(distortion|filteroffset|sirskernel|photom|flat|dark|bias|readnoise|gain|linearity|saturation|superbias|ipc|area|specwcs|regions|wavelengthrange|trappars|mask|drizpars|throughput|psfmask)_\d+\.asdf$/i;

export function isCalibRefAsdf(name: string): boolean {
  return CALIB_REF_RE.test(name);
}

function baseName(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

export function astroFileFromPath(path: string): AstroFile {
  return { name: baseName(path), path, size: 0 };
}

export interface IncomingPartition<T> {
  accepted: T[];
  skipped: T[];
  calib: T[];
}

export function partitionIncoming<T = string>(items: readonly T[], pathOf: (item: T) => string = String): IncomingPartition<T> {
  const partition: IncomingPartition<T> = { accepted: [], skipped: [], calib: [] };
  for (const item of items) {
    const path = pathOf(item);
    if (!isIngestibleFile(path)) partition.skipped.push(item);
    else if (isCalibRefAsdf(baseName(path))) partition.calib.push(item);
    else partition.accepted.push(item);
  }
  return partition;
}

export interface IngestReport {
  skipped: number;
  calib: number;
  message: string | null;
}

export function rejectionReport(partition: IncomingPartition<unknown>): IngestReport | null {
  const skipped = partition.skipped.length;
  const calib = partition.calib.length;
  return skipped === 0 && calib === 0 ? null : { skipped, calib, message: null };
}

export function folderReport(dir: string, partition: IncomingPartition<unknown>): IngestReport | null {
  const calib = partition.calib.length;
  if (partition.accepted.length > 0) return calib > 0 ? { skipped: 0, calib, message: null } : null;
  const kind = calib > 0 ? "loadable FITS/ASDF files" : "FITS/ASDF files";
  return { skipped: 0, calib, message: `No ${kind} directly in ${dir} (subfolders are not scanned)` };
}

export function folderErrorReport(dir: string, error: unknown): IngestReport {
  const reason = error instanceof Error ? error.message : String(error);
  return { skipped: 0, calib: 0, message: `Could not read ${dir}: ${reason}` };
}

export interface DirEntryLike {
  name?: string | null;
  isDirectory: boolean;
}

export type DirReader = (dir: string) => Promise<readonly DirEntryLike[]>;

export interface DroppedPaths {
  paths: string[];
  emptyFolders: string[];
}

export function childPath(dir: string, name: string): string {
  const sep = dir.includes("\\") ? "\\" : "/";
  return `${dir.replace(/[/\\]$/, "")}${sep}${name}`;
}

export async function expandDroppedPaths(paths: readonly string[], readDir: DirReader): Promise<DroppedPaths> {
  const expanded: DroppedPaths = { paths: [], emptyFolders: [] };
  for (const path of paths) {
    if (isIngestibleFile(path)) {
      expanded.paths.push(path);
      continue;
    }
    let entries: readonly DirEntryLike[];
    try {
      entries = await readDir(path);
    } catch {
      expanded.paths.push(path);
      continue;
    }
    const files = entries.flatMap((entry) => (!entry.isDirectory && entry.name && isIngestibleFile(entry.name) ? [childPath(path, entry.name)] : []));
    if (files.length === 0) expanded.emptyFolders.push(path);
    expanded.paths.push(...files);
  }
  return expanded;
}

export function dropReport(expanded: DroppedPaths, partition: IncomingPartition<unknown>): IngestReport | null {
  const base = rejectionReport(partition);
  if (expanded.emptyFolders.length === 0) return base;
  return {
    skipped: base?.skipped ?? 0,
    calib: base?.calib ?? 0,
    message: `No FITS/ASDF files directly in ${expanded.emptyFolders.join(", ")} (subfolders are not scanned)`,
  };
}

export async function ingestDrop(paths: readonly string[], readDir: DirReader): Promise<{ accepted: string[]; report: IngestReport | null }> {
  const expanded = await expandDroppedPaths(paths, readDir);
  const partition = partitionIncoming(expanded.paths);
  return { accepted: partition.accepted, report: dropReport(expanded, partition) };
}
