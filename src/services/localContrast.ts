import { withPreview } from "../infrastructure/tauri";
import type { HdrConfig, LheConfig, LocalContrastResult } from "../shared/types/localContrast";

export function applyLhe(
  path: string,
  outputDir: string | undefined,
  config: LheConfig,
): Promise<LocalContrastResult> {
  return withPreview<LocalContrastResult>("lhe_cmd", outputDir, { path, config });
}

export function applyLheComposite(
  outputDir: string | undefined,
  config: LheConfig,
): Promise<LocalContrastResult> {
  return withPreview<LocalContrastResult>("lhe_composite_cmd", outputDir, { config });
}

export function applyHdrmt(
  path: string,
  outputDir: string | undefined,
  config: HdrConfig,
): Promise<LocalContrastResult> {
  return withPreview<LocalContrastResult>("hdrmt_cmd", outputDir, { path, config });
}

export function applyHdrmtComposite(
  outputDir: string | undefined,
  config: HdrConfig,
): Promise<LocalContrastResult> {
  return withPreview<LocalContrastResult>("hdrmt_composite_cmd", outputDir, { config });
}
