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

export function updateCompositeChannel(channel: string, path: string) {
  return safeInvoke("update_composite_channel_cmd", { channel, path });
}

export function blendChannels(
  channelPaths: string[],
  weights: { channelIdx: number; r: number; g: number; b: number }[],
  outputDir?: string,
  options: { preset?: string; autoStretch?: boolean; linkedStf?: boolean } = {},
) {
  return withPreview("blend_channels_cmd", outputDir, {
    channelPaths,
    weights,
    preset: options.preset ?? "",
    autoStretch: options.autoStretch ?? true,
    linkedStf: options.linkedStf ?? false,
  });
}

export function alignChannels(
  paths: string[],
  outputDir?: string,
  alignMethod = "phase_correlation",
) {
  return safeInvoke("align_channels_cmd", {
    paths,
    outputDir: outputDir ?? "./output",
    alignMethod,
  });
}

export function applyScnr(
  outputDir?: string,
  options: { method?: string; amount?: number; preserveLuminance?: boolean } = {},
) {
  return withPreview("apply_scnr_cmd", outputDir, {
    method: options.method ?? "average",
    amount: options.amount ?? 0.5,
    preserveLuminance: options.preserveLuminance ?? false,
  });
}

export function calibrateComposite(
  outputDir: string,
  rFactor: number,
  gFactor: number,
  bFactor: number,
) {
  return safeInvoke("calibrate_composite_cmd", {
    outputDir,
    rFactor,
    gFactor,
    bFactor,
  });
}

export function computeAutoWb(): Promise<{
  r_factor: number;
  g_factor: number;
  b_factor: number;
  ref_channel: string;
}> {
  return safeInvoke("compute_auto_wb_cmd", {});
}
