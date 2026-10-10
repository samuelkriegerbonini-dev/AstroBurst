import { withPreview } from "../infrastructure/tauri";

export type GeometryOp = "rot90" | "rot180" | "rot270" | "flip_h" | "flip_v";

export const GEOMETRY_OPS: readonly GeometryOp[] = ["rot90", "rot180", "rot270", "flip_h", "flip_v"];

export interface GeometryResult {
  png_path: string;
  fits_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  original_dimensions: [number, number];
  op: GeometryOp;
  is_rgb: boolean;
  wcs_updated: boolean;
  elapsed_ms: number;
}

export function transformGeometry(path: string, outputDir: string | undefined, op: GeometryOp): Promise<GeometryResult> {
  return withPreview<GeometryResult>("transform_geometry_cmd", outputDir, { path, op });
}
