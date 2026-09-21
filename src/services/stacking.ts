import { typedInvoke, withPreview } from "../infrastructure/tauri";
import type { CalibrateResult, StackResult, PipelineRequest, PipelineResult, CalibrateOptions, StackOptions, DrizzleRgbOptions, DrizzleRgbResult } from "../shared/types/stacking";
import { evaluateNoiseBatch } from "./statistics";
import { noiseWeightsFromSigmas, type NoiseWeightSummary } from "../utils/noiseWeights";

export async function noiseWeightsFor(paths: string[]): Promise<NoiseWeightSummary> {
  const batch = await evaluateNoiseBatch(paths);
  const sigmaByPath = new Map(batch.results.map((entry) => [entry.path, entry.sigma]));
  return noiseWeightsFromSigmas(paths.map((path) => sigmaByPath.get(path) ?? null));
}

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
  options: {
    scale?: number;
    pixfrac?: number;
    kernel?: string;
    align?: boolean;
    name?: string;
    rejection?: string;
    sigmaLow?: number;
    sigmaHigh?: number;
  } = {},
): Promise<StackResult> {
  const { name, ...rest } = options;
  return withPreview<StackResult>("drizzle_stack", outputDir, { paths, name, ...rest });
}

export function runCalibrationPipeline(request: PipelineRequest): Promise<PipelineResult> {
  return typedInvoke<PipelineResult>("run_pipeline_cmd", { request });
}

export const MIN_DRIZZLE_FRAMES_PER_CHANNEL = 2;

export function channelOrNull(paths: string[]): string[] | null {
  return paths.length >= MIN_DRIZZLE_FRAMES_PER_CHANNEL ? paths : null;
}

export async function drizzleRgbStack(
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
    rejection: options.rejection ?? null,
    wbMode: options.wbMode ?? null,
    wbR: options.wbR ?? null,
    wbG: options.wbG ?? null,
    wbB: options.wbB ?? null,
    scnrEnabled: options.scnrEnabled ?? null,
    scnrAmount: options.scnrAmount ?? null,
    scnrMethod: options.scnrMethod ?? null,
    saveFits: options.saveFits ?? false,
  });
}
