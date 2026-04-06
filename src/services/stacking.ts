import { typedInvoke, withPreview } from "../infrastructure/tauri";
import type { CalibrateResult, StackResult, PipelineRequest, PipelineResult, CalibrateOptions, StackOptions } from "../shared/types/stacking";

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

export function runCalibrationPipeline(request: PipelineRequest): Promise<PipelineResult> {
  return typedInvoke<PipelineResult>("run_pipeline_cmd", { request });
}
