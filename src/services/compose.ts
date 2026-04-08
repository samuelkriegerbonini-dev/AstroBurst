import { withPreview, typedInvoke, getOutputDir } from "../infrastructure/tauri";
import type { StfParams } from "../shared/types";
import type {
  BlendResult,
  AlignResult,
  RestretchResult,
  AutoWbResult,
  CalibrateAndScnrResult,
  ResetWbResult,
  ScnrOptions,
} from "../shared/types/compose";

export interface CropResult {
  paths: string[];
  cache_keys?: string[];
  dimensions: [number, number];
  crop_top: number;
  crop_bottom: number;
  crop_left: number;
  crop_right: number;
  elapsed_ms: number;
}

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
  options: { preset?: string } = {},
): Promise<BlendResult> {
  return withPreview<BlendResult>("blend_channels_cmd", outputDir, {
    channelPaths,
    weights,
    preset: options.preset ?? "",
  });
}

export async function alignChannels(
  paths: string[],
  outputDir?: string,
  alignMethod = "phase_correlation",
  binIds?: string[],
): Promise<AlignResult> {
  const dir = outputDir ?? await getOutputDir();
  return typedInvoke<AlignResult>("align_channels_cmd", {
    paths,
    outputDir: dir,
    alignMethod,
    binIds: binIds ?? null,
    persistToDisk: false,
  });
}

export async function cropChannels(
  paths: string[],
  outputDir?: string,
  top?: number,
  bottom?: number,
  left?: number,
  right?: number,
  autoDetect?: boolean,
  binIds?: string[],
): Promise<CropResult> {
  const dir = outputDir ?? await getOutputDir();
  return typedInvoke<CropResult>("crop_channels_cmd", {
    paths,
    outputDir: dir,
    top: top ?? 0,
    bottom: bottom ?? 0,
    left: left ?? 0,
    right: right ?? 0,
    autoDetect: autoDetect ?? true,
    binIds: binIds ?? null,
    persistToDisk: false,
  });
}

export function calibrateAndScnr(
  outputDir: string,
  rFactor: number,
  gFactor: number,
  bFactor: number,
  scnr?: {
    enabled: boolean;
    method: string;
    amount: number;
    preserveLuminance: boolean;
  },
): Promise<CalibrateAndScnrResult> {
  return typedInvoke<CalibrateAndScnrResult>("calibrate_and_scnr_cmd", {
    outputDir,
    rFactor,
    gFactor,
    bFactor,
    scnrEnabled: scnr?.enabled ?? false,
    scnrMethod: scnr?.method,
    scnrAmount: scnr?.amount,
    scnrPreserveLuminance: scnr?.preserveLuminance,
  });
}

export function computeAutoWb(): Promise<AutoWbResult> {
  return typedInvoke<AutoWbResult>("compute_auto_wb_cmd", {});
}

export function resetWb(outputDir: string): Promise<ResetWbResult> {
  return typedInvoke<ResetWbResult>("reset_wb_cmd", { outputDir });
}
