import { withPreview } from "../infrastructure/tauri";

export function composeRgb(
  lPath: string | null,
  rPath: string | null,
  gPath: string | null,
  bPath: string | null,
  outputDir?: string,
  options: Record<string, any> = {},
) {
  return withPreview("compose_rgb_cmd", outputDir, { lPath, rPath, gPath, bPath, ...options });
}

export function drizzleStack(
  paths: string[],
  outputDir?: string,
  options: Record<string, any> = {},
) {
  return withPreview("drizzle_stack_cmd", outputDir, { paths, ...options }, [
    ["png_path", "previewUrl"],
    ["weight_map_path", "weightMapUrl"],
  ]);
}

export function drizzleRgb(
  rPaths: string[] | null,
  gPaths: string[] | null,
  bPaths: string[] | null,
  outputDir?: string,
  options: Record<string, any> = {},
) {
  return withPreview("drizzle_rgb_cmd", outputDir, { rPaths, gPaths, bPaths, ...options });
}
