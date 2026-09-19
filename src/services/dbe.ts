import { withPreview } from "../infrastructure/tauri";
import type { DbeConfig, DbeResult } from "../shared/types/dbe";

export function extractBackgroundDbe(
  path: string,
  outputDir: string | undefined,
  config: DbeConfig,
): Promise<DbeResult> {
  return withPreview<DbeResult>("extract_background_dbe_cmd", outputDir, { path, config }, [
    ["corrected_png", "previewUrl"],
    ["model_png", "modelUrl"],
  ]);
}
