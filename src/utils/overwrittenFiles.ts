import { parseImageRef } from "./imageRef";
import { samePath } from "./processingChain";

const FITS_OUTPUT_KEYS = ["fits_path", "r_path", "g_path", "b_path", "corrected_fits", "model_fits", "output_path"] as const;

export function writtenFitsPaths(result: unknown): string[] {
  const out: string[] = [];
  const visit = (value: unknown) => {
    if (!value || typeof value !== "object") return;
    const record = value as Record<string, unknown>;
    for (const key of FITS_OUTPUT_KEYS) {
      const path = record[key];
      if (typeof path === "string" && path.length > 0) out.push(path);
    }
    if (Array.isArray(record.results)) record.results.forEach(visit);
  };
  visit(result);
  return out;
}

export interface LoadedFileRef {
  id: string;
  path: string;
}

export function overwrittenFileIds(files: readonly LoadedFileRef[], written: readonly string[]): string[] {
  if (written.length === 0) return [];
  return files
    .filter((file) => {
      const source = parseImageRef(file.path).path;
      return written.some((path) => samePath(source, path));
    })
    .map((file) => file.id);
}
