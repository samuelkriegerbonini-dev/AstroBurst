import { withPreview, typedInvoke, getOutputDir } from "../infrastructure/tauri";
import type { StfParams } from "../shared/types";
import type {
  BlendResult,
  AlignResult,
  RestretchResult,
  AutoWbResult,
  CalibrateCompositeResult,
  ScnrOptions,
} from "../shared/types/compose";

export function restretchComposite(
  outputDir: string,
  stfR: StfParams,
  stfG: StfParams,
  stfB: StfParams,
  scnr?: ScnrOptions,
): Promise<RestretchResult> {
  return typedInvoke<RestretchResult>("restretch_composite_cmd", {
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
  return typedInvoke<void>("clear_composite_cache_cmd", {});
}

export function updateCompositeChannel(channel: string, path: string): Promise<void> {
  return typedInvoke<void>("update_composite_channel_cmd", { channel, path });
}

export function blendChannels(
  channelPaths: string[],
  weights: { channelIdx: number; r: number; g: number; b: number }[],
  outputDir?: string,
  options: { preset?: string; autoStretch?: boolean; linkedStf?: boolean } = {},
): Promise<BlendResult> {
  return withPreview<BlendResult>("blend_channels_cmd", outputDir, {
    channelPaths,
    weights,
    preset: options.preset ?? "",
    autoStretch: options.autoStretch ?? true,
    linkedStf: options.linkedStf ?? false,
  });
}

export async function alignChannels(
  paths: string[],
  outputDir?: string,
  alignMethod = "phase_correlation",
): Promise<AlignResult> {
  const dir = outputDir ?? await getOutputDir();
  return typedInvoke<AlignResult>("align_channels_cmd", {
    paths,
    outputDir: dir,
    alignMethod,
  });
}

export function calibrateComposite(
  outputDir: string,
  rFactor: number,
  gFactor: number,
  bFactor: number,
): Promise<CalibrateCompositeResult> {
  return typedInvoke<CalibrateCompositeResult>("calibrate_composite_cmd", {
    outputDir,
    rFactor,
    gFactor,
    bFactor,
  });
}

export function computeAutoWb(): Promise<AutoWbResult> {
  return typedInvoke<AutoWbResult>("compute_auto_wb_cmd", {});
}

export function resetWb(outputDir = "./output"): Promise<void> {
  return typedInvoke<void>("reset_wb_cmd", { outputDir });
}
