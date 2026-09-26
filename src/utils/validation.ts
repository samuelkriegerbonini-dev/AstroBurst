import type { AstroFile } from "../shared/types";

const VALID_EXTENSIONS = [".fits", ".fit", ".fts", ".fz", ".asdf", ".zip"];

export const SUPPORTED_EXTENSIONS: readonly string[] = VALID_EXTENSIONS.map((ext) => ext.slice(1));

export const SUPPORTED_EXTENSIONS_LABEL = VALID_EXTENSIONS.join(" ");

export function isValidFitsFile(nameOrPath: string): boolean {
  const lower = nameOrPath.toLowerCase();
  return VALID_EXTENSIONS.some((ext) => lower.endsWith(ext));
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
    if (!isValidFitsFile(path)) partition.skipped.push(item);
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
