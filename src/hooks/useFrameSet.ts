import { useMemo } from "react";
import { subframeLightFiles } from "../utils/frameKind";
import type { ProcessedFile } from "../shared/types";

export interface MatchingFrames {
  files: ProcessedFile[];
  leftOutLine: string | null;
}

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

export function matchingLightFrames(files: ProcessedFile[], referenceDims: [number, number] | null): MatchingFrames {
  return subframeLightFiles(filterMatchingFrames(files, referenceDims));
}

export function orderWithReferenceFirst(files: ProcessedFile[], referencePath: string | null): ProcessedFile[] {
  if (!referencePath) return files;
  const reference = files.find((f) => pathOf(f) === referencePath);
  if (!reference) return files;
  return [reference, ...files.filter((f) => f !== reference)];
}

export function useMatchingFrames(files: ProcessedFile[], referenceDims: [number, number] | null): MatchingFrames {
  const width = referenceDims?.[0];
  const height = referenceDims?.[1];
  return useMemo(
    () => matchingLightFrames(files, width !== undefined && height !== undefined ? [width, height] : null),
    [files, width, height],
  );
}
