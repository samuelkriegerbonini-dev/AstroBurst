import { withPreview, safeInvoke } from "../infrastructure/tauri";
import type { StfParams } from "../shared/types";

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

export function restretchComposite(
  outputDir: string,
  stfR: StfParams,
  stfG: StfParams,
  stfB: StfParams,
  scnr?: { enabled: boolean; method?: string; amount?: number },
) {
  return safeInvoke("restretch_composite_cmd", {
    outputDir,
    shadowR: stfR.shadow,
    midtoneR: stfR.midtone,
    highlightR: stfR.highlight,
    shadowG: stfG.shadow,
    midtoneG: stfG.midtone,
    highlightG: stfG.highlight,
    shadowB: stfB.shadow,
    midtoneB: stfB.midtone,
    highlightB: stfB.highlight,
    scnrEnabled: scnr?.enabled ?? false,
    scnrMethod: scnr?.method,
    scnrAmount: scnr?.amount,
  });
}

export function clearCompositeCache(): Promise<void> {
  return safeInvoke("clear_composite_cache_cmd", {});
}
