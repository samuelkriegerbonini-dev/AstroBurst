import { isFileRgbView } from "./analysisTarget";
import type { ProcessedResult } from "../shared/types/preview";

export const RGB_PLANES = 3;

export type RgbProduct =
  | { kind: "file"; path: string }
  | { kind: "result"; path: string; label: string; displayReferred: boolean }
  | { kind: "composite" };

export interface RgbProductInput {
  compositePreviewUrl: string | null;
  fileIsRgb: boolean;
  filePath: string | null;
  filePreviewUrl: string | null;
  processed: Pick<ProcessedResult, "fitsPath" | "label" | "planes" | "domain"> | null;
}

export function rgbProductOf(input: RgbProductInput): RgbProduct | null {
  if (input.compositePreviewUrl !== null && !isFileRgbView(input)) return { kind: "composite" };
  const { processed } = input;
  if (processed !== null) {
    if (processed.planes !== RGB_PLANES || processed.fitsPath === null) return null;
    return { kind: "result", path: processed.fitsPath, label: processed.label, displayReferred: processed.domain === "display_referred" };
  }
  return input.fileIsRgb && input.filePath !== null ? { kind: "file", path: input.filePath } : null;
}

export function rgbViewOf(input: RgbProductInput): RgbProduct | null {
  const product = rgbProductOf(input);
  return product?.kind === "file" && !isFileRgbView(input) ? null : product;
}

export function rgbProductPath(product: RgbProduct | null): string | null {
  return product === null || product.kind === "composite" ? null : product.path;
}
