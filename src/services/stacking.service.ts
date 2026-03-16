import { safeInvoke, withPreview } from "../infrastructure/tauri";

export function calibrate(
  sciencePath: string,
  outputDir?: string,
  options: Record<string, any> = {},
) {
  return withPreview("calibrate", outputDir, { sciencePath, ...options });
}

export function stackFrames(
  paths: string[],
  outputDir?: string,
  options: Record<string, any> = {},
) {
  return withPreview("stack", outputDir, { paths, ...options });
}

export function runCalibrationPipeline(request: {
  channels: { label: string; paths: string[] }[];
  dark_paths: string[];
  flat_paths: string[];
  bias_paths: string[];
  sigma_low?: number;
  sigma_high?: number;
  normalize?: boolean;
}) {
  return safeInvoke("run_pipeline_cmd", { request });
}
