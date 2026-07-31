import { typedInvoke, withPreview } from "../infrastructure/tauri";
import type { CalibrateResult, StackResult, PipelineRequest, PipelineResult, CalibrateOptions, StackOptions, DrizzleRgbOptions, DrizzleRgbResult } from "../shared/types/stacking";

export function calibrate(
  sciencePath: string,
  outputDir?: string,
  options: CalibrateOptions = {},
): Promise<CalibrateResult> {
  return withPreview<CalibrateResult>("calibrate", outputDir, { sciencePath, ...options });
}

export function stackFrames(
  paths: string[],
  outputDir?: string,
  options: StackOptions = {},
): Promise<StackResult> {
  const { name, ...rest } = options;
  return withPreview<StackResult>("stack", outputDir, { paths, name, ...rest });
}

export function drizzleFrames(
  paths: string[],
  outputDir?: string,
  options: { scale?: number; pixfrac?: number; kernel?: string; align?: boolean; name?: string } = {},
): Promise<StackResult> {
  const { name, ...rest } = options;
  return withPreview<StackResult>("drizzle_stack", outputDir, { paths, name, ...rest });
}

export function runCalibrationPipeline(request: PipelineRequest): Promise<PipelineResult> {
  return typedInvoke<PipelineResult>("run_pipeline_cmd", { request });
}

function channelOrNull(paths: string[]): string[] | null {
  return paths.length >= 2 ? paths : null;
}

export function drizzleRgbStack(
  rPaths: string[],
  gPaths: string[],
  bPaths: string[],
  outputDir?: string,
  options: DrizzleRgbOptions = {},
): Promise<DrizzleRgbResult> {
  return withPreview<DrizzleRgbResult>("drizzle_rgb_cmd", outputDir, {
    rPaths: channelOrNull(rPaths),
    gPaths: channelOrNull(gPaths),
    bPaths: channelOrNull(bPaths),
    scale: options.scale ?? 2.0,
    pixfrac: options.pixfrac ?? 0.7,
    kernel: options.kernel ?? "square",
    align: options.align ?? true,
    alignmentMethod: options.alignmentMethod ?? null,
    sigmaLow: options.sigmaLow ?? null,
    sigmaHigh: options.sigmaHigh ?? null,
    wbMode: options.wbMode ?? null,
    wbR: options.wbR ?? null,
    wbG: options.wbG ?? null,
    wbB: options.wbB ?? null,
    scnrEnabled: options.scnrEnabled ?? null,
    scnrAmount: options.scnrAmount ?? null,
    saveFits: options.saveFits ?? false,
  });
}
