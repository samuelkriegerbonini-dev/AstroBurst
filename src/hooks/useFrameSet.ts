import { useMemo } from "react";
import { useDoneFiles } from "./useFileStore";
import type { ProcessedFile } from "../shared/types";

export function pathOf(file: ProcessedFile): string {
  return file.path;
}

export function sameDimensions(a: [number, number] | null | undefined, b: [number, number] | null | undefined): boolean {
  return !!a && !!b && a[0] === b[0] && a[1] === b[1];
}

export function filterMatchingFrames(files: ProcessedFile[], referenceDims: [number, number] | null): ProcessedFile[] {
  if (!referenceDims) return [];
  const seen = new Set<string>();
  return files.filter((f) => {
    const path = pathOf(f);
    if (!sameDimensions(f.result?.dimensions, referenceDims) || seen.has(path)) return false;
    seen.add(path);
    return true;
  });
}

export function orderWithReferenceFirst(files: ProcessedFile[], referencePath: string | null): ProcessedFile[] {
  if (!referencePath) return files;
  const reference = files.find((f) => pathOf(f) === referencePath);
  if (!reference) return files;
  return [reference, ...files.filter((f) => f !== reference)];
}

export function useMatchingFrames(referenceDims: [number, number] | null): ProcessedFile[] {
  const files = useDoneFiles();
  const width = referenceDims?.[0];
  const height = referenceDims?.[1];
  return useMemo(
    () => filterMatchingFrames(files, width !== undefined && height !== undefined ? [width, height] : null),
    [files, width, height],
  );
}
